//! Exact boundaries built from the geometry graph a host meshes.
//!
//! A host that compiles an Axiolid [`GeometryGraph`] into a tessellated mesh
//! can ask [`exact_boundary`] for the exact solid of the same graph node, to
//! register beside the mesh ([`AxiolidGeometry::with_exact_boundary`]). It is
//! built only where the kernel's construction is exact and placed in world
//! coordinates without approximation:
//!
//! - an extrusion, under any chain of instance placements (a single-child
//!   collection is looked through), whose profile plane the placement keeps
//!   horizontal and whose extrusion it keeps vertical: a vertical prism.
//!   Placement and mirroring are pushed onto the profile exactly
//!   (`axiolid_construct::profile_lower::lower_derived`), so a circle stays
//!   a circle and an arc an arc;
//! - over rectangle (sharp, rounded or hollow), circle (filled or hollow),
//!   structural section and contour profiles of lines and circular arcs,
//!   with at most one void.
//!
//! Everything else is refused with the reason, and the host keeps the mesh
//! alone: ellipses (an arc ring cannot carry one), revolutions, swept disks
//! and directrix sweeps, tilted or horizontal extrusions (the kernel has no
//! rigid placement for an exact B-rep yet), booleans (openings, clippings),
//! several items, and anything else. The vertical prism is built by the
//! kernel's exact coaxial arc-prism boolean (`boolean_arc_prisms_exact`),
//! the one exact constructor that takes world heights; it takes one ring per
//! operand, so a section with several voids is refused.
//!
//! The result carries the extent of the solid it describes, so the host can
//! check it against the mesh before registering it
//! ([`AxiolidGeometry::check_exact_boundary`]).

use axiolid_brep::ExactBRep;
use axiolid_construct::boolean_exact::{ArcPrism, boolean_arc_prisms_exact};
use axiolid_construct::contour_lower::contour_to_arc_ring;
use axiolid_construct::profile_lower::lower_derived;
use axiolid_construct::section_lower::{circle_contour, section_contour};
use axiolid_core::{BooleanOperator, Point2, Tolerance, Transform2, Transform3, Vec3};
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use axiolid_overlay::{ArcRing, arc_ring_area, reverse_arc_ring};
use axiolid_profile::{ContourProfile, Profile};

use crate::geometry::{AxiolidGeometry, Extent, mesh_extent};

/// Placement and collection nodes walked before giving up on a graph.
const MAX_DEPTH: usize = 64;

/// The kernel's tolerance for geometry already in metres.
const TOLERANCE: Tolerance = Tolerance::METRE;

/// An exact solid and the axis-aligned extent it occupies, in metres.
#[derive(Clone, Debug)]
pub struct ExactBoundary {
    brep: ExactBRep,
    extent: Extent,
}

impl ExactBoundary {
    /// The exact solid.
    #[must_use]
    pub fn brep(&self) -> &ExactBRep {
        &self.brep
    }

    /// The exact solid, for [`AxiolidGeometry::with_exact_boundary`].
    #[must_use]
    pub fn into_brep(self) -> ExactBRep {
        self.brep
    }

    /// The solid's axis-aligned extent as `(min, max)`, computed from the
    /// section and heights it was built from: arc extremes included.
    #[must_use]
    pub fn extent(&self) -> ([f64; 3], [f64; 3]) {
        self.extent
    }
}

/// The exact solid of `root` in world coordinates, or why it has none.
///
/// # Errors
///
/// Returns the reason whenever the node is not a vertically placed
/// extrusion of a supported profile, or the kernel refuses to build it.
pub fn exact_boundary(graph: &GeometryGraph, root: NodeId) -> Result<ExactBoundary, String> {
    let (transform, profile, direction, depth) = placed_extrusion(graph, root)?;
    let matrix = transform.matrix3;
    // The profile plane must stay horizontal and the extrusion vertical,
    // exactly: any tilt would make the solid something other than the
    // vertical prism built here.
    if matrix.x_axis.z != 0.0 || matrix.y_axis.z != 0.0 {
        return Err("the profile is not placed horizontally".into());
    }
    if !direction.is_finite() || direction.length() <= 0.0 || !depth.is_finite() || depth <= 0.0 {
        return Err("the extrusion is degenerate".into());
    }
    // The mesh compiler's extrusion: the normalised direction times depth.
    let offset = matrix * (direction.normalize() * depth);
    if offset.x != 0.0 || offset.y != 0.0 || !offset.z.is_finite() || offset.z == 0.0 {
        return Err("the extrusion is not vertical".into());
    }
    let base = transform.translation.z;
    let (bottom, top) = if offset.z > 0.0 {
        (base, base + offset.z)
    } else {
        (base + offset.z, base)
    };
    if !bottom.is_finite() || !top.is_finite() || top <= bottom {
        return Err("the extrusion is degenerate".into());
    }
    let plan = Transform2::from_cols(
        matrix.x_axis.truncate(),
        matrix.y_axis.truncate(),
        transform.translation.truncate(),
    );
    let (outer, holes) = world_rings(profile, &plan)?;
    let extent = extent(&outer, bottom, top);
    let brep = vertical_prism(outer, holes, (bottom, top), &extent)?;
    Ok(ExactBoundary { brep, extent })
}

