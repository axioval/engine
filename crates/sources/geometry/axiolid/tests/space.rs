//! Space validation measured from real geometry, one aspect at a time.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidSpaceService};
use axioval_engine::{Cap, Containment, SpaceError, SpaceService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A box spanning `x0..x1`, `y0..y1`, `z0..z1`.
fn body(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
        vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
    )
}

/// Clear height comes from the space's own vertical extent.
#[test]
fn clear_height_is_measured_from_the_body() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 2.7));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let measured = service
        .measure_clear_height(&id("space"))
        .expect("measurable");
    assert!(
        (measured.metres() - 2.7).abs() < 1e-9,
        "{}",
        measured.metres()
    );
}

/// A space stacked directly above another is not a duplicate of it.
///
/// Plan footprint alone cannot tell them apart; the vertical extent can.
#[test]
fn a_stacked_space_is_not_a_duplicate() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("lower"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("upper"), body(0.0, 4.0, 0.0, 4.0, 3.0, 6.0));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("lower"))
        .with_space(id("upper"));
    assert!(
        service
            .measure_duplicates(&id("lower"))
            .expect("measurable")
            .is_empty(),
        "a space on the storey above is a different space"
    );
}

/// Two spaces modelled over the same body are duplicates.
#[test]
fn a_coincident_space_is_a_duplicate() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("copy"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("copy"));
    assert_eq!(
        service
            .measure_duplicates(&id("space"))
            .expect("measurable"),
        vec![id("copy")]
    );
}

/// Bodies sharing plan area on different storeys do not intersect.
#[test]
fn plan_overlap_without_vertical_overlap_is_not_an_intersection() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("above"), body(0.0, 4.0, 0.0, 4.0, 5.0, 8.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    assert!(
        service
            .measure_overlaps(&id("space"))
            .expect("measurable")
            .is_empty(),
        "a body on another storey is not an intersection"
    );
}

/// A body inside the space is reported as containment, not partial overlap.
#[test]
fn a_body_inside_the_space_is_contained() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 10.0, 0.0, 10.0, 0.0, 3.0))
        .with_mesh(id("column"), body(4.0, 5.0, 4.0, 5.0, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let overlaps = service.measure_overlaps(&id("space")).expect("measurable");
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].containment(), Containment::OtherInsideSubject);
    assert!(!overlaps[0].other_is_space(), "a column is not a space");
}

/// A partially overlapping body is a partial overlap, and its measured area
/// and height are the shared ones.
#[test]
fn a_partial_intersection_reports_its_shared_extent() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("wall"), body(3.0, 6.0, 0.0, 4.0, 1.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let overlaps = service.measure_overlaps(&id("space")).expect("measurable");
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].containment(), Containment::Partial);
    // 1 m x 4 m of shared plan, 2 m of shared height.
    assert!((overlaps[0].area_square_metres() - 4.0).abs() < 1e-6);
    assert!((overlaps[0].height_metres() - 2.0).abs() < 1e-9);
}

/// A slab over half the space covers half the cap.
#[test]
fn cap_coverage_is_measured_as_a_fraction() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("slab"), body(0.0, 2.0, 0.0, 4.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab"));
    let coverage = service
        .measure_cap_coverage(&id("space"), Cap::Top)
        .expect("measurable");
    assert!(
        (coverage.covered_ratio() - 0.5).abs() < 1e-6,
        "half-covered cap, got {}",
        coverage.covered_ratio()
    );
    assert_eq!(coverage.elements(), &[id("slab")]);
}

/// Two overlapping slabs cannot cover more than the cap's own area.
#[test]
fn overlapping_cap_elements_do_not_exceed_the_cap() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("slab-a"), body(0.0, 4.0, 0.0, 4.0, 3.0, 3.2))
        .with_mesh(id("slab-b"), body(0.0, 4.0, 0.0, 4.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab-a"))
        .with_slab(id("slab-b"));
    let coverage = service
        .measure_cap_coverage(&id("space"), Cap::Top)
        .expect("measurable");
    assert!(
        coverage.covered_ratio() <= 1.0,
        "a cap cannot be more than fully covered: {}",
        coverage.covered_ratio()
    );
}

/// A wall crossing the ceiling plane is not a cap element.
#[test]
fn only_declared_cap_elements_cover_a_cap() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("wall"), body(0.0, 4.0, 0.0, 4.0, 0.0, 4.0));
    // The wall is not declared a slab or roof.
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let coverage = service
        .measure_cap_coverage(&id("space"), Cap::Top)
        .expect("measurable");
    assert!(
        coverage.elements().is_empty(),
        "a wall does not cap a space"
    );
}

/// Floor a space does not account for is reported as residual.
#[test]
fn unallocated_floor_area_is_reported_per_storey() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("floor"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.2))
        .with_mesh(id("space"), body(0.0, 5.0, 0.0, 10.0, 0.2, 3.0));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("floor"))
        .with_storey(id("floor"), id("level-0"))
        .with_storey(id("space"), id("level-0"));
    let residuals = service.measure_storey_residuals().expect("measurable");
    assert_eq!(residuals.len(), 1);
    // 100 m2 of floor, 50 m2 covered by the space.
    assert!(
        (residuals[0].area_square_metres() - 50.0).abs() < 1e-6,
        "got {}",
        residuals[0].area_square_metres()
    );
    assert_eq!(residuals[0].storey(), &id("level-0"));
}

/// Support counts report what the model has, not a decision about it.
#[test]
fn support_counts_report_declared_cap_elements() {
    let service = AxiolidSpaceService::new(AxiolidGeometry::new(), source())
        .with_slab(id("s1"))
        .with_slab(id("s2"))
        .with_roof(id("r1"))
        .with_space(id("space"))
        .with_building(id("b1"));
    let counts = service.measure_support_counts().expect("measurable");
    assert_eq!((counts.slabs(), counts.roofs()), (2, 1));
    assert_eq!(counts.buildings(), &[id("b1")]);
}

