#![allow(clippy::doc_markdown)]

//! Axioval rule packages from buildingSMART IDS documents.
//!
//! A package importer, not a source adapter: it reads an IDS document through
//! [`openbim_ids`] and writes a [`DefinitionPackage`] and a [`RuleSetPackage`]
//! that select the engine's trusted capabilities. It never looks at a model.
//!
//! # Exact or not at all
//!
//! An IDS facet becomes a rule only when an existing capability decides it
//! exactly as IDS does. Everything else is reported as a [`Gap`], with the
//! part of the specification it concerns and why, so a caller can refuse an
//! incomplete translation instead of mistaking silence for a pass.
//!
//! The two sides of a specification fail differently:
//!
//! - **Applicability** decides which objects are checked. Dropping a facet
//!   would widen the population and report objects IDS never applied to, so
//!   one untranslatable applicability facet leaves the whole specification
//!   without rules.
//! - **Requirements** are independent of each other. Dropping one can only
//!   miss failures, never invent them, so the others are still translated.
//!
//! # What becomes what
//!
//! - The applicability is one selector: the entity's classes (never their
//!   subclasses), its predefined type, and a selector for every other facet.
//! - A property requirement becomes `property-required`,
//!   `property-data-type` or `property-value`; an attribute requirement the
//!   same in the reserved attribute set. A prohibited property or attribute
//!   without a value becomes an excluded `not-empty` row of
//!   `property-requirements`. A property value is judged under `quantifier`
//!   `any` (one value of a list, bounded, table or enumerated value), or
//!   `all` for a range restriction, with `si_units`, as IDS states measures
//!   in SI.
//! - A property set or property named by pattern, or by an enumeration of
//!   several names, is enumerated: every matching property must satisfy the
//!   facet and one must match in each matching set. Its value becomes
//!   `property-value` with `property_set_pattern` and `property_pattern`
//!   (an enumeration as an escaped alternation), its presence a
//!   `property-requirements` row with the pattern columns, required, or
//!   excluded `not-empty` when prohibited. An enumeration narrowed by
//!   patterns is the names that match them.
//! - Entity, material, part-of and name-restricted attribute requirements
//!   become `selector-conformance` with the facet's selector, negated when
//!   prohibited. A material value is tested against the material set's
//!   `Names` list, every name and category the material goes by.
//! - A classification requirement becomes `classification`: systems and
//!   codes as literals or patterns, a system alone, optional or prohibited.
//! - The applicability's `minOccurs`/`maxOccurs` become `object-count` in
//!   each source: a required specification reports a model with no
//!   applicable object, a prohibited one a model with any.
//!
//! The predefined type is resolved as IDS resolves it: the type object's
//! own, or its element or process type when that is user-defined or unset,
//! unless the result is empty or `NOTDEFINED`; otherwise the occurrence's
//! own, or its object type when user-defined or unset. A literal
//! `USERDEFINED` asks whether the type is user-defined at all.
//!
//! # Releases
//!
//! A specification's concepts are named only in the type systems of the IFC
//! releases it lists, so a rule for an IFC4-only specification cannot bind to
//! an IFC2X3 model: the engine reports it as not evaluated
//! (`InvalidDeclaration`) rather than applying it. `IFC4X3_ADD2` has no type
//! system any Axioval adapter declares and is reported as a gap.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axioval_ir::contract::{
    ColumnKind, ComparisonOperator, DefinitionPackage, ExternalName, LocalizedText,
    ObjectTypeDefinition, PackageMetadata, ParameterDefinition, ParameterKind, ParameterValue,
    PropertyDefinition, PropertySetDefinition, PropertyValueKind, Quantifier, RelatedQuantifier,
    RuleApplicability, RuleDefinition, RuleFolder, RuleInstance, RuleSetPackage, Selector,
    Severity, TableColumnDefinition, TableRow,
};
use axioval_ir::{ATTRIBUTE_SET, MATERIAL_KIND, MATERIAL_NAMES, MATERIAL_SET, TYPE_ATTRIBUTE_SET};
use ifc_schema::{Schema, TypeKind};
use openbim_ids::{
    Attribute, Classification, Entity, Facet, Ids, IfcVersion, Material, Occurrence, PartOf,
    Property, Relation, Requirement, Restriction, Specification, Value,
};
use regex::Regex;
use thiserror::Error;

/// Type system of IFC2X3 TC1, as the IFC adapter declares it.
///
/// A test pins this to the adapter's constant so this crate need not depend
/// on the adapter.
pub const IFC2X3_TYPE_SYSTEM: &str =
    "https://standards.buildingsmart.org/IFC/RELEASE/IFC2x3/TC1/HTML/";

/// Type system of IFC4 ADD2 TC1, as the IFC adapter declares it.
pub const IFC4_TYPE_SYSTEM: &str = "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4";

/// The package schema version the engine compiles.
const SCHEMA_VERSION: &str = "0.1.0";

const PROPERTY_REQUIRED: &str = "axioval:capability.property-required";
const PROPERTY_DATA_TYPE: &str = "axioval:capability.property-data-type";
const PROPERTY_VALUE: &str = "axioval:capability.property-value";
const SELECTOR_CONFORMANCE: &str = "axioval:capability.selector-conformance";
const PROPERTY_REQUIREMENTS: &str = "axioval:capability.property-requirements";
const OBJECT_COUNT: &str = "axioval:capability.object-count";
const CLASSIFICATION: &str = "axioval:capability.classification";

/// The longest `length`, `minLength` or `maxLength` a selector spells as a
/// repetition; a longer one would exceed the regular expression size limit.
const MAX_REPETITION: u64 = 1000;

/// Identity of the packages written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Qualified id of the ruleset package, such as `ids:fire-safety`. The
    /// definition package and every concept are named under it.
    pub package_id: String,
    /// Semantic version of both packages.
    pub version: String,
}

/// Why the options were refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OptionsError {
    /// The package id is not `scheme:name` with a lower-case scheme.
    #[error("package id {0:?} is not a qualified id such as `ids:fire-safety`")]
    PackageId(String),
    /// The version is not `major.minor.patch`.
    #[error("version {0:?} is not a semantic version such as `1.0.0`")]
    Version(String),
}

/// An IDS document translated into Axioval packages.
#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
    /// Object-type, property and property-set concepts, and the rule
    /// definitions the ruleset uses.
    pub definitions: DefinitionPackage,
    /// One folder per specification that produced rules.
    pub ruleset: RuleSetPackage,
    /// What became of each specification, in document order.
    pub specifications: Vec<SpecificationOutcome>,
}

impl Translation {
    /// Whether every specification translated without a gap.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.specifications
            .iter()
            .all(SpecificationOutcome::is_complete)
    }

    /// Every gap, with the specification it belongs to.
    pub fn gaps(&self) -> impl Iterator<Item = (&SpecificationOutcome, &Gap)> {
        self.specifications
            .iter()
            .flat_map(|outcome| outcome.gaps.iter().map(move |gap| (outcome, gap)))
    }
}

/// What became of one specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecificationOutcome {
    /// Position in the document, from 1.
    pub number: usize,
    /// `@name`.
    pub name: String,
    /// Ids of the rules written for it.
    pub rules: Vec<String>,
    /// What was not translated.
    pub gaps: Vec<Gap>,
}

impl SpecificationOutcome {
    /// Whether the rules decide the specification exactly as IDS does.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }

    /// Whether nothing of the specification is checked: its applicability
    /// is untranslatable, or none of its releases is supported.
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        self.gaps.iter().any(Gap::skips)
    }
}

/// Something in a specification that has no exact translation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    /// Where in the specification.
    pub part: Part,
    /// Why.
    pub reason: Reason,
}

impl Gap {
    /// Whether this gap leaves the specification without rules: an
    /// untranslatable applicability, or no release to bind to. An
    /// unsupported release beside a supported one only narrows the models
    /// the rules run on.
    fn skips(&self) -> bool {
        matches!(self.part, Part::Applicability { .. }) || self.reason == Reason::NoSupportedRelease
    }
}

impl fmt::Display for Gap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.part, self.reason)
    }
}

/// The part of a specification a [`Gap`] concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// `@ifcVersion`.
    Releases,
    /// The applicability's `minOccurs`/`maxOccurs`.
    Occurrence,
    /// An applicability facet, numbered from 1 in document order.
    Applicability {
        /// Position among the applicability facets.
        facet: usize,
    },
    /// A requirement facet, numbered from 1 in document order.
    Requirement {
        /// Position among the requirement facets.
        facet: usize,
    },
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Releases => f.write_str("ifcVersion"),
            Part::Occurrence => f.write_str("applicability occurrence"),
            Part::Applicability { facet } => write!(f, "applicability facet {facet}"),
            Part::Requirement { facet } => write!(f, "requirement facet {facet}"),
        }
    }
}

/// Why a part has no exact translation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// `IFC4X3_ADD2`, for which no adapter declares a type system.
    UnsupportedRelease(IfcVersion),
    /// No listed release is supported, so no rule could bind to any model.
    NoSupportedRelease,
    /// An applicability without facets.
    EmptyApplicability,
    /// An applicability without an entity facet, which in IDS also covers
    /// type objects and other entities a model session does not check.
    WithoutEntity,
    /// An applicability class the release does not define.
    UnknownEntity {
        /// The class as written.
        entity: String,
        /// The release that lacks it.
        release: IfcVersion,
    },
    /// An applicability or part-of class whose instances are not `IfcObject`
    /// occurrences (a type object, an IFC4 `IfcProject`, a resource), which
    /// an IFC session does not make project objects of.
    NotAnObject(String),
    /// An entity name that is not upper case, which IDS never matches.
    EntityCase(String),
    /// A name given as an `xs:restriction` with facets other than an
    /// enumeration and patterns.
    Restriction,
    /// A restriction facet no capability applies to this value, named as XML
    /// Schema spells it (`minInclusive`, `totalDigits`, ...).
    RestrictionFacet(&'static str),
    /// A restriction with no facets, whose meaning IDS leaves open.
    EmptyRestriction,
    /// An XML Schema pattern that cannot be translated exactly.
    Pattern {
        /// The pattern as written.
        pattern: String,
        /// Why it cannot be translated.
        why: String,
    },
    /// A property facet in the applicability. IDS casts its value to each
    /// property's own type; a property selector compares one declared type,
    /// and without it cannot tell a null or blank value from a present one.
    PropertyApplicability,
    /// A prohibited property facet with a value or a data type: no capability
    /// negates `property-value`.
    ProhibitedValue,
    /// An attribute whose declared type a selector cannot compare as IDS
    /// does (a real, a measure, a date, a select, a reference, an aggregate,
    /// or different types across the applicable classes).
    AttributeType {
        /// The attribute.
        attribute: String,
        /// Its declared type, or why none applies.
        declared: String,
    },
    /// An attribute name that differs from the schema's only in case: the
    /// source matches attribute names ignoring case, IDS does not.
    AttributeCase(String),
    /// An attribute facet with a restricted name and a value or a
    /// cardinality other than required; only the required name check
    /// (any of them holds a value) translates.
    AttributeNameRestriction,
    /// A value literal a selector cannot compare exactly (a boolean other
    /// than `true`/`false`, an integer written with a fraction).
    ValueLiteral(String),
    /// An applicability classification facet without a value, which asks
    /// for the system alone; the classification selector names a code. A
    /// requirement translates through the `classification` capability.
    ClassificationSystem,
    /// An applicability classification system or value given as a pattern.
    /// A requirement translates through the `classification` capability.
    ClassificationPattern,
    /// A material value restricted by several facets (an enumeration and
    /// patterns together, or a length), which one of the material's names
    /// must meet at once; separate tests over the name list cannot say so.
    MaterialValue,
    /// A part-of facet without a relation (every relation, mixed along the
    /// chain) or through `IFCRELVOIDSELEMENT IFCRELFILLSELEMENT`.
    PartOfRelation(Option<Relation>),
    /// Requirements on a prohibited specification, which IDS declares
    /// invalid: no applicable object may exist at all.
    ProhibitedRequirements,
}

