//! Geometry an application supplies for its model objects.
//!
//! Shared by every service in this crate: the store maps a source-qualified
//! object to its mesh and knows nothing about what will be measured from it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axiolid_brep::ExactBRep;
use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_engine::{GeometryFidelity, ProximityError};
use axioval_ir::ObjectId;

use crate::exact_boundary::ExactBody;

/// Geometry for one object, keyed by the identity the engine uses.
///
/// Holding meshes by `ObjectId` is what keeps this adapter source-neutral:
/// the host decides how its native elements map onto identities.
///
/// Besides a mesh, the host can state two facts about an object. It has **no
/// body** ([`Self::with_no_body`]): a storey or a zone occupies no volume, so
/// it obstructs nothing. Or its body is **unmeasured**
/// ([`Self::with_unmeasured`]): it exists but could not be meshed. The two
/// must not be confused. Treating an unmeasured slab as bodiless would let a
/// wall above it look unsupported, or a room look free where it is not, and
/// still report the result as exact. Services therefore refuse whenever an
/// unmeasured object could have changed their answer.
#[derive(Clone, Debug, Default)]
pub struct AxiolidGeometry {
    meshes: BTreeMap<ObjectId, TriMesh>,
    chord_deviations: BTreeMap<ObjectId, f64>,
    bodiless: BTreeSet<ObjectId>,
    unmeasured: BTreeMap<ObjectId, String>,
    unmeasured_bounds: BTreeMap<ObjectId, Extent>,
    /// Host-certified bounds on a tessellated body's true extent
    /// ([`Self::with_extent_bounds`]): `(outer, inner)`.
    extent_bounds: BTreeMap<ObjectId, (Extent, Option<Extent>)>,
    groups: BTreeMap<ObjectId, Result<Vec<ObjectId>, String>>,
    boundaries: BTreeMap<ObjectId, Arc<ExactBody>>,
    /// Wholes measured through their parts ([`Self::with_composed_body`]).
    compositions: BTreeMap<ObjectId, Arc<Composition>>,
    /// Hosts whose registered body already carries some openings' voids
    /// ([`Self::with_applied_openings`]).
    applied_openings: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Bodies the host subtracted some wholes' openings from
    /// ([`Self::with_whole_openings`]).
    whole_openings: BTreeMap<ObjectId, Vec<ObjectId>>,
}

/// How a whole's body is built from its parts.
#[derive(Debug)]
struct Composition {
    /// The parts stated for the whole, in identity order.
    parts: Vec<ObjectId>,
    /// The objects whose own bodies make up the whole: its parts, and the
    /// parts of any part measured through its parts in turn.
    pieces: BTreeSet<ObjectId>,
}

/// A whole's body as the union of its parts' bodies, built by
/// [`AxiolidGeometry::compose`] from the parts already registered, and
/// registered with [`AxiolidGeometry::with_composed_body`].
///
/// The mesh is the parts' meshes side by side, unwelded, so each stays the
/// closed solid it was. Its surface therefore includes the faces where parts
/// meet, and a point inside the whole near such a face measures its depth to
/// that face: a lower bound, as every witnessed depth is. A distance from
/// outside the whole is the least distance to any part, which is exactly the
/// distance to their union.
#[derive(Clone, Debug)]
pub struct ComposedBody {
    parts: Vec<ObjectId>,
    pieces: BTreeSet<ObjectId>,
    mesh: TriMesh,
    /// The largest chord deviation of a tessellated part; `None` when every
    /// part is exact.
    deviation: Option<f64>,
    exact: Option<ExactBody>,
}

impl ComposedBody {
    /// The parts stated for the whole, in identity order.
    #[must_use]
    pub fn parts(&self) -> &[ObjectId] {
        &self.parts
    }

    /// The union's mesh.
    #[must_use]
    pub fn mesh(&self) -> &TriMesh {
        &self.mesh
    }

