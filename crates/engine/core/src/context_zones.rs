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

use std::sync::Arc;

use serde_json::Value;

use crate::coordinate_system::SourceCoordinateSystem;
use crate::zone_area::{PlanPolygon, signed_double_area};

/// The locator prefix of a zone's statement:
/// `axioval:zone;id=<id>;kind=<kind>;sha256=<digest>#feature=<n>`.
pub const ZONE_LOCATOR_PREFIX: &str = "axioval:zone";

/// The reference-system name stating coordinates in the model's own plan
/// coordinates, in metres.
pub const MODEL_REFERENCE: &str = "axioval:model";

/// How far a map conversion's scale may differ from one and still be read
/// as the map's own scale factor rather than a unit conversion.
const SCALE_TOLERANCE: f64 = 1e-3;

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
}

impl ContextZone {
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
        let locator = format!(
            "{ZONE_LOCATOR_PREFIX};id={};kind={};sha256={}#feature={}",
            self.id, self.kind, file.sha256, self.feature
        );
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
        let map = system.map().ok_or_else(|| {
            format!(
                "zone `{}` is stated in `{name}`, and the model states no map conversion",
                self.id
            )
        })?;
        let target = map.target().ok_or_else(|| {
            format!(
                "zone `{}` is stated in `{name}`, and the model's map conversion names no \
                 reference system",
                self.id
            )
        })?;
        if reference_key(target) != reference_key(name) {
            return Err(format!(
                "zone `{}` is stated in `{name}`, and the model's map conversion is to `{target}`",
                self.id
            ));
        }
        let (Some(metres), Some(offset)) = (map.metres_per_map_unit(), map.offset_metres()) else {
            return Err(format!(
                "the unit of `{target}` is not known exactly, so zone `{}` cannot be placed",
                self.id
            ));
        };
        // The identity as stated, compared exactly.
        #[allow(clippy::float_cmp)]
        let identity = system.world().is_some_and(|world| {
            let axes = world.axes().map(|axis| axis.components());
            axes == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
                && world.origin_metres() == [0.0; 3]
        });
        if !identity {
            return Err(format!(
                "the model's world frame is not stated as the identity, so zone `{}` cannot be \
                 placed through its map conversion",
                self.id
            ));
        }
        let scale = map.scale();
        if (scale - 1.0).abs() > SCALE_TOLERANCE {
            return Err(format!(
                "the model's map conversion scales by {scale}, which may convert units, so \
                 zone `{}` cannot be placed",
                self.id
            ));
        }
        let [cos, sin] = map.x_axis();
        let mut rounding = 0.0_f64;
        let shape = self.shape.map_points(|[east, north]| {
            let (east, north) = (east * metres, north * metres);
            let (dx, dy) = (east - offset[0], north - offset[1]);
            // Every operation rounds once, relative to the largest operand.
            let size = east.abs() + north.abs() + offset[0].abs() + offset[1].abs();
            rounding = rounding.max(16.0 * f64::EPSILON * size / scale);
            [
                dx.mul_add(cos, dy * sin) / scale,
                (-dx).mul_add(sin, dy * cos) / scale,
            ]
        })?;
        let accuracy = (self.accuracy_metres + rounding).next_up();
        Ok(placed(shape, accuracy, false))
    }
}

/// The name of a coordinate reference system as compared: an EPSG code by
/// its number, whatever form names it, anything else as written.
fn reference_key(name: &str) -> String {
    let upper = name.trim().to_ascii_uppercase();
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
        })
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
                .filter(|kind| {
                    !kind.is_empty()
                        && kind
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
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
            zones.push(ContextZone {
                id,
                kind,
                shape,
                accuracy_metres,
                feature: number,
            });
        }
        Self::new(
            name,
            crate::table_files::sha256_hex(bytes),
            reference,
            zones,
        )
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

/// The zones a plan's rules and values read: every `zone` a measured name
/// states or a rule's parameter binds, and whether any reads a zone the
/// plan cannot name before it runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneReads {
    /// The zone ids read, sorted.
    pub zones: std::collections::BTreeSet<String>,
    /// Whether any measured value reads a zone at all.
    pub any: bool,
}

/// Whether `name` is a measured value reading a context zone.
#[must_use]
pub fn reads_zone(name: &str) -> bool {
    name.starts_with("zone_")
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
            let Ok(call) = axioval_ir::measured::parse(text) else {
                return;
            };
            if !reads_zone(call.name()) {
                return;
            }
            reads.any = true;
            match call.argument("zone") {
                Some(axioval_ir::measured::MeasuredArgument::Text(id)) => {
                    reads.zones.insert(id.clone());
                }
                Some(axioval_ir::measured::MeasuredArgument::Parameter(parameter)) => {
                    if let Some(id) = parameters(parameter) {
                        reads.zones.insert(id);
                    }
                }
                _ => {}
            }
        }
        Value::Array(items) => {
            for item in items {
                zone_reads(item, parameters, reads);
            }
        }
        Value::Object(fields) => {
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
            "other": ["zone_inside_area;zone=@parcel", "area", "not;a=measured"],
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
            ["P1".to_owned(), "P2".to_owned()]
        );
    }
}