impl fmt::Display for Reason {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::UnsupportedRelease(release) => {
                write!(f, "{release} has no type system an adapter declares")
            }
            Reason::NoSupportedRelease => f.write_str("no listed IFC release is supported"),
            Reason::EmptyApplicability => f.write_str("the applicability has no facets"),
            Reason::WithoutEntity => f.write_str(
                "an applicability without an entity facet also covers type objects, which a model session does not check",
            ),
            Reason::UnknownEntity { entity, release } => {
                write!(f, "{release} defines no entity {entity}")
            }
            Reason::NotAnObject(entity) => write!(
                f,
                "{entity} is not an IfcObject occurrence, which is all a model session checks"
            ),
            Reason::EntityCase(name) => {
                write!(
                    f,
                    "entity name {name:?} is not upper case, which IDS never matches"
                )
            }
            Reason::Restriction => f.write_str(
                "a name restriction with facets other than an enumeration and patterns is not translated",
            ),
            Reason::RestrictionFacet(facet) => {
                write!(f, "no capability applies the restriction facet {facet} here")
            }
            Reason::EmptyRestriction => f.write_str("a restriction without facets"),
            Reason::Pattern { pattern, why } => {
                write!(f, "pattern {pattern:?} cannot be translated exactly: {why}")
            }
            Reason::PropertyApplicability => f.write_str(
                "an applicability property facet casts its value to each property's own type, which a property selector cannot",
            ),
            Reason::ProhibitedValue => f.write_str(
                "a prohibited property facet with a value or data type has no negated property-value",
            ),
            Reason::AttributeType {
                attribute,
                declared,
            } => write!(
                f,
                "attribute {attribute} is declared {declared}, which a selector cannot compare as IDS does"
            ),
            Reason::AttributeCase(name) => write!(
                f,
                "attribute name {name:?} differs from the schema's in case, which IDS never matches"
            ),
            Reason::AttributeNameRestriction => f.write_str(
                "an attribute name restriction translates only as a required check without a value",
            ),
            Reason::ValueLiteral(literal) => {
                write!(f, "value {literal:?} cannot be compared exactly by a selector")
            }
            Reason::ClassificationSystem => f.write_str(
                "an applicability classification without a value asks for the system alone, which no selector names",
            ),
            Reason::ClassificationPattern => f.write_str(
                "an applicability classification system or value pattern has no selector",
            ),
            Reason::MaterialValue => f.write_str(
                "a material value restricted by several facets must hold for one name at once",
            ),
            Reason::PartOfRelation(None) => f.write_str(
                "a part-of facet without a relation mixes every relation along the chain",
            ),
            Reason::PartOfRelation(Some(relation)) => {
                write!(f, "a part-of facet through {relation} is not translated")
            }
            Reason::ProhibitedRequirements => f.write_str(
                "a prohibited specification takes no requirements; IDS declares them invalid",
            ),
        }
    }
}

/// Translates `ids` into packages identified by `options`.
///
/// # Errors
///
/// Returns an [`OptionsError`] when the package id or version is malformed.
/// A document that cannot be translated fully is not an error; see
/// [`Translation::specifications`].
pub fn translate(ids: &Ids, options: &Options) -> Result<Translation, OptionsError> {
    if !is_qualified_id(&options.package_id) {
        return Err(OptionsError::PackageId(options.package_id.clone()));
    }
    if !is_semver(&options.version) {
        return Err(OptionsError::Version(options.version.clone()));
    }
    let mut writer = Writer::new(options);
    let mut folders = Vec::new();
    let mut specifications = Vec::new();
    for (index, specification) in ids.specifications.iter().enumerate() {
        let number = index + 1;
        let (rules, gaps) = writer.specification(number, specification);
        let outcome = SpecificationOutcome {
            number,
            name: specification.name.clone(),
            rules: rules.iter().map(|rule| rule.id.clone()).collect(),
            gaps,
        };
        if !rules.is_empty() {
            folders.push(RuleFolder {
                id: format!("spec{number}"),
                name: LocalizedText::plain(&specification.name),
                description: specification
                    .description
                    .as_deref()
                    .map(LocalizedText::plain),
                rules,
                folders: Vec::new(),
            });
        }
        specifications.push(outcome);
    }
    let title = &ids.info.title;
    let definitions_id = format!("{}.definitions", options.package_id);
    let metadata = |id: &str, name: String| PackageMetadata {
        id: id.to_owned(),
        name: LocalizedText::plain(name),
        version: options.version.clone(),
        description: ids.info.description.as_deref().map(LocalizedText::plain),
        repository: None,
        license: None,
        authors: ids.info.author.iter().cloned().collect(),
    };
    let concepts = writer.concepts;
    Ok(Translation {
        definitions: DefinitionPackage {
            schema_version: SCHEMA_VERSION.to_owned(),
            package: metadata(&definitions_id, format!("{title}: definitions")),
            sources: BTreeMap::new(),
            object_types: concepts.object_types,
            properties: concepts.properties,
            property_sets: concepts.property_sets,
            definitions: writer.definitions,
        },
        ruleset: RuleSetPackage {
            schema_version: SCHEMA_VERSION.to_owned(),
            package: metadata(&options.package_id, title.clone()),
            sources: BTreeMap::new(),
            definition_packages: vec![definitions_id],
            root: RuleFolder {
                id: "ids".to_owned(),
                name: LocalizedText::plain(title),
                description: None,
                rules: Vec::new(),
                folders,
            },
        },
        specifications,
    })
}

/// Concepts allocated so far; cloned to roll back an untranslatable part.
#[derive(Clone, Default)]
struct Concepts {
    /// Concept ids by (kind, releases, name), so equal concepts are shared.
    ids: BTreeMap<(&'static str, Vec<IfcVersion>, String), String>,
    object_types: BTreeMap<String, ObjectTypeDefinition>,
    properties: BTreeMap<String, PropertyDefinition>,
    property_sets: BTreeMap<String, PropertySetDefinition>,
}

/// What an applicability selects, for the facets that depend on it.
struct Scope<'s> {
    releases: &'s [IfcVersion],
    /// The applicability's classes, which declare the attributes it reads.
    classes: Vec<String>,
    /// The applicability's entity facet.
    entity: Option<&'s Entity>,
}

/// A named property of a reserved set: `(set, property concept)`.
type Slot = (String, String);

/// Accumulates concepts and definitions across specifications.
struct Writer<'o> {
    options: &'o Options,
    concepts: Concepts,
    definitions: BTreeMap<String, RuleDefinition>,
}

impl<'o> Writer<'o> {
    fn new(options: &'o Options) -> Self {
        Self {
            options,
            concepts: Concepts::default(),
            definitions: BTreeMap::new(),
        }
    }

    /// Runs `translate`, keeping the concepts it allocates only if it succeeds.
    fn attempt<T>(
        &mut self,
        translate: impl FnOnce(&mut Self) -> Result<T, Reason>,
    ) -> Result<T, Reason> {
        let saved = self.concepts.clone();
        let result = translate(self);
        if result.is_err() {
            self.concepts = saved;
        }
        result
    }

    fn specification(
        &mut self,
        number: usize,
        specification: &Specification,
    ) -> (Vec<RuleInstance>, Vec<Gap>) {
        let saved = self.concepts.clone();
        let mut gaps = Vec::new();
        let releases = releases(specification, &mut gaps);
        let applicability = self.applicability(specification, &releases, &mut gaps);
        let scope = Scope {
            releases: &releases,
            classes: applicability
                .as_ref()
                .map(|(_, classes, _)| classes.clone())
                .unwrap_or_default(),
            entity: applicability.as_ref().map(|(_, _, entity)| *entity),
        };
        let prohibited = specification.applicability.max_occurs == Some(0);
        let facets: &[Requirement] = specification
            .requirements
            .as_ref()
            .map_or(&[], |requirements| requirements.facets.as_slice());
        let mut checks = Vec::new();
        if prohibited && !facets.is_empty() {
            gaps.push(Gap {
                part: Part::Occurrence,
                reason: Reason::ProhibitedRequirements,
            });
        } else {
            for (index, requirement) in facets.iter().enumerate() {
                match self.attempt(|writer| writer.requirement(requirement, &scope)) {
                    Ok(Some(check)) => checks.push((index + 1, requirement, check)),
                    Ok(None) => {}
                    Err(reason) => gaps.push(Gap {
                        part: Part::Requirement { facet: index + 1 },
                        reason,
                    }),
                }
            }
        }
        let skipped = gaps.iter().any(Gap::skips);
        let Some((selector, ..)) = applicability.filter(|_| !skipped) else {
            // Nothing is written, so nothing it would have named is either.
            self.concepts = saved;
            return (Vec::new(), gaps);
        };
        let mut rules = Vec::new();
        if let Some(check) = occurrence(specification) {
            rules.push(self.rule(number, None, specification, None, check, &selector));
        }
        for (facet, requirement, check) in checks {
            rules.push(self.rule(
                number,
                Some(facet),
                specification,
                Some(requirement),
                check,
                &selector,
            ));
        }
        (rules, gaps)
    }

