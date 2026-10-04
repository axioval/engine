//! Positions along IFC4X3 alignments: the [`AlignmentService`] of the
//! geometry bridge.
//!
//! Each `IfcAlignment` is read as its 3D centreline, an elevated curve (a
//! plan measured by its own length and a gradient line written against
//! that length): the curve its `Axis` representation holds, lowered by
//! `ifc-geometry` exactly as a linear placement's basis curve is, else the
//! curve `ifc-alignment` composes from its horizontal and vertical layouts.
//! A segmented reference curve is read by its gradient line. The curve is
//! in the alignment's own frame, so a point is carried into it through the
//! alignment's placement (and its representation context's frame), which
//! must be rigid with `+Z` up.
//!
//! # Locating a point
//!
//! The object's reference point is its placement origin, as the bridge
//! places the object (a linear placement derived from its basis curve).
//! There is no point-to-curve projection in the kernel, so the foot is
//! found here and certified. In plan, a foot at distance `d` is a root of
//!
//! ```text
//! g(d) = (P - c(d)) . c'(d)
//! ```
//!
//! and `|g'| <= B1^2 + R B2` on a stretch, where `B1` and `B2` bound the
//! curve's first and second derivatives there (`axiolid_evaluate::bound::
//! curve_derivative_bounds3`, which bounds an elevated curve's plan and
//! profile together, so its plan part too) and `R` bounds `|P - c|`. The
//! whole length is bisected, split first where the curve may not be `C^2`
//! (`continuity_breaks3`): a stretch whose midpoint value exceeds that
//! Lipschitz bound times its half-width holds no foot and is dropped, so
//! what remains after bisecting to [`RESOLUTION`] are brackets every foot
//! lies in. A bracket where `g` falls from positive to negative holds a
//! nearest point (the plan distance has a local minimum there); one where
//! it rises holds a farthest point and is dropped; any other is undecided.
//! The ends are candidates too: the start when the distance grows into the
//! curve, the end when it shrinks towards it.
//!
//! The nearest candidate wins only when its distance interval lies wholly
//! below every other's: two that overlap are ambiguous, an undecided one
//! nearest cannot be decided, and an end nearest leaves the point off the
//! range. Station, offset and height are evaluated at the winning
//! bracket's ends and widened by the same bounds over it, so each is an
//! interval holding the exact value. Every evaluation is allowed
//! [`slack`]: the reference evaluator's stated accuracy and rounding.
//!
//! # Parameters
//!
//! Over a stretch of plan distances: the plan curvature from its law (a
//! line, constant and polynomial curvature, piecewise of those, in an
//! intrinsic curve or a chain of intrinsic pieces), signed positive to the
//! left; the gradient from the elevation law (polynomials, circular arcs,
//! piecewise of those), whose grade is monotone on each arc; and the cant
//! from the alignment's `IfcAlignmentCant` segments, whose every transition
//! shape is monotone along its segment, so each rail's elevation is bounded
//! by its values at the stretch's ends within each segment. Other laws are
//! refused by name. An alignment nesting no cant layout states no cant.

use std::collections::{BTreeMap, BTreeSet};

use axiolid_curve::{ChainPiece2, CurvatureLaw, Curve2, Curve3, Elevated3, ElevationLaw};
use axiolid_evaluate::bound::{continuity_breaks3, curve_derivative_bounds3};
use axiolid_evaluate::{elevated_derivative, elevated_point, elevation_grade, station_length3};
use axioval::engine::{
    AlignmentError, AlignmentInterval, AlignmentParameter, AlignmentParameterRequest,
    AlignmentParameterValue, AlignmentPosition, AlignmentRequest, AlignmentService,
};
use axioval::ir::{Evidence, ObjectId, SourceId};
use ifc_alignment::{AlignmentError as IfcAlignmentError, AlignmentUnits, CantLayout, Stationing};
use ifc_geometry::Transform;
use ifc_model::{EntityId, Model};

use super::{PRODUCT_REPRESENTATION, Parsed, context_frame, entity_id, session, world_transform};

/// Width, in metres of plan distance, below which a bracket is no longer
/// split.
const RESOLUTION: f64 = 1e-7;

/// Evaluations one location may spend before it is refused.
const BUDGET: usize = 400_000;

/// How far, in each component, a frame may lie from rigid with `+Z` up.
const RIGID: f64 = 1e-9;

/// The error every evaluation is allowed, in metres: the reference
/// evaluator's stated accuracy (its quadrature reproduces a clothoid to
/// machine precision) and rounding, at `scale`, the largest coordinate or
/// length involved.
fn slack(scale: f64) -> f64 {
    1e-9 * scale.max(1.0)
}

/// Every alignment of the session's sources read as a centreline, and the
/// reference point of every object of a source that has one.
pub(super) struct IfcAlignmentService {
    points: BTreeMap<ObjectId, Result<[f64; 3], String>>,
    alignments: BTreeMap<ObjectId, Result<Centreline, String>>,
}

/// The service over `objects`, read from `parsed`.
pub(super) fn alignment_service(
    parsed: &BTreeMap<SourceId, Parsed>,
    objects: &[ObjectId],
) -> IfcAlignmentService {
    let mut alignments = BTreeMap::new();
    for id in objects {
        let (Some(source), Some(entity)) = (parsed.get(&id.source), entity_id(id)) else {
            continue;
        };
        let is_alignment = source
            .model
            .get(entity)
            .is_some_and(|found| found.type_name.eq_ignore_ascii_case("IFCALIGNMENT"));
        if is_alignment {
            alignments.insert(id.clone(), Centreline::read(source, entity));
        }
    }
    let sources: BTreeSet<&SourceId> = alignments.keys().map(|id| &id.source).collect();
    let mut points = BTreeMap::new();
    for id in objects {
        if !sources.contains(&id.source) || alignments.contains_key(id) {
            continue;
        }
        let (Some(source), Some(entity)) = (parsed.get(&id.source), entity_id(id)) else {
            continue;
        };
        let point = match source.linear.own_refusal(&source.model, entity) {
            Some(reason) => Err(reason),
            None => world_transform(&source.model, &source.units, entity)
                .map(|placed| placed.origin)
                .map_err(|error| error.to_string()),
        };
        points.insert(id.clone(), point);
    }
    IfcAlignmentService { points, alignments }
}

/// An alignment's 3D centreline and what labels and cants it.
struct Centreline {
    curve: Curve3,
    /// Plan length.
    length: f64,
    /// Where the curve may not be `C^2`, strictly inside `(0, length)`.
    breaks: Vec<f64>,
    /// World to the curve's frame; `None` at the identity.
    frame: Option<Transform>,
    /// The station equations, `None` when it states none.
    stationing: Result<Option<Stationing>, String>,
    /// The cant layout, `None` when it nests none.
    cant: Result<Option<CantLayout>, String>,
    /// Where the curve was read from.
    locator: String,
}

