//! How high an edge stands above the surfaces beneath it.
//!
//! Answers [`axioval_engine::ProximityService::measure_height_above`]. The
//! edge is the subject's faces looking up (top) or down (bottom), the
//! surfaces the requested bodies' faces looking up; a face looks up when its
//! outward normal lies within 45° of up, as [`axioval_engine::FaceClass`]
//! reads it, and a face within rounding of 45° refuses the measurement.
//!
//! - **Pieces.** Each surface triangle is clipped in plan to each edge
//!   triangle. Over such a piece both are planar, so the vertical gap from
//!   the edge down to the surface is linear; the part of the piece where the
//!   surface lies at or below the edge (within the linear tolerance) is
//!   kept, and the least gap over it is at one of its vertices. The least
//!   over every kept piece is the height: the nearest surface beneath each
//!   point of the edge, over the whole edge. A sloped roof under a level
//!   upstand top is measured where it runs highest under it.
//! - **Coverage.** The kept pieces' plan union must cover the edge's plan.
//!   A part of the edge with no surface beneath it (past the surface's edge,
//!   over an opening or a gap) is reported with its area, never measured to
//!   the surfaces beside it.
//!
//! Every body must be an exact closed solid: a tessellation's chords do not
//! state its true faces' normals or heights, and an open body has no
//! outward side. The height is the arithmetic on exact positions, widened
//! by the rounding of the clipping, so its evidence is exact.

use axiolid_core::{Point2, Point3};
use axiolid_overlay::{Polygon, Ring};
use axioval_engine::{
    HeightAbove, HeightAboveEvidence, HeightAboveRequest, ProximityError, SubjectSurface,
};
use axioval_ir::Evidence;

use crate::geometry::Triangle;
use crate::planar::{footprint_polygons, polygon_area, polygons_overlap_area, projected_polygons};
use crate::proximity::{AxiolidProximityService, Body, LINEAR_TOLERANCE, tolerance};
use crate::vertical_surface::{Plan, apart, clip, over, plan_area, triangle_box};

/// How close to 45° a face's normal may come before its direction is
/// undecided.
const CLASS_TOLERANCE: f64 = 1e-9;

/// Plan area below which a piece only touches the edge.
const TOUCH_AREA: f64 = 1e-12;

/// Which way the faces kept look.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Looking {
    Up,
    Down,
}

/// Measures the request.
pub(crate) fn measure(
    service: &AxiolidProximityService,
    request: &HeightAboveRequest,
) -> Result<HeightAboveEvidence, ProximityError> {
    for object in std::iter::once(request.subject()).chain(request.surfaces()) {
        if !service.fidelity(object)?.is_exact() {
            return Err(ProximityError::Refused(
                "a tessellated body's faces are not its true faces, so no height above a \
                 surface is measured on it",
            ));
        }
    }
    let subject = service.body(request.subject())?;
    let looking = match request.edge() {
        SubjectSurface::Top => Looking::Up,
        SubjectSurface::Bottom => Looking::Down,
    };
    let edge = faces(&subject, looking)?;
    if edge.is_empty() {
        return Err(ProximityError::Refused(
            "the body has no face looking that way, so it has no such edge",
        ));
    }
    let mut scale = magnitude(&subject);
    let mut surfaces = Vec::new();
    for object in request.surfaces() {
        let body = service.body(object)?;
        scale = scale.max(magnitude(&body));
        surfaces.extend(faces(&body, Looking::Up)?);
    }
    let (least, beneath) = least_gap(&edge, &surfaces);
    let height = coverage(&edge, beneath)?.unwrap_or_else(|| {
        // Each clipped coordinate and the plane's height at it round by a
        // few ulps of the largest coordinate; sixteen hold them.
        let margin = 16.0 * f64::EPSILON * scale;
        HeightAbove::Beneath {
            lower_metres: (least - margin).max(0.0),
            upper_metres: least + margin,
        }
    });
    let surfaces: Vec<String> = request.surfaces().iter().map(ToString::to_string).collect();
    HeightAboveEvidence::try_new(
        request.clone(),
        height,
        Evidence {
            source: request.subject().source.clone(),
            locator: format!(
                "axiolid:height-above:{}:{}:{}",
                request.edge().name(),
                request.subject(),
                surfaces.join("+")
            ),
            exact: true,
        },
    )
}

/// The largest coordinate magnitude of a body, at least one metre.
fn magnitude(body: &Body<'_>) -> f64 {
    let bounds = body.soup.bounds;
    bounds
        .min()
        .into_iter()
        .chain(bounds.max())
        .map(f64::abs)
        .fold(1.0, f64::max)
}

