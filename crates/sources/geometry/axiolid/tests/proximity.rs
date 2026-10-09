//! Proximity measurement over real Axiolid geometry.
//!
//! Each case is a joint a clash check must classify correctly. The cases
//! where zero separation means *touching* matter as much as the clashes: a
//! slab resting on a wall is how buildings stand up, not a defect.

use std::f64::consts::TAU;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    BodyContainment, GeometryFidelity, LengthInterval, MetricDirection, OverlapAlongRequest,
    ProximityError, ProximityRequest, ProximityService,
};
use axioval_ir::{ObjectId, SourceId};

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

/// A closed vertical prism approximating a cylinder with `sides` chords.
fn column(centre: [f64; 2], radius: f64, z: [f64; 2], sides: u32) -> TriMesh {
    let mut positions = Vec::new();
    for level in z {
        for side in 0..sides {
            let angle = TAU * f64::from(side) / f64::from(sides);
            positions.push(Point3::new(
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(centre[0], centre[1], z[0]));
    positions.push(Point3::new(centre[0], centre[1], z[1]));
    let (bottom_centre, top_centre) = (2 * sides, 2 * sides + 1);
    let mut indices = Vec::new();
    for side in 0..sides {
        let next = (side + 1) % sides;
        let (b0, b1, t0, t1) = (side, next, side + sides, next + sides);
        indices.extend([b0, b1, t1, b0, t1, t0]);
        indices.extend([bottom_centre, b1, b0]);
        indices.extend([top_centre, t0, t1]);
    }
    TriMesh::new(positions, indices)
}

/// A single open quad: a surface, not a solid.
fn quad(z: f64) -> TriMesh {
    TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, z),
            Point3::new(1.0, 0.0, z),
            Point3::new(1.0, 1.0, z),
            Point3::new(0.0, 1.0, z),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn wall() -> TriMesh {
    cuboid([0.0, 0.0, 0.0], [4.0, 0.2, 3.0])
}

fn measure(
    geometry: AxiolidGeometry,
    subject: &str,
    counterpart: &str,
) -> axioval_engine::ProximityEvidence {
    AxiolidProximityService::new(geometry)
        .measure_proximity(
            &ProximityRequest::try_new(id(subject), id(counterpart))
                .unwrap()
                .with_plan_overlap(),
        )
        .expect("measurable")
}

#[test]
fn a_pipe_through_a_wall_penetrates_by_half_the_wall() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("pipe"), cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]));
    let measured = measure(geometry, "pipe", "wall");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    let depth = measured.penetration_metres().expect("both are solids");
    // No vertex of either body lies inside the other: the witness is the
    // midpoint between where the pipe's edges cross the two wall faces.
    assert!((depth - 0.1).abs() < 1e-9, "depth {depth}");
    assert!((measured.plan_overlap_square_metres().expect("measured") - 0.02).abs() < 1e-6);
    assert!(measured.evidence().exact);
}

/// The plan overlap costs an overlay per pair; a request that does not ask
/// for it leaves it unmeasured and every measurement in space unchanged.
#[test]
fn a_request_not_asking_for_the_plan_overlap_leaves_it_unmeasured() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("pipe"), cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]));
    let asked = measure(geometry.clone(), "pipe", "wall");
    let unasked = AxiolidProximityService::new(geometry)
        .measure_proximity(&ProximityRequest::try_new(id("pipe"), id("wall")).unwrap())
        .expect("measurable");
    assert!(asked.plan_overlap_square_metres().is_some());
    assert_eq!(unasked.plan_overlap_square_metres(), None);
    assert_eq!(
        unasked.separation_metres().to_bits(),
        asked.separation_metres().to_bits()
    );
    assert_eq!(
        unasked.penetration_metres().map(f64::to_bits),
        asked.penetration_metres().map(f64::to_bits)
    );
    assert_eq!(unasked.intersection_volume(), asked.intersection_volume());
}

/// A wall whose front face leans by `lean` metres over its height: both
/// its triangles cast slivers in plan, two corners `lean` apart, as the
/// near-vertical faces of a modelled wall do after rounding.
fn leaning_wall(lean: f64) -> TriMesh {
    let mut mesh = wall();
    for top_front in [4, 5] {
        mesh.positions[top_front].y = lean;
    }
    mesh
}

/// The overlay refuses a ring with two corners within its tolerance, so
/// the plan overlap of a leaning wall is unknown; nothing in space rests on
/// it, and the clash is still measured.
#[test]
fn a_wall_casting_a_sliver_in_plan_is_still_measured() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), leaning_wall(1e-10))
        .with_mesh(id("cross-wall"), cuboid([1.0, -1.0, 0.0], [1.2, 1.0, 3.0]));
    let measured = measure(geometry, "cross-wall", "wall");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    let depth = measured.penetration_metres().expect("both are solids");
    assert!(depth > 0.05, "depth {depth}");
    assert_eq!(measured.plan_overlap_square_metres(), None);
    assert!(measured.evidence().exact);
}

#[test]
fn a_slab_resting_on_a_wall_touches_without_penetrating() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("slab"), cuboid([-1.0, -1.0, 3.0], [5.0, 5.0, 3.2]));
    let measured = measure(geometry, "slab", "wall");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    assert_eq!(measured.penetration_metres(), Some(0.0));
    // In plan the wall lies entirely under the slab.
    // The overlay rounds through single precision.
    assert!((measured.plan_overlap_square_metres().expect("measured") - 0.8).abs() < 1e-6);
}

#[test]
fn walls_butted_end_to_end_touch_without_penetrating() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), wall())
        .with_mesh(id("b"), cuboid([4.0, 0.0, 0.0], [8.0, 0.2, 3.0]));
    let measured = measure(geometry, "a", "b");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    assert_eq!(measured.penetration_metres(), Some(0.0));
}

