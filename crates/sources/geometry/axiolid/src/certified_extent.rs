//! Certified vertical extents read from the geometry graph a host meshes.
//!
//! A tessellated body's mesh lies within its chord deviation of the true
//! surface, so every face may move by that deviation, the flat top of a
//! slab with a curved outline included: widened, a slab whose top meets a
//! floor rises into the room above. Where the construction states the
//! body's lowest and highest elevations exactly, [`certified_vertical_extent`]
//! reads them from the graph, whatever the plan profile is (a B-spline
//! outline the kernel only tessellates included), for
//! [`AxiolidGeometry::with_vertical_extent_bounds`](crate::AxiolidGeometry::with_vertical_extent_bounds):
//!
//! - an extrusion whose profile plane, after every placement above it, is
//!   horizontal (its local x and y axes have no vertical component, exactly
//!   as composed): the profile lies at one elevation and the sweep adds
//!   the same vertical offset to every point, so the solid spans the two
//!   caps' elevations exactly, both reached. The direction need not be
//!   vertical: every face bounding the solid's height is a horizontal cap;
//! - an extrusion whose profile plane is tilted (a pipe lying down, a wall
//!   profile stood upright, #475): a profile point `(u, v)` rises by
//!   `u * x + v * y` and the sweep adds `0..=along`, so the solid lies in
//!   the box of those elevations over the profile's box, with no claim that
//!   it reaches them. The box is read where the construction states it: a
//!   rectangle or ellipse about its origin, a circle, a contour of
//!   polylines and arcs (an arc by its whole circle or ellipse), their
//!   placements and collections. A section family, a centre line or a
//!   contour with any other segment states none;
//! - an authored mesh (a triangulated or polygonal face set) or a B-rep
//!   whose edges and faces are all straight and planar (a faceted brep):
//!   the hull of the vertices its faces use, placed by the transform, both
//!   ends reached by a vertex;
//! - a difference, and a clip or other intersection: within its subject's
//!   (an intersection within either operand's, the tighter where both are
//!   read), with no claim that it reaches them, since a boolean may remove
//!   a cap;
//! - a union or a collection: the hull of its members', each reached where
//!   every member's is.
//!
//! Every other node (a revolution, a swept disk or profile, a half-space
//! alone, a B-rep with a curved edge or face, a tapered extrusion, a
//! primitive) is refused with the reason, and the host keeps the
//! deviation-widened extent. The
//! elevations are computed from the composed `f64` placement, as the
//! compiler places the mesh, and widened outward (narrowed inward) by 64
//! units in the last place of their magnitude, so a compiler composing the
//! same placements in another order stays inside them.

use axiolid_core::{BooleanOperator, Frame2, Transform3, Vec2, Vec3};
use axiolid_curve::Curve2;
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use axiolid_profile::Profile;

use crate::geometry::VerticalExtentBounds;

/// Placement and collection levels walked before giving up.
const MAX_DEPTH: usize = 64;

/// Nodes visited before giving up: a body is never worth more.
const MAX_VISITS: usize = 4096;

/// Units in the last place, of an elevation's magnitude, by which a cap's
/// elevation may differ between two orders of composing its placements.
const ROUNDING_ULPS: f64 = 64.0;

/// Certified bounds on the elevations of `root`'s solid: it lies between
/// `outer`, and, where `inner` is given, reaches below its low end and
/// above its high end.
///
/// # Errors
///
/// Returns the reason when the graph does not state the solid's height
/// exactly (see the module documentation), or the graph is malformed.
pub fn certified_vertical_extent(
    graph: &GeometryGraph,
    root: NodeId,
) -> Result<VerticalExtentBounds, String> {
    let mut walk = Walk { visits: 0 };
    let height = walk.height(graph, root, Transform3::IDENTITY, 0)?;
    Ok(VerticalExtentBounds {
        outer: height.outer,
        inner: height.inner,
    })
}

