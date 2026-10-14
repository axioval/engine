//! Context zones (#327): plan polygons and lines supplied beside the model,
//! such as parcels, zoning areas, building limits and building lines, that
//! measured values relate the model's objects to.
//!
//! A zone is a statement of whoever supplied it, never the model's: its
//! id, its kind (`parcel`, `zone`, `building-limit`, `building-line`, or
//! any other lowercase word a reader assigns), its plan polygons or line,
//! the accuracy within which its boundary is stated, and its provenance
//! (the file's SHA-256 and the feature's position). The host reads zones
//! from a file ([`ContextZones::read_geojson`]) and supplies them to a
//! plan ([`crate::ExecutionPlan::supply_context_zones`]); the runtime
//! installs them for every run as [`SuppliedZones`]. Readers of other
//! exchanges (a cadastre, a zoning plan) produce the same
//! [`ContextZones`], so every measurement reads zones one way.
//!
//! A zone states its coordinates in the model's own plan coordinates
//! ([`ZoneReference::Model`]) or in a named map coordinate reference system
//! ([`ZoneReference::Map`]). A map zone is placed into a source's model
//! coordinates only through the map conversion that source states for the
//! same reference system ([`ContextZone::place`]); anything that leaves the
//! transform uncertain (no conversion, another reference system, an unknown
//! map unit, a world frame other than the identity, a scale that may hold a
//! unit factor) refuses the placement, and the value reading the zone is
//! not evaluated. Nothing is ever defaulted: a zone without a stated
//! accuracy is refused when read.
//!
//! Besides `GeoJSON`, cadastral parcels and public traffic areas are read
//! from a GML subset ([`ContextZones::read_cadastre_gml`], #319), and the
//! regulating areas and lines of a zoning plan from an `XPlanGML` subset
//! with the values they state ([`ContextZones::read_xplan_gml`], #321),
//! both through the GML 3 geometry subset of [`gml`]. A rule names a zone by its id, or
//! by the kind of zone the object stands in (`stands_in`), which the rules
//! crate resolves per object by plan containment, never by a guess.

mod cadastre;
mod gml;
mod xplan;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::coordinate_system::{MapPlacement, MapPlacementRefusal, SourceCoordinateSystem};
use crate::local_origin::LocalOrigin;
use crate::zone_area::{PlanPolygon, signed_double_area};

/// The locator prefix of a zone's statement:
/// `axioval:zone;id=<id>;kind=<kind>;sha256=<digest>#feature=<n>`.
pub const ZONE_LOCATOR_PREFIX: &str = "axioval:zone";

/// The reference-system name stating coordinates in the model's own plan
/// coordinates, in metres.
pub const MODEL_REFERENCE: &str = "axioval:model";

/// The coordinate reference system a file states its zones in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneReference {
    /// The model's own plan coordinates, in metres, as every source's
    /// geometry is stated.
    Model,
    /// A map coordinate reference system, by the name the file gives (such
    /// as `EPSG:25832`), in that system's unit.
    Map(String),
}

/// A zone's geometry.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneShape {
    /// An area: polygons that do not overlap, each with its holes.
    Area(Vec<PlanPolygon>),
    /// A line, such as a building line: its points in order.
    Line(Vec<[f64; 2]>),
}

impl ZoneShape {
    /// Every straight piece of its boundary (each ring closed) or line.
    #[must_use]
    pub fn segments(&self) -> Vec<([f64; 2], [f64; 2])> {
        match self {
            Self::Area(polygons) => polygons
                .iter()
                .flat_map(PlanPolygon::rings)
                .flat_map(|ring| {
                    ring.iter()
                        .copied()
                        .zip(ring.iter().copied().cycle().skip(1))
                })
                .collect(),
            Self::Line(points) => points.windows(2).map(|pair| (pair[0], pair[1])).collect(),
        }
    }

    fn map_points(&self, mut place: impl FnMut([f64; 2]) -> [f64; 2]) -> Result<Self, String> {
        let mut ring = |points: &[[f64; 2]]| points.iter().map(|point| place(*point)).collect();
        Ok(match self {
            Self::Area(polygons) => Self::Area(
                polygons
                    .iter()
                    .map(|polygon| {
                        let outer = ring(polygon.outer());
                        let holes = polygon.holes().iter().map(|hole| ring(hole)).collect();
                        PlanPolygon::try_new(outer, holes)
                    })
                    .collect::<Result<_, _>>()
                    .map_err(|_| "the zone does not keep its area in model coordinates")?,
            ),
            Self::Line(points) => Self::Line(ring(points)),
        })
    }
}

/// One zone as its file states it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextZone {
    id: String,
    kind: String,
    shape: ZoneShape,
    accuracy_metres: f64,
    feature: usize,
    values: BTreeMap<String, ZoneValue>,
}

/// A value a zone states (#321), such as a zoning area's greatest coverage
/// ratio, by its key ([`axioval_ir::measured::ZONE_NUMBERS`],
/// [`axioval_ir::measured::ZONE_LENGTHS`] or
/// [`axioval_ir::measured::ZONE_ANGLES`]), or, from a `GeoJSON` feature's
/// `values` (#324), by any key: text, a plain number or a quantity, such
/// as a snow load zone and its characteristic load. A key a zone does not
/// hold is a value its file does not state: `null`, never a default.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneValue {
    /// Stated: a plain number, a length in metres, or an angle in
    /// `degrees` (else radians), as the file states it, with the name of
    /// what stated it (`GRZ`, `hoehenangabe[TH].hMax`).
    Stated {
        /// The value, in the unit stated.
        value: f64,
        /// Whether an angle is stated in degrees.
        degrees: bool,
        /// What in the file stated it.
        statement: String,
    },
    /// Stated in a way the reader does not read (a height above another
    /// reference, a value stated twice differently): reading it is not
    /// evaluated, naming why, never `null`.
    Unread {
        /// Why it is not read.
        why: String,
    },
    /// Stated as text (#324), such as a load zone's name.
    Text {
        /// The text.
        text: String,
        /// What in the file stated it.
        statement: String,
    },
    /// Stated as a quantity of a unit the engine reads (#324), such as a
    /// characteristic snow load in kN/m².
    Quantity {
        /// The value, in `unit`.
        value: f64,
        /// The unit, as stated.
        unit: String,
        /// What in the file stated it.
        statement: String,
    },
}

