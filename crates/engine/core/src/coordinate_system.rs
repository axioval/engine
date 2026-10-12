//! Source coordinate systems: where a source's coordinates sit.
//!
//! Every object frame and every mesh is stated in its source's own
//! coordinates. Whether two sources, or two revisions of one source, share a
//! coordinate system is a fact about the source as a whole, not about any
//! object, so this seam answers per source.
//!
//! A source may state three things, each independently:
//!
//! - its **world frame**: the frame its model coordinates are stated in,
//!   with the origin in canonical metres;
//! - **true north**: the plan direction of geographic north in those
//!   coordinates;
//! - a **map conversion**: how its coordinates map onto a named map
//!   coordinate reference system (offset, rotation, scale);
//! - its **site placement**: the frame of the one site the source places
//!   its building on, in the world frame's coordinates ([`SitePlacement`]).
//!
//! What a source does not state is `None`, never a default: a missing map
//! conversion means the source is not georeferenced, not that it sits at the
//! map origin. A service that does not read sites reports the site placement
//! unknown, never absent.

use std::sync::Arc;

use axioval_ir::{Evidence, SourceId};
use thiserror::Error;

use crate::services::reviewable_exact_evidence;
use crate::{MetricDirection, SnapshotBoundService, SourceSnapshot};

/// Failure to supply a source's coordinate system.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CoordinateSystemError {
    /// The service does not cover the source.
    #[error("coordinate-system service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The source states its coordinate system more than once, for example
    /// two model contexts, and nothing says which one applies.
    #[error("coordinate system is stated ambiguously: {0}")]
    Ambiguous(String),
    /// The source states it by a construct the service cannot resolve
    /// exactly.
    #[error("coordinate system unsupported: {0}")]
    Unsupported(String),
    /// A statement, or the unit it is stated in, is malformed or cannot be
    /// read exactly.
    #[error("coordinate system cannot be read exactly: {0}")]
    Unreadable(String),
    /// A value is non-finite, an axis triple is not right-handed and
    /// orthonormal, or a scale is not positive.
    #[error("coordinate system is invalid")]
    InvalidMeasurement,
    /// The evidence is not exact, not reviewable, or from another source.
    #[error("coordinate-system evidence is not exact and reviewable")]
    InexactEvidence,
    /// The service answered for another source.
    #[error("coordinate-system service answered for another source")]
    ResponseRequestMismatch,
}

/// A right-handed orthonormal frame with its origin in canonical metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateFrame {
    origin_metres: [f64; 3],
    axes: [MetricDirection; 3],
}

impl CoordinateFrame {
    /// A frame; the origin must be finite and the axes a right-handed
    /// orthonormal triple.
    pub fn try_new(
        origin_metres: [f64; 3],
        x: MetricDirection,
        y: MetricDirection,
        z: MetricDirection,
    ) -> Result<Self, CoordinateSystemError> {
        if !origin_metres.iter().all(|value| value.is_finite())
            || !crate::free_space::right_handed_orthonormal(x, y, z)
        {
            return Err(CoordinateSystemError::InvalidMeasurement);
        }
        Ok(Self {
            origin_metres,
            axes: [x, y, z],
        })
    }

    /// The origin, in canonical metres.
    #[must_use]
    pub fn origin_metres(&self) -> [f64; 3] {
        self.origin_metres
    }

    /// The X, Y and Z axes.
    #[must_use]
    pub fn axes(&self) -> [MetricDirection; 3] {
        self.axes
    }
}

/// How a source's coordinates map onto a map coordinate reference system.
#[derive(Clone, Debug, PartialEq)]
pub struct MapConversion {
    target: Option<String>,
    offset: [f64; 3],
    x_axis: [f64; 2],
    scale: f64,
    metres_per_map_unit: Option<f64>,
    map_unit_by_default: bool,
}

