//! Rule packages written back as IDS 1.0 documents.
//!
//! Only rules IDS states exactly are exported: a rule becomes a
//! specification when translating that specification gives the rule again,
//! with the same capability, parameters, applicability and gates, concept
//! for concept. Every other rule is listed as [`NotExported`] with the
//! [`Refusal`] that explains it; nothing is approximated and nothing is
//! dropped silently.
//!
//! A specification comes from one of two places:
//!
//! - **Its origin.** A folder [`translate()`](crate::translate) wrote keeps the
//!   specification it came from in its [`SPECIFICATION_ANNOTATION`]. It is
//!   exported as it stands, names, instructions, identifiers, cardinalities
//!   and releases included, when translating it again gives exactly the
//!   folder's rules; a folder whose rules were edited, or translated with a
//!   prefilter or with gaps, is not.
//! - **One rule.** Any other rule is read as one specification: its
//!   applicability as an entity facet and the facets its selector states,
//!   and its capability as one requirement (`property-required`,
//!   `property-data-type`, `property-value`, `property-requirements`,
//!   `classification`, `selector-conformance` over an entity, attribute,
//!   material or part-of selector), or `object-count` as the
//!   applicability's cardinality. That reading is only a candidate: it is
//!   exported only when its translation is the rule.

use std::collections::BTreeMap;
use std::fmt;

use axioval_engine::RuleCapability;
use axioval_export::compare::{self, Comparison};
use axioval_export::precheck::{self, PreCheck};
use axioval_export::{ExportOutcome, ExportProfile, Loss};
use axioval_ir::contract::{
    ComparisonOperator, DefinitionPackage, ExternalName, ParameterValue, Quantifier,
    RelatedQuantifier, RuleFolder, RuleInstance, RuleSetPackage, Selector, Severity,
};
use axioval_ir::{ATTRIBUTE_SET, MATERIAL_KIND, MATERIAL_NAMES, MATERIAL_SET, TYPE_ATTRIBUTE_SET};
use openbim_ids::{
    Applicability, Attribute, Classification, Entity, Facet, Ids, IfcVersion, Info, Material,
    Occurrence, PartOf, Property, Relation, Requirement, Requirements, Restriction, Specification,
    Value, WriteError,
};
use regex::Regex;
use thiserror::Error;

use crate::{
    AuditFinding, INFO_ANNOTATION, Invalid, Options, SPECIFICATION_ANNOTATION,
    UNWRITABLE_ANNOTATION, translate_parts, write,
};

/// A ruleset written as IDS: the specifications it states exactly, and the
/// rules it could not state.
#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    /// The document's `<info>`: the one the ruleset was translated from, or
    /// the ruleset's name, version, description and first e-mail author.
    pub info: Info,
    /// The exported specifications, in ruleset order.
    pub specifications: Vec<ExportedSpecification>,
    /// Every rule not exported, in ruleset order, with why.
    pub not_exported: Vec<NotExported>,
}

impl Export {
    /// Whether every rule was exported.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.not_exported.is_empty()
    }

    /// The IDS 1.0 document, `None` when no specification was exported: IDS
    /// requires at least one.
    ///
    /// Every specification [`export()`] exports is one IDS 1.0 writes and
    /// the [`audit()`](crate::audit) accepts, so only the `<info>` should
    /// make this fail: an author that is no e-mail address or a date that
    /// is no `xs:date`, as a root folder's [`INFO_ANNOTATION`] may hold
    /// them. The document is audited again before it is written; an error
    /// then is an exporter bug, and nothing is written.
    ///
    /// # Errors
    ///
    /// [`DocumentError::Write`], the `openbim-ids` writer's error with the
    /// location of the part it refused, or [`DocumentError::Invalid`],
    /// every finding of an audit that found an error.
    pub fn to_xml(&self) -> Result<Option<String>, DocumentError> {
        if self.specifications.is_empty() {
            return Ok(None);
        }
        let specifications: Vec<&Specification> = self
            .specifications
            .iter()
            .map(|exported| &exported.specification)
            .collect();
        write::document(&self.info, &specifications).map(Some)
    }
}

/// Why [`Export::to_xml`] wrote no document.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DocumentError {
    /// The IDS 1.0 writer refused part of the document.
    #[error(transparent)]
    Write(#[from] WriteError),
    /// The audit found an error: the document cannot work as written.
    #[error(transparent)]
    Invalid(#[from] Invalid),
}

/// One exported specification and the rules it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedSpecification {
    /// The specification.
    pub specification: Specification,
    /// The ids of the rules it states, auxiliary rules included.
    pub rules: Vec<String>,
}

/// A rule the export left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotExported {
    /// The rule's id.
    pub rule: String,
    /// Why IDS cannot state it.
    pub reason: Refusal,
}

impl fmt::Display for NotExported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rule, self.reason)
    }
}

/// Why a rule is not exported.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The rule is disabled; an IDS specification always applies.
    Disabled,
    /// An auxiliary rule outside the specification it was translated for.
    Auxiliary,
    /// The rule, or a folder around it, is gated on another rule.
    Gated,
    /// Severity bands, overrides or categories shape what the rule reports.
    Graded,
    /// A severity other than error; an IDS requirement is always one.
    Severity(String),
    /// Named target groups instead of one population.
    Groups,
    /// The rule's definition is in none of the definition packages.
    UnknownDefinition(String),
    /// A capability no IDS facet states.
    Capability(String),
    /// A selector no IDS facet states.
    Selector(String),
    /// A parameter no IDS facet states.
    Parameter(String),
    /// A concept not named as IDS names things.
    Concept(String),
    /// The nearest specification has no exact translation.
    Gaps(String),
    /// The nearest specification translates to different rules.
    Differs(String),
    /// A rule of a folder translated from an IDS specification that cannot
    /// be written back.
    Origin {
        /// The specification's name.
        specification: String,
        /// Why not.
        why: String,
    },
    /// The specification the rule reads as is one the IDS 1.0 writer
    /// refuses.
    Unwritable {
        /// The specification's name.
        specification: String,
        /// The refused part, as a path within the specification such as
        /// `applicability/facets[1]`; empty for the specification itself.
        location: String,
        /// What the writer refused.
        why: String,
    },
    /// The specification the rule reads as, or was translated from, is one
    /// the [`audit()`](crate::audit) refuses: it cannot work as written for
    /// a release it lists.
    Invalid {
        /// The specification's name.
        specification: String,
        /// Every finding of the audit, at least one an error.
        findings: Vec<AuditFinding>,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Disabled => f.write_str("the rule is disabled; an IDS specification always applies"),
            Refusal::Auxiliary => f.write_str(
                "an auxiliary rule reports nothing itself; it is exported only with the IDS specification it was translated for",
            ),
            Refusal::Gated => f.write_str(
                "the rule runs only as another rule's outcome allows, which IDS cannot state",
            ),
            Refusal::Graded => f.write_str(
                "severity bands, severity overrides or categories shape what the rule reports, which IDS cannot state",
            ),
            Refusal::Severity(severity) => write!(
                f,
                "severity {severity}; an IDS requirement is always an error"
            ),
            Refusal::Groups => f.write_str(
                "the rule applies to named target groups; an IDS specification applies to one population",
            ),
            Refusal::UnknownDefinition(definition) => {
                write!(f, "definition {definition} is in no definition package")
            }
            Refusal::Capability(capability) => {
                write!(f, "capability {capability} has no IDS facet")
            }
            Refusal::Selector(why)
            | Refusal::Parameter(why)
            | Refusal::Concept(why) => f.write_str(why),
            Refusal::Gaps(gaps) => write!(
                f,
                "the IDS specification it reads as has no exact translation: {gaps}"
            ),
            Refusal::Differs(path) => write!(
                f,
                "the IDS specification it reads as checks something else: its translation differs at {path}"
            ),
            Refusal::Origin { specification, why } => write!(
                f,
                "IDS specification {specification:?}, which the rule was translated from, cannot be written back: {why}"
            ),
            Refusal::Unwritable {
                specification,
                location,
                why,
            } if location.is_empty() => write!(
                f,
                "IDS 1.0 cannot write the specification {specification:?} it reads as: {why}"
            ),
            Refusal::Unwritable {
                specification,
                location,
                why,
            } => write!(
                f,
                "IDS 1.0 cannot write the specification {specification:?} it reads as, at {location}: {why}"
            ),
            Refusal::Invalid {
                specification,
                findings,
            } => {
                let errors: Vec<String> = findings
                    .iter()
                    .filter(|finding| finding.severity() == crate::AuditSeverity::Error)
                    .map(shown_finding)
                    .collect();
                write!(
                    f,
                    "the IDS specification {specification:?} it reads as cannot work as written: {}",
                    errors.join("; ")
                )
            }
        }
    }
}

