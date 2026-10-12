//! Roof feature lines as a measured member list (`roof_features`): the
//! lines where a roof body's upper faces meet each other or end, each
//! classified as a ridge, hip, valley, eaves, verge or abutment, with its
//! length and the height of its lowest and highest point.
//!
//! The geometry service lists the edges where the body's pieces meet
//! ([`VerticalExtentServiceHandle::measure_face_edges`]): each line's ends
//! as boxes and the faces beside it, their normal boxes and the side of
//! the line each lies on. A face is an upper face where its normal leans
//! less than `90° - tolerance` from the vertical. Every predicate is taken
//! with sound interval arithmetic over the boxes, three-valued:
//!
//! - two upper faces meeting at a convex line make a ridge where the line
//!   is level (within the tolerance) and a hip where it is not; meeting at
//!   a concave line, a valley;
//! - an upper face meeting a face that is not one at a concave line (a
//!   wall or a dormer's cheek rising from the roof) makes an abutment;
//! - an upper face ending at a convex line or an open surface's border
//!   makes a verge where the line is sloped, and where it is level eaves
//!   if the face rises away from it and a ridge (a mono-pitch roof's top)
//!   if it falls away;
//! - a line between faces neither of which is an upper face is no roof
//!   feature.
//!
//! A line is classified only where every assignment the undecided
//! predicates allow gives the same class. Two faces meeting within the
//! tolerance of flat, and a level line along a face within the tolerance of
//! level, are never classified: such a line is listed undecided, a possible
//! member of every list it might belong to, never guessed.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::f64::consts::PI;

use axioval_engine::expression::Interval;
use axioval_engine::{
    EdgeSide, FaceEdge, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, ObjectFrameServiceHandle, PointBox, PropertyResolutionError, RuleContext,
    VerticalExtentServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use crate::body_extent::{extent_error, frame_error};
use crate::measured_kinds::{interval, refused};
use crate::support::{Traversal, Unavailable, invalid};

/// Measures `roof_features`.
pub(crate) struct RoofFeatureMeasures;

const LIST: &str = "roof_features";

/// The class of a roof feature line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Feature {
    Ridge,
    Hip,
    Valley,
    Eaves,
    Verge,
    Abutment,
}

impl Feature {
    /// The class's name, as `feature` chooses it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ridge => "ridge",
            Self::Hip => "hip",
            Self::Valley => "valley",
            Self::Eaves => "eaves",
            Self::Verge => "verge",
            Self::Abutment => "abutment",
        }
    }

    /// The class `name` chooses.
    pub(crate) fn named(name: &str) -> Option<Self> {
        [
            Self::Ridge,
            Self::Hip,
            Self::Valley,
            Self::Eaves,
            Self::Verge,
            Self::Abutment,
        ]
        .into_iter()
        .find(|feature| feature.name() == name)
    }
}

/// What a line may be: a class, no roof feature, or nothing the geometry
/// can tell, with why.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Feature(Feature),
    None,
    Unclassifiable(&'static str),
}

/// A three-valued predicate: its possible values.
fn possible(lower_above: bool, upper_below: bool) -> &'static [bool] {
    // Surely true where the whole interval clears the bound, surely false
    // where it stays short of it, otherwise either.
    match (lower_above, upper_below) {
        (true, _) => &[true],
        (_, true) => &[false],
        _ => &[false, true],
    }
}

/// How a face runs away from a line: up, down, or within the tolerance of
/// level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Rises,
    Falls,
    Level,
}

type Vector = [Interval; 3];

fn of_box(lower: [f64; 3], upper: [f64; 3]) -> Vector {
    [0, 1, 2].map(|axis| Interval {
        lower: lower[axis],
        upper: upper[axis],
    })
}

fn cross(a: &Vector, b: &Vector) -> Option<Vector> {
    let component = |i: usize, j: usize| a[i].times(b[j]).ok()?.minus(a[j].times(b[i]).ok()?).ok();
    Some([component(1, 2)?, component(2, 0)?, component(0, 1)?])
}

fn dot(a: &Vector, b: &Vector) -> Option<Interval> {
    let mut sum = Interval::point(0.0);
    for axis in 0..3 {
        sum = sum.plus(a[axis].times(b[axis]).ok()?).ok()?;
    }
    Some(sum)
}

fn norm(a: &Vector) -> Option<Interval> {
    let mut sum = Interval::point(0.0);
    for component in a {
        let magnitude = component.abs();
        sum = sum.plus(magnitude.times(magnitude).ok()?).ok()?;
    }
    sum.sqrt().ok()
}