/// The subjects `root`'s solid lies within as a difference (#401), each
/// with the placement composed above it, outermost first: walking down
/// instances (and collections of one member), every left operand of a
/// difference is a solid its result lies within, since a difference only
/// removes material from its subject. A clip or other intersection, a
/// union, or any other node ends the walk.
///
/// A host whose body is refused (a curved wall less its openings, whose
/// exact boolean cannot classify a point) compiles these in turn and
/// bounds the body by the first it can measure: an enclosure
/// ([`AxiolidGeometry::with_unmeasured_enclosure`](crate::AxiolidGeometry::with_unmeasured_enclosure)).
/// Each subject is compiled in its own frame, so its mesh is to be placed
/// by the transform given with it.
///
/// Empty when `root` is no difference, or the graph is malformed: then no
/// subject bounds the body.
#[must_use]
pub fn difference_subjects(graph: &GeometryGraph, root: NodeId) -> Vec<(NodeId, Transform3)> {
    let mut subjects = Vec::new();
    let mut transform = Transform3::IDENTITY;
    let mut id = root;
    for _ in 0..MAX_VISITS {
        match graph.get(id) {
            Some(GeometryNode::Instance(instance)) => {
                transform *= instance.transform;
                id = instance.source;
            }
            Some(GeometryNode::Collection(children)) if children.len() == 1 => id = children[0],
            Some(GeometryNode::SolidOperation(SolidOperation::Boolean {
                left,
                operator: BooleanOperator::Difference,
                ..
            })) => {
                subjects.push((*left, transform));
                id = *left;
            }
            _ => break,
        }
    }
    subjects
}

/// A node's elevations: within `outer`, reaching `inner` where known.
#[derive(Clone, Copy, Debug)]
struct Height {
    outer: (f64, f64),
    inner: Option<(f64, f64)>,
}

impl Height {
    /// The hull of two members of a union: each reached where both are.
    fn hull(self, other: Self) -> Self {
        Self {
            outer: (
                self.outer.0.min(other.outer.0),
                self.outer.1.max(other.outer.1),
            ),
            inner: match (self.inner, other.inner) {
                (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            },
        }
    }

    /// As the result of a difference with this subject when `subject` is
    /// set: within its elevations only, since a boolean may remove what
    /// reached their ends.
    fn reached(self, subject: bool) -> Self {
        Self {
            outer: self.outer,
            inner: self.inner.filter(|_| !subject),
        }
    }
}

struct Walk {
    visits: usize,
}

impl Walk {
    fn height(
        &mut self,
        graph: &GeometryGraph,
        node: NodeId,
        mut transform: Transform3,
        depth: usize,
    ) -> Result<Height, String> {
        let mut id = node;
        // Set once a difference's subject is followed: the result lies
        // within the subject's elevations without surely reaching them. A
        // chain of openings is walked along its subjects, never nested.
        let mut subject = false;
        for _ in 0..MAX_VISITS {
            self.visits += 1;
            if self.visits > MAX_VISITS {
                return Err("the body's graph is too large to read its height".into());
            }
            match graph.get(id) {
                Some(GeometryNode::Instance(instance)) => {
                    transform *= instance.transform;
                    id = instance.source;
                }
                Some(GeometryNode::Collection(children)) => {
                    let mut hull: Option<Height> = None;
                    for child in children {
                        let member = self.nested(graph, *child, transform, depth)?;
                        hull = Some(hull.map_or(member, |so_far| so_far.hull(member)));
                    }
                    return hull
                        .map(|hull| hull.reached(subject))
                        .ok_or_else(|| "the body has no solid".into());
                }
                Some(GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left,
                    right,
                    operator,
                })) => match operator {
                    BooleanOperator::Difference => {
                        subject = true;
                        id = *left;
                    }
                    BooleanOperator::Union => {
                        let a = self.nested(graph, *left, transform, depth)?;
                        let b = self.nested(graph, *right, transform, depth)?;
                        return Ok(a.hull(b).reached(subject));
                    }
                    BooleanOperator::Intersection => {
                        let a = self.nested(graph, *left, transform, depth);
                        let b = self.nested(graph, *right, transform, depth);
                        let outer = match (a, b) {
                            (Ok(a), Ok(b)) => (a.outer.0.max(b.outer.0), a.outer.1.min(b.outer.1)),
                            (Ok(one), Err(_)) | (Err(_), Ok(one)) => one.outer,
                            (Err(error), Err(_)) => return Err(error),
                        };
                        if outer.0 > outer.1 {
                            return Err("the intersection's operands are apart vertically".into());
                        }
                        return Ok(Height { outer, inner: None });
                    }
                    _ => return Err("this boolean's height is not stated by its operands".into()),
                },
                Some(GeometryNode::SolidOperation(SolidOperation::Extrusion {
                    profile,
                    direction,
                    depth: length,
                })) => {
                    return extrusion(graph, &transform, *profile, *direction, *length)
                        .map(|height| height.reached(subject));
                }
                Some(
                    leaf @ (GeometryNode::PolygonMesh(_)
                    | GeometryNode::TriMesh(_)
                    | GeometryNode::BRep(_)),
                ) => {
                    return faceted(leaf, &transform).map(|height| height.reached(subject));
                }
                Some(
                    GeometryNode::HalfSpace(_)
                    | GeometryNode::SolidOperation(
                        SolidOperation::BoundedHalfSpace { .. }
                        | SolidOperation::BoundedHalfSpaceAlongZ { .. },
                    ),
                ) => return Err("a half-space has no height".into()),
                Some(_) => {
                    return Err(
                        "the solid's height is not stated exactly by its construction".into(),
                    );
                }
                None => return Err("the graph does not hold the node".into()),
            }
        }
        Err("the body's graph is too large to read its height".into())
    }

    /// The height of a member or operand, one level deeper.
    fn nested(
        &mut self,
        graph: &GeometryGraph,
        node: NodeId,
        transform: Transform3,
        depth: usize,
    ) -> Result<Height, String> {
        if depth >= MAX_DEPTH {
            return Err("the body's graph is nested too deep".into());
        }
        self.height(graph, node, transform, depth + 1)
    }
}

