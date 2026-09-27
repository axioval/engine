//! Relationships derived from geometry: element to space, opening to space,
//! space to group space.
//!
//! ADR 0004: this module measures which spaces an object lies in, borders or
//! falls within; whether a count or a comparison over them passes is a
//! capability's decision. Which objects are spaces and which are openings is
//! semantic, so the host declares them as plain `ObjectId`s; this crate never
//! reads a source schema.
//!
//! Every derivation compares exact planar bodies. A tessellated subject, or a
//! tessellated space close enough to change the answer, refuses; so does an
//! unmeasured or bodiless space, since it could be the answer, and a point
//! lying on a space's boundary, where inside and outside are undecided.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axiolid_core::{Point2, Point3, Ray3, Tolerance, Vec2, Vec3};
use axiolid_measure::{WindingMesh, closest_point_on_triangle};
use axiolid_mesh::{TriMesh, TriangleMeshView, audit_mesh};
use axiolid_ray_mesh::intersect_triangle;
use axioval_engine::{
    CompleteRelationshipSelection, Derivation, DerivedRelationshipService, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, TraversalDirection,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Extent, Triangle, extent_gap, mesh_extent, triangles};
use crate::planar::{footprint_measure, plan_overlap_area};

/// Distance below which a point is taken to lie on a surface, and the
/// tolerance handed to the kernel's predicates.
const ON_SURFACE: f64 = 1e-9;

/// How far past an opening's face each probe starts, so a face flush with a
/// space boundary does not start the probe on that boundary.
const PROBE_OFFSET: f64 = 1e-6;

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

/// One derived edge from a subject to a space, and how it was found.
#[derive(Clone, Debug)]
struct Edge {
    target: ObjectId,
    note: String,
}

/// Everything derived for one subject: its edges and the notes that make
/// the absence of an edge reviewable (an opening's outside side).
#[derive(Debug, Default)]
struct Derived {
    edges: Vec<Edge>,
    notes: Vec<String>,
}

type Cached = Result<Arc<Derived>, String>;

/// A shape the host supplied for an opening that has no material body.
#[derive(Clone, Debug)]
enum Void {
    Mesh(TriMesh, Option<f64>),
    Unmeasured(String),
}

/// A subject's shape: its mesh and whether it is a tessellation.
struct Shape<'a> {
    mesh: &'a TriMesh,
    tessellated: bool,
}

/// A declared space ready to be probed.
struct SpaceBody<'a> {
    id: &'a ObjectId,
    mesh: &'a TriMesh,
    triangles: Vec<Triangle>,
    extent: Extent,
    tessellated: bool,
}

/// Derives space relationships from supplied geometry and declarations.
///
/// The host declares which objects are spaces ([`Self::with_space`]) and
/// which are doors, windows or openings ([`Self::with_opening`]). An opening
/// is probed through its own mesh; one the host declares bodiless, such as a
/// void cut into a wall, is probed through the shape given with
/// [`Self::with_opening_void`].
pub struct AxiolidDerivedRelationshipService {
    geometry: AxiolidGeometry,
    spaces: BTreeSet<ObjectId>,
    openings: BTreeSet<ObjectId>,
    voids: BTreeMap<ObjectId, Void>,
    cache: Mutex<BTreeMap<(String, ObjectId), Cached>>,
}

impl AxiolidDerivedRelationshipService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self {
            geometry,
            spaces: BTreeSet::new(),
            openings: BTreeSet::new(),
            voids: BTreeMap::new(),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Declares a space: a target of every derivation and a subject of the
    /// group-space derivation.
    #[must_use]
    pub fn with_space(mut self, space: ObjectId) -> Self {
        self.spaces.insert(space);
        self
    }

    /// Declares a door, window or opening, the subjects of `adjacent-space`.
    #[must_use]
    pub fn with_opening(mut self, opening: ObjectId) -> Self {
        self.openings.insert(opening);
        self
    }

