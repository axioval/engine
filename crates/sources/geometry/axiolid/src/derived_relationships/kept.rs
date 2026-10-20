//! What the relationships derived from geometry derive, as derivations of
//! the check's store (ADR 0006, #443): the declared spaces' bodies
//! ([`SpaceBodies`]), each subject's edges under a derivation
//! ([`DerivedEdges`]) and each pair `intersects` decides
//! ([`IntersectingPairs`]).
//!
//! The service once kept the edges and pairs in maps of its own for its
//! whole life, and read every declared space's body again for each request
//! that derived a subject's edges. Kept in the store, each is built once at
//! a time for every request, rule and thread, counted against the check's
//! memory budget and released after the last rule that may derive a
//! relationship.

use std::any::TypeId;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axioval_engine::{
    Derivation, DerivationError, DerivationKind, DerivationVersion,
    DerivedRelationshipServiceHandle, Inputs, Precision,
};
use axioval_ir::ObjectId;

use super::{Cached, Declared, Derived, KeptSpace, Pair};
use crate::derivations::{
    self, AXIOLID_CORE, AXIOLID_INSPECT, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_OVERLAY,
    AXIOLID_RAY_MESH, AXIOLID_SPATIAL, Over,
};
use crate::geometry::Triangle;

/// Every kind here is read through the derived-relationship service alone.
fn readers() -> Vec<TypeId> {
    vec![TypeId::of::<DerivedRelationshipServiceHandle>()]
}

/// A derivation as a key: its variant and its tolerances by their bits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asked(pub(crate) Derivation);

impl Asked {
    fn bits(&self) -> (u8, [u64; 2]) {
        let bits = |a: f64, b: f64| [a.to_bits(), b.to_bits()];
        match self.0 {
            Derivation::Intersects => (0, [0, 0]),
            Derivation::AdjacentAcross {
                tolerance_metres,
                overlap_metres,
            } => (1, bits(tolerance_metres, overlap_metres)),
            Derivation::ContainedInSpace {
                horizontal_metres,
                vertical_metres,
            } => (2, bits(horizontal_metres, vertical_metres)),
            Derivation::AdjacentSpace { reach_metres } => (3, bits(reach_metres, 0.0)),
            Derivation::OverlappingGroupSpace {
                minimum_ratio,
                vertical_metres,
            } => (4, bits(minimum_ratio, vertical_metres)),
            Derivation::SpansLevel { overlap_metres } => (5, bits(overlap_metres, 0.0)),
        }
    }
}

impl PartialEq for Asked {
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}

impl Eq for Asked {}

impl Hash for Asked {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits().hash(state);
    }
}

/// Everything the declared spaces' bodies are read from (#439): the
/// geometry and declarations (by identity) and the precision class (#438);
/// exact, as every body here.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SpacesKey {
    pub(crate) declared: Over<Declared>,
    pub(crate) precision: Precision,
}

/// The derivation kind of the declared spaces' bodies: each closed space's
/// triangles, extent and exactness, or why one cannot be probed.
pub(crate) struct SpaceBodies;

impl DerivationKind for SpaceBodies {
    const NAME: &'static str = "space bodies";
    /// Bumped whenever the same declarations could read other bodies: the
    /// audit changing what it accepts.
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_MESH]);
    type Key = SpacesKey;
    type Value = Result<Vec<KeptSpace>, String>;

    fn build(key: &SpacesKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.declared.0.space_bodies())
    }

    fn precision(key: &SpacesKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, |spaces| {
            spaces
                .iter()
                .map(|space| {
                    space.triangles.len() * std::mem::size_of::<Triangle>()
                        + std::mem::size_of::<KeptSpace>()
                })
                .sum()
        }))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    fn read_through() -> Option<Vec<TypeId>> {
        Some(readers())
    }
}

/// Everything one subject's edges under a derivation are derived from
/// (#439): the geometry and declarations (by identity), the derivation
/// with its tolerances by their bits, the subject and the precision class
/// (#438); exact, as every probe here.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct EdgesKey {
    pub(crate) declared: Over<Declared>,
    pub(crate) derivation: Asked,
    pub(crate) subject: ObjectId,
    pub(crate) precision: Precision,
}

impl EdgesKey {
    pub(crate) fn new(
        declared: &Arc<Declared>,
        derivation: &Derivation,
        subject: &ObjectId,
    ) -> Self {
        Self {
            declared: Over(Arc::clone(declared)),
            derivation: Asked(*derivation),
            subject: subject.clone(),
            precision: Precision::Exact,
        }
    }
}

