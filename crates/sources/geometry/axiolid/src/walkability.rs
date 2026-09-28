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
    LengthInterval, StretchLimit, SweptDoor, VerifiedWalkablePassage, VerticalConnector,
    WalkabilityError, WalkabilityRegion, WalkabilityRegionId, WalkabilityRequest,
    WalkabilityService, WalkabilitySnapshot, WalkableStretch,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::free_space::swept_rings;
use crate::geometry::AxiolidGeometry;
use crate::placement;
use crate::walkable::{
    Blocker, Clearance, Floor, MARGIN, ON_SURFACE, Plan, PortalFacts, PortalFrame, REACH, Side,
    UNBOUNDED, band_limits, centroid, closest, contains, corridor, floor, grown, inner_point,
    intersect, join, landing, mid_line_width, obstacles, obstruction, obstructions, plan_gap,
    sides, subtract, sweep, sweep_inside, touching, trapezoids, union, within, witness,
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
///
/// With an obstruction depth, what an obstacle occupies within that depth
/// of the floor's boundary is tolerated. Where exactly that band ends is
/// bracketed by the one-sided erosions of the footprint, so the free region
/// a witness is proven in (`plain`) tolerates less than the exact one, and
/// the one width bounds and separations are measured in (`wide`, `sure`)
/// more.
struct Ground {
    /// The floor less what the obstacles occupy in the band, less the part
    /// surely within the obstruction depth of its boundary.
    plain: Plan,
    /// The floor less what the obstacles surely occupy beyond the depth:
    /// it contains the exact free region, so chords across it bound widths.
    wide: Plan,
    /// `plain` less every swept sector's circumscribed polygon.
    free: Plan,
    /// What the obstacles occupy in the band over the floor grown by the
    /// surface gap, nothing tolerated: a gap to another surface is no
    /// boundary.
    raw: Plan,
    /// Per obstacle, what it surely occupies beyond the depth.
    sure: Vec<Blocker>,
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

/// A plan as an overlay region, through its convex pieces.
fn region_of(plan: &Plan) -> Result<Region, String> {
    let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
    let rings: Vec<Ring> = trapezoids(plan)
        .into_iter()
        .map(|piece| piece.outer)
        .collect();
    placement::union(&rings, t).map_err(|e| e.to_string())
}

fn joined_all<'p>(plans: impl IntoIterator<Item = &'p Plan>) -> Result<Plan, String> {
    let mut all = Plan::empty();
    for plan in plans {
        all = join(&all, plan)?;
    }
    Ok(all)
}

impl Ground {
    fn new(
        floor: &Floor,
        blockers: Vec<Blocker>,
        swept: &[SweptDoor],
        top: f64,
        depth: f64,
    ) -> Result<Self, String> {
        let raw = joined_all(blockers.iter().map(|blocker| &blocker.plan))?;
        let (blocked, sure) = if depth > 0.0 && !blockers.is_empty() {
            let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
            let footprint = region_of(&floor.footprint)?;
            let beyond_outer = Plan::of(
                footprint
                    .erode_outer(depth, t)
                    .map_err(overlay_error("obstruction depth"))?
                    .polygons(),
            );
            let beyond_inner = Plan::of(
                footprint
                    .erode_inner(depth, t)
                    .map_err(overlay_error("obstruction depth"))?
                    .polygons(),
            );
            let mut sure = Vec::new();
            for blocker in blockers {
                let plan = intersect(&blocker.plan, &beyond_inner)?;
                if !plan.is_empty() {
                    sure.push(Blocker { plan, ..blocker });
                }
            }
            (intersect(&raw, &beyond_outer)?, sure)
        } else {
            (raw.clone(), blockers)
        };
        let plain = subtract(&floor.footprint, &blocked)?;
        let wide = subtract(
            &floor.footprint,
            &joined_all(sure.iter().map(|blocker| &blocker.plan))?,
        )?;
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
            wide,
            raw,
            sure,
            swings,
        };
        ground.free = subtract(&ground.plain, &ground.swings_but(None)?)?;
        Ok(ground)
    }

    /// The obstacles, and the doors whose inscribed swing, meeting `area`;
    /// only those standing on the floor, or only those hanging above it.
    fn meeting(&self, area: &Plan, floor: &Floor, overhead: bool) -> Result<Vec<ObjectId>, String> {
        let mut found = Vec::new();
        for blocker in &self.sure {
            if is_overhead(blocker, floor) == overhead
                && !intersect(&blocker.plan, area)?.is_empty()
            {
                found.push(blocker.id.clone());
            }
        }
        if !overhead {
            for (door, inner, _) in &self.swings {
                if !intersect(&union(inner.clone())?, area)?.is_empty() {
                    found.push(door.clone());
                }
            }
        }
        Ok(found)
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
    /// body's width.
    Split(Box<Split>),
}

/// A piece a portal face may reach: its index, its region's name and
/// whether the body may reach it from the face.
type Reach = (usize, String, bool);

/// A surface's possible pieces, and what they would be with fewer
/// obstacles, to tell what a separation depends on.
struct Split {
    pieces: Vec<Region>,
    /// The piece holding the hub, and the first without one.
    hub: usize,
    /// Per piece, the footprint polygon it lies in: pieces of one polygon
    /// are joined by floor too narrow for the body.
    components: Vec<Option<usize>>,
    /// The pieces without the obstacles hanging above the floor, when there
    /// are any.
    standing: Option<Vec<Region>>,
    /// The pieces without any obstacle or swing.
    bare: Vec<Region>,
}

fn overlay_error(context: &str) -> impl Fn(axiolid_overlay::OverlayError) -> String + '_ {
    move |error| format!("{context}: {error:?}")
}

