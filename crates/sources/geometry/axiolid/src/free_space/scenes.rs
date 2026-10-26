//! Placement scenes as a derivation of the check's store (ADR 0006, #408).
//!
//! A scene (a scope's footprint less its obstacles in a band, with the
//! swept doors' sectors) is a pure function of what it is built from, so
//! one build answers every rule asking for it: a circle, a rectangle, a
//! largest-disc estimate and a circulation map over the same space,
//! obstacles and band share it across rules and threads, built once at a
//! time (single flight). [`SceneKey`] names everything a scene's value
//! depends on, the service state it is built over included.
//!
//! The value keeps what searches of the scene took (its clear parts, their
//! largest discs, simplified and coarse regions, the regions found for
//! a shape and the rectangle searches asked, #427), merged back after each
//! search ([`KeptScene::keep`]).
//! Every search is charged what it takes as if it had built them
//! (`planar::budget::Work`, #434), so no search decides otherwise for
//! what another kept (#441).
//!
//! A scene's key carries its precision class (#438): Exact, as every
//! request asks today, or Fast with its budget, whose bracketing
//! morphology runs in the overlay's fast mode. The two are two entries,
//! and a Fast scene carries its excess ([`KeptScene::excess`]), so an
//! Exact read never receives a Fast scene, however alike the rest of
//! their keys. What a scene's memo keeps is held to its class too: a
//! region stating an excess (a search's disc morphology in the fast mode)
//! is never kept with an Exact scene.

use std::any::TypeId;
use std::cell::OnceCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use axiolid_overlay::{MorphologyMode, Region};
use axioval_engine::{
    DerivationError, DerivationKind, DerivationStore, DerivationVersion, ElevationBand,
    FreeSpaceError, FreeSpaceServiceHandle, Inputs, Precision, SweptDoor, derivation,
};
use axioval_ir::ObjectId;

use super::Measuring;
use crate::derivations::{
    self, AXIOLID_CORE, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_OVERLAY, Over,
};
use crate::placement::{self, Inscribed, Scene, region_points};
use crate::planar::budget::Work;

/// A list of obstacles, interned by the service ([`Lists`]): equal lists
/// share one allocation, so a key compares and hashes the list by its
/// address and holds it once however many scenes are keyed by it (every
/// object of a large model can be every space's obstacle).
#[derive(Clone)]
pub(crate) struct Listed(Arc<[ObjectId]>);

impl PartialEq for Listed {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Listed {}

impl Hash for Listed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).cast::<ObjectId>().hash(state);
    }
}

/// The obstacle lists a service's scenes are keyed by, each once.
#[derive(Default)]
pub(crate) struct Lists(Mutex<HashSet<Arc<[ObjectId]>>>);

impl Lists {
    /// `objects` as the one list of this service equal to it.
    pub(crate) fn listed(&self, objects: &[ObjectId]) -> Listed {
        let mut lists = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(list) = lists.get(objects) {
            return Listed(Arc::clone(list));
        }
        let list: Arc<[ObjectId]> = objects.into();
        lists.insert(Arc::clone(&list));
        Listed(list)
    }
}

/// Swept doors, compared and hashed by their sectors' bits.
#[derive(Clone)]
pub(crate) struct Doors(pub(crate) Vec<SweptDoor>);

impl Doors {
    /// Each door with the bits of its sectors: hinge, radius, closed and
    /// open directions, and whether it is double-acting.
    fn bits(&self) -> impl Iterator<Item = (&ObjectId, Vec<[u64; 11]>)> {
        self.0.iter().map(|door| {
            let sectors = door
                .sectors()
                .iter()
                .map(|sector| {
                    let [hx, hy, hz] = sector.hinge();
                    let [cx, cy, cz] = sector.closed().components();
                    let [ox, oy, oz] = sector.open().components();
                    [
                        hx.to_bits(),
                        hy.to_bits(),
                        hz.to_bits(),
                        sector.radius_metres().to_bits(),
                        cx.to_bits(),
                        cy.to_bits(),
                        cz.to_bits(),
                        ox.to_bits(),
                        oy.to_bits(),
                        oz.to_bits(),
                        u64::from(sector.is_double_acting()),
                    ]
                })
                .collect();
            (door.door(), sectors)
        })
    }
}

impl PartialEq for Doors {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.bits().eq(other.bits())
    }
}

impl Eq for Doors {}

