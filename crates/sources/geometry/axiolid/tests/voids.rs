//! Openings measured as the solids of their voids (#383).
//!
//! An opening occupies no material: every service iterating bodies, and the
//! proximity service's material view, keep refusing it as bodiless. Only
//! the view measuring voids, and the extents, answer for it by name.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService, AxiolidVerticalExtentService};
use axioval_engine::{
    BodyContainment, MetricDirection, ProximityError, ProximityRequest, ProximityService,
    ProximityServiceHandle, SurfaceFace, VerticalExtentError, VerticalExtentService,
};
use axioval_ir::{ObjectId, SourceId};
use std::sync::Arc;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

/// A wall (x 0..4, y 0..0.2, 3 m high) with an opening (x 1.5..2.5, the
/// wall's depth and 0.1 m either side, z 0..2.1), a duct passing through
/// the opening (x 1.9..2.1, y -0.5..0.7, z 1..1.2), a duct inside its void
/// (x 1.9..2.1, y 0..0.2, z 1..1.2) and a duct beside it, through the wall
/// (x 3.4..3.6).
fn provision() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid([0.0, 0.0, 0.0], [4.0, 0.2, 3.0]))
        .with_void(id("opening"), cuboid([1.5, -0.1, 0.0], [2.5, 0.3, 2.1]))
        .with_mesh(id("through"), cuboid([1.9, -0.5, 1.0], [2.1, 0.7, 1.2]))
        .with_mesh(id("inside"), cuboid([1.9, 0.0, 1.0], [2.1, 0.2, 1.2]))
        .with_mesh(id("beside"), cuboid([3.4, -0.5, 1.0], [3.6, 0.7, 1.2]))
}

fn request(subject: &str, counterpart: &str) -> ProximityRequest {
    ProximityRequest::try_new(id(subject), id(counterpart)).unwrap()
}

/// The material view is unchanged: an opening occupies no material, so a
/// clash or a contact never pairs it.
#[test]
fn an_opening_stays_bodiless_to_the_material_view() {
    let geometry = provision();
    assert!(geometry.has_no_body(&id("opening")));
    assert!(geometry.mesh(&id("opening")).is_none());
    assert_eq!(geometry.measured_voids(), 1);
    let material = AxiolidProximityService::new(geometry);
    assert_eq!(
        material.bounds(&id("opening")).unwrap_err(),
        ProximityError::NoBody
    );
    assert_eq!(
        material
            .measure_proximity(&request("through", "opening"))
            .unwrap_err(),
        ProximityError::NoBody
    );
}

/// The view measuring voids measures the opening as the solid of its void:
/// the duct through it meets it, the one beside it is 0.9 m away, and
/// every evidence notes the void.
#[test]
fn a_distance_to_an_opening_is_measured_to_its_void() {
    let voids = AxiolidProximityService::new(provision()).measuring_voids();
    let through = voids
        .measure_distance(&request("through", "opening"))
        .unwrap();
    assert_eq!(through.interval_metres(), (0.0, 0.0));
    assert!(through.evidence().exact);
    assert!(
        through
            .evidence()
            .locator
            .ends_with(&format!(";void:{}", id("opening"))),
        "{}",
        through.evidence().locator
    );
    let beside = voids
        .measure_distance(&request("opening", "beside"))
        .unwrap();
    let (lower, upper) = beside.interval_metres();
    assert!((lower - 0.9).abs() < 1e-9 && (upper - 0.9).abs() < 1e-9);
    let bounds = voids.bounds(&id("opening")).unwrap();
    let min = bounds.enclosing().min();
    assert!(
        min.iter()
            .zip([1.5, -0.1, 0.0])
            .all(|(got, want)| (got - want).abs() < 1e-12),
        "{min:?}"
    );
    // The wall's own material is measured as before.
    let wall = voids.measure_distance(&request("beside", "wall")).unwrap();
    assert_eq!(wall.interval_metres(), (0.0, 0.0));
}