impl From<PreCheck> for Refusal {
    fn from(reason: PreCheck) -> Self {
        match reason {
            PreCheck::Disabled => Refusal::Disabled,
            PreCheck::Auxiliary => Refusal::Auxiliary,
            PreCheck::Gated => Refusal::Gated,
            PreCheck::Graded => Refusal::Graded,
            PreCheck::Severity(severity) => Refusal::Severity(severity),
            PreCheck::Groups => Refusal::Groups,
        }
    }
}

/// The type systems of the releases IDS names, in which every concept a
/// translation writes is named.
const TYPE_SYSTEMS: [&str; 3] = [
    crate::IFC2X3_TYPE_SYSTEM,
    crate::IFC4_TYPE_SYSTEM,
    crate::IFC4X3_TYPE_SYSTEM,
];

/// Exports `ruleset`, whose definitions are among `definitions`, as IDS.
///
/// Only rules IDS states exactly are exported: a rule becomes a
/// specification when translating that specification gives the rule
/// again, with the same capability, parameters, applicability and gates,
/// concept for concept.
///
/// - A folder [`translate()`](crate::translate) wrote is exported as the
///   specification its [`SPECIFICATION_ANNOTATION`] keeps (names,
///   instructions, identifiers, cardinalities and releases included), when
///   translating it gives exactly the folder's rules; one whose rules were
///   edited, or that was translated with a prefilter or with gaps, is not.
/// - Any other rule is read as one specification: an entity facet and the
///   facets its selector states, and one property, attribute,
///   classification, material or part-of requirement, or `object-count` as
///   the applicability's cardinality.
///
/// The result lists every rule it left out with its [`Refusal`];
/// [`Export::is_complete`] says whether any was.
#[must_use]
pub fn export(definitions: &[DefinitionPackage], ruleset: &RuleSetPackage) -> Export {
    let catalog = Catalog::new(definitions);
    let mut export = Export {
        info: info(ruleset),
        specifications: Vec::new(),
        not_exported: Vec::new(),
    };
    catalog.folder(&ruleset.root, false, &mut export);
    export
}

/// The IDS export profile, id `ids`: [`export()`] behind the
/// [`ExportProfile`] every export target shares.
///
/// The artifact is the IDS document, `None` when no rule is exported; each
/// [`NotExported`] rule becomes a refused [`Loss`] at its id, with its
/// [`Refusal`] as the reason. IDS degrades nothing: a rule it holds, it
/// holds exactly, and anything else is refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IdsProfile;

impl IdsProfile {
    /// The profile's id.
    pub const ID: &'static str = "ids";
}

impl ExportProfile for IdsProfile {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn format(&self) -> &'static str {
        "IDS"
    }

    fn export(&self, definitions: &[DefinitionPackage], ruleset: &RuleSetPackage) -> ExportOutcome {
        export(definitions, ruleset).into()
    }
}

impl From<Export> for ExportOutcome {
    /// The document and its losses; a document IDS 1.0 cannot write or the
    /// audit refuses (see [`Export::to_xml`]) is no artifact, and every
    /// rule is refused with the location the writer names, or the audit's
    /// errors.
    fn from(export: Export) -> Self {
        let artifact = match export.to_xml() {
            Ok(artifact) => artifact,
            Err(error) => {
                let reason = match error {
                    DocumentError::Write(error) => {
                        format!("IDS 1.0 cannot write the document: {error}")
                    }
                    DocumentError::Invalid(invalid) => {
                        format!("the exported IDS document is refused, never written: {invalid}")
                    }
                };
                return ExportOutcome {
                    artifact: None,
                    exported: Vec::new(),
                    losses: export
                        .specifications
                        .iter()
                        .flat_map(|specification| &specification.rules)
                        .map(|rule| Loss::refused(rule, reason.clone()))
                        .chain(
                            export
                                .not_exported
                                .iter()
                                .map(|entry| Loss::refused(&entry.rule, entry.reason.to_string())),
                        )
                        .collect(),
                    contents: Some("0 specification(s)".to_owned()),
                };
            }
        };
        ExportOutcome {
            artifact: artifact.map(String::into_bytes),
            exported: export
                .specifications
                .iter()
                .flat_map(|specification| specification.rules.iter().cloned())
                .collect(),
            losses: export
                .not_exported
                .iter()
                .map(|entry| Loss::refused(&entry.rule, entry.reason.to_string()))
                .collect(),
            contents: Some(format!("{} specification(s)", export.specifications.len())),
        }
    }
}

/// Whether the [`audit()`](crate::audit) accepts `specification`, alone in
/// a document.
fn audited(specification: &Specification) -> Result<(), Refusal> {
    let mut ids = Ids::new(Info::new(specification.name.clone()));
    ids.specifications.push(specification.clone());
    crate::audit(&ids)
        .map(drop)
        .map_err(|invalid| Refusal::Invalid {
            specification: specification.name.clone(),
            findings: invalid.findings,
        })
}