    /// Declares an opening and the exact planar shape of its void, for an
    /// opening the geometry declares bodiless.
    #[must_use]
    pub fn with_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.openings.insert(opening.clone());
        self.voids.insert(opening, Void::Mesh(mesh, None));
        self
    }

    /// Declares an opening whose void is a tessellation of curved faces. It
    /// is never probed: its faces are approximate.
    #[must_use]
    pub fn with_tessellated_opening_void(
        mut self,
        opening: ObjectId,
        mesh: TriMesh,
        chord_deviation_metres: f64,
    ) -> Self {
        self.openings.insert(opening.clone());
        self.voids
            .insert(opening, Void::Mesh(mesh, Some(chord_deviation_metres)));
        self
    }

    /// Declares an opening whose void the host could not mesh.
    #[must_use]
    pub fn with_unmeasured_opening_void(
        mut self,
        opening: ObjectId,
        reason: impl Into<String>,
    ) -> Self {
        self.openings.insert(opening.clone());
        self.voids.insert(opening, Void::Unmeasured(reason.into()));
        self
    }

    /// Whether `object` can have edges under `derivation`.
    fn is_subject(&self, derivation: &Derivation, object: &ObjectId) -> bool {
        match derivation {
            Derivation::ContainedInSpace { .. } => !self.spaces.contains(object),
            Derivation::AdjacentSpace { .. } => self.openings.contains(object),
            Derivation::OverlappingGroupSpace { .. } => self.spaces.contains(object),
        }
    }

    /// A subject's shape: `None` when the host declared it bodiless and gave
    /// no void, so it occupies no place.
    fn shape(&self, object: &ObjectId) -> Result<Option<Shape<'_>>, String> {
        match self.voids.get(object) {
            Some(Void::Mesh(mesh, deviation)) => {
                return Ok(Some(Shape {
                    mesh,
                    tessellated: deviation.is_some(),
                }));
            }
            Some(Void::Unmeasured(reason)) => {
                return Err(format!("the void of {object} was not measured: {reason}"));
            }
            None => {}
        }
        if let Some(mesh) = self.geometry.mesh(object) {
            return Ok(Some(Shape {
                mesh,
                tessellated: self.geometry.is_tessellated(object),
            }));
        }
        if let Some((_, reason)) = self.geometry.unmeasured().find(|(id, _)| *id == object) {
            return Err(format!(
                "{object} has a body that was not measured: {reason}"
            ));
        }
        if self.geometry.has_no_body(object) {
            return Ok(None);
        }
        Err(format!("{object} has no described geometry"))
    }

    /// Every declared space with a closed exact-or-tessellated body.
    fn space_bodies(&self) -> Result<Vec<SpaceBody<'_>>, String> {
        self.spaces
            .iter()
            .map(|space| {
                let Some(mesh) = self.geometry.mesh(space) else {
                    return Err(if self.geometry.is_unmeasured(space) {
                        format!("space {space} has a body that was not measured")
                    } else {
                        format!("space {space} has no body, so what lies in it is undecided")
                    });
                };
                let health = audit_mesh(mesh, tolerance()?);
                if !health.is_surface_usable()
                    || health.degenerate_triangles != 0
                    || !health.is_closed_two_manifold()
                {
                    return Err(format!(
                        "space {space} is not a closed solid, so it has no inside"
                    ));
                }
                let extent =
                    mesh_extent(mesh).ok_or_else(|| format!("space {space} has an empty mesh"))?;
                Ok(SpaceBody {
                    id: space,
                    mesh,
                    triangles: triangles(mesh),
                    extent,
                    tessellated: self.geometry.is_tessellated(space),
                })
            })
            .collect()
    }

    /// The edges of one subject, derived once per derivation and cached.
    fn derived<'s>(
        &'s self,
        derivation: &Derivation,
        subject: &ObjectId,
        spaces: &mut Option<Result<Vec<SpaceBody<'s>>, String>>,
    ) -> Cached {
        let key = (derivation.to_string(), subject.clone());
        if let Some(cached) = self.cache.lock().map_err(|_| poisoned())?.get(&key) {
            return cached.clone();
        }
        let result = if self.is_subject(derivation, subject) {
            let spaces = spaces
                .get_or_insert_with(|| self.space_bodies())
                .as_ref()
                .map_err(Clone::clone)?;
            match derivation {
                Derivation::ContainedInSpace {
                    horizontal_metres,
                    vertical_metres,
                } => self.contained(subject, spaces, *horizontal_metres, *vertical_metres),
                Derivation::AdjacentSpace { reach_metres } => {
                    self.adjacent(subject, spaces, *reach_metres)
                }
                Derivation::OverlappingGroupSpace {
                    minimum_ratio,
                    vertical_metres,
                } => Self::group(subject, spaces, *minimum_ratio, *vertical_metres),
            }
            .map(Arc::new)
        } else {
            Ok(Arc::new(Derived::default()))
        };
        self.cache
            .lock()
            .map_err(|_| poisoned())?
            .insert(key, result.clone());
        result
    }

    /// The spaces containing a subject's reference point, the centre of its
    /// extent, or else the nearest one within the tolerances.
    fn contained(
        &self,
        subject: &ObjectId,
        spaces: &[SpaceBody<'_>],
        horizontal: f64,
        vertical: f64,
    ) -> Result<Derived, String> {
        let Some(shape) = self.shape(subject)? else {
            // Declared bodiless: it occupies no place, so it is in no space.
            return Ok(Derived::default());
        };
        if shape.tessellated {
            return Err(format!(
                "{subject} is a tessellation, so its reference point is approximate"
            ));
        }
        let (min, max) =
            mesh_extent(shape.mesh).ok_or_else(|| format!("{subject} has an empty mesh"))?;
        let point = (Point3::from_array(min) + Point3::from_array(max)) * 0.5;
        let reach = horizontal.hypot(vertical);
        let probe: Extent = (point.to_array(), point.to_array());
        let mut containing = Vec::new();
        let mut near: Vec<(f64, f64, f64, &ObjectId)> = Vec::new();
        for space in spaces {
            let gap = extent_gap(&probe, &space.extent, false);
            if space.tessellated && gap <= reach + ON_SURFACE {
                return Err(format!(
                    "space {} is a tessellation near {subject}, so whether it holds it is \
                     approximate",
                    space.id
                ));
            }
            if gap > reach {
                continue;
            }
            let nearest = nearest_point(point, &space.triangles)?;
            let distance = (nearest - point).length();
            if distance <= ON_SURFACE {
                return Err(format!(
                    "the reference point of {subject} lies on the boundary of space {}",
                    space.id
                ));
            }
            if inside(space.mesh, point)? {
                containing.push(space.id);
            } else {
                let offset = nearest - point;
                let plan = offset.x.hypot(offset.y);
                if plan <= horizontal + ON_SURFACE && offset.z.abs() <= vertical + ON_SURFACE {
                    near.push((distance, plan, offset.z.abs(), space.id));
                }
            }
        }
        let point_text = format!("({:.6},{:.6},{:.6})", point.x, point.y, point.z);
        if !containing.is_empty() {
            return Ok(Derived {
                edges: containing
                    .into_iter()
                    .map(|space| Edge {
                        target: space.clone(),
                        note: format!("contains-point{point_text}"),
                    })
                    .collect(),
                notes: Vec::new(),
            });
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.3.cmp(b.3)));
        match near.as_slice() {
            [] => Ok(Derived {
                edges: Vec::new(),
                notes: vec![format!("{subject}:in-no-space{point_text}")],
            }),
            [first, second, ..] if second.0 - first.0 <= ON_SURFACE => Err(format!(
                "spaces {} and {} are equally near {subject}",
                first.3, second.3
            )),
            [(distance, plan, rise, space), ..] => Ok(Derived {
                edges: vec![Edge {
                    target: (*space).clone(),
                    note: format!(
                        "nearest{point_text}:distance={distance:.6}:horizontal={plan:.6}:\
                         vertical={rise:.6}"
                    ),
                }],
                notes: Vec::new(),
            }),
        }
    }

    /// The spaces a probe first enters on each side of an opening.
    fn adjacent(
        &self,
        subject: &ObjectId,
        spaces: &[SpaceBody<'_>],
        reach: f64,
    ) -> Result<Derived, String> {
        let shape = self
            .shape(subject)?
            .ok_or_else(|| format!("opening {subject} has no shape to probe from"))?;
        if shape.tessellated {
            return Err(format!(
                "opening {subject} is a tessellation, so its faces are approximate"
            ));
        }
        let (min, max) =
            mesh_extent(shape.mesh).ok_or_else(|| format!("{subject} has an empty mesh"))?;
        let plan: Vec<Point2> = (0..shape.mesh.position_count())
            .map(|index| {
                let position = shape.mesh.position(index);
                Point2::new(position.x, position.y)
            })
            .collect();
        let axis = thin_axis(&plan).ok_or_else(|| {
            format!("opening {subject} has no single direction through its thickness")
        })?;
        let elevation = f64::midpoint(min[2], max[2]);
        let normal = Vec3::new(axis.normal.x, axis.normal.y, 0.0);
        let centre = Point3::new(axis.centre.x, axis.centre.y, elevation);
        let starts = [1.0, -1.0].map(|sign| {
            (
                sign,
                centre + normal * (sign * (axis.thickness * 0.5 + PROBE_OFFSET)),
            )
        });
        let normal_text = format!("({:.6},{:.6})", axis.normal.x, axis.normal.y);

        // Which spaces could each probe reach at all.
        let mut sides: Vec<(f64, Vec<(f64, &ObjectId)>)> = Vec::new();
        let mut inside_at: BTreeMap<&ObjectId, [bool; 2]> = BTreeMap::new();
        let mut entries: Vec<Vec<(f64, &ObjectId)>> = vec![Vec::new(), Vec::new()];
        for (index, (sign, start)) in starts.iter().enumerate() {
            let direction = normal * *sign;
            let end = *start + direction * reach;
            let segment: Extent = (start.min(end).to_array(), start.max(end).to_array());
            for space in spaces {
                let gap = extent_gap(&segment, &space.extent, false);
                if space.tessellated && gap <= ON_SURFACE {
                    return Err(format!(
                        "space {} is a tessellation near opening {subject}, so where it begins \
                         is approximate",
                        space.id
                    ));
                }
                if gap > 0.0 {
                    continue;
                }
                let nearest = nearest_point(*start, &space.triangles)?;
                if (nearest - *start).length() <= ON_SURFACE {
                    return Err(format!(
                        "a probe from opening {subject} starts on the boundary of space {}",
                        space.id
                    ));
                }
                if inside(space.mesh, *start)? {
                    inside_at.entry(space.id).or_default()[index] = true;
                    entries[index].push((0.0, space.id));
                } else if let Some(t) = first_hit(*start, direction, reach, &space.triangles)? {
                    entries[index].push((t, space.id));
                }
            }
        }
        // A space holding both faces encloses the opening; it is on no side.
        let enclosing: BTreeSet<&ObjectId> = inside_at
            .into_iter()
            .filter(|(_, at)| at[0] && at[1])
            .map(|(space, _)| space)
            .collect();
        for (index, (sign, _)) in starts.iter().enumerate() {
            let mut found: Vec<(f64, &ObjectId)> = entries[index]
                .iter()
                .filter(|(_, space)| !enclosing.contains(space))
                .copied()
                .collect();
            found.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
            let first = found.first().map(|(t, _)| *t);
            found.retain(|(t, _)| first.is_some_and(|first| *t - first <= ON_SURFACE));
            sides.push((*sign, found));
        }
        let mut derived = Derived::default();
        for (sign, found) in sides {
            let side = if sign > 0.0 { "+" } else { "-" };
            if found.is_empty() {
                derived.notes.push(format!(
                    "{subject}:side={side}{normal_text}:outside:reach={reach}"
                ));
            }
            for (t, space) in found {
                derived.edges.push(Edge {
                    target: space.clone(),
                    note: format!("side={side}{normal_text}:entered={t:.6}"),
                });
            }
        }
        Ok(derived)
    }

    /// The larger spaces covering at least `ratio` of a space's footprint.
    fn group(
        subject: &ObjectId,
        spaces: &[SpaceBody<'_>],
        ratio: f64,
        vertical: f64,
    ) -> Result<Derived, String> {
        let Some(own) = spaces.iter().find(|space| space.id == subject) else {
            return Err(format!("space {subject} has no body"));
        };
        if own.tessellated {
            return Err(format!(
                "space {subject} is a tessellation, so its footprint is approximate"
            ));
        }
        let tolerance = tolerance()?;
        let (area, _) = footprint_measure(&own.triangles, tolerance)
            .ok_or_else(|| format!("the footprint of {subject} cannot be computed"))?;
        if area <= ON_SURFACE {
            return Err(format!("space {subject} has no footprint"));
        }
        let mut derived = Derived::default();
        for other in spaces.iter().filter(|space| space.id != subject) {
            let plan_gap = extent_gap(&own.extent, &other.extent, true);
            if other.tessellated && plan_gap <= ON_SURFACE {
                return Err(format!(
                    "space {} is a tessellation overlapping {subject}, so their overlap is \
                     approximate",
                    other.id
                ));
            }
            let rise = (other.extent.0[2] - own.extent.1[2])
                .max(own.extent.0[2] - other.extent.1[2])
                .max(0.0);
            if plan_gap > 0.0 || rise > vertical {
                continue;
            }
            let (other_area, _) = footprint_measure(&other.triangles, tolerance)
                .ok_or_else(|| format!("the footprint of {} cannot be computed", other.id))?;
            let overlap = plan_overlap_area(&own.triangles, &other.triangles, tolerance)
                .ok_or_else(|| {
                    format!(
                        "the overlap of {subject} and {} cannot be computed",
                        other.id
                    )
                })?;
            let covered = overlap.min(area) / area;
            if other_area > area && covered >= ratio {
                derived.edges.push(Edge {
                    target: other.id.clone(),
                    note: format!("covers={covered:.6}:vertical-gap={rise:.6}"),
                });
            }
        }
        Ok(derived)
    }
}

/// One request's traversal: the subjects' edges and the evidence cited.
struct Walk<'s> {
    service: &'s AxiolidDerivedRelationshipService,
    derivation: Derivation,
    identity: String,
    spaces: Option<Result<Vec<SpaceBody<'s>>, String>>,
    cited: BTreeSet<String>,
}