/// `a·b / (|a||b|)`, clamped to `[-1, 1]`.
fn cosine(a: &Vector, b: &Vector) -> Option<Interval> {
    let value = dot(a, b)?.divided_by(norm(a)?.times(norm(b)?).ok()?).ok()?;
    Some(Interval {
        lower: value.lower.clamp(-1.0, 1.0),
        upper: value.upper.clamp(-1.0, 1.0),
    })
}

/// The `z` component of `a` over its length.
fn rise(a: &Vector) -> Option<Interval> {
    a[2].divided_by(norm(a)?).ok()
}

/// The tolerance's sine and cosine.
struct Tolerance {
    sine: Interval,
    cosine: Interval,
}

impl Tolerance {
    fn of(degrees: f64) -> Option<Self> {
        let pi = Interval {
            lower: PI,
            upper: PI.next_up(),
        };
        let radians = Interval::point(degrees)
            .times(pi)
            .ok()?
            .divided_by(Interval::point(180.0))
            .ok()?;
        Some(Self {
            sine: radians.sin(),
            cosine: radians.cos(),
        })
    }
}

/// The possible values of the predicates a line's class reads.
struct Predicates {
    /// Whether each side is an upper face.
    upper: Vec<&'static [bool]>,
    /// How each side runs away from the line.
    runs: Vec<Vec<Run>>,
    /// Whether two faces are bent apart by more than the tolerance.
    bent: &'static [bool],
    /// Whether the line is convex (the second face below the first's
    /// plane).
    convex: &'static [bool],
    /// Whether the line is level within the tolerance.
    level: &'static [bool],
}

/// Either value, where an interval computation fails.
const EITHER: &[bool] = &[false, true];

fn predicates(edge: &FaceEdge, tolerance: &Tolerance) -> Predicates {
    let (start, end) = (edge.start(), edge.end());
    let along: Option<Vector> = (|| {
        let (from, to) = (
            of_box(start.lower(), start.upper()),
            of_box(end.lower(), end.upper()),
        );
        Some([
            to[0].minus(from[0]).ok()?,
            to[1].minus(from[1]).ok()?,
            to[2].minus(from[2]).ok()?,
        ])
    })();
    let normals: Vec<Vector> = edge
        .sides()
        .iter()
        .map(|side| of_box(side.normal().lower(), side.normal().upper()))
        .collect();
    let sine = tolerance.sine;
    let upper = normals
        .iter()
        .map(|normal| {
            rise(normal).map_or(EITHER, |rise| {
                possible(rise.lower > sine.upper, rise.upper <= sine.lower)
            })
        })
        .collect();
    // The direction from the line into each face, square to it.
    let across = |side: &EdgeSide, normal: &Vector| -> Option<Vector> {
        let along = along.as_ref()?;
        if side.left() {
            cross(normal, along)
        } else {
            cross(along, normal)
        }
    };
    let runs = edge
        .sides()
        .iter()
        .zip(&normals)
        .map(|(side, normal)| {
            let Some(rise) = across(side, normal).and_then(|into| rise(&into)) else {
                return vec![Run::Rises, Run::Falls, Run::Level];
            };
            let mut runs = Vec::new();
            if rise.upper > sine.lower {
                runs.push(Run::Rises);
            }
            if rise.lower < -sine.lower {
                runs.push(Run::Falls);
            }
            if rise.lower <= sine.upper && rise.upper >= -sine.upper {
                runs.push(Run::Level);
            }
            runs
        })
        .collect();
    let (bent, convex) = match (edge.sides(), normals.as_slice()) {
        ([_, second], [first_normal, second_normal]) => {
            let bent = cosine(first_normal, second_normal).map_or(EITHER, |cosine| {
                possible(
                    cosine.upper < tolerance.cosine.lower,
                    cosine.lower >= tolerance.cosine.upper,
                )
            });
            let convex = across(second, second_normal)
                .and_then(|into| dot(first_normal, &into))
                .map_or(EITHER, |height| {
                    possible(height.upper < 0.0, height.lower > 0.0)
                });
            (bent, convex)
        }
        _ => (&[true][..], &[true][..]),
    };
    let level = along.as_ref().and_then(rise).map_or(EITHER, |rise| {
        let slope = rise.abs();
        possible(slope.upper <= sine.lower, slope.lower > sine.upper)
    });
    Predicates {
        upper,
        runs,
        bent,
        convex,
        level,
    }
}

