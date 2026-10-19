//! Pairwise proximity measured with the Axiolid kernel.
//!
//! ADR 0004: this module **measures**. It reports how close two bodies come,
//! how far their footprints overlap and how deep one reaches into the other.
//! Whether that is a clash is a rule's decision.
//!
//! Three measurements, each from a published Axiolid primitive:
//!
//! - **Separation** folds `closest_points_on_triangles` over triangle pairs,
//!   skipping pairs whose boxes are already farther apart than the best found.
//! - **Plan overlap** intersects the projected triangle soups, as contact does.
//! - **Penetration** is witnessed. Zero separation means the surfaces meet;
//!   it does not say whether the bodies only touch or interpenetrate. Points of
//!   one body are sampled -- vertices, edge and face centres, and the midpoints
//!   between where each edge crosses the other surface -- and a point the
//!   other body's winding number places inside it contributes its distance to
//!   that surface. A pipe through a wall has no vertex inside the wall, but its
//!   edges cross both wall faces and the midpoint between the crossings lies
//!   half a wall deep. The deepest witness is a lower bound on the true depth.
//!
//! Two shape comparisons ride along:
//!
//! - **Overlap extents** bound the intersection's box along each axis. The
//!   lower bound spans points witnessed in both bodies: where an edge of
//!   either crosses the other's surface, and vertices of either inside the
//!   other. For polyhedra those are the intersection's vertices, so the bound
//!   is tight wherever the crossings are found. The upper bound is the
//!   overlap of the two bodies' boxes.
//!   The same measurement answers extents along any stated directions
//!   ([`ProximityService::measure_overlap_along`]), such as a wall's own
//!   axes: positions are projections, the upper bound the overlap of the
//!   two bodies' ranges along the direction, and off the coordinate axes
//!   every projection widens by a bound on its rounding.
//! - **Intersection volume** between two closed solids, with each body's
//!   own volume, from `axiolid-inspect`'s certified volume integrals: every
//!   value an interval sure to hold the true one. Bodies apart at the
//!   surface and not holding one another share exactly nothing. A mesh the
//!   kernel refuses (open, self-intersecting, no volume) leaves the volume
//!   unmeasured rather than failing the measurement. A tessellation widens
//!   each volume by the volume within its chord deviation of the mesh
//!   (`tube_volume`); an exact body's own volume is widened by the band of
//!   its coordinates' rounding (`volume_band`).
//! - **Hausdorff distance** between the surfaces. Its lower bound is the
//!   farthest any vertex lies from the other surface. Its upper bound holds
//!   per triangle: a distance to one convex triangle is convex, so the
//!   farthest point of a triangle from it is a vertex, and the least over
//!   the other body's triangles of that farthest vertex distance bounds the
//!   whole triangle. Triangles whose bound exceeds the lower bound are split
//!   a few times to tighten it. Identical meshes come out exactly zero.
//!
//! Winding numbers need an inside, so points are only tested against a closed
//! two-manifold mesh. A surface reaching into a solid is measured; two open
//! surfaces share no volume and report `None` rather than a depth of zero.
//!
//! A zero-area triangle is measured as the segment it is, and a mesh closed
//! only with such triangles is a solid whose winding number is taken
//! without them (`zero_area`).
//!
//! Distances in a projection ([`ProximityService::measure_distance`]) reuse
//! the same pieces:
//!
//! - **Horizontal** distance folds 2D closest points over the projected
//!   triangles, edge-on ones as segments, through the same indexed search as
//!   separation. The footprint is the union of the projected triangles and the
//!   distance between two unions is the least distance between their parts,
//!   so non-convex footprints are measured exactly without a polygon
//!   boundary-distance primitive.
//! - **Vertical** distance is the gap between the two meshes' vertical
//!   extents, for bodies whose footprints are related (see
//!   [`axioval_engine::ProximityProjection::Vertical`]). A direction takes
//!   the one-sided gap and relates only a counterpart on that side
//!   ([`axioval_engine::VerticalDirection`]), compared end by end.
//! - **Plan overlap** is the footprint overlay of plan overlap measurement.
//!
//! **Certified separation.** When both bodies of a tessellated pair have an
//! exact boundary registered ([`AxiolidGeometry::with_exact_boundary`]), the
//! distance in space is also bounded by `axiolid-measure`'s certified
//! `boundary_distance` (branch and bound over the exact faces and edges), to
//! [`CERTIFIED_ACCURACY_METRES`] and widened by a bound on the rounding of
//! its witness points. Both intervals hold the true separation, so the
//! evidence carries their intersection; an empty one refuses, since boundary
//! and mesh then describe different bodies. A boundary the kernel cannot
//! bound leaves the chord-widened interval, never a guess. Two
//! boundaries the kernel shows to touch or cross stop early with a point
//! of each that they share (axiolid-measure 0.3.10, axiolid/kernel#273);
//! the evidence locator then names their midpoint as
//! `:contact=(x,y,z)`. Like the mesh
//! separation it is the distance between the surfaces, so a body inside
//! another is apart from it; containment stays the winding test's.
//!
//! **Certified plan relations.** The same pair's footprints are certified
//! with the kernel's plan measurements, which see a solid's shadow as its
//! boundary's; a body of several items casts its items' shadows
//! (`body_plan_*`, axiolid-measure 0.3.9). `Horizontal` intersects the chord-widened interval with
//! `plan_boundary_distance` (zero also for a body standing inside another's
//! footprint), widened as above. The footprint relation of `PlanOverlap`
//! and of `Vertical` with a zero offset takes `plan_overlap`: `Overlapping`
//! (two planar faces sharing an open patch in plan) relates, `Disjoint`
//! with a gap above rounding does not, `Undecided` keeps the mesh's
//! relation; a certificate contradicting a decided mesh relation refuses.
//! The kernel's search is order-dependent, so an undecided order is asked
//! the other way round. With a positive offset the relation comes from
//! `plan_boundary_clearance` against the offset, intersected with the
//! chord-widened plan distance. `Vertical`'s `Nearest` surface is still
//! found on the meshes.
//!
//! A tessellation widens every distance by the combined chord deviation. Its
//! footprint may differ from the mesh footprint by up to the deviation, so
//! overlap is only asserted from a witness point lying deeper than the
//! deviations inside both mesh footprints, and denied only when the plan
//! distance exceeds them; anything between is reported open. A direction is
//! decided the same way: a tessellated end may move by the deviation, so ends
//! nearer each other than the combined deviation leave the side open.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axiolid_core::{Aabb, Point3, Ray3, Tolerance};
use axiolid_inspect::{enclosed_volume, intersection_volume};
use axiolid_measure::{
    BodyPlanOverlap, DistanceBounds, PlacedBody, PlanOverlap, body_boundary_distance,
    body_plan_boundary_clearance, body_plan_boundary_distance, body_plan_overlap,
    boundary_distance, closest_point_on_triangle, closest_points_on_segments,
    closest_points_on_triangles, one_sided_body_boundary_hausdorff_with_budget,
    one_sided_boundary_hausdorff_with_budget, plan_boundary_clearance, plan_boundary_distance,
    plan_overlap,
};
use axiolid_mesh::TriMesh;
use axiolid_spatial::{Bvh, SpatialItem};
use axioval_engine::{
    BodyContainment, BodyVolume, Bounds3, ConvexPlanRegion, FaceDistanceError,
    FaceDistanceEvidence, FaceDistanceRequest, GeometryFidelity, IntersectionVolume,
    LengthInterval, LineDistanceEvidence, LineDistanceRequest, LineProjection, MetricDirection,
    ObjectBounds, OverlapAlongEvidence, OverlapAlongRequest, OverlapExtents, PointBox,
    ProjectedDistanceEvidence, ProximityError, ProximityEvidence, ProximityProjection,
    ProximityRequest, ProximityService, RegionDistanceEvidence, RegionDistanceRequest,
    VerticalDirection, VerticalSurfaces, VolumeInterval,
};
use axioval_engine::{
    BodySurface, DrainageEvidence, DrainageRequest, ExactBoundaryHandle, HeightAboveEvidence,
    HeightAboveRequest, SurfaceDistanceEvidence, SurfaceDistanceRequest,
};
use axioval_ir::{
    Evidence, ObjectId, Witness, WitnessBox, WitnessExtent, WitnessKind, WitnessPoint,
};

use crate::exact_boundary::ExactBody;
use crate::geometry::{AxiolidGeometry, Triangle};
use crate::planar::{BoundedOverlap, bounded_plan_overlap, plan_overlap_area, polygon_area};

mod bodies;
mod shells;
mod zero_area;

pub(crate) use zero_area::Winding;
use zero_area::{point_triangle_distance, segment_hit, triangle_pair_distance};

/// Linear tolerance for mesh audits, overlay and crossing tests.
///
/// Also the distance below which surfaces are taken to meet: floating-point
/// closest points of two touching faces rarely come out exactly zero.
pub(crate) const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// Plan overlap area below which exact footprints only touch.
///
/// The overlay snaps its output to a grid (axiolid/kernel#173), so footprints
/// meeting along an edge can come back as a sliver of rounding.
const OVERLAP_AREA_TOLERANCE: f64 = 1e-9;

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

/// How often a triangle is split in four to tighten its Hausdorff bound.
const HAUSDORFF_REFINEMENT: u32 = 2;

/// Width the certified boundary distance is refined to: a micrometre, far
/// below any clearance a rule states and above the rounding of building
/// coordinates. Refinement stops earlier only when its step budget runs
/// out, and the interval is sound either way.
pub const CERTIFIED_ACCURACY_METRES: f64 = 1e-6;

/// Splits the boundary Hausdorff distance may spend in each direction of
/// one pair before it stops with the interval it has.
///
/// Identical and translated copies, the changes a revision mostly makes,
/// close without a split (axiolid/kernel#227); a turned or reshaped body
/// closes only at first order and would otherwise spend the kernel's whole
/// budget of 200,000 splits a side. The interval left at this cap is sound,
/// only wider, and the comparison judges it as every interval: changed
/// above the tolerance, unchanged within it, undetermined across it.
pub const BOUNDARY_HAUSDORFF_SPLITS: usize = 4096;

/// An enclosed volume as `(lower, upper)`, or why it cannot be measured.
type Volume = Result<(f64, f64), ProximityError>;

/// Measures pairwise proximity between registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidProximityService {
    geometry: Arc<AxiolidGeometry>,
    /// Whether a bodiless object's registered void is measured as the
    /// solid of that void ([`Self::measuring_voids`]).
    voids: bool,
    /// Each whole measured through its parts' enclosed volume
    /// (`union_volume`), taken once: every pair the whole is in reads it,
    /// and it costs a mesh boolean per two of its pieces.
    volumes: Mutex<BTreeMap<ObjectId, Volume>>,
    /// Each body's closed shells, where it has several (`shells::shells`),
    /// split once: a body the volume kernel refuses as self-intersecting is
    /// measured shell by shell against every closed body it meets.
    shells: Mutex<BTreeMap<ObjectId, Option<Arc<Vec<shells::Shell>>>>>,
}