/// The derivation kind of one subject's edges under a derivation.
pub(crate) struct DerivedEdges;

impl DerivationKind for DerivedEdges {
    const NAME: &'static str = "derived edges";
    /// Bumped whenever the same key could derive other edges: the probes,
    /// the winding numbers, the ray casts or the plan overlaps changing
    /// what they return.
    const VERSION: DerivationVersion = DerivationVersion::new(
        1,
        &[
            AXIOLID_CORE,
            AXIOLID_MEASURE,
            AXIOLID_MESH,
            AXIOLID_OVERLAY,
            AXIOLID_RAY_MESH,
        ],
    );
    type Key = EdgesKey;
    /// A subject whose edges are undecided is refused alike for every
    /// request.
    type Value = Cached;

    fn build(key: &EdgesKey, inputs: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        let spaces = || {
            inputs
                .derive::<SpaceBodies>(&SpacesKey {
                    declared: key.declared.clone(),
                    precision: key.precision,
                })
                .map_err(|error| error.to_string())
        };
        Ok(key
            .declared
            .0
            .derive_edges(&key.derivation.0, &key.subject, spaces))
    }

    fn precision(key: &EdgesKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, |derived| {
            derived
                .edges
                .iter()
                .map(|edge| edge.note.len() + std::mem::size_of::<super::Edge>())
                .sum::<usize>()
                + derived.notes.iter().map(String::len).sum::<usize>()
                + std::mem::size_of::<Derived>()
                + std::mem::size_of::<EdgesKey>()
        }))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    fn read_through() -> Option<Vec<TypeId>> {
        Some(readers())
    }
}

/// Everything whether two bodies share volume is decided from (#439): the
/// geometry and declarations (by identity), the pair with the lesser
/// identity first (the answer is symmetric) and the precision class.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct PairKey {
    pub(crate) declared: Over<Declared>,
    pub(crate) pair: (ObjectId, ObjectId),
    pub(crate) precision: Precision,
}

impl PairKey {
    pub(crate) fn new(declared: &Arc<Declared>, a: &ObjectId, b: &ObjectId) -> Self {
        let pair = if a < b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        Self {
            declared: Over(Arc::clone(declared)),
            pair,
            precision: Precision::Exact,
        }
    }
}

/// The derivation kind of the pairs `intersects` decides.
pub(crate) struct IntersectingPairs;

impl DerivationKind for IntersectingPairs {
    const NAME: &'static str = "intersecting pair";
    /// Bumped whenever the same pair could be decided otherwise: the
    /// proximity measurement or its thresholds changing what they return.
    const VERSION: DerivationVersion = DerivationVersion::new(
        1,
        &[
            AXIOLID_CORE,
            AXIOLID_INSPECT,
            AXIOLID_MEASURE,
            AXIOLID_MESH,
            AXIOLID_SPATIAL,
        ],
    );
    type Key = PairKey;
    type Value = Pair;

