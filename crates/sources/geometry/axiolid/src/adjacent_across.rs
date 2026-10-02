//! The spaces beside a separating element: `adjacent-across`.
//!
//! A separating element is a straight wall or a flat slab. A wall's faces
//! are the two long sides of its plan rectangle, and a space lies beside one
//! when the part of its body within the height the two share comes within
//! the tolerance of the face along at least the overlap. A slab's faces are
//! its top and its bottom, and a space lies beside one when the part of its
//! body within the tolerance of the face shares at least the overlap
//! squared of footprint with the slab.
//!
//! Every measurement is bracketed: an inner band, shrunk by the margin,
//! shows a space surely beside a face; an outer band, grown by it, shows it
//! surely not. The margin is the overlay's rounding plus the chord
//! deviations of both bodies, so a tessellation is never decided closer
//! than its deviation. A space between the two brackets is undecided, and
//! so is an element that is neither a straight wall nor a flat slab.

use axiolid_core::{Point2, Vec2};
use axiolid_mesh::{TriMesh, TriangleMeshView};

use axioval_ir::ObjectId;

use crate::derived_relationships::{convex_hull, span, thin_axis};
use crate::geometry::mesh_extent;
use crate::planar::{ring_area, ring_perimeter};
use crate::walkable::{Plan, band_footprint, intersect, polygon};

/// The overlay's rounding and the snapping of its output, in metres: no
/// band edge is trusted closer than this.
const ROUNDING: f64 = 1e-6;

/// A straight wall: a prism over a plan rectangle.
#[derive(Clone, Debug)]
pub(crate) struct Wall {
    /// Unit plan normal through the thickness; the `+` face lies along it.
    normal: Vec2,
    /// Unit plan direction along the wall.
    along: Vec2,
    /// The `-` and `+` faces, as positions along `normal`.
    faces: (f64, f64),
    /// The wall's ends, as positions along `along`.
    ends: (f64, f64),
    bottom: f64,
    top: f64,
}

/// A flat slab: a plate between two horizontal planes.
#[derive(Clone, Debug)]
pub(crate) struct Slab {
    bottom: f64,
    top: f64,
    footprint: Plan,
}

/// A separating element ready to be measured against spaces.
#[derive(Clone, Debug)]
pub(crate) enum Separator {
    Wall(Wall),
    Slab(Slab),
}

impl Separator {
    /// How the evidence writes the `+` face's direction.
    pub(crate) fn normal_text(&self) -> String {
        match self {
            Self::Wall(wall) => format!(
                "({:.6},{:.6},0.000000)",
                wall.normal.x + 0.0,
                wall.normal.y + 0.0
            ),
            Self::Slab(_) => "(0.000000,0.000000,1.000000)".into(),
        }
    }
}

/// A space surely beside one face, and the least overlap it surely has.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Beside {
    /// `+1` beside the `+` face, `-1` beside the `-` face.
    pub(crate) sign: f64,
    /// The overlap's lower bound: a length along a wall, an area under a
    /// slab.
    pub(crate) overlap: f64,
}

/// The separating element `element` with body `mesh`, whose surface lies
/// within `deviation` metres of the mesh.
///
/// # Errors
///
/// An element that is neither a flat slab (every vertex on its bottom or
/// top plane, thinner than it is wide) nor a straight wall (a plan of one
/// rectangle with a single narrowest direction).
pub(crate) fn separator(element: &ObjectId, mesh: &TriMesh) -> Result<Separator, String> {
    let (min, max) =
        mesh_extent(mesh).ok_or_else(|| format!("element {element} has an empty mesh"))?;
    let (bottom, top) = (min[2], max[2]);
    let positions: Vec<_> = (0..mesh.position_count())
        .map(|index| mesh.position(index))
        .collect();
    let plan: Vec<Point2> = positions.iter().map(|p| Point2::new(p.x, p.y)).collect();
    let hull = convex_hull(&plan);
    if hull.len() < 3 {
        return Err(format!("element {element} has no plan area"));
    }
    let flat = positions
        .iter()
        .all(|p| (p.z - bottom).abs() <= ROUNDING || (p.z - top).abs() <= ROUNDING);
    if flat && top - bottom < least_width(&hull) - ROUNDING {
        let footprint = band_footprint(element, mesh, bottom, top)?;
        if footprint.is_empty() {
            return Err(format!("slab {element} has no footprint"));
        }
        return Ok(Separator::Slab(Slab {
            bottom,
            top,
            footprint,
        }));
    }
    let axis = thin_axis(&plan).ok_or_else(|| {
        format!("element {element} is neither a flat slab nor a wall with one direction across it")
    })?;
    let along = axis.normal.perp();
    let faces = span(&hull, axis.normal);
    let ends = span(&hull, along);
    let rectangle = axis.thickness * (ends.1 - ends.0);
    let slack = ROUNDING * (axis.thickness + ends.1 - ends.0) * 4.0;
    let hull_area = ring_area(&axiolid_overlay::Ring {
        points: hull.clone(),
    })
    .abs();
    let footprint = band_footprint(element, mesh, bottom, top)?.area();
    if (hull_area - rectangle).abs() > slack || (footprint - hull_area).abs() > slack {
        return Err(format!(
            "wall {element} is not a straight wall of one thickness: its plan is not one \
             rectangle"
        ));
    }
    Ok(Separator::Wall(Wall {
        normal: axis.normal,
        along,
        faces,
        ends,
        bottom,
        top,
    }))
}

