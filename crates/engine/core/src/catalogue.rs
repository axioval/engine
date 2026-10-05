//! The authoring catalogue: everything a rule may be built from, as one
//! versioned, deterministic JSON document a block editor is generated
//! from (`axioval catalogue`).
//!
//! It lists the registered capabilities with their parameters, the
//! measured values and member lists, the expression node kinds with their
//! fields and results, the comparison operators by value type, aggregate
//! functions and sources, selector kinds, stated and derived
//! relationships, the unit symbols and dimensions, and the concept
//! vocabulary of the definition packages given. A capability built as a
//! template carries its composition. Every entry carries labels
//! and help in English and German. The contract tables live in
//! [`axioval_ir::catalogue`]; the capabilities' texts come from the host,
//! which must supply them for every capability it registered.

use std::collections::BTreeMap;

use axioval_ir::DefinitionPackage;
use axioval_ir::blocks::{Block, to_blocks};
use axioval_ir::catalogue::{
    AGGREGATE_FUNCTIONS, AGGREGATE_SOURCES, AggregateFunctionEntry, CATALOGUE_SCHEMA_VERSION,
    Category, EXPRESSION_COMPARISONS, EXPRESSION_KINDS, NODE_CATEGORIES, NodeKind, Operator,
    SELECTOR_COMPARISONS, SELECTOR_KINDS, SLOPE_FORM_ENTRIES, SelectorKind, SlopeFormEntry,
    SourceKind, VALUE_TYPES, ValueTypeEntry,
};
use axioval_ir::contract::{
    Expression, LocalizedText as PackageText, ObjectTypeDefinition, PropertyDefinition,
    PropertySetDefinition, RuleDefinition,
};
use axioval_ir::measured::{
    LocalizedText, MEASURED_MEMBERS, MEASURED_VALUES, MeasuredDescriptor, MemberDescriptor, en_de,
};
use serde::Serialize;

use crate::derived_relationships::{DERIVATIONS, DerivationEntry};
use crate::expression::{UNIT_SYMBOLS, UnitSymbol};
use crate::relationships::RelationshipKind;
use crate::template::Template;
use crate::{CapabilityRegistry, ColumnKind, ParameterType};

/// The texts a host supplies for one capability it registered.
#[derive(Clone, Copy, Debug)]
pub struct CapabilityTexts {
    /// A short name for editors, in English and German.
    pub label: &'static [LocalizedText],
    /// What it checks.
    pub help: &'static [LocalizedText],
    /// Help for each parameter, by name.
    pub parameters: &'static [ParameterText],
}

/// The help a host supplies for one capability parameter.
#[derive(Clone, Copy, Debug)]
pub struct ParameterText {
    pub name: &'static str,
    pub help: &'static [LocalizedText],
}

/// Why no catalogue was built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueError {
    /// A registered capability has no texts.
    #[error("capability `{0}` has no catalogue texts")]
    MissingCapability(String),
    /// A capability parameter has no help.
    #[error("parameter `{parameter}` of capability `{capability}` has no catalogue help")]
    MissingParameter {
        capability: String,
        parameter: String,
    },
    /// A text is not stated in English and German, or is blank.
    #[error("{0} is not labelled in English and German")]
    Untranslated(String),
    /// A locale the catalogue is not written in.
    #[error("locale `{0}` is not supported; the catalogue is written in {LANGUAGES:?}")]
    UnsupportedLocale(String),
    /// A template's composition does not map onto a block tree.
    #[error("the composition of capability `{capability}` has no block tree: {problem}")]
    UnmappedTemplate { capability: String, problem: String },
}

/// The languages every catalogue text is stated in, English first: the
/// language a missing translation falls back to.
pub const LANGUAGES: &[&str] = &["en", "de"];

/// A catalogue in one language: every list of texts (`[{"language",
/// "text"}, …]`) replaced by the text in `locale`, or, where none is
/// stated in it, by the English one, and every package text by its
/// translation into `locale`, else its default. Returns the JSON and the
/// JSON pointer of every built-in text that fell back, in document order.
///
/// # Errors
///
/// [`CatalogueError::UnsupportedLocale`] for a locale outside
/// [`LANGUAGES`].
pub fn localized(
    catalogue: &Catalogue,
    locale: &str,
) -> Result<(serde_json::Value, Vec<String>), CatalogueError> {
    if !LANGUAGES.contains(&locale) {
        return Err(CatalogueError::UnsupportedLocale(locale.to_owned()));
    }
    let mut value = serde_json::to_value(catalogue).unwrap_or_default();
    let mut fallbacks = Vec::new();
    localize(&mut value, locale, &mut String::new(), &mut fallbacks);
    if let Some(languages) = value.get_mut("languages") {
        *languages = serde_json::json!([locale]);
    }
    Ok((value, fallbacks))
}

