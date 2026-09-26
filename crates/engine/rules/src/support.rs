//! Parameter access, property resolution and relationship traversal shared by
//! the semantic capabilities.

use axioval_engine::{
    AbsentEndPolicy, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PropertyResolution, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionServiceHandle,
    RuleContext, SemanticRelationship, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{
    Evidence, Finding, Object, ObjectId, Property, PropertyValue, QuantityDimension, Severity,
};

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

/// One step of a relationship path.
pub(crate) struct Step<'a> {
    relationship: &'a str,
    direction: TraversalDirection,
}

/// A declared relationship traversal from each anchor.
///
/// Either one `relationship` (with `direction` and `follow_chain`) or a
/// `path` of steps, each `Relationship` or `Relationship:direction`, walked
/// one after another: `IfcRelVoidsElement:forward` then
/// `IfcRelFillsElement:forward` goes from a wall through its openings to the
/// doors and windows filling them. Intermediate objects may be anything; the
/// objects the last step reaches are restricted to the caller's universe.
pub(crate) struct Traversal<'a> {
    /// How messages name the traversal: the relationship, or the steps.
    pub(crate) relationship: String,
    steps: Vec<Step<'a>>,
    follow_chain: bool,
    absent_ends: AbsentEndPolicy,
}

impl<'a> Parameters<'a> {
    /// An optional relationship traversal declared by the `relationship`,
    /// `direction`, `follow_chain`, `path` and `skip_absent_relationship_ends`
    /// parameters.
    pub(crate) fn traversal(&self) -> Result<Option<Traversal<'a>>, Unavailable> {
        let direction = |value: Option<&str>| match value {
            None | Some("forward") => Ok(TraversalDirection::Forward),
            Some("backward") => Ok(TraversalDirection::Backward),
            Some("either") => Ok(TraversalDirection::Either),
            Some(other) => Err(invalid(format!("direction `{other}` is unsupported"))),
        };
        let relationship = self.string("relationship")?;
        let path = self.strings("path")?;
        let follow_chain = self.boolean("follow_chain")?.unwrap_or(false);
        let steps = match (relationship, path) {
            (None, None) => return Ok(None),
            (Some(_), Some(_)) => {
                return Err(invalid("declare either `relationship` or `path`, not both"));
            }
            (Some(relationship), None) => vec![Step {
                relationship,
                direction: direction(self.string("direction")?)?,
            }],
            (None, Some(path)) => {
                if path.is_empty() {
                    return Err(invalid("`path` has no steps"));
                }
                if self.string("direction")?.is_some() || follow_chain {
                    return Err(invalid(
                        "a `path` states each step's direction and cannot follow chains",
                    ));
                }
                path.iter()
                    .map(|step| {
                        let (relationship, stated) = match step.split_once(':') {
                            Some((relationship, stated)) => (relationship, Some(stated)),
                            None => (step.as_str(), None),
                        };
                        Ok(Step {
                            relationship: relationship.trim(),
                            direction: direction(stated.map(str::trim))?,
                        })
                    })
                    .collect::<Result<Vec<_>, Unavailable>>()?
            }
        };
        Ok(Some(Traversal {
            relationship: steps
                .iter()
                .map(|step| step.relationship)
                .collect::<Vec<_>>()
                .join(" then "),
            steps,
            follow_chain,
            absent_ends: if self.boolean("skip_absent_relationship_ends")? == Some(true) {
                AbsentEndPolicy::Skip
            } else {
                AbsentEndPolicy::Refuse
            },
        }))
    }
}

/// Descriptors of the traversal parameters every relationship-scoped capability takes.
pub(crate) fn traversal_parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("relationship", ParameterType::String),
        ParameterDescriptor::optional("direction", ParameterType::String),
        ParameterDescriptor::optional("follow_chain", ParameterType::Boolean),
        ParameterDescriptor::optional("path", ParameterType::StringList),
        ParameterDescriptor::optional("skip_absent_relationship_ends", ParameterType::Boolean),
    ]
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
        let everything: Vec<&Object> = context.project.objects().collect();
        let mut frontier = vec![anchor.clone()];
        let mut evidence = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            let last = index + 1 == self.steps.len();
            let scope = if last { universe } else { &everything[..] };
            let relationship = SemanticRelationship::try_new(step.relationship)
                .map_err(|error| invalid(error.to_string()))?;
            let mut reached = std::collections::BTreeSet::new();
            for from in &frontier {
                let request = RelationshipSelectionRequest::try_new(
                    from.clone(),
                    scope.iter().map(|object| object.id.clone()).collect(),
                    RelationshipQuery::Related {
                        relationship: relationship.clone(),
                        direction: step.direction,
                        follow_chain: self.follow_chain,
                    },
                )
                .map_err(|error| invalid(error.to_string()))?
                .with_absent_ends(self.absent_ends);
                let selection = service.select(&request).map_err(|error| match error {
                    RelationshipSelectionError::Unavailable(message) => {
                        (NotEvaluatedReason::BackendUnavailable, message)
                    }
                    other => (NotEvaluatedReason::InvalidEvidence, other.to_string()),
                })?;
                reached.extend(selection.candidates().iter().cloned());
                evidence.extend(selection.evidence().iter().cloned());
            }
            // The anchor is never its own relative, even through a round trip.
            reached.remove(anchor);
            frontier = reached.into_iter().collect();
        }
        evidence.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
        evidence.dedup();
        Ok((frontier, evidence))
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
        Some(PropertyValue::Quantity { value, dimension }) => {
            format!("{value} {}", dimension.unit_symbol())
        }
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

