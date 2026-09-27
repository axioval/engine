//! Passing spaces along an accessible route: a free box at most every
//! `passing_spacing_metres` of the route, decided by placement searches in
//! the route spaces along the route's segments.
//!
//! The route is the one the metric-routing service walks from the start's
//! representative point to the destination's, a polyline measured in plan.
//! Its ends count as passing spaces. A passing space stands at a position of
//! the route when its centre lies across the route from that position,
//! within `passing_reach_metres` of it, with its length along the segment.
//!
//! The route is cut into equal tiles no longer than half the spacing (less a
//! slop), and each tile is searched segment piece by segment piece with a
//! frame-offset domain anchored on the segment. A witness in every tile but
//! the two at the ends leaves no gap longer than the spacing. Consecutive
//! tiles proven empty that together are longer than the spacing prove a gap
//! that long. Anything else is undecided.

use std::collections::BTreeSet;

use axioval_engine::{
    BoxClearance, FrameOffsetPlacement, FreeSpaceError, FreeSpaceServiceHandle, MetricDirection,
    MetricFrame, MetricPoint, MetricRouteOutcome, MetricRouteRequest, MetricRoutingServiceHandle,
    MobilityProfile, NotEvaluatedReason, ParameterDescriptor, ParameterType, PlacementDomain,
    PlacementOrientation, PlacementOutcome, PlacementRequest, PlacementShape, RuleContext,
    SignedDistanceInterval,
};
use axioval_ir::{Evidence, ObjectId};

use crate::level_spacing::metres;
use crate::space_distance::representative_point;
use crate::support::{Parameters, Unavailable, invalid};

/// Widens every searched piece of the route by this much at both ends, so
/// that no piece is too short to search; the tiles are shortened by it.
const SLOP: f64 = 1.0e-3;

/// Keeps a segment anchor's floor within this of the waypoint's elevation.
const FLOOR_MARGIN: f64 = 1.0e-3;

/// The optional parameters passing spaces take.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("passing_width_metres", ParameterType::Number),
        ParameterDescriptor::optional("passing_length_metres", ParameterType::Number),
        ParameterDescriptor::optional("passing_spacing_metres", ParameterType::Number),
        ParameterDescriptor::optional("passing_reach_metres", ParameterType::Number),
    ]
}

/// The passing spaces a rule requires.
pub(crate) struct PassingSpaces {
    width: f64,
    length: f64,
    height: f64,
    spacing: f64,
    reach: f64,
}

impl PassingSpaces {
    /// Reads the declaration; `height` is the route's clear height, which a
    /// passing space needs.
    pub(crate) fn parse(
        parameters: &Parameters<'_>,
        height: Option<f64>,
    ) -> Result<Option<Self>, Unavailable> {
        let read = |name: &str| match parameters.number(name)? {
            Some(value) if value.is_finite() && value > 0.0 => Ok(Some(value)),
            Some(_) => Err(invalid(format!("`{name}` must be positive"))),
            None => Ok(None),
        };
        let (width, length, spacing, reach) = (
            read("passing_width_metres")?,
            read("passing_length_metres")?,
            read("passing_spacing_metres")?,
            read("passing_reach_metres")?,
        );
        let (width, length, spacing) = match (width, length, spacing) {
            (Some(width), Some(length), Some(spacing)) => (width, length, spacing),
            (None, None, None) if reach.is_none() => return Ok(None),
            _ => {
                return Err(invalid(
                    "`passing_width_metres`, `passing_length_metres` and \
                     `passing_spacing_metres` go together",
                ));
            }
        };
        let height = height.ok_or_else(|| {
            invalid("passing spaces need `clear_height_metres`, the height they must be free to")
        })?;
        if spacing <= 4.0 * SLOP {
            return Err(invalid("`passing_spacing_metres` is too short to search"));
        }
        Ok(Some(Self {
            width,
            length,
            height,
            spacing,
            reach: reach.unwrap_or(width / 2.0),
        }))
    }

    fn describe(&self) -> String {
        format!("{} by {}", metres(self.width), metres(self.length))
    }
}

/// The services passing spaces need.
pub(crate) struct Services<'a> {
    context: &'a RuleContext<'a>,
    routes: &'a MetricRoutingServiceHandle,
    free_space: &'a FreeSpaceServiceHandle,
}

impl<'a> Services<'a> {
    pub(crate) fn of(context: &'a RuleContext<'a>) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("passing spaces need the {what} service"),
            )
        };
        Ok(Self {
            context,
            routes: context
                .services
                .get::<MetricRoutingServiceHandle>()
                .ok_or_else(|| missing("metric-routing"))?,
            free_space: context
                .services
                .get::<FreeSpaceServiceHandle>()
                .ok_or_else(|| missing("free-space"))?,
        })
    }
}

