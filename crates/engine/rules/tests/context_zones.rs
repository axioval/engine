//! Context zones (#327): footprints inside and outside a zone supplied
//! beside the model, a zone's area, and the plot ratios GRZ and GFZ over
//! a parcel's buildings, measured over a plan-area service of rectangles.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ContextZones, PlanArea, PlanAreaError, PlanAreaService,
    PlanAreaServiceHandle, PropertyResolution, ServiceRegistry, SuppliedZones, ZoneArea,
    ZoneAreaRequest, measured_value,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::{Model, id, source};

/// Axis-aligned rectangular footprints `[x0, y0, x1, y1]`, each exact or
/// tessellated; zones are read by their outer ring's bounding box, so the
/// tests state rectangles only, and footprints measured together must not
/// overlap.
struct Rectangles(BTreeMap<ObjectId, ([f64; 4], bool)>);

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let width = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let depth = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    width * depth
}

fn size(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1])
}

impl PlanAreaService for Rectangles {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (rectangle, _) = self
            .0
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let area = size(*rectangle);
        PlanArea::try_new(area, area, Evidence::exact(source(), "footprint"))
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }

    fn measure_zone_area(&self, request: &ZoneAreaRequest) -> Result<ZoneArea, PlanAreaError> {
        let outer = request.zone()[0].outer();
        let fold = |pick: fn(f64, f64) -> f64, axis: usize, start: f64| {
            outer.iter().map(|point| point[axis]).fold(start, pick)
        };
        let zone = [
            fold(f64::min, 0, f64::INFINITY),
            fold(f64::min, 1, f64::INFINITY),
            fold(f64::max, 0, f64::NEG_INFINITY),
            fold(f64::max, 1, f64::NEG_INFINITY),
        ];
        let a = request.accuracy_metres();
        let sure = [zone[0] + a, zone[1] + a, zone[2] - a, zone[3] - a];
        let possible = [zone[0] - a, zone[1] - a, zone[2] + a, zone[3] + a];
        let (mut whole, mut inside, mut outside, mut exact) = (0.0, (0.0, 0.0), (0.0, 0.0), true);
        for object in request.objects() {
            let (rectangle, tessellated) = self
                .0
                .get(object)
                .copied()
                .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
            let area = size(rectangle);
            let (least, most) = (overlap(rectangle, sure), overlap(rectangle, possible));
            whole += area;
            inside.0 += least;
            inside.1 += most;
            outside.0 += area - most;
            outside.1 += area - least;
            exact &= !tessellated;
        }
        let widen = |(lower, upper): (f64, f64)| {
            if exact {
                (lower, upper)
            } else {
                ((lower - 0.1_f64).max(0.0), upper + 0.1)
            }
        };
        let footprint = widen((whole, whole));
        let (inside, outside) = (widen(inside), widen(outside));
        #[allow(clippy::float_cmp)]
        let points = [footprint, inside, outside]
            .iter()
            .all(|(lower, upper)| lower == upper);
        let mut evidence = Evidence::exact(source(), "zone-area");
        evidence.exact = points;
        ZoneArea::try_new(
            request.objects().to_vec(),
            footprint,
            inside,
            outside,
            evidence,
        )
    }
}