/// The group an object is judged in: its source, and optionally the objects a
/// declared relationship reaches from it (a storey, a zone).
///
/// Several reached objects form one combined group; reaching none forms the
/// group of everything in the source that reaches nothing.
pub(crate) fn scope_key(
    context: &RuleContext<'_>,
    traversal: Option<&Traversal<'_>>,
    across_sources: bool,
    object: &Object,
) -> Result<(String, Vec<Evidence>), Unavailable> {
    let mut key = if across_sources {
        String::new()
    } else {
        object.id.source.to_string()
    };
    let mut evidence = Vec::new();
    if let Some(traversal) = traversal {
        let universe: Vec<&Object> = context.project.objects().collect();
        let (reached, found) = traversal.related(context, &object.id, &universe)?;
        evidence = found;
        for id in reached {
            key.push('\n');
            key.push_str(&id.to_string());
        }
    }
    Ok((key, evidence))
}

/// A value as a grouping key: text trimmed and folded as declared.
pub(crate) fn value_key(value: &PropertyValue, trim: bool, case_sensitive: bool) -> String {
    match value {
        PropertyValue::String(text) => {
            let text = if trim { text.trim() } else { text.as_str() };
            let text = if case_sensitive {
                text.to_owned()
            } else {
                text.to_lowercase()
            };
            format!("text:{text}")
        }
        other => format!("value:{}", display(Some(other))),
    }
}

/// A declared quantity in canonical SI: the value and its dimension.
///
/// Units are the ones rule authors write for building checks: lengths
/// (`m`, `cm`, `mm`, `km`), areas (`m2`, `cm2`, `mm2`), volumes (`m3`,
/// `cm3`, `mm3`, `l`) and plane angles (`rad`, `deg`); `²`, `³` and `°` are
/// accepted too. Anything else is a declaration error, never a guess.
pub(crate) fn si_quantity(value: f64, unit: &str) -> Result<(f64, QuantityDimension), Unavailable> {
    use QuantityDimension::{Area, Length, PlaneAngle, Volume};
    let unit = unit
        .trim()
        .replace('²', "2")
        .replace('³', "3")
        .replace('°', "deg");
    let (scale, dimension) = match unit.as_str() {
        "m" => (1.0, Length),
        "cm" => (1e-2, Length),
        "mm" => (1e-3, Length),
        "km" => (1e3, Length),
        "m2" => (1.0, Area),
        "cm2" => (1e-4, Area),
        "mm2" => (1e-6, Area),
        "m3" => (1.0, Volume),
        "cm3" => (1e-6, Volume),
        "mm3" => (1e-9, Volume),
        "l" | "L" => (1e-3, Volume),
        "rad" => (1.0, PlaneAngle),
        "deg" => (std::f64::consts::PI / 180.0, PlaneAngle),
        other => return Err(invalid(format!("unit `{other}` is not supported"))),
    };
    let si = value * scale;
    if si.is_finite() {
        Ok((si, dimension))
    } else {
        Err(invalid("quantity is not finite"))
    }
}

impl Parameters<'_> {
    /// A quantity parameter in canonical SI.
    pub(crate) fn quantity(
        &self,
        name: &str,
    ) -> Result<Option<(f64, QuantityDimension)>, Unavailable> {
        match self.typed(name, |value| match value {
            ParameterValue::Quantity { value, unit } => Some((*value, unit.as_str())),
            _ => None,
        })? {
            Some((value, unit)) => si_quantity(value, unit)
                .map(Some)
                .map_err(|(reason, message)| (reason, format!("parameter `{name}`: {message}"))),
            None => Ok(None),
        }
    }
}
