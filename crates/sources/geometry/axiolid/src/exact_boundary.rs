//! Exact boundaries built from the geometry graph a host meshes.
//!
//! A host that compiles an Axiolid [`GeometryGraph`] into a tessellated mesh
//! can ask [`exact_boundary`] for the exact solid of the same graph node, to
//! register beside the mesh ([`AxiolidGeometry::with_exact_boundary`]). It is
//! built only where the kernel's construction is exact:
//!
//! - a chain of instance placements (a single-child collection is looked
//!   through), composed into one transform and applied once with
//!   [`ExactBRep::transformed`]: any rotation, reflection and translation,
//!   so tilted and horizontal bodies and mapped items too. A scale or shear
//!   is refused, never approximated. The solid is exact for the composed
//!   `f64` transform, each coordinate rounded once;
//! - over an extrusion of a rectangle (sharp, rounded or hollow), circle
//!   (filled or hollow), ellipse, structural section or contour of lines and
//!   circular arcs with any number of voids, along the profile normal (or
//!   obliquely where the kernel builds it);
//! - a revolution of the same profiles about the profile's local y axis,
//!   a full turn or part of one, never touching the axis on a full turn;
//! - a disk, solid or bored, swept along one straight segment or one
//!   circular arc, the directrix read by the kernel's own
//!   [`exact_directrix`], as its compilers read it.
//!
//! Everything else is refused with the reason, and the host keeps the mesh
//! alone: an ellipse revolved, a directrix with corners or curved other than
//! by a circle, tapered or directrix sweeps of a profile, booleans (openings,
//! clippings), several items, and anything the kernel refuses.
//!
//! The result carries the axis-aligned extent of the solid it describes,
//! computed in closed form from the construction and the transform, so the
//! host can check it against the mesh before registering it
//! ([`AxiolidGeometry::check_exact_boundary`]).

use std::f64::consts::{PI, TAU};

use axiolid_brep::ExactBRep;
use axiolid_construct::extrude::extrude_profile_exact;
use axiolid_construct::revolve_exact::revolve_profile_exact;
use axiolid_construct::section_lower::{circle_contour, section_contour};
use axiolid_construct::swept_disk_exact::{
    swept_disk_along_arc_exact, swept_disk_along_line_exact,
};
use axiolid_construct::{contour_lower::contour_to_arc_ring, profile_lower::lower_derived};
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{Interval, Point2, Point3, Tolerance, Transform2, Transform3, Vec2, Vec3};
use axiolid_curve::Circle3;
use axiolid_mesh_compile::{ExactDirectrix, exact_directrix};
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use axiolid_overlay::ArcRing;
use axiolid_profile::{CircleProfile, Profile};

use crate::geometry::{AxiolidGeometry, Extent, mesh_extent};

/// Placement, collection and curve relation nodes walked before giving up.
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

    /// The solid's axis-aligned extent as `(min, max)`, computed in closed
    /// form from its construction and placement: arc and surface extremes
    /// included.
    #[must_use]
    pub fn extent(&self) -> ([f64; 3], [f64; 3]) {
        self.extent
    }
}

/// The exact solid of `root` in world coordinates, or why it has none.
///
/// # Errors
///
/// Returns the reason whenever the node is not a rigidly placed extrusion,
/// revolution or swept disk the kernel builds exactly, or the kernel
/// refuses to build or place it.
pub fn exact_boundary(graph: &GeometryGraph, root: NodeId) -> Result<ExactBoundary, String> {
    let (transform, leaf) = placed_solid(graph, root)?;
    let (local, shape, transform) = construct(graph, leaf, transform)?;
    let brep = if transform == Transform3::IDENTITY {
        local
    } else {
        local
            .transformed(&transform)
            .map_err(|error| format!("the placement has no exact rigid copy: {error}"))?
    };
    let extent = shape.extent(&transform);
    if !extent
        .0
        .iter()
        .chain(&extent.1)
        .all(|value| value.is_finite())
    {
        return Err("the solid's extent is not finite".into());
    }
    Ok(ExactBoundary { brep, extent })
}

