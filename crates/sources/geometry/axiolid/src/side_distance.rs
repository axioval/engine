//! What lies beside each side of a footprint's least-area rectangle.
//!
//! Every candidate's plan triangles are taken into the frame of the side:
//! `u` along its outward direction from the rectangle's centre, `v` across
//! it. Each triangle is clipped to two rectangles in that frame: an inner
//! one surely inside the true strip and an outer one surely holding it. A
//! triangle keeping positive area in the inner one makes the candidate
//! surely present, and its least `u` there bounds the distance from above;
//! one keeping positive area in the outer one makes it possibly present,
//! and the least `u` over them all bounds the distance from below. A linear
//! function is least over a convex polygon at a vertex, so only the clipped
//! vertices are compared.
//!
//! The rectangle's centre may lie `r` from the true one, its axes `θ` from
//! the true ones, and a tessellated candidate's footprint `d` from its true
//! footprint. A point `ρ` from the centre then moves by at most
//! `r + ρ·θ + d` in either coordinate, and the clipped vertices and the
//! frame change round besides; the slack `e` covers all of it for every
//! point within the reach, and the two rectangles and every distance move
//! by `e`. The evidence is therefore never exact.

use axioval_engine::{
    PlanLength, PlanSpanError, PlanSpanService, RectangleOrientation, RectangleSide, SideDistance,
    SideDistanceRequest, SideDistances, SidePresence,
};
use axioval_ir::Evidence;

use crate::plan_span::AxiolidPlanSpanService;

/// The rounding of a clipped vertex and of the frame change, in metres,
/// plus this share of the largest coordinate involved.
const ROUNDING: f64 = 1e-9;
const ROUNDING_SCALE: f64 = 1e-12;

/// The largest slack a measurement is answered with: beyond it the
/// rectangle is too uncertain for its sides to mean anything.
const MAXIMUM_SLACK: f64 = 0.01;

/// A point in a side's frame: `(u, v)`.
type Local = (f64, f64);

/// The part of a convex polygon where `inside` is non-negative, `inside`
/// being affine; the cut points are interpolated.
fn clip(polygon: &[Local], inside: impl Fn(Local) -> f64) -> Vec<Local> {
    let mut kept = Vec::new();
    for (index, &a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        let (fa, fb) = (inside(a), inside(b));
        if fa >= 0.0 {
            kept.push(a);
        }
        if (fa >= 0.0) != (fb >= 0.0) {
            let t = fa / (fa - fb);
            kept.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    kept
}

/// Twice the signed area of a polygon.
fn doubled_area(polygon: &[Local]) -> f64 {
    (0..polygon.len())
        .map(|index| {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// The part of `triangle` in `u_from..=u_to`, `-half..=half`, and whether
/// it has positive area.
fn within(triangle: &[Local], (u_from, u_to): (f64, f64), half: f64) -> Option<Vec<Local>> {
    if half <= 0.0 || u_to <= u_from {
        return None;
    }
    let mut part = triangle.to_vec();
    for cut in [
        &(|p: Local| p.0 - u_from) as &dyn Fn(Local) -> f64,
        &|p: Local| u_to - p.0,
        &|p: Local| p.1 + half,
        &|p: Local| half - p.1,
    ] {
        part = clip(&part, cut);
        if part.len() < 3 {
            return None;
        }
    }
    (doubled_area(&part).abs() > 0.0).then_some(part)
}

fn least_u(part: &[Local]) -> f64 {
    part.iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min)
}

/// Answers a side-distance request from `service`'s footprints.
pub(crate) fn measure(
    service: &AxiolidPlanSpanService,
    request: &SideDistanceRequest,
) -> Result<SideDistances, PlanSpanError> {
    let object = request.object();
    let rectangle = service.measure_rectangle(object)?;
    if rectangle.orientation() != RectangleOrientation::Unique {
        return Err(PlanSpanError::Unavailable(format!(
            "the least-area rectangle of {object} is {}, so its sides are not the footprint's \
             own",
            rectangle.orientation().name()
        )));
    }
    let reach = request.reach_metres();
    let inset = request.inset_metres();
    let centre = rectangle.centre();
    let halves = rectangle.half_extents_metres();
    let turn = rectangle.axis_error_radians();
    let widest = halves[0].1.max(halves[1].1);
    let mut candidates = Vec::new();
    for candidate in request.candidates() {
        if service.has_no_body(candidate) {
            continue;
        }
        candidates.push((candidate, service.footprint(candidate)?));
    }
    let scale = candidates
        .iter()
        .flat_map(|(_, footprint)| footprint.soup.iter().flatten())
        .map(|point| point.x.abs().max(point.y.abs()))
        .fold(centre[0].abs().max(centre[1].abs()), f64::max);
    let mut distances = Vec::new();
    for (candidate, footprint) in &candidates {
        // Every point of interest lies within `reach + widest + 1` of the
        // centre, as long as the slack stays below a metre.
        let slack = rectangle.centre_radius_metres()
            + turn * (reach + widest + 1.0)
            + footprint.deviation
            + ROUNDING
            + ROUNDING_SCALE * (scale + reach);
        if slack > MAXIMUM_SLACK {
            return Err(PlanSpanError::Unavailable(format!(
                "the rectangle of {object} or the footprint of {candidate} is known only within \
                 {slack} m, too loosely to tell which side {candidate} lies beside"
            )));
        }
        for side in RectangleSide::ALL {
            let out = side.outward(&rectangle);
            let across = [-out[1], out[0]];
            let (half_low, half_high) = halves[1 - side.axis()];
            let inner = (half_low - inset - slack, (slack, reach - slack));
            let outer = (half_high - inset + slack, (-slack, reach + slack));
            let (mut sure, mut possible) = (f64::INFINITY, f64::INFINITY);
            for triangle in &footprint.soup {
                let local: Vec<Local> = triangle
                    .iter()
                    .map(|point| {
                        let (x, y) = (point.x - centre[0], point.y - centre[1]);
                        (x * out[0] + y * out[1], x * across[0] + y * across[1])
                    })
                    .collect();
                if let Some(part) = within(&local, outer.1, outer.0) {
                    possible = possible.min(least_u(&part));
                    if let Some(part) = within(&local, inner.1, inner.0) {
                        sure = sure.min(least_u(&part));
                    }
                }
            }
            if possible.is_infinite() {
                continue;
            }
            let lower = (possible - slack).max(0.0);
            let (presence, upper) = if sure.is_finite() {
                (SidePresence::Sure, (sure + slack).max(lower))
            } else {
                (SidePresence::Possible, reach + slack)
            };
            let distance = PlanLength::try_new(
                lower,
                upper.max(lower),
                Evidence {
                    source: candidate.source.clone(),
                    locator: format!("plan-side-distance:{object}:{}:{candidate}", side.name()),
                    exact: false,
                },
            )?;
            distances.push(SideDistance::new(
                (*candidate).clone(),
                side,
                presence,
                distance,
            ));
        }
    }
    SideDistances::try_new(
        request.clone(),
        rectangle,
        distances,
        Evidence {
            source: object.source.clone(),
            locator: format!("plan-side-distances:{object}:reach={reach}:inset={inset}"),
            exact: false,
        },
    )
}
