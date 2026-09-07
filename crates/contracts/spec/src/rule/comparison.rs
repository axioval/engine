use serde::{Deserialize, Serialize};

use crate::rule::assertion::Severity;
use crate::rule::execution::{ElementField, ElementScopeSpec, PredicateValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    Equals,
    NotEquals,
    Greater,
    Smaller,
    AtLeast,
    AtMost,
    Contains,
    OneOf,
    NoneOf,
    IsDefined,
    IsUndefined,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyComparisonComponentMode {
    #[default]
    CheckedComponent,
    RelatedComponent,
    SameSpace,
    SameWall,
    SameFederatedFloor,
    SameBuilding,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyComparisonQuantifier {
    #[default]
    Each,
    AtLeastOne,
    Count,
    Sum,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyComparisonNumericType {
    Area,
    Volume,
    Length,
    #[default]
    Float,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyComparisonTargetControls {
    #[serde(default)]
    pub numeric_type: PropertyComparisonNumericType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enumeration_values: Vec<String>,
    #[serde(default = "one")]
    pub numeric_value: f64,
    #[serde(default)]
    pub boolean_value: bool,
}

impl Default for PropertyComparisonTargetControls {
    fn default() -> Self {
        Self {
            numeric_type: PropertyComparisonNumericType::Float,
            string_value: None,
            enumeration_values: Vec::new(),
            numeric_value: 1.0,
            boolean_value: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PropertyComparisonTarget {
    String(String),
    Numeric {
        value: f64,
        numeric_type: PropertyComparisonNumericType,
    },
    Enumeration(Vec<String>),
    CheckedProperty(PropertyComparisonValueRef),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PropertyComparisonCategorization {
    ComponentType,
    Property(PropertyComparisonValueRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PropertyComparisonValueRef {
    Field(ElementField),
    /// Named identity such as `TYPE`, `NAME`, or `MATERIAL`.
    Identification(String),
    /// Provider-owned fact not expressible as a portable IFC field.
    Opaque {
        namespace: String,
        key: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyComparisonRelationDirection {
    #[default]
    Forward,
    Backward,
    Either,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyComparisonRelation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_defined_name: Option<String>,
    #[serde(default)]
    pub direction: PropertyComparisonRelationDirection,
    #[serde(default)]
    pub follow_chain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyComparisonPlanSpec {
    #[serde(default)]
    pub selection: ElementScopeSpec,
    pub fallback_type: String,
    pub checked_field: ElementField,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compared_value: Option<PropertyComparisonValueRef>,
    pub operator: ComparisonOperator,
    /// Legacy constant target retained for backwards-compatible neutral specs.
    /// New native profiles carry the exact target mode in `target_source`.
    pub target: PredicateValue,
    #[serde(default = "one")]
    pub factor: f64,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default)]
    pub component_mode: PropertyComparisonComponentMode,
    #[serde(default)]
    pub related_selection: ElementScopeSpec,
    #[serde(default)]
    pub quantifier: PropertyComparisonQuantifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_source: Option<PropertyComparisonTarget>,
    #[serde(default)]
    pub target_controls: PropertyComparisonTargetControls,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categorization: Vec<PropertyComparisonCategorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<PropertyComparisonRelation>,
}

impl PropertyComparisonPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.fallback_type.trim().is_empty() {
            return Err("property comparison fallback_type must not be empty".into());
        }
        validate_field(&self.checked_field)?;
        if let Some(value) = &self.compared_value {
            validate_value_ref(value)?;
        }
        if !self.factor.is_finite() || self.factor == 0.0 {
            return Err("property comparison factor must be finite and non-zero".into());
        }
        if !self.target_controls.numeric_value.is_finite() {
            return Err("comparison latent numeric target must be finite".into());
        }
        if self
            .target_controls
            .enumeration_values
            .iter()
            .any(std::string::String::is_empty)
        {
            return Err("comparison latent enumeration targets must not be empty".into());
        }
        if let Some(target) = &self.target_source {
            validate_target(target)?;
        }
        for categorization in &self.categorization {
            if let PropertyComparisonCategorization::Property(property) = categorization {
                validate_value_ref(property)?;
            }
        }
        if self.component_mode == PropertyComparisonComponentMode::RelatedComponent {
            let relation = self.relation.as_ref().ok_or_else(|| {
                "related-component comparison requires a relation identity".to_string()
            })?;
            let class_name = relation
                .class_name
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let user_name = relation
                .user_defined_name
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            if class_name == user_name {
                return Err(
                    "comparison relation requires exactly one class or user-defined identity"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

fn validate_target(target: &PropertyComparisonTarget) -> Result<(), String> {
    match target {
        PropertyComparisonTarget::String(value) if value.is_empty() => {
            Err("comparison string target must not be empty".into())
        }
        PropertyComparisonTarget::Numeric { value, .. } if !value.is_finite() => {
            Err("comparison numeric target must be finite".into())
        }
        PropertyComparisonTarget::Enumeration(values)
            if values.is_empty() || values.iter().any(std::string::String::is_empty) =>
        {
            Err("comparison enumeration targets must be non-empty".into())
        }
        PropertyComparisonTarget::CheckedProperty(property) => validate_value_ref(property),
        _ => Ok(()),
    }
}

fn validate_value_ref(value: &PropertyComparisonValueRef) -> Result<(), String> {
    match value {
        PropertyComparisonValueRef::Field(field) => validate_field(field),
        PropertyComparisonValueRef::Identification(name) if name.trim().is_empty() => {
            Err("comparison identification name must not be empty".into())
        }
        PropertyComparisonValueRef::Identification(_) => Ok(()),
        PropertyComparisonValueRef::Opaque { namespace, key }
            if namespace.trim().is_empty() || key.trim().is_empty() =>
        {
            Err("comparison opaque fact identity must not be empty".into())
        }
        PropertyComparisonValueRef::Opaque { .. } => Ok(()),
    }
}

fn validate_field(field: &ElementField) -> Result<(), String> {
    match field {
        ElementField::Property { name, .. } | ElementField::BooleanProperty { name, .. }
            if name.trim().is_empty() =>
        {
            Err("property comparison field name must not be empty".into())
        }
        ElementField::Classification { scheme } if scheme.trim().is_empty() => {
            Err("property comparison classification scheme must not be empty".into())
        }
        _ => Ok(()),
    }
}

fn one() -> f64 {
    1.0
}
