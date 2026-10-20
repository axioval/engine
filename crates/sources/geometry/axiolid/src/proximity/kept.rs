//! What the proximity service measures of one body for every pair it is
//! in, as derivations of the check's store (ADR 0006, #443): a body's
//! closed shells, where the volume kernel refuses it whole
//! ([`BodyShells`]), and a whole's volume through its parts
//! ([`UnionVolumes`]).
//!
//! The service once kept both in maps of its own for its whole life, one
//! map per view (material or voids). Kept in the store, they are built
//! once at a time for every rule and thread, counted against the check's
//! memory budget and released after the last rule that may measure a
//! pair.

use std::any::TypeId;
use std::sync::Arc;

use axioval_engine::{
    DerivationError, DerivationKind, DerivationVersion, DerivedRelationshipServiceHandle, Inputs,
    Precision, ProximityServiceHandle,
};
use axioval_ir::ObjectId;

use super::bodies::BodyKey;
use super::{AxiolidProximityService, Volume, shells};
use crate::derivations::{
    self, AXIOLID_CORE, AXIOLID_INSPECT, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_SPATIAL, Over,
};
use crate::geometry::AxiolidGeometry;

/// The services a body's measurements are read through: the proximity
/// service's pairs, and the relationships derived from geometry, which
/// measure through it.
fn readers() -> Vec<TypeId> {
    vec![
        TypeId::of::<ProximityServiceHandle>(),
        TypeId::of::<DerivedRelationshipServiceHandle>(),
    ]
}

/// The derivation kind of a body's closed shells, where it has several
/// the volume kernel certifies (`shells::shells`): `None` otherwise. Keyed
/// as the body's index ([`BodyKey`]), which names the mesh split.
pub(crate) struct BodyShells;

impl DerivationKind for BodyShells {
    const NAME: &'static str = "body shells";
    /// Bumped whenever the same key could split another set of shells:
    /// the grouping or the volume kernel changing what they return.
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_INSPECT, AXIOLID_MESH]);
    type Key = BodyKey;
    /// A body that cannot be read has no shells: the pair then measures
    /// it whole, as before.
    type Value = Option<Arc<Vec<shells::Shell>>>;

    fn build(key: &BodyKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key
            .mesh(&key.geometry.0)
            .ok()
            .and_then(shells::shells)
            .map(Arc::new))
    }

    fn precision(key: &BodyKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(
            value
                .as_ref()
                .map_or(0, |shells| shells.iter().map(shells::Shell::bytes).sum()),
        )
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    fn read_through() -> Option<Vec<TypeId>> {
        Some(readers())
    }
}

/// Everything a whole's volume through its parts is built from (#439):
/// the geometry (by identity, its compositions included), the whole,
/// whether the view measures voids in place of bodiless parts, and the
/// precision class (#438); the kind's version beside it. The volume is
/// certified, not computed in a fast mode, so every reader asks Exact.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VolumeKey {
    pub(crate) geometry: Over<AxiolidGeometry>,
    pub(crate) object: ObjectId,
    pub(crate) voids: bool,
    pub(crate) precision: Precision,
}

impl VolumeKey {
    pub(crate) fn new(geometry: &Arc<AxiolidGeometry>, object: &ObjectId, voids: bool) -> Self {
        Self {
            geometry: Over(Arc::clone(geometry)),
            object: object.clone(),
            voids,
            precision: Precision::Exact,
        }
    }

    /// The volume of the whole's pieces, measured by the view the key
    /// names.
    pub(crate) fn measure(&self) -> Volume {
        let view = AxiolidProximityService {
            geometry: Arc::clone(&self.geometry.0),
            voids: self.voids,
        };
        view.pieces_volume(&self.geometry.0.pieces(&self.object))
    }
}

/// The derivation kind of wholes' enclosed volumes through their parts
/// (`union_volume`): every pair the whole is in reads it, and it costs a
/// mesh boolean per two of its pieces.
pub(crate) struct UnionVolumes;

impl DerivationKind for UnionVolumes {
    const NAME: &'static str = "union volume";
    /// Bumped whenever the same key could build another volume: the
    /// pieces' indexes, the volume kernel or the bands changing what they
    /// return.
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
    type Key = VolumeKey;
    /// A volume the kernel refuses is refused alike for every pair.
    type Value = Volume;