impl AxiolidProximityService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self {
            geometry: Arc::new(geometry),
            voids: false,
            volumes: Mutex::new(BTreeMap::new()),
            shells: Mutex::new(BTreeMap::new()),
        }
    }

    /// The view of the same geometry that measures voids: an object
    /// declared bodiless with a registered void
    /// ([`AxiolidGeometry::with_void`], an opening) is measured as the
    /// solid of its void, exact or tessellated as registered, and one
    /// whose void is unmeasured is [`ProximityError::Unavailable`]. Every
    /// other object answers as through [`Self::new`]. The host registers
    /// it beside the material view
    /// ([`axioval_engine::ProximityServiceHandle::with_voids`]); only rules
    /// naming voids in their own right ask for it.
    #[must_use]
    pub fn measuring_voids(&self) -> Self {
        Self {
            geometry: Arc::clone(&self.geometry),
            voids: true,
            volumes: Mutex::new(BTreeMap::new()),
            shells: Mutex::new(BTreeMap::new()),
        }
    }

    /// How faithfully an object's measured mesh represents it: its void's
    /// in the view measuring voids, its body's otherwise.
    pub(crate) fn fidelity(&self, object: &ObjectId) -> Result<GeometryFidelity, ProximityError> {
        if self.voids && self.geometry.void(object).is_some() {
            return self.geometry.void_fidelity(object);
        }
        self.geometry.fidelity(object)
    }

    /// One closed body's enclosed volume, widened by its tessellation band,
    /// as `(lower, upper)`.
    fn piece_volume(&self, object: &ObjectId) -> Result<(f64, f64), ProximityError> {
        let body = self.body(object)?;
        // Only a closed two-manifold encloses a volume.
        if !body.solid {
            return Err(ProximityError::Refused(OPEN_SURFACE));
        }
        let deviation = self.fidelity(object)?.deviation_metres();
        let enclosed = enclosed_volume(body.mesh)
            .map_err(|error| ProximityError::Refused(volume_refusal(error)))?;
        let band = volume_band(&body, deviation);
        Ok(((enclosed.lower - band).max(0.0), enclosed.upper + band))
    }

    /// The volume two closed pieces share, widened by both tessellation
    /// bands, as `(lower, upper)`; nothing where their boxes lie apart.
    fn shared_volume(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<(f64, f64), ProximityError> {
        let (one, other) = (self.body(first)?, self.body(second)?);
        if !one.solid || !other.solid {
            return Err(ProximityError::Refused(OPEN_SURFACE));
        }
        let apart = (0..3).any(|axis| {
            one.soup.bounds.max()[axis] < other.soup.bounds.min()[axis]
                || other.soup.bounds.max()[axis] < one.soup.bounds.min()[axis]
        });
        if apart {
            return Ok((0.0, 0.0));
        }
        let shared = intersection_volume(one.mesh, other.mesh)
            .map_err(|error| ProximityError::Refused(volume_refusal(error)))?;
        let band = volume_band(&one, self.fidelity(first)?.deviation_metres())
            + volume_band(&other, self.fidelity(second)?.deviation_metres());
        Ok(((shared.lower - band).max(0.0), shared.upper + band))
    }

    /// The volume an object encloses, as `(lower, upper)`. A whole measured
    /// through its parts encloses the union of its pieces, whose meshes may
    /// overlap: at most the sum of their volumes, at least that sum less
    /// every volume two of them share, and at least its largest piece.
    fn union_volume(&self, object: &ObjectId) -> Result<(f64, f64), ProximityError> {
        let pieces = self.geometry.pieces(object);
        if let [single] = pieces.as_slice() {
            return self.piece_volume(single);
        }
        let cached = self
            .volumes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(object)
            .copied();
        if let Some(volume) = cached {
            return volume;
        }
        // Measured outside the lock: two rules may ask for two wholes at once.
        let volume = self.pieces_volume(&pieces);
        self.volumes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(object.clone(), volume);
        volume
    }

    /// The enclosed volume of the union of several pieces (`union_volume`).
    fn pieces_volume(&self, pieces: &[&ObjectId]) -> Result<(f64, f64), ProximityError> {
        let mut sum = (0.0, 0.0);
        let mut largest: f64 = 0.0;
        for piece in pieces {
            let (lower, upper) = self.piece_volume(piece)?;
            sum = (sum.0 + lower, sum.1 + upper);
            largest = largest.max(lower);
        }
        let mut overlap = 0.0;
        for (index, first) in pieces.iter().enumerate() {
            for second in &pieces[index + 1..] {
                overlap += self.shared_volume(first, second)?.1;
            }
        }
        Ok(((sum.0 - overlap).max(largest), sum.1))
    }

    /// The certified volume a pair shares when either is a whole measured
    /// through its parts, with each one's own volume (`union_volume`).
    ///
    /// Concatenated meshes would count a region two pieces share twice, so
    /// the shared volume is bounded piece by piece: at most the sum over
    /// every pair of pieces, at least the largest of them. `None` where a
    /// piece is not a closed solid, the kernel refuses one, or the two
    /// share a piece.
    fn composed_intersection(
        &self,
        subject: &ObjectId,
        counterpart: &ObjectId,
        disjoint: bool,
    ) -> Option<IntersectionVolume> {
        if self.geometry.shares_body(subject, counterpart) {
            return None;
        }
        let (subject_volume, counterpart_volume) = (
            self.union_volume(subject).ok()?,
            self.union_volume(counterpart).ok()?,
        );
        let shared = if disjoint {
            (0.0, 0.0)
        } else {
            let mut shared = (0.0_f64, 0.0);
            for first in self.geometry.pieces(subject) {
                for second in self.geometry.pieces(counterpart) {
                    let (lower, upper) = self.shared_volume(first, second).ok()?;
                    shared = (shared.0.max(lower), shared.1 + upper);
                }
            }
            shared
        };
        let interval = |(lower, upper): (f64, f64)| VolumeInterval::try_new(lower, upper).ok();
        IntersectionVolume::try_new(
            interval(shared)?,
            interval(subject_volume)?,
            interval(counterpart_volume)?,
        )
        .ok()
    }

    /// One body's closed shells where it has several, split once.
    fn shells_of(&self, object: &ObjectId, body: &Body<'_>) -> Option<Arc<Vec<shells::Shell>>> {
        let cached = self
            .shells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(object)
            .cloned();
        if let Some(shells) = cached {
            return shells;
        }
        // Split outside the lock: two rules may ask for two bodies at once.
        let split = shells::shells(body.mesh).map(Arc::new);
        self.shells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(object.clone(), split.clone());
        split
    }

    /// The volume a pair shares when one body is several closed shells
    /// the volume kernel refuses as a whole and the other a closed body it
    /// measures, bounded shell by shell (`shells`), with what the shells
    /// have outside the other. Only for two closed, exact bodies; `None`
    /// otherwise, and where both or neither are such shells.
    fn shells_intersection(
        &self,
        request: &ProximityRequest,
        (subject, counterpart): (&Body<'_>, &Body<'_>),
        disjoint: bool,
        exact: bool,
    ) -> Option<IntersectionVolume> {
        if !exact || !subject.solid || !counterpart.solid {
            return None;
        }
        let split = (
            self.shells_of(request.subject(), subject),
            self.shells_of(request.counterpart(), counterpart),
        );
        let (shells, other, shells_are_subject) = match split {
            (Some(shells), None) => (shells, counterpart, true),
            (None, Some(shells)) => (shells, subject, false),
            _ => return None,
        };
        let volume = enclosed_volume(other.mesh).ok()?;
        shells::intersection(
            &shells,
            other.mesh,
            (volume.lower, volume.upper),
            (other.soup.bounds.min(), other.soup.bounds.max()),
            disjoint,
            shells_are_subject,
        )
    }

    /// `object`'s body: its mesh, measured as its void's in the view
    /// measuring voids, with the index the check builds of it once
    /// ([`bodies::IndexedBodies`], #408).
    pub(crate) fn body(&self, object: &ObjectId) -> Result<Body<'_>, ProximityError> {
        let key = bodies::BodyKey::new(&self.geometry, object, self.voids);
        let mesh = key.mesh(&self.geometry)?;
        Ok(Body {
            mesh,
            indexed: bodies::indexed(&key)?,
        })
    }

    /// The exact boundaries of a tessellated pair whose objects both have
    /// one, which certify its measurements.
    ///
    /// `None` leaves the chord-widened mesh measurements: an exact pair needs
    /// no certificate (its evidence is a point), and a pair without both
    /// boundaries has none.
    pub(crate) fn boundaries(
        &self,
        subject: &ObjectId,
        counterpart: &ObjectId,
    ) -> Result<Option<Boundaries<'_>>, ProximityError> {
        let fidelity = self
            .fidelity(subject)?
            .combined(self.fidelity(counterpart)?);
        if fidelity.is_exact() {
            return Ok(None);
        }
        Ok(self
            .geometry
            .exact_boundary(subject)
            .zip(self.geometry.exact_boundary(counterpart))
            .map(|(subject, counterpart)| Boundaries {
                subject,
                counterpart,
            }))
    }

    /// The certified Hausdorff distance between the subject's exact boundary
    /// and the counterpart's, when both have one this kernel reads.
    ///
    /// `None` leaves the mesh distance: a side without a boundary, a
    /// boundary of another backend, a kernel refusal or an ill-formed
    /// interval. Each direction spends at most
    /// [`BOUNDARY_HAUSDORFF_SPLITS`] splits, so the interval may come back
    /// wider than the request's accuracy (a turned or reshaped pair closes
    /// only at first order); it is sound either way, and judging its width
    /// is the caller's.
    fn boundary_surface_distance(
        &self,
        request: &SurfaceDistanceRequest,
    ) -> Result<Option<SurfaceDistanceEvidence>, ProximityError> {
        let Some((subject, counterpart)) = self.geometry.exact_boundary(request.subject()).zip(
            request
                .counterpart()
                .exact_boundary()
                .and_then(ExactBoundaryHandle::downcast_ref::<ExactBody>),
        ) else {
            return Ok(None);
        };
        // A surface distance is certified only on exact surfaces: a body
        // built from operands moved within a tolerance is not the model's
        // exactly, so its meshes are compared instead. A boolean the kernel
        // built without such a decision (a wall less its openings, placed
        // along the axes) is, up to its rounding, which widens each side.
        if !subject.is_exact() || !counterpart.is_exact() {
            return Ok(None);
        }
        let directed = |from: &ExactBody, to: &ExactBody| {
            let bounds = match (from.single(), to.single()) {
                (Some(from), Some(to)) => one_sided_boundary_hausdorff_with_budget(
                    from,
                    to,
                    request.accuracy_metres(),
                    Tolerance::METRE,
                    BOUNDARY_HAUSDORFF_SPLITS,
                )
                .ok()?,
                // A refusal of the items' layout (contact on a plane no
                // axis is normal to, items nearly sharing or overlapping a
                // face, a placement) leaves the mesh distance.
                _ => {
                    one_sided_body_boundary_hausdorff_with_budget(
                        placed(from),
                        placed(to),
                        request.accuracy_metres(),
                        Tolerance::METRE,
                        BOUNDARY_HAUSDORFF_SPLITS,
                    )
                    .ok()?
                    .bounds
                }
            };
            certified_directed(&bounds, from.widening_metres() + to.widening_metres())
        };
        let (Some(forward), Some(backward)) = (
            directed(subject, counterpart),
            directed(counterpart, subject),
        ) else {
            return Ok(None);
        };
        SurfaceDistanceEvidence::try_from_boundaries(
            request.clone(),
            forward,
            backward,
            Evidence {
                source: request.subject().source.clone(),
                locator: format!(
                    "axiolid:boundary-hausdorff:{}:{}",
                    request.subject(),
                    request.counterpart().object()
                ),
                exact: true,
                witness: None,
            },
        )
        .map(Some)
    }

    /// The interval of a distance in space measured as `distance` on the
    /// meshes: widened by the pair's chord deviation, narrowed by the
    /// certified boundary distance where both bodies have one.
    fn space_interval(
        &self,
        request: &ProximityRequest,
        distance: f64,
    ) -> Result<(f64, f64), ProximityError> {
        let deviation = self
            .fidelity(request.subject())?
            .combined(self.fidelity(request.counterpart())?)
            .deviation_metres();
        narrow(
            ((distance - deviation).max(0.0), distance + deviation),
            self.boundaries(request.subject(), request.counterpart())?
                .and_then(Boundaries::separation),
        )
    }

    /// A local point of `object`'s body in model coordinates (#372), as a
    /// witness point: within the body's chord deviation of its true
    /// geometry, widened by the rounding of the conversion.
    fn witness_point(
        &self,
        request: &ProximityRequest,
        object: &ObjectId,
        point: Point3,
    ) -> Result<WitnessPoint, ProximityError> {
        let ([x, y], rounding) = self
            .geometry
            .local_origin(&request.subject().source)
            .to_model_bounded([point.x, point.y]);
        WitnessPoint::try_new(
            [x, y, point.z],
            self.fidelity(object)?.deviation_metres() + rounding,
        )
        .map_err(|_| ProximityError::InvalidMeasurement)
    }

    /// The closest pair of a separation, `a` on the subject and `b` on the
    /// counterpart, as a witness.
    fn separation_witness(
        &self,
        request: &ProximityRequest,
        (a, b): (Point3, Point3),
    ) -> Result<Witness, ProximityError> {
        Ok(Witness::new(
            WitnessKind::Separation,
            self.witness_point(request, request.subject(), a)?,
            self.witness_point(request, request.counterpart(), b)?,
        ))
    }

    /// Where a measurement in space happens (#388): the deepest witnessed
    /// point of one body inside the other and the nearest point of the
    /// other's surface where the bodies interpenetrate, the closest pair
    /// otherwise, and the bracket of the shared region's box where they
    /// share anything. `None` where the spot could not be found.
    fn proximity_witness(
        &self,
        request: &ProximityRequest,
        bodies: (&Body<'_>, &Body<'_>),
        closest: Option<(Point3, Point3)>,
        deepest: Option<Deepest>,
        boxes: Option<OverlapBoxes>,
    ) -> Result<Option<Witness>, ProximityError> {
        let witness = match deepest {
            Some(Deepest {
                into_counterpart,
                point,
            }) => {
                let (inside, other) = if into_counterpart {
                    (request.subject(), bodies.1)
                } else {
                    (request.counterpart(), bodies.0)
                };
                let Some(leaving) = nearest_surface_point(point, other)? else {
                    return Ok(None);
                };
                let deep = self.witness_point(request, inside, point)?;
                if into_counterpart {
                    Witness::new(
                        WitnessKind::PenetrationIntoCounterpart,
                        deep,
                        self.witness_point(request, request.counterpart(), leaving)?,
                    )
                } else {
                    Witness::new(
                        WitnessKind::PenetrationIntoSubject,
                        self.witness_point(request, request.subject(), leaving)?,
                        deep,
                    )
                }
            }
            None => match closest {
                Some(pair) => self.separation_witness(request, pair)?,
                None => return Ok(None),
            },
        };
        let extent = boxes.and_then(|boxes| self.witness_extent(request, boxes));
        Ok(Some(match extent {
            Some(extent) => witness.with_extent(extent),
            None => witness,
        }))
    }

    /// The bracket of the shared region's box in model coordinates, the
    /// outer box grown and the inner one shrunk by the rounding of the
    /// conversion; `None` for bodies sharing nothing.
    fn witness_extent(
        &self,
        request: &ProximityRequest,
        boxes: OverlapBoxes,
    ) -> Option<WitnessExtent> {
        let origin = self.geometry.local_origin(&request.subject().source);
        let model = |(min, max): ([f64; 3], [f64; 3]), grow: f64| {
            let (low, low_rounding) = origin.to_model_bounded([min[0], min[1]]);
            let (high, high_rounding) = origin.to_model_bounded([max[0], max[1]]);
            let margin = grow * low_rounding.max(high_rounding);
            WitnessBox::try_new(
                [low[0] - margin, low[1] - margin, min[2]],
                [high[0] + margin, high[1] + margin, max[2]],
            )
            .ok()
        };
        let outer = model(boxes.outer?, 1.0)?;
        let inner = boxes.inner.and_then(|inner| model(inner, -1.0));
        WitnessExtent::try_new(inner.filter(|inner| outer.contains(inner)), outer).ok()
    }

    /// The certified volume the pair shares, with their own volumes: of a
    /// composed body through its parts, else of the two meshes, else shell
    /// by shell; `None` where none of them is measured.
    fn pair_volume(
        &self,
        request: &ProximityRequest,
        (subject, counterpart): (&Body<'_>, &Body<'_>),
        disjoint: bool,
    ) -> Result<Option<IntersectionVolume>, ProximityError> {
        let composed = self.geometry.parts_of(request.subject()).is_some()
            || self.geometry.parts_of(request.counterpart()).is_some();
        if composed {
            return Ok(self.composed_intersection(
                request.subject(),
                request.counterpart(),
                disjoint,
            ));
        }
        let subject_fidelity = self.fidelity(request.subject())?;
        let counterpart_fidelity = self.fidelity(request.counterpart())?;
        Ok(intersection(
            subject,
            counterpart,
            disjoint,
            subject_fidelity.deviation_metres(),
            counterpart_fidelity.deviation_metres(),
        )
        .or_else(|| {
            let exact = subject_fidelity.is_exact() && counterpart_fidelity.is_exact();
            self.shells_intersection(request, (subject, counterpart), disjoint, exact)
        }))
    }

    /// Measures the pair in the request's projection.
    fn projected_interval(
        &self,
        request: &ProximityRequest,
        subject: &Body<'_>,
        counterpart: &Body<'_>,
    ) -> Result<(f64, f64), ProximityError> {
        let subject_fidelity = self.fidelity(request.subject())?;
        let counterpart_fidelity = self.fidelity(request.counterpart())?;
        let deviation = subject_fidelity
            .combined(counterpart_fidelity)
            .deviation_metres();
        let widen = |distance: f64| ((distance - deviation).max(0.0), distance + deviation);
        let boundaries = self.boundaries(request.subject(), request.counterpart())?;
        Ok(match request.projection() {
            ProximityProjection::Minimum3d => {
                self.space_interval(request, separation(&subject.soup, &counterpart.soup)?)?
            }
            ProximityProjection::Horizontal => narrow(
                widen(plan_separation(subject, counterpart)?),
                boundaries.and_then(Boundaries::plan_distance),
            )?,
            ProximityProjection::PlanOverlap => {
                match relation(
                    subject,
                    counterpart,
                    0.0,
                    subject_fidelity,
                    counterpart_fidelity,
                    boundaries,
                )? {
                    Relation::Related => (0.0, 0.0),
                    Relation::Unrelated => (f64::INFINITY, f64::INFINITY),
                    Relation::Open => (0.0, f64::INFINITY),
                }
            }
            ProximityProjection::Vertical {
                footprint_offset_metres,
                direction,
                surfaces:
                    VerticalSurfaces::Between {
                        subject: from,
                        counterpart: to,
                    },
            } => crate::vertical_surface::interval(
                &crate::vertical_surface::Pair {
                    subject,
                    counterpart,
                    subject_fidelity,
                    counterpart_fidelity,
                    boundaries,
                },
                footprint_offset_metres,
                direction,
                from,
                to,
            )?,
            ProximityProjection::Vertical {
                footprint_offset_metres,
                direction,
                surfaces: VerticalSurfaces::Extents,
            } => {
                let side = side(subject, counterpart, direction, deviation);
                if matches!(side, Relation::Unrelated) {
                    return Ok((f64::INFINITY, f64::INFINITY));
                }
                let (lower, upper) = widen(vertical_gap(subject, counterpart, direction));
                match (
                    side,
                    relation(
                        subject,
                        counterpart,
                        footprint_offset_metres,
                        subject_fidelity,
                        counterpart_fidelity,
                        boundaries,
                    )?,
                ) {
                    (_, Relation::Unrelated) => (f64::INFINITY, f64::INFINITY),
                    (Relation::Related, Relation::Related) => (lower, upper),
                    _ => (lower, f64::INFINITY),
                }
            }
        })
    }
}

