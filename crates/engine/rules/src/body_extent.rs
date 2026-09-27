//! `body-extent`: the depth of an object's body along one of its own
//! placement axes, against a stated length or a range.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, MetricDirection, NotEvaluatedReason, ObjectFrame,
    ObjectFrameError, ObjectFrameServiceHandle, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext, VerticalExtentError, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, Object, PropertyValue, QuantityDimension};

use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
use crate::selection::select_objects;
use crate::support::{Parameters, PropertyRef, Resolved, Unavailable, finding, invalid, resolve};

/// Requires each selected object's body, measured along one of its own
/// placement axes, to be as long as a property states or within a range.
///
/// A straight wall's layer set states its thickness; the body measures it
/// across the wall. With `axis` `forward` (the placement's second axis,
/// across a wall whose layers run along its first) and `target_property`
/// the material set's `TotalThickness`, a wall whose body is thicker or
/// thinner than its layers is found. `minimum` and `maximum` bound the
/// extent instead.
///
/// The extent is the whole body's depth along the axis: the highest less
/// the lowest point projected onto it. It is the thickness only where the
/// body is a slab of constant thickness across that axis; a curved wall, or
/// one with a projecting part, measures deeper. Select the objects the
/// measure fits.
///
/// Extents are intervals: a tessellated body measures within its chord
/// deviation, and an axis off the coordinate axes within the rounding of
/// the projection. A verdict needs the whole interval on one side of a
/// bound; one straddling it is not evaluated. The target and range are
/// widened by the binary rounding of decimal coordinates, a few units in
/// the last place, so an exactly modelled body is not found by rounding.
pub struct BodyExtent;

#[derive(Clone, Copy)]
enum Axis {
    Right,
    Forward,
    Up,
}

impl Axis {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "right" => Ok(Self::Right),
            "forward" => Ok(Self::Forward),
            "up" => Ok(Self::Up),
            other => Err(invalid(format!(
                "axis `{other}` is unsupported; use `right`, `forward` or `up`"
            ))),
        }
    }

    fn of(self, frame: &ObjectFrame) -> MetricDirection {
        let frame = frame.frame();
        match self {
            Self::Right => frame.right(),
            Self::Forward => frame.forward(),
            Self::Up => frame.up(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Forward => "forward",
            Self::Up => "up",
        }
    }
}

/// What the extent is compared with.
enum Bound<'a> {
    /// A length the object states, equal within a tolerance.
    Property(PropertyRef<'a>, f64),
    /// Inclusive bounds in metres.
    Range(Option<f64>, Option<f64>),
}

struct Config<'a> {
    axis: Axis,
    bound: Bound<'a>,
}

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("`{name}` is negative"))),
        Some(_) => Err(invalid(format!("`{name}` is not a length"))),
    }
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let axis = Axis::parse(parameters.required_string("axis")?)?;
        let target = parameters.property("target_property")?;
        let tolerance = length(&parameters, "tolerance")?;
        let minimum = length(&parameters, "minimum")?;
        let maximum = length(&parameters, "maximum")?;
        let bound = match (target, minimum, maximum) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(invalid(
                    "declare either `target_property` or `minimum`/`maximum`, not both",
                ));
            }
            (Some(property), None, None) => Bound::Property(property, tolerance.unwrap_or(0.0)),
            (None, None, None) => {
                return Err(invalid(
                    "`target_property`, `minimum` or `maximum` is required",
                ));
            }
            (None, minimum, maximum) => {
                if tolerance.is_some() {
                    return Err(invalid("`tolerance` applies to `target_property` only"));
                }
                if let (Some(minimum), Some(maximum)) = (minimum, maximum)
                    && minimum > maximum
                {
                    return Err(invalid("`minimum` exceeds `maximum`"));
                }
                Bound::Range(minimum, maximum)
            }
        };
        Ok(Self { axis, bound })
    }
}