/// The texts of `value` if it is a list of localized texts.
fn texts(value: &serde_json::Value) -> Option<Vec<(&str, &str)>> {
    let items = value.as_array()?;
    if items.is_empty() {
        return None;
    }
    items
        .iter()
        .map(|item| {
            let object = item.as_object()?;
            if object.len() != 2 {
                return None;
            }
            Some((
                object.get("language")?.as_str()?,
                object.get("text")?.as_str()?,
            ))
        })
        .collect()
}

fn localize(
    value: &mut serde_json::Value,
    locale: &str,
    pointer: &mut String,
    fallbacks: &mut Vec<String>,
) {
    if let Some(stated) = texts(value) {
        let chosen = stated
            .iter()
            .find(|(language, _)| *language == locale)
            .or_else(|| {
                fallbacks.push(pointer.clone());
                stated
                    .iter()
                    .find(|(language, _)| *language == LANGUAGES[0])
            })
            .map_or("", |(_, text)| *text)
            .to_owned();
        *value = serde_json::Value::String(chosen);
        return;
    }
    // A package's own text: its translation, else its default.
    if let Some(object) = value.as_object()
        && object.len() == 2
        && let (
            Some(serde_json::Value::String(default)),
            Some(serde_json::Value::Object(translations)),
        ) = (object.get("default"), object.get("translations"))
    {
        let chosen = translations
            .get(locale)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(default)
            .to_owned();
        *value = serde_json::Value::String(chosen);
        return;
    }
    let length = pointer.len();
    match value {
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                pointer.push('/');
                pointer.push_str(&index.to_string());
                localize(item, locale, pointer, fallbacks);
                pointer.truncate(length);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, item) in fields.iter_mut() {
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                localize(item, locale, pointer, fallbacks);
                pointer.truncate(length);
            }
        }
        _ => {}
    }
}

/// The authoring catalogue, in its JSON form.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    /// [`CATALOGUE_SCHEMA_VERSION`]: a reader accepts its own major version.
    pub schema_version: &'static str,
    /// The languages every text is stated in, in this order.
    pub languages: &'static [&'static str],
    pub value_types: &'static [ValueTypeEntry],
    pub units: Units,
    pub capabilities: Vec<CapabilityEntry>,
    pub measured_values: Vec<MeasuredEntry>,
    pub measured_members: Vec<MemberEntry>,
    pub expression_categories: &'static [Category],
    pub expression_kinds: &'static [NodeKind],
    pub expression_comparisons: &'static [Operator],
    pub aggregate_functions: &'static [AggregateFunctionEntry],
    pub aggregate_sources: &'static [SourceKind],
    pub slope_forms: &'static [SlopeFormEntry],
    pub selector_kinds: &'static [SelectorKind],
    pub selector_comparisons: &'static [Operator],
    pub relationships: Relationships,
    /// The concept vocabulary of each definition package given, by id.
    pub concepts: Vec<Concepts>,
}

/// Unit symbols and the dimensions measured values are answered in.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Units {
    /// The base units an exponent counts, in order.
    pub bases: &'static [&'static str],
    pub symbols: &'static [UnitSymbol],
    pub dimensions: &'static [DimensionEntry],
}

/// One dimension with its coherent unit.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionEntry {
    pub dimension: &'static str,
    pub unit: &'static str,
    pub label: &'static [LocalizedText],
}

const DIMENSIONS: &[DimensionEntry] = &[
    DimensionEntry {
        dimension: "length",
        unit: "m",
        label: &en_de("Length", "Länge"),
    },
    DimensionEntry {
        dimension: "area",
        unit: "m2",
        label: &en_de("Area", "Fläche"),
    },
    DimensionEntry {
        dimension: "volume",
        unit: "m3",
        label: &en_de("Volume", "Volumen"),
    },
    DimensionEntry {
        dimension: "planeAngle",
        unit: "rad",
        label: &en_de("Plane angle", "Ebener Winkel"),
    },
    DimensionEntry {
        dimension: "none",
        unit: "1",
        label: &en_de("Plain number", "Zahl ohne Einheit"),
    },
];

