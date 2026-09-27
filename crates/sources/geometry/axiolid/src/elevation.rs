//! Elevation cover: how much of an object's projection onto a vertical plane
//! of its own its cover leaves uncovered.
//!
//! ADR 0004: this module measures; whether a wall's face is covered enough
//! is a rule's decision.
//!
//! Each body is taken into the request's frame: `s` along the plan axis,
//! `t` across it, `z` up. The elevation is the projection onto `(s, z)`.
//!
//! A cover body counts only within the subject's depth across the axis,
//! widened by the along growth on both sides (the slab `lo ≤ t ≤ hi`). The
//! projection of a closed body cut to the slab is the projection of the
//! body's surface inside the slab together with its two cross-sections at
//! `t = lo` and `t = hi`, which lie parallel to the elevation and so are
//! their own projections. A cross-section is found without walking its
//! outline: a point of the plane `t = lo` lies inside the body exactly when
//! a line from it towards smaller `t` crosses the surface an odd number of
//! times, so the cross-section is the even-odd fill of the surface below the
//! plane, projected. This needs a closed surface but no consistent winding.
//!
//! Growth by `a` along the axis and `b` in height is a Minkowski sum with an
//! axis-aligned rectangle, which has an exact polygon: the grown region is
//! the region itself together with every boundary edge swept over the
//! rectangle (the convex hull of the edge's ends moved to the rectangle's
//! corners). The frame's hull is grown the same way.
//!
//! Exact geometry measured along a coordinate axis measures exactly. A
//! tessellated body lies within its chord deviation of its mesh, and an
//! axis off the coordinate axes rounds every projected coordinate; both are
//! bracketed as the plan uncovered area brackets a tessellated cover: the
//! surely covered part grows by less and is cut to a narrower slab, the
//! possibly covered part grows by more and is cut to a wider one, and the
//! subject's own band `2·P·e + π·e²` widens both ends. Where a growth is
//! smaller than the cover's uncertainty, the surely covered part does not
//! grow in that direction and loses the band of its own outline instead.

use axiolid_core::{Point2, Point3};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_engine::{ElevationCover, ElevationRequest, PlanAreaError};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::Triangle;
use crate::plan_area::{AxiolidPlanAreaService, band, tolerance};
use crate::planar::{
    footprint_measure, hull_of, plan_frame, polygons_overlap_area, projected_polygons, ring_area,
    ring_perimeter,
};

/// How far a projected coordinate may round when the axis is off the
/// coordinate axes, in metres, plus `ROUNDING_SCALE` of the largest
/// coordinate: far above the rounding of a two-term dot product.
const ROUNDING: f64 = 1e-9;
const ROUNDING_SCALE: f64 = 1e-14;

/// Points of a clipped polygon closer than this are one point: the overlay
/// refuses vertices closer than its tolerance.
const MERGE: f64 = 1e-8;

/// The smallest growth drawn as a polygon, as in plan: a smaller inner
/// growth is dropped, a smaller outer one rounded up.
const MINIMUM_GROWTH: f64 = 1e-6;

/// One body in the request's frame, `(s, z, t)` stored as `(x, y, z)` so the
/// plan helpers project it onto the elevation.
struct Local {
    soup: Vec<Triangle>,
    deviation: f64,
}

/// Which side of the true value a measurement brackets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Surely covered: less growth, a narrower slab.
    Inner,
    /// Possibly covered: more growth, a wider slab.
    Outer,
}

fn unavailable(object: &ObjectId, what: &str) -> PlanAreaError {
    PlanAreaError::Unavailable(format!("the {what} of {object} cannot be computed"))
}

/// The request's bodies in its frame, with what bounds their uncertainty.
struct Scene<'r> {
    object: &'r ObjectId,
    subject: Local,
    cover: Vec<Local>,
    frame: Vec<Local>,
    /// The subject's uncertainty, and the largest of the others'.
    own: f64,
    theirs: f64,
    /// The subject's depth across the axis.
    depth: (f64, f64),
    /// The subject's elevation area and outline length.
    area: f64,
    perimeter: f64,
    along: f64,
    vertical: f64,
    tolerance: axiolid_core::Tolerance,
}

