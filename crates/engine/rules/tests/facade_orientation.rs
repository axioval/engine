//! Which way an object faces and how high obstacles rise in front of it:
//! `azimuth`, `tilt` and `obstruction_angle` over the outward faces the
//! facade-area service lists and the elevations the line-of-sight service
//! measures.
#![allow(missing_docs, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CoordinateSystemError, CoordinateSystemService,
    CoordinateSystemServiceHandle, FacadeArea, FacadeAreaError, FacadeAreaService,
    FacadeAreaServiceHandle, FacadeFace, FacadeFaceSide, FacadeFaces, FaceNormal,
    PropertyResolution, ServiceRegistry, SightError, SightEvidence, SightRequest, SightService,
    SightServiceHandle, SkyObstruction, SkyObstructionRequest, SourceCoordinateSystem,
    SourceSnapshot, measured_value,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::runtime::snapshot;
use common::{Model, id, source};

/// The outward faces of each object, and whether they are of its exact
/// shape.
struct Faces(BTreeMap<ObjectId, (Vec<FacadeFace>, bool)>);

impl FacadeAreaService for Faces {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        Err(FacadeAreaError::UnknownObject(object.clone()))
    }

    fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let (faces, exact) = self
            .0
            .get(object)
            .cloned()
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("faces:{object}"));
        evidence.exact = exact;
        FacadeFaces::try_new(object.clone(), faces, evidence)
    }
}

/// A source whose true north points along `north`.
struct North([f64; 2], Vec<SourceSnapshot>);

impl CoordinateSystemService for North {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }

    fn coordinate_system(
        &self,
        source: &axioval_ir::SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        SourceCoordinateSystem::try_new(
            source.clone(),
            None,
            Some(self.0),
            None,
            Evidence::exact(source.clone(), "north"),
        )
    }
}

/// Each obstacle rises to a fixed elevation angle; every request is
/// recorded.
struct Elevations(BTreeMap<ObjectId, f64>);

impl SightService for Elevations {
    fn assess_sight(&self, _: &SightRequest) -> Result<SightEvidence, SightError> {
        Err(SightError::Unavailable("elevations only".into()))
    }

