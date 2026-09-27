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
    OverlapRequest, SpaceError, SpaceOverlap, SpaceService, SupportCounts, UnallocatedRegion,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{
    boundary_rings, footprint_polygons, plan_frame, polygon_area, projected_polygons, ring_segments,
};
use axiolid_core::Point2;
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, overlay};

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

    /// Refuses while a declared space, slab, roof or storey member could
    /// not be measured: every measurement here scans those objects, and one
    /// missing would change clear heights, overlaps, coverage or residuals.
    fn complete(&self) -> Result<(), SpaceError> {
        if self
            .roles
            .keys()
            .chain(self.storeys.keys())
            .any(|object| self.geometry.is_unmeasured(object))
        {
            return Err(SpaceError::Unavailable);
        }
        Ok(())
    }

    /// Refuses as [`Self::complete`] does, and also while an element a
    /// request names could not be measured: it may be the one that caps.
    fn complete_with(&self, requested: &[ObjectId]) -> Result<(), SpaceError> {
        self.complete()?;
        if requested
            .iter()
            .any(|object| self.geometry.is_unmeasured(object))
        {
            return Err(SpaceError::Unavailable);
        }
        Ok(())
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

/// Plan area of a triangle set.
fn plan_area(triangles: &[Triangle], tolerance: axiolid_core::Tolerance) -> f64 {
    let polygons = projected_polygons(triangles);
    if polygons.is_empty() {
        return 0.0;
    }
    // Union first: overlapping triangles of one body must not be counted twice.
    let input = OverlayInput {
        frame: plan_frame(),
        polygons,
    };
    overlay(
        &input,
        &input,
        OverlayOperation::Union,
        FillRule::NonZero,
        tolerance,
    )
    .map(|r| r.polygons.iter().map(polygon_area).sum())
    .unwrap_or(0.0)
}

/// Plan area shared by two triangle sets.
fn shared_area(
    first: &[Triangle],
    second: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<f64, SpaceError> {
    let (a, b) = (projected_polygons(first), projected_polygons(second));
    if a.is_empty() || b.is_empty() {
        return Ok(0.0);
    }
    let frame = plan_frame();
    let result = overlay(
        &OverlayInput { frame, polygons: a },
        &OverlayInput { frame, polygons: b },
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|_| SpaceError::Unavailable)?;
    Ok(result.polygons.iter().map(polygon_area).sum())
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
fn containment(subject_area: f64, other_area: f64, shared: f64) -> Containment {
    if subject_area > 0.0 && shared >= subject_area * CONTAINMENT_RATIO {
        Containment::SubjectInsideOther
    } else if other_area > 0.0 && shared >= other_area * CONTAINMENT_RATIO {
        Containment::OtherInsideSubject
    } else {
        Containment::Partial
    }
}

impl SpaceService for AxiolidSpaceService {
    fn measure_duplicates(&self, space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError> {
        self.complete()?;
        let subject = self.triangles_of(space)?;
        self.require_exact(space, 0.0, false, |candidate| self.is_space(candidate))?;
        let tolerance = tolerance()?;
        let subject_area = plan_area(&subject, tolerance);
        let subject_span = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;

        let mut duplicates = Vec::new();
        for (candidate, mesh) in self.geometry.objects() {
            if candidate == space || !self.is_space(candidate) {
                continue;
            }
            let other = triangles(mesh);
            let other_area = plan_area(&other, tolerance);
            let shared = shared_area(&subject, &other, tolerance)?;
            // Coincident means each body is essentially the other: mutual
            // containment in plan AND the same vertical extent. Plan alone
            // would call a stacked space on the storey above a duplicate.
            let mutual = subject_area > 0.0
                && other_area > 0.0
                && shared >= subject_area * CONTAINMENT_RATIO
                && shared >= other_area * CONTAINMENT_RATIO;
            let Some(other_span) = vertical_span(&other) else {
                continue;
            };
            let spans_match = (subject_span.0 - other_span.0).abs() < 1.0e-6
                && (subject_span.1 - other_span.1).abs() < 1.0e-6;
            if mutual && spans_match {
                duplicates.push(candidate.clone());
            }
        }
        Ok(duplicates)
    }

    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError> {
        self.complete()?;
        let subject = self.triangles_of(space)?;
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
        self.complete_with(request.elements().unwrap_or_default())?;
        let subject = self.triangles_of(space)?;
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
        self.complete_with(requested.unwrap_or_default())?;
        let subject = self.triangles_of(space)?;
        let chosen = |candidate: &ObjectId| {
            requested.is_none_or(|elements| elements.binary_search(candidate).is_ok())
        };
        self.require_exact(space, 0.0, false, chosen)?;
        let tolerance = tolerance()?;
        let subject_area = plan_area(&subject, tolerance);
        let subject_span = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;

        let mut overlaps = Vec::new();
        for (candidate, mesh) in self.geometry.objects() {
            if candidate == space || !chosen(candidate) {
                continue;
            }
            let other = triangles(mesh);
            let shared = shared_area(&subject, &other, tolerance)?;
            if shared <= AREA_EPSILON_M2 {
                continue;
            }
            let Some(other_span) = vertical_span(&other) else {
                continue;
            };
            // Bodies on different storeys share plan area without touching:
            // the vertical overlap is what makes it a real intersection.
            let height = overlapping_height(subject_span, other_span);
            if height <= 0.0 {
                continue;
            }
            overlaps.push(SpaceOverlap::try_new(
                candidate.clone(),
                self.is_space(candidate),
                shared,
                height,
                containment(subject_area, plan_area(&other, tolerance), shared),
            )?);
        }
        Ok(overlaps)
    }

    fn measure_cap_coverage(
        &self,
        space: &ObjectId,
        request: &CapRequest,
    ) -> Result<CapCoverage, SpaceError> {
        self.complete_with(request.elements().unwrap_or_default())?;
        let cap = request.cap();
        let subject = self.triangles_of(space)?;
        self.require_exact(space, CAP_PLANE_TOLERANCE_M, false, |candidate| {
            self.caps(request, candidate)
        })?;
        let tolerance = tolerance()?;
        let whole = plan_area(&subject, tolerance);
        if whole <= 0.0 {
            return Err(SpaceError::InvalidQuantity);
        }
        let (floor, ceiling) = vertical_span(&subject).ok_or(SpaceError::Unavailable)?;

        let mut covering = Vec::new();
        let mut covered_polygons = Vec::new();
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
            if shared_area(&subject, &other, tolerance)? <= AREA_EPSILON_M2 {
                continue;
            }
            covering.push(candidate.clone());
            covered_polygons.extend(projected_polygons(&other));
        }

        // Union the covering elements before intersecting: two slabs meeting
        // over the space would otherwise double-count their shared edge and
        // report more coverage than the cap has area.
        let frame = plan_frame();
        let covered = if covered_polygons.is_empty() {
            0.0
        } else {
            let union = OverlayInput {
                frame,
                polygons: covered_polygons,
            };
            let merged = overlay(
                &union,
                &union,
                OverlayOperation::Union,
                FillRule::NonZero,
                tolerance,
            )
            .map_err(|_| SpaceError::Unavailable)?;
            let subject_input = OverlayInput {
                frame,
                polygons: projected_polygons(&subject),
            };
            overlay(
                &subject_input,
                &OverlayInput {
                    frame,
                    polygons: merged.polygons,
                },
                OverlayOperation::Intersection,
                FillRule::NonZero,
                tolerance,
            )
            .map_err(|_| SpaceError::Unavailable)?
            .polygons
            .iter()
            .map(polygon_area)
            .sum()
        };
        // No clamp: `covered` is already the intersection with the subject
        // footprint, so it cannot exceed `whole`. CapCoverage::try_new rejects
        // an incoherent pair if that ever stops holding, which is where the
        // invariant belongs -- clamping here would silently repair a real bug.
        CapCoverage::try_new(whole, covered, covering)
    }

    fn measure_unallocated_regions(&self) -> Result<Vec<UnallocatedRegion>, SpaceError> {
        self.complete()?;
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
                .map_err(|_| SpaceError::Unavailable)?
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

    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError> {
        self.complete()?;
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