/// The body's triangles looking `looking`, by their outward normals.
fn faces(body: &Body<'_>, looking: Looking) -> Result<Vec<Triangle>, ProximityError> {
    const NOT_CLOSED: &str =
        "the body is not a closed solid, so which way its faces look is unknown";
    if !body.solid {
        return Err(ProximityError::Refused(NOT_CLOSED));
    }
    // Twice the signed volume's sign says which way the winding faces.
    let orientation: f64 = body
        .soup
        .items
        .iter()
        .map(|[a, b, c]| a.dot(b.cross(*c)))
        .sum();
    if !orientation.is_finite() || orientation == 0.0 {
        return Err(ProximityError::Refused(NOT_CLOSED));
    }
    let outward = orientation.signum();
    let limit = std::f64::consts::FRAC_1_SQRT_2;
    let mut kept = Vec::new();
    for triangle in &body.soup.items {
        let [a, b, c] = *triangle;
        let normal = (b - a).cross(c - a) * outward;
        let length = normal.length();
        if length == 0.0 || !length.is_finite() {
            continue;
        }
        let up = normal.z / length;
        if (up.abs() - limit).abs() <= CLASS_TOLERANCE {
            return Err(ProximityError::Refused(
                "a face lies at 45° within rounding, so whether it looks up or sideways is \
                 undecided",
            ));
        }
        let admitted = match looking {
            Looking::Up => up > limit,
            Looking::Down => up < -limit,
        };
        if admitted {
            kept.push(*triangle);
        }
    }
    Ok(kept)
}

/// A non-vertical triangle's plane as `z = a + b x + c y`.
fn plane([first, second, third]: &Triangle) -> Option<(f64, f64, f64)> {
    let normal = (*second - *first).cross(*third - *first);
    if normal.z == 0.0 || !normal.z.is_finite() {
        return None;
    }
    let (b, c) = (-normal.x / normal.z, -normal.y / normal.z);
    Some((first.z - b * first.x - c * first.y, b, c))
}

/// A triangle's plan, counter-clockwise.
fn plan_of(triangle: &Triangle) -> Plan {
    let mut points: Plan = triangle.iter().map(|point| (point.x, point.y)).collect();
    let [(ax, ay), (bx, by), (cx, cy)] = [points[0], points[1], points[2]];
    if (bx - ax).mul_add(cy - ay, -((by - ay) * (cx - ax))) < 0.0 {
        points.reverse();
    }
    points
}

/// The least gap from the edge down to a surface beneath it, and the plan
/// of every piece of surface found beneath it, counter-clockwise.
fn least_gap(edge: &[Triangle], surfaces: &[Triangle]) -> (f64, Vec<Polygon>) {
    let boxes: Vec<[f64; 4]> = surfaces.iter().map(triangle_box).collect();
    let mut least = f64::INFINITY;
    let mut beneath = Vec::new();
    for face in edge {
        let Some((a, b, c)) = plane(face) else {
            continue;
        };
        let gap = |point: &Point3| b.mul_add(point.x, c.mul_add(point.y, a)) - point.z;
        let region = plan_of(face);
        let region_box = triangle_box(face);
        for (surface, surface_box) in surfaces.iter().zip(&boxes) {
            if apart(*surface_box, region_box) {
                continue;
            }
            let piece = over(surface, &region);
            if piece.len() < 3 || plan_area(&piece) <= TOUCH_AREA {
                continue;
            }
            let below = clip(&piece, |point| gap(point) + LINEAR_TOLERANCE);
            if below.len() < 3 || plan_area(&below) <= TOUCH_AREA {
                continue;
            }
            least = below
                .iter()
                .map(|point| gap(point).max(0.0))
                .fold(least, f64::min);
            let mut points: Vec<Point2> = below
                .iter()
                .map(|point| Point2::new(point.x, point.y))
                .collect();
            if signed_area(&points) < 0.0 {
                points.reverse();
            }
            beneath.push(Polygon {
                outer: Ring { points },
                holes: Vec::new(),
            });
        }
    }
    (least, beneath)
}

fn signed_area(points: &[Point2]) -> f64 {
    let mut sum = 0.0;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        sum += a.x.mul_add(b.y, -(b.x * a.y));
    }
    sum / 2.0
}

/// The part of the edge's plan with no surface beneath it, when there is
/// one.
fn coverage(
    edge: &[Triangle],
    beneath: Vec<Polygon>,
) -> Result<Option<HeightAbove>, ProximityError> {
    const REFUSED: &str = "the plan overlay refused the edge's plan";
    let tolerance = tolerance()?;
    let whole: f64 = footprint_polygons(edge, tolerance)
        .ok_or(ProximityError::Refused(REFUSED))?
        .iter()
        .map(polygon_area)
        .sum();
    let covered = polygons_overlap_area(projected_polygons(edge), beneath, tolerance)
        .ok_or(ProximityError::Refused(REFUSED))?;
    let uncovered = whole - covered;
    // The overlay snaps its output to a grid, so a covered edge may leave a
    // sliver of rounding.
    Ok(
        (uncovered > 1e-9 + whole * 1e-9).then_some(HeightAbove::Uncovered {
            uncovered_square_metres: uncovered,
            edge_square_metres: whole,
        }),
    )
}
