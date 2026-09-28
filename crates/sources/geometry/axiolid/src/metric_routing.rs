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
//! the maximum step. Lengths are measured in plan. A request carrying
//! connectors climbs through them instead, across levels (see `climb`).
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
//!
//! # Many targets
//!
//! Nearest-target and farthest-point queries build one `axiolid-route`
//! distance map over the level's free region, from every target placed on
//! the level at once. A target may stand in a portal as well as on a
//! surface, and the level then also holds the corridor of every portal
//! opening from it onto nothing else of it (an exit to the outside). A
//! target the service cannot place counts only by its straight-line
//! distance, which no route beats; one placed off a closed level is
//! unreachable.
//!
//! - **Nearest target**: the map's point distance is the lower bound; for a
//!   point body it is also the upper bound, since the kernel's path
//!   (axiolid/kernel#187, #189) stays in the closed free region. For a body
//!   with a radius the upper bound is a proven sweep to one target, as for a
//!   single route. Unreachable needs complete evidence and every placed
//!   target separated from the origin, as `Blocked` does.
//! - **Farthest point**: `axiolid-route`'s bracket of the largest map
//!   distance over the region's free part, measured for a point only (a
//!   body's positions need one-sided erosion). The upper bound holds
//!   whatever is missing, because missing free space only lengthens routes;
//!   the lower bound is the bracket's only on a closed level with every
//!   target placed, and otherwise the witness's straight-line distance to
//!   the nearest target. A part of the region no target reaches is
//!   reported, with a point of it, only on a closed level.

mod climb;
mod weighted;

use std::collections::BTreeSet;
use std::sync::OnceLock;

use axiolid_core::Point2;
use axiolid_mesh::TriMesh;
use axiolid_route::{FarthestError, MapError, Unreachable};
use axioval_engine::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, FarthestPointEvidence,
    FarthestPointOutcome, FarthestPointRequest, ForcedWalkOutcome, ForcedWalkRequest,
    LengthInterval, MetricPoint, MetricRouteEvidence, MetricRouteOutcome, MetricRouteRequest,
    MetricRoutingError, MetricRoutingService, MobilityProfile, NearestTargetEvidence,
    NearestTargetOutcome, NearestTargetRequest, PathTrace, PathTraceRequest,
    UnreachableRegionEvidence, UnreachableTargetsEvidence,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, triangles};
use crate::planar::projected_polygons;
use crate::walkable::{
    Clearance, Floor, MARGIN, ON_SURFACE, Obstacle, Plan, PortalFacts, PortalFrame, REACH,
    ROUTE_BUDGET, Side, body, contains, corridor, crosses_mid_line, floor, intersect, join,
    mid_line, mid_line_barriers, obstacles, obstruction, plan_gap, polygon, segment_cover,
    separated, sides, subtract, touching, trapezoids, union, witness,
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

/// Where a point stands: on a surface, or inside a portal's thickness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Floor(usize),
    Portal(usize),
}

/// The surfaces a route from some start may use, and how they join.
struct Level {
    floors: BTreeSet<usize>,
    /// Portals joining two of the level's floors.
    crossings: Vec<usize>,
    /// Portals opening from one of the level's floors onto no other floor
    /// of it; only many-target queries walk into them.
    ends: Vec<usize>,
    /// Why the level may not be closed; empty when it is.
    incomplete: Vec<String>,
    /// The level's surfaces, for evidence.
    text: String,
}

impl Level {
    fn complete(&self) -> bool {
        self.incomplete.is_empty()
    }

    /// Whether a place lies in the level's free region (with its ends).
    fn holds(&self, place: Place) -> bool {
        match place {
            Place::Floor(index) => self.floors.contains(&index),
            Place::Portal(index) => self.crossings.contains(&index) || self.ends.contains(&index),
        }
    }
}

/// Portals too narrow for a body: mid-line barriers for a point path, and
/// bands no body centre enters.
struct Narrow {
    barriers: Vec<Vec<Point2>>,
    cuts: Vec<axiolid_overlay::Polygon>,
    names: Vec<String>,
}

/// How each target of a many-target query stands against the level.
struct Targets {
    /// Indices of the targets placed on the level.
    placed: Vec<usize>,
    /// Targets bounded only by a straight line, with why.
    straight: Vec<(usize, String)>,
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

    fn completeness(&self, locator: String) -> Result<CompleteMetricEvidence, String> {
        CompleteMetricEvidence::try_new(self.evidence(locator)).map_err(|e| e.to_string())
    }

