//! Walks across levels through a request's vertical connectors.
//!
//! A request carrying a [`ConnectorRouting`] climbs only through its own
//! connectors. Each is measured by `crate::connector`: a stair or ramp
//! with an exactly known walking line has two **landings**, one on the
//! floor each end meets, and a climb length between them. The levels a
//! walk may reach are the start's level and every level a climbed
//! connector lands on, transitively; each is a free region as for a walk
//! on one level.
//!
//! On each level `axiolid-route` measures the walks between the points
//! that matter there (the start, the landings, the targets) with one
//! distance map per landing and one from the level's targets: lower
//! bounds from the point map on a closed level, upper bounds from the
//! point path (or a proven sweep for a body with a radius). The level
//! walks and the climbs form a small graph, and a shortest path over it
//! (once on lower bounds, once on upper bounds) bounds the walk across
//! levels: every real walk through the connectors is such a sequence of
//! level walks and climbs.
//!
//! A requested connector that cannot be measured or placed is not climbed,
//! and every level it touches is not closed: the lower bound then falls
//! back to straight lines, and nothing is reported unreachable.
//!
//! The farthest point of a region weighs each landing on the region's
//! level by the walk beyond it. One distance map over every source, each
//! point seeded with its source's weight (axiolid/kernel#197,
//! `axiolid-route` 0.3.4), brackets the largest walk from below with the
//! walks beyond bounded from below, and one seeded with the walks bounded
//! from above (sources with no upper bound left out) brackets it from
//! above; where both bounds agree one map does. The lower bound is also
//! evaluated at every witness found. Only when a weighted map cannot be
//! built do the per-source maps bound it from above, their farthest
//! distance plus their weight.
//!
//! **Weighted travel.** A request's travel costs weigh each level's walks
//! with that level's own cost regions (`weighted::cost_regions`: only the
//! costs whose object lies in the storey of one of the level's floors), so
//! every level walk, lower and upper bound, is a weighted map's bracket.
//! The farthest point's sources are seeded with the weighted walks beyond
//! them (`weighted_distance_map_seeded`, axiolid/kernel#198), each bound
//! into the map of its own kind. A climb counts its measured length at
//! least once and at most times the largest factor of a cost meeting its
//! connector (`weighted::climb_factor`), so the graph's shortest paths
//! still bound every weighted walk through the connectors from both sides.
//! Weighted walks are measured for a point only.

use std::collections::{BTreeMap, BTreeSet};

use axiolid_core::Point2;
use axiolid_route::{CostRegion, FarthestError, MapError, Unreachable};
use axioval_engine::{
    ConnectorRouting, FarthestPointEvidence, FarthestPointOutcome, FarthestPointRequest,
    LengthInterval, MetricPoint, MetricRouteEvidence, MetricRouteOutcome, MetricRouteRequest,
    MobilityProfile, NearestTargetEvidence, NearestTargetOutcome, NearestTargetRequest, TravelCost,
    UnreachableRegionEvidence, UnreachableTargetsEvidence,
};
use axioval_ir::{Evidence, ObjectId};

use super::weighted::{Costed, Map, climb_factor, costs_text, describe};
use super::{
    AxiolidMetricRoutingService, LENGTH_ROUNDING, Level, Narrow, Prepared, holder, inner_point,
    length, plan, rounded_up,
};
use crate::connector::{self, Climb, Passable};
use crate::walkable::{
    ON_SURFACE, Obstacle, Plan, REACH, contains, crosses_mid_line, intersect, plan_gap, polygon,
    separated, witness,
};

/// A source of walks on the farthest point's level: its points, and the
/// walk beyond them bounded from below and from above.
type Source = (Vec<Point2>, f64, f64);

/// A connector a walk may climb: measured, its landings placed.
struct Link {
    climb: Climb,
    landings: [Point2; 2],
    floors: [usize; 2],
    length: LengthInterval,
    /// Whether the body fits through it; never `Refused` here.
    passable: Passable,
    /// The largest factor its climb may count, one when unweighted.
    factor: f64,
}

/// One level a walk may reach, with its free region and the cost regions
/// weighing its walks.
struct Stage {
    level: Level,
    domain: Plan,
    narrow: Narrow,
    regions: Vec<CostRegion>,
}

/// The levels a walk may reach through the request's connectors.
struct Tour {
    stages: Vec<Stage>,
    links: Vec<Link>,
    /// The stage of each link's landings.
    at: Vec<[usize; 2]>,
    /// Requested connectors that are not climbed, with why.
    undecided: Vec<String>,
}

impl Tour {
    fn closed(&self) -> bool {
        self.stages.iter().all(|stage| stage.level.complete())
    }

    fn incomplete(&self) -> Vec<String> {
        let mut reasons: Vec<String> = self
            .stages
            .iter()
            .flat_map(|stage| stage.level.incomplete.iter().cloned())
            .collect();
        reasons.sort();
        reasons.dedup();
        reasons
    }