/// One registered capability.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEntry {
    pub id: String,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
    /// Whether findings state how far a value misses its bound, so a rule
    /// may grade them with severity bands.
    pub grades_deviation: bool,
    /// Whether a definition may declare parameters of its own beyond
    /// `parameters`, which its expressions read.
    pub takes_authored_parameters: bool,
    pub parameters: Vec<ParameterEntry>,
    /// The composition a built-in template is made of, for an editor to
    /// expand; absent for a capability implemented in code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateEntry>,
}

/// The composition of a capability built as a [`Template`]: its
/// declaration checks, values, decisions and messages, and each form as
/// the one expression a rule forked from it starts from, with its block
/// tree. The expressions still hold the template's slots (`{axis}`) and
/// `parameter` reads, bound to a rule's parameters when forked.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateEntry {
    #[serde(flatten)]
    pub template: Template,
    /// Each form's [`crate::template::Form::requirement`], in form order.
    pub requirements: Vec<RequirementEntry>,
}

/// One form of a template as an expression.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementEntry {
    /// The parameters whose statement selects the form.
    pub when: &'static [&'static str],
    pub expression: Expression,
    pub blocks: Block,
}

/// The catalogue entry of `template`, the capability `id`'s composition.
///
/// # Errors
///
/// [`CatalogueError::UnmappedTemplate`] when a form's expression does not
/// map onto a block tree.
pub fn template_entry(id: &str, template: &Template) -> Result<TemplateEntry, CatalogueError> {
    let requirements = template
        .forms
        .iter()
        .map(|form| {
            let expression = form.requirement();
            let blocks =
                to_blocks(&expression).map_err(|problem| CatalogueError::UnmappedTemplate {
                    capability: id.to_owned(),
                    problem: problem.to_string(),
                })?;
            Ok(RequirementEntry {
                when: form.when,
                expression,
                blocks,
            })
        })
        .collect::<Result<_, CatalogueError>>()?;
    Ok(TemplateEntry {
        template: template.clone(),
        requirements,
    })
}

/// One parameter of a capability.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterEntry {
    pub name: String,
    /// Its kind in a definition package.
    pub kind: &'static str,
    /// What an `expression` parameter's value must be: `boolean` or
    /// `number`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_type: Option<&'static str>,
    pub required: bool,
    /// Whether a rule may give it as an expression evaluated per object.
    pub per_object: bool,
    /// Whether a `string` holds an expression's text form.
    pub expression_text: bool,
    /// The columns of a `table`, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnEntry>,
    pub help: &'static [LocalizedText],
}

/// One column of a table parameter.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnEntry {
    pub id: &'static str,
    pub kind: ColumnKind,
    pub required: bool,
}

/// One measured value, and whether this host measures it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredEntry {
    #[serde(flatten)]
    pub descriptor: &'static MeasuredDescriptor,
    /// The coherent SI unit it is answered in.
    pub unit: &'static str,
    /// Whether a registered provider or the engine measures it; a rule
    /// reading one that is not is never evaluated.
    pub available: bool,
}

/// One measured member list, and whether this host measures it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberEntry {
    #[serde(flatten)]
    pub descriptor: &'static MemberDescriptor,
    pub available: bool,
}

/// Stated and derived relationships a path step may name.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationships {
    /// The directions a step may take, after `:`.
    pub directions: &'static [&'static str],
    pub stated: Vec<StatedRelationship>,
    pub derived: &'static [DerivationEntry],
    /// A relation a ruleset declares, by its id.
    pub declared: DeclaredRelation,
}

/// One source-neutral kind of stated relationship.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatedRelationship {
    pub id: String,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

/// The relationship of a relation a ruleset declares.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredRelation {
    /// Written with the relation's id after `;id=`.
    pub id: &'static str,
    pub label: &'static [LocalizedText],
    pub help: &'static [LocalizedText],
}

/// The concept vocabulary of one definition package.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Concepts {
    pub package: String,
    pub name: PackageText,
    pub object_types: BTreeMap<String, ObjectTypeDefinition>,
    pub property_sets: BTreeMap<String, PropertySetDefinition>,
    pub properties: BTreeMap<String, PropertyDefinition>,
    pub definitions: BTreeMap<String, RuleDefinition>,
}

