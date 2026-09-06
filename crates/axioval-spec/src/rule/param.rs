//! Typed rule parameters.
//!
//! A rule *definition* declares a [`ParameterSpec`] (type, unit, domain,
//! default, docs); a rule *instance* supplies a [`ParamValue`] per parameter.
//! This is deliberately richer than the runtime `rules::RuleParams` bag —
//! authoring and cross-format compilation need units, enum domains, and
//! validation that a bare `HashMap<String, scalar>` cannot express.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rule::text::LocalizedText;

/// A concrete parameter value supplied by a rule instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamValue {
    Bool {
        value: bool,
    },
    Int {
        value: i64,
    },
    /// A real number. `unit` is a free-form unit token (`"mm"`, `"m2"`, `"deg"`).
    /// Backends convert as needed; the IR does not force a base unit.
    Float {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Text {
        value: String,
    },
    /// One choice from a [`ParamType::Enum`] domain (stored by its stable key).
    Enum {
        value: String,
    },
    /// A list of strings (e.g. included IFC classes, property names).
    StringList {
        value: Vec<String>,
    },
    /// Ordered rows with stable named cells. Cells may themselves be tables.
    Table {
        rows: Vec<BTreeMap<String, ParamValue>>,
    },
}

impl ParamValue {
    pub fn bool(v: bool) -> Self {
        ParamValue::Bool { value: v }
    }
    pub fn int(v: i64) -> Self {
        ParamValue::Int { value: v }
    }
    pub fn float(v: f64) -> Self {
        ParamValue::Float {
            value: v,
            unit: None,
        }
    }
    pub fn float_with_unit(v: f64, unit: impl Into<String>) -> Self {
        ParamValue::Float {
            value: v,
            unit: Some(unit.into()),
        }
    }
    pub fn text(v: impl Into<String>) -> Self {
        ParamValue::Text { value: v.into() }
    }
    pub fn enum_choice(v: impl Into<String>) -> Self {
        ParamValue::Enum { value: v.into() }
    }
    pub fn string_list<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ParamValue::StringList {
            value: items.into_iter().map(Into::into).collect(),
        }
    }

    pub fn table(rows: impl IntoIterator<Item = BTreeMap<String, ParamValue>>) -> Self {
        ParamValue::Table {
            rows: rows.into_iter().collect(),
        }
    }

    /// The [`ParamType`] discriminant this value satisfies (ignoring domain).
    pub fn type_tag(&self) -> ParamTypeTag {
        match self {
            ParamValue::Bool { .. } => ParamTypeTag::Bool,
            ParamValue::Int { .. } => ParamTypeTag::Int,
            ParamValue::Float { .. } => ParamTypeTag::Float,
            ParamValue::Text { .. } => ParamTypeTag::Text,
            ParamValue::Enum { .. } => ParamTypeTag::Enum,
            ParamValue::StringList { .. } => ParamTypeTag::StringList,
            ParamValue::Table { .. } => ParamTypeTag::Table,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::Text { value } | ParamValue::Enum { value } => Some(value),
            _ => None,
        }
    }
    /// Numeric view of a parameter.
    ///
    /// An integer is only converted when `f64` can represent it *exactly*.
    /// Beyond 2^53 the nearest `f64` is a different number, so a lossy
    /// conversion here would silently answer a comparison with a value the
    /// package never declared. Such an integer is refused rather than
    /// approximated; callers wanting the exact value use [`ParamValue::as_i64`].
    pub fn as_f64(&self) -> Option<f64> {
        const EXACT_INT_LIMIT: i64 = 1 << 53;
        match self {
            ParamValue::Float { value, .. } => Some(*value),
            // Guarded above: |value| <= 2^53, so this cast is exact by
            // construction and the pedantic precision warning does not apply.
            #[allow(clippy::cast_precision_loss)]
            ParamValue::Int { value } if value.unsigned_abs() <= EXACT_INT_LIMIT as u64 => {
                Some(*value as f64)
            }
            _ => None,
        }
    }
    /// Exact integer view, with no lossy floating-point round trip.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ParamValue::Int { value } => Some(*value),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool { value } => Some(*value),
            _ => None,
        }
    }
    pub fn as_string_list(&self) -> Option<&[String]> {
        match self {
            ParamValue::StringList { value } => Some(value),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&[BTreeMap<String, ParamValue>]> {
        match self {
            ParamValue::Table { rows } => Some(rows),
            _ => None,
        }
    }
}

/// The type discriminant of a parameter (what kind of value it accepts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamTypeTag {
    Bool,
    Int,
    Float,
    Text,
    Enum,
    StringList,
    Table,
}

