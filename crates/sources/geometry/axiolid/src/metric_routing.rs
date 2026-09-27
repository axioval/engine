//! Metric routes over walkable surfaces on one level.
//!
//! ADR 0004: this module measures how long a route is and whether one
//! exists; whether that length is acceptable is a capability's decision.
//!
//! # What is measured
//!
//! The host declares the walkable surfaces (spaces), the portals (doors and
//! openings) and the vertical connectors. Every other object with a body is an
//! obstacle where it enters the band between the mobility profile's maximum
//! step and its clear height above a floor, so a threshold lower than a step
//! is stepped over and a ceiling above the head is not in the way.
//!
//! A route stays on the origin's **level**: the surfaces reachable from it
//! through portals and shared boundaries whose floors differ by no more than
//! the maximum step. Lengths are measured in plan.
//!
//! # Verdicts available with `axiolid-overlay` 0.3.0
//!
//! - **Reachable**: a path proposed through the free region eroded from
//!   inside, accepted only once the body's sweep along
//!   it is proven inside the free region with exact booleans, and only if
//!   every door it crosses admits the body (a bodiless opening does; a door
//!   only through a stated clear width). Its length is the upper bound.
//! - **Lower bound**: the exact visibility-graph shortest path for a point
//!   through the level's free region, with every portal too narrow for the
//!   body cut at its mid-line. A body's centre can only move where a point
//!   can, so no route is shorter. Without complete evidence the bound falls
//!   back to the straight line between the points.
//! - **Blocked**, with complete evidence, when that point path does not exist
//!   (the destination is off the level, or free space and narrow portals
//!   separate the points) and no vertical connector touches the level.
//! - **Refused** otherwise. In particular, a gap between obstacles narrower
//!   than the body inside a room cannot be proven blocking until the kernel
//!   publishes one-sided erosion (`Region::erode_inner`, axiolid-overlay
//!   0.3.1); today such a route refuses rather than reports blocked.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use axiolid_core::Point2;
use axiolid_mesh::TriMesh;
use axioval_engine::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, LengthInterval, MetricPoint,
    MetricRouteEvidence, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingService,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::AxiolidGeometry;
use crate::walkable::{
    Clearance, Floor, MARGIN, ON_SURFACE, Plan, PortalFacts, PortalFrame, REACH, ROUTE_BUDGET,
    Side, contains, corridor, crosses_mid_line, floor, join, mid_line, mid_line_barriers,
    obstacles, obstruction, plan_gap, polygon, separated, sides, subtract, touching, witness,
};

/// Relative allowance for the rounding of a summed polyline length.
const LENGTH_ROUNDING: f64 = 1e-12;

/// Metric routes over host-declared walkable surfaces and portals.
pub struct AxiolidMetricRoutingService {
    geometry: AxiolidGeometry,
    source: SourceId,
    surfaces: BTreeSet<ObjectId>,
    portals: BTreeSet<ObjectId>,
    connectors: BTreeSet<ObjectId>,
    facts: PortalFacts,
    prepared: OnceLock<Prepared>,
}

/// What the declarations yield, independent of any request.
struct Prepared {
    floors: Vec<Floor>,
    /// Declared surfaces, portals and connectors that could not be measured.
    gaps: Vec<String>,
    portals: Vec<(PortalFrame, [Option<Side>; 2], Clearance)>,
    touching: Vec<(usize, usize)>,
    /// Connector extents, for detours off the level.
    connectors: Vec<(ObjectId, crate::geometry::Extent)>,
}