impl Walk<'_> {
    fn edges(&mut self, subject: &ObjectId) -> Cached {
        self.service
            .derived(&self.derivation, subject, &mut self.spaces)
    }

    /// Cites a subject's edges (only those to `target` when given) and its
    /// notes.
    fn cite(&mut self, subject: &ObjectId, derived: &Derived, target: Option<&ObjectId>) {
        let identity = &self.identity;
        for edge in &derived.edges {
            if target.is_none_or(|target| *target == edge.target) {
                self.cited.insert(format!(
                    "{identity}:{subject}->{}:{}",
                    edge.target, edge.note
                ));
            }
        }
        for note in &derived.notes {
            self.cited.insert(format!("{identity}:{note}"));
        }
    }

    /// The objects one step from `current`.
    fn step(
        &mut self,
        current: &ObjectId,
        direction: TraversalDirection,
        domain: &BTreeSet<&ObjectId>,
    ) -> Result<Vec<ObjectId>, String> {
        let mut found = Vec::new();
        if matches!(
            direction,
            TraversalDirection::Forward | TraversalDirection::Either
        ) {
            let derived = self.edges(current)?;
            self.cite(current, &derived, None);
            found.extend(derived.edges.iter().map(|edge| edge.target.clone()));
        }
        // Every edge ends at a space, so only a space has subjects.
        if matches!(
            direction,
            TraversalDirection::Backward | TraversalDirection::Either
        ) && self.service.spaces.contains(current)
        {
            for subject in domain {
                let derived = self.edges(subject)?;
                if derived.edges.iter().any(|edge| edge.target == *current) {
                    self.cite(subject, &derived, Some(current));
                    found.push((*subject).clone());
                }
            }
        }
        Ok(found)
    }

    /// Everything the query reaches from `anchor`, the anchor included.
    fn reach(
        &mut self,
        anchor: &ObjectId,
        universe: &[ObjectId],
        query: &RelationshipQuery,
    ) -> Result<BTreeSet<ObjectId>, String> {
        let mut reached = BTreeSet::new();
        match query {
            RelationshipQuery::Related {
                direction,
                follow_chain,
                ..
            } => {
                // Subjects a backward step may reach: the universe, and for
                // the group derivation every space, since a chain climbs
                // through them.
                let mut domain: BTreeSet<&ObjectId> = universe.iter().collect();
                if matches!(self.derivation, Derivation::OverlappingGroupSpace { .. }) {
                    domain.extend(self.service.spaces.iter());
                }
                let mut seen = BTreeSet::from([anchor.clone()]);
                let mut frontier = vec![anchor.clone()];
                while !frontier.is_empty() {
                    let mut next = Vec::new();
                    for current in &frontier {
                        for object in self.step(current, *direction, &domain)? {
                            reached.insert(object.clone());
                            if seen.insert(object.clone()) {
                                next.push(object);
                            }
                        }
                    }
                    if !follow_chain {
                        break;
                    }
                    frontier = next;
                }
            }
            RelationshipQuery::SharedGroup { .. } => {
                let own = self.edges(anchor)?;
                self.cite(anchor, &own, None);
                let groups: BTreeSet<&ObjectId> =
                    own.edges.iter().map(|edge| &edge.target).collect();
                for member in universe {
                    let derived = self.edges(member)?;
                    if derived
                        .edges
                        .iter()
                        .any(|edge| groups.contains(&edge.target))
                    {
                        self.cite(member, &derived, None);
                        reached.insert(member.clone());
                    }
                }
            }
        }
        Ok(reached)
    }
}