/// An audit finding of a one-specification document: its code, where in
/// the specification, for which release, and why.
fn shown_finding(finding: &AuditFinding) -> String {
    let at = if finding.path.is_empty() {
        String::new()
    } else {
        format!(" at {}", finding.path)
    };
    let release = finding
        .ifc_version
        .map_or_else(String::new, |version| format!(" ({version})"));
    format!("{}{at}{release}: {}", finding.code, finding.message)
}

/// Whether the IDS 1.0 writer writes `specification`.
fn writable(specification: &Specification) -> Result<(), Refusal> {
    write::specification(specification)
        .map(drop)
        .map_err(|unwritable| Refusal::Unwritable {
            specification: specification.name.clone(),
            location: unwritable.location,
            why: unwritable.why,
        })
}

/// A facet's place in the applicability sequence `ids.xsd` declares:
/// entity, partOf, classification, attribute, property, material.
fn sequence(facet: &Facet) -> u8 {
    match facet {
        Facet::Entity(_) => 0,
        Facet::PartOf(_) => 1,
        Facet::Classification(_) => 2,
        Facet::Attribute(_) => 3,
        Facet::Property(_) => 4,
        Facet::Material(_) => 5,
    }
}

/// The `<info>` the root folder keeps, or one made from the package.
fn info(ruleset: &RuleSetPackage) -> Info {
    let annotations = &ruleset.root.annotations;
    let field = |name: &str| {
        annotations
            .get(&format!("{INFO_ANNOTATION}{name}"))
            .cloned()
    };
    if let Some(title) = field("title") {
        return Info {
            title,
            copyright: field("copyright"),
            version: field("version"),
            description: field("description"),
            author: field("author"),
            date: field("date"),
            purpose: field("purpose"),
            milestone: field("milestone"),
        };
    }
    let package = &ruleset.package;
    // `ids.xsd` requires the author to look like an e-mail address.
    let email = Regex::new(r"\A[^@]+@[^\.]+\..+\z").expect("valid pattern");
    Info {
        title: package.name.default.clone(),
        version: Some(package.version.clone()),
        description: package
            .description
            .as_ref()
            .map(|text| text.default.clone()),
        author: package
            .authors
            .iter()
            .find(|author| email.is_match(author))
            .cloned(),
        ..Info::default()
    }
}

/// The definitions and concepts of every definition package.
struct Catalog<'p> {
    shared: compare::Catalog<'p>,
}

/// How IDS compares rules: class names ignore case, as IFC names classes,
/// and a conformance rule's message says what a finding reads, not what it
/// finds.
fn comparison() -> Comparison {
    Comparison::default()
        .presentation_parameter(axioval_rules::SelectorConformance.id(), "message")
        .case_insensitive_object_types()
}

impl<'p> Catalog<'p> {
    fn new(packages: &'p [DefinitionPackage]) -> Self {
        Self {
            shared: compare::Catalog::new(packages),
        }
    }

    /// Exports `folder` and its subfolders; `gated` when a folder around it
    /// has a gate.
    fn folder(&self, folder: &RuleFolder, gated: bool, export: &mut Export) {
        let gated = precheck::is_folder_gated(folder, gated);
        if let Some(unwritable) = folder.annotations.get(UNWRITABLE_ANNOTATION) {
            // `<location>: <why>`; a location holds no space.
            let (location, why) = unwritable
                .split_once(": ")
                .unwrap_or(("", unwritable.as_str()));
            let reason = Refusal::Unwritable {
                specification: folder.name.default.clone(),
                location: location.to_owned(),
                why: why.to_owned(),
            };
            export
                .not_exported
                .extend(folder.rules.iter().map(|rule| NotExported {
                    rule: rule.id.clone(),
                    reason: reason.clone(),
                }));
        } else if let Some(fragment) = folder.annotations.get(SPECIFICATION_ANNOTATION) {
            match self.origin(folder, fragment, gated) {
                Ok(specification) => export.specifications.push(ExportedSpecification {
                    specification,
                    rules: folder.rules.iter().map(|rule| rule.id.clone()).collect(),
                }),
                Err(refusal) => {
                    export
                        .not_exported
                        .extend(folder.rules.iter().map(|rule| NotExported {
                            rule: rule.id.clone(),
                            reason: refusal.clone(),
                        }));
                }
            }
        } else {
            for rule in &folder.rules {
                match self.rule(rule, gated) {
                    Ok(specification) => export.specifications.push(ExportedSpecification {
                        specification,
                        rules: vec![rule.id.clone()],
                    }),
                    Err(reason) => export.not_exported.push(NotExported {
                        rule: rule.id.clone(),
                        reason,
                    }),
                }
            }
        }
        for child in &folder.folders {
            self.folder(child, gated, export);
        }
    }

    /// The specification a translated folder keeps, when its rules are
    /// still exactly its translation.
    fn origin(
        &self,
        folder: &RuleFolder,
        fragment: &str,
        gated: bool,
    ) -> Result<Specification, Refusal> {
        let refused = |specification: &str, why: String| Refusal::Origin {
            specification: specification.to_owned(),
            why,
        };
        let mut specification = write::read_specification(fragment).map_err(|why| {
            refused(
                &folder.name.default,
                format!("its {SPECIFICATION_ANNOTATION} annotation does not read: {why}"),
            )
        })?;
        // The folder's own name and description, should they have changed.
        specification.name.clone_from(&folder.name.default);
        specification.description = folder.description.as_ref().map(|text| text.default.clone());
        let name = specification.name.clone();
        if gated {
            return Err(refused(&name, Refusal::Gated.to_string()));
        }
        let rules: Vec<&RuleInstance> = folder.rules.iter().collect();
        self.matches(&specification, &rules).map_err(|refusal| {
            let why = match refusal {
                Refusal::Gaps(gaps) => format!("it does not translate exactly: {gaps}"),
                Refusal::Differs(path) => format!(
                    "the folder's rules are no longer its translation (edited, or translated with a prefilter): they differ at {path}"
                ),
                other => other.to_string(),
            };
            refused(&name, why)
        })?;
        writable(&specification)?;
        audited(&specification)?;
        Ok(specification)
    }

    /// The specification one rule reads as, when its translation is the
    /// rule.
    fn rule(&self, rule: &RuleInstance, gated: bool) -> Result<Specification, Refusal> {
        let selector = precheck::pre_check(rule, gated, &Severity::Error)?;
        let definition = self
            .shared
            .definitions
            .get(rule.definition_id.as_str())
            .ok_or_else(|| Refusal::UnknownDefinition(rule.definition_id.clone()))?;
        let reading = self.reading(&definition.capability, &rule.parameters)?;
        let mut facets = self.applicability(selector)?;
        // The order `ids.xsd` requires; the translation is checked on it.
        facets.sort_by_key(sequence);
        let (min_occurs, max_occurs, requirement) = match reading {
            Reading::Requirement(requirement) => (0, None, Some(requirement)),
            Reading::Count { minimum, maximum } => (minimum, maximum, None),
        };
        let specification = Specification {
            name: rule.name.default.clone(),
            ifc_versions: IfcVersion::ALL.to_vec(),
            identifier: Some(rule.id.clone()),
            description: rule.description.as_ref().map(|text| text.default.clone()),
            instructions: None,
            applicability: Applicability {
                min_occurs,
                max_occurs,
                facets,
            },
            requirements: requirement.map(|requirement| Requirements {
                description: None,
                facets: vec![requirement],
            }),
        };
        self.matches(&specification, &[rule])?;
        writable(&specification)?;
        audited(&specification)?;
        Ok(specification)
    }