/// Items with their boxes and a bounding-volume hierarchy over them, so a
/// query touches only the items near it instead of scanning them all.
pub(crate) struct Indexed<T> {
    pub(crate) items: Vec<T>,
    pub(crate) boxes: Vec<Bounds3>,
    index: Bvh<usize>,
    pub(crate) bounds: Bounds3,
}

impl<T> Indexed<T> {
    pub(crate) fn build(items: Vec<T>, boxes: Vec<Bounds3>) -> Result<Self, ProximityError> {
        let mut all = boxes.iter();
        let first = *all.next().ok_or(ProximityError::Unavailable)?;
        let (mut min, mut max) = (first.min(), first.max());
        for bounds in all {
            for axis in 0..3 {
                min[axis] = min[axis].min(bounds.min()[axis]);
                max[axis] = max[axis].max(bounds.max()[axis]);
            }
        }
        let bounds = Bounds3::try_new(min, max)?;
        let index = Bvh::build(
            boxes
                .iter()
                .enumerate()
                .map(|(item, bounds)| SpatialItem::new(item, aabb(bounds))),
        );
        // Audited coordinates are finite, so every box is accepted; a rejected
        // one would be an item no query could ever find.
        if index.rejected_items() != 0 {
            return Err(ProximityError::Unavailable);
        }
        Ok(Self {
            items,
            boxes,
            index,
            bounds,
        })
    }

    /// Indices of the items whose boxes meet `probe`, in index order.
    pub(crate) fn near(&self, probe: &Bounds3) -> Vec<usize> {
        let mut hits = Vec::new();
        self.index.query_aabb(&aabb(probe), &mut hits);
        let mut items: Vec<usize> = hits
            .into_iter()
            .filter_map(|hit| self.index.item(hit).map(|item| item.key))
            .collect();
        items.sort_unstable();
        items
    }
}

/// A body's mesh with its index ([`bodies::IndexedBody`]: its triangles
/// in a hierarchy, whether it is solid, its surface without zero-area
/// triangles), which it reads through.
pub(crate) struct Body<'a> {
    pub(crate) mesh: &'a TriMesh,
    indexed: Arc<bodies::IndexedBody>,
}

impl std::ops::Deref for Body<'_> {
    type Target = bodies::IndexedBody;

    fn deref(&self) -> &bodies::IndexedBody {
        &self.indexed
    }
}

impl Body<'_> {
    /// The body's winding number, taken without its zero-area triangles.
    pub(crate) fn winding(&self) -> Result<Winding<'_>, ProximityError> {
        match &self.surface {
            None => Winding::prepare(self.mesh, 0.0),
            Some(surface) => Winding::prepare(surface.mesh(), surface.omitted_area()),
        }
    }
}

/// Why a body has no volume to measure.
const OPEN_SURFACE: &str = "the body's mesh is an open surface, which encloses no volume";

/// Why the volume kernel refused, in a report's words.
fn volume_refusal(error: axiolid_inspect::OverlapError) -> &'static str {
    use axiolid_inspect::OverlapError;
    match error {
        OverlapError::NonFinite { .. } => "the volume kernel refused a non-finite coordinate",
        OverlapError::NotClosed { .. } => {
            "the volume kernel refused a mesh that is not a closed, consistently wound \
             two-manifold without zero-area triangles (NotClosed)"
        }
        OverlapError::SelfIntersecting { .. } => {
            "the volume kernel refused a self-intersecting mesh (SelfIntersecting)"
        }
        OverlapError::NoVolume { .. } => {
            "the volume kernel could not certify a nonzero volume (NoVolume)"
        }
        _ => "the volume kernel refused the mesh",
    }
}

fn aabb(bounds: &Bounds3) -> Aabb {
    Aabb {
        min: Point3::from_array(bounds.min()),
        max: Point3::from_array(bounds.max()),
    }
}

pub(crate) fn tolerance() -> Result<Tolerance, ProximityError> {
    Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE).map_err(|_| ProximityError::Unavailable)
}

pub(crate) fn triangle_box(triangle: &Triangle) -> Bounds3 {
    let [a, b, c] = *triangle;
    let min = a.min(b).min(c);
    let max = a.max(b).max(c);
    // Coordinates were audited finite, so the box is well-formed.
    Bounds3::try_new(min.to_array(), max.to_array())
        .unwrap_or_else(|_| unreachable!("audited mesh has finite coordinates"))
}

/// Shortest distance between two indexed item sets under `distance`.
///
/// Each item of `first` is measured only against the items of `second`
/// inside its box grown by the best distance so far. Anything outside that box
/// is farther than the best already found, so the skip loses nothing.
pub(crate) fn nearest<T>(
    first: &Indexed<T>,
    second: &Indexed<T>,
    distance: impl Fn(&T, &T) -> Result<f64, ProximityError>,
) -> Result<f64, ProximityError> {
    nearest_pair(first, second, distance).map(|(best, _)| best)
}

/// [`nearest`], with the indices of the first pair of items found at that
/// distance (within [`LINEAR_TOLERANCE`] of it when the sets meet).
pub(crate) fn nearest_pair<T>(
    first: &Indexed<T>,
    second: &Indexed<T>,
    distance: impl Fn(&T, &T) -> Result<f64, ProximityError>,
) -> Result<(f64, (usize, usize)), ProximityError> {
    let mut best = f64::INFINITY;
    let mut pair = (0, 0);
    for (a_index, (a, a_box)) in first.items.iter().zip(&first.boxes).enumerate() {
        if a_box.gap(&second.bounds) >= best {
            continue;
        }
        let probe = if best.is_finite() {
            a_box.expanded(best)
        } else {
            second.bounds
        };
        for index in second.near(&probe) {
            let b_box = &second.boxes[index];
            if a_box.gap(b_box) >= best {
                continue;
            }
            let measured = distance(a, &second.items[index])?;
            if !measured.is_finite() {
                return Err(ProximityError::InvalidMeasurement);
            }
            if measured < best {
                best = measured;
                pair = (a_index, index);
            }
        }
        if best <= LINEAR_TOLERANCE {
            return Ok((0.0, pair));
        }
    }
    if best.is_finite() {
        Ok((best, pair))
    } else {
        Err(ProximityError::Unavailable)
    }
}

/// Shortest distance between two triangle sets.
pub(crate) fn separation(
    first: &Indexed<Triangle>,
    second: &Indexed<Triangle>,
) -> Result<f64, ProximityError> {
    nearest(first, second, |a, b| triangle_pair_distance(*a, *b))
}

/// [`separation`], with the closest pair of points realizing it: a point
/// of each set's surface. `None` for the pair when the nearest triangles
/// include a zero-area one, measured as its edges: the spot is then left
/// out rather than guessed.
pub(crate) fn separation_with_witness(
    first: &Indexed<Triangle>,
    second: &Indexed<Triangle>,
) -> Result<(f64, Option<(Point3, Point3)>), ProximityError> {
    let (distance, (a, b)) = nearest_pair(first, second, |a, b| triangle_pair_distance(*a, *b))?;
    let pair = closest_points_on_triangles(first.items[a], second.items[b])
        .ok()
        .map(|closest| (closest.point_a, closest.point_b))
        .filter(|(a, b)| a.is_finite() && b.is_finite());
    Ok((distance, pair))
}

/// One projected triangle: a triangle in the plan, or a segment when the
/// triangle stands edge-on (a wall face, an open vertical sheet).
#[derive(Clone, Copy)]
enum Flat {
    Triangle(Triangle),
    Segment([Point3; 2]),
}

/// `triangle` projected onto z = 0.
fn flatten(triangle: &Triangle) -> Flat {
    let [a, b, c] = triangle.map(|point| Point3::new(point.x, point.y, 0.0));
    if (b - a).cross(c - a).length_squared() != 0.0 {
        return Flat::Triangle([a, b, c]);
    }
    // Collinear: the projection is the segment between the two points
    // farthest apart.
    [[a, b], [b, c], [c, a]]
        .into_iter()
        .max_by(|[p, q], [r, t]| {
            (*q - *p)
                .length_squared()
                .total_cmp(&(*t - *r).length_squared())
        })
        .map_or(Flat::Segment([a, a]), Flat::Segment)
}

fn flat_box(flat: &Flat) -> Bounds3 {
    match flat {
        Flat::Triangle(triangle) => triangle_box(triangle),
        Flat::Segment([a, b]) => Bounds3::try_new(a.min(*b).to_array(), a.max(*b).to_array())
            .unwrap_or_else(|_| unreachable!("audited mesh has finite coordinates")),
    }
}

fn segment_distance(a: [Point3; 2], b: [Point3; 2]) -> Result<f64, ProximityError> {
    closest_points_on_segments(a, b)
        .map(|pair| pair.distance_squared.sqrt())
        .map_err(|_| ProximityError::Unavailable)
}

/// Plan distance from a segment to a triangle in the same plane: zero when an
/// end lies inside, otherwise the least distance to one of its edges.
fn segment_triangle_distance(
    segment: [Point3; 2],
    triangle: Triangle,
) -> Result<f64, ProximityError> {
    let mut best = f64::INFINITY;
    for point in segment {
        let closest =
            closest_point_on_triangle(point, triangle).map_err(|_| ProximityError::Unavailable)?;
        best = best.min(closest.distance(point));
    }
    let [a, b, c] = triangle;
    for edge in [[a, b], [b, c], [c, a]] {
        best = best.min(segment_distance(segment, edge)?);
    }
    Ok(best)
}

fn flat_distance(first: &Flat, second: &Flat) -> Result<f64, ProximityError> {
    match (first, second) {
        (Flat::Triangle(a), Flat::Triangle(b)) => closest_points_on_triangles(*a, *b)
            .map(|pair| pair.distance_squared.sqrt())
            .map_err(|_| ProximityError::Unavailable),
        (Flat::Segment(a), Flat::Segment(b)) => segment_distance(*a, *b),
        (Flat::Segment(segment), Flat::Triangle(triangle))
        | (Flat::Triangle(triangle), Flat::Segment(segment)) => {
            segment_triangle_distance(*segment, *triangle)
        }
    }
}

/// A body's footprint as indexed projected triangles.
fn footprint(body: &Body<'_>) -> Result<Indexed<Flat>, ProximityError> {
    let flats: Vec<Flat> = body.soup.items.iter().map(flatten).collect();
    let boxes = flats.iter().map(flat_box).collect();
    Indexed::build(flats, boxes)
}