/// Every surface point of an exact duplicate lies on the other's surface;
/// only its interior shows the overlap.
#[test]
fn an_exact_duplicate_penetrates_to_its_centre() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), wall())
        .with_mesh(id("b"), wall());
    let depth = measure(geometry, "a", "b")
        .penetration_metres()
        .expect("solids");
    assert!((depth - 0.1).abs() < 1e-9, "depth {depth}");
}

#[test]
fn separated_bodies_report_their_gap() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("duct"), cuboid([0.0, 0.5, 1.0], [1.0, 1.0, 1.5]));
    let measured = measure(geometry, "duct", "wall");
    assert!((measured.separation_metres() - 0.3).abs() < 1e-9);
    assert_eq!(measured.penetration_metres(), Some(0.0));
    assert_eq!(measured.containment(), None);
    assert!(
        measured
            .plan_overlap_square_metres()
            .expect("measured")
            .abs()
            < f64::EPSILON
    );
}

#[test]
fn a_body_wholly_inside_another_is_contained() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("box"), cuboid([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]));
    let measured = measure(geometry, "box", "room");
    assert!((measured.separation_metres() - 1.0).abs() < 1e-9);
    assert_eq!(
        measured.containment(),
        Some(BodyContainment::SubjectInsideCounterpart)
    );
    assert!((measured.penetration_metres().unwrap() - 1.5).abs() < 1e-9);

    let reversed = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
            .with_mesh(id("box"), cuboid([1.0, 1.0, 1.0], [2.0, 2.0, 2.0])),
    )
    .measure_proximity(&ProximityRequest::try_new(id("room"), id("box")).unwrap())
    .unwrap();
    assert_eq!(
        reversed.containment(),
        Some(BodyContainment::CounterpartInsideSubject)
    );
}

#[test]
fn a_tessellated_column_through_a_slab_is_approximate() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_tessellated_mesh(id("column"), column([2.0, 2.0], 0.2, [0.0, 4.0], 24), 0.002);
    let measured = measure(geometry, "column", "slab");
    assert_eq!(
        measured.fidelity(),
        GeometryFidelity::Tessellated {
            chord_deviation_metres: 0.002
        }
    );
    assert!(
        !measured.evidence().exact,
        "a tessellation is not exact evidence"
    );
    // The column's edges cross the slab and witness half its thickness; the
    // slab's diagonal edges cross the column near its axis and witness more.
    // Either is a valid lower bound on the true overlap.
    let depth = measured.penetration_metres().expect("solids");
    assert!((0.1 - 1e-9..=0.2).contains(&depth), "depth {depth}");
}

#[test]
fn bounds_carry_fidelity() {
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("wall"), wall())
            .with_tessellated_mesh(id("column"), column([0.0, 0.0], 0.2, [0.0, 3.0], 12), 0.003),
    );
    let wall = service.bounds(&id("wall")).unwrap();
    let max = wall.bounds().max();
    assert!((max[0] - 4.0).abs() + (max[1] - 0.2).abs() + (max[2] - 3.0).abs() < 1e-12);
    assert_eq!(wall.fidelity(), GeometryFidelity::Exact);
    let column = service.bounds(&id("column")).unwrap();
    assert!(!column.fidelity().is_exact());
    // The enclosing box covers the true cylinder, which bulges past the chords.
    assert!(column.enclosing().max()[0] >= 0.2);
}

/// A sheet crossing a wall is measured against the wall's inside: a surface
/// has no volume, so the sheet entering the wall is the whole overlap.
#[test]
fn an_open_surface_crossing_a_solid_penetrates_it() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("sheet"), quad(1.0));
    let measured = measure(geometry, "sheet", "wall");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    let depth = measured.penetration_metres().expect("the wall is closed");
    assert!((depth - 0.1).abs() < 1e-9, "depth {depth}");
}

#[test]
fn an_open_surface_lying_on_a_solid_touches_it() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("membrane"), quad(3.0));
    let measured = measure(geometry, "membrane", "wall");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    assert_eq!(measured.penetration_metres(), Some(0.0));
}

#[test]
fn an_open_surface_inside_a_solid_is_contained() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([-1.0, -1.0, 0.0], [2.0, 2.0, 3.0]))
        .with_mesh(id("sheet"), quad(1.0));
    let measured = measure(geometry, "room", "sheet");
    assert_eq!(
        measured.containment(),
        Some(BodyContainment::CounterpartInsideSubject)
    );
}

/// Two surfaces share no volume. Whether meeting sheets touch or cross is not
/// a penetration depth, so none is reported -- not zero.
#[test]
fn two_open_surfaces_have_no_penetration_measurement() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), quad(1.0))
        .with_mesh(id("b"), quad(1.0));
    let measured = measure(geometry, "a", "b");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    assert_eq!(measured.penetration_metres(), None);
}

#[test]
fn missing_geometry_and_bad_deviation_are_refused() {
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("wall"), wall())
            .with_tessellated_mesh(id("bad"), column([9.0, 9.0], 0.2, [0.0, 3.0], 8), f64::NAN),
    );
    let request = |subject: &str| {
        ProximityRequest::try_new(id(subject), id("wall"))
            .unwrap()
            .with_plan_overlap()
    };
    assert_eq!(
        service.measure_proximity(&request("ghost")).unwrap_err(),
        ProximityError::Unavailable
    );
    assert_eq!(
        service.measure_proximity(&request("bad")).unwrap_err(),
        ProximityError::InvalidMeasurement
    );
}