impl Hash for Doors {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for door in self.bits() {
            door.hash(state);
        }
    }
}

/// The overlay's morphology mode for a scene of `precision`: exact, or
/// fast within its budget.
fn mode(precision: Precision) -> MorphologyMode {
    precision
        .budget_metres()
        .map_or(MorphologyMode::Exact, MorphologyMode::fast)
}

/// Everything a placement scene is built from (#408, #439): the service
/// state (its geometry, by identity), the scope and merged scopes, the
/// obstacles, the swept doors, the band's limits and the tolerance by their
/// bits and the precision class (#438, Exact for every request; Fast only
/// where a caller asks for it, [`Self::at`]); the kind's version
/// ([`PlacementScenes::VERSION`]) beside it. Never the shape, its domain
/// or its entrance reach, which a search sets on its copy. The scene's
/// plan frame (the scope's corner, `Measuring::floor_scene`) is a function
/// of the geometry and the scope.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SceneKey {
    pub(crate) over: Over<Measuring>,
    pub(crate) scope: ObjectId,
    pub(crate) merged: Vec<ObjectId>,
    pub(crate) obstacles: Listed,
    pub(crate) swept: Doors,
    pub(crate) band: [u64; 2],
    pub(crate) tolerance: [u64; 2],
    pub(crate) precision: Precision,
}

impl SceneKey {
    /// The key of the scene of `scope` (with `merged`), less `obstacles`
    /// and the `swept` doors' sectors in `band`, built over `over` at
    /// `tolerance`.
    pub(crate) fn new(
        over: &Arc<Measuring>,
        (scope, merged): (&ObjectId, &[ObjectId]),
        (obstacles, swept): (&[ObjectId], &[SweptDoor]),
        band: ElevationBand,
        tolerance: axiolid_core::Tolerance,
    ) -> Self {
        Self {
            over: Over(Arc::clone(over)),
            scope: scope.clone(),
            merged: merged.to_vec(),
            obstacles: over.lists.listed(obstacles),
            swept: Doors(swept.to_vec()),
            band: [band.from_metres().to_bits(), band.to_metres().to_bits()],
            tolerance: [tolerance.linear().to_bits(), tolerance.angular().to_bits()],
            precision: Precision::Exact,
        }
    }

    /// The same scene in the precision class `precision`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn at(self, precision: Precision) -> Self {
        Self { precision, ..self }
    }

    fn band(&self) -> Result<ElevationBand, FreeSpaceError> {
        ElevationBand::try_new(f64::from_bits(self.band[0]), f64::from_bits(self.band[1]))
    }

    fn tolerance(&self) -> Result<axiolid_core::Tolerance, FreeSpaceError> {
        axiolid_core::Tolerance::new(
            f64::from_bits(self.tolerance[0]),
            f64::from_bits(self.tolerance[1]),
        )
        .map_err(|error| FreeSpaceError::Unavailable(format!("tolerance: {error:?}")))
    }

    /// Builds the scene, as `Measuring::floor_scene` does, its bracketing
    /// morphology in the key's precision class.
    fn build(&self) -> Result<KeptScene, FreeSpaceError> {
        let (scene, floor, excess) = self.over.0.floor_scene(
            &self.scope,
            &self.merged,
            &self.obstacles.0,
            &self.swept.0,
            self.band()?,
            self.tolerance()?,
            mode(self.precision),
            None,
        )?;
        Ok(KeptScene::of(&scene, floor, self.precision, excess))
    }
}

/// The derivation kind of placement scenes.
pub(crate) struct PlacementScenes;

impl DerivationKind for PlacementScenes {
    const NAME: &'static str = "placement scene";
    /// Bumped whenever the same key could build another scene: the
    /// footprints, the bracketing of tessellated bodies, the overlay or its
    /// kernel crates changing what they return, so a value kept across
    /// checks (#409) is never read for another.
    const VERSION: DerivationVersion = DerivationVersion::new(
        2,
        &[AXIOLID_CORE, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_OVERLAY],
    );
    type Key = SceneKey;
    /// A scene the service refuses to build is refused alike for every
    /// rule asking for it.
    type Value = Result<KeptScene, FreeSpaceError>;

