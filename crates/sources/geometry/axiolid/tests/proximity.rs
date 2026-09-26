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
    BodyContainment, GeometryFidelity, ProximityError, ProximityRequest, ProximityService,
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
