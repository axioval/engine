//! Walkable-region topology from Axiolid geometry.
//!
//! ADR 0004: this module measures which walkable surfaces a body of the
//! requested width can move between, and bounds the width of every passage;
//! whether a route is acceptable is a capability's decision.
//!
//! # The graph
//!
//! Every selected surface is one region, `surface:{id}`, standing for a
//! **hub**: one position in the surface where the body provably fits (the
//! first landing in front of one of its portals, else its centroid), or no
//! position when none is proven. Every selected entrance has two regions,
//! `portal:{id}:-` and `portal:{id}:+`, one per face, standing for the
//! landing in front of that face (or the portal's mid-line point when the
//! face opens onto no surface).
//!
//! - A **spoke** joins a surface to the portal face opening onto it. It is
//!   definite when the body's sweep from the hub to the landing is proven
//!   inside the surface's free region.
//! - A **crossing** joins a portal's two faces and names the portal. Its
//!   width is at most the longest free interval of the portal's mid-line
//!   (and at most a stated clear width); it is definite only when the sweep
//!   from landing to landing through the mid-line point is proven inside the
//!   corridor and both surfaces, and the portal's leaf and lining admit the
//!   body: a bodiless opening does, a door only when its clear width is
//!   stated.
//! - Surfaces or portal faces that touch in plan at overlapping heights are
//!   joined without a width bound; surface pairs are definite when a sweep
//!   between their hubs is proven.
//! - A selected vertical connector joins every pair of surfaces whose floor
//!   lies within 1 m of its height range and whose plan lies within 1 m of
//!   its plan. The climb itself is not measured, so these
//!   passages are never definite and bound no width; forbidding the kind
//!   removes them.
//!
//! Every definite passage ends at the position the next one starts from, so
//! a definite route concatenates proven sweeps. Every width bound is a chord
//! bound or none, so a route absent from the possible graph cannot exist.

use axiolid_core::Point2;
use axiolid_mesh::TriMesh;
use axioval_engine::{
    LengthInterval, VerifiedWalkablePassage, VerticalConnector, WalkabilityError,
    WalkabilityRegion, WalkabilityRegionId, WalkabilityRequest, WalkabilityService,
    WalkabilitySnapshot,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::AxiolidGeometry;
use crate::walkable::{
    Clearance, Floor, ON_SURFACE, Plan, PortalFacts, PortalFrame, REACH, Side, UNBOUNDED,
    band_limits, centroid, contains, corridor, floor, join, landing, mid_line_width, obstacles,
    obstruction, plan_gap, sides, subtract, sweep_inside, touching, union, witness,
};

/// Walkable regions and passages from supplied geometry.
///
/// Surfaces, entrances, obstacles and vertical connectors are the request's
/// (the rule's selection). What a mesh cannot show is the host's to declare:
/// the void of a bodiless opening ([`Self::with_opening_void`]) and the clear
/// width a door's leaf and lining leave ([`Self::with_clear_width`]).
pub struct AxiolidWalkabilityService {
    geometry: AxiolidGeometry,
    source: SourceId,
    facts: PortalFacts,
}

impl AxiolidWalkabilityService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            facts: PortalFacts::default(),
        }
    }

    /// Declares the exact planar void of a bodiless opening. Its corridor is
    /// its clear passage, with no leaf or lining to narrow it.
    #[must_use]
    pub fn with_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.facts = self.facts.with_void(opening, mesh, true);
        self
    }

    /// Declares an opening's void as a tessellation; such a portal refuses.
    #[must_use]
    pub fn with_tessellated_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.facts = self.facts.with_void(opening, mesh, false);
        self
    }

    /// Declares an opening whose void could not be meshed; it refuses.
    #[must_use]
    pub fn with_unmeasured_opening_void(
        mut self,
        opening: ObjectId,
        reason: impl Into<String>,
    ) -> Self {
        self.facts = self.facts.with_unmeasured_void(opening, reason.into());
        self
    }

    /// States the clear width, in metres, a door's leaf and lining leave.
    ///
    /// A mesh does not show which part of a door is leaf or lining. Without
    /// this statement a door bounds widths from above only.
    #[must_use]
    pub fn with_clear_width(mut self, portal: ObjectId, metres: f64) -> Self {
        self.facts = self.facts.with_clear_width(portal, metres);
        self
    }

    /// Set-level evidence: the snapshot as a whole, cited under the source
    /// given to the constructor.
    fn evidence(&self, locator: String) -> Evidence {
        Evidence::exact(self.source.clone(), locator)
    }
}