impl ContextZone {
    /// A zone a reader made: its `id`, its `kind` (lowercase letters,
    /// digits and `-`), its shape, the accuracy its boundary is stated
    /// within, in metres, and its `feature` position in its file, from one.
    ///
    /// # Errors
    ///
    /// An id unusable in a measured name, a kind of other characters, or
    /// an accuracy not finite and non-negative.
    pub fn try_new(
        id: impl Into<String>,
        kind: impl Into<String>,
        shape: ZoneShape,
        accuracy_metres: f64,
        feature: usize,
    ) -> Result<Self, String> {
        let (id, kind) = (id.into(), kind.into());
        check_id(&id)?;
        if !check_kind(&kind) {
            return Err(format!(
                "zone `{id}`: the kind `{kind}` is not lowercase letters, digits and `-`"
            ));
        }
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(format!(
                "zone `{id}`: the accuracy {accuracy_metres} is not a non-negative number of \
                 metres"
            ));
        }
        Ok(Self {
            id,
            kind,
            shape,
            accuracy_metres,
            feature,
            values: BTreeMap::new(),
        })
    }

    /// The same zone stating `values`, by key.
    #[must_use]
    pub fn with_values(mut self, values: BTreeMap<String, ZoneValue>) -> Self {
        self.values = values;
        self
    }

    /// The values it states, by key.
    #[must_use]
    pub fn values(&self) -> &BTreeMap<String, ZoneValue> {
        &self.values
    }

    /// Where it is stated in `file`, for evidence:
    /// `axioval:zone;id=<id>;kind=<kind>;sha256=<digest>#feature=<n>`.
    #[must_use]
    pub fn locator(&self, file: &ContextZones) -> String {
        format!(
            "{ZONE_LOCATOR_PREFIX};id={};kind={};sha256={}#feature={}",
            self.id, self.kind, file.sha256, self.feature
        )
    }

    /// The zone's id, unique among the zones supplied to a run.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the zone is: `parcel`, `zone`, `building-limit`,
    /// `building-line` or another lowercase word.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Its geometry, as stated.
    #[must_use]
    pub fn shape(&self) -> &ZoneShape {
        &self.shape
    }

    /// How far the true boundary may lie from the stated one, in metres.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }

    /// The feature's position in its file, from one.
    #[must_use]
    pub fn feature(&self) -> usize {
        self.feature
    }

    /// The zone placed in the model coordinates of a source whose
    /// coordinate system is `system` (`None` when the source states none
    /// or it cannot be read): as stated for [`ZoneReference::Model`], and
    /// through the source's map conversion for [`ZoneReference::Map`],
    /// its accuracy widened by the transform's rounding.
    ///
    /// # Errors
    ///
    /// Why the transform into the source's model coordinates cannot be
    /// established.
    pub fn place(
        &self,
        file: &ContextZones,
        system: Option<&SourceCoordinateSystem>,
    ) -> Result<PlacedZone, String> {
        self.place_in(file, system, LocalOrigin::default())
    }

    /// The zone placed as [`Self::place`] places it, then brought into the
    /// local frame about `origin` the source's geometry is measured in
    /// (#372): every point less the origin, the accuracy widened by the
    /// rounding of that difference unless every point's is exact.
    ///
    /// # Errors
    ///
    /// As [`Self::place`].
    pub fn place_in(
        &self,
        file: &ContextZones,
        system: Option<&SourceCoordinateSystem>,
        origin: LocalOrigin,
    ) -> Result<PlacedZone, String> {
        let placed = self.place_in_model(file, system)?;
        if origin.is_zero() {
            return Ok(placed);
        }
        let (mut exact, mut rounding) = (true, 0.0_f64);
        let shape = placed.shape.map_points(|point| {
            let (local, held) = origin.to_local_exactly(point);
            exact &= held;
            rounding = rounding.max(origin.rounding(point[0].abs().max(point[1].abs())));
            local
        })?;
        let (accuracy_metres, exact) = if exact {
            (placed.accuracy_metres, placed.exact)
        } else {
            ((placed.accuracy_metres + rounding).next_up(), false)
        };
        Ok(PlacedZone {
            shape,
            accuracy_metres,
            exact,
            ..placed
        })
    }

    /// [`Self::place`]: the zone in the source's model coordinates.
    fn place_in_model(
        &self,
        file: &ContextZones,
        system: Option<&SourceCoordinateSystem>,
    ) -> Result<PlacedZone, String> {
        let locator = self.locator(file);
        let placed = |shape: ZoneShape, accuracy_metres: f64, exact: bool| PlacedZone {
            id: self.id.clone(),
            kind: self.kind.clone(),
            shape,
            accuracy_metres,
            exact,
            locator: locator.clone(),
        };
        let name = match &file.reference {
            ZoneReference::Model => {
                return Ok(placed(
                    self.shape.clone(),
                    self.accuracy_metres,
                    self.accuracy_metres == 0.0,
                ));
            }
            ZoneReference::Map(name) => name,
        };
        let system = system.ok_or_else(|| {
            format!(
                "zone `{}` is stated in `{name}`, and the model's coordinate system cannot be read",
                self.id
            )
        })?;
        let placement = MapPlacement::of(system, Some(name)).map_err(|refusal| match refusal {
            MapPlacementRefusal::NoConversion
            | MapPlacementRefusal::NoTarget
            | MapPlacementRefusal::OtherTarget { .. } => {
                format!("zone `{}` is stated in `{name}`, and {refusal}", self.id)
            }
            MapPlacementRefusal::WorldNotIdentity => format!(
                "{refusal}, so zone `{}` cannot be placed through its map conversion",
                self.id
            ),
            MapPlacementRefusal::UnitUnknown { .. } | MapPlacementRefusal::Scaled(_) => {
                format!("{refusal}, so zone `{}` cannot be placed", self.id)
            }
        })?;
        let (metres, offset, scale) = (
            placement.metres_per_map_unit(),
            placement.offset_metres(),
            placement.scale(),
        );
        let mut rounding = 0.0_f64;
        let shape = self.shape.map_points(|[east, north]| {
            let (east, north) = (east * metres, north * metres);
            // Every operation rounds once, relative to the largest operand.
            let size = east.abs() + north.abs() + offset[0].abs() + offset[1].abs();
            rounding = rounding.max(16.0 * f64::EPSILON * size / scale);
            placement.to_model([east, north])
        })?;
        let accuracy = (self.accuracy_metres + rounding).next_up();
        Ok(placed(shape, accuracy, false))
    }
}

