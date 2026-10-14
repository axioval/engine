//! Sound outer boxes for a product whose body is unmeasured (#358).
//!
//! A product the mesh compiler or the lowering refuses has no body, but
//! much of what it is authored as still bounds where that body can be.
//! Every `Body` item is bounded on its own, and the body lies within the
//! union of those boxes, kept apart (up to 256, then joined into one) so a
//! search between two items far apart is not refused by the box around
//! both:
//!
//! - an item that lowers and compiles is bounded by its mesh, grown by the
//!   deviation certified for it;
//! - a boolean difference or intersection, and a clip by a half-space,
//!   only removes material from its first operand, so the first operand's
//!   bound holds for it; a union lies within both operands' bounds;
//! - a polygon mesh, a triangle mesh and a B-rep whose faces are planar and
//!   whose edges are straight lie within the box of their authored
//!   vertices, whatever face of them is refused;
//! - a linear extrusion the kernel refuses (one along its profile's plane)
//!   lies within its profile's region swept along the extrusion: the box
//!   of the outer ring the kernel flattens the profile to, grown by the
//!   deviation it certifies for that ring, at both ends of the sweep;
//! - placements (instances, mapped items) carry the box's corners along.
//!
//! An item that lowers is bounded through its graph; one whose lowering is
//! refused (a half-space whose boundary cannot exist as written) through
//! its IFC entities, for booleans and mapped items only. Anything else, and
//! anything unreadable, gives no bound: the product may then be anywhere,
//! and every measurement it could change stays refused.

use axiolid_construct::profile::{ProfileDeviation, profile_deviation, profile_rings};
use axiolid_core::{BooleanOperator, Point3};
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use ifc_geometry::lower::lower_representation_item;
use ifc_geometry::resource::operator::operator_transform;
use ifc_geometry::resource::{MappedItem, RepresentationMap, axis_placement_transform};
use ifc_geometry::{RepresentationPurpose, Transform};
use ifc_model::{EntityId, Model};

use super::{
    CACHED_POSITIONS, Compiler, EVALUATOR, Extent, Fit, Linear, NODE_BUDGET, TOLERANCE, compile,
    mesh_box, planar, session,
};

/// Where an item's material can be: within the union of some boxes, none
/// when it holds no material.
#[derive(Clone, Debug, Default, PartialEq)]
struct Reach(Vec<Extent>);

/// The most boxes a reach keeps apart; more are joined into the one box
/// around them, which still holds them.
const MAX_BOXES: usize = 256;

impl Reach {
    /// Material in either: the union of both reaches' boxes.
    fn join(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        if self.0.len() > MAX_BOXES {
            self = self.hull();
        }
        self
    }

    /// The one box around every box of this reach. A sweep must be read
    /// so: its material passes between the boxes at its ends.
    fn hull(self) -> Self {
        let mut boxes = self.0.into_iter();
        let Some(first) = boxes.next() else {
            return Self::default();
        };
        Self(vec![boxes.fold(first, |(min, max), (low, high)| {
            (
                std::array::from_fn(|axis| min[axis].min(low[axis])),
                std::array::from_fn(|axis| max[axis].max(high[axis])),
            )
        })])
    }

    /// Each box replaced by the box around its corners, each moved by
    /// `place`. An affine placement carries a box into the hull of its
    /// moved corners, so the union of those holds the moved material.
    fn mapped(self, place: impl Fn([f64; 3]) -> [f64; 3]) -> Option<Self> {
        let mut boxes = Vec::with_capacity(self.0.len());
        for (min, max) in self.0 {
            let corners = (0..8).map(|index: usize| {
                place(std::array::from_fn(|axis| {
                    if (index >> axis) & 1 == 1 {
                        max[axis]
                    } else {
                        min[axis]
                    }
                }))
            });
            boxes.extend(points(corners)?.0);
        }
        Some(Self(boxes))
    }
}

/// The one box around `points`, none when there are none; `None` when any
/// coordinate is not finite.
fn points(points: impl IntoIterator<Item = [f64; 3]>) -> Option<Reach> {
    let mut extent: Option<Extent> = None;
    for point in points {
        if point.iter().any(|value| !value.is_finite()) {
            return None;
        }
        extent = Some(match extent {
            None => (point, point),
            Some((min, max)) => (
                std::array::from_fn(|axis| min[axis].min(point[axis])),
                std::array::from_fn(|axis| max[axis].max(point[axis])),
            ),
        });
    }
    Some(Reach(extent.into_iter().collect()))
}

