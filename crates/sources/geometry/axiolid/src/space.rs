//! Space validation measured from application-supplied geometry.
//!
//! ADR 0004: every method here measures one aspect and fails independently.
//! A model missing slabs still yields clear heights; an unmeasurable space
//! does not suppress the storey residual.
//!
//! Roles (which object is a space, a slab, a roof, a storey) are semantic
//! facts a mesh does not carry, so the host declares them. Inferring a role
//! from geometry alone would present a guess as a measurement. A cap,
//! boundary or overlap request that names its own elements replaces the
//! declared defaults: the rule then says what bounds or intersects a space.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    BoundaryRequest, Cap, CapCoverage, CapRequest, ClearHeightEvidence, Containment,
    OverlapRequest, SpaceAspect, SpaceError, SpaceOverlap, SpaceService, SupportCounts,
    UnallocatedRegion,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, extent_gap, triangles};
use crate::planar::{
    BoundedOverlap, boundary_rings, bounded_footprint, bounded_plan_overlap, footprint_polygons,
    overlay_refusal, plan_frame, polygon_area, ring_segments, snapping_area,
};
use axiolid_core::Point2;
use axiolid_overlay::{FillRule, OverlayError, OverlayInput, OverlayOperation, Polygon, overlay};

/// Areas below this are numerical dust, not a measured overlap.
///
/// Two bodies sharing a hairline sliver in plan have not been shown to
/// intersect; without a floor, projection noise would read as a real clash.
const AREA_EPSILON_M2: f64 = 1.0e-9;

/// Fraction of a body's plan area that must lie inside another to count as
/// containment rather than partial overlap.
///
/// Not 1.0: a contained body whose boundary grazes its container would
/// otherwise be demoted to a partial overlap and reported as a clash.
const CONTAINMENT_RATIO: f64 = 0.999;

/// How far an element may sit from a cap plane and still cap it.
const CAP_PLANE_TOLERANCE_M: f64 = 1.0e-6;

/// How far an unmeasured object's declared bound may lie from a space and
/// still be taken to reach it: the rounding of placing that bound in world
/// coordinates, which the host computes from the source.
const BOUND_MARGIN_M: f64 = 1.0e-6;

/// One storey's geometry while unallocated floor regions are measured.
#[derive(Default)]
struct StoreyBodies {
    /// Non-space bodies contributing floor area.
    floor: Vec<Triangle>,
    /// Spaces that account for part of that floor.
    spaces: Vec<Triangle>,
}

/// What a declared object is, for the purposes of space validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Space,
    Slab,
    Roof,
}

/// Measures space geometry using Axiolid.
pub struct AxiolidSpaceService {
    geometry: AxiolidGeometry,
    source: SourceId,
    roles: BTreeMap<ObjectId, Role>,
    /// Which storey each object belongs to, as the host declares it.
    storeys: BTreeMap<ObjectId, ObjectId>,
    buildings: BTreeSet<ObjectId>,
}

