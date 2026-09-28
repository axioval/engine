//! Weighted travel and walks forced through an object, on one level.
//!
//! A request's [`TravelCost`]s become `axiolid-route` cost regions
//! (axiolid/kernel#195): each costed object's exact plan footprint, cut to
//! the level's free region so that no cost edge crosses an obstacle. A
//! tessellated object's footprint is only known as a box around it, which
//! bounds its cost from one side only, so such a request is refused. The
//! weighted map brackets the least weighted cost to first order in the
//! spacing of the points along cost edges; the spacing is chosen so that
//! the points stay within the kernel's node budget.
//!
//! A forced walk (axiolid/kernel#196) brackets the shortest walk from the
//! origin to a target that enters an object's plan footprint, from a map
//! out of the origin and one out of the targets over the same free
//! region. Its lower bound needs every shortcut and every target known, so
//! it is answered only on a closed level with every target placed; its
//! upper bound only for a point and an exact footprint.

use axiolid_core::Point2;
use axiolid_overlay::Polygon;
use axiolid_route::{CostRegion, DistanceMap, Farthest, FarthestError, WeightedMap};
use axioval_engine::{
    ForcedWalkEvidence, ForcedWalkOutcome, ForcedWalkRequest, NeverEnteredEvidence, TravelCost,
};
use axioval_ir::Evidence;

use super::{AxiolidMetricRoutingService, LENGTH_ROUNDING, Prepared, plan, rounded_up};
use crate::walkable::{Plan, intersect};

/// The finest spacing of points along cost edges, in metres.
const FINEST_SPACING: f64 = 0.005;

/// How many points along cost edges a map may place at most; the rest of
/// the kernel's node budget is left for the region's vertices.
const COST_POINTS: f64 = 1024.0;

/// The distance map a query walks: plain, or weighted by cost regions.
// One map lives per query, never in a collection: its size is no cost.
#[allow(clippy::large_enum_variant)]
pub(super) enum Map {
    Plain(DistanceMap),
    Weighted(WeightedMap, f64),
}

/// The walk a map answers from a point: the placed target's index, the
/// path, and the bracket on its (weighted) length.
pub(super) struct Reached {
    pub(super) target: usize,
    pub(super) path: Vec<Point2>,
    pub(super) lower: f64,
    pub(super) upper: f64,
}

impl Map {
    /// The nearest target from `from`, or why none is reached.
    pub(super) fn nearest(
        &self,
        from: Point2,
    ) -> Result<Result<Reached, axiolid_route::Unreachable>, String> {
        match self {
            Self::Plain(map) => map
                .nearest(from)
                .map(|reach| {
                    reach.map(|reach| {
                        let length = super::length(&reach.route.polyline);
                        Reached {
                            target: reach.target,
                            path: reach.route.polyline,
                            lower: reach.route.length,
                            upper: length,
                        }
                    })
                })
                .map_err(|error| format!("the nearest target was not found: {error:?}")),
            Self::Weighted(map, _) => map
                .nearest(from)
                .map(|reach| {
                    reach.map(|reach| Reached {
                        target: reach.target,
                        path: reach.route.polyline,
                        lower: reach.cost.lower,
                        upper: reach.cost.upper,
                    })
                })
                .map_err(|error| format!("the nearest target was not found: {error:?}")),
        }
    }

    /// The farthest (weighted) distance over one piece of a region.
    pub(super) fn farthest(
        &self,
        piece: &Polygon,
        tolerance: f64,
    ) -> Result<Farthest, FarthestError> {
        match self {
            Self::Plain(map) => axiolid_route::farthest_point(map, piece, tolerance),
            Self::Weighted(map, _) => axiolid_route::weighted_farthest_point(map, piece, tolerance),
        }
    }

    /// The spacing along cost edges, for evidence; `None` for a plain map.
    pub(super) fn spacing(&self) -> Option<f64> {
        match self {
            Self::Plain(_) => None,
            Self::Weighted(_, spacing) => Some(*spacing),
        }
    }
}

/// Names the costs for evidence: `object*factor`, comma-separated.
pub(super) fn costs_text(costs: &[TravelCost]) -> String {
    costs
        .iter()
        .map(|cost| format!("{}*{}", cost.object(), cost.factor()))
        .collect::<Vec<_>>()
        .join(",")
}

/// A polygon's boundary length, holes included.
fn perimeter(polygon: &Polygon) -> f64 {
    std::iter::once(&polygon.outer)
        .chain(&polygon.holes)
        .map(|ring| {
            let points = &ring.points;
            (0..points.len())
                .map(|i| (points[(i + 1) % points.len()] - points[i]).length())
                .sum::<f64>()
        })
        .sum()
}

impl AxiolidMetricRoutingService {
    /// The cost regions of `costs` within `domain`: each object's exact
    /// footprint cut to the free region. A tessellated or bodiless object
    /// is refused, since its cost could then be missed or overstated.
    pub(super) fn cost_regions(
        &self,
        prepared: &Prepared,
        costs: &[TravelCost],
        domain: &Plan,
    ) -> Result<Vec<CostRegion>, String> {
        let mut regions = Vec::new();
        for cost in costs {
            let (footprint, exact) = self.footprint(prepared, cost.object())?;
            if !exact {
                return Err(format!(
                    "{} is tessellated: its footprint is known only as a box around it, which \
                     would overstate its cost",
                    cost.object()
                ));
            }
            if footprint.is_empty() {
                return Err(format!("{} has no plan footprint to weigh", cost.object()));
            }
            let free = intersect(&footprint, domain)?;
            regions.extend(
                free.polygons()
                    .iter()
                    .map(|piece| CostRegion::new(piece.clone(), cost.factor())),
            );
        }
        Ok(regions)
    }