    fn build(key: &VolumeKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.measure())
    }

    fn precision(key: &VolumeKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(_: &Self::Value) -> Option<usize> {
        Some(std::mem::size_of::<Volume>() + std::mem::size_of::<VolumeKey>())
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
    use axiolid_core::Point3;
    use axiolid_mesh::TriMesh;
    use axioval_engine::KeyCompleteness;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    /// The corners and triangles of a closed, outward-wound box, its
    /// corners numbered from `first`.
    fn box_parts(min: [f64; 3], max: [f64; 3], first: u32) -> (Vec<Point3>, Vec<u32>) {
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
        let indices = [
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ]
        .map(|index| index + first)
        .to_vec();
        (positions, indices)
    }

    fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
        let (positions, indices) = box_parts(min, max, 0);
        TriMesh::new(positions, indices)
    }

    /// Two boxes in one mesh, apart: two closed shells.
    fn two_boxes(second: [f64; 3]) -> TriMesh {
        let (mut positions, mut indices) = box_parts([0.0; 3], [1.0; 3], 0);
        let (more, more_indices) = box_parts([2.0, 0.0, 0.0], second, 8);
        positions.extend(more);
        indices.extend(more_indices);
        TriMesh::new(positions, indices)
    }

    // The fingerprint takes the kind's value as the test hands it.
    #[allow(clippy::ref_option)]
    fn shells_of(value: &Option<Arc<Vec<shells::Shell>>>) -> String {
        value.as_ref().map_or_else(
            || "whole".to_owned(),
            |shells| {
                format!(
                    "{:?}",
                    shells.iter().map(shells::Shell::volume).collect::<Vec<_>>()
                )
            },
        )
    }

    fn volume(value: &Volume) -> String {
        match value {
            Ok((lower, upper)) => format!("[{lower:.9}, {upper:.9}]"),
            Err(error) => format!("refused: {error:?}"),
        }
    }

    /// Every input a body's shells are split from is in its key (#439).
    #[test]
    fn changing_any_input_of_a_body_s_shells_changes_its_key() {
        let geometry = |second| {
            Arc::new(
                AxiolidGeometry::new()
                    .with_mesh(id("pair"), two_boxes(second))
                    .with_mesh(id("single"), cuboid([0.0; 3], [1.0; 3]))
                    .with_no_body(id("opening"))
                    .with_void(id("opening"), two_boxes([4.0, 1.0, 1.0])),
            )
        };
        let (one, other) = (geometry([3.0, 1.0, 1.0]), geometry([3.0, 1.0, 2.0]));
        let base = BodyKey::new(&one, &id("pair"), false);
        KeyCompleteness::<BodyShells, _>::new(base.clone(), shells_of)
            .same("the same inputs", BodyKey::new(&one, &id("pair"), false))
            .changes_value("the geometry", BodyKey::new(&other, &id("pair"), false))
            .changes_value("the object", BodyKey::new(&one, &id("single"), false))
            .changes_value("measuring a void", BodyKey::new(&one, &id("opening"), true))
            // The split reads no tolerance.
            .input(
                "the tolerance",
                BodyKey {
                    tolerance: [1.0_f64.to_bits(), base.tolerance[1]],
                    ..base.clone()
                },
            )
            .precision(
                "the precision class",
                BodyKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }

    /// Every input a whole's volume through its parts is built from is in
    /// its key (#439).
    #[test]
    fn changing_any_input_of_a_union_volume_changes_its_key() {
        let geometry = |top: f64| {
            let parts = AxiolidGeometry::new()
                .with_mesh(id("leg"), cuboid([0.0; 3], [1.0, 1.0, top]))
                .with_mesh(id("top"), cuboid([0.0, 0.0, 1.0], [2.0, 2.0, 1.5]))
                .with_mesh(id("rail"), cuboid([3.0; 3], [4.0; 3]));
            let table = parts.compose(&[id("leg"), id("top")]).unwrap();
            let frame = parts.compose(&[id("leg"), id("rail")]).unwrap();
            Arc::new(
                parts
                    .with_composed_body(id("table"), table)
                    .with_composed_body(id("frame"), frame),
            )
        };
        let (one, other) = (geometry(1.0), geometry(1.25));
        let base = VolumeKey::new(&one, &id("table"), false);
        KeyCompleteness::<UnionVolumes, _>::new(base.clone(), volume)
            .same("the same inputs", VolumeKey::new(&one, &id("table"), false))
            .changes_value("the geometry", VolumeKey::new(&other, &id("table"), false))
            .changes_value("the whole", VolumeKey::new(&one, &id("frame"), false))
            // No part here is bodiless: the same volume in either view.
            .input("measuring voids", VolumeKey::new(&one, &id("table"), true))
            .precision(
                "the precision class",
                VolumeKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }
}