    /// Whether every part is exact, so the union is too.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.deviation.is_none()
    }

    /// The deviation the union is declared within: the largest of its
    /// tessellated parts', zero when every part is exact.
    #[must_use]
    pub fn deviation_metres(&self) -> f64 {
        self.deviation.unwrap_or(0.0)
    }

    /// Whether every part had an exact body, so the union has one too.
    #[must_use]
    pub fn has_exact_body(&self) -> bool {
        self.exact.is_some()
    }
}

impl AxiolidGeometry {
    /// Creates an empty geometry set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that an object occupies no volume, e.g. a storey or a zone.
    ///
    /// Services skip it where they would otherwise need its mesh, such as
    /// when every other object is a candidate obstacle.
    #[must_use]
    pub fn with_no_body(mut self, object: ObjectId) -> Self {
        self.bodiless.insert(object);
        self
    }

    /// Declares that an object has a body this host could not mesh.
    ///
    /// Its extent is unknown, so any measurement it could affect is refused
    /// rather than taken as if the object were not there.
    #[must_use]
    pub fn with_unmeasured(mut self, object: ObjectId, reason: impl Into<String>) -> Self {
        self.unmeasured.insert(object, reason.into());
        self
    }

    /// Declares a box an unmeasured object's body is known to lie within,
    /// in world metres, such as one the source states beside the body it
    /// could not mesh.
    ///
    /// The body stays unmeasured; the box only lets a service tell where it
    /// cannot be, so a measurement far from it need not refuse. A box that
    /// is not finite or whose minimum exceeds its maximum is ignored: an
    /// object without a bound may be anywhere.
    #[must_use]
    pub fn with_unmeasured_bound(mut self, object: ObjectId, min: [f64; 3], max: [f64; 3]) -> Self {
        let valid = min
            .iter()
            .zip(&max)
            .all(|(low, high)| low.is_finite() && high.is_finite() && low <= high);
        if valid {
            self.unmeasured_bounds.insert(object, (min, max));
        } else {
            self.unmeasured_bounds.remove(&object);
        }
        self
    }

    /// The box an unmeasured object's body lies within, when the host
    /// declared one; `None` for a measured object or one that may be
    /// anywhere.
    pub(crate) fn unmeasured_bound(&self, object: &ObjectId) -> Option<&Extent> {
        if !self.is_unmeasured(object) {
            return None;
        }
        self.unmeasured_bounds.get(object)
    }

    /// Certifies bounds on the extent of a tessellated object's true body,
    /// in world metres, beside its chord deviation: the body lies within
    /// `outer`, and, where `inner` is given, reaches out to it along every
    /// axis (its lowest point lies at or below `inner`'s minimum, its
    /// highest at or above its maximum). An exact boundary's
    /// [`ExactBoundary::extent_bounds`](crate::ExactBoundary::extent_bounds)
    /// gives both, from the construction rather than the mesh.
    ///
    /// The deviation alone leaves every face of the body free to move by
    /// it, so a slab whose top meets a floor could rise into the room
    /// above. These bounds pin the extent closer where they are tighter:
    /// services read them through the body's vertical bounds only, and only
    /// for a tessellated body (an exact mesh is its own extent). Bounds
    /// that are not finite, are reversed or whose `inner` leaves `outer`
    /// are ignored.
    #[must_use]
    pub fn with_extent_bounds(
        mut self,
        object: ObjectId,
        outer: ([f64; 3], [f64; 3]),
        inner: Option<([f64; 3], [f64; 3])>,
    ) -> Self {
        let finite = |(min, max): &Extent| {
            min.iter()
                .zip(max)
                .all(|(low, high)| low.is_finite() && high.is_finite() && low <= high)
        };
        let within = |(min, max): &Extent| {
            (0..3).all(|k| {
                min[k].is_finite()
                    && max[k].is_finite()
                    && outer.0[k] <= min[k]
                    && max[k] <= outer.1[k]
            })
        };
        if finite(&outer) && inner.as_ref().is_none_or(within) {
            self.extent_bounds.insert(object, (outer, inner));
        } else {
            self.extent_bounds.remove(&object);
        }
        self
    }