impl Centreline {
    fn read(source: &Parsed, alignment: EntityId) -> Result<Self, String> {
        let (model, units) = (&source.model, &source.units);
        let alignment_units = AlignmentUnits {
            length_to_metres: units.length_to_metres,
            angle_to_radians: units.angle_to_radians,
        };
        let placement = if ifc_geometry::Slots::new(
            alignment,
            model.get(alignment).ok_or("the alignment does not exist")?,
        )
        .opt_ref(super::OBJECT_PLACEMENT)
        .is_some()
        {
            world_transform(model, units, alignment)
                .map_err(|error| format!("its placement cannot be resolved ({error})"))?
        } else {
            Transform::identity()
        };
        let (curve, frame, locator) = match axis_curve(source, alignment)? {
            Some((curve, representation, item)) => {
                let context = context_frame(model, units, representation).ok_or_else(|| {
                    format!(
                        "the context of its axis representation {representation} cannot be read"
                    )
                })?;
                (
                    curve,
                    context.compose(&placement),
                    format!("{alignment} (IFCALIGNMENT) axis {item}"),
                )
            }
            None => (
                ifc_alignment::gradient_curve3(model, alignment, alignment_units).map_err(
                    |error| {
                        format!(
                            "it has no Axis curve, and its layouts compose no centreline ({error})"
                        )
                    },
                )?,
                placement,
                format!("{alignment} (IFCALIGNMENT) layouts"),
            ),
        };
        let curve = match curve {
            Curve3::Elevated(_) => curve,
            Curve3::Banked(banked) => Curve3::Elevated(banked.base),
            _ => {
                return Err(
                    "its axis is no gradient curve (a plan with a vertical profile)".into(),
                );
            }
        };
        let length = match station_length3(&curve) {
            Ok(Some(length)) if length.is_finite() && length > 0.0 => length,
            Ok(_) => return Err("its centreline has no finite length".into()),
            Err(error) => return Err(format!("its centreline has no length ({error})")),
        };
        let breaks = continuity_breaks3(&curve, 2)
            .into_iter()
            .filter(|at| *at > 0.0 && *at < length)
            .collect();
        let stationing = match Stationing::resolve(model, alignment, alignment_units) {
            Ok(stationing) if stationing.equations().is_empty() => Ok(None),
            Ok(stationing) => Ok(Some(stationing)),
            Err(error) => Err(format!("its stationing cannot be read ({error})")),
        };
        let cant = match CantLayout::for_alignment(model, alignment, alignment_units) {
            Ok(layout) => Ok(Some(layout)),
            Err(IfcAlignmentError::SemanticViolation {
                rule: "the alignment nests no IfcAlignmentCant layout",
                ..
            }) => Ok(None),
            Err(error) => Err(format!("its cant cannot be read ({error})")),
        };
        Ok(Self {
            curve,
            length,
            breaks,
            frame: rigid(&frame)?,
            stationing,
            cant,
            locator,
        })
    }

    fn elevated(&self) -> &Elevated3 {
        match &self.curve {
            Curve3::Elevated(elevated) => elevated,
            _ => unreachable!("a centreline is elevated"),
        }
    }

    /// `point`, in world coordinates, in the curve's frame.
    fn to_local(&self, point: [f64; 3]) -> [f64; 3] {
        let Some(frame) = &self.frame else {
            return point;
        };
        let relative: [f64; 3] = std::array::from_fn(|i| point[i] - frame.origin[i]);
        std::array::from_fn(|axis| {
            (0..3)
                .map(|i| frame.basis[axis][i] * relative[i])
                .sum::<f64>()
        })
    }
}

/// The single curve of `alignment`'s `Axis` representation, lowered, with
/// that representation and item; `None` when it has none.
fn axis_curve(
    source: &Parsed,
    alignment: EntityId,
) -> Result<Option<(Curve3, EntityId, EntityId)>, String> {
    let model = &source.model;
    let Some(shape) = model.get(alignment).and_then(|entity| {
        ifc_geometry::Slots::new(alignment, entity).opt_ref(PRODUCT_REPRESENTATION)
    }) else {
        return Ok(None);
    };
    let representations = model
        .get(shape)
        .and_then(|entity| {
            ifc_geometry::ProductShape::new(shape, entity)
                .representations()
                .ok()
        })
        .unwrap_or_default();
    for representation in representations {
        let Some(entity) = model.get(representation) else {
            continue;
        };
        let view = ifc_geometry::Representation::new(representation, entity);
        if !view
            .identifier()
            .is_some_and(|identifier| identifier.eq_ignore_ascii_case("Axis"))
        {
            continue;
        }
        let items = view.items().map_err(|error| error.to_string())?;
        let [item] = items.as_slice() else {
            return Err(format!(
                "its Axis representation {representation} holds {} items, not one curve",
                items.len()
            ));
        };
        return lower_curve(model, &source.units, *item)
            .map(|curve| Some((curve, representation, *item)));
    }
    Ok(None)
}

/// `item` lowered as one neutral 3D curve, in metres.
fn lower_curve(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    item: EntityId,
) -> Result<Curve3, String> {
    // The bridge's session, so a curve holding a linear placement derives
    // it as every other lowering does.
    let mut session = session(model, units);
    let root =
        ifc_geometry::lower::curve::lower_curve_node(&mut session, item, Transform::identity())
            .map_err(|error| format!("its axis curve {item} cannot be lowered ({error})"))?;
    let lowered = session
        .finish(root)
        .map_err(|error| format!("its axis curve {item} cannot be lowered ({error})"))?;
    match lowered.graph.get(lowered.root) {
        Some(axiolid_model::GeometryNode::Curve3(curve)) => Ok(curve.clone()),
        _ => Err(format!(
            "its axis curve {item} lowers to no single 3D curve"
        )),
    }
}

/// `None` when the alignment's curve lies in world coordinates. An
/// alignment placed off the identity is refused: placing products along
/// it does not yet compose the alignment's placement
/// (openbimrs/ifc#357), so stations, offsets and heights read against it
/// would not match where its linearly placed products lie.
fn rigid(frame: &Transform) -> Result<Option<Transform>, String> {
    if frame.is_identity(RIGID) {
        return Ok(None);
    }
    Err(
        "it is placed off the identity, which placements along it do not yet compose \
         (openbimrs/ifc#357), so it is refused rather than misread"
            .into(),
    )
}

/// Why a point has no position along a curve.
#[derive(Debug, PartialEq)]
enum Refusal {
    OffRange(String),
    Ambiguous(String),
    Unavailable(String),
}