/// The extrusion under `root` and the placement composed down to it.
fn placed_extrusion(
    graph: &GeometryGraph,
    root: NodeId,
) -> Result<(Transform3, &Profile, Vec3, f64), String> {
    let mut transform = Transform3::IDENTITY;
    let mut id = root;
    for _ in 0..MAX_DEPTH {
        match graph.get(id) {
            Some(GeometryNode::Instance(instance)) => {
                transform *= instance.transform;
                id = instance.source;
            }
            Some(GeometryNode::Collection(children)) => match children.as_slice() {
                [child] => id = *child,
                _ => return Err("a body of several solids has no exact construction".into()),
            },
            Some(GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction,
                depth,
            })) => {
                return match graph.get(*profile) {
                    Some(GeometryNode::Profile(profile)) => {
                        Ok((transform, profile, *direction, *depth))
                    }
                    _ => Err("the extrusion has no profile".into()),
                };
            }
            Some(GeometryNode::SolidOperation(operation)) => {
                return Err(format!(
                    "{} has no exact construction with a placement",
                    solid_family(operation)
                ));
            }
            Some(_) => return Err("the body is not a solid with an exact construction".into()),
            None => return Err("the graph does not hold the node".into()),
        }
    }
    Err("the placement chain is too deep".into())
}

/// A solid operation's name, for refusals.
fn solid_family(operation: &SolidOperation) -> &'static str {
    match operation {
        SolidOperation::TaperedExtrusion { .. } => "a tapered extrusion",
        SolidOperation::Revolution { .. } => "a revolution",
        SolidOperation::TaperedRevolution { .. } => "a tapered revolution",
        SolidOperation::SweptDisk { .. } => "a swept disk",
        SolidOperation::Boolean { .. } => "a boolean result (an opening or a clipping)",
        _ => "this solid",
    }
}