fn stated_texts(kind: RelationshipKind) -> (&'static [LocalizedText], &'static [LocalizedText]) {
    match kind {
        RelationshipKind::Containment => (
            &const { en_de("Contains", "Enthält") },
            &const {
                en_de(
                    "A spatial structure element to the elements it contains.",
                    "Ein räumliches Strukturelement zu den Elementen, die es enthält.",
                )
            },
        ),
        RelationshipKind::Aggregation => (
            &const { en_de("Has parts", "Hat Teile") },
            &const { en_de("A whole to its parts.", "Ein Ganzes zu seinen Teilen.") },
        ),
        RelationshipKind::Voids => (
            &const { en_de("Is voided by", "Wird geöffnet durch") },
            &const {
                en_de(
                    "An element to the openings voiding it.",
                    "Ein Bauteil zu den Öffnungen darin.",
                )
            },
        ),
        RelationshipKind::Fills => (
            &const { en_de("Is filled by", "Wird gefüllt durch") },
            &const {
                en_de(
                    "An opening to the elements filling it.",
                    "Eine Öffnung zu den Bauteilen, die sie füllen.",
                )
            },
        ),
        RelationshipKind::SpaceBoundary => (
            &const { en_de("Is bounded by", "Wird begrenzt durch") },
            &const {
                en_de(
                    "A space to the elements bounding it.",
                    "Ein Raum zu den Bauteilen, die ihn begrenzen.",
                )
            },
        ),
        RelationshipKind::TypeAssignment => (
            &const { en_de("Types", "Typisiert") },
            &const {
                en_de(
                    "A type to the occurrences it is assigned to.",
                    "Ein Typ zu den Exemplaren, denen er zugewiesen ist.",
                )
            },
        ),
        RelationshipKind::GroupMembership => (
            &const { en_de("Has members", "Hat Mitglieder") },
            &const {
                en_de(
                    "A group, system or zone to its members.",
                    "Eine Gruppe, ein System oder eine Zone zu ihren Mitgliedern.",
                )
            },
        ),
        RelationshipKind::Connection => (
            &const { en_de("Is connected to", "Ist verbunden mit") },
            &const {
                en_de(
                    "An element to the elements it is connected to.",
                    "Ein Bauteil zu den Bauteilen, mit denen es verbunden ist.",
                )
            },
        ),
    }
}

const DECLARED_RELATION: DeclaredRelation = DeclaredRelation {
    id: "axioval:derived.relation",
    label: &en_de("Declared relation", "Deklarierte Beziehung"),
    help: &en_de(
        "A relation the ruleset declares between objects the model does not relate, by \
         `;id=` and the relation's id.",
        "Eine Beziehung, die das Regelwerk zwischen Objekten deklariert, die das Modell nicht \
         verbindet, mit `;id=` und der Kennung der Beziehung.",
    ),
};

/// Whether `texts` states English then German, none blank.
fn bilingual(texts: &[LocalizedText]) -> bool {
    texts.iter().map(|text| text.language).eq(["en", "de"])
        && texts.iter().all(|text| !text.text.trim().is_empty())
}

fn checked(
    texts: &'static [LocalizedText],
    what: impl FnOnce() -> String,
) -> Result<&'static [LocalizedText], CatalogueError> {
    if bilingual(texts) {
        Ok(texts)
    } else {
        Err(CatalogueError::Untranslated(what()))
    }
}

/// The `expressionType` of an expression parameter.
fn expression_type(parameter_type: ParameterType) -> Option<&'static str> {
    match parameter_type {
        ParameterType::Expression => Some("boolean"),
        ParameterType::NumberExpression => Some("number"),
        _ => None,
    }
}

/// Every measured value and member list's texts are in English and German.
fn check_measured() -> Result<(), CatalogueError> {
    let descriptors = MEASURED_VALUES
        .iter()
        .chain(MEASURED_MEMBERS.iter().map(|members| &members.list));
    for descriptor in descriptors {
        let name = descriptor.name;
        checked(descriptor.label, || format!("measured value `{name}`"))?;
        checked(descriptor.help, || format!("measured value `{name}`"))?;
        for parameter in descriptor.parameters {
            checked(parameter.help, || {
                format!("parameter `{}` of measured value `{name}`", parameter.key)
            })?;
        }
    }
    for members in MEASURED_MEMBERS {
        for field in members.fields {
            let what = || {
                format!(
                    "field `{}` of member list `{}`",
                    field.name, members.list.name
                )
            };
            checked(field.label, what)?;
            checked(field.help, what)?;
        }
    }
    Ok(())
}