/// How one route stands.
pub(crate) enum Spacing {
    /// No stretch longer than the spacing lacks a passing space.
    Met,
    /// A stretch this long is proven to lack one: the message and evidence.
    Missed(String, Vec<Evidence>),
    /// Neither is proven.
    Unknown(NotEvaluatedReason, String),
}

/// What the route may cross, and what obstructs a passing space.
pub(crate) struct Ground<'a> {
    /// Surfaces a passing space may stand on: the route spaces, the start
    /// and the destination.
    pub(crate) spaces: &'a BTreeSet<ObjectId>,
    /// Portals the route may pass.
    pub(crate) portals: &'a BTreeSet<ObjectId>,
    pub(crate) obstacles: &'a [ObjectId],
    /// The body's width, which the route is walked for.
    pub(crate) body: f64,
}

/// A route segment in plan.
struct Segment {
    /// Start point, on the floor.
    start: [f64; 3],
    /// Unit direction in plan.
    direction: [f64; 2],
    /// Plan length and the route length at its start.
    length: f64,
    at: f64,
}

/// Whether the route from `from` to `to` has its passing spaces.
pub(crate) fn judge(
    passing: &PassingSpaces,
    services: &Services<'_>,
    ground: &Ground<'_>,
    from: &ObjectId,
    to: &ObjectId,
) -> Spacing {
    match walk(passing, services, ground, from, to) {
        Ok(spacing) => spacing,
        Err((reason, message)) => {
            Spacing::Unknown(reason, format!("on the route from {from}, {message}"))
        }
    }
}