impl From<Refusal> for AlignmentError {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::OffRange(reason) => Self::OffRange(reason),
            Refusal::Ambiguous(reason) => Self::Ambiguous(reason),
            Refusal::Unavailable(reason) => Self::Unavailable(reason),
        }
    }
}

/// A point's foot on a centreline: intervals holding the plan distance,
/// the signed offset (left positive) and the height above the curve.
#[derive(Debug, PartialEq)]
struct Foot {
    distance: (f64, f64),
    offset: (f64, f64),
    height: (f64, f64),
}

/// What a candidate for the nearest point is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// A bracket holding a local minimum of the plan distance.
    Foot,
    /// A bracket that may hold one, undecided.
    Undecided,
    /// The start, approached from before it.
    BeforeStart,
    /// The end, approached from beyond it.
    BeyondEnd,
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    kind: Kind,
    bracket: (f64, f64),
    distance: (f64, f64),
}

/// Evaluates a centreline at plan distances against one point.
struct Probe<'c> {
    curve: &'c Curve3,
    elevated: &'c Elevated3,
    length: f64,
    breaks: &'c [f64],
    point: [f64; 3],
    slack: f64,
    spent: usize,
}

/// The point, plan derivative and height at one plan distance.
struct Sample {
    x: f64,
    y: f64,
    z: f64,
    dx: f64,
    dy: f64,
}

impl Probe<'_> {
    fn sample(&mut self, at: f64) -> Result<Sample, Refusal> {
        self.spent += 1;
        if self.spent > BUDGET {
            return Err(Refusal::Unavailable(format!(
                "locating the point spent {BUDGET} evaluations without isolating its foot \
                 (it may lie at a centre of curvature)"
            )));
        }
        let unevaluated = |error| {
            Refusal::Unavailable(format!(
                "the centreline cannot be evaluated at {at} m ({error})"
            ))
        };
        let point = elevated_point(self.elevated, at).map_err(unevaluated)?;
        let derivative = elevated_derivative(self.elevated, at).map_err(unevaluated)?;
        Ok(Sample {
            x: point.x,
            y: point.y,
            z: point.z,
            dx: derivative.x,
            dy: derivative.y,
        })
    }

    /// `g`, `|P - c|` in plan and `|c'|` in plan at a sample.
    fn measures(&self, sample: &Sample) -> (f64, f64, f64) {
        let (px, py) = (self.point[0] - sample.x, self.point[1] - sample.y);
        (
            px * sample.dx + py * sample.dy,
            px.hypot(py),
            sample.dx.hypot(sample.dy),
        )
    }

    /// Bounds `(B1, B2)` on the first two derivatives over `[a, b]`, split
    /// at the curve's breaks.
    fn bounds(&self, a: f64, b: f64) -> Result<(f64, f64), Refusal> {
        let mut cuts = vec![a];
        cuts.extend(self.breaks.iter().copied().filter(|at| *at > a && *at < b));
        cuts.push(b);
        let (mut first, mut second) = (0.0_f64, 0.0_f64);
        for pair in cuts.windows(2) {
            let bounds =
                curve_derivative_bounds3(self.curve, pair[0], pair[1]).ok_or_else(|| {
                    Refusal::Unavailable(format!(
                        "the centreline has no certified derivative bound over [{}, {}] m",
                        pair[0], pair[1]
                    ))
                })?;
            first = first.max(bounds.first);
            second = second.max(bounds.second);
        }
        Ok((first, second))
    }

    /// The tolerance `g` is read at, where `|P - c|` is at most `reach`.
    fn tolerance(&self, first: f64, reach: f64) -> f64 {
        self.slack * (1.0 + first) * (1.0 + first + reach)
    }

    /// Brackets of at most [`RESOLUTION`] that every root of `g` lies in,
    /// ascending and merged where they touch.
    fn brackets(&mut self) -> Result<Vec<(f64, f64)>, Refusal> {
        let mut cuts = vec![0.0];
        cuts.extend(self.breaks.iter().copied());
        cuts.push(self.length);
        let mut pending: Vec<(f64, f64)> = cuts
            .windows(2)
            .rev()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        let mut found: Vec<(f64, f64)> = Vec::new();
        while let Some((a, b)) = pending.pop() {
            let (first, second) = self.bounds(a, b)?;
            let middle = 0.5 * (a + b);
            let half = 0.5 * (b - a);
            let sample = self.sample(middle)?;
            let (g, reach, _) = self.measures(&sample);
            let reach = reach + first * half + self.slack;
            let lipschitz = first * first + reach * second;
            if g.abs() > lipschitz * half + self.tolerance(first, reach) {
                continue;
            }
            if b - a <= RESOLUTION || middle <= a || middle >= b {
                match found.last_mut() {
                    Some(last) if last.1 >= a => last.1 = b,
                    _ => found.push((a, b)),
                }
                continue;
            }
            pending.push((middle, b));
            pending.push((a, middle));
        }
        Ok(found)
    }

    /// The plan distance from the point to the curve over `[a, b]`.
    fn distance_over(&mut self, a: f64, b: f64) -> Result<(f64, f64), Refusal> {
        let (first, _) = self.bounds(a, b)?;
        let sample = self.sample(0.5 * (a + b))?;
        let (_, reach, _) = self.measures(&sample);
        let spread = first * 0.5 * (b - a) + self.slack;
        Ok(((reach - spread).max(0.0), reach + spread))
    }

    /// The sign of `g` at `at`, `0` within its tolerance.
    fn sign(&mut self, at: f64) -> Result<i8, Refusal> {
        let (first, _) = self.bounds(at, at)?;
        let sample = self.sample(at)?;
        let (g, reach, _) = self.measures(&sample);
        let tolerance = self.tolerance(first, reach);
        Ok(if g > tolerance {
            1
        } else if g < -tolerance {
            -1
        } else {
            0
        })
    }

    /// The nearest foot of the point, or why there is none.
    fn locate(&mut self) -> Result<Foot, Refusal> {
        let mut candidates = Vec::new();
        for (a, b) in self.brackets()? {
            let mut before = self.sign(a)?;
            let mut after = self.sign(b)?;
            // At an end, a foot within the tolerance is on the range.
            if a <= 0.0 && before == 0 {
                before = 1;
            }
            if b >= self.length && after == 0 {
                after = -1;
            }
            let kind = match (before, after) {
                (1, -1) => Kind::Foot,
                (-1, 1) => continue,
                _ => Kind::Undecided,
            };
            candidates.push(Candidate {
                kind,
                bracket: (a, b),
                distance: self.distance_over(a, b)?,
            });
        }
        if self.sign(0.0)? < 0 {
            candidates.push(Candidate {
                kind: Kind::BeforeStart,
                bracket: (0.0, 0.0),
                distance: self.distance_over(0.0, 0.0)?,
            });
        }
        if self.sign(self.length)? > 0 {
            candidates.push(Candidate {
                kind: Kind::BeyondEnd,
                bracket: (self.length, self.length),
                distance: self.distance_over(self.length, self.length)?,
            });
        }
        candidates.sort_by(|a, b| a.distance.1.total_cmp(&b.distance.1));
        let Some(nearest) = candidates.first().copied() else {
            return Err(Refusal::Ambiguous(
                "no foot on the alignment could be certified".into(),
            ));
        };
        if let Some(rival) = candidates
            .iter()
            .skip(1)
            .find(|other| other.distance.0 <= nearest.distance.1)
        {
            return Err(Refusal::Ambiguous(format!(
                "points near {} m and {} m along it are equally near, {} to {} m away",
                describe(nearest),
                describe(*rival),
                nearest.distance.0.min(rival.distance.0),
                nearest.distance.1.max(rival.distance.1)
            )));
        }
        match nearest.kind {
            Kind::Foot => self.foot(nearest.bracket),
            Kind::Undecided => Err(Refusal::Ambiguous(format!(
                "the nearest foot, near {} m along it, cannot be decided",
                describe(nearest)
            ))),
            Kind::BeforeStart => Err(Refusal::OffRange(format!(
                "the point lies before the alignment's start, {} m from it in plan",
                nearest.distance.1
            ))),
            Kind::BeyondEnd => Err(Refusal::OffRange(format!(
                "the point lies beyond the alignment's end at {} m, {} m from it in plan",
                self.length, nearest.distance.1
            ))),
        }
    }

    /// The offset and height over a foot's bracket `[a, b]`.
    fn foot(&mut self, (a, b): (f64, f64)) -> Result<Foot, Refusal> {
        let (first, second) = self.bounds(a, b)?;
        let width = b - a;
        let ends = [self.sample(a)?, self.sample(b)?];
        let mut offsets = Vec::new();
        let mut heights = Vec::new();
        let mut reach: f64 = 0.0;
        let mut speed = f64::INFINITY;
        for sample in &ends {
            let (_, distance, norm) = self.measures(sample);
            if norm <= 0.0 {
                return Err(Refusal::Unavailable(
                    "the centreline has no plan direction at the foot".into(),
                ));
            }
            let (px, py) = (self.point[0] - sample.x, self.point[1] - sample.y);
            offsets.push((-px * sample.dy + py * sample.dx) / norm);
            heights.push(self.point[2] - sample.z);
            reach = reach.max(distance);
            speed = speed.min(norm);
        }
        reach += first * width;
        speed -= second * width;
        if speed < 0.5 {
            return Err(Refusal::Unavailable(
                "the centreline is not measured by its plan length at the foot".into(),
            ));
        }
        // The unit normal turns at most |c''| / |c'|, and c' is
        // perpendicular to it, so the offset moves at most R |c''| / |c'|.
        let offset_spread = width * reach * second / speed + self.slack * (2.0 + reach);
        let height_spread = width * first + self.slack;
        let hull = |values: &[f64], spread: f64| {
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (low - spread, high + spread)
        };
        Ok(Foot {
            distance: (a.max(0.0), b.min(self.length)),
            offset: hull(&offsets, offset_spread),
            height: hull(&heights, height_spread),
        })
    }
}

