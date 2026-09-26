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
//! # Releases
//!
//! A specification's concepts are named only in the type systems of the IFC
//! releases it lists, so a rule for an IFC4-only specification cannot bind to
//! an IFC2X3 model: the engine reports it as not evaluated
//! (`InvalidDeclaration`) rather than applying it. `IFC4X3_ADD2` has no type
//! system any Axioval adapter declares and is reported as a gap.

use std::collections::BTreeMap;
use std::fmt;

use axioval_ir::contract::{
    DefinitionPackage, ExternalName, LocalizedText, ObjectTypeDefinition, PackageMetadata,
    ParameterDefinition, ParameterKind, ParameterValue, PropertyDefinition, PropertySetDefinition,
    PropertyValueKind, RuleApplicability, RuleDefinition, RuleFolder, RuleInstance, RuleSetPackage,
    Selector, Severity,
};
use openbim_ids::{
    Entity, Facet, Ids, IfcVersion, Occurrence, Property, Requirement, Specification, Value,
};
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
    /// At least one applicable object must exist. A report carries findings
    /// about objects only, so "none exists" has nowhere to go.
    Existence,
    /// No applicable object may exist.
    Prohibition,
    /// A bounded count of applicable objects other than "at least one".
    Count {
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`; `None` is unbounded.
        max: Option<u32>,
    },
    /// An applicability without facets.
    EmptyApplicability,
    /// A facet kind no capability decides yet, named as IDS spells it.
    FacetKind(&'static str),
    /// An entity facet naming a predefined type.
    PredefinedType,
    /// An entity name that is not upper case, which IDS never matches.
    EntityCase(String),
    /// A value given as an `xs:restriction` where only literals translate.
    Restriction,
    /// A required value. Comparing it the way IDS does needs the property's
    /// IFC defined type, which resolved properties do not carry.
    PropertyValue,
    /// A `dataType`, which needs the property's IFC defined type.
    DataType(String),
    /// A prohibited facet.
    Prohibited,
    /// An entity requirement naming a class other than the applicability's.
    EntityRequirement,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::UnsupportedRelease(release) => {
                write!(f, "{release} has no type system an adapter declares")
            }
            Reason::NoSupportedRelease => f.write_str("no listed IFC release is supported"),
            Reason::Existence => f.write_str(
                "requires at least one applicable object; reports cannot state that none exists",
            ),
            Reason::Prohibition => f.write_str("requires that no applicable object exists"),
            Reason::Count { min, max } => match max {
                Some(max) => write!(f, "requires {min} to {max} applicable objects"),
                None => write!(f, "requires at least {min} applicable objects"),
            },
            Reason::EmptyApplicability => f.write_str("the applicability has no facets"),
            Reason::FacetKind(kind) => write!(f, "no capability decides a {kind} facet"),
            Reason::PredefinedType => f.write_str("no capability decides a predefined type"),
            Reason::EntityCase(name) => {
                write!(f, "entity name {name:?} is not upper case, which IDS never matches")
            }
            Reason::Restriction => f.write_str("only simple values translate, not restrictions"),
            Reason::PropertyValue => f.write_str(
                "comparing a value as IDS does needs the property's IFC type, which evidence does not carry",
            ),
            Reason::DataType(data_type) => write!(
                f,
                "dataType {data_type} needs the property's IFC type, which evidence does not carry"
            ),
            Reason::Prohibited => f.write_str("no capability decides a prohibited facet"),
            Reason::EntityRequirement => f.write_str(
                "an entity requirement other than the applicability's own entity is not decided",
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
    Ok(Translation {
        definitions: DefinitionPackage {
            schema_version: SCHEMA_VERSION.to_owned(),
            package: metadata(&definitions_id, format!("{title}: definitions")),
            sources: BTreeMap::new(),
            object_types: writer.object_types,
            properties: writer.properties,
            property_sets: writer.property_sets,
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

/// Accumulates concepts and definitions across specifications.
struct Writer<'o> {
    options: &'o Options,
    /// Concept ids by (kind, releases, name), so equal concepts are shared.
    concepts: BTreeMap<(&'static str, Vec<IfcVersion>, String), String>,
    object_types: BTreeMap<String, ObjectTypeDefinition>,
    properties: BTreeMap<String, PropertyDefinition>,
    property_sets: BTreeMap<String, PropertySetDefinition>,
    definitions: BTreeMap<String, RuleDefinition>,
}

impl<'o> Writer<'o> {
    fn new(options: &'o Options) -> Self {
        Self {
            options,
            concepts: BTreeMap::new(),
            object_types: BTreeMap::new(),
            properties: BTreeMap::new(),
            property_sets: BTreeMap::new(),
            definitions: BTreeMap::new(),
        }
    }

    fn specification(
        &mut self,
        number: usize,
        specification: &Specification,
    ) -> (Vec<RuleInstance>, Vec<Gap>) {
        let mut gaps = Vec::new();
        let releases = releases(specification, &mut gaps);
        occurrence(specification, &mut gaps);
        let applicability = applicability(specification, &mut gaps);
        let requirements = specification
            .requirements
            .iter()
            .flat_map(|requirements| requirements.facets.iter())
            .enumerate()
            .filter_map(|(index, requirement)| {
                let part = Part::Requirement { facet: index + 1 };
                match check(requirement, applicability.as_ref().map(|(_, e)| *e)) {
                    Ok(check) => check.map(|check| (index + 1, requirement, check)),
                    Err(reason) => {
                        gaps.push(Gap { part, reason });
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        let skipped = gaps.iter().any(Gap::skips);
        let Some((entities, _)) = applicability.filter(|_| !skipped) else {
            return (Vec::new(), gaps);
        };
        let selector = self.selector(&entities, &releases);
        let rules = requirements
            .into_iter()
            .map(|(facet, requirement, check)| {
                self.rule(
                    number,
                    facet,
                    specification,
                    requirement,
                    check,
                    &selector,
                    &releases,
                )
            })
            .collect();
        (rules, gaps)
    }

    fn selector(&mut self, entities: &[String], releases: &[IfcVersion]) -> Selector {
        let mut operands: Vec<Selector> = entities
            .iter()
            .map(|entity| Selector::EntityType {
                object_type: self.object_type(entity, releases),
                // IDS matches the named class only, never its subclasses.
                include_subtypes: false,
            })
            .collect();
        if operands.len() == 1 {
            operands.remove(0)
        } else {
            Selector::AnyOf { operands }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn rule(
        &mut self,
        number: usize,
        facet: usize,
        specification: &Specification,
        requirement: &Requirement,
        check: Check,
        selector: &Selector,
        releases: &[IfcVersion],
    ) -> RuleInstance {
        let Check::PropertyRequired { set, name } = check;
        let definition_id = self.property_required_definition();
        let property = self.property(&set, &name, releases);
        let property_set = self.property_set(&set, releases);
        let description = requirement
            .instructions
            .as_deref()
            .or(specification.instructions.as_deref())
            .map(LocalizedText::plain);
        RuleInstance {
            id: format!("spec{number}.facet{facet}"),
            definition_id,
            name: LocalizedText::plain(format!("{set}.{name} is required")),
            description,
            enabled: true,
            severity: Severity::Error,
            message: None,
            parameters: BTreeMap::from([(
                "property".to_owned(),
                ParameterValue::PropertyReference {
                    property,
                    property_set: Some(property_set),
                },
            )]),
            applicability: RuleApplicability::Selector(selector.clone()),
            requirements: Vec::new(),
            citations: Vec::new(),
            parameter_citations: Vec::new(),
            explanatory_images: Vec::new(),
            tags: vec!["ids".to_owned()],
        }
    }

    fn property_required_definition(&mut self) -> String {
        let id = format!("{}.property-required", self.options.package_id);
        self.definitions
            .entry(id.clone())
            .or_insert_with(|| RuleDefinition {
                id: id.clone(),
                name: LocalizedText::plain("Property is required"),
                description: Some(LocalizedText::plain(
                    "An IDS property facet without a value: the property must exist with a non-empty value.",
                )),
                capability: PROPERTY_REQUIRED.to_owned(),
                parameters: BTreeMap::from([(
                    "property".to_owned(),
                    ParameterDefinition {
                        id: "property".to_owned(),
                        name: LocalizedText::plain("Property"),
                        description: None,
                        kind: ParameterKind::PropertyReference,
                        referenced_value_kind: None,
                        required: true,
                        default_value: None,
                        allowed_values: Vec::new(),
                        unit_dimension: None,
                        citations: Vec::new(),
                    },
                )]),
                tags: vec!["ids".to_owned()],
                citations: Vec::new(),
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
        if let Some(id) = self.concepts.get(&key) {
            return (id.clone(), false);
        }
        let count = self.concepts.keys().filter(|(k, ..)| *k == kind).count();
        let id = format!("{}.{kind}-{}", self.options.package_id, count + 1);
        self.concepts.insert(key, id.clone());
        (id, true)
    }

    fn object_type(&mut self, entity: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("entity", releases, entity);
        if new {
            self.object_types.insert(
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
            self.properties.insert(
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

    fn property_set(&mut self, set: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("property-set", releases, set);
        if new {
            self.property_sets.insert(
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

/// One exactly translatable requirement.
enum Check {
    /// The property must exist with a non-empty value.
    PropertyRequired { set: String, name: String },
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

/// Records a gap unless the bounds ask only that every applicable object
/// meet the requirements.
fn occurrence(specification: &Specification, gaps: &mut Vec<Gap>) {
    let applicability = &specification.applicability;
    let reason = match (applicability.min_occurs, applicability.max_occurs) {
        (0, None) => return,
        (_, Some(0)) => Reason::Prohibition,
        (1, None) => Reason::Existence,
        (min, max) => Reason::Count { min, max },
    };
    gaps.push(Gap {
        part: Part::Occurrence,
        reason,
    });
}

/// The entity names the applicability selects, and the entity facet itself.
///
/// `None` when any facet is untranslatable; the gaps say which.
fn applicability<'s>(
    specification: &'s Specification,
    gaps: &mut Vec<Gap>,
) -> Option<(Vec<String>, &'s Entity)> {
    let facets = &specification.applicability.facets;
    if facets.is_empty() {
        gaps.push(Gap {
            part: Part::Applicability { facet: 1 },
            reason: Reason::EmptyApplicability,
        });
        return None;
    }
    let mut selected = None;
    for (index, facet) in facets.iter().enumerate() {
        let part = Part::Applicability { facet: index + 1 };
        let result = match facet {
            Facet::Entity(entity) => {
                entity_names(entity).map(|names| selected = Some((names, entity)))
            }
            other => Err(Reason::FacetKind(other.kind())),
        };
        if let Err(reason) = result {
            gaps.push(Gap { part, reason });
        }
    }
    let translated = !gaps
        .iter()
        .any(|gap| matches!(gap.part, Part::Applicability { .. }));
    selected.filter(|_| translated)
}

/// The class names an entity facet matches exactly.
fn entity_names(entity: &Entity) -> Result<Vec<String>, Reason> {
    if entity.predefined_type.is_some() {
        return Err(Reason::PredefinedType);
    }
    let names = match &entity.name {
        Value::Simple(name) => vec![name.clone()],
        // An enumeration of literals is a set of classes; nothing else in a
        // restriction is a literal.
        Value::Restriction(restriction)
            if restriction.base == "string"
                && !restriction.enumeration.is_empty()
                && restriction.patterns.is_empty()
                && restriction.min_inclusive.is_none()
                && restriction.max_inclusive.is_none()
                && restriction.min_exclusive.is_none()
                && restriction.max_exclusive.is_none()
                && restriction.length.is_none()
                && restriction.min_length.is_none()
                && restriction.max_length.is_none()
                && restriction.total_digits.is_none()
                && restriction.fraction_digits.is_none() =>
        {
            restriction.enumeration.clone()
        }
        Value::Restriction(_) => return Err(Reason::Restriction),
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

/// The exact check for a requirement, `None` when it always holds.
fn check(
    requirement: &Requirement,
    applicability: Option<&Entity>,
) -> Result<Option<Check>, Reason> {
    match &requirement.facet {
        Facet::Property(property) => property_check(property, requirement.occurrence),
        Facet::Entity(entity) => {
            // Requiring the class the applicability already selected always
            // holds. Anything else needs an entity capability.
            let same = applicability.is_some_and(|applicable| {
                applicable.predefined_type.is_none()
                    && entity.predefined_type.is_none()
                    && matches!((&applicable.name, &entity.name), (Value::Simple(a), Value::Simple(b)) if a == b)
            });
            if same {
                Ok(None)
            } else {
                Err(Reason::EntityRequirement)
            }
        }
        other => Err(Reason::FacetKind(other.kind())),
    }
}

fn property_check(property: &Property, occurrence: Occurrence) -> Result<Option<Check>, Reason> {
    let (Value::Simple(set), Value::Simple(name)) = (&property.property_set, &property.base_name)
    else {
        return Err(Reason::Restriction);
    };
    if let Some(data_type) = &property.data_type {
        return Err(Reason::DataType(data_type.clone()));
    }
    if property.value.is_some() {
        return Err(Reason::PropertyValue);
    }
    match occurrence {
        Occurrence::Required => Ok(Some(Check::PropertyRequired {
            set: set.clone(),
            name: name.clone(),
        })),
        // Without a value, an optional property is satisfied whether or not
        // it is there.
        Occurrence::Optional => Ok(None),
        Occurrence::Prohibited => Err(Reason::Prohibited),
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