    fn build(key: &PairKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key
            .declared
            .0
            .measure_intersection(&key.pair.0, &key.pair.1))
    }

    fn precision(key: &PairKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        let note = match value {
            Ok(note) => note.as_ref().map_or(0, String::len),
            Err(reason) => reason.len(),
        };
        Some(note + std::mem::size_of::<Pair>() + std::mem::size_of::<PairKey>())
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    fn read_through() -> Option<Vec<TypeId>> {
        Some(readers())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derived_relationships::AxiolidDerivedRelationshipService;
    use crate::geometry::AxiolidGeometry;
    use axiolid_core::Point3;
    use axiolid_mesh::TriMesh;
    use axioval_engine::KeyCompleteness;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
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

    /// Two rooms side by side, a hall over both, a table in the first and
    /// a duct through the table; `room` sizes the first room.
    fn declared(room: [f64; 3]) -> Arc<Declared> {
        let geometry = AxiolidGeometry::new()
            .with_mesh(id("room"), cuboid([0.0; 3], room))
            .with_mesh(id("side"), cuboid([5.0, 0.0, 0.0], [8.0, 4.0, 3.0]))
            .with_mesh(id("hall"), cuboid([0.0, 0.0, 0.0], [8.0, 4.0, 3.5]))
            .with_mesh(id("table"), cuboid([1.0, 1.0, 0.0], [2.0, 2.0, 0.8]))
            .with_mesh(id("duct"), cuboid([1.5, 0.0, 0.5], [1.7, 4.0, 0.7]))
            .with_mesh(id("lamp"), cuboid([6.0, 1.0, 2.0], [6.5, 1.5, 2.5]));
        AxiolidDerivedRelationshipService::new(geometry)
            .with_space(id("room"))
            .with_space(id("side"))
            .with_space(id("hall"))
            .declared
    }

    fn edges(value: &Cached) -> String {
        match value {
            Ok(derived) => format!(
                "{:?} {:?}",
                derived
                    .edges
                    .iter()
                    .map(|edge| (&edge.target.local_id, &edge.note))
                    .collect::<Vec<_>>(),
                derived.notes
            ),
            Err(reason) => format!("refused: {reason}"),
        }
    }

    fn spaces(value: &Result<Vec<KeptSpace>, String>) -> String {
        match value {
            Ok(spaces) => format!(
                "{:?}",
                spaces
                    .iter()
                    .map(|space| (&space.id.local_id, space.extent))
                    .collect::<Vec<_>>()
            ),
            Err(reason) => format!("refused: {reason}"),
        }
    }

    fn pair(value: &Pair) -> String {
        format!("{value:?}")
    }

    /// Every input the declared spaces' bodies are read from is in their
    /// key (#439).
    #[test]
    fn changing_any_input_of_the_space_bodies_changes_their_key() {
        let one = declared([5.0, 4.0, 3.0]);
        let base = SpacesKey {
            declared: Over(Arc::clone(&one)),
            precision: Precision::Exact,
        };
        KeyCompleteness::<SpaceBodies, _>::new(base.clone(), spaces)
            .same(
                "the same inputs",
                SpacesKey {
                    declared: Over(Arc::clone(&one)),
                    precision: Precision::Exact,
                },
            )
            .changes_value(
                "the declarations",
                SpacesKey {
                    declared: Over(declared([4.0, 4.0, 3.0])),
                    precision: Precision::Exact,
                },
            )
            .precision(
                "the precision class",
                SpacesKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }

    /// Every input a subject's edges are derived from is in their key
    /// (#439), each tolerance of the derivation included.
    #[test]
    fn changing_any_input_of_derived_edges_changes_their_key() {
        let one = declared([5.0, 4.0, 3.0]);
        let contained = |horizontal_metres, vertical_metres| Derivation::ContainedInSpace {
            horizontal_metres,
            vertical_metres,
        };
        let key = |declared: &Arc<Declared>, derivation: Derivation, subject: &str| {
            EdgesKey::new(declared, &derivation, &id(subject))
        };
        let base = key(&one, contained(0.0, 0.0), "table");
        KeyCompleteness::<DerivedEdges, _>::new(base.clone(), edges)
            .same("the same inputs", key(&one, contained(0.0, 0.0), "table"))
            // The first room no longer holds the table.
            .changes_value(
                "the declarations",
                key(&declared([0.5, 4.0, 3.0]), contained(0.0, 0.0), "table"),
            )
            .changes_value("the subject", key(&one, contained(0.0, 0.0), "lamp"))
            .changes_value(
                "the derivation",
                key(
                    &one,
                    Derivation::OverlappingGroupSpace {
                        minimum_ratio: 0.5,
                        vertical_metres: 0.0,
                    },
                    "table",
                ),
            )
            // Inside two spaces either way: the same edges, another key.
            .input(
                "the horizontal reach",
                key(&one, contained(0.5, 0.0), "table"),
            )
            .input(
                "the vertical reach",
                key(&one, contained(0.0, 0.5), "table"),
            )
            .precision(
                "the precision class",
                EdgesKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }

    /// Every input a pair is decided from is in its key (#439), and the
    /// pair is one key in either order.
    #[test]
    fn changing_any_input_of_an_intersecting_pair_changes_its_key() {
        let one = declared([5.0, 4.0, 3.0]);
        let base = PairKey::new(&one, &id("duct"), &id("table"));
        KeyCompleteness::<IntersectingPairs, _>::new(base.clone(), pair)
            .same(
                "the same inputs",
                PairKey::new(&one, &id("duct"), &id("table")),
            )
            .same(
                "the pair reversed",
                PairKey::new(&one, &id("table"), &id("duct")),
            )
            // The same bodies in other declarations.
            .input(
                "the declarations",
                PairKey::new(&declared([4.0, 4.0, 3.0]), &id("duct"), &id("table")),
            )
            .changes_value(
                "the first body",
                PairKey::new(&one, &id("lamp"), &id("table")),
            )
            .changes_value(
                "the second body",
                PairKey::new(&one, &id("duct"), &id("lamp")),
            )
            .precision(
                "the precision class",
                PairKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
        assert_eq!(Asked(Derivation::Intersects), Asked(Derivation::Intersects));
    }
}
