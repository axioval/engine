//! Space validation measured from real geometry, one aspect at a time.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidSpaceService};
use axioval_engine::{
    BoundaryRequest, Cap, CapRequest, Containment, OverlapRequest, SpaceAspect, SpaceError,
    SpaceService,
};
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
            .measure_overlaps(&id("space"), &OverlapRequest::new())
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
    let overlaps = service
        .measure_overlaps(&id("space"), &OverlapRequest::new())
        .expect("measurable");
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
    let overlaps = service
        .measure_overlaps(&id("space"), &OverlapRequest::new())
        .expect("measurable");
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
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
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
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
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
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
        .expect("measurable");
    assert!(
        coverage.elements().is_empty(),
        "a wall does not cap a space"
    );
}

/// Floor a space does not account for is reported as an unallocated region.
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
    let residuals = service.measure_unallocated_regions().expect("measurable");
    assert_eq!(residuals.len(), 1);
    // 100 m2 of floor, 50 m2 covered by the space.
    assert!(
        (residuals[0].area_square_metres() - 50.0).abs() < 1e-6,
        "got {}",
        residuals[0].area_square_metres()
    );
    assert_eq!(residuals[0].storey(), &id("level-0"));
    // The gross floor area the region is a share of.
    let gross = residuals[0].floor_area_square_metres().expect("stated");
    assert!((gross - 100.0).abs() < 1e-6, "got {gross}");
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
    assert!(
        service
            .measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
            .is_ok()
    );
    assert!(service.measure_clear_height(&id("space")).is_ok());
    assert!(service.measure_duplicates(&id("space")).is_ok());
    assert!(
        service
            .measure_overlaps(&id("space"), &OverlapRequest::new())
            .is_ok()
    );
    assert!(
        service
            .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
            .is_ok()
    );
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
            .measure_overlaps(&id("space"), &OverlapRequest::new())
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
    let overlaps = service
        .measure_overlaps(&id("space"), &OverlapRequest::new())
        .expect("measurable");
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
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
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
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
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
        .measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
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
        .measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
        .expect("measurable");
    assert!(
        gaps.is_empty(),
        "an enclosed boundary has no gaps: {gaps:?}"
    );
}

/// A closed, outward-oriented box, as real exports produce: its bottom face
/// winds opposite to its top when seen from above.
fn closed_box(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

/// Closed, outward-oriented bodies must be measured by their footprint too;
/// opposite cap windings cancelling would erase every space.
#[test]
fn closed_coincident_spaces_are_duplicates() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), closed_box(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("copy"), closed_box(0.0, 4.0, 0.0, 4.0, 0.0, 3.0));
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

/// The refusal for `aspect` naming `objects`.
fn blocked(aspect: SpaceAspect, objects: &[&str]) -> SpaceError {
    SpaceError::unmeasured(aspect, objects.iter().map(|local| id(local)).collect())
}

/// A declared slab that could not be measured would silently drop out of
/// every scan, so every measurement it could change refuses, naming it.
/// Without a bound it may be anywhere, so it refuses every space.
#[test]
fn an_unmeasured_declared_object_refuses_what_it_could_change() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 2.7))
        .with_mesh(id("far"), body(50.0, 54.0, 0.0, 4.0, 0.0, 2.7))
        .with_unmeasured(id("slab"), "unsupported representation");
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("far"))
        .with_slab(id("slab"));
    let top = CapRequest::new(Cap::Top);
    for space in ["space", "far"] {
        assert_eq!(
            service.measure_cap_coverage(&id(space), &top),
            Err(blocked(SpaceAspect::CapCoverage(Cap::Top), &["slab"]))
        );
        assert_eq!(
            service.measure_overlaps(&id(space), &OverlapRequest::new()),
            Err(blocked(SpaceAspect::Overlaps, &["slab"]))
        );
        assert_eq!(
            service.measure_boundary_gaps(&id(space), &BoundaryRequest::new()),
            Err(blocked(SpaceAspect::BoundaryGaps, &["slab"]))
        );
        // Only another space can duplicate a space, and the clear height
        // is the space's own: an unmeasured slab changes neither.
        assert!(service.measure_duplicates(&id(space)).is_ok());
        assert!(service.measure_clear_height(&id(space)).is_ok());
    }
    let refusal = service
        .measure_overlaps(&id("space"), &OverlapRequest::new())
        .expect_err("refused");
    assert!(refusal.to_string().contains("cad:model/slab"), "{refusal}");
    // Storey residuals scan every storey member, so they stay refused.
    assert_eq!(
        service.measure_unallocated_regions(),
        Err(blocked(SpaceAspect::UnallocatedRegions, &["slab"]))
    );
    // The slab is still a declared slab: the counts read no body.
    assert_eq!(service.measure_support_counts().map(|c| c.slabs()), Ok(1));
    // An unmeasured object with no role or storey does not concern spaces.
    let unrelated = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 2.7))
        .with_unmeasured(id("railing"), "unsupported representation");
    let service = AxiolidSpaceService::new(unrelated, source()).with_space(id("space"));
    assert!(
        service
            .measure_overlaps(&id("space"), &OverlapRequest::new())
            .is_ok()
    );
}