    /// Every body that obstructs: all but the surfaces and portals. An
    /// unmeasured one leaves free space unknown.
    fn obstacles(&self) -> Result<Vec<Obstacle<'_>>, String> {
        if let Some((id, reason)) = self
            .geometry
            .unmeasured()
            .find(|(id, _)| !self.surfaces.contains(*id) && !self.portals.contains(*id))
        {
            return Err(format!(
                "{id} has a body that was not measured, so free space is unknown: {reason}"
            ));
        }
        obstacles(
            &self.geometry,
            self.geometry
                .objects()
                .map(|(id, _)| id)
                .filter(|id| !self.surfaces.contains(*id) && !self.portals.contains(*id)),
        )
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

    /// Where a point stands: on one surface, or else inside the thickness
    /// of one portal that opens onto a surface, no lower than a step below
    /// its sill.
    fn place(prepared: &Prepared, point: &MetricPoint, step: f64) -> Result<Place, String> {
        let on_surface = match Self::locate(prepared, point, step) {
            Ok(index) => return Ok(Place::Floor(index)),
            Err(reason) => reason,
        };
        let [x, y, z] = point.coordinates_metres();
        let at = Point2::new(x, y);
        let inside: Vec<usize> = prepared
            .portals
            .iter()
            .enumerate()
            .filter(|(_, (frame, sides, _))| {
                let relative = at - frame.centre;
                let (along, across) = (relative.dot(frame.along), relative.dot(frame.normal));
                sides.iter().any(Option::is_some)
                    && along >= frame.u0 - ON_SURFACE
                    && along <= frame.u1 + ON_SURFACE
                    && across.abs() <= frame.half + ON_SURFACE
                    && z >= frame.z0 - step - ON_SURFACE
                    && z <= frame.z1 + ON_SURFACE
            })
            .map(|(index, _)| index)
            .collect();
        match inside.as_slice() {
            [single] => Ok(Place::Portal(*single)),
            [] => Err(on_surface),
            _ => Err(format!("{} lies in more than one portal", point.subject())),
        }
    }

    /// The floors a place starts a level from.
    fn seeds(prepared: &Prepared, place: Place, step: f64) -> BTreeSet<usize> {
        match place {
            Place::Floor(index) => BTreeSet::from([index]),
            Place::Portal(index) => {
                let (frame, sides, _) = &prepared.portals[index];
                sides
                    .iter()
                    .flatten()
                    .filter(|side| {
                        (frame.z0 - prepared.floors[side.floor].z0).abs() <= step + ON_SURFACE
                    })
                    .map(|side| side.floor)
                    .collect()
            }
        }
    }