    fn build(key: &SceneKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.build())
    }

    fn precision(key: &SceneKey) -> Option<Precision> {
        Some(key.precision)
    }

    /// A Fast scene's excess, what its memo keeps included; an Exact
    /// scene, and a refused one, carry none.
    fn excess(value: &Self::Value) -> Option<f64> {
        value.as_ref().ok().and_then(KeptScene::excess)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, KeptScene::bytes))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Scenes are read by the free-space service's searches, estimates and
    /// circulation maps alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<FreeSpaceServiceHandle>()])
    }
}

/// A placement scene read from the store, with its key.
pub(crate) struct Derived {
    key: SceneKey,
    value: Arc<Result<KeptScene, FreeSpaceError>>,
    /// Whether it is the store's (kept for later reads), so what its
    /// searches take is counted against it.
    kept: bool,
}

impl Derived {
    /// The scene of `key`: the check's, built once (single flight), or,
    /// with `reuse` off, built for this request alone.
    pub(crate) fn of(key: SceneKey, reuse: bool) -> Result<Self, FreeSpaceError> {
        if !reuse {
            let value = Arc::new(key.build());
            return Ok(Self {
                key,
                value,
                kept: false,
            });
        }
        let value = DerivationStore::derive_current::<PlacementScenes>(&key)
            .map_err(|error| FreeSpaceError::Unavailable(error.to_string()))?;
        Ok(Self {
            key,
            value,
            kept: true,
        })
    }

    /// The scene as built, or why it was refused.
    pub(crate) fn kept(&self) -> Result<&KeptScene, FreeSpaceError> {
        self.value.as_ref().as_ref().map_err(Clone::clone)
    }

    /// Keeps what a search of the scene took, for the next search over it,
    /// and states again the bytes the store holds for it.
    pub(crate) fn keep(&self, scene: &Scene) {
        let Ok(kept) = self.value.as_ref() else {
            return;
        };
        kept.keep(scene);
        if self.kept
            && let Some(store) = derivation::current()
        {
            store.resize::<PlacementScenes>(&self.key, kept.bytes());
        }
    }
}

/// What searches of a scene took ([`Scene::clear`] and the rest), each
/// region within the scene's precision class.
struct Memo {
    clear: [OnceCell<Region>; 2],
    inscribed: [OnceCell<Option<Inscribed>>; 2],
    simple: [OnceCell<Option<Region>>; 4],
    coarse: [OnceCell<Option<Region>>; 6],
    found: placement::Found,
    searched: placement::Searched,
    /// The greatest excess a kept region states; `None` while none does.
    excess: Option<f64>,
}

/// A placement scene as built, before a window or a reach is set, with
/// what its searches took and its floor, in its precision class.
pub(crate) struct KeptScene {
    scope: Region,
    room: Region,
    obstacles: Region,
    sure: Region,
    tolerance: axiolid_core::Tolerance,
    bracketed: Vec<(ObjectId, f64)>,
    bounded: Vec<ObjectId>,
    pub(crate) floor: f64,
    /// The class it was built in (#438).
    precision: Precision,
    /// The greatest excess its bracketing morphology states: `None` for an
    /// Exact scene, and for a Fast one where no morphology ran.
    excess: Option<f64>,
    memo: Mutex<Memo>,
}

/// Fills `held` with what `taken` has, unless it holds a value already.
fn fill_one<T: Clone>(held: &OnceCell<T>, taken: &OnceCell<T>) {
    if let Some(value) = taken.get() {
        let _ = held.set(value.clone());
    }
}

/// Fills the cells of `held` that `taken` has and it lacks, with the
/// values `admits` keeps.
fn fill<T: Clone, const N: usize>(
    held: &[OnceCell<T>; N],
    taken: &[OnceCell<T>; N],
    mut admits: impl FnMut(&T) -> bool,
) {
    for (held, taken) in held.iter().zip(taken) {
        if taken.get().is_some_and(&mut admits) {
            fill_one(held, taken);
        }
    }
}

/// The corners of the regions of `cells`, those not taken counting none.
fn cell_points<const N: usize>(cells: &[OnceCell<Option<Region>>; N]) -> usize {
    cells
        .iter()
        .filter_map(|cell| cell.get().and_then(Option::as_ref))
        .map(region_points)
        .sum()
}

/// What a corner of a kept region takes, its ring's share included.
const CORNER_BYTES: usize = 24;