/// The elevations of an authored mesh or a B-rep with no curved edge or
/// face: the vertices its faces (a B-rep's edges) use, placed.
fn faceted(node: &GeometryNode, transform: &Transform3) -> Result<Height, String> {
    match node {
        GeometryNode::PolygonMesh(mesh) => {
            let used = mesh
                .faces
                .iter()
                .flat_map(|face| face.outer.iter().chain(face.holes.iter().flatten()))
                .map(|index| *index as usize);
            vertices(&mesh.positions, used, transform)
        }
        GeometryNode::TriMesh(mesh) => {
            let used = mesh.indices.iter().map(|index| *index as usize);
            vertices(&mesh.positions, used, transform)
        }
        GeometryNode::BRep(brep) => {
            if brep.edges().iter().any(|edge| edge.curve.is_some())
                || brep.faces().iter().any(|face| face.surface.is_some())
            {
                return Err("the B-rep has a curved edge or face".into());
            }
            let positions: Vec<Vec3> = brep
                .vertices()
                .iter()
                .map(|vertex| vertex.position)
                .collect();
            let used = brep
                .edges()
                .iter()
                .flat_map(|edge| [edge.start.index(), edge.end.index()]);
            vertices(&positions, used, transform)
        }
        _ => Err("the node is no mesh".into()),
    }
}

/// The elevations of an extrusion placed by `transform`: exact where its
/// profile plane is horizontal, bounded by the profile's box otherwise.
fn extrusion(
    graph: &GeometryGraph,
    transform: &Transform3,
    profile: NodeId,
    direction: Vec3,
    depth: f64,
) -> Result<Height, String> {
    let (rise_u, rise_v) = (
        transform.transform_vector3(Vec3::X).z,
        transform.transform_vector3(Vec3::Y).z,
    );
    let length = direction.length();
    if !(direction.is_finite() && depth.is_finite() && depth > 0.0 && length > 0.0) {
        return Err("the extrusion is degenerate".into());
    }
    let base = transform.translation.z;
    let along = transform.transform_vector3(direction / length * depth).z;
    if rise_u != 0.0 || rise_v != 0.0 {
        return tilted(graph, profile, (rise_u, rise_v), base, along);
    }
    let (low, high) = (base.min(base + along), base.max(base + along));
    if !(low.is_finite() && high.is_finite()) || high <= low {
        return Err("the extrusion sweeps its profile within its plane".into());
    }
    let rounding = ROUNDING_ULPS * f64::EPSILON * low.abs().max(high.abs());
    if high - low <= 2.0 * rounding {
        return Err("the extrusion is too thin to bound its height".into());
    }
    Ok(Height {
        outer: (low - rounding, high + rounding),
        inner: Some((low + rounding, high - rounding)),
    })
}