    /// The applicability's selector, classes and entity facet.
    ///
    /// `None` when any facet is untranslatable; the gaps say which.
    fn applicability<'s>(
        &mut self,
        specification: &'s Specification,
        releases: &[IfcVersion],
        gaps: &mut Vec<Gap>,
    ) -> Option<(Selector, Vec<String>, &'s Entity)> {
        let facets = &specification.applicability.facets;
        if facets.is_empty() {
            gaps.push(Gap {
                part: Part::Applicability { facet: 1 },
                reason: Reason::EmptyApplicability,
            });
            return None;
        }
        // The entity comes first: the other facets read attributes its
        // classes declare.
        let mut entity = None;
        let mut failed = false;
        let mut operands = Vec::new();
        for (index, facet) in facets.iter().enumerate() {
            let Facet::Entity(facet_entity) = facet else {
                continue;
            };
            match self.attempt(|writer| writer.entity_selector(facet_entity, releases)) {
                Ok((selector, classes)) => {
                    operands.push(selector);
                    entity.get_or_insert((classes, facet_entity));
                }
                Err(reason) => {
                    failed = true;
                    gaps.push(Gap {
                        part: Part::Applicability { facet: index + 1 },
                        reason,
                    });
                }
            }
        }
        if entity.is_none() && !failed {
            gaps.push(Gap {
                part: Part::Applicability { facet: 1 },
                reason: Reason::WithoutEntity,
            });
            return None;
        }
        let scope = Scope {
            releases,
            classes: entity
                .as_ref()
                .map(|(classes, _)| classes.clone())
                .unwrap_or_default(),
            entity: entity.as_ref().map(|(_, facet)| *facet),
        };
        for (index, facet) in facets.iter().enumerate() {
            if matches!(facet, Facet::Entity(_)) {
                continue;
            }
            // Being applicable means meeting the facet as a requirement would.
            match self.attempt(|writer| writer.facet_selector(facet, &scope)) {
                Ok(selector) => operands.push(selector),
                Err(reason) => {
                    failed = true;
                    gaps.push(Gap {
                        part: Part::Applicability { facet: index + 1 },
                        reason,
                    });
                }
            }
        }
        if failed {
            return None;
        }
        let (classes, facet) = entity?;
        Some((all_of(operands), classes, facet))
    }

    /// The selector a facet other than the entity stands for, as required.
    fn facet_selector(&mut self, facet: &Facet, scope: &Scope<'_>) -> Result<Selector, Reason> {
        match facet {
            Facet::Entity(entity) => Ok(self.entity_selector(entity, scope.releases)?.0),
            Facet::Attribute(attribute) => self.attribute_selector(attribute, scope),
            Facet::Classification(classification) => classification_selector(classification),
            Facet::Material(material) => self.material_selector(material, scope.releases),
            Facet::PartOf(part_of) => self.part_of_selector(part_of, scope.releases),
            Facet::Property(_) => Err(Reason::PropertyApplicability),
        }
    }

    /// The exact check for a requirement, `None` when it always holds.
    fn requirement(
        &mut self,
        requirement: &Requirement,
        scope: &Scope<'_>,
    ) -> Result<Option<Check>, Reason> {
        let occurrence = requirement.occurrence;
        match &requirement.facet {
            Facet::Property(property) => self.property_check(property, occurrence, scope),
            Facet::Attribute(attribute) => self.attribute_check(attribute, occurrence, scope),
            Facet::Entity(entity) => {
                // Requiring the class the applicability already selected
                // always holds.
                if scope.entity.is_some_and(|applicable| applicable == entity) {
                    return Ok(None);
                }
                let (selector, _) = self.entity_requirement_selector(entity, scope.releases)?;
                Ok(Some(conformance(
                    selector,
                    occurrence,
                    &format!("is a {}", shown_entity(entity)),
                )))
            }
            Facet::Classification(classification) => {
                classification_check(classification, occurrence).map(Some)
            }
            Facet::Material(material) => {
                let selector = self.material_selector(material, scope.releases)?;
                let holds = match &material.value {
                    Some(value) => format!("has a material named {}", shown_value(value)),
                    None => "has a material".to_owned(),
                };
                if occurrence == Occurrence::Optional {
                    let Some(value) = &material.value else {
                        // Without a value: any material, or none.
                        return Ok(None);
                    };
                    // No material at all, or one going by the name.
                    let kind = (
                        MATERIAL_SET.to_owned(),
                        self.attribute(MATERIAL_KIND, scope.releases),
                    );
                    let selector = any_of(vec![unset(&kind), selector]);
                    return Ok(Some(conformance(
                        selector,
                        occurrence,
                        &format!("has no material or one named {}", shown_value(value)),
                    )));
                }
                Ok(Some(conformance(selector, occurrence, &holds)))
            }
            Facet::PartOf(part_of) => {
                let selector = self.part_of_selector(part_of, scope.releases)?;
                if occurrence == Occurrence::Optional {
                    // Being a part or not: IDS gives an optional part-of no
                    // meaning.
                    return Ok(None);
                }
                let shown = format!(
                    "is part of {} through {}",
                    shown_entity(&part_of.entity),
                    part_of.relation.map_or("any relation", Relation::as_str)
                );
                Ok(Some(conformance(selector, occurrence, &shown)))
            }
        }
    }

    fn property_check(
        &mut self,
        property: &Property,
        occurrence: Occurrence,
        scope: &Scope<'_>,
    ) -> Result<Option<Check>, Reason> {
        let sets = Names::of(&property.property_set)?;
        let names = Names::of(&property.base_name)?;
        let shown = format!("{}.{}", sets.shown(), names.shown());
        if occurrence == Occurrence::Prohibited {
            if property.value.is_some() || property.data_type.is_some() {
                return Err(Reason::ProhibitedValue);
            }
            return Ok(Some(self.prohibited_property(&sets, &names, &shown, scope)));
        }
        let optional = occurrence == Occurrence::Optional;
        let single = match (&sets, &names) {
            (Names::Literals(sets), Names::Literals(names))
                if sets.len() == 1 && names.len() == 1 =>
            {
                Some((sets[0].clone(), names[0].clone()))
            }
            _ => None,
        };
        let mut parameters = BTreeMap::new();
        if let Some(data_type) = &property.data_type {
            parameters.insert("data_type".to_owned(), string(data_type));
        }
        let kind = match (&property.value, optional, &property.data_type) {
            // Every matching property holds a value, and one matches.
            (None, false, None) if single.is_none() => {
                return Ok(Some(Check::Rows {
                    kind: Kind::Present,
                    rows: vec![pattern_row(&sets, &names, &[("requirement", "required")])],
                    title: format!("{shown} is required"),
                }));
            }
            (None, false, None) => Kind::Required,
            (None, false, Some(_)) if single.is_some() => Kind::DataType,
            // Without a value or type, an optional property is satisfied
            // whether or not it is there.
            (None, true, None) => return Ok(None),
            (None, _, Some(_)) => Kind::Value,
            (Some(value), ..) => {
                value_parameters(value, &mut parameters)?;
                Kind::Value
            }
        };
        if matches!(kind, Kind::Value) {
            value_options(property.value.as_ref(), &mut parameters);
        }
        if optional {
            parameters.insert(
                "optional".to_owned(),
                ParameterValue::Boolean { value: true },
            );
        }
        if let Some((set, name)) = single {
            let reference = ParameterValue::PropertyReference {
                property: self.property(&set, &name, scope.releases),
                property_set: Some(self.property_set(&set, scope.releases)),
            };
            parameters.insert("property".to_owned(), reference);
        } else {
            {
                parameters.insert("property_set_pattern".to_owned(), string(&sets.pattern()));
                parameters.insert("property_pattern".to_owned(), string(&names.pattern()));
            }
        }
        Ok(Some(Check::Capability {
            kind,
            title: kind.title(&shown, optional),
            parameters,
        }))
    }

    /// A prohibited property facet without a value: none of the named
    /// properties may hold a value.
    fn prohibited_property(
        &mut self,
        sets: &Names,
        names: &Names,
        shown: &str,
        scope: &Scope<'_>,
    ) -> Check {
        let rows = match (sets, names) {
            (Names::Literals(sets), Names::Literals(names)) => {
                let mut rows = Vec::new();
                for set in sets {
                    for name in names {
                        rows.push(forbidden_row(
                            self.property_set(set, scope.releases),
                            self.property(set, name, scope.releases),
                        ));
                    }
                }
                rows
            }
            // Every matching property, enumerated.
            _ => vec![pattern_row(
                sets,
                names,
                &[("state", "exclude"), ("presence", "not-empty")],
            )],
        };
        Check::Rows {
            kind: Kind::Forbidden,
            rows,
            title: format!("{shown} must not hold a value"),
        }
    }

    fn attribute_check(
        &mut self,
        attribute: &Attribute,
        occurrence: Occurrence,
        scope: &Scope<'_>,
    ) -> Result<Option<Check>, Reason> {
        let Value::Simple(name) = &attribute.name else {
            // Any of the named attributes: only the required name check
            // has a selector.
            if occurrence != Occurrence::Required || attribute.value.is_some() {
                return Err(Reason::AttributeNameRestriction);
            }
            let selector = self.attribute_selector(attribute, scope)?;
            return Ok(Some(conformance(
                selector,
                occurrence,
                &format!("has a value in {}", shown_value(&attribute.name)),
            )));
        };
        attribute_case(name, scope)?;
        let subject = format!("attribute {name}");
        let optional = occurrence == Occurrence::Optional;
        let mut parameters = BTreeMap::new();
        let kind = match (&attribute.value, occurrence) {
            // An optional attribute with no value passes whether or not it
            // is set.
            (None, Occurrence::Optional) => return Ok(None),
            (None, Occurrence::Required) => Kind::Required,
            (None, Occurrence::Prohibited) => {
                let row = forbidden_row(
                    ATTRIBUTE_SET.to_owned(),
                    self.attribute(name, scope.releases),
                );
                return Ok(Some(Check::Rows {
                    kind: Kind::Forbidden,
                    rows: vec![row],
                    title: format!("{subject} must not hold a value"),
                }));
            }
            (Some(value), Occurrence::Prohibited) => {
                let selector = self.attribute_selector(attribute, scope)?;
                return Ok(Some(conformance(
                    selector,
                    occurrence,
                    &format!("has {subject} {}", shown_value(value)),
                )));
            }
            (Some(value), Occurrence::Required | Occurrence::Optional) => {
                value_parameters(value, &mut parameters)?;
                if optional {
                    parameters.insert(
                        "optional".to_owned(),
                        ParameterValue::Boolean { value: true },
                    );
                }
                Kind::Value
            }
        };
        parameters.insert(
            "property".to_owned(),
            ParameterValue::PropertyReference {
                property: self.attribute(name, scope.releases),
                property_set: Some(ATTRIBUTE_SET.to_owned()),
            },
        );
        Ok(Some(Check::Capability {
            kind,
            title: kind.title(&subject, optional),
            parameters,
        }))
    }

    /// The selector for an entity facet whose classes must be `IfcObject`
    /// occurrences in every release: an applicability, or a part-of whole.
    fn entity_selector(
        &mut self,
        entity: &Entity,
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let names = entity_names(entity, releases)?;
        let names = occurrences(names, releases, pattern_named(entity))?;
        self.classes_selector(entity, names, releases)
    }

    /// The selector for an entity requirement: an object of another class,
    /// or of a class a release lacks, simply does not meet it.
    fn entity_requirement_selector(
        &mut self,
        entity: &Entity,
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let names = entity_names(entity, releases)?;
        self.classes_selector(entity, names, releases)
    }

    fn classes_selector(
        &mut self,
        entity: &Entity,
        names: Vec<String>,
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let classes = any_of(
            names
                .iter()
                .map(|name| Selector::EntityType {
                    object_type: self.object_type(name, releases),
                    // IDS matches the named class only, never its subclasses.
                    include_subtypes: false,
                })
                .collect(),
        );
        let selector = match &entity.predefined_type {
            None => classes,
            Some(value) => all_of(vec![classes, self.predefined_type(value, releases)?]),
        };
        Ok((selector, names))
    }

    /// Whether the object's predefined type, as IDS resolves it, meets
    /// `value`.
    fn predefined_type(
        &mut self,
        value: &Value,
        releases: &[IfcVersion],
    ) -> Result<Selector, Reason> {
        let mut slot = |set: &str, name: &str| (set.to_owned(), self.attribute(name, releases));
        let type_own = slot(TYPE_ATTRIBUTE_SET, "PredefinedType");
        let type_element = slot(TYPE_ATTRIBUTE_SET, "ElementType");
        let type_process = slot(TYPE_ATTRIBUTE_SET, "ProcessType");
        let own = slot(ATTRIBUTE_SET, "PredefinedType");
        let object = slot(ATTRIBUTE_SET, "ObjectType");
        let type_user_defined = any_of(vec![is(&type_own, "USERDEFINED"), unset(&type_own)]);
        let type_label = any_of(vec![filled(&type_element), filled(&type_process)]);
        if value.as_simple() == Some("USERDEFINED") {
            let falls_through = any_of(vec![
                all_of(vec![unset(&type_own), not(type_label.clone())]),
                is(&type_own, "NOTDEFINED"),
            ]);
            return Ok(any_of(vec![
                is(&type_own, "USERDEFINED"),
                all_of(vec![unset(&type_own), type_label]),
                all_of(vec![
                    falls_through,
                    any_of(vec![
                        is(&own, "USERDEFINED"),
                        all_of(vec![unset(&own), filled(&object)]),
                    ]),
                ]),
            ]));
        }
        let test = |(set, name): &Slot| text_test(set, name, value);
        let defined = |slot: &Slot| all_of(vec![filled(slot), not(is(slot, "NOTDEFINED"))]);
        let type_enumerated = all_of(vec![
            exists(&type_own),
            not(is(&type_own, "USERDEFINED")),
            not(is(&type_own, "NOTDEFINED")),
        ]);
        let type_decides = any_of(vec![
            type_enumerated.clone(),
            all_of(vec![
                type_user_defined.clone(),
                any_of(vec![defined(&type_element), defined(&type_process)]),
            ]),
        ]);
        let type_matches = any_of(vec![
            all_of(vec![type_enumerated, test(&type_own)?]),
            all_of(vec![
                type_user_defined,
                any_of(vec![
                    all_of(vec![defined(&type_element), test(&type_element)?]),
                    all_of(vec![defined(&type_process), test(&type_process)?]),
                ]),
            ]),
        ]);
        let occurrence_matches = any_of(vec![
            all_of(vec![
                exists(&own),
                not(is(&own, "USERDEFINED")),
                test(&own)?,
            ]),
            all_of(vec![
                any_of(vec![is(&own, "USERDEFINED"), unset(&own)]),
                test(&object)?,
            ]),
        ]);
        Ok(any_of(vec![
            type_matches,
            all_of(vec![not(type_decides), occurrence_matches]),
        ]))
    }

    /// An attribute facet as required: one of the named attributes holds a
    /// value, and every one that holds a value meets the facet's value.
    fn attribute_selector(
        &mut self,
        attribute: &Attribute,
        scope: &Scope<'_>,
    ) -> Result<Selector, Reason> {
        let names = match &attribute.name {
            Value::Simple(name) => {
                attribute_case(name, scope)?;
                vec![name.clone()]
            }
            Value::Restriction(restriction) => declared_attributes(restriction, scope)?,
        };
        let mut filled_any = Vec::new();
        let mut valued = Vec::new();
        for name in &names {
            // Declared by no applicable class: never present.
            let Some(kind) = attribute_kind(name, scope)? else {
                continue;
            };
            let slot = (
                ATTRIBUTE_SET.to_owned(),
                self.attribute(name, scope.releases),
            );
            let has_value = match kind {
                AttributeKind::Text => filled(&slot),
                // Null reads as absent; any other value is one.
                _ => exists(&slot),
            };
            if let Some(value) = &attribute.value {
                let meets = match kind {
                    AttributeKind::Text | AttributeKind::Enumeration => {
                        text_test(&slot.0, &slot.1, value)?
                    }
                    AttributeKind::Boolean => boolean_test(&slot, value)?,
                    AttributeKind::Integer => integer_test(&slot, value)?,
                };
                valued.push(any_of(vec![not(has_value.clone()), meets]));
            }
            filled_any.push(has_value);
        }
        let mut operands = vec![any_of(filled_any)];
        operands.extend(valued);
        Ok(all_of(operands))
    }

    fn material_selector(
        &mut self,
        material: &Material,
        releases: &[IfcVersion],
    ) -> Result<Selector, Reason> {
        let Some(value) = &material.value else {
            // Every material states its composition.
            let kind = (
                MATERIAL_SET.to_owned(),
                self.attribute(MATERIAL_KIND, releases),
            );
            return Ok(exists(&kind));
        };
        // IDS matches the value against every name and category the
        // material goes by, members' and their materials' included; one of
        // them must meet the whole value.
        let names = (
            MATERIAL_SET.to_owned(),
            self.attribute(MATERIAL_NAMES, releases),
        );
        let tests = match value {
            Value::Simple(literal) => vec![is(&names, literal)],
            Value::Restriction(restriction) => {
                let only_enumeration = only_enumeration(restriction);
                let only_patterns = restriction.enumeration.is_empty()
                    && !restriction.patterns.is_empty()
                    && only_names(restriction).is_ok();
                if only_enumeration {
                    vec![test(
                        &names,
                        ComparisonOperator::OneOf,
                        Some(ParameterValue::StringList {
                            value: restriction.enumeration.clone(),
                        }),
                    )]
                } else if only_patterns {
                    restriction
                        .patterns
                        .iter()
                        .map(|pattern| {
                            Ok(test(
                                &names,
                                ComparisonOperator::Matches,
                                Some(string(&translated(pattern)?)),
                            ))
                        })
                        .collect::<Result<_, Reason>>()?
                } else {
                    // Several facets must hold for one name, which
                    // separate tests over the list cannot require.
                    return Err(Reason::MaterialValue);
                }
            }
        };
        Ok(any_of(tests.into_iter().map(any_element).collect()))
    }

    fn part_of_selector(
        &mut self,
        part_of: &PartOf,
        releases: &[IfcVersion],
    ) -> Result<Selector, Reason> {
        let relationship = match part_of.relation {
            Some(Relation::Aggregates) => "IfcRelAggregates",
            Some(Relation::AssignsToGroup) => "IfcRelAssignsToGroup",
            Some(Relation::ContainedInSpatialStructure) => "IfcRelContainedInSpatialStructure",
            Some(Relation::Nests) => "IfcRelNests",
            other @ (None | Some(Relation::VoidsElementFillsElement)) => {
                return Err(Reason::PartOfRelation(other));
            }
        };
        let names = whole_names(&part_of.entity, relationship, releases)?;
        let (whole, _) = self.classes_selector(&part_of.entity, names, releases)?;
        // IDS follows the relation recursively from the part up to its
        // wholes, the relating ends.
        Ok(Selector::Related {
            path: vec![format!("{relationship}:backward+")],
            quantifier: RelatedQuantifier::Any,
            selector: Box::new(whole),
        })
    }

    fn rule(
        &mut self,
        number: usize,
        facet: Option<usize>,
        specification: &Specification,
        requirement: Option<&Requirement>,
        check: Check,
        selector: &Selector,
    ) -> RuleInstance {
        let (kind, title, parameters) = check.lower();
        let definition_id = self.definition(kind);
        let description = requirement
            .and_then(|requirement| requirement.instructions.as_deref())
            .or(specification.instructions.as_deref())
            .map(LocalizedText::plain);
        RuleInstance {
            id: match facet {
                Some(facet) => format!("spec{number}.facet{facet}"),
                None => format!("spec{number}.occurrence"),
            },
            definition_id,
            name: LocalizedText::plain(title),
            description,
            enabled: true,
            severity: Severity::Error,
            message: None,
            parameters,
            applicability: RuleApplicability::Selector(selector.clone()),
            requirements: Vec::new(),
            citations: Vec::new(),
            parameter_citations: Vec::new(),
            explanatory_images: Vec::new(),
            tags: vec!["ids".to_owned()],
            severity_bands: Vec::new(),
        }
    }

    /// The rule definition a kind of check uses, written once.
    fn definition(&mut self, kind: Kind) -> String {
        let (suffix, name, description, capability) = kind.catalog();
        let id = format!("{}.{suffix}", self.options.package_id);
        self.definitions.entry(id.clone()).or_insert_with(|| {
            let parameters = kind
                .parameters()
                .into_iter()
                .map(|parameter| (parameter.id.clone(), parameter))
                .collect();
            RuleDefinition {
                id: id.clone(),
                name: LocalizedText::plain(name),
                description: Some(LocalizedText::plain(description)),
                capability: capability.to_owned(),
                parameters,
                tags: vec!["ids".to_owned()],
                citations: Vec::new(),
            }
        });
        id
    }

    /// The id of the concept `(kind, releases, name)`, allocating it once.
    fn concept(
        &mut self,
        kind: &'static str,
        releases: &[IfcVersion],
        name: &str,
    ) -> (String, bool) {
        let key = (kind, releases.to_vec(), name.to_owned());
        if let Some(id) = self.concepts.ids.get(&key) {
            return (id.clone(), false);
        }
        let count = self
            .concepts
            .ids
            .keys()
            .filter(|(k, ..)| *k == kind)
            .count();
        let id = format!("{}.{kind}-{}", self.options.package_id, count + 1);
        self.concepts.ids.insert(key, id.clone());
        (id, true)
    }

    fn object_type(&mut self, entity: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("entity", releases, entity);
        if new {
            self.concepts.object_types.insert(
                id.clone(),
                ObjectTypeDefinition {
                    id: id.clone(),
                    name: LocalizedText::plain(entity),
                    description: None,
                    external_names: names(releases, entity),
                    citations: Vec::new(),
                },
            );
        }
        id
    }

    fn property(&mut self, set: &str, name: &str, releases: &[IfcVersion]) -> String {
        // A property is identified by its set too: `Width` in two sets is two
        // properties, even though only the name is bound.
        let (id, new) = self.concept("property", releases, &format!("{set}\u{0}{name}"));
        if new {
            self.concepts.properties.insert(
                id.clone(),
                PropertyDefinition {
                    id: id.clone(),
                    name: LocalizedText::plain(name),
                    description: Some(LocalizedText::plain(format!(
                        "{set}.{name}. IDS states no value kind for a presence check; `string` is nominal."
                    ))),
                    value_kind: PropertyValueKind::String,
                    unit_dimension: None,
                    external_names: names(releases, name),
                    citations: Vec::new(),
                },
            );
        }
        id
    }

    /// A property of a reserved set (an attribute, a material property),
    /// as a property concept without a set of its own.
    fn attribute(&mut self, name: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("attribute", releases, name);
        if new {
            self.concepts.properties.insert(
                id.clone(),
                PropertyDefinition {
                    id: id.clone(),
                    name: LocalizedText::plain(name),
                    description: Some(LocalizedText::plain(format!(
                        "{name}, read in a reserved set. IDS states no value kind; `string` is nominal."
                    ))),
                    value_kind: PropertyValueKind::String,
                    unit_dimension: None,
                    external_names: names(releases, name),
                    citations: Vec::new(),
                },
            );
        }
        id
    }

    fn property_set(&mut self, set: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("property-set", releases, set);
        if new {
            self.concepts.property_sets.insert(
                id.clone(),
                PropertySetDefinition {
                    id: id.clone(),
                    name: LocalizedText::plain(set),
                    description: None,
                    external_names: names(releases, set),
                    citations: Vec::new(),
                },
            );
        }
        id
    }
}

