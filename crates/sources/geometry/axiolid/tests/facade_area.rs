//! Facade areas over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFacadeAreaService, AxiolidGeometry};
use axioval_engine::{FacadeAreaError, FacadeAreaService, FacadeFaceSide};
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(geometry);
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");

    // A room set back 0.2 m from the wall is still what the inner face
    // looks into; without it, the inner face looks outside.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("room"), cuboid([0.0, 0.2, 0.0], [10.0, 4.0, 3.0]));
    let declared = AxiolidFacadeAreaService::new(geometry.clone()).with_space(id("room"));
    let area = declared.measure_facade_area(&id("wall")).unwrap();
    assert!(close(area.lower_square_metres(), 31.8), "{area:?}");
    let alone = AxiolidFacadeAreaService::new(AxiolidGeometry::new().with_mesh(id("wall"), wall()));
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
    let area = service.measure_facade_area(&id("wall")).unwrap();
    assert!(!area.is_exact() && !area.evidence().exact);
    assert!(area.lower_square_metres() < 31.8 && 31.8 < area.upper_square_metres());

    // A declared zero deviation still never measures a point.
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("wall"), wall(), 0.0)
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
    assert!(!service.measure_facade_area(&id("wall")).unwrap().is_exact());
}

#[test]
fn a_tessellation_within_reach_refuses_and_one_beyond_it_does_not() {
    let near = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_tessellated_mesh(id("room"), room(), 0.001);
    let service = AxiolidFacadeAreaService::new(near).with_space(id("room"));
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
    let service = AxiolidFacadeAreaService::new(far);
    assert!(service.measure_facade_area(&id("wall")).is_ok());
}

#[test]
fn unknown_bodiless_and_unmeasured_objects_refuse() {
    let base = || AxiolidGeometry::new().with_mesh(id("wall"), wall());
    let service = AxiolidFacadeAreaService::new(base());
    assert_eq!(
        service.measure_facade_area(&id("ghost")),
        Err(FacadeAreaError::UnknownObject(id("ghost")))
    );

    let service = AxiolidFacadeAreaService::new(base().with_no_body(id("storey")));
    assert!(matches!(
        service.measure_facade_area(&id("storey")),
        Err(FacadeAreaError::Unavailable(_))
    ));

    // An unmeasured body could stand against any face.
    let service = AxiolidFacadeAreaService::new(base().with_unmeasured(id("pipe"), "no mesh"));
    assert!(matches!(
        service.measure_facade_area(&id("wall")),
        Err(FacadeAreaError::Unavailable(_))
    ));

    // So could a declared space without a body.
    let service =
        AxiolidFacadeAreaService::new(base().with_no_body(id("room"))).with_space(id("room"));
    assert!(matches!(
        service.measure_facade_area(&id("wall")),
        Err(FacadeAreaError::Unavailable(_))
    ));
}

#[test]
fn a_face_area_is_the_largest_plane_face() {
    // One wall modelled as two boxes: the coplanar sides join into one
    // 10 x 3 face, and the butt joint's faces stay two small ones.
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("wall"),
            boxes(&[
                ([0.0, -0.3, 0.0], [4.0, 0.0, 3.0]),
                ([4.0, -0.3, 0.0], [10.0, 0.0, 3.0]),
            ]),
        )
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [10.0, 4.0, 3.25]))
        .with_tessellated_mesh(id("vault"), room(), 0.001)
        .with_no_body(id("storey"));
    let service = AxiolidFacadeAreaService::new(geometry);
    let area = service.measure_face_area(&id("wall")).unwrap();
    assert!(area.is_exact() && area.evidence().exact);
    assert!(close(area.lower_square_metres(), 30.0), "{area:?}");
    assert!(area.evidence().locator.starts_with("face-area:"));
    let area = service.measure_face_area(&id("slab")).unwrap();
    assert!(close(area.upper_square_metres(), 40.0), "{area:?}");

    // A tessellated body's planes are chords, never certified faces.
    assert!(matches!(
        service.measure_face_area(&id("vault")),
        Err(FacadeAreaError::Unavailable(_))
    ));
    assert!(matches!(
        service.measure_face_area(&id("storey")),
        Err(FacadeAreaError::Unavailable(_))
    ));
    assert_eq!(
        service.measure_face_area(&id("ghost")),
        Err(FacadeAreaError::UnknownObject(id("ghost")))
    );
}