/// The elevations of an extrusion whose profile plane is tilted: a point
/// `(u, v)` of the profile rises by `u * rise_u + v * rise_v` above the plane's
/// origin at `base`, and the sweep adds `0..=along`. Within the profile's
/// box that is a box of elevations, with no claim that it reaches its ends
/// (a circle touches its box at four points only).
fn tilted(
    graph: &GeometryGraph,
    profile: NodeId,
    (rise_u, rise_v): (f64, f64),
    base: f64,
    along: f64,
) -> Result<Height, String> {
    let Some(GeometryNode::Profile(profile)) = graph.get(profile) else {
        return Err("the extrusion's profile is not a profile".into());
    };
    let ([u0, u1], [v0, v1]) = profile_box(profile)?;
    let (a, b) = (
        (rise_u * u0).min(rise_u * u1),
        (rise_u * u0).max(rise_u * u1),
    );
    let (c, d) = (
        (rise_v * v0).min(rise_v * v1),
        (rise_v * v0).max(rise_v * v1),
    );
    let (low, high) = (base + a + c + along.min(0.0), base + b + d + along.max(0.0));
    let magnitude = base.abs()
        + rise_u.abs() * u0.abs().max(u1.abs())
        + rise_v.abs() * v0.abs().max(v1.abs())
        + along.abs();
    let rounding = ROUNDING_ULPS * f64::EPSILON * magnitude;
    if !(low.is_finite() && high.is_finite() && rounding.is_finite()) {
        return Err("the extrusion's elevations are not finite".into());
    }
    Ok(Height {
        outer: (low - rounding, high + rounding),
        inner: None,
    })
}

/// A box `([u0, u1], [v0, v1])` around a profile in its own plane, or the
/// reason none is read: only profiles whose extent the construction states
/// (a rectangle or ellipse about its origin, a circle, a contour of
/// polylines and arcs, and their placements and collections). A section
/// family, a centre line or a contour with any other segment is refused.
fn profile_box(profile: &Profile) -> Result<([f64; 2], [f64; 2]), String> {
    let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
    let centred = |hu: f64, hv: f64| {
        if finite(&[hu, hv]) && hu > 0.0 && hv > 0.0 {
            Ok(([-hu, hu], [-hv, hv]))
        } else {
            Err("the profile has no positive size".to_owned())
        }
    };
    match profile {
        Profile::Rectangle(rectangle) => centred(rectangle.x / 2.0, rectangle.y / 2.0),
        Profile::Circle(circle) => centred(circle.radius, circle.radius),
        Profile::Ellipse(ellipse) => centred(ellipse.semi_axis_x, ellipse.semi_axis_y),
        Profile::Derived { basis, transform } => {
            let ([u0, u1], [v0, v1]) = profile_box(basis)?;
            corners_box(
                [(u0, v0), (u0, v1), (u1, v0), (u1, v1)]
                    .map(|(u, v)| transform.transform_point2(Vec2::new(u, v))),
            )
        }
        Profile::Composite(members) => {
            let mut hull: Option<([f64; 2], [f64; 2])> = None;
            for member in members {
                let ([u0, u1], [v0, v1]) = profile_box(member)?;
                hull = Some(hull.map_or(([u0, u1], [v0, v1]), |([a, b], [c, d])| {
                    ([a.min(u0), b.max(u1)], [c.min(v0), d.max(v1)])
                }));
            }
            hull.ok_or_else(|| "the profile has no member".to_owned())
        }
        Profile::Contour(contour) => {
            let mut points = Vec::new();
            for segment in &contour.outer.segments {
                match &segment.curve {
                    Curve2::Polyline(line) => points.extend(line.points.iter().copied()),
                    // An arc lies within its whole conic's box, whatever
                    // its domain: the frame's axes need not be unit.
                    Curve2::Circle(circle) => {
                        points.extend(conic_corners(&circle.frame, circle.radius, circle.radius));
                    }
                    Curve2::Ellipse(ellipse) => points.extend(conic_corners(
                        &ellipse.frame,
                        ellipse.semi_axis_x,
                        ellipse.semi_axis_y,
                    )),
                    other => {
                        let curve = format!("{other:?}");
                        let kind = curve.split(['(', ' ', '{']).next().unwrap_or_default();
                        return Err(format!("the profile's contour has a {kind} segment"));
                    }
                }
            }
            if points.is_empty() {
                return Err("the profile's contour has no point".into());
            }
            corners_box(points)
        }
        Profile::Section(_) => Err("a section profile's extent is not read".into()),
        _ => Err("the profile's extent is not stated by its construction".into()),
    }
}