fn assert_interval(interval: axioval_engine::LengthInterval, lower: f64, upper: f64) {
    assert!(
        (interval.lower_metres() - lower).abs() < 1e-9
            && (interval.upper_metres() - upper).abs() < 1e-9,
        "{interval:?}, expected [{lower}, {upper}]"
    );
}

/// For two boxes the intersection's vertices are all witnessed, so each
/// extent is known exactly: the pipe's width in x and z, the wall's
/// thickness in y.
#[test]
fn a_pipe_through_a_wall_overlaps_by_its_section_and_the_wall_thickness() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("pipe"), cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]));
    let extents = measure(geometry, "pipe", "wall")
        .overlap_extents()
        .expect("both are solids");
    assert_interval(extents.x(), 0.1, 0.1);
    assert_interval(extents.y(), 0.2, 0.2);
    assert_interval(extents.z(), 0.1, 0.1);
    assert_interval(extents.horizontal(), 0.1, 0.1);
    assert_interval(extents.vertical(), 0.1, 0.1);
}

/// A duct sunk 5 mm into a slab overlaps it widely in plan and barely in
/// height: the extents tell the two apart.
#[test]
fn a_shallow_overlap_is_thin_vertically() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_mesh(id("duct"), cuboid([1.0, 1.0, 2.5], [3.0, 1.5, 3.005]));
    let extents = measure(geometry, "duct", "slab")
        .overlap_extents()
        .expect("solids");
    assert_interval(extents.horizontal(), 0.5, 0.5);
    assert_interval(extents.vertical(), 0.005, 0.005);
}

#[test]
fn a_contained_body_overlaps_by_its_own_extent_and_apart_bodies_by_none() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("box"), cuboid([1.0, 1.0, 1.0], [2.0, 2.5, 2.0]));
    let extents = measure(geometry, "box", "room").overlap_extents().unwrap();
    assert_interval(extents.x(), 1.0, 1.0);
    assert_interval(extents.y(), 1.5, 1.5);
    assert_interval(extents.z(), 1.0, 1.0);

    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("duct"), cuboid([0.0, 0.5, 1.0], [1.0, 1.0, 1.5]));
    let extents = measure(geometry, "duct", "wall").overlap_extents().unwrap();
    for axis in [extents.x(), extents.y(), extents.z()] {
        assert_interval(axis, 0.0, 0.0);
    }
    // Two open surfaces share no volume to have extents.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), quad(1.0))
        .with_mesh(id("b"), quad(1.0));
    assert_eq!(measure(geometry, "a", "b").overlap_extents(), None);
}

#[test]
fn identical_bodies_are_zero_apart_in_hausdorff_distance() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), wall())
        .with_mesh(id("b"), wall());
    let hausdorff = measure(geometry, "a", "b")
        .hausdorff_interval_metres()
        .unwrap();
    assert_interval(hausdorff, 0.0, 0.0);
}

/// A copy shifted by 3 mm is 3 mm away at its ends, and the bound on every
/// face holds it close to that.
#[test]
fn a_shifted_copy_is_as_far_as_its_shift() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), wall())
        .with_mesh(id("b"), cuboid([0.003, 0.0, 0.0], [4.003, 0.2, 3.0]));
    let hausdorff = measure(geometry, "a", "b")
        .hausdorff_interval_metres()
        .unwrap();
    assert!(
        (hausdorff.lower_metres() - 0.003).abs() < 1e-9,
        "{hausdorff:?}"
    );
    assert!(hausdorff.upper_metres() < 0.0035, "{hausdorff:?}");
}

#[test]
fn different_bodies_are_far_apart_in_hausdorff_distance() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("pipe"), cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]));
    let hausdorff = measure(geometry, "pipe", "wall")
        .hausdorff_interval_metres()
        .unwrap();
    // The wall's far corner lies metres from the pipe.
    assert!(hausdorff.lower_metres() > 2.0, "{hausdorff:?}");
    assert!(hausdorff.upper_metres() >= hausdorff.lower_metres());
}

/// Each tessellated surface may lie its deviation from its mesh, so two
/// identical meshes bound the true distance by both deviations.
#[test]
fn tessellated_duplicates_widen_by_the_deviations() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("a"), column([0.0, 0.0], 0.2, [0.0, 3.0], 16), 0.002)
        .with_tessellated_mesh(id("b"), column([0.0, 0.0], 0.2, [0.0, 3.0], 16), 0.001);
    let measured = measure(geometry, "a", "b");
    assert_interval(measured.hausdorff_interval_metres().unwrap(), 0.0, 0.003);
    let extents = measured.overlap_extents().unwrap();
    // Witnessed across the whole column less both ends' deviations; the
    // boxes grown by each deviation bound it above.
    assert_interval(extents.z(), 3.0 - 0.006, 3.002);
}

fn assert_volume(interval: axioval_engine::VolumeInterval, expected: f64) {
    assert!(
        interval.lower_cubic_metres() <= expected + 1e-12
            && interval.upper_cubic_metres() >= expected - 1e-12
            && interval.upper_cubic_metres() - interval.lower_cubic_metres() < 1e-9,
        "{interval:?}, expected {expected}"
    );
}

/// A 0.1 m square pipe 2.2 m long through a 0.2 m wall shares 0.002 m³.
#[test]
fn a_pipe_through_a_wall_shares_its_section_times_the_wall_thickness() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("pipe"), cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]));
    let volume = measure(geometry, "pipe", "wall")
        .intersection_volume()
        .expect("two closed solids");
    assert_volume(volume.shared(), 0.002);
    assert_volume(volume.subject(), 0.022);
    assert_volume(volume.counterpart(), 2.4);
    let (lower, upper) = volume.ratio_of_smaller();
    assert!(lower <= 1.0 / 11.0 && upper >= 1.0 / 11.0 && upper - lower < 1e-9);
}

