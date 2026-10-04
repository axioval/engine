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
//! # Audit first
//!
//! [`translate`] first [audits](audit()) the document against the IFC
//! schemas of its listed releases. A document with any audit error cannot
//! work as written (an entity a release does not define, an attribute the
//! entity does not have, a value no IFC value of its type could equal) and
//! is refused whole, with every finding ([`Invalid`]); warnings refuse
//! nothing and come with the translation ([`Translation::warnings`]). The
//! export never writes a document the audit refuses either.
//!
//! # What becomes what
//!
//! - The applicability is one selector: the entity's classes (never their
//!   subclasses), its predefined type, and a selector for every other facet
//!   a selector states exactly. A facet none does (a property, an attribute
//!   a selector cannot compare as IDS does, a classification system given
//!   as a pattern, a material value restricted by several facets) is
//!   checked as a requirement by an auxiliary rule over the rest of the
//!   applicability, and the applicable objects are those it passed
//!   (`ruleOutcome`). The auxiliary rule reports nothing itself; what it
//!   leaves undecided is not evaluated by every rule of the specification.
//! - A property requirement becomes `property-required`,
//!   `property-data-type` or `property-value`; an attribute requirement the
//!   same in the reserved attribute set. A prohibited property or attribute
//!   without a value becomes an excluded `not-empty` row of
//!   `property-requirements`. A prohibited facet no capability negates (a
//!   property with a value or a data type, an attribute value a selector
//!   cannot compare, a material value restricted by several facets) becomes
//!   an auxiliary rule checking it as required and a `selector-conformance`
//!   rule failing every object that rule passed. A property value is judged under `quantifier`
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
//! - In IFC2X3 a class the IDS type mapping table renames (`IFCAIRTERMINAL`)
//!   is its occurrence class typed, through `IfcRelDefinesByType`, by a type
//!   object of its type class (`IFCFLOWTERMINAL` by `IFCAIRTERMINALTYPE`);
//!   in an IFC2X3 source only, since other releases define the class itself.
//! - Entity, material, part-of and name-restricted attribute requirements
//!   become `selector-conformance` with the facet's selector, negated when
//!   prohibited. A material value is tested against the material set's
//!   `Names` list, every name and category the material goes by; one
//!   restricted by several facets that one name must meet together becomes
//!   `property-value` over that list with `quantifier` `any`.
//! - A classification requirement becomes `classification`: systems and
//!   codes as literals or patterns, a system alone, optional or prohibited.
//!   In the applicability a literal or enumerated system becomes
//!   `classification` selectors, with a code, a `codePattern`, or neither
//!   for the system alone.
//! - The applicability's `minOccurs`/`maxOccurs` become `object-count` in
//!   each source: a required specification reports a model with no
//!   applicable object, a prohibited one a model with any.
//!
//! The predefined type is resolved as IDS resolves it: the type object's
//! own, or its element or process type when that is user-defined or unset,
//! unless the result is empty or `NOTDEFINED`; otherwise the occurrence's
//! own, or its object type when user-defined or unset. A literal
//! `USERDEFINED` asks whether the type is user-defined at all. A type
//! object, typed by nothing, is checked as itself: its own predefined type,
//! or its element, process or resource type when user-defined or unset.
//!
//! # Releases
//!
//! `@ifcVersion` is metadata and never changes a verdict, as the
//! buildingSMART test cases require: every concept is named in the type
//! systems of all three releases IDS names (IFC2X3, IFC4, IFC4X3_ADD2),
//! so an IFC2X3 specification checks an IFC4 model. A class some release
//! lacks matches nothing in its models, as in IDS; one a listed release does not
//! define is refused by the audit.
//!
//! # Export
//!
//! [`export()`] writes a ruleset back as IDS, as exactly as it reads it: a
//! rule is exported only when translating the specification it reads as
//! gives the rule again, and every other rule is listed with a
//! [`Refusal`]. Every folder [`translate`] writes keeps its specification
//! in the [`SPECIFICATION_ANNOTATION`], and the root folder the document's
//! `<info>` under [`INFO_ANNOTATION`], so a translated document is exported
//! again specification by specification. [`IdsProfile`] is the same export
//! as the `ids` export profile of `axioval-export`, whose comparator judges
//! what "gives the rule again" means.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axioval_engine::{ParameterType, RuleCapability};
use axioval_ir::contract::{
    ComparisonOperator, DefinitionPackage, ExternalName, LocalizedText, ObjectTypeDefinition,
    PackageMetadata, ParameterDefinition, ParameterKind, ParameterValue, PropertyDefinition,
    PropertySetDefinition, PropertyValueKind, Quantifier, RelatedQuantifier, RuleApplicability,
    RuleDefinition, RuleFolder, RuleInstance, RuleOutcomeKind, RuleSetPackage, Selector, Severity,
    SourceField, TableColumnDefinition, TableRow,
};
use axioval_ir::{ATTRIBUTE_SET, MATERIAL_KIND, MATERIAL_NAMES, MATERIAL_SET, TYPE_ATTRIBUTE_SET};
use ifc_schema::{Schema, TypeKind};
use openbim_ids::{
    Attribute, Classification, Entity, Facet, Ids, IfcVersion, Info, Material, Occurrence, PartOf,
    Property, Relation, Requirement, Restriction, Specification, Value,
};

mod export;
mod write;

pub use export::{
    DocumentError, Export, ExportedSpecification, IdsProfile, NotExported, Refusal, export,
};
pub use openbim_ids::{AuditCode, AuditFinding, Severity as AuditSeverity};
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

/// Type system of IFC4X3 ADD2, as the IFC adapter declares it.
pub const IFC4X3_TYPE_SYSTEM: &str =
    "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3";

/// The package schema version the engine compiles.
const SCHEMA_VERSION: &str = "0.1.0";

/// The longest `length`, `minLength` or `maxLength` a selector spells as a
/// repetition; a longer one would exceed the regular expression size limit.
const MAX_REPETITION: u64 = 1000;

/// Identity of the packages written, and what they check.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Qualified id of the ruleset package, such as `ids:fire-safety`. The
    /// definition package and every concept are named under it.
    pub package_id: String,
    /// Semantic version of both packages.
    pub version: String,
    /// A prefilter restricting every specification to part of the model
    /// (one storey, one discipline), combined with each specification's
    /// applicability through `allOf`. `None` checks what the document
    /// states.
    ///
    /// It is written in IFC names, as IDS writes its facets: an
    /// `entityType` names a class (`IfcBuildingStorey`), a `property` a
    /// property set and property (`Pset_WallCommon`, `FireRating`), or a
    /// reserved set (`axioval:attributes`) and a property in it (`Name`).
    /// Each is bound to a concept in each specification's releases, as the
    /// specification's own names are. Relationship paths, classification
    /// systems, name patterns, disciplines and source fields are used as
    /// written. A `ruleOutcome` selector is refused: the rules it could
    /// name are the translation's own.
    pub filter: Option<Selector>,
}

