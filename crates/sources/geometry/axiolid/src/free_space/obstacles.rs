//! What a placement scene takes of each obstacle, as derivations of the
//! check's store (ADR 0006, #443): its band footprint and, for a
//! tessellated body, its sure column.
//!
//! Spaces on one floor share their band and many of their obstacles, and a
//! large obstacle's footprint can take the overlay seconds, so each is
//! built once per check and read by every scene keyed over the same
//! service state ([`ObstacleKey`]). Kept in the store, they count against
//! the check's memory budget, give way to the steps it admits and are
//! released after the last rule that may ask the free-space service, where
//! the service once kept them for its whole life (#440).

use std::any::TypeId;
use std::sync::Arc;

use axiolid_mesh::TriMesh;
use axioval_engine::{
    DerivationError, DerivationKind, DerivationVersion, FreeSpaceServiceHandle, Inputs, Precision,
};
use axioval_ir::ObjectId;

use super::Measuring;
use crate::derivations::{self, AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY, Over};
use crate::walkable::{Plan, band_footprint, solid_column};

/// Which mesh of an obstacle is measured: its own body, or the closed
/// solid the host encloses an unmeasured body in (#401).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Body {
    Mesh,
    Enclosure,
}

/// Everything an obstacle's footprint or column is built from (#439): the
/// service state (its geometry, by identity), the obstacle, which of its
/// meshes is measured, the two limits by their bits (a footprint's open
/// band, `low < z < high`; a column's centre and half height) and the
/// precision class (#438); the kind's version beside it. Both are exact:
/// the overlay snaps them alike for every caller, no fast mode runs, so
/// every reader asks Exact.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct ObstacleKey {
    pub(crate) over: Over<Measuring>,
    pub(crate) obstacle: ObjectId,
    pub(crate) body: Body,
    pub(crate) limits: [u64; 2],
    pub(crate) precision: Precision,
}

impl ObstacleKey {
    /// The key of `obstacle`'s `body` over `over` between `limits`.
    pub(crate) fn new(
        over: &Arc<Measuring>,
        obstacle: &ObjectId,
        body: Body,
        limits: [f64; 2],
    ) -> Self {
        Self {
            over: Over(Arc::clone(over)),
            obstacle: obstacle.clone(),
            body,
            limits: limits.map(f64::to_bits),
            precision: Precision::Exact,
        }
    }

    fn limits(&self) -> [f64; 2] {
        self.limits.map(f64::from_bits)
    }

    /// The mesh measured: the obstacle's own, or its enclosure's.
    fn mesh(&self) -> Result<&TriMesh, String> {
        let geometry = &self.over.0.geometry;
        match self.body {
            Body::Mesh => geometry.mesh(&self.obstacle),
            Body::Enclosure => geometry
                .enclosure(&self.obstacle)
                .map(|enclosure| &enclosure.mesh),
        }
        .ok_or_else(|| format!("{} has no mesh to measure", self.obstacle))
    }

    /// [`band_footprint`] of the key's mesh in its open band.
    pub(crate) fn footprint(&self) -> Result<Plan, String> {
        let [low, high] = self.limits();
        band_footprint(&self.obstacle, self.mesh()?, low, high)
    }

    /// [`solid_column`] of the key's mesh about its centre.
    pub(crate) fn column(&self) -> Column {
        let [level, half] = self.limits();
        solid_column(&self.obstacle, self.mesh()?, level, half)
    }
}

/// A [`solid_column`]'s section and boundary shadow, or why it failed.
pub(crate) type Column = Result<(Plan, Plan), String>;

/// The derivation kind of obstacles' band footprints.
pub(crate) struct BandFootprints;

impl DerivationKind for BandFootprints {
    const NAME: &'static str = "band footprint";
    /// Bumped whenever the same key could build another footprint: the
    /// clipping, the solid pieces, the snapping or the overlay changing
    /// what they return.
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY]);
    type Key = ObstacleKey;
    /// A footprint the overlay refuses is refused alike for every scene.
    type Value = Result<Plan, String>;

    fn build(key: &ObstacleKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.footprint())
    }

    fn precision(key: &ObstacleKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, Plan::bytes))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Footprints are read by the free-space service's scenes alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<FreeSpaceServiceHandle>()])
    }
}

/// The derivation kind of tessellated obstacles' sure columns.
pub(crate) struct SolidColumns;