impl KeptScene {
    fn of(scene: &Scene, floor: f64, precision: Precision, excess: Option<f64>) -> Self {
        let kept = Self {
            scope: scene.scope.clone(),
            room: scene.room.clone(),
            obstacles: scene.obstacles.clone(),
            sure: scene.sure.clone(),
            tolerance: scene.tolerance,
            bracketed: scene.bracketed.clone(),
            bounded: scene.bounded.clone(),
            floor,
            precision,
            excess,
            memo: Mutex::new(Memo {
                clear: Default::default(),
                inscribed: Default::default(),
                simple: Default::default(),
                coarse: Default::default(),
                found: placement::Found::default(),
                searched: placement::Searched::default(),
                excess: None,
            }),
        };
        kept.keep(scene);
        kept
    }

    /// The greatest excess the scene carries (#438): its bracketing
    /// morphology's, or a region its memo keeps; `None` for an Exact
    /// scene, which keeps no region stating one.
    pub(crate) fn excess(&self) -> Option<f64> {
        let kept = self.memo().excess;
        match (self.excess, kept) {
            (Some(built), Some(kept)) => Some(built.max(kept)),
            (built, kept) => built.or(kept),
        }
    }

    fn memo(&self) -> std::sync::MutexGuard<'_, Memo> {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps what a search of the scene took that this one lacks: its
    /// clear parts, their largest discs, the simplified and coarse
    /// regions and the regions it found. A region stating an excess the
    /// scene's class does not hold (#438: a fast result, for an Exact
    /// scene) is never kept; one it holds raises the scene's excess.
    fn keep(&self, scene: &Scene) {
        let mut memo = self.memo();
        let precision = self.precision;
        let mut excess = memo.excess;
        let mut admits = |region: &Region| {
            let more = region.excess();
            if !precision.holds(more) {
                return false;
            }
            if let Some(more) = more {
                excess = Some(excess.map_or(more, |held: f64| held.max(more)));
            }
            true
        };
        // A clear part's largest disc is kept only where the part may be.
        for (side, taken) in scene.clear.iter().enumerate() {
            if taken.get().is_none_or(&mut admits) {
                fill_one(&memo.clear[side], taken);
                fill_one(&memo.inscribed[side], &scene.inscribed[side]);
            }
        }
        fill(&memo.simple, &scene.simple, |region| {
            region.as_ref().is_none_or(&mut admits)
        });
        fill(&memo.coarse, &scene.coarse, |region| {
            region.as_ref().is_none_or(&mut admits)
        });
        for (what, corners, steps, free) in scene.found.borrow().iter() {
            if admits(free) {
                placement::keep_found(&memo.found, *what, corners, steps, free);
            }
        }
        // A search's verdict states no excess: it holds in the scene's
        // class, over which it was searched.
        for asked in scene.searched.borrow().iter() {
            placement::keep_searched(&memo.searched, asked);
        }
        memo.excess = excess;
    }

    /// The scene again, as built, with what its searches took; a search
    /// starts its own work afresh.
    pub(crate) fn scene(&self) -> Scene {
        let memo = self.memo();
        Scene {
            clear: memo.clear.clone(),
            inscribed: memo.inscribed.clone(),
            simple: memo.simple.clone(),
            coarse: memo.coarse.clone(),
            found: memo.found.clone(),
            searched: memo.searched.clone(),
            ..self.bare()
        }
    }

    /// The scene as built, without what searches took.
    pub(crate) fn bare(&self) -> Scene {
        Scene {
            scope: self.scope.clone(),
            room: self.room.clone(),
            obstacles: self.obstacles.clone(),
            sure: self.sure.clone(),
            tolerance: self.tolerance,
            window: None,
            reach: None,
            bracketed: self.bracketed.clone(),
            bounded: self.bounded.clone(),
            clear: Default::default(),
            inscribed: Default::default(),
            simple: Default::default(),
            coarse: Default::default(),
            found: placement::Found::default(),
            searched: placement::Searched::default(),
            work: Work::default(),
        }
    }