    /// Whether translating `specification` alone gives exactly `rules`.
    fn matches(
        &self,
        specification: &Specification,
        rules: &[&RuleInstance],
    ) -> Result<(), Refusal> {
        let options = Options::new("ids:export", "0.0.0");
        let info = Info {
            title: specification.name.clone(),
            ..Info::default()
        };
        let translation =
            translate_parts(&info, &[specification], &options).expect("valid options");
        let gaps: Vec<String> = translation.gaps().map(|(_, gap)| gap.to_string()).collect();
        if !gaps.is_empty() {
            return Err(Refusal::Gaps(gaps.join("; ")));
        }
        let translated = compare::Catalog::new(std::slice::from_ref(&translation.definitions));
        let written: Vec<&RuleInstance> = translation
            .ruleset
            .root
            .folders
            .iter()
            .flat_map(|folder| &folder.rules)
            .collect();
        let difference =
            compare::verdict_difference(&self.shared, rules, &translated, &written, &comparison())
                .map_err(|unknown| Refusal::UnknownDefinition(unknown.0))?;
        match difference {
            None => Ok(()),
            Some(path) => Err(Refusal::Differs(path)),
        }
    }

    /// The one name `names` gives in every release IDS names.
    fn ifc_name(kind: &str, id: &str, names: &[ExternalName]) -> Result<String, Refusal> {
        let mut found: Option<&str> = None;
        for type_system in TYPE_SYSTEMS {
            let spelled: Vec<&str> = names
                .iter()
                .filter(|name| name.type_system == type_system)
                .map(|name| name.name.as_str())
                .collect();
            let [name] = spelled.as_slice() else {
                return Err(Refusal::Concept(format!(
                    "{kind} {id} is not named once in {type_system}; IDS names everything in IFC2X3, IFC4 and IFC4X3_ADD2 alike"
                )));
            };
            match found {
                Some(other) if other != *name => {
                    return Err(Refusal::Concept(format!(
                        "{kind} {id} is named {other} in one release and {name} in another; IDS names it alike in every release"
                    )));
                }
                _ => found = Some(name),
            }
        }
        if let Some(other) = names
            .iter()
            .find(|name| !TYPE_SYSTEMS.contains(&name.type_system.as_str()))
        {
            return Err(Refusal::Concept(format!(
                "{kind} {id} is also named in {}, which IDS does not name",
                other.type_system
            )));
        }
        Ok(found.unwrap_or_default().to_owned())
    }

    /// The IDS class name of an object-type concept, upper case.
    fn class(&self, id: &str) -> Result<String, Refusal> {
        let concept = self.shared.object_types.get(id).ok_or_else(|| {
            Refusal::Concept(format!("object type {id} is in no definition package"))
        })?;
        Ok(Self::ifc_name("object type", id, &concept.external_names)?.to_ascii_uppercase())
    }

    /// The name of a property concept: a property, or an attribute in a
    /// reserved set.
    fn property(&self, id: &str) -> Result<String, Refusal> {
        let concept = self.shared.properties.get(id).ok_or_else(|| {
            Refusal::Concept(format!("property {id} is in no definition package"))
        })?;
        Self::ifc_name("property", id, &concept.external_names)
    }

    fn property_set(&self, id: &str) -> Result<String, Refusal> {
        let concept = self.shared.property_sets.get(id).ok_or_else(|| {
            Refusal::Concept(format!("property set {id} is in no definition package"))
        })?;
        Self::ifc_name("property set", id, &concept.external_names)
    }

    /// Whether `selector` tests the attribute `name` in the reserved `set`.
    fn is_slot(&self, selector: &Selector, set: &str, name: &str) -> bool {
        matches!(
            selector,
            Selector::Property { property_set: Some(s), property, .. }
                if s == set && self.property(property).is_ok_and(|found| found == name)
        )
    }

    /// The applicability facets `selector` states: an entity first, then
    /// one facet per operand.
    fn applicability(&self, selector: &Selector) -> Result<Vec<Facet>, Refusal> {
        let operands: Vec<&Selector> = match selector {
            Selector::AllOf { operands } if self.entity(selector).is_err() => {
                operands.iter().collect()
            }
            other => vec![other],
        };
        let Some((first, rest)) = operands.split_first() else {
            return Err(Refusal::Selector(
                "the rule selects nothing an IDS applicability states".to_owned(),
            ));
        };
        let entity = self.entity(first).map_err(|refusal| match refusal {
            Refusal::Selector(why) => Refusal::Selector(format!(
                "an IDS applicability selects an entity first, and the rule's does not: {why}"
            )),
            other => other,
        })?;
        let mut facets = vec![Facet::Entity(entity)];
        for operand in rest {
            facets.push(self.facet(operand)?);
        }
        Ok(facets)
    }

    /// The facet a selector states, other than as an applicability's
    /// operands.
    fn facet(&self, selector: &Selector) -> Result<Facet, Refusal> {
        if let Ok(entity) = self.entity(selector) {
            return Ok(Facet::Entity(entity));
        }
        if let Selector::Related { .. } = selector {
            return self.part_of(selector).map(Facet::PartOf);
        }
        if is_classification(selector) {
            return Ok(Facet::Classification(classification(selector)?));
        }
        if mentions(selector, MATERIAL_SET) {
            return self.material(selector).map(Facet::Material);
        }
        if mentions(selector, ATTRIBUTE_SET) {
            return self.attribute(selector).map(Facet::Attribute);
        }
        Err(Refusal::Selector(format!(
            "no IDS facet states the selector {}",
            shown(selector)
        )))
    }

    /// An entity facet: exact classes, and a predefined type as IDS
    /// resolves it.
    fn entity(&self, selector: &Selector) -> Result<Entity, Refusal> {
        let (classes, predefined_type) = match selector {
            Selector::AllOf { operands }
                if operands.len() == 2 && mentions(&operands[1], TYPE_ATTRIBUTE_SET) =>
            {
                (
                    self.classes(&operands[0])?,
                    Some(self.predefined_type(&operands[1])?),
                )
            }
            other => (self.classes(other)?, None),
        };
        Ok(Entity {
            name: names_value(classes),
            predefined_type,
        })
    }