    /// The chord deviation of an object's mesh: zero for an exact one,
    /// `None` for a declared deviation that bounds nothing (negative or not
    /// finite).
    pub(crate) fn deviation(&self, object: &ObjectId) -> Option<f64> {
        self.fidelity(object)
            .ok()
            .map(|fidelity| fidelity.deviation_metres())
    }

    /// Where an object's true body may begin and end vertically: its mesh's
    /// lowest and highest points, each widened by its chord deviation and
    /// narrowed by the bounds the host certified
    /// ([`Self::with_extent_bounds`]). Points for an exact mesh. `None`
    /// without a mesh, with a deviation that bounds nothing, or where the
    /// certified bounds contradict the mesh.
    pub(crate) fn vertical_bounds(&self, object: &ObjectId) -> Option<VerticalBounds> {
        let (min, max) = mesh_extent(self.meshes.get(object)?)?;
        let deviation = self.deviation(object)?;
        let mut bounds = VerticalBounds {
            bottom: (min[2] - deviation, min[2] + deviation),
            top: (max[2] - deviation, max[2] + deviation),
        };
        if deviation > 0.0
            && let Some((outer, inner)) = self.extent_bounds.get(object)
        {
            bounds.bottom.0 = bounds.bottom.0.max(outer.0[2]);
            bounds.top.1 = bounds.top.1.min(outer.1[2]);
            if let Some(inner) = inner {
                bounds.bottom.1 = bounds.bottom.1.min(inner.0[2]);
                bounds.top.0 = bounds.top.0.max(inner.1[2]);
            }
            if bounds.bottom.0 > bounds.bottom.1 || bounds.top.0 > bounds.top.1 {
                return None;
            }
        }
        Some(bounds)
    }

    /// Declares a bodiless group (a zone, say) and the objects it groups.
    ///
    /// Membership is a semantic fact a mesh cannot show, so the host states
    /// it. A group has no body of its own, so it is also declared bodiless:
    /// it obstructs nothing. Its plan footprint is the union of its members'
    /// footprints; a member may itself be a declared group.
    #[must_use]
    pub fn with_group(
        mut self,
        group: ObjectId,
        members: impl IntoIterator<Item = ObjectId>,
    ) -> Self {
        let mut members: Vec<ObjectId> = members.into_iter().collect();
        members.sort();
        members.dedup();
        self.bodiless.insert(group.clone());
        self.groups.insert(group, Ok(members));
        self
    }

    /// Declares a bodiless group whose membership the host could not decide.
    ///
    /// Measurements that need its members refuse with `reason`, rather than
    /// take the group as empty.
    #[must_use]
    pub fn with_undecided_group(mut self, group: ObjectId, reason: impl Into<String>) -> Self {
        self.bodiless.insert(group.clone());
        self.groups.insert(group, Err(reason.into()));
        self
    }

    /// A declared group's members in identity order, or the reason its
    /// membership is undecided; `None` when the object is no declared group.
    #[must_use]
    pub fn group_members(&self, group: &ObjectId) -> Option<Result<&[ObjectId], &str>> {
        self.groups.get(group).map(|members| match members {
            Ok(members) => Ok(members.as_slice()),
            Err(reason) => Err(reason.as_str()),
        })
    }

    /// Whether the host declared the object bodiless.
    #[must_use]
    pub fn has_no_body(&self, object: &ObjectId) -> bool {
        self.bodiless.contains(object)
    }

    /// Whether the host declared the object's body unmeasured.
    #[must_use]
    pub fn is_unmeasured(&self, object: &ObjectId) -> bool {
        self.unmeasured.contains_key(object)
    }