#[test]
fn a_window_before_a_room_lists_one_face_looking_out() {
    // A 2 x 1.5 m window in the wall's plane, the room behind it.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("window"), cuboid([4.0, -0.3, 1.0], [6.0, 0.0, 2.5]))
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
    let faces = service.measure_facade_faces(&id("window")).unwrap();
    assert!(faces.evidence().exact);
    assert!(faces.evidence().locator.starts_with("facade-faces:"));
    // Its outer face looks south; its sides, top and bottom look outside
    // too, since nothing holds them.
    let south: Vec<_> = faces
        .faces()
        .iter()
        .filter(|face| face.normal().upper()[1] < 0.0)
        .collect();
    assert_eq!(south.len(), 2, "{faces:?}");
    let area: f64 = south.iter().map(|face| face.area_square_metres().0).sum();
    assert!(close(area, 3.0), "{area}");
    assert!(
        south
            .iter()
            .all(|face| face.side() == FacadeFaceSide::Outside && close(face.centroid()[1], -0.3))
    );
    assert!(
        faces
            .faces()
            .iter()
            .all(|face| face.normal().lower()[1] <= 0.0),
        "the face against the room looks into it: {faces:?}"
    );

    // Standing alone, its inner face looks outside as well.
    let alone = AxiolidFacadeAreaService::new(
        AxiolidGeometry::new().with_mesh(id("window"), cuboid([4.0, -0.3, 1.0], [6.0, 0.0, 2.5])),
    );
    let faces = alone.measure_facade_faces(&id("window")).unwrap();
    assert!(
        faces
            .faces()
            .iter()
            .any(|face| face.normal().lower()[1] > 0.0)
    );
}

#[test]
fn a_roof_looks_up_and_a_slab_under_a_room_does_not() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("floor"), cuboid([0.0, 0.0, -0.3], [10.0, 4.0, 0.0]))
        .with_mesh(id("roof"), cuboid([0.0, 0.0, 3.0], [10.0, 4.0, 3.3]))
        .with_mesh(id("room"), room());
    let service = AxiolidFacadeAreaService::new(geometry).with_space(id("room"));
    let up = |object: &str| -> f64 {
        let faces = service.measure_facade_faces(&id(object)).unwrap();
        faces
            .faces()
            .iter()
            .filter(|face| face.normal().lower()[2] > 0.0)
            .map(|face| face.area_square_metres().0)
            .sum()
    };
    assert!(close(up("roof"), 40.0));
    assert!(close(up("floor"), 0.0));
    // The facade area still counts steep faces only.
    let roof = service.measure_facade_area(&id("roof")).unwrap();
    assert!(close(roof.lower_square_metres(), 8.4), "{roof:?}");
}

/// A box building 10 x 8 m in plan and 3.6 m high: four walls 0.3 m thick
/// (the south and north ones running the whole length), a floor slab and a
/// roof slab 0.3 m thick, a full-height 2 m window between the south wall's
/// two panels and the room inside.
fn building() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("south"), cuboid([0.0, 0.0, 0.0], [2.0, 0.3, 3.0]))
        .with_mesh(id("window"), cuboid([2.0, 0.0, 0.0], [4.0, 0.3, 3.0]))
        .with_mesh(id("south-east"), cuboid([4.0, 0.0, 0.0], [10.0, 0.3, 3.0]))
        .with_mesh(id("north"), cuboid([0.0, 7.7, 0.0], [10.0, 8.0, 3.0]))
        .with_mesh(id("west"), cuboid([0.0, 0.3, 0.0], [0.3, 7.7, 3.0]))
        .with_mesh(id("east"), cuboid([9.7, 0.3, 0.0], [10.0, 7.7, 3.0]))
        .with_mesh(id("floor"), cuboid([0.0, 0.0, -0.3], [10.0, 8.0, 0.0]))
        .with_mesh(id("roof"), cuboid([0.0, 0.0, 3.0], [10.0, 8.0, 3.3]))
        .with_mesh(id("room"), cuboid([0.3, 0.3, 0.0], [9.7, 7.7, 3.0]))
}

#[test]
fn the_outward_faces_of_a_building_enclose_its_gross_volume() {
    let service = AxiolidFacadeAreaService::new(building()).with_space(id("room"));
    let all: Vec<ObjectId> = [
        "south",
        "window",
        "south-east",
        "north",
        "west",
        "east",
        "floor",
        "roof",
    ]
    .map(id)
    .to_vec();
    let volume = service.measure_enclosed_volume(&all).unwrap();
    let (lower, upper) = volume.cubic_metres();
    assert!(
        lower <= 288.0 && 288.0 <= upper && upper - lower < 1e-4,
        "{lower} {upper}"
    );
    assert!(volume.evidence().exact);
    assert_eq!(volume.objects().len(), 8);
    // Without the roof, or the window, the faces do not close.
    for left_out in ["roof", "window"] {
        let some: Vec<ObjectId> = all
            .iter()
            .filter(|object| object.local_id != left_out)
            .cloned()
            .collect();
        let why = service.measure_enclosed_volume(&some).unwrap_err();
        assert!(
            why.to_string().contains("do not close"),
            "{left_out}: {why}"
        );
    }
}

