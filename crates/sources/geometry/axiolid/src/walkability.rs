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
//!
//! # Swings and pieces
//!
//! Swept door sectors (the request's) are obstacles on the floors they
//! stand on. A hub stands clear of every swing's circumscribed polygon; a
//! portal's landings, spokes and crossing are proven clear of every swing
//! but the portal's own, which the body walks through.
//!
//! A surface that touches no other surface and that no connector joins is
//! split into its **possible pieces**: its footprint eroded from outside by
//! half the width less `MARGIN` (0.1 mm), less the obstacles and the swings'
//! inscribed polygons grown from inside by as much. A centre of the body
//! clear of walls, obstacles and swings lies inside one piece with a disc
//! of `MARGIN` (0.1 mm) around it, and the body moves between pieces only by
//! leaving the surface: through a portal's half on that side or the
//! portal's own swing. So each face is joined to the pieces within half the
//! width (and `MARGIN` (0.1 mm)) of those, and to the hub's piece always, with no
//! width when out of reach; faces whose such zones meet are joined too. The
//! hub's piece is `surface:{id}`, every other `surface:{id}#{k}`.

use std::collections::BTreeSet;

use axiolid_core::Point2;
use axiolid_mesh::TriMesh;
use axiolid_overlay::{Polygon, Region, Ring};
use axioval_engine::{
    LengthInterval, SweptDoor, VerifiedWalkablePassage, VerticalConnector, WalkabilityError,
    WalkabilityRegion, WalkabilityRegionId, WalkabilityRequest, WalkabilityService,
    WalkabilitySnapshot,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::free_space::swept_rings;
use crate::geometry::AxiolidGeometry;
use crate::placement;
use crate::walkable::{
    Clearance, Floor, MARGIN, ON_SURFACE, Plan, PortalFacts, PortalFrame, REACH, Side, UNBOUNDED,
    band_limits, centroid, contains, corridor, floor, join, landing, mid_line_width, obstacles,
    obstruction, plan_gap, sides, subtract, sweep_inside, touching, trapezoids, union, witness,
};

/// Walkable regions and passages from supplied geometry.
///
/// Surfaces, entrances, obstacles and vertical connectors are the request's
/// (the rule's selection). What a mesh cannot show is the host's to declare:
/// the void of a bodiless opening ([`Self::with_opening_void`]) and the clear
/// width a door's leaf and lining leave ([`Self::with_clear_width`]). A
/// request may state clear widths too
/// ([`WalkabilityRequest::with_stated_clear_widths`], read by the rule from
/// its source); where both state one, the narrower counts.
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

/// One surface's free region, with and without the swept sectors on it.
struct Ground {
    /// The floor less what the obstacles occupy in the band.
    plain: Plan,
    /// `plain` less every swept sector's circumscribed polygon.
    free: Plan,
    /// What the obstacles occupy in the band.
    blocked: Plan,
    /// Per swept door standing on the floor: its sectors' inscribed and
    /// circumscribed polygons.
    swings: Vec<(ObjectId, Vec<Polygon>, Vec<Polygon>)>,
}

fn polygons(rings: Vec<Ring>) -> Vec<Polygon> {
    rings
        .into_iter()
        .map(|outer| Polygon {
            outer,
            holes: Vec::new(),
        })
        .collect()
}

impl Ground {
    fn new(floor: &Floor, blocked: Plan, swept: &[SweptDoor], top: f64) -> Result<Self, String> {
        let plain = subtract(&floor.footprint, &blocked)?;
        let mut swings = Vec::new();
        for door in swept {
            let (inner, outer) = swept_rings(std::slice::from_ref(door), floor.z0, top);
            if !inner.is_empty() {
                swings.push((door.door().clone(), polygons(inner), polygons(outer)));
            }
        }
        let mut ground = Self {
            free: plain.clone(),
            plain,
            blocked,
            swings,
        };
        ground.free = subtract(&ground.plain, &ground.swings_but(None)?)?;
        Ok(ground)
    }

    /// The circumscribed polygons of every swing but `door`'s.
    fn swings_but(&self, door: Option<&ObjectId>) -> Result<Plan, String> {
        union(
            self.swings
                .iter()
                .filter(|(swung, _, _)| Some(swung) != door)
                .flat_map(|(_, _, outer)| outer.iter().cloned())
                .collect(),
        )
    }

    /// The free region a body passing through `door` may use: clear of
    /// every swing but the door's own, which it walks through.
    fn past(&self, door: &ObjectId) -> Result<Plan, String> {
        if self.swing_of(door).is_empty() {
            return Ok(self.free.clone());
        }
        subtract(&self.plain, &self.swings_but(Some(door))?)
    }

    /// The circumscribed polygons of `door`'s own sectors here.
    fn swing_of(&self, door: &ObjectId) -> &[Polygon] {
        self.swings
            .iter()
            .find(|(swung, _, _)| swung == door)
            .map_or(&[], |(_, _, outer)| outer.as_slice())
    }
}

/// How a surface enters the possible graph.
enum Parts {
    /// One region: whatever a body can do on it is possible.
    Whole,
    /// One region per possible piece of its free region eroded by half the
    /// body's width; `hub` is the piece holding the hub, and the first
    /// without one.
    Split { pieces: Vec<Region>, hub: usize },
}

fn overlay_error(context: &str) -> impl Fn(axiolid_overlay::OverlayError) -> String + '_ {
    move |error| format!("{context}: {error:?}")
}