/// A portal measured for one request.
struct Portal {
    frame: PortalFrame,
    sides: [Option<Side>; 2],
    corridor: Plan,
    clearance: Clearance,
}

fn region_id(value: String) -> Result<WalkabilityRegionId, WalkabilityError> {
    WalkabilityRegionId::new(value)
}

fn surface_region(id: &ObjectId) -> String {
    format!("surface:{id}")
}

fn face_region(id: &ObjectId, slot: usize) -> String {
    format!("portal:{id}:{}", if slot == 0 { "-" } else { "+" })
}

fn unavailable(reason: impl Into<String>) -> WalkabilityError {
    WalkabilityError::Unavailable(reason.into())
}

fn width(lower: f64, upper: f64) -> Result<LengthInterval, WalkabilityError> {
    LengthInterval::try_new(lower.min(upper), upper)
        .map_err(|_| unavailable("a passage width could not be bounded"))
}

impl AxiolidWalkabilityService {
    #[allow(clippy::too_many_lines)]
    fn build(&self, request: &WalkabilityRequest) -> Result<WalkabilitySnapshot, String> {
        if request.includes_motion_envelopes() {
            return Err("moving envelopes (door swings) are not measured".into());
        }
        let required = request.minimum_width_metres();
        let radius = required * 0.5;
        let band = request.elevation_band();
        // A selected surface is walked on and a selected entrance walked
        // through, so neither obstructs even when a broad selector also
        // picks it as an obstacle.
        let obstacles = obstacles(
            &self.geometry,
            request.obstacles().iter().filter(|object| {
                request.surfaces().binary_search(object).is_err()
                    && request.entrances().binary_search(object).is_err()
            }),
        )?;

        let floors: Vec<Floor> = request
            .surfaces()
            .iter()
            .map(|surface| floor(&self.geometry, surface))
            .collect::<Result<_, _>>()?;
        let mut free = Vec::with_capacity(floors.len());
        for floor in &floors {
            let (lo, hi) = band_limits(band, floor.z0, floor.top);
            let blocked = obstruction(&obstacles, &floor.bounds, lo, hi)?;
            free.push(subtract(&floor.footprint, &blocked)?);
        }

        let mut portals = Vec::new();
        for entrance in request.entrances() {
            let frame = self.facts.frame(&self.geometry, entrance)?;
            let sides = sides(&frame, &floors)?;
            let (lo, hi) = band_limits(band, frame.z0, frame.z1);
            let reach = frame.half + REACH;
            let bounds = {
                let a = frame.point(frame.u0, -reach);
                let b = frame.point(frame.u1, reach);
                let c = frame.point(frame.u0, reach);
                let d = frame.point(frame.u1, -reach);
                (
                    [
                        a.x.min(b.x).min(c.x).min(d.x),
                        a.y.min(b.y).min(c.y).min(d.y),
                    ],
                    [
                        a.x.max(b.x).max(c.x).max(d.x),
                        a.y.max(b.y).max(c.y).max(d.y),
                    ],
                )
            };
            let blocked = obstruction(&obstacles, &bounds, lo, hi)?;
            let corridor = corridor(&frame, &sides, &blocked)?;
            let clearance = self.facts.clearance(entrance)?;
            portals.push(Portal {
                frame,
                sides,
                corridor,
                clearance,
            });
        }

        // Landings, and each surface's hub.
        let mut landings: Vec<[Option<Point2>; 2]> = Vec::with_capacity(portals.len());
        let mut mid_points: Vec<Option<(f64, Point2)>> = Vec::with_capacity(portals.len());
        let mut crossing_bounds = Vec::with_capacity(portals.len());
        for portal in &portals {
            let frame = &portal.frame;
            let pieces: Vec<&Plan> = floors
                .iter()
                .zip(&free)
                .filter(|(floor, _)| floor.z0 < frame.z1 && floor.top > frame.z0)
                .map(|(_, free)| free)
                .chain(
                    portals
                        .iter()
                        .filter(|other| other.frame.z0 < frame.z1 && other.frame.z1 > frame.z0)
                        .map(|other| &other.corridor),
                )
                .collect();
            let (upper, interval) = mid_line_width(frame, &pieces);
            crossing_bounds.push(upper);
            let mid = interval.map(|(a, b)| {
                let u = f64::midpoint(a, b);
                (u, frame.point(u, 0.0))
            });
            mid_points.push(mid);
            let mut found = [None, None];
            if let Some((u, _)) = mid {
                for (slot, side) in portal.sides.iter().enumerate() {
                    let Some(side) = side else { continue };
                    let sign = if slot == 0 { -1.0 } else { 1.0 };
                    if let Some(point) = landing(frame, &floors[side.floor], u, sign, radius)
                        && sweep_inside(&free[side.floor], &[point], radius)?
                    {
                        found[slot] = Some(point);
                    }
                }
            }
            landings.push(found);
        }
        let mut hubs: Vec<Option<Point2>> = vec![None; floors.len()];
        for (portal, found) in portals.iter().zip(&landings) {
            for (side, point) in portal.sides.iter().zip(found) {
                if let (Some(side), Some(point)) = (side, point)
                    && hubs[side.floor].is_none()
                {
                    hubs[side.floor] = Some(*point);
                }
            }
        }
        for (index, hub) in hubs.iter_mut().enumerate() {
            if hub.is_none()
                && let Some(point) = centroid(&free[index])
                && contains(&free[index], point)
                && sweep_inside(&free[index], &[point], radius)?
            {
                *hub = Some(point);
            }
        }

        let mut regions = Vec::new();
        for floor in &floors {
            regions.push(WalkabilityRegion::new(
                region_id(surface_region(&floor.id)).map_err(|e| e.to_string())?,
                vec![floor.id.clone()],
            ));
        }
        for portal in &portals {
            for slot in 0..2 {
                regions.push(WalkabilityRegion::new(
                    region_id(face_region(&portal.frame.id, slot)).map_err(|e| e.to_string())?,
                    vec![portal.frame.id.clone()],
                ));
            }
        }

        let mut passages = Vec::new();
        // A passage cites the source of the object it measures: in a set
        // holding several sources, that object's own file.
        let passage = |cite: &ObjectId,
                       a: String,
                       b: String,
                       portal: Option<ObjectId>,
                       lower: f64,
                       upper: f64,
                       locator: String|
         -> Result<VerifiedWalkablePassage, String> {
            VerifiedWalkablePassage::try_new(
                region_id(a).map_err(|e| e.to_string())?,
                region_id(b).map_err(|e| e.to_string())?,
                portal,
                width(lower, upper).map_err(|e| e.to_string())?,
                Evidence::exact(
                    cite.source.clone(),
                    format!("axiolid:walkability:{locator}"),
                ),
            )
            .map_err(|e| e.to_string())
        };

        // Spokes and crossings.
        for (index, portal) in portals.iter().enumerate() {
            let frame = &portal.frame;
            let id = &frame.id;
            for (slot, side) in portal.sides.iter().enumerate() {
                let Some(side) = side else { continue };
                let floor = &floors[side.floor];
                let proven = match (hubs[side.floor], landings[index][slot]) {
                    (Some(hub), Some(point)) => {
                        witness(&free[side.floor], hub, point, radius)?.is_some()
                    }
                    _ => false,
                };
                passages.push(passage(
                    &floor.id,
                    surface_region(&floor.id),
                    face_region(id, slot),
                    None,
                    if proven { required } else { 0.0 },
                    UNBOUNDED,
                    format!(
                        "spoke:{}->{id}:side={}:gap={:.6}:sweep={}",
                        floor.id,
                        if slot == 0 { "-" } else { "+" },
                        side.gap,
                        if proven { "proven" } else { "unproven" }
                    ),
                )?);
            }
            if !request.traverses_verified_portals() {
                continue;
            }
            let mut upper = crossing_bounds[index];
            if let Clearance::Stated(stated) = portal.clearance {
                upper = upper.min(stated);
            }
            let proven = match mid_points[index] {
                Some((_, mid)) if portal.clearance.admits(required) && required <= upper => {
                    let ends: Vec<Option<Point2>> = (0..2)
                        .map(|slot| match &portal.sides[slot] {
                            Some(_) => landings[index][slot],
                            None => Some(mid),
                        })
                        .collect();
                    match (ends[0], ends[1]) {
                        (Some(start), Some(end)) => {
                            let mut local = portal.corridor.clone();
                            for side in portal.sides.iter().flatten() {
                                local = join(&local, &free[side.floor])?;
                            }
                            sweep_inside(&local, &[start, mid, end], radius)?
                        }
                        _ => false,
                    }
                }
                _ => false,
            };
            passages.push(passage(
                id,
                face_region(id, 0),
                face_region(id, 1),
                Some(id.clone()),
                if proven { required } else { 0.0 },
                upper,
                format!(
                    "crossing:{id}:mid-line-width<={:.6}:clearance={}:sweep={}",
                    crossing_bounds[index],
                    portal.clearance.label(),
                    if proven { "proven" } else { "unproven" }
                ),
            )?);
        }

        // Surfaces touching surfaces.
        for a in 0..floors.len() {
            for b in a + 1..floors.len() {
                let (first, second) = (&floors[a], &floors[b]);
                if first.z0 >= second.top - ON_SURFACE
                    || second.z0 >= first.top - ON_SURFACE
                    || !touching(&first.footprint, &second.footprint)?
                {
                    continue;
                }
                let proven = match (hubs[a], hubs[b]) {
                    (Some(from), Some(to)) => {
                        witness(&join(&free[a], &free[b])?, from, to, radius)?.is_some()
                    }
                    _ => false,
                };
                passages.push(passage(
                    &first.id,
                    surface_region(&first.id),
                    surface_region(&second.id),
                    None,
                    if proven { required } else { 0.0 },
                    UNBOUNDED,
                    format!(
                        "touch:{}<->{}:sweep={}",
                        first.id,
                        second.id,
                        if proven { "proven" } else { "unproven" }
                    ),
                )?);
            }
        }

        // Portal faces touching anything but their own side.
        let halves: Vec<(usize, usize, Plan)> = portals
            .iter()
            .enumerate()
            .flat_map(|(index, portal)| {
                let frame = &portal.frame;
                let reach = |slot: usize| {
                    portal.sides[slot]
                        .as_ref()
                        .map_or(0.0, |side| side.gap + 1e-5)
                };
                [
                    (index, 0, frame.band(-frame.half - reach(0), 0.0)),
                    (index, 1, frame.band(0.0, frame.half + reach(1))),
                ]
            })
            .filter_map(|(index, slot, shape)| Some((index, slot, shape?)))
            .map(|(index, slot, shape)| Ok((index, slot, union(vec![shape])?)))
            .collect::<Result<_, String>>()?;
        for (index, slot, half) in &halves {
            let portal = &portals[*index];
            let own = portal.sides[*slot].as_ref().map(|side| side.floor);
            for (other, floor) in floors.iter().enumerate() {
                if Some(other) == own
                    || floor.z0 >= portal.frame.z1
                    || floor.top <= portal.frame.z0
                    || !touching(half, &floor.footprint)?
                {
                    continue;
                }
                passages.push(passage(
                    &portal.frame.id,
                    surface_region(&floor.id),
                    face_region(&portal.frame.id, *slot),
                    None,
                    0.0,
                    UNBOUNDED,
                    format!(
                        "touch:{}<->{}",
                        floor.id,
                        face_region(&portal.frame.id, *slot)
                    ),
                )?);
            }
        }
        for (i, (a_index, a_slot, a_half)) in halves.iter().enumerate() {
            for (b_index, b_slot, b_half) in halves.iter().skip(i + 1) {
                let (a, b) = (&portals[*a_index].frame, &portals[*b_index].frame);
                if a_index == b_index || a.z0 >= b.z1 || b.z0 >= a.z1 || !touching(a_half, b_half)?
                {
                    continue;
                }
                passages.push(passage(
                    &a.id,
                    face_region(&a.id, *a_slot),
                    face_region(&b.id, *b_slot),
                    None,
                    0.0,
                    UNBOUNDED,
                    format!(
                        "touch:{}<->{}",
                        face_region(&a.id, *a_slot),
                        face_region(&b.id, *b_slot)
                    ),
                )?);
            }
        }

        // Vertical connectors.
        for connector in request.connectors() {
            let joined = self.joined(connector, &floors)?;
            for (i, a) in joined.iter().enumerate() {
                for b in joined.iter().skip(i + 1) {
                    passages.push(
                        passage(
                            connector.object(),
                            surface_region(&floors[*a].id),
                            surface_region(&floors[*b].id),
                            None,
                            0.0,
                            UNBOUNDED,
                            format!(
                                "connector:{}:{}:{}<->{}:climb=unmeasured",
                                connector.kind().as_str(),
                                connector.object(),
                                floors[*a].id,
                                floors[*b].id
                            ),
                        )?
                        .with_connector(connector.clone())
                        .map_err(|e| e.to_string())?,
                    );
                }
            }
        }

        let band_text = band.map_or_else(
            || "own-height".to_owned(),
            |band| format!("{:.6}..{:.6}", band.lower_metres(), band.upper_metres()),
        );
        let evidence = self.evidence(format!(
            "axiolid:walkability:complete:surfaces={}:entrances={}:obstacles={}:connectors={}:\
             width={required:.6}:band={band_text}:portals-traversed={}",
            request.surfaces().len(),
            request.entrances().len(),
            request.obstacles().len(),
            request.connectors().len(),
            request.traverses_verified_portals(),
        ));
        WalkabilitySnapshot::try_new(request.clone(), regions, passages, evidence)
            .map_err(|e| e.to_string())
    }

    /// The surfaces a vertical connector may join: floors within [`REACH`]
    /// of its height range and plan.
    fn joined(
        &self,
        connector: &VerticalConnector,
        floors: &[Floor],
    ) -> Result<Vec<usize>, String> {
        let object = connector.object();
        crate::walkable::body(&self.geometry, object, "vertical connector")?;
        let extent = self.geometry.enclosing_extent(object).ok_or_else(|| {
            format!("vertical connector {object} has an empty mesh or an invalid chord deviation")
        })?;
        Ok(floors
            .iter()
            .enumerate()
            .filter(|(_, floor)| {
                floor.z0 >= extent.0[2] - REACH
                    && floor.z0 <= extent.1[2] + REACH
                    && plan_gap(&floor.bounds, &extent) <= REACH + ON_SURFACE
            })
            .map(|(index, _)| index)
            .collect())
    }
}

impl WalkabilityService for AxiolidWalkabilityService {
    fn snapshot(
        &self,
        request: &WalkabilityRequest,
    ) -> Result<WalkabilitySnapshot, WalkabilityError> {
        self.build(request).map_err(unavailable)
    }
}
