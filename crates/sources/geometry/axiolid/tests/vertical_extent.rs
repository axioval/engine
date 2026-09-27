//! Bottom and top elevations over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_axiolid::{AxiolidGeometry, AxiolidVerticalExtentService};
use axioval_engine::{MetricDirection, VerticalExtentError, VerticalExtentService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented 4 x 3 box from `z0` to `z1`.
fn slab(z0: f64, z1: f64) -> TriMesh {
    let points = vec![
        Point3::new(0.0, 0.0, z0),
        Point3::new(4.0, 0.0, z0),
        Point3::new(4.0, 3.0, z0),
        Point3::new(0.0, 3.0, z0),
        Point3::new(0.0, 0.0, z1),
        Point3::new(4.0, 0.0, z1),
        Point3::new(4.0, 3.0, z1),
        Point3::new(0.0, 3.0, z1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // floor, facing down
        4, 5, 6, 4, 6, 7, // ceiling, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn service(geometry: AxiolidGeometry) -> AxiolidVerticalExtentService {
    AxiolidVerticalExtentService::new(geometry)
}

#[test]
fn a_planar_slab_measures_its_bottom_and_top_exactly() {
    let extents = service(AxiolidGeometry::new().with_mesh(id("slab"), slab(2.8, 3.0)));
    let extent = extents.measure_vertical_extent(&id("slab")).unwrap();
    assert!(extent.is_exact());
    assert_eq!(extent.object(), &id("slab"));
    assert!((extent.bottom().lower_metres() - 2.8).abs() < 1e-12);
    assert!((extent.top().upper_metres() - 3.0).abs() < 1e-12);
    assert!(extent.bottom().is_exact() && extent.top().is_exact());
    assert_eq!(
        extent.evidence().locator,
        format!("vertical-extent:{}", id("slab"))
    );
}

/// A tessellation is never exact: each elevation widens by the chord
/// deviation, and the evidence says it is approximate.
#[test]
fn a_tessellated_slab_measures_intervals_with_approximate_evidence() {
    let extents =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("vault"), slab(2.8, 3.0), 0.01));
    let extent = extents.measure_vertical_extent(&id("vault")).unwrap();
    assert!(!extent.is_exact());
    assert!(!extent.evidence().exact);
    assert!((extent.top().lower_metres() - 2.99).abs() < 1e-12);
    assert!((extent.top().upper_metres() - 3.01).abs() < 1e-12);
    assert!((extent.bottom().lower_metres() - 2.79).abs() < 1e-12);

    // Even a declared zero deviation stays approximate.
    let zero =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("vault"), slab(2.8, 3.0), 0.0));
    let extent = zero.measure_vertical_extent(&id("vault")).unwrap();
    assert!(!extent.is_exact() && !extent.top().is_exact());
}

#[test]
fn bodiless_unmeasured_and_unknown_objects_have_no_extent() {
    let extents = service(
        AxiolidGeometry::new()
            .with_no_body(id("storey"))
            .with_unmeasured(id("broken"), "meshing failed"),
    );
    assert!(matches!(
        extents.measure_vertical_extent(&id("storey")),
        Err(VerticalExtentError::Unavailable(_))
    ));
    assert!(matches!(
        extents.measure_vertical_extent(&id("broken")),
        Err(VerticalExtentError::Unavailable(_))
    ));
    assert_eq!(
        extents.measure_vertical_extent(&id("missing")),
        Err(VerticalExtentError::UnknownObject(id("missing")))
    );
}

#[test]
fn an_invalid_declared_deviation_is_refused_not_exact() {
    let extents = service(AxiolidGeometry::new().with_tessellated_mesh(
        id("vault"),
        slab(0.0, 0.2),
        f64::NAN,
    ));
    assert_eq!(
        extents.measure_vertical_extent(&id("vault")),
        Err(VerticalExtentError::InvalidMeasurement)
    );
}