/// A zone in one source's model coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedZone {
    id: String,
    kind: String,
    shape: ZoneShape,
    accuracy_metres: f64,
    exact: bool,
    locator: String,
}

impl PlacedZone {
    /// The zone's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The zone's kind.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Its geometry in model coordinates, in metres.
    #[must_use]
    pub fn shape(&self) -> &ZoneShape {
        &self.shape
    }

    /// How far its true boundary may lie from the placed one, in metres:
    /// the stated accuracy and the transform's rounding.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }

    /// Whether its boundary is known exactly: stated with no tolerance in
    /// the model's own coordinates.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Where it was stated, for evidence.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }

    /// The zone's area in square metres, an interval sure to hold the
    /// area of every boundary its accuracy allows and the rounding of its
    /// computation; `None` for a line.
    #[must_use]
    pub fn area_square_metres(&self) -> Option<(f64, f64)> {
        let ZoneShape::Area(polygons) = &self.shape else {
            return None;
        };
        let (mut area, mut terms, mut perimeter, mut rings, mut points) =
            (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
        for ring in polygons.iter().flat_map(PlanPolygon::rings) {
            area += signed_double_area(ring) / 2.0;
            let [x0, y0] = ring[0];
            for ([ax, ay], [bx, by]) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                terms += ((ax - x0) * (by - y0)).abs() + ((bx - x0) * (ay - y0)).abs();
                perimeter += (bx - ax).hypot(by - ay);
                points += 1.0;
            }
            rings += 1.0;
        }
        let rounding = 4.0 * points * f64::EPSILON * terms;
        let accuracy = self.accuracy_metres;
        let band = perimeter.mul_add(accuracy, rings * std::f64::consts::PI * accuracy * accuracy);
        Some((
            (area - band - rounding).next_down().max(0.0),
            (area + band + rounding).next_up(),
        ))
    }
}

/// The zones of one file: where they came from, the SHA-256 of its bytes,
/// the reference system they are stated in, and the zones in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextZones {
    origin: String,
    sha256: String,
    reference: ZoneReference,
    zones: Vec<ContextZone>,
    format: Option<&'static str>,
}

/// A zone id usable in a measured name and a locator.
fn check_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty()
        || id.starts_with('@')
        || id
            .chars()
            .any(|c| matches!(c, ';' | '=' | '#' | '|' | ',') || c.is_whitespace())
    {
        return Err(format!(
            "the id `{id}` must not be blank, start with `@` or hold `;`, `=`, `#`, `|`, `,` \
             or whitespace"
        ));
    }
    Ok(())
}

/// Whether `kind` is a zone kind: lowercase letters, digits and `-`.
fn check_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A non-negative finite accuracy in metres.
fn accuracy(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|metres| metres.is_finite() && *metres >= 0.0)
        .ok_or_else(|| format!("the accuracy {value} is not a non-negative number of metres"))
}

/// A position: two or three finite numbers, of which the plan two are
/// kept.
fn position(value: &Value) -> Result<[f64; 2], String> {
    let numbers = value
        .as_array()
        .filter(|numbers| (2..=3).contains(&numbers.len()))
        .ok_or_else(|| format!("{value} is not a position of two or three numbers"))?;
    let coordinate = |index: usize| {
        numbers[index]
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{value} is not a position of finite numbers"))
    };
    Ok([coordinate(0)?, coordinate(1)?])
}

fn positions(value: &Value) -> Result<Vec<[f64; 2]>, String> {
    value
        .as_array()
        .ok_or_else(|| "coordinates are not a list of positions".to_owned())?
        .iter()
        .map(position)
        .collect()
}

fn polygon(value: &Value) -> Result<PlanPolygon, String> {
    let rings = value
        .as_array()
        .filter(|rings| !rings.is_empty())
        .ok_or_else(|| "a polygon is not a list of rings".to_owned())?;
    let mut rings = rings.iter().map(positions);
    let outer = rings.next().transpose()?.unwrap_or_default();
    let holes = rings.collect::<Result<Vec<_>, _>>()?;
    PlanPolygon::try_new(outer, holes).map_err(|_| {
        "a ring has fewer than three points, repeats a point beside itself or encloses no area"
            .to_owned()
    })
}