impl Parts {
    /// The possible pieces of `floor`: its footprint eroded from outside by
    /// the radius less [`MARGIN`], less the sure obstacles (the obstacles'
    /// band footprint and every sector's inscribed polygon) dilated from
    /// inside by as much. Every centre of the body clear of walls and
    /// obstacles lies inside one piece, with a disc of [`MARGIN`] around it.
    fn split(
        floor: &Floor,
        ground: &Ground,
        hub: Option<Point2>,
        radius: f64,
    ) -> Result<Self, String> {
        let shrunk = radius - MARGIN;
        if shrunk <= 0.0 {
            return Ok(Self::Whole);
        }
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let rings = |plan: &Plan| -> Vec<Ring> {
            trapezoids(plan)
                .into_iter()
                .map(|piece| piece.outer)
                .collect()
        };
        let footprint = placement::union(&rings(&floor.footprint), t).map_err(|e| e.to_string())?;
        let mut sure = rings(&ground.blocked);
        sure.extend(
            ground
                .swings
                .iter()
                .flat_map(|(_, inner, _)| inner.iter().map(|polygon| polygon.outer.clone())),
        );
        let sure = placement::union(&sure, t).map_err(|e| e.to_string())?;
        let mut room = footprint
            .erode_outer(shrunk, t)
            .map_err(overlay_error("possible pieces"))?;
        if !sure.is_empty() && !room.is_empty() {
            room = room
                .difference(
                    &sure
                        .dilate_inner(shrunk, t)
                        .map_err(overlay_error("possible pieces"))?,
                    t,
                )
                .map_err(overlay_error("possible pieces"))?;
        }
        let pieces: Vec<Region> = room
            .polygons()
            .iter()
            .map(|polygon| Region::new(vec![polygon.clone()], t))
            .collect::<Result<_, _>>()
            .map_err(overlay_error("possible piece"))?;
        let hub = match hub {
            None => 0,
            Some(point) => {
                let holder = pieces
                    .iter()
                    .position(|piece| contains(&Plan::piece(piece.polygons()[0].clone()), point));
                // A proven hub lies in the exact erosion, hence in a piece;
                // one rounded out of every piece keeps the surface whole.
                match holder {
                    Some(index) => index,
                    None => return Ok(Self::Whole),
                }
            }
        };
        Ok(Self::Split { pieces, hub })
    }