    /// Every object whose body could not be measured, with the host's reason.
    pub fn unmeasured(&self) -> impl Iterator<Item = (&ObjectId, &str)> {
        self.unmeasured
            .iter()
            .map(|(object, reason)| (object, reason.as_str()))
    }

    /// Registers one object's mesh, asserting every face is planar so the
    /// mesh is the object's exact shape. Curved parts belong in
    /// [`Self::with_tessellated_mesh`].
    ///
    /// A mesh whose faces repeat their corners (equal coordinates under
    /// different indices) and that is a closed two-manifold once equal
    /// positions are taken as one is registered with them shared, so it is
    /// measured as the closed solid it is. Its coordinates and triangles are
    /// unchanged; any other mesh is registered as given.
    #[must_use]
    pub fn with_mesh(mut self, object: ObjectId, mesh: TriMesh) -> Self {
        self.chord_deviations.remove(&object);
        self.meshes
            .insert(object, crate::shell::welded_where_closed(mesh));
        self
    }

    /// Registers one object's mesh as a tessellation of curved faces.
    ///
    /// `chord_deviation_metres` bounds how far the true surface may lie from
    /// the mesh. Measurements that honour fidelity report such an object as
    /// approximate, never exact. An invalid deviation is kept and refused when
    /// measured, so it cannot silently become exact. Repeated corners are
    /// shared as in [`Self::with_mesh`].
    #[must_use]
    pub fn with_tessellated_mesh(
        mut self,
        object: ObjectId,
        mesh: TriMesh,
        chord_deviation_metres: f64,
    ) -> Self {
        self.chord_deviations
            .insert(object.clone(), chord_deviation_metres);
        self.meshes
            .insert(object, crate::shell::welded_where_closed(mesh));
        self
    }

    /// Registers the exact boundary of an object whose mesh is registered
    /// too, typically a tessellated one.
    ///
    /// The mesh stays the basis of every measurement. The boundary only
    /// narrows proximity between two objects that both have one
    /// ([`crate::AxiolidProximityService`]): the kernel's certified
    /// `boundary_distance` bounds the distance in space, and its plan
    /// measurements (`plan_boundary_distance`, `plan_boundary_clearance`,
    /// `plan_overlap`) the horizontal distance, plan overlap and the
    /// footprint relation of vertical distances, where the chord deviation
    /// would leave them wide or open. The surface distance between two
    /// revisions is measured between their boundaries where both have one
    /// (`one_sided_boundary_hausdorff_with_budget` each way). The host asserts that boundary and
    /// mesh describe one body; a pair whose certified answer contradicts the
    /// mesh's widened one refuses.
    ///
    /// The solid is one item in world coordinates, exactly the model's;
    /// [`Self::with_exact_body`] takes several items, a placement and a
    /// perturbation.
    #[must_use]
    pub fn with_exact_boundary(self, object: ObjectId, boundary: ExactBRep) -> Self {
        self.with_exact_body(object, ExactBody::new(boundary))
    }

    /// Registers the exact body of an object whose mesh is registered too:
    /// one or more items in the body's frame and their placement
    /// (axiolid/kernel#229), as [`fn@crate::exact_boundary`] builds it.
    ///
    /// Several items are measured by the kernel's body queries
    /// (`body_boundary_distance`, `one_sided_body_boundary_hausdorff` and
    /// the plan queries `body_plan_*`), which certify the distance in
    /// space, the surface distance and the plan relations. A perturbed body
    /// ([`ExactBody::perturbation_metres`]) widens every distance measured
    /// on it by its perturbation, never certifies a plan overlap and is not
    /// used for a surface distance (which needs exact surfaces); a
    /// boolean's rounding ([`ExactBody::rounding_metres`]) widens every
    /// distance too, but the body stays exact.
    #[must_use]
    pub fn with_exact_body(mut self, object: ObjectId, body: ExactBody) -> Self {
        self.boundaries.insert(object, Arc::new(body));
        self
    }