/// One exactly translatable requirement, or the specification's count.
enum Check {
    /// A property capability with its full parameters.
    Capability {
        kind: Kind,
        title: String,
        parameters: BTreeMap<String, ParameterValue>,
    },
    /// `selector-conformance`.
    Conforms {
        requirement: Selector,
        title: String,
        message: String,
    },
    /// `property-requirements` rows: forbidding a value, or requiring
    /// every property a pattern matches.
    Rows {
        kind: Kind,
        rows: Vec<TableRow>,
        title: String,
    },
    /// `object-count`.
    Count {
        minimum: Option<u32>,
        maximum: Option<u32>,
    },
}

impl Check {
    /// The capability, the rule's title and its parameters.
    fn lower(self) -> (Kind, String, BTreeMap<String, ParameterValue>) {
        match self {
            Check::Capability {
                kind,
                title,
                parameters,
            } => (kind, title, parameters),
            Check::Conforms {
                requirement,
                title,
                message,
            } => (
                Kind::Conformance,
                title,
                BTreeMap::from([
                    (
                        "requirement".to_owned(),
                        ParameterValue::Selector {
                            value: Box::new(requirement),
                        },
                    ),
                    ("message".to_owned(), string(&message)),
                ]),
            ),
            Check::Rows { kind, rows, title } => (
                kind,
                title,
                BTreeMap::from([(
                    "requirements".to_owned(),
                    ParameterValue::Table { value: rows },
                )]),
            ),
            Check::Count { minimum, maximum } => {
                let bound = |value: u32| ParameterValue::Integer {
                    value: value.into(),
                };
                let mut parameters = BTreeMap::new();
                if let Some(minimum) = minimum {
                    parameters.insert("minimum".to_owned(), bound(minimum));
                }
                if let Some(maximum) = maximum {
                    parameters.insert("maximum".to_owned(), bound(maximum));
                }
                let title = match (minimum, maximum) {
                    (_, Some(0)) => "no applicable object may exist".to_owned(),
                    (minimum, None) => {
                        format!("at least {} applicable object(s)", minimum.unwrap_or(0))
                    }
                    (minimum, Some(maximum)) => {
                        format!("{} to {maximum} applicable objects", minimum.unwrap_or(0))
                    }
                };
                (Kind::Count, title, parameters)
            }
        }
    }
}