/// The solid under `root` and the placement composed down to it.
fn placed_solid(graph: &GeometryGraph, root: NodeId) -> Result<(Transform3, NodeId), String> {
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
            Some(GeometryNode::SolidOperation(_)) => return Ok((transform, id)),
            Some(_) => return Err("the body is not a solid with an exact construction".into()),
            None => return Err("the graph does not hold the node".into()),
        }
    }
    Err("the placement chain is too deep".into())
}

/// The solid of `leaf` in its own coordinates, the shape its extent is
/// computed from, and the transform that places it (the given one, or with
/// a reflection added for an extrusion against the profile normal).
fn construct(
    graph: &GeometryGraph,
    leaf: NodeId,
    transform: Transform3,
) -> Result<(ExactBRep, Shape, Transform3), String> {
    match graph.get(leaf) {
        Some(GeometryNode::SolidOperation(SolidOperation::Extrusion {
            profile,
            direction,
            depth,
        })) => {
            let profile = profile_of(graph, *profile)?;
            if !direction.is_finite() || !depth.is_finite() || *depth <= 0.0 {
                return Err("the extrusion is degenerate".into());
            }
            let length = direction.length();
            if length <= 0.0 || direction.z == 0.0 {
                return Err("the extrusion does not leave the profile plane".into());
            }
            let section = Section::of(profile)?;
            // The kernel extrudes along the profile normal's side only; an
            // extrusion against it is the mirror image in the profile plane
            // of one along it, so the reflection joins the placement.
            let (built, placement) = if direction.z > 0.0 {
                (*direction, transform)
            } else {
                (
                    Vec3::new(direction.x, direction.y, -direction.z),
                    transform * Transform3::from_scale(Vec3::new(1.0, 1.0, -1.0)),
                )
            };
            let local = extrude_profile_exact(&buildable(profile)?, built, *depth, TOLERANCE)
                .map_err(refused)?;
            // In the coordinates the placement (with its reflection) maps.
            let offset = built / length * *depth;
            Ok((local, Shape::Prism { section, offset }, placement))
        }
        Some(GeometryNode::SolidOperation(SolidOperation::Revolution {
            profile,
            axis_origin,
            axis_direction,
            angle,
        })) => {
            let profile = profile_of(graph, *profile)?;
            let local =
                revolve_profile_exact(profile, *axis_origin, *axis_direction, *angle, TOLERANCE)
                    .map_err(refused)?;
            let section = Section::of(profile)?;
            let shape = Shape::revolved(section, *axis_origin, *axis_direction, *angle)?;
            Ok((local, shape, transform))
        }
        Some(GeometryNode::SolidOperation(SolidOperation::SweptDisk {
            directrix,
            radius,
            inner_radius,
            parameter_range,
            fillet_radius: _,
        })) => {
            // A single segment or arc has no corners, so a fillet radius has
            // nothing to round.
            let spine = match exact_directrix(
                graph,
                *directrix,
                *parameter_range,
                &ExecutionOptions::new(TOLERANCE),
            )
            .map_err(refused)?
            {
                ExactDirectrix::Segment(start, end) => Spine::Segment(start, end),
                ExactDirectrix::Arc(circle, span) => Spine::Arc(circle, span),
                _ => return Err("the directrix has a form this adapter does not build".into()),
            };
            let local = match &spine {
                Spine::Segment(start, end) => {
                    swept_disk_along_line_exact(*start, *end, *radius, *inner_radius, TOLERANCE)
                }
                Spine::Arc(circle, span) => {
                    swept_disk_along_arc_exact(circle, *span, *radius, *inner_radius, TOLERANCE)
                }
            }
            .map_err(refused)?;
            Ok((
                local,
                Shape::Tube {
                    spine,
                    radius: *radius,
                },
                transform,
            ))
        }
        Some(GeometryNode::SolidOperation(operation)) => Err(format!(
            "{} has no exact construction",
            solid_family(operation)
        )),
        _ => Err("the body is not a solid with an exact construction".into()),
    }
}