    /// The exact body registered for an object.
    #[must_use]
    pub fn exact_boundary(&self, object: &ObjectId) -> Option<&ExactBody> {
        self.boundaries.get(object).map(Arc::as_ref)
    }

    /// The exact body registered for an object, shared, to hand out beside
    /// its surface.
    pub(crate) fn shared_exact_boundary(&self, object: &ObjectId) -> Option<Arc<ExactBody>> {
        self.boundaries.get(object).cloned()
    }

    /// How faithfully an object's mesh represents it.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared chord deviation is negative or
    /// non-finite.
    pub fn fidelity(&self, object: &ObjectId) -> Result<GeometryFidelity, ProximityError> {
        self.chord_deviations
            .get(object)
            .map_or(Ok(GeometryFidelity::Exact), |deviation| {
                GeometryFidelity::tessellated(*deviation)
            })
    }

    /// Returns the mesh registered for an object.
    #[must_use]
    pub fn mesh(&self, object: &ObjectId) -> Option<&TriMesh> {
        self.meshes.get(object)
    }

    /// Whether an object's mesh was registered as a tessellation.
    pub(crate) fn is_tessellated(&self, object: &ObjectId) -> bool {
        self.chord_deviations.contains_key(object)
    }

    /// An object's mesh extent grown by its chord deviation, so it encloses
    /// the true body. `None` without a mesh or with an invalid deviation.
    pub(crate) fn enclosing_extent(&self, object: &ObjectId) -> Option<Extent> {
        let (min, max) = mesh_extent(self.meshes.get(object)?)?;
        let deviation = self.fidelity(object).ok()?.deviation_metres();
        Some((min.map(|v| v - deviation), max.map(|v| v + deviation)))
    }

    /// A tessellated object that could change a measurement taken around
    /// `probe`: its enclosing extent lies within `reach` of it, in plan when
    /// `plan` is set. Objects `skip` accepts are ignored, and an invalid
    /// declared deviation counts as near, since nothing bounds it.
    ///
    /// Services that report exact evidence refuse when this finds anything:
    /// a chord approximation near the measurement makes the result an
    /// estimate, and presenting it as exact would launder it into fact.
    pub(crate) fn tessellated_near(
        &self,
        probe: &Extent,
        reach: f64,
        plan: bool,
        skip: impl Fn(&ObjectId) -> bool,
    ) -> Option<&ObjectId> {
        self.chord_deviations.keys().find(|object| {
            !skip(object)
                && self
                    .enclosing_extent(object)
                    .is_none_or(|extent| extent_gap(probe, &extent, plan) <= reach)
        })
    }