impl<'r> Scene<'r> {
    fn read(
        service: &AxiolidPlanAreaService,
        request: &'r ElevationRequest,
    ) -> Result<Self, PlanAreaError> {
        let object = request.object();
        let [ux, uy] = request.axis();
        let to_local = |point: &Point3| {
            Point3::new(
                point.x.mul_add(ux, point.y * uy),
                point.z,
                point.y.mul_add(ux, -(point.x * uy)),
            )
        };
        let read = |member: &ObjectId| -> Result<Local, PlanAreaError> {
            let footprint = service.measure(member)?;
            Ok(Local {
                soup: footprint
                    .soup
                    .iter()
                    .map(|triangle| triangle.map(|point| to_local(&point)))
                    .collect(),
                deviation: footprint.deviation,
            })
        };
        let subject = read(object)?;
        let read_all =
            |objects: &[ObjectId]| objects.iter().map(read).collect::<Result<Vec<_>, _>>();
        let cover = read_all(request.cover())?;
        let frame = read_all(request.frame())?;
        // Rounding of the projection when the axis is not a coordinate axis.
        #[allow(clippy::float_cmp)]
        let along_coordinate_axis = ux.abs() * uy.abs() == 0.0;
        let rounding = if along_coordinate_axis {
            0.0
        } else {
            let size = subject
                .soup
                .iter()
                .chain(cover.iter().chain(&frame).flat_map(|local| &local.soup))
                .flatten()
                .fold(0.0_f64, |size, point| {
                    size.max(point.x.abs())
                        .max(point.y.abs())
                        .max(point.z.abs())
                });
            ROUNDING_SCALE.mul_add(size, ROUNDING)
        };
        let theirs = cover
            .iter()
            .chain(&frame)
            .fold(0.0_f64, |deviation, local| deviation.max(local.deviation))
            + rounding;
        let tolerance = tolerance()?;
        let (area, perimeter) = footprint_measure(&subject.soup, tolerance)
            .ok_or_else(|| unavailable(object, "elevation"))?;
        let depth = subject
            .soup
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point.z), high.max(point.z))
            });
        if !depth.0.is_finite() || !depth.1.is_finite() {
            return Err(unavailable(object, "depth"));
        }
        Ok(Self {
            object,
            own: subject.deviation + rounding,
            subject,
            cover,
            frame,
            theirs,
            depth,
            area,
            perimeter,
            along: request.along_growth_metres(),
            vertical: request.vertical_growth_metres(),
            tolerance,
        })
    }

    /// The part of the subject's elevation that the cover surely (`Inner`)
    /// or possibly (`Outer`) covers, and the band the sure part loses when
    /// a growth is smaller than the uncertainty.
    fn covered(&self, side: Side) -> Result<(f64, f64), PlanAreaError> {
        let (low, high) = self.depth;
        let (along, vertical, theirs) = (self.along, self.vertical, self.theirs);
        let widen = self.own + theirs;
        let (lo, hi, growth) = match side {
            Side::Inner => (
                low + widen - along,
                high - widen + along,
                (along - theirs, vertical - theirs),
            ),
            Side::Outer => (
                low - widen - along,
                high + widen + along,
                (along + theirs, vertical + theirs),
            ),
        };
        // An inner growth smaller than the uncertainty is no growth along
        // that direction, and the grown cover then loses its own band.
        let clamped = side == Side::Inner && (growth.0 < 0.0 || growth.1 < 0.0);
        let growth = (growth.0.max(0.0), growth.1.max(0.0));
        let mut region = Vec::new();
        if lo <= hi {
            for local in &self.cover {
                region.extend(within(&local.soup, lo, hi, self.tolerance, self.object)?);
            }
            let mut framed = Vec::new();
            for local in &self.frame {
                framed.extend(within(&local.soup, lo, hi, self.tolerance, self.object)?);
            }
            region.extend(hull(&framed));
        }
        let region = grown(&region, growth, side);
        let overlap = polygons_overlap_area(
            projected_polygons(&self.subject.soup),
            region.clone(),
            self.tolerance,
        )
        .ok_or_else(|| unavailable(self.object, "cover"))?
        .min(self.area);
        let lost = if clamped && theirs > 0.0 {
            band(
                union_perimeter(region, self.tolerance, self.object)?,
                theirs,
            )
        } else {
            0.0
        };
        Ok((overlap, lost))
    }
}

