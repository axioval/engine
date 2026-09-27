//! `centre-line-distance`: how far the centre line of a component's
//! footprint lies from the walls beside it, such as a WC's axis from the
//! side wall.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlanSpanServiceHandle, RectangleSide, RuleCapability, RuleContext, SideDistances,
};
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::level_spacing::{metres, shown};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};
use crate::wall_sides::{Nearest, Walls};

/// Requires the centre line of each selected component's footprint to lie
/// between `minimum` and `maximum` from the walls beside it.
///
/// The footprint's least-area rectangle gives the centre line: along its
/// long or short axis (`centre_line` `long` or `short`), or, with
/// `against-wall`, from the wall the component stands against to its front
/// (as `component-clearance` derives it). The distance is measured square
/// to the centre line, to the nearest `wall_selector` wall in the strip
/// beside the footprint on either side of it (as long as the footprint,
/// narrowed by `inset` at both ends), within `reach` of the centre line.
/// With `sides` `nearest` the nearer of the two is judged; with `both`
/// each side is judged on its own.
///
/// A distance is an interval: a sure wall surely closer than `minimum` is
/// "too close", every wall that may be there farther than `maximum` "too
/// far", no wall within `reach` "no wall nearby"; a pass needs every wall
/// that may be there no closer than `minimum` and a sure wall no farther
/// than `maximum`. Anything else is not evaluated.
pub struct CentreLineDistance;

const ID: &str = "axioval:capability.centre-line-distance";

#[derive(Clone, Copy)]
enum Line {
    Long,
    Short,
    AgainstWall,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sides {
    Nearest,
    Both,
}

struct Config {
    line: Line,
    sides: Sides,
    minimum: Option<f64>,
    maximum: Option<f64>,
    reach: f64,
    inset: f64,
}

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => {
            Ok(Some(value))
        }
        Some((_, QuantityDimension::Length)) => Err(invalid(format!(
            "`{name}` must be a finite length, not negative"
        ))),
        Some(_) => Err(invalid(format!("`{name}` is not a length"))),
    }
}

impl Config {
    fn parse(rule: &CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let line = match parameters.required_string("centre_line")? {
            "long" => Line::Long,
            "short" => Line::Short,
            "against-wall" => Line::AgainstWall,
            other => {
                return Err(invalid(format!(
                    "centre_line `{other}` is unsupported; use `long`, `short` or `against-wall`"
                )));
            }
        };
        let sides = match parameters.required_string("sides")? {
            "nearest" => Sides::Nearest,
            "both" => Sides::Both,
            other => {
                return Err(invalid(format!(
                    "sides `{other}` is unsupported; use `nearest` or `both`"
                )));
            }
        };
        let (minimum, maximum) = (
            length(&parameters, "minimum")?,
            length(&parameters, "maximum")?,
        );
        let reach = length(&parameters, "reach")?
            .filter(|reach| *reach > 0.0)
            .ok_or_else(|| invalid("`reach` is required and must be positive"))?;
        match (minimum, maximum) {
            (None, None) => return Err(invalid("declare `minimum`, `maximum` or both")),
            (Some(minimum), Some(maximum)) if minimum > maximum => {
                return Err(invalid("`minimum` exceeds `maximum`"));
            }
            _ => {}
        }
        if minimum
            .into_iter()
            .chain(maximum)
            .any(|bound| bound > reach)
        {
            return Err(invalid(
                "`reach` must be at least `minimum` and `maximum`: a wall beyond it is none",
            ));
        }
        Ok(Self {
            line,
            sides,
            minimum,
            maximum,
            reach,
            inset: length(&parameters, "inset")?.unwrap_or(0.0),
        })
    }
}

impl RuleCapability for CentreLineDistance {
    fn id(&self) -> &'static str {
        ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("wall_selector", ParameterType::Selector),
            ParameterDescriptor::required("centre_line", ParameterType::String),
            ParameterDescriptor::required("sides", ParameterType::String),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
            ParameterDescriptor::required("reach", ParameterType::Quantity),
            ParameterDescriptor::optional("inset", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("centre-line-distance: {message}"),
                );
            }
        };
        let Some(spans) = context.services.get::<PlanSpanServiceHandle>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "centre-line-distance needs the plan-span service",
            );
        };
        let walls = match Parameters(rule).required_selector("wall_selector") {
            Ok(selector) => Walls::select(context, selector),
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("centre-line-distance: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            for (label, result) in check(&config, spans, &walls, object) {
                match result {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!("{label}: {message}"),
                        evidence,
                        related,
                    )),
                    Err((reason, message)) => evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        format!("{label}: {message}"),
                    ),
                }
            }
        }
        evaluation
    }
}

type Judged = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