    /// The body of a whole with no body of its own, built from its parts'
    /// registered bodies, or why it cannot be.
    ///
    /// Fails closed. The parts are taken in identity order, and the first
    /// that is unmeasured, or has neither a mesh nor a declared lack of body,
    /// leaves the whole unmeasured with a reason naming it. A part declared
    /// bodiless ([`Self::with_no_body`]) occupies no material and adds
    /// nothing; a whole none of whose parts occupies material has no body to
    /// measure either. The union is exact when every part is, and otherwise
    /// tessellated within the largest deviation its parts carry (an invalid
    /// one kept, so it is refused when measured). It has an exact body when
    /// every part with a mesh has one registered ([`Self::with_exact_body`]):
    /// their items side by side in the world, perturbed and rounded as the
    /// most perturbed and rounded part.
    ///
    /// A part may itself be a whole measured through its parts: register
    /// the inner whole first.
    ///
    /// # Errors
    ///
    /// Returns the reason the whole stays unmeasured.
    pub fn compose(&self, parts: &[ObjectId]) -> Result<ComposedBody, String> {
        let mut parts = parts.to_vec();
        parts.sort();
        parts.dedup();
        let stated = parts.len();
        let union = |rest: String| {
            format!(
                "its body is the union of its {stated} part{}, and {rest}",
                if stated == 1 { "" } else { "s" }
            )
        };
        if parts.is_empty() {
            return Err("it states no parts to measure it by".to_owned());
        }
        let mut meshed: Vec<&ObjectId> = Vec::new();
        for part in &parts {
            if let Some(reason) = self.unmeasured.get(part) {
                return Err(union(format!("part {part} is unmeasured: {reason}")));
            }
            if self.meshes.contains_key(part) {
                meshed.push(part);
            } else if !self.bodiless.contains(part) {
                return Err(union(format!("part {part} has no measured body")));
            }
        }
        if meshed.is_empty() {
            return Err(union("none of them occupies material".to_owned()));
        }

        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut deviation: Option<f64> = None;
        let mut pieces = BTreeSet::new();
        for part in &meshed {
            let mesh = &self.meshes[*part];
            let offset = u32::try_from(positions.len())
                .map_err(|_| union("their meshes hold too many positions".to_owned()))?;
            positions.extend(mesh.positions.iter().copied());
            for index in &mesh.indices {
                indices.push(
                    index
                        .checked_add(offset)
                        .ok_or_else(|| union("their meshes hold too many positions".to_owned()))?,
                );
            }
            if let Some(part_deviation) = self.chord_deviations.get(*part) {
                // An invalid (NaN) deviation is kept, so it is refused when
                // measured rather than lost to a comparison.
                deviation = Some(match deviation {
                    Some(held) if held.is_nan() || held >= *part_deviation => held,
                    _ => *part_deviation,
                });
            }
            match self.compositions.get(*part) {
                Some(inner) => pieces.extend(inner.pieces.iter().cloned()),
                None => {
                    pieces.insert((*part).clone());
                }
            }
        }
        let exact = meshed
            .iter()
            .map(|part| self.boundaries.get(*part).map(Arc::as_ref))
            .collect::<Option<Vec<&ExactBody>>>()
            .and_then(|bodies| ExactBody::union(bodies).ok());
        Ok(ComposedBody {
            parts,
            pieces,
            mesh: TriMesh::new(positions, indices),
            deviation,
            exact,
        })
    }

    /// The box enclosing a whole whose body is the union of `parts` but
    /// could not be composed, in world metres: each meshed part's mesh box
    /// grown by its chord deviation and each unmeasured part's declared
    /// bound ([`Self::with_unmeasured_bound`]); a bodiless part adds
    /// nothing.
    ///
    /// `None` when any part is unbounded (unmeasured without a bound,
    /// undescribed, or with an invalid deviation) or none occupies
    /// material: the whole may then be anywhere. Hand the result to
    /// [`Self::with_unmeasured_bound`] for the whole.
    #[must_use]
    pub fn parts_bound(&self, parts: &[ObjectId]) -> Option<([f64; 3], [f64; 3])> {
        let mut bound: Option<Extent> = None;
        for part in parts {
            let extent = if self.meshes.contains_key(part) {
                self.enclosing_extent(part)?
            } else if self.is_unmeasured(part) {
                *self.unmeasured_bound(part)?
            } else if self.bodiless.contains(part) {
                continue;
            } else {
                return None;
            };
            bound = Some(match bound {
                None => extent,
                Some((min, max)) => (
                    std::array::from_fn(|axis| min[axis].min(extent.0[axis])),
                    std::array::from_fn(|axis| max[axis].max(extent.1[axis])),
                ),
            });
        }
        bound
    }