/// Parcel `P1`, 30 x 20 m, with buildings `house` (10 x 8 m) and `garage`
/// (6 x 5 m) on it and `shed` (4 x 3 m) beside it; building limit `B1`,
/// 20 x 15 m, which `house` keeps to and `wing` (10 x 8 m) crosses by
/// 0.3 m; and building line `L1` along y = 15. The zones are stated
/// within `accuracy` metres.
const ZONES: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "axioval:model"}},
  "accuracy": ACCURACY,
  "features": [
    {"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
     "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [30, 0], [30, 20], [0, 20], [0, 0]]]}},
    {"type": "Feature", "id": "B1", "properties": {"kind": "building-limit"},
     "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [20, 0], [20, 15], [0, 15], [0, 0]]]}},
    {"type": "Feature", "id": "L1", "properties": {"kind": "building-line"},
     "geometry": {"type": "LineString", "coordinates": [[0, 15], [30, 15]]}}
  ]}"#;

fn session(accuracy: f64, zones: bool, tessellated: bool) -> (Project, ServiceRegistry) {
    let model = Model::default()
        .object("house", "building")
        .object("garage", "building")
        .object("shed", "building")
        .object("wing", "wing")
        .object("site", "site");
    let rectangles = BTreeMap::from([
        (id("house"), ([2.0, 2.0, 12.0, 10.0], tessellated)),
        (id("garage"), ([15.0, 2.0, 21.0, 7.0], false)),
        (id("shed"), ([32.0, 0.0, 36.0, 3.0], false)),
        (id("wing"), ([10.3, 4.0, 20.3, 12.0], false)),
    ]);
    let mut services = ServiceRegistry::new();
    services
        .register(PlanAreaServiceHandle::new(Arc::new(Rectangles(rectangles))))
        .unwrap();
    if zones {
        let text = ZONES.replace("ACCURACY", &accuracy.to_string());
        let file = ContextZones::read_geojson(text.as_bytes(), "zones.geojson").unwrap();
        services
            .register(SuppliedZones::try_from_files(vec![file]).unwrap())
            .unwrap();
    }
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// `name` read on `object`: its bounds and whether its evidence is exact.
fn value(
    (project, services): &(Project, ServiceRegistry),
    object: &str,
    name: &str,
) -> Result<(f64, f64, bool), String> {
    match measured_value(services, project, &id(object), name) {
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.into_property();
            let exact = property.evidence.iter().all(|evidence| evidence.exact);
            match property.value {
                PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => {
                    Ok((value, value, exact))
                }
                PropertyValue::Measured { lower, upper, .. } => Ok((lower, upper, exact)),
                other => Err(format!("{other:?}")),
            }
        }
        Ok(PropertyResolution::Absent(_)) => Err("absent".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn holds((lower, upper, _): (f64, f64, bool), expected: f64) -> bool {
    lower <= expected + 1e-9 && expected - 1e-9 <= upper && upper - lower < 1e-6
}

#[test]
fn a_building_crossing_its_limit_by_30_cm_has_its_strip_outside() {
    let site = session(0.0, true, false);
    let outside = value(&site, "wing", "zone_outside_area;zone=B1").unwrap();
    assert!(holds(outside, 2.4), "{outside:?}");
    assert!(outside.2, "an exact zone and footprint measure exactly");
    let inside = value(&site, "wing", "zone_inside_area;zone=B1").unwrap();
    assert!(holds(inside, 77.6), "{inside:?}");
    let kept = value(&site, "house", "zone_outside_area;zone=B1").unwrap();
    assert!(holds(kept, 0.0), "{kept:?}");
}

#[test]
fn straddling_the_limit_within_its_accuracy_is_never_surely_inside() {
    let site = session(0.5, true, false);
    let (lower, upper, exact) = value(&site, "wing", "zone_outside_area;zone=B1").unwrap();
    assert!(lower <= 0.0 && upper > 2.4, "{lower} {upper}");
    assert!(!exact);
}

#[test]
fn the_plot_ratios_of_a_parcel_with_two_buildings_are_measured() {
    let site = session(0.0, true, false);
    let parcel = value(&site, "site", "zone_area;zone=P1").unwrap();
    assert!(holds(parcel, 600.0), "{parcel:?}");
    assert!(parcel.2);
    // GRZ: 80 + 30 m² of 600 m²; the shed stands beside the parcel.
    let covered = value(
        &site,
        "site",
        "zone_footprint_area;zone=P1;elements=building",
    )
    .unwrap();
    assert!(holds(covered, 110.0), "{covered:?}");
    assert!(covered.2);
    let floors = value(&site, "site", "zone_floor_area;zone=P1;elements=building").unwrap();
    assert!(
        floors.0 <= 110.0 && floors.1 >= 110.0 && floors.1 - floors.0 < 1e-9,
        "{floors:?}"
    );
}

#[test]
fn a_zone_measured_on_a_tessellation_is_inexact() {
    let site = session(0.0, true, true);
    for name in [
        "zone_outside_area;zone=B1",
        "zone_inside_area;zone=B1",
        "zone_footprint_area;zone=P1;elements=building",
        "zone_floor_area;zone=P1;elements=building",
    ] {
        let (lower, upper, exact) = value(&site, "house", name).unwrap();
        assert!(!exact, "{name}");
        assert!(lower <= upper, "{name}");
    }
}

#[test]
fn a_zone_that_is_not_supplied_or_encloses_nothing_is_not_evaluated() {
    let site = session(0.0, true, false);
    let why = value(&site, "house", "zone_outside_area;zone=P9").unwrap_err();
    assert!(why.contains("no supplied context zone is `P9`"), "{why}");
    let why = value(&site, "house", "zone_outside_area;zone=L1").unwrap_err();
    assert!(why.contains("encloses no area"), "{why}");
    let why = value(&site, "house", "zone_area;zone=L1").unwrap_err();
    assert!(why.contains("encloses no area"), "{why}");
    let bare = session(0.0, false, false);
    let why = value(&bare, "house", "zone_outside_area;zone=B1").unwrap_err();
    assert!(why.contains("no context zones were supplied"), "{why}");
    let why = value(
        &bare,
        "site",
        "zone_footprint_area;zone=P1;elements=building",
    )
    .unwrap_err();
    assert!(why.contains("no context zones were supplied"), "{why}");
}