/// The world boxes `product`'s `Body` lies within the union of, read item
/// by item as the module documentation says; `None` when any item cannot
/// be bounded, the product is not placed, or its `Body` holds no material.
pub(super) fn outer_bound(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    product: EntityId,
) -> Option<Vec<Extent>> {
    if linear.refusal(model, product).is_some() {
        return None;
    }
    let representation =
        ifc_geometry::select_product_representation(model, product, RepresentationPurpose::Body)
            .ok()??;
    let frame = ifc_geometry::product_representation_frame_with_evaluator(
        model,
        units,
        product,
        RepresentationPurpose::Body,
        &EVALUATOR,
        CACHED_POSITIONS,
    )
    .ok()??;
    let items = ifc_geometry::Representation::new(representation, model.get(representation)?)
        .items()
        .ok()?;
    let reader = Reader {
        backend,
        model,
        units,
    };
    let mut budget = NODE_BUDGET;
    let mut reach = Reach::default();
    for item in items {
        reach = reach.join(reader.item(item, frame, &mut budget)?);
    }
    (!reach.0.is_empty()).then_some(reach.0)
}

struct Reader<'a> {
    backend: &'a Compiler,
    model: &'a Model,
    units: &'a ifc_geometry::units::UnitScale,
}

impl Reader<'_> {
    /// One representation item placed by `frame`: through its lowered
    /// graph, else through its IFC entities.
    fn item(&self, item: EntityId, frame: Transform, budget: &mut usize) -> Option<Reach> {
        *budget = budget.checked_sub(1)?;
        let mut lowering = session(self.model, self.units);
        if let Ok(root) = lower_representation_item(&mut lowering, item, frame)
            && let Ok(lowered) = lowering.finish(root)
            && let Some(reach) = graph(self.backend, &lowered.graph, lowered.root, budget)
        {
            return Some(reach);
        }
        let entity = self.model.get(item)?;
        let slots = ifc_geometry::Slots::new(item, entity);
        match entity.type_name.to_ascii_uppercase().as_str() {
            "IFCBOOLEANRESULT" | "IFCBOOLEANCLIPPINGRESULT" => {
                let first = slots.opt_ref(1)?;
                match slots
                    .opt_enum(0)?
                    .trim_matches('.')
                    .to_ascii_uppercase()
                    .as_str()
                {
                    "DIFFERENCE" | "INTERSECTION" => self.item(first, frame, budget),
                    "UNION" => {
                        let second = slots.opt_ref(2)?;
                        Some(
                            self.item(first, frame, budget)?
                                .join(self.item(second, frame, budget)?),
                        )
                    }
                    _ => None,
                }
            }
            "IFCMAPPEDITEM" => {
                let mapped = MappedItem::new(item, entity);
                let (source, target) =
                    (mapped.mapping_source().ok()?, mapped.mapping_target().ok()?);
                let map = RepresentationMap::new(source, self.model.get(source)?);
                let (origin, representation) = (
                    map.mapping_origin().ok()?,
                    map.mapped_representation().ok()?,
                );
                let target = operator_transform(self.model, target, self.model.get(target)?)
                    .ok()?
                    .to_metres(self.units);
                let origin = axis_placement_transform(self.model, origin, self.model.get(origin)?)
                    .ok()?
                    .to_metres(self.units);
                // As lowering places it: world, then target, then origin.
                let placed = frame.compose(&target).compose(&origin);
                let items = ifc_geometry::Representation::new(
                    representation,
                    self.model.get(representation)?,
                )
                .items()
                .ok()?;
                let mut reach = Reach::default();
                for inner in items {
                    reach = reach.join(self.item(inner, placed, budget)?);
                }
                Some(reach)
            }
            _ => None,
        }
    }
}