/// Measures `request` over the plan-area service's bodies.
pub(crate) fn measure(
    service: &AxiolidPlanAreaService,
    request: &ElevationRequest,
) -> Result<ElevationCover, PlanAreaError> {
    let scene = Scene::read(service, request)?;
    let (inner, lost) = scene.covered(Side::Inner)?;
    let (outer, _) = scene.covered(Side::Outer)?;
    let (area, slack) = (scene.area, band(scene.perimeter, scene.own));
    let upper = (area - inner + slack + lost).min(area + slack).max(0.0);
    let lower = (area - outer - slack).max(0.0).min(upper);
    let named = |objects: &[ObjectId]| {
        objects
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let [ux, uy] = request.axis();
    let locator = format!(
        "elevation-cover:{}:({ux:.6},{uy:.6}):{}:{}:{}|{}",
        request.object(),
        scene.along,
        scene.vertical,
        named(request.cover()),
        named(request.frame())
    );
    #[allow(clippy::float_cmp)]
    let exact = slack == 0.0 && lower == upper;
    ElevationCover::try_new(
        request.object().clone(),
        ((area - slack).max(0.0), area + slack),
        (lower, upper),
        Evidence {
            source: service.source().clone(),
            locator,
            exact,
        },
    )
}

/// The part of a convex polygon where `side` is not negative.
fn clip(polygon: &[Point3], side: impl Fn(&Point3) -> f64) -> Vec<Point3> {
    let mut kept = Vec::with_capacity(polygon.len() + 1);
    for (index, a) in polygon.iter().enumerate() {
        let b = &polygon[(index + 1) % polygon.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            kept.push(*a);
        }
        if (sa < 0.0) != (sb < 0.0) {
            let t = sa / (sa - sb);
            kept.push(*a + (*b - *a) * t);
        }
    }
    kept
}

/// A convex polygon as fan triangles, near-repeated points merged and
/// triangles too small for the overlay dropped.
fn fan(mut points: Vec<Point3>) -> Vec<Triangle> {
    points.dedup_by(|a, b| (*a - *b).length() < MERGE);
    while points.len() > 1 && (points[0] - points[points.len() - 1]).length() < MERGE {
        points.pop();
    }
    if points.len() < 3 {
        return Vec::new();
    }
    let apex = points[0];
    points[1..]
        .windows(2)
        .map(|pair| [apex, pair[0], pair[1]])
        .filter(|triangle| {
            let flat = |p: &Point3| (p.x, p.y);
            let [a, b, c] = triangle.map(|point| flat(&point));
            let apart = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1) >= MERGE;
            apart(a, b) && apart(b, c) && apart(c, a)
        })
        .collect()
}

/// The elevation of a closed body cut to the slab `lo ≤ t ≤ hi`: its
/// surface inside the slab and its cross-sections at both faces.
fn within(
    soup: &[Triangle],
    lo: f64,
    hi: f64,
    tolerance: axiolid_core::Tolerance,
    object: &ObjectId,
) -> Result<Vec<Polygon>, PlanAreaError> {
    let mut inside = Vec::new();
    let (mut below, mut above) = (Vec::new(), Vec::new());
    for triangle in soup {
        inside.extend(fan(clip(&clip(triangle, |p| p.z - lo), |p| hi - p.z)));
        below.extend(fan(clip(triangle, |p| lo - p.z)));
        above.extend(fan(clip(triangle, |p| p.z - hi)));
    }
    let mut polygons = projected_polygons(&inside);
    for part in [below, above] {
        polygons.extend(
            even_odd(projected_polygons(&part), tolerance).ok_or_else(|| {
                PlanAreaError::Unavailable(format!(
                    "a cross-section of a cover of {object} cannot be computed"
                ))
            })?,
        );
    }
    Ok(polygons)
}

/// The even-odd fill of a polygon set.
fn even_odd(polygons: Vec<Polygon>, tolerance: axiolid_core::Tolerance) -> Option<Vec<Polygon>> {
    if polygons.is_empty() {
        return Some(Vec::new());
    }
    let input = OverlayInput {
        frame: plan_frame(),
        polygons,
    };
    overlay(
        &input,
        &input,
        OverlayOperation::Union,
        FillRule::EvenOdd,
        tolerance,
    )
    .ok()
    .map(|result| result.polygons)
}

fn triangle_polygon(points: [(f64, f64); 3]) -> Option<Polygon> {
    let ring = Ring {
        points: points.into_iter().map(|(x, y)| Point2::new(x, y)).collect(),
    };
    (ring_area(&ring) > f64::EPSILON).then_some(Polygon {
        outer: ring,
        holes: Vec::new(),
    })
}

/// A convex hull as counter-clockwise fan triangles.
fn hull_fan(hull: &[(f64, f64)]) -> Vec<Polygon> {
    let Some(&apex) = hull.first() else {
        return Vec::new();
    };
    hull.get(1..)
        .unwrap_or_default()
        .windows(2)
        .filter_map(|pair| triangle_polygon([apex, pair[0], pair[1]]))
        .collect()
}

/// The convex hull of a polygon set's vertices, as fan triangles.
fn hull(polygons: &[Polygon]) -> Vec<Polygon> {
    let points: Vec<(f64, f64)> = polygons
        .iter()
        .flat_map(|polygon| &polygon.outer.points)
        .map(|point| (point.x, point.y))
        .collect();
    hull_fan(&hull_of(points))
}

