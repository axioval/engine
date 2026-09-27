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
        .measure_proximity(&ProximityRequest::try_new(id(subject), id(counterpart)).unwrap())
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
    assert!((measured.plan_overlap_square_metres() - 0.02).abs() < 1e-6);
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
    assert!((measured.plan_overlap_square_metres() - 0.8).abs() < 1e-6);
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
    assert!(measured.plan_overlap_square_metres().abs() < f64::EPSILON);
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
    let request = |subject: &str| ProximityRequest::try_new(id(subject), id("wall")).unwrap();
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