/// Where the solid of node `id` can be, in the graph's coordinates.
fn graph(
    backend: &Compiler,
    graph_: &GeometryGraph,
    id: NodeId,
    budget: &mut usize,
) -> Option<Reach> {
    *budget = budget.checked_sub(1)?;
    let node = graph_.get(id)?;
    match node {
        GeometryNode::Instance(instance) => graph(backend, graph_, instance.source, budget)?
            .mapped(|point| instance.transform.transform_point3(point.into()).to_array()),
        GeometryNode::Collection(children) => {
            let mut reach = Reach::default();
            for child in children {
                reach = reach.join(graph(backend, graph_, *child, budget)?);
            }
            Some(reach)
        }
        _ => {
            if let Some(reach) = compiled(backend, graph_, id) {
                return Some(reach);
            }
            match node {
                GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left,
                    right,
                    operator,
                }) => match operator {
                    BooleanOperator::Difference | BooleanOperator::Intersection => {
                        graph(backend, graph_, *left, budget)
                    }
                    // Both lie within the two operands.
                    BooleanOperator::Union | BooleanOperator::SymmetricDifference => Some(
                        graph(backend, graph_, *left, budget)?
                            .join(graph(backend, graph_, *right, budget)?),
                    ),
                    _ => None,
                },
                // A linear extrusion is its profile's region swept along
                // one segment, so it lies within the region's box swept
                // along it: the box at both ends (the segment taken both as
                // stated and normalised, as IFC reads it). An extrusion in
                // its profile's plane, which the kernel refuses by name, is
                // bounded so too.
                GeometryNode::SolidOperation(SolidOperation::Extrusion {
                    profile,
                    direction,
                    depth,
                }) => {
                    let region = profile_region(graph_, *profile)?;
                    let mut reach = region.clone();
                    for sweep in [*direction * *depth, direction.normalize_or_zero() * *depth] {
                        reach = reach.join(region.clone().mapped(|point| {
                            std::array::from_fn(|axis| point[axis] + sweep[axis])
                        })?);
                    }
                    // The sweep passes between its ends: one box.
                    Some(reach.hull())
                }
                GeometryNode::PolygonMesh(mesh) => {
                    points(mesh.positions.iter().map(Point3::to_array))
                }
                GeometryNode::TriMesh(mesh) => points(mesh.positions.iter().map(Point3::to_array)),
                // Planar faces with straight edges lie within the hull of
                // their vertices.
                GeometryNode::BRep(brep) if planar(graph_, id, &mut NODE_BUDGET.clone()) => points(
                    brep.vertices()
                        .iter()
                        .map(|vertex| vertex.position.to_array()),
                ),
                GeometryNode::BoundingBox(aabb) => {
                    points([aabb.min.to_array(), aabb.max.to_array()])
                }
                _ => None,
            }
        }
    }
}

/// The box, in its plane (`z = 0`), of the region of the profile node
/// `id`: its outer ring as the kernel flattens it, grown by the deviation
/// the kernel certifies for that flattening. `None` for anything but a
/// profile, or a profile without a certified deviation.
fn profile_region(graph: &GeometryGraph, id: NodeId) -> Option<Reach> {
    let Some(GeometryNode::Profile(profile)) = graph.get(id) else {
        return None;
    };
    let chord = TOLERANCE.linear();
    let rings = profile_rings(profile, chord, TOLERANCE).ok()?;
    let ProfileDeviation::Bounded(deviation) = profile_deviation(profile, chord, TOLERANCE).ok()?
    else {
        return None;
    };
    if !(deviation.is_finite() && deviation >= 0.0) {
        return None;
    }
    let &[(min, max)] = points(rings.outer.iter().map(|point| [point.x, point.y, 0.0]))?
        .0
        .as_slice()
    else {
        return None;
    };
    points([
        [min[0] - deviation, min[1] - deviation, 0.0],
        [max[0] + deviation, max[1] + deviation, 0.0],
    ])
}

/// Node `id` compiled into a mesh, its box grown by its certified
/// deviation; `None` when the compiler refuses it or it has no triangles.
fn compiled(backend: &Compiler, graph: &GeometryGraph, id: NodeId) -> Option<Reach> {
    let (mesh, fit) = compile(backend, graph, id).ok()?;
    let (min, max) = mesh_box(&mesh)?;
    let grow = match fit {
        Fit::Exact => 0.0,
        Fit::Within(deviation) => deviation,
    };
    points([min.map(|value| value - grow), max.map(|value| value + grow)])
}