/// An unmeasured slab whose declared bound lies on another storey refuses
/// only the spaces it can reach: the space below it, whose ceiling it may
/// cap and whose volume it may enter. A space on the storey below that one
/// is measured.
#[test]
fn an_unmeasured_object_with_a_bound_refuses_only_the_spaces_it_reaches() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("ground"), body(0.0, 4.0, 0.0, 4.0, 0.0, 2.7))
        .with_mesh(id("ground-slab"), body(0.0, 4.0, 0.0, 4.0, 2.7, 3.0))
        .with_mesh(id("upper"), body(0.0, 4.0, 0.0, 4.0, 3.0, 5.7))
        .with_unmeasured(id("roof-slab"), "unsupported representation")
        .with_unmeasured_bound(id("roof-slab"), [0.0, 0.0, 5.7], [4.0, 4.0, 6.0]);
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("ground"))
        .with_space(id("upper"))
        .with_slab(id("ground-slab"))
        .with_slab(id("roof-slab"))
        .with_storey(id("ground"), id("level-0"))
        .with_storey(id("ground-slab"), id("level-0"))
        .with_storey(id("upper"), id("level-1"))
        .with_storey(id("roof-slab"), id("level-1"));
    let top = CapRequest::new(Cap::Top);

    // The ground floor space lies 2.7 m below the bound: measured.
    let covered = service
        .measure_cap_coverage(&id("ground"), &top)
        .expect("the unmeasured slab cannot reach the ground floor");
    assert_eq!(covered.elements(), &[id("ground-slab")]);
    assert!(
        service
            .measure_overlaps(&id("ground"), &OverlapRequest::new())
            .is_ok()
    );
    // The upper space's ceiling meets the bound: refused, naming the slab.
    assert_eq!(
        service.measure_cap_coverage(&id("upper"), &top),
        Err(blocked(SpaceAspect::CapCoverage(Cap::Top), &["roof-slab"]))
    );
    assert_eq!(
        service.measure_overlaps(&id("upper"), &OverlapRequest::new()),
        Err(blocked(SpaceAspect::Overlaps, &["roof-slab"]))
    );
    // Its floor is the measured slab below, which the bound cannot reach.
    assert!(
        service
            .measure_cap_coverage(&id("upper"), &CapRequest::new(Cap::Bottom))
            .is_ok()
    );
    // Boundary coverage is judged in plan, where the bound covers both.
    for space in ["ground", "upper"] {
        assert_eq!(
            service.measure_boundary_gaps(&id(space), &BoundaryRequest::new()),
            Err(blocked(SpaceAspect::BoundaryGaps, &["roof-slab"]))
        );
    }
}

/// Two spaces side by side on one storey: an unmeasured wall bounded far
/// from one of them refuses only the other, even in plan.
#[test]
fn a_bound_far_away_in_plan_leaves_the_space_measured() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("near"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("far"), body(20.0, 24.0, 0.0, 4.0, 0.0, 3.0))
        .with_unmeasured(id("wall"), "unsupported representation")
        .with_unmeasured_bound(id("wall"), [4.0, 0.0, 0.0], [4.2, 4.0, 3.0])
        .with_unmeasured(id("pillar"), "unsupported representation");
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("near"))
        .with_space(id("far"))
        .with_storey(id("near"), id("level"))
        .with_storey(id("far"), id("level"))
        .with_storey(id("wall"), id("level"));
    let gaps = |space: &str| service.measure_boundary_gaps(&id(space), &BoundaryRequest::new());
    assert!(gaps("far").is_ok());
    assert_eq!(
        gaps("near"),
        Err(blocked(SpaceAspect::BoundaryGaps, &["wall"]))
    );

    // A second unmeasured object without a bound refuses both, and the
    // refusal of the near space names both.
    let service = service.with_storey(id("pillar"), id("level"));
    let gaps = |space: &str| service.measure_boundary_gaps(&id(space), &BoundaryRequest::new());
    assert_eq!(
        gaps("far"),
        Err(blocked(SpaceAspect::BoundaryGaps, &["pillar"]))
    );
    assert_eq!(
        gaps("near"),
        Err(blocked(SpaceAspect::BoundaryGaps, &["pillar", "wall"]))
    );
}