impl MapConversion {
    /// A map conversion.
    ///
    /// `offset` is easting, northing and orthogonal height in the map's unit;
    /// `x_axis` is the direction of the source's X axis in the map's plan
    /// (abscissa, ordinate) and is normalized; `scale` is the factor from
    /// source to map lengths. `metres_per_map_unit` is `None` when the map's
    /// unit is not known exactly, never a guess. A unit the source does not
    /// state but its standard prescribes is marked with
    /// [`Self::with_map_unit_by_default`].
    pub fn try_new(
        target: Option<String>,
        offset: [f64; 3],
        x_axis: [f64; 2],
        scale: f64,
        metres_per_map_unit: Option<f64>,
    ) -> Result<Self, CoordinateSystemError> {
        let norm = x_axis[0].hypot(x_axis[1]);
        let valid = offset.iter().all(|value| value.is_finite())
            && norm.is_finite()
            && norm > f64::EPSILON
            && scale.is_finite()
            && scale > 0.0
            && metres_per_map_unit.is_none_or(|unit| unit.is_finite() && unit > 0.0)
            && target.as_deref().is_none_or(|name| !name.trim().is_empty());
        if !valid {
            return Err(CoordinateSystemError::InvalidMeasurement);
        }
        Ok(Self {
            target,
            offset,
            x_axis: [x_axis[0] / norm, x_axis[1] / norm],
            scale,
            metres_per_map_unit,
            map_unit_by_default: false,
        })
    }

    /// The same conversion, its map unit not stated by the source but the
    /// default its standard prescribes (for IFC, the project length unit).
    /// The unit is known, so offsets compare in metres; the mark keeps it
    /// from being presented as stated.
    #[must_use]
    pub fn with_map_unit_by_default(mut self) -> Self {
        self.map_unit_by_default = true;
        self
    }

    /// Whether the map unit is the standard's default rather than stated
    /// by the source ([`Self::with_map_unit_by_default`]).
    #[must_use]
    pub fn map_unit_by_default(&self) -> bool {
        self.map_unit_by_default
    }

    /// The name of the target map coordinate reference system, when stated.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Easting, northing and orthogonal height, in the map's unit.
    #[must_use]
    pub fn offset(&self) -> [f64; 3] {
        self.offset
    }

    /// Unit direction of the source's X axis in the map's plan.
    #[must_use]
    pub fn x_axis(&self) -> [f64; 2] {
        self.x_axis
    }

    /// Scale from source lengths to map lengths.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Metres per map unit, or `None` when the unit is not known exactly.
    #[must_use]
    pub fn metres_per_map_unit(&self) -> Option<f64> {
        self.metres_per_map_unit
    }

    /// The offset in metres, when the map's unit is known.
    #[must_use]
    pub fn offset_metres(&self) -> Option<[f64; 3]> {
        self.metres_per_map_unit
            .map(|unit| self.offset.map(|value| value * unit))
    }
}

/// Where a source places its site.
#[derive(Clone, Debug, PartialEq)]
pub enum SitePlacement {
    /// The source holds no site.
    Absent,
    /// The frame of the source's one site, origin in canonical metres.
    Stated(CoordinateFrame),
    /// The placement is not known: several sites, a placement that cannot be
    /// resolved exactly, or a service that does not read sites. Never read
    /// as absent or as any frame.
    Unknown(String),
}

/// One source's coordinate system, as far as the source states it.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceCoordinateSystem {
    source: SourceId,
    world: Option<CoordinateFrame>,
    true_north: Option<[f64; 2]>,
    map: Option<MapConversion>,
    site: SitePlacement,
    evidence: Evidence,
}