/// A duct within the void is contained in it, and the void's volume is
/// the opening's.
#[test]
fn a_duct_is_contained_in_an_openings_void() {
    let voids = AxiolidProximityService::new(provision()).measuring_voids();
    let measured = voids
        .measure_proximity(&request("inside", "opening"))
        .unwrap();
    assert_eq!(
        measured.containment(),
        Some(BodyContainment::SubjectInsideCounterpart)
    );
    let volume = voids.measure_body_volume(&id("opening")).unwrap();
    let expected = 1.0 * 0.4 * 2.1;
    assert!((volume.volume().lower_cubic_metres() - expected).abs() < 1e-9);
    assert!((volume.volume().upper_cubic_metres() - expected).abs() < 1e-9);
}

/// A void the host could not mesh is unmeasured, never an object without
/// a void; a bodiless object without a void stays bodiless.
#[test]
fn an_unmeasured_void_fails_closed() {
    let geometry = provision()
        .with_unmeasured_void(id("refused"), "no body representation")
        .with_no_body(id("storey"));
    let voids = AxiolidProximityService::new(geometry.clone()).measuring_voids();
    assert_eq!(
        voids.bounds(&id("refused")).unwrap_err(),
        ProximityError::Unavailable
    );
    assert_eq!(
        voids.bounds(&id("storey")).unwrap_err(),
        ProximityError::NoBody
    );
    let extents = AxiolidVerticalExtentService::new(geometry);
    let refused = extents.measure_vertical_extent(&id("refused")).unwrap_err();
    assert!(
        matches!(&refused, VerticalExtentError::Unavailable(reason)
            if reason.contains("void that was not measured: no body representation")),
        "{refused:?}"
    );
}

/// The handle measures material unless a rule asks for the view measuring
/// voids; a host registering none leaves voids bodiless there too.
#[test]
fn the_handle_measures_voids_only_through_its_void_view() {
    let material = AxiolidProximityService::new(provision());
    let voids = material.measuring_voids();
    let handle = ProximityServiceHandle::new(Arc::new(material)).with_voids(Arc::new(voids));
    assert_eq!(
        handle.bounds(&id("opening")).unwrap_err(),
        ProximityError::NoBody
    );
    assert!(handle.measuring_voids().bounds(&id("opening")).is_ok());
    let plain = ProximityServiceHandle::new(Arc::new(AxiolidProximityService::new(provision())));
    assert_eq!(
        plain.measuring_voids().bounds(&id("opening")).unwrap_err(),
        ProximityError::NoBody
    );
}

/// An opening's height and width are its void's; its faces describe no
/// material and stay refused.
#[test]
fn an_openings_extent_is_its_voids() {
    let extents = AxiolidVerticalExtentService::new(provision());
    let extent = extents.measure_vertical_extent(&id("opening")).unwrap();
    assert!(extent.is_exact());
    assert_eq!(extent.height_metres(), (2.1, 2.1));
    assert!(
        extent
            .evidence()
            .locator
            .ends_with(&format!(";void:{}", id("opening")))
    );
    let along = extents
        .measure_directional_extent(
            &id("opening"),
            MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        )
        .unwrap();
    assert_eq!(along.length_metres(), (1.0, 1.0));
    assert!(
        extents
            .measure_face_normals(&id("opening"), SurfaceFace::Top)
            .is_err()
    );
}

/// A tessellated void is measured within its declared deviation, never as
/// exact.
#[test]
fn a_tessellated_void_is_approximate() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_void(id("round"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]), 0.001)
        .with_mesh(id("duct"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]));
    let voids = AxiolidProximityService::new(geometry.clone()).measuring_voids();
    let measured = voids.measure_distance(&request("duct", "round")).unwrap();
    let (lower, upper) = measured.interval_metres();
    assert!((lower - 0.999).abs() < 1e-9 && (upper - 1.001).abs() < 1e-9);
    assert!(!measured.evidence().exact);
    let extent = AxiolidVerticalExtentService::new(geometry)
        .measure_vertical_extent(&id("round"))
        .unwrap();
    assert!(!extent.is_exact());
}