/// A whole whose parts could not be composed is bounded by them: the
/// measured parts' boxes and the unmeasured parts' bounds, a bodiless part
/// adding nothing. One unbounded part leaves the whole unbounded. Bounded,
/// it refuses only the space it reaches.
#[test]
fn a_whole_is_bounded_by_its_parts() {
    let base = || {
        AxiolidGeometry::new()
            .with_mesh(id("near"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
            .with_mesh(id("far"), body(20.0, 24.0, 0.0, 4.0, 0.0, 3.0))
            .with_mesh(id("layer"), body(0.0, 4.0, 0.0, 4.0, 3.0, 3.1))
            .with_unmeasured(id("finish"), "unsupported representation")
            .with_no_body(id("opening"))
    };
    let parts = [id("layer"), id("finish"), id("opening")];
    assert_eq!(base().parts_bound(&parts), None, "an unbounded part");
    let geometry = base().with_unmeasured_bound(id("finish"), [0.0, 0.0, 3.1], [4.0, 4.5, 3.2]);
    let bound = geometry.parts_bound(&parts).expect("every part is bounded");
    assert_eq!(bound, ([0.0, 0.0, 3.0], [4.0, 4.5, 3.2]));
    assert_eq!(geometry.parts_bound(&[id("unknown")]), None);

    let geometry = geometry
        .with_unmeasured(id("roof"), "a part is unmeasured")
        .with_unmeasured_bound(id("roof"), bound.0, bound.1);
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("near"))
        .with_space(id("far"))
        .with_roof(id("roof"));
    let top = CapRequest::new(Cap::Top);
    assert!(service.measure_cap_coverage(&id("far"), &top).is_ok());
    assert_eq!(
        service.measure_cap_coverage(&id("near"), &top),
        Err(blocked(SpaceAspect::CapCoverage(Cap::Top), &["roof"]))
    );
}

/// A bound that cannot hold a body (not finite, or inverted) is no bound:
/// the object may be anywhere.
#[test]
fn an_invalid_bound_places_nothing() {
    for (min, max) in [
        ([10.0, 10.0, 10.0], [f64::NAN, 11.0, 11.0]),
        ([11.0, 10.0, 10.0], [10.0, 11.0, 11.0]),
    ] {
        let geometry = AxiolidGeometry::new()
            .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
            .with_unmeasured(id("slab"), "unsupported representation")
            .with_unmeasured_bound(id("slab"), min, max);
        let service = AxiolidSpaceService::new(geometry, source())
            .with_space(id("space"))
            .with_slab(id("slab"));
        assert_eq!(
            service.measure_overlaps(&id("space"), &OverlapRequest::new()),
            Err(blocked(SpaceAspect::Overlaps, &["slab"]))
        );
    }
}

/// A space whose own body could not be measured refuses every aspect by
/// its own name.
#[test]
fn an_unmeasured_space_names_itself() {
    let geometry =
        AxiolidGeometry::new().with_unmeasured(id("space"), "unsupported representation");
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    assert_eq!(
        service.measure_clear_height(&id("space")),
        Err(blocked(SpaceAspect::ClearHeight, &["space"]))
    );
    assert_eq!(
        service.measure_duplicates(&id("space")),
        Err(blocked(SpaceAspect::Duplicates, &["space"]))
    );
}

/// A request naming its cap elements replaces the host's declared slabs and
/// roofs: an undeclared element the rule selects covers the cap, and a
/// declared slab the rule leaves out does not.
#[test]
fn a_request_naming_cap_elements_replaces_the_declared_ones() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        // A covering the host did not declare, over the west half.
        .with_mesh(id("ceiling"), body(0.0, 2.0, 0.0, 4.0, 3.0, 3.1))
        // A declared slab over the east half.
        .with_mesh(id("slab"), body(2.0, 4.0, 0.0, 4.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab"));

    let declared = service
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
        .expect("measurable");
    assert_eq!(declared.elements(), &[id("slab")]);

    let chosen = service
        .measure_cap_coverage(
            &id("space"),
            &CapRequest::new(Cap::Top).with_elements(vec![id("ceiling")]),
        )
        .expect("measurable");
    assert_eq!(chosen.elements(), &[id("ceiling")]);
    assert!((chosen.covered_ratio() - 0.5).abs() < 1e-6);

    // An empty choice covers nothing, even where slabs are declared.
    let none = service
        .measure_cap_coverage(
            &id("space"),
            &CapRequest::new(Cap::Top).with_elements(Vec::new()),
        )
        .expect("measurable");
    assert!(none.elements().is_empty());
}

/// A requested cap element that could not be measured may be the one that
/// covers the cap, so the coverage refuses rather than under-report.
#[test]
fn an_unmeasured_requested_cap_element_makes_the_cap_unavailable() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_unmeasured(id("ceiling"), "unsupported representation");
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    assert_eq!(
        service.measure_cap_coverage(
            &id("space"),
            &CapRequest::new(Cap::Top).with_elements(vec![id("ceiling")]),
        ),
        Err(blocked(SpaceAspect::CapCoverage(Cap::Top), &["ceiling"]))
    );
    // Not requested, it concerns no space measurement.
    assert!(
        service
            .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
            .is_ok()
    );
}

/// Each connected region of floor no space covers is its own entry: two
/// 0.5 m² shafts and a 20 m² hole on one storey are three regions, each
/// naming the bodies around it.
#[test]
fn each_unallocated_region_is_measured_apart() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("floor"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.2))
        // Leaves a 0.5 m x 1 m shaft at its east end.
        .with_mesh(id("a"), body(0.0, 9.5, 0.0, 1.0, 0.2, 3.0))
        .with_mesh(id("b"), body(0.0, 10.0, 1.0, 2.0, 0.2, 3.0))
        // Leaves a second shaft.
        .with_mesh(id("c"), body(0.0, 9.5, 2.0, 3.0, 0.2, 3.0))
        .with_mesh(id("d"), body(0.0, 10.0, 3.0, 6.0, 0.2, 3.0))
        // Leaves a 5 m x 4 m hole.
        .with_mesh(id("e"), body(0.0, 5.0, 6.0, 10.0, 0.2, 3.0));
    let mut service = AxiolidSpaceService::new(geometry, source())
        .with_slab(id("floor"))
        .with_storey(id("floor"), id("level-0"));
    for space in ["a", "b", "c", "d", "e"] {
        service = service
            .with_space(id(space))
            .with_storey(id(space), id("level-0"));
    }
    let mut regions = service.measure_unallocated_regions().expect("measurable");
    regions.sort_by(|x, y| x.area_square_metres().total_cmp(&y.area_square_metres()));
    let areas: Vec<f64> = regions
        .iter()
        .map(axioval_engine::UnallocatedRegion::area_square_metres)
        .collect();
    assert_eq!(areas.len(), 3, "{areas:?}");
    for (area, expected) in areas.iter().zip([0.5, 0.5, 20.0]) {
        assert!((area - expected).abs() < 1e-6, "{areas:?}");
    }
    assert!(
        regions
            .iter()
            .all(|region| region.storey() == &id("level-0"))
    );
    let hole = &regions[2];
    assert_eq!(hole.elements(), &[id("d"), id("e"), id("floor")]);
}