/// Whether an obstacle's body begins above the floor: it hangs in the
/// headroom band rather than standing on the floor.
fn is_overhead(blocker: &Blocker, floor: &Floor) -> bool {
    blocker.bottom > floor.z0 + ON_SURFACE
}

/// Where the floor between two pieces lies: of the footprint that bodies
/// centred in neither piece cover, the largest part next to both, and a
/// point in it. `None` when no part is next to both.
fn between(floor: &Floor, a: &Plan, b: &Plan) -> Result<Option<(Point2, Plan)>, String> {
    let rest = subtract(&subtract(&floor.footprint, a)?, b)?;
    let mut best: Option<(f64, Plan)> = None;
    for polygon in rest.polygons() {
        let part = Plan::piece(polygon.clone());
        if within(&part, a, STRETCH_CONTACT)?
            && within(&part, b, STRETCH_CONTACT)?
            && best.as_ref().is_none_or(|(area, _)| part.area() > *area)
        {
            best = Some((part.area(), part));
        }
    }
    Ok(best.and_then(|(_, part)| {
        let point = centroid(&part)
            .filter(|point| contains(&part, *point))
            .or_else(|| inner_point(&part))?;
        Some((point, part))
    }))
}

/// How near a part of the floor must come to what a piece's bodies cover
/// to lie next to it: the overlay's snapping, with room to spare.
const STRETCH_CONTACT: f64 = 1e-3;

/// The polygons of a region, one region each.
fn split_region(room: &Region) -> Result<Vec<Region>, String> {
    let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
    room.polygons()
        .iter()
        .map(|polygon| Region::new(vec![polygon.clone()], t))
        .collect::<Result<_, _>>()
        .map_err(overlay_error("possible piece"))
}

fn plan_of(region: &Region) -> Plan {
    Plan::of(region.polygons())
}