impl AxiolidSpaceService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            roles: BTreeMap::new(),
            storeys: BTreeMap::new(),
            buildings: BTreeSet::new(),
        }
    }

    /// Declares an object to be a space.
    #[must_use]
    pub fn with_space(mut self, space: ObjectId) -> Self {
        self.roles.insert(space, Role::Space);
        self
    }

    /// Declares an object to be a slab.
    #[must_use]
    pub fn with_slab(mut self, slab: ObjectId) -> Self {
        self.roles.insert(slab, Role::Slab);
        self
    }

    /// Declares an object to be a roof.
    #[must_use]
    pub fn with_roof(mut self, roof: ObjectId) -> Self {
        self.roles.insert(roof, Role::Roof);
        self
    }

    /// Assigns an object to a storey.
    #[must_use]
    pub fn with_storey(mut self, object: ObjectId, storey: ObjectId) -> Self {
        self.storeys.insert(object, storey);
        self
    }

    /// Declares a building the model contains.
    #[must_use]
    pub fn with_building(mut self, building: ObjectId) -> Self {
        self.buildings.insert(building);
        self
    }

    /// Whether an unmeasured object is one space validation scans: a
    /// declared space, slab, roof or storey member, or an element a request
    /// names.
    fn concerns(&self, object: &ObjectId, requested: &[ObjectId]) -> bool {
        self.roles.contains_key(object)
            || self.storeys.contains_key(object)
            || requested.binary_search(object).is_ok()
    }

    /// Refuses the model-wide `aspect` while any declared space, slab, roof
    /// or storey member could not be measured: it scans every one of them,
    /// and one missing would change the residuals.
    fn complete(&self, aspect: SpaceAspect) -> Result<(), SpaceError> {
        let blockers: Vec<ObjectId> = self
            .geometry
            .unmeasured()
            .map(|(object, _)| object)
            .filter(|object| self.concerns(object, &[]))
            .cloned()
            .collect();
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(SpaceError::unmeasured(aspect, blockers))
        }
    }

    /// Refuses `aspect` of `space` while an unmeasured object could change
    /// it: a declared role, storey member or `requested` element that
    /// `affects` accepts and that may lie within `reach` of the space (in
    /// plan when `plan` is set; only of its plane `at` a cap, when set).
    ///
    /// Only a bound the host declared
    /// ([`AxiolidGeometry::with_unmeasured_bound`]) places an unmeasured
    /// object; one without may be anywhere, so it refuses every space. The
    /// refusal names every object that blocks this space.
    #[allow(clippy::too_many_arguments)]
    fn complete_near(
        &self,
        aspect: SpaceAspect,
        space: &ObjectId,
        requested: &[ObjectId],
        reach: f64,
        plan: bool,
        at: Option<Cap>,
        affects: impl Fn(&ObjectId) -> bool,
    ) -> Result<(), SpaceError> {
        let extent = self.geometry.enclosing_extent(space).map(|(min, max)| {
            // A cap element must reach the cap's plane: the space's lowest
            // or highest point, as its coverage is measured.
            let plane = match at {
                None => return (min, max),
                Some(Cap::Top) => max[2],
                Some(Cap::Bottom) => min[2],
            };
            ([min[0], min[1], plane], [max[0], max[1], plane])
        });
        let blockers: Vec<ObjectId> = self
            .geometry
            .unmeasured()
            .map(|(object, _)| object)
            .filter(|object| {
                *object != space
                    && self.concerns(object, requested)
                    && affects(object)
                    && match (&extent, self.geometry.unmeasured_bound(object)) {
                        (Some(extent), Some(bound)) => {
                            extent_gap(extent, bound, plan) <= reach + BOUND_MARGIN_M
                        }
                        _ => true,
                    }
            })
            .cloned()
            .collect();
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(SpaceError::unmeasured(aspect, blockers))
        }
    }

    /// The triangles of the space `aspect` measures, refused by name when
    /// the space itself could not be measured.
    fn subject(&self, aspect: SpaceAspect, space: &ObjectId) -> Result<Vec<Triangle>, SpaceError> {
        if self.geometry.is_unmeasured(space) {
            return Err(SpaceError::unmeasured(aspect, vec![space.clone()]));
        }
        self.triangles_of(space)
    }

    /// Whether `candidate` may form `request`'s cap: one of the requested
    /// elements when the request names them, otherwise a declared slab (or,
    /// for the top cap, roof).
    fn caps(&self, request: &CapRequest, candidate: &ObjectId) -> bool {
        if let Some(elements) = request.elements() {
            return elements.binary_search(candidate).is_ok();
        }
        let role = self.role(candidate);
        match request.cap() {
            Cap::Top => matches!(role, Some(Role::Slab | Role::Roof)),
            Cap::Bottom => role == Some(Role::Slab),
        }
    }

    fn role(&self, object: &ObjectId) -> Option<Role> {
        self.roles.get(object).copied()
    }

    fn is_space(&self, object: &ObjectId) -> bool {
        self.role(object) == Some(Role::Space)
    }

    /// Refuses a measurement of `space` that a tessellation could change: the
    /// space itself, or an object `candidate` accepts whose true body could
    /// come within `reach` of it. Space evidence is exact.
    fn require_exact(
        &self,
        space: &ObjectId,
        reach: f64,
        plan: bool,
        candidate: impl Fn(&ObjectId) -> bool,
    ) -> Result<(), SpaceError> {
        let extent = self
            .geometry
            .enclosing_extent(space)
            .ok_or(SpaceError::Unavailable)?;
        if self.geometry.is_tessellated(space)
            || self
                .geometry
                .tessellated_near(&extent, reach, plan, |object| {
                    object == space || !candidate(object)
                })
                .is_some()
        {
            return Err(SpaceError::InexactEvidence);
        }
        Ok(())
    }

    /// Triangles of a declared object, or `Unavailable` when it has no mesh.
    fn triangles_of(&self, object: &ObjectId) -> Result<Vec<Triangle>, SpaceError> {
        let mesh = self.geometry.mesh(object).ok_or(SpaceError::Unavailable)?;
        let triangles = triangles(mesh);
        if triangles.is_empty() {
            return Err(SpaceError::Unavailable);
        }
        Ok(triangles)
    }
}