/// One unmeasurable aspect does not suppress the others.
///
/// This is the whole point of the per-aspect split: the bundled predecessor
/// failed every aspect together, so a model missing boundary segments lost
/// its clear height too.
#[test]
fn an_unavailable_aspect_does_not_suppress_the_rest() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 2.5));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));

    // An undeclared space has no geometry, so that aspect cannot be measured.
    assert_eq!(
        service.measure_clear_height(&id("absent")),
        Err(SpaceError::Unavailable),
        "an unsupplied input must be reported, not guessed"
    );
    // Every aspect of the space that IS present still answers.
    assert!(service.measure_boundary_gaps(&id("space")).is_ok());
    assert!(service.measure_clear_height(&id("space")).is_ok());
    assert!(service.measure_duplicates(&id("space")).is_ok());
    assert!(service.measure_overlaps(&id("space")).is_ok());
    assert!(service.measure_cap_coverage(&id("space"), Cap::Top).is_ok());
}

/// A space with no geometry is unavailable rather than silently zero.
#[test]
fn a_space_without_geometry_is_unavailable() {
    let service =
        AxiolidSpaceService::new(AxiolidGeometry::new(), source()).with_space(id("ghost"));
    assert_eq!(
        service.measure_clear_height(&id("ghost")),
        Err(SpaceError::Unavailable)
    );
    assert_eq!(
        service.measure_duplicates(&id("ghost")),
        Err(SpaceError::Unavailable)
    );
}

/// A body touching the space only along a face is not an intersection.
///
/// Abutting bodies share a boundary, not a volume. Without the area floor,
/// projection noise on that shared face reads as a real clash and every
/// adjacent room reports overlapping its neighbour.
#[test]
fn an_abutting_body_is_not_an_intersection() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // Shares the x = 4 face exactly: zero plan area in common.
        .with_mesh(id("neighbour"), body(4.0, 8.0, 0.0, 4.0, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("neighbour"));
    assert!(
        service
            .measure_overlaps(&id("space"))
            .expect("measurable")
            .is_empty(),
        "sharing a face is adjacency, not intersection"
    );
}

/// A space inside a larger body reports that IT is the contained one.
///
/// The direction matters to a reviewer: "this space sits inside another body"
/// is a different defect from "something is inside this space".
#[test]
fn a_space_inside_another_body_reports_subject_containment() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(2.0, 4.0, 2.0, 4.0, 0.0, 3.0))
        .with_mesh(id("shell"), body(0.0, 10.0, 0.0, 10.0, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let overlaps = service.measure_overlaps(&id("space")).expect("measurable");
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].containment(), Containment::SubjectInsideOther);
}

/// A slab on a different storey does not cap this space.
///
/// Being a slab is not enough: the element must sit at the cap plane. Without
/// that check a floor two storeys up is credited as this space's ceiling.
#[test]
fn a_slab_away_from_the_cap_plane_does_not_cover_it() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // Same plan footprint, but 5 m above the space's ceiling.
        .with_mesh(id("slab-above"), body(0.0, 4.0, 0.0, 4.0, 8.0, 8.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab-above"));
    let coverage = service
        .measure_cap_coverage(&id("space"), Cap::Top)
        .expect("measurable");
    assert!(
        coverage.elements().is_empty(),
        "a slab on another storey does not cap this space: {:?}",
        coverage.elements()
    );
    assert!(coverage.covered_area_square_metres() < 1e-12);
}

/// A slab overhanging the space covers only the part above it.
///
/// The clamp is what keeps coverage a fraction of THIS cap: an element
/// larger than the space must not report more coverage than the cap has area.
#[test]
fn an_overhanging_slab_covers_only_the_cap() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // Four times the space's footprint, resting on its ceiling.
        .with_mesh(id("slab"), body(-4.0, 8.0, -4.0, 8.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab"));
    let coverage = service
        .measure_cap_coverage(&id("space"), Cap::Top)
        .expect("measurable");
    assert!(
        (coverage.covered_ratio() - 1.0).abs() < 1e-9,
        "a fully covered cap is exactly full, got {}",
        coverage.covered_ratio()
    );
}

/// A space walled on one side reports the rest of its perimeter as gaps.
///
/// The measurement walks the real perimeter: unioning the triangle soup first
/// collapses the interior edge between the two triangles of the floor, which
/// would otherwise be reported as a phantom uncovered run.
#[test]
fn an_unwalled_boundary_is_reported_as_a_gap() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // A wall along the whole south edge only.
        .with_mesh(id("wall"), body(-0.1, 4.1, -0.1, 0.1, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let gaps = service
        .measure_boundary_gaps(&id("space"))
        .expect("measurable");

    let uncovered: f64 = gaps
        .iter()
        .map(axioval_engine::BoundaryGap::length_metres)
        .sum();
    assert!(
        uncovered > 0.0,
        "three unwalled sides must be reported as gaps"
    );
    assert!(
        uncovered < 16.0,
        "the walled side must not count as a gap, got {uncovered} of 16 m"
    );
}

/// A fully enclosed space has no boundary gaps.
#[test]
fn a_fully_walled_space_has_no_gaps() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // A slab covering the whole footprint sits on every boundary point.
        .with_mesh(id("enclosure"), body(-0.5, 4.5, -0.5, 4.5, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let gaps = service
        .measure_boundary_gaps(&id("space"))
        .expect("measurable");
    assert!(
        gaps.is_empty(),
        "an enclosed boundary has no gaps: {gaps:?}"
    );
}