impl DerivedRelationshipService for AxiolidDerivedRelationshipService {
    fn derive(
        &self,
        derivation: &Derivation,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let identity = derivation.to_string();
        let anchor = request.anchor();
        let universe = request.candidate_universe();
        let mut walk = Walk {
            service: self,
            derivation: *derivation,
            identity: identity.clone(),
            spaces: None,
            cited: BTreeSet::new(),
        };
        let mut reached = walk
            .reach(anchor, universe, request.query())
            .map_err(|reason| {
                RelationshipSelectionError::Unavailable(format!("{identity}: {reason}"))
            })?;
        reached.remove(anchor);
        let candidates = universe
            .iter()
            .filter(|candidate| reached.contains(*candidate))
            .cloned()
            .collect();
        // The scan locator makes an empty answer reviewable: every subject
        // was measured against every declared space.
        // Cited under the anchor's source: in a set over several sources the
        // derivation is about the anchor.
        let cite = |locator| Evidence::exact(anchor.source.clone(), locator);
        let mut evidence = vec![cite(format!(
            "{identity}:derived-from:{anchor}:{} space(s)",
            self.spaces.len()
        ))];
        evidence.extend(walk.cited.into_iter().map(cite));
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

fn poisoned() -> String {
    "the derivation cache is poisoned".into()
}

fn tolerance() -> Result<Tolerance, String> {
    Tolerance::new(ON_SURFACE, ON_SURFACE).map_err(|_| "invalid tolerance".to_owned())
}

/// Whether `point` lies inside the closed body `mesh`.
fn inside(mesh: &TriMesh, point: Point3) -> Result<bool, String> {
    let winding = WindingMesh::prepare(mesh, tolerance()?)
        .map_err(|error| format!("winding number unavailable: {error:?}"))?;
    let number = winding
        .winding_number(point)
        .map_err(|error| format!("winding number unavailable: {error:?}"))?;
    Ok(number.value.abs() >= INSIDE_WINDING)
}

/// The point of `triangles` nearest `point`.
fn nearest_point(point: Point3, triangles: &[Triangle]) -> Result<Point3, String> {
    let mut best: Option<(f64, Point3)> = None;
    for triangle in triangles {
        let candidate = closest_point_on_triangle(point, *triangle)
            .map_err(|error| format!("closest point unavailable: {error:?}"))?;
        let distance = (candidate - point).length_squared();
        if best.is_none_or(|(least, _)| distance < least) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, point)| point)
        .ok_or_else(|| "empty mesh".to_owned())
}

/// Distance along the unit `direction` at which a ray from `start` first
/// meets `triangles`, when that is within `reach`.
fn first_hit(
    start: Point3,
    direction: Vec3,
    reach: f64,
    triangles: &[Triangle],
) -> Result<Option<f64>, String> {
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let tolerance = tolerance()?;
    let mut first: Option<f64> = None;
    for (index, triangle) in triangles.iter().enumerate() {
        let hit = intersect_triangle(&ray, *triangle, tolerance, index)
            .map_err(|error| format!("ray intersection unavailable: {error:?}"))?;
        if let Some(hit) = hit.filter(|hit| (0.0..=reach).contains(&hit.t)) {
            first = Some(first.map_or(hit.t, |least: f64| least.min(hit.t)));
        }
    }
    Ok(first)
}

/// The direction through an opening's thickness, from its plan points.
struct Axis {
    /// Unit plan normal, canonically signed: `x` positive, or `x` zero and
    /// `y` positive.
    normal: Vec2,
    thickness: f64,
    centre: Point2,
}

/// The narrowest direction across the convex hull of `points`, found by
/// rotating calipers over the hull's edges; `None` for a degenerate hull or
/// when two directions are equally narrow.
fn thin_axis(points: &[Point2]) -> Option<Axis> {
    let hull = convex_hull(points);
    if hull.len() < 3 {
        return None;
    }
    let mut widths: Vec<(f64, Vec2)> = Vec::new();
    for index in 0..hull.len() {
        let edge = hull[(index + 1) % hull.len()] - hull[index];
        let length = edge.length();
        if length <= ON_SURFACE {
            continue;
        }
        let mut normal = Vec2::new(-edge.y, edge.x) / length;
        if normal.x < -ON_SURFACE || (normal.x.abs() <= ON_SURFACE && normal.y < 0.0) {
            normal = -normal;
        }
        // Adding zero turns a negative zero positive, so the normal prints
        // the same for every opening facing that way.
        let normal = Vec2::new(normal.x + 0.0, normal.y + 0.0);
        let (low, high) = span(&hull, normal);
        widths.push((high - low, normal));
    }
    widths.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (thickness, normal) = *widths.first()?;
    if thickness <= ON_SURFACE {
        return None;
    }
    // Opposite hull edges share an axis; another axis just as narrow leaves
    // the direction through the opening undecided.
    if widths.iter().skip(1).any(|(width, other)| {
        *width - thickness <= ON_SURFACE && normal.perp_dot(*other).abs() > ON_SURFACE
    }) {
        return None;
    }
    let along = normal.perp();
    let (low, high) = span(&hull, normal);
    let (first, last) = span(&hull, along);
    let centre = normal * f64::midpoint(low, high) + along * f64::midpoint(first, last);
    Some(Axis {
        normal,
        thickness,
        centre,
    })
}

fn span(points: &[Point2], direction: Vec2) -> (f64, f64) {
    points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
            let along = point.dot(direction);
            (low.min(along), high.max(along))
        })
}