/// A candidate's place along the curve, for a reason.
fn describe(candidate: Candidate) -> String {
    format!("{:.3}", 0.5 * (candidate.bracket.0 + candidate.bracket.1))
}

/// The foot of `point` (in the curve's frame) on `centreline`.
fn locate(centreline: &Centreline, point: [f64; 3]) -> Result<Foot, Refusal> {
    let scale = point
        .iter()
        .fold(centreline.length, |scale, value| scale.max(value.abs()));
    Probe {
        curve: &centreline.curve,
        elevated: centreline.elevated(),
        length: centreline.length,
        breaks: &centreline.breaks,
        point,
        slack: slack(scale),
        spent: 0,
    }
    .locate()
}

/// The stations labelling the plan distances `[a, b]`: the distances
/// themselves without station equations, else the hull of the labels at
/// both ends and on both sides of every equation inside.
fn stations(stationing: Option<&Stationing>, (a, b): (f64, f64)) -> Result<(f64, f64), String> {
    let Some(stationing) = stationing else {
        return Ok((a, b));
    };
    let at = |distance: f64| {
        stationing
            .station_at(distance)
            .map_err(|error| format!("no station labels {distance} m along it ({error})"))
    };
    let mut values = vec![at(a)?, at(b)?];
    for equation in stationing.equations() {
        if equation.distance_along > a && equation.distance_along < b {
            values.push(equation.station);
            values.extend(equation.incoming_station);
        }
    }
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // The labels are sums of a stated station and a distance: one rounding.
    Ok((low - slack(low.abs()), high + slack(high.abs())))
}

/// Outward rounding of an interval computed in floating point.
fn widen((low, high): (f64, f64)) -> (f64, f64) {
    if low.to_bits() == high.to_bits() && low == 0.0 {
        return (low, high);
    }
    let margin = 1e-12 * low.abs().max(high.abs()) + f64::MIN_POSITIVE;
    (low - margin, high + margin)
}

/// The hull of two intervals.
fn hull(a: Option<(f64, f64)>, b: (f64, f64)) -> (f64, f64) {
    a.map_or(b, |a| (a.0.min(b.0), a.1.max(b.1)))
}

/// Every value of the polynomial `coefficients` (ascending powers) over
/// `[a, b]`, by interval Horner evaluation.
fn polynomial(coefficients: &[f64], (a, b): (f64, f64)) -> (f64, f64) {
    let Some((last, rest)) = coefficients.split_last() else {
        return (0.0, 0.0);
    };
    if rest.is_empty() {
        return (*last, *last);
    }
    let mut value = (*last, *last);
    for coefficient in rest.iter().rev() {
        let products = [value.0 * a, value.0 * b, value.1 * a, value.1 * b];
        let low = products.iter().copied().fold(f64::INFINITY, f64::min);
        let high = products.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        value = widen((low + coefficient, high + coefficient));
    }
    value
}

/// A piece of a piecewise law and a stretch in its own distance.
type Piece<'l, L> = (&'l L, (f64, f64));