/// Two opposite corners of the box around the conic of `frame` with
/// semi-axes `a` along its x and `b` along its y: a point
/// `origin + a cos(t) x + b sin(t) y` strays at most `hypot(a x.u, b y.u)`
/// from the origin along each world axis `u`.
fn conic_corners(frame: &Frame2, a: f64, b: f64) -> [Vec2; 2] {
    let half = Vec2::new(
        (a * frame.x.x).hypot(b * frame.y.x),
        (a * frame.x.y).hypot(b * frame.y.y),
    );
    [frame.origin - half, frame.origin + half]
}

/// The box around `points`, refusing a non-finite one.
fn corners_box(points: impl IntoIterator<Item = Vec2>) -> Result<([f64; 2], [f64; 2]), String> {
    let (mut u, mut v) = (
        [f64::INFINITY, f64::NEG_INFINITY],
        [f64::INFINITY, f64::NEG_INFINITY],
    );
    for point in points {
        if !point.is_finite() {
            return Err("the profile has a point that is not finite".into());
        }
        u = [u[0].min(point.x), u[1].max(point.x)];
        v = [v[0].min(point.y), v[1].max(point.y)];
    }
    if u[0] > u[1] || v[0] > v[1] {
        return Err("the profile has no point".into());
    }
    Ok((u, v))
}

