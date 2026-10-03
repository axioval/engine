//! Exact boundaries built from the geometry graph a host meshes.
//!
//! A host that compiles an Axiolid [`GeometryGraph`] into a tessellated mesh
//! can ask [`exact_boundary`] for the exact body of the same graph node, to
//! register beside the mesh ([`AxiolidGeometry::with_exact_body`]). It is
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
//!   [`exact_directrix`], as its compilers read it;
//! - a difference of placed extrusions (a wall or slab less its openings,
//!   an `IfcBooleanResult` difference), several nested, built by the
//!   kernel's own [`ReferenceExactCompiler`] (axiolid/kernel#228). Its
//!   general boolean decides faces that agree only up to rounding within
//!   the tolerance it is given, and the result is then the exact boolean of
//!   operands moved by at most that tolerance. So a difference is first
//!   built with no tolerance at all, where nothing can have been snapped,
//!   and only where that is refused with [`Tolerance::METRE`]: the body is
//!   then marked perturbed ([`ExactBody::perturbation_metres`]) by the
//!   linear tolerance plus the angular one over the body's extent, and
//!   every distance measured on it is widened by that (and never cited
//!   as exact);
//! - a collection of several such items (axiolid/kernel#229): each item is
//!   built in the body's own frame (under its own placements below the
//!   collection) and the placement above the collection is kept apart, as
//!   the kernel's body measurements take it ([`ExactBody::placement`]).
//!
//! Everything else is refused with the reason, and the host keeps the mesh
//! alone: an ellipse revolved, a directrix with corners or curved other than
//! by a circle, tapered or directrix sweeps of a profile, unions and
//! intersections, operands that are no extrusions, half-space clippings,
//! and anything the kernel refuses.
//!
//! The result carries the axis-aligned extent of the body it describes,
//! computed in closed form from the construction and the transform (a
//! difference by its subject's, which encloses it), so the host can check
//! it against the mesh before registering it
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
use axiolid_core::{
    BooleanOperator, Interval, Point2, Point3, Tolerance, Transform2, Transform3, Vec2, Vec3,
};
use axiolid_curve::Circle3;
use axiolid_exact_compile_contract::ExactCompiler;
use axiolid_mesh_compile::{ExactDirectrix, ReferenceExactCompiler, exact_directrix};
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use axiolid_overlay::ArcRing;
use axiolid_profile::{CircleProfile, Profile};

use crate::geometry::{AxiolidGeometry, Extent, mesh_extent};

/// Placement, collection and curve relation nodes walked before giving up.
const MAX_DEPTH: usize = 64;

/// The kernel's tolerance for geometry already in metres.
const TOLERANCE: Tolerance = Tolerance::METRE;

/// An object's exact body: one or more exact solids (its items) in the
/// body's own frame, the rigid placement that puts them in the world, and
/// how far any face may have been moved to build them.
///
/// A body of one item built by [`exact_boundary`] is placed in the world
/// already ([`Self::placement`] is the identity).
#[derive(Clone, Debug)]
pub struct ExactBody {
    items: Vec<ExactBRep>,
    placement: Transform3,
    perturbation: f64,
}

impl ExactBody {
    /// One exact solid, in world coordinates, exactly as the model states it.
    #[must_use]
    pub fn new(brep: ExactBRep) -> Self {
        Self::placed(vec![brep], Transform3::IDENTITY)
    }

    /// Several items in the body's own frame, placed by `placement`.
    #[must_use]
    pub fn placed(items: Vec<ExactBRep>, placement: Transform3) -> Self {
        Self {
            items,
            placement,
            perturbation: 0.0,
        }
    }

    /// The same body, built from operands whose faces may each have been
    /// moved by up to `metres` (a boolean that decided near-coincident faces
    /// within its tolerance). Every distance measured on it is widened by
    /// that much and reported inexact. A negative or non-finite value is
    /// kept as unbounded, and such a body is never measured.
    #[must_use]
    pub fn with_perturbation(mut self, metres: f64) -> Self {
        self.perturbation = if metres.is_finite() && metres >= 0.0 {
            self.perturbation.max(metres)
        } else {
            f64::INFINITY
        };
        self
    }

    /// The items, in the body's own frame.
    #[must_use]
    pub fn items(&self) -> &[ExactBRep] {
        &self.items
    }

    /// The rigid placement of the body's frame in the world.
    #[must_use]
    pub fn placement(&self) -> Transform3 {
        self.placement
    }

    /// How far a face of the body may lie from the model's: zero when the
    /// body is exactly the model's solid.
    #[must_use]
    pub fn perturbation_metres(&self) -> f64 {
        self.perturbation
    }