/// A boundary request naming its elements replaces the default: a space
/// bounded only by furniture is covered by default, uncovered when only
/// walls bound it, and covered again once furniture is selected.
#[test]
fn a_boundary_request_chooses_the_bounding_elements() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("furniture"), body(-0.5, 4.5, -0.5, 4.5, 0.0, 1.0))
        .with_mesh(id("wall"), body(20.0, 24.0, 0.0, 0.2, 0.0, 3.0));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let uncovered = |request: &BoundaryRequest| -> f64 {
        service
            .measure_boundary_gaps(&id("space"), request)
            .expect("measurable")
            .iter()
            .map(axioval_engine::BoundaryGap::length_metres)
            .sum()
    };
    assert!(uncovered(&BoundaryRequest::new()) < 1e-9);
    let walls = BoundaryRequest::new().with_elements(vec![id("wall")]);
    assert!((uncovered(&walls) - 16.0).abs() < 1e-6);
    let furniture = BoundaryRequest::new().with_elements(vec![id("wall"), id("furniture")]);
    assert!(uncovered(&furniture) < 1e-9);
}

/// An overlap request naming its elements measures only those.
#[test]
fn an_overlap_request_chooses_the_intersecting_elements() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("column"), body(1.0, 2.0, 1.0, 2.0, 0.0, 3.0))
        .with_mesh(id("duct"), body(3.0, 6.0, 0.0, 4.0, 2.0, 2.5));
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    let others = |request: &OverlapRequest| -> Vec<ObjectId> {
        service
            .measure_overlaps(&id("space"), request)
            .expect("measurable")
            .iter()
            .map(|overlap| overlap.other().clone())
            .collect()
    };
    assert_eq!(
        others(&OverlapRequest::new()),
        vec![id("column"), id("duct")]
    );
    assert_eq!(
        others(&OverlapRequest::new().with_elements(vec![id("duct")])),
        vec![id("duct")]
    );
}

