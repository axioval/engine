//! Distances in a projection, measured over real Axiolid geometry.
//!
//! Each projection answers a different question: how far apart two bodies
//! are in plan, how far one sits above the other, whether their footprints
//! overlap. Exact meshes give points; tessellated ones give intervals and may
//! leave a relation open, never decide it on geometry they only approximate.

use std::f64::consts::TAU;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    ProjectedDistanceEvidence, ProximityError, ProximityProjection, ProximityRequest,
    ProximityService,
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

/// Two meshes as one body, for footprints no single box describes.
fn joined(first: TriMesh, second: &TriMesh) -> TriMesh {
    let offset = u32::try_from(first.positions.len()).unwrap();
    let mut positions = first.positions;
    positions.extend(second.positions.iter().copied());
    let mut indices = first.indices;
    indices.extend(second.indices.iter().map(|index| index + offset));
    TriMesh::new(positions, indices)
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

fn measure(
    geometry: &AxiolidGeometry,
    subject: &str,
    counterpart: &str,
    projection: ProximityProjection,
) -> ProjectedDistanceEvidence {
    AxiolidProximityService::new(geometry.clone())
        .measure_distance(
            &ProximityRequest::projected(id(subject), id(counterpart), projection).unwrap(),
        )
        .expect("measurable")
}

fn assert_point(measured: &ProjectedDistanceEvidence, expected: f64) {
    let (lower, upper) = measured.interval_metres();
    assert!(measured.evidence().exact, "exact geometry, exact evidence");
    if expected.is_infinite() {
        assert!(
            lower.is_infinite() && upper.is_infinite(),
            "{lower}..{upper}"
        );
    } else {
        assert!(
            (lower - expected).abs() < 1e-9 && (upper - expected).abs() < 1e-9,
            "{lower}..{upper}, expected {expected}"
        );
    }
}

fn vertical(offset: f64) -> ProximityProjection {
    ProximityProjection::Vertical {
        footprint_offset_metres: offset,
    }
}

#[test]
fn minimum_3d_is_the_separation_in_space() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("b"), cuboid([4.0, 0.0, 4.0], [5.0, 1.0, 5.0]));
    assert_point(
        &measure(&geometry, "a", "b", ProximityProjection::Minimum3d),
        18.0_f64.sqrt(),
    );
}

/// Bodies on different storeys are three metres apart in plan, whatever
/// their height.
#[test]
fn horizontal_distance_ignores_height() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("low"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("high"), cuboid([4.0, 0.0, 9.0], [5.0, 1.0, 10.0]));
    assert_point(
        &measure(&geometry, "low", "high", ProximityProjection::Horizontal),
        3.0,
    );
    let above = AxiolidGeometry::new()
        .with_mesh(id("low"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("high"), cuboid([0.5, 0.5, 9.0], [2.0, 2.0, 10.0]));
    assert_point(
        &measure(&above, "low", "high", ProximityProjection::Horizontal),
        0.0,
    );
}

/// The notch of an L-shaped footprint: its convex hull or its box would put
/// the post inside; the footprint itself is half a metre away.
#[test]
fn horizontal_distance_to_a_non_convex_footprint_is_exact() {
    let l_shape = joined(
        cuboid([0.0, 0.0, 0.0], [3.0, 1.0, 1.0]),
        &cuboid([0.0, 0.0, 0.0], [1.0, 3.0, 1.0]),
    );
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("l"), l_shape)
        .with_mesh(id("post"), cuboid([1.5, 1.5, 0.0], [2.0, 2.0, 1.0]));
    assert_point(
        &measure(&geometry, "post", "l", ProximityProjection::Horizontal),
        0.5,
    );
}

/// A single vertical sheet has no plan area; its footprint is a line.
#[test]
fn horizontal_distance_reaches_an_edge_on_sheet() {
    let sheet = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 3.0),
            Point3::new(0.0, 0.0, 3.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("sheet"), sheet)
        .with_mesh(id("box"), cuboid([2.0, 0.2, 5.0], [3.0, 0.8, 6.0]));
    assert_point(
        &measure(&geometry, "box", "sheet", ProximityProjection::Horizontal),
        2.0,
    );
}

#[test]
fn vertical_distance_needs_overlapping_footprints() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), cuboid([1.0, 1.0, 0.0], [1.4, 1.4, 2.6]))
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_mesh(id("beside"), cuboid([5.0, 0.0, 3.0], [6.0, 1.0, 3.2]));
    assert_point(&measure(&geometry, "column", "slab", vertical(0.0)), 0.4);
    // Nothing is above or below a body standing beside it.
    assert_point(
        &measure(&geometry, "column", "beside", vertical(0.0)),
        f64::INFINITY,
    );
}