    fn text(&self, routing: &ConnectorRouting, radius: f64, costs: &[TravelCost]) -> String {
        let levels: Vec<String> = self
            .stages
            .iter()
            .map(|stage| format!("[{}]", stage.level.text))
            .collect();
        let climbs: Vec<String> = self
            .links
            .iter()
            .map(|link| {
                let text = link.climb.text(routing.climb(), radius);
                if link.factor > 1.0 {
                    format!("{text}*<={}", link.factor)
                } else {
                    text
                }
            })
            .collect();
        let weighed = if costs.is_empty() {
            String::new()
        } else {
            format!(
                ":costs=[{}]:weighed-levels={}",
                costs_text(costs),
                self.stages
                    .iter()
                    .filter(|stage| !stage.regions.is_empty())
                    .count()
            )
        };
        format!(
            "levels={}:climbs=[{}]:unclimbed={}{weighed}",
            levels.join(","),
            climbs.join(","),
            self.undecided.len()
        )
    }
}

/// A node of the level graph: the start, or a connector's landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Start,
    Landing(usize, usize),
}

/// A walk on one level between two nodes, or a climb between a link's
/// landings.
struct Edge {
    a: usize,
    b: usize,
    lower: f64,
    upper: f64,
    /// The walk from `a` to `b` in plan, for a level walk with an upper
    /// bound.
    path: Option<Vec<Point2>>,
    /// The stage of a level walk; `None` for a climb.
    stage: Option<usize>,
    /// The target a walk into the sink reaches.
    target: Option<usize>,
}

/// How the targets stand against the reached levels.
struct Placed {
    /// Per stage, the indices of the targets placed on it.
    on: Vec<Vec<usize>>,
    /// Targets bounded only by a straight line, with why.
    straight: Vec<(usize, String)>,
    /// Targets placed off every reached level.
    off: Vec<usize>,
}

/// A walk on one level: its lower and upper bounds, its path when proven,
/// and the index of the map point it reaches.
type Leg = (f64, f64, Option<Vec<Point2>>, Option<usize>);

/// What a walk across levels found.
pub(super) enum Walked {
    Reached {
        target: usize,
        distance: LengthInterval,
        waypoints: Vec<MetricPoint>,
        traversed: Vec<ObjectId>,
        locator: String,
    },
    Unreachable(String),
}

impl AxiolidMetricRoutingService {
    /// The requested connectors measured, and the levels reached from
    /// `seeds` through them.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn tour(
        &self,
        prepared: &Prepared,
        routing: &ConnectorRouting,
        avoided: &[ObjectId],
        seeds: BTreeSet<usize>,
        obstacles: &[Obstacle<'_>],
        profile: MobilityProfile,
        costed: &[Costed],
    ) -> Result<Tour, String> {
        let (radius, step) = (profile.radius_metres(), profile.maximum_step_metres());
        let mut links = Vec::new();
        let mut unclimbed: Vec<(ObjectId, String)> = Vec::new();
        for connector in routing.connectors() {
            let id = connector.object();
            // An avoided connector obstructs as any avoided body does, and
            // is no way between levels.
            if avoided.binary_search(id).is_ok() {
                continue;
            }
            let measured = connector::measure(&self.geometry, connector).and_then(|climb| {
                let landings = [climb.landing(0, radius), climb.landing(1, radius)];
                let mut floors = [0; 2];
                for end in 0..2 {
                    floors[end] = ground(prepared, landings[end], climb.ends[end].z, step)
                        .map_err(|reason| {
                            format!(
                                "the {} landing of {id} is not placed: {reason}",
                                if end == 0 { "lower" } else { "upper" }
                            )
                        })?;
                }
                let length = climb.length(routing.climb(), radius)?;
                let passable = climb.passable(
                    &self.geometry,
                    obstacles.iter().map(|obstacle| obstacle.id.clone()),
                    2.0 * radius,
                    profile.height_metres(),
                );
                let factor = climb_factor(costed, id, self.geometry.enclosing_extent(id).as_ref());
                Ok(Link {
                    climb,
                    landings,
                    floors,
                    length,
                    passable,
                    factor,
                })
            });
            match measured {
                // A connector the body surely cannot walk is no way, and a
                // decided one.
                Ok(link) if matches!(link.passable, Passable::Refused(_)) => {}
                Ok(link) => links.push(link),
                Err(reason) => unclimbed.push((id.clone(), reason)),
            }
        }
        let floors = &prepared.floors;
        let mut stages: Vec<Stage> = Vec::new();
        let mut stage_of: BTreeMap<usize, usize> = BTreeMap::new();
        let mut queue = vec![seeds];
        while let Some(seeds) = queue.pop() {
            if seeds.iter().any(|floor| stage_of.contains_key(floor)) {
                continue;
            }
            let mut level = Self::level(prepared, seeds, step)?;
            // Host-declared connectors are no way for this request; the
            // request's own that are not climbed leave the level open.
            level.incomplete.clone_from(&prepared.gaps);
            for (id, reason) in &unclimbed {
                let touches = self.geometry.enclosing_extent(id).is_none_or(|extent| {
                    level.floors.iter().any(|index| {
                        let floor = &floors[*index];
                        floor.z0 >= extent.0[2] - REACH
                            && floor.z0 <= extent.1[2] + REACH
                            && plan_gap(&floor.bounds, &extent) <= REACH + ON_SURFACE
                    })
                });
                if touches {
                    level
                        .incomplete
                        .push(format!("vertical connector {id} is not climbed: {reason}"));
                }
            }
            let domain = Self::domain(prepared, &level, obstacles, profile, true)?;
            let narrow = Self::narrow(prepared, &level, &domain, radius);
            let regions = Self::cost_regions(costed, &level, &domain)?;
            let index = stages.len();
            for floor in &level.floors {
                stage_of.insert(*floor, index);
            }
            for link in &links {
                for end in 0..2 {
                    let other = link.floors[1 - end];
                    if level.floors.contains(&link.floors[end]) && !stage_of.contains_key(&other) {
                        queue.push(BTreeSet::from([other]));
                    }
                }
            }
            stages.push(Stage {
                level,
                domain,
                narrow,
                regions,
            });
        }
        let links: Vec<Link> = links
            .into_iter()
            .filter(|link| link.floors.iter().all(|floor| stage_of.contains_key(floor)))
            .collect();
        let at = links
            .iter()
            .map(|link| link.floors.map(|floor| stage_of[&floor]))
            .collect();
        Ok(Tour {
            stages,
            links,
            at,
            undecided: unclimbed
                .into_iter()
                .map(|(id, reason)| format!("{id}: {reason}"))
                .collect(),
        })
    }