/// One result per judged side (or one for the nearer of the two).
fn check(
    config: &Config,
    spans: &PlanSpanServiceHandle,
    walls: &Walls,
    object: &Object,
) -> Vec<(String, Judged)> {
    let label = "centre line";
    let measured = match walls.measure(spans, &object.id, config.reach, config.inset) {
        Ok(measured) => measured,
        Err(error) => return vec![(label.to_owned(), Err(error))],
    };
    let (axis, front, mut evidence) = match line(config, walls, &measured) {
        Ok(found) => found,
        Err(error) => return vec![(label.to_owned(), Err(error))],
    };
    evidence.push(measured.evidence().clone());
    evidence.push(measured.rectangle().evidence().clone());
    // The two sides facing square to the centre line, named left and right
    // when the front is known.
    let across = 1 - axis;
    let pair = [RectangleSide::ALL[across], RectangleSide::ALL[across + 2]];
    let named = |side: RectangleSide| match front {
        Some(front) => {
            let out = side.outward(measured.rectangle());
            // Facing along the front, left is a quarter turn anticlockwise.
            if front[0] * out[1] - front[1] * out[0] > 0.0 {
                format!("{label} to the left")
            } else {
                format!("{label} to the right")
            }
        }
        None => format!("{label} beside the {} side", side.name()),
    };
    match config.sides {
        Sides::Both => pair
            .into_iter()
            .map(|side| {
                let nearest = walls.nearest(&measured, side);
                (named(side), judge(config, &nearest, &evidence))
            })
            .collect(),
        Sides::Nearest => {
            let [first, second] = pair.map(|side| walls.nearest(&measured, side));
            let lower = match (first.lower, second.lower) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let sure = match (first.sure, second.sure) {
                (Some(a), Some(b)) => Some(if b.2 < a.2 { b } else { a }),
                (a, b) => a.or(b),
            };
            vec![(
                format!("{label} to the nearest wall"),
                judge(config, &Nearest { lower, sure }, &evidence),
            )]
        }
    }
}

/// The axis a centre line runs along, the front (for `against-wall`) and
/// the evidence deriving them.
type CentreLine = (usize, Option<[f64; 2]>, Vec<Evidence>);

/// The axis the centre line runs along, the front (for `against-wall`)
/// and the evidence deriving them.
fn line(
    config: &Config,
    walls: &Walls,
    measured: &SideDistances,
) -> Result<CentreLine, Unavailable> {
    let rectangle = measured.rectangle();
    match config.line {
        Line::Long | Line::Short => {
            let long = rectangle.long_axis().map_err(|reason| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the footprint has no long axis: {reason}"),
                )
            })?;
            let axis = if matches!(config.line, Line::Long) {
                long
            } else {
                1 - long
            };
            Ok((axis, None, Vec::new()))
        }
        Line::AgainstWall => {
            let back = walls.back(measured)?;
            let front = back.side.opposite().outward(rectangle);
            Ok((back.side.axis(), Some(front), back.evidence))
        }
    }
}

fn judge(config: &Config, nearest: &Nearest, evidence: &[Evidence]) -> Judged {
    let cited = || evidence.to_vec();
    let Some(lower) = nearest.lower else {
        return Ok(Some((
            format!("no wall nearby (none within {})", metres(config.reach)),
            cited(),
            Vec::new(),
        )));
    };
    if let (Some(minimum), Some((wall, low, high))) = (config.minimum, &nearest.sure) {
        if *high < minimum {
            return Ok(Some((
                format!(
                    "too close: {} from {wall}, less than the minimum {}",
                    shown(*low, *high),
                    metres(minimum)
                ),
                cited(),
                vec![wall.clone()],
            )));
        }
    }
    if let Some(maximum) = config.maximum {
        if lower > maximum {
            let message = match &nearest.sure {
                Some((wall, low, high)) => format!(
                    "too far: {} from {wall}, more than the maximum {}",
                    shown(*low, *high),
                    metres(maximum)
                ),
                None => format!("too far: no wall within the maximum {}", metres(maximum)),
            };
            let related = nearest
                .sure
                .iter()
                .map(|(wall, _, _)| wall.clone())
                .collect();
            return Ok(Some((message, cited(), related)));
        }
    }
    let near_enough = config.minimum.is_none_or(|minimum| lower >= minimum);
    let far_enough = config.maximum.is_none_or(|maximum| {
        nearest
            .sure
            .as_ref()
            .is_some_and(|(_, _, high)| *high <= maximum)
    });
    if near_enough && far_enough {
        return Ok(None);
    }
    Err((
        NotEvaluatedReason::IncompleteEvidence,
        match &nearest.sure {
            Some((wall, low, high)) => format!(
                "the nearest wall lies {} from the centre line ({wall} {}), which does not \
                 decide the bounds",
                shown(lower, *high),
                shown(*low, *high)
            ),
            None => format!(
                "a wall may lie {} or farther from the centre line, but none surely does",
                metres(lower)
            ),
        },
    ))
}