/// A grown footprint reaches a body that is not directly above.
#[test]
fn a_footprint_offset_relates_bodies_that_are_nearly_above() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("lamp"), cuboid([0.0, 0.0, 2.0], [0.5, 0.5, 2.5]))
        .with_mesh(id("desk"), cuboid([1.0, 0.0, 0.0], [2.0, 1.0, 0.8]));
    assert_point(
        &measure(&geometry, "lamp", "desk", vertical(0.0)),
        f64::INFINITY,
    );
    assert_point(&measure(&geometry, "lamp", "desk", vertical(0.6)), 1.2);
    // The offset is exclusive: a footprint exactly at it only touches.
    assert_point(
        &measure(&geometry, "lamp", "desk", vertical(0.5)),
        f64::INFINITY,
    );
}

#[test]
fn plan_overlap_needs_positive_area() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("duct"), cuboid([3.0, 1.0, 5.0], [6.0, 1.5, 5.5]))
        .with_mesh(id("next-room"), cuboid([4.0, 0.0, 0.0], [8.0, 4.0, 3.0]))
        .with_mesh(id("away"), cuboid([9.0, 0.0, 0.0], [10.0, 1.0, 3.0]));
    assert_point(
        &measure(&geometry, "duct", "room", ProximityProjection::PlanOverlap),
        0.0,
    );
    // Rooms sharing a wall line meet in plan without overlapping.
    assert_point(
        &measure(
            &geometry,
            "next-room",
            "room",
            ProximityProjection::PlanOverlap,
        ),
        f64::INFINITY,
    );
    assert_point(
        &measure(&geometry, "away", "room", ProximityProjection::PlanOverlap),
        f64::INFINITY,
    );
}

/// Every projection on a tessellation is an interval, and inexact evidence.
#[test]
fn tessellated_projections_are_intervals() {
    let deviation = 0.002;
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_mesh(id("wall"), cuboid([3.0, 0.0, 0.0], [3.2, 4.0, 2.5]))
        .with_tessellated_mesh(
            id("column"),
            column([2.0, 2.0], 0.2, [0.0, 2.5], 24),
            deviation,
        );
    let horizontal = measure(&geometry, "column", "wall", ProximityProjection::Horizontal);
    let (lower, upper) = horizontal.interval_metres();
    assert!(!horizontal.evidence().exact);
    assert!(
        (lower - (0.8 - deviation)).abs() < 1e-9 && (upper - (0.8 + deviation)).abs() < 1e-9,
        "{lower}..{upper}"
    );
    let under = measure(&geometry, "column", "slab", vertical(0.0));
    let (lower, upper) = under.interval_metres();
    assert!(
        (lower - (0.5 - deviation)).abs() < 1e-9 && (upper - (0.5 + deviation)).abs() < 1e-9,
        "{lower}..{upper}"
    );
    // The column lies deep inside the slab's footprint: overlap is certain.
    let overlap = measure(
        &geometry,
        "column",
        "slab",
        ProximityProjection::PlanOverlap,
    );
    assert_eq!(overlap.interval_metres(), (0.0, 0.0));
    assert!(!overlap.evidence().exact);
}

/// A sliver of overlap thinner than the deviation may not exist on the true
/// surface; the relation stays open rather than being guessed either way.
#[test]
fn a_tessellated_overlap_within_the_deviation_is_open() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_tessellated_mesh(
            id("column"),
            column([4.195, 2.0], 0.2, [0.0, 2.5], 24),
            0.02,
        );
    let overlap = measure(
        &geometry,
        "column",
        "slab",
        ProximityProjection::PlanOverlap,
    );
    assert_eq!(overlap.interval_metres(), (0.0, f64::INFINITY));
    let under = measure(&geometry, "column", "slab", vertical(0.0));
    let (lower, upper) = under.interval_metres();
    assert!((lower - 0.48).abs() < 1e-9 && upper.is_infinite());
    // Well clear of the deviation, it is certainly not above.
    let clear = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
        .with_tessellated_mesh(id("column"), column([4.5, 2.0], 0.2, [0.0, 2.5], 24), 0.02);
    let apart = measure(&clear, "column", "slab", ProximityProjection::PlanOverlap);
    assert_eq!(apart.interval_metres(), (f64::INFINITY, f64::INFINITY));
}

#[test]
fn a_projected_request_is_not_a_clash_measurement() {
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
            .with_mesh(id("b"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0])),
    );
    let request =
        ProximityRequest::projected(id("a"), id("b"), ProximityProjection::Horizontal).unwrap();
    assert_eq!(
        service.measure_proximity(&request).unwrap_err(),
        ProximityError::UnsupportedProjection
    );
}