/// A body inside another shares all of itself; bodies apart share nothing,
/// exactly.
#[test]
fn a_contained_body_shares_its_whole_volume_and_apart_bodies_none() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("box"), cuboid([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]))
        .with_mesh(id("far"), cuboid([10.0, 0.0, 0.0], [11.0, 1.0, 1.0]));
    let service = AxiolidProximityService::new(geometry);
    let inside = service
        .measure_proximity(&ProximityRequest::try_new(id("box"), id("room")).unwrap())
        .unwrap()
        .intersection_volume()
        .unwrap();
    assert_volume(inside.shared(), 1.0);
    let (lower, upper) = inside.ratio_of_smaller();
    assert!(upper >= 1.0 && lower > 0.999_999, "{lower} {upper}");
    let apart = service
        .measure_proximity(&ProximityRequest::try_new(id("far"), id("room")).unwrap())
        .unwrap()
        .intersection_volume()
        .unwrap();
    assert!(apart.shared().is_exact());
    assert!(apart.shared().upper_cubic_metres().abs() < f64::EPSILON);
}

/// Walls butted end to end touch: they share no volume, within rounding.
#[test]
fn touching_bodies_share_no_volume() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), wall())
        .with_mesh(id("b"), cuboid([4.0, 0.0, 0.0], [8.0, 0.2, 3.0]));
    let volume = measure(geometry, "a", "b").intersection_volume().unwrap();
    assert_volume(volume.shared(), 0.0);
}

/// An open surface encloses no volume, so none is reported.
#[test]
fn an_open_surface_has_no_intersection_volume() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("sheet"), quad(0.5));
    assert!(
        measure(geometry, "sheet", "slab")
            .intersection_volume()
            .is_none()
    );
}