/// A plan area measured without the slivers the overlay refuses: the true
/// area lies in `[lower, upper]`, and `polygons` hold the measured part.
struct PlanArea {
    lower: f64,
    upper: f64,
    polygons: Vec<Polygon>,
}

impl PlanArea {
    fn of(bounded: BoundedOverlap) -> Self {
        let lower: f64 = bounded.polygons.iter().map(polygon_area).sum();
        Self {
            lower,
            upper: lower + bounded.slivers,
            polygons: bounded.polygons,
        }
    }

    /// Whether the slivers left out could add anything.
    fn uncertain(&self) -> bool {
        self.upper > self.lower
    }
}

/// Why a decision is refused when the slivers left out of the plan
/// overlay could change it.
const SLIVERS_COULD_DECIDE: &str = "the slivers of near-vertical faces the plan overlay refuses \
     (RepeatedVertex, ZeroArea) could change the plan area this decision rests on";

/// Why a reported area is refused when the slivers left out of the plan
/// overlay could move it by more than the overlay's own rounding.
const SLIVERS_EXCEED_ROUNDING: &str = "the slivers of near-vertical faces the plan overlay \
     refuses (RepeatedVertex, ZeroArea) could change the area by more than the overlay's \
     rounding";

/// A plan-overlay refusal that leaving the slivers out did not remove,
/// with the overlay's reason; never a zero area.
fn refused(error: &OverlayError) -> SpaceError {
    SpaceError::Refused(overlay_refusal(error))
}

