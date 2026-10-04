//! JSON Schemas of the package contract, generated from its types.
//!
//! [`definitions`] states a [`DefinitionPackage`], [`ruleset`] a
//! [`RuleSetPackage`] with every selector, expression and parameter value it
//! reaches. Both are JSON Schema 2020-12 documents generated from the serde
//! wire form, never written by hand, so an editor can validate a draft
//! without linking Rust. They state structure only: whether an expression
//! type checks, a concept binds or a capability exists is decided when a
//! package is compiled.
//!
//! The documentation publishes them as [`DEFINITIONS_FILE`] and
//! [`RULESET_FILE`] under [`BASE_URL`]; a test keeps those copies equal to
//! [`to_pretty_json`] of these values.

use crate::{DefinitionPackage, RuleSetPackage};
use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value};

/// Where the documentation publishes the schemas.
pub const BASE_URL: &str = "https://axioval.github.io/engine/schema/";
/// The definition package schema's file name.
pub const DEFINITIONS_FILE: &str = "definitions.schema.json";
/// The ruleset package schema's file name.
pub const RULESET_FILE: &str = "ruleset.schema.json";

/// The schema of a [`DefinitionPackage`].
#[must_use]
pub fn definitions() -> Value {
    generate::<DefinitionPackage>(
        DEFINITIONS_FILE,
        "Axioval definition package",
        "Rule definitions, their parameters and the object types, properties and property sets they name.",
    )
}

/// The schema of a [`RuleSetPackage`], selectors, expressions and parameter
/// values included.
#[must_use]
pub fn ruleset() -> Value {
    generate::<RuleSetPackage>(
        RULESET_FILE,
        "Axioval ruleset package",
        "Rule instances in folders, with their parameter values, applicability, classifications, groupings, relations and derived values.",
    )
}

/// The canonical text of a schema: keys sorted at every level, two-space
/// indentation and a final newline, whatever order the generator chose.
#[must_use]
pub fn to_pretty_json(schema: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sorted(schema))
        .unwrap_or_else(|_| unreachable!("a JSON value always serializes"));
    text.push('\n');
    text
}

fn generate<T: JsonSchema>(file: &str, title: &str, description: &str) -> Value {
    let schema: Schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut value = schema.to_value();
    if let Value::Object(root) = &mut value {
        root.insert("$id".into(), format!("{BASE_URL}{file}").into());
        root.insert("title".into(), title.into());
        root.insert("description".into(), description.into());
        root.insert(
            "$comment".into(),
            format!(
                "Generated from axioval-ir {} by `axioval_ir::schema`; never edit by hand.",
                env!("CARGO_PKG_VERSION")
            )
            .into(),
        );
    }
    value
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}