/// A `selector-conformance` check: `selector` must hold, or must not when
/// prohibited. `holds` says what the selector states, such as `is a
/// IFCWALL`.
fn conformance(selector: Selector, occurrence: Occurrence, holds: &str) -> Check {
    match occurrence {
        Occurrence::Prohibited => Check::Conforms {
            requirement: not(selector),
            title: format!("prohibited: {holds}"),
            message: format!("{holds}, which IDS prohibits"),
        },
        Occurrence::Required | Occurrence::Optional => Check::Conforms {
            requirement: selector,
            title: format!("required: {holds}"),
            message: format!("does not meet the IDS requirement that it {holds}"),
        },
    }
}

/// Which capability decides a check.
#[derive(Clone, Copy)]
enum Kind {
    /// `property-required`.
    Required,
    /// `property-data-type`.
    DataType,
    /// `property-value`.
    Value,
    /// `selector-conformance`.
    Conformance,
    /// `property-requirements` with rows forbidding a value.
    Forbidden,
    /// `property-requirements` with rows requiring every property a
    /// pattern or enumeration matches to hold a value.
    Present,
    /// `object-count`.
    Count,
    /// `classification`.
    Classification,
}

/// The columns of the `property-requirements` table, as the capability
/// declares them.
const REQUIREMENT_COLUMNS: &[(&str, ColumnKind)] = &[
    ("applies_to", ColumnKind::Selector),
    ("property_set", ColumnKind::TextPattern),
    ("property", ColumnKind::TextPattern),
    ("property_set_pattern", ColumnKind::String),
    ("property_pattern", ColumnKind::String),
    ("requirement", ColumnKind::String),
    ("state", ColumnKind::String),
    ("presence", ColumnKind::String),
    ("value_like", ColumnKind::TextPattern),
    ("one_of", ColumnKind::String),
    ("one_of_like", ColumnKind::String),
    ("contains", ColumnKind::String),
    ("minimum", ColumnKind::Number),
    ("maximum", ColumnKind::Number),
    ("unit", ColumnKind::String),
    ("per", ColumnKind::String),
    ("decimals", ColumnKind::Integer),
];

impl Kind {
    /// Definition id suffix, name, description and capability.
    fn catalog(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Kind::Required => (
                "property-required",
                "Property is required",
                "An IDS property or attribute facet without a value: it must exist with a non-empty value.",
                PROPERTY_REQUIRED,
            ),
            Kind::DataType => (
                "property-data-type",
                "Property is required with a data type",
                "An IDS property facet with a dataType and no value: the property must exist with a non-empty value of that declared type.",
                PROPERTY_DATA_TYPE,
            ),
            Kind::Value => (
                "property-value",
                "Property value meets constraints",
                "An IDS property or attribute facet with a value, or an optional one with a dataType: literals and XML Schema facets cast to the value.",
                PROPERTY_VALUE,
            ),
            Kind::Conformance => (
                "facet",
                "Facet holds",
                "An IDS entity, classification, material, part-of or name-restricted attribute requirement: the facet's selector must hold, or must not when prohibited.",
                SELECTOR_CONFORMANCE,
            ),
            Kind::Forbidden => (
                "prohibited-property",
                "Property is prohibited",
                "A prohibited IDS property or attribute facet without a value: none of the named properties may hold a value.",
                PROPERTY_REQUIREMENTS,
            ),
            Kind::Present => (
                "required-properties",
                "Properties are required",
                "An IDS property facet naming its set or property by pattern or enumeration, without a value: one property must match, and every matching one must hold a value.",
                PROPERTY_REQUIREMENTS,
            ),
            Kind::Count => (
                "occurrence",
                "Applicable objects",
                "An IDS specification's minOccurs/maxOccurs: how many applicable objects each model may hold.",
                OBJECT_COUNT,
            ),
            Kind::Classification => (
                "classification",
                "Classification is required",
                "An IDS classification requirement: an assignment in a matching system with a matching code or ancestor code, none when prohibited, or none at all when optional.",
                CLASSIFICATION,
            ),
        }
    }

    /// The parameters, exactly as the capability declares them.
    fn parameters(self) -> Vec<ParameterDefinition> {
        let property = || parameter("property", ParameterKind::PropertyReference, true);
        match self {
            Kind::Required => vec![property()],
            Kind::DataType => vec![
                property(),
                parameter("data_type", ParameterKind::String, true),
            ],
            Kind::Value => {
                let mut parameters = vec![parameter(
                    "property",
                    ParameterKind::PropertyReference,
                    false,
                )];
                for id in [
                    "property_set_pattern",
                    "property_pattern",
                    "data_type",
                    "min_inclusive",
                    "max_inclusive",
                    "min_exclusive",
                    "max_exclusive",
                    "precision",
                    "quantifier",
                ] {
                    parameters.push(parameter(id, ParameterKind::String, false));
                }
                for id in ["values", "patterns"] {
                    parameters.push(parameter(id, ParameterKind::StringList, false));
                }
                for id in [
                    "length",
                    "min_length",
                    "max_length",
                    "total_digits",
                    "fraction_digits",
                ] {
                    parameters.push(parameter(id, ParameterKind::Integer, false));
                }
                for id in ["optional", "si_units"] {
                    parameters.push(parameter(id, ParameterKind::Boolean, false));
                }
                parameters
            }
            Kind::Conformance => vec![
                parameter("requirement", ParameterKind::Selector, true),
                parameter("message", ParameterKind::String, false),
            ],
            Kind::Forbidden | Kind::Present => {
                let mut table = parameter("requirements", ParameterKind::Table, true);
                table.columns = REQUIREMENT_COLUMNS
                    .iter()
                    .map(|(id, kind)| TableColumnDefinition {
                        id: (*id).to_owned(),
                        name: LocalizedText::plain(*id),
                        description: None,
                        kind: *kind,
                        required: false,
                        unit_dimension: None,
                    })
                    .collect();
                vec![
                    table,
                    parameter("case_sensitive", ParameterKind::Boolean, false),
                    parameter("area_property", ParameterKind::PropertyReference, false),
                    parameter("volume_property", ParameterKind::PropertyReference, false),
                    parameter("group_by_value", ParameterKind::Boolean, false),
                    parameter("category_property", ParameterKind::PropertyReference, false),
                ]
            }
            Kind::Count => vec![
                parameter("minimum", ParameterKind::Integer, false),
                parameter("maximum", ParameterKind::Integer, false),
                parameter("across_sources", ParameterKind::Boolean, false),
            ],
            Kind::Classification => {
                let mut parameters: Vec<_> =
                    ["codes", "code_patterns", "systems", "system_patterns"]
                        .into_iter()
                        .map(|id| parameter(id, ParameterKind::StringList, false))
                        .collect();
                for id in ["optional", "prohibited"] {
                    parameters.push(parameter(id, ParameterKind::Boolean, false));
                }
                parameters
            }
        }
    }

    /// The title of a property or attribute check on `subject`.
    fn title(self, subject: &str, optional: bool) -> String {
        match self {
            Kind::Required => format!("{subject} is required"),
            Kind::DataType => format!("{subject} is required with a declared type"),
            Kind::Value if optional => format!("{subject}, where present, meets its constraints"),
            _ => format!("{subject} meets its constraints"),
        }
    }
}