    /// The piece that holds the hub, `surface:{id}` itself.
    fn hub_piece(&self) -> usize {
        match self {
            Self::Whole => 0,
            Self::Split { hub, .. } => *hub,
        }
    }

    fn name(&self, id: &ObjectId, piece: usize) -> String {
        if piece == self.hub_piece() {
            surface_region(id)
        } else {
            format!("{}#{piece}", surface_region(id))
        }
    }

    /// Every region of the surface `id`.
    fn names(&self, id: &ObjectId) -> Vec<String> {
        match self {
            Self::Split { pieces, .. } if !pieces.is_empty() => (0..pieces.len())
                .map(|piece| self.name(id, piece))
                .collect(),
            _ => vec![surface_region(id)],
        }
    }

    /// Where a body may leave the surface through a portal: within the
    /// radius (and [`MARGIN`]) of the portal's half on this side or of the
    /// portal's own swing, which it walks through.
    fn zone(half: Option<&Plan>, swing: &[Polygon], radius: f64) -> Result<Region, String> {
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let mut rings: Vec<Ring> = half
            .map(|half| {
                trapezoids(half)
                    .into_iter()
                    .map(|piece| piece.outer)
                    .collect()
            })
            .unwrap_or_default();
        rings.extend(swing.iter().map(|polygon| polygon.outer.clone()));
        placement::union(&rings, t)
            .map_err(|e| e.to_string())?
            .dilate_outer(radius + MARGIN, t)
            .map_err(overlay_error("portal zone"))
    }

    /// The pieces a body entering through a portal may reach, by index,
    /// region name and whether it may: every piece within reach of the
    /// portal's zone, and the hub's piece always, so a separation is a
    /// passage too narrow rather than a missing one.
    fn near(
        &self,
        id: &ObjectId,
        half: Option<&Plan>,
        swing: &[Polygon],
        radius: f64,
    ) -> Result<Vec<(usize, String, bool)>, String> {
        let Self::Split { pieces, hub } = self else {
            return Ok(vec![(0, surface_region(id), true)]);
        };
        if pieces.is_empty() {
            return Ok(vec![(0, surface_region(id), false)]);
        }
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let zone = Self::zone(half, swing, radius)?;
        let mut near = Vec::new();
        for (index, piece) in pieces.iter().enumerate() {
            let meets = !piece
                .intersection(&zone, t)
                .map_err(overlay_error("portal zone"))?
                .is_empty();
            if meets || index == *hub {
                near.push((index, self.name(id, index), meets));
            }
        }
        Ok(near)
    }

    /// Whether a body may pass from one portal to another without standing
    /// wholly on this surface: their zones meet.
    fn faces_near(
        &self,
        (a_half, a_swing): (&Plan, &[Polygon]),
        (b_half, b_swing): (&Plan, &[Polygon]),
        radius: f64,
    ) -> Result<bool, String> {
        if matches!(self, Self::Whole) {
            return Ok(false);
        }
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let a = Self::zone(Some(a_half), a_swing, radius)?;
        let b = Self::zone(Some(b_half), b_swing, radius)?;
        Ok(!a
            .intersection(&b, t)
            .map_err(overlay_error("portal zone"))?
            .is_empty())
    }
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
        let mut grounds = Vec::with_capacity(floors.len());
        for floor in &floors {
            let (lo, hi) = band_limits(band, floor.z0, floor.top);
            let blocked = obstruction(&obstacles, &floor.bounds, lo, hi)?;
            grounds.push(Ground::new(floor, blocked, request.swept_doors(), hi)?);
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
            let clearance = self
                .facts
                .clearance(entrance)?
                .with_stated(request.stated_clear_width(entrance));
            portals.push(Portal {
                frame,
                sides,
                corridor,
                clearance,
            });
        }

