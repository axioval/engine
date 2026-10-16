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
//! - a difference, and a clip or other intersection: within its subject's
//!   (an intersection within either operand's, the tighter where both are
//!   read), with no claim that it reaches them, since a boolean may remove
//!   a cap;
//! - a union or a collection: the hull of its members', each reached where
//!   every member's is.
//!
//! Every other node (a revolution, a swept disk or profile, a tilted
//! profile plane, a half-space alone, an authored mesh or B-rep) is refused
//! with the reason, and the host keeps the deviation-widened extent. The
//! elevations are computed from the composed `f64` placement, as the
//! compiler places the mesh, and widened outward (narrowed inward) by 64
//! units in the last place of their magnitude, so a compiler composing the
//! same placements in another order stays inside them.

use axiolid_core::{BooleanOperator, Transform3, Vec3};
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};

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
                    profile: _,
                    direction,
                    depth: length,
                })) => {
                    return extrusion(&transform, *direction, *length)
                        .map(|height| height.reached(subject));
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

/// The elevations of an extrusion placed by `transform`: exact where its
/// profile plane is horizontal.
fn extrusion(transform: &Transform3, direction: Vec3, depth: f64) -> Result<Height, String> {
    let (x, y) = (
        transform.transform_vector3(Vec3::X),
        transform.transform_vector3(Vec3::Y),
    );
    if x.z != 0.0 || y.z != 0.0 {
        return Err("the extrusion's profile plane is not horizontal".into());
    }
    let length = direction.length();
    if !(direction.is_finite() && depth.is_finite() && depth > 0.0 && length > 0.0) {
        return Err("the extrusion is degenerate".into());
    }
    let base = transform.translation.z;
    let along = transform.transform_vector3(direction / length * depth).z;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axiolid_model::{GeometryGraphBuilder, Instance};
    use axiolid_profile::{CircleProfile, Profile};

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
    fn a_tilted_profile_plane_or_another_solid_is_refused() {
        // A horizontal pipe: its top is curved, so only the mesh bounds it.
        let lying = Transform3::from_translation(Vec3::new(0.0, 0.0, 3.6))
            * Transform3::from_rotation_y(std::f64::consts::FRAC_PI_2);
        assert!(read(|b| extruded(b, 0.1, Vec3::Z, 3.0, lying)).is_err());
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
}