impl SourceCoordinateSystem {
    /// The coordinate system of `source`.
    ///
    /// `true_north` is a plan direction and is normalized. The evidence must
    /// be exact, reviewable and from `source`: a coordinate system is stated,
    /// never estimated.
    pub fn try_new(
        source: SourceId,
        world: Option<CoordinateFrame>,
        true_north: Option<[f64; 2]>,
        map: Option<MapConversion>,
        evidence: Evidence,
    ) -> Result<Self, CoordinateSystemError> {
        let true_north = match true_north {
            Some([x, y]) => {
                let norm = x.hypot(y);
                if !norm.is_finite() || norm <= f64::EPSILON {
                    return Err(CoordinateSystemError::InvalidMeasurement);
                }
                Some([x / norm, y / norm])
            }
            None => None,
        };
        if !reviewable_exact_evidence(&evidence) || evidence.source != source {
            return Err(CoordinateSystemError::InexactEvidence);
        }
        Ok(Self {
            source,
            world,
            true_north,
            map,
            site: SitePlacement::Unknown(
                "the coordinate-system service does not report site placements".into(),
            ),
            evidence,
        })
    }

    /// The same coordinate system with the source's site placement.
    #[must_use]
    pub fn with_site(mut self, site: SitePlacement) -> Self {
        self.site = site;
        self
    }

    /// The source this coordinate system belongs to.
    #[must_use]
    pub fn source(&self) -> &SourceId {
        &self.source
    }

    /// The frame the source's coordinates are stated in, when stated.
    #[must_use]
    pub fn world(&self) -> Option<&CoordinateFrame> {
        self.world.as_ref()
    }

    /// Unit plan direction of true north, when stated.
    #[must_use]
    pub fn true_north(&self) -> Option<[f64; 2]> {
        self.true_north
    }

    /// The map conversion, when the source is georeferenced.
    #[must_use]
    pub fn map(&self) -> Option<&MapConversion> {
        self.map.as_ref()
    }

    /// Where the source places its site; unknown unless the service stated
    /// it ([`Self::with_site`]).
    #[must_use]
    pub fn site(&self) -> &SitePlacement {
        &self.site
    }

    /// Reviewable provenance of the statements.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Trusted adapter seam supplying sources' coordinate systems.
pub trait CoordinateSystemService: Send + Sync + 'static {
    /// Exact source snapshots this service answers for.
    // gate: reads
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The coordinate system of `source`, or why it cannot be read.
    // gate: reads
    fn coordinate_system(
        &self,
        source: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError>;
}

/// Registry handle for a [`CoordinateSystemService`].
#[derive(Clone)]
pub struct CoordinateSystemServiceHandle(Arc<dyn CoordinateSystemService>);

impl CoordinateSystemServiceHandle {
    /// Wraps a trusted coordinate-system service.
    #[must_use]
    pub fn new(service: Arc<dyn CoordinateSystemService>) -> Self {
        Self(service)
    }

    /// The coordinate system of `source`. An uncovered source is refused,
    /// and so is an answer about another source.
    pub fn coordinate_system(
        &self,
        source: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| snapshot.source() == source)
        {
            return Err(CoordinateSystemError::UncoveredSource(source.clone()));
        }
        let answer = self.0.coordinate_system(source)?;
        if answer.source() != source {
            return Err(CoordinateSystemError::ResponseRequestMismatch);
        }
        Ok(answer)
    }
}

impl SnapshotBoundService for CoordinateSystemServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

/// How far a map conversion's scale may differ from one and still be read
/// as the map's own scale factor rather than a unit conversion.
pub const MAP_SCALE_TOLERANCE: f64 = 1e-3;

/// Why a source's map conversion cannot place map coordinates in its model
/// coordinates ([`MapPlacement::of`]).
#[derive(Clone, Debug, PartialEq)]
pub enum MapPlacementRefusal {
    /// The source states no map conversion.
    NoConversion,
    /// The conversion names no reference system, and one must match.
    NoTarget,
    /// The conversion maps onto another reference system than the one
    /// stated.
    OtherTarget { target: String },
    /// The conversion's map unit is not known exactly.
    UnitUnknown { target: Option<String> },
    /// The source's world frame is not stated as the identity, so how the
    /// conversion relates to its coordinates is not established.
    WorldNotIdentity,
    /// The conversion scales by more than [`MAP_SCALE_TOLERANCE`] from one,
    /// which may convert units.
    Scaled(f64),
}