/// The class of a line for one assignment of its predicates.
fn class(upper: &[bool], runs: &[Run], bent: bool, convex: bool, level: bool) -> Outcome {
    let border = |run: Run| match (level, run) {
        (false, _) => Outcome::Feature(Feature::Verge),
        (true, Run::Rises) => Outcome::Feature(Feature::Eaves),
        (true, Run::Falls) => Outcome::Feature(Feature::Ridge),
        (true, Run::Level) => Outcome::Unclassifiable(
            "the face runs within the tolerance of level away from the line",
        ),
    };
    match upper {
        [false] | [false, false] => Outcome::None,
        [true] => border(runs[0]),
        _ if !bent => Outcome::Unclassifiable("the faces meet within the tolerance of flat"),
        [true, true] => match (convex, level) {
            (true, true) => Outcome::Feature(Feature::Ridge),
            (true, false) => Outcome::Feature(Feature::Hip),
            (false, _) => Outcome::Feature(Feature::Valley),
        },
        _ if !convex => Outcome::Feature(Feature::Abutment),
        [true, false] => border(runs[0]),
        _ => border(runs[1]),
    }
}

/// Every class a line may have, over all assignments its predicates allow.
fn outcomes(predicates: &Predicates) -> BTreeSet<Outcome> {
    let mut found = BTreeSet::new();
    let sides = predicates.upper.len();
    let mut uppers: Vec<Vec<bool>> = vec![Vec::new()];
    let mut runs: Vec<Vec<Run>> = vec![Vec::new()];
    for side in 0..sides {
        uppers = uppers
            .iter()
            .flat_map(|chosen| {
                predicates.upper[side].iter().map(move |value| {
                    let mut next = chosen.clone();
                    next.push(*value);
                    next
                })
            })
            .collect();
        runs = runs
            .iter()
            .flat_map(|chosen| {
                predicates.runs[side].iter().map(move |value| {
                    let mut next = chosen.clone();
                    next.push(*value);
                    next
                })
            })
            .collect();
    }
    for upper in &uppers {
        for run in &runs {
            for &bent in predicates.bent {
                for &convex in predicates.convex {
                    for &level in predicates.level {
                        found.insert(class(upper, run, bent, convex, level));
                    }
                }
            }
        }
    }
    found
}

/// A coordinate's middle, in millimetres, as words name a point.
fn words(point: PointBox) -> String {
    let (lower, upper) = (point.lower(), point.upper());
    let [x, y, z] = [0, 1, 2].map(|axis| {
        let millimetres = (f64::midpoint(lower[axis], upper[axis]) * 1000.0).round() / 1000.0;
        // `-0` reads as `0`.
        millimetres + 0.0
    });
    format!("({x}, {y}, {z})")
}

/// The words a class's possible outcomes read in.
fn alternatives(outcomes: &BTreeSet<Outcome>) -> String {
    let mut names: Vec<&str> = Vec::new();
    let mut reasons: Vec<&str> = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Feature(feature) => names.push(feature.name()),
            Outcome::None => names.push("no roof feature"),
            Outcome::Unclassifiable(why) => reasons.push(why),
        }
    }
    let mut text = String::new();
    if names.len() > 1 {
        text.push_str("it may be a ");
        text.push_str(&names.join(" or a ").replace("a no ", "no "));
    }
    for reason in reasons {
        if !text.is_empty() {
            text.push_str("; ");
        }
        text.push_str(reason);
    }
    text
}

/// The elevation heights are measured from: the level's the path reaches,
/// or the source's zero.
fn datum(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(f64, Vec<Evidence>), Unavailable> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("level_path") else {
        return Ok((0.0, Vec::new()));
    };
    let frames = context
        .services
        .get::<ObjectFrameServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "the object-frame service is not registered".to_owned(),
            )
        })?;
    let universe: Vec<_> = context.project.objects().collect();
    let (reached, mut evidence) = Traversal::path(steps)?.related(context, object, &universe)?;
    let mut elevation: Option<(ObjectId, f64)> = None;
    for level in reached {
        let frame = frames.object_frame(&level).map_err(|error| {
            let (reason, why) = frame_error(&error);
            (reason, format!("level {level}: {why}"))
        })?;
        if !frame.evidence().exact {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the placement of level {level} is not stated exactly"),
            ));
        }
        let height = frame.frame().origin().coordinates_metres()[2];
        evidence.push(frame.evidence().clone());
        match &elevation {
            None => elevation = Some((level, height)),
            #[allow(clippy::float_cmp)]
            Some((first, other)) if *other != height => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the path from {object} reaches levels {first} and {level} at different \
                         elevations"
                    ),
                ));
            }
            Some(_) => {}
        }
    }
    let (_, height) = elevation.ok_or_else(|| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("the path from {object} reaches no level"),
        )
    })?;
    Ok((height, evidence))
}