/// A requested element that could not be measured may be the one covering
/// or intersecting, so the measurement refuses.
#[test]
fn an_unmeasured_requested_element_makes_boundary_and_overlaps_unavailable() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), body(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_unmeasured(id("wall"), "unsupported representation");
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("space"));
    assert_eq!(
        service.measure_boundary_gaps(
            &id("space"),
            &BoundaryRequest::new().with_elements(vec![id("wall")])
        ),
        Err(blocked(SpaceAspect::BoundaryGaps, &["wall"]))
    );
    assert_eq!(
        service.measure_overlaps(
            &id("space"),
            &OverlapRequest::new().with_elements(vec![id("wall")])
        ),
        Err(blocked(SpaceAspect::Overlaps, &["wall"]))
    );
}

/// Two rooms meeting at one corner leave floor whose two pieces touch at
/// that corner (axiolid/kernel#253, #262): it is measured, never refused.
#[test]
fn unallocated_floor_pinched_at_a_corner_is_measured() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("floor"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.2))
        .with_mesh(id("a"), body(0.0, 5.0, 0.0, 5.0, 0.2, 3.0))
        .with_mesh(id("b"), body(5.0, 10.0, 5.0, 10.0, 0.2, 3.0));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_slab(id("floor"))
        .with_storey(id("floor"), id("level-0"))
        .with_space(id("a"))
        .with_storey(id("a"), id("level-0"))
        .with_space(id("b"))
        .with_storey(id("b"), id("level-0"));
    let regions = service.measure_unallocated_regions().expect("measurable");
    let areas: Vec<f64> = regions
        .iter()
        .map(axioval_engine::UnallocatedRegion::area_square_metres)
        .collect();
    // The two pieces are two regions: a point joins nothing.
    assert_eq!(areas.len(), 2, "{areas:?}");
    assert!(
        areas.iter().all(|area| (area - 25.0).abs() < 1e-6),
        "{areas:?}"
    );
}

/// A closed room whose front face (`y = 0`) leans by `lean` metres over its
/// height: both its triangles cast slivers in plan, two corners `lean`
/// apart, as the near-vertical faces of a modelled room do after rounding.
/// The plan overlay refuses a ring with two corners within its tolerance
/// (`RepeatedVertex`).
fn leaning_room(lean: f64) -> TriMesh {
    let mut mesh = closed_box(0.0, 4.0, 0.0, 4.0, 0.0, 3.0);
    for top_front in [4, 5] {
        mesh.positions[top_front].y = lean;
    }
    mesh
}

