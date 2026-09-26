//! Parameter access, property resolution and relationship traversal shared by
//! the semantic capabilities.

use axioval_engine::{
    AbsentEndPolicy, CompiledRule, NotEvaluatedReason, PropertyResolution,
    PropertyResolutionServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, RuleContext,
    SemanticRelationship, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Finding, Object, ObjectId, Property, PropertyValue, Severity};

use crate::selection::{bound_property_request, property_error};

/// Why an object or rule could not be evaluated.
pub(crate) type Unavailable = (NotEvaluatedReason, String);

pub(crate) fn invalid(message: impl Into<String>) -> Unavailable {
    (NotEvaluatedReason::InvalidDeclaration, message.into())
}

/// Typed read access to a compiled rule's parameters.
pub(crate) struct Parameters<'a>(pub(crate) &'a CompiledRule);

impl<'a> Parameters<'a> {
    fn get(&self, name: &str) -> Option<&'a ParameterValue> {
        self.0.parameters.get(name)
    }

    /// A present parameter of the wrong type is a declaration error, not absence.
    fn typed<T>(
        &self,
        name: &str,
        read: impl FnOnce(&'a ParameterValue) -> Option<T>,
    ) -> Result<Option<T>, Unavailable> {
        match self.get(name) {
            None => Ok(None),
            Some(value) => read(value)
                .map(Some)
                .ok_or_else(|| invalid(format!("parameter `{name}` has the wrong type"))),
        }
    }

    fn required<T>(name: &str, value: Option<T>) -> Result<T, Unavailable> {
        value.ok_or_else(|| invalid(format!("parameter `{name}` is required")))
    }

    pub(crate) fn string(&self, name: &str) -> Result<Option<&'a str>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::String { value }
            | ParameterValue::Enum { value }
            | ParameterValue::Reference { value } => Some(value.as_str()),
            _ => None,
        })
    }

    pub(crate) fn required_string(&self, name: &str) -> Result<&'a str, Unavailable> {
        let value = self.string(name)?;
        Self::required(name, value)
    }

    pub(crate) fn integer(&self, name: &str) -> Result<Option<i64>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::Integer { value } => Some(*value),
            _ => None,
        })
    }

    pub(crate) fn number(&self, name: &str) -> Result<Option<f64>, Unavailable> {
        match self.typed(name, |value| match value {
            ParameterValue::Number { value } => Some(*value),
            _ => None,
        })? {
            Some(value) if !value.is_finite() => {
                Err(invalid(format!("parameter `{name}` is not finite")))
            }
            other => Ok(other),
        }
    }

    pub(crate) fn boolean(&self, name: &str) -> Result<Option<bool>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::Boolean { value } => Some(*value),
            _ => None,
        })
    }

    pub(crate) fn strings(&self, name: &str) -> Result<Option<&'a [String]>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::StringList { value } | ParameterValue::ReferenceList { value } => {
                Some(value.as_slice())
            }
            _ => None,
        })
    }

    pub(crate) fn selector(&self, name: &str) -> Result<Option<&'a Selector>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::Selector { value } => Some(value.as_ref()),
            _ => None,
        })
    }

    pub(crate) fn required_selector(&self, name: &str) -> Result<&'a Selector, Unavailable> {
        let value = self.selector(name)?;
        Self::required(name, value)
    }

    pub(crate) fn property(&self, name: &str) -> Result<Option<PropertyRef<'a>>, Unavailable> {
        self.typed(name, |value| match value {
            ParameterValue::PropertyReference {
                property,
                property_set,
            } => Some(PropertyRef {
                set: property_set.as_deref(),
                name: property.as_str(),
            }),
            _ => None,
        })
    }

    pub(crate) fn required_property(&self, name: &str) -> Result<PropertyRef<'a>, Unavailable> {
        let value = self.property(name)?;
        Self::required(name, value)
    }

    /// An optional relationship traversal declared by the `relationship`,
    /// `direction`, `follow_chain` and `skip_absent_relationship_ends` parameters.
    pub(crate) fn traversal(&self) -> Result<Option<Traversal<'a>>, Unavailable> {
        let Some(relationship) = self.string("relationship")? else {
            return Ok(None);
        };
        let direction = match self.string("direction")? {
            None | Some("forward") => TraversalDirection::Forward,
            Some("backward") => TraversalDirection::Backward,
            Some("either") => TraversalDirection::Either,
            Some(other) => return Err(invalid(format!("direction `{other}` is unsupported"))),
        };
        Ok(Some(Traversal {
            relationship,
            direction,
            follow_chain: self.boolean("follow_chain")?.unwrap_or(false),
            absent_ends: if self.boolean("skip_absent_relationship_ends")? == Some(true) {
                AbsentEndPolicy::Skip
            } else {
                AbsentEndPolicy::Refuse
            },
        }))
    }
}