fn along(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

/// A closed 4 m long, `thickness` thick, 3 m high wall along the x axis,
/// turned by `angle` radians about the vertical through the origin.
fn wall(thickness: f64, angle: f64) -> TriMesh {
    let (sin, cos) = angle.sin_cos();
    let box_mesh = slab(0.0, 3.0);
    let points = (0..box_mesh.position_count())
        .map(|index| {
            let [x, y, z] = box_mesh.position(index).to_array();
            // Scale the 4 x 3 box to 4 x `thickness`, then turn it.
            let y = y / 3.0 * thickness;
            Point3::new(x * cos - y * sin, x * sin + y * cos, z)
        })
        .collect();
    let indices = (0..box_mesh.triangle_count())
        .flat_map(|index| box_mesh.triangle(index).map(|i| u32::try_from(i).unwrap()))
        .collect();
    TriMesh::new(points, indices)
}

#[test]
fn a_planar_wall_measures_its_thickness_along_an_axis_exactly() {
    let extents = service(AxiolidGeometry::new().with_mesh(id("wall"), wall(0.3, 0.0)));
    let extent = extents
        .measure_directional_extent(&id("wall"), along([0.0, 1.0, 0.0]))
        .unwrap();
    assert!(extent.is_exact());
    assert_eq!(extent.object(), &id("wall"));
    let (low, high) = extent.length_metres();
    assert!(
        low <= 0.3 && 0.3 <= high && high - low < 1e-15,
        "{low} {high}"
    );
    // Its length and its height, along the other axes.
    let (length, _) = extents
        .measure_directional_extent(&id("wall"), along([-1.0, 0.0, 0.0]))
        .unwrap()
        .length_metres();
    assert!((length - 4.0).abs() < 1e-12);
    let (height, _) = extents
        .measure_directional_extent(&id("wall"), along([0.0, 0.0, 1.0]))
        .unwrap()
        .length_metres();
    assert!((height - 3.0).abs() < 1e-12);
}

/// Along a direction off the coordinate axes the projection rounds, so it
/// is an interval holding the thickness, reported as approximate.
#[test]
fn a_turned_wall_measures_a_narrow_interval_around_its_thickness() {
    let angle = 0.5_f64;
    let extents = service(AxiolidGeometry::new().with_mesh(id("wall"), wall(0.3, angle)));
    let normal = along([-angle.sin(), angle.cos(), 0.0]);
    let extent = extents
        .measure_directional_extent(&id("wall"), normal)
        .unwrap();
    assert!(!extent.is_exact() && !extent.evidence().exact);
    let (low, high) = extent.length_metres();
    assert!(
        low < 0.3 && 0.3 < high && high - low < 1e-12,
        "{low} {high}"
    );
    assert!(extent.evidence().locator.starts_with("directional-extent:"));
}

#[test]
fn a_tessellated_wall_widens_its_extent_by_the_deviation() {
    let extents =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("wall"), wall(0.3, 0.0), 0.01));
    let extent = extents
        .measure_directional_extent(&id("wall"), along([0.0, 1.0, 0.0]))
        .unwrap();
    assert!(!extent.is_exact());
    let (low, high) = extent.length_metres();
    assert!(
        (low - 0.28).abs() < 1e-12 && (high - 0.32).abs() < 1e-12,
        "{low} {high}"
    );
}

/// One geometry set may hold several files; each extent cites its object's.
#[test]
fn a_directional_extent_cites_the_measured_objects_own_source() {
    let other = ObjectId::new(SourceId::new("cad", "structure").unwrap(), "wall").unwrap();
    let extents = service(AxiolidGeometry::new().with_mesh(other.clone(), wall(0.3, 0.0)));
    let extent = extents
        .measure_directional_extent(&other, along([0.0, 1.0, 0.0]))
        .unwrap();
    assert_eq!(extent.evidence().source, other.source);
}

#[test]
fn bodiless_and_unmeasured_objects_have_no_directional_extent() {
    let extents = service(
        AxiolidGeometry::new()
            .with_no_body(id("storey"))
            .with_unmeasured(id("broken"), "meshing failed"),
    );
    for object in ["storey", "broken"] {
        assert!(matches!(
            extents.measure_directional_extent(&id(object), along([0.0, 1.0, 0.0])),
            Err(VerticalExtentError::Unavailable(_))
        ));
    }
    assert_eq!(
        extents.measure_directional_extent(&id("missing"), along([0.0, 1.0, 0.0])),
        Err(VerticalExtentError::UnknownObject(id("missing")))
    );
}
