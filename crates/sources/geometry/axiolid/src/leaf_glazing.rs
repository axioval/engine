//! Leaf glazing: the glazed area of each leaf of a door or window, and the
//! leaf's area, measured from the representation items of its body.
//!
//! ADR 0004: this module measures areas; whether a leaf is glazed enough is
//! a rule's decision.
//!
//! The host supplies each door's body item by item ([`GlazingItems`]): the
//! item's mesh in world metres, its chord deviation (zero for an exact
//! mesh) and whether its material is glazing ([`ItemMaterial`]), which is
//! a fact of the source (a transparent surface style, a glass material),
//! never guessed here from the shape. The leaves are the ones the request
//! carries, as the source states them.
//!
//! Each item is taken into the leaf's frame: `s` along the leaf from its
//! origin, `z` along the door's up axis. An item belongs to the leaf whose
//! extent `0 ≤ s ≤ width` holds its whole extent along `s`, widened by its
//! deviation and the rounding of the projection; one that reaches into the
//! leaf but also beyond its edge cannot be assigned to it. Only items that
//! could be glazing are assigned: a glazed item, and one whose material is
//! undecided, which cannot be told apart from the frame. Either kind that
//! cannot be assigned, or an undecided one inside the leaf, leaves the leaf
//! undecided. Opaque items count only towards the body's height.
//!
//! The glazed area is the union of the leaf's glazed items projected onto
//! the leaf's plane `(s, z)`, overlaps counted once, widened by the band
//! `2·P·e + π·e²` of its outline (`plan_area::band`) for the largest
//! deviation `e` among them. The leaf's area is its stated width times the
//! body's height along `z` (all items, widened by the largest deviation
//! among them). The rounding of a projection off the coordinate axes
//! (`ROUNDING` plus `ROUNDING_SCALE` of the largest coordinate) is added
//! to every deviation, so an exact body off the axes is not exact.

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_engine::{
    DoorGlazing, DoorLeaf, DoorLeaves, LeafGlazing, LeafGlazingError, LeafGlazingService,
    SourceSnapshot,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{Triangle, triangles};
use crate::plan_area::{band, tolerance};
use crate::planar::footprint_measure;

/// How far a projected coordinate may round when the leaf's axes are off
/// the coordinate axes, in metres, plus `ROUNDING_SCALE` of the largest
/// coordinate: far above the rounding of a three-term dot product.
const ROUNDING: f64 = 1e-9;
const ROUNDING_SCALE: f64 = 1e-14;

/// How far an item may reach past a leaf's edge, in metres, and still lie
/// within it: the distance at which two points are read as one.
const EDGE: f64 = 1e-9;

/// Whether a representation item is glazing, as its source states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemMaterial {
    /// Styled as, or associated with, a transparent or glass material.
    Glazed,
    /// Stated opaque.
    Opaque,
    /// Its material cannot be decided: why, in plain words.
    Undecided(String),
}

/// One representation item of a door's body.
#[derive(Clone, Debug)]
pub struct GlazingItem {
    /// How a message names the item (`#42`).
    pub label: String,
    /// The item's mesh in world metres.
    pub mesh: TriMesh,
    /// How far the true surface may lie from the mesh, in metres; zero for
    /// an exact mesh.
    pub deviation: f64,
    /// Whether the item is glazing.
    pub material: ItemMaterial,
}

/// The items of doors' bodies, as the host reads them.
pub trait GlazingItems: Send + Sync + 'static {
    /// The items of `door`'s body; `Ok(None)` for a door the host knows has
    /// no body, an error for one whose items cannot be read or meshed.
    ///
    /// # Errors
    ///
    /// Why the items cannot be supplied.
    fn items(&self, door: &ObjectId) -> Result<Option<Arc<Vec<GlazingItem>>>, String>;
}

/// Items registered up front, door by door; a door not registered is
/// unknown.
#[derive(Clone, Debug, Default)]
pub struct RegisteredGlazingItems(BTreeMap<ObjectId, DoorItems>);

/// A door's items, `None` for no body, or why they cannot be read.
type DoorItems = Result<Option<Arc<Vec<GlazingItem>>>, String>;

impl RegisteredGlazingItems {
    /// No door registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `door`'s body is `items`.
    #[must_use]
    pub fn with_items(mut self, door: ObjectId, items: Vec<GlazingItem>) -> Self {
        self.0.insert(door, Ok(Some(Arc::new(items))));
        self
    }