/// A box whose top face is split at a point inside an edge the right face
/// keeps whole: a T-junction, as a B-rep face with a ring touching another
/// ring's edge can leave since axiolid-mesh-compile 0.3.14 (#262). The
/// shell looks closed but its mesh is not, so it is measured as a
/// surface: it encloses no volume to share.
#[test]
fn a_box_with_a_t_junction_is_no_closed_solid() {
    let mut box_ = cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    box_.positions.push(Point3::new(1.0, 0.5, 1.0));
    // The top's first triangle (4, 5, 6) becomes (4, 5, 8) and (4, 8, 6).
    let top = 6;
    box_.indices.splice(top..top + 3, [4, 5, 8, 4, 8, 6]);
    let slab = || cuboid([0.5, 0.0, 0.5], [2.0, 1.0, 2.0]);
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), slab())
        .with_mesh(id("box"), box_);
    assert!(
        measure(geometry, "box", "slab")
            .intersection_volume()
            .is_none()
    );
    // The same box closed shares a quarter of its volume.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), slab())
        .with_mesh(id("box"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    let volume = measure(geometry, "box", "slab")
        .intersection_volume()
        .unwrap();
    assert_volume(volume.shared(), 0.25);
}

/// The box of `a_box_with_a_t_junction_is_no_closed_solid` with the gap
/// closed by a zero-area triangle along the right face's top edge, as a
/// warped face's triangulation leaves one (#221). Closed, zero-area
/// triangle counted.
fn box_with_a_sliver() -> TriMesh {
    let mut box_ = cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    box_.positions.push(Point3::new(1.0, 0.5, 1.0));
    let top = 6;
    box_.indices.splice(top..top + 3, [4, 5, 8, 4, 8, 6]);
    box_.indices.extend([5, 6, 8]);
    box_
}

/// A closed rod with one edge from `start` to `end`, `width` along y and
/// `thickness` along `normal` (a unit vector square to the edge and to y).
fn rod(start: [f64; 3], end: [f64; 3], width: f64, normal: [f64; 3], thickness: f64) -> TriMesh {
    let at = |base: [f64; 3], wide: bool, thick: bool| {
        Point3::new(
            base[0] + if thick { normal[0] * thickness } else { 0.0 },
            base[1] + if wide { width } else { 0.0 },
            base[2] + if thick { normal[2] * thickness } else { 0.0 },
        )
    };
    let mut positions = Vec::new();
    for base in [start, end] {
        for (wide, thick) in [(false, false), (true, false), (true, true), (false, true)] {
            positions.push(at(base, wide, thick));
        }
    }
    // Closed and consistently wound; the winding number takes it either
    // way round.
    TriMesh::new(
        positions,
        vec![
            0, 1, 2, 0, 2, 3, // start cap
            4, 6, 5, 4, 7, 6, // end cap
            0, 4, 5, 0, 5, 1, // sides
            1, 5, 6, 1, 6, 2, //
            2, 6, 7, 2, 7, 3, //
            3, 7, 4, 3, 4, 0, //
        ],
    )
}

/// Every quantity two measurements share, compared within rounding.
fn assert_same_measurement(
    with_sliver: &axioval_engine::ProximityEvidence,
    plain: &axioval_engine::ProximityEvidence,
    same_penetration: bool,
) {
    let close = |what: &str, a: f64, b: f64| {
        assert!((a - b).abs() < 1e-12, "{what}: {a} against {b}");
    };
    close(
        "separation",
        with_sliver.separation_metres(),
        plain.separation_metres(),
    );
    assert_eq!(
        with_sliver.penetration_metres().is_some(),
        plain.penetration_metres().is_some()
    );
    if let (Some(a), Some(b), true) = (
        with_sliver.penetration_metres(),
        plain.penetration_metres(),
        same_penetration,
    ) {
        close("penetration", a, b);
    }
    assert_eq!(with_sliver.containment(), plain.containment());
    let (a, b) = (
        with_sliver.overlap_extents().map(|e| [e.x(), e.y(), e.z()]),
        plain.overlap_extents().map(|e| [e.x(), e.y(), e.z()]),
    );
    assert_eq!(a.is_some(), b.is_some());
    if let (Some(a), Some(b)) = (a, b) {
        for (a, b) in a.iter().zip(&b) {
            close("extent lower", a.lower_metres(), b.lower_metres());
            close("extent upper", a.upper_metres(), b.upper_metres());
        }
    }
    let (a, b) = (
        with_sliver.hausdorff_interval_metres().unwrap(),
        plain.hausdorff_interval_metres().unwrap(),
    );
    // The bounds follow the triangulation (the sliver's box has another
    // top), but both hold the one true distance, so they overlap.
    assert!(
        a.lower_metres().max(b.lower_metres()) <= a.upper_metres().min(b.upper_metres()) + 1e-12,
        "hausdorff {a:?} against {b:?}"
    );
    assert_eq!(
        with_sliver.plan_overlap_square_metres().is_some(),
        plain.plan_overlap_square_metres().is_some()
    );
    if let (Some(a), Some(b)) = (
        with_sliver.plan_overlap_square_metres(),
        plain.plan_overlap_square_metres(),
    ) {
        assert!((a - b).abs() < 1e-6, "{a} against {b}");
    }
    assert_eq!(with_sliver.evidence().exact, plain.evidence().exact);
}

/// A closed box holding a zero-area triangle is measured as the box it
/// is: the sliver lies on an edge the remaining faces share, so every
/// separation, penetration, containment, extent and Hausdorff distance
/// agrees with the plain box's, in either order (#221).
#[test]
fn a_closed_box_with_a_zero_area_triangle_is_measured_as_the_box() {
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let counterparts = [
        ("pipe", cuboid([0.4, -1.0, 0.4], [0.6, 2.0, 0.6])),
        ("slab", cuboid([0.5, 0.0, 0.5], [2.0, 1.0, 2.0])),
        ("apart", cuboid([3.0, 0.0, 0.0], [4.0, 1.0, 1.0])),
        ("inner", cuboid([0.25, 0.25, 0.25], [0.75, 0.75, 0.75])),
        ("around", cuboid([-1.0, -1.0, -1.0], [2.0, 2.0, 2.0])),
        ("over-the-edge", cuboid([0.9, 0.4, 0.9], [1.1, 0.6, 1.1])),
        // A rod whose edge runs through the sliver's segment (not at its
        // corner), standing on the box's top edge and reaching into it.
        (
            "touching-rod",
            rod(
                [0.5, 0.25, 1.5],
                [1.5, 0.25, 0.5],
                0.1,
                [diagonal, 0.0, diagonal],
                0.1,
            ),
        ),
        (
            "reaching-rod",
            rod(
                [0.45, 0.25, 1.45],
                [1.45, 0.25, 0.45],
                0.1,
                [diagonal, 0.0, diagonal],
                0.1,
            ),
        ),
    ];
    for (name, counterpart) in counterparts {
        let with_sliver = AxiolidGeometry::new()
            .with_mesh(id("box"), box_with_a_sliver())
            .with_mesh(id(name), counterpart.clone());
        let plain = AxiolidGeometry::new()
            .with_mesh(id("box"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
            .with_mesh(id(name), counterpart);
        for (subject, other) in [("box", name), (name, "box")] {
            let measured = measure(with_sliver.clone(), subject, other);
            let expected = measure(plain.clone(), subject, other);
            if name == "around" {
                // The box's deepest witness is the centroid of its
                // triangles' corners, which follows the triangulation (the
                // sliver's box has another top). Both are witnesses: at
                // least a corner's depth of 1, at most the centre's 1.5.
                let depth = measured.penetration_metres().expect("contained");
                assert!((1.0..=1.5).contains(&depth), "{depth}");
                assert_same_measurement(&measured, &expected, false);
            } else {
                assert_same_measurement(&measured, &expected, true);
            }
            // The volume kernel refuses a mesh holding a zero-area
            // triangle; that leaves the volume unmeasured, never zero.
            assert!(measured.intersection_volume().is_none(), "{name}");
        }
    }
    // The pipe through the sliver's box is witnessed at the box's centre,
    // half a box deep: the box has an inside although the audit counts a
    // zero-area triangle in it.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("box"), box_with_a_sliver())
        .with_mesh(id("pipe"), cuboid([0.4, -1.0, 0.4], [0.6, 2.0, 0.6]));
    let measured = measure(geometry, "box", "pipe");
    assert!(measured.separation_metres().abs() < f64::EPSILON);
    let depth = measured.penetration_metres().expect("the box is closed");
    assert!((depth - 0.5).abs() < 1e-9, "depth {depth}");
}

/// An open sheet holding a zero-area triangle along one edge measures as
/// the sheet: the sliver adds no surface and decides no distance.
#[test]
fn an_open_sheet_with_a_zero_area_triangle_is_measured_as_the_sheet() {
    let mut sheet = quad(1.5);
    sheet.positions.push(Point3::new(0.5, 0.0, 1.5));
    sheet.indices.extend([0, 4, 1]);
    let with_sliver = AxiolidGeometry::new()
        .with_mesh(id("sheet"), sheet)
        .with_mesh(id("box"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    let plain = AxiolidGeometry::new()
        .with_mesh(id("sheet"), quad(1.5))
        .with_mesh(id("box"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    let measured = measure(with_sliver, "sheet", "box");
    assert!((measured.separation_metres() - 0.5).abs() < 1e-12);
    assert_same_measurement(&measured, &measure(plain, "sheet", "box"), true);
}

/// A mesh whose every triangle has zero area bounds no surface. It is
/// refused with that reason, never as a bare "unavailable".
#[test]
fn a_mesh_of_zero_area_triangles_only_is_refused_by_name() {
    let wire = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ],
        vec![0, 1, 2, 2, 1, 0],
    );
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("wire"), wire)
            .with_mesh(id("box"), cuboid([0.0, 0.0, 1.0], [1.0, 1.0, 2.0])),
    );
    let refused = service
        .measure_proximity(&ProximityRequest::try_new(id("wire"), id("box")).unwrap())
        .unwrap_err();
    let ProximityError::Refused(reason) = refused else {
        panic!("refused by name expected, got {refused:?}");
    };
    assert!(reason.contains("zero area"), "{reason}");
}

/// A tessellated column's volumes widen by the band within its chord
/// deviation, so the true cylinder's volumes lie inside.
#[test]
fn a_tessellated_column_widens_its_volumes() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_tessellated_mesh(id("column"), column([2.0, 2.0], 0.2, [0.0, 4.0], 24), 0.002);
    let volume = measure(geometry, "column", "slab")
        .intersection_volume()
        .unwrap();
    let cylinder = std::f64::consts::PI * 0.2 * 0.2;
    let shared = volume.shared();
    assert!(
        shared.lower_cubic_metres() <= cylinder * 0.2
            && shared.upper_cubic_metres() >= cylinder * 0.2,
        "{shared:?}"
    );
    assert!(!shared.is_exact());
    let own = volume.subject();
    assert!(
        own.lower_cubic_metres() <= cylinder * 4.0 && own.upper_cubic_metres() >= cylinder * 4.0,
        "{own:?}"
    );
}

/// A box of `min`..`max` in its own frame, turned by `angle` about the
/// vertical axis through the origin. A rotation keeps the outward winding.
fn turned_box(min: [f64; 3], max: [f64; 3], angle: f64) -> TriMesh {
    let (sin, cos) = angle.sin_cos();
    let local = cuboid(min, max);
    let positions = local
        .positions
        .iter()
        .map(|point| {
            let [u, v, z] = point.to_array();
            Point3::new(u * cos - v * sin, u * sin + v * cos, z)
        })
        .collect();
    TriMesh::new(positions, local.indices.clone())
}

/// A slab edge sunk 10 mm into a wall standing at 30°: along the world
/// axes the intersection reaches metres, along the wall's own thickness
/// 10 mm, along its length the slab's 3 m, and up the slab's 0.2 m.
#[test]
fn extents_along_a_walls_own_axes_measure_its_thickness() {
    let angle = std::f64::consts::PI / 6.0;
    let (sin, cos) = angle.sin_cos();
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("wall"),
            turned_box([-2.0, -0.1, 0.0], [2.0, 0.1, 3.0], angle),
        )
        .with_mesh(
            id("slab"),
            turned_box([-1.5, 0.09, 1.0], [1.5, 3.09, 1.2], angle),
        );
    let direction = |vector| MetricDirection::try_new(vector).unwrap();
    let request = OverlapAlongRequest::try_new(
        id("slab"),
        id("wall"),
        vec![
            direction([cos, sin, 0.0]),
            direction([-sin, cos, 0.0]),
            direction([0.0, 0.0, 1.0]),
            direction([1.0, 0.0, 0.0]),
        ],
    )
    .unwrap();
    let service = AxiolidProximityService::new(geometry.clone());
    let measured = service.measure_overlap_along(&request).expect("solids");
    assert!(measured.evidence().exact);
    let close = |interval: LengthInterval, expected: f64| {
        assert!(
            interval.lower_metres() <= expected + 1e-9
                && interval.upper_metres() >= expected - 1e-9
                && interval.upper_metres() - interval.lower_metres() < 1e-9,
            "{interval:?}, expected {expected}"
        );
    };
    let [length, thickness, height, world_x] = measured.extents() else {
        panic!("four extents expected");
    };
    close(*length, 3.0);
    close(*thickness, 0.01);
    close(*height, 0.2);
    assert!(world_x.lower_metres() > 2.0, "{world_x:?}");

    // Along the world's x axis the answer is the world-axis extent.
    let world = measure(geometry, "slab", "wall")
        .overlap_extents()
        .expect("solids");
    assert_eq!(world.x(), *world_x);

    // Two open surfaces share no volume: nothing to measure along.
    let open = AxiolidGeometry::new()
        .with_mesh(id("a"), quad(0.5))
        .with_mesh(id("b"), quad(0.5));
    let request =
        OverlapAlongRequest::try_new(id("a"), id("b"), vec![direction([0.0, 0.0, 1.0])]).unwrap();
    assert_eq!(
        AxiolidProximityService::new(open).measure_overlap_along(&request),
        Err(ProximityError::Unavailable)
    );
}

#[test]
fn a_closed_body_encloses_its_certified_volume() {
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("wall"), wall())
            .with_tessellated_mesh(id("column"), column([0.0, 0.0], 0.5, [0.0, 3.0], 32), 0.01)
            .with_mesh(id("sheet"), quad(0.0))
            .with_no_body(id("storey")),
    );
    let volume = service.measure_body_volume(&id("wall")).unwrap();
    assert_eq!(volume.object(), &id("wall"));
    assert!(volume.evidence().exact && volume.fidelity().is_exact());
    assert_volume(volume.volume(), 2.4);

    // A tessellated body is widened by its chord band, so the true
    // cylinder's volume lies inside and the evidence is approximate.
    let volume = service.measure_body_volume(&id("column")).unwrap();
    let true_volume = std::f64::consts::PI * 0.25 * 3.0;
    assert!(!volume.evidence().exact);
    assert!(
        volume.volume().lower_cubic_metres() < true_volume
            && true_volume < volume.volume().upper_cubic_metres(),
        "{volume:?}"
    );

    // An open surface encloses nothing; a bodiless or unknown object has no
    // volume to measure.
    assert_eq!(
        service.measure_body_volume(&id("sheet")),
        Err(ProximityError::Refused(
            "the body's mesh is an open surface, which encloses no volume"
        ))
    );
    assert_eq!(
        service.measure_body_volume(&id("storey")),
        Err(ProximityError::NoBody)
    );
    assert_eq!(
        service.measure_body_volume(&id("ghost")),
        Err(ProximityError::Unavailable)
    );
}