    fn classes(&self, selector: &Selector) -> Result<Vec<String>, Refusal> {
        let mut classes = Vec::new();
        match selector {
            Selector::EntityType {
                object_type,
                include_subtypes,
            } => {
                let class = self.class(object_type)?;
                if *include_subtypes {
                    return Err(Refusal::Selector(format!(
                        "it selects {class} with its subtypes; IDS names classes exactly, never their subtypes"
                    )));
                }
                classes.push(class);
            }
            Selector::AnyOf { operands } => {
                for operand in operands {
                    match operand {
                        Selector::EntityType { .. } => classes.extend(self.classes(operand)?),
                        // A class IFC2X3 spells as a typed occurrence, which
                        // translating the class writes again.
                        Selector::AllOf { operands }
                            if matches!(operands.first(), Some(Selector::Source { .. })) => {}
                        other => {
                            return Err(Refusal::Selector(format!(
                                "{} is not an entity type",
                                shown(other)
                            )));
                        }
                    }
                }
            }
            other => {
                return Err(Refusal::Selector(format!(
                    "{} is not an entity type",
                    shown(other)
                )));
            }
        }
        if classes.is_empty() {
            return Err(Refusal::Selector("it names no class".to_owned()));
        }
        classes.dedup();
        Ok(classes)
    }

    /// The value of a predefined type, read from the selector a translation
    /// writes for it: the occurrence's own type compared with the value, or
    /// the test for a user-defined type.
    fn predefined_type(&self, selector: &Selector) -> Result<Value, Refusal> {
        if let Selector::AnyOf { operands } = selector
            && let Some(Selector::Property {
                operator: ComparisonOperator::Equals,
                value: Some(ParameterValue::String { value }),
                ..
            }) = operands.first()
            && value == "USERDEFINED"
            && self.is_slot(&operands[0], TYPE_ATTRIBUTE_SET, "PredefinedType")
        {
            return Ok(Value::Simple(value.clone()));
        }
        let mut found = None;
        visit(selector, &mut |node| {
            if found.is_some() {
                return;
            }
            if let Selector::AllOf { operands } = node
                && let [exists, Selector::Not { .. }, test] = operands.as_slice()
                && matches!(
                    exists,
                    Selector::Property {
                        operator: ComparisonOperator::Exists,
                        ..
                    }
                )
                && self.is_slot(exists, ATTRIBUTE_SET, "PredefinedType")
            {
                found = Some(test);
            }
        });
        let test = found.ok_or_else(|| {
            Refusal::Selector(
                "it tests type attributes other than as IDS resolves a predefined type".to_owned(),
            )
        })?;
        value_of(test)
    }

    fn part_of(&self, selector: &Selector) -> Result<PartOf, Refusal> {
        let not_part_of = || {
            Refusal::Selector(format!(
                "{} follows no relation an IDS part-of facet names",
                shown(selector)
            ))
        };
        let Selector::Related {
            path,
            quantifier: RelatedQuantifier::Any,
            selector: whole,
        } = selector
        else {
            return Err(not_part_of());
        };
        let [step] = path.as_slice() else {
            return Err(not_part_of());
        };
        let relationships = step.strip_suffix(":backward+").ok_or_else(not_part_of)?;
        let mut relationships: Vec<&str> = relationships.split('|').collect();
        relationships.sort_unstable();
        let relation = match relationships.as_slice() {
            ["IfcRelAggregates"] => Some(Relation::Aggregates),
            ["IfcRelAssignsToGroup"] => Some(Relation::AssignsToGroup),
            ["IfcRelContainedInSpatialStructure"] => Some(Relation::ContainedInSpatialStructure),
            ["IfcRelNests"] => Some(Relation::Nests),
            ["IfcRelFillsElement", "IfcRelVoidsElement"] => {
                Some(Relation::VoidsElementFillsElement)
            }
            [
                "IfcRelAggregates",
                "IfcRelAssignsToGroup",
                "IfcRelContainedInSpatialStructure",
                "IfcRelFillsElement",
                "IfcRelNests",
                "IfcRelVoidsElement",
            ] => None,
            _ => return Err(not_part_of()),
        };
        Ok(PartOf {
            entity: self.entity(whole)?,
            relation,
        })
    }

    fn material(&self, selector: &Selector) -> Result<Material, Refusal> {
        if matches!(
            selector,
            Selector::Property {
                operator: ComparisonOperator::Exists,
                value: None,
                ..
            }
        ) && self.is_slot(selector, MATERIAL_SET, MATERIAL_KIND)
        {
            return Ok(Material { value: None });
        }
        let tests: Vec<&Selector> = match selector {
            Selector::AnyOf { operands } => operands.iter().collect(),
            other => vec![other],
        };
        let mut restriction = restriction("string");
        let mut literal = None;
        for test in &tests {
            let Selector::Property {
                operator,
                value,
                quantifier: Some(Quantifier::Any),
                case_sensitive: true,
                trim: false,
                precision: None,
                ..
            } = test
            else {
                return Err(not_material(selector));
            };
            if !self.is_slot(test, MATERIAL_SET, MATERIAL_NAMES) {
                return Err(not_material(selector));
            }
            match (operator, value) {
                (ComparisonOperator::Equals, Some(ParameterValue::String { value }))
                    if tests.len() == 1 =>
                {
                    literal = Some(value.clone());
                }
                (ComparisonOperator::OneOf, Some(ParameterValue::StringList { value })) => {
                    restriction.enumeration.clone_from(value);
                }
                (ComparisonOperator::Matches, Some(ParameterValue::String { value })) => {
                    restriction.patterns.push(xsd_pattern(value)?);
                }
                _ => return Err(not_material(selector)),
            }
        }
        let value = match literal {
            Some(literal) => Value::Simple(literal),
            None => Value::Restriction(Box::new(restriction)),
        };
        Ok(Material { value: Some(value) })
    }

    /// An attribute facet with one name: the attribute holds a value, and
    /// one that does meets the value.
    fn attribute(&self, selector: &Selector) -> Result<Attribute, Refusal> {
        let not_attribute =
            || Refusal::Selector(format!("{} states no IDS attribute facet", shown(selector)));
        let has_value = |candidate: &Selector| -> Option<String> {
            let Selector::Property {
                property_set: Some(set),
                property,
                operator,
                value,
                ..
            } = candidate
            else {
                return None;
            };
            let holds = match (operator, value) {
                (ComparisonOperator::Exists, None) => true,
                (ComparisonOperator::Matches, Some(ParameterValue::String { value })) => {
                    value == "(?s).+"
                }
                _ => false,
            };
            (holds && set == ATTRIBUTE_SET)
                .then(|| self.property(property).ok())
                .flatten()
        };
        if let Some(name) = has_value(selector) {
            return Ok(Attribute {
                name: Value::Simple(name),
                value: None,
            });
        }
        // Any of several attributes holds a value: a restricted name.
        if let Selector::AnyOf { operands } = selector {
            let names: Option<Vec<String>> = operands.iter().map(has_value).collect();
            let names = names.ok_or_else(not_attribute)?;
            return Ok(Attribute {
                name: names_value(names),
                value: None,
            });
        }
        let Selector::AllOf { operands } = selector else {
            return Err(not_attribute());
        };
        let [has, Selector::AnyOf { operands: valued }] = operands.as_slice() else {
            return Err(not_attribute());
        };
        let name = has_value(has).ok_or_else(not_attribute)?;
        let [Selector::Not { operand }, meets] = valued.as_slice() else {
            return Err(not_attribute());
        };
        if operand.as_ref() != has {
            return Err(not_attribute());
        }
        Ok(Attribute {
            name: Value::Simple(name),
            value: Some(value_of(meets)?),
        })
    }