/// Andrew's monotone chain, counter-clockwise, without collinear points.
fn convex_hull(points: &[Point2]) -> Vec<Point2> {
    let mut sorted: Vec<Point2> = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let cross = |o: Point2, a: Point2, b: Point2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Point2> = Vec::with_capacity(sorted.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Point2>> = if pass == 0 {
            Box::new(sorted.iter())
        } else {
            Box::new(sorted.iter().rev())
        };
        for point in iter {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
            {
                hull.pop();
            }
            hull.push(*point);
        }
        hull.pop();
    }
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thin_axis_crosses_a_rotated_slab_and_refuses_a_square() {
        let angle = 0.3_f64;
        let (along, across) = (
            Vec2::new(angle.cos(), angle.sin()),
            Vec2::new(-angle.sin(), angle.cos()),
        );
        let corners: Vec<Point2> = [(0.0, 0.0), (0.9, 0.0), (0.9, 0.2), (0.0, 0.2)]
            .into_iter()
            .map(|(u, v)| Point2::new(5.0, 1.0) + along * u + across * v)
            .collect();
        let axis = thin_axis(&corners).unwrap();
        assert!((axis.thickness - 0.2).abs() < 1e-9);
        assert!(axis.normal.perp_dot(across).abs() < 1e-9);
        let centre = Point2::new(5.0, 1.0) + along * 0.45 + across * 0.1;
        assert!((axis.centre - centre).length() < 1e-9);

        let square = [
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 1.0),
            Point2::new(0.0, 1.0),
        ];
        assert!(thin_axis(&square).is_none());
    }
}
