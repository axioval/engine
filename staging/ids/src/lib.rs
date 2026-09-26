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
    Attribute, Classification, Entity, Facet, Ids, IfcVersion, Material, Occurrence, PartOf,
    Property, Relation, Requirement, Specification, Value,
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
const PROPERTY_DATA_TYPE: &str = "axioval:capability.property-data-type";
const PROPERTY_VALUE: &str = "axioval:capability.property-value";
const ATTRIBUTE_VALUE: &str = "axioval:capability.attribute-value";
const PREDEFINED_TYPE: &str = "axioval:capability.predefined-type";
const CLASSIFICATION: &str = "axioval:capability.classification";
const MATERIAL: &str = "axioval:capability.material";
const PART_OF: &str = "axioval:capability.part-of";
const ENTITY: &str = "axioval:capability.entity";

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
    /// An applicability class the release does not define.
    UnknownEntity {
        /// The class as written.
        entity: String,
        /// The release that lacks it.
        release: IfcVersion,
    },
    /// An applicability class whose instances are not `IfcObject`
    /// occurrences (a type object, an IFC4 `IfcProject`, a resource), which
    /// an IFC session does not make project objects of.
    NotAnObject(String),
    /// An entity facet naming a predefined type.
    PredefinedType,
    /// An entity name that is not upper case, which IDS never matches.
    EntityCase(String),
    /// A class, property set or property name given as an `xs:restriction`;
    /// only an enumeration of classes translates.
    Restriction,
    /// A restriction facet no capability applies, named as XML Schema
    /// spells it (`totalDigits`, `fractionDigits`).
    RestrictionFacet(&'static str),
    /// A restriction with no facets, whose meaning IDS leaves open.
    EmptyRestriction,
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
            Reason::UnknownEntity { entity, release } => {
                write!(f, "{release} defines no entity {entity}")
            }
            Reason::NotAnObject(entity) => write!(
                f,
                "{entity} is not an IfcObject occurrence, which is all a model session checks"
            ),
            Reason::PredefinedType => f.write_str("no capability decides a predefined type"),
            Reason::EntityCase(name) => {
                write!(
                    f,
                    "entity name {name:?} is not upper case, which IDS never matches"
                )
            }
            Reason::Restriction => f.write_str(
                "a class, property set or property name given as a restriction is not translated",
            ),
            Reason::RestrictionFacet(facet) => {
                write!(f, "no capability applies the restriction facet {facet}")
            }
            Reason::EmptyRestriction => f.write_str("a restriction without facets"),
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
        let applicability = applicability(specification, &releases, &mut gaps);
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
        let Check {
            set,
            name,
            kind,
            parameters: extra,
        } = check;
        let definition_id = self.definition(kind);
        let (reference, subject) = match &set {
            _ if kind.reference().is_none() => (
                ("", ParameterValue::Boolean { value: false }),
                String::new(),
            ),
            Some(set) => (
                (
                    "property",
                    ParameterValue::PropertyReference {
                        property: self.property(set, &name, releases),
                        property_set: Some(self.property_set(set, releases)),
                    },
                ),
                format!("{set}.{name}"),
            ),
            None => (
                (
                    "attribute",
                    ParameterValue::PropertyReference {
                        property: self.attribute(&name, releases),
                        property_set: None,
                    },
                ),
                format!("attribute {name}"),
            ),
        };
        let mut parameters = BTreeMap::new();
        if !reference.0.is_empty() {
            parameters.insert(reference.0.to_owned(), reference.1);
        }
        parameters.extend(extra);
        let optional = parameters.contains_key("optional");
        let title = match kind {
            CheckKind::PredefinedType => format!("{name} has the required predefined type"),
            CheckKind::Classification => format!("classification {name}"),
            CheckKind::Material => format!("material {name}"),
            CheckKind::PartOf => format!("part of {name}"),
            CheckKind::Entity => format!("is a {name}"),
            CheckKind::Required => format!("{subject} is required"),
            CheckKind::DataType => format!("{subject} is required with a declared type"),
            CheckKind::Value | CheckKind::Attribute if optional => {
                format!("{subject}, where present, meets its constraints")
            }
            CheckKind::Attribute if parameters.len() == 1 => format!("{subject} is required"),
            CheckKind::Value | CheckKind::Attribute => format!("{subject} meets its constraints"),
        };
        let description = requirement
            .instructions
            .as_deref()
            .or(specification.instructions.as_deref())
            .map(LocalizedText::plain);
        RuleInstance {
            id: format!("spec{number}.facet{facet}"),
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
        }
    }

    /// The rule definition a kind of check uses, written once.
    fn definition(&mut self, kind: CheckKind) -> String {
        let (suffix, name, description, capability) = kind.catalog();
        let id = format!("{}.{suffix}", self.options.package_id);
        self.definitions.entry(id.clone()).or_insert_with(|| {
            let mut parameters = BTreeMap::new();
            if let Some(reference) = kind.reference() {
                parameters.insert(
                    reference.to_owned(),
                    parameter(reference, ParameterKind::PropertyReference, true),
                );
            }
            for (id, parameter_kind, required) in kind.parameters() {
                parameters.insert(
                    (*id).to_owned(),
                    parameter(id, parameter_kind.clone(), *required),
                );
            }
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

    /// A direct attribute, as a property concept without a set.
    fn attribute(&mut self, name: &str, releases: &[IfcVersion]) -> String {
        let (id, new) = self.concept("attribute", releases, name);
        if new {
            self.properties.insert(
                id.clone(),
                PropertyDefinition {
                    id: id.clone(),
                    name: LocalizedText::plain(name),
                    description: Some(LocalizedText::plain(format!(
                        "The IFC attribute {name}. IDS states no value kind; `string` is nominal."
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
        citations: Vec::new(),
    }
}

/// One exactly translatable requirement.
struct Check {
    /// The property set; `None` for a direct attribute.
    set: Option<String>,
    name: String,
    kind: CheckKind,
    /// Parameters besides the property reference.
    parameters: BTreeMap<String, ParameterValue>,
}

/// Which capability decides a check.
#[derive(Clone, Copy)]
enum CheckKind {
    /// `property-required`.
    Required,
    /// `property-data-type`.
    DataType,
    /// `property-value`.
    Value,
    /// `attribute-value`.
    Attribute,
    /// `predefined-type`.
    PredefinedType,
    /// `classification`.
    Classification,
    /// `material`.
    Material,
    /// `part-of`.
    PartOf,
    /// `entity`.
    Entity,
}

const VALUE_PARAMETERS: &[(&str, ParameterKind, bool)] = &[
    ("data_type", ParameterKind::String, false),
    ("values", ParameterKind::StringList, false),
    ("patterns", ParameterKind::StringList, false),
    ("min_inclusive", ParameterKind::String, false),
    ("max_inclusive", ParameterKind::String, false),
    ("min_exclusive", ParameterKind::String, false),
    ("max_exclusive", ParameterKind::String, false),
    ("length", ParameterKind::Integer, false),
    ("min_length", ParameterKind::Integer, false),
    ("max_length", ParameterKind::Integer, false),
    ("optional", ParameterKind::Boolean, false),
    ("prohibited", ParameterKind::Boolean, false),
];

impl CheckKind {
    /// Definition id suffix, name, description and capability.
    fn catalog(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            CheckKind::Required => (
                "property-required",
                "Property is required",
                "An IDS property facet without a value: the property must exist with a non-empty value.",
                PROPERTY_REQUIRED,
            ),
            CheckKind::DataType => (
                "property-data-type",
                "Property is required with a data type",
                "An IDS property facet with a dataType and no value: the property must exist with a non-empty value of that declared type.",
                PROPERTY_DATA_TYPE,
            ),
            CheckKind::Entity => (
                "entity",
                "Entity",
                "An IDS entity requirement: the object's class (and predefined type) given as literals or patterns.",
                ENTITY,
            ),
            CheckKind::PartOf => (
                "part-of",
                "Part of",
                "An IDS partOf requirement: a whole of a class (and predefined type) through a relation.",
                PART_OF,
            ),
            CheckKind::Material => (
                "material",
                "Material",
                "An IDS material requirement: a material, known by a name or category given as a literal or patterns.",
                MATERIAL,
            ),
            CheckKind::Classification => (
                "classification",
                "Classification",
                "An IDS classification requirement: codes (with their ancestors) and systems as literals or patterns.",
                CLASSIFICATION,
            ),
            CheckKind::PredefinedType => (
                "predefined-type",
                "Predefined type",
                "An IDS entity requirement with a predefinedType on the applicability's own class.",
                PREDEFINED_TYPE,
            ),
            CheckKind::Attribute => (
                "attribute-value",
                "Attribute meets constraints",
                "An IDS attribute facet: the attribute must hold a value, and meet the value constraints if any.",
                ATTRIBUTE_VALUE,
            ),
            CheckKind::Value => (
                "property-value",
                "Property value meets constraints",
                "An IDS property facet with a value, or an optional one with a dataType: literals and XML Schema facets cast to the property's value.",
                PROPERTY_VALUE,
            ),
        }
    }

    /// The reference parameter naming what is checked, if any.
    fn reference(self) -> Option<&'static str> {
        match self {
            CheckKind::Attribute => Some("attribute"),
            CheckKind::Required | CheckKind::DataType | CheckKind::Value => Some("property"),
            CheckKind::PredefinedType
            | CheckKind::Classification
            | CheckKind::Material
            | CheckKind::PartOf
            | CheckKind::Entity => None,
        }
    }

    /// The other parameters: name, kind, and whether required.
    fn parameters(self) -> &'static [(&'static str, ParameterKind, bool)] {
        match self {
            CheckKind::Required => &[],
            CheckKind::DataType => &[("data_type", ParameterKind::String, true)],
            CheckKind::Value | CheckKind::Attribute => VALUE_PARAMETERS,
            CheckKind::PredefinedType => &[
                ("values", ParameterKind::StringList, false),
                ("patterns", ParameterKind::StringList, false),
                ("user_defined", ParameterKind::Boolean, false),
            ],
            CheckKind::Entity => &[
                ("classes", ParameterKind::StringList, false),
                ("class_patterns", ParameterKind::StringList, false),
                ("predefined_types", ParameterKind::StringList, false),
                ("predefined_patterns", ParameterKind::StringList, false),
            ],
            CheckKind::PartOf => &[
                ("relation", ParameterKind::String, false),
                ("classes", ParameterKind::StringList, false),
                ("class_patterns", ParameterKind::StringList, false),
                ("predefined_types", ParameterKind::StringList, false),
                ("predefined_patterns", ParameterKind::StringList, false),
                ("prohibited", ParameterKind::Boolean, false),
            ],
            CheckKind::Material => &[
                ("values", ParameterKind::StringList, false),
                ("patterns", ParameterKind::StringList, false),
                ("optional", ParameterKind::Boolean, false),
                ("prohibited", ParameterKind::Boolean, false),
            ],
            CheckKind::Classification => &[
                ("codes", ParameterKind::StringList, false),
                ("code_patterns", ParameterKind::StringList, false),
                ("systems", ParameterKind::StringList, false),
                ("system_patterns", ParameterKind::StringList, false),
                ("optional", ParameterKind::Boolean, false),
                ("prohibited", ParameterKind::Boolean, false),
            ],
        }
    }
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
    releases: &[IfcVersion],
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
            Facet::Entity(entity) => entity_names(entity)
                .and_then(|names| occurrences(names, releases))
                .map(|names| selected = Some((names, entity))),
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

/// Refuses a class whose instances are not checked objects.
///
/// An IFC session makes a project object of every `IfcObject` occurrence and
/// of nothing else. A rule over a type object, an IFC4 `IfcProject` (an
/// `IfcContext`) or a resource would select nothing and pass silently, so
/// such a class, and one the release does not define, is a gap.
fn occurrences(names: Vec<String>, releases: &[IfcVersion]) -> Result<Vec<String>, Reason> {
    for release in releases {
        let schema = match release {
            IfcVersion::Ifc2x3 => ifc_schema::ifc2x3(),
            IfcVersion::Ifc4 => ifc_schema::ifc4(),
            // No type system either; reported as a release gap.
            IfcVersion::Ifc4x3Add2 => continue,
        };
        for name in &names {
            if schema.entity(name).is_none() {
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
        Facet::Attribute(attribute) => attribute_check(attribute, requirement.occurrence),
        Facet::Classification(classification) => {
            classification_check(classification, requirement.occurrence)
        }
        Facet::Material(material) => material_check(material, requirement.occurrence),
        Facet::PartOf(part_of) => part_of_check(part_of, requirement.occurrence),
        Facet::Entity(entity) => {
            // Requiring the class the applicability already selected always
            // holds; a predefined type on it is checked on its own.
            let same = applicability.is_some_and(|applicable| {
                applicable.predefined_type.is_none()
                    && matches!((&applicable.name, &entity.name), (Value::Simple(a), Value::Simple(b)) if a == b)
            });
            if !same {
                return entity_check(entity).map(Some);
            }
            let Some(designation) = &entity.predefined_type else {
                return Ok(None);
            };
            let Value::Simple(class) = &entity.name else {
                return Err(Reason::EntityRequirement);
            };
            Ok(Some(Check {
                set: None,
                name: class.clone(),
                kind: CheckKind::PredefinedType,
                parameters: predefined_type_parameters(designation)?,
            }))
        }
    }
}

fn property_check(property: &Property, occurrence: Occurrence) -> Result<Option<Check>, Reason> {
    let (Value::Simple(set), Value::Simple(name)) = (&property.property_set, &property.base_name)
    else {
        return Err(Reason::Restriction);
    };
    let (optional, prohibited) = match occurrence {
        Occurrence::Required => (false, false),
        Occurrence::Optional => (true, false),
        Occurrence::Prohibited => (false, true),
    };
    let mut parameters = BTreeMap::new();
    if let Some(data_type) = &property.data_type {
        parameters.insert(
            "data_type".to_owned(),
            ParameterValue::String {
                value: data_type.clone(),
            },
        );
    }
    let kind = match (&property.value, optional, &property.data_type) {
        // A prohibited facet is the required one inverted.
        (None, false, _) if prohibited => CheckKind::Value,
        (None, false, None) => CheckKind::Required,
        (None, false, Some(_)) => CheckKind::DataType,
        // Without a value or type, an optional property is satisfied
        // whether or not it is there.
        (None, true, None) => return Ok(None),
        (None, true, Some(_)) => CheckKind::Value,
        (Some(value), ..) => {
            value_parameters(value, &mut parameters)?;
            CheckKind::Value
        }
    };
    occurrence_flag(occurrence, &mut parameters);
    Ok(Some(Check {
        set: Some(set.clone()),
        name: name.clone(),
        kind,
        parameters,
    }))
}

fn attribute_check(attribute: &Attribute, occurrence: Occurrence) -> Result<Option<Check>, Reason> {
    let Value::Simple(name) = &attribute.name else {
        return Err(Reason::Restriction);
    };
    let optional = occurrence == Occurrence::Optional;
    let mut parameters = BTreeMap::new();
    match &attribute.value {
        Some(value) => value_parameters(value, &mut parameters)?,
        // An optional attribute with no value passes whether or not it is set.
        None if optional => return Ok(None),
        None => {}
    }
    occurrence_flag(occurrence, &mut parameters);
    Ok(Some(Check {
        set: None,
        name: name.clone(),
        kind: CheckKind::Attribute,
        parameters,
    }))
}

/// The `predefined-type` parameters an IDS predefined type stands for.
///
/// A literal `USERDEFINED` asks whether the type is user-defined at all, as
/// IDS reads it; any other literal, an enumeration and patterns name the
/// designation itself.
fn predefined_type_parameters(value: &Value) -> Result<BTreeMap<String, ParameterValue>, Reason> {
    if value.as_simple() == Some("USERDEFINED") {
        return Ok(BTreeMap::from([(
            "user_defined".to_owned(),
            ParameterValue::Boolean { value: true },
        )]));
    }
    let mut parameters = BTreeMap::new();
    name_parameters(value, "values", "patterns", &mut parameters)?;
    Ok(parameters)
}

/// A name given as a literal, an enumeration or patterns, as the literal and
/// pattern lists a capability takes. Any other restriction facet is a gap.
fn name_parameters(
    value: &Value,
    literals: &str,
    patterns: &str,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<(), Reason> {
    let strings = |values: &[String]| ParameterValue::StringList {
        value: values.to_vec(),
    };
    match value {
        Value::Simple(literal) => {
            parameters.insert(literals.to_owned(), strings(std::slice::from_ref(literal)));
            Ok(())
        }
        Value::Restriction(restriction) => {
            let only_names = restriction.min_inclusive.is_none()
                && restriction.max_inclusive.is_none()
                && restriction.min_exclusive.is_none()
                && restriction.max_exclusive.is_none()
                && restriction.length.is_none()
                && restriction.min_length.is_none()
                && restriction.max_length.is_none()
                && restriction.total_digits.is_none()
                && restriction.fraction_digits.is_none()
                && !(restriction.enumeration.is_empty() && restriction.patterns.is_empty());
            if !only_names {
                return Err(Reason::Restriction);
            }
            if !restriction.enumeration.is_empty() {
                parameters.insert(literals.to_owned(), strings(&restriction.enumeration));
            }
            if !restriction.patterns.is_empty() {
                parameters.insert(patterns.to_owned(), strings(&restriction.patterns));
            }
            Ok(())
        }
    }
}

/// An entity requirement other than the applicability's own class.
fn entity_check(entity: &Entity) -> Result<Check, Reason> {
    let mut parameters = BTreeMap::new();
    name_parameters(&entity.name, "classes", "class_patterns", &mut parameters)?;
    if let Some(predefined) = &entity.predefined_type {
        name_parameters(
            predefined,
            "predefined_types",
            "predefined_patterns",
            &mut parameters,
        )?;
    }
    Ok(Check {
        set: None,
        name: entity
            .name
            .as_simple()
            .unwrap_or("a required class")
            .to_owned(),
        kind: CheckKind::Entity,
        parameters,
    })
}

fn part_of_check(part_of: &PartOf, occurrence: Occurrence) -> Result<Option<Check>, Reason> {
    let relation = match part_of.relation {
        None => "any",
        Some(Relation::Aggregates) => "aggregation",
        Some(Relation::AssignsToGroup) => "grouping",
        Some(Relation::ContainedInSpatialStructure) => "containment",
        Some(Relation::Nests) => "nesting",
        Some(Relation::VoidsElementFillsElement) => "voiding",
    };
    let mut parameters = BTreeMap::from([(
        "relation".to_owned(),
        ParameterValue::String {
            value: relation.to_owned(),
        },
    )]);
    name_parameters(
        &part_of.entity.name,
        "classes",
        "class_patterns",
        &mut parameters,
    )?;
    if let Some(predefined) = &part_of.entity.predefined_type {
        name_parameters(
            predefined,
            "predefined_types",
            "predefined_patterns",
            &mut parameters,
        )?;
    }
    occurrence_flag(occurrence, &mut parameters);
    Ok(Some(Check {
        set: None,
        name: part_of
            .entity
            .name
            .as_simple()
            .unwrap_or("a whole")
            .to_owned(),
        kind: CheckKind::PartOf,
        parameters,
    }))
}

fn material_check(material: &Material, occurrence: Occurrence) -> Result<Option<Check>, Reason> {
    let mut parameters = BTreeMap::new();
    if let Some(value) = &material.value {
        name_parameters(value, "values", "patterns", &mut parameters)?;
    }
    occurrence_flag(occurrence, &mut parameters);
    let name = material
        .value
        .as_ref()
        .and_then(Value::as_simple)
        .unwrap_or("requirement")
        .to_owned();
    Ok(Some(Check {
        set: None,
        name,
        kind: CheckKind::Material,
        parameters,
    }))
}

/// `optional` or `prohibited` for a facet's occurrence; required sets none.
fn occurrence_flag(occurrence: Occurrence, parameters: &mut BTreeMap<String, ParameterValue>) {
    let flag = match occurrence {
        Occurrence::Required => return,
        Occurrence::Optional => "optional",
        Occurrence::Prohibited => "prohibited",
    };
    parameters.insert(flag.to_owned(), ParameterValue::Boolean { value: true });
}

fn classification_check(
    classification: &Classification,
    occurrence: Occurrence,
) -> Result<Option<Check>, Reason> {
    let mut parameters = BTreeMap::new();
    name_parameters(
        &classification.system,
        "systems",
        "system_patterns",
        &mut parameters,
    )?;
    if let Some(value) = &classification.value {
        name_parameters(value, "codes", "code_patterns", &mut parameters)?;
    }
    occurrence_flag(occurrence, &mut parameters);
    let name = match (&classification.system, &classification.value) {
        (Value::Simple(system), Some(Value::Simple(code))) => format!("{system} {code}"),
        (Value::Simple(system), _) => system.clone(),
        _ => "requirement".to_owned(),
    };
    Ok(Some(Check {
        set: None,
        name,
        kind: CheckKind::Classification,
        parameters,
    }))
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
    if restriction.total_digits.is_some() {
        return Err(Reason::RestrictionFacet("totalDigits"));
    }
    if restriction.fraction_digits.is_some() {
        return Err(Reason::RestrictionFacet("fractionDigits"));
    }
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
            add(
                name,
                ParameterValue::String {
                    value: bound.clone(),
                },
            );
        }
    }
    for (name, count) in [
        ("length", restriction.length),
        ("min_length", restriction.min_length),
        ("max_length", restriction.max_length),
    ] {
        if let Some(count) = count {
            // A length beyond `i64` cannot be met by any value; `MAX` keeps
            // that meaning for `max_length` and fails every `length`.
            let value = i64::try_from(count).unwrap_or(i64::MAX);
            add(name, ParameterValue::Integer { value });
        }
    }
    if parameters.len() == before {
        return Err(Reason::EmptyRestriction);
    }
    Ok(())
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