/// Closed boxes side by side in one mesh, each with its own corners, as a
/// body of several items is meshed.
fn shells(boxes: &[([f64; 3], [f64; 3])]) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for &(min, max) in boxes {
        let part = cuboid(min, max);
        let offset = u32::try_from(positions.len()).unwrap();
        positions.extend(part.positions);
        indices.extend(part.indices.into_iter().map(|index| index + offset));
    }
    TriMesh::new(positions, indices)
}

/// A table of a top and a leg that runs into it (#312): one closed mesh
/// whose triangles cross where the two meet, which the volume kernel
/// refuses as a whole. Measured shell by shell, a table inside a room lies
/// wholly in it, one in the next room shares nothing with it, and one
/// across the wall between them stays undecided in between.
#[test]
fn a_body_of_overlapping_shells_is_measured_shell_by_shell() {
    let table = |x: f64| {
        shells(&[
            ([x, 1.0, 0.7], [x + 1.0, 2.0, 0.75]),
            ([x + 0.4, 1.4, 0.0], [x + 0.5, 1.5, 0.72]),
        ])
    };
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("inside"), table(1.0))
        .with_mesh(id("next door"), table(5.0))
        .with_mesh(id("across"), table(3.5));
    let service = AxiolidProximityService::new(geometry);
    let volume = |table: &str| {
        service
            .measure_proximity(&ProximityRequest::try_new(id(table), id("room")).unwrap())
            .unwrap()
            .intersection_volume()
            .unwrap_or_else(|| panic!("{table}: no volume"))
    };
    let top = 0.05;
    let leg = 0.1 * 0.1 * 0.72;
    let inside = volume("inside");
    // The shells overlap, so the table's own volume lies between its top
    // and the sum of both; the share is bounded by what lies outside.
    assert!(
        inside.subject_outside().is_some(),
        "measured as a whole: {inside:?}"
    );
    let own = inside.subject();
    assert!(own.lower_cubic_metres() <= top + leg - 0.1 * 0.1 * 0.02);
    assert!(own.upper_cubic_metres() >= top + leg - 1e-12);
    let (lower, upper) = inside.ratio_of_smaller();
    assert!(lower > 0.999_999 && upper >= 1.0, "{lower} {upper}");
    let (lower, upper) = volume("next door").ratio_of_smaller();
    assert!(lower == 0.0 && upper < 1e-9, "{lower} {upper}");
    // Half the top outside, the leg inside: a share between, undecided.
    let (lower, upper) = volume("across").ratio_of_smaller();
    assert!(
        lower < 0.6 && upper > 0.4 && upper < 0.99,
        "{lower} {upper}"
    );
    // The reversed request reads the same.
    let reversed = service
        .measure_proximity(&ProximityRequest::try_new(id("room"), id("inside")).unwrap())
        .unwrap()
        .intersection_volume()
        .unwrap();
    assert!(reversed.ratio_of_smaller().0 > 0.999_999);
}