impl Parts {
    /// The possible pieces of `floor`: its footprint eroded from outside by
    /// the radius less [`MARGIN`], less the sure obstacles (the obstacles'
    /// band footprint beyond the obstruction depth and every sector's
    /// inscribed polygon) dilated from inside by as much. Every centre of
    /// the body clear of walls and obstacles lies inside one piece, with a
    /// disc of [`MARGIN`] around it.
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
        let bare = region_of(&floor.footprint)?
            .erode_outer(shrunk, t)
            .map_err(overlay_error("possible pieces"))?;
        let carve = |overhead: bool| -> Result<Region, String> {
            let mut sure: Vec<Ring> = Vec::new();
            for blocker in &ground.sure {
                if overhead || !is_overhead(blocker, floor) {
                    sure.extend(trapezoids(&blocker.plan).into_iter().map(|p| p.outer));
                }
            }
            sure.extend(
                ground
                    .swings
                    .iter()
                    .flat_map(|(_, inner, _)| inner.iter().map(|polygon| polygon.outer.clone())),
            );
            let sure = placement::union(&sure, t).map_err(|e| e.to_string())?;
            if sure.is_empty() || bare.is_empty() {
                return Ok(bare.clone());
            }
            bare.difference(
                &sure
                    .dilate_inner(shrunk, t)
                    .map_err(overlay_error("possible pieces"))?,
                t,
            )
            .map_err(overlay_error("possible pieces"))
        };
        let pieces = split_region(&carve(true)?)?;
        let hub = match hub {
            None => 0,
            Some(point) => {
                let holder = pieces
                    .iter()
                    .position(|piece| contains(&plan_of(piece), point));
                // A proven hub lies in the exact erosion, hence in a piece;
                // one rounded out of every piece keeps the surface whole.
                match holder {
                    Some(index) => index,
                    None => return Ok(Self::Whole),
                }
            }
        };
        let components = pieces
            .iter()
            .map(|piece| {
                inner_point(&plan_of(piece)).and_then(|point| {
                    floor
                        .footprint
                        .polygons()
                        .iter()
                        .position(|polygon| contains(&Plan::piece(polygon.clone()), point))
                })
            })
            .collect();
        let standing = if ground
            .sure
            .iter()
            .any(|blocker| is_overhead(blocker, floor))
        {
            Some(split_region(&carve(false)?)?)
        } else {
            None
        };
        Ok(Self::Split(Box::new(Split {
            pieces,
            hub,
            components,
            standing,
            bare: split_region(&bare)?,
        })))
    }

    /// Whether floor too narrow for the body joins pieces `a` and `b`.
    fn joined(split: &Split, a: usize, b: usize) -> bool {
        split.components[a].is_some() && split.components[a] == split.components[b]
    }

    /// What a separation depends on, given whether it disappears among
    /// some pieces: the overhead obstacles when it disappears without them,
    /// else the standing obstacles and swings when it disappears without
    /// any, else the floor's own shape.
    fn limit(
        split: &Split,
        gone: impl Fn(&[Region]) -> Result<bool, String>,
    ) -> Result<StretchLimit, String> {
        if let Some(standing) = &split.standing
            && gone(standing)?
        {
            return Ok(StretchLimit::Low);
        }
        Ok(if gone(&split.bare)? {
            StretchLimit::Obstructed
        } else {
            StretchLimit::Narrow
        })
    }

    /// A stretch of `limit` at `at`, relating what it depends on near
    /// `area`: the headroom under overhead obstacles, or the standing
    /// obstacles and swings.
    fn stretch(
        floor: &Floor,
        ground: &Ground,
        limit: StretchLimit,
        at: Point2,
        area: &Plan,
    ) -> Result<WalkableStretch, String> {
        let related = match limit {
            StretchLimit::Narrow => Vec::new(),
            StretchLimit::Obstructed => ground.meeting(area, floor, false)?,
            StretchLimit::Low => ground.meeting(area, floor, true)?,
        };
        let headroom = (limit == StretchLimit::Low)
            .then(|| {
                ground
                    .sure
                    .iter()
                    .filter(|blocker| related.contains(&blocker.id))
                    .map(|blocker| blocker.bottom - floor.z0)
                    .reduce(f64::min)
            })
            .flatten();
        let stretch =
            WalkableStretch::try_new(floor.id.clone(), limit, [at.x, at.y, floor.z0], related)
                .map_err(|e| e.to_string())?;
        match headroom {
            Some(height) => stretch
                .with_headroom(width(height, height).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string()),
            None => Ok(stretch),
        }
    }

    /// Every pair of pieces floor too narrow for the body joins, with the
    /// stretch between them: where the two come closest.
    fn stretches(
        &self,
        floor: &Floor,
        ground: &Ground,
        radius: f64,
    ) -> Result<Vec<(usize, usize, WalkableStretch)>, String> {
        let Self::Split(split) = self else {
            return Ok(Vec::new());
        };
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let plans: Vec<Plan> = split.pieces.iter().map(plan_of).collect();
        let points: Vec<Option<Point2>> = plans.iter().map(inner_point).collect();
        // What bodies centred in each piece cover, only for pieces with a
        // stretch.
        let mut covered: Vec<Option<Plan>> = vec![None; plans.len()];
        let mut found = Vec::new();
        for a in 0..plans.len() {
            for b in a + 1..plans.len() {
                let (Some(p), Some(q)) = (points[a], points[b]) else {
                    continue;
                };
                if !Self::joined(split, a, b) {
                    continue;
                }
                let limit = Self::limit(split, |pieces| {
                    Ok(pieces.iter().any(|piece| {
                        let plan = plan_of(piece);
                        contains(&plan, p) && contains(&plan, q)
                    }))
                })?;
                for index in [a, b] {
                    if covered[index].is_none() {
                        covered[index] = Some(plan_of(
                            &split.pieces[index]
                                .dilate_outer(radius, t)
                                .map_err(overlay_error("stretch"))?,
                        ));
                    }
                }
                let (Some(cover_a), Some(cover_b)) = (&covered[a], &covered[b]) else {
                    continue;
                };
                let (at, area) = if let Some(found) = between(floor, cover_a, cover_b)? {
                    found
                } else {
                    let Some((near_a, near_b)) = closest(&plans[a], &plans[b]) else {
                        continue;
                    };
                    (
                        near_a + (near_b - near_a) * 0.5,
                        sweep(&[near_a, near_b], radius + MARGIN)?,
                    )
                };
                found.push((a, b, Self::stretch(floor, ground, limit, at, &area)?));
            }
        }
        Ok(found)
    }

    /// The stretch in front of a portal face whose zone reaches no piece:
    /// nowhere on the surface there does the body fit.
    fn blocked_face(
        &self,
        floor: &Floor,
        ground: &Ground,
        zone: &Region,
        at: Point2,
    ) -> Result<Option<WalkableStretch>, String> {
        let Self::Split(split) = self else {
            return Ok(None);
        };
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let limit = Self::limit(split, |pieces| {
            for piece in pieces {
                if !piece
                    .intersection(zone, t)
                    .map_err(overlay_error("portal zone"))?
                    .is_empty()
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
        Self::stretch(floor, ground, limit, at, &plan_of(zone)).map(Some)
    }

    /// The piece that holds the hub, `surface:{id}` itself.
    fn hub_piece(&self) -> usize {
        match self {
            Self::Whole => 0,
            Self::Split(split) => split.hub,
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
            Self::Split(split) if !split.pieces.is_empty() => (0..split.pieces.len())
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
    /// portal's zone, and the hub's piece unless floor too narrow for the
    /// body joins it to one of those (a stretch then says where), so a
    /// separation is a passage too narrow rather than a missing one. With
    /// them the zone, when the surface is split.
    fn near(
        &self,
        id: &ObjectId,
        half: Option<&Plan>,
        swing: &[Polygon],
        radius: f64,
    ) -> Result<(Vec<Reach>, Option<Region>), String> {
        let Self::Split(split) = self else {
            return Ok((vec![(0, surface_region(id), true)], None));
        };
        let zone = Self::zone(half, swing, radius)?;
        if split.pieces.is_empty() {
            return Ok((vec![(0, surface_region(id), false)], Some(zone)));
        }
        let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
        let mut near = Vec::new();
        for (index, piece) in split.pieces.iter().enumerate() {
            let meets = !piece
                .intersection(&zone, t)
                .map_err(overlay_error("portal zone"))?
                .is_empty();
            if meets {
                near.push((index, self.name(id, index), true));
            }
        }
        let hub = split.hub;
        if near
            .iter()
            .all(|(index, ..)| *index != hub && !Self::joined(split, *index, hub))
        {
            near.push((hub, self.name(id, hub), false));
        }
        Ok((near, Some(zone)))
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

/// The floor between two surfaces within `gap` of each other: the points
/// within the gap of both (from inside the exact set), less what any
/// obstacle occupies over either and every swing's circumscribed polygon.
fn bridge(
    first: &Floor,
    second: &Floor,
    one: &Ground,
    other: &Ground,
    gap: f64,
) -> Result<Plan, String> {
    let t = crate::free_space::tolerance().map_err(|error| error.to_string())?;
    let near = |floor: &Floor| -> Result<Region, String> {
        region_of(&floor.footprint)?
            .dilate_inner(gap, t)
            .map_err(overlay_error("surface gap"))
    };
    let both = near(first)?
        .intersection(&near(second)?, t)
        .map_err(overlay_error("surface gap"))?;
    let mut floor = plan_of(&both);
    for ground in [one, other] {
        floor = subtract(&floor, &ground.raw)?;
        floor = subtract(&floor, &ground.swings_but(None)?)?;
    }
    Ok(floor)
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
        let gap = request.surface_gap_metres();
        let mut grounds = Vec::with_capacity(floors.len());
        for floor in &floors {
            let (lo, hi) = band_limits(band, floor.z0, floor.top);
            let blockers = obstructions(&obstacles, &grown(&floor.bounds, gap), lo, hi)?;
            grounds.push(Ground::new(
                floor,
                blockers,
                request.swept_doors(),
                hi,
                request.obstruction_depth_metres(),
            )?);
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
                .map(|(_, ground)| &ground.wide)
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

        // Surfaces that touch, or lie within the surface gap, and those a
        // connector joins, stay one region: a body may stand across their
        // shared boundary, or the gap between them.
        let mut touches = Vec::new();
        for a in 0..floors.len() {
            for b in a + 1..floors.len() {
                let (first, second) = (&floors[a], &floors[b]);
                if first.z0 >= second.top - ON_SURFACE
                    || second.z0 >= first.top - ON_SURFACE
                    || !within(&first.footprint, &second.footprint, gap)?
                {
                    continue;
                }
                let bridged = gap > 0.0 && !touching(&first.footprint, &second.footprint)?;
                touches.push((a, b, bridged));
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
                && touches.iter().all(|&(a, b, _)| a != index && b != index);
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
                let (near, zone) = split.near(&floor.id, half, ground.swing_of(id), radius)?;
                // Nowhere in front of the face does the body fit: the
                // stretch there says why.
                let stranded = match &zone {
                    Some(zone) if near.iter().all(|(_, _, meets)| !meets) => {
                        let sign = if slot == 0 { -1.0 } else { 1.0 };
                        let at = frame.point(
                            f64::midpoint(frame.u0, frame.u1),
                            sign * (frame.half + side.gap),
                        );
                        split.blocked_face(floor, ground, zone, at)?
                    }
                    _ => None,
                };
                for (piece, name, meets) in near {
                    let definite = proven && piece == split.hub_piece();
                    let open = definite || meets;
                    let mut spoke = passage(
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
                            match (&stranded, open) {
                                (_, true) => String::new(),
                                (Some(stretch), false) =>
                                    format!(":separated:{}", stretch.limit().as_str()),
                                (None, false) => ":separated".to_owned(),
                            }
                        ),
                    )?;
                    if let (Some(stretch), false) = (&stranded, open) {
                        spoke = spoke
                            .with_stretch(stretch.clone())
                            .map_err(|e| e.to_string())?;
                    }
                    passages.push(spoke);
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

        // Surfaces touching surfaces, or across a gap within the surface
        // gap: the points within the gap of both surfaces are floor too,
        // less what any obstacle occupies there.
        for &(a, b, bridged) in &touches {
            let (first, second) = (&floors[a], &floors[b]);
            let proven = match (hubs[a], hubs[b]) {
                (Some(from), Some(to)) => {
                    let mut domain = join(&grounds[a].free, &grounds[b].free)?;
                    if bridged {
                        domain = join(
                            &domain,
                            &bridge(first, second, &grounds[a], &grounds[b], gap)?,
                        )?;
                    }
                    witness(&domain, from, to, radius)?.is_some()
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
                    "{}:{}<->{}:sweep={}",
                    if bridged {
                        format!("gap<={gap:.6}")
                    } else {
                        "touch".to_owned()
                    },
                    first.id,
                    second.id,
                    if proven { "proven" } else { "unproven" }
                ),
            )?);
        }

        // Floor too narrow for the body between the pieces of one surface.
        for ((floor, ground), split) in floors.iter().zip(&grounds).zip(&parts) {
            for (a, b, stretch) in split.stretches(floor, ground, radius)? {
                let [x, y, z] = stretch.at();
                let locator = format!(
                    "stretch:{}:{}:at={x:.4},{y:.4},{z:.4}",
                    floor.id,
                    stretch.limit().as_str()
                );
                passages.push(
                    passage(
                        &floor.id,
                        split.name(&floor.id, a),
                        split.name(&floor.id, b),
                        None,
                        0.0,
                        required - 2.0 * MARGIN,
                        locator,
                    )?
                    .with_stretch(stretch)
                    .map_err(|e| e.to_string())?,
                );
            }
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