    /// What a capability's parameters state: one requirement, or the
    /// applicability's cardinality.
    fn reading(
        &self,
        capability: &str,
        parameters: &BTreeMap<String, ParameterValue>,
    ) -> Result<Reading, Refusal> {
        let flag = |name: &str| {
            matches!(
                parameters.get(name),
                Some(ParameterValue::Boolean { value: true })
            )
        };
        let occurrence = if flag("prohibited") {
            Occurrence::Prohibited
        } else if flag("optional") {
            Occurrence::Optional
        } else {
            Occurrence::Required
        };
        if capability == axioval_rules::PropertyRequired.id() {
            let facet = match self.target(parameters)? {
                Target::Attribute(name) => Facet::Attribute(Attribute {
                    name: Value::Simple(name),
                    value: None,
                }),
                Target::Property { set, name } => Facet::Property(Property {
                    property_set: set,
                    base_name: name,
                    value: None,
                    data_type: None,
                }),
                Target::Material => {
                    return Err(Refusal::Parameter(
                        "a required list of material names has no IDS facet".to_owned(),
                    ));
                }
            };
            return Ok(requirement(facet, Occurrence::Required));
        }
        if capability == axioval_rules::PropertyDataType.id()
            || capability == axioval_rules::PropertyValueConstraint.id()
        {
            let data_type = match parameters.get("data_type") {
                Some(ParameterValue::String { value }) => Some(value.clone()),
                _ => None,
            };
            let value = value_of_parameters(parameters);
            let facet = match self.target(parameters)? {
                Target::Attribute(name) if data_type.is_none() => Facet::Attribute(Attribute {
                    name: Value::Simple(name),
                    value,
                }),
                Target::Material if data_type.is_none() => Facet::Material(Material { value }),
                Target::Property { set, name } => Facet::Property(Property {
                    property_set: set,
                    base_name: name,
                    value,
                    data_type,
                }),
                _ => {
                    return Err(Refusal::Parameter(
                        "only a property facet states a data type".to_owned(),
                    ));
                }
            };
            return Ok(requirement(facet, occurrence));
        }
        if capability == axioval_rules::PropertyRequirements.id() {
            return self.rows(parameters);
        }
        if capability == axioval_rules::ClassificationRequirement.id() {
            let list = |name: &str| match parameters.get(name) {
                Some(ParameterValue::StringList { value }) => value.clone(),
                _ => Vec::new(),
            };
            let system =
                lists_value(list("systems"), list("system_patterns")).ok_or_else(|| {
                    Refusal::Parameter("a classification requirement without a system".to_owned())
                })?;
            let value = lists_value(list("codes"), list("code_patterns"));
            return Ok(requirement(
                Facet::Classification(Classification { value, system }),
                occurrence,
            ));
        }
        if capability == axioval_rules::SelectorConformance.id() {
            return self.conformance(parameters);
        }
        if capability == axioval_rules::ObjectCount.id() {
            return count(parameters);
        }
        Err(Refusal::Capability(capability.to_owned()))
    }

    /// A `selector-conformance` rule: the facet its requirement states,
    /// negated when prohibited.
    fn conformance(
        &self,
        parameters: &BTreeMap<String, ParameterValue>,
    ) -> Result<Reading, Refusal> {
        let Some(ParameterValue::Selector { value: selector }) = parameters.get("requirement")
        else {
            return Err(Refusal::Parameter(
                "a conformance rule without a requirement".to_owned(),
            ));
        };
        match selector.as_ref() {
            Selector::Not { operand } if operand.as_ref() == &Selector::All => {
                Err(Refusal::Selector(
                    "it fails every object it selects, which IDS states only as a prohibited specification".to_owned(),
                ))
            }
            Selector::Not { operand } => {
                let facet = self.facet(operand)?;
                if matches!(facet, Facet::Entity(_)) {
                    return Err(Refusal::Selector(
                        "it prohibits an entity, which an IDS requirement states only as required".to_owned(),
                    ));
                }
                Ok(requirement(facet, Occurrence::Prohibited))
            }
            // No material at all, or one going by the value.
            Selector::AnyOf { operands }
                if operands.len() == 2
                    && matches!(&operands[0], Selector::Not { operand } if self.material(operand).is_ok_and(|material| material.value.is_none())) =>
            {
                let material = self.material(&operands[1])?;
                if material.value.is_none() {
                    return Err(Refusal::Selector(
                        "no material or any material holds for every object".to_owned(),
                    ));
                }
                Ok(requirement(Facet::Material(material), Occurrence::Optional))
            }
            other => match self.facet(other)? {
                // One attribute a restricted name selects; a name alone
                // is `property-required`.
                Facet::Attribute(Attribute {
                    name: Value::Simple(name),
                    value: None,
                }) => Ok(requirement(
                    Facet::Attribute(Attribute {
                        name: Value::Restriction(Box::new(Restriction {
                            enumeration: vec![name],
                            ..restriction("string")
                        })),
                        value: None,
                    }),
                    Occurrence::Required,
                )),
                facet => Ok(requirement(facet, Occurrence::Required)),
            },
        }
    }

    /// The property a property capability names.
    fn target(&self, parameters: &BTreeMap<String, ParameterValue>) -> Result<Target, Refusal> {
        if let Some(ParameterValue::PropertyReference {
            property,
            property_set,
        }) = parameters.get("property")
        {
            return match property_set.as_deref() {
                Some(ATTRIBUTE_SET) => Ok(Target::Attribute(self.property(property)?)),
                Some(MATERIAL_SET) if self.property(property)? == MATERIAL_NAMES => {
                    Ok(Target::Material)
                }
                Some(set)
                    if axioval_ir::is_reserved_set(set) || axioval_ir::is_derived_set(set) =>
                {
                    Err(Refusal::Parameter(format!(
                        "it reads the reserved set {set}, which no IDS facet names"
                    )))
                }
                Some(set) => Ok(Target::Property {
                    set: Value::Simple(self.property_set(set)?),
                    name: Value::Simple(self.property(property)?),
                }),
                None => Err(Refusal::Parameter(
                    "a property without a set, which no IDS facet names".to_owned(),
                )),
            };
        }
        match (
            parameters.get("property_set_pattern"),
            parameters.get("property_pattern"),
        ) {
            (
                Some(ParameterValue::String { value: sets }),
                Some(ParameterValue::String { value: names }),
            ) => Ok(Target::Property {
                set: pattern_names(sets),
                name: pattern_names(names),
            }),
            _ => Err(Refusal::Parameter(
                "it names no property set and property as IDS does".to_owned(),
            )),
        }
    }