    /// The level reachable from `seeds` through portals and shared
    /// boundaries within a step.
    fn level(prepared: &Prepared, seeds: BTreeSet<usize>, step: f64) -> Result<Level, String> {
        let floors = &prepared.floors;
        let within = |frame: &PortalFrame, floor: usize| {
            (frame.z0 - floors[floor].z0).abs() <= step + ON_SURFACE
        };
        let mut level = seeds.clone();
        let mut frontier: Vec<usize> = seeds.into_iter().collect();
        let mut crossings: Vec<usize> = Vec::new();
        while let Some(current) = frontier.pop() {
            let mut next = Vec::new();
            // A portal is walked through only when its sill is within a step
            // of both floors: a window opening is no door.
            for (index, (frame, sides, _)) in prepared.portals.iter().enumerate() {
                if let [Some(a), Some(b)] = sides
                    && (a.floor == current || b.floor == current)
                    && within(frame, a.floor)
                    && within(frame, b.floor)
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
        let ends: Vec<usize> = prepared
            .portals
            .iter()
            .enumerate()
            .filter(|(index, (frame, sides, _))| {
                !crossings.contains(index)
                    && sides
                        .iter()
                        .flatten()
                        .any(|side| level.contains(&side.floor) && within(frame, side.floor))
            })
            .map(|(index, _)| index)
            .collect();
        let mut incomplete: Vec<String> = prepared.gaps.clone();
        incomplete.extend(
            prepared
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
                .map(|(id, _)| format!("vertical connector {id} leaves the level")),
        );
        let text = level
            .iter()
            .map(|index| floors[*index].id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        Ok(Level {
            floors: level,
            crossings,
            ends,
            incomplete,
            text,
        })
    }

    /// The level's free region: its floors less what obstructs the band
    /// above each, joined by its crossings' corridors and, with `ends`, the
    /// corridors of portals opening from it onto nothing else of it.
    fn domain(
        prepared: &Prepared,
        level: &Level,
        obstacles: &[Obstacle<'_>],
        profile: MobilityProfile,
        ends: bool,
    ) -> Result<Plan, String> {
        let (step, height) = (profile.maximum_step_metres(), profile.height_metres());
        let floors = &prepared.floors;
        let mut domain = Plan::empty();
        for index in &level.floors {
            let floor = &floors[*index];
            let blocked =
                obstruction(obstacles, &floor.bounds, floor.z0 + step, floor.z0 + height)?;
            domain = join(&domain, &subtract(&floor.footprint, &blocked)?)?;
        }
        let opened = level
            .crossings
            .iter()
            .chain(if ends { level.ends.iter() } else { [].iter() });
        for index in opened {
            let (frame, sides, _) = &prepared.portals[*index];
            // An end walks into its corridor only from its own level.
            let sides = sides.clone().map(|side| {
                side.filter(|side| {
                    level.floors.contains(&side.floor)
                        && (frame.z0 - floors[side.floor].z0).abs() <= step + ON_SURFACE
                })
            });
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
            let blocked = obstruction(obstacles, &bounds, frame.z0 + step, frame.z0 + height)?;
            domain = join(&domain, &corridor(frame, &sides, &blocked)?)?;
        }
        Ok(domain)
    }

    /// Portals too narrow for the body. Their mid-line is a barrier for the
    /// point path (no body crosses it). Where the corridor's own chord rules
    /// the body out, a band around the mid-line holds no body centre at all:
    /// a centre at distance `t` from the line covers a chord of
    /// `2 sqrt(r^2 - t^2)` on it, longer than the free interval while
    /// `t < sqrt(r^2 - (L/2)^2)`. Cutting half that band out of the free
    /// region leaves every body path inside what remains.
    fn narrow(prepared: &Prepared, level: &Level, domain: &Plan, radius: f64) -> Narrow {
        let body = 2.0 * radius;
        let mut narrow = Narrow {
            barriers: Vec::new(),
            cuts: Vec::new(),
            names: Vec::new(),
        };
        for index in &level.crossings {
            let (frame, _, clearance) = &prepared.portals[*index];
            let intervals = mid_line(frame, &[domain]);
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
            narrow.names.push(frame.id.to_string());
            if chord >= body {
                // Only the stated clear width rules the body out, and it
                // speaks for the door's own extent alone.
                narrow
                    .barriers
                    .push(vec![frame.point(frame.u0, 0.0), frame.point(frame.u1, 0.0)]);
            } else {
                narrow.barriers.extend(mid_line_barriers(frame, &[domain]));
                let half_band = 0.5 * (radius * radius - (longest * 0.5).powi(2)).sqrt();
                if half_band > MARGIN {
                    narrow
                        .cuts
                        .extend(intervals.iter().filter_map(|(a, b, before, after)| {
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
        narrow
    }

    /// Refuses a route that crosses a door whose leaf and lining may not
    /// admit the body.
    fn admitted(prepared: &Prepared, path: &[Point2], body: f64) -> Result<(), String> {
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
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn measure(&self, request: &MetricRouteRequest) -> Result<MetricRouteOutcome, String> {
        if let Some(routing) = request.connectors() {
            return self.route_across(request, routing);
        }
        let profile = request.profile();
        let radius = profile.radius_metres();
        let step = profile.maximum_step_metres();
        let height = profile.height_metres();
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        let prepared = self.prepared();
        let obstacles = self.obstacles()?;

        let origin = Self::locate(prepared, request.origin(), step)?;
        let destination = Self::locate(prepared, request.destination(), step)?;
        let level = Self::level(prepared, BTreeSet::from([origin]), step)?;
        let complete = level.complete();
        let level_text = &level.text;
        let blocked = |reason: &str| -> Result<MetricRouteOutcome, String> {
            let completeness = self.completeness(format!(
                "axiolid:metric-route:blocked:{reason}:level=[{level_text}]:surfaces={}:\
                 portals={}:obstacles={}:connectors={}:radius={radius:.6}:step={step:.6}:\
                 height={height:.6}",
                self.surfaces.len(),
                self.portals.len(),
                obstacles.len(),
                self.connectors.len(),
            ))?;
            Ok(MetricRouteOutcome::Blocked(
                BlockedMetricRouteEvidence::new(request.clone(), completeness),
            ))
        };
        if !level.floors.contains(&destination) {
            if complete {
                return blocked("destination-off-level");
            }
            return Err(format!(
                "the destination is off the origin's level and the level is not closed: {}",
                level.incomplete.join("; ")
            ));
        }

        let domain = Self::domain(prepared, &level, &obstacles, profile, false)?;
        let narrow = Self::narrow(prepared, &level, &domain, radius);

        let [ox, oy, _] = request.origin().coordinates_metres();
        let [dx, dy, _] = request.destination().coordinates_metres();
        let (from, to) = (Point2::new(ox, oy), Point2::new(dx, dy));
        if complete && separated(&domain, narrow.cuts.clone(), from, to)? {
            return blocked(&format!(
                "separated:narrow-portals=[{}]",
                narrow.names.join(",")
            ));
        }
        let straight = (to - from).length();
        let mut lower = straight;
        let mut point_bound = false;
        if complete {
            match axiolid_route::shortest_path_within(
                domain.polygons(),
                &narrow.barriers,
                from,
                to,
                ROUTE_BUDGET,
            ) {
                Ok(Ok(route)) => {
                    lower = lower.max(route.length * (1.0 - LENGTH_ROUNDING));
                    point_bound = true;
                }
                Ok(Err(Unreachable::StartOutside | Unreachable::GoalOutside)) => {
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
                format!("; the level is not closed: {}", level.incomplete.join("; "))
            };
            return Err(format!(
                "no route wide enough for the body could be proven, and a blocking gap cannot \
                 be proven without one-sided erosion; shortest distance is at least \
                 {lower:.6}{open}"
            ));
        };
        Self::admitted(prepared, &path, 2.0 * radius)?;

        let upper = rounded_up(length(&path));
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        let waypoints = Self::waypoints(
            request.origin(),
            &path,
            request.destination(),
            &level,
            prepared,
        )?;
        let traversed = Self::traversed(request, &path, &level, prepared);
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

    /// Sorts many targets: placed on the level, proven off a closed level
    /// (dropped), or bounded only by a straight line.
    fn targets(prepared: &Prepared, level: &Level, targets: &[MetricPoint], step: f64) -> Targets {
        let mut sorted = Targets {
            placed: Vec::new(),
            straight: Vec::new(),
        };
        for (index, target) in targets.iter().enumerate() {
            match Self::place(prepared, target, step) {
                Ok(place) if level.holds(place) => sorted.placed.push(index),
                Ok(_) if level.complete() => {}
                Ok(_) => sorted.straight.push((
                    index,
                    format!("{} is off the level, which is not closed", target.subject()),
                )),
                Err(reason) => sorted.straight.push((index, reason)),
            }
        }
        sorted
    }

    /// A distance map from the placed targets over the domain.
    fn map(
        domain: &Plan,
        barriers: &[Vec<Point2>],
        targets: &[MetricPoint],
        placed: &[usize],
    ) -> Result<axiolid_route::DistanceMap, String> {
        let points: Vec<Point2> = placed.iter().map(|index| plan(&targets[*index])).collect();
        axiolid_route::distance_map_within(domain.polygons(), barriers, &points, ROUTE_BUDGET)
            .map_err(|error| match error {
                MapError::TargetOutside { index } => format!(
                    "{} is not in free walkable space",
                    targets[placed[index]].subject()
                ),
                other => format!("the distance map could not be built: {other:?}"),
            })
    }

    #[allow(clippy::too_many_lines)]
    fn nearest(&self, request: &NearestTargetRequest) -> Result<NearestTargetOutcome, String> {
        let profile = request.profile();
        let (radius, step, height) = (
            profile.radius_metres(),
            profile.maximum_step_metres(),
            profile.height_metres(),
        );
        if radius > 0.0 && !request.costs().is_empty() {
            return Err(
                "weighted travel is measured for a point only: a body's walk is proven by \
                 its sweep, which the weighted map does not propose"
                    .into(),
            );
        }
        if let Some(routing) = request.connectors() {
            return self.nearest_across(request, routing);
        }
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        let prepared = self.prepared();
        let mut obstacles = self.obstacles()?;
        // An avoided object obstructs like any other body, even a surface
        // or portal.
        obstacles.extend(crate::walkable::obstacles(
            &self.geometry,
            request.avoided(),
        )?);
        let start = Self::place(prepared, request.origin(), step)?;
        let level = Self::level(prepared, Self::seeds(prepared, start, step), step)?;
        let domain = Self::domain(prepared, &level, &obstacles, profile, true)?;
        let narrow = Self::narrow(prepared, &level, &domain, radius);
        let targets = request.targets();
        let sorted = Self::targets(prepared, &level, targets, step);
        let from = plan(request.origin());
        let straight = sorted
            .straight
            .iter()
            .map(|(index, _)| (plan(&targets[*index]) - from).length())
            .fold(f64::INFINITY, f64::min);
        let why = || {
            let mut reasons: Vec<String> = sorted
                .straight
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect();
            reasons.extend(level.incomplete.iter().cloned());
            reasons.join("; ")
        };
        let avoided = request
            .avoided()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let unreachable = |reason: &str| -> Result<NearestTargetOutcome, String> {
            let completeness = self.completeness(format!(
                "axiolid:metric-route:nearest:unreachable:{reason}:level=[{}]:targets={}:\
                 avoided=[{avoided}]:surfaces={}:portals={}:obstacles={}:connectors={}:\
                 radius={radius:.6}:step={step:.6}:height={height:.6}",
                level.text,
                targets.len(),
                self.surfaces.len(),
                self.portals.len(),
                obstacles.len(),
                self.connectors.len(),
            ))?;
            Ok(NearestTargetOutcome::Unreachable(
                UnreachableTargetsEvidence::new(request.clone(), completeness),
            ))
        };
        if sorted.placed.is_empty() {
            if sorted.straight.is_empty() {
                return unreachable("targets-off-level");
            }
            return Err(format!(
                "no target is placed on the origin's level; the nearest is at least \
                 {straight:.6} away in a straight line: {}",
                why()
            ));
        }
        let costed = self.costed(prepared, request.costs())?;
        let regions = Self::cost_regions(&costed, &level, &domain)?;
        let points: Vec<Point2> = sorted
            .placed
            .iter()
            .map(|index| plan(&targets[*index]))
            .collect();
        let map = if regions.is_empty() {
            weighted::Map::Plain(Self::map(
                &domain,
                &narrow.barriers,
                targets,
                &sorted.placed,
            )?)
        } else {
            Self::weighed_map(&domain, &narrow.barriers, &points, &regions, 0.0)?
        };
        let reach = match map.nearest(from) {
            Ok(Ok(reach)) => reach,
            Ok(Err(Unreachable::StartOutside)) => {
                return Err(format!(
                    "{} is not in free walkable space",
                    request.origin().subject()
                ));
            }
            Ok(Err(_)) => {
                let mut apart = level.complete() && sorted.straight.is_empty();
                for index in &sorted.placed {
                    if !apart {
                        break;
                    }
                    apart = separated(&domain, narrow.cuts.clone(), from, plan(&targets[*index]))?;
                }
                if apart {
                    return unreachable(&format!(
                        "separated:narrow-portals=[{}]",
                        narrow.names.join(",")
                    ));
                }
                return Err(format!(
                    "no placed target is reachable for a point, and the targets cannot be \
                     proven cut off: {}",
                    why()
                ));
            }
            Err(error) => return Err(error),
        };
        // The map misses shortcuts through what the level does not close
        // over (an unmeasured surface, a connector), so only a closed level
        // bounds from below by it; otherwise every target's straight line,
        // which no walk undercuts, weighted or not.
        let lower = if level.complete() {
            (reach.lower * (1.0 - LENGTH_ROUNDING)).min(straight)
        } else {
            targets
                .iter()
                .map(|target| (plan(target) - from).length())
                .fold(f64::INFINITY, f64::min)
                .min(reach.lower)
        };
        let (target, path) = if radius <= 0.0 {
            (sorted.placed[reach.target], reach.path.clone())
        } else {
            // Any proven route to any target bounds the nearest from above:
            // the map's nearest first, then the rest by straight line.
            let mut order: Vec<usize> = (0..sorted.placed.len()).collect();
            order.sort_by(|a, b| {
                let key = |k: &usize| {
                    (
                        *k != reach.target,
                        (plan(&targets[sorted.placed[*k]]) - from).length(),
                    )
                };
                let (ka, kb) = (key(a), key(b));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            });
            let mut found = None;
            for k in order {
                let index = sorted.placed[k];
                if let Some(path) = witness(&domain, from, plan(&targets[index]), radius)?
                    && Self::admitted(prepared, &path, 2.0 * radius).is_ok()
                {
                    found = Some((index, path));
                    break;
                }
            }
            found.ok_or_else(|| {
                format!(
                    "no route wide enough for the body to any target could be proven; the \
                     nearest target is at least {lower:.6} away"
                )
            })?
        };
        // A point's walk is the map's own, weighted or not; a body's is the
        // proven sweep, only ever plain.
        let upper = if radius <= 0.0 {
            rounded_up(reach.upper)
        } else {
            rounded_up(length(&path))
        };
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        let waypoints =
            Self::waypoints(request.origin(), &path, &targets[target], &level, prepared)?;
        let weighed = map.spacing().map_or_else(String::new, |spacing| {
            format!(
                ":costs=[{}]:spacing={spacing:.6}",
                weighted::costs_text(request.costs())
            )
        });
        let evidence = Evidence::exact(
            request.origin().subject().source.clone(),
            format!(
                "axiolid:metric-route:nearest:level=[{}]:targets={}:placed={}:avoided=[{}]{weighed}:\
                 radius={radius:.6}:step={step:.6}:height={height:.6}:lower={}:upper={upper:.6}:\
                 witness={}",
                level.text,
                targets.len(),
                sorted.placed.len(),
                avoided,
                if level.complete() {
                    format!("point-path={:.6}", distance.lower_metres())
                } else {
                    format!("straight-line={:.6}", distance.lower_metres())
                },
                if radius <= 0.0 {
                    "point-path"
                } else {
                    "sweep=proven"
                },
            ),
        );
        Ok(NearestTargetOutcome::Reached(
            NearestTargetEvidence::try_new(target, distance, waypoints, evidence)
                .map_err(|e| e.to_string())?,
        ))
    }

    #[allow(clippy::too_many_lines)]
    fn farthest(&self, request: &FarthestPointRequest) -> Result<FarthestPointOutcome, String> {
        if let Some(routing) = request.connectors() {
            return self.farthest_across(request, routing);
        }
        let profile = request.profile();
        let (radius, step, height) = (
            profile.radius_metres(),
            profile.maximum_step_metres(),
            profile.height_metres(),
        );
        if radius > 0.0 {
            return Err(
                "the farthest point is measured for a point only: where a body of a radius \
                 can stand needs one-sided erosion"
                    .into(),
            );
        }
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        let prepared = self.prepared();
        let obstacles = self.obstacles()?;
        let region_id = request.region();
        let region = prepared
            .floors
            .iter()
            .position(|floor| &floor.id == region_id)
            .ok_or_else(|| {
                if self.surfaces.contains(region_id) {
                    format!(
                        "{region_id} was not measured as a walkable surface: {}",
                        prepared.gaps.join("; ")
                    )
                } else {
                    format!("{region_id} is not a declared walkable surface")
                }
            })?;
        let floor = &prepared.floors[region];
        let level = Self::level(prepared, BTreeSet::from([region]), step)?;
        let domain = Self::domain(prepared, &level, &obstacles, profile, true)?;
        let targets = request.targets();
        let sorted = Self::targets(prepared, &level, targets, step);
        let closed = level.complete() && sorted.straight.is_empty();
        let why = || {
            let mut reasons: Vec<String> = sorted
                .straight
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect();
            reasons.extend(level.incomplete.iter().cloned());
            reasons.join("; ")
        };
        let at = |point: Point2| {
            MetricPoint::try_new(region_id.clone(), [point.x, point.y, floor.z0])
                .map_err(|e| e.to_string())
        };
        let unreachable = |point: Point2, reason: &str| -> Result<FarthestPointOutcome, String> {
            let completeness = self.completeness(format!(
                "axiolid:metric-route:farthest:unreachable:{reason}:region={region_id}:\
                 level=[{}]:targets={}:surfaces={}:portals={}:obstacles={}:connectors={}:\
                 step={step:.6}:height={height:.6}:witness=({:.6},{:.6})",
                level.text,
                targets.len(),
                self.surfaces.len(),
                self.portals.len(),
                obstacles.len(),
                self.connectors.len(),
                point.x,
                point.y,
            ))?;
            Ok(FarthestPointOutcome::Unreachable(
                UnreachableRegionEvidence::new(request.clone(), at(point)?, completeness),
            ))
        };
        let free = intersect(&floor.footprint, &domain)?;
        if sorted.placed.is_empty() {
            if closed {
                let point = inner_point(&free)
                    .ok_or_else(|| format!("{region_id} has no free walkable area"))?;
                return unreachable(point, "targets-off-level");
            }
            return Err(format!(
                "no target is placed on {region_id}'s level: {}",
                why()
            ));
        }
        let tolerance = request.tolerance_metres();
        let costed = self.costed(prepared, request.costs())?;
        let regions = Self::cost_regions(&costed, &level, &domain)?;
        let map = if regions.is_empty() {
            weighted::Map::Plain(Self::map(&domain, &[], targets, &sorted.placed)?)
        } else {
            let points: Vec<Point2> = sorted
                .placed
                .iter()
                .map(|index| plan(&targets[*index]))
                .collect();
            // The weighted bracket is first order in the spacing, about
            // twice it wide.
            Self::weighed_map(&domain, &[], &points, &regions, tolerance / 2.0)?
        };
        let mut best: Option<(f64, f64, Point2)> = None;
        let mut upper = 0.0_f64;
        let mut cells = 0;
        for piece in floor.footprint.polygons() {
            match map.farthest(piece, tolerance) {
                Ok(found) => {
                    cells += found.cells;
                    upper = upper.max(found.distance.upper);
                    let Some(witness) = found.witness else {
                        return Err(format!(
                            "the farthest point of {region_id} was bounded without sampling \
                             any point of it"
                        ));
                    };
                    if best.is_none_or(|(lower, _, _)| found.distance.lower > lower) {
                        best = Some((found.distance.lower, found.distance.upper, witness));
                    }
                }
                Err(FarthestError::Empty) => {}
                Err(FarthestError::Unreachable { triangle }) => {
                    if !closed {
                        return Err(format!(
                            "part of {region_id} reaches no placed target, and the level is \
                             not closed: {}",
                            why()
                        ));
                    }
                    let cell = polygon(triangle.to_vec())
                        .ok_or_else(|| "an unreachable cell is degenerate".to_owned())?;
                    let part = intersect(&Plan::piece(cell), &Plan::piece(piece.clone()))?;
                    let point = inner_point(&part).ok_or_else(|| {
                        format!(
                            "part of {region_id} reaches no target, but no point of it could \
                             be placed inside the region"
                        )
                    })?;
                    return unreachable(point, "cut-off");
                }
                Err(error) => {
                    return Err(format!(
                        "the farthest point of {region_id} could not be bracketed: {error:?}"
                    ));
                }
            }
        }
        let Some((bracket, _, witness)) = best else {
            return Err(format!("{region_id} has no free walkable area"));
        };
        // Missing free space only lengthens map routes, so the upper bound
        // stands; the lower bound needs every shortcut and target known.
        let lower = if closed {
            bracket
        } else {
            targets
                .iter()
                .map(|target| (plan(target) - witness).length())
                .fold(f64::INFINITY, f64::min)
                .min(upper)
        };
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        let converged = upper - distance.lower_metres() <= tolerance;
        let evidence = Evidence::exact(
            region_id.source.clone(),
            format!(
                "axiolid:metric-route:farthest:region={region_id}:level=[{}]:targets={}:\
                 placed={}{}:step={step:.6}:height={height:.6}:tolerance={tolerance:.6}:\
                 cells={cells}:lower={}:upper={upper:.6}:witness=({:.6},{:.6})",
                level.text,
                targets.len(),
                sorted.placed.len(),
                map.spacing().map_or_else(String::new, |spacing| format!(
                    ":costs=[{}]:spacing={spacing:.6}",
                    weighted::costs_text(request.costs())
                )),
                if closed {
                    format!("bracket={lower:.6}")
                } else {
                    format!("straight-line={lower:.6}")
                },
                witness.x,
                witness.y,
            ),
        );
        Ok(FarthestPointOutcome::Bounded(
            FarthestPointEvidence::try_new(distance, at(witness)?, converged, evidence)
                .map_err(|e| e.to_string())?,
        ))
    }

    /// How much of a polyline lies over each requested object's footprint.
    fn trace(&self, request: &PathTraceRequest) -> Result<PathTrace, String> {
        let points: Vec<Point2> = request.waypoints().iter().map(plan).collect();
        let prepared = self.prepared();
        let lengths = request
            .objects()
            .iter()
            .map(|object| {
                let (footprint, sure) = self.footprint(prepared, object)?;
                let (inside, over) = points.windows(2).fold((0.0, 0.0), |(inside, over), pair| {
                    let (a, b) = segment_cover(&footprint, pair[0], pair[1]);
                    (inside + a, over + b)
                });
                let inside = if sure {
                    inside * (1.0 - LENGTH_ROUNDING)
                } else {
                    0.0
                };
                let over = if over > 0.0 { rounded_up(over) } else { 0.0 };
                LengthInterval::try_new(inside.min(over), over).map_err(|e| e.to_string())
            })
            .collect();
        let evidence = Evidence::exact(
            self.source.clone(),
            format!(
                "axiolid:metric-route:trace:waypoints={}:length={:.6}:objects=[{}]",
                points.len(),
                length(&points),
                request
                    .objects()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        );
        PathTrace::try_new(lengths, evidence).map_err(|e| e.to_string())
    }

    /// An object's plan footprint, and whether it is exact: a declared
    /// surface's measured floor, a body's projection, or, for a
    /// tessellation, the plan box enclosing its true body (only an upper
    /// bound). A bodiless object covers nothing.
    fn footprint(&self, prepared: &Prepared, object: &ObjectId) -> Result<(Plan, bool), String> {
        if let Some(floor) = prepared.floors.iter().find(|floor| &floor.id == object) {
            return Ok((floor.footprint.clone(), true));
        }
        if self.geometry.mesh(object).is_none() && self.geometry.has_no_body(object) {
            return Ok((Plan::empty(), true));
        }
        let mesh = body(&self.geometry, object, "traced object")?;
        if self.geometry.is_tessellated(object) {
            let ([x0, y0, _], [x1, y1, _]) =
                self.geometry.enclosing_extent(object).ok_or_else(|| {
                    format!("{object} has an empty mesh or an invalid chord deviation")
                })?;
            // Grown by the margin, so even a flat body has an area.
            let (x0, y0, x1, y1) = (x0 - MARGIN, y0 - MARGIN, x1 + MARGIN, y1 + MARGIN);
            let corners = vec![
                Point2::new(x0, y0),
                Point2::new(x1, y0),
                Point2::new(x1, y1),
                Point2::new(x0, y1),
            ];
            let footprint = polygon(corners).map_or_else(Plan::empty, Plan::piece);
            return Ok((footprint, false));
        }
        Ok((union(projected_polygons(&triangles(mesh)))?, true))
    }

    /// The route's waypoints between two given end points, each inner one
    /// grounded on the surface of the level holding it.
    fn waypoints(
        start: &MetricPoint,
        path: &[Point2],
        end: &MetricPoint,
        level: &Level,
        prepared: &Prepared,
    ) -> Result<Vec<MetricPoint>, String> {
        let mut waypoints = vec![start.clone()];
        for point in path.iter().skip(1).take(path.len().saturating_sub(2)) {
            let floor = holder(prepared, level, *point)
                .unwrap_or(&prepared.floors[*level.floors.first().unwrap_or(&0)]);
            waypoints.push(
                MetricPoint::try_new(floor.id.clone(), [point.x, point.y, floor.z0])
                    .map_err(|e| e.to_string())?,
            );
        }
        waypoints.push(end.clone());
        Ok(waypoints)
    }

    /// The objects a route traverses, in order.
    fn traversed(
        request: &MetricRouteRequest,
        path: &[Point2],
        level: &Level,
        prepared: &Prepared,
    ) -> Vec<ObjectId> {
        let mut traversed: Vec<ObjectId> = Vec::new();
        let mut push = |id: &ObjectId| {
            if traversed.last() != Some(id) {
                traversed.push(id.clone());
            }
        };
        for pair in path.windows(2) {
            if let Some(floor) = holder(prepared, level, pair[0]) {
                push(&floor.id);
            }
            for (frame, _, _) in &prepared.portals {
                if crosses_mid_line(frame, pair[0], pair[1]) {
                    push(&frame.id);
                }
            }
            if let Some(floor) = holder(prepared, level, pair[1]) {
                push(&floor.id);
            }
        }
        if traversed.is_empty() {
            traversed.push(request.origin().subject().clone());
        }
        traversed
    }
}

/// The level's floor holding a plan point.
fn holder<'p>(prepared: &'p Prepared, level: &Level, point: Point2) -> Option<&'p Floor> {
    level
        .floors
        .iter()
        .map(|index| &prepared.floors[*index])
        .find(|floor| contains(&floor.footprint, point))
}

/// A point's plan position.
fn plan(point: &MetricPoint) -> Point2 {
    let [x, y, _] = point.coordinates_metres();
    Point2::new(x, y)
}

/// A polyline's summed length.
fn length(path: &[Point2]) -> f64 {
    path.windows(2)
        .map(|pair| (pair[1] - pair[0]).length())
        .sum()
}

/// A summed length raised past its rounding.
fn rounded_up(length: f64) -> f64 {
    length * (1.0 + LENGTH_ROUNDING) + f64::EPSILON
}

/// A point strictly inside a region: the centre of its largest convex
/// piece.
fn inner_point(region: &Plan) -> Option<Point2> {
    let pieces = trapezoids(region);
    let piece = pieces
        .iter()
        .max_by(|a, b| crate::planar::polygon_area(a).total_cmp(&crate::planar::polygon_area(b)))?;
    let points = &piece.outer.points;
    #[allow(clippy::cast_precision_loss)]
    let count = points.len() as f64;
    let sum = points.iter().fold(Point2::new(0.0, 0.0), |sum, p| {
        Point2::new(sum.x + p.x, sum.y + p.y)
    });
    Some(Point2::new(sum.x / count, sum.y / count))
}

impl MetricRoutingService for AxiolidMetricRoutingService {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        self.measure(request)
            .map_err(MetricRoutingError::Unavailable)
    }

    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        self.nearest(request)
            .map_err(MetricRoutingError::Unavailable)
    }

    fn avoids_objects(&self) -> bool {
        true
    }

    fn weighs_travel(&self) -> bool {
        true
    }

    fn forced_walk(
        &self,
        request: &ForcedWalkRequest,
    ) -> Result<ForcedWalkOutcome, MetricRoutingError> {
        self.forced(request)
            .map_err(MetricRoutingError::Unavailable)
    }

    fn climbs_connectors(&self) -> bool {
        true
    }

    fn trace_path(&self, request: &PathTraceRequest) -> Result<PathTrace, MetricRoutingError> {
        self.trace(request).map_err(MetricRoutingError::Unavailable)
    }

    fn farthest_point(
        &self,
        request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        self.farthest(request)
            .map_err(MetricRoutingError::Unavailable)
    }
}