fn shape(geometry: &Value) -> Result<ZoneShape, String> {
    let coordinates = &geometry["coordinates"];
    match geometry["type"].as_str() {
        Some("Polygon") => Ok(ZoneShape::Area(vec![polygon(coordinates)?])),
        Some("MultiPolygon") => Ok(ZoneShape::Area(
            coordinates
                .as_array()
                .filter(|polygons| !polygons.is_empty())
                .ok_or_else(|| "a multipolygon is not a list of polygons".to_owned())?
                .iter()
                .map(polygon)
                .collect::<Result<_, _>>()?,
        )),
        Some("LineString") => {
            let points = positions(coordinates)?;
            #[allow(clippy::float_cmp)] // Equal as stated.
            let repeated = points.windows(2).any(|pair| pair[0] == pair[1]);
            if points.len() < 2 || repeated {
                return Err(
                    "a line has fewer than two points or repeats a point beside itself".into(),
                );
            }
            Ok(ZoneShape::Line(points))
        }
        Some(other) => Err(format!(
            "a `{other}` geometry states no zone; a zone is a Polygon, a MultiPolygon or a \
             LineString"
        )),
        None => Err("it has no geometry".into()),
    }
}

impl ContextZones {
    /// Zones a reader already made, from `origin` (such as a file name)
    /// whose bytes have the lowercase hex SHA-256 `sha256`.
    ///
    /// # Errors
    ///
    /// Two zones of one id, or an id unusable in a measured name.
    pub fn new(
        origin: impl Into<String>,
        sha256: impl Into<String>,
        reference: ZoneReference,
        zones: Vec<ContextZone>,
    ) -> Result<Self, String> {
        let mut seen = std::collections::BTreeSet::new();
        for zone in &zones {
            check_id(&zone.id)?;
            if !seen.insert(zone.id.as_str()) {
                return Err(format!("two zones have the id `{}`", zone.id));
            }
        }
        Ok(Self {
            origin: origin.into(),
            sha256: sha256.into(),
            reference,
            zones,
            format: None,
        })
    }

    /// The same zones, read from the exchange `format`.
    pub(crate) fn read_as(mut self, format: &'static str) -> Self {
        self.format = Some(format);
        self
    }

    /// The exchange the zones were read from: `geojson`, `cadastre-gml`
    /// or `xplan-gml`; `None` for zones a host reader made with
    /// [`Self::new`].
    #[must_use]
    pub fn format(&self) -> Option<&'static str> {
        self.format
    }

    /// The plan extent `[west, south, east, north]` of every zone placed
    /// in the model coordinates of a source whose coordinate system is
    /// `system` ([`ContextZone::place`]), grown by each zone's accuracy:
    /// what a host compares with the model's extent to tell whether the
    /// coordinates match (#319). `None` for a file stating no zone.
    ///
    /// # Errors
    ///
    /// Why the first zone that cannot be placed cannot: the reference
    /// systems do not match, or the map conversion cannot place them.
    pub fn placed_extent(
        &self,
        system: Option<&SourceCoordinateSystem>,
    ) -> Result<Option<[f64; 4]>, String> {
        let mut extent: Option<[f64; 4]> = None;
        for zone in &self.zones {
            let placed = zone.place(self, system)?;
            let grow = placed.accuracy_metres();
            for (start, end) in placed.shape().segments() {
                for [x, y] in [start, end] {
                    let [west, south, east, north] = extent.get_or_insert([
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ]);
                    *west = west.min(x - grow);
                    *south = south.min(y - grow);
                    *east = east.max(x + grow);
                    *north = north.max(y + grow);
                }
            }
        }
        Ok(extent)
    }

    /// Reads the zones of a `GeoJSON` `FeatureCollection`, the file `name`.
    ///
    /// The collection names its reference system in the member `crs`
    /// (`{"type": "name", "properties": {"name": …}}`): `axioval:model`
    /// for the model's own plan coordinates in metres, or a map system
    /// such as `EPSG:25832`. A collection naming none is refused, since
    /// `GeoJSON`'s default (longitude and latitude) is never placed. Each
    /// feature is one zone: its `id` (the feature's, or its property `id`),
    /// its property `kind` (a lowercase word), its `accuracy` in metres
    /// (the feature's property, else the collection's member `accuracy`;
    /// one of them must state it) and a `Polygon`, `MultiPolygon` or
    /// `LineString` geometry. Other properties are kept in the file, never
    /// read.
    ///
    /// # Errors
    ///
    /// Why the file is refused, naming the feature.
    pub fn read_geojson(bytes: &[u8], name: &str) -> Result<Self, String> {
        let document: Value =
            serde_json::from_slice(bytes).map_err(|error| format!("not JSON: {error}"))?;
        if document["type"].as_str() != Some("FeatureCollection") {
            return Err("it is not a GeoJSON FeatureCollection".into());
        }
        let reference = match document["crs"]["properties"]["name"].as_str() {
            Some(MODEL_REFERENCE) => ZoneReference::Model,
            Some(name) if !name.trim().is_empty() => ZoneReference::Map(name.trim().to_owned()),
            _ => {
                return Err(format!(
                    "it names no coordinate reference system (`crs`, `{MODEL_REFERENCE}` for \
                     the model's coordinates or a map system such as `EPSG:25832`)"
                ));
            }
        };
        let default_accuracy = match document.get("accuracy") {
            Some(value) => Some(accuracy(value).map_err(|why| format!("the collection: {why}"))?),
            None => None,
        };
        let features = document["features"]
            .as_array()
            .ok_or("it has no `features` list")?;
        let mut zones = Vec::with_capacity(features.len());
        for (index, feature) in features.iter().enumerate() {
            let number = index + 1;
            let refuse = |why: String| format!("feature {number}: {why}");
            let properties = &feature["properties"];
            let id = match (&feature["id"], &properties["id"]) {
                (Value::String(id), _) | (Value::Null, Value::String(id)) => id.clone(),
                (Value::Number(id), _) | (Value::Null, Value::Number(id)) => id.to_string(),
                _ => return Err(refuse("it states no id".into())),
            };
            check_id(&id).map_err(refuse)?;
            let kind = properties["kind"]
                .as_str()
                .filter(|kind| check_kind(kind))
                .ok_or_else(|| {
                    refuse(format!(
                        "zone `{id}` states no `kind` of lowercase letters, digits and `-` (such \
                         as `parcel`, `zone`, `building-limit` or `building-line`)"
                    ))
                })?
                .to_owned();
            let accuracy_metres = match properties.get("accuracy") {
                Some(value) => {
                    accuracy(value).map_err(|why| refuse(format!("zone `{id}`: {why}")))?
                }
                None => default_accuracy.ok_or_else(|| {
                    refuse(format!(
                        "zone `{id}` states no `accuracy`, and the collection none for every zone"
                    ))
                })?,
            };
            let shape =
                shape(&feature["geometry"]).map_err(|why| refuse(format!("zone `{id}`: {why}")))?;
            let values = match properties.get("values") {
                None | Some(Value::Null) => BTreeMap::new(),
                Some(values) => {
                    stated_values(values).map_err(|why| refuse(format!("zone `{id}`: {why}")))?
                }
            };
            zones.push(ContextZone {
                id,
                kind,
                shape,
                accuracy_metres,
                feature: number,
                values,
            });
        }
        Self::new(
            name,
            crate::table_files::sha256_hex(bytes),
            reference,
            zones,
        )
        .map(|zones| zones.read_as("geojson"))
    }

    /// Reads the zones of a GML document, the file `name`, whose
    /// boundaries are stated within `accuracy_metres` (GML states none): a
    /// zoning plan, rooted in `XPlanAuszug` ([`Self::read_xplan_gml`]), or
    /// else cadastral parcels ([`Self::read_cadastre_gml`]).
    ///
    /// # Errors
    ///
    /// As the reader of its exchange.
    pub fn read_gml(bytes: &[u8], name: &str, accuracy_metres: f64) -> Result<Self, String> {
        let root = gml::parse(bytes, "GML")?;
        if root.name == "XPlanAuszug" {
            return Self::xplan_from(&root, bytes, name, accuracy_metres);
        }
        Self::cadastre_from(&root, bytes, name, accuracy_metres)
    }

    /// Where the zones came from.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The SHA-256 of the bytes they were read from.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The reference system their coordinates are stated in.
    #[must_use]
    pub fn reference(&self) -> &ZoneReference {
        &self.reference
    }

    /// The zones, in the file's order.
    #[must_use]
    pub fn zones(&self) -> &[ContextZone] {
        &self.zones
    }
}

