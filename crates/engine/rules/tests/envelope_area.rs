//! The building envelope as a whole: `envelope_area` by compass
//! direction and element kind, `envelope_volume` and `compactness` over
//! the outward faces the facade-area service lists.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CoordinateSystemError, CoordinateSystemService,
    CoordinateSystemServiceHandle, EnclosedVolume, FacadeArea, FacadeAreaError, FacadeAreaService,
    FacadeAreaServiceHandle, FacadeFace, FacadeFaceSide, FacadeFaces, FaceNormal,
    PropertyResolution, ServiceRegistry, SourceCoordinateSystem, SourceSnapshot, measured_value,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::runtime::snapshot;
use common::{Model, id, source};

/// The outward faces of each object, whether they are of its exact shape,
/// and the volume every listed object's faces enclose together.
struct Envelope {
    faces: BTreeMap<ObjectId, (Vec<FacadeFace>, bool)>,
    volume: f64,
}

impl FacadeAreaService for Envelope {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        Err(FacadeAreaError::UnknownObject(object.clone()))
    }

    fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let (faces, exact) = self
            .faces
            .get(object)
            .cloned()
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("faces:{object}"));
        evidence.exact = exact;
        FacadeFaces::try_new(object.clone(), faces, evidence)
    }

    fn measure_enclosed_volume(
        &self,
        objects: &[ObjectId],
    ) -> Result<EnclosedVolume, FacadeAreaError> {
        if objects.len() != self.faces.len() {
            return Err(FacadeAreaError::Unavailable(
                "the faces looking outside do not close".into(),
            ));
        }
        let exact = objects.iter().all(|object| self.faces[object].1);
        let mut evidence = Evidence::exact(source(), "volume");
        evidence.exact = exact;
        EnclosedVolume::try_new(objects.to_vec(), self.volume, self.volume, evidence)
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

fn face(normal: [f64; 3], area: f64, side: FacadeFaceSide) -> FacadeFace {
    FacadeFace::try_new(
        FaceNormal::exact(normal).unwrap(),
        (area, area),
        [0.0; 3],
        side,
    )
    .unwrap()
}

/// A box 10 x 8 m in plan and 4 m high: walls `south`, `east`, `north`
/// and `west` (their outer faces net of the windows), slabs `floor` and
/// `roof` (their edges 0.5 m high), windows `s1` and `s2` of 3 m² facing
/// south and `e1` facing east. `exact` is whether the faces are of the
/// exact shape; `partly` leaves a third of the roof's top only partly
/// outside.
fn session(exact: bool, partly: bool) -> (Project, ServiceRegistry) {
    use FacadeFaceSide::{Outside, Partly};
    let model = ["south", "east", "north", "west"]
        .iter()
        .fold(Model::default(), |model, local| model.object(local, "wall"))
        .object("floor", "slab")
        .object("roof", "slab")
        .object("s1", "window")
        .object("s2", "window")
        .object("e1", "window")
        .object("room", "space");
    let [s, e, n, w, up, down] = [
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
    ];
    let slab = |cap: [f64; 3]| {
        vec![
            face(cap, 80.0, Outside),
            face(s, 5.0, Outside),
            face(e, 4.0, Outside),
            face(n, 5.0, Outside),
            face(w, 4.0, Outside),
        ]
    };
    let mut roof = slab(up);
    if partly {
        roof[0] = face(up, 50.0, Outside);
        roof.push(face(up, 30.0, Partly));
    }
    let faces = BTreeMap::from([
        (id("south"), (vec![face(s, 24.0, Outside)], exact)),
        (id("east"), (vec![face(e, 21.0, Outside)], exact)),
        (id("north"), (vec![face(n, 30.0, Outside)], exact)),
        (id("west"), (vec![face(w, 24.0, Outside)], exact)),
        (id("floor"), (slab(down), exact)),
        (id("roof"), (roof, exact)),
        (id("s1"), (vec![face(s, 3.0, Outside)], exact)),
        (id("s2"), (vec![face(s, 3.0, Outside)], exact)),
        (id("e1"), (vec![face(e, 3.0, Outside)], exact)),
    ]);
    let mut services = ServiceRegistry::new();
    services
        .register(FacadeAreaServiceHandle::new(Arc::new(Envelope {
            faces,
            volume: 320.0,
        })))
        .unwrap();
    services
        .register(CoordinateSystemServiceHandle::new(Arc::new(North(
            [0.0, 1.0],
            vec![snapshot()],
        ))))
        .unwrap();
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// `name` of the project, read on the room: its bounds and whether its
/// evidence is exact.
fn value(
    (project, services): &(Project, ServiceRegistry),
    name: &str,
) -> Result<(f64, f64, bool), String> {
    match measured_value(services, project, &id("room"), name) {
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
    lower <= expected && expected <= upper && upper - lower < 1e-9 * expected.max(1.0)
}

#[test]
fn envelope_areas_are_summed_by_direction_and_kind() {
    let building = session(true, false);
    let every = "elements=wall,slab,window";
    // 2 x 80 m² of slabs, 2 x 40 m² facing south and north, 2 x 32 m²
    // facing east and west.
    let whole = value(&building, &format!("envelope_area;{every}")).unwrap();
    assert!(holds(whole, 304.0), "{whole:?}");
    assert!(whole.2);
    for (toward, expected) in [
        ("south", 40.0),
        ("east", 32.0),
        ("north", 40.0),
        ("west", 32.0),
    ] {
        let side = value(&building, &format!("envelope_area;{every};toward={toward}")).unwrap();
        assert!(holds(side, expected), "{toward}: {side:?}");
    }
    for (toward, expected) in [("south", 6.0), ("east", 3.0), ("north", 0.0), ("west", 0.0)] {
        let windows = value(
            &building,
            &format!("envelope_area;elements=window;toward={toward}"),
        )
        .unwrap();
        assert!(holds(windows, expected), "{toward}: {windows:?}");
    }
    let walls = value(&building, "envelope_area;elements=wall;toward=south").unwrap();
    assert!(holds(walls, 24.0), "{walls:?}");
}

#[test]
fn the_volume_and_compactness_of_the_envelope_are_measured() {
    let building = session(true, false);
    let every = "elements=wall,slab,window";
    let volume = value(&building, &format!("envelope_volume;{every}")).unwrap();
    assert!(holds(volume, 320.0), "{volume:?}");
    let ratio = value(&building, &format!("compactness;{every}")).unwrap();
    assert!(holds(ratio, 0.95), "{ratio:?}");
    assert!(ratio.2);
    // Without the windows, the faces do not close.
    let why = value(&building, "envelope_volume;elements=wall,slab").unwrap_err();
    assert!(why.contains("do not close"), "{why}");
}

#[test]
fn a_partly_outward_face_adds_only_to_the_most() {
    let building = session(true, true);
    let (lower, upper, exact) =
        value(&building, "envelope_area;elements=wall,slab,window").unwrap();
    assert!(
        lower <= 274.0 && (274.0 - lower) < 1e-9 && upper >= 304.0 && upper - 304.0 < 1e-9,
        "{lower} {upper}"
    );
    assert!(!exact);
}

#[test]
fn an_envelope_measured_on_a_tessellation_is_inexact() {
    let building = session(false, false);
    for name in ["envelope_area", "envelope_volume", "compactness"] {
        let (lower, upper, exact) =
            value(&building, &format!("{name};elements=wall,slab,window")).unwrap();
        assert!(!exact, "{name}");
        assert!(lower <= upper);
    }
}