/// A convex plan region as a fan of flat triangles at z = 0.
fn region_flats(region: &ConvexPlanRegion) -> Result<Indexed<Flat>, ProximityError> {
    let ring = region.ring();
    let at = |[x, y]: [f64; 2]| Point3::new(x, y, 0.0);
    let flats: Vec<Flat> = (1..ring.len() - 1)
        .map(|index| flatten(&[at(ring[0]), at(ring[index]), at(ring[index + 1])]))
        .collect();
    let boxes = flats.iter().map(flat_box).collect();
    Indexed::build(flats, boxes)
}

/// Plan distance between two bodies' footprints; zero when they meet.
fn plan_separation(first: &Body<'_>, second: &Body<'_>) -> Result<f64, ProximityError> {
    nearest(&footprint(first)?, &footprint(second)?, flat_distance)
}

/// Gap between the two meshes' vertical extents in `direction`: from the
/// subject's top up to the counterpart's bottom (`Above`), from its bottom
/// down to the counterpart's top (`Below`), or either; zero when they overlap.
fn vertical_gap(subject: &Body<'_>, counterpart: &Body<'_>, direction: VerticalDirection) -> f64 {
    let (a, b) = (subject.soup.bounds, counterpart.soup.bounds);
    let rise = b.min()[2] - a.max()[2];
    let drop = a.min()[2] - b.max()[2];
    match direction {
        VerticalDirection::Either => rise.max(drop),
        VerticalDirection::Above => rise,
        VerticalDirection::Below => drop,
    }
    .max(0.0)
}

/// Whether the counterpart lies in `direction` from the subject: above
/// unless lower at both ends, below unless higher at both ends.
///
/// Each true end may lie up to its body's chord deviation from the mesh end,
/// so an end difference within the combined `deviation` of zero decides
/// nothing: the side is asserted only beyond it and denied only when both
/// ends are beyond it on the other side.
fn side(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    direction: VerticalDirection,
    deviation: f64,
) -> Relation {
    let (a, b) = (subject.soup.bounds, counterpart.soup.bounds);
    // How far the counterpart's top and bottom lie in `direction` past the
    // subject's.
    let ends = match direction {
        VerticalDirection::Either => return Relation::Related,
        VerticalDirection::Above => [b.max()[2] - a.max()[2], b.min()[2] - a.min()[2]],
        VerticalDirection::Below => [a.max()[2] - b.max()[2], a.min()[2] - b.min()[2]],
    };
    if ends.iter().any(|past| past - deviation >= 0.0) {
        Relation::Related
    } else if ends.iter().all(|past| past + deviation < 0.0) {
        Relation::Unrelated
    } else {
        Relation::Open
    }
}

/// The exact boundaries of a tessellated pair whose objects both have one
/// ([`AxiolidGeometry::with_exact_boundary`]). The kernel's certified
/// measurements on them narrow what the chord meshes leave open; each
/// returns `None` (or `Open`) where the kernel refuses, which is no evidence
/// either way.
#[derive(Clone, Copy)]
pub(crate) struct Boundaries<'a> {
    subject: &'a ExactBody,
    counterpart: &'a ExactBody,
}

impl<'a> Boundaries<'a> {
    /// The two bodies when both are one solid in the world, which the
    /// one-solid measurements take; otherwise the body measurements over
    /// their items (`body_*`, axiolid-measure 0.3.8 and 0.3.9).
    fn single(self) -> Option<(&'a axiolid_brep::ExactBRep, &'a axiolid_brep::ExactBRep)> {
        self.subject.single().zip(self.counterpart.single())
    }

    /// How far both bodies' faces may lie from the model's together, by
    /// perturbation and rounding; every certified distance is widened by
    /// it.
    fn widening(self) -> f64 {
        self.subject.widening_metres() + self.counterpart.widening_metres()
    }

    /// Whether either body was built from operands moved within a
    /// tolerance: a shared patch or a gap within that may not be the
    /// model's.
    fn perturbed(self) -> bool {
        !self.subject.is_exact() || !self.counterpart.is_exact()
    }

    /// The certified distance in space between the two boundaries: the
    /// least over the items' boundaries for a body of several
    /// (`body_boundary_distance`, which needs no layout of the items).
    fn separation(self) -> Option<(f64, f64)> {
        self.certified_separation().map(|(interval, _)| interval)
    }

    /// [`Self::separation`], with the kernel's contact witness where the
    /// boundaries are shown to meet: since axiolid-measure 0.3.10 a pair
    /// whose elements touch or cross stops early with a point of each that
    /// they share (axiolid/kernel#273), `[0, d]` with `d` within rounding.
    /// The witness is the midpoint of the two, given only when the
    /// certified lower bound is zero and the points lie within
    /// [`CERTIFIED_ACCURACY_METRES`] of each other.
    fn certified_separation(self) -> Option<((f64, f64), Option<Point3>)> {
        let bounds = match self.single() {
            Some((subject, counterpart)) => boundary_distance(
                subject,
                counterpart,
                CERTIFIED_ACCURACY_METRES,
                Tolerance::METRE,
            )
            .ok()?,
            None => {
                body_boundary_distance(
                    placed(self.subject),
                    placed(self.counterpart),
                    CERTIFIED_ACCURACY_METRES,
                    Tolerance::METRE,
                )
                .ok()?
                .bounds
            }
        };
        let interval = certified(&bounds, self.widening())?;
        let contact = (bounds.lower <= 0.0 && bounds.upper <= CERTIFIED_ACCURACY_METRES)
            .then(|| bounds.point_a.lerp(bounds.point_b, 0.5))
            .filter(|point| point.is_finite());
        Some((interval, contact))
    }

    /// The certified distance between the two boundaries' plan projections,
    /// which are the solids' shadows: zero when they overlap, also when one
    /// stands inside the other's footprint; the least over item pairs for
    /// a body of several (`body_plan_boundary_distance`).
    fn plan_distance(self) -> Option<(f64, f64)> {
        let bounds = match self.single() {
            Some((subject, counterpart)) => plan_boundary_distance(
                subject,
                counterpart,
                CERTIFIED_ACCURACY_METRES,
                Tolerance::METRE,
            )
            .ok()?,
            None => {
                body_plan_boundary_distance(
                    placed(self.subject),
                    placed(self.counterpart),
                    CERTIFIED_ACCURACY_METRES,
                    Tolerance::METRE,
                )
                .ok()?
                .bounds
            }
        };
        certified(&bounds, self.widening())
    }

    /// The certified plan distance, refined only until it clears `limit`.
    fn plan_clearance(self, limit: f64) -> Option<(f64, f64)> {
        let bounds = match self.single() {
            Some((subject, counterpart)) => {
                plan_boundary_clearance(subject, counterpart, limit, Tolerance::METRE)
                    .ok()?
                    .0
            }
            None => {
                body_plan_boundary_clearance(
                    placed(self.subject),
                    placed(self.counterpart),
                    limit,
                    Tolerance::METRE,
                )
                .ok()?
                .0
                .bounds
            }
        };
        certified(&bounds, self.widening())
    }

    /// Whether the shadows overlap with positive area: shown by two planar
    /// faces sharing an open patch in plan, denied by a certified gap wider
    /// than the rounding of coordinates up to `magnitude` and the bodies'
    /// own rounding, open otherwise (shadows that only touch, or overlap
    /// between curved faces alone). Never for a perturbed body: a shared
    /// patch or a gap within its perturbation may not be the model's. A
    /// body of several items is measured over every item pair
    /// (`body_plan_overlap`).
    ///
    /// Overlap is symmetric but the kernel's search is not: axiolid-measure
    /// 0.3.4 showed a column's base over a slab only with the slab first.
    /// So an undecided order is asked again the other way round; either
    /// answer is certified.
    fn plan_overlap(self, magnitude: f64) -> Relation {
        if self.perturbed() {
            return Relation::Open;
        }
        let widening = self.widening();
        let decide = |first: &ExactBody, second: &ExactBody| {
            let verdict = match first.single().zip(second.single()) {
                Some((first, second)) => match plan_overlap(first, second, Tolerance::METRE) {
                    Ok(PlanOverlap::Overlapping { .. }) => Some(None),
                    Ok(PlanOverlap::Disjoint { gap }) => Some(Some(gap)),
                    _ => None,
                },
                None => match body_plan_overlap(placed(first), placed(second), Tolerance::METRE) {
                    Ok(BodyPlanOverlap::Overlapping { .. }) => Some(None),
                    Ok(BodyPlanOverlap::Disjoint { gap }) => Some(Some(gap)),
                    _ => None,
                },
            };
            match verdict {
                Some(None) => Relation::Related,
                Some(Some(gap))
                    if gap.is_finite() && gap > rounding(magnitude.max(gap)) + widening =>
                {
                    Relation::Unrelated
                }
                _ => Relation::Open,
            }
        };
        match decide(self.subject, self.counterpart) {
            Relation::Open => decide(self.counterpart, self.subject),
            decided => decided,
        }
    }
}

/// A body as the kernel's body measurements take it.
fn placed(body: &ExactBody) -> PlacedBody<'_> {
    PlacedBody::new(body.items(), body.placement())
}

/// A bound on the rounding of a distance between points whose coordinates
/// are at most `magnitude`.
fn rounding(magnitude: f64) -> f64 {
    16.0 * f64::EPSILON * magnitude
}

/// A kernel distance interval widened by a bound on the rounding of its
/// witness points and by the bodies' `widening` (perturbation and
/// rounding); `None` when the kernel's interval is not well-formed or the
/// widening unbounded.
fn certified(bounds: &DistanceBounds, widening: f64) -> Option<(f64, f64)> {
    if !bounds.lower.is_finite()
        || !bounds.upper.is_finite()
        || bounds.lower > bounds.upper
        || !widening.is_finite()
    {
        return None;
    }
    // The upper bound is a floating-point distance between two evaluated
    // boundary points; widen both ends by a bound on that rounding.
    let magnitude = [bounds.point_a, bounds.point_b]
        .iter()
        .flat_map(Point3::to_array)
        .fold(bounds.upper, |largest, value| largest.max(value.abs()));
    let margin = rounding(magnitude) + widening;
    Some(((bounds.lower - margin).max(0.0), bounds.upper + margin))
}

/// The chord-widened `measured` interval narrowed by a `certified` one.
/// Both hold the true value, so the result is their intersection; an empty
/// one is refused, since mesh and boundary then describe different bodies.
fn narrow(
    (lower, upper): (f64, f64),
    certified: Option<(f64, f64)>,
) -> Result<(f64, f64), ProximityError> {
    let Some((certified_lower, certified_upper)) = certified else {
        return Ok((lower, upper));
    };
    let (lower, upper) = (lower.max(certified_lower), upper.min(certified_upper));
    if lower > upper {
        return Err(ProximityError::InvalidMeasurement);
    }
    Ok((lower, upper))
}

/// Whether two bodies are related in plan, or whether the geometry's fidelity
/// leaves it open.
pub(crate) enum Relation {
    Related,
    Unrelated,
    Open,
}

/// Whether the footprints overlap with positive area (`offset` zero) or come
/// closer than `offset` (positive).
///
/// Exact footprints are decided. A tessellated footprint may lie anywhere
/// within its chord deviation of the mesh footprint, so a relation is only
/// asserted or denied when the deviations cannot change it.
pub(crate) fn relation(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    offset: f64,
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
    boundaries: Option<Boundaries<'_>>,
) -> Result<Relation, ProximityError> {
    let exact = subject_fidelity.is_exact() && counterpart_fidelity.is_exact();
    let deviation = subject_fidelity.deviation_metres() + counterpart_fidelity.deviation_metres();
    if offset > 0.0 {
        let distance = plan_separation(subject, counterpart)?;
        let (lower, upper) = narrow(
            ((distance - deviation).max(0.0), distance + deviation),
            boundaries.and_then(|pair| pair.plan_clearance(offset)),
        )?;
        return Ok(if upper < offset {
            Relation::Related
        } else if lower >= offset {
            Relation::Unrelated
        } else {
            Relation::Open
        });
    }
    let depth = subject_fidelity
        .deviation_metres()
        .max(counterpart_fidelity.deviation_metres());
    let measured = footprint_overlap(subject, counterpart, exact, deviation, depth)?;
    let Some(pair) = boundaries else {
        return Ok(measured);
    };
    // A bound on the coordinates the kernel measured with: every boundary
    // point lies within the deviation of its mesh.
    let magnitude = [subject.soup.bounds, counterpart.soup.bounds]
        .iter()
        .flat_map(|bounds| bounds.min().into_iter().chain(bounds.max()))
        .fold(0.0_f64, |largest, value| largest.max(value.abs()))
        + deviation;
    match (measured, pair.plan_overlap(magnitude)) {
        (Relation::Related, Relation::Unrelated) | (Relation::Unrelated, Relation::Related) => {
            Err(ProximityError::InvalidMeasurement)
        }
        (Relation::Open, certified) => Ok(certified),
        (measured, _) => Ok(measured),
    }
}

/// Whether the mesh footprints overlap with positive area, decided for
/// exact meshes and only beyond `depth` (the larger chord deviation) or the
/// combined `deviation` for tessellated ones.
fn footprint_overlap(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    exact: bool,
    deviation: f64,
    depth: f64,
) -> Result<Relation, ProximityError> {
    if exact {
        let overlap = footprints_overlap(subject, counterpart)?;
        let area: f64 = overlap.polygons.iter().map(polygon_area).sum();
        // The slivers left out may add up to their area to the overlap. An
        // exact pair answers a point, so one they could tip is refused.
        return if area > OVERLAP_AREA_TOLERANCE {
            Ok(Relation::Related)
        } else if area + overlap.slivers <= OVERLAP_AREA_TOLERANCE {
            Ok(Relation::Unrelated)
        } else {
            Err(ProximityError::Refused(
                "the plan overlap lies within the area of footprint slivers the plan overlay \
                 refuses (RepeatedVertex, ZeroArea)",
            ))
        };
    }
    if plan_separation(subject, counterpart)? > deviation {
        return Ok(Relation::Unrelated);
    }
    // A deep point of the overlap without the slivers is one of the whole
    // overlap: leaving shadows out only shrinks it.
    let overlap = footprints_overlap(subject, counterpart)?.polygons;
    Ok(
        if overlap.iter().any(|polygon| deep_point(polygon, depth)) {
            Relation::Related
        } else {
            Relation::Open
        },
    )
}