impl std::fmt::Display for MapPlacementRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoConversion => f.write_str("the model states no map conversion"),
            Self::NoTarget => f.write_str("the model's map conversion names no reference system"),
            Self::OtherTarget { target } => {
                write!(f, "the model's map conversion is to `{target}`")
            }
            Self::UnitUnknown { target } => match target {
                Some(target) => write!(f, "the unit of `{target}` is not known exactly"),
                None => f.write_str("the unit of the model's map is not known exactly"),
            },
            Self::WorldNotIdentity => {
                f.write_str("the model's world frame is not stated as the identity")
            }
            Self::Scaled(scale) => write!(
                f,
                "the model's map conversion scales by {scale}, which may convert units"
            ),
        }
    }
}

/// A source's map conversion, established for placing map coordinates in
/// its model coordinates and back: `E = E0 + s (a x - b y)`,
/// `N = N0 + s (b x + a y)`, `H = z + H0`, every length in metres, with
/// `(a, b)` the map direction of the source's x axis and `s` its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPlacement {
    offset_metres: [f64; 3],
    x_axis: [f64; 2],
    scale: f64,
    metres_per_map_unit: f64,
}

impl MapPlacement {
    /// The placement through `system`'s map conversion, onto the reference
    /// system `stated` names when it names one.
    ///
    /// # Errors
    ///
    /// Without a map conversion; with `stated`, a conversion naming no
    /// reference system or another one; a map unit not known exactly; a
    /// world frame not stated as the identity; and a scale more than
    /// [`MAP_SCALE_TOLERANCE`] from one. Never placed by a guess.
    pub fn of(
        system: &SourceCoordinateSystem,
        stated: Option<&str>,
    ) -> Result<Self, MapPlacementRefusal> {
        let map = system.map().ok_or(MapPlacementRefusal::NoConversion)?;
        if let Some(stated) = stated {
            let target = map.target().ok_or(MapPlacementRefusal::NoTarget)?;
            if !same_reference_system(target, stated) {
                return Err(MapPlacementRefusal::OtherTarget {
                    target: target.to_owned(),
                });
            }
        }
        let (Some(metres_per_map_unit), Some(offset_metres)) =
            (map.metres_per_map_unit(), map.offset_metres())
        else {
            return Err(MapPlacementRefusal::UnitUnknown {
                target: map.target().map(str::to_owned),
            });
        };
        // The identity as stated, compared exactly.
        #[allow(clippy::float_cmp)]
        let identity = system.world().is_some_and(|world| {
            let axes = world.axes().map(|axis| axis.components());
            axes == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
                && world.origin_metres() == [0.0; 3]
        });
        if !identity {
            return Err(MapPlacementRefusal::WorldNotIdentity);
        }
        let scale = map.scale();
        if (scale - 1.0).abs() > MAP_SCALE_TOLERANCE {
            return Err(MapPlacementRefusal::Scaled(scale));
        }
        Ok(Self {
            offset_metres,
            x_axis: map.x_axis(),
            scale,
            metres_per_map_unit,
        })
    }

    /// The map offset (easting, northing, height), in metres.
    #[must_use]
    pub fn offset_metres(&self) -> [f64; 3] {
        self.offset_metres
    }

    /// The map's scale from model to map lengths.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Metres per map unit.
    #[must_use]
    pub fn metres_per_map_unit(&self) -> f64 {
        self.metres_per_map_unit
    }

    /// A model plan point, in metres, on the map, in metres.
    #[must_use]
    pub fn to_map(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [a, b] = self.x_axis;
        [
            self.scale
                .mul_add(a.mul_add(x, -(b * y)), self.offset_metres[0]),
            self.scale
                .mul_add(b.mul_add(x, a * y), self.offset_metres[1]),
        ]
    }