/// The pieces of a piecewise law overlapping `[a, b]`.
fn pieces<'l, L>(
    breaks: &[f64],
    laws: &'l [L],
    (a, b): (f64, f64),
) -> Result<Vec<Piece<'l, L>>, String> {
    if laws.len() != breaks.len() + 1 {
        return Err("a piecewise law whose pieces and seams do not match".into());
    }
    let mut out = Vec::new();
    for (index, law) in laws.iter().enumerate() {
        let start = if index == 0 { 0.0 } else { breaks[index - 1] };
        let end = breaks.get(index).copied().unwrap_or(f64::INFINITY);
        let (low, high) = (a.max(start), b.min(end));
        if low <= high {
            out.push((law, (low - start, high - start)));
        }
    }
    Ok(out)
}

/// Every curvature `law` takes over `[a, b]` of its own arc length.
fn curvature_law(law: &CurvatureLaw, stretch: (f64, f64)) -> Result<(f64, f64), String> {
    match law {
        CurvatureLaw::Constant { curvature } => Ok((*curvature, *curvature)),
        CurvatureLaw::Polynomial { coefficients } => Ok(polynomial(coefficients, stretch)),
        CurvatureLaw::Piecewise { breaks, laws } => {
            let mut out = None;
            for (law, local) in pieces(breaks, laws, stretch)? {
                out = Some(hull(out, curvature_law(law, local)?));
            }
            out.ok_or_else(|| "the stretch lies outside the curvature law".into())
        }
        _ => Err(
            "a transition whose curvature law is no polynomial (a sinusoidal spiral) has \
                  no certified bound here"
                .into(),
        ),
    }
}

/// Every plan curvature of `plan` over the plan distances `stretch`,
/// positive turning left.
fn curvature(plan: &Curve2, stretch: (f64, f64)) -> Result<(f64, f64), String> {
    let oriented = |frame: &axiolid_core::Frame2| {
        let unit = |v: axiolid_core::Vec2| (v.length() - 1.0).abs() <= RIGID;
        if !(unit(frame.x) && unit(frame.y) && frame.x.dot(frame.y).abs() <= RIGID) {
            return Err("the plan's start frame is not orthonormal".to_owned());
        }
        Ok(frame.x.perp_dot(frame.y).signum())
    };
    let signed = |sign: f64, (low, high): (f64, f64)| {
        if sign > 0.0 {
            (low, high)
        } else {
            (-high, -low)
        }
    };
    match plan {
        Curve2::Line(_) => Ok((0.0, 0.0)),
        Curve2::Intrinsic(intrinsic) => {
            let sign = oriented(&intrinsic.start)?;
            Ok(signed(sign, curvature_law(&intrinsic.curvature, stretch)?))
        }
        Curve2::Chain(chain) => {
            let sign = oriented(&chain.start)?;
            let mut start = 0.0;
            let mut out = None;
            for piece in &chain.pieces {
                let ChainPiece2::Intrinsic { curvature, length } = piece else {
                    return Err(
                        "a plan piece given as a parametric curve (a cubic parabola) has no \
                         certified curvature bound here"
                            .into(),
                    );
                };
                let end = start + length;
                let (low, high) = (stretch.0.max(start), stretch.1.min(end));
                if low <= high {
                    out = Some(hull(
                        out,
                        curvature_law(curvature, (low - start, high - start))?,
                    ));
                }
                start = end;
            }
            out.map(|value| signed(sign, value))
                .ok_or_else(|| "the stretch lies outside the plan".into())
        }
        _ => Err("the plan is no curve whose curvature law is stated".into()),
    }
}

/// Every gradient of `law` over the plan distances `stretch`.
fn gradient(law: &ElevationLaw, stretch: (f64, f64)) -> Result<(f64, f64), String> {
    match law {
        ElevationLaw::Polynomial { coefficients } => {
            let derivative: Vec<f64> = coefficients
                .iter()
                .skip(1)
                .scan(0.0, |power, coefficient| {
                    *power += 1.0;
                    Some(coefficient * *power)
                })
                .collect();
            Ok(polynomial(&derivative, stretch))
        }
        ElevationLaw::Piecewise { breaks, laws } => {
            let mut out = None;
            for (law, local) in pieces(breaks, laws, stretch)? {
                out = Some(hull(out, gradient(law, local)?));
            }
            out.ok_or_else(|| "the stretch lies outside the gradient line".into())
        }
        // The grade angle turns one way along an arc, so its tangent is
        // monotone: the ends bound it.
        ElevationLaw::CircularArc { .. } => {
            let at = |distance: f64| {
                elevation_grade(law, distance).map_err(|error| {
                    format!("the vertical arc has no grade at {distance} m ({error})")
                })
            };
            let (start, end) = (at(stretch.0)?, at(stretch.1)?);
            Ok(widen((start.min(end), start.max(end))))
        }
        _ => Err(
            "a vertical segment whose law is no polynomial or circular arc has no \
                  certified gradient bound here"
                .into(),
        ),
    }
}

/// Every cant magnitude of `layout` over the plan distances `[a, b]`.
fn cant(layout: &CantLayout, (a, b): (f64, f64)) -> Result<(f64, f64), String> {
    let mut left: Option<(f64, f64)> = None;
    let mut right: Option<(f64, f64)> = None;
    let mut covered = a;
    for segment in layout.segments() {
        let start = segment.start_dist_along;
        let end = start + segment.horizontal_length;
        let (low, high) = (a.max(start), b.min(end));
        if low > high || segment.horizontal_length <= 0.0 {
            continue;
        }
        if low > covered + RESOLUTION {
            return Err(format!("no cant segment covers {covered} m along it"));
        }
        for at in [low, high] {
            let xi = ((at - start) / segment.horizontal_length).clamp(0.0, 1.0);
            let rails = ifc_alignment::cant_at(segment, xi, Some(layout.rail_head_distance))
                .map_err(|error| {
                    format!("cant segment {} cannot be read ({error})", segment.entity)
                })?;
            left = Some(hull(left, (rails.left, rails.left)));
            right = Some(hull(right, (rails.right, rails.right)));
        }
        covered = covered.max(high);
    }
    let (Some(left), Some(right)) = (left, right) else {
        return Err(format!("no cant segment covers {a} m along it"));
    };
    if covered + RESOLUTION < b {
        return Err(format!("no cant segment covers {covered} m along it"));
    }
    let (low, high) = widen((left.0 - right.1, left.1 - right.0));
    Ok(if low >= 0.0 {
        (low, high)
    } else if high <= 0.0 {
        (-high, -low)
    } else {
        (0.0, (-low).max(high))
    })
}

fn interval((low, high): (f64, f64)) -> Result<AlignmentInterval, AlignmentError> {
    AlignmentInterval::try_new(low, high)
}

fn evidence(source: &SourceId, locator: String, exact: bool) -> Evidence {
    let mut evidence = Evidence::exact(source.clone(), locator);
    evidence.exact = exact;
    evidence
}