fn profile_of(graph: &GeometryGraph, id: NodeId) -> Result<&Profile, String> {
    match graph.get(id) {
        Some(GeometryNode::Profile(profile)) => Ok(profile),
        _ => Err("the solid has no profile".into()),
    }
}

/// The profile as the kernel's exact extrusion takes it: a hollow circle as
/// its contour (the dedicated circle path takes a full disk only).
fn buildable(profile: &Profile) -> Result<Profile, String> {
    Ok(match profile {
        Profile::Circle(
            circle @ CircleProfile {
                thickness: Some(_), ..
            },
        ) => Profile::Contour(circle_contour(circle).map_err(refused)?),
        other => other.clone(),
    })
}

/// A solid operation's name, for refusals.
fn solid_family(operation: &SolidOperation) -> &'static str {
    match operation {
        SolidOperation::TaperedExtrusion { .. } => "a tapered extrusion",
        SolidOperation::TaperedRevolution { .. } => "a tapered revolution",
        SolidOperation::FixedReferenceSweep { .. } => "a fixed-reference sweep",
        SolidOperation::SurfaceCurveSweep { .. } => "a surface-curve sweep",
        SolidOperation::SectionedSpine { .. } => "a sectioned spine",
        SolidOperation::Boolean { .. } => "a boolean result (an opening or a clipping)",
        SolidOperation::BoundedHalfSpace { .. } => "a bounded half-space",
        _ => "this solid",
    }
}

fn refused(error: impl std::fmt::Display) -> String {
    format!("the kernel refused the exact construction: {error}")
}

/// A profile's outer boundary in its own plane, for its extent.
enum Section {
    /// An outer ring of lines and arcs.
    Ring(ArcRing),
    /// An ellipse about the origin with these semi-axes along x and y.
    Ellipse(f64, f64),
}

impl Section {
    fn of(profile: &Profile) -> Result<Self, String> {
        // A section has no derived form of its own; its exact contour does.
        let basis = match profile {
            Profile::Section(section) => {
                Profile::Contour(section_contour(section).map_err(refused)?)
            }
            Profile::Ellipse(ellipse) => {
                return Ok(Self::Ellipse(ellipse.semi_axis_x, ellipse.semi_axis_y));
            }
            other => other.clone(),
        };
        let contour =
            match lower_derived(&basis, &Transform2::IDENTITY, TOLERANCE).map_err(refused)? {
                Profile::Contour(contour) => contour,
                Profile::Circle(circle) => circle_contour(&circle).map_err(refused)?,
                _ => return Err("the profile has no exact contour".into()),
            };
        Ok(Self::Ring(
            contour_to_arc_ring(&contour.outer, TOLERANCE).map_err(refused)?,
        ))
    }