/// Shells are read only where each is a closed solid: one open shell
/// leaves the volume unmeasured.
#[test]
fn a_body_of_shells_one_of_them_open_has_no_volume() {
    let mut body = shells(&[
        ([1.0, 1.0, 0.7], [2.0, 2.0, 0.75]),
        ([1.4, 1.4, 0.0], [1.5, 1.5, 0.72]),
    ]);
    // Drop the leg's bottom: its shell is open, the whole no longer closed.
    body.indices.drain(36..42);
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("table"), body);
    let measured = measure(geometry, "table", "room");
    assert!(measured.intersection_volume().is_none());
}

/// A counter-clockwise profile in (x, z), its caps triangulated by `caps`,
/// extruded over y from `y0` to `y1`: a closed, outward mesh.
fn extruded(profile: &[[f64; 2]], caps: &[[u32; 3]], y0: f64, y1: f64) -> TriMesh {
    let n = u32::try_from(profile.len()).unwrap();
    let mut positions: Vec<Point3> = profile
        .iter()
        .map(|[x, z]| Point3::new(*x, y0, *z))
        .collect();
    positions.extend(profile.iter().map(|[x, z]| Point3::new(*x, y1, *z)));
    let mut indices = Vec::new();
    for [a, b, c] in caps {
        indices.extend([*a, *b, *c, a + n, c + n, b + n]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n]);
    }
    TriMesh::new(positions, indices)
}

/// A layer 4 m long, 3 m high and 0.1 m thick with a door hole 1 m wide and
/// 2.1 m high already cut. Its vertex centroid lies in the hole.
fn layer_with_a_door_hole() -> TriMesh {
    extruded(
        &[
            [0.0, 0.0],
            [1.5, 0.0],
            [1.5, 2.1],
            [2.5, 2.1],
            [2.5, 0.0],
            [4.0, 0.0],
            [4.0, 3.0],
            [0.0, 3.0],
        ],
        &[
            [0, 1, 2],
            [0, 2, 7],
            [2, 3, 7],
            [3, 6, 7],
            [3, 4, 6],
            [4, 5, 6],
        ],
        -0.1,
        0.0,
    )
}

