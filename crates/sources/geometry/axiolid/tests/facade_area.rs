//! Facade areas over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFacadeAreaService, AxiolidGeometry};
use axioval_engine::{FacadeAreaError, FacadeAreaService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// The points and outward-oriented triangles of a closed box, with indices
/// starting at `base`.
fn shell(min: [f64; 3], max: [f64; 3], base: u32) -> (Vec<Point3>, Vec<u32>) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let points = vec![
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
        0, 2, 1, 0, 3, 2, // floor, facing down
        4, 5, 6, 4, 6, 7, // ceiling, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ]
    .map(|index| index + base)
    .to_vec();
    (points, indices)
}

/// One mesh of closed boxes, each `(min, max)`.
fn boxes(parts: &[([f64; 3], [f64; 3])]) -> TriMesh {
    let (mut points, mut indices) = (Vec::new(), Vec::new());
    for (min, max) in parts {
        let base = u32::try_from(points.len()).unwrap();
        let (more, faces) = shell(*min, *max, base);
        points.extend(more);
        indices.extend(faces);
    }
    TriMesh::new(points, indices)
}

fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    boxes(&[(min, max)])
}

/// A 10 m long, 0.3 m thick, 3 m high wall south of a 10 x 4 m room.
fn wall() -> TriMesh {
    cuboid([0.0, -0.3, 0.0], [10.0, 0.0, 3.0])
}

fn room() -> TriMesh {
    cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 3.0])
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

#[test]
fn a_wall_facing_a_room_counts_its_outer_face_and_free_ends() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    // The 10 x 3 outer face and the two 0.3 x 3 ends; the inner face is
    // flush with the room, and the top and bottom are not steep.
    assert!(area.is_exact());
    assert_eq!(area.object(), &id("wall"));
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");
    assert!(area.evidence().locator.starts_with("facade-area:"));
}

#[test]
fn only_a_declared_space_makes_a_face_interior() {
    // Undeclared, the room is just another body flush with the inner face.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source());
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");

    // A room set back 0.2 m from the wall is still what the inner face
    // looks into; without it, the inner face looks outside.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("room"), cuboid([0.0, 0.2, 0.0], [10.0, 4.0, 3.0]));
    let declared = AxiolidFacadeAreaService::new(geometry.clone(), source()).with_space(id("room"));
    let area = declared.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");
    let alone = AxiolidFacadeAreaService::new(
        AxiolidGeometry::new().with_mesh(id("wall"), wall()),
        source(),
    );
    let area = alone.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 61.8), "{area:?}");
}

#[test]
fn a_window_between_wall_panels_covers_their_ends() {
    // Panels x 0..4 and 6..10, a window x 4..6 filling the gap.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("west"), cuboid([0.0, -0.3, 0.0], [4.0, 0.0, 3.0]))
        .with_mesh(id("east"), cuboid([6.0, -0.3, 0.0], [10.0, 0.0, 3.0]))
        .with_mesh(id("window"), cuboid([4.0, -0.3, 0.0], [6.0, 0.0, 3.0]))
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let west = service.measure_facade_area(&id("west")).unwrap();
    assert!(close(west.lower_square_metres(), 12.9), "{west:?}");
    let window = service.measure_facade_area(&id("window")).unwrap();
    assert!(close(window.lower_square_metres(), 6.0), "{window:?}");
}

#[test]
fn a_reveal_facing_its_own_body_is_not_facade() {
    // One wall with a 0.8 m full-height gap: the reveals face each other.
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("wall"),
            boxes(&[
                ([0.0, -0.3, 0.0], [4.0, 0.0, 3.0]),
                ([4.8, -0.3, 0.0], [8.8, 0.0, 3.0]),
            ]),
        )
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [8.8, 4.0, 3.0]));
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 25.8), "{area:?}");
}

#[test]
fn a_body_in_front_of_a_face_does_not_make_it_interior() {
    // Another wall 0.5 m in front of this one, across a narrow courtyard:
    // every probe from the outer face meets it first.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("opposite"), cuboid([0.0, -1.0, 0.0], [10.0, -0.8, 3.0]))
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");
}

#[test]
fn a_partly_covered_face_widens_the_interval_instead_of_being_guessed() {
    // An annex against the first 2 m of the outer face. The face is two
    // triangles; one is uncovered at every sample, the other only at some.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("annex"), cuboid([0.0, -2.0, 0.0], [2.0, -0.3, 3.0]))
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(!area.is_exact());
    assert!(close(area.lower_square_metres(), 16.8), "{area:?}");
    assert!(close(area.upper_square_metres(), 31.8), "{area:?}");
    assert!(area.evidence().locator.ends_with(":partial=1"), "{area:?}");
}

#[test]
fn a_tessellated_wall_measures_an_interval_around_its_mesh() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("wall"), wall(), 0.001)
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(!area.is_exact() && !area.evidence().exact);
    assert!(area.lower_square_metres() < 31.8 && 31.8 < area.upper_square_metres());

    // A declared zero deviation still never measures a point.
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("wall"), wall(), 0.0)
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry, source()).with_space(id("room"));
    assert!(!service.measure_facade_area(&id("wall")).unwrap().is_exact());
}

#[test]
fn a_tessellation_within_reach_refuses_and_one_beyond_it_does_not() {
    let near = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_tessellated_mesh(id("room"), room(), 0.001);
    let service = AxiolidFacadeAreaService::new(near, source()).with_space(id("room"));
    assert!(matches!(
        service.measure_facade_area(&id("wall")),
        Err(FacadeAreaError::Unavailable(_))
    ));

    let far = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_tessellated_mesh(
            id("vault"),
            cuboid([20.0, 0.0, 0.0], [22.0, 2.0, 3.0]),
            0.001,
        );
    let service = AxiolidFacadeAreaService::new(far, source());
    assert!(service.measure_facade_area(&id("wall")).is_ok());
}

#[test]
fn unknown_bodiless_and_unmeasured_objects_refuse() {
    let base = || AxiolidGeometry::new().with_mesh(id("wall"), wall());
    let service = AxiolidFacadeAreaService::new(base(), source());
    assert_eq!(
        service.measure_facade_area(&id("ghost")),
        Err(FacadeAreaError::UnknownObject(id("ghost")))
    );

    let service = AxiolidFacadeAreaService::new(base().with_no_body(id("storey")), source());
    assert!(matches!(
        service.measure_facade_area(&id("storey")),
        Err(FacadeAreaError::Unavailable(_))
    ));

    // An unmeasured body could stand against any face.
    let service =
        AxiolidFacadeAreaService::new(base().with_unmeasured(id("pipe"), "no mesh"), source());
    assert!(matches!(
        service.measure_facade_area(&id("wall")),
        Err(FacadeAreaError::Unavailable(_))
    ));

    // So could a declared space without a body.
    let service = AxiolidFacadeAreaService::new(base().with_no_body(id("room")), source())
        .with_space(id("room"));
    assert!(matches!(
        service.measure_facade_area(&id("wall")),
        Err(FacadeAreaError::Unavailable(_))
    ));
}