/// The faces of `separator` (body deviation `element_deviation`) the space
/// `space` (body `mesh`, deviation `space_deviation`) surely lies beside.
///
/// # Errors
///
/// When whether the space lies beside a face is undecided: it may or may
/// not lie within `tolerance` of it, or share `overlap` with it, as far as
/// the margins tell; or a band could not be computed.
pub(crate) fn beside(
    element: &ObjectId,
    separator: &Separator,
    element_deviation: f64,
    space: &ObjectId,
    mesh: &TriMesh,
    space_deviation: f64,
    (tolerance, overlap): (f64, f64),
) -> Result<Vec<Beside>, String> {
    let deviation = (element_deviation, space_deviation);
    let mut found = Vec::new();
    match separator {
        Separator::Wall(wall) => {
            beside_wall(
                element,
                wall,
                space,
                mesh,
                deviation,
                (tolerance, overlap),
                &mut found,
            )?;
        }
        Separator::Slab(slab) => {
            beside_slab(
                element,
                slab,
                space,
                mesh,
                deviation,
                (tolerance, overlap),
                &mut found,
            )?;
        }
    }
    Ok(found)
}

/// [`beside`] for a wall.
fn beside_wall(
    element: &ObjectId,
    wall: &Wall,
    space: &ObjectId,
    mesh: &TriMesh,
    (element_deviation, space_deviation): (f64, f64),
    (tolerance, overlap): (f64, f64),
    found: &mut Vec<Beside>,
) -> Result<(), String> {
    let deviation = element_deviation + space_deviation;
    let margin = ROUNDING + deviation;
    let (min, max) = mesh_extent(mesh).ok_or_else(|| format!("space {space} has an empty mesh"))?;
    let (low, high) = (wall.bottom.max(min[2]), wall.top.min(max[2]));
    let shared = high - low;
    if shared + deviation <= ROUNDING {
        return Ok(());
    }
    if shared - deviation <= ROUNDING {
        return Err(format!(
            "whether space {space} shares any height with wall {element} is undecided: \
             they share {shared:.6} m within a deviation of {deviation:.6} m"
        ));
    }
    let region = band_footprint(space, mesh, low, high)?;
    for sign in [1.0, -1.0] {
        let face = if sign > 0.0 {
            wall.faces.1
        } else {
            wall.faces.0
        };
        let outer = wall_band(
            wall,
            (face - sign * margin, face + sign * (tolerance + margin)),
            (wall.ends.0 - margin, wall.ends.1 + margin),
        );
        let upper = outer
            .map(|band| intersect(&region, &band))
            .transpose()?
            .and_then(|hit| extent_along(&hit, wall.along))
            .map(|length| length + 2.0 * space_deviation);
        let inner = (tolerance > 2.0 * margin && wall.ends.1 - wall.ends.0 > 2.0 * margin)
            .then(|| {
                wall_band(
                    wall,
                    (face + sign * margin, face + sign * (tolerance - margin)),
                    (wall.ends.0 + margin, wall.ends.1 - margin),
                )
            })
            .flatten();
        let lower = inner
            .map(|band| intersect(&region, &band))
            .transpose()?
            .and_then(|hit| extent_along(&hit, wall.along))
            .map(|length| length - 2.0 * space_deviation);
        decide(sign, lower, upper, overlap, found).map_err(|(lower, upper)| {
            format!(
                "whether space {space} lies within {tolerance} m of the {} face of wall \
                 {element} along at least {overlap} m is undecided: it does along between \
                 {lower:.6} and {upper:.6} m",
                if sign > 0.0 { '+' } else { '-' }
            )
        })?;
    }
    Ok(())
}