impl Options {
    /// Options identifying the packages, with no prefilter.
    #[must_use]
    pub fn new(package_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            package_id: package_id.into(),
            version: version.into(),
            filter: None,
        }
    }

    /// The same options, restricting every specification by `filter`; see
    /// [`Options::filter`].
    #[must_use]
    pub fn with_filter(mut self, filter: Selector) -> Self {
        self.filter = Some(filter);
        self
    }
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
    /// The prefilter selects by the outcome of the named rule, which only
    /// the translation itself defines.
    #[error(
        "the prefilter selects by the outcome of rule {0:?}; a prefilter selects objects by what the model states, never by a rule"
    )]
    FilterRuleOutcome(String),
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
    /// The [`audit()`]'s warnings: checks it could not decide, such as a
    /// pattern using an XML Schema construct it does not evaluate. They
    /// refuse nothing.
    pub warnings: Vec<AuditFinding>,
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
    /// resources (a material, a classification) a model session does not
    /// check.
    WithoutEntity,
    /// A class no IFC release IDS names defines.
    UnknownEntity(String),
    /// A part-of whole whose instances are neither `IfcObject` occurrences,
    /// `IfcContext`s nor `IfcTypeObject`s (a resource object), which no
    /// relationship traversal reaches.
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
                "an applicability without an entity facet also covers resources, which a model session does not check",
            ),
            Reason::UnknownEntity(entity) => {
                write!(f, "no IFC release IDS names defines an entity {entity}")
            }
            Reason::NotAnObject(entity) => write!(
                f,
                "{entity} is neither an IfcObject occurrence, an IfcContext nor an IfcTypeObject, so no part-of relation reaches it"
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
            Reason::ProhibitedRequirements => f.write_str(
                "a prohibited specification takes no requirements; IDS declares them invalid",
            ),
        }
    }
}

/// Why [`translate`] refused a document.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TranslateError {
    /// The options are malformed.
    #[error(transparent)]
    Options(#[from] OptionsError),
    /// The document cannot work as written for its releases.
    #[error(transparent)]
    Invalid(#[from] Invalid),
}

/// An IDS document the [`audit()`] refuses: a specification that cannot
/// work as written for a release it lists (an entity the release does not
/// define, an attribute the entity does not have, a value no IFC value of
/// its type could equal, requirements on a prohibited specification).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    /// Every finding of the audit, in document order: at least one error,
    /// and any warnings beside.
    pub findings: Vec<AuditFinding>,
}

impl Invalid {
    /// The findings that refuse the document.
    pub fn errors(&self) -> impl Iterator<Item = &AuditFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.severity() == AuditSeverity::Error)
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let errors: Vec<String> = self.errors().map(ToString::to_string).collect();
        write!(
            f,
            "the IDS document cannot work as written: {} audit error(s): {}",
            errors.len(),
            errors.join("; ")
        )
    }
}

impl std::error::Error for Invalid {}

/// Audits `ids` against the IFC schemas of the releases each specification
/// lists, with [`openbim_ids::audit()`]: every check except those against
/// the standard property and quantity set templates.
///
/// # Errors
///
/// [`Invalid`], with every finding, when any finding is an
/// [`AuditSeverity::Error`]. Otherwise the result is the warnings: checks
/// the audit could not decide, which refuse nothing.
pub fn audit(ids: &Ids) -> Result<Vec<AuditFinding>, Invalid> {
    let findings = openbim_ids::audit(ids);
    if findings
        .iter()
        .any(|finding| finding.severity() == AuditSeverity::Error)
    {
        Err(Invalid { findings })
    } else {
        Ok(findings)
    }
}

/// Translates `ids` into packages identified by `options`, once the
/// [`audit()`] accepted it.
///
/// # Errors
///
/// [`TranslateError::Options`] when the package id or version is
/// malformed, [`TranslateError::Invalid`] when the audit finds an error.
/// A document that cannot be translated fully is not an error; see
/// [`Translation::specifications`].
pub fn translate(ids: &Ids, options: &Options) -> Result<Translation, TranslateError> {
    let warnings = audit(ids)?;
    let specifications: Vec<&Specification> = ids.specifications.iter().collect();
    let mut translation = translate_parts(&ids.info, &specifications, options)?;
    translation.warnings = warnings;
    Ok(translation)
}

/// The annotation key of the folder a specification's rules are written
/// to, holding the specification as IDS XML, so [`export()`] can write it
/// back: one `<specification>` element in the IDS namespace, without
/// declaring it. Annotations written by earlier releases still read.
pub const SPECIFICATION_ANNOTATION: &str = "ids:specification";

/// The annotation key a folder keeps instead of the
/// [`SPECIFICATION_ANNOTATION`] when the IDS 1.0 writer refuses its
/// specification (a restriction without facets, say): where in the
/// specification, a colon and a space, and why; the location is empty for
/// the specification itself. [`export()`] refuses the folder's rules with it.
pub const UNWRITABLE_ANNOTATION: &str = "ids:unwritable";

/// The prefix of the root folder's annotation keys holding the document's
/// `<info>`, one per field present: `ids:info.title`, `ids:info.author`,
/// and so on.
pub const INFO_ANNOTATION: &str = "ids:info.";