impl RuleCapability for BodyExtent {
    fn id(&self) -> &'static str {
        "axioval:capability.body-extent"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("axis", ParameterType::String),
            ParameterDescriptor::optional("target_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("body-extent: {message}"),
                );
            }
        };
        let (Some(frames), Some(extents)) = (
            context.services.get::<ObjectFrameServiceHandle>(),
            context.services.get::<VerticalExtentServiceHandle>(),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "body-extent needs the object-frame and vertical-extent services",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            match check(context, rule, &config, frames, extents, object) {
                Ok(Some(found)) => evaluation.push_finding(found),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

pub(crate) fn frame_error(error: &ObjectFrameError) -> Unavailable {
    let reason = match error {
        ObjectFrameError::UncoveredSource(_) => NotEvaluatedReason::MissingService,
        ObjectFrameError::NotPlaced(_)
        | ObjectFrameError::Unsupported(_)
        | ObjectFrameError::Unreadable(_) => NotEvaluatedReason::IncompleteEvidence,
        ObjectFrameError::UnknownObject(_)
        | ObjectFrameError::InvalidFrame
        | ObjectFrameError::InexactEvidence
        | ObjectFrameError::ResponseRequestMismatch => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("object frame: {error}"))
}

pub(crate) fn extent_error(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, format!("body extent: {error}"))
}

/// A few units in the last place of the largest magnitude involved: decimal
/// coordinates and lengths read in binary differ from what was meant by
/// that much, and no more.
fn rounding_slack(magnitudes: &[f64]) -> f64 {
    4.0 * f64::EPSILON
        * magnitudes
            .iter()
            .fold(0.0_f64, |most, value| most.max(value.abs()))
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    config: &Config<'_>,
    frames: &ObjectFrameServiceHandle,
    extents: &VerticalExtentServiceHandle,
    object: &Object,
) -> Result<Option<axioval_ir::Finding>, Unavailable> {
    let frame = frames
        .object_frame(&object.id)
        .map_err(|error| frame_error(&error))?;
    let direction = config.axis.of(&frame);
    let extent = extents
        .measure_directional_extent(&object.id, direction)
        .map_err(|error| extent_error(&error))?;
    let (lower, upper) = extent.length_metres();
    let positions = [extent.lower().lower_metres(), extent.upper().upper_metres()];
    let mut evidence: Vec<Evidence> = vec![frame.evidence().clone(), extent.evidence().clone()];
    let axis = config.axis.name();
    let measured = format!("body extent along `{axis}` is {}", shown(lower, upper));
    let (minimum, maximum, stated) = match &config.bound {
        Bound::Range(minimum, maximum) => {
            let slack = rounding_slack(&[positions[0], positions[1], upper]);
            (
                minimum.map(|bound| bound - slack),
                maximum.map(|bound| bound + slack),
                String::new(),
            )
        }
        Bound::Property(property, tolerance) => {
            let resolved = resolve(context, object, *property)?;
            evidence.extend(resolved.evidence());
            let target = match &resolved {
                Resolved::Absent(_) => {
                    return Ok(Some(finding(
                        rule,
                        &object.id,
                        format!("{measured}; `{property}` is absent"),
                        evidence,
                        vec![],
                    )));
                }
                Resolved::Present(_) => match resolved.value() {
                    Some(PropertyValue::Quantity {
                        value,
                        dimension: QuantityDimension::Length,
                    }) if value.is_finite() => *value,
                    other => {
                        return Err((
                            NotEvaluatedReason::InvalidEvidence,
                            format!(
                                "`{property}` is {}, not a length",
                                crate::support::display(other)
                            ),
                        ));
                    }
                },
            };
            let slack = rounding_slack(&[positions[0], positions[1], target, *tolerance]);
            let within = if *tolerance > 0.0 {
                format!(" within {}", metres(*tolerance))
            } else {
                String::new()
            };
            (
                Some(target - tolerance - slack),
                Some(target + tolerance + slack),
                format!("; `{property}` states {}{within}", metres(target)),
            )
        }
    };
    match judge(lower, upper, minimum, maximum) {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => {
            let required = if stated.is_empty() {
                format!("; required {}", in_metres(&bound))
            } else {
                stated
            };
            Ok(Some(finding(
                rule,
                &object.id,
                format!("{measured}{required}"),
                evidence,
                vec![],
            )))
        }
        Verdict::Undecided(bound) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{measured}, which straddles {}{stated}",
                if stated.is_empty() {
                    in_metres(&bound)
                } else {
                    "the stated length".to_owned()
                }
            ),
        )),
    }
}

/// `at least 0.24` as `at least 0.24 m`, rounded as lengths are shown.
fn in_metres(bound: &str) -> String {
    match bound.rsplit_once(' ') {
        Some((words, value)) => match value.parse::<f64>() {
            Ok(value) => format!("{words} {}", metres(value)),
            Err(_) => bound.to_owned(),
        },
        None => bound.to_owned(),
    }
}