/// Plan area of a triangle set: the union of its shadows, so overlapping
/// triangles of one body are not counted twice.
fn plan_area(
    triangles: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<PlanArea, SpaceError> {
    bounded_footprint(triangles, tolerance)
        .map(PlanArea::of)
        .map_err(|error| refused(&error))
}

/// Plan area shared by two triangle sets.
fn shared_area(
    first: &[Triangle],
    second: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<PlanArea, SpaceError> {
    bounded_plan_overlap(first, second, tolerance)
        .map(PlanArea::of)
        .map_err(|error| refused(&error))
}

/// Whether `shared` holds at least [`CONTAINMENT_RATIO`] of a positive
/// `area`, for every value the slivers allow; `None` when they could tip
/// it.
fn contains(area: &PlanArea, shared: &PlanArea) -> Option<bool> {
    if area.lower > 0.0 && shared.lower >= area.upper * CONTAINMENT_RATIO {
        Some(true)
    } else if area.upper <= 0.0 || shared.upper < area.lower * CONTAINMENT_RATIO {
        Some(false)
    } else {
        None
    }
}

/// The covered part of a cap whose whole area is `whole`.
///
/// `covered` is the intersection with the subject footprint, so it cannot
/// exceed `whole` but by the overlay's snapping of the two separate
/// arrangements (axiolid/kernel#173): a cap covered over all its area can
/// measure a hair more than the area itself. Within that `rounding` the two
/// are one area; beyond it `CapCoverage::try_new` rejects the incoherent
/// pair, which is where the invariant belongs.
fn within_rounding(whole: f64, covered: f64, rounding: f64) -> f64 {
    if covered > whole && covered - whole <= rounding {
        whole
    } else {
        covered
    }
}

/// Vertical span of a triangle set as `(min_z, max_z)`.
fn vertical_span(triangles: &[Triangle]) -> Option<(f64, f64)> {
    let mut span: Option<(f64, f64)> = None;
    for point in triangles.iter().flatten() {
        span = Some(match span {
            None => (point.z, point.z),
            Some((lo, hi)) => (lo.min(point.z), hi.max(point.z)),
        });
    }
    span
}

/// Height of the vertical overlap between two spans, zero when disjoint.
fn overlapping_height(first: (f64, f64), second: (f64, f64)) -> f64 {
    (first.1.min(second.1) - first.0.max(second.0)).max(0.0)
}

/// How `subject` sits relative to `other`, given their shared plan area.
///
/// Containment is decided on the plan footprint each body actually has: a
/// body almost entirely inside another is contained, not merely overlapping.
/// `None` when the slivers left out of the overlay could change the class.
fn containment(subject: &PlanArea, other: &PlanArea, shared: &PlanArea) -> Option<Containment> {
    if contains(subject, shared)? {
        Some(Containment::SubjectInsideOther)
    } else if contains(other, shared)? {
        Some(Containment::OtherInsideSubject)
    } else {
        Some(Containment::Partial)
    }
}

impl SpaceService for AxiolidSpaceService {
    fn measure_duplicates(&self, space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError> {
        let aspect = SpaceAspect::Duplicates;
        let subject = self.subject(aspect, space)?;
        // Only another space can duplicate it, and only one meeting it.
        self.complete_near(aspect, space, &[], 0.0, false, None, |candidate| {
            self.is_space(candidate)
        })?;
        self.require_exact(space, 0.0, false, |candidate| self.is_space(candidate))?;
        let tolerance = tolerance()?;
        let subject_span = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;
        let mut subject_area = None;

        let mut duplicates = Vec::new();
        for (candidate, mesh) in self.geometry.objects() {
            if candidate == space || !self.is_space(candidate) {
                continue;
            }
            let other = triangles(mesh);
            // Coincident means each body is essentially the other: mutual
            // containment in plan AND the same vertical extent. Plan alone
            // would call a stacked space on the storey above a duplicate.
            let Some(other_span) = vertical_span(&other) else {
                continue;
            };
            let spans_match = (subject_span.0 - other_span.0).abs() < 1.0e-6
                && (subject_span.1 - other_span.1).abs() < 1.0e-6;
            if !spans_match {
                continue;
            }
            let subject_area = match &subject_area {
                Some(area) => area,
                None => subject_area.insert(plan_area(&subject, tolerance)?),
            };
            let other_area = plan_area(&other, tolerance)?;
            let shared = shared_area(&subject, &other, tolerance)?;
            // A pair the slivers left out could make or unmake is refused,
            // never passed.
            let mutual = match (
                contains(subject_area, &shared),
                contains(&other_area, &shared),
            ) {
                (Some(false), _) | (_, Some(false)) => false,
                (Some(true), Some(true)) => true,
                _ => return Err(SpaceError::Refused(SLIVERS_COULD_DECIDE)),
            };
            if mutual {
                duplicates.push(candidate.clone());
            }
        }
        Ok(duplicates)
    }

    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError> {
        // Measured from the space's own body: no other object changes it.
        let subject = self.subject(SpaceAspect::ClearHeight, space)?;
        self.require_exact(space, 0.0, false, |_| false)?;
        let (floor, ceiling) = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;
        ClearHeightEvidence::try_new(
            space.clone(),
            (ceiling - floor).max(0.0),
            Evidence::exact(space.source.clone(), "axiolid:space"),
        )
    }

    fn measure_boundary_gaps(
        &self,
        space: &ObjectId,
        request: &BoundaryRequest,
    ) -> Result<Vec<axioval_engine::BoundaryGap>, SpaceError> {
        let aspect = SpaceAspect::BoundaryGaps;
        let subject = self.subject(aspect, space)?;
        // Coverage is judged in plan alone, so an element on another storey
        // may cover the boundary too.
        self.complete_near(
            aspect,
            space,
            request.elements().unwrap_or_default(),
            0.0,
            true,
            None,
            |candidate| self.bounds(request.elements(), candidate),
        )?;
        // Any bounding footprint touching the boundary in plan may cover it.
        self.require_exact(space, 0.0, true, |candidate| {
            self.bounds(request.elements(), candidate)
        })?;
        let tolerance = tolerance()?;
        // Unioning the triangle soup collapses interior edges, leaving the
        // real perimeter: the shared edge between two triangles of one slab is
        // not boundary, and walking it as such would report phantom gaps.
        let rings = boundary_rings(&subject, tolerance).ok_or(SpaceError::Unavailable)?;

        let mut gaps = Vec::new();
        for ring in &rings {
            let mut uncovered_run = 0.0;
            let mut covering: Vec<ObjectId> = Vec::new();
            for (a, b) in ring_segments(ring) {
                let length = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
                let midpoint = Point2::new(f64::midpoint(a.x, b.x), f64::midpoint(a.y, b.y));
                let coverer = self.covering_element(space, midpoint, request.elements());
                match coverer {
                    Some(element) => {
                        // A covered segment closes the current run. The gap
                        // carries the elements bounding it, so a reviewer can
                        // open the walls the uncovered stretch runs between.
                        if uncovered_run > 0.0 {
                            if !covering.contains(&element) {
                                covering.push(element);
                            }
                            gaps.push(axioval_engine::BoundaryGap::try_new(
                                uncovered_run,
                                std::mem::take(&mut covering),
                            )?);
                            uncovered_run = 0.0;
                        } else if !covering.contains(&element) {
                            // Remember the element preceding a future run.
                            covering.clear();
                            covering.push(element);
                        }
                    }
                    None => uncovered_run += length,
                }
            }
            if uncovered_run > 0.0 {
                gaps.push(axioval_engine::BoundaryGap::try_new(
                    uncovered_run,
                    covering,
                )?);
            }
        }
        Ok(gaps)
    }

    fn measure_overlaps(
        &self,
        space: &ObjectId,
        request: &OverlapRequest,
    ) -> Result<Vec<SpaceOverlap>, SpaceError> {
        let requested = request.elements();
        let aspect = SpaceAspect::Overlaps;
        let subject = self.subject(aspect, space)?;
        let chosen = |candidate: &ObjectId| {
            requested.is_none_or(|elements| elements.binary_search(candidate).is_ok())
        };
        // An overlap needs shared plan area and shared height.
        self.complete_near(
            aspect,
            space,
            requested.unwrap_or_default(),
            0.0,
            false,
            None,
            chosen,
        )?;
        self.require_exact(space, 0.0, false, chosen)?;
        let tolerance = tolerance()?;
        let subject_span = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;
        let mut subject_area = None;

        let mut overlaps = Vec::new();
        for (candidate, mesh) in self.geometry.objects() {
            if candidate == space || !chosen(candidate) {
                continue;
            }
            let other = triangles(mesh);
            let Some(other_span) = vertical_span(&other) else {
                continue;
            };
            // Bodies on different storeys share plan area without touching:
            // the vertical overlap is what makes it a real intersection.
            let height = overlapping_height(subject_span, other_span);
            if height <= 0.0 {
                continue;
            }
            let shared = shared_area(&subject, &other, tolerance)?;
            if shared.upper <= AREA_EPSILON_M2 {
                continue;
            }
            let subject_area = match &subject_area {
                Some(area) => area,
                None => subject_area.insert(plan_area(&subject, tolerance)?),
            };
            let class = containment(subject_area, &plan_area(&other, tolerance)?, &shared)
                .ok_or(SpaceError::Refused(SLIVERS_COULD_DECIDE))?;
            let overlap = |area| {
                SpaceOverlap::try_new(
                    candidate.clone(),
                    self.is_space(candidate),
                    area,
                    height,
                    class,
                )
            };
            // Every reading of the overlap (`intersects` at any height
            // tolerance) must hold at both ends of what the slivers allow.
            let (lower, upper) = (overlap(shared.lower)?, overlap(shared.upper)?);
            let intersects = |overlap: &SpaceOverlap| overlap.intersects(f64::NEG_INFINITY);
            if intersects(&lower) != intersects(&upper) {
                return Err(SpaceError::Refused(SLIVERS_COULD_DECIDE));
            }
            if shared.lower <= AREA_EPSILON_M2 {
                // Dust either way, and never an intersection: nothing
                // reads it.
                if intersects(&upper) {
                    return Err(SpaceError::Refused(SLIVERS_COULD_DECIDE));
                }
                continue;
            }
            if shared.upper - shared.lower > snapping_area(&subject_area.polygons) {
                return Err(SpaceError::Refused(SLIVERS_EXCEED_ROUNDING));
            }
            overlaps.push(lower);
        }
        Ok(overlaps)
    }

    fn measure_cap_coverage(
        &self,
        space: &ObjectId,
        request: &CapRequest,
    ) -> Result<CapCoverage, SpaceError> {
        let cap = request.cap();
        let aspect = SpaceAspect::CapCoverage(cap);
        let subject = self.subject(aspect, space)?;
        // A cap element meets the space's cap plane and its plan footprint.
        self.complete_near(
            aspect,
            space,
            request.elements().unwrap_or_default(),
            CAP_PLANE_TOLERANCE_M,
            false,
            Some(cap),
            |candidate| self.caps(request, candidate),
        )?;
        self.require_exact(space, CAP_PLANE_TOLERANCE_M, false, |candidate| {
            self.caps(request, candidate)
        })?;
        let tolerance = tolerance()?;
        let whole = plan_area(&subject, tolerance)?;
        // A footprint measured empty is an incoherent quantity; one only the
        // slivers left out could give area is not measured at all.
        if whole.lower <= 0.0 && whole.uncertain() {
            return Err(SpaceError::Refused(SLIVERS_COULD_DECIDE));
        }
        let (floor, ceiling) = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;

        let mut covering = Vec::new();
        let mut cover = Vec::new();
        for (candidate, mesh) in self.geometry.objects() {
            // Only cap elements cap a space; a wall crossing the ceiling
            // plane is not a cap. The space never caps itself.
            if candidate == space || !self.caps(request, candidate) {
                continue;
            }
            let other = triangles(mesh);
            let Some((low, high)) = vertical_span(&other) else {
                continue;
            };
            // The element must actually sit at the cap it is claimed to cover.
            let plane = match cap {
                Cap::Top => ceiling,
                Cap::Bottom => floor,
            };
            if plane < low - CAP_PLANE_TOLERANCE_M || plane > high + CAP_PLANE_TOLERANCE_M {
                continue;
            }
            let shared = shared_area(&subject, &other, tolerance)?;
            if shared.upper <= AREA_EPSILON_M2 {
                continue;
            }
            // An element the slivers alone could make cover counts towards
            // the covered area, which bounds it either way; it is cited only
            // where it surely covers more than dust.
            if shared.lower > AREA_EPSILON_M2 {
                covering.push(candidate.clone());
            }
            cover.extend(other);
        }

        // The cover is one footprint under the non-zero rule: two slabs
        // meeting over the space count their shared edge once, never twice.
        let covered = shared_area(&subject, &cover, tolerance)?;
        let rounding = snapping_area(&whole.polygons);
        if whole.upper - whole.lower > rounding || covered.upper - covered.lower > rounding {
            return Err(SpaceError::Refused(SLIVERS_EXCEED_ROUNDING));
        }
        let covered = within_rounding(whole.lower, covered.lower, rounding);
        CapCoverage::try_new(whole.lower, covered, covering)
    }

    fn measure_unallocated_regions(&self) -> Result<Vec<UnallocatedRegion>, SpaceError> {
        self.complete(SpaceAspect::UnallocatedRegions)?;
        let tolerance = tolerance()?;
        // Regions are cut from every storey-assigned body, so any tessellated
        // one makes them estimates.
        if self
            .storeys
            .keys()
            .any(|object| self.geometry.is_tessellated(object))
        {
            return Err(SpaceError::InexactEvidence);
        }
        let mut per_storey: BTreeMap<ObjectId, StoreyBodies> = BTreeMap::new();
        for (object, storey) in &self.storeys {
            let Some(mesh) = self.geometry.mesh(object) else {
                continue;
            };
            let entry = per_storey.entry(storey.clone()).or_default();
            let body = triangles(mesh);
            if self.is_space(object) {
                entry.spaces.extend(body);
            } else {
                entry.floor.extend(body);
            }
        }

        let mut regions = Vec::new();
        for (storey, bodies) in per_storey {
            if bodies.floor.is_empty() {
                continue;
            }
            // Each polygon of the floor less the spaces is one connected
            // region: judged apart, a hole is not hidden among small shafts.
            let floor =
                footprint_polygons(&bodies.floor, tolerance).ok_or(SpaceError::Unavailable)?;
            let spaces =
                footprint_polygons(&bodies.spaces, tolerance).ok_or(SpaceError::Unavailable)?;
            // The storey's gross floor area, each region's share of it.
            let gross: f64 = floor.iter().map(polygon_area).sum();
            let left = if spaces.is_empty() || floor.is_empty() {
                floor
            } else {
                overlay(
                    &OverlayInput {
                        frame: plan_frame(),
                        polygons: floor,
                    },
                    &OverlayInput {
                        frame: plan_frame(),
                        polygons: spaces,
                    },
                    OverlayOperation::Difference,
                    FillRule::NonZero,
                    tolerance,
                )
                .map_err(|error| refused(&error))?
                .polygons
            };
            for region in left {
                let area = polygon_area(&region);
                if area <= AREA_EPSILON_M2 {
                    continue;
                }
                regions.push(
                    UnallocatedRegion::try_new(storey.clone(), area, self.surrounding(&region))?
                        .with_floor_area(gross)?,
                );
            }
        }
        Ok(regions)
    }

    /// Counts the declared roles, measured or not: an unmeasured slab is
    /// still a slab, and no body is read.
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError> {
        let mut slabs = 0usize;
        let mut roofs = 0usize;
        for role in self.roles.values() {
            match role {
                Role::Slab => slabs += 1,
                Role::Roof => roofs += 1,
                Role::Space => {}
            }
        }
        Ok(SupportCounts::new(
            slabs,
            roofs,
            self.buildings.iter().cloned().collect(),
        ))
    }

    fn evidence(&self) -> Evidence {
        Evidence::exact(self.source.clone(), "axiolid:space")
    }
}

impl AxiolidSpaceService {
    /// Whether `candidate` may cover a space boundary: one of `elements` when
    /// a request names them, otherwise any body that is not a space.
    fn bounds(&self, elements: Option<&[ObjectId]>, candidate: &ObjectId) -> bool {
        match elements {
            Some(elements) => elements.binary_search(candidate).is_ok(),
            None => !self.is_space(candidate),
        }
    }

    /// The element covering a point on the space boundary, if any.
    ///
    /// "Covered" means a bounding object other than the space itself has plan
    /// footprint at that point: a wall standing on the boundary covers it, and
    /// an unbounded stretch has nothing there.
    fn covering_element(
        &self,
        space: &ObjectId,
        point: Point2,
        elements: Option<&[ObjectId]>,
    ) -> Option<ObjectId> {
        for (candidate, mesh) in self.geometry.objects() {
            if candidate == space || !self.bounds(elements, candidate) {
                continue;
            }
            let body = triangles(mesh);
            if point_in_footprint(&body, point) {
                return Some(candidate.clone());
            }
        }
        None
    }
}

impl AxiolidSpaceService {
    /// The bodies whose footprint meets `region`'s boundary: the floor it
    /// lies in, and the spaces and elements around it. Sampled at every
    /// vertex and edge midpoint of its rings, so a reviewer can open what
    /// encloses the unallocated area.
    fn surrounding(&self, region: &Polygon) -> Vec<ObjectId> {
        let samples: Vec<Point2> = std::iter::once(&region.outer)
            .chain(&region.holes)
            .flat_map(ring_segments)
            .flat_map(|(a, b)| {
                [
                    a,
                    Point2::new(f64::midpoint(a.x, b.x), f64::midpoint(a.y, b.y)),
                ]
            })
            .collect();
        self.geometry
            .objects()
            .filter(|(_, mesh)| {
                let body = triangles(mesh);
                samples
                    .iter()
                    .any(|point| point_in_footprint(&body, *point))
            })
            .map(|(object, _)| object.clone())
            .collect()
    }
}

/// Whether a plan point lies within a triangle set's projected footprint.
fn point_in_footprint(triangles: &[Triangle], point: Point2) -> bool {
    triangles.iter().any(|[a, b, c]| {
        let sign = |p: Point2, q: (f64, f64), r: (f64, f64)| {
            (p.x - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.y - r.1)
        };
        let d1 = sign(point, (a.x, a.y), (b.x, b.y));
        let d2 = sign(point, (b.x, b.y), (c.x, c.y));
        let d3 = sign(point, (c.x, c.y), (a.x, a.y));
        let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(has_negative && has_positive)
    })
}

/// The audit tolerance every measurement here shares.
fn tolerance() -> Result<axiolid_core::Tolerance, SpaceError> {
    axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).map_err(|_| SpaceError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(lower: f64, upper: f64) -> PlanArea {
        PlanArea {
            lower,
            upper,
            polygons: Vec::new(),
        }
    }

    /// Containment is decided only where every area the slivers allow
    /// agrees; between, it is undecided, so a duplicate is refused rather
    /// than passed.
    #[test]
    fn slivers_that_could_tip_containment_leave_it_undecided() {
        let whole = area(16.0, 16.0);
        assert_eq!(contains(&whole, &area(16.0, 16.0)), Some(true));
        assert_eq!(contains(&whole, &area(8.0, 8.0)), Some(false));
        // The slivers could lift the shared area over the ratio.
        assert_eq!(contains(&whole, &area(15.9, 16.0)), None);
        // Or the whole beyond it.
        assert_eq!(contains(&area(15.99, 16.1), &area(15.99, 15.99)), None);
        // An area only the slivers give is no footprint to contain.
        assert_eq!(contains(&area(0.0, 1e-9), &area(0.0, 1e-9)), None);
        assert_eq!(contains(&area(0.0, 0.0), &area(0.0, 0.0)), Some(false));
        assert_eq!(
            containment(&whole, &area(32.0, 32.0), &area(15.9, 16.0)),
            None
        );
        assert_eq!(
            containment(&whole, &area(32.0, 32.0), &area(4.0, 4.0)),
            Some(Containment::Partial)
        );
    }

    /// A cap covered over all its area may measure a hair above it from the
    /// overlay's snapping; within that rounding it is the whole area, and
    /// beyond it the incoherence is left for the evidence to reject.
    #[test]
    fn a_covered_area_past_the_whole_by_rounding_is_the_whole() {
        // As measured on a real space, the cover's arrangement snapped apart.
        let (whole, covered) = (0.499_999_999_759_893_4, 0.499_999_999_885_403_67);
        assert!(CapCoverage::try_new(whole, covered, Vec::new()).is_err());
        let kept = within_rounding(whole, covered, 1e-7);
        assert!((kept - whole).abs() < f64::MIN_POSITIVE);
        assert!(CapCoverage::try_new(whole, kept, Vec::new()).is_ok());
        assert!((within_rounding(whole, 0.25, 1e-7) - 0.25).abs() < f64::MIN_POSITIVE);
        assert!((within_rounding(whole, 0.6, 1e-7) - 0.6).abs() < f64::MIN_POSITIVE);
    }

    /// A refusal names the overlay and its error, never the bare
    /// "unavailable" a missing body gives.
    #[test]
    fn an_overlay_refusal_names_the_overlays_error() {
        let message = refused(&OverlayError::SelfIntersection).to_string();
        assert!(message.contains("plan overlay"), "{message}");
        assert!(message.contains("SelfIntersection"), "{message}");
        assert_ne!(
            refused(&OverlayError::RepeatedVertex),
            SpaceError::Unavailable
        );
        let message = SpaceError::Refused(SLIVERS_COULD_DECIDE).to_string();
        assert!(message.contains("RepeatedVertex"), "{message}");
    }
}