/// The slivers of a leaning face are left out of the overlay and their
/// area bounds what they could add: every aspect resting on plan area is
/// measured, far from any decision they could tip (#217).
#[test]
fn a_space_casting_slivers_in_plan_is_measured() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), leaning_room(1e-10))
        .with_mesh(id("copy"), closed_box(0.0, 4.0, 0.0, 4.0, 0.0, 3.0))
        .with_mesh(id("wall"), closed_box(3.0, 6.0, 0.0, 4.0, 1.0, 3.0))
        .with_mesh(id("slab"), closed_box(0.0, 2.0, 0.0, 4.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("copy"))
        .with_slab(id("slab"));
    assert_eq!(
        service.measure_duplicates(&id("space")).expect("measured"),
        vec![id("copy")]
    );
    let overlaps = service
        .measure_overlaps(
            &id("space"),
            &OverlapRequest::new().with_elements(vec![id("wall")]),
        )
        .expect("measured");
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0].containment(), Containment::Partial);
    assert!((overlaps[0].area_square_metres() - 4.0).abs() < 1e-6);
    let coverage = service
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
        .expect("measured");
    assert!((coverage.covered_ratio() - 0.5).abs() < 1e-6);
    assert!((coverage.whole_area_square_metres() - 16.0).abs() < 1e-6);
    assert_eq!(coverage.elements(), &[id("slab")]);
}

/// Two coincident spaces whose plan the overlay refuses unfiltered are
/// duplicates: a refused plan area is never read as zero, which would
/// pass the pair as "not mutually contained" (#217).
#[test]
fn coincident_spaces_casting_slivers_are_duplicates() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), leaning_room(1e-10))
        .with_mesh(id("copy"), leaning_room(1e-10));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("copy"));
    assert_eq!(
        service.measure_duplicates(&id("space")).expect("measured"),
        vec![id("copy")]
    );
    assert_eq!(
        service.measure_duplicates(&id("copy")).expect("measured"),
        vec![id("space")]
    );
}

/// A body abutting the leaning face shares at most the slivers' area,
/// dust that can never be an intersection: it is left out, not refused,
/// and a slab abutting the space there neither covers its cap nor is cited.
#[test]
fn a_body_abutting_a_sliver_face_shares_nothing() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), leaning_room(9e-10))
        .with_mesh(id("wall"), closed_box(0.0, 4.0, -1.0, 0.0, 0.0, 3.0))
        .with_mesh(id("slab"), closed_box(0.0, 4.0, -2.0, 0.0, 3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_slab(id("slab"));
    let overlaps = service
        .measure_overlaps(
            &id("space"),
            &OverlapRequest::new().with_elements(vec![id("wall")]),
        )
        .expect("measured");
    assert!(overlaps.is_empty(), "{overlaps:?}");
    let coverage = service
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
        .expect("measured");
    assert!(
        coverage.covered_ratio() < 1e-6,
        "{}",
        coverage.covered_ratio()
    );
    assert!(coverage.elements().is_empty());
}

/// Two plan triangles sharing an edge some 10⁶ m from the origin, as a
/// fan of a round corner's chords lies in a georeferenced model, at two
/// heights; the first is thin and wound clockwise. A shoelace sum over
/// the coordinates rounds its area (about 7.7e-5 m²) to the wrong sign
/// there, which left it clockwise in the plan soup and made the overlay
/// refuse the footprint as self-intersecting.
fn far_fan(z0: f64, z1: f64) -> TriMesh {
    let corners = [
        (600_026.029_859_079, 5_599_990.725_323_705),
        (600_026.142_665_062_9, 5_599_990.741_066_861),
        (600_026.086_448_961, 5_599_990.731_856_141),
        (600_026.253_470_314_9, 5_599_990.767_434_134),
    ];
    let mut positions = Vec::new();
    for z in [z0, z1] {
        positions.extend(corners.iter().map(|&(x, y)| Point3::new(x, y, z)));
    }
    TriMesh::new(positions, vec![0, 1, 2, 0, 1, 3, 4, 5, 6, 4, 5, 7])
}

/// A space whose plan holds a thin triangle far from the origin is
/// measured: its footprint keeps every triangle's winding.
#[test]
fn a_thin_triangle_far_from_the_origin_keeps_its_winding() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), far_fan(0.0, 3.0))
        .with_mesh(id("copy"), far_fan(0.0, 3.0))
        .with_mesh(id("slab"), far_fan(3.0, 3.2));
    let service = AxiolidSpaceService::new(geometry, source())
        .with_space(id("space"))
        .with_space(id("copy"))
        .with_slab(id("slab"));
    assert_eq!(
        service.measure_duplicates(&id("space")).expect("measured"),
        vec![id("copy")]
    );
    let coverage = service
        .measure_cap_coverage(&id("space"), &CapRequest::new(Cap::Top))
        .expect("measured");
    assert!(
        (coverage.covered_ratio() - 1.0).abs() < 1e-6,
        "{}",
        coverage.covered_ratio()
    );
}