    /// The largest `direction · p` over the section.
    fn support(&self, direction: Vec2) -> f64 {
        match self {
            Self::Ellipse(a, b) => (a * direction.x).hypot(b * direction.y),
            Self::Ring(ring) => {
                let count = ring.vertices.len();
                ring.vertices
                    .iter()
                    .enumerate()
                    .map(|(index, vertex)| {
                        let at_vertex = direction.dot(vertex.point);
                        if vertex.bulge == 0.0 {
                            return at_vertex;
                        }
                        let end = ring.vertices[(index + 1) % count].point;
                        arc_support(vertex.point, end, vertex.bulge, direction)
                            .map_or(at_vertex, |on_arc| on_arc.max(at_vertex))
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            }
        }
    }
}

/// The largest `direction · p` over the inside of the arc from `start` to
/// `end` with `bulge` (`tan` of a quarter of its signed sweep, positive
/// counter-clockwise), when the arc passes the point facing `direction`.
fn arc_support(start: Point2, end: Point2, bulge: f64, direction: Vec2) -> Option<f64> {
    let chord = end - start;
    let length = chord.length();
    let reach = direction.length();
    if length == 0.0 || reach == 0.0 {
        return None;
    }
    let left = Point2::new(-chord.y, chord.x) / length;
    let midpoint = (start + end) / 2.0;
    let centre = midpoint + left * (length * (1.0 - bulge * bulge) / (4.0 * bulge));
    let radius = (start - centre).length();
    let sweep = 4.0 * bulge.atan();
    let from = (start.y - centre.y).atan2(start.x - centre.x);
    let facing = direction.y.atan2(direction.x);
    // How far along the sweep's own sense the direction lies.
    let along = ((facing - from) * sweep.signum()).rem_euclid(TAU);
    (along <= sweep.abs()).then(|| direction.dot(centre) + reach * radius)
}

/// The least and largest `cos(θ − α)` for `θ` from `low` to `high`.
fn cosine_range(low: f64, high: f64, alpha: f64) -> (f64, f64) {
    if high - low >= TAU {
        return (-1.0, 1.0);
    }
    let reaches = |angle: f64| low + (angle - low).rem_euclid(TAU) <= high;
    let ends = [(low - alpha).cos(), (high - alpha).cos()];
    let least = if reaches(alpha + PI) {
        -1.0
    } else {
        ends[0].min(ends[1])
    };
    let largest = if reaches(alpha) {
        1.0
    } else {
        ends[0].max(ends[1])
    };
    (least, largest)
}

/// The directrix an exact swept disk follows, as
/// [`exact_directrix`] reads it.
enum Spine {
    Segment(Point3, Point3),
    /// Over an angle span of the circle, start to end.
    Arc(Circle3, Interval),
}

/// What a solid's extent is computed from, in its own coordinates.
enum Shape {
    /// The section (in the plane `z = 0`) swept by `offset`.
    Prism { section: Section, offset: Vec3 },
    /// The section turned about the in-plane axis through `origin` along
    /// `axis` by every angle from zero to `angle`; `radial` is the in-plane
    /// unit normal to the axis on the section's side.
    Revolved {
        section: Section,
        origin: Point3,
        axis: Vec3,
        radial: Vec3,
        angle: f64,
    },
    /// A disk of `radius` kept square to the spine.
    Tube { spine: Spine, radius: f64 },
}

impl Shape {
    fn revolved(section: Section, origin: Point3, axis: Vec3, angle: f64) -> Result<Self, String> {
        let axis = axis.normalize();
        let radial = Vec3::Z.cross(axis);
        let plan = |v: Vec3| Vec2::new(v.x, v.y);
        let at = plan(radial).dot(plan(origin));
        // Distances from the axis on either side; the kernel has refused a
        // section crossing it.
        let beyond = section.support(plan(radial)) - at;
        let short = section.support(-plan(radial)) + at;
        let radial = if short <= TOLERANCE.linear() {
            radial
        } else if beyond <= TOLERANCE.linear() {
            -radial
        } else {
            return Err("the section crosses the revolution's axis".into());
        };
        Ok(Self::Revolved {
            section,
            origin,
            axis,
            radial,
            angle,
        })
    }

    /// The axis-aligned extent once placed by `transform`.
    fn extent(&self, transform: &Transform3) -> Extent {
        let mut min = [0.0; 3];
        let mut max = [0.0; 3];
        let linear = |v: Vec3| transform.transform_vector3(v);
        let plan = |v: Vec3| Vec2::new(v.x, v.y);
        for k in 0..3 {
            let (low, high) = match self {
                Self::Prism { section, offset } => {
                    let base = transform.translation[k];
                    let across = Vec2::new(linear(Vec3::X)[k], linear(Vec3::Y)[k]);
                    let along = linear(*offset)[k];
                    (
                        base - section.support(-across) + along.min(0.0),
                        base + section.support(across) + along.max(0.0),
                    )
                }
                Self::Revolved {
                    section,
                    origin,
                    axis,
                    radial,
                    angle,
                } => {
                    // A point at height `h` along the axis and `ρ ≥ 0` from
                    // it, turned by θ, lies at `h n + ρ (cos θ m + sin θ w)`
                    // from the axis origin: `ρ A cos(θ − α)` across, so the
                    // extremes over the turn are those of the section in
                    // the direction `n_k n + A c m` for the extreme `c`.
                    let base = transform.transform_point3(*origin)[k];
                    let (n, m, w) = (
                        linear(*axis)[k],
                        linear(*radial)[k],
                        linear(axis.cross(*radial))[k],
                    );
                    let (reach, alpha) = (m.hypot(w), w.atan2(m));
                    let (least, largest) = cosine_range(angle.min(0.0), angle.max(0.0), alpha);
                    let at = |c: f64| plan(*axis) * n + plan(*radial) * (reach * c);
                    let o = plan(*origin);
                    let (down, up) = (at(least), at(largest));
                    (
                        base - section.support(-down) - down.dot(o),
                        base + section.support(up) - up.dot(o),
                    )
                }
                Self::Tube { spine, radius } => match spine {
                    Spine::Segment(start, end) => {
                        let (start, end) = (
                            transform.transform_point3(*start),
                            transform.transform_point3(*end),
                        );
                        let direction = (end - start).normalize();
                        let across = radius * (1.0 - direction[k] * direction[k]).max(0.0).sqrt();
                        (start[k].min(end[k]) - across, start[k].max(end[k]) + across)
                    }
                    Spine::Arc(circle, span) => {
                        let frame = &circle.frame;
                        let centre = transform.transform_point3(frame.origin)[k];
                        let (x, y, z) =
                            (linear(frame.x)[k], linear(frame.y)[k], linear(frame.z)[k]);
                        let (reach, alpha) = (x.hypot(y), y.atan2(x));
                        let (low, high) = (span.start.min(span.end), span.start.max(span.end));
                        let (least, largest) = cosine_range(low, high, alpha);
                        // At angle θ the spine lies `R ρ cos(θ − α)` across
                        // and the disk, spanning the radial direction and
                        // the circle's normal, reaches `r √(ρ²c² + z²)`
                        // further; both ends grow with `c` while `r < R`.
                        let bend = circle.radius;
                        let disk = |c: f64| radius * (reach * reach * c * c + z * z).sqrt();
                        (
                            centre + bend * reach * least - disk(least),
                            centre + bend * reach * largest + disk(largest),
                        )
                    }
                },
            };
            min[k] = low;
            max[k] = high;
        }
        (min, max)
    }
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
#[allow(clippy::float_cmp)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::{arc_support, cosine_range};
    use axiolid_core::{Point2, Vec2};

    #[test]
    fn arc_support_follows_the_sweep() {
        // A counter-clockwise quarter from +x to +y about the origin faces
        // the diagonal but not -x; the clockwise one the long way round
        // faces -x.
        let (east, north) = (Point2::new(1.0, 0.0), Point2::new(0.0, 1.0));
        let quarter = (FRAC_PI_2 / 4.0).tan();
        let reach = arc_support(east, north, quarter, Vec2::new(1.0, 1.0)).unwrap();
        assert!((reach - 2.0_f64.sqrt()).abs() < 1e-12, "{reach}");
        assert_eq!(arc_support(east, north, quarter, -Vec2::X), None);
        let long = -(3.0 * FRAC_PI_2 / 4.0).tan();
        let reach = arc_support(east, north, long, -Vec2::X).unwrap();
        assert!((reach - 1.0).abs() < 1e-12, "{reach}");
    }

    #[test]
    fn cosine_range_spans_the_angles_reached() {
        assert_eq!(cosine_range(0.0, 2.0 * PI, 1.0), (-1.0, 1.0));
        // From 0 to a quarter about 0, the cosine runs from 1 down to 0.
        let (least, largest) = cosine_range(0.0, FRAC_PI_2, 0.0);
        assert!(least.abs() < 1e-12 && largest == 1.0);
        // About π the same quarter runs from -1 up to 0.
        let (least, largest) = cosine_range(0.0, FRAC_PI_2, PI);
        assert!(least == -1.0 && largest.abs() < 1e-12);
        // A span past a turn's end reaches angles beyond a turn.
        let (_, largest) = cosine_range(1.5 * PI, 2.25 * PI, 0.0);
        assert_eq!(largest, 1.0);
    }
}