/// A property reference: optional set qualifier and name.
#[derive(Clone, Copy)]
pub(crate) struct PropertyRef<'a> {
    pub(crate) set: Option<&'a str>,
    pub(crate) name: &'a str,
}

impl std::fmt::Display for PropertyRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.set {
            Some(set) => write!(f, "{set}.{}", self.name),
            None => f.write_str(self.name),
        }
    }
}

/// An exact property answer.
pub(crate) enum Resolved {
    Present(Property),
    Absent(Evidence),
}

impl Resolved {
    /// The value, or `None` when absent.
    pub(crate) fn value(&self) -> Option<&PropertyValue> {
        match self {
            Self::Present(property) => Some(&property.value),
            Self::Absent(_) => None,
        }
    }

    /// The exact evidence behind this answer.
    pub(crate) fn evidence(&self) -> Vec<Evidence> {
        match self {
            Self::Present(property) => property.evidence.iter().cloned().collect(),
            Self::Absent(evidence) => vec![evidence.clone()],
        }
    }
}

/// Resolves one property of one object exactly, in the object's own vocabulary.
pub(crate) fn resolve(
    context: &RuleContext<'_>,
    object: &Object,
    property: PropertyRef<'_>,
) -> Result<Resolved, Unavailable> {
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        ));
    };
    let request = bound_property_request(context, object, property.set, property.name)?;
    match service.resolve(&request) {
        Ok(PropertyResolution::Present(resolved)) => {
            Ok(Resolved::Present(resolved.property().clone()))
        }
        Ok(PropertyResolution::Absent(proof)) => Ok(Resolved::Absent(proof.evidence().clone())),
        Err(error) => Err(property_error(error)),
    }
}

/// A declared relationship traversal from each anchor.
pub(crate) struct Traversal<'a> {
    pub(crate) relationship: &'a str,
    pub(crate) direction: TraversalDirection,
    pub(crate) follow_chain: bool,
    pub(crate) absent_ends: AbsentEndPolicy,
}

impl Traversal<'_> {
    /// Objects of `universe` related to `anchor`, with the completeness evidence.
    pub(crate) fn related(
        &self,
        context: &RuleContext<'_>,
        anchor: &ObjectId,
        universe: &[&Object],
    ) -> Result<(Vec<ObjectId>, Vec<Evidence>), Unavailable> {
        let Some(service) = context.services.get::<RelationshipSelectionServiceHandle>() else {
            return Err((
                NotEvaluatedReason::MissingService,
                "relationship-selection service is not registered".into(),
            ));
        };
        let relationship = SemanticRelationship::try_new(self.relationship)
            .map_err(|error| invalid(error.to_string()))?;
        let request = RelationshipSelectionRequest::try_new(
            anchor.clone(),
            universe.iter().map(|object| object.id.clone()).collect(),
            RelationshipQuery::Related {
                relationship,
                direction: self.direction,
                follow_chain: self.follow_chain,
            },
        )
        .map_err(|error| invalid(error.to_string()))?
        .with_absent_ends(self.absent_ends);
        service
            .select(&request)
            .map(|selection| {
                (
                    selection.candidates().to_vec(),
                    selection.evidence().to_vec(),
                )
            })
            .map_err(|error| match error {
                RelationshipSelectionError::Unavailable(message) => {
                    (NotEvaluatedReason::BackendUnavailable, message)
                }
                other => (NotEvaluatedReason::InvalidEvidence, other.to_string()),
            })
    }
}

/// A finding of `rule` against `object`, evidence sorted and deduplicated.
pub(crate) fn finding(
    rule: &CompiledRule,
    object: &ObjectId,
    message: String,
    mut evidence: Vec<Evidence>,
    related: Vec<ObjectId>,
) -> Finding {
    evidence.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
    evidence.dedup();
    Finding {
        rule_id: rule.id.clone(),
        object_id: object.clone(),
        related: Vec::new(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
    }
    .with_related(related)
}

/// A value as a reviewer reads it in a message.
pub(crate) fn display(value: Option<&PropertyValue>) -> String {
    match value {
        None => "absent".into(),
        Some(PropertyValue::Null) => "null".into(),
        Some(PropertyValue::Boolean(value)) => value.to_string(),
        Some(PropertyValue::Integer(value)) => value.to_string(),
        Some(PropertyValue::Decimal(value)) => value.to_string(),
        Some(PropertyValue::Quantity { value, dimension }) => format!("{value} {dimension:?}"),
        Some(PropertyValue::String(value)) => format!("`{value}`"),
    }
}

/// Whether a value is missing in the sense of "nothing stated": absent, null or blank text.
pub(crate) fn undefined(value: Option<&PropertyValue>) -> bool {
    match value {
        None | Some(PropertyValue::Null) => true,
        Some(PropertyValue::String(text)) => text.trim().is_empty(),
        Some(_) => false,
    }
}
