//! Context zones (#327): footprints inside and outside a zone supplied
//! beside the model, a zone's area, and the plot ratios GRZ and GFZ over
//! a parcel's buildings, measured over a plan-area service of rectangles.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ContextZones, Measurement, MemberValue, PlanArea, PlanAreaError,
    PlanAreaService, PlanAreaServiceHandle, PropertyResolution, ServiceRegistry, SuppliedZones,
    ZoneArea, ZoneAreaRequest, measured_members, measured_value,
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

/// A cadastral GML subset (#319) in the model's coordinates: parcels
/// `0001`, `0002` and `0003` side by side (x 0..20, 20..40, 40..60, y
/// 0..30) and the street `S1` along their south edge (y -10..0), stated
/// within `accuracy` metres.
fn cadastre(accuracy: f64) -> ContextZones {
    let ring = |x0: f64, y0: f64, x1: f64, y1: f64| {
        format!("{x0} {y0} {x1} {y0} {x1} {y1} {x0} {y1} {x0} {y0}")
    };
    let polygon = |ring: String| {
        format!(
            "<position><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>{ring}\
             </gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></position>"
        )
    };
    let mut members = String::new();
    for (number, x0) in [(1, 0.0), (2, 20.0), (3, 40.0)] {
        let _ = write!(
            members,
            "<gml:featureMember><AX_Flurstueck gml:id=\"F{number}\">\
             <flurstueckskennzeichen>000{number}</flurstueckskennzeichen>{}\
             </AX_Flurstueck></gml:featureMember>",
            polygon(ring(x0, 0.0, x0 + 20.0, 30.0))
        );
    }
    let _ = write!(
        members,
        "<gml:featureMember><AX_Strassenverkehr gml:id=\"S1\">{}</AX_Strassenverkehr>\
         </gml:featureMember>",
        polygon(ring(0.0, -10.0, 60.0, 0.0))
    );
    let text = format!(
        "<wfs:FeatureCollection xmlns:wfs=\"http://www.opengis.net/wfs\" \
         xmlns:gml=\"http://www.opengis.net/gml/3.2\"><gml:boundedBy>\
         <gml:Envelope srsName=\"axioval:model\"/></gml:boundedBy>{members}\
         </wfs:FeatureCollection>"
    );
    ContextZones::read_cadastre_gml(text.as_bytes(), "alkis.xml", accuracy).unwrap()
}

/// Buildings beside [`cadastre`]: `house` on parcel `0002`, `straddler`
/// across the boundary of `0001` and `0002`, `afar` on no parcel.
fn on_parcels(accuracy: f64) -> (Project, ServiceRegistry) {
    on_parcels_with(accuracy, Vec::new())
}