impl IfcAlignmentService {
    fn centreline(&self, alignment: &ObjectId) -> Result<&Centreline, AlignmentError> {
        match self.alignments.get(alignment) {
            None => Err(AlignmentError::NotAlignment(alignment.clone())),
            Some(Err(reason)) => Err(AlignmentError::Unavailable(format!(
                "{alignment} cannot be read as a centreline: {reason}"
            ))),
            Some(Ok(centreline)) => Ok(centreline),
        }
    }
}

impl AlignmentService for IfcAlignmentService {
    fn measure_alignment_position(
        &self,
        request: &AlignmentRequest,
    ) -> Result<AlignmentPosition, AlignmentError> {
        let (object, alignment) = (request.object(), request.alignment());
        let centreline = self.centreline(alignment)?;
        if object.source != alignment.source {
            return Err(AlignmentError::Unavailable(format!(
                "{object} and {alignment} are in different sources"
            )));
        }
        let point = match self.points.get(object) {
            None => return Err(AlignmentError::UnknownObject(object.clone())),
            Some(Err(reason)) => {
                return Err(AlignmentError::Unavailable(format!(
                    "the reference point of {object} cannot be resolved: {reason}"
                )));
            }
            Some(Ok(point)) => *point,
        };
        let foot =
            locate(centreline, centreline.to_local(point)).map_err(|refusal| match refusal {
                Refusal::OffRange(reason) => Refusal::OffRange(format!("{reason} ({alignment})")),
                other => other,
            })?;
        let stationing = centreline
            .stationing
            .as_ref()
            .map_err(|reason| AlignmentError::Unavailable(reason.clone()))?;
        let station =
            stations(stationing.as_ref(), foot.distance).map_err(AlignmentError::Unavailable)?;
        let locator = format!(
            "{} along {}: foot in [{}, {}] m",
            object.local_id, centreline.locator, foot.distance.0, foot.distance.1
        );
        AlignmentPosition::try_new(
            request.clone(),
            interval(foot.distance)?,
            interval(station)?,
            interval(foot.offset)?,
            interval(foot.height)?,
            evidence(&object.source, locator, false),
        )
    }