/// A parameter without default or allowed values.
fn parameter(id: &str, kind: ParameterKind, required: bool) -> ParameterDefinition {
    ParameterDefinition {
        id: id.to_owned(),
        name: LocalizedText::plain(id),
        description: None,
        kind,
        referenced_value_kind: None,
        required,
        default_value: None,
        allowed_values: Vec::new(),
        unit_dimension: None,
        columns: Vec::new(),
        citations: Vec::new(),
    }
}

/// A `property-requirements` row: the property must not hold a value.
fn forbidden_row(set: String, name: String) -> TableRow {
    TableRow::from([
        (
            "property_set".to_owned(),
            ParameterValue::String { value: set },
        ),
        (
            "property".to_owned(),
            ParameterValue::String { value: name },
        ),
        ("state".to_owned(), string("exclude")),
        ("presence".to_owned(), string("not-empty")),
    ])
}

/// The supported releases, recording a gap for each unsupported one.
fn releases(specification: &Specification, gaps: &mut Vec<Gap>) -> Vec<IfcVersion> {
    let mut supported = Vec::new();
    for release in &specification.ifc_versions {
        if type_system(*release).is_some() {
            if !supported.contains(release) {
                supported.push(*release);
            }
        } else {
            gaps.push(Gap {
                part: Part::Releases,
                reason: Reason::UnsupportedRelease(*release),
            });
        }
    }
    supported.sort_unstable();
    if supported.is_empty() {
        gaps.push(Gap {
            part: Part::Releases,
            reason: Reason::NoSupportedRelease,
        });
    }
    supported
}

fn type_system(release: IfcVersion) -> Option<&'static str> {
    match release {
        IfcVersion::Ifc2x3 => Some(IFC2X3_TYPE_SYSTEM),
        IfcVersion::Ifc4 => Some(IFC4_TYPE_SYSTEM),
        IfcVersion::Ifc4x3Add2 => None,
    }
}

fn schema(release: IfcVersion) -> Option<&'static Schema> {
    match release {
        IfcVersion::Ifc2x3 => Some(ifc_schema::ifc2x3()),
        IfcVersion::Ifc4 => Some(ifc_schema::ifc4()),
        // No type system either; reported as a release gap.
        IfcVersion::Ifc4x3Add2 => None,
    }
}

fn schemas(releases: &[IfcVersion]) -> impl Iterator<Item = &'static Schema> + '_ {
    releases.iter().filter_map(|release| schema(*release))
}

fn names(releases: &[IfcVersion], name: &str) -> Vec<ExternalName> {
    releases
        .iter()
        .filter_map(|release| type_system(*release))
        .map(|type_system| ExternalName {
            type_system: type_system.to_owned(),
            name: name.to_owned(),
        })
        .collect()
}

/// The count the applicability's bounds call for, if any: at least
/// `minOccurs` and at most `maxOccurs` applicable objects in each model.
fn occurrence(specification: &Specification) -> Option<Check> {
    let applicability = &specification.applicability;
    let minimum = Some(applicability.min_occurs).filter(|minimum| *minimum > 0);
    let maximum = applicability.max_occurs;
    (minimum.is_some() || maximum.is_some()).then_some(Check::Count { minimum, maximum })
}

/// Refuses a class whose instances are not checked objects.
///
/// An IFC session makes a project object of every `IfcObject` occurrence and
/// of nothing else. A rule over a type object, an IFC4 `IfcProject` (an
/// `IfcContext`) or a resource would select nothing and pass silently, so
/// such a class, and one the release does not define, is a gap. A class a
/// pattern matched need only exist in one release.
fn occurrences(
    names: Vec<String>,
    releases: &[IfcVersion],
    matched: bool,
) -> Result<Vec<String>, Reason> {
    for release in releases {
        let Some(schema) = schema(*release) else {
            continue;
        };
        for name in &names {
            if schema.entity(name).is_none() {
                if matched {
                    continue;
                }
                return Err(Reason::UnknownEntity {
                    entity: name.clone(),
                    release: *release,
                });
            }
            if !schema.is_a(name, "IFCOBJECT") {
                return Err(Reason::NotAnObject(name.clone()));
            }
        }
    }
    Ok(names)
}

/// The classes a part-of whole names that can be the relating end of
/// `relationship` in some release; each must be an `IfcObject` occurrence.
///
/// A class that can never be the whole (a wall is never a spatial
/// container) matches nothing, as in IDS, so a pattern drops it. A class
/// that can be the whole but is no project object (an IFC4 `IfcProject`, a
/// type object) would silently fail every part, so it is a gap.
fn whole_names(
    entity: &Entity,
    relationship: &str,
    releases: &[IfcVersion],
) -> Result<Vec<String>, Reason> {
    let names = entity_names(entity, releases)?;
    let mut wholes = Vec::new();
    for name in names {
        let mut relating = false;
        for release in releases {
            let Some(schema) = schema(*release) else {
                continue;
            };
            if schema.entity(&name).is_none() {
                if pattern_named(entity) {
                    continue;
                }
                return Err(Reason::UnknownEntity {
                    entity: name,
                    release: *release,
                });
            }
            let end = schema
                .attributes(relationship)
                .into_iter()
                .find(|attribute| attribute.name.starts_with("Relating"))
                .map(|attribute| attribute.type_name.clone());
            // An end the schema does not spell as one entity type is
            // assumed to accept anything.
            let accepts =
                end.is_none_or(|end| schema.entity(&end).is_none() || schema.is_a(&name, &end));
            if accepts {
                relating = true;
                if !schema.is_a(&name, "IFCOBJECT") {
                    return Err(Reason::NotAnObject(name));
                }
            }
        }
        if relating || !pattern_named(entity) {
            wholes.push(name);
        }
    }
    Ok(wholes)
}

fn pattern_named(entity: &Entity) -> bool {
    matches!(&entity.name, Value::Restriction(restriction) if !restriction.patterns.is_empty())
}

/// The upper-case class names an entity facet matches exactly.
///
/// A pattern is matched against every class the releases define.
fn entity_names(entity: &Entity, releases: &[IfcVersion]) -> Result<Vec<String>, Reason> {
    let names = match &entity.name {
        Value::Simple(name) => vec![name.clone()],
        Value::Restriction(restriction) => {
            only_names(restriction)?;
            if restriction.patterns.is_empty() {
                restriction.enumeration.clone()
            } else {
                let patterns = compiled(&restriction.patterns)?;
                let mut found = BTreeSet::new();
                for schema in schemas(releases) {
                    for name in schema.entity_names() {
                        let upper = name.to_ascii_uppercase();
                        let listed = restriction.enumeration.is_empty()
                            || restriction.enumeration.contains(&upper);
                        if listed && patterns.iter().any(|pattern| pattern.is_match(&upper)) {
                            found.insert(upper);
                        }
                    }
                }
                found.into_iter().collect()
            }
        }
    };
    // The engine compares kinds case-insensitively; IDS does not.
    if let Some(name) = names
        .iter()
        .find(|name| name.chars().any(char::is_lowercase))
    {
        return Err(Reason::EntityCase(name.clone()));
    }
    Ok(names)
}

/// Refuses a name restriction with facets other than an enumeration and
/// patterns.
fn only_names(restriction: &Restriction) -> Result<(), Reason> {
    let other = restriction.min_inclusive.is_some()
        || restriction.max_inclusive.is_some()
        || restriction.min_exclusive.is_some()
        || restriction.max_exclusive.is_some()
        || restriction.length.is_some()
        || restriction.min_length.is_some()
        || restriction.max_length.is_some()
        || restriction.total_digits.is_some()
        || restriction.fraction_digits.is_some();
    if other {
        return Err(Reason::Restriction);
    }
    if restriction.enumeration.is_empty() && restriction.patterns.is_empty() {
        return Err(Reason::EmptyRestriction);
    }
    Ok(())
}

/// An XML Schema pattern in `regex` syntax.
fn translated(pattern: &str) -> Result<String, Reason> {
    axioval_rules::translate_xsd_pattern(pattern).map_err(|why| Reason::Pattern {
        pattern: pattern.to_owned(),
        why,
    })
}

/// Patterns compiled to whole-value matchers with XML Schema meaning.
fn compiled(patterns: &[String]) -> Result<Vec<Regex>, Reason> {
    patterns
        .iter()
        .map(|pattern| {
            let translated = translated(pattern)?;
            Regex::new(&format!(r"\A(?:{translated})\z")).map_err(|error| Reason::Pattern {
                pattern: pattern.clone(),
                why: error.to_string(),
            })
        })
        .collect()
}

/// The property sets or properties a facet names.
#[derive(Clone, Debug)]
enum Names {
    /// Exact names: a literal, an enumeration, or an enumeration narrowed by
    /// patterns.
    Literals(Vec<String>),
    /// XML Schema patterns, any of which a name may match.
    Patterns(Vec<String>),
}