/// [`on_parcels`] with the zones of `more` supplied after the cadastre.
fn on_parcels_with(accuracy: f64, more: Vec<ContextZones>) -> (Project, ServiceRegistry) {
    let model = Model::default()
        .object("house", "building")
        .object("straddler", "building")
        .object("afar", "building");
    let rectangles = BTreeMap::from([
        (id("house"), ([24.0, 5.0, 34.0, 15.0], false)),
        (id("straddler"), ([15.0, 5.0, 25.0, 12.0], false)),
        (id("afar"), ([70.0, 5.0, 75.0, 10.0], false)),
    ]);
    let mut services = ServiceRegistry::new();
    services
        .register(PlanAreaServiceHandle::new(Arc::new(Rectangles(rectangles))))
        .unwrap();
    services
        .register(SuppliedZones::try_from_files([vec![cadastre(accuracy)], more].concat()).unwrap())
        .unwrap();
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// engine#319: the parcel a building stands on is selected by its kind,
/// not its id, and its area measured; a building straddling two parcels,
/// and one on none, is not evaluated, never given one.
#[test]
fn the_parcel_a_building_stands_on_is_selected_by_kind() {
    let site = on_parcels(0.0);
    let parcel = value(&site, "house", "zone_area;stands_in=parcel").unwrap();
    assert!(holds(parcel, 600.0), "{parcel:?}");
    let outside = value(&site, "house", "zone_outside_area;stands_in=parcel").unwrap();
    assert!(holds(outside, 0.0), "{outside:?}");
    let why = value(&site, "straddler", "zone_area;stands_in=parcel").unwrap_err();
    assert!(
        why.contains("may stand in `0001`, `0002` of kind `parcel`"),
        "{why}"
    );
    let why = value(&site, "afar", "zone_area;stands_in=parcel").unwrap_err();
    assert!(
        why.contains("stands in no supplied context zone of kind `parcel`"),
        "{why}"
    );
    let why = value(&site, "house", "zone_area;stands_in=garden").unwrap_err();
    assert!(
        why.contains("no supplied context zone is an area of kind `garden`"),
        "{why}"
    );
    // Within its accuracy of a boundary, a building may stand on either.
    let near = on_parcels(5.0);
    let why = value(&near, "house", "zone_area;stands_in=parcel").unwrap_err();
    assert!(why.contains("may stand in `0001`, `0002`"), "{why}");
}

/// engine#319: the neighbours of the parcel a building stands on are its
/// two neighbouring parcels and the street; by kind, the street alone.
#[test]
fn the_neighbours_of_a_buildings_parcel_are_listed_by_kind() {
    let (_, services) = on_parcels(0.0);
    let listed = |name: &str| -> Vec<(String, bool)> {
        measured_members(&services, &id("house"), name)
            .unwrap()
            .into_iter()
            .map(|member| match &member.fields["zone"] {
                MemberValue::Text { text } => (text.clone(), member.certain),
                other => panic!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(
        listed("zone_neighbours;stands_in=parcel"),
        [
            ("0001".to_owned(), true),
            ("0003".to_owned(), true),
            ("S1".to_owned(), true)
        ]
    );
    assert_eq!(
        listed("zone_neighbours;stands_in=parcel;kinds=street,way,square"),
        [("S1".to_owned(), true)]
    );
    assert_eq!(
        listed("zone_neighbours;zone=0001"),
        [("0002".to_owned(), true), ("S1".to_owned(), true)]
    );
    // Within their accuracies of each other, a parcel apart may border it.
    let (_, services) = on_parcels(0.5);
    let far: Vec<bool> = measured_members(&services, &id("house"), "zone_neighbours;zone=0001")
        .unwrap()
        .iter()
        .map(|member| member.certain)
        .collect();
    assert_eq!(far, [true, true]);
    let area = &measured_members(&services, &id("house"), "zone_neighbours;zone=0001").unwrap()[0]
        .fields["area"];
    let MemberValue::Measured(Measurement::Cited { lower, upper, .. }) = area else {
        panic!("{area:?}");
    };
    assert!(*lower < 600.0 && *upper > 600.0, "{lower} {upper}");
}

#[test]
fn a_zone_is_named_by_id_or_by_kind_never_both() {
    for name in ["zone_area", "zone_area;zone=0001;stands_in=parcel"] {
        let error = axioval_ir::measured::parse(name).unwrap_err().to_string();
        assert!(error.contains("zone"), "{name}: {error}");
    }
    assert!(
        axioval_ir::measured::parse("zone_footprint_area;stands_in=parcel;elements=x").is_err()
    );
}

/// A zoning plan (#321) beside [`cadastre`], in the model's coordinates:
/// the use area `WA1` over the three parcels (x 0..60, y 0..30) stating a
/// greatest coverage ratio of 0.4, roof pitches of 30 to 45 degrees, an
/// eaves height of at most 6.5 m above the terrain and a ridge height
/// above sea level, which is not read.
const PLAN: &str = r#"<xplan:XPlanAuszug xmlns:xplan="http://www.xplanung.de/xplangml/6/0"
    xmlns:gml="http://www.opengis.net/gml/3.2" gml:id="A">
  <gml:boundedBy><gml:Envelope srsName="axioval:model"/></gml:boundedBy>
  <gml:featureMember><xplan:BP_Plan gml:id="P"/></gml:featureMember>
  <gml:featureMember>
    <xplan:BP_BaugebietsTeilFlaeche gml:id="WA1">
      <xplan:position><gml:Polygon><gml:exterior><gml:LinearRing>
        <gml:posList>0 0 60 0 60 30 0 30 0 0</gml:posList>
      </gml:LinearRing></gml:exterior></gml:Polygon></xplan:position>
      <xplan:GRZ>0.4</xplan:GRZ>
      <xplan:dachgestaltung><xplan:BP_Dachgestaltung>
        <xplan:DNmin uom="grad">30</xplan:DNmin><xplan:DNmax uom="grad">45</xplan:DNmax>
      </xplan:BP_Dachgestaltung></xplan:dachgestaltung>
      <xplan:hoehenangabe><xplan:XP_Hoehenangabe>
        <xplan:hoehenbezug>2000</xplan:hoehenbezug><xplan:bezugspunkt>1000</xplan:bezugspunkt>
        <xplan:hMax uom="m">6.5</xplan:hMax>
      </xplan:XP_Hoehenangabe></xplan:hoehenangabe>
      <xplan:hoehenangabe><xplan:XP_Hoehenangabe>
        <xplan:hoehenbezug>1000</xplan:hoehenbezug><xplan:bezugspunkt>2000</xplan:bezugspunkt>
        <xplan:h uom="m">62.0</xplan:h>
      </xplan:XP_Hoehenangabe></xplan:hoehenangabe>
    </xplan:BP_BaugebietsTeilFlaeche>
  </gml:featureMember>
</xplan:XPlanAuszug>"#;

/// [`on_parcels`] with the zoning plan [`PLAN`] supplied too.
fn zoned() -> (Project, ServiceRegistry) {
    let plan = ContextZones::read_gml(PLAN.as_bytes(), "plan.gml", 0.0).unwrap();
    on_parcels_with(0.0, vec![plan])
}

/// engine#321: a building reads the values the use area it stands in
/// states, exactly as stated; a value the plan does not state is `null`,
/// one it states above another reference is not evaluated, and a building
/// in no use area is not evaluated.
#[test]
fn a_building_reads_the_values_of_the_use_area_it_stands_in() {
    let site = zoned();
    let grz = value(
        &site,
        "house",
        "zone_value;stands_in=use-area;value=coverage_ratio_max",
    )
    .unwrap();
    assert_eq!(grz, (0.4, 0.4, true));
    let eaves = value(
        &site,
        "house",
        "zone_length;stands_in=use-area;value=eaves_height_max",
    )
    .unwrap();
    assert_eq!(eaves, (6.5, 6.5, true));
    let (lower, upper, exact) = value(
        &site,
        "house",
        "zone_angle;stands_in=use-area;value=roof_pitch_max",
    )
    .unwrap();
    assert!(lower <= 45_f64.to_radians() && 45_f64.to_radians() <= upper && upper - lower < 1e-12);
    assert!(exact, "a stated pitch, rounded only");
    // By id, the same value; the building's measured coverage ratio of its
    // parcel, 100 of 600 m², is within it.
    let by_id = value(
        &site,
        "house",
        "zone_value;zone=WA1;value=coverage_ratio_max",
    )
    .unwrap();
    assert_eq!(by_id, (0.4, 0.4, true));
    let footprint = value(&site, "house", "zone_inside_area;stands_in=parcel").unwrap();
    let parcel = value(&site, "house", "zone_area;stands_in=parcel").unwrap();
    assert!(footprint.1 / parcel.0 <= grz.0, "{footprint:?} {parcel:?}");
    let why = value(
        &site,
        "house",
        "zone_value;stands_in=use-area;value=coverage_ratio_min",
    )
    .unwrap_err();
    assert_eq!(why, "absent", "a value the plan does not state is null");
    let why = value(
        &site,
        "house",
        "zone_length;stands_in=use-area;value=ridge_height_max",
    )
    .unwrap_err();
    assert!(why.contains("reference `1000`"), "{why}");
    let why = value(
        &site,
        "afar",
        "zone_value;stands_in=use-area;value=coverage_ratio_max",
    )
    .unwrap_err();
    assert!(
        why.contains("stands in no supplied context zone of kind `use-area`"),
        "{why}"
    );
    assert!(
        axioval_ir::measured::parse("zone_length;zone=WA1;value=coverage_ratio_max").is_err(),
        "a ratio is no length"
    );
}
