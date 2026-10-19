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
//! largest discs, simplified and coarse regions, and the regions found for
//! a shape, #427), merged back after each search ([`KeptScene::keep`]).
//! Every search is charged what it takes as if it had built them
//! (`planar::budget::Work`, #434), so no search decides otherwise for
//! what another kept (#441).

use std::cell::OnceCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use axiolid_overlay::Region;
use axioval_engine::{
    DerivationError, DerivationKind, DerivationStore, ElevationBand, FreeSpaceError, Inputs,
    SweptDoor, derivation,
};
use axioval_ir::ObjectId;

use super::Measuring;
use crate::derivations::{self, Over};
use crate::placement::{self, Inscribed, Scene, region_points};
use crate::planar::budget::Work;

/// The version of how a scene is built from its key: bumped whenever the
/// same key could build another scene (the footprints, the bracketing of
/// tessellated bodies, the overlay or its kernel crates changing what they
/// return), so a value kept across checks (#409) is never read for another.
pub(crate) const SCENE_VERSION: u32 = 1;

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

/// The precision a value is built at (#438): a scene's overlay operations
/// are exact; a Fast scene, should one be built, never answers a key
/// asking for an Exact one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Precision {
    Exact,
}

/// Everything a placement scene is built from (#408, #439): the service
/// state (its geometry, by identity), the scope and merged scopes, the
/// obstacles, the swept doors, the band's limits and the tolerance by their
/// bits, the precision and [`SCENE_VERSION`]. Never the shape, its domain
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
    pub(crate) version: u32,
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
            version: SCENE_VERSION,
        }
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

    /// Builds the scene, as `Measuring::floor_scene` does.
    fn build(&self) -> Result<KeptScene, FreeSpaceError> {
        let (scene, floor) = self.over.0.floor_scene(
            &self.scope,
            &self.merged,
            &self.obstacles.0,
            &self.swept.0,
            self.band()?,
            self.tolerance()?,
            None,
        )?;
        Ok(KeptScene::of(&scene, floor))
    }
}

/// The derivation kind of placement scenes.
pub(crate) struct PlacementScenes;

impl DerivationKind for PlacementScenes {
    const NAME: &'static str = "placement scene";
    type Key = SceneKey;
    /// A scene the service refuses to build is refused alike for every
    /// rule asking for it.
    type Value = Result<KeptScene, FreeSpaceError>;

    fn build(key: &SceneKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.build())
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, KeptScene::bytes))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
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

/// What searches of a scene took ([`Scene::clear`] and the rest).
struct Memo {
    clear: [OnceCell<Region>; 2],
    inscribed: [OnceCell<Option<Inscribed>>; 2],
    simple: [OnceCell<Option<Region>>; 4],
    coarse: [OnceCell<Option<Region>>; 6],
    found: placement::Found,
}

/// A placement scene as built, before a window or a reach is set, with
/// what its searches took and its floor.
pub(crate) struct KeptScene {
    scope: Region,
    room: Region,
    obstacles: Region,
    sure: Region,
    tolerance: axiolid_core::Tolerance,
    bracketed: Vec<(ObjectId, f64)>,
    bounded: Vec<ObjectId>,
    pub(crate) floor: f64,
    memo: Mutex<Memo>,
}