    /// Places the targets on the reached levels.
    fn place_all(prepared: &Prepared, tour: &Tour, targets: &[MetricPoint], step: f64) -> Placed {
        let mut placed = Placed {
            on: vec![Vec::new(); tour.stages.len()],
            straight: Vec::new(),
            off: Vec::new(),
        };
        for (index, target) in targets.iter().enumerate() {
            match Self::place(prepared, target, step) {
                Ok(place) => match tour
                    .stages
                    .iter()
                    .position(|stage| stage.level.holds(place))
                {
                    Some(stage) => placed.on[stage].push(index),
                    None => placed.off.push(index),
                },
                Err(reason) => placed.straight.push((index, reason)),
            }
        }
        placed
    }

    /// A distance map over a stage from some points, each starting at its
    /// weight, weighted by the stage's cost regions; `None` when a point is
    /// not in its free region or a plain map cannot be built. A weighted
    /// map that cannot be built for any other reason refuses, never falls
    /// back to a plain one.
    fn stage_map(
        stage: &Stage,
        seeded: &[(Point2, f64)],
        wanted: f64,
    ) -> Result<Option<Map>, String> {
        match Self::seeded_map(
            &stage.domain,
            &stage.narrow.barriers,
            seeded,
            &stage.regions,
            wanted,
        ) {
            Ok(map) => Ok(Some(map)),
            Err(MapError::TargetOutside { .. }) => Ok(None),
            Err(_) if stage.regions.is_empty() => Ok(None),
            Err(error) => Err(describe(error)),
        }
    }

    /// Bounds the walk on a stage from `from` to the map's nearest point:
    /// `(lower, upper, path, index of that point)`. The lower bound is the
    /// map's on a closed level (infinite where a separation is proven),
    /// the straight line to `toward` otherwise; the upper bound is the
    /// point path's (weighted) cost, or infinite.
    fn along_map(
        stage: &Stage,
        map: Option<&Map>,
        from: Point2,
        toward: &[Point2],
    ) -> Result<Leg, String> {
        let straight = toward
            .iter()
            .map(|point| (*point - from).length())
            .fold(f64::INFINITY, f64::min);
        let complete = stage.level.complete();
        let Some(map) = map else {
            return Ok((straight, f64::INFINITY, None, None));
        };
        match map.nearest(from) {
            Ok(Ok(reach)) => {
                // Every factor is at least one: no walk, weighted or not,
                // undercuts the straight line.
                let lower = if complete {
                    reach.lower * (1.0 - LENGTH_ROUNDING)
                } else {
                    straight
                };
                Ok((
                    lower.min(reach.lower),
                    rounded_up(reach.upper),
                    Some(reach.path),
                    Some(reach.target),
                ))
            }
            Ok(Err(Unreachable::StartOutside)) | Err(_) => {
                Ok((straight, f64::INFINITY, None, None))
            }
            Ok(Err(_)) => {
                let mut apart = complete;
                for point in toward {
                    if !apart {
                        break;
                    }
                    apart = separated(&stage.domain, stage.narrow.cuts.clone(), from, *point)?;
                }
                Ok((
                    if apart { f64::INFINITY } else { straight },
                    f64::INFINITY,
                    None,
                    None,
                ))
            }
        }
    }