/// A wall `min` to `max`, its thickness along y, with a rectangular hole
/// through it over `x` and `z`: one closed mesh wound outward.
fn holed(min: [f64; 3], max: [f64; 3], x: [f64; 2], z: [f64; 2]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let mut points = Vec::new();
    for y in [y0, y1] {
        for [px, pz] in [[x0, z0], [x1, z0], [x1, z1], [x0, z1]] {
            points.push(Point3::new(px, y, pz));
        }
        for [px, pz] in [[x[0], z[0]], [x[1], z[0]], [x[1], z[1]], [x[0], z[1]]] {
            points.push(Point3::new(px, y, pz));
        }
    }
    // Front (y0) corners: outer 0..4, hole 4..8; the back's eight on.
    let (outer, hole) = (|i: u32| i % 4, |i: u32| 4 + i % 4);
    let mut indices = Vec::new();
    for i in 0..4 {
        let j = i + 1;
        // The front's frame piece along edge i, facing -y, and the back's.
        indices.extend([outer(i), outer(j), hole(j), outer(i), hole(j), hole(i)]);
        indices.extend([
            8 + outer(i),
            8 + hole(j),
            8 + outer(j),
            8 + outer(i),
            8 + hole(i),
            8 + hole(j),
        ]);
        // The outer side and the reveal along edge i.
        indices.extend([
            outer(i),
            8 + outer(i),
            8 + outer(j),
            outer(i),
            8 + outer(j),
            outer(j),
        ]);
        indices.extend([
            hole(j),
            8 + hole(j),
            8 + hole(i),
            hole(j),
            8 + hole(i),
            hole(i),
        ]);
    }
    TriMesh::new(points, indices)
}

/// [`building`] with the south wall one piece holed over `x` and z 1..2.5
/// and, when `window` is set, a window 0.1 m thick filling the hole 0.1 m
/// back from the wall's outer face.
fn recessed_building(x: [f64; 2], window: bool) -> AxiolidGeometry {
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("south"),
            holed([0.0, 0.0, 0.0], [10.0, 0.3, 3.0], x, [1.0, 2.5]),
        )
        .with_mesh(id("north"), cuboid([0.0, 7.7, 0.0], [10.0, 8.0, 3.0]))
        .with_mesh(id("west"), cuboid([0.0, 0.3, 0.0], [0.3, 7.7, 3.0]))
        .with_mesh(id("east"), cuboid([9.7, 0.3, 0.0], [10.0, 7.7, 3.0]))
        .with_mesh(id("floor"), cuboid([0.0, 0.0, -0.3], [10.0, 8.0, 0.0]))
        .with_mesh(id("roof"), cuboid([0.0, 0.0, 3.0], [10.0, 8.0, 3.3]))
        .with_mesh(id("room"), cuboid([0.3, 0.3, 0.0], [9.7, 7.7, 3.0]));
    if window {
        geometry.with_mesh(id("window"), cuboid([x[0], 0.1, 1.0], [x[1], 0.2, 2.5]))
    } else {
        geometry
    }
}

/// The envelope of [`recessed_building`] less `left_out`.
fn recessed_envelope(left_out: &[&str]) -> Vec<ObjectId> {
    ["south", "window", "north", "west", "east", "floor", "roof"]
        .into_iter()
        .filter(|name| !left_out.contains(name))
        .map(id)
        .collect()
}

#[test]
fn reveals_before_a_window_set_back_close_the_envelope() {
    // engine#362: a 2 m wide window, whose reveals look across more than
    // the probes' reach, and a 0.6 m wide one. Either way the envelope
    // closes over the 0.1 m deep recess in front of the window and leaves
    // out the reveal behind it: 288 m³ less 0.1 m times the window's face.
    for (x, recess) in [([2.0, 4.0], 0.3), ([2.0, 2.6], 0.09)] {
        let service =
            AxiolidFacadeAreaService::new(recessed_building(x, true)).with_space(id("room"));
        let volume = service
            .measure_enclosed_volume(&recessed_envelope(&[]))
            .unwrap();
        let (lower, upper) = volume.cubic_metres();
        let expected = 288.0 - recess;
        assert!(
            lower <= expected && expected <= upper && upper - lower < 1e-4,
            "{x:?}: {lower} {upper}"
        );
        assert!(volume.evidence().exact);
        assert!(
            volume.evidence().locator.contains(":reveal-faces="),
            "{volume:?}"
        );
        // The reveals are no outward faces: the wall's are its outer face
        // less the hole and its two ends.
        let faces = service.measure_facade_faces(&id("south")).unwrap();
        let area: f64 = faces
            .faces()
            .iter()
            .map(|face| face.area_square_metres().0)
            .sum();
        let hole = (x[1] - x[0]) * 1.5;
        assert!(close(area, 30.0 - hole + 2.0 * 0.9), "{x:?}: {area}");
        assert!(
            faces
                .faces()
                .iter()
                .all(|face| face.side() == FacadeFaceSide::Outside)
        );
    }
}

#[test]
fn an_envelope_open_at_a_wall_or_a_hole_still_refuses() {
    let service =
        AxiolidFacadeAreaService::new(recessed_building([2.0, 4.0], true)).with_space(id("room"));
    let why = service
        .measure_enclosed_volume(&recessed_envelope(&["east"]))
        .unwrap_err();
    assert!(why.to_string().contains("do not close"), "{why}");
    // A hole with no window: its reveals run from the outer face to the
    // room and close nothing.
    let service =
        AxiolidFacadeAreaService::new(recessed_building([2.0, 4.0], false)).with_space(id("room"));
    let why = service
        .measure_enclosed_volume(&recessed_envelope(&["window"]))
        .unwrap_err();
    assert!(why.to_string().contains("do not close"), "{why}");
}