/// Fills the cells of `held` that `taken` has and it lacks.
fn fill<T: Clone, const N: usize>(held: &[OnceCell<T>; N], taken: &[OnceCell<T>; N]) {
    for (held, taken) in held.iter().zip(taken) {
        if let Some(value) = taken.get() {
            let _ = held.set(value.clone());
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
    fn of(scene: &Scene, floor: f64) -> Self {
        Self {
            scope: scene.scope.clone(),
            room: scene.room.clone(),
            obstacles: scene.obstacles.clone(),
            sure: scene.sure.clone(),
            tolerance: scene.tolerance,
            bracketed: scene.bracketed.clone(),
            bounded: scene.bounded.clone(),
            floor,
            memo: Mutex::new(Memo {
                clear: scene.clear.clone(),
                inscribed: scene.inscribed.clone(),
                simple: scene.simple.clone(),
                coarse: scene.coarse.clone(),
                found: scene.found.clone(),
            }),
        }
    }

    fn memo(&self) -> std::sync::MutexGuard<'_, Memo> {
        self.memo.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps what a search of the scene took that this one lacks: its
    /// clear parts, their largest discs, the simplified and coarse
    /// regions and the regions it found.
    fn keep(&self, scene: &Scene) {
        let memo = self.memo();
        fill(&memo.clear, &scene.clear);
        fill(&memo.inscribed, &scene.inscribed);
        fill(&memo.simple, &scene.simple);
        fill(&memo.coarse, &scene.coarse);
        for (side, corners, steps, free) in scene.found.borrow().iter() {
            placement::keep_found(&memo.found, *side, corners, steps, free);
        }
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
        let points = built + clear + cell_points(&memo.simple) + cell_points(&memo.coarse) + found;
        points * CORNER_BYTES
            + std::mem::size_of::<Self>()
            + self.bracketed.len() * std::mem::size_of::<(ObjectId, f64)>()
            + self.bounded.len() * std::mem::size_of::<ObjectId>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::free_space::AxiolidFreeSpaceService;
    use crate::geometry::AxiolidGeometry;
    use axioval_engine::{MetricDirection, SwingSector};
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

    fn service() -> AxiolidFreeSpaceService {
        AxiolidFreeSpaceService::new(AxiolidGeometry::new(), SourceId::new("cad", "m").unwrap())
    }

    /// Every input a scene is built from is in its key (#439): changing any
    /// one of them changes the key, and the same inputs give the same key.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn changing_any_input_of_a_scene_changes_its_key() {
        let (one, other) = (service(), service());
        let tolerance = axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let band = ElevationBand::try_new(0.0, 2.0).unwrap();
        let obstacles = [id("table"), id("chair")];
        let swept = [door("door", [0.0, 0.0, 0.0], 0.9, false)];
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
        let base = key(
            &one,
            (&id("room"), &[]),
            (&obstacles, &swept),
            band,
            tolerance,
        );
        assert!(
            base == key(
                &one,
                (&id("room"), &[]),
                (&obstacles, &swept),
                band,
                tolerance
            )
        );

        let changed = [
            (
                "service",
                key(
                    &other,
                    (&id("room"), &[]),
                    (&obstacles, &swept),
                    band,
                    tolerance,
                ),
            ),
            (
                "scope",
                key(
                    &one,
                    (&id("hall"), &[]),
                    (&obstacles, &swept),
                    band,
                    tolerance,
                ),
            ),
            (
                "merged scopes",
                key(
                    &one,
                    (&id("room"), &[id("hall")]),
                    (&obstacles, &swept),
                    band,
                    tolerance,
                ),
            ),
            (
                "obstacles",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles[..1], &swept),
                    band,
                    tolerance,
                ),
            ),
            (
                "obstacle order",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&[id("chair"), id("table")], &swept),
                    band,
                    tolerance,
                ),
            ),
            (
                "no swept door",
                key(&one, (&id("room"), &[]), (&obstacles, &[]), band, tolerance),
            ),
            (
                "the swept door",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &[door("other", [0.0, 0.0, 0.0], 0.9, false)]),
                    band,
                    tolerance,
                ),
            ),
            (
                "the hinge",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &[door("door", [0.0, 1.0e-12, 0.0], 0.9, false)]),
                    band,
                    tolerance,
                ),
            ),
            (
                "the radius",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &[door("door", [0.0, 0.0, 0.0], 0.91, false)]),
                    band,
                    tolerance,
                ),
            ),
            (
                "double-acting",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &[door("door", [0.0, 0.0, 0.0], 0.9, true)]),
                    band,
                    tolerance,
                ),
            ),
            (
                "the band's bottom",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &swept),
                    ElevationBand::try_new(0.1, 2.0).unwrap(),
                    tolerance,
                ),
            ),
            (
                "the band's top",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &swept),
                    ElevationBand::try_new(0.0, 2.1).unwrap(),
                    tolerance,
                ),
            ),
            (
                "the linear tolerance",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &swept),
                    band,
                    axiolid_core::Tolerance::new(1.0e-6, 1.0e-9).unwrap(),
                ),
            ),
            (
                "the angular tolerance",
                key(
                    &one,
                    (&id("room"), &[]),
                    (&obstacles, &swept),
                    band,
                    axiolid_core::Tolerance::new(1.0e-9, 1.0e-6).unwrap(),
                ),
            ),
            (
                "the version",
                SceneKey {
                    version: SCENE_VERSION + 1,
                    ..base.clone()
                },
            ),
        ];
        for (input, key) in changed {
            assert!(key != base, "changing {input} leaves the scene's key alike");
        }
        // The precision has one class today; a Fast one would be a variant.
        assert_eq!(base.precision, Precision::Exact);
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