#[allow(clippy::too_many_lines)]
fn walk(
    passing: &PassingSpaces,
    services: &Services<'_>,
    ground: &Ground<'_>,
    from: &ObjectId,
    to: &ObjectId,
) -> Result<Spacing, Unavailable> {
    let incomplete = |message: String| (NotEvaluatedReason::IncompleteEvidence, message);
    let (origin, mut evidence) = representative_point(services.context, from)?;
    let (destination, cited) = representative_point(services.context, to)?;
    evidence.extend(cited);
    let profile = MobilityProfile::try_new(ground.body / 2.0, passing.height, 0.0, 0.0)
        .map_err(|error| invalid(format!("walking profile: {error}")))?;
    let route = match services
        .routes
        .route(&MetricRouteRequest::new(origin, destination, profile))
    {
        Ok(MetricRouteOutcome::Reachable(route)) => route,
        Ok(MetricRouteOutcome::Blocked(_)) => {
            return Err(incomplete(
                "the metric route is blocked where the walkability snapshot proves one".into(),
            ));
        }
        Err(error) => return Err(incomplete(format!("no route is measured: {error}"))),
    };
    evidence.push(route.evidence().clone());
    let mut scopes: Vec<ObjectId> = Vec::new();
    for object in route.traversed_objects() {
        if ground.spaces.contains(object) {
            if !scopes.contains(object) {
                scopes.push(object.clone());
            }
        } else if !ground.portals.contains(object) {
            return Err(incomplete(format!(
                "the measured route crosses {object}, which is neither a route space nor a portal"
            )));
        }
    }
    let Some((scope, merged)) = scopes.split_first() else {
        return Err(incomplete(
            "the measured route crosses no route space".into(),
        ));
    };
    let segments = segments(route.waypoints());
    let total = segments
        .last()
        .map_or(0.0, |segment| segment.at + segment.length);
    // The ends count as passing spaces.
    if total <= passing.spacing {
        return Ok(Spacing::Met);
    }
    let usable = passing.spacing / 2.0 - SLOP;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let (tiles, width) = {
        let tiles = (total / usable).ceil().max(1.0) as usize;
        (tiles, total / tiles as f64)
    };
    let obstacles: Vec<ObjectId> = ground
        .obstacles
        .iter()
        .filter(|id| !scopes.contains(id))
        .cloned()
        .collect();
    let search = Search {
        passing,
        services,
        scope,
        merged,
        obstacles: &obstacles,
    };
    let mut states = Vec::with_capacity(tiles);
    for tile in 0..tiles {
        #[allow(clippy::cast_precision_loss)]
        let (low, high) = (tile as f64 * width, (tile + 1) as f64 * width);
        states.push(search.tile(&segments, low, high));
    }
    // Tiles are shorter than half the spacing and the route is longer, so
    // there are at least three.
    let interior = &states[1..tiles - 1];
    if interior.iter().all(|state| matches!(state, Tile::Found)) {
        return Ok(Spacing::Met);
    }
    // The longest run of tiles proven empty.
    let mut best: Option<(usize, usize)> = None;
    let mut start = None;
    for (index, state) in states.iter().enumerate() {
        match (state, start) {
            (Tile::Empty(_), None) => start = Some(index),
            (Tile::Empty(_), Some(_)) | (_, None) => {}
            (_, Some(first)) => {
                best = Some(longer(best, (first, index)));
                start = None;
            }
        }
    }
    if let Some(first) = start {
        best = Some(longer(best, (first, tiles)));
    }
    #[allow(clippy::cast_precision_loss)]
    if let Some((first, end)) = best
        && (end - first) as f64 * width > passing.spacing
    {
        for state in &states[first..end] {
            if let Tile::Empty(proofs) = state {
                evidence.extend(proofs.iter().cloned());
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let (low, high) = (first as f64 * width, end as f64 * width);
        return Ok(Spacing::Missed(
            format!(
                "the route from {from} has no passing space ({}) between {} and {} along it, \
                 a stretch longer than the {} allowed",
                passing.describe(),
                metres(low),
                metres(high),
                metres(passing.spacing)
            ),
            evidence,
        ));
    }
    let reasons: BTreeSet<String> = states
        .into_iter()
        .filter_map(|state| match state {
            Tile::Unknown(reason) => Some(reason),
            _ => None,
        })
        .collect();
    let mut message = format!(
        "no passing space ({}) is proven at most every {}, and no longer gap is proven",
        passing.describe(),
        metres(passing.spacing)
    );
    if !reasons.is_empty() {
        message.push_str(": ");
        message.push_str(&reasons.into_iter().collect::<Vec<_>>().join("; "));
    }
    Err(incomplete(message))
}

fn longer(best: Option<(usize, usize)>, run: (usize, usize)) -> (usize, usize) {
    match best {
        Some(best) if best.1 - best.0 >= run.1 - run.0 => best,
        _ => run,
    }
}

/// The route's segments with a plan length, in order.
fn segments(waypoints: &[MetricPoint]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut at = 0.0;
    for pair in waypoints.windows(2) {
        let (a, b) = (pair[0].coordinates_metres(), pair[1].coordinates_metres());
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        if length <= f64::EPSILON {
            continue;
        }
        segments.push(Segment {
            start: a,
            direction: [dx / length, dy / length],
            length,
            at,
        });
        at += length;
    }
    segments
}

/// What a tile holds.
enum Tile {
    Found,
    /// Proven empty: the proofs.
    Empty(Vec<Evidence>),
    Unknown(String),
}

struct Search<'a> {
    passing: &'a PassingSpaces,
    services: &'a Services<'a>,
    scope: &'a ObjectId,
    merged: &'a [ObjectId],
    obstacles: &'a [ObjectId],
}

impl Search<'_> {
    /// Searches the route between `low` and `high` along it, piece by
    /// segment piece.
    fn tile(&self, segments: &[Segment], low: f64, high: f64) -> Tile {
        let mut proofs = Vec::new();
        let mut unknown = None;
        for segment in segments {
            let (from, to) = (
                (low - segment.at).max(0.0),
                (high - segment.at).min(segment.length),
            );
            if to <= from {
                continue;
            }
            match self.piece(segment, from - SLOP, to + SLOP) {
                Ok(PlacementOutcome::Found(_)) => return Tile::Found,
                Ok(PlacementOutcome::NoPlacement(proof)) => proofs.push(proof.evidence().clone()),
                Err(message) => unknown = Some(message),
            }
        }
        match unknown {
            Some(message) => Tile::Unknown(message),
            None => Tile::Empty(proofs),
        }
    }

    fn piece(&self, segment: &Segment, from: f64, to: f64) -> Result<PlacementOutcome, String> {
        let error = |error: FreeSpaceError| error.to_string();
        let [dx, dy] = segment.direction;
        let forward = MetricDirection::try_new([dx, dy, 0.0]).map_err(error)?;
        let right = MetricDirection::try_new([dy, -dx, 0.0]).map_err(error)?;
        let up = MetricDirection::try_new([0.0, 0.0, 1.0]).map_err(error)?;
        let origin = MetricPoint::try_new(self.scope.clone(), segment.start)
            .map_err(|error| error.to_string())?;
        let anchor = MetricFrame::try_new(origin, right, forward, up).map_err(error)?;
        let reach = self.passing.reach;
        let offsets = FrameOffsetPlacement::new(
            anchor.clone(),
            SignedDistanceInterval::try_new(-reach, reach).map_err(error)?,
            SignedDistanceInterval::try_new(from, to).map_err(error)?,
            SignedDistanceInterval::try_new(-FLOOR_MARGIN, FLOOR_MARGIN).map_err(error)?,
        );
        let shape = PlacementShape::Box {
            shape: BoxClearance::try_new(
                self.passing.width,
                self.passing.length,
                self.passing.height,
            )
            .map_err(error)?,
            orientation: PlacementOrientation::Fixed(anchor),
        };
        let request = PlacementRequest::new_in_domain(
            self.scope.clone(),
            shape,
            self.obstacles.to_vec(),
            PlacementDomain::FrameOffsets(offsets),
        )
        .and_then(|request| request.with_merged_scopes(self.merged.to_vec()))
        .map_err(error)?;
        self.services
            .free_space
            .find_placement(&request)
            .map_err(error)
    }
}