/// The zones a host supplied to a run, from every file: what the runtime
/// installs in every run, empty when none were supplied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SuppliedZones {
    files: Vec<Arc<ContextZones>>,
}

impl SuppliedZones {
    /// The zones of `files`, in order, as a run holds them.
    ///
    /// # Errors
    ///
    /// A zone id stated in two files.
    pub fn try_from_files(files: Vec<ContextZones>) -> Result<Self, String> {
        let mut supplied = Self::default();
        for file in files {
            supplied.supply(file)?;
        }
        Ok(supplied)
    }

    /// Whether any file of zones was supplied.
    #[must_use]
    pub fn is_supplied(&self) -> bool {
        !self.files.is_empty()
    }

    /// The files supplied, in order.
    #[must_use]
    pub fn files(&self) -> &[Arc<ContextZones>] {
        &self.files
    }

    /// The zone `id` and the file stating it.
    #[must_use]
    pub fn zone(&self, id: &str) -> Option<(&ContextZones, &ContextZone)> {
        self.files.iter().find_map(|file| {
            file.zones
                .iter()
                .find(|zone| zone.id == id)
                .map(|zone| (file.as_ref(), zone))
        })
    }

    /// Every zone of `kind` with the file stating it, in the order the
    /// files were supplied and each file's order.
    pub fn of_kind<'a>(
        &'a self,
        kind: &'a str,
    ) -> impl Iterator<Item = (&'a ContextZones, &'a ContextZone)> + 'a {
        self.all().filter(move |(_, zone)| zone.kind == kind)
    }

    /// Every zone with the file stating it, in the order the files were
    /// supplied and each file's order.
    pub fn all(&self) -> impl Iterator<Item = (&ContextZones, &ContextZone)> {
        self.files
            .iter()
            .flat_map(|file| file.zones.iter().map(move |zone| (file.as_ref(), zone)))
    }

    /// Adds the zones of one more file.
    pub(crate) fn supply(&mut self, zones: ContextZones) -> Result<(), String> {
        if let Some(zone) = zones
            .zones
            .iter()
            .find(|zone| self.zone(&zone.id).is_some())
        {
            return Err(format!(
                "zone `{}` of {} is also stated in an earlier file",
                zone.id, zones.origin
            ));
        }
        self.files.push(Arc::new(zones));
        Ok(())
    }
}

/// The values a `GeoJSON` feature states under `properties.values` (#324),
/// by key: a key of lowercase letters, digits and `_`; a string is text, a
/// number a plain number, `{"value": …, "unit": …}` a quantity of a unit
/// the engine reads, `null` no value. A key of the zoning plan's neutral
/// lists keeps its dimension: a plain number for
/// [`axioval_ir::measured::ZONE_NUMBERS`], a length for
/// [`axioval_ir::measured::ZONE_LENGTHS`] and an angle for
/// [`axioval_ir::measured::ZONE_ANGLES`], each stated in coherent units.
/// Anything else is refused, never skipped or guessed.
fn stated_values(values: &Value) -> Result<BTreeMap<String, ZoneValue>, String> {
    use axioval_ir::measured::{ZONE_ANGLES, ZONE_LENGTHS, ZONE_NUMBERS};
    let Value::Object(values) = values else {
        return Err("its `values` is not an object of values by key".into());
    };
    let mut stated = BTreeMap::new();
    for (key, value) in values {
        if key.is_empty()
            || !key.starts_with(|c: char| c.is_ascii_lowercase())
            || !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!(
                "the value key `{key}` is not lowercase letters, digits and `_`"
            ));
        }
        let statement = format!("properties.values.{key}");
        let refuse = |why: &str| format!("its value `{key}` {why}");
        let read = match value {
            Value::Null => continue,
            Value::String(text) if text.trim().is_empty() => return Err(refuse("is blank")),
            Value::String(text)
                if ZONE_NUMBERS.contains(&key.as_str())
                    || ZONE_LENGTHS.contains(&key.as_str())
                    || ZONE_ANGLES.contains(&key.as_str()) =>
            {
                return Err(refuse(&format!("is the text `{text}`, not a number")));
            }
            Value::String(text) => ZoneValue::Text {
                text: text.clone(),
                statement,
            },
            Value::Number(number) => {
                let number = number
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| refuse("is not a finite number"))?;
                if ZONE_LENGTHS.contains(&key.as_str()) || ZONE_ANGLES.contains(&key.as_str()) {
                    return Err(refuse("states no unit (`{\"value\": …, \"unit\": …}`)"));
                }
                ZoneValue::Stated {
                    value: number,
                    degrees: false,
                    statement,
                }
            }
            Value::Object(quantity) => stated_quantity(key, quantity, statement)?,
            _ => return Err(refuse("is no text, number or quantity")),
        };
        stated.insert(key.clone(), read);
    }
    Ok(stated)
}