    /// `property-requirements` rows: a prohibited property or attribute, or
    /// a required one named by pattern.
    fn rows(&self, parameters: &BTreeMap<String, ParameterValue>) -> Result<Reading, Refusal> {
        let refused = || {
            Refusal::Parameter(
                "its requirement rows state no IDS property or attribute facet".to_owned(),
            )
        };
        let Some(ParameterValue::Table { value: rows }) = parameters.get("requirements") else {
            return Err(refused());
        };
        let text = |row: &BTreeMap<String, ParameterValue>, column: &str| match row.get(column) {
            Some(ParameterValue::String { value }) => Some(value.clone()),
            _ => None,
        };
        let forbidding = |row: &BTreeMap<String, ParameterValue>| {
            text(row, "state").as_deref() == Some("exclude")
                && text(row, "presence").as_deref() == Some("not-empty")
        };
        if let [row] = rows.as_slice()
            && let (Some(sets), Some(names)) = (
                text(row, "property_set_pattern"),
                text(row, "property_pattern"),
            )
        {
            let occurrence = if forbidding(row) {
                Occurrence::Prohibited
            } else if text(row, "requirement").as_deref() == Some("required") {
                Occurrence::Required
            } else {
                return Err(refused());
            };
            let facet = Facet::Property(Property {
                property_set: pattern_names(&sets),
                base_name: pattern_names(&names),
                value: None,
                data_type: None,
            });
            return Ok(requirement(facet, occurrence));
        }
        if rows.is_empty() || !rows.iter().all(forbidding) {
            return Err(refused());
        }
        let mut sets = Vec::new();
        let mut names = Vec::new();
        for row in rows {
            let (Some(set), Some(name)) = (text(row, "property_set"), text(row, "property")) else {
                return Err(refused());
            };
            if set == ATTRIBUTE_SET {
                if rows.len() != 1 {
                    return Err(refused());
                }
                let facet = Facet::Attribute(Attribute {
                    name: Value::Simple(self.property(&name)?),
                    value: None,
                });
                return Ok(requirement(facet, Occurrence::Prohibited));
            }
            let set = self.property_set(&set)?;
            let name = self.property(&name)?;
            if !sets.contains(&set) {
                sets.push(set);
            }
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let facet = Facet::Property(Property {
            property_set: names_value(sets),
            base_name: names_value(names),
            value: None,
            data_type: None,
        });
        Ok(requirement(facet, Occurrence::Prohibited))
    }
}

/// What one rule's capability states.
enum Reading {
    Requirement(Requirement),
    Count { minimum: u32, maximum: Option<u32> },
}

/// The property a property capability reads.
enum Target {
    /// An attribute, by name.
    Attribute(String),
    /// Every name a material goes by.
    Material,
    /// A property set and property, literal or by pattern.
    Property { set: Value, name: Value },
}

/// An `object-count` rule: the applicability's cardinality.
fn count(parameters: &BTreeMap<String, ParameterValue>) -> Result<Reading, Refusal> {
    let bound = |name: &str| -> Result<Option<u32>, Refusal> {
        match parameters.get(name) {
            None => Ok(None),
            Some(ParameterValue::Integer { value }) => u32::try_from(*value)
                .map(Some)
                .map_err(|_| Refusal::Parameter(format!("{name} {value} is no count IDS states"))),
            Some(_) => Err(Refusal::Parameter(format!("{name} is not an integer"))),
        }
    };
    Ok(Reading::Count {
        minimum: bound("minimum")?.unwrap_or(0),
        maximum: bound("maximum")?,
    })
}

/// One requirement facet with `occurrence`, and nothing else.
fn requirement(facet: Facet, occurrence: Occurrence) -> Reading {
    Reading::Requirement(Requirement {
        facet,
        occurrence,
        uri: None,
        instructions: None,
    })
}

fn not_material(selector: &Selector) -> Refusal {
    Refusal::Selector(format!("{} states no IDS material facet", shown(selector)))
}

/// Visits `selector` and every selector inside it.
fn visit<'s>(selector: &'s Selector, f: &mut impl FnMut(&'s Selector)) {
    f(selector);
    match selector {
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                visit(operand, f);
            }
        }
        Selector::Not { operand } => visit(operand, f),
        Selector::Related { selector, .. } => visit(selector, f),
        _ => {}
    }
}

/// Whether any property selector in `selector` reads the reserved `set`.
fn mentions(selector: &Selector, set: &str) -> bool {
    let mut found = false;
    visit(selector, &mut |node| {
        if let Selector::Property {
            property_set: Some(s),
            ..
        } = node
            && s == set
        {
            found = true;
        }
    });
    found
}

fn is_classification(selector: &Selector) -> bool {
    match selector {
        Selector::Classification { .. } => true,
        Selector::AnyOf { operands } => operands
            .iter()
            .all(|operand| matches!(operand, Selector::Classification { .. })),
        _ => false,
    }
}

/// A classification facet from its selectors: the systems and codes they
/// name, each once.
fn classification(selector: &Selector) -> Result<Classification, Refusal> {
    let operands: Vec<&Selector> = match selector {
        Selector::AnyOf { operands } => operands.iter().collect(),
        other => vec![other],
    };
    let mut systems = Vec::new();
    let mut codes = Vec::new();
    let mut patterns = Vec::new();
    for operand in operands {
        let Selector::Classification {
            system,
            code,
            code_pattern,
            ..
        } = operand
        else {
            return Err(Refusal::Selector("not a classification".to_owned()));
        };
        if !systems.contains(system) {
            systems.push(system.clone());
        }
        if let Some(code) = code
            && !codes.contains(code)
        {
            codes.push(code.clone());
        }
        if let Some(pattern) = code_pattern
            && !patterns.contains(pattern)
        {
            patterns.push(pattern.clone());
        }
    }
    Ok(Classification {
        value: lists_value(codes, patterns),
        system: names_value(systems),
    })
}

/// A value naming exactly `names`: a literal for one, an enumeration for
/// several.
fn names_value(mut names: Vec<String>) -> Value {
    if names.len() == 1 {
        Value::Simple(names.remove(0))
    } else {
        Value::Restriction(Box::new(Restriction {
            enumeration: names,
            ..restriction("string")
        }))
    }
}

/// A value from literals and patterns, `None` for neither.
fn lists_value(literals: Vec<String>, patterns: Vec<String>) -> Option<Value> {
    match (literals.len(), patterns.is_empty()) {
        (0, true) => None,
        (1, true) => Some(names_value(literals)),
        _ => Some(Value::Restriction(Box::new(Restriction {
            enumeration: literals,
            patterns,
            ..restriction("string")
        }))),
    }
}

fn restriction(base: &str) -> Restriction {
    Restriction {
        base: base.to_owned(),
        ..Restriction::default()
    }
}