/// The stated, derived and declared relationships a path step may name.
fn relationships() -> Relationships {
    Relationships {
        directions: &["forward", "backward", "either"],
        stated: RelationshipKind::ALL
            .into_iter()
            .map(|kind| {
                let (label, help) = stated_texts(kind);
                StatedRelationship {
                    id: kind.relationship().as_str().to_owned(),
                    label,
                    help,
                }
            })
            .collect(),
        derived: DERIVATIONS,
        declared: DECLARED_RELATION,
    }
}

/// Every capability `registry` holds, described by `texts`.
fn capability_entries(
    registry: &CapabilityRegistry,
    texts: impl Fn(&str) -> Option<CapabilityTexts>,
) -> Result<Vec<CapabilityEntry>, CatalogueError> {
    let mut capabilities = Vec::new();
    for id in registry.ids() {
        let Some(capability) = registry.get(id) else {
            continue;
        };
        let described = texts(id).ok_or_else(|| CatalogueError::MissingCapability(id.into()))?;
        let mut parameters = Vec::new();
        for descriptor in capability.parameters() {
            let help = described
                .parameters
                .iter()
                .find(|text| text.name == descriptor.name)
                .map(|text| text.help)
                .ok_or_else(|| CatalogueError::MissingParameter {
                    capability: id.into(),
                    parameter: descriptor.name.clone(),
                })?;
            let help = checked(help, || {
                format!("parameter `{}` of capability `{id}`", descriptor.name)
            })?;
            let columns = match descriptor.parameter_type {
                ParameterType::Table(columns) => columns
                    .iter()
                    .map(|column| ColumnEntry {
                        id: column.id,
                        kind: column.kind,
                        required: column.required,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            parameters.push(ParameterEntry {
                kind: descriptor.parameter_type.package_kind(),
                expression_type: expression_type(descriptor.parameter_type),
                required: descriptor.required,
                per_object: descriptor.per_object,
                expression_text: descriptor.expression_text,
                columns,
                help,
                name: descriptor.name,
            });
        }
        capabilities.push(CapabilityEntry {
            id: id.to_owned(),
            label: checked(described.label, || format!("capability `{id}`"))?,
            help: checked(described.help, || format!("capability `{id}`"))?,
            grades_deviation: capability.grades_deviation(),
            takes_authored_parameters: capability.takes_authored_parameters(),
            parameters,
            template: capability
                .template()
                .map(|template| template_entry(id, template))
                .transpose()?,
        });
    }
    Ok(capabilities)
}

/// The catalogue of what `registry` runs, with `texts` naming each
/// registered capability's labels and help and the concept vocabulary of
/// `packages`. Every list is in a stable order: capabilities by id,
/// measured values and member lists as their registries order them,
/// packages as given.
///
/// # Errors
///
/// [`CatalogueError`] when a registered capability, one of its parameters,
/// or a measured value lacks texts in English and German: a capability is
/// never offered to an editor undescribed.
pub fn catalogue(
    registry: &CapabilityRegistry,
    texts: impl Fn(&str) -> Option<CapabilityTexts>,
    packages: &[DefinitionPackage],
) -> Result<Catalogue, CatalogueError> {
    check_measured()?;
    let capabilities = capability_entries(registry, texts)?;
    Ok(Catalogue {
        schema_version: CATALOGUE_SCHEMA_VERSION,
        languages: LANGUAGES,
        value_types: VALUE_TYPES,
        units: Units {
            bases: &["m", "kg", "s", "A", "K", "mol", "cd", "rad"],
            symbols: UNIT_SYMBOLS,
            dimensions: DIMENSIONS,
        },
        capabilities,
        measured_values: MEASURED_VALUES
            .iter()
            .map(|descriptor| MeasuredEntry {
                descriptor,
                unit: descriptor.unit(),
                available: registry.measures(descriptor.name),
            })
            .collect(),
        measured_members: MEASURED_MEMBERS
            .iter()
            .map(|descriptor| MemberEntry {
                descriptor,
                available: registry.measures_members(descriptor.list.name),
            })
            .collect(),
        expression_categories: NODE_CATEGORIES,
        expression_kinds: EXPRESSION_KINDS,
        expression_comparisons: EXPRESSION_COMPARISONS,
        aggregate_functions: AGGREGATE_FUNCTIONS,
        aggregate_sources: AGGREGATE_SOURCES,
        slope_forms: SLOPE_FORM_ENTRIES,
        selector_kinds: SELECTOR_KINDS,
        selector_comparisons: SELECTOR_COMPARISONS,
        relationships: relationships(),
        concepts: packages
            .iter()
            .map(|package| Concepts {
                package: package.package.id.clone(),
                name: package.package.name.clone(),
                object_types: package.object_types.clone(),
                property_sets: package.property_sets.clone(),
                properties: package.properties.clone(),
                definitions: package.definitions.clone(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measured_value_and_relationship_is_described() {
        check_measured().unwrap();
        for kind in RelationshipKind::ALL {
            let (label, help) = stated_texts(kind);
            assert!(bilingual(label) && bilingual(help), "{}", kind.name());
        }
        for derivation in DERIVATIONS {
            assert!(bilingual(derivation.label) && bilingual(derivation.help));
            assert!(
                derivation
                    .parameters
                    .iter()
                    .all(|parameter| bilingual(parameter.help))
            );
        }
        assert!(UNIT_SYMBOLS.iter().all(|unit| bilingual(unit.label)));
        assert!(
            DIMENSIONS
                .iter()
                .all(|dimension| bilingual(dimension.label))
        );
    }

    #[test]
    fn a_missing_translation_falls_back_to_english_and_is_listed() {
        let mut value = serde_json::json!({
            "label": [{"language": "en", "text": "Wall"}, {"language": "de", "text": "Wand"}],
            "items": [{"help": [{"language": "en", "text": "Only English"}]}],
            "name": {"default": "Door", "translations": {"de": "Tür"}},
            "other": {"default": "Kept", "translations": {}},
        });
        let mut fallbacks = Vec::new();
        localize(&mut value, "de", &mut String::new(), &mut fallbacks);
        assert_eq!(
            value,
            serde_json::json!({
                "label": "Wand",
                "items": [{"help": "Only English"}],
                "name": "Tür",
                "other": "Kept",
            })
        );
        assert_eq!(fallbacks, ["/items/0/help"]);
    }

    #[test]
    fn every_derivation_parses_with_its_defaults() {
        for derivation in DERIVATIONS {
            let relationship = crate::SemanticRelationship::try_new(derivation.id).unwrap();
            assert!(
                crate::Derivation::parse(&relationship).unwrap().is_some(),
                "{}",
                derivation.id
            );
        }
    }

    #[test]
    fn a_capability_without_texts_is_refused() {
        const LABEL: &[LocalizedText] = &en_de("Bare", "Bloß");
        const HELP: &[LocalizedText] = &en_de("A limit.", "Eine Grenze.");
        struct Bare;
        impl crate::RuleCapability for Bare {
            fn id(&self) -> &'static str {
                "test:bare"
            }
            fn parameters(&self) -> Vec<crate::ParameterDescriptor> {
                vec![crate::ParameterDescriptor::required(
                    "limit",
                    ParameterType::Number,
                )]
            }
            fn evaluate(
                &self,
                _: &crate::RuleContext<'_>,
                _: &crate::CompiledRule,
            ) -> crate::CapabilityEvaluation {
                crate::CapabilityEvaluation::default()
            }
        }
        let registry = CapabilityRegistry::new().register(Bare).unwrap();
        assert_eq!(
            catalogue(&registry, |_| None, &[]).unwrap_err(),
            CatalogueError::MissingCapability("test:bare".into())
        );
        let label = LABEL;
        let without_parameter = |_: &str| {
            Some(CapabilityTexts {
                label,
                help: label,
                parameters: &[],
            })
        };
        assert!(matches!(
            catalogue(&registry, without_parameter, &[]),
            Err(CatalogueError::MissingParameter { .. })
        ));
        let described = |_: &str| {
            Some(CapabilityTexts {
                label,
                help: label,
                parameters: &[ParameterText {
                    name: "limit",
                    help: HELP,
                }],
            })
        };
        let built = catalogue(&registry, described, &[]).unwrap();
        assert_eq!(built.capabilities.len(), 1);
        let english_only = |_: &str| {
            Some(CapabilityTexts {
                label: &label[..1],
                help: label,
                parameters: &[ParameterText {
                    name: "limit",
                    help: HELP,
                }],
            })
        };
        assert!(matches!(
            catalogue(&registry, english_only, &[]),
            Err(CatalogueError::Untranslated(_))
        ));
    }
}