/// The pair's plan overlap without the slivers the overlay refuses, or the
/// overlay's own refusal, with its reason.
fn footprints_overlap(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
) -> Result<BoundedOverlap, ProximityError> {
    bounded_plan_overlap(&subject.soup.items, &counterpart.soup.items, tolerance()?)
        .map_err(|error| ProximityError::Refused(error.reason()))
}

/// Whether `polygon` holds a point farther than `depth` from its boundary.
///
/// A witness only: the candidates are the ring's area centroid and the
/// centroids of its fan triangles. Finding none leaves the question open; it
/// never denies depth.
fn deep_point(polygon: &axiolid_overlay::Polygon, depth: f64) -> bool {
    use axiolid_core::Point2;
    let outer = &polygon.outer.points;
    if outer.len() < 3 {
        return false;
    }
    let mut candidates = Vec::new();
    let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for index in 1..outer.len() - 1 {
        let (a, b, c) = (outer[0], outer[index], outer[index + 1]);
        let signed = ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)) / 2.0;
        let centre = Point2::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
        area += signed;
        cx += signed * centre.x;
        cy += signed * centre.y;
        candidates.push(centre);
    }
    if area != 0.0 {
        candidates.insert(0, Point2::new(cx / area, cy / area));
    }
    let rings: Vec<&Vec<Point2>> = std::iter::once(outer)
        .chain(polygon.holes.iter().map(|hole| &hole.points))
        .collect();
    candidates.into_iter().any(|point| {
        let inside = contains(outer, point)
            && !polygon
                .holes
                .iter()
                .any(|hole| contains(&hole.points, point));
        inside
            && rings
                .iter()
                .all(|ring| ring_distance(ring, point) > depth + LINEAR_TOLERANCE)
    })
}