    fn measure_alignment_parameter(
        &self,
        request: &AlignmentParameterRequest,
    ) -> Result<AlignmentParameterValue, AlignmentError> {
        let alignment = request.alignment();
        let centreline = self.centreline(alignment)?;
        let stretch = (request.distance().lower(), request.distance().upper());
        if stretch.1 > centreline.length + RESOLUTION {
            return Err(AlignmentError::OffRange(format!(
                "{} m lies beyond the end of {alignment} at {} m",
                stretch.1, centreline.length
            )));
        }
        let elevated = centreline.elevated();
        let (value, what) = match request.parameter() {
            AlignmentParameter::Curvature => (
                Some(curvature(&elevated.plan, stretch).map_err(AlignmentError::Unavailable)?),
                "plan curvature",
            ),
            AlignmentParameter::Gradient => (
                Some(gradient(&elevated.elevation, stretch).map_err(AlignmentError::Unavailable)?),
                "gradient",
            ),
            AlignmentParameter::Cant => {
                let layout = centreline
                    .cant
                    .as_ref()
                    .map_err(|reason| AlignmentError::Unavailable(reason.clone()))?;
                (
                    layout
                        .as_ref()
                        .map(|layout| cant(layout, stretch))
                        .transpose()
                        .map_err(AlignmentError::Unavailable)?,
                    "cant",
                )
            }
        };
        let value = value.map(interval).transpose()?;
        let exact = value.is_none_or(|value| value.is_exact());
        let locator = format!(
            "{what} of {} over [{}, {}] m",
            centreline.locator, stretch.0, stretch.1
        );
        AlignmentParameterValue::try_new(
            request.clone(),
            value,
            evidence(&alignment.source, locator, exact),
        )
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_alignment_placed_off_the_identity_is_refused() {
        assert_eq!(rigid(&Transform::identity()), Ok(None));
        let mut moved = Transform::identity();
        moved.origin = [10.0, 0.0, 0.0];
        let why = rigid(&moved).unwrap_err();
        assert!(why.contains("openbimrs/ifc#357"), "{why}");
    }

    use super::*;
    use axiolid_core::{Frame2, Point2, Vec2};
    use axiolid_curve::Intrinsic2;

    /// A centreline starting at the origin heading `+X`: `law` over
    /// `length` in plan, rising at `grade`.
    fn centreline(law: CurvatureLaw, length: f64, grade: f64) -> Centreline {
        let plan = Curve2::Intrinsic(Intrinsic2::new(
            Frame2 {
                origin: Point2::new(0.0, 0.0),
                x: Vec2::new(1.0, 0.0),
                y: Vec2::new(0.0, 1.0),
            },
            law,
            length,
        ));
        let curve = Curve3::Elevated(Elevated3::new(
            plan,
            ElevationLaw::Polynomial {
                coefficients: vec![10.0, grade],
            },
        ));
        let breaks = continuity_breaks3(&curve, 2)
            .into_iter()
            .filter(|at| *at > 0.0 && *at < length)
            .collect();
        Centreline {
            curve,
            length,
            breaks,
            frame: None,
            stationing: Ok(None),
            cant: Ok(None),
            locator: "test".into(),
        }
    }

    fn straight() -> Centreline {
        centreline(CurvatureLaw::Constant { curvature: 0.0 }, 20.0, 0.02)
    }

    /// A left-turning arc of radius 10 m through a quarter turn: its centre
    /// is at (0, 10).
    fn arc() -> Centreline {
        centreline(
            CurvatureLaw::Constant { curvature: 0.1 },
            10.0 * std::f64::consts::FRAC_PI_2,
            0.0,
        )
    }

    fn holds((low, high): (f64, f64), value: f64, width: f64) {
        assert!(
            low <= value && value <= high && high - low <= width,
            "[{low}, {high}] must hold {value} within {width}"
        );
    }

    #[test]
    fn a_point_beside_a_straight_has_its_station_offset_and_height() {
        let line = straight();
        let foot = locate(&line, [5.0, 1.5, 12.0]).unwrap();
        holds(foot.distance, 5.0, 1e-5);
        holds(foot.offset, 1.5, 1e-5);
        // The gradient line is 10.1 m high at 5 m.
        holds(foot.height, 1.9, 1e-5);
        let right = locate(&line, [7.25, -2.0, 10.0]).unwrap();
        holds(right.distance, 7.25, 1e-5);
        holds(right.offset, -2.0, 1e-5);
        // On the axis itself, and at its very start.
        holds(locate(&line, [3.0, 0.0, 10.06]).unwrap().offset, 0.0, 1e-5);
        holds(locate(&line, [0.0, 4.0, 10.0]).unwrap().distance, 0.0, 1e-5);
    }

    #[test]
    fn a_point_off_the_range_is_refused_with_the_side() {
        let line = straight();
        let Err(Refusal::OffRange(before)) = locate(&line, [-1.0, 0.5, 10.0]) else {
            panic!("before the start must be refused");
        };
        assert!(before.contains("before the alignment's start"), "{before}");
        let Err(Refusal::OffRange(beyond)) = locate(&line, [23.0, 0.0, 10.0]) else {
            panic!("beyond the end must be refused");
        };
        assert!(
            beyond.contains("beyond the alignment's end at 20 m"),
            "{beyond}"
        );
    }

    #[test]
    fn a_point_beside_an_arc_is_located_by_its_radial_foot() {
        let arc = arc();
        // 12 m from the centre at 30 degrees around: 2 m right of the arc.
        let angle = std::f64::consts::FRAC_PI_6;
        let point = [12.0 * angle.sin(), 10.0 - 12.0 * angle.cos(), 10.0];
        let foot = locate(&arc, point).unwrap();
        holds(foot.distance, 10.0 * angle, 1e-5);
        holds(foot.offset, -2.0, 1e-5);
        holds(foot.height, 0.0, 1e-5);
        // Inside the bend, 3 m left of the arc.
        let point = [7.0 * angle.sin(), 10.0 - 7.0 * angle.cos(), 10.0];
        holds(locate(&arc, point).unwrap().offset, 3.0, 1e-5);
    }

    #[test]
    fn the_centre_of_an_arc_is_ambiguous() {
        // Every point of the arc is 10 m from its centre.
        let refusal = locate(&arc(), [0.0, 10.0, 10.0]).unwrap_err();
        assert!(
            matches!(refusal, Refusal::Ambiguous(_) | Refusal::Unavailable(_)),
            "{refusal:?}"
        );
        // A half-turn arc of radius 10 m, its centre at (0, 10), seen from
        // beyond that centre: its start and its end are equally near, so
        // neither side it lies off is chosen.
        let half = centreline(
            CurvatureLaw::Constant { curvature: 0.1 },
            10.0 * std::f64::consts::PI,
            0.0,
        );
        let Err(Refusal::Ambiguous(reason)) = locate(&half, [-3.0, 10.0, 10.0]) else {
            panic!("two equal feet must be ambiguous");
        };
        assert!(reason.contains("equally near"), "{reason}");
    }

    #[test]
    fn of_two_equally_near_feet_none_is_chosen() {
        // A hairpin: 10 m straight along +X, a half turn of radius 10 m,
        // 10 m straight back along y = 20. A point midway between the
        // straights is 10 m from both.
        let turn = 10.0 * std::f64::consts::PI;
        let hairpin = centreline(
            CurvatureLaw::Piecewise {
                breaks: vec![10.0, 10.0 + turn],
                laws: vec![
                    CurvatureLaw::Constant { curvature: 0.0 },
                    CurvatureLaw::Constant { curvature: 0.1 },
                    CurvatureLaw::Constant { curvature: 0.0 },
                ],
            },
            20.0 + turn,
            0.0,
        );
        let Err(Refusal::Ambiguous(reason)) = locate(&hairpin, [5.0, 10.0, 10.0]) else {
            panic!("two equal feet must be ambiguous");
        };
        assert!(reason.contains("equally near"), "{reason}");
        // Nearer one straight, the foot is decided.
        let foot = locate(&hairpin, [5.0, 9.0, 10.0]).unwrap();
        holds(foot.distance, 5.0, 1e-5);
        holds(foot.offset, 9.0, 1e-5);
        let foot = locate(&hairpin, [5.0, 11.0, 10.0]).unwrap();
        holds(foot.distance, 15.0 + turn, 1e-5);
        holds(foot.offset, 9.0, 1e-5);
    }

    /// An IFC4X3 model: the gradient curve of the CLI's tests (#79, a 15 m
    /// straight plan from the origin along `+X`, its profile a sag arc of
    /// radius 1000 m from 10 m at -2 %) as the `Axis` of alignment #175,
    /// a cube on a linear placement 5 m along it and 2 m to its left
    /// (#143), and cubes placed at `(3, -1, 10)` (#183), `(-1, 0.5, 10)`
    /// (#193, before the start) and `(20, 0, 10)` (#203, beyond the end).
    fn aligned_cubes() -> String {
        let placed = |id: u32, x: f64, y: f64| {
            format!(
                "#{}=IFCCARTESIANPOINT(({x:?},{y:?},10.));\n\
                 #{}=IFCAXIS2PLACEMENT3D(#{},$,$);\n\
                 #{}=IFCLOCALPLACEMENT($,#{});\n\
                 #{id}=IFCBUILDINGELEMENTPROXY('{id:022}',$,$,$,$,#{},#124,$,$);\n",
                id - 3,
                id - 2,
                id - 3,
                id - 1,
                id - 2,
                id - 1,
            )
        };
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4X3_ADD2'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#2);\n\
             #4=IFCDIRECTION((0.,0.,1.));\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             #60=IFCCARTESIANPOINT((0.,0.));\n\
             #61=IFCDIRECTION((1.,0.));\n\
             #62=IFCVECTOR(#61,1.);\n\
             #63=IFCLINE(#60,#62);\n\
             #64=IFCAXIS2PLACEMENT2D(#60,#61);\n\
             #65=IFCCURVESEGMENT(.CONTINUOUS.,#64,IFCLENGTHMEASURE(0.),IFCLENGTHMEASURE(15.),#63);\n\
             #66=IFCCARTESIANPOINT((15.,0.));\n\
             #67=IFCAXIS2PLACEMENT2D(#66,#61);\n\
             #68=IFCCURVESEGMENT(.CONTINUOUS.,#67,IFCLENGTHMEASURE(0.),IFCLENGTHMEASURE(0.),#63);\n\
             #69=IFCCOMPOSITECURVE((#65,#68),.F.);\n\
             #70=IFCCIRCLE(#64,1000.);\n\
             #71=IFCCARTESIANPOINT((0.,10.));\n\
             #72=IFCDIRECTION((0.9998000599800071,-0.01999600119960014));\n\
             #73=IFCAXIS2PLACEMENT2D(#71,#72);\n\
             #74=IFCCURVESEGMENT(.CONTINUOUS.,#73,IFCLENGTHMEASURE(0.),IFCLENGTHMEASURE(15.001311989928656),#70);\n\
             #75=IFCCARTESIANPOINT((15.,9.812540071876587));\n\
             #76=IFCDIRECTION((1.,-0.00499606355093224));\n\
             #77=IFCAXIS2PLACEMENT2D(#75,#76);\n\
             #78=IFCCURVESEGMENT(.CONTINUOUS.,#77,IFCLENGTHMEASURE(0.),IFCLENGTHMEASURE(0.),#63);\n\
             #79=IFCGRADIENTCURVE((#74,#78),.F.,#69,$);\n\
             #120=IFCRECTANGLEPROFILEDEF(.AREA.,$,#121,1.,1.);\n\
             #121=IFCAXIS2PLACEMENT2D(#60,$);\n\
             #122=IFCEXTRUDEDAREASOLID(#120,#2,#4,1.);\n\
             #123=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#122));\n\
             #124=IFCPRODUCTDEFINITIONSHAPE($,$,(#123));\n\
             #140=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(5.),2.,$,$,#79);\n\
             #141=IFCAXIS2PLACEMENTLINEAR(#140,$,$);\n\
             #142=IFCLINEARPLACEMENT($,#141,$);\n\
             #143=IFCBUILDINGELEMENTPROXY('0000000000000000000143',$,$,$,$,#142,#124,$,$);\n\
             #173=IFCSHAPEREPRESENTATION(#5,'Axis','Curve3D',(#79));\n\
             #174=IFCPRODUCTDEFINITIONSHAPE($,$,(#173));\n\
             #175=IFCALIGNMENT('0000000000000000000175',$,$,$,$,#3,#174,$);\n\
             {}{}{}ENDSEC;\nEND-ISO-10303-21;\n",
            placed(183, 3.0, -1.0),
            placed(193, -1.0, 0.5),
            placed(203, 20.0, 0.0),
        )
    }