impl DerivationKind for SolidColumns {
    const NAME: &'static str = "solid column";
    /// Bumped whenever the same key could build another column, as
    /// [`BandFootprints::VERSION`].
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY]);
    type Key = ObstacleKey;
    type Value = Column;

    fn build(key: &ObstacleKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.column())
    }

    fn precision(key: &ObstacleKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(
            value
                .as_ref()
                .map_or(0, |(section, shadow)| section.bytes() + shadow.bytes()),
        )
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Columns are read by the free-space service's scenes alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<FreeSpaceServiceHandle>()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::free_space::AxiolidFreeSpaceService;
    use crate::geometry::AxiolidGeometry;
    use axiolid_core::Point3;
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

    /// A table, a shelf hung a metre above the floor, and an unmeasured
    /// cabinet the host encloses in a box.
    fn service(table: [f64; 3]) -> AxiolidFreeSpaceService {
        let geometry = AxiolidGeometry::new()
            .with_mesh(id("table"), cuboid([0.0; 3], table))
            .with_mesh(id("shelf"), cuboid([2.0, 0.0, 1.0], [3.0, 0.5, 2.5]))
            .with_unmeasured(id("cabinet"), "test")
            .with_unmeasured_enclosure(
                id("cabinet"),
                cuboid([4.0, 0.0, 0.0], [5.0, 1.0, 1.0]),
                None,
                None,
            );
        AxiolidFreeSpaceService::new(geometry, SourceId::new("cad", "m").unwrap())
    }

    fn plan(value: &Result<Plan, String>) -> String {
        match value {
            Ok(plan) => format!("{:?}", plan.polygons()),
            Err(error) => format!("refused: {error}"),
        }
    }

    fn column(value: &Column) -> String {
        match value {
            Ok((section, shadow)) => {
                format!("{:?} less {:?}", section.polygons(), shadow.polygons())
            }
            Err(error) => format!("refused: {error}"),
        }
    }

    /// Every input a band footprint is built from is in its key (#439):
    /// changing any one changes the key, and the footprint where it
    /// measures another.
    #[test]
    fn changing_any_input_of_a_band_footprint_changes_its_key() {
        let (one, other) = (service([1.2, 0.8, 0.75]), service([1.0, 0.8, 0.75]));
        let key = |over: &AxiolidFreeSpaceService, object: &str, body, limits| {
            ObstacleKey::new(&over.over, &id(object), body, limits)
        };
        let base = key(&one, "table", Body::Mesh, [0.0, 2.0]);
        KeyCompleteness::<BandFootprints, _>::new(base.clone(), plan)
            .same(
                "the same inputs",
                key(&one, "table", Body::Mesh, [0.0, 2.0]),
            )
            .changes_value("the service", key(&other, "table", Body::Mesh, [0.0, 2.0]))
            .changes_value("the obstacle", key(&one, "shelf", Body::Mesh, [0.0, 2.0]))
            .changes_value(
                "the body measured",
                key(&one, "cabinet", Body::Enclosure, [0.0, 2.0]),
            )
            .changes_value(
                "the band's bottom",
                key(&one, "table", Body::Mesh, [0.8, 2.0]),
            )
            // Below the shelf, which the base band holds.
            .changes_value("the band's top", key(&one, "shelf", Body::Mesh, [0.0, 0.9]))
            // Within the table either way: the same footprint, another key.
            .input("the band's top", key(&one, "table", Body::Mesh, [0.0, 2.5]))
            .precision(
                "the precision class",
                ObstacleKey {
                    precision: Precision::fast(1.0e-3),
                    ..base.clone()
                },
            )
            .check();
        assert_eq!(base.precision, Precision::Exact);
    }

    /// Every input a solid column is built from is in its key (#439).
    #[test]
    fn changing_any_input_of_a_solid_column_changes_its_key() {
        let (one, other) = (service([1.2, 0.8, 0.75]), service([1.0, 0.8, 0.75]));
        let key = |over: &AxiolidFreeSpaceService, object: &str, body, limits| {
            ObstacleKey::new(&over.over, &id(object), body, limits)
        };
        let base = key(&one, "table", Body::Mesh, [0.3, 0.1]);
        KeyCompleteness::<SolidColumns, _>::new(base.clone(), column)
            .same(
                "the same inputs",
                key(&one, "table", Body::Mesh, [0.3, 0.1]),
            )
            .changes_value("the service", key(&other, "table", Body::Mesh, [0.3, 0.1]))
            // The shelf hangs above the column.
            .changes_value("the obstacle", key(&one, "shelf", Body::Mesh, [0.3, 0.1]))
            .changes_value(
                "the body measured",
                key(&one, "cabinet", Body::Enclosure, [0.3, 0.1]),
            )
            // Above the table.
            .changes_value("the centre", key(&one, "table", Body::Mesh, [0.9, 0.1]))
            // Reaching past the table's top, whose faces then shadow it all.
            .changes_value(
                "the half height",
                key(&one, "table", Body::Mesh, [0.3, 0.5]),
            )
            .precision(
                "the precision class",
                ObstacleKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }
}