        // Landings, and each surface's hub. A portal's landings are proven
        // clear of every swing but its own, which the body walks through.
        let mut landings: Vec<[Option<Point2>; 2]> = Vec::with_capacity(portals.len());
        let mut mid_points: Vec<Option<(f64, Point2)>> = Vec::with_capacity(portals.len());
        let mut crossing_bounds = Vec::with_capacity(portals.len());
        for portal in &portals {
            let frame = &portal.frame;
            let pieces: Vec<&Plan> = floors
                .iter()
                .zip(&grounds)
                .filter(|(floor, _)| floor.z0 < frame.z1 && floor.top > frame.z0)
                .map(|(_, ground)| &ground.plain)
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
                        && sweep_inside(&grounds[side.floor].past(&frame.id)?, &[point], radius)?
                    {
                        found[slot] = Some(point);
                    }
                }
            }
            landings.push(found);
        }
        // A hub stands clear of every swing, so every spoke from it is a
        // sweep out of its surface's shared free region.
        let mut hubs: Vec<Option<Point2>> = vec![None; floors.len()];
        for (portal, found) in portals.iter().zip(&landings) {
            for (side, point) in portal.sides.iter().zip(found) {
                if let (Some(side), Some(point)) = (side, point)
                    && hubs[side.floor].is_none()
                    && sweep_inside(&grounds[side.floor].free, &[*point], radius)?
                {
                    hubs[side.floor] = Some(*point);
                }
            }
        }
        for (index, hub) in hubs.iter_mut().enumerate() {
            let free = &grounds[index].free;
            if hub.is_none()
                && let Some(point) = centroid(free)
                && contains(free, point)
                && sweep_inside(free, &[point], radius)?
            {
                *hub = Some(point);
            }
        }

        // Surfaces that touch, and those a connector joins, stay one region:
        // a body may stand across their shared boundary.
        let mut touches = Vec::new();
        for a in 0..floors.len() {
            for b in a + 1..floors.len() {
                let (first, second) = (&floors[a], &floors[b]);
                if first.z0 >= second.top - ON_SURFACE
                    || second.z0 >= first.top - ON_SURFACE
                    || !touching(&first.footprint, &second.footprint)?
                {
                    continue;
                }
                touches.push((a, b));
            }
        }
        let mut joined_by_connector = vec![false; floors.len()];
        let mut connected = Vec::new();
        for connector in request.connectors() {
            let joined = self.joined(connector, &floors)?;
            for index in &joined {
                joined_by_connector[*index] = true;
            }
            connected.push((connector, joined));
        }
        let mut parts: Vec<Parts> = Vec::with_capacity(floors.len());
        for (index, floor) in floors.iter().enumerate() {
            let alone = !joined_by_connector[index]
                && touches.iter().all(|&(a, b)| a != index && b != index);
            parts.push(if alone {
                Parts::split(floor, &grounds[index], hubs[index], radius)?
            } else {
                Parts::Whole
            });
        }

        let mut regions = Vec::new();
        for (floor, split) in floors.iter().zip(&parts) {
            for name in split.names(&floor.id) {
                regions.push(WalkabilityRegion::new(
                    region_id(name).map_err(|e| e.to_string())?,
                    vec![floor.id.clone()],
                ));
            }
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

        // The half of each portal on each side: its corridor from the
        // mid-line out to where that side's surface begins.
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

        // Spokes and crossings.
        for (index, portal) in portals.iter().enumerate() {
            let frame = &portal.frame;
            let id = &frame.id;
            for (slot, side) in portal.sides.iter().enumerate() {
                let Some(side) = side else { continue };
                let floor = &floors[side.floor];
                let ground = &grounds[side.floor];
                let proven = match (hubs[side.floor], landings[index][slot]) {
                    (Some(hub), Some(point)) => {
                        witness(&ground.past(id)?, hub, point, radius)?.is_some()
                    }
                    _ => false,
                };
                let face = face_region(id, slot);
                let half = halves
                    .iter()
                    .find(|(at, s, _)| *at == index && *s == slot)
                    .map(|(_, _, half)| half);
                let split = &parts[side.floor];
                for (piece, name, meets) in
                    split.near(&floor.id, half, ground.swing_of(id), radius)?
                {
                    let definite = proven && piece == split.hub_piece();
                    let open = definite || meets;
                    passages.push(passage(
                        &floor.id,
                        name,
                        face.clone(),
                        None,
                        if definite { required } else { 0.0 },
                        if open { UNBOUNDED } else { 0.0 },
                        format!(
                            "spoke:{}->{id}:side={}:gap={:.6}:sweep={}{}",
                            floor.id,
                            if slot == 0 { "-" } else { "+" },
                            side.gap,
                            if definite { "proven" } else { "unproven" },
                            if open { "" } else { ":separated" }
                        ),
                    )?);
                }
            }
            if !request.traverses_verified_portals() {
                continue;
            }
            let mut upper = crossing_bounds[index];
            if let Clearance::Stated(stated) = portal.clearance {
                upper = upper.min(stated);
            }
            // A width the request states bounds even an opening's void.
            if let Some(stated) = request.stated_clear_width(id) {
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
                                let ground = &grounds[side.floor];
                                local = subtract(&local, &ground.swings_but(Some(id))?)?;
                                local = join(&local, &ground.past(id)?)?;
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
        for &(a, b) in &touches {
            let (first, second) = (&floors[a], &floors[b]);
            let proven = match (hubs[a], hubs[b]) {
                (Some(from), Some(to)) => {
                    witness(&join(&grounds[a].free, &grounds[b].free)?, from, to, radius)?.is_some()
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

        // Portal faces touching anything but their own side, and faces a
        // body may pass between over a split surface without standing
        // wholly on it.
        let mut linked: BTreeSet<(String, String)> = BTreeSet::new();
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
                let face = face_region(&portal.frame.id, *slot);
                for name in parts[other].names(&floor.id) {
                    if linked.insert((name.clone(), face.clone())) {
                        passages.push(passage(
                            &portal.frame.id,
                            name.clone(),
                            face.clone(),
                            None,
                            0.0,
                            UNBOUNDED,
                            format!("touch:{name}<->{face}"),
                        )?);
                    }
                }
            }
        }
        for (i, (a_index, a_slot, a_half)) in halves.iter().enumerate() {
            for (b_index, b_slot, b_half) in halves.iter().skip(i + 1) {
                let (a, b) = (&portals[*a_index], &portals[*b_index]);
                if a_index == b_index || a.frame.z0 >= b.frame.z1 || b.frame.z0 >= a.frame.z1 {
                    continue;
                }
                let near = match (&a.sides[*a_slot], &b.sides[*b_slot]) {
                    (Some(first), Some(second)) if first.floor == second.floor => {
                        let ground = &grounds[first.floor];
                        parts[first.floor].faces_near(
                            (a_half, ground.swing_of(&a.frame.id)),
                            (b_half, ground.swing_of(&b.frame.id)),
                            radius,
                        )?
                    }
                    _ => false,
                };
                if !near && !touching(a_half, b_half)? {
                    continue;
                }
                let (first, second) = (
                    face_region(&a.frame.id, *a_slot),
                    face_region(&b.frame.id, *b_slot),
                );
                if linked.insert((first.clone(), second.clone())) {
                    passages.push(passage(
                        &a.frame.id,
                        first.clone(),
                        second.clone(),
                        None,
                        0.0,
                        UNBOUNDED,
                        format!("touch:{first}<->{second}"),
                    )?);
                }
            }
        }

        // Vertical connectors.
        for (connector, joined) in &connected {
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
                        .with_connector((*connector).clone())
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
             swept={}:width={required:.6}:band={band_text}:portals-traversed={}",
            request.surfaces().len(),
            request.entrances().len(),
            request.obstacles().len(),
            request.connectors().len(),
            request.swept_doors().len(),
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