    /// A map plan point, in metres, in the model's coordinates, in metres.
    #[must_use]
    pub fn to_model(&self, [east, north]: [f64; 2]) -> [f64; 2] {
        let [cos, sin] = self.x_axis;
        let (dx, dy) = (east - self.offset_metres[0], north - self.offset_metres[1]);
        [
            dx.mul_add(cos, dy * sin) / self.scale,
            (-dx).mul_add(sin, dy * cos) / self.scale,
        ]
    }
}

/// Whether two names of coordinate reference systems name one: an EPSG
/// code by its number, whatever form names it (a bare number too),
/// anything else as written but for case and surrounding spaces.
#[must_use]
pub fn same_reference_system(first: &str, second: &str) -> bool {
    reference_key(first) == reference_key(second)
}

/// The name of a coordinate reference system as compared.
fn reference_key(name: &str) -> String {
    let upper: String = name
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if !upper.is_empty() && upper.chars().all(|c| c.is_ascii_digit()) {
        return format!("EPSG:{upper}");
    }
    if upper.contains("EPSG") {
        let digits: String = upper
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !digits.is_empty() {
            return format!("EPSG:{digits}");
        }
    }
    upper
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(name: &str) -> SourceId {
        SourceId::new("cad", name).unwrap()
    }

    fn direction(vector: [f64; 3]) -> MetricDirection {
        MetricDirection::try_new(vector).unwrap()
    }

    fn identity() -> [MetricDirection; 3] {
        [
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
        ]
    }

    #[test]
    fn a_frame_must_be_right_handed_and_finite() {
        let [x, y, z] = identity();
        assert!(CoordinateFrame::try_new([0.0; 3], x, y, z).is_ok());
        assert_eq!(
            CoordinateFrame::try_new([0.0; 3], direction([-1.0, 0.0, 0.0]), y, z),
            Err(CoordinateSystemError::InvalidMeasurement)
        );
        assert_eq!(
            CoordinateFrame::try_new([f64::NAN, 0.0, 0.0], x, y, z),
            Err(CoordinateSystemError::InvalidMeasurement)
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // Axis-aligned arithmetic on whole metres is exact.
    fn a_map_placement_is_established_only_from_what_the_source_states() {
        let [x, y, z] = identity();
        let world = CoordinateFrame::try_new([0.0; 3], x, y, z).unwrap();
        let system = |world: Option<CoordinateFrame>, map: Option<MapConversion>| {
            SourceCoordinateSystem::try_new(
                source("a"),
                world,
                None,
                map,
                Evidence::exact(source("a"), "crs"),
            )
            .unwrap()
        };
        let map = |scale: f64, unit: Option<f64>| {
            MapConversion::try_new(
                Some("EPSG:25832".into()),
                [500_000.0, 5_600_000.0, 50.0],
                [0.0, 1.0],
                scale,
                unit,
            )
            .unwrap()
        };
        let placed = MapPlacement::of(
            &system(Some(world), Some(map(1.0, Some(1.0)))),
            Some("25832"),
        )
        .unwrap();
        // The source's x axis points north: x 10 is 10 m north, and back.
        assert_eq!(placed.to_map([10.0, 0.0]), [500_000.0, 5_600_010.0]);
        assert_eq!(placed.to_map([0.0, 10.0]), [499_990.0, 5_600_000.0]);
        assert_eq!(placed.to_model([499_990.0, 5_600_000.0]), [0.0, 10.0]);
        for (stated, map, world, refusal) in [
            (None, None, Some(world), MapPlacementRefusal::NoConversion),
            (
                Some("EPSG:25833"),
                Some(map(1.0, Some(1.0))),
                Some(world),
                MapPlacementRefusal::OtherTarget {
                    target: "EPSG:25832".into(),
                },
            ),
            (
                None,
                Some(map(1.0, None)),
                Some(world),
                MapPlacementRefusal::UnitUnknown {
                    target: Some("EPSG:25832".into()),
                },
            ),
            (
                None,
                Some(map(1.0, Some(1.0))),
                None,
                MapPlacementRefusal::WorldNotIdentity,
            ),
            (
                None,
                Some(map(0.001, Some(1.0))),
                Some(world),
                MapPlacementRefusal::Scaled(0.001),
            ),
        ] {
            assert_eq!(MapPlacement::of(&system(world, map), stated), Err(refusal));
        }
        assert!(same_reference_system("EPSG:25832", "epsg: 25832"));
        assert!(same_reference_system(
            "25832",
            "urn:ogc:def:crs:EPSG::25832"
        ));
        assert!(!same_reference_system("EPSG:25832", "EPSG:25833"));
    }

    #[test]
    #[allow(clippy::float_cmp)] // Normalizing an axis-aligned vector is exact.
    fn a_map_conversion_needs_a_positive_scale_and_a_direction() {
        assert!(MapConversion::try_new(None, [1.0, 2.0, 3.0], [1.0, 0.0], 1.0, None).is_ok());
        for (axis, scale, unit) in [
            ([0.0, 0.0], 1.0, None),
            ([1.0, 0.0], 0.0, None),
            ([1.0, 0.0], 1.0, Some(-1.0)),
        ] {
            assert_eq!(
                MapConversion::try_new(None, [0.0; 3], axis, scale, unit),
                Err(CoordinateSystemError::InvalidMeasurement)
            );
        }
        let map =
            MapConversion::try_new(None, [1.0, 2.0, 3.0], [0.0, 2.0], 1.0, Some(0.001)).unwrap();
        assert_eq!(map.x_axis(), [0.0, 1.0]);
        assert_eq!(map.offset_metres(), Some([0.001, 0.002, 0.003]));
        assert!(!map.map_unit_by_default(), "a unit is stated unless marked");
        assert!(map.with_map_unit_by_default().map_unit_by_default());
    }

    #[test]
    fn evidence_must_be_exact_and_from_the_source() {
        let foreign = Evidence::exact(source("b"), "crs");
        assert_eq!(
            SourceCoordinateSystem::try_new(source("a"), None, None, None, foreign),
            Err(CoordinateSystemError::InexactEvidence)
        );
        let system = SourceCoordinateSystem::try_new(
            source("a"),
            None,
            Some([0.0, 3.0]),
            None,
            Evidence::exact(source("a"), "crs"),
        )
        .unwrap();
        assert_eq!(system.true_north(), Some([0.0, 1.0]));
        assert!(
            matches!(system.site(), SitePlacement::Unknown(_)),
            "a site the service did not state is unknown, never absent"
        );
        let [x, y, z] = identity();
        let frame = CoordinateFrame::try_new([1.0, 2.0, 0.0], x, y, z).unwrap();
        assert_eq!(
            system.with_site(SitePlacement::Stated(frame)).site(),
            &SitePlacement::Stated(frame)
        );
    }

    struct Fixed(Vec<SourceSnapshot>, SourceCoordinateSystem);
    impl CoordinateSystemService for Fixed {
        fn source_snapshots(&self) -> &[SourceSnapshot] {
            &self.0
        }
        fn coordinate_system(
            &self,
            _: &SourceId,
        ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
            Ok(self.1.clone())
        }
    }

    #[test]
    fn the_handle_binds_answers_to_the_request() {
        let snapshots = vec![
            SourceSnapshot::try_new(source("a"), "r", "sha256:a").unwrap(),
            SourceSnapshot::try_new(source("b"), "r", "sha256:b").unwrap(),
        ];
        let answer = SourceCoordinateSystem::try_new(
            source("a"),
            None,
            None,
            None,
            Evidence::exact(source("a"), "crs"),
        )
        .unwrap();
        let handle = CoordinateSystemServiceHandle::new(Arc::new(Fixed(snapshots, answer)));
        assert!(handle.coordinate_system(&source("a")).is_ok());
        assert_eq!(
            handle.coordinate_system(&source("b")),
            Err(CoordinateSystemError::ResponseRequestMismatch)
        );
        assert_eq!(
            handle.coordinate_system(&source("c")),
            Err(CoordinateSystemError::UncoveredSource(source("c")))
        );
    }
}