    /// The map a query walks: weighted when any cost region lies in the
    /// free region, with points along cost edges `wanted` apart or as
    /// close as the node budget allows.
    pub(super) fn weighed_map(
        domain: &Plan,
        barriers: &[Vec<Point2>],
        points: &[Point2],
        regions: &[CostRegion],
        wanted: f64,
    ) -> Result<Map, String> {
        let describe = |error: axiolid_route::MapError| match error {
            axiolid_route::MapError::CostCrossing { .. } => format!(
                "the costed footprints cannot be weighed: one crosses another or runs along a \
                 narrow portal's mid-line ({error:?})"
            ),
            other => format!("the weighted distance map could not be built: {other:?}"),
        };
        if regions.is_empty() {
            return axiolid_route::distance_map_within(
                domain.polygons(),
                barriers,
                points,
                crate::walkable::ROUTE_BUDGET,
            )
            .map(Map::Plain)
            .map_err(|error| format!("the distance map could not be built: {error:?}"));
        }
        let boundary: f64 = regions
            .iter()
            .map(|region| perimeter(&region.polygon))
            .sum();
        let spacing = wanted.max(FINEST_SPACING).max(boundary / COST_POINTS);
        axiolid_route::weighted_distance_map(domain.polygons(), barriers, points, regions, spacing)
            .map(|map| Map::Weighted(map, spacing))
            .map_err(describe)
    }

    /// Brackets the shortest walk from the origin to a target that enters
    /// the requested object.
    #[allow(clippy::too_many_lines)]
    pub(super) fn forced(&self, request: &ForcedWalkRequest) -> Result<ForcedWalkOutcome, String> {
        if request.connectors().is_some() {
            return Err("walks forced through an object are measured on one level only".into());
        }
        let profile = request.profile();
        let (radius, step, height) = (
            profile.radius_metres(),
            profile.maximum_step_metres(),
            profile.height_metres(),
        );
        if height <= step {
            return Err("the clear height does not exceed the maximum step".into());
        }
        let prepared = self.prepared();
        let mut obstacles = self.obstacles()?;
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
        if !level.complete() || !sorted.straight.is_empty() {
            let mut reasons: Vec<String> = sorted
                .straight
                .iter()
                .map(|(_, reason)| reason.clone())
                .collect();
            reasons.extend(level.incomplete.iter().cloned());
            return Err(format!(
                "a walk forced through {} is bounded only on a closed level with every target \
                 placed: {}",
                request.through(),
                reasons.join("; ")
            ));
        }
        let (footprint, exact) = self.footprint(prepared, request.through())?;
        if footprint.is_empty() {
            return Err(format!(
                "{} has no plan footprint to enter",
                request.through()
            ));
        }
        let avoided = request
            .avoided()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let text = format!(
            "level=[{}]:through={}:targets={}:placed={}:avoided=[{avoided}]:radius={radius:.6}:\
             step={step:.6}:height={height:.6}",
            level.text,
            request.through(),
            targets.len(),
            sorted.placed.len(),
        );
        let never = |reason: &str| -> Result<ForcedWalkOutcome, String> {
            let completeness = self.completeness(format!(
                "axiolid:metric-route:forced:never:{reason}:{text}:surfaces={}:portals={}:\
                 obstacles={}:connectors={}",
                self.surfaces.len(),
                self.portals.len(),
                obstacles.len(),
                self.connectors.len(),
            ))?;
            Ok(ForcedWalkOutcome::NeverEntered(Box::new(
                NeverEnteredEvidence::new(request.clone(), completeness),
            )))
        };
        if sorted.placed.is_empty() {
            return never("targets-off-level");
        }
        let from = Self::map(
            &domain,
            &narrow.barriers,
            std::slice::from_ref(request.origin()),
            &[0],
        )?;
        let to = Self::map(&domain, &narrow.barriers, targets, &sorted.placed)?;
        let tolerance = request.tolerance_metres();
        let (mut lower, mut upper) = (f64::INFINITY, f64::INFINITY);
        let mut cells = 0;
        let mut converged = true;
        for piece in footprint.polygons() {
            let walk =
                axiolid_route::forced_walk(&from, &to, piece, tolerance).map_err(|error| {
                    format!(
                        "the walk forced through {} could not be bracketed: {error:?}",
                        request.through()
                    )
                })?;
            cells += walk.cells;
            converged &= walk.converged || walk.length.lower.is_infinite();
            lower = lower.min(walk.length.lower);
            upper = upper.min(walk.length.upper);
        }
        if lower.is_infinite() {
            return never("unreached");
        }
        let lower = lower * (1.0 - LENGTH_ROUNDING);
        // Only a point's walk into the exact footprint is a walk entering
        // the object; a body's walk is at least as long.
        let upper = if radius <= 0.0 && exact && upper.is_finite() {
            rounded_up(upper)
        } else {
            f64::INFINITY
        };
        let converged = converged && upper - lower <= tolerance;
        let evidence = Evidence::exact(
            request.origin().subject().source.clone(),
            format!(
                "axiolid:metric-route:forced:{text}:origin=({:.6},{:.6}):tolerance={tolerance:.6}:\
                 cells={cells}:lower={lower:.6}:upper={upper:.6}",
                plan(request.origin()).x,
                plan(request.origin()).y,
            ),
        );
        ForcedWalkEvidence::try_new(lower, upper, converged, evidence)
            .map(ForcedWalkOutcome::Bounded)
            .map_err(|error| error.to_string())
    }
}