/// The elevations of the vertices `used` of `positions`, placed by
/// `transform`: the solid they bound (planar faces on straight edges) is
/// their hull, so it lies between them and reaches both. A placed vertex
/// is `row . p + translation`, rounded to 64 units in the last place of the
/// largest term it sums.
fn vertices(
    positions: &[Vec3],
    used: impl Iterator<Item = usize>,
    transform: &Transform3,
) -> Result<Height, String> {
    let row = Vec3::new(
        transform.transform_vector3(Vec3::X).z,
        transform.transform_vector3(Vec3::Y).z,
        transform.transform_vector3(Vec3::Z).z,
    );
    let base = transform.translation.z;
    let (mut low, mut high, mut magnitude) = (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64);
    for index in used {
        let Some(position) = positions.get(index) else {
            return Err("the body names a vertex it does not hold".into());
        };
        let z = row.dot(*position) + base;
        if !(position.is_finite() && z.is_finite()) {
            return Err("the body has a vertex that is not finite".into());
        }
        low = low.min(z);
        high = high.max(z);
        magnitude = magnitude.max(
            base.abs()
                + (row.x * position.x).abs()
                + (row.y * position.y).abs()
                + (row.z * position.z).abs(),
        );
    }
    if low > high {
        return Err("the body has no vertex".into());
    }
    let rounding = ROUNDING_ULPS * f64::EPSILON * magnitude;
    let inner = (low + rounding <= high - rounding).then_some((low + rounding, high - rounding));
    Ok(Height {
        outer: (low - rounding, high + rounding),
        inner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiolid_core::Interval;
    use axiolid_curve::Polyline2;
    use axiolid_mesh::{PolygonFace, PolygonMesh, TriMesh};
    use axiolid_model::{GeometryGraphBuilder, Instance};
    use axiolid_profile::{
        CircleProfile, Contour, ContourProfile, ProfileSegment, RectangleProfile,
    };
    use axiolid_topology::{BRep, Edge, Vertex};

    fn push(builder: &mut GeometryGraphBuilder, node: GeometryNode) -> NodeId {
        builder.push(node).unwrap()
    }

    fn disk(builder: &mut GeometryGraphBuilder, radius: f64) -> NodeId {
        push(
            builder,
            GeometryNode::Profile(Profile::Circle(CircleProfile {
                radius,
                thickness: None,
            })),
        )
    }

    fn extruded(
        builder: &mut GeometryGraphBuilder,
        radius: f64,
        direction: Vec3,
        depth: f64,
        transform: Transform3,
    ) -> NodeId {
        let profile = disk(builder, radius);
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction,
                depth,
            }),
        );
        push(
            builder,
            GeometryNode::Instance(Instance {
                source: solid,
                transform,
            }),
        )
    }

    fn read(
        build: impl FnOnce(&mut GeometryGraphBuilder) -> NodeId,
    ) -> Result<VerticalExtentBounds, String> {
        let mut builder = GeometryGraphBuilder::new();
        let root = build(&mut builder);
        let graph = builder.finish(vec![root]).unwrap();
        certified_vertical_extent(&graph, root)
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn a_vertical_extrusion_of_a_curved_profile_spans_its_caps_exactly() {
        let at = Transform3::from_translation(Vec3::new(10.0, -4.0, 3.5));
        let bounds = read(|b| extruded(b, 4.0, Vec3::Z, 0.2, at)).unwrap();
        assert!(close(bounds.outer, (3.5, 3.7)), "{bounds:?}");
        assert!(close(bounds.inner.unwrap(), (3.5, 3.7)), "{bounds:?}");
        assert!(bounds.outer.0 <= 3.5 && bounds.outer.1 >= 3.5 + 0.2);
        // Turned about the vertical, mirrored and swept obliquely, the caps
        // stay horizontal.
        let turned = Transform3::from_translation(Vec3::new(0.0, 0.0, 3.7))
            * Transform3::from_rotation_z(0.7)
            * Transform3::from_scale(Vec3::new(1.0, 1.0, -1.0));
        let oblique = Vec3::new(1.0, 0.0, 1.0);
        let bounds = read(|b| extruded(b, 0.5, oblique, 2.0_f64.sqrt() * 0.2, turned)).unwrap();
        assert!(close(bounds.outer, (3.5, 3.7)), "{bounds:?}");
    }

    #[test]
    fn a_difference_lies_within_its_subject_and_a_union_spans_its_operands() {
        let up = |z: f64| Transform3::from_translation(Vec3::new(0.0, 0.0, z));
        let bounds = read(|b| {
            let slab = extruded(b, 4.0, Vec3::Z, 0.2, up(3.5));
            let hole = extruded(b, 0.5, Vec3::Z, 1.0, up(3.0));
            push(
                b,
                GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left: slab,
                    right: hole,
                    operator: BooleanOperator::Difference,
                }),
            )
        })
        .unwrap();
        assert!(
            close(bounds.outer, (3.5, 3.7)) && bounds.inner.is_none(),
            "{bounds:?}"
        );
        let bounds = read(|b| {
            let column = extruded(b, 0.2, Vec3::Z, 3.7, up(0.0));
            let capital = extruded(b, 0.4, Vec3::Z, 0.3, up(3.7));
            push(b, GeometryNode::Collection(vec![column, capital]))
        })
        .unwrap();
        assert!(close(bounds.outer, (0.0, 4.0)), "{bounds:?}");
        assert!(close(bounds.inner.unwrap(), (0.0, 4.0)), "{bounds:?}");
    }

    #[test]
    fn a_difference_chain_is_walked_along_its_subjects_with_their_placements() {
        let up = |z: f64| Transform3::from_translation(Vec3::new(0.0, 0.0, z));
        let mut builder = GeometryGraphBuilder::new();
        let wall = extruded(&mut builder, 4.0, Vec3::Z, 3.0, up(0.0));
        let window = extruded(&mut builder, 0.5, Vec3::Z, 1.0, up(1.0));
        let door = extruded(&mut builder, 0.5, Vec3::Z, 2.0, up(0.0));
        let cut = |builder: &mut GeometryGraphBuilder, left, right| {
            push(
                builder,
                GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left,
                    right,
                    operator: BooleanOperator::Difference,
                }),
            )
        };
        let once = cut(&mut builder, wall, window);
        let twice = cut(&mut builder, once, door);
        let placed = push(
            &mut builder,
            GeometryNode::Instance(Instance {
                source: twice,
                transform: up(3.5),
            }),
        );
        let graph = builder.finish(vec![placed]).unwrap();
        let subjects = difference_subjects(&graph, placed);
        assert_eq!(subjects, vec![(once, up(3.5)), (wall, up(3.5))]);
        // The subject's height holds the difference's.
        let bounds = certified_vertical_extent(&graph, placed).unwrap();
        assert!(close(bounds.outer, (3.5, 6.5)) && bounds.inner.is_none());
        // A solid that is no difference has no subject to bound it.
        assert!(difference_subjects(&graph, wall).is_empty());
    }

    #[test]
    fn a_sweep_within_its_plane_or_another_solid_is_refused() {
        assert!(read(|b| extruded(b, 0.1, Vec3::X, 3.0, Transform3::IDENTITY)).is_err());
        let refused = read(|b| {
            let profile = disk(b, 0.5);
            push(
                b,
                GeometryNode::SolidOperation(SolidOperation::Revolution {
                    profile,
                    axis_origin: axiolid_core::Point3::new(1.0, 0.0, 0.0),
                    axis_direction: Vec3::Y,
                    angle: 1.0,
                }),
            )
        });
        assert!(refused.is_err());
    }

    /// `profile` extruded by `depth` along its normal, placed by `at`.
    fn swept(
        builder: &mut GeometryGraphBuilder,
        profile: Profile,
        depth: f64,
        at: Transform3,
    ) -> NodeId {
        let profile = push(builder, GeometryNode::Profile(profile));
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction: Vec3::Z,
                depth,
            }),
        );
        push(
            builder,
            GeometryNode::Instance(Instance {
                source: solid,
                transform: at,
            }),
        )
    }

    fn within(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    /// Stood upright on the x axis: local y becomes world z, local z world -y.
    fn upright(z: f64) -> Transform3 {
        Transform3::from_translation(Vec3::new(0.0, 0.0, z))
            * Transform3::from_rotation_x(std::f64::consts::FRAC_PI_2)
    }

    #[test]
    fn an_upright_profile_is_bounded_by_its_box_without_reaching_it() {
        // A horizontal pipe: its top is curved, so no cap is reached.
        let lying = Transform3::from_translation(Vec3::new(0.0, 0.0, 3.6))
            * Transform3::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let bounds = read(|b| extruded(b, 0.1, Vec3::Z, 3.0, lying)).unwrap();
        assert!(within(bounds.outer, (3.5, 3.7)) && bounds.inner.is_none());
        assert!(bounds.outer.0 <= 3.5 && bounds.outer.1 >= 3.7, "{bounds:?}");
        // A 2 m by 4 m rectangle stood upright about z = 1, 0.3 m thick.
        let rectangle = Profile::Rectangle(RectangleProfile {
            x: 2.0,
            y: 4.0,
            thickness: None,
            outer_radius: None,
            inner_radius: None,
        });
        let bounds = read(|b| swept(b, rectangle.clone(), 0.3, upright(1.0))).unwrap();
        assert!(within(bounds.outer, (-1.0, 3.0)), "{bounds:?}");
        // A derived profile is the box of its placed corners, a collection
        // the hull of its members'.
        let derived = Profile::Derived {
            basis: Box::new(rectangle.clone()),
            transform: axiolid_core::Transform2::from_translation(axiolid_core::Vec2::new(
                0.0, 5.0,
            )),
        };
        let bounds = read(|b| swept(b, derived.clone(), 0.3, upright(0.0))).unwrap();
        assert!(within(bounds.outer, (3.0, 7.0)), "{bounds:?}");
        let both = Profile::Composite(vec![rectangle, derived]);
        let bounds = read(|b| swept(b, both.clone(), 0.3, upright(0.0))).unwrap();
        assert!(within(bounds.outer, (-2.0, 7.0)), "{bounds:?}");
    }

    #[test]
    fn an_upright_contour_is_bounded_by_its_points_and_arcs() {
        let outline = |points: Vec<axiolid_core::Vec2>| {
            Profile::Contour(ContourProfile {
                outer: Contour::new(vec![ProfileSegment {
                    curve: Curve2::Polyline(Polyline2 {
                        points,
                        closed: true,
                    }),
                    domain: Interval {
                        start: 0.0,
                        end: 1.0,
                    },
                    same_sense: true,
                }]),
                holes: Vec::new(),
            })
        };
        let wall = outline(
            [(0.0, 0.0), (6.0, 0.0), (6.0, 3.0), (0.0, 2.0)]
                .map(|(x, y)| axiolid_core::Vec2::new(x, y))
                .to_vec(),
        );
        let bounds = read(|b| swept(b, wall.clone(), 0.2, upright(10.0))).unwrap();
        assert!(within(bounds.outer, (10.0, 13.0)), "{bounds:?}");
        // An arc is bounded by its whole circle, whatever its domain; a
        // line, whose points follow from its domain, is not read.
        let arc = Profile::Contour(ContourProfile {
            outer: Contour::new(vec![ProfileSegment {
                curve: Curve2::Circle(axiolid_curve::Circle2 {
                    frame: axiolid_core::Frame2 {
                        origin: axiolid_core::Vec2::ZERO,
                        x: axiolid_core::Vec2::X,
                        y: axiolid_core::Vec2::Y,
                    },
                    radius: 1.0,
                }),
                domain: Interval {
                    start: 0.0,
                    end: 3.0,
                },
                same_sense: true,
            }]),
            holes: Vec::new(),
        });
        let bounds = read(|b| swept(b, arc.clone(), 0.2, upright(0.0))).unwrap();
        assert!(within(bounds.outer, (-1.0, 1.0)), "{bounds:?}");
        let line = Profile::Contour(ContourProfile {
            outer: Contour::new(vec![ProfileSegment {
                curve: Curve2::Line(axiolid_curve::Line2 {
                    origin: axiolid_core::Vec2::ZERO,
                    direction: axiolid_core::Vec2::X,
                }),
                domain: Interval {
                    start: 0.0,
                    end: 3.0,
                },
                same_sense: true,
            }]),
            holes: Vec::new(),
        });
        let refused = read(|b| swept(b, line.clone(), 0.2, upright(0.0))).unwrap_err();
        assert!(refused.contains("Line"), "{refused}");
        let empty = outline(Vec::new());
        assert!(read(|b| swept(b, empty.clone(), 0.2, upright(0.0))).is_err());
    }

    #[test]
    fn meshes_and_straight_b_reps_span_their_placed_vertices() {
        let corners: Vec<Vec3> = [
            (0.0, 0.0, 0.0),
            (2.0, 0.0, 0.0),
            (0.0, 2.0, 1.0),
            (0.0, 0.0, 3.0),
        ]
        .map(|(x, y, z)| Vec3::new(x, y, z))
        .to_vec();
        let lifted = Transform3::from_translation(Vec3::new(5.0, 5.0, 10.0));
        let placed = |builder: &mut GeometryGraphBuilder, node| {
            let source = push(builder, node);
            push(
                builder,
                GeometryNode::Instance(Instance {
                    source,
                    transform: lifted,
                }),
            )
        };
        let tetrahedron = TriMesh::new(corners.clone(), vec![0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3]);
        let bounds = read(|b| placed(b, GeometryNode::TriMesh(tetrahedron.clone()))).unwrap();
        assert!(within(bounds.outer, (10.0, 13.0)), "{bounds:?}");
        assert!(within(bounds.inner.unwrap(), (10.0, 13.0)), "{bounds:?}");
        // A vertex no face uses is not part of the solid.
        let mut stray = corners.clone();
        stray.push(Vec3::new(0.0, 0.0, 99.0));
        let loose = TriMesh::new(stray, vec![0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3]);
        let bounds = read(|b| placed(b, GeometryNode::TriMesh(loose.clone()))).unwrap();
        assert!(within(bounds.outer, (10.0, 13.0)), "{bounds:?}");
        let faces = PolygonMesh {
            positions: corners.clone(),
            faces: [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]
                .map(|outer| PolygonFace {
                    outer: outer.to_vec(),
                    holes: Vec::new(),
                })
                .to_vec(),
        };
        let bounds = read(|b| placed(b, GeometryNode::PolygonMesh(faces.clone()))).unwrap();
        assert!(within(bounds.outer, (10.0, 13.0)), "{bounds:?}");
        // An index out of range is a malformed mesh.
        let broken = TriMesh::new(corners.clone(), vec![0, 1, 9]);
        assert!(read(|b| placed(b, GeometryNode::TriMesh(broken.clone()))).is_err());
        // A B-rep of straight edges spans the vertices its edges join; one
        // with a curved edge is refused.
        let mut brep: BRep<NodeId> = BRep::default();
        let ids: Vec<_> = corners
            .iter()
            .map(|position| {
                brep.add_vertex(Vertex {
                    position: *position,
                })
            })
            .collect();
        for pair in [(0, 1), (0, 2), (0, 3)] {
            brep.add_edge(Edge {
                start: ids[pair.0],
                end: ids[pair.1],
                curve: None,
            });
        }
        let bounds = read(|b| placed(b, GeometryNode::BRep(brep.clone()))).unwrap();
        assert!(within(bounds.outer, (10.0, 13.0)), "{bounds:?}");
        let curved = read(|b| {
            let curve = push(
                b,
                GeometryNode::Curve3(axiolid_curve::Curve3::Line(axiolid_curve::Line3 {
                    origin: Vec3::ZERO,
                    direction: Vec3::X,
                })),
            );
            let mut curved = brep.clone();
            curved.add_edge(Edge {
                start: ids[1],
                end: ids[2],
                curve: Some(curve),
            });
            placed(b, GeometryNode::BRep(curved))
        });
        assert!(curved.is_err());
    }
}