/// Whether a line of `outcomes` (surely of the class `decided`, if any)
/// is a member of the list of `chosen` (every class without one): surely
/// (`Some(true)`), possibly (`Some(false)`), or not at all (`None`).
fn membership(
    outcomes: &BTreeSet<Outcome>,
    decided: Option<Feature>,
    chosen: Option<Feature>,
) -> Option<bool> {
    let unclassifiable = outcomes
        .iter()
        .any(|outcome| matches!(outcome, Outcome::Unclassifiable(_)));
    let included = match chosen {
        Some(feature) => outcomes.contains(&Outcome::Feature(feature)) || unclassifiable,
        None => !outcomes.iter().all(|outcome| *outcome == Outcome::None),
    };
    included.then_some(match (chosen, decided) {
        (Some(feature), Some(found)) => feature == found,
        (None, Some(_)) => true,
        _ => false,
    })
}

/// The items of one roof: its feature lines in the service's order.
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let chosen = match call.choice("feature") {
        Some(name) => Some(
            Feature::named(name).ok_or_else(|| invalid(format!("`feature` {name} is unknown")))?,
        ),
        None => None,
    };
    let degrees = match call.argument("tolerance") {
        Some(MeasuredArgument::Number(degrees)) => *degrees,
        _ => return Err(invalid("`tolerance` is required")),
    };
    let tolerance = Tolerance::of(degrees).ok_or_else(|| invalid("`tolerance` is no angle"))?;
    let service = context
        .services
        .get::<VerticalExtentServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "the vertical-extent service is not registered".to_owned(),
            )
        })?;
    let (datum, datum_evidence) = datum(call, object, context)?;
    let edges = service.measure_face_edges(object).map_err(|error| {
        let (reason, why) = extent_error(&error);
        (reason, format!("{object}: {why}"))
    })?;
    let exact = edges.evidence().exact;
    let length = Some(QuantityDimension::Length);
    let mut items = Vec::new();
    for (index, edge) in edges.edges().iter().enumerate() {
        let outcomes = outcomes(&predicates(edge, &tolerance));
        let decided = match outcomes.iter().collect::<Vec<_>>().as_slice() {
            [Outcome::Feature(feature)] => Some(*feature),
            _ => None,
        };
        let Some(certain) = membership(&outcomes, decided, chosen) else {
            continue;
        };
        let (start, end) = (edge.start(), edge.end());
        let line = format!("{} - {}", words(start), words(end));
        let locator = |field: &str| format!("{LIST}:{object}:{index}:{field}");
        let (feature, what) = match decided {
            Some(feature) => (
                MemberValue::Text {
                    text: feature.name().to_owned(),
                },
                format!("the {} {line}", feature.name()),
            ),
            None => (
                MemberValue::Undecided {
                    why: format!(
                        "the line {line} of {object} cannot be classified: {}",
                        alternatives(&outcomes)
                    ),
                },
                format!("the line {line}"),
            ),
        };
        let (from, to) = (
            of_box(start.lower(), start.upper()),
            of_box(end.lower(), end.upper()),
        );
        let along: Option<Vector> = (|| {
            Some([
                to[0].minus(from[0]).ok()?,
                to[1].minus(from[1]).ok()?,
                to[2].minus(from[2]).ok()?,
            ])
        })();
        let span = along.as_ref().and_then(norm);
        let measured = |value: Option<Interval>, field: &str| match value {
            Some(value) => MemberValue::Measured(interval(
                (value.lower, value.upper),
                length,
                exact,
                locator(field),
            )),
            None => MemberValue::Undecided {
                why: format!("the {field} of the line {line} of {object} cannot be bounded"),
            },
        };
        let above = |value: Interval| value.minus(Interval::point(datum)).ok();
        let low = above(from[2].min(to[2]));
        let high = above(from[2].max(to[2]));
        let mut evidence = vec![edges.evidence().clone()];
        evidence.extend(datum_evidence.iter().cloned());
        items.push(MeasuredMember {
            certain,
            exact,
            evidence,
            fields: BTreeMap::from([
                ("feature", feature),
                ("what", MemberValue::Text { text: what }),
                ("length", measured(span, "length")),
                ("low", measured(low, "low")),
                ("high", measured(high, "high")),
            ]),
        });
    }
    Ok(items)
}

impl MeasuredProvider for RoofFeatureMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LIST]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        items(call, object, context).map_err(refused(call.name(), object))
    }
}