    /// Whether the body is exactly the model's solid.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.perturbation == 0.0
    }

    /// The single item in world coordinates, which the kernel's one-solid
    /// measurements (plan distances and overlap) take; `None` for several
    /// items or a body kept in its own frame.
    #[must_use]
    pub fn single(&self) -> Option<&ExactBRep> {
        match self.items.as_slice() {
            [brep] if self.placement == Transform3::IDENTITY => Some(brep),
            _ => None,
        }
    }
}

/// An exact body and the axis-aligned extent it occupies, in metres.
#[derive(Clone, Debug)]
pub struct ExactBoundary {
    body: ExactBody,
    extent: Extent,
}

impl ExactBoundary {
    /// The exact body.
    #[must_use]
    pub fn body(&self) -> &ExactBody {
        &self.body
    }

    /// The exact solid of a body of one item, in world coordinates.
    #[must_use]
    pub fn brep(&self) -> Option<&ExactBRep> {
        self.body.single()
    }

    /// The exact body, for [`AxiolidGeometry::with_exact_body`].
    #[must_use]
    pub fn into_body(self) -> ExactBody {
        self.body
    }

    /// The body's axis-aligned extent as `(min, max)`, computed in closed
    /// form from its construction and placement: arc and surface extremes
    /// included, a difference by its subject's.
    #[must_use]
    pub fn extent(&self) -> ([f64; 3], [f64; 3]) {
        self.extent
    }
}