/// A region grown by `a` along the axis and `b` in height: the region with
/// every boundary edge swept over the rectangle `[-a, a] × [-b, b]`.
fn grown(polygons: &[Polygon], (a, b): (f64, f64), side: Side) -> Vec<Polygon> {
    let round = |growth: f64| match side {
        Side::Inner if growth < MINIMUM_GROWTH => 0.0,
        Side::Outer if growth > 0.0 => growth.max(MINIMUM_GROWTH),
        _ => growth.max(0.0),
    };
    let (a, b) = (round(a), round(b));
    let mut result = polygons.to_vec();
    if a <= 0.0 && b <= 0.0 {
        return result;
    }
    let corners = [(-a, -b), (a, -b), (a, b), (-a, b)];
    for polygon in polygons {
        for ring in std::iter::once(&polygon.outer).chain(&polygon.holes) {
            let points = &ring.points;
            for index in 0..points.len() {
                let (p, q) = (points[index], points[(index + 1) % points.len()]);
                let swept: Vec<(f64, f64)> = [p, q]
                    .iter()
                    .flat_map(|point| corners.map(|(dx, dy)| (point.x + dx, point.y + dy)))
                    .collect();
                result.extend(hull_fan(&hull_of(swept)));
            }
        }
    }
    result
}

/// The perimeter of a polygon set's union.
fn union_perimeter(
    polygons: Vec<Polygon>,
    tolerance: axiolid_core::Tolerance,
    object: &ObjectId,
) -> Result<f64, PlanAreaError> {
    if polygons.is_empty() {
        return Ok(0.0);
    }
    let input = OverlayInput {
        frame: plan_frame(),
        polygons,
    };
    let merged = overlay(
        &input,
        &input,
        OverlayOperation::Union,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|_| unavailable(object, "cover outline"))?;
    Ok(merged
        .polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .map(ring_perimeter)
        .sum())
}

#[cfg(test)]
mod tests {
    use super::{Side, even_odd, grown, projected_polygons, within};
    use crate::planar::polygon_area;
    use axiolid_core::{Point3, Tolerance};
    use axioval_ir::{ObjectId, SourceId};

    fn tolerance() -> Tolerance {
        Tolerance::new(1e-9, 1e-9).unwrap()
    }

    /// The twelve triangles of an axis-aligned box.
    fn cuboid(min: [f64; 3], max: [f64; 3]) -> Vec<super::Triangle> {
        let corner = |i: usize| {
            Point3::new(
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            )
        };
        let quads = [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ];
        quads
            .iter()
            .flat_map(|[a, b, c, d]| {
                [
                    [corner(*a), corner(*b), corner(*c)],
                    [corner(*a), corner(*c), corner(*d)],
                ]
            })
            .collect()
    }

    fn area(polygons: Vec<axiolid_overlay::Polygon>) -> f64 {
        even_odd(polygons, tolerance())
            .unwrap()
            .iter()
            .map(polygon_area)
            .sum()
    }

    fn union(polygons: Vec<axiolid_overlay::Polygon>) -> f64 {
        let input = axiolid_overlay::OverlayInput {
            frame: crate::planar::plan_frame(),
            polygons,
        };
        axiolid_overlay::overlay(
            &input,
            &input,
            axiolid_overlay::OverlayOperation::Union,
            axiolid_overlay::FillRule::NonZero,
            tolerance(),
        )
        .unwrap()
        .polygons
        .iter()
        .map(polygon_area)
        .sum()
    }

    fn id() -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), "a").unwrap()
    }

    #[test]
    fn a_body_passing_through_the_slab_projects_its_whole_cross_section() {
        // x 0..2, y (depth) −1..1, z 0..3 seen from the side: the slab
        // 0 ≤ t ≤ 0.2 lies inside the body, so no face meets it.
        let body = cuboid([0.0, 0.0, -1.0], [2.0, 3.0, 1.0]);
        let polygons = within(&body, 0.0, 0.2, tolerance(), &id()).unwrap();
        assert!((union(polygons) - 6.0).abs() < 1e-9);
        // A slab beside the body meets nothing.
        let polygons = within(&body, 1.5, 2.0, tolerance(), &id()).unwrap();
        assert!(polygons.is_empty() || union(polygons) < 1e-12);
    }

    #[test]
    fn the_even_odd_fill_cancels_front_and_back_faces() {
        let body = cuboid([0.0, 0.0, -1.0], [2.0, 3.0, 1.0]);
        assert!(area(projected_polygons(&body)) < 1e-9);
    }

    #[test]
    fn a_rectangle_grows_by_its_two_growths_exactly() {
        let body = cuboid([0.0, 0.0, 0.0], [2.0, 3.0, 1.0]);
        let polygons = within(&body, -1.0, 2.0, tolerance(), &id()).unwrap();
        let grown = grown(&polygons, (0.5, 0.25), Side::Outer);
        assert!((union(grown) - 3.0 * 3.5).abs() < 1e-9);
    }
}