/// The `<info>` fields by the name they are annotated under.
fn info_fields(info: &Info) -> [(&'static str, Option<&String>); 8] {
    [
        ("title", Some(&info.title)),
        ("copyright", info.copyright.as_ref()),
        ("version", info.version.as_ref()),
        ("description", info.description.as_ref()),
        ("author", info.author.as_ref()),
        ("date", info.date.as_ref()),
        ("purpose", info.purpose.as_ref()),
        ("milestone", info.milestone.as_ref()),
    ]
}

/// [`translate`] over a document's parts, so [`export()`] can translate one
/// specification alone.
fn translate_parts(
    info: &Info,
    specifications: &[&Specification],
    options: &Options,
) -> Result<Translation, OptionsError> {
    if !is_qualified_id(&options.package_id) {
        return Err(OptionsError::PackageId(options.package_id.clone()));
    }
    if !is_semver(&options.version) {
        return Err(OptionsError::Version(options.version.clone()));
    }
    if let Some(rule) = options.filter.as_ref().and_then(rule_outcome) {
        return Err(OptionsError::FilterRuleOutcome(rule.to_owned()));
    }
    let mut writer = Writer::new(options);
    let mut folders = Vec::new();
    let mut outcomes = Vec::new();
    for (index, specification) in specifications.iter().enumerate() {
        let number = index + 1;
        let (rules, gaps) = writer.specification(number, specification);
        let outcome = SpecificationOutcome {
            number,
            name: specification.name.clone(),
            rules: rules.iter().map(|rule| rule.id.clone()).collect(),
            gaps,
        };
        // A complete specification whose facets always hold writes no rule,
        // but still a folder, so it is exported again.
        if !rules.is_empty() || outcome.is_complete() {
            folders.push(RuleFolder {
                id: format!("spec{number}"),
                name: LocalizedText::plain(&specification.name),
                description: specification
                    .description
                    .as_deref()
                    .map(LocalizedText::plain),
                rules,
                folders: Vec::new(),
                gate: None,
                // A specification IDS 1.0 cannot write back keeps why
                // instead, so its rules are refused, never exported one by
                // one.
                annotations: BTreeMap::from([match write::specification(specification) {
                    Ok(fragment) => (SPECIFICATION_ANNOTATION.to_owned(), fragment),
                    Err(unwritable) => (
                        UNWRITABLE_ANNOTATION.to_owned(),
                        format!("{}: {}", unwritable.location, unwritable.why),
                    ),
                }]),
            });
        }
        outcomes.push(outcome);
    }
    let title = &info.title;
    let definitions_id = format!("{}.definitions", options.package_id);
    let metadata = |id: &str, name: String| PackageMetadata {
        id: id.to_owned(),
        name: LocalizedText::plain(name),
        version: options.version.clone(),
        description: info.description.as_deref().map(LocalizedText::plain),
        repository: None,
        license: None,
        authors: info.author.iter().cloned().collect(),
    };
    let annotations = info_fields(info)
        .into_iter()
        .filter_map(|(field, value)| Some((format!("{INFO_ANNOTATION}{field}"), value?.clone())))
        .collect();
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
                gate: None,
                annotations,
            },
            classifications: BTreeMap::new(),
            groupings: BTreeMap::new(),
            relations: BTreeMap::new(),
        },
        specifications: outcomes,
        warnings: Vec::new(),
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
                .map(|applicable| applicable.classes.clone())
                .unwrap_or_default(),
            entity: applicability.as_ref().map(|applicable| applicable.entity),
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
                match self.attempt(|writer| {
                    writer.requirement(&requirement.facet, requirement.occurrence, &scope)
                }) {
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
        let Some(mut applicable) = applicability.filter(|_| !skipped) else {
            // Nothing is written, so nothing it would have named is either.
            self.concepts = saved;
            return (Vec::new(), gaps);
        };
        // The prefilter narrows the applicability itself, so every rule of
        // the specification, auxiliary and count rules included, sees only
        // the objects it selects.
        let options = self.options;
        if let Some(filter) = &options.filter {
            let filter = self.bind_filter(filter, &releases);
            applicable.selector = all_of(vec![applicable.selector, filter]);
        }
        let mut rules = Vec::new();
        // A facet no selector states is checked by an auxiliary rule over
        // the rest of the applicability; the objects it passes are
        // applicable.
        let mut operands = vec![applicable.selector.clone()];
        for (facet, check) in applicable.checked {
            let id = format!("spec{number}.applicability{facet}");
            operands.push(passed(&id));
            rules.push(self.rule(id, specification, None, check, &applicable.selector, true));
        }
        let selector = all_of(operands);
        if let Some(check) = occurrence(specification) {
            let id = format!("spec{number}.occurrence");
            rules.push(self.rule(id, specification, None, check, &selector, false));
        }
        for (facet, requirement, check) in checks {
            let id = format!("spec{number}.facet{facet}");
            let Check::Negated { required, holds } = check else {
                rules.push(self.rule(
                    id,
                    specification,
                    Some(requirement),
                    check,
                    &selector,
                    false,
                ));
                continue;
            };
            // The facet as required, run for this rule alone: every object
            // it passes breaks the prohibition.
            let required_id = format!("{id}.required");
            let breaks = all_of(vec![selector.clone(), passed(&required_id)]);
            rules.push(self.rule(
                required_id,
                specification,
                Some(requirement),
                *required,
                &selector,
                true,
            ));
            rules.push(self.rule(
                id,
                specification,
                Some(requirement),
                breaking(&holds),
                &breaks,
                false,
            ));
        }
        (rules, gaps)
    }

    /// The applicability's selector, classes, entity facet, and the facets
    /// only a rule can check.
    ///
    /// `None` when any facet is untranslatable; the gaps say which.
    fn applicability<'s>(
        &mut self,
        specification: &'s Specification,
        releases: &[IfcVersion],
        gaps: &mut Vec<Gap>,
    ) -> Option<Applicable<'s>> {
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
        let mut checked = Vec::new();
        for (index, facet) in facets.iter().enumerate() {
            if matches!(facet, Facet::Entity(_)) {
                continue;
            }
            // Being applicable means meeting the facet as a requirement would.
            let translated = self.attempt(|writer| match writer.facet_selector(facet, &scope)? {
                Some(selector) => Ok(Ok(selector)),
                None => writer
                    .requirement(facet, Occurrence::Required, &scope)
                    .map(Err),
            });
            match translated {
                Ok(Ok(selector)) => operands.push(selector),
                Ok(Err(Some(check))) => checked.push((index + 1, check)),
                // Met by every object.
                Ok(Err(None)) => {}
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
        Some(Applicable {
            selector: all_of(operands),
            classes,
            entity: facet,
            checked,
        })
    }

    /// The prefilter `filter`, written in IFC names, with every class, set
    /// and property it names bound to a concept in `releases`.
    fn bind_filter(&mut self, filter: &Selector, releases: &[IfcVersion]) -> Selector {
        match filter {
            Selector::EntityType {
                object_type,
                include_subtypes,
            } => Selector::EntityType {
                object_type: self.object_type(object_type, releases),
                include_subtypes: *include_subtypes,
            },
            Selector::Property {
                property_set,
                property,
                ..
            } => {
                let (set, name) = match property_set.as_deref() {
                    // A derived set's names are the engine's, never concepts.
                    Some(set) if axioval_ir::is_derived_set(set) => {
                        (Some(set.to_owned()), property.clone())
                    }
                    Some(set) if axioval_ir::is_reserved_set(set) => {
                        (Some(set.to_owned()), self.attribute(property, releases))
                    }
                    Some(set) => (
                        Some(self.property_set(set, releases)),
                        self.property(set, property, releases),
                    ),
                    None => (None, self.attribute(property, releases)),
                };
                let mut bound = filter.clone();
                if let Selector::Property {
                    property_set,
                    property,
                    ..
                } = &mut bound
                {
                    *property_set = set;
                    *property = name;
                }
                bound
            }
            Selector::AllOf { operands } => Selector::AllOf {
                operands: operands
                    .iter()
                    .map(|operand| self.bind_filter(operand, releases))
                    .collect(),
            },
            Selector::AnyOf { operands } => Selector::AnyOf {
                operands: operands
                    .iter()
                    .map(|operand| self.bind_filter(operand, releases))
                    .collect(),
            },
            Selector::Not { operand } => Selector::Not {
                operand: Box::new(self.bind_filter(operand, releases)),
            },
            Selector::Related {
                path,
                quantifier,
                selector,
            } => Selector::Related {
                path: path.clone(),
                quantifier: *quantifier,
                selector: Box::new(self.bind_filter(selector, releases)),
            },
            // Names no concept: patterns, classification systems, source
            // facts. A rule outcome was refused before translating.
            Selector::All
            | Selector::PropertyPattern { .. }
            | Selector::Classification { .. }
            | Selector::DerivedClass { .. }
            | Selector::DerivedGroup { .. }
            | Selector::Discipline { .. }
            | Selector::Source { .. }
            | Selector::RuleOutcome { .. } => filter.clone(),
        }
    }

    /// The selector a facet other than the entity stands for, as required;
    /// `None` when no selector states it as IDS decides it, so a rule
    /// checks it.
    fn facet_selector(
        &mut self,
        facet: &Facet,
        scope: &Scope<'_>,
    ) -> Result<Option<Selector>, Reason> {
        match facet {
            Facet::Entity(entity) => Ok(Some(self.entity_selector(entity, scope.releases)?.0)),
            Facet::Attribute(attribute) => match self.attribute_selector(attribute, scope) {
                Ok(selector) => Ok(Some(selector)),
                // Compared by `property-value`, which casts as IDS does.
                Err(Reason::AttributeType { .. }) if matches!(attribute.name, Value::Simple(_)) => {
                    Ok(None)
                }
                Err(reason) => Err(reason),
            },
            Facet::Classification(classification) => classification_selector(classification),
            Facet::Material(material) => self.material_selector(material, scope.releases),
            Facet::PartOf(part_of) => Ok(Some(self.part_of_selector(part_of, scope.releases)?)),
            // IDS casts a property's literal to the property's own type,
            // which `property-value` does and a selector does not.
            Facet::Property(_) => Ok(None),
        }
    }

    /// The exact check for a facet with `occurrence`, `None` when it always
    /// holds.
    fn requirement(
        &mut self,
        facet: &Facet,
        occurrence: Occurrence,
        scope: &Scope<'_>,
    ) -> Result<Option<Check>, Reason> {
        match facet {
            Facet::Property(property) => {
                if occurrence == Occurrence::Prohibited
                    && (property.value.is_some() || property.data_type.is_some())
                {
                    return self.negated(facet, scope);
                }
                self.property_check(property, occurrence, scope)
            }
            Facet::Attribute(attribute) => {
                match self.attempt(|writer| writer.attribute_check(attribute, occurrence, scope)) {
                    Err(Reason::AttributeType { .. })
                        if occurrence == Occurrence::Prohibited
                            && matches!(attribute.name, Value::Simple(_)) =>
                    {
                        self.negated(facet, scope)
                    }
                    other => other,
                }
            }
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
            Facet::Material(material) => self.material_check(facet, material, occurrence, scope),
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

    /// A prohibited facet no capability negates: the facet as required,
    /// whose every pass breaks the prohibition.
    fn negated(&mut self, facet: &Facet, scope: &Scope<'_>) -> Result<Option<Check>, Reason> {
        let holds = holds(facet);
        Ok(Some(
            match self.requirement(facet, Occurrence::Required, scope)? {
                Some(required) => Check::Negated {
                    required: Box::new(required),
                    holds,
                },
                // Met by every object, so broken by every one.
                None => breaking(&holds),
            },
        ))
    }

    fn material_check(
        &mut self,
        facet: &Facet,
        material: &Material,
        occurrence: Occurrence,
        scope: &Scope<'_>,
    ) -> Result<Option<Check>, Reason> {
        let Some(selector) = self.material_selector(material, scope.releases)? else {
            // Several facets one name must meet together: `property-value`
            // over the names the material goes by, any of which may.
            if occurrence == Occurrence::Prohibited {
                return self.negated(facet, scope);
            }
            let Some(value) = &material.value else {
                unreachable!("a material without a value has a selector");
            };
            let mut parameters = BTreeMap::new();
            value_parameters(value, &mut parameters)?;
            parameters.insert("quantifier".to_owned(), string("any"));
            let optional = occurrence == Occurrence::Optional;
            if optional {
                parameters.insert(
                    "optional".to_owned(),
                    ParameterValue::Boolean { value: true },
                );
            }
            parameters.insert(
                "property".to_owned(),
                ParameterValue::PropertyReference {
                    property: self.attribute(MATERIAL_NAMES, scope.releases),
                    property_set: Some(MATERIAL_SET.to_owned()),
                },
            );
            return Ok(Some(Check::Capability {
                kind: Kind::Value,
                title: Kind::Value.title(
                    &format!("a material named {}", shown_value(value)),
                    optional,
                ),
                parameters,
            }));
        };
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
            // With a value or a data type, `requirement` negates the
            // required facet instead.
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

    /// The selector for an entity facet whose classes must be checked
    /// objects in every release: an applicability, or a part-of whole.
    fn entity_selector(
        &mut self,
        entity: &Entity,
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let names = entity_names(entity, releases)?;
        let names = occurrences(names, releases, pattern_named(entity))?;
        self.classes_selector(entity, &names, releases)
    }

    /// The selector for an entity requirement: an object of another class,
    /// or of a class a release lacks, simply does not meet it.
    fn entity_requirement_selector(
        &mut self,
        entity: &Entity,
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let names = entity_names(entity, releases)?;
        self.classes_selector(entity, &names, releases)
    }

    fn classes_selector(
        &mut self,
        entity: &Entity,
        names: &[String],
        releases: &[IfcVersion],
    ) -> Result<(Selector, Vec<String>), Reason> {
        let mut operands = Vec::new();
        let mut classes = names.to_vec();
        for name in names {
            // IDS matches the named class only, never its subclasses.
            let own = self.class(name, releases);
            let Some((occurrence, type_object)) = mapped(name, names, releases) else {
                operands.push(own);
                continue;
            };
            // In IFC2X3 the class is an occurrence typed by a type object.
            let typed = all_of(vec![
                self.class(occurrence, releases),
                Selector::Related {
                    path: vec!["IfcRelDefinesByType:backward".to_owned()],
                    quantifier: RelatedQuantifier::Any,
                    selector: Box::new(self.class(type_object, releases)),
                },
            ]);
            // Another release defines the class itself, and there the
            // occurrence class is no such object: the mapping holds for
            // IFC2X3 models only.
            operands.push(own);
            operands.push(all_of(vec![
                Selector::Source {
                    field: SourceField::Schema,
                    operator: ComparisonOperator::Equals,
                    value: Some(ParameterValue::String {
                        value: "IFC2X3".to_owned(),
                    }),
                    case_sensitive: true,
                    trim: false,
                    quantifier: None,
                },
                typed,
            ]));
            if !classes.iter().any(|class| class == occurrence) {
                classes.push(occurrence.to_owned());
            }
        }
        let classes_selector = any_of(operands);
        let selector = match &entity.predefined_type {
            None => classes_selector,
            Some(value) => {
                let labels = user_defined_labels(&classes, releases);
                all_of(vec![
                    classes_selector,
                    self.predefined_type(value, &labels, releases)?,
                ])
            }
        };
        Ok((selector, classes))
    }

    /// Whether the object's predefined type, as IDS resolves it, meets
    /// `value`.
    ///
    /// `labels` are the attributes that state a user-defined type on the
    /// checked object itself: `ObjectType` on an occurrence, `ElementType`,
    /// `ProcessType` or `ResourceType` on a type object, which is typed by
    /// nothing, so its type attributes are absent and its own decide.
    fn predefined_type(
        &mut self,
        value: &Value,
        labels: &[&str],
        releases: &[IfcVersion],
    ) -> Result<Selector, Reason> {
        let mut slot = |set: &str, name: &str| (set.to_owned(), self.attribute(name, releases));
        let type_own = slot(TYPE_ATTRIBUTE_SET, "PredefinedType");
        let type_element = slot(TYPE_ATTRIBUTE_SET, "ElementType");
        let type_process = slot(TYPE_ATTRIBUTE_SET, "ProcessType");
        let own = slot(ATTRIBUTE_SET, "PredefinedType");
        let labels: Vec<Slot> = labels
            .iter()
            .map(|label| slot(ATTRIBUTE_SET, label))
            .collect();
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
                        all_of(vec![
                            unset(&own),
                            any_of(labels.iter().map(filled).collect()),
                        ]),
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
                any_of(labels.iter().map(test).collect::<Result<_, _>>()?),
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

    /// The selector a material facet stands for; `None` for a value
    /// restricted by several facets one name must meet together, which
    /// separate tests over the name list cannot require.
    fn material_selector(
        &mut self,
        material: &Material,
        releases: &[IfcVersion],
    ) -> Result<Option<Selector>, Reason> {
        let Some(value) = &material.value else {
            // Every material states its composition.
            let kind = (
                MATERIAL_SET.to_owned(),
                self.attribute(MATERIAL_KIND, releases),
            );
            return Ok(Some(exists(&kind)));
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
                    return Ok(None);
                }
            }
        };
        Ok(Some(any_of(tests.into_iter().map(any_element).collect())))
    }

    fn part_of_selector(
        &mut self,
        part_of: &PartOf,
        releases: &[IfcVersion],
    ) -> Result<Selector, Reason> {
        const VOIDS_FILLS: [&str; 2] = ["IfcRelFillsElement", "IfcRelVoidsElement"];
        let relationships: &[&str] = match part_of.relation {
            Some(Relation::Aggregates) => &["IfcRelAggregates"],
            Some(Relation::AssignsToGroup) => &["IfcRelAssignsToGroup"],
            Some(Relation::ContainedInSpatialStructure) => &["IfcRelContainedInSpatialStructure"],
            Some(Relation::Nests) => &["IfcRelNests"],
            // An element fills an opening that voids its host: the opening
            // is the relating end of the one, the host of the other.
            Some(Relation::VoidsElementFillsElement) => &VOIDS_FILLS,
            // Every relation IDS names, mixed along the chain.
            None => &[
                "IfcRelAggregates",
                "IfcRelAssignsToGroup",
                "IfcRelContainedInSpatialStructure",
                "IfcRelNests",
                VOIDS_FILLS[0],
                VOIDS_FILLS[1],
            ],
        };
        let names = whole_names(&part_of.entity, relationships, releases)?;
        let (whole, _) = self.classes_selector(&part_of.entity, &names, releases)?;
        // IDS follows the relations recursively from the part up to its
        // wholes, the relating ends, one step taking any of them.
        Ok(Selector::Related {
            path: vec![format!("{}:backward+", relationships.join("|"))],
            quantifier: RelatedQuantifier::Any,
            selector: Box::new(whole),
        })
    }

    /// A rule with `id` checking `check` over `selector`; an auxiliary one
    /// only serves the rules that read its outcome.
    fn rule(
        &mut self,
        id: String,
        specification: &Specification,
        requirement: Option<&Requirement>,
        check: Check,
        selector: &Selector,
        auxiliary: bool,
    ) -> RuleInstance {
        let (kind, title, parameters) = check.lower();
        let definition_id = self.definition(kind);
        let description = requirement
            .and_then(|requirement| requirement.instructions.as_deref())
            .or(specification.instructions.as_deref())
            .map(LocalizedText::plain);
        RuleInstance {
            id,
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
            gate: None,
            auxiliary,
            explanatory_images: Vec::new(),
            tags: vec!["ids".to_owned()],
            severity_bands: Vec::new(),
            severity_overrides: Vec::new(),
            categories: Vec::new(),
        }
    }

    /// The rule definition a kind of check uses, written once.
    fn definition(&mut self, kind: Kind) -> String {
        let (suffix, name, description) = kind.catalog();
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
                capability: kind.capability().id().to_owned(),
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

    /// Objects of exactly the class `entity`, never of its subclasses.
    fn class(&mut self, entity: &str, releases: &[IfcVersion]) -> Selector {
        Selector::EntityType {
            object_type: self.object_type(entity, releases),
            include_subtypes: false,
        }
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
    /// A prohibited facet as the negation of `required`: an auxiliary rule
    /// checks it as required, and every object that rule passes breaks the
    /// prohibition. `holds` says what the facet states.
    Negated { required: Box<Check>, holds: String },
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
            Check::Negated { .. } => unreachable!("a negated check is written as two rules"),
        }
    }
}

/// What an applicability selects: the facets a selector states, the
/// entity's classes and facet, and the facets only a rule checks, by their
/// position.
struct Applicable<'s> {
    selector: Selector,
    classes: Vec<String>,
    entity: &'s Entity,
    checked: Vec<(usize, Check)>,
}

/// The rule the first `ruleOutcome` selector in `selector` names, if any.
fn rule_outcome(selector: &Selector) -> Option<&str> {
    match selector {
        Selector::RuleOutcome { rule, .. } => Some(rule),
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            operands.iter().find_map(rule_outcome)
        }
        Selector::Not { operand } => rule_outcome(operand),
        Selector::Related { selector, .. } => rule_outcome(selector),
        _ => None,
    }
}

