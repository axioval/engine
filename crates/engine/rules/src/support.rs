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
        scope: axioval_ir::Scope::Object(object.clone()),
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

/// `value` as a float, when the conversion is exact (magnitude up to 2^53).
pub(crate) fn exact_f64(value: i64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    (value.unsigned_abs() <= 1 << 53).then_some(value as f64)
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

/// A declared numeric tolerance: absolute and relative, or rounding to decimals.
///
/// With `tolerance` and/or `relative_tolerance`, two numbers are equal when
/// `|a - b| <= tolerance + relative_tolerance * max(|a|, |b|)`, the boundary
/// included. The bound is symmetric but not transitive. Values are decimals
/// as a reviewer reads them, so the comparison allows a few units in the last
/// place for binary rounding: `1.1` and `1.0` are within `0.1`.
///
/// With `decimals`, both numbers are first rounded half away from zero to
/// that many decimal places of their shortest decimal form (`2.345` rounds
/// to `2.35`, as displayed) and then compared exactly. Rounding is
/// transitive; it cannot be combined with a tolerance.
///
/// Quantities are compared in canonical SI units, so a tolerance or rounding
/// on a length is in metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Tolerance {
    absolute: f64,
    relative: f64,
    decimals: Option<u32>,
}

/// The largest number of decimals a rule may round to.
const MAX_DECIMALS: i64 = 15;

impl Tolerance {
    /// Exact up to the binary rounding of one unit conversion.
    ///
    /// A quantity declared in `mm` is scaled to metres before it is compared
    /// with a value the source stated in metres; the product may differ from
    /// the decimal the author meant in the last place. A few units in the
    /// last place are equal, anything more is not.
    pub(crate) fn unit_conversion() -> Self {
        Self {
            relative: 4.0 * f64::EPSILON,
            ..Self::default()
        }
    }

    /// Whether this is exact comparison: no tolerance and no rounding.
    pub(crate) fn is_exact(&self) -> bool {
        self.decimals.is_none() && self.absolute == 0.0 && self.relative == 0.0
    }

    /// Whether this rounds to decimals rather than allowing a distance.
    pub(crate) fn rounds(&self) -> bool {
        self.decimals.is_some()
    }

    /// `value` rounded as declared; unchanged without `decimals`.
    pub(crate) fn round(&self, value: f64) -> f64 {
        match self.decimals {
            Some(decimals) => round_decimal(value, decimals),
            None => value,
        }
    }

    /// Whether two finite numbers are equal under this tolerance.
    pub(crate) fn equal(&self, left: f64, right: f64) -> bool {
        if self.decimals.is_some() {
            return self.round(left).total_cmp(&self.round(right)).is_eq();
        }
        let magnitude = left.abs().max(right.abs());
        let bound = self.absolute + self.relative * magnitude;
        // Binary rounding of decimal inputs and of the bound itself; an
        // exact comparison takes none.
        let slack = if bound > 0.0 {
            4.0 * f64::EPSILON * magnitude.max(bound)
        } else {
            0.0
        };
        (left - right).abs() <= bound + slack
    }

    /// The order of two finite numbers, `Equal` when they are equal under
    /// this tolerance; `None` when either is not finite.
    pub(crate) fn order(&self, left: f64, right: f64) -> Option<std::cmp::Ordering> {
        if !left.is_finite() || !right.is_finite() {
            return None;
        }
        if self.equal(left, right) {
            Some(std::cmp::Ordering::Equal)
        } else {
            self.round(left).partial_cmp(&self.round(right))
        }
    }

    /// How findings state the tolerance, such as `within tolerance 0.01`.
    pub(crate) fn describe(&self) -> String {
        match self.decimals {
            Some(decimals) => format!("rounded to {decimals} decimal(s)"),
            None if self.relative == 0.0 => format!("within tolerance {}", self.absolute),
            None if self.absolute == 0.0 => {
                format!("within relative tolerance {}", self.relative)
            }
            None => format!(
                "within tolerance {} plus relative tolerance {}",
                self.absolute, self.relative
            ),
        }
    }

    /// ` (<description>)` for a finding message, or nothing when exact.
    pub(crate) fn suffix(&self) -> String {
        if self.is_exact() {
            String::new()
        } else {
            format!(" ({})", self.describe())
        }
    }
}