    /// The bytes the scene holds, by its regions' corners.
    pub(crate) fn bytes(&self) -> usize {
        let memo = self.memo();
        let built = [&self.scope, &self.room, &self.obstacles, &self.sure]
            .into_iter()
            .map(region_points)
            .sum::<usize>();
        let clear = memo
            .clear
            .iter()
            .filter_map(OnceCell::get)
            .map(region_points)
            .sum::<usize>();
        let found = memo
            .found
            .borrow()
            .iter()
            .map(|(_, _, _, region)| region_points(region))
            .sum::<usize>();
        let searched = memo.searched.borrow();
        let reached = searched.iter().map(placement::Asked::points).sum::<usize>();
        let points =
            built + clear + cell_points(&memo.simple) + cell_points(&memo.coarse) + found + reached;
        points * CORNER_BYTES
            + std::mem::size_of::<Self>()
            + searched.len() * std::mem::size_of::<placement::Asked>()
            + self.bracketed.len() * std::mem::size_of::<(ObjectId, f64)>()
            + self.bounded.len() * std::mem::size_of::<ObjectId>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::free_space::AxiolidFreeSpaceService;
    use crate::geometry::AxiolidGeometry;
    use axiolid_core::Point3;
    use axiolid_mesh::TriMesh;
    use axioval_engine::{KeyCompleteness, MetricDirection, SwingSector};
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn direction(components: [f64; 3]) -> MetricDirection {
        MetricDirection::try_new(components).unwrap()
    }

    fn door(local: &str, hinge: [f64; 3], radius: f64, double: bool) -> SweptDoor {
        let sector = SwingSector::try_new(
            hinge,
            radius,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            double,
        )
        .unwrap();
        SweptDoor::try_new(id(local), vec![sector]).unwrap()
    }

    /// A closed, outward-wound box.
    fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        let positions = vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ];
        TriMesh::new(positions, indices)
    }

    /// A 5 m by 4 m room beside a 3 m by 4 m hall, with a table, a chair
    /// and a shelf hung a metre above the floor.
    fn furnished() -> AxiolidGeometry {
        AxiolidGeometry::new()
            .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [5.0, 4.0, 3.0]))
            .with_mesh(id("hall"), cuboid([5.0, 0.0, 0.0], [8.0, 4.0, 3.0]))
            .with_mesh(id("table"), cuboid([0.0, 0.0, 0.0], [1.2, 0.8, 0.75]))
            .with_mesh(id("chair"), cuboid([2.0, 2.0, 0.0], [2.5, 2.5, 0.9]))
            .with_mesh(id("shelf"), cuboid([4.0, 3.0, 1.0], [5.0, 4.0, 2.5]))
    }

    fn service() -> AxiolidFreeSpaceService {
        AxiolidFreeSpaceService::new(furnished(), SourceId::new("cad", "m").unwrap())
    }

    /// What of a scene a test compares: the areas of its scope and room,
    /// its obstacles' outlines, its floor, or why it was refused.
    fn fingerprint(value: &Result<KeptScene, FreeSpaceError>) -> String {
        match value {
            Ok(scene) => format!(
                "scope {:.6}, room {:.6}, obstacles {:?} (surely {:?}), floor {:.6}",
                scene.scope.area(),
                scene.room.area(),
                scene.obstacles.boundary_rings(),
                scene.sure.boundary_rings(),
                scene.floor
            ),
            Err(error) => format!("refused: {error:?}"),
        }
    }

    /// Every input a scene is built from is in its key (#439): changing any
    /// one of them changes the key, and the scene where it builds another;
    /// the same inputs give the same key.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn changing_any_input_of_a_scene_changes_its_key() {
        let (one, other) = (service(), service());
        let tolerance = axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let band = ElevationBand::try_new(0.0, 2.0).unwrap();
        let obstacles = [id("table"), id("chair"), id("shelf")];
        let swept = [door("door", [3.0, 0.0, 0.0], 0.9, false)];
        let key = |over: &AxiolidFreeSpaceService,
                   (scope, merged): (&ObjectId, &[ObjectId]),
                   (obstacles, swept): (&[ObjectId], &[SweptDoor]),
                   band: ElevationBand,
                   tolerance: axiolid_core::Tolerance| {
            SceneKey::new(
                &over.over,
                (scope, merged),
                (obstacles, swept),
                band,
                tolerance,
            )
        };
        let room = (&id("room"), &[][..]);
        let base = key(&one, room, (&obstacles, &swept), band, tolerance);
        KeyCompleteness::<PlacementScenes, _>::new(base.clone(), fingerprint)
            .same(
                "the same inputs",
                key(&one, room, (&obstacles, &swept), band, tolerance),
            )
            // Another service over equal geometry builds an equal scene.
            .input(
                "the service",
                key(&other, room, (&obstacles, &swept), band, tolerance),
            )
            .changes_value(
                "the scope",
                key(
                    &one,
                    (&id("hall"), &[]),
                    (&obstacles, &swept),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "the merged scopes",
                key(
                    &one,
                    (&id("room"), &[id("hall")]),
                    (&obstacles, &swept),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "the obstacles",
                key(&one, room, (&obstacles[..1], &swept), band, tolerance),
            )
            // The scene unites them in any order.
            .input(
                "the obstacles' order",
                key(
                    &one,
                    room,
                    (&[id("chair"), id("table"), id("shelf")], &swept),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "no swept door",
                key(&one, room, (&obstacles, &[]), band, tolerance),
            )
            // The same sector of another door.
            .input(
                "the swept door",
                key(
                    &one,
                    room,
                    (&obstacles, &[door("other", [3.0, 0.0, 0.0], 0.9, false)]),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "the hinge",
                key(
                    &one,
                    room,
                    (&obstacles, &[door("door", [2.0, 0.0, 0.0], 0.9, false)]),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "the radius",
                key(
                    &one,
                    room,
                    (&obstacles, &[door("door", [3.0, 0.0, 0.0], 0.8, false)]),
                    band,
                    tolerance,
                ),
            )
            .changes_value(
                "double-acting",
                key(
                    &one,
                    room,
                    (&obstacles, &[door("door", [3.0, 0.0, 0.0], 0.9, true)]),
                    band,
                    tolerance,
                ),
            )
            // Above the table and below the chair's top.
            .changes_value(
                "the band's bottom",
                key(
                    &one,
                    room,
                    (&obstacles, &swept),
                    ElevationBand::try_new(0.8, 2.0).unwrap(),
                    tolerance,
                ),
            )
            // Below the shelf.
            .changes_value(
                "the band's top",
                key(
                    &one,
                    room,
                    (&obstacles, &swept),
                    ElevationBand::try_new(0.0, 0.95).unwrap(),
                    tolerance,
                ),
            )
            // The overlay snaps within a tolerance no corner here is near.
            .input(
                "the linear tolerance",
                key(
                    &one,
                    room,
                    (&obstacles, &swept),
                    band,
                    axiolid_core::Tolerance::new(1.0e-6, 1.0e-9).unwrap(),
                ),
            )
            .input(
                "the angular tolerance",
                key(
                    &one,
                    room,
                    (&obstacles, &swept),
                    band,
                    axiolid_core::Tolerance::new(1.0e-9, 1.0e-6).unwrap(),
                ),
            )
            // Over exact boxes the fast mode has nothing to fill: the same
            // regions, another entry.
            .precision(
                "the precision class",
                base.clone().at(Precision::fast(FAST_BUDGET)),
            )
            .input(
                "the fast budget",
                base.clone().at(Precision::fast(2.0 * FAST_BUDGET)),
            )
            .check();
        assert_eq!(base.precision, Precision::Exact);
    }

    /// The budget the tests' Fast scenes are asked within.
    const FAST_BUDGET: f64 = 1.0e-3;

    /// The room with a round table, tessellated within 2 mm: its
    /// bracketing morphology is what a Fast scene takes in the fast mode.
    fn with_round_table() -> AxiolidFreeSpaceService {
        let sides = 48;
        let (centre, radius, height) = ([2.5, 2.0], 0.6, 0.75);
        let mut positions = Vec::new();
        for z in [0.0, height] {
            for i in 0..sides {
                let angle = 2.0 * std::f64::consts::PI * f64::from(i) / f64::from(sides);
                positions.push(Point3::new(
                    centre[0] + radius * angle.cos(),
                    centre[1] + radius * angle.sin(),
                    z,
                ));
            }
        }
        positions.push(Point3::new(centre[0], centre[1], 0.0));
        positions.push(Point3::new(centre[0], centre[1], height));
        let (bottom, top) = (2 * sides, 2 * sides + 1);
        let mut indices = Vec::new();
        for i in 0..sides {
            let j = (i + 1) % sides;
            indices.extend([bottom, j, i, top, sides + i, sides + j]);
            indices.extend([i, j, sides + j, i, sides + j, sides + i]);
        }
        let geometry = furnished().with_tessellated_mesh(
            id("round"),
            TriMesh::new(positions, indices),
            2.0e-3,
        );
        AxiolidFreeSpaceService::new(geometry, SourceId::new("cad", "m").unwrap())
    }

    /// The done-when of #438: one scene asked once Exact and once Fast is
    /// two entries of the check's store, and an Exact read never returns
    /// the Fast one, whichever was asked first and however often.
    #[test]
    fn a_scene_asked_exact_and_fast_is_two_entries_and_exact_never_reads_the_fast_one() {
        let service = with_round_table();
        let tolerance = axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let exact = SceneKey::new(
            &service.over,
            (&id("room"), &[]),
            (&[id("round"), id("chair")], &[]),
            ElevationBand::try_new(0.0, 2.0).unwrap(),
            tolerance,
        );
        let fast = exact.clone().at(Precision::fast(FAST_BUDGET));
        let store = DerivationStore::new();
        let _entered = store.enter();
        let fast_scene = Derived::of(fast.clone(), true).unwrap();
        // Held Fast, the Exact key finds nothing and builds its own.
        assert!(store.held::<PlacementScenes>(&exact).is_none());
        let exact_scene = Derived::of(exact.clone(), true).unwrap();
        assert!(!Arc::ptr_eq(&exact_scene.value, &fast_scene.value));
        let statistics = store.statistics();
        assert_eq!((statistics[0].entries, statistics[0].builds), (2, 2));
        // The Fast scene carries its excess, within the budget; the Exact
        // one none, and each key reads its own again.
        let excess = fast_scene.kept().unwrap().excess();
        assert!(excess.is_some_and(|excess| (0.0..=FAST_BUDGET).contains(&excess)));
        assert_eq!(exact_scene.kept().unwrap().excess(), None);
        for _ in 0..3 {
            let again = Derived::of(exact.clone(), true).unwrap();
            assert!(Arc::ptr_eq(&again.value, &exact_scene.value));
            assert_eq!(again.kept().unwrap().excess(), None);
        }
        let again = Derived::of(fast, true).unwrap();
        assert!(Arc::ptr_eq(&again.value, &fast_scene.value));
        // Searched alike, both decide alike here.
        for scene in [&exact_scene, &fast_scene] {
            let searched = scene.kept().unwrap().scene();
            let verdict = placement::circle(&searched, 0.4).unwrap();
            assert!(matches!(verdict, placement::Search::Found { .. }));
            scene.keep(&searched);
        }
        assert_eq!(exact_scene.kept().unwrap().excess(), None);
    }

    /// What a scene's memo keeps is held to its class: a region stating an
    /// excess (a disc morphology in the fast mode) is never kept with an
    /// Exact scene, and raises a Fast scene's excess within its budget.
    #[test]
    fn an_exact_scene_never_keeps_a_fast_region() {
        let service = with_round_table();
        let tolerance = axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let exact = SceneKey::new(
            &service.over,
            (&id("room"), &[]),
            (&[id("round")], &[]),
            ElevationBand::try_new(0.0, 2.0).unwrap(),
            tolerance,
        );
        for precision in [Precision::Exact, Precision::fast(FAST_BUDGET)] {
            let derived = Derived::of(exact.clone().at(precision), false).unwrap();
            let kept = derived.kept().unwrap();
            let built = kept.excess();
            let searched = kept.scene();
            let fast = searched
                .scope
                .erode_inner_with(0.3, tolerance, MorphologyMode::fast(FAST_BUDGET))
                .unwrap();
            assert!(fast.excess().is_some());
            let _ = searched.clear[0].set(fast.clone());
            let _ = searched.simple[2].set(Some(fast));
            kept.keep(&searched);
            let again = kept.scene();
            assert_eq!(again.clear[0].get().is_some(), !precision.is_exact());
            assert_eq!(again.simple[2].get().is_some(), !precision.is_exact());
            if precision.is_exact() {
                assert_eq!(kept.excess(), None);
            } else {
                assert!(kept.excess().is_some_and(|excess| excess <= FAST_BUDGET));
                assert!(kept.excess() >= built);
            }
        }
    }

    /// Equal obstacle lists are one list in a service's keys, however
    /// often a request states them.
    #[test]
    fn a_service_keeps_each_obstacle_list_once() {
        let lists = Lists::default();
        let first = lists.listed(&[id("a"), id("b")]);
        let again = lists.listed(&[id("a"), id("b")]);
        assert!(first == again && Arc::ptr_eq(&first.0, &again.0));
        assert!(first != lists.listed(&[id("b"), id("a")]));
    }
}