/// The objects the rule `id` passed.
fn passed(id: &str) -> Selector {
    Selector::RuleOutcome {
        rule: id.to_owned(),
        outcome: RuleOutcomeKind::Passed,
    }
}

/// A `selector-conformance` check every selected object fails: each breaks
/// the prohibition that it `holds`.
fn breaking(holds: &str) -> Check {
    Check::Conforms {
        requirement: not(Selector::All),
        title: format!("prohibited: {holds}"),
        message: format!("{holds}, which IDS prohibits"),
    }
}

/// What a facet states of an object, as required.
fn holds(facet: &Facet) -> String {
    match facet {
        Facet::Entity(entity) => format!("is a {}", shown_entity(entity)),
        Facet::Attribute(attribute) => match &attribute.value {
            Some(value) => format!(
                "has attribute {} {}",
                shown_value(&attribute.name),
                shown_value(value)
            ),
            None => format!("has attribute {}", shown_value(&attribute.name)),
        },
        Facet::Property(property) => {
            let mut shown = format!(
                "has {}.{}",
                shown_value(&property.property_set),
                shown_value(&property.base_name)
            );
            if let Some(data_type) = &property.data_type {
                shown.push_str(" of type ");
                shown.push_str(data_type);
            }
            if let Some(value) = &property.value {
                shown.push_str(" = ");
                shown.push_str(&shown_value(value));
            }
            shown
        }
        Facet::Classification(classification) => {
            format!("is classified {}", shown_classification(classification))
        }
        Facet::Material(material) => match &material.value {
            Some(value) => format!("has a material named {}", shown_value(value)),
            None => "has a material".to_owned(),
        },
        Facet::PartOf(part_of) => format!(
            "is part of {} through {}",
            shown_entity(&part_of.entity),
            part_of.relation.map_or("any relation", Relation::as_str)
        ),
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

impl Kind {
    /// Definition id suffix, name, description and capability.
    fn catalog(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Kind::Required => (
                "property-required",
                "Property is required",
                "An IDS property or attribute facet without a value: it must exist with a non-empty value.",
            ),
            Kind::DataType => (
                "property-data-type",
                "Property is required with a data type",
                "An IDS property facet with a dataType and no value: the property must exist with a non-empty value of that declared type.",
            ),
            Kind::Value => (
                "property-value",
                "Property value meets constraints",
                "An IDS property or attribute facet with a value, or an optional one with a dataType: literals and XML Schema facets cast to the value.",
            ),
            Kind::Conformance => (
                "facet",
                "Facet holds",
                "An IDS entity, classification, material, part-of or name-restricted attribute requirement: the facet's selector must hold, or must not when prohibited.",
            ),
            Kind::Forbidden => (
                "prohibited-property",
                "Property is prohibited",
                "A prohibited IDS property or attribute facet without a value: none of the named properties may hold a value.",
            ),
            Kind::Present => (
                "required-properties",
                "Properties are required",
                "An IDS property facet naming its set or property by pattern or enumeration, without a value: one property must match, and every matching one must hold a value.",
            ),
            Kind::Count => (
                "occurrence",
                "Applicable objects",
                "An IDS specification's minOccurs/maxOccurs: how many applicable objects each model may hold.",
            ),
            Kind::Classification => (
                "classification",
                "Classification is required",
                "An IDS classification requirement: an assignment in a matching system with a matching code or ancestor code, none when prohibited, or none at all when optional.",
            ),
        }
    }

    /// The trusted capability that decides the check.
    fn capability(self) -> &'static dyn RuleCapability {
        match self {
            Kind::Required => &axioval_rules::PropertyRequired,
            Kind::DataType => &axioval_rules::PropertyDataType,
            Kind::Value => &axioval_rules::PropertyValueConstraint,
            Kind::Conformance => &axioval_rules::SelectorConformance,
            Kind::Forbidden | Kind::Present => &axioval_rules::PropertyRequirements,
            Kind::Count => &axioval_rules::ObjectCount,
            Kind::Classification => &axioval_rules::ClassificationRequirement,
        }
    }

    /// The parameters, exactly as the capability declares them: read from
    /// the capability itself, so a new optional parameter or table column
    /// never leaves the definitions behind.
    fn parameters(self) -> Vec<ParameterDefinition> {
        self.capability()
            .parameters()
            .into_iter()
            .map(|descriptor| {
                let mut definition = parameter(
                    &descriptor.name,
                    kind_of(descriptor.parameter_type),
                    descriptor.required,
                );
                if let ParameterType::Table(columns) = descriptor.parameter_type {
                    definition.columns = columns
                        .iter()
                        .map(|column| TableColumnDefinition {
                            id: column.id.to_owned(),
                            name: LocalizedText::plain(column.id),
                            description: None,
                            kind: column.kind,
                            required: column.required,
                            unit_dimension: None,
                        })
                        .collect();
                }
                definition
            })
            .collect()
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

/// The definition-package spelling of a capability's parameter type.
fn kind_of(parameter_type: ParameterType) -> ParameterKind {
    match parameter_type {
        ParameterType::Boolean => ParameterKind::Boolean,
        ParameterType::Integer => ParameterKind::Integer,
        ParameterType::Number => ParameterKind::Number,
        ParameterType::String => ParameterKind::String,
        ParameterType::Quantity => ParameterKind::Quantity,
        ParameterType::Enum => ParameterKind::Enum,
        ParameterType::Date => ParameterKind::Date,
        ParameterType::DateTime => ParameterKind::DateTime,
        ParameterType::Reference => ParameterKind::Reference,
        ParameterType::ObjectTypeReference => ParameterKind::ObjectTypeReference,
        ParameterType::PropertyReference => ParameterKind::PropertyReference,
        ParameterType::Selector => ParameterKind::Selector,
        ParameterType::Expression => ParameterKind::Expression,
        ParameterType::StringList => ParameterKind::StringList,
        ParameterType::ReferenceList => ParameterKind::ReferenceList,
        ParameterType::Table(_) => ParameterKind::Table,
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

/// The releases a specification is checked in: every supported release
/// IDS names, whichever it lists, since `@ifcVersion` is metadata that
/// does not change a verdict (the buildingSMART case "specification version
/// is purely metadata"). A listed release no adapter reads is still a gap.
fn releases(specification: &Specification, gaps: &mut Vec<Gap>) -> Vec<IfcVersion> {
    let mut listed = false;
    for release in &specification.ifc_versions {
        if type_system(*release).is_some() {
            listed = true;
        } else {
            gaps.push(Gap {
                part: Part::Releases,
                reason: Reason::UnsupportedRelease(*release),
            });
        }
    }
    if !listed {
        gaps.push(Gap {
            part: Part::Releases,
            reason: Reason::NoSupportedRelease,
        });
    }
    IfcVersion::ALL
        .into_iter()
        .filter(|release| type_system(*release).is_some())
        .collect()
}

/// The type system a release binds to, `None` for one the IFC adapter
/// does not read. Every release IDS 1.0 names is read today.
#[allow(clippy::unnecessary_wraps)]
fn type_system(release: IfcVersion) -> Option<&'static str> {
    match release {
        IfcVersion::Ifc2x3 => Some(IFC2X3_TYPE_SYSTEM),
        IfcVersion::Ifc4 => Some(IFC4_TYPE_SYSTEM),
        IfcVersion::Ifc4x3Add2 => Some(IFC4X3_TYPE_SYSTEM),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn schema(release: IfcVersion) -> Option<&'static Schema> {
    match release {
        IfcVersion::Ifc2x3 => Some(ifc_schema::ifc2x3()),
        IfcVersion::Ifc4 => Some(ifc_schema::ifc4()),
        IfcVersion::Ifc4x3Add2 => Some(ifc_schema::ifc4x3()),
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

/// Refuses a class no release defines.
///
/// An IFC session makes a project object of every `IfcObject` occurrence,
/// `IfcContext` (an IFC4 `IfcProject`) and `IfcTypeObject`; every other
/// instance (a material, a classification, a relationship) is a resource
/// object, which an `entityType` selector naming its class selects as
/// exactly. A class some release lacks matches nothing in its models, as in
/// IDS.
fn occurrences(
    names: Vec<String>,
    releases: &[IfcVersion],
    matched: bool,
) -> Result<Vec<String>, Reason> {
    if !matched {
        if let Some(unknown) = names.iter().find(|name| !defined(name, &names, releases)) {
            return Err(Reason::UnknownEntity(unknown.clone()));
        }
    }
    Ok(names)
}

/// The class `name` stands for in `release`: its IFC2X3 occurrence class
/// when the type mapping table maps it there, else itself.
fn in_release<'a>(
    name: &'a str,
    names: &[String],
    releases: &[IfcVersion],
    release: IfcVersion,
) -> &'a str {
    match mapped(name, names, releases) {
        Some((occurrence, _)) if release == IfcVersion::Ifc2x3 => occurrence,
        _ => name,
    }
}

/// Whether some release defines `name`, or the class it stands for there.
fn defined(name: &str, names: &[String], releases: &[IfcVersion]) -> bool {
    releases.iter().any(|release| {
        schema(*release).is_some_and(|schema| {
            schema
                .entity(in_release(name, names, releases, *release))
                .is_some()
        })
    })
}

/// Whether an IFC session makes project objects of `name`'s instances:
/// occurrences, contexts and type objects.
///
/// An abstract class has no instances of its own, and IDS matches the named
/// class only, so it matches nothing in any model, as its selector does
/// (`IFCOBJECTDEFINITION`, which a pattern for a whole names).
fn checked(schema: &Schema, name: &str) -> bool {
    ["IFCOBJECT", "IFCCONTEXT", "IFCTYPEOBJECT"]
        .iter()
        .any(|ancestor| schema.is_a(name, ancestor))
        || schema.entity(name).is_some_and(|entity| entity.abstract_)
}

/// The attributes stating a user-defined type on objects of `names`
/// themselves: `ObjectType` for an occurrence or a context, and for a type
/// object its `ElementType`, `ProcessType` or `ResourceType`, whichever its
/// class declares.
fn user_defined_labels(names: &[String], releases: &[IfcVersion]) -> Vec<&'static str> {
    let mut labels = Vec::new();
    for schema in schemas(releases) {
        for name in names {
            if schema.entity(name).is_none() {
                continue;
            }
            let candidates: &[&'static str] = if schema.is_a(name, "IFCTYPEOBJECT") {
                &["ElementType", "ProcessType", "ResourceType"]
            } else {
                &["ObjectType"]
            };
            for label in candidates {
                let declared = schema
                    .attributes(name)
                    .iter()
                    .any(|attribute| attribute.name.eq_ignore_ascii_case(label));
                if declared && !labels.contains(label) {
                    labels.push(*label);
                }
            }
        }
    }
    if labels.is_empty() {
        labels.push("ObjectType");
    }
    labels
}

/// The classes a part-of whole names that can be the relating end of one
/// of `relationships` in some release; each must be a checked object.
///
/// A class that can never be the whole (a wall is never a spatial
/// container) matches nothing, as in IDS, so a pattern drops it. A class
/// that can be the whole but is no project object (a resource) would
/// silently fail every part, so it is a gap.
fn whole_names(
    entity: &Entity,
    relationships: &[&str],
    releases: &[IfcVersion],
) -> Result<Vec<String>, Reason> {
    let names = entity_names(entity, releases)?;
    let mut wholes = Vec::new();
    for named in &names {
        let mut relating = false;
        for release in releases {
            let Some(schema) = schema(*release) else {
                continue;
            };
            let name = in_release(named, &names, releases, *release).to_owned();
            if schema.entity(&name).is_none() {
                if pattern_named(entity) || defined(named, &names, releases) {
                    continue;
                }
                return Err(Reason::UnknownEntity(name));
            }
            let accepts = relationships.iter().any(|relationship| {
                let end = schema
                    .attributes(relationship)
                    .into_iter()
                    .find(|attribute| attribute.name.starts_with("Relating"))
                    .map(|attribute| attribute.type_name.clone());
                // An end the schema does not spell as one entity type is
                // assumed to accept anything.
                end.is_none_or(|end| schema.entity(&end).is_none() || schema.is_a(&name, &end))
            });
            if accepts {
                relating = true;
                if !checked(schema, &name) {
                    return Err(Reason::NotAnObject(name));
                }
            }
        }
        if relating || !pattern_named(entity) {
            wholes.push(named.clone());
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

/// The IFC2X3 occurrence and type object classes the IDS IFC2X3 type
/// mapping table gives `name`, when IFC2X3 is among `releases` and does not
/// define `name` itself: IDS then matches an occurrence of that class typed
/// by a type object of that class. `None` also when `names` names the
/// occurrence class too (a pattern matching both), whose objects already
/// cover the mapped ones.
///
/// The mapping reads the occurrence's class and its `IfcRelDefinesByType`
/// type object's class only, never `IfcTypeObject.ApplicableOccurrence`.
fn mapped(
    name: &str,
    names: &[String],
    releases: &[IfcVersion],
) -> Option<(&'static str, &'static str)> {
    if !releases.contains(&IfcVersion::Ifc2x3) || ifc_schema::ifc2x3().entity(name).is_some() {
        return None;
    }
    IFC2X3_TYPE_MAPPING
        .iter()
        .find(|(mapped, _, _)| *mapped == name)
        .filter(|(_, occurrence, _)| !names.iter().any(|named| named == occurrence))
        .map(|(_, occurrence, type_object)| (*occurrence, *type_object))
}

/// The IDS IFC2X3 occurrence and type mapping table
/// (`Documentation/ImplementersDocumentation/ifc2x3-occurrence-type-mapping-table.md`):
/// the class an IDS facet names, and the IFC2X3 occurrence and type object
/// classes it stands for.
const IFC2X3_TYPE_MAPPING: &[(&str, &str, &str)] = &[
    ("IFCFURNITURE", "IFCFURNISHINGELEMENT", "IFCFURNITURETYPE"),
    (
        "IFCSYSTEMFURNITUREELEMENT",
        "IFCFURNISHINGELEMENT",
        "IFCSYSTEMFURNITUREELEMENTTYPE",
    ),
    (
        "IFCACTUATOR",
        "IFCDISTRIBUTIONCONTROLELEMENT",
        "IFCACTUATORTYPE",
    ),
    ("IFCALARM", "IFCDISTRIBUTIONCONTROLELEMENT", "IFCALARMTYPE"),
    (
        "IFCCONTROLLER",
        "IFCDISTRIBUTIONCONTROLELEMENT",
        "IFCCONTROLLERTYPE",
    ),
    (
        "IFCFLOWINSTRUMENT",
        "IFCDISTRIBUTIONCONTROLELEMENT",
        "IFCFLOWINSTRUMENTTYPE",
    ),
    (
        "IFCSENSOR",
        "IFCDISTRIBUTIONCONTROLELEMENT",
        "IFCSENSORTYPE",
    ),
    (
        "IFCAIRTOAIRHEATRECOVERY",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCAIRTOAIRHEATRECOVERYTYPE",
    ),
    ("IFCBOILER", "IFCENERGYCONVERSIONDEVICE", "IFCBOILERTYPE"),
    ("IFCCHILLER", "IFCENERGYCONVERSIONDEVICE", "IFCCHILLERTYPE"),
    ("IFCCOIL", "IFCENERGYCONVERSIONDEVICE", "IFCCOILTYPE"),
    (
        "IFCCONDENSER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCCONDENSERTYPE",
    ),
    (
        "IFCCOOLEDBEAM",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCCOOLEDBEAMTYPE",
    ),
    (
        "IFCCOOLINGTOWER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCCOOLINGTOWERTYPE",
    ),
    (
        "IFCELECTRICGENERATOR",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCELECTRICGENERATORTYPE",
    ),
    (
        "IFCELECTRICMOTOR",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCELECTRICMOTORTYPE",
    ),
    (
        "IFCEVAPORATIVECOOLER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCEVAPORATIVECOOLERTYPE",
    ),
    (
        "IFCEVAPORATOR",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCEVAPORATORTYPE",
    ),
    (
        "IFCHEATEXCHANGER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCHEATEXCHANGERTYPE",
    ),
    (
        "IFCHUMIDIFIER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCHUMIDIFIERTYPE",
    ),
    (
        "IFCMOTORCONNECTION",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCMOTORCONNECTIONTYPE",
    ),
    (
        "IFCSPACEHEATER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCSPACEHEATERTYPE",
    ),
    (
        "IFCTRANSFORMER",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCTRANSFORMERTYPE",
    ),
    (
        "IFCTUBEBUNDLE",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCTUBEBUNDLETYPE",
    ),
    (
        "IFCUNITARYEQUIPMENT",
        "IFCENERGYCONVERSIONDEVICE",
        "IFCUNITARYEQUIPMENTTYPE",
    ),
    (
        "IFCAIRTERMINALBOX",
        "IFCFLOWCONTROLLER",
        "IFCAIRTERMINALBOXTYPE",
    ),
    ("IFCDAMPER", "IFCFLOWCONTROLLER", "IFCDAMPERTYPE"),
    (
        "IFCELECTRICTIMECONTROL",
        "IFCFLOWCONTROLLER",
        "IFCELECTRICTIMECONTROLTYPE",
    ),
    ("IFCFLOWMETER", "IFCFLOWCONTROLLER", "IFCFLOWMETERTYPE"),
    (
        "IFCPROTECTIVEDEVICE",
        "IFCFLOWCONTROLLER",
        "IFCPROTECTIVEDEVICETYPE",
    ),
    (
        "IFCSWITCHINGDEVICE",
        "IFCFLOWCONTROLLER",
        "IFCSWITCHINGDEVICETYPE",
    ),
    ("IFCVALVE", "IFCFLOWCONTROLLER", "IFCVALVETYPE"),
    (
        "IFCCABLECARRIERFITTING",
        "IFCFLOWFITTING",
        "IFCCABLECARRIERFITTINGTYPE",
    ),
    ("IFCDUCTFITTING", "IFCFLOWFITTING", "IFCDUCTFITTINGTYPE"),
    ("IFCJUNCTIONBOX", "IFCFLOWFITTING", "IFCJUNCTIONBOXTYPE"),
    ("IFCPIPEFITTING", "IFCFLOWFITTING", "IFCPIPEFITTINGTYPE"),
    ("IFCCOMPRESSOR", "IFCFLOWMOVINGDEVICE", "IFCCOMPRESSORTYPE"),
    ("IFCFAN", "IFCFLOWMOVINGDEVICE", "IFCFANTYPE"),
    ("IFCPUMP", "IFCFLOWMOVINGDEVICE", "IFCPUMPTYPE"),
    (
        "IFCCABLECARRIERSEGMENT",
        "IFCFLOWSEGMENT",
        "IFCCABLECARRIERSEGMENTTYPE",
    ),
    ("IFCCABLESEGMENT", "IFCFLOWSEGMENT", "IFCCABLESEGMENTTYPE"),
    ("IFCDUCTSEGMENT", "IFCFLOWSEGMENT", "IFCDUCTSEGMENTTYPE"),
    ("IFCPIPESEGMENT", "IFCFLOWSEGMENT", "IFCPIPESEGMENTTYPE"),
    (
        "IFCELECTRICFLOWSTORAGEDEVICE",
        "IFCFLOWSTORAGEDEVICE",
        "IFCELECTRICFLOWSTORAGEDEVICETYPE",
    ),
    ("IFCTANK", "IFCFLOWSTORAGEDEVICE", "IFCTANKTYPE"),
    ("IFCAIRTERMINAL", "IFCFLOWTERMINAL", "IFCAIRTERMINALTYPE"),
    (
        "IFCELECTRICAPPLIANCE",
        "IFCFLOWTERMINAL",
        "IFCELECTRICAPPLIANCETYPE",
    ),
    (
        "IFCFIRESUPPRESSIONTERMINAL",
        "IFCFLOWTERMINAL",
        "IFCFIRESUPPRESSIONTERMINALTYPE",
    ),
    ("IFCLAMP", "IFCFLOWTERMINAL", "IFCLAMPTYPE"),
    ("IFCLIGHTFIXTURE", "IFCFLOWTERMINAL", "IFCLIGHTFIXTURETYPE"),
    ("IFCOUTLET", "IFCFLOWTERMINAL", "IFCOUTLETTYPE"),
    (
        "IFCSANITARYTERMINAL",
        "IFCFLOWTERMINAL",
        "IFCSANITARYTERMINALTYPE",
    ),
    (
        "IFCSTACKTERMINAL",
        "IFCFLOWTERMINAL",
        "IFCSTACKTERMINALTYPE",
    ),
    (
        "IFCWASTETERMINAL",
        "IFCFLOWTERMINAL",
        "IFCWASTETERMINALTYPE",
    ),
    (
        "IFCDUCTSILENCER",
        "IFCFLOWTREATMENTDEVICE",
        "IFCDUCTSILENCERTYPE",
    ),
    ("IFCFILTER", "IFCFLOWTREATMENTDEVICE", "IFCFILTERTYPE"),
    (
        "IFCVIBRATIONISOLATOR",
        "IFCEQUIPMENTELEMENT",
        "IFCVIBRATIONISOLATORTYPE",
    ),
];

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
            Some(TypeKind::Defined(alias)) => current.clone_from(alias),
            // A select, and any type kind added upstream, is no known kind.
            Some(_) => return None,
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

/// A classification facet as a selector: literal or enumerated systems, and
/// a literal, enumerated or patterned code, or none for the system alone.
///
/// A reference matches its own code and every ancestor's, so a value names a
/// class and all its subclasses, as IDS reads full classifications. The
/// system and the code are matched on the same reference. A system given
/// as a pattern has no selector (`None`); the `classification` capability
/// checks it.
fn classification_selector(classification: &Classification) -> Result<Option<Selector>, Reason> {
    let systems = match &classification.system {
        Value::Simple(literal) => vec![literal.clone()],
        Value::Restriction(restriction) if only_enumeration(restriction) => {
            restriction.enumeration.clone()
        }
        Value::Restriction(restriction) => {
            only_names(restriction)?;
            for pattern in &restriction.patterns {
                translated(pattern)?;
            }
            return Ok(None);
        }
    };
    // (code, pattern) alternatives; neither for the system alone.
    let codes: Vec<(Option<String>, Option<String>)> = match &classification.value {
        None => vec![(None, None)],
        Some(Value::Simple(code)) => vec![(Some(code.clone()), None)],
        Some(Value::Restriction(restriction)) => {
            match Names::of(&Value::Restriction(restriction.clone()))? {
                Names::Literals(codes) => {
                    codes.into_iter().map(|code| (Some(code), None)).collect()
                }
                Names::Patterns(patterns) => patterns
                    .into_iter()
                    .map(|pattern| (None, Some(pattern)))
                    .collect(),
            }
        }
    };
    let mut operands = Vec::new();
    for system in &systems {
        for (code, code_pattern) in &codes {
            operands.push(Selector::Classification {
                system: system.clone(),
                code: code.clone(),
                code_pattern: code_pattern.clone(),
                // Only a code or a pattern names a class to descend from.
                include_descendants: code.is_some() || code_pattern.is_some(),
            });
        }
    }
    Ok(Some(any_of(operands)))
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