/// A single choice in an enum domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumChoice {
    /// Stable key stored in a [`ParamValue::Enum`].
    pub key: String,
    /// Human-facing label.
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub label: LocalizedText,
}

impl EnumChoice {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: LocalizedText::empty(),
        }
    }
    pub fn labeled(key: impl Into<String>, label: impl Into<LocalizedText>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

/// One named column in a structured table parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamColumnSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub ty: ParamType,
    #[serde(default)]
    pub required: bool,
}

impl ParamColumnSpec {
    pub fn new(id: impl Into<String>, ty: ParamType) -> Self {
        Self {
            id: id.into(),
            ty,
            required: false,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// The declared type + domain of a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParamType {
    Bool,
    /// Integer, optional inclusive `[min, max]`.
    Int {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// Real, optional inclusive `[min, max]`, optional expected unit token.
    Float {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Text,
    /// A closed set of choices.
    Enum {
        choices: Vec<EnumChoice>,
    },
    StringList,
    Table {
        columns: Vec<ParamColumnSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_rows: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_rows: Option<usize>,
    },
}

impl ParamType {
    pub fn tag(&self) -> ParamTypeTag {
        match self {
            ParamType::Bool => ParamTypeTag::Bool,
            ParamType::Int { .. } => ParamTypeTag::Int,
            ParamType::Float { .. } => ParamTypeTag::Float,
            ParamType::Text => ParamTypeTag::Text,
            ParamType::Enum { .. } => ParamTypeTag::Enum,
            ParamType::StringList => ParamTypeTag::StringList,
            ParamType::Table { .. } => ParamTypeTag::Table,
        }
    }
}

/// The declaration of one parameter in a rule definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSpec {
    /// Stable id, referenced by assertions and instance values.
    pub id: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub label: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub description: LocalizedText,
    #[serde(rename = "type")]
    pub ty: ParamType,
    /// Whether an instance must supply this parameter (when no default exists).
    #[serde(default)]
    pub required: bool,
    /// Default value used when an instance omits this parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParamValue>,
    /// Example values for docs / MCP prompting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ParamValue>,
}

impl ParameterSpec {
    /// Minimal constructor; chain setters for the rest.
    pub fn new(id: impl Into<String>, ty: ParamType) -> Self {
        Self {
            id: id.into(),
            label: LocalizedText::empty(),
            description: LocalizedText::empty(),
            ty,
            required: false,
            default: None,
            examples: Vec::new(),
        }
    }
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
    pub fn with_default(mut self, v: ParamValue) -> Self {
        self.default = Some(v);
        self
    }
    pub fn with_label(mut self, l: impl Into<LocalizedText>) -> Self {
        self.label = l.into();
        self
    }
    pub fn with_example(mut self, v: ParamValue) -> Self {
        self.examples.push(v);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_type_tags() {
        assert_eq!(ParamValue::bool(true).type_tag(), ParamTypeTag::Bool);
        assert_eq!(ParamValue::float(1.0).type_tag(), ParamTypeTag::Float);
        assert_eq!(
            ParamValue::string_list(["a", "b"]).type_tag(),
            ParamTypeTag::StringList
        );
    }

    #[test]
    fn float_with_unit_roundtrip() {
        let v = ParamValue::float_with_unit(200.0, "mm");
        let json = serde_json::to_string(&v).unwrap();
        let back: ParamValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
        assert_eq!(v.as_f64(), Some(200.0));
    }

    #[test]
    fn spec_builder() {
        let s = ParameterSpec::new(
            "thickness",
            ParamType::Float {
                min: Some(0.0),
                max: None,
                unit: Some("mm".into()),
            },
        )
        .required()
        .with_example(ParamValue::float_with_unit(100.0, "mm"));
        assert!(s.required);
        assert_eq!(s.ty.tag(), ParamTypeTag::Float);
        assert_eq!(s.examples.len(), 1);
    }
}

#[cfg(test)]
mod exactness_tests {
    use super::ParamValue;

    /// 2^53 is the last integer `f64` represents exactly; 2^53+1 is not
    /// representable and must never be answered as an approximation.
    #[test]
    fn as_f64_refuses_integers_f64_cannot_represent_exactly() {
        let exact = ParamValue::Int { value: 1 << 53 };
        assert_eq!(exact.as_f64(), Some(9_007_199_254_740_992.0));

        let inexact = ParamValue::Int {
            value: (1i64 << 53) + 1,
        };
        assert_eq!(inexact.as_f64(), None, "9007199254740993 has no exact f64");
        assert_eq!(inexact.as_i64(), Some(9_007_199_254_740_993));

        let negative = ParamValue::Int {
            value: -((1i64 << 53) + 1),
        };
        assert_eq!(negative.as_f64(), None, "sign must not widen the domain");
    }
}