/// [`beside`] for a slab.
fn beside_slab(
    element: &ObjectId,
    slab: &Slab,
    space: &ObjectId,
    mesh: &TriMesh,
    (element_deviation, space_deviation): (f64, f64),
    (tolerance, overlap): (f64, f64),
    found: &mut Vec<Beside>,
) -> Result<(), String> {
    let deviation = element_deviation + space_deviation;
    let margin = ROUNDING + deviation;
    let need = overlap * overlap;
    for sign in [1.0, -1.0] {
        let (outer, inner) = if sign > 0.0 {
            (
                (slab.top - margin, slab.top + tolerance + margin),
                (slab.top + margin, slab.top + tolerance - margin),
            )
        } else {
            (
                (slab.bottom - tolerance - margin, slab.bottom + margin),
                (slab.bottom - tolerance + margin, slab.bottom - margin),
            )
        };
        let hit = intersect(
            &band_footprint(space, mesh, outer.0, outer.1)?,
            &slab.footprint,
        )?;
        let upper = (!hit.is_empty()).then(|| {
            let (area, perimeter, vertices) = measure(&hit);
            area + deviation * perimeter + std::f64::consts::PI * deviation * deviation * vertices
        });
        let lower = if inner.1 - inner.0 > 0.0 {
            let hit = intersect(
                &band_footprint(space, mesh, inner.0, inner.1)?,
                &slab.footprint,
            )?;
            (!hit.is_empty()).then(|| {
                let (area, perimeter, _) = measure(&hit);
                area - deviation * perimeter
            })
        } else {
            None
        };
        decide(sign, lower, upper, need, found).map_err(|(lower, upper)| {
            format!(
                "whether space {space} lies within {tolerance} m of the {} of slab \
                 {element} over at least {need} m² is undecided: it does over between \
                 {lower:.6} and {upper:.6} m²",
                if sign > 0.0 { "top" } else { "bottom" }
            )
        })?;
    }
    Ok(())
}

/// Records a face the space is surely beside; `Err` with both bounds when
/// it may or may not be.
fn decide(
    sign: f64,
    lower: Option<f64>,
    upper: Option<f64>,
    need: f64,
    found: &mut Vec<Beside>,
) -> Result<(), (f64, f64)> {
    match (lower, upper) {
        (Some(lower), _) if lower >= need => {
            found.push(Beside {
                sign,
                overlap: lower,
            });
            Ok(())
        }
        (_, None) => Ok(()),
        (_, Some(upper)) if upper < need => Ok(()),
        (lower, Some(upper)) => Err((lower.unwrap_or(0.0).max(0.0), upper)),
    }
}

/// The plan rectangle across `across` and along `along` a wall, as
/// positions along its normal and its length.
fn wall_band(wall: &Wall, across: (f64, f64), along: (f64, f64)) -> Option<Plan> {
    let (v0, v1) = (across.0.min(across.1), across.0.max(across.1));
    let (u0, u1) = along;
    if v1 <= v0 || u1 <= u0 {
        return None;
    }
    let at = |u: f64, v: f64| {
        let point = wall.along * u + wall.normal * v;
        Point2::new(point.x, point.y)
    };
    polygon(vec![at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]).map(Plan::piece)
}

/// How far a plan region reaches along `direction`, `None` when it is empty.
fn extent_along(plan: &Plan, direction: Vec2) -> Option<f64> {
    let (low, high) = plan
        .polygons()
        .iter()
        .flat_map(|polygon| &polygon.outer.points)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
            let at = point.x * direction.x + point.y * direction.y;
            (low.min(at), high.max(at))
        });
    (high >= low).then_some(high - low)
}

/// A region's area, perimeter and number of vertices.
#[allow(clippy::cast_precision_loss)]
fn measure(plan: &Plan) -> (f64, f64, f64) {
    let rings = plan
        .polygons()
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes));
    let (perimeter, vertices) = rings.fold((0.0, 0.0), |(length, count), ring| {
        (
            length + ring_perimeter(ring),
            count + ring.points.len() as f64,
        )
    });
    (plan.area(), perimeter, vertices)
}

/// The least width of a convex hull over every direction, found by
/// rotating calipers over its edges.
fn least_width(hull: &[Point2]) -> f64 {
    (0..hull.len())
        .filter_map(|index| {
            let edge = hull[(index + 1) % hull.len()] - hull[index];
            let length = edge.length();
            (length > ROUNDING).then(|| {
                let normal = Vec2::new(-edge.y, edge.x) / length;
                let (low, high) = span(hull, normal);
                high - low
            })
        })
        .fold(f64::INFINITY, f64::min)
}