    /// The level graph over `nodes`: level walks between the nodes on each
    /// stage, into the sink through each stage's targets, and climbs.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn edges(
        prepared: &Prepared,
        tour: &Tour,
        nodes: &[(Node, usize, Point2)],
        targets: &[MetricPoint],
        placed: &Placed,
        radius: f64,
    ) -> Result<Vec<Edge>, String> {
        let sink = nodes.len();
        let mut edges = Vec::new();
        for (stage_index, stage) in tour.stages.iter().enumerate() {
            let here: Vec<usize> = (0..nodes.len())
                .filter(|index| nodes[*index].1 == stage_index)
                .collect();
            // Between nodes: one map per node, queried from the others.
            for (i, &a) in here.iter().enumerate() {
                let map = Self::stage_map(stage, &[(nodes[a].2, 0.0)], 0.0)?;
                for &b in here.iter().skip(i + 1) {
                    let (from, to) = (nodes[b].2, nodes[a].2);
                    let (lower, mut upper, mut path, _) =
                        Self::along_map(stage, map.as_ref(), from, &[to])?;
                    if radius > 0.0 {
                        upper = f64::INFINITY;
                        path = None;
                        if lower.is_finite()
                            && let Some(proven) = witness(&stage.domain, from, to, radius)?
                            && Self::admitted(prepared, &proven, 2.0 * radius).is_ok()
                        {
                            upper = rounded_up(length(&proven));
                            path = Some(proven);
                        }
                    }
                    // The path runs from `b` to `a`.
                    edges.push(Edge {
                        a: b,
                        b: a,
                        lower,
                        upper,
                        path,
                        stage: Some(stage_index),
                        target: None,
                    });
                }
            }
            // Into the sink through the stage's targets.
            let on = &placed.on[stage_index];
            if on.is_empty() {
                continue;
            }
            let points: Vec<Point2> = on.iter().map(|index| plan(&targets[*index])).collect();
            let seeded: Vec<(Point2, f64)> = points.iter().map(|point| (*point, 0.0)).collect();
            let map = Self::seeded_map(
                &stage.domain,
                &stage.narrow.barriers,
                &seeded,
                &stage.regions,
                0.0,
            )
            .map_err(|error| match error {
                MapError::TargetOutside { index } => format!(
                    "{} is not in free walkable space",
                    targets[on[index]].subject()
                ),
                other => describe(other),
            })?;
            for &a in &here {
                let from = nodes[a].2;
                let (lower, mut upper, mut path, reached) =
                    Self::along_map(stage, Some(&map), from, &points)?;
                let mut target = reached.map(|k| on[k]);
                if radius > 0.0 {
                    upper = f64::INFINITY;
                    path = None;
                    target = None;
                    if lower.is_finite() {
                        for (k, point) in points.iter().enumerate() {
                            if let Some(proven) = witness(&stage.domain, from, *point, radius)?
                                && Self::admitted(prepared, &proven, 2.0 * radius).is_ok()
                            {
                                let walked = rounded_up(length(&proven));
                                if walked < upper {
                                    upper = walked;
                                    path = Some(proven);
                                    target = Some(on[k]);
                                }
                            }
                        }
                    }
                }
                edges.push(Edge {
                    a,
                    b: sink,
                    lower,
                    upper,
                    path,
                    stage: Some(stage_index),
                    target,
                });
            }
        }
        for (index, link) in tour.links.iter().enumerate() {
            let find = |end: usize| {
                nodes
                    .iter()
                    .position(|(node, _, _)| *node == Node::Landing(index, end))
            };
            if let (Some(a), Some(b)) = (find(0), find(1)) {
                // A climb counts its length at least once, and at most by
                // the largest factor of a cost meeting its connector.
                edges.push(Edge {
                    a,
                    b,
                    lower: link.length.lower_metres(),
                    upper: if link.passable != Passable::Proven {
                        f64::INFINITY
                    } else if link.factor > 1.0 {
                        rounded_up(link.length.upper_metres() * link.factor)
                    } else {
                        link.length.upper_metres()
                    },
                    path: None,
                    stage: None,
                    target: None,
                });
            }
        }
        Ok(edges)
    }

    /// The nodes of a tour: the start (if any) and every landing.
    fn nodes(tour: &Tour, start: Option<(usize, Point2)>) -> Vec<(Node, usize, Point2)> {
        let mut nodes: Vec<(Node, usize, Point2)> = start
            .map(|(stage, at)| (Node::Start, stage, at))
            .into_iter()
            .collect();
        for (index, link) in tour.links.iter().enumerate() {
            for end in 0..2 {
                nodes.push((
                    Node::Landing(index, end),
                    tour.at[index][end],
                    link.landings[end],
                ));
            }
        }
        nodes
    }

    /// The walk from `origin` to the nearest of `targets` through the
    /// request's connectors.
    #[allow(clippy::too_many_lines)]
    pub(super) fn walk_across(
        &self,
        origin: &MetricPoint,
        targets: &[MetricPoint],
        profile: MobilityProfile,
        avoided: &[ObjectId],
        routing: &ConnectorRouting,
        costs: &[TravelCost],
    ) -> Result<Walked, String> {
        let (radius, step, height) = (
            profile.radius_metres(),
            profile.maximum_step_metres(),
            profile.height_metres(),
        );
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        if radius > 0.0 && !costs.is_empty() {
            return Err(
                "weighted travel is measured for a point only: a body's walk is proven by \
                 its sweep, which the weighted map does not propose"
                    .into(),
            );
        }
        let prepared = self.prepared();
        let mut obstacles = self.obstacles()?;
        obstacles.extend(crate::walkable::obstacles(&self.geometry, avoided)?);
        let start = Self::place(prepared, origin, step)?;
        let costed = self.costed(prepared, costs)?;
        let tour = self.tour(
            prepared,
            routing,
            avoided,
            Self::seeds(prepared, start, step),
            &obstacles,
            profile,
            &costed,
        )?;
        let placed = Self::place_all(prepared, &tour, targets, step);
        let from = plan(origin);
        let nodes = Self::nodes(&tour, Some((0, from)));
        let edges = Self::edges(prepared, &tour, &nodes, targets, &placed, radius)?;
        let sink = nodes.len();
        let (lowest, _) = shortest(nodes.len() + 1, &edges, 0, |edge| edge.lower);
        let (highest, via) = shortest(nodes.len() + 1, &edges, 0, |edge| edge.upper);
        let closed = tour.closed() && placed.straight.is_empty();
        let why = || {
            let mut reasons: Vec<String> = placed
                .straight
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect();
            reasons.extend(tour.incomplete());
            reasons.join("; ")
        };
        let straight_all = targets
            .iter()
            .map(|target| (plan(target) - from).length())
            .fold(f64::INFINITY, f64::min);
        let lower = if closed {
            lowest[sink]
        } else {
            lowest[sink].min(straight_all)
        };
        let text = tour.text(routing, radius, costs);
        if lower.is_infinite() {
            // Only a closed tour lets a lower bound be infinite.
            return Ok(Walked::Unreachable(format!(
                "separated-across-levels:{text}"
            )));
        }
        let upper = highest[sink];
        if upper.is_infinite() {
            return Err(format!(
                "no walk to any target through the levels could be proven; the nearest is at \
                 least {lower:.6} away{}",
                if closed {
                    String::new()
                } else {
                    format!(": {}", why())
                }
            ));
        }
        // Follow the upper bound's edges back from the sink.
        let mut hops = Vec::new();
        let mut node = sink;
        while node != 0 {
            let Some(edge) = via[node] else {
                return Err("the walk across levels could not be traced".into());
            };
            let edge_ref = &edges[edge];
            let previous = if edge_ref.a == node {
                edge_ref.b
            } else {
                edge_ref.a
            };
            hops.push((edge, previous, node));
            node = previous;
        }
        hops.reverse();
        let target = hops
            .last()
            .and_then(|(edge, _, _)| edges[*edge].target)
            .ok_or_else(|| "the walk across levels reaches no target".to_owned())?;
        let mut waypoints = vec![origin.clone()];
        let mut traversed: Vec<ObjectId> = Vec::new();
        let push = |id: &ObjectId, traversed: &mut Vec<ObjectId>| {
            if traversed.last() != Some(id) {
                traversed.push(id.clone());
            }
        };
        for (edge, from_node, to_node) in &hops {
            let edge = &edges[*edge];
            match edge.stage {
                None => {
                    let Node::Landing(link, end) = nodes[*to_node].0 else {
                        return Err("a climb ends off a landing".into());
                    };
                    let climb = &tour.links[link];
                    push(&climb.climb.id, &mut traversed);
                    waypoints.push(landing_point(climb, end)?);
                }
                Some(stage) => {
                    let level = &tour.stages[stage].level;
                    let mut path = edge
                        .path
                        .clone()
                        .ok_or_else(|| "a level walk has no path".to_owned())?;
                    if edge.a != *from_node {
                        path.reverse();
                    }
                    for pair in path.windows(2) {
                        if let Some(floor) = holder(prepared, level, pair[0]) {
                            push(&floor.id, &mut traversed);
                        }
                        for (frame, _, _) in &prepared.portals {
                            if crosses_mid_line(frame, pair[0], pair[1]) {
                                push(&frame.id, &mut traversed);
                            }
                        }
                        if let Some(floor) = holder(prepared, level, pair[1]) {
                            push(&floor.id, &mut traversed);
                        }
                    }
                    for point in path.iter().skip(1).take(path.len().saturating_sub(2)) {
                        let floor = holder(prepared, level, *point)
                            .unwrap_or(&prepared.floors[*level.floors.first().unwrap_or(&0)]);
                        waypoints.push(
                            MetricPoint::try_new(floor.id.clone(), [point.x, point.y, floor.z0])
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    if *to_node == sink {
                        waypoints.push(targets[target].clone());
                    } else if let Node::Landing(link, end) = nodes[*to_node].0 {
                        waypoints.push(landing_point(&tour.links[link], end)?);
                    }
                }
            }
        }
        if traversed.is_empty() {
            traversed.push(origin.subject().clone());
        }
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        Ok(Walked::Reached {
            target,
            distance,
            waypoints,
            traversed,
            locator: format!(
                "{text}:targets={}:placed={}:radius={radius:.6}:step={step:.6}:\
                 height={height:.6}:lower={}:upper={upper:.6}:witness={}",
                targets.len(),
                placed.on.iter().map(Vec::len).sum::<usize>(),
                if closed {
                    format!("level-graph={:.6}", distance.lower_metres())
                } else {
                    format!("straight-line={:.6}", distance.lower_metres())
                },
                if radius <= 0.0 {
                    "point-path"
                } else {
                    "sweep=proven"
                },
            ),
        })
    }

    pub(super) fn nearest_across(
        &self,
        request: &NearestTargetRequest,
        routing: &ConnectorRouting,
    ) -> Result<NearestTargetOutcome, String> {
        match self.walk_across(
            request.origin(),
            request.targets(),
            request.profile(),
            request.avoided(),
            routing,
            request.costs(),
        )? {
            Walked::Reached {
                target,
                distance,
                waypoints,
                locator,
                ..
            } => Ok(NearestTargetOutcome::Reached(
                NearestTargetEvidence::try_new(
                    target,
                    distance,
                    waypoints,
                    Evidence::exact(
                        request.origin().subject().source.clone(),
                        format!("axiolid:metric-route:nearest:{locator}"),
                    ),
                )
                .map_err(|e| e.to_string())?,
            )),
            Walked::Unreachable(reason) => {
                let completeness = self.completeness(format!(
                    "axiolid:metric-route:nearest:unreachable:{reason}:targets={}",
                    request.targets().len()
                ))?;
                Ok(NearestTargetOutcome::Unreachable(
                    UnreachableTargetsEvidence::new(request.clone(), completeness),
                ))
            }
        }
    }

    pub(super) fn route_across(
        &self,
        request: &MetricRouteRequest,
        routing: &ConnectorRouting,
    ) -> Result<MetricRouteOutcome, String> {
        match self.walk_across(
            request.origin(),
            std::slice::from_ref(request.destination()),
            request.profile(),
            &[],
            routing,
            &[],
        )? {
            Walked::Reached {
                distance,
                waypoints,
                traversed,
                locator,
                ..
            } => Ok(MetricRouteOutcome::Reachable(
                MetricRouteEvidence::try_new(
                    distance,
                    waypoints,
                    traversed,
                    Evidence::exact(
                        request.origin().subject().source.clone(),
                        format!("axiolid:metric-route:reachable:{locator}"),
                    ),
                )
                .map_err(|e| e.to_string())?,
            )),
            Walked::Unreachable(reason) => {
                let completeness =
                    self.completeness(format!("axiolid:metric-route:blocked:{reason}"))?;
                Ok(MetricRouteOutcome::Blocked(
                    axioval_engine::BlockedMetricRouteEvidence::new(request.clone(), completeness),
                ))
            }
        }
    }

    /// The farthest point of a region from the nearest target, walking
    /// through the request's connectors. For a point only.
    #[allow(clippy::too_many_lines)]
    pub(super) fn farthest_across(
        &self,
        request: &FarthestPointRequest,
        routing: &ConnectorRouting,
    ) -> Result<FarthestPointOutcome, String> {
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
        let costs = request.costs();
        let costed = self.costed(prepared, costs)?;
        let tour = self.tour(
            prepared,
            routing,
            &[],
            BTreeSet::from([region]),
            &obstacles,
            profile,
            &costed,
        )?;
        let targets = request.targets();
        let placed = Self::place_all(prepared, &tour, targets, step);
        let nodes = Self::nodes(&tour, None);
        let edges = Self::edges(prepared, &tour, &nodes, targets, &placed, radius)?;
        let sink = nodes.len();
        // Each landing's walk beyond it, from the sink back.
        let (beyond_lower, _) = shortest(nodes.len() + 1, &edges, sink, |edge| edge.lower);
        let (beyond_upper, _) = shortest(nodes.len() + 1, &edges, sink, |edge| edge.upper);
        let mut closed = tour.closed() && placed.straight.is_empty();
        let stage = &tour.stages[0];
        let text = tour.text(routing, radius, costs);
        let why = || {
            let mut reasons: Vec<String> = placed
                .straight
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect();
            reasons.extend(tour.incomplete());
            reasons.join("; ")
        };
        let at = |point: Point2| {
            MetricPoint::try_new(region_id.clone(), [point.x, point.y, floor.z0])
                .map_err(|e| e.to_string())
        };
        let unreachable = |point: Point2, reason: &str| -> Result<FarthestPointOutcome, String> {
            let completeness = self.completeness(format!(
                "axiolid:metric-route:farthest:unreachable:{reason}:region={region_id}:{text}:\
                 targets={}:step={step:.6}:height={height:.6}:witness=({:.6},{:.6})",
                targets.len(),
                point.x,
                point.y,
            ))?;
            Ok(FarthestPointOutcome::Unreachable(
                UnreachableRegionEvidence::new(request.clone(), at(point)?, completeness),
            ))
        };
        // The sources on the region's level: its targets, at no weight,
        // and each landing with a walk beyond it.
        let mut sources: Vec<Source> = Vec::new();
        if !placed.on[0].is_empty() {
            sources.push((
                placed.on[0]
                    .iter()
                    .map(|index| plan(&targets[*index]))
                    .collect(),
                0.0,
                0.0,
            ));
        }
        for (index, (node, stage_index, point)) in nodes.iter().enumerate() {
            if *stage_index == 0
                && matches!(node, Node::Landing(..))
                && beyond_lower[index].is_finite()
            {
                sources.push((vec![*point], beyond_lower[index], beyond_upper[index]));
            }
        }
        let free = intersect(&floor.footprint, &stage.domain)?;
        if sources.is_empty() {
            if closed {
                let point = inner_point(&free)
                    .ok_or_else(|| format!("{region_id} has no free walkable area"))?;
                return unreachable(point, "targets-off-reached-levels");
            }
            return Err(format!(
                "no target is reachable from {region_id}'s level through the connectors: {}",
                why()
            ));
        }
        let tolerance = request.tolerance_metres();
        // The weighted bracket is first order in the spacing along cost
        // edges, about twice it wide.
        let wanted = tolerance / 2.0;
        let mut maps = Vec::with_capacity(sources.len());
        for (points, _, _) in &sources {
            let seeded: Vec<(Point2, f64)> = points.iter().map(|point| (*point, 0.0)).collect();
            let map = Self::stage_map(stage, &seeded, wanted)?;
            if map.is_none() {
                // A source the map cannot hold may still be walked to.
                closed = false;
            }
            maps.push(map);
        }
        // One map over every source the stage holds, each point starting
        // at its source's weight (axiolid/kernel#197, weighted by the
        // level's costs #198): the walk beyond it bounded from below, and,
        // where that bound is not already the upper one, from above (a
        // source with no upper bound left out, which only lengthens the
        // walks).
        let weighted = |weight: &dyn Fn(&Source) -> f64| -> Result<Option<Map>, String> {
            let seeded: Vec<(Point2, f64)> = sources
                .iter()
                .zip(&maps)
                .filter(|(source, map)| map.is_some() && weight(source).is_finite())
                .flat_map(|(source, _)| source.0.iter().map(|point| (*point, weight(source))))
                .collect();
            if seeded.is_empty() {
                return Ok(None);
            }
            Self::stage_map(stage, &seeded, wanted)
        };
        #[allow(clippy::float_cmp)]
        let settled = sources.iter().all(|(_, low, high)| low == high);
        let least = weighted(&|(_, low, _)| *low)?;
        let most = if settled {
            None
        } else {
            weighted(&|(_, _, high)| *high)?
        };
        let search = |map: &Map| -> Result<Result<(f64, f64, Point2), Point2>, String> {
            let mut best: Option<(f64, Point2)> = None;
            let mut upper = 0.0_f64;
            for piece in floor.footprint.polygons() {
                match map.farthest(piece, tolerance) {
                    Ok(found) => {
                        upper = upper.max(found.distance.upper);
                        let Some(witness) = found.witness else {
                            return Err(format!(
                                "the farthest point of {region_id} was bounded without \
                                 sampling any point of it"
                            ));
                        };
                        if best.is_none_or(|(lower, _)| found.distance.lower > lower) {
                            best = Some((found.distance.lower, witness));
                        }
                    }
                    Err(FarthestError::Empty) => {}
                    Err(FarthestError::Unreachable { triangle }) => {
                        let cell = polygon(triangle.to_vec())
                            .ok_or_else(|| "an unreachable cell is degenerate".to_owned())?;
                        let part = intersect(&Plan::piece(cell), &Plan::piece(piece.clone()))?;
                        let point = inner_point(&part).ok_or_else(|| {
                            format!(
                                "part of {region_id} reaches no source, but no point of it \
                                 could be placed inside the region"
                            )
                        })?;
                        return Ok(Err(point));
                    }
                    Err(error) => {
                        return Err(format!(
                            "the farthest point of {region_id} could not be bracketed: \
                             {error:?}"
                        ));
                    }
                }
            }
            best.map(|(lower, witness)| Ok((lower, upper, witness)))
                .ok_or_else(|| format!("{region_id} has no free walkable area"))
        };
        let mut upper = f64::INFINITY;
        let mut candidates: Vec<Point2> = Vec::new();
        let mut floor_bound: Option<(f64, Point2)> = None;
        if let Some(map) = &least {
            match search(map)? {
                Ok((lower, high, witness)) => {
                    candidates.push(witness);
                    if settled {
                        upper = upper.min(high);
                    }
                    floor_bound = Some((lower, witness));
                }
                Err(point) => {
                    if closed {
                        return unreachable(point, "cut-off");
                    }
                    return Err(format!(
                        "part of {region_id} reaches no connector or target on its level, and \
                         the levels are not closed: {}",
                        why()
                    ));
                }
            }
        }
        if let Some(map) = &most
            && let Ok((_, far, witness)) = search(map)?
        {
            upper = upper.min(far);
            candidates.push(witness);
        }
        if least.is_none() || (!settled && most.is_none()) {
            // No weighted map: each source's own farthest distance plus
            // its weight still bounds the largest from above.
            for ((_, _, high), map) in sources.iter().zip(&maps) {
                let Some(map) = map else { continue };
                if let Ok((_, far, witness)) = search(map)? {
                    upper = upper.min(far + high);
                    candidates.push(witness);
                }
            }
        }
        if upper.is_infinite() {
            return Err(format!(
                "the farthest point of {region_id} could not be bounded through the \
                 connectors: {}",
                why()
            ));
        }
        // The lower bound: the least walk from a candidate point, over
        // every source.
        let Some(first) = candidates.first().copied() else {
            return Err(format!(
                "no point of {region_id} was sampled through the connectors"
            ));
        };
        let (lower, witness) = if closed {
            let mut best = floor_bound.unwrap_or((0.0, first));
            for point in &candidates {
                let mut value = f64::INFINITY;
                for ((points, low, _), map) in sources.iter().zip(&maps) {
                    // A source the map cannot show reached counts by its
                    // straight line, never as out of reach.
                    let walked = match map.as_ref().map(|map| map.nearest(*point)) {
                        Some(Ok(Ok(reach))) => reach.lower * (1.0 - LENGTH_ROUNDING),
                        _ => points
                            .iter()
                            .map(|source| (*source - *point).length())
                            .fold(f64::INFINITY, f64::min),
                    };
                    value = value.min(walked + low);
                }
                if value.is_finite() && value > best.0 {
                    best = (value, *point);
                }
            }
            best
        } else {
            let witness = first;
            let straight = targets
                .iter()
                .map(|target| (plan(target) - witness).length())
                .fold(f64::INFINITY, f64::min);
            (straight, witness)
        };
        let upper = rounded_up(upper);
        let distance =
            LengthInterval::try_new(lower.min(upper), upper).map_err(|e| e.to_string())?;
        let converged = upper - distance.lower_metres() <= tolerance;
        let evidence = Evidence::exact(
            region_id.source.clone(),
            format!(
                "axiolid:metric-route:farthest:region={region_id}:{text}:targets={}:\
                 sources={}:step={step:.6}:height={height:.6}:tolerance={tolerance:.6}:lower={}:\
                 upper={upper:.6}:witness=({:.6},{:.6})",
                targets.len(),
                sources.len(),
                if closed {
                    format!("level-graph={:.6}", distance.lower_metres())
                } else {
                    format!("straight-line={:.6}", distance.lower_metres())
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
}

/// The one floor a landing at `at` and elevation `z` stands on: its plan
/// inside the floor's footprint, its elevation within a step of the floor.
fn ground(prepared: &Prepared, at: Point2, z: f64, step: f64) -> Result<usize, String> {
    let holding: Vec<usize> = prepared
        .floors
        .iter()
        .enumerate()
        .filter(|(_, floor)| {
            (z - floor.z0).abs() <= step + ON_SURFACE && contains(&floor.footprint, at)
        })
        .map(|(index, _)| index)
        .collect();
    match holding.as_slice() {
        [single] => Ok(*single),
        [] if prepared.gaps.is_empty() => Err("it lies on no walkable surface".into()),
        [] => Err(format!(
            "it lies on no measured walkable surface, and some were not measured: {}",
            prepared.gaps.join("; ")
        )),
        _ => Err("it lies on more than one walkable surface".into()),
    }
}

/// A landing as a waypoint, grounded on its connector.
fn landing_point(link: &Link, end: usize) -> Result<MetricPoint, String> {
    let at = link.landings[end];
    MetricPoint::try_new(link.climb.id.clone(), [at.x, at.y, link.climb.ends[end].z])
        .map_err(|e| e.to_string())
}

/// Shortest distances from `source` over undirected edges weighted by
/// `weight` (infinite weights are no edge), and the edge each node was
/// reached by.
fn shortest(
    count: usize,
    edges: &[Edge],
    source: usize,
    weight: impl Fn(&Edge) -> f64,
) -> (Vec<f64>, Vec<Option<usize>>) {
    let mut distance = vec![f64::INFINITY; count];
    let mut via = vec![None; count];
    let mut done = vec![false; count];
    distance[source] = 0.0;
    loop {
        let next = (0..count)
            .filter(|node| !done[*node] && distance[*node].is_finite())
            .min_by(|a, b| distance[*a].total_cmp(&distance[*b]));
        let Some(node) = next else { break };
        done[node] = true;
        for (index, edge) in edges.iter().enumerate() {
            let w = weight(edge);
            if !w.is_finite() {
                continue;
            }
            let other = if edge.a == node {
                edge.b
            } else if edge.b == node {
                edge.a
            } else {
                continue;
            };
            let candidate = distance[node] + w;
            if candidate < distance[other] {
                distance[other] = candidate;
                via[other] = Some(index);
            }
        }
    }
    (distance, via)
}