    /// `door` has no body.
    #[must_use]
    pub fn with_no_body(mut self, door: ObjectId) -> Self {
        self.0.insert(door, Ok(None));
        self
    }

    /// `door` has a body whose items cannot be read, for `reason`.
    #[must_use]
    pub fn with_unmeasured(mut self, door: ObjectId, reason: impl Into<String>) -> Self {
        self.0.insert(door, Err(reason.into()));
        self
    }
}

impl GlazingItems for RegisteredGlazingItems {
    fn items(&self, door: &ObjectId) -> Result<Option<Arc<Vec<GlazingItem>>>, String> {
        self.0
            .get(door)
            .cloned()
            .unwrap_or_else(|| Err(format!("no geometry registered for {door}")))
    }
}

/// Measures leaf glazing from the items a host supplies.
pub struct AxiolidLeafGlazingService {
    snapshots: Vec<SourceSnapshot>,
    items: Arc<dyn GlazingItems>,
}

impl std::fmt::Debug for AxiolidLeafGlazingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AxiolidLeafGlazingService")
            .field("snapshots", &self.snapshots)
            .finish_non_exhaustive()
    }
}

impl AxiolidLeafGlazingService {
    /// A service answering for `snapshots` from the items `items` supplies.
    #[must_use]
    pub fn new(snapshots: Vec<SourceSnapshot>, items: Arc<dyn GlazingItems>) -> Self {
        Self { snapshots, items }
    }
}

/// One item taken into a leaf's frame.
struct Placed<'a> {
    item: &'a GlazingItem,
    /// The item's extent along the leaf, from its origin.
    along: (f64, f64),
    /// The item's extent along up, from the leaf's origin.
    up: (f64, f64),
    /// The deviation and rounding together.
    slack: f64,
    triangles: Vec<Triangle>,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Whether every component of `axis` is exactly `0` or `±1`: only then
/// does a projection onto it round nothing.
#[allow(clippy::float_cmp)]
fn on_coordinate_axis(axis: [f64; 3]) -> bool {
    axis.iter().all(|c| *c == 0.0 || c.abs() == 1.0)
}

/// `item` in `leaf`'s frame.
fn place<'a>(item: &'a GlazingItem, leaf: &DoorLeaf) -> Result<Placed<'a>, LeafGlazingError> {
    let origin = leaf.origin();
    let (along, up) = (leaf.along().components(), leaf.up().components());
    let soup = triangles(&item.mesh);
    if soup.is_empty() {
        return Err(LeafGlazingError::Unavailable(format!(
            "item {} has no triangles",
            item.label
        )));
    }
    let mut largest = origin.iter().fold(0.0_f64, |m, c| m.max(c.abs()));
    let (mut s, mut z) = (
        (f64::INFINITY, f64::NEG_INFINITY),
        (f64::INFINITY, f64::NEG_INFINITY),
    );
    let projected: Vec<Triangle> = soup
        .iter()
        .map(|triangle| {
            triangle.map(|point| {
                let p = [point.x, point.y, point.z];
                largest = p.iter().fold(largest, |m, c| m.max(c.abs()));
                let offset = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
                let (ps, pz) = (dot(offset, along), dot(offset, up));
                s = (s.0.min(ps), s.1.max(ps));
                z = (z.0.min(pz), z.1.max(pz));
                Point3::new(ps, pz, 0.0)
            })
        })
        .collect();
    if ![s.0, s.1, z.0, z.1].iter().all(|v| v.is_finite()) {
        return Err(LeafGlazingError::Unavailable(format!(
            "item {} has a position that is not finite",
            item.label
        )));
    }
    let rounding = if on_coordinate_axis(along) && on_coordinate_axis(up) {
        0.0
    } else {
        ROUNDING + ROUNDING_SCALE * largest
    };
    if !item.deviation.is_finite() || item.deviation < 0.0 {
        return Err(LeafGlazingError::Unavailable(format!(
            "item {} declares no valid deviation",
            item.label
        )));
    }
    Ok(Placed {
        item,
        along: s,
        up: z,
        slack: item.deviation + rounding,
        triangles: projected,
    })
}

/// Where an item lies against a leaf `width` long.
enum Fit {
    Inside,
    Outside,
    Straddles,
}