    #[test]
    fn products_are_located_along_an_alignment_of_the_model() {
        use axioval::engine::AlignmentServiceHandle;
        let bytes = aligned_cubes();
        let session = axioval::ifc::import_ifc_session("aligned.ifc", bytes.as_bytes()).unwrap();
        let models: super::super::ModelBytes = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), bytes.as_bytes().to_vec()))
            .collect();
        let (session, _) =
            super::super::attach(session, &models, super::super::Options::default()).unwrap();
        let source = session.snapshots().next().unwrap().source().clone();
        let id = |local: &str| ObjectId::new(source.clone(), local).unwrap();
        let service = session.service::<AlignmentServiceHandle>().unwrap();
        let along = |object: &str| {
            service.measure_alignment_position(
                &AlignmentRequest::try_new(id(object), id("#175")).unwrap(),
            )
        };
        let bounds = |value: AlignmentInterval| (value.lower(), value.upper());

        let linear = along("#143").unwrap();
        holds(bounds(linear.station()), 5.0, 1e-5);
        holds(bounds(linear.offset()), 2.0, 1e-5);
        holds(bounds(linear.height()), 0.0, 1e-5);
        assert!(!linear.evidence().exact);
        let local = along("#183").unwrap();
        holds(bounds(local.station()), 3.0, 1e-5);
        holds(bounds(local.offset()), -1.0, 1e-5);

        for (object, side) in [
            ("#193", "before the alignment's start"),
            ("#203", "beyond the alignment's end"),
        ] {
            let Err(AlignmentError::OffRange(reason)) = along(object) else {
                panic!("{object} must be off the range: {:?}", along(object));
            };
            assert!(reason.contains(side) && reason.contains("#175"), "{reason}");
        }
        assert!(matches!(
            service.measure_alignment_position(
                &AlignmentRequest::try_new(id("#175"), id("#143")).unwrap()
            ),
            Err(AlignmentError::NotAlignment(_))
        ));

        let parameter = |parameter| {
            service
                .measure_alignment_parameter(
                    &AlignmentParameterRequest::try_new(id("#175"), parameter, linear.distance())
                        .unwrap(),
                )
                .unwrap()
                .value()
        };
        let curvature = parameter(AlignmentParameter::Curvature).unwrap();
        assert_eq!(bounds(curvature), (0.0, 0.0));
        // On the sag arc: the sine of the grade angle grows by d / R.
        let sine: f64 = -0.019_996_001_199_600_14 + 5.0 / 1000.0;
        let grade = sine / (1.0 - sine * sine).sqrt();
        holds(
            bounds(parameter(AlignmentParameter::Gradient).unwrap()),
            grade,
            1e-5,
        );
        assert_eq!(parameter(AlignmentParameter::Cant), None);
    }

    #[test]
    fn parameters_follow_their_laws() {
        // A clothoid from straight to 1/100 per metre over 50 m.
        let clothoid = CurvatureLaw::Polynomial {
            coefficients: vec![0.0, 0.01 / 50.0],
        };
        let (low, high) = curvature_law(&clothoid, (10.0, 20.0)).unwrap();
        assert!(low <= 0.002 && high >= 0.004 && high - low < 0.002 + 1e-12);
        let piecewise = CurvatureLaw::Piecewise {
            breaks: vec![50.0],
            laws: vec![clothoid, CurvatureLaw::Constant { curvature: 0.01 }],
        };
        let (low, high) = curvature_law(&piecewise, (60.0, 70.0)).unwrap();
        assert_eq!((low, high), (0.01, 0.01));
        // A right turn read through a mirrored start frame.
        let plan = Curve2::Intrinsic(Intrinsic2::new(
            Frame2 {
                origin: Point2::new(0.0, 0.0),
                x: Vec2::new(1.0, 0.0),
                y: Vec2::new(0.0, -1.0),
            },
            CurvatureLaw::Constant { curvature: 0.01 },
            100.0,
        ));
        assert_eq!(curvature(&plan, (0.0, 1.0)).unwrap(), (-0.01, -0.01));
        // A parabolic sag from -2 % to +2 % over 100 m.
        let sag = ElevationLaw::Polynomial {
            coefficients: vec![10.0, -0.02, 0.0002],
        };
        let (low, high) = gradient(&sag, (0.0, 50.0)).unwrap();
        assert!(low <= -0.02 && (0.0..1e-12).contains(&high));
    }

    #[test]
    fn stations_follow_equations() {
        assert_eq!(stations(None, (1.0, 2.0)).unwrap(), (1.0, 2.0));
    }

    #[test]
    fn polynomials_are_bounded_over_their_stretch() {
        let (low, high) = polynomial(&[1.0, -2.0, 1.0], (0.0, 2.0));
        // (x - 1)^2 over [0, 2] is [0, 1]; interval Horner may widen.
        assert!(low <= 0.0 && high >= 1.0);
        assert_eq!(polynomial(&[], (0.0, 1.0)), (0.0, 0.0));
        assert_eq!(polynomial(&[3.0], (0.0, 1.0)), (3.0, 3.0));
    }
}