/// Rounds `value` half away from zero to `decimals` places of its shortest
/// decimal form.
fn round_decimal(value: f64, decimals: u32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    // `{:e}` prints the shortest digits that read back as `value`.
    let text = format!("{:e}", value.abs());
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return value;
    };
    let Ok(exponent) = exponent.parse::<i64>() else {
        return value;
    };
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    // Digits kept: those before the point plus `decimals` after it.
    let Ok(keep) = usize::try_from(exponent + 1 + i64::from(decimals)) else {
        // Every digit lies below half a unit of the last kept place.
        return 0.0;
    };
    if keep >= digits.len() {
        return value;
    }
    let kept = digits[..keep]
        .iter()
        .fold(0_u64, |total, digit| total * 10 + u64::from(digit - b'0'));
    let units = kept + u64::from(digits[keep] >= b'5');
    let rounded: f64 = format!("{units}e-{decimals}")
        .parse()
        .expect("a decimal literal parses");
    // `+ 0.0` turns a negative zero into zero, so it keys like zero.
    value.signum() * rounded + 0.0
}

/// Descriptors of the tolerance parameters numeric comparisons take.
pub(crate) fn tolerance_parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("tolerance", ParameterType::Number),
        ParameterDescriptor::optional("relative_tolerance", ParameterType::Number),
        ParameterDescriptor::optional("decimals", ParameterType::Integer),
    ]
}

impl Parameters<'_> {
    /// The tolerance declared by `tolerance`, `relative_tolerance` and
    /// `decimals`; exact when none is given.
    pub(crate) fn tolerance(&self) -> Result<Tolerance, Unavailable> {
        let absolute = self.number("tolerance")?;
        let relative = self.number("relative_tolerance")?;
        let decimals = match self.integer("decimals")? {
            None => None,
            Some(value) if (0..=MAX_DECIMALS).contains(&value) => {
                Some(u32::try_from(value).expect("bounded above"))
            }
            Some(_) => {
                return Err(invalid(format!(
                    "`decimals` must be between 0 and {MAX_DECIMALS}"
                )));
            }
        };
        if absolute.is_some_and(|value| value < 0.0) {
            return Err(invalid("`tolerance` is negative"));
        }
        if relative.is_some_and(|value| !(0.0..1.0).contains(&value)) {
            return Err(invalid(
                "`relative_tolerance` must be at least 0 and below 1",
            ));
        }
        if decimals.is_some() && (absolute.is_some() || relative.is_some()) {
            return Err(invalid(
                "declare either `decimals` or a tolerance, not both",
            ));
        }
        Ok(Tolerance {
            absolute: absolute.unwrap_or(0.0),
            relative: relative.unwrap_or(0.0),
            decimals,
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Tolerance, round_decimal};

    #[test]
    fn rounding_reads_the_shortest_decimal_form_half_away_from_zero() {
        assert_eq!(round_decimal(2.345, 2), 2.35);
        assert_eq!(round_decimal(1.005, 2), 1.01);
        assert_eq!(round_decimal(2.344_999, 2), 2.34);
        assert_eq!(round_decimal(-2.345, 2), -2.35);
        assert_eq!(round_decimal(0.6, 0), 1.0);
        assert_eq!(round_decimal(0.4, 0), 0.0);
        assert_eq!(round_decimal(0.000_4, 2), 0.0);
        assert_eq!(round_decimal(9.999, 2), 10.0);
        assert_eq!(round_decimal(123.0, 2), 123.0);
        assert_eq!(round_decimal(1e300, 2), 1e300);
        assert!(round_decimal(-0.001, 2).is_sign_positive());
    }

    #[test]
    fn a_tolerance_includes_its_boundary_as_written_in_decimal() {
        let absolute = Tolerance {
            absolute: 0.1,
            ..Tolerance::default()
        };
        assert!(absolute.equal(1.0, 1.1));
        assert!(absolute.equal(1.1, 1.0));
        assert!(!absolute.equal(1.0, 1.100_001));
        let relative = Tolerance {
            relative: 0.25,
            ..Tolerance::default()
        };
        assert!(relative.equal(3.0, 4.0));
        assert!(!relative.equal(2.9, 4.0));
        assert!(Tolerance::default().is_exact());
        assert!(!Tolerance::default().equal(1.0, 1.0 + f64::EPSILON * 8.0));
    }
}