    /// Registers a whole with no body of its own, measured as the union of
    /// its parts ([`Self::compose`]): its mesh, exactness and exact body are
    /// the union's, and its identity stays its own.
    ///
    /// The whole and each of its parts (at any depth) then share material,
    /// so they never form a pair ([`Self::shares_body`]).
    #[must_use]
    pub fn with_composed_body(mut self, whole: ObjectId, body: ComposedBody) -> Self {
        self.unmeasured.remove(&whole);
        self.bodiless.remove(&whole);
        self = match body.deviation {
            None => self.with_mesh(whole.clone(), body.mesh),
            Some(deviation) => self.with_tessellated_mesh(whole.clone(), body.mesh, deviation),
        };
        if let Some(exact) = body.exact {
            self = self.with_exact_body(whole.clone(), exact);
        }
        self.compositions.insert(
            whole,
            Arc::new(Composition {
                parts: body.parts,
                pieces: body.pieces,
            }),
        );
        self
    }

    /// The parts a whole measured through its parts was registered with,
    /// in identity order; `None` for any other object.
    #[must_use]
    pub fn parts_of(&self, whole: &ObjectId) -> Option<&[ObjectId]> {
        self.compositions
            .get(whole)
            .map(|composition| composition.parts.as_slice())
    }

    /// The objects whose own bodies make up `object`: its pieces when it is
    /// measured through its parts, the object itself otherwise.
    pub(crate) fn pieces<'a>(&'a self, object: &'a ObjectId) -> Vec<&'a ObjectId> {
        match self.compositions.get(object) {
            Some(composition) => composition.pieces.iter().collect(),
            None => vec![object],
        }
    }

    /// Declares that `host`'s registered body already carries the voids of
    /// `openings`, as the source states, so nothing was subtracted for them.
    ///
    /// The body is measured as registered; this records only how it was
    /// obtained, for the evidence of every measurement of it (the locator
    /// suffix `;applied-openings:<host>=<opening>+...`). The source's
    /// reason is the host's to report. An empty list records nothing.
    #[must_use]
    pub fn with_applied_openings(mut self, host: ObjectId, mut openings: Vec<ObjectId>) -> Self {
        openings.sort();
        openings.dedup();
        if openings.is_empty() {
            self.applied_openings.remove(&host);
        } else {
            self.applied_openings.insert(host, openings);
        }
        self
    }

    /// The openings whose voids `host`'s body already carries, in identity
    /// order ([`Self::with_applied_openings`]); `None` when none was
    /// declared.
    #[must_use]
    pub fn applied_openings(&self, host: &ObjectId) -> Option<&[ObjectId]> {
        self.applied_openings.get(host).map(Vec::as_slice)
    }

    /// Declares that the host subtracted `openings`, each voiding a whole
    /// measured through its parts, from `object`'s registered body: from a
    /// part's own body where the opening cuts it, and so from the body of
    /// every whole that part is a piece of.
    ///
    /// The body is measured as registered; this records only how it was
    /// obtained, for the evidence of every measurement of it (the locator
    /// suffix `;whole-openings:<object>=<opening>+...`). An empty list
    /// records nothing.
    #[must_use]
    pub fn with_whole_openings(mut self, object: ObjectId, mut openings: Vec<ObjectId>) -> Self {
        openings.sort();
        openings.dedup();
        if openings.is_empty() {
            self.whole_openings.remove(&object);
        } else {
            self.whole_openings.insert(object, openings);
        }
        self
    }

    /// The wholes' openings subtracted from `object`'s body, in identity
    /// order ([`Self::with_whole_openings`]); `None` when none was
    /// declared.
    #[must_use]
    pub fn whole_openings(&self, object: &ObjectId) -> Option<&[ObjectId]> {
        self.whole_openings.get(object).map(Vec::as_slice)
    }