impl Names {
    /// The names a set or property facet stands for.
    ///
    /// A restriction's enumeration and patterns must both hold, as its
    /// facets do in XML Schema, so patterns next to an enumeration only
    /// narrow it. Every pattern must translate exactly.
    fn of(value: &Value) -> Result<Self, Reason> {
        match value {
            Value::Simple(name) => Ok(Self::Literals(vec![name.clone()])),
            Value::Restriction(restriction) => {
                only_names(restriction)?;
                let compiled = compiled(&restriction.patterns)?;
                if restriction.patterns.is_empty() {
                    Ok(Self::Literals(restriction.enumeration.clone()))
                } else if restriction.enumeration.is_empty() {
                    Ok(Self::Patterns(restriction.patterns.clone()))
                } else {
                    let listed: Vec<String> = restriction
                        .enumeration
                        .iter()
                        .filter(|name| compiled.iter().any(|pattern| pattern.is_match(name)))
                        .cloned()
                        .collect();
                    if listed.is_empty() {
                        // No name meets both facets.
                        return Err(Reason::EmptyRestriction);
                    }
                    Ok(Self::Literals(listed))
                }
            }
        }
    }

    /// One XML Schema pattern matching exactly these names.
    fn pattern(&self) -> String {
        let alternatives: Vec<String> = match self {
            Self::Literals(names) => names.iter().map(|name| escape_xsd(name)).collect(),
            Self::Patterns(patterns) => patterns.clone(),
        };
        if alternatives.len() == 1 {
            alternatives[0].clone()
        } else {
            alternatives
                .iter()
                .map(|alternative| format!("({alternative})"))
                .collect::<Vec<_>>()
                .join("|")
        }
    }

    fn shown(&self) -> String {
        match self {
            Self::Literals(names) if names.len() == 1 => names[0].clone(),
            Self::Literals(names) => format!("({})", names.join("|")),
            Self::Patterns(patterns) => format!("/{}/", patterns.join("|")),
        }
    }
}

/// The options every `property-value` rule of a property facet takes: a
/// list, bounded, table or enumerated value holds when one of its values
/// does, and within a range only when all do; IDS states measures in SI.
fn value_options(value: Option<&Value>, parameters: &mut BTreeMap<String, ParameterValue>) {
    let bounded = matches!(
        value,
        Some(Value::Restriction(restriction)) if restriction.min_inclusive.is_some()
            || restriction.max_inclusive.is_some()
            || restriction.min_exclusive.is_some()
            || restriction.max_exclusive.is_some()
    );
    parameters.insert(
        "quantifier".to_owned(),
        string(if bounded { "all" } else { "any" }),
    );
    parameters.insert(
        "si_units".to_owned(),
        ParameterValue::Boolean { value: true },
    );
}

/// `name` as an XML Schema pattern matching it alone.
fn escape_xsd(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if "\\|.?*+(){}[]^-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A `property-requirements` row naming its set and property by pattern,
/// with further cells.
fn pattern_row(sets: &Names, names: &Names, cells: &[(&str, &str)]) -> TableRow {
    let mut row = TableRow::from([
        ("property_set_pattern".to_owned(), string(&sets.pattern())),
        ("property_pattern".to_owned(), string(&names.pattern())),
    ]);
    for (column, cell) in cells {
        row.insert((*column).to_owned(), string(cell));
    }
    row
}

/// The declared type of an attribute, as far as a selector compares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttributeKind {
    /// A string type: compared as text, and empty text is no value.
    Text,
    /// An enumeration: its item, as text.
    Enumeration,
    /// `BOOLEAN` or `LOGICAL`.
    Boolean,
    /// `INTEGER`.
    Integer,
}

/// Refuses an attribute name that matches a schema attribute only ignoring
/// case: the source would read it, IDS would not.
fn attribute_case(name: &str, scope: &Scope<'_>) -> Result<(), Reason> {
    for schema in schemas(scope.releases) {
        for class in &scope.classes {
            for attribute in schema.attributes(class) {
                if attribute.name.eq_ignore_ascii_case(name) && attribute.name != name {
                    return Err(Reason::AttributeCase(name.to_owned()));
                }
            }
        }
    }
    Ok(())
}

/// The attribute names of the applicable classes a restriction matches.
fn declared_attributes(
    restriction: &Restriction,
    scope: &Scope<'_>,
) -> Result<Vec<String>, Reason> {
    only_names(restriction)?;
    let patterns = compiled(&restriction.patterns)?;
    let mut found = BTreeSet::new();
    for schema in schemas(scope.releases) {
        for class in &scope.classes {
            for attribute in schema.attributes(class) {
                let name = &attribute.name;
                let listed =
                    restriction.enumeration.is_empty() || restriction.enumeration.contains(name);
                let matched =
                    patterns.is_empty() || patterns.iter().any(|pattern| pattern.is_match(name));
                if listed && matched {
                    found.insert(name.clone());
                }
            }
        }
    }
    Ok(found.into_iter().collect())
}

/// The kind of `name` as every applicable class declares it; `None` when no
/// class declares it, so it is never present.
fn attribute_kind(name: &str, scope: &Scope<'_>) -> Result<Option<AttributeKind>, Reason> {
    let refuse = |declared: String| Reason::AttributeType {
        attribute: name.to_owned(),
        declared,
    };
    let mut kind = None;
    for schema in schemas(scope.releases) {
        for class in &scope.classes {
            let attributes = schema.attributes(class);
            let Some(attribute) = attributes.iter().find(|attribute| attribute.name == name) else {
                continue;
            };
            if attribute.aggregate {
                return Err(refuse("an aggregate".into()));
            }
            let found = declared_kind(schema, &attribute.type_name)
                .ok_or_else(|| refuse(attribute.type_name.clone()))?;
            match kind {
                None => kind = Some(found),
                Some(known) if known == found => {}
                Some(_) => {
                    return Err(refuse("differently across the applicable classes".into()));
                }
            }
        }
    }
    Ok(kind)
}

/// Types the source reads as dates, times or durations rather than text.
const TEMPORAL_TYPES: &[&str] = &[
    "IFCDATE",
    "IFCDATETIME",
    "IFCTIME",
    "IFCTIMESTAMP",
    "IFCDURATION",
    "IFCCALENDARDATE",
    "IFCLOCALTIME",
    "IFCDATEANDTIME",
];

/// The kind a declared attribute type is read as, if a selector compares it
/// as IDS does.
fn declared_kind(schema: &Schema, type_name: &str) -> Option<AttributeKind> {
    if schema.entity(type_name).is_some() {
        return None;
    }
    // Walk the aliases: a measure or a date anywhere in the chain is read
    // as such, however the chain ends.
    let mut current = type_name.to_owned();
    for _ in 0..32 {
        let upper = current.to_ascii_uppercase();
        if upper.ends_with("MEASURE") || TEMPORAL_TYPES.contains(&upper.as_str()) {
            return None;
        }
        match schema.type_def(&current).map(|definition| &definition.kind) {
            Some(TypeKind::Enumeration(_)) => return Some(AttributeKind::Enumeration),
            Some(TypeKind::Select(_)) => return None,
            Some(TypeKind::Defined(alias)) => current.clone_from(alias),
            None => break,
        }
    }
    let base = schema.resolve_defined(type_name).to_ascii_uppercase();
    if base.starts_with("STRING") {
        Some(AttributeKind::Text)
    } else if base == "BOOLEAN" || base == "LOGICAL" {
        Some(AttributeKind::Boolean)
    } else if base == "INTEGER" {
        Some(AttributeKind::Integer)
    } else {
        None
    }
}

/// A case-sensitive property selector on a slot.
fn test(slot: &Slot, operator: ComparisonOperator, value: Option<ParameterValue>) -> Selector {
    Selector::property(Some(slot.0.clone()), slot.1.as_str(), operator, value)
}

/// The slot holds exactly `literal`.
fn is(slot: &Slot, literal: &str) -> Selector {
    test(slot, ComparisonOperator::Equals, Some(string(literal)))
}

/// The slot holds a value; null reads as absent.
fn exists(slot: &Slot) -> Selector {
    test(slot, ComparisonOperator::Exists, None)
}

/// The slot holds no value.
fn unset(slot: &Slot) -> Selector {
    not(exists(slot))
}

/// The slot holds non-empty text, as IDS reads an empty string as no value.
fn filled(slot: &Slot) -> Selector {
    test(slot, ComparisonOperator::Matches, Some(string("(?s).+")))
}

/// A selector testing a text value in `set.name` against an IDS value: a
/// literal, or a restriction whose facets all hold.
fn text_test(set: &str, name: &str, value: &Value) -> Result<Selector, Reason> {
    let slot = (set.to_owned(), name.to_owned());
    let restriction = match value {
        Value::Simple(literal) => return Ok(is(&slot, literal)),
        Value::Restriction(restriction) => restriction,
    };
    for (facet, present) in [
        ("minInclusive", restriction.min_inclusive.is_some()),
        ("maxInclusive", restriction.max_inclusive.is_some()),
        ("minExclusive", restriction.min_exclusive.is_some()),
        ("maxExclusive", restriction.max_exclusive.is_some()),
        ("totalDigits", restriction.total_digits.is_some()),
        ("fractionDigits", restriction.fraction_digits.is_some()),
    ] {
        if present {
            return Err(Reason::RestrictionFacet(facet));
        }
    }
    let mut operands = Vec::new();
    if !restriction.enumeration.is_empty() {
        operands.push(test(
            &slot,
            ComparisonOperator::OneOf,
            Some(ParameterValue::StringList {
                value: restriction.enumeration.clone(),
            }),
        ));
    }
    if !restriction.patterns.is_empty() {
        let patterns = restriction
            .patterns
            .iter()
            .map(|pattern| {
                Ok(test(
                    &slot,
                    ComparisonOperator::Matches,
                    Some(string(&translated(pattern)?)),
                ))
            })
            .collect::<Result<_, Reason>>()?;
        operands.push(any_of(patterns));
    }
    let repetition = |bound: Option<u64>, facet| match bound {
        Some(count) if count > MAX_REPETITION => Err(Reason::RestrictionFacet(facet)),
        other => Ok(other),
    };
    let length = repetition(restriction.length, "length")?;
    let min_length = repetition(restriction.min_length, "minLength")?;
    let max_length = repetition(restriction.max_length, "maxLength")?;
    let matches =
        |pattern: String| test(&slot, ComparisonOperator::Matches, Some(string(&pattern)));
    if let Some(length) = length {
        operands.push(matches(format!("(?s).{{{length}}}")));
    }
    if min_length.is_some() || max_length.is_some() {
        let low = min_length.unwrap_or(0);
        let high = max_length.map_or_else(String::new, |high| high.to_string());
        operands.push(matches(format!("(?s).{{{low},{high}}}")));
    }
    if operands.is_empty() {
        return Err(Reason::EmptyRestriction);
    }
    Ok(all_of(operands))
}