/// A quantity a `GeoJSON` feature states under `key` (#324):
/// `{"value": <number>, "unit": <unit>}`, a neutral length or angle in
/// coherent units, any other key as stated; a neutral plain number takes
/// no unit.
fn stated_quantity(
    key: &str,
    quantity: &serde_json::Map<String, Value>,
    statement: String,
) -> Result<ZoneValue, String> {
    use axioval_ir::measured::{ZONE_ANGLES, ZONE_LENGTHS, ZONE_NUMBERS};
    let refuse = |why: &str| format!("its value `{key}` {why}");
    let dimension = |unit: &str| {
        crate::expression::parse_unit(unit)
            .map(|(_, unit)| unit)
            .map_err(|why| refuse(&format!("states the unit `{unit}`: {why}")))
    };
    let (Some(number), Some(Value::String(unit)), 2) = (
        quantity.get("value").and_then(Value::as_f64),
        quantity.get("unit"),
        quantity.len(),
    ) else {
        return Err(refuse(
            "is no quantity: `{\"value\": <number>, \"unit\": <unit>}`",
        ));
    };
    if !number.is_finite() {
        return Err(refuse("is not a finite number"));
    }
    let found = dimension(unit)?;
    // In coherent units, as an expression reads a quantity.
    let coherent =
        match crate::expression::Value::from_literal(&axioval_ir::contract::ScalarValue::Quantity {
            value: number,
            unit: unit.clone(),
        }) {
            Ok(crate::expression::Value::Number { value, .. }) => value.lower,
            _ => return Err(refuse("overflows in coherent units")),
        };
    let wanted = if ZONE_LENGTHS.contains(&key) {
        Some(("a length", dimension("m")?))
    } else if ZONE_ANGLES.contains(&key) {
        Some(("an angle", dimension("rad")?))
    } else if ZONE_NUMBERS.contains(&key) {
        return Err(refuse("is a plain number, stated without a unit"));
    } else {
        None
    };
    Ok(match wanted {
        Some((_, wanted)) if wanted == found => ZoneValue::Stated {
            value: coherent,
            degrees: false,
            statement,
        },
        Some((what, _)) => return Err(refuse(&format!("is {what}, not stated in `{unit}`"))),
        None => ZoneValue::Quantity {
            value: number,
            unit: unit.clone(),
            statement,
        },
    })
}

/// The zones a plan's rules and values read: every `zone` a measured name
/// states or a rule's parameter binds, and whether any reads a zone the
/// plan cannot name before it runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneReads {
    /// The zone ids read, sorted.
    pub zones: std::collections::BTreeSet<String>,
    /// The kinds of zone read as the zone an object stands in
    /// (`stands_in`), sorted.
    pub kinds: std::collections::BTreeSet<String>,
    /// Whether any measured value reads a zone at all.
    pub any: bool,
}

/// Whether `name` is a measured value reading a context zone.
#[must_use]
pub fn reads_zone(name: &str) -> bool {
    name.starts_with("zone_") || name == "setback_outside_area"
}