    fn measure_sky_obstruction(
        &self,
        request: &SkyObstructionRequest,
    ) -> Result<SkyObstruction, SightError> {
        // Only the south-facing window's eye, looking south, is answered.
        assert_eq!(request.toward(), [0.0, -1.0]);
        assert_eq!(request.eye(), [5.0, -0.3, 1.75]);
        let highest = request
            .obstacles()
            .iter()
            .filter_map(|obstacle| self.0.get(obstacle).map(|angle| (obstacle, *angle)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match highest {
            Some((obstacle, angle)) => SkyObstruction::try_new(
                (angle, angle),
                vec![obstacle.clone()],
                Evidence::exact(source(), "sky"),
            ),
            None => SkyObstruction::try_new((0.0, 0.0), vec![], Evidence::exact(source(), "sky")),
        }
    }
}

fn face(normal: [f64; 3], area: f64, centroid: [f64; 3], side: FacadeFaceSide) -> FacadeFace {
    FacadeFace::try_new(
        FaceNormal::exact(normal).unwrap(),
        (area, area),
        centroid,
        side,
    )
    .unwrap()
}

/// A window `south` whose 3 m² outer face looks south at (5, -0.3, 1.75)
/// over its top and bottom edges; `both` looks out both ways; `roof` up a
/// south pitch of 30°; `level` straight up; `curved` is a tessellation.
fn session(north: [f64; 2]) -> (Project, ServiceRegistry) {
    use FacadeFaceSide::{Outside, Partly};
    let model = ["south", "both", "roof", "level", "curved"]
        .iter()
        .fold(Model::default(), |model, local| {
            model.object(local, "window")
        })
        .object("block", "building")
        .object("tower", "building")
        .object("kiosk", "kiosk");
    let window = vec![
        face([0.0, -1.5, 0.0], 1.5, [4.5, -0.3, 1.5], Outside),
        face([0.0, -1.5, 0.0], 1.5, [5.5, -0.3, 2.0], Outside),
        face([0.0, 0.0, 0.6], 0.6, [5.0, -0.15, 2.5], Outside),
        face([0.0, 0.0, -0.6], 0.6, [5.0, -0.15, 1.0], Partly),
    ];
    let (sine, cosine) = (30.0_f64.to_radians().sin(), 30.0_f64.to_radians().cos());
    let faces = Faces(BTreeMap::from([
        (id("south"), (window.clone(), true)),
        (
            id("both"),
            (
                vec![
                    face([0.0, -1.0, 0.0], 3.0, [5.0, -0.3, 1.75], Outside),
                    face([0.0, 1.0, 0.0], 3.0, [5.0, 0.0, 1.75], Outside),
                ],
                true,
            ),
        ),
        (
            id("roof"),
            (
                vec![face([0.0, -sine, cosine], 20.0, [0.0; 3], Outside)],
                true,
            ),
        ),
        (
            id("level"),
            (vec![face([0.0, 0.0, 1.0], 20.0, [0.0; 3], Outside)], true),
        ),
        (id("curved"), (window, false)),
    ]));
    let elevations = Elevations(BTreeMap::from([
        (id("block"), 0.5_f64.atan()),
        (id("tower"), 0.2),
        (id("kiosk"), 0.6),
    ]));
    let mut services = ServiceRegistry::new();
    services
        .register(FacadeAreaServiceHandle::new(Arc::new(faces)))
        .unwrap();
    services
        .register(CoordinateSystemServiceHandle::new(Arc::new(North(
            north,
            vec![snapshot()],
        ))))
        .unwrap();
    services
        .register(SightServiceHandle::new(Arc::new(elevations)))
        .unwrap();
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// `name` of `object`: its bounds and whether its evidence is exact.
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
                PropertyValue::Quantity { value, .. } => Ok((value, value, exact)),
                PropertyValue::Measured { lower, upper, .. } => Ok((lower, upper, exact)),
                other => Err(format!("{other:?}")),
            }
        }
        Ok(PropertyResolution::Absent(_)) => Err("absent".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn holds((lower, upper, _): (f64, f64, bool), expected: f64) -> bool {
    lower <= expected && expected <= upper && upper - lower < 1e-6
}

#[test]
fn a_south_facing_window_has_an_azimuth_of_half_a_turn() {
    let north = session([0.0, 1.0]);
    let azimuth = value(&north, "south", "azimuth").unwrap();
    assert!(holds(azimuth, PI), "{azimuth:?}");
    assert!(azimuth.2);
    // With true north along +x, east lies along -y: the window faces east.
    let turned = session([1.0, 0.0]);
    assert!(holds(
        value(&turned, "south", "azimuth").unwrap(),
        FRAC_PI_2
    ));
    // Measured from the plan's y axis, the turn is ignored.
    assert!(holds(
        value(&turned, "south", "azimuth;reference=project_north").unwrap(),
        PI
    ));
    // A pitched roof faces down its slope; a flat one faces no direction.
    assert!(holds(value(&north, "roof", "azimuth").unwrap(), PI));
    let why = value(&north, "level", "azimuth").unwrap_err();
    assert!(why.contains("level"), "{why}");
}

#[test]
fn tilts_are_measured_from_straight_up() {
    let north = session([0.0, 1.0]);
    assert!(holds(value(&north, "south", "tilt").unwrap(), FRAC_PI_2));
    assert!(holds(
        value(&north, "roof", "tilt").unwrap(),
        30.0_f64.to_radians()
    ));
    assert!(holds(value(&north, "level", "tilt").unwrap(), 0.0));
}

#[test]
fn a_body_looking_out_both_ways_has_no_outward_side() {
    let north = session([0.0, 1.0]);
    for name in ["azimuth", "tilt", "obstruction_angle;obstacles=building"] {
        let why = value(&north, "both", name).unwrap_err();
        assert!(
            why.contains("which side faces out is undecided"),
            "{name}: {why}"
        );
    }
}

#[test]
fn the_obstruction_angle_is_the_highest_obstacle_in_front() {
    let north = session([0.0, 1.0]);
    let angle = value(&north, "south", "obstruction_angle;obstacles=building").unwrap();
    // A block 10 m up, 20 m ahead: 26.57°.
    assert!(holds(angle, 0.5_f64.atan()), "{angle:?}");
    assert!((angle.0.to_degrees() - 26.565).abs() < 1e-3);
    assert!(angle.2);
    let higher = value(
        &north,
        "south",
        "obstruction_angle;obstacles=building,kiosk;sector=60",
    )
    .unwrap();
    assert!(holds(higher, 0.6), "{higher:?}");
}

#[test]
fn an_orientation_measured_on_a_tessellation_is_inexact() {
    let north = session([0.0, 1.0]);
    for name in ["azimuth", "tilt"] {
        let (lower, upper, exact) = value(&north, "curved", name).unwrap();
        assert!(!exact, "{name}");
        assert!(lower <= upper);
    }
    let why = value(&north, "curved", "obstruction_angle;obstacles=building").unwrap_err();
    assert!(why.contains("tessellation"), "{why}");
}