/// The profile's outer ring and holes in world plan coordinates: outer
/// counter-clockwise, holes clockwise.
fn world_rings(profile: &Profile, plan: &Transform2) -> Result<(ArcRing, Vec<ArcRing>), String> {
    // A section has no derived form of its own; its exact contour does.
    let basis = match profile {
        Profile::Section(section) => Profile::Contour(section_contour(section).map_err(refused)?),
        Profile::Ellipse(_) => return Err("an ellipse has no exact arc section".into()),
        other => other.clone(),
    };
    let contour = match lower_derived(&basis, plan, TOLERANCE).map_err(refused)? {
        Profile::Contour(contour) => contour,
        Profile::Circle(circle) => circle_contour(&circle).map_err(refused)?,
        _ => return Err("the profile has no exact contour".into()),
    };
    let ContourProfile { outer, holes } = contour;
    let oriented = |ring: ArcRing, counter_clockwise: bool| {
        if (arc_ring_area(&ring) > 0.0) == counter_clockwise {
            ring
        } else {
            reverse_arc_ring(&ring)
        }
    };
    let outer = oriented(
        contour_to_arc_ring(&outer, TOLERANCE).map_err(refused)?,
        true,
    );
    let holes = holes
        .iter()
        .map(|hole| {
            contour_to_arc_ring(hole, TOLERANCE)
                .map(|ring| oriented(ring, false))
                .map_err(refused)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((outer, holes))
}

fn refused(error: impl std::fmt::Display) -> String {
    format!("the kernel refused the exact construction: {error}")
}

/// The vertical prism over `outer` less `holes` between `bottom` and `top`.
///
/// The kernel's exact prism constructor at any height is the coaxial boolean
/// of arc prisms, so the section is intersected with a box around it (which
/// leaves it whole), or its one hole subtracted.
fn vertical_prism(
    outer: ArcRing,
    holes: Vec<ArcRing>,
    (bottom, top): (f64, f64),
    extent: &Extent,
) -> Result<ExactBRep, String> {
    let (min, max) = extent;
    let margin = 1.0 + (max[0] - min[0]).max(max[1] - min[1]);
    let frame = [
        Point2::new(min[0] - margin, min[1] - margin),
        Point2::new(max[0] + margin, min[1] - margin),
        Point2::new(max[0] + margin, max[1] + margin),
        Point2::new(min[0] - margin, max[1] + margin),
    ];
    let prism = |section: ArcRing| ArcPrism {
        section,
        bottom,
        top,
    };
    match <[ArcRing; 1]>::try_from(holes) {
        Ok([hole]) => boolean_arc_prisms_exact(
            &prism(outer),
            // The hole as a region of its own, counter-clockwise.
            &prism(reverse_arc_ring(&hole)),
            BooleanOperator::Difference,
            TOLERANCE,
        ),
        Err(holes) if holes.is_empty() => boolean_arc_prisms_exact(
            &prism(outer),
            &prism(ArcRing::from_points(&frame)),
            BooleanOperator::Intersection,
            TOLERANCE,
        ),
        Err(_) => return Err("a section with several voids has no exact construction".into()),
    }
    .map_err(refused)
}

/// The extent of the prism over `outer` between `bottom` and `top`.
fn extent(outer: &ArcRing, bottom: f64, top: f64) -> Extent {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut include = |point: Point2| {
        min = [min[0].min(point.x), min[1].min(point.y)];
        max = [max[0].max(point.x), max[1].max(point.y)];
    };
    let count = outer.vertices.len();
    for (index, vertex) in outer.vertices.iter().enumerate() {
        include(vertex.point);
        if vertex.bulge != 0.0 {
            let end = outer.vertices[(index + 1) % count].point;
            for point in arc_extremes(vertex.point, end, vertex.bulge) {
                include(point);
            }
        }
    }
    ([min[0], min[1], bottom], [max[0], max[1], top])
}

/// The points of the arc from `start` to `end` with `bulge` (`tan` of a
/// quarter of its signed sweep, positive counter-clockwise) that are
/// extreme along the coordinate axes.
fn arc_extremes(start: Point2, end: Point2, bulge: f64) -> Vec<Point2> {
    let chord = end - start;
    let length = chord.length();
    if length == 0.0 {
        return Vec::new();
    }
    let left = Point2::new(-chord.y, chord.x) / length;
    let midpoint = (start + end) / 2.0;
    let centre = midpoint + left * (length * (1.0 - bulge * bulge) / (4.0 * bulge));
    let radius = (start - centre).length();
    let sweep = 4.0 * bulge.atan();
    let from = (start.y - centre.y).atan2(start.x - centre.x);
    [
        (0.0, Point2::X),
        (std::f64::consts::FRAC_PI_2, Point2::Y),
        (std::f64::consts::PI, -Point2::X),
        (-std::f64::consts::FRAC_PI_2, -Point2::Y),
    ]
    .into_iter()
    .filter(|(angle, _)| {
        // How far along the sweep's own sense the direction lies.
        let along = (angle - from) * sweep.signum();
        along.rem_euclid(std::f64::consts::TAU) <= sweep.abs()
    })
    .map(|(_, direction)| centre + direction * radius)
    .collect()
}

impl AxiolidGeometry {
    /// Checks that `boundary` describes the body of `object`'s registered
    /// mesh: their extents must agree within the mesh's chord deviation
    /// (plus rounding) on every side.
    ///
    /// A cheap guard before [`Self::with_exact_boundary`], not a proof:
    /// the proximity service still refuses a pair whose certified answer
    /// contradicts the mesh.
    ///
    /// # Errors
    ///
    /// Returns the disagreement, or why the object has nothing to compare.
    pub fn check_exact_boundary(
        &self,
        object: &axioval_ir::ObjectId,
        boundary: &ExactBoundary,
    ) -> Result<(), String> {
        let mesh = self
            .mesh(object)
            .ok_or_else(|| format!("{object} has no mesh"))?;
        let (mesh_min, mesh_max) =
            mesh_extent(mesh).ok_or_else(|| format!("{object} has an empty mesh"))?;
        let deviation = self
            .fidelity(object)
            .map_err(|error| error.to_string())?
            .deviation_metres();
        let (min, max) = boundary.extent;
        let magnitude = min
            .iter()
            .chain(&max)
            .chain(&mesh_min)
            .chain(&mesh_max)
            .fold(1.0_f64, |largest, value| largest.max(value.abs()));
        let allowed = deviation + 1e-9 + 1e-12 * magnitude;
        for axis in 0..3 {
            let gap = (min[axis] - mesh_min[axis])
                .abs()
                .max((max[axis] - mesh_max[axis]).abs());
            if gap.is_nan() || gap > allowed {
                return Err(format!(
                    "the exact boundary's extent differs from the mesh's by {gap} m along axis \
                     {axis}, more than the {allowed} m allowed"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{arc_extremes, exact_boundary};
    use axiolid_core::{Point2, Transform3, Vec3};
    use axiolid_model::{GeometryGraphBuilder, GeometryNode, Instance, SolidOperation};
    use axiolid_profile::{CircleProfile, EllipseProfile, Profile, RectangleProfile};

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-12)
    }

    /// `profile` extruded by `depth` along `direction`, placed by `transform`.
    fn placed(
        profile: Profile,
        direction: Vec3,
        depth: f64,
        transform: Transform3,
    ) -> Result<super::ExactBoundary, String> {
        let mut builder = GeometryGraphBuilder::new();
        let profile = builder.push(GeometryNode::Profile(profile)).unwrap();
        let extrusion = builder
            .push(GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction,
                depth,
            }))
            .unwrap();
        let root = builder
            .push(GeometryNode::Instance(Instance {
                source: extrusion,
                transform,
            }))
            .unwrap();
        let graph = builder.finish(vec![root]).unwrap();
        exact_boundary(&graph, root)
    }

    fn circle(radius: f64) -> Profile {
        Profile::Circle(CircleProfile {
            radius,
            thickness: None,
        })
    }

    #[test]
    fn a_placed_round_column_is_a_vertical_prism() {
        let boundary = placed(
            circle(0.2),
            Vec3::Z,
            3.0,
            Transform3::from_translation(Vec3::new(5.0, -2.0, 1.5)),
        )
        .unwrap();
        let (min, max) = boundary.extent();
        assert!(close(min, [4.8, -2.2, 1.5]), "{min:?}");
        assert!(close(max, [5.2, -1.8, 4.5]), "{max:?}");
    }

    #[test]
    fn a_rotated_mirrored_and_flipped_rectangle_keeps_its_extent() {
        // Mirrored in x, turned a quarter about z and extruded downwards.
        let transform = Transform3::from_translation(Vec3::new(1.0, 2.0, 3.0))
            * Transform3::from_rotation_z(std::f64::consts::FRAC_PI_2)
            * Transform3::from_scale(Vec3::new(-1.0, 1.0, -1.0));
        let boundary = placed(
            Profile::Rectangle(RectangleProfile {
                x: 2.0,
                y: 0.5,
                thickness: None,
                outer_radius: None,
                inner_radius: None,
            }),
            Vec3::Z,
            1.0,
            transform,
        )
        .unwrap();
        let (min, max) = boundary.extent();
        assert!(close(min, [0.75, 1.0, 2.0]), "{min:?}");
        assert!(close(max, [1.25, 3.0, 3.0]), "{max:?}");
    }

    #[test]
    fn a_hollow_circle_is_built_with_its_bore() {
        let boundary = placed(
            Profile::Circle(CircleProfile {
                radius: 0.3,
                thickness: Some(0.05),
            }),
            Vec3::Z,
            2.0,
            Transform3::from_translation(Vec3::new(1.0, 1.0, 0.0)),
        )
        .unwrap();
        let (min, max) = boundary.extent();
        assert!(close(min, [0.7, 0.7, 0.0]), "{min:?}");
        assert!(close(max, [1.3, 1.3, 2.0]), "{max:?}");
    }

    #[test]
    fn tilted_horizontal_and_elliptical_bodies_are_refused() {
        let horizontal = Transform3::from_rotation_x(std::f64::consts::FRAC_PI_2);
        assert!(placed(circle(0.2), Vec3::Z, 3.0, horizontal).is_err());
        assert!(
            placed(
                circle(0.2),
                Vec3::new(1.0, 0.0, 1.0),
                3.0,
                Transform3::IDENTITY
            )
            .is_err()
        );
        let ellipse = Profile::Ellipse(EllipseProfile {
            semi_axis_x: 0.3,
            semi_axis_y: 0.2,
        });
        assert!(placed(ellipse, Vec3::Z, 3.0, Transform3::IDENTITY).is_err());
    }

    #[test]
    fn arc_extremes_follow_the_sweep() {
        // A counter-clockwise quarter from +x to +y about the origin passes
        // no other axis direction; the clockwise one the long way round
        // passes the other three.
        let (east, north) = (Point2::new(1.0, 0.0), Point2::new(0.0, 1.0));
        let quarter = (std::f64::consts::FRAC_PI_2 / 4.0).tan();
        let short = arc_extremes(east, north, quarter);
        assert!(short.iter().all(|p| p.x > -0.5 && p.y > -0.5), "{short:?}");
        let long = arc_extremes(
            east,
            north,
            -(3.0 * std::f64::consts::FRAC_PI_2 / 4.0).tan(),
        );
        assert_eq!(long.len(), 4, "{long:?}");
        assert!(
            long.iter()
                .any(|p| (p.x + 1.0).abs() < 1e-12 && p.y.abs() < 1e-12)
        );
    }
}