/// The zones the strings in `value` read as measured names, the
/// references among them bound by `parameters`.
pub(crate) fn zone_reads(
    value: &Value,
    parameters: &dyn Fn(&str) -> Option<String>,
    reads: &mut ZoneReads,
) {
    match value {
        Value::String(text) => {
            let Ok(call) = axioval_ir::measured::parse(text)
                .or_else(|_| axioval_ir::measured::parse_members(text))
            else {
                return;
            };
            // A value within a zone (`count_within;…;zone=…`) reads it too.
            if !reads_zone(call.name())
                && call.argument("zone").is_none()
                && call.argument("stands_in").is_none()
            {
                return;
            }
            reads.any = true;
            for (key, read) in [("zone", &mut reads.zones), ("stands_in", &mut reads.kinds)] {
                match call.argument(key) {
                    Some(axioval_ir::measured::MeasuredArgument::Text(id)) => {
                        read.insert(id.clone());
                    }
                    Some(axioval_ir::measured::MeasuredArgument::Parameter(parameter)) => {
                        if let Some(id) = parameters(parameter) {
                            read.insert(id);
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                zone_reads(item, parameters, reads);
            }
        }
        Value::Object(fields) => {
            // A `zone` expression (#324) reads its zone too.
            if fields.get("kind").and_then(Value::as_str) == Some("zone") {
                for (key, read) in [("zone", &mut reads.zones), ("standsIn", &mut reads.kinds)] {
                    if let Some(Value::String(named)) = fields.get(key) {
                        reads.any = true;
                        read.insert(named.clone());
                    }
                }
            }
            for item in fields.values() {
                zone_reads(item, parameters, reads);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoordinateFrame, MapConversion, MetricDirection};
    use axioval_ir::{Evidence, SourceId};

    fn file(crs: &str, features: &str) -> String {
        format!(
            r#"{{"type": "FeatureCollection", "crs": {{"type": "name", "properties": {{"name": "{crs}"}}}},
                "accuracy": 0, "features": [{features}]}}"#
        )
    }

    const PARCEL: &str = r#"{"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
        "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [30, 0], [30, 20], [0, 20], [0, 0]]]}}"#;
    const LINE: &str = r#"{"type": "Feature", "properties": {"id": 7, "kind": "building-line", "accuracy": 0.05},
        "geometry": {"type": "LineString", "coordinates": [[0, 5], [30, 5]]}}"#;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn system(map: Option<MapConversion>, origin: [f64; 3]) -> SourceCoordinateSystem {
        let axis = |v: [f64; 3]| MetricDirection::try_new(v).unwrap();
        let world = CoordinateFrame::try_new(
            origin,
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap();
        SourceCoordinateSystem::try_new(
            source(),
            Some(world),
            None,
            map,
            Evidence::exact(source(), "crs"),
        )
        .unwrap()
    }

    #[test]
    fn a_feature_collection_states_zones_with_their_accuracy() {
        let text = file(MODEL_REFERENCE, &format!("{PARCEL}, {LINE}"));
        let zones = ContextZones::read_geojson(text.as_bytes(), "zones.geojson").unwrap();
        assert_eq!(zones.reference(), &ZoneReference::Model);
        assert_eq!(zones.zones().len(), 2);
        let parcel = &zones.zones()[0];
        assert_eq!((parcel.id(), parcel.kind()), ("P1", "parcel"));
        assert!((parcel.accuracy_metres() - 0.0).abs() < f64::EPSILON);
        let line = &zones.zones()[1];
        assert_eq!((line.id(), line.feature()), ("7", 2));
        assert!((line.accuracy_metres() - 0.05).abs() < f64::EPSILON);
        let placed = parcel.place(&zones, None).unwrap();
        assert!(placed.is_exact());
        let (lower, upper) = placed.area_square_metres().unwrap();
        assert!(lower <= 600.0 && upper >= 600.0 && upper - lower < 1e-9);
        assert!(
            placed
                .locator()
                .starts_with("axioval:zone;id=P1;kind=parcel;sha256=")
        );
        assert_eq!(line.place(&zones, None).unwrap().area_square_metres(), None);
        assert_eq!(placed.shape().segments().len(), 4);
    }

    #[test]
    fn a_zone_with_an_accuracy_has_an_area_interval() {
        let text =
            file(MODEL_REFERENCE, PARCEL).replace(r#""accuracy": 0,"#, r#""accuracy": 0.1,"#);
        let zones = ContextZones::read_geojson(text.as_bytes(), "zones.geojson").unwrap();
        let placed = zones.zones()[0].place(&zones, None).unwrap();
        assert!(!placed.is_exact());
        let (lower, upper) = placed.area_square_metres().unwrap();
        // A perimeter of 100 m moved by 0.1 m either way.
        assert!(lower < 590.0 && lower > 589.0, "{lower}");
        assert!(upper > 610.0 && upper < 611.0, "{upper}");
    }

    #[test]
    fn a_file_is_refused_for_what_it_leaves_unstated() {
        for (text, why) in [
            (
                r#"{"type": "FeatureCollection", "features": []}"#.to_owned(),
                "coordinate reference system",
            ),
            (
                file(MODEL_REFERENCE, PARCEL).replace(r#""accuracy": 0,"#, ""),
                "states no `accuracy`",
            ),
            (
                file(MODEL_REFERENCE, &format!("{PARCEL}, {PARCEL}")),
                "two zones",
            ),
            (
                file(MODEL_REFERENCE, &PARCEL.replace("parcel", "Parcel")),
                "`kind`",
            ),
            (
                file(MODEL_REFERENCE, &PARCEL.replace("\"P1\"", "\"P 1\"")),
                "whitespace",
            ),
            (
                file(MODEL_REFERENCE, &PARCEL.replace("Polygon", "Point")),
                "states no zone",
            ),
            (
                file(MODEL_REFERENCE, &PARCEL.replace("[30, 20], [0, 20], ", "")),
                "fewer than three",
            ),
        ] {
            let error = ContextZones::read_geojson(text.as_bytes(), "z.geojson").unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
    }

    #[test]
    fn a_map_zone_is_placed_through_the_map_conversion_only() {
        // The model's x axis points north-east of the map, its origin at
        // easting 500 000 and northing 5 700 000.
        let (cos, sin) = (0.6, 0.8);
        let text = file(
            "urn:ogc:def:crs:EPSG::25832",
            r#"{"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
               "geometry": {"type": "Polygon", "coordinates": [[[500000, 5700000], [500006, 5700008], [499998, 5700014], [500000, 5700000]]]}}"#,
        );
        let zones = ContextZones::read_geojson(text.as_bytes(), "z.geojson").unwrap();
        let zone = &zones.zones()[0];
        let map = |target: &str, scale: f64, unit: Option<f64>| {
            MapConversion::try_new(
                Some(target.into()),
                [500_000.0, 5_700_000.0, 0.0],
                [cos, sin],
                scale,
                unit,
            )
            .unwrap()
        };
        let placed = zone
            .place(
                &zones,
                Some(&system(Some(map("EPSG:25832", 1.0, Some(1.0))), [0.0; 3])),
            )
            .unwrap();
        let ZoneShape::Area(polygons) = placed.shape() else {
            panic!("an area");
        };
        // (6, 8) in the map is 10 m along the model's x axis.
        let outer = polygons[0].outer();
        assert!(
            outer
                .iter()
                .any(|[x, y]| (x - 10.0).abs() < 1e-6 && y.abs() < 1e-6),
            "{outer:?}"
        );
        assert!(!placed.is_exact());
        assert!(placed.accuracy_metres() > 0.0 && placed.accuracy_metres() < 1e-6);
        for (system, why) in [
            (None, "cannot be read"),
            (Some(system(None, [0.0; 3])), "no map conversion"),
            (
                Some(system(Some(map("EPSG:31467", 1.0, Some(1.0))), [0.0; 3])),
                "EPSG:31467",
            ),
            (
                Some(system(Some(map("EPSG:25832", 1.0, None)), [0.0; 3])),
                "not known exactly",
            ),
            (
                Some(system(Some(map("EPSG:25832", 0.001, Some(1.0))), [0.0; 3])),
                "may convert units",
            ),
            (
                Some(system(
                    Some(map("EPSG:25832", 1.0, Some(1.0))),
                    [1.0, 0.0, 0.0],
                )),
                "world frame",
            ),
        ] {
            let error = zone.place(&zones, system.as_ref()).unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
    }

    #[test]
    fn a_zone_id_is_supplied_once() {
        let text = file(MODEL_REFERENCE, PARCEL);
        let zones = ContextZones::read_geojson(text.as_bytes(), "a.geojson").unwrap();
        let mut supplied = SuppliedZones::default();
        assert!(!supplied.is_supplied());
        supplied.supply(zones.clone()).unwrap();
        assert!(supplied.zone("P1").is_some());
        assert!(supplied.zone("P2").is_none());
        assert!(supplied.supply(zones).unwrap_err().contains("earlier file"));
    }

    #[test]
    fn zone_reads_are_found_in_measured_names_and_bound_parameters() {
        let value = serde_json::json!({
            "requirement": {"kind": "property", "propertySet": "axioval:measured",
                            "property": "zone_outside_area;zone=P1"},
            "other": ["zone_inside_area;zone=@parcel", "area", "not;a=measured",
                      "count_within;to=tree;zone=P3"],
        });
        let mut reads = ZoneReads::default();
        zone_reads(
            &value,
            &|name| (name == "parcel").then(|| "P2".to_owned()),
            &mut reads,
        );
        assert!(reads.any);
        assert_eq!(
            reads.zones.into_iter().collect::<Vec<_>>(),
            ["P1".to_owned(), "P2".to_owned(), "P3".to_owned()]
        );
        // A `zone` expression reads its zone or kind.
        let mut reads = ZoneReads::default();
        zone_reads(
            &serde_json::json!({"left": {"kind": "zone", "standsIn": "snow-load-zone",
                                         "value": "load_zone"},
                                "right": {"kind": "zone", "zone": "S1"}}),
            &|_| None,
            &mut reads,
        );
        assert!(reads.any);
        assert_eq!(
            reads.kinds.into_iter().collect::<Vec<_>>(),
            ["snow-load-zone"]
        );
        assert_eq!(reads.zones.into_iter().collect::<Vec<_>>(), ["S1"]);
    }

    fn zone_with(values: &str) -> Result<ContextZone, String> {
        let feature = format!(
            r#"{{"type": "Feature", "id": "S1", "properties": {{"kind": "snow-load-zone",
                "values": {values}}},
                "geometry": {{"type": "Polygon", "coordinates": [[[0, 0], [9, 0], [9, 9], [0, 0]]]}}}}"#
        );
        ContextZones::read_geojson(file(MODEL_REFERENCE, &feature).as_bytes(), "s.geojson")
            .map(|file| file.zones()[0].clone())
    }

    #[test]
    fn a_geojson_feature_states_values_by_key() {
        let zone = zone_with(
            r#"{"load_zone": "2a", "characteristic_load": {"value": 0.85, "unit": "kN/m²"},
                "altitude_class": 3, "eaves_height_max": {"value": 6500, "unit": "mm"},
                "roof_pitch_max": {"value": 45, "unit": "°"}, "note": null}"#,
        )
        .unwrap();
        let values = zone.values();
        assert_eq!(
            values["load_zone"],
            ZoneValue::Text {
                text: "2a".into(),
                statement: "properties.values.load_zone".into()
            }
        );
        assert!(matches!(
            &values["characteristic_load"],
            ZoneValue::Quantity { value, unit, .. } if (*value - 0.85).abs() < 1e-12 && unit == "kN/m²"
        ));
        assert!(matches!(
            values["altitude_class"],
            ZoneValue::Stated { value, degrees: false, .. } if (value - 3.0).abs() < 1e-12
        ));
        // A neutral length or angle is stated in coherent units.
        assert!(matches!(
            values["eaves_height_max"],
            ZoneValue::Stated { value, .. } if (value - 6.5).abs() < 1e-12
        ));
        assert!(matches!(
            values["roof_pitch_max"],
            ZoneValue::Stated { value, .. } if (value - std::f64::consts::FRAC_PI_4).abs() < 1e-12
        ));
        assert!(!values.contains_key("note"));
        // Anything else is refused by name.
        for (values, why) in [
            (r#"{"Load": 1}"#, "is not lowercase"),
            (r#"{"load_zone": ""}"#, "is blank"),
            (r#"{"load_zone": true}"#, "no text, number or quantity"),
            (r#"{"eaves_height": 6}"#, "states no unit"),
            (
                r#"{"eaves_height": {"value": 6, "unit": "kg"}}"#,
                "is a length",
            ),
            (r#"{"storeys": {"value": 2, "unit": "m"}}"#, "plain number"),
            (r#"{"storeys": "II"}"#, "not a number"),
            (r#"{"load": {"value": 1, "unit": "furlong"}}"#, "furlong"),
            (r#"{"load": {"value": 1}}"#, "is no quantity"),
            (r"[1]", "not an object"),
        ] {
            let refused = zone_with(values).unwrap_err();
            assert!(refused.contains(why), "{values}: {refused}");
        }
    }
}