/// Even-odd point-in-ring test.
fn contains(ring: &[axiolid_core::Point2], point: axiolid_core::Point2) -> bool {
    let mut inside = false;
    for index in 0..ring.len() {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        if (a.y > point.y) != (b.y > point.y) {
            let x = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Distance from `point` to the nearest edge of a ring.
fn ring_distance(ring: &[axiolid_core::Point2], point: axiolid_core::Point2) -> f64 {
    let lift = |p: axiolid_core::Point2| Point3::new(p.x, p.y, 0.0);
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            closest_points_on_segments([lift(a), lift(b)], [lift(point), lift(point)])
                .map_or(0.0, |pair| pair.distance_squared.sqrt())
        })
        .fold(f64::INFINITY, f64::min)
}

/// Distance from a point to a triangle set's surface.
///
/// The triangle whose box is nearest bounds the answer from above; only
/// triangles within that bound can improve on it.
pub(crate) fn surface_distance(point: Point3, body: &Body<'_>) -> Result<f64, ProximityError> {
    soup_distance_above(point, &body.soup, 0.0)
}

/// [`surface_distance`] when it exceeds `floor`; otherwise some distance no
/// greater than `floor`, found without the full search.
pub(crate) fn soup_distance_above(
    point: Point3,
    soup: &Indexed<Triangle>,
    floor: f64,
) -> Result<f64, ProximityError> {
    let to = |index: usize| point_triangle_distance(point, soup.items[index]);
    let nearest = soup
        .index
        .nearest_to(&Aabb::from_point(point), |_| true)
        .ok_or(ProximityError::Unavailable)?;
    let mut best = to(nearest.key)?;
    if best <= floor {
        return Ok(best);
    }
    let probe = Bounds3::try_new(point.to_array(), point.to_array())?.expanded(best);
    for index in soup.near(&probe) {
        best = best.min(to(index)?);
    }
    Ok(best)
}

/// Points of `body` at which to test whether it reaches into `other`.
///
/// `surface` points lie on the body's own mesh: its corners, its
/// triangles' centroids and edge midpoints, and the midpoints between an
/// edge's crossings of the other surface. Each is a point of the body
/// whatever its shape. A body lying exactly on another's faces, a duplicate
/// for instance, has every surface point on the other's surface, so a closed
/// body also offers `interior` candidates: its vertex centroid, and the
/// midpoints of the chords its largest triangle's normal cuts through it.
/// Neither need lie in the body (a ring's centroid lies in its hole), so
/// [`deepest_inside`] takes one only after the body's own winding number
/// places it inside, clear of the body's surface.
struct Samples {
    surface: Vec<Point3>,
    interior: Vec<Point3>,
}

fn sample_points(body: &Body<'_>, other: &Body<'_>) -> Result<Samples, ProximityError> {
    let tolerance = tolerance()?;
    let mut surface = Vec::new();
    let mut centroid_sum = Point3::ZERO;
    let mut corners = 0.0;
    for [a, b, c] in &body.soup.items {
        surface.extend([*a, *b, *c, (*a + *b + *c) / 3.0]);
        centroid_sum += *a + *b + *c;
        corners += 3.0;
        for (start, end) in [(*a, *b), (*b, *c), (*c, *a)] {
            surface.push((start + end) / 2.0);
            surface.extend(crossing_midpoints(start, end, other, tolerance)?);
        }
    }
    let mut interior = Vec::new();
    if body.solid && corners > 0.0 {
        interior.push(centroid_sum / corners);
        interior.extend(chord_midpoints(body, tolerance)?);
    }
    Ok(Samples { surface, interior })
}

/// Midpoints of the chords through `body` along its largest triangle's
/// normal, from the triangle's centroid to the nearest other face it meets,
/// one each way: whichever way is inward runs through the body's thickness
/// there. Candidates only, for the caller to place with a winding number.
fn chord_midpoints(body: &Body<'_>, tolerance: Tolerance) -> Result<Vec<Point3>, ProximityError> {
    let area = |[a, b, c]: &Triangle| (*b - *a).cross(*c - *a).length();
    let Some((largest, triangle)) = body.soup.items.iter().enumerate().fold(
        None,
        |best: Option<(usize, &Triangle)>, (index, triangle)| match best {
            Some((_, known)) if area(known) >= area(triangle) => best,
            _ => Some((index, triangle)),
        },
    ) else {
        return Ok(Vec::new());
    };
    let [a, b, c] = *triangle;
    let normal = (b - a).cross(c - a);
    let reach = Point3::from(body.soup.bounds.max()).distance(Point3::from(body.soup.bounds.min()));
    if normal.length_squared() == 0.0 || reach == 0.0 || !reach.is_finite() {
        return Ok(Vec::new());
    }
    let origin = (a + b + c) / 3.0;
    let mut midpoints = Vec::new();
    for sign in [1.0, -1.0] {
        let direction = normal.normalize() * (sign * 2.0 * reach);
        let end = origin + direction;
        let segment = Bounds3::try_new(origin.min(end).to_array(), origin.max(end).to_array())?;
        let ray = Ray3 { origin, direction };
        let mut nearest: Option<f64> = None;
        for index in body.soup.near(&segment) {
            if index == largest {
                continue;
            }
            let hit = segment_hit(&ray, body.soup.items[index], tolerance, index)?;
            if let Some(t) =
                hit.filter(|t| (0.0..=1.0).contains(t) && *t * 2.0 * reach > LINEAR_TOLERANCE)
            {
                nearest = Some(nearest.map_or(t, |known| known.min(t)));
            }
        }
        if let Some(t) = nearest {
            midpoints.push(origin + direction * (t / 2.0));
        }
    }
    Ok(midpoints)
}

/// Midpoints between successive crossings of segment `start..end` with the
/// other body's surface, where a stretch of the edge may lie inside it.
fn crossing_midpoints(
    start: Point3,
    end: Point3,
    other: &Body<'_>,
    tolerance: Tolerance,
) -> Result<Vec<Point3>, ProximityError> {
    let direction = end - start;
    if direction.length_squared() == 0.0 {
        return Ok(Vec::new());
    }
    let segment = Bounds3::try_new(start.min(end).to_array(), start.max(end).to_array())?;
    if segment.gap(&other.soup.bounds) > 0.0 {
        return Ok(Vec::new());
    }
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let mut crossings = vec![0.0, 1.0];
    for index in other.soup.near(&segment) {
        let hit = segment_hit(&ray, other.soup.items[index], tolerance, index)?;
        if let Some(t) = hit.filter(|t| (0.0..=1.0).contains(t)) {
            crossings.push(t);
        }
    }
    if crossings.len() == 2 {
        return Ok(Vec::new());
    }
    crossings.sort_by(f64::total_cmp);
    Ok(crossings
        .windows(2)
        .map(|pair| start + direction * f64::midpoint(pair[0], pair[1]))
        .collect())
}

/// Deepest witnessed point of `body` inside `other`, zero when none is.
///
/// Every witness is a point of `body` (on its surface, or inside it by its
/// own winding number, clear of its surface) lying inside `other` farther
/// than rounding from `other`'s surface. Its depth, its distance to that
/// surface, is therefore a lower bound on how far `body` reaches into
/// `other`; a candidate that is not surely a point of `body` witnesses
/// nothing, however deep it lies in `other`.
fn deepest_inside(
    body: &Body<'_>,
    other: &Body<'_>,
) -> Result<(f64, Option<Point3>), ProximityError> {
    // A point's depth is its distance to the other surface, which the index
    // answers cheaply; whether it is inside at all costs a winding number over
    // every triangle. So rank the candidates by depth and test them deepest
    // first: the first one inside is the deepest witness, and the rest need
    // no winding test. Points outside the other body's box cannot be inside,
    // and points within tolerance of its surface are contact, not depth.
    let samples = sample_points(body, other)?;
    let mut candidates = Vec::new();
    let tagged = samples
        .surface
        .into_iter()
        .map(|point| (point, false))
        .chain(samples.interior.into_iter().map(|point| (point, true)));
    for (point, interior) in tagged {
        let within = (0..3).all(|axis| {
            (other.soup.bounds.min()[axis]..=other.soup.bounds.max()[axis]).contains(&point[axis])
        });
        if !within {
            continue;
        }
        let depth = surface_distance(point, other)?;
        if depth > LINEAR_TOLERANCE {
            candidates.push((depth, point, interior));
        }
    }
    candidates.sort_by(|(a_depth, a, _), (b_depth, b, _)| {
        b_depth.total_cmp(a_depth).then_with(|| {
            a.to_array()
                .partial_cmp(&b.to_array())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let winding = other.winding()?;
    let mut own = None;
    for (depth, point, interior) in candidates {
        if interior {
            // An interior candidate is a point of the body only where the
            // body's own winding number says so, decided clear of its
            // surface; an undecided one is dropped, never assumed inside.
            let margin = surface_distance(point, body)?;
            if margin <= LINEAR_TOLERANCE {
                continue;
            }
            let own = match &mut own {
                Some(own) => own,
                None => own.insert(body.winding()?),
            };
            if own.inside(point, || Ok(margin))? != Some(true) {
                continue;
            }
        }
        // A point too near a left-out zero-area triangle witnesses nothing.
        if winding.inside(point, || Ok(depth))? == Some(true) {
            return Ok((depth, Some(point)));
        }
    }
    // No sample lies inside by more than rounding: the bodies touch.
    Ok((0.0, None))
}

/// The point of `body`'s surface nearest to `point`, among its triangles
/// with area; `None` when a zero-area triangle, measured as its edges, is
/// nearer, so the spot is left out rather than guessed.
fn nearest_surface_point(point: Point3, body: &Body<'_>) -> Result<Option<Point3>, ProximityError> {
    let distance = surface_distance(point, body)?;
    let probe =
        Bounds3::try_new(point.to_array(), point.to_array())?.expanded(distance + LINEAR_TOLERANCE);
    let mut best: Option<(f64, Point3)> = None;
    for index in body.soup.near(&probe) {
        let Ok(closest) = closest_point_on_triangle(point, body.soup.items[index]) else {
            continue;
        };
        let to = closest.distance(point);
        if best.is_none_or(|(least, _)| to < least) {
            best = Some((to, closest));
        }
    }
    Ok(best
        .filter(|(to, closest)| (to - distance).abs() <= LINEAR_TOLERANCE && closest.is_finite())
        .map(|(_, closest)| closest))
}

/// The deepest witnessed point of one body inside the other, as the pair
/// of a penetration witness.
#[derive(Clone, Copy)]
struct Deepest {
    /// Whether the point is the subject's, inside the counterpart.
    into_counterpart: bool,
    point: Point3,
}

/// Whether separated body `inner` lies inside `outer`.
///
/// With the surfaces apart, a body is wholly inside or wholly outside the
/// other, so one vertex decides; it lies at least `separation` from the
/// other surface.
fn contained(inner: &Body<'_>, outer: &Body<'_>, separation: f64) -> Result<bool, ProximityError> {
    outer
        .winding()?
        .decide(inner.soup.items[0][0], || Ok(separation))
}

/// Witnessed penetration and containment between two bodies.
///
/// Only a closed body has an inside, so only points reaching into a closed
/// body are tested. That is still complete when the other body is an open
/// surface: a surface has no volume for anything to reach into, so the
/// surface entering the solid is the whole of their overlap. Two open
/// surfaces share no volume to measure, and report `None`.
fn penetration(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    separation: f64,
) -> Result<(Option<f64>, Option<BodyContainment>), ProximityError> {
    penetration_witnessed(subject, counterpart, separation)
        .map(|(depth, containment, _)| (depth, containment))
}

/// What [`penetration`] measures, with the deepest witnessed point.
type Penetration = (Option<f64>, Option<BodyContainment>, Option<Deepest>);

/// [`penetration`], with the deepest witnessed point where the depth is
/// positive.
fn penetration_witnessed(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    separation: f64,
) -> Result<Penetration, ProximityError> {
    let deepest = |into_counterpart: bool, (depth, point): (f64, Option<Point3>)| {
        (
            depth,
            point.map(|point| Deepest {
                into_counterpart,
                point,
            }),
        )
    };
    if !subject.solid && !counterpart.solid {
        return Ok((None, None, None));
    }
    if separation > 0.0 {
        // Apart at the surface: one body is wholly inside the other or they
        // share nothing. Only a closed body can hold the other.
        if counterpart.solid && contained(subject, counterpart, separation)? {
            let (depth, point) = deepest(true, deepest_inside(subject, counterpart)?);
            return Ok((
                Some(depth),
                Some(BodyContainment::SubjectInsideCounterpart),
                point,
            ));
        }
        if subject.solid && contained(counterpart, subject, separation)? {
            let (depth, point) = deepest(false, deepest_inside(counterpart, subject)?);
            return Ok((
                Some(depth),
                Some(BodyContainment::CounterpartInsideSubject),
                point,
            ));
        }
        return Ok((Some(0.0), None, None));
    }
    let into_counterpart = if counterpart.solid {
        deepest(true, deepest_inside(subject, counterpart)?)
    } else {
        (0.0, None)
    };
    let into_subject = if subject.solid {
        deepest(false, deepest_inside(counterpart, subject)?)
    } else {
        (0.0, None)
    };
    // The deeper side witnesses the depth; a tie goes to the subject's.
    let (depth, point) = if into_subject.0 > into_counterpart.0 {
        into_subject
    } else {
        into_counterpart
    };
    Ok((Some(depth), None, point))
}

/// The certified volume two closed bodies share, with their own volumes,
/// widened for tessellation; `None` when either is not a closed solid or the
/// kernel refuses a mesh.
///
/// `disjoint` bodies (apart at the surface, neither inside the other) share
/// nothing, so their intersection is not integrated.
fn intersection(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    disjoint: bool,
    subject_deviation: f64,
    counterpart_deviation: f64,
) -> Option<IntersectionVolume> {
    if !subject.solid || !counterpart.solid {
        return None;
    }
    let own = |body: &Body<'_>| enclosed_volume(body.mesh).ok();
    let (subject_volume, counterpart_volume) = (own(subject)?, own(counterpart)?);
    let shared = if disjoint {
        axiolid_inspect::VolumeInterval {
            lower: 0.0,
            upper: 0.0,
        }
    } else {
        intersection_volume(subject.mesh, counterpart.mesh).ok()?
    };
    let widened = |volume: axiolid_inspect::VolumeInterval, band: f64| {
        VolumeInterval::try_new((volume.lower - band).max(0.0), volume.upper + band).ok()
    };
    // The shared volume is widened by the declared deviations only: those
    // who read it judge touching against the rounding of bodies that meet.
    IntersectionVolume::try_new(
        widened(
            shared,
            tube_volume(subject, subject_deviation)
                + tube_volume(counterpart, counterpart_deviation),
        )?,
        widened(subject_volume, volume_band(subject, subject_deviation))?,
        widened(
            counterpart_volume,
            volume_band(counterpart, counterpart_deviation),
        )?,
    )
    .ok()
}

/// An upper bound on the volume within `deviation` of the body's mesh.
///
/// A tessellated body's true surface lies within its chord deviation of the
/// mesh, so the true body differs from the mesh's only inside that band,
/// and a volume measured on the mesh is off by no more than the band holds.
/// The band is covered by the neighbourhoods of the triangles, each of
/// volume `2·d·A + (π/2)·P·d² + (4/3)·π·d³` (Steiner's formula for a flat
/// convex set of area `A` and perimeter `P`).
fn tube_volume(body: &Body<'_>, deviation: f64) -> f64 {
    if deviation == 0.0 {
        return 0.0;
    }
    let band: f64 = body
        .soup
        .items
        .iter()
        .map(|[a, b, c]| {
            let area = (*b - *a).cross(*c - *a).length() / 2.0;
            let perimeter = (*b - *a).length() + (*c - *b).length() + (*a - *c).length();
            2.0 * deviation * area
                + std::f64::consts::FRAC_PI_2 * perimeter * deviation * deviation
                + 4.0 / 3.0 * std::f64::consts::PI * deviation.powi(3)
        })
        .sum();
    // The sum is rounded; a relative margin keeps it an upper bound.
    band * (1.0 + 1e-9)
}

/// The band a body's own volume is widened by: the volume within its
/// declared deviation of the mesh ([`tube_volume`]), or for an exact mesh
/// within the rounding of its coordinates.
///
/// An exact mesh is the solid up to that rounding, which the host declares
/// as a deviation wherever it matters beside the solid's size (a body
/// placed in map coordinates, engine#368). The kernel certifies the volume
/// of the mesh as given, so the band of the rounding ([`rounding`] of the
/// largest coordinate) still widens it, and the interval holds the solid's
/// volume wherever the body is placed.
fn volume_band(body: &Body<'_>, deviation: f64) -> f64 {
    if deviation != 0.0 {
        return tube_volume(body, deviation);
    }
    let (min, max) = (body.soup.bounds.min(), body.soup.bounds.max());
    let magnitude = min
        .iter()
        .chain(&max)
        .fold(0.0_f64, |largest, value| largest.max(value.abs()));
    tube_volume(body, rounding(magnitude))
}

/// Every distinct vertex the body's triangles use.
pub(crate) fn vertices(body: &Body<'_>) -> Vec<Point3> {
    let mut points: Vec<Point3> = body.soup.items.iter().flatten().copied().collect();
    points.sort_by(|a, b| {
        a.to_array()
            .partial_cmp(&b.to_array())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    points.dedup_by(|a, b| a.to_array().map(f64::to_bits) == b.to_array().map(f64::to_bits));
    points
}

/// Points where the edges of `body` cross the surface of `other`.
pub(crate) fn crossings(body: &Body<'_>, other: &Body<'_>) -> Result<Vec<Point3>, ProximityError> {
    let tolerance = tolerance()?;
    let mut points = Vec::new();
    for [a, b, c] in &body.soup.items {
        for (start, end) in [(*a, *b), (*b, *c), (*c, *a)] {
            let direction = end - start;
            if direction.length_squared() == 0.0 {
                continue;
            }
            let segment = Bounds3::try_new(start.min(end).to_array(), start.max(end).to_array())?;
            if segment.gap(&other.soup.bounds) > 0.0 {
                continue;
            }
            let ray = Ray3 {
                origin: start,
                direction,
            };
            for index in other.soup.near(&segment) {
                let hit = segment_hit(&ray, other.soup.items[index], tolerance, index)?;
                if let Some(t) = hit.filter(|t| (0.0..=1.0).contains(t)) {
                    points.push(start + direction * t);
                }
            }
        }
    }
    Ok(points)
}

/// The coordinate axes: the directions of [`OverlapExtents`].
const AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A point's position along a unit direction, and a bound on the rounding
/// of that dot product.
///
/// Along a coordinate axis the position is one coordinate, exactly. Along
/// any other direction three products and two sums lie within
/// `3u·Σ|terms|` of the exact dot product (`u = ε/2`); `2ε·Σ|terms|` covers
/// that and the rounding of the bound itself.
fn along(point: Point3, direction: [f64; 3]) -> (f64, f64) {
    let p = point.to_array();
    let terms = [
        p[0] * direction[0],
        p[1] * direction[1],
        p[2] * direction[2],
    ];
    let position = terms[0] + terms[1] + terms[2];
    let on_axis = direction
        .iter()
        .filter(|component| **component != 0.0)
        .count()
        == 1;
    let rounding = if on_axis {
        0.0
    } else {
        2.0 * f64::EPSILON * terms.iter().map(|term| term.abs()).sum::<f64>()
    };
    (position, rounding)
}

/// Witnessed points of the intersection, spanned along each direction.
struct Witnessed<'d> {
    directions: &'d [[f64; 3]],
    spans: Vec<Option<Span>>,
}

/// The positions of witnessed points along one direction.
#[derive(Clone, Copy)]
struct Span {
    least: f64,
    greatest: f64,
    /// The greatest position less its rounding and the least plus its: a
    /// span sure to lie within the true one.
    inner: (f64, f64),
}

impl<'d> Witnessed<'d> {
    fn new(directions: &'d [[f64; 3]]) -> Self {
        Self {
            directions,
            spans: vec![None; directions.len()],
        }
    }
    fn add(&mut self, point: Point3) {
        for (span, direction) in self.spans.iter_mut().zip(self.directions) {
            let (position, rounding) = along(point, *direction);
            let span = span.get_or_insert(Span {
                least: position,
                greatest: position,
                inner: (position - rounding, position + rounding),
            });
            span.least = span.least.min(position);
            span.greatest = span.greatest.max(position);
            span.inner.0 = span.inner.0.max(position - rounding);
            span.inner.1 = span.inner.1.min(position + rounding);
        }
    }
    /// Whether a point at `position` would widen the span on the given side
    /// of direction `index`.
    fn extends(&self, index: usize, upward: bool, position: f64) -> bool {
        match self.spans[index] {
            None => true,
            Some(span) => {
                if upward {
                    position > span.greatest
                } else {
                    position < span.least
                }
            }
        }
    }
    /// The witnessed extent along direction `index`: a lower bound on the
    /// intersection's.
    fn extent(&self, index: usize) -> f64 {
        self.spans[index].map_or(0.0, |span| (span.inner.0 - span.inner.1).max(0.0))
    }
}

/// Widens `witnessed` by the vertices of `body` that lie inside `other`.
///
/// Only a vertex that would widen the span matters, and on each side of each
/// direction the outermost inside vertex is the only one that does. So
/// vertices are tried outermost first and each side stops at its first
/// inside vertex or at the first that would not widen the span: the winding
/// test, linear in the other body's size, runs only where it can change the
/// answer.
fn add_inside_vertices(
    body: &Body<'_>,
    other: &Body<'_>,
    witnessed: &mut Witnessed<'_>,
) -> Result<(), ProximityError> {
    let bounds = other.soup.bounds;
    let points: Vec<Point3> = vertices(body)
        .into_iter()
        .filter(|point| {
            (0..3).all(|axis| (bounds.min()[axis]..=bounds.max()[axis]).contains(&point[axis]))
        })
        .collect();
    if points.is_empty() {
        return Ok(());
    }
    let winding = other.winding()?;
    let mut known: Vec<Option<bool>> = vec![None; points.len()];
    for (index, direction) in witnessed.directions.iter().enumerate() {
        let positions: Vec<f64> = points
            .iter()
            .map(|point| along(*point, *direction).0)
            .collect();
        for upward in [false, true] {
            let mut order: Vec<usize> = (0..points.len()).collect();
            order.sort_by(|&a, &b| {
                let ordering = positions[a].total_cmp(&positions[b]);
                if upward { ordering.reverse() } else { ordering }
            });
            for point in order {
                if !witnessed.extends(index, upward, positions[point]) {
                    break;
                }
                let inside_other = if let Some(answer) = known[point] {
                    answer
                } else {
                    // A vertex too near a left-out zero-area triangle
                    // witnesses nothing.
                    let answer = winding
                        .inside(points[point], || surface_distance(points[point], other))?
                        == Some(true);
                    known[point] = Some(answer);
                    answer
                };
                if inside_other {
                    witnessed.add(points[point]);
                    break;
                }
            }
        }
    }
    Ok(())
}

/// The range of a body's positions along a direction, widened by the
/// rounding of each and by `deviation`, within which the true surface lies.
fn body_range(body: &Body<'_>, direction: [f64; 3], deviation: f64) -> (f64, f64) {
    let (mut least, mut greatest) = (f64::INFINITY, f64::NEG_INFINITY);
    for point in body.soup.items.iter().flatten() {
        let (position, rounding) = along(*point, direction);
        least = least.min(position - rounding);
        greatest = greatest.max(position + rounding);
    }
    (least - deviation, greatest + deviation)
}

/// Extents of the bodies' intersection along each of `directions` (unit
/// vectors), widened for the geometry's fidelity.
///
/// The lower bound spans witnessed points of the intersection; the upper
/// bound is the overlap of the true bodies' ranges along the direction,
/// which the intersection, lying in both, cannot exceed.
fn overlap_along(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    contact: (f64, Option<BodyContainment>),
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
    directions: &[[f64; 3]],
) -> Result<Vec<LengthInterval>, ProximityError> {
    Ok(overlap_spans(
        subject,
        counterpart,
        contact,
        subject_fidelity,
        counterpart_fidelity,
        directions,
    )?
    .into_iter()
    .map(|span| span.extent)
    .collect())
}

/// How far the bodies' intersection reaches along one direction, and
/// where.
#[derive(Clone, Copy)]
struct OverlapSpan {
    /// Its extent, as [`overlap_along`] answers it.
    extent: LengthInterval,
    /// Positions spanned by witnessed points of the intersection, each end
    /// moved inwards by the fidelity's deviation and the rounding: within
    /// the true intersection's span. `None` when nothing was witnessed or
    /// the deviation swallows the span.
    inner: Option<(f64, f64)>,
    /// The overlap of the true bodies' ranges, which holds the
    /// intersection; `None` for bodies sharing nothing.
    outer: Option<(f64, f64)>,
}

/// [`overlap_along`], with where the intersection lies along each
/// direction.
fn overlap_spans(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    (separation, containment): (f64, Option<BodyContainment>),
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
    directions: &[[f64; 3]],
) -> Result<Vec<OverlapSpan>, ProximityError> {
    let mut witnessed = Witnessed::new(directions);
    // Bodies apart at the surface share nothing unless one holds the other.
    let shared = separation <= 0.0 || containment.is_some();
    if shared {
        for point in crossings(subject, counterpart)?
            .into_iter()
            .chain(crossings(counterpart, subject)?)
        {
            witnessed.add(point);
        }
        if counterpart.solid {
            add_inside_vertices(subject, counterpart, &mut witnessed)?;
        }
        if subject.solid {
            add_inside_vertices(counterpart, subject, &mut witnessed)?;
        }
    }
    let deviation = subject_fidelity
        .combined(counterpart_fidelity)
        .deviation_metres();
    directions
        .iter()
        .enumerate()
        .map(|(index, direction)| {
            if !shared {
                return Ok(OverlapSpan {
                    extent: LengthInterval::exact(0.0)
                        .map_err(|_| ProximityError::InvalidMeasurement)?,
                    inner: None,
                    outer: None,
                });
            }
            let a = body_range(subject, *direction, subject_fidelity.deviation_metres());
            let b = body_range(
                counterpart,
                *direction,
                counterpart_fidelity.deviation_metres(),
            );
            let (from, to) = (a.0.max(b.0), a.1.min(b.1));
            let upper = (to - from).max(0.0);
            // Each end of a tessellated extent may move by the deviation.
            let lower = (witnessed.extent(index) - 2.0 * deviation).max(0.0);
            let inner = witnessed.spans[index]
                .map(|span| {
                    (
                        (span.inner.1 + deviation).max(from),
                        (span.inner.0 - deviation).min(to),
                    )
                })
                .filter(|(least, greatest)| least <= greatest);
            Ok(OverlapSpan {
                extent: LengthInterval::try_new(lower.min(upper), upper)
                    .map_err(|_| ProximityError::InvalidMeasurement)?,
                inner,
                outer: (from <= to).then_some((from, to)),
            })
        })
        .collect()
}

/// Extents of the bodies' intersection along each world axis, with the
/// boxes bracketing it in the local frame: the witnessed box within it
/// (`None` when an axis witnessed nothing) and the box holding it (`None`
/// for bodies sharing nothing).
fn overlap_extents(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    contact: (f64, Option<BodyContainment>),
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
) -> Result<(OverlapExtents, OverlapBoxes), ProximityError> {
    let axes = overlap_spans(
        subject,
        counterpart,
        contact,
        subject_fidelity,
        counterpart_fidelity,
        &AXES,
    )?;
    let corners = |span: fn(&OverlapSpan) -> Option<(f64, f64)>| {
        let [x, y, z] = [0, 1, 2].map(|axis| span(&axes[axis]));
        Some(([x?.0, y?.0, z?.0], [x?.1, y?.1, z?.1]))
    };
    let boxes = OverlapBoxes {
        inner: corners(|span| span.inner),
        outer: corners(|span| span.outer),
    };
    Ok((
        OverlapExtents::new(axes[0].extent, axes[1].extent, axes[2].extent),
        boxes,
    ))
}

/// The boxes bracketing two bodies' intersection, `(min, max)` in the
/// local frame.
#[derive(Clone, Copy)]
struct OverlapBoxes {
    inner: Option<([f64; 3], [f64; 3])>,
    outer: Option<([f64; 3], [f64; 3])>,
}

/// Distance from `point` to one triangle.
fn triangle_distance(point: Point3, triangle: Triangle) -> Result<f64, ProximityError> {
    point_triangle_distance(point, triangle)
}

/// An upper bound on how far any point of `triangle` lies from `other`'s
/// surface: the least, over `other`'s triangles, of the farthest vertex
/// distance to that one triangle.
///
/// A bound already no greater than `floor` is returned as soon as it is
/// found: it cannot raise a maximum that has reached `floor`.
fn triangle_bound(triangle: Triangle, other: &Body<'_>, floor: f64) -> Result<f64, ProximityError> {
    let farthest = |target: Triangle| -> Result<f64, ProximityError> {
        let mut worst = 0.0_f64;
        for vertex in triangle {
            worst = worst.max(triangle_distance(vertex, target)?);
        }
        Ok(worst)
    };
    let [a, b, c] = triangle;
    let centre = (a + b + c) / 3.0;
    let first = other
        .soup
        .index
        .nearest_to(&Aabb::from_point(centre), |_| true)
        .ok_or(ProximityError::Unavailable)?;
    let mut best = farthest(other.soup.items[first.key])?;
    if best <= floor {
        return Ok(best);
    }
    // A triangle doing better comes within `best` of every vertex of this
    // one, so its box meets the box around each vertex grown by `best`:
    // it reaches up to the highest vertex less `best` and down to the lowest
    // plus `best` on every axis. Only those are measured.
    let (highest, lowest) = (a.max(b).max(c).to_array(), a.min(b).min(c).to_array());
    for index in other.soup.near(&triangle_box(&triangle).expanded(best)) {
        let candidate = &other.soup.boxes[index];
        let reaches = (0..3).all(|axis| {
            candidate.max()[axis] >= highest[axis] - best
                && candidate.min()[axis] <= lowest[axis] + best
        });
        if reaches {
            best = best.min(farthest(other.soup.items[index])?);
        }
    }
    Ok(best)
}

/// [`triangle_bound`], tightened by splitting the triangle while its bound
/// exceeds `floor`, a distance the answer is already known to reach.
fn refined_bound(
    triangle: Triangle,
    other: &Body<'_>,
    floor: f64,
    depth: u32,
) -> Result<f64, ProximityError> {
    let bound = triangle_bound(triangle, other, floor)?;
    if bound <= floor || depth == 0 {
        return Ok(bound);
    }
    let [a, b, c] = triangle;
    let (ab, bc, ca) = ((a + b) / 2.0, (b + c) / 2.0, (c + a) / 2.0);
    let mut split = 0.0_f64;
    for child in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
        split = split.max(refined_bound(child, other, floor, depth - 1)?);
        if split >= bound {
            return Ok(bound);
        }
    }
    Ok(split)
}

/// The Hausdorff distance between the two meshes' surfaces, as `(lower,
/// upper)`.
fn hausdorff(first: &Body<'_>, second: &Body<'_>) -> Result<(f64, f64), ProximityError> {
    let mut lower = 0.0_f64;
    for (from, to) in [(first, second), (second, first)] {
        for vertex in vertices(from) {
            lower = lower.max(soup_distance_above(vertex, &to.soup, lower)?);
        }
    }
    let mut upper = lower;
    for (from, to) in [(first, second), (second, first)] {
        for triangle in &from.soup.items {
            upper = upper.max(refined_bound(*triangle, to, upper, HAUSDORFF_REFINEMENT)?);
        }
    }
    let snap = |value: f64| {
        if value <= LINEAR_TOLERANCE {
            0.0
        } else {
            value
        }
    };
    Ok((snap(lower), snap(upper)))
}

/// A surface handed over by another service, as the kernel's mesh.
fn counterpart_mesh(surface: &BodySurface) -> TriMesh {
    TriMesh::new(
        surface
            .positions()
            .iter()
            .map(|[x, y, z]| Point3::new(*x, *y, *z))
            .collect(),
        surface.triangles().iter().flatten().copied().collect(),
    )
}

/// One side of the kernel's certified Hausdorff distance as evidence.
fn directed(
    bounds: &axiolid_measure::HausdorffBounds,
) -> Result<axioval_engine::DirectedDistance, ProximityError> {
    let point = |point: Point3| [point.x, point.y, point.z];
    axioval_engine::DirectedDistance::try_new(
        LengthInterval::try_new(bounds.lower.max(0.0), bounds.upper)
            .map_err(|_| ProximityError::InvalidMeasurement)?,
        point(bounds.point_from),
        point(bounds.point_to),
    )
}

/// One side of the kernel's certified boundary Hausdorff distance as
/// evidence, its ends widened by a bound on the rounding of the evaluated
/// boundary points and by the bodies' `widening` (their booleans'
/// rounding), as [`certified`] widens a boundary distance. `None` when the
/// kernel's interval is not well-formed or the widening unbounded.
fn certified_directed(
    bounds: &axiolid_measure::HausdorffBounds,
    widening: f64,
) -> Option<axioval_engine::DirectedDistance> {
    if !bounds.lower.is_finite()
        || !bounds.upper.is_finite()
        || bounds.lower > bounds.upper
        || !widening.is_finite()
    {
        return None;
    }
    let magnitude = [bounds.point_from, bounds.point_to]
        .iter()
        .flat_map(Point3::to_array)
        .fold(bounds.upper, |largest, value| largest.max(value.abs()));
    let margin = rounding(magnitude) + widening;
    let point = |point: Point3| [point.x, point.y, point.z];
    axioval_engine::DirectedDistance::try_new(
        LengthInterval::try_new((bounds.lower - margin).max(0.0), bounds.upper + margin).ok()?,
        point(bounds.point_from),
        point(bounds.point_to),
    )
    .ok()
}

impl ProximityService for AxiolidProximityService {
    fn shares_body(&self, first: &ObjectId, second: &ObjectId) -> bool {
        self.geometry.shares_body(first, second)
    }

    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let body = self.body(object)?;
        ObjectBounds::try_new(object.clone(), body.soup.bounds, self.fidelity(object)?)
    }

    /// The registered mesh as it stands, in the host's world coordinates.
    fn body_surface(&self, object: &ObjectId) -> Result<BodySurface, ProximityError> {
        let body = self.body(object)?;
        let triangles = body
            .mesh
            .indices
            .chunks_exact(3)
            .map(|corners| [corners[0], corners[1], corners[2]])
            .collect();
        BodySurface::try_new(
            object.clone(),
            body.mesh
                .positions
                .iter()
                .map(|point| [point.x, point.y, point.z])
                .collect(),
            triangles,
            self.fidelity(object)?,
        )
        .map(
            |surface| match self.geometry.shared_exact_boundary(object) {
                Some(boundary) => surface.with_exact_boundary(ExactBoundaryHandle::new(boundary)),
                None => surface,
            },
        )
    }

    /// The kernel's certified two-sided Hausdorff distance, between exact
    /// surfaces only. Where the subject and the counterpart both have an
    /// exact boundary, between the boundaries (`one_sided_boundary_hausdorff_with_budget` each way,
    /// [`axioval_engine::SurfaceBasis::ExactBoundary`]) whatever the meshes'
    /// fidelity; otherwise, or where the kernel refuses the boundaries, between the
    /// meshes (`hausdorff_distance`). A tessellation bounds its true
    /// surface's distance from the mesh one way only, so a mesh distance
    /// with either side tessellated refuses rather than widen an
    /// uncertified interval.
    fn measure_surface_distance(
        &self,
        request: &SurfaceDistanceRequest,
    ) -> Result<SurfaceDistanceEvidence, ProximityError> {
        let subject = self.body(request.subject())?;
        if let Some(measured) = self.boundary_surface_distance(request)? {
            return Ok(measured);
        }
        if !self.fidelity(request.subject())?.is_exact()
            || !request.counterpart().fidelity().is_exact()
        {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        let counterpart = counterpart_mesh(request.counterpart());
        let measured = axiolid_measure::hausdorff_distance(
            subject.mesh,
            &counterpart,
            request.accuracy_metres(),
        )
        .map_err(|_| ProximityError::InvalidMeasurement)?;
        SurfaceDistanceEvidence::try_new(
            request.clone(),
            directed(&measured.forward)?,
            directed(&measured.backward)?,
            Evidence {
                source: request.subject().source.clone(),
                locator: format!(
                    "axiolid:hausdorff:{}:{}",
                    request.subject(),
                    request.counterpart().object()
                ),
                exact: true,
                witness: None,
            },
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        if request.projection() != ProximityProjection::Minimum3d {
            return Err(ProximityError::UnsupportedProjection);
        }
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let fidelity = self
            .fidelity(request.subject())?
            .combined(self.fidelity(request.counterpart())?);

        let (separation, closest) = separation_with_witness(&subject.soup, &counterpart.soup)?;
        // No separation, penetration or containment rests on the plan
        // overlap, so an overlay refusing the footprints leaves it unknown
        // instead of the whole pair unmeasured. It costs an overlay per
        // pair, so it is measured only when the request asks for it.
        let plan_overlap = if request.asks_plan_overlap() {
            plan_overlap_area(&subject.soup.items, &counterpart.soup.items, tolerance()?)
        } else {
            None
        };

        let (penetration, containment, deepest) =
            penetration_witnessed(&subject, &counterpart, separation)?;
        let subject_fidelity = self.fidelity(request.subject())?;
        let counterpart_fidelity = self.fidelity(request.counterpart())?;
        let (extents, boxes) = match penetration {
            Some(_) => {
                let (extents, boxes) = overlap_extents(
                    &subject,
                    &counterpart,
                    (separation, containment),
                    subject_fidelity,
                    counterpart_fidelity,
                )?;
                (Some(extents), Some(boxes))
            }
            None => (None, None),
        };
        let witness = self.proximity_witness(
            request,
            (&subject, &counterpart),
            closest,
            deepest.filter(|_| penetration.is_some_and(|depth| depth > 0.0)),
            boxes,
        )?;
        let disjoint = separation > 0.0 && containment.is_none();
        let volume = self.pair_volume(request, (&subject, &counterpart), disjoint)?;
        let deviation = fidelity.deviation_metres();
        let (lower, upper) = hausdorff(&subject, &counterpart)?;
        let hausdorff = LengthInterval::try_new((lower - deviation).max(0.0), upper + deviation)
            .map_err(|_| ProximityError::InvalidMeasurement)?;
        let certified = self
            .boundaries(request.subject(), request.counterpart())?
            .and_then(Boundaries::certified_separation);
        // Where the exact boundaries are shown to meet, the evidence names
        // the point they share (axiolid/kernel#273).
        let contact =
            certified
                .and_then(|(_, contact)| contact)
                .map_or_else(String::new, |point| {
                    // Reported in model coordinates (#372).
                    let [x, y, z] = self
                        .geometry
                        .local_origin(&request.subject().source)
                        .point_to_model([point.x, point.y, point.z]);
                    format!(":contact=({x:.6},{y:.6},{z:.6})")
                });

        let measured = ProximityEvidence::try_new(
            request.clone(),
            separation,
            penetration,
            plan_overlap,
            containment,
            fidelity,
            Evidence {
                source: request.subject().source.clone(),
                locator: format!(
                    "axiolid:proximity:{}:{}{}{contact}",
                    request.subject(),
                    request.counterpart(),
                    self.geometry
                        .body_note(&[request.subject(), request.counterpart()])
                ),
                exact: fidelity.is_exact(),
                witness: None,
            },
        )?;
        let measured = match certified.map(|(interval, _)| interval) {
            Some((lower, upper)) => measured.with_certified_separation(
                LengthInterval::try_new(lower, upper)
                    .map_err(|_| ProximityError::InvalidMeasurement)?,
            )?,
            None => measured,
        };
        let measured = measured.with_hausdorff(hausdorff)?;
        let measured = match extents {
            Some(extents) => measured.with_overlap_extents(extents)?,
            None => measured,
        };
        // A witness the measurement does not support is left out, never
        // reported in its stead.
        let measured = match witness {
            Some(witness) => measured.clone().with_witness(witness).unwrap_or(measured),
            None => measured,
        };
        match volume {
            Some(volume) if penetration.is_some() => measured.with_intersection_volume(volume),
            _ => Ok(measured),
        }
    }

    /// Extents along the request's directions, measured as the world-axis
    /// extents are: witnessed crossings and inside vertices below, the
    /// bodies' own ranges along each direction above. Off the coordinate
    /// axes every position is a rounded dot product, widened by a bound on
    /// its rounding. Two open surfaces share no volume and are refused.
    fn measure_overlap_along(
        &self,
        request: &OverlapAlongRequest,
    ) -> Result<OverlapAlongEvidence, ProximityError> {
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let subject_fidelity = self.fidelity(request.subject())?;
        let counterpart_fidelity = self.fidelity(request.counterpart())?;
        let fidelity = subject_fidelity.combined(counterpart_fidelity);
        let separation = separation(&subject.soup, &counterpart.soup)?;
        let (penetration, containment) = penetration(&subject, &counterpart, separation)?;
        if penetration.is_none() {
            return Err(ProximityError::Unavailable);
        }
        let directions: Vec<[f64; 3]> = request
            .directions()
            .iter()
            .map(MetricDirection::components)
            .collect();
        let extents = overlap_along(
            &subject,
            &counterpart,
            (separation, containment),
            subject_fidelity,
            counterpart_fidelity,
            &directions,
        )?;
        OverlapAlongEvidence::try_new(
            request.clone(),
            extents,
            fidelity,
            Evidence {
                source: request.subject().source.clone(),
                locator: format!(
                    "axiolid:overlap-along:{}:{}",
                    request.subject(),
                    request.counterpart()
                ),
                exact: fidelity.is_exact(),
                witness: None,
            },
        )
    }

    fn measure_face_distance(
        &self,
        request: &FaceDistanceRequest,
    ) -> Result<FaceDistanceEvidence, FaceDistanceError> {
        crate::face_distance::measure(self, request)
    }

    fn measure_height_above(
        &self,
        request: &HeightAboveRequest,
    ) -> Result<HeightAboveEvidence, ProximityError> {
        crate::height_above::measure(self, request)
    }

    fn measure_drainage(
        &self,
        request: &DrainageRequest,
    ) -> Result<DrainageEvidence, ProximityError> {
        crate::drainage::measure(self, request)
    }

    fn measure_body_volume(&self, object: &ObjectId) -> Result<BodyVolume, ProximityError> {
        let fidelity = self.fidelity(object)?;
        let (lower, upper) = self.union_volume(object)?;
        let volume = VolumeInterval::try_new(lower, upper)?;
        BodyVolume::try_new(
            object.clone(),
            volume,
            fidelity,
            Evidence {
                source: object.source.clone(),
                locator: format!(
                    "axiolid:volume:{object}{}",
                    self.geometry.body_note(&[object])
                ),
                exact: fidelity.is_exact(),
                witness: None,
            },
        )
    }

    fn measure_region_distance(
        &self,
        request: &RegionDistanceRequest,
    ) -> Result<RegionDistanceEvidence, ProximityError> {
        let counterpart = request.counterpart();
        let body = self.body(counterpart)?;
        let fidelity = self.fidelity(counterpart)?;
        let distance = nearest(
            &region_flats(request.region())?,
            &footprint(&body)?,
            flat_distance,
        )?;
        let deviation = fidelity.deviation_metres();
        RegionDistanceEvidence::try_new(
            request.clone(),
            (distance - deviation).max(0.0),
            distance + deviation,
            fidelity,
            Evidence {
                source: counterpart.source.clone(),
                locator: format!(
                    "axiolid:distance:region:{}-gon:{counterpart}",
                    request.region().ring().len()
                ),
                exact: fidelity.is_exact(),
                witness: None,
            },
        )
    }

    /// The least distance from the body to the line between its ends'
    /// middles, in space (a closed body holding the line is at zero) or
    /// between the footprint and the line's plan; widened by how far the
    /// ends may lie from their middles and by the chord deviation.
    fn measure_line_distance(
        &self,
        request: &LineDistanceRequest,
    ) -> Result<LineDistanceEvidence, ProximityError> {
        let object = request.object();
        let body = self.body(object)?;
        let fidelity = self.fidelity(object)?;
        let plan = request.projection() == LineProjection::Plan;
        let axes = if plan { 2 } else { 3 };
        let (start, end) = (request.start(), request.end());
        let middle = |point: PointBox| {
            let [x, y, z] =
                [0, 1, 2].map(|axis| f64::midpoint(point.lower()[axis], point.upper()[axis]));
            Point3::new(x, y, if plan { 0.0 } else { z })
        };
        // How far a point of the box may lie from its middle, rounded up.
        let reach = |point: PointBox| {
            let squared: f64 = (0..axes)
                .map(|axis| {
                    let half = (point.upper()[axis] - point.lower()[axis]) / 2.0;
                    half * half
                })
                .sum();
            if squared == 0.0 {
                0.0
            } else {
                (squared.sqrt() * (1.0 + 4.0 * f64::EPSILON)).next_up()
            }
        };
        let (from, to) = (middle(start), middle(end));
        let distance = if plan {
            let line = Flat::Segment([from, to]);
            let boxes = vec![flat_box(&line)];
            nearest(
                &Indexed::build(vec![line], boxes)?,
                &footprint(&body)?,
                flat_distance,
            )?
        } else {
            let line = [from, to, to];
            let boxes = vec![triangle_box(&line)];
            let measured = nearest(&Indexed::build(vec![line], boxes)?, &body.soup, |a, b| {
                triangle_pair_distance(*a, *b)
            })?;
            // A line crossing no face lies wholly inside or outside.
            if measured > 0.0 && body.solid && body.winding()?.decide(from, || Ok(measured))? {
                0.0
            } else {
                measured
            }
        };
        let exact = fidelity.is_exact() && start.is_exact() && end.is_exact();
        let widening = reach(start).max(reach(end)) + fidelity.deviation_metres();
        let (lower, upper) = if widening > 0.0 {
            (
                (distance - widening).next_down().max(0.0),
                (distance + widening).next_up(),
            )
        } else {
            (distance, distance)
        };
        LineDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            Evidence {
                source: object.source.clone(),
                locator: format!(
                    "axiolid:distance:line-{}:{object}{}",
                    request.projection().name(),
                    self.geometry.body_note(&[object])
                ),
                exact,
                witness: None,
            },
        )
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let fidelity = self
            .fidelity(request.subject())?
            .combined(self.fidelity(request.counterpart())?);
        // A distance in space carries its closest pair (#388); no other
        // projection's distance is witnessed by a pair of points in space.
        let ((lower, upper), closest) = match request.projection() {
            ProximityProjection::Minimum3d => {
                let (distance, closest) =
                    separation_with_witness(&subject.soup, &counterpart.soup)?;
                (self.space_interval(request, distance)?, closest)
            }
            _ => (
                self.projected_interval(request, &subject, &counterpart)?,
                None,
            ),
        };
        let witness = closest
            .map(|pair| self.separation_witness(request, pair))
            .transpose()?;
        let measured = ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            Evidence {
                source: request.subject().source.clone(),
                locator: match request.projection() {
                    ProximityProjection::Vertical {
                        direction,
                        surfaces:
                            VerticalSurfaces::Between {
                                subject,
                                counterpart,
                            },
                        ..
                    } => format!(
                        "axiolid:distance:vertical-{}-{}-to-{}:{}:{}",
                        direction.name(),
                        subject.name(),
                        counterpart.name(),
                        request.subject(),
                        request.counterpart()
                    ),
                    ProximityProjection::Vertical {
                        direction: direction @ (VerticalDirection::Above | VerticalDirection::Below),
                        ..
                    } => format!(
                        "axiolid:distance:vertical-{}:{}:{}",
                        direction.name(),
                        request.subject(),
                        request.counterpart()
                    ),
                    projection => format!(
                        "axiolid:distance:{}:{}:{}",
                        projection.name(),
                        request.subject(),
                        request.counterpart()
                    ),
                } + &self
                    .geometry
                    .body_note(&[request.subject(), request.counterpart()]),
                exact: fidelity.is_exact(),
                witness: None,
            },
        )?;
        // A witness the interval does not support is left out.
        Ok(match witness {
            Some(witness) => measured.clone().with_witness(witness).unwrap_or(measured),
            None => measured,
        })
    }
}

#[cfg(test)]
mod tests {

    use axiolid_overlay::OverlayError;
    use axioval_ir::SourceId;

    use super::*;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    /// An overlay refusal reaches the outcome with the overlay's own error,
    /// never as a body that could not be read.
    #[test]
    fn an_overlay_refusal_names_the_overlays_error() {
        for (error, name) in [
            (OverlayError::RepeatedVertex, "RepeatedVertex"),
            (OverlayError::SelfIntersection, "SelfIntersection"),
            (OverlayError::ZeroArea, "ZeroArea"),
        ] {
            let refused = ProximityError::Refused(crate::planar::overlay_refusal(&error));
            let message = refused.to_string();
            assert!(message.contains(name), "{message}");
            assert!(message.contains("plan overlay"), "{message}");
            assert_ne!(refused, ProximityError::Unavailable);
        }
    }

    /// A closed prism around `axis` with `sides` chords and `rings` bands.
    fn column(centre: [f64; 3], radius: f64, height: f64, sides: u32, rings: u32) -> TriMesh {
        let mut positions = Vec::new();
        for ring in 0..=rings {
            let z = centre[2] + height * f64::from(ring) / f64::from(rings);
            for side in 0..sides {
                let angle = std::f64::consts::TAU * f64::from(side) / f64::from(sides);
                positions.push(Point3::new(
                    centre[0] + radius * angle.cos(),
                    centre[1] + radius * angle.sin(),
                    z,
                ));
            }
        }
        let bottom = u32::try_from(positions.len()).unwrap();
        positions.push(Point3::new(centre[0], centre[1], centre[2]));
        positions.push(Point3::new(centre[0], centre[1], centre[2] + height));
        let mut indices = Vec::new();
        for ring in 0..rings {
            for side in 0..sides {
                let next = (side + 1) % sides;
                let (a, b) = (ring * sides + side, ring * sides + next);
                let (c, d) = (a + sides, b + sides);
                indices.extend([a, b, d, a, d, c]);
            }
        }
        for side in 0..sides {
            let next = (side + 1) % sides;
            indices.extend([bottom, next, side]);
            let top = rings * sides;
            indices.extend([bottom + 1, top + side, top + next]);
        }
        TriMesh::new(positions, indices)
    }

    fn brute_separation(first: &Body<'_>, second: &Body<'_>) -> f64 {
        let mut best = f64::INFINITY;
        for a in &first.soup.items {
            for b in &second.soup.items {
                let pair = closest_points_on_triangles(*a, *b).unwrap();
                best = best.min(pair.distance_squared.sqrt());
            }
        }
        best
    }

    /// The index may only skip work, never change the answer.
    #[test]
    fn indexed_separation_matches_the_exhaustive_scan() {
        for (offset, height) in [(0.55, 3.0), (1.3, 2.0), (0.9, 0.5)] {
            let geometry = AxiolidGeometry::new()
                .with_mesh(id("a"), column([0.0, 0.0, 0.0], 0.3, 3.0, 24, 6))
                .with_mesh(id("b"), column([offset, 0.2, 1.0], 0.25, height, 20, 5));
            let service = AxiolidProximityService::new(geometry);
            let (a, b) = (
                service.body(&id("a")).unwrap(),
                service.body(&id("b")).unwrap(),
            );
            let indexed = separation(&a.soup, &b.soup).unwrap();
            let exhaustive = brute_separation(&a, &b);
            let exhaustive = if exhaustive <= LINEAR_TOLERANCE {
                0.0
            } else {
                exhaustive
            };
            assert!(
                (indexed - exhaustive).abs() < 1e-12,
                "offset {offset}: {indexed} vs {exhaustive}"
            );
        }
    }

    #[test]
    fn indexed_surface_distance_matches_the_exhaustive_scan() {
        let geometry =
            AxiolidGeometry::new().with_mesh(id("a"), column([0.0, 0.0, 0.0], 0.3, 3.0, 24, 6));
        let service = AxiolidProximityService::new(geometry);
        let body = service.body(&id("a")).unwrap();
        for point in [
            Point3::new(0.0, 0.0, 1.5),
            Point3::new(0.1, -0.05, 0.2),
            Point3::new(2.0, 1.0, 4.0),
        ] {
            let exhaustive = body
                .soup
                .items
                .iter()
                .map(|t| {
                    closest_point_on_triangle(point, *t)
                        .unwrap()
                        .distance(point)
                })
                .fold(f64::INFINITY, f64::min);
            let indexed = surface_distance(point, &body).unwrap();
            assert!(
                (indexed - exhaustive).abs() < 1e-12,
                "{point}: {indexed} vs {exhaustive}"
            );
        }
    }
}