/// Whether a restriction holds only an enumeration.
fn only_enumeration(restriction: &Restriction) -> bool {
    !restriction.enumeration.is_empty()
        && restriction.patterns.is_empty()
        && only_names(restriction).is_ok()
}

/// A boolean attribute against `true` or `false`, as IDS spells them.
fn boolean_test(slot: &Slot, value: &Value) -> Result<Selector, Reason> {
    let literal = |text: &str| match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(Reason::ValueLiteral(other.to_owned())),
    };
    let literals = match value {
        Value::Simple(text) => vec![literal(text)?],
        Value::Restriction(restriction) if only_enumeration(restriction) => restriction
            .enumeration
            .iter()
            .map(|text| literal(text))
            .collect::<Result<_, _>>()?,
        Value::Restriction(_) => return Err(Reason::Restriction),
    };
    Ok(any_of(
        literals
            .into_iter()
            .map(|value| {
                test(
                    slot,
                    ComparisonOperator::Equals,
                    Some(ParameterValue::Boolean { value }),
                )
            })
            .collect(),
    ))
}

/// An integer attribute against integer literals and bounds.
fn integer_test(slot: &Slot, value: &Value) -> Result<Selector, Reason> {
    let integer = |text: &str| {
        parse_integer(text)
            .map(|value| ParameterValue::Integer { value })
            .ok_or_else(|| Reason::ValueLiteral(text.to_owned()))
    };
    let restriction = match value {
        Value::Simple(text) => {
            return Ok(test(slot, ComparisonOperator::Equals, Some(integer(text)?)));
        }
        Value::Restriction(restriction) => restriction,
    };
    for (facet, present) in [
        ("pattern", !restriction.patterns.is_empty()),
        ("length", restriction.length.is_some()),
        ("minLength", restriction.min_length.is_some()),
        ("maxLength", restriction.max_length.is_some()),
        ("totalDigits", restriction.total_digits.is_some()),
        ("fractionDigits", restriction.fraction_digits.is_some()),
    ] {
        if present {
            return Err(Reason::RestrictionFacet(facet));
        }
    }
    let mut operands = Vec::new();
    if !restriction.enumeration.is_empty() {
        let literals = restriction
            .enumeration
            .iter()
            .map(|text| Ok(test(slot, ComparisonOperator::Equals, Some(integer(text)?))))
            .collect::<Result<_, Reason>>()?;
        operands.push(any_of(literals));
    }
    for (bound, operator) in [
        (
            &restriction.min_inclusive,
            ComparisonOperator::GreaterThanOrEquals,
        ),
        (
            &restriction.max_inclusive,
            ComparisonOperator::LessThanOrEquals,
        ),
        (&restriction.min_exclusive, ComparisonOperator::GreaterThan),
        (&restriction.max_exclusive, ComparisonOperator::LessThan),
    ] {
        if let Some(bound) = bound {
            operands.push(test(slot, operator, Some(integer(bound)?)));
        }
    }
    if operands.is_empty() {
        return Err(Reason::EmptyRestriction);
    }
    Ok(all_of(operands))
}

/// The `xs:integer` lexical form: an optional sign and digits.
fn parse_integer(literal: &str) -> Option<i64> {
    let digits = literal.strip_prefix(['+', '-']).unwrap_or(literal);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    literal.strip_prefix('+').unwrap_or(literal).parse().ok()
}

/// A classification facet with a literal or enumerated system and value.
///
/// A reference matches its own code and every ancestor's, so a value names a
/// class and all its subclasses, as IDS reads full classifications. The
/// system and the code are matched on the same reference.
fn classification_selector(classification: &Classification) -> Result<Selector, Reason> {
    let literals = |value: &Value| match value {
        Value::Simple(literal) => Ok(vec![literal.clone()]),
        Value::Restriction(restriction) if only_enumeration(restriction) => {
            Ok(restriction.enumeration.clone())
        }
        Value::Restriction(_) => Err(Reason::ClassificationPattern),
    };
    let systems = literals(&classification.system)?;
    let Some(value) = &classification.value else {
        return Err(Reason::ClassificationSystem);
    };
    let codes = literals(value)?;
    let mut operands = Vec::new();
    for system in &systems {
        for code in &codes {
            operands.push(Selector::Classification {
                system: system.clone(),
                code: code.clone(),
                include_descendants: true,
            });
        }
    }
    Ok(any_of(operands))
}

/// A property selector over a list value, holding when any element does.
fn any_element(selector: Selector) -> Selector {
    match selector {
        Selector::Property {
            property_set,
            property,
            operator,
            value,
            case_sensitive,
            trim,
            precision,
            quantifier: _,
        } => Selector::Property {
            property_set,
            property,
            operator,
            value,
            case_sensitive,
            trim,
            quantifier: Some(Quantifier::Any),
            precision,
        },
        other => other,
    }
}

/// A classification requirement as the `classification` capability states
/// it: literal or pattern systems and codes, optional or prohibited.
///
/// A restriction's enumeration and patterns must both hold, as its facets
/// do in XML Schema; any other facet is a gap.
fn classification_check(
    classification: &Classification,
    occurrence: Occurrence,
) -> Result<Check, Reason> {
    let mut parameters = BTreeMap::new();
    let mut add = |literals: &str, patterns: &str, value: &Value| -> Result<(), Reason> {
        let list = |value: &[String]| ParameterValue::StringList {
            value: value.to_vec(),
        };
        match value {
            Value::Simple(literal) => {
                parameters.insert(literals.to_owned(), list(std::slice::from_ref(literal)));
            }
            Value::Restriction(restriction) => {
                only_names(restriction)?;
                if !restriction.enumeration.is_empty() {
                    parameters.insert(literals.to_owned(), list(&restriction.enumeration));
                }
                if !restriction.patterns.is_empty() {
                    for pattern in &restriction.patterns {
                        translated(pattern)?;
                    }
                    parameters.insert(patterns.to_owned(), list(&restriction.patterns));
                }
            }
        }
        Ok(())
    };
    add("systems", "system_patterns", &classification.system)?;
    if let Some(value) = &classification.value {
        add("codes", "code_patterns", value)?;
    }
    let flag = match occurrence {
        Occurrence::Required => None,
        Occurrence::Optional => Some("optional"),
        Occurrence::Prohibited => Some("prohibited"),
    };
    if let Some(flag) = flag {
        parameters.insert(flag.to_owned(), ParameterValue::Boolean { value: true });
    }
    let shown = shown_classification(classification);
    let title = match occurrence {
        Occurrence::Required => format!("required: is classified {shown}"),
        Occurrence::Optional => format!("where classified at all: is classified {shown}"),
        Occurrence::Prohibited => format!("prohibited: is classified {shown}"),
    };
    Ok(Check::Capability {
        kind: Kind::Classification,
        title,
        parameters,
    })
}

/// The `property-value` parameters an IDS value stands for.
fn value_parameters(
    value: &Value,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<(), Reason> {
    let restriction = match value {
        Value::Simple(literal) => {
            parameters.insert(
                "values".to_owned(),
                ParameterValue::StringList {
                    value: vec![literal.clone()],
                },
            );
            return Ok(());
        }
        Value::Restriction(restriction) => restriction,
    };
    let before = parameters.len();
    let mut add = |name: &str, value: ParameterValue| {
        parameters.insert(name.to_owned(), value);
    };
    for (name, list) in [
        ("values", &restriction.enumeration),
        ("patterns", &restriction.patterns),
    ] {
        if !list.is_empty() {
            add(
                name,
                ParameterValue::StringList {
                    value: list.clone(),
                },
            );
        }
    }
    for (name, bound) in [
        ("min_inclusive", &restriction.min_inclusive),
        ("max_inclusive", &restriction.max_inclusive),
        ("min_exclusive", &restriction.min_exclusive),
        ("max_exclusive", &restriction.max_exclusive),
    ] {
        if let Some(bound) = bound {
            add(name, string(bound));
        }
    }
    for (name, count) in [
        ("length", restriction.length),
        ("min_length", restriction.min_length),
        ("max_length", restriction.max_length),
        ("total_digits", restriction.total_digits),
        ("fraction_digits", restriction.fraction_digits),
    ] {
        if let Some(count) = count {
            // A count beyond `i64` cannot be met by any value; `MAX` keeps
            // that meaning for the upper limits and fails every `length`.
            let value = i64::try_from(count).unwrap_or(i64::MAX);
            add(name, ParameterValue::Integer { value });
        }
    }
    if parameters.len() == before {
        return Err(Reason::EmptyRestriction);
    }
    Ok(())
}

fn string(value: &str) -> ParameterValue {
    ParameterValue::String {
        value: value.to_owned(),
    }
}

fn not(selector: Selector) -> Selector {
    Selector::Not {
        operand: Box::new(selector),
    }
}

/// All operands; one operand stands alone.
fn all_of(mut operands: Vec<Selector>) -> Selector {
    if operands.len() == 1 {
        operands.remove(0)
    } else {
        Selector::AllOf { operands }
    }
}

/// Any operand; one stands alone, and none never matches.
fn any_of(mut operands: Vec<Selector>) -> Selector {
    match operands.len() {
        0 => not(Selector::All),
        1 => operands.remove(0),
        _ => Selector::AnyOf { operands },
    }
}

fn shown_value(value: &Value) -> String {
    match value {
        Value::Simple(literal) => literal.clone(),
        Value::Restriction(restriction) if !restriction.enumeration.is_empty() => {
            restriction.enumeration.join("|")
        }
        Value::Restriction(restriction) if !restriction.patterns.is_empty() => {
            format!("/{}/", restriction.patterns.join("|"))
        }
        Value::Restriction(_) => "a restricted value".to_owned(),
    }
}

fn shown_entity(entity: &Entity) -> String {
    let name = shown_value(&entity.name);
    match &entity.predefined_type {
        Some(predefined) => format!("{name}.{}", shown_value(predefined)),
        None => name,
    }
}

fn shown_classification(classification: &Classification) -> String {
    match &classification.value {
        Some(value) => format!(
            "{} in {}",
            shown_value(value),
            shown_value(&classification.system)
        ),
        None => format!("in {}", shown_value(&classification.system)),
    }
}

/// `scheme:rest` with a lower-case scheme, as MCS qualified ids are.
fn is_qualified_id(id: &str) -> bool {
    let Some((scheme, rest)) = id.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c))
        && !rest.is_empty()
        && !id.chars().any(char::is_whitespace)
}

/// `major.minor.patch` without leading zeros; pre-release and build suffixes
/// are accepted as MCS accepts them.
fn is_semver(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}