/// The exact body of `root` in world coordinates, or why it has none.
///
/// # Errors
///
/// Returns the reason whenever the node is not a rigidly placed extrusion,
/// revolution, swept disk or difference of extrusions the kernel builds
/// exactly (or a collection of them), or the kernel refuses to build or
/// place it.
pub fn exact_boundary(graph: &GeometryGraph, root: NodeId) -> Result<ExactBoundary, String> {
    let (placement, members) = placed_members(graph, root)?;
    let mut items = Vec::with_capacity(members.len());
    let mut perturbed = false;
    let mut extent: Option<Extent> = None;
    for (transform, leaf) in members {
        let item = item(graph, leaf, transform)?;
        // Each item's extent in the world, through the outer placement.
        let placed = item.shape.extent(&(placement * item.transform));
        extent = Some(match extent {
            None => placed,
            Some((min, max)) => (
                std::array::from_fn(|k| min[k].min(placed.0[k])),
                std::array::from_fn(|k| max[k].max(placed.1[k])),
            ),
        });
        perturbed |= item.perturbed;
        items.push(item.brep);
    }
    let extent = extent.ok_or("the body has no solid")?;
    if !extent
        .0
        .iter()
        .chain(&extent.1)
        .all(|value| value.is_finite())
    {
        return Err("the solid's extent is not finite".into());
    }
    // A face turned within the angular tolerance turns about a point of
    // the body, so it moves no further than that angle over the body's
    // extent; a face moved along it, no further than the linear tolerance.
    let perturbation = if perturbed {
        let diagonal = (0..3)
            .map(|k| (extent.1[k] - extent.0[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        TOLERANCE.linear() + TOLERANCE.angular() * diagonal
    } else {
        0.0
    };
    // One item is placed in the world, as the one-solid measurements take
    // it; several stay in the body's frame, where their contacts are cut.
    let body = match items.as_slice() {
        [single] => ExactBody::new(place(single.clone(), &placement)?),
        _ => ExactBody::placed(items, placement),
    };
    Ok(ExactBoundary {
        body: body.with_perturbation(perturbation),
        extent,
    })
}

/// The placement above `root`'s solids, composed, and each solid below it
/// with its own placement relative to it: one for a single solid, several
/// for a collection (nested collections flattened).
fn placed_members(
    graph: &GeometryGraph,
    root: NodeId,
) -> Result<(Transform3, Vec<(Transform3, NodeId)>), String> {
    let (placement, node) = placed(graph, root, Transform3::IDENTITY)?;
    let Some(GeometryNode::Collection(children)) = graph.get(node) else {
        return Ok((placement, vec![(Transform3::IDENTITY, node)]));
    };
    let mut members = Vec::new();
    let mut pending: Vec<(Transform3, NodeId, usize)> = children
        .iter()
        .rev()
        .map(|child| (Transform3::IDENTITY, *child, 0))
        .collect();
    while let Some((above, child, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err("the collection is nested too deep".into());
        }
        let (transform, node) = placed(graph, child, above)?;
        match graph.get(node) {
            Some(GeometryNode::Collection(children)) => pending.extend(
                children
                    .iter()
                    .rev()
                    .map(|grandchild| (transform, *grandchild, depth + 1)),
            ),
            _ => members.push((transform, node)),
        }
    }
    if members.is_empty() {
        return Err("the body has no solid".into());
    }
    Ok((placement, members))
}

/// The placements below `root` composed onto `transform`, down to a solid
/// or a collection of several (a single-child collection is looked
/// through).
fn placed(
    graph: &GeometryGraph,
    root: NodeId,
    mut transform: Transform3,
) -> Result<(Transform3, NodeId), String> {
    let mut id = root;
    for _ in 0..MAX_DEPTH {
        match graph.get(id) {
            Some(GeometryNode::Instance(instance)) => {
                transform *= instance.transform;
                id = instance.source;
            }
            Some(GeometryNode::Collection(children)) => match children.as_slice() {
                [child] => id = *child,
                [] => return Err("the body has no solid".into()),
                _ => return Ok((transform, id)),
            },
            Some(GeometryNode::SolidOperation(_)) => return Ok((transform, id)),
            Some(_) => return Err("the body is not a solid with an exact construction".into()),
            None => return Err("the graph does not hold the node".into()),
        }
    }
    Err("the placement chain is too deep".into())
}

/// One solid of a body, in the body's own frame.
struct Item {
    brep: ExactBRep,
    shape: Shape,
    /// The placement of `shape` in the body's frame.
    transform: Transform3,
    /// Whether the kernel decided faces within its tolerance.
    perturbed: bool,
}

/// The solid `leaf` placed by `transform` in the body's frame.
fn item(graph: &GeometryGraph, leaf: NodeId, transform: Transform3) -> Result<Item, String> {
    if let Some(GeometryNode::SolidOperation(SolidOperation::Boolean { .. })) = graph.get(leaf) {
        let (brep, perturbed) = difference(graph, leaf)?;
        return Ok(Item {
            brep: place(brep, &transform)?,
            shape: subject_shape(graph, leaf)?,
            transform,
            perturbed,
        });
    }
    let (local, shape, transform) = construct(graph, leaf, transform)?;
    Ok(Item {
        brep: place(local, &transform)?,
        shape,
        transform,
        perturbed: false,
    })
}

fn place(brep: ExactBRep, transform: &Transform3) -> Result<ExactBRep, String> {
    if *transform == Transform3::IDENTITY {
        return Ok(brep);
    }
    brep.transformed(transform)
        .map_err(|error| format!("the placement has no exact rigid copy: {error}"))
}

/// A difference of placed extrusions, built by the kernel's exact compiler
/// in its own coordinates, and whether faces were decided within a
/// tolerance: first with none, so nothing can be snapped, and only where
/// that is refused within [`TOLERANCE`].
fn difference(graph: &GeometryGraph, node: NodeId) -> Result<(ExactBRep, bool), String> {
    differences_only(graph, node)?;
    let compile = |tolerance: Tolerance| {
        ReferenceExactCompiler::new().compile_exact(graph, node, &ExecutionOptions::new(tolerance))
    };
    match compile(Tolerance::ZERO) {
        Ok(brep) => Ok((brep, false)),
        Err(_) => compile(TOLERANCE).map(|brep| (brep, true)).map_err(refused),
    }
}

/// Refuses a union or intersection by name before the kernel is asked.
fn differences_only(graph: &GeometryGraph, mut node: NodeId) -> Result<(), String> {
    for _ in 0..MAX_DEPTH {
        match graph.get(node) {
            Some(GeometryNode::SolidOperation(SolidOperation::Boolean {
                left, operator, ..
            })) => {
                if *operator != BooleanOperator::Difference {
                    return Err("a boolean union or intersection has no exact construction".into());
                }
                node = *left;
            }
            Some(GeometryNode::Instance(instance)) => node = instance.source,
            _ => return Ok(()),
        }
    }
    Err("the boolean chain is too deep".into())
}

/// The shape of a difference's innermost subject, placed in the
/// difference's coordinates: the difference lies inside it.
fn subject_shape(graph: &GeometryGraph, node: NodeId) -> Result<Shape, String> {
    let mut transform = Transform3::IDENTITY;
    let mut id = node;
    for _ in 0..MAX_DEPTH {
        match graph.get(id) {
            Some(GeometryNode::SolidOperation(SolidOperation::Boolean { left, .. })) => id = *left,
            Some(GeometryNode::Instance(instance)) => {
                transform *= instance.transform;
                id = instance.source;
            }
            Some(GeometryNode::SolidOperation(_)) => {
                let (_, shape, transform) = construct(graph, id, transform)?;
                return Ok(Shape::Placed(Box::new(shape), transform));
            }
            _ => return Err("the boolean's subject is not a solid".into()),
        }
    }
    Err("the boolean chain is too deep".into())
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
        SolidOperation::Boolean { .. } => "a boolean operand",
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
    /// A shape placed by a transform of its own, applied first.
    Placed(Box<Shape>, Transform3),
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
        if let Self::Placed(shape, inner) = self {
            return shape.extent(&(*transform * *inner));
        }
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
                Self::Placed(..) => unreachable!("placed shapes return above"),
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
    /// A cheap guard before [`Self::with_exact_body`], not a proof:
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