    /// The suffix an evidence locator carries about how the bodies of
    /// `objects` were obtained, empty when there is nothing to state: for
    /// each object measured through its parts `;union:<object>=<n>-parts`,
    /// so the evidence states that its body is the union of them, and for
    /// each host whose body already carries openings
    /// `;applied-openings:<host>=<opening>+<opening>`, and for each body
    /// some wholes' openings were subtracted from
    /// `;whole-openings:<object>=<opening>+<opening>`.
    pub(crate) fn body_note(&self, objects: &[&ObjectId]) -> String {
        use std::fmt::Write as _;
        let mut note = String::new();
        for object in objects {
            if let Some(parts) = self.parts_of(object) {
                let _ = write!(note, ";union:{object}={}-parts", parts.len());
            }
            if let Some(openings) = self.applied_openings(object) {
                let openings: Vec<String> = openings.iter().map(ToString::to_string).collect();
                let _ = write!(note, ";applied-openings:{object}={}", openings.join("+"));
            }
            if let Some(openings) = self.whole_openings(object) {
                let openings: Vec<String> = openings.iter().map(ToString::to_string).collect();
                let _ = write!(note, ";whole-openings:{object}={}", openings.join("+"));
            }
        }
        note
    }

    /// Whether two distinct objects' bodies share material: one is a whole
    /// measured through its parts and the other one of those parts, at any
    /// depth, or both are wholes holding a part in common.
    #[must_use]
    pub fn shares_body(&self, first: &ObjectId, second: &ObjectId) -> bool {
        if first == second {
            return false;
        }
        let first_pieces = self.pieces(first);
        let second_pieces: BTreeSet<&ObjectId> = self.pieces(second).into_iter().collect();
        // A part is its own only piece and an inner whole's pieces are among
        // the outer whole's, so one common piece decides every case.
        first_pieces
            .iter()
            .any(|piece| second_pieces.contains(piece))
    }

    /// Every registered object and its mesh, in identity order.
    pub(crate) fn objects(&self) -> impl Iterator<Item = (&ObjectId, &TriMesh)> {
        self.meshes.iter()
    }
}

/// An axis-aligned `(min, max)` extent in metres.
pub(crate) type Extent = ([f64; 3], [f64; 3]);

/// Where a body's true lowest (`bottom`) and highest (`top`) points lie,
/// each as `(low, high)` elevations in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VerticalBounds {
    pub(crate) bottom: (f64, f64),
    pub(crate) top: (f64, f64),
}

impl VerticalBounds {
    /// Whether the body surely stays out of the open band `low < z < high`,
    /// up to `slack` at either limit: it ends at or below `low`, or begins
    /// at or above `high`, whichever reading of its bounds is true.
    pub(crate) fn clear_of(&self, low: f64, high: f64, slack: f64) -> bool {
        self.top.1 <= low + slack || self.bottom.0 >= high - slack
    }
}

/// The extent of a mesh's positions; `None` for an empty mesh.
pub(crate) fn mesh_extent(mesh: &TriMesh) -> Option<Extent> {
    let mut positions = (0..mesh.position_count()).map(|i| mesh.position(i));
    let first = positions.next()?;
    let (min, max) = positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
    Some((min.to_array(), max.to_array()))
}

/// Euclidean gap between two extents, over x and y only when `plan` is set.
pub(crate) fn extent_gap(a: &Extent, b: &Extent, plan: bool) -> f64 {
    let axes = if plan { 2 } else { 3 };
    (0..axes)
        .map(|axis| {
            let gap = (b.0[axis] - a.1[axis]).max(a.0[axis] - b.1[axis]).max(0.0);
            gap * gap
        })
        .sum::<f64>()
        .sqrt()
}

/// A triangle as three points, the form the geometry primitives consume.
pub(crate) type Triangle = [axiolid_core::Point3; 3];

/// The triangles of a mesh as coordinate triples.
pub(crate) fn triangles(mesh: &TriMesh) -> Vec<Triangle> {
    (0..mesh.triangle_count())
        .map(|index| {
            let [a, b, c] = mesh.triangle(index);
            // Indices come from a foreign mesh, so a value that cannot be a
            // position index is a corrupt mesh, not something to truncate.
            [a, b, c].map(|index| {
                usize::try_from(index)
                    .ok()
                    .filter(|i| *i < mesh.position_count())
                    .map_or(Point3::ZERO, |i| mesh.position(i))
            })
        })
        .collect()
}