/// The names a property set or property pattern stands for: an escaped
/// alternation of literals is their enumeration, anything else the
/// pattern itself.
fn pattern_names(pattern: &str) -> Value {
    let alternatives: Option<Vec<String>> = pattern
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .map(|inner| inner.split(")|(").map(unescape).collect())
        .and_then(|names: Vec<Option<String>>| names.into_iter().collect());
    match alternatives {
        Some(names) if names.len() > 1 => names_value(names),
        _ => Value::Restriction(Box::new(Restriction {
            patterns: vec![pattern.to_owned()],
            ..restriction("string")
        })),
    }
}

/// The literal an XML Schema pattern escaping every metacharacter matches,
/// `None` for any other pattern.
fn unescape(pattern: &str) -> Option<String> {
    const META: &str = "\\|.?*+(){}[]^-";
    let mut out = String::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let escaped = chars.next()?;
            if !META.contains(escaped) {
                return None;
            }
            out.push(escaped);
        } else if META.contains(c) {
            return None;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// The IDS value `property-value` parameters state, `None` for none.
fn value_of_parameters(parameters: &BTreeMap<String, ParameterValue>) -> Option<Value> {
    let list = |name: &str| match parameters.get(name) {
        Some(ParameterValue::StringList { value }) => value.clone(),
        _ => Vec::new(),
    };
    let text = |name: &str| match parameters.get(name) {
        Some(ParameterValue::String { value }) => Some(value.clone()),
        _ => None,
    };
    let count = |name: &str| match parameters.get(name) {
        Some(ParameterValue::Integer { value }) => u64::try_from(*value).ok(),
        _ => None,
    };
    let bounds = [
        text("min_inclusive"),
        text("max_inclusive"),
        text("min_exclusive"),
        text("max_exclusive"),
    ];
    let numeric = bounds
        .iter()
        .flatten()
        .all(|bound| bound.parse::<f64>().is_ok());
    let base = if bounds.iter().any(Option::is_some) && numeric {
        "double"
    } else {
        "string"
    };
    let [min_inclusive, max_inclusive, min_exclusive, max_exclusive] = bounds;
    let restriction = Restriction {
        enumeration: list("values"),
        patterns: list("patterns"),
        min_inclusive,
        max_inclusive,
        min_exclusive,
        max_exclusive,
        length: count("length"),
        min_length: count("min_length"),
        max_length: count("max_length"),
        total_digits: count("total_digits"),
        fraction_digits: count("fraction_digits"),
        ..restriction(base)
    };
    if restriction == self::restriction(base) {
        return None;
    }
    let rest = Restriction {
        enumeration: Vec::new(),
        ..restriction.clone()
    };
    if restriction.enumeration.len() == 1 && rest == self::restriction(base) {
        return Some(Value::Simple(restriction.enumeration[0].clone()));
    }
    Some(Value::Restriction(Box::new(restriction)))
}

/// The IDS value a selector compares a slot with: a literal, or the
/// restriction whose facets its operands state.
fn value_of(selector: &Selector) -> Result<Value, Refusal> {
    let refused = || {
        Refusal::Selector(format!(
            "{} compares a value no IDS restriction states",
            shown(selector)
        ))
    };
    if let Selector::Property {
        operator: ComparisonOperator::Equals,
        value: Some(value),
        ..
    } = selector
    {
        return Ok(Value::Simple(literal(value).ok_or_else(refused)?));
    }
    let parts: Vec<&Selector> = match selector {
        Selector::AllOf { operands } => operands.iter().collect(),
        other => vec![other],
    };
    let mut restriction = restriction("string");
    let length = Regex::new(r"\A\(\?s\)\.\{(\d+)(,(\d*))?\}\z").expect("valid pattern");
    for part in parts {
        match part {
            Selector::Property {
                operator: ComparisonOperator::OneOf,
                value: Some(ParameterValue::StringList { value }),
                ..
            } => restriction.enumeration.clone_from(value),
            Selector::Property {
                operator: ComparisonOperator::Matches,
                value: Some(ParameterValue::String { value }),
                ..
            } => match length.captures(value) {
                Some(captures) => {
                    let number = |index: usize| {
                        captures
                            .get(index)
                            .filter(|found| !found.as_str().is_empty())
                            .and_then(|found| found.as_str().parse::<u64>().ok())
                    };
                    if captures.get(2).is_none() {
                        restriction.length = number(1);
                    } else {
                        restriction.min_length = number(1).filter(|low| *low > 0);
                        restriction.max_length = number(3);
                    }
                }
                None => restriction.patterns.push(xsd_pattern(value)?),
            },
            Selector::Property {
                operator,
                value: Some(bound @ ParameterValue::Integer { .. }),
                ..
            } => {
                "integer".clone_into(&mut restriction.base);
                let bound = literal(bound);
                match operator {
                    ComparisonOperator::GreaterThanOrEquals => restriction.min_inclusive = bound,
                    ComparisonOperator::LessThanOrEquals => restriction.max_inclusive = bound,
                    ComparisonOperator::GreaterThan => restriction.min_exclusive = bound,
                    ComparisonOperator::LessThan => restriction.max_exclusive = bound,
                    _ => return Err(refused()),
                }
            }
            Selector::AnyOf { operands } => {
                for operand in operands {
                    match operand {
                        Selector::Property {
                            operator: ComparisonOperator::Equals,
                            value: Some(value),
                            ..
                        } => {
                            if !matches!(value, ParameterValue::String { .. }) {
                                match value {
                                    ParameterValue::Boolean { .. } => "boolean",
                                    _ => "integer",
                                }
                                .clone_into(&mut restriction.base);
                            }
                            restriction
                                .enumeration
                                .push(literal(value).ok_or_else(refused)?);
                        }
                        Selector::Property {
                            operator: ComparisonOperator::Matches,
                            value: Some(ParameterValue::String { value }),
                            ..
                        } => restriction.patterns.push(xsd_pattern(value)?),
                        _ => return Err(refused()),
                    }
                }
            }
            _ => return Err(refused()),
        }
    }
    Ok(Value::Restriction(Box::new(restriction)))
}

/// A literal as IDS spells it.
fn literal(value: &ParameterValue) -> Option<String> {
    match value {
        ParameterValue::String { value } => Some(value.clone()),
        ParameterValue::Boolean { value } => Some(value.to_string()),
        ParameterValue::Integer { value } => Some(value.to_string()),
        _ => None,
    }
}

/// `regex` as an XML Schema pattern meaning the same, when it is written
/// alike in both.
fn xsd_pattern(regex: &str) -> Result<String, Refusal> {
    if axioval_rules::translate_xsd_pattern(regex).is_ok_and(|translated| translated == regex) {
        Ok(regex.to_owned())
    } else {
        Err(Refusal::Selector(format!(
            "the regular expression {regex:?} is written differently as an XML Schema pattern"
        )))
    }
}

/// A selector as a package spells it.
fn shown(selector: &Selector) -> String {
    serde_json::to_string(selector).unwrap_or_default()
}