fn fit(placed: &Placed<'_>, width: f64) -> Fit {
    let edge = EDGE + ROUNDING_SCALE * width;
    let (lower, upper) = (placed.along.0 - placed.slack, placed.along.1 + placed.slack);
    if lower >= -edge && upper <= width + edge {
        Fit::Inside
    } else if upper <= edge || lower >= width - edge {
        Fit::Outside
    } else {
        Fit::Straddles
    }
}

/// The glazing of one leaf, and whether it was measured exactly.
fn leaf_glazing(
    items: &[GlazingItem],
    leaf: &DoorLeaf,
    index: usize,
) -> Result<(LeafGlazing, bool), LeafGlazingError> {
    let placed = items
        .iter()
        .map(|item| place(item, leaf))
        .collect::<Result<Vec<_>, _>>()?;
    let width = leaf.width_metres();
    let mut undecided = Vec::new();
    let mut glazed: Vec<&Placed<'_>> = Vec::new();
    for item in &placed {
        let fit = fit(item, width);
        match (&item.item.material, fit) {
            (ItemMaterial::Opaque, _) | (_, Fit::Outside) => {}
            (ItemMaterial::Glazed, Fit::Inside) => glazed.push(item),
            (ItemMaterial::Glazed, Fit::Straddles) => undecided.push(format!(
                "glazed item {} reaches past the edge of leaf {}, so it cannot be assigned to \
                 one leaf",
                item.item.label,
                index + 1
            )),
            (ItemMaterial::Undecided(why), _) => undecided.push(format!(
                "item {} reaches into leaf {} but cannot be told apart from the frame: {why}",
                item.item.label,
                index + 1
            )),
        }
    }
    let exact = placed.iter().all(|item| item.slack == 0.0);
    if !undecided.is_empty() {
        return Ok((LeafGlazing::Undecided(undecided.join("; ")), exact));
    }
    let slack = placed.iter().map(|item| item.slack).fold(0.0, f64::max);
    let (bottom, top) = placed
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(bottom, top), item| {
            (bottom.min(item.up.0), top.max(item.up.1))
        });
    let height = (
        (top - bottom - 2.0 * slack).max(0.0),
        top - bottom + 2.0 * slack,
    );
    if height.0 <= 0.0 {
        return Err(LeafGlazingError::Unavailable(
            "the body has no height along the door's up axis".into(),
        ));
    }
    let leaf_area = (width * height.0, width * height.1);
    let area = if glazed.is_empty() {
        (0.0, 0.0)
    } else {
        let soup: Vec<Triangle> = glazed
            .iter()
            .flat_map(|item| item.triangles.iter().copied())
            .collect();
        let tolerance = tolerance().map_err(|_| {
            LeafGlazingError::Unavailable("the projection overlay has no tolerance".into())
        })?;
        let (area, perimeter) = footprint_measure(&soup, tolerance).ok_or_else(|| {
            LeafGlazingError::Unavailable(format!(
                "the glazed items of leaf {} cannot be projected onto its plane",
                index + 1
            ))
        })?;
        let deviation = glazed.iter().map(|item| item.slack).fold(0.0, f64::max);
        let widen = band(perimeter, deviation);
        ((area - widen).max(0.0), area + widen)
    };
    Ok((
        LeafGlazing::Measured {
            glazed: area,
            leaf: leaf_area,
            items: glazed.len(),
        },
        exact,
    ))
}

impl LeafGlazingService for AxiolidLeafGlazingService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn measure_leaf_glazing(&self, leaves: &DoorLeaves) -> Result<DoorGlazing, LeafGlazingError> {
        let door = leaves.door();
        let items = self
            .items
            .items(door)
            .map_err(LeafGlazingError::Unavailable)?
            .ok_or_else(|| LeafGlazingError::NoBody(door.clone()))?;
        if items.is_empty() {
            return Err(LeafGlazingError::NoBody(door.clone()));
        }
        let mut exact = true;
        let mut measured = Vec::with_capacity(leaves.leaves().len());
        for (index, leaf) in leaves.leaves().iter().enumerate() {
            let (glazing, leaf_exact) = leaf_glazing(&items, leaf, index)?;
            exact &= leaf_exact;
            measured.push(glazing);
        }
        let evidence = Evidence {
            source: door.source.clone(),
            locator: format!("leaf-glazing:{door}"),
            exact,
            witness: None,
        };
        DoorGlazing::try_new(door.clone(), measured, evidence)
    }
}