impl AxiolidMetricRoutingService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            surfaces: BTreeSet::new(),
            portals: BTreeSet::new(),
            connectors: BTreeSet::new(),
            facts: PortalFacts::default(),
            prepared: OnceLock::new(),
        }
    }

    /// Declares a walkable surface: a closed body standing on its floor.
    #[must_use]
    pub fn with_surface(mut self, surface: ObjectId) -> Self {
        self.surfaces.insert(surface);
        self
    }

    /// Declares a door or opening a route may pass through. Its body is not
    /// an obstacle.
    #[must_use]
    pub fn with_portal(mut self, portal: ObjectId) -> Self {
        self.portals.insert(portal);
        self
    }

    /// Declares a bodiless opening and the exact planar shape of its void.
    #[must_use]
    pub fn with_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.portals.insert(opening.clone());
        self.facts = self.facts.with_void(opening, mesh, true);
        self
    }

    /// Declares an opening whose void is a tessellation; it is not measured.
    #[must_use]
    pub fn with_tessellated_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.portals.insert(opening.clone());
        self.facts = self.facts.with_void(opening, mesh, false);
        self
    }

    /// Declares an opening whose void could not be meshed.
    #[must_use]
    pub fn with_unmeasured_opening_void(
        mut self,
        opening: ObjectId,
        reason: impl Into<String>,
    ) -> Self {
        self.portals.insert(opening.clone());
        self.facts = self.facts.with_unmeasured_void(opening, reason.into());
        self
    }

    /// States the clear width, in metres, a door's leaf and lining leave.
    #[must_use]
    pub fn with_clear_width(mut self, portal: ObjectId, metres: f64) -> Self {
        self.facts = self.facts.with_clear_width(portal, metres);
        self
    }

    /// Declares a vertical connector (a lift, stair or ramp). Routes do not
    /// climb it, but a level it touches is not closed, so no route leaving
    /// that level is reported blocked.
    #[must_use]
    pub fn with_connector(mut self, connector: ObjectId) -> Self {
        self.connectors.insert(connector);
        self
    }

    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| self.prepare())
    }

    fn prepare(&self) -> Prepared {
        let mut gaps = Vec::new();
        let mut floors = Vec::new();
        for surface in &self.surfaces {
            match floor(&self.geometry, surface) {
                Ok(found) => floors.push(found),
                Err(reason) => gaps.push(reason),
            }
        }
        let mut portals = Vec::new();
        for portal in &self.portals {
            let measured = self.facts.frame(&self.geometry, portal).and_then(|frame| {
                let sides = sides(&frame, &floors)?;
                Ok((frame, sides, self.facts.clearance(portal)?))
            });
            match measured {
                Ok(found) => portals.push(found),
                Err(reason) => gaps.push(reason),
            }
        }
        let mut pairs = Vec::new();
        for a in 0..floors.len() {
            for b in a + 1..floors.len() {
                let (first, second) = (&floors[a], &floors[b]);
                if first.z0 >= second.top - ON_SURFACE || second.z0 >= first.top - ON_SURFACE {
                    continue;
                }
                match touching(&first.footprint, &second.footprint) {
                    Ok(true) => pairs.push((a, b)),
                    Ok(false) => {}
                    Err(reason) => gaps.push(reason),
                }
            }
        }
        let mut connectors = Vec::new();
        for connector in &self.connectors {
            match self.geometry.enclosing_extent(connector) {
                Some(extent) => connectors.push((connector.clone(), extent)),
                None => gaps.push(format!(
                    "vertical connector {connector} has no measured body"
                )),
            }
        }
        Prepared {
            floors,
            gaps,
            portals,
            touching: pairs,
            connectors,
        }
    }

    fn evidence(&self, locator: String) -> Evidence {
        Evidence::exact(self.source.clone(), locator)
    }

    /// The floor holding a point: its plan inside the footprint, its height
    /// between the floor (less a step) and the surface's top.
    fn locate(prepared: &Prepared, point: &MetricPoint, step: f64) -> Result<usize, String> {
        let [x, y, z] = point.coordinates_metres();
        let holding: Vec<usize> = prepared
            .floors
            .iter()
            .enumerate()
            .filter(|(_, floor)| {
                z >= floor.z0 - step - ON_SURFACE
                    && z <= floor.top + ON_SURFACE
                    && contains(&floor.footprint, Point2::new(x, y))
            })
            .map(|(index, _)| index)
            .collect();
        match holding.as_slice() {
            [single] => Ok(*single),
            [] if prepared.gaps.is_empty() => {
                Err(format!("{} lies on no walkable surface", point.subject()))
            }
            [] => Err(format!(
                "{} lies on no measured walkable surface, and some were not measured: {}",
                point.subject(),
                prepared.gaps.join("; ")
            )),
            _ => Err(format!(
                "{} lies on more than one walkable surface",
                point.subject()
            )),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn measure(&self, request: &MetricRouteRequest) -> Result<MetricRouteOutcome, String> {
        let profile = request.profile();
        let radius = profile.radius_metres();
        let step = profile.maximum_step_metres();
        let height = profile.height_metres();
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        let prepared = self.prepared();
        // Every body obstructs unless it is walked on or through.
        let others: Vec<&ObjectId> = self
            .geometry
            .objects()
            .map(|(id, _)| id)
            .filter(|id| !self.surfaces.contains(*id) && !self.portals.contains(*id))
            .collect();
        if let Some((id, reason)) = self
            .geometry
            .unmeasured()
            .find(|(id, _)| !self.surfaces.contains(*id) && !self.portals.contains(*id))
        {
            return Err(format!(
                "{id} has a body that was not measured, so free space is unknown: {reason}"
            ));
        }
        let obstacles = obstacles(&self.geometry, others)?;

        let origin = Self::locate(prepared, request.origin(), step)?;
        let destination = Self::locate(prepared, request.destination(), step)?;

        // The origin's level.
        let floors = &prepared.floors;
        let mut level = BTreeSet::from([origin]);
        let mut frontier = vec![origin];
        let mut crossings: Vec<usize> = Vec::new();
        while let Some(current) = frontier.pop() {
            let mut next = Vec::new();
            // A portal is walked through only when its sill is within a step
            // of both floors: a window opening is no door.
            for (index, (frame, sides, _)) in prepared.portals.iter().enumerate() {
                if let [Some(a), Some(b)] = sides
                    && (a.floor == current || b.floor == current)
                    && (frame.z0 - floors[a.floor].z0).abs() <= step + ON_SURFACE
                    && (frame.z0 - floors[b.floor].z0).abs() <= step + ON_SURFACE
                {
                    next.extend([a.floor, b.floor]);
                    if !crossings.contains(&index) {
                        crossings.push(index);
                    }
                }
            }
            for (a, b) in &prepared.touching {
                if (*a == current || *b == current)
                    && (floors[*a].z0 - floors[*b].z0).abs() <= step + ON_SURFACE
                {
                    next.extend([*a, *b]);
                }
            }
            for floor in next {
                if level.insert(floor) {
                    frontier.push(floor);
                }
            }
        }
        crossings.sort_unstable();
        for (a, b) in &prepared.touching {
            if level.contains(a)
                && level.contains(b)
                && (floors[*a].z0 - floors[*b].z0).abs() > step + ON_SURFACE
            {
                return Err(format!(
                    "{} and {} share a boundary with a step higher than the profile allows, \
                     and both are on the route's level",
                    floors[*a].id, floors[*b].id
                ));
            }
        }
        let detours: Vec<&ObjectId> = prepared
            .connectors
            .iter()
            .filter(|(_, extent)| {
                level.iter().any(|index| {
                    let floor = &floors[*index];
                    floor.z0 >= extent.0[2] - REACH
                        && floor.z0 <= extent.1[2] + REACH
                        && plan_gap(&floor.bounds, extent) <= REACH + ON_SURFACE
                })
            })
            .map(|(id, _)| id)
            .collect();
        let mut incomplete: Vec<String> = prepared.gaps.clone();
        incomplete.extend(
            detours
                .iter()
                .map(|id| format!("vertical connector {id} leaves the level")),
        );
        let complete = incomplete.is_empty();

        let level_text = level
            .iter()
            .map(|index| floors[*index].id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let blocked = |reason: &str| -> Result<MetricRouteOutcome, String> {
            let completeness = CompleteMetricEvidence::try_new(self.evidence(format!(
                "axiolid:metric-route:blocked:{reason}:level=[{level_text}]:surfaces={}:\
                 portals={}:obstacles={}:connectors={}:radius={radius:.6}:step={step:.6}:\
                 height={height:.6}",
                self.surfaces.len(),
                self.portals.len(),
                obstacles.len(),
                self.connectors.len(),
            )))
            .map_err(|e| e.to_string())?;
            Ok(MetricRouteOutcome::Blocked(
                BlockedMetricRouteEvidence::new(request.clone(), completeness),
            ))
        };
        if !level.contains(&destination) {
            if complete {
                return blocked("destination-off-level");
            }
            return Err(format!(
                "the destination is off the origin's level and the level is not closed: {}",
                incomplete.join("; ")
            ));
        }

        // The level's free region.
        let mut domain = Plan::empty();
        for index in &level {
            let floor = &floors[*index];
            let blocked = obstruction(
                &obstacles,
                &floor.bounds,
                floor.z0 + step,
                floor.z0 + height,
            )?;
            domain = join(&domain, &subtract(&floor.footprint, &blocked)?)?;
        }
        for index in &crossings {
            let (frame, sides, _) = &prepared.portals[*index];
            let reach = frame.half + REACH;
            let corners = [
                frame.point(frame.u0, -reach),
                frame.point(frame.u1, reach),
                frame.point(frame.u0, reach),
                frame.point(frame.u1, -reach),
            ];
            let bounds = corners.iter().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(min, max), p| {
                    (
                        [min[0].min(p.x), min[1].min(p.y)],
                        [max[0].max(p.x), max[1].max(p.y)],
                    )
                },
            );
            let blocked = obstruction(&obstacles, &bounds, frame.z0 + step, frame.z0 + height)?;
            domain = join(&domain, &corridor(frame, sides, &blocked)?)?;
        }

        // Portals too narrow for the body. Their mid-line is a barrier for the
        // point path (no body crosses it). Where the corridor's own chord
        // rules the body out, a band around the mid-line holds no body centre
        // at all: a centre at distance `t` from the line covers a chord of
        // `2 sqrt(r^2 - t^2)` on it, longer than the free interval while
        // `t < sqrt(r^2 - (L/2)^2)`. Cutting half that band out of the free
        // region leaves every body path inside what remains.
        let body = 2.0 * radius;
        let mut barriers: Vec<Vec<Point2>> = Vec::new();
        let mut cuts = Vec::new();
        let mut narrow = Vec::new();
        for index in &crossings {
            let (frame, _, clearance) = &prepared.portals[*index];
            let intervals = mid_line(frame, &[&domain]);
            let longest = intervals
                .iter()
                .map(|(a, b, _, _)| b - a)
                .fold(0.0, f64::max);
            let chord = longest + 2.0 * MARGIN;
            let stated = match clearance {
                Clearance::Stated(stated) => *stated,
                _ => f64::INFINITY,
            };
            if chord.min(stated) >= body {
                continue;
            }
            narrow.push(frame.id.to_string());
            if chord >= body {
                // Only the stated clear width rules the body out, and it
                // speaks for the door's own extent alone.
                barriers.push(vec![frame.point(frame.u0, 0.0), frame.point(frame.u1, 0.0)]);
            } else {
                barriers.extend(mid_line_barriers(frame, &[&domain]));
                let half_band = 0.5 * (radius * radius - (longest * 0.5).powi(2)).sqrt();
                if half_band > MARGIN {
                    cuts.extend(intervals.iter().filter_map(|(a, b, before, after)| {
                        let start = a - (before / 3.0).min(MARGIN);
                        let end = b + (after / 3.0).min(MARGIN);
                        polygon(vec![
                            frame.point(start, -half_band),
                            frame.point(end, -half_band),
                            frame.point(end, half_band),
                            frame.point(start, half_band),
                        ])
                    }));
                }
            }
        }

        let [ox, oy, _] = request.origin().coordinates_metres();
        let [dx, dy, _] = request.destination().coordinates_metres();
        let (from, to) = (Point2::new(ox, oy), Point2::new(dx, dy));
        if complete && separated(&domain, cuts, from, to)? {
            return blocked(&format!("separated:narrow-portals=[{}]", narrow.join(",")));
        }
        // The kernel's visibility graph may accept an edge running along
        // collinear boundary edges across a gap outside the region, so its
        // path can be too short, never too long: a lower bound, and a
        // proposal at most.
        let straight = (to - from).length();
        let mut lower = straight;
        let mut point_bound = false;
        if complete {
            match axiolid_route::shortest_path_within(
                domain.polygons(),
                &barriers,
                from,
                to,
                ROUTE_BUDGET,
            ) {
                Ok(Ok(route)) => {
                    lower = lower.max(route.length * (1.0 - LENGTH_ROUNDING));
                    point_bound = true;
                }
                Ok(Err(
                    axiolid_route::Unreachable::StartOutside
                    | axiolid_route::Unreachable::GoalOutside,
                )) => {
                    return Err("an end point is not in free walkable space".into());
                }
                Err(axiolid_route::RouteError::TooManyVertices { lower_bound, .. }) => {
                    lower = lower.max(lower_bound);
                }
                Ok(Err(_)) | Err(_) => {}
            }
        }

        let path = witness(&domain, from, to, radius)?;
        let Some(path) = path else {
            let open = if complete {
                String::new()
            } else {
                format!("; the level is not closed: {}", incomplete.join("; "))
            };
            return Err(format!(
                "no route wide enough for the body could be proven, and a blocking gap cannot \
                 be proven without one-sided erosion; shortest distance is at least \
                 {lower:.6}{open}"
            ));
        };
        for pair in path.windows(2) {
            for (frame, _, clearance) in &prepared.portals {
                if crosses_mid_line(frame, pair[0], pair[1]) && !clearance.admits(body) {
                    return Err(format!(
                        "the route crosses {} whose clear width ({}) does not admit the body",
                        frame.id,
                        clearance.label()
                    ));
                }
            }
        }

        let length: f64 = path
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .sum();
        let upper = length * (1.0 + LENGTH_ROUNDING) + f64::EPSILON;
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        let (waypoints, traversed) = Self::describe(request, &path, &level, prepared)?;
        let route = MetricRouteEvidence::try_new(
            distance,
            waypoints,
            traversed,
            Evidence::exact(
                request.origin().subject().source.clone(),
                format!(
                    "axiolid:metric-route:reachable:level=[{level_text}]:radius={radius:.6}:\
                 step={step:.6}:height={height:.6}:sweep=proven:lower={}:upper={upper:.6}",
                    if point_bound {
                        format!("point-path={lower:.6}")
                    } else {
                        format!("straight-line={lower:.6}")
                    }
                ),
            ),
        )
        .map_err(|e| e.to_string())?;
        Ok(MetricRouteOutcome::Reachable(route))
    }

    /// The route's waypoints, each grounded on the surface or portal it lies
    /// in, and the objects it traverses in order.
    fn describe(
        request: &MetricRouteRequest,
        path: &[Point2],
        level: &BTreeSet<usize>,
        prepared: &Prepared,
    ) -> Result<(Vec<MetricPoint>, Vec<ObjectId>), String> {
        let holder = |point: Point2| {
            level
                .iter()
                .map(|index| &prepared.floors[*index])
                .find(|floor| contains(&floor.footprint, point))
        };
        let mut waypoints = vec![request.origin().clone()];
        for point in path.iter().skip(1).take(path.len().saturating_sub(2)) {
            let floor = holder(*point).unwrap_or(&prepared.floors[*level.first().unwrap_or(&0)]);
            waypoints.push(
                MetricPoint::try_new(floor.id.clone(), [point.x, point.y, floor.z0])
                    .map_err(|e| e.to_string())?,
            );
        }
        waypoints.push(request.destination().clone());
        let mut traversed: Vec<ObjectId> = Vec::new();
        let mut push = |id: &ObjectId| {
            if traversed.last() != Some(id) {
                traversed.push(id.clone());
            }
        };
        for pair in path.windows(2) {
            if let Some(floor) = holder(pair[0]) {
                push(&floor.id);
            }
            for (frame, _, _) in &prepared.portals {
                if crosses_mid_line(frame, pair[0], pair[1]) {
                    push(&frame.id);
                }
            }
            if let Some(floor) = holder(pair[1]) {
                push(&floor.id);
            }
        }
        if traversed.is_empty() {
            traversed.push(request.origin().subject().clone());
        }
        Ok((waypoints, traversed))
    }
}

impl MetricRoutingService for AxiolidMetricRoutingService {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        self.measure(request)
            .map_err(MetricRoutingError::Unavailable)
    }
}