/// The opening box spanning x `x0..x1`, 2.1 m high, deeper than the layer.
fn door_opening(x0: f64, x1: f64) -> TriMesh {
    extruded(
        &[[x0, 0.0], [x1, 0.0], [x1, 2.1], [x0, 2.1]],
        &[[0, 1, 2], [0, 2, 3]],
        -0.2,
        0.1,
    )
}

/// The opening filling a hole already cut in the layer only touches it: the
/// layer's centroid lies in the hole, inside the opening, but it is no point
/// of the layer and witnesses nothing (#315).
#[test]
fn an_opening_filling_a_hole_already_cut_touches_the_layer() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("layer"), layer_with_a_door_hole())
        .with_mesh(id("opening"), door_opening(1.5, 2.5));
    for (subject, counterpart) in [("layer", "opening"), ("opening", "layer")] {
        let measured = measure(geometry.clone(), subject, counterpart);
        assert!(measured.separation_metres().abs() < f64::EPSILON);
        assert_eq!(measured.penetration_metres(), Some(0.0), "{subject}");
        assert_eq!(measured.containment(), None);
        let shared = measured.intersection_volume().expect("solids").shared();
        assert!(shared.upper_cubic_metres() < 1e-12, "{shared:?}");
    }
}

/// An opening reaching 0.05 m past the hole's edge into the layer is
/// witnessed 0.05 m deep, both ways.
#[test]
fn an_opening_overlapping_the_layer_reports_its_depth() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("layer"), layer_with_a_door_hole())
        .with_mesh(id("opening"), door_opening(1.45, 2.5));
    for (subject, counterpart) in [("layer", "opening"), ("opening", "layer")] {
        let measured = measure(geometry.clone(), subject, counterpart);
        let depth = measured.penetration_metres().expect("solids");
        assert!((depth - 0.05).abs() < 1e-9, "{subject}: {depth}");
        let shared = measured.intersection_volume().expect("solids").shared();
        // 0.05 m wide, 2.1 m high, the layer's 0.1 m thick.
        assert!(
            (shared.lower_cubic_metres() - 0.0105).abs() < 1e-9,
            "{shared:?}"
        );
    }
}

/// A square frame 3 m across with a 1 m square hole, `z0..z1` high: a ring,
/// whose vertex centroid lies in its hole.
fn frame(z0: f64, z1: f64) -> TriMesh {
    let outer = [[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0]];
    let inner = [[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]];
    // 0..4 outer bottom, 4..8 outer top, 8..12 inner bottom, 12..16 inner top.
    let mut positions = Vec::new();
    for (ring, z) in [(outer, z0), (outer, z1), (inner, z0), (inner, z1)] {
        positions.extend(ring.iter().map(|[x, y]| Point3::new(*x, *y, z)));
    }
    let (ob, ot, ib, it) = (0, 4, 8, 12);
    let mut indices = Vec::new();
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        // The caps: the quad outer i, outer j, inner j, inner i, counter-
        // clockwise from above.
        indices.extend([ot + i, ot + j, it + j, ot + i, it + j, it + i]);
        indices.extend([ob + i, ib + j, ob + j, ob + i, ib + i, ib + j]);
        // The outer wall faces away from the hole, the inner one into it.
        indices.extend([ob + i, ob + j, ot + j, ob + i, ot + j, ot + i]);
        indices.extend([ib + i, it + j, ib + j, ib + i, it + i, it + j]);
    }
    TriMesh::new(positions, indices)
}

/// A ring is witnessed through its own thickness, never at its centroid.
#[test]
fn a_ring_is_witnessed_only_at_its_own_points() {
    // A box filling the ring's hole touches it.
    let filled = AxiolidGeometry::new()
        .with_mesh(id("ring"), frame(0.0, 0.2))
        .with_mesh(id("plug"), cuboid([1.0, 1.0, 0.0], [2.0, 2.0, 0.2]));
    for (subject, counterpart) in [("ring", "plug"), ("plug", "ring")] {
        let measured = measure(filled.clone(), subject, counterpart);
        assert_eq!(measured.penetration_metres(), Some(0.0), "{subject}");
        let shared = measured.intersection_volume().expect("solids").shared();
        assert!(shared.upper_cubic_metres() < 1e-12, "{shared:?}");
    }
    // A duplicate ring has no surface point off the other's surface and its
    // centroid in the hole of both; the chord through its 0.2 m thickness
    // witnesses it half that deep.
    let duplicate = AxiolidGeometry::new()
        .with_mesh(id("ring"), frame(0.0, 0.2))
        .with_mesh(id("copy"), frame(0.0, 0.2));
    let measured = measure(duplicate, "ring", "copy");
    let depth = measured.penetration_metres().expect("solids");
    assert!((depth - 0.1).abs() < 1e-9, "{depth}");
    // A box crossing one side of the ring, 0.3 m in from its outer face.
    let crossing = AxiolidGeometry::new()
        .with_mesh(id("ring"), frame(0.0, 0.2))
        .with_mesh(id("box"), cuboid([1.0, -1.0, -0.5], [2.0, 0.3, 0.5]));
    for (subject, counterpart) in [("ring", "box"), ("box", "ring")] {
        let measured = measure(crossing.clone(), subject, counterpart);
        let depth = measured.penetration_metres().expect("solids");
        assert!((depth - 0.3).abs() < 1e-9, "{subject}: {depth}");
        let shared = measured.intersection_volume().expect("solids").shared();
        assert!(
            (shared.lower_cubic_metres() - 0.06).abs() < 1e-9,
            "{shared:?}"
        );
    }
}
