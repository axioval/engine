//! Certified plan relations between curved bodies with exact boundaries.
//!
//! Round columns of radius 0.2 m are meshed with 16 chords, which fall up to
//! `0.2 · (1 − cos(π/16))` ≈ 3.8 mm inside the circle. On the meshes alone a
//! plan distance, a plan overlap or the footprint relation of a vertical
//! distance stays open within that deviation. With both exact boundaries
//! registered, the kernel's certified plan measurements (`plan_boundary_distance`,
//! `plan_boundary_clearance`, `plan_overlap`) decide them against closed forms:
//! two columns whose axes stand 1 m apart are 0.6 m apart in plan whatever
//! their heights, and a column standing on a slab overlaps it in plan.

// Lower bounds clamped at zero and exact points are compared exactly.
#![allow(clippy::float_cmp)]

use std::f64::consts::{PI, TAU};

use axiolid_construct::boolean_exact::{ArcPrism, boolean_arc_prisms_exact};
use axiolid_core::{BooleanOperator, Point2, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_overlay::ArcRing;
use axioval_axiolid::proximity::CERTIFIED_ACCURACY_METRES;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    ProximityError, ProximityProjection, ProximityRequest, ProximityService, VerticalDirection,
    VerticalSurfaces,
};
use axioval_ir::{ObjectId, SourceId};

const RADIUS: f64 = 0.2;
const SIDES: u32 = 16;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// How far the chords fall inside the circle.
fn chord_deviation() -> f64 {
    RADIUS * (1.0 - (PI / f64::from(SIDES)).cos())
}

/// A closed exact prism over a section between `bottom` and `top` (the
/// intersection with a copy scaled about the centre is the section itself).
fn exact_prism(section: impl Fn(f64) -> ArcRing, bottom: f64, top: f64) -> axiolid_brep::ExactBRep {
    let prism = |scale: f64| ArcPrism {
        section: section(scale),
        bottom,
        top,
    };
    boolean_arc_prisms_exact(
        &prism(1.0),
        &prism(2.0),
        BooleanOperator::Intersection,
        Tolerance::METRE,
    )
    .expect("an exact prism")
}

fn exact_column(centre: [f64; 2], levels: [f64; 2]) -> axiolid_brep::ExactBRep {
    exact_prism(
        |scale| ArcRing::circle(Point2::new(centre[0], centre[1]), RADIUS * scale),
        levels[0],
        levels[1],
    )
}

/// An exact box over `min..max`.
fn exact_box(min: [f64; 3], max: [f64; 3]) -> axiolid_brep::ExactBRep {
    let centre = (f64::midpoint(min[0], max[0]), f64::midpoint(min[1], max[1]));
    let corners = [
        (min[0], min[1]),
        (max[0], min[1]),
        (max[0], max[1]),
        (min[0], max[1]),
    ];
    exact_prism(
        |scale| {
            ArcRing::from_points(
                &corners
                    .iter()
                    .map(|(x, y)| {
                        Point2::new(
                            centre.0 + (x - centre.0) * scale,
                            centre.1 + (y - centre.1) * scale,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        },
        min[2],
        max[2],
    )
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

/// The column's chord mesh between two levels, vertices at angles zero and
/// π, so the mesh's plan extremes along x are the circle's.
fn column_mesh(centre: [f64; 2], levels: [f64; 2]) -> TriMesh {
    let mut positions = Vec::new();
    for level in levels {
        for side in 0..SIDES {
            let angle = TAU * f64::from(side) / f64::from(SIDES);
            positions.push(Point3::new(
                centre[0] + RADIUS * angle.cos(),
                centre[1] + RADIUS * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(centre[0], centre[1], levels[0]));
    positions.push(Point3::new(centre[0], centre[1], levels[1]));
    let (bottom_centre, top_centre) = (2 * SIDES, 2 * SIDES + 1);
    let mut indices = Vec::new();
    for side in 0..SIDES {
        let next = (side + 1) % SIDES;
        let (b0, b1, t0, t1) = (side, next, side + SIDES, next + SIDES);
        indices.extend([b0, b1, t1, b0, t1, t0]);
        indices.extend([bottom_centre, b1, b0]);
        indices.extend([top_centre, t0, t1]);
    }
    TriMesh::new(positions, indices)
}

/// A column at `centre` between `levels`, tessellated, with its exact
/// boundary when `certified`.
fn column(
    geometry: AxiolidGeometry,
    name: &str,
    centre: [f64; 2],
    levels: [f64; 2],
    certified: bool,
) -> AxiolidGeometry {
    let geometry =
        geometry.with_tessellated_mesh(id(name), column_mesh(centre, levels), chord_deviation());
    if certified {
        geometry.with_exact_boundary(id(name), exact_column(centre, levels))
    } else {
        geometry
    }
}

/// Two columns whose axes stand 1 m apart in plan, one from 0 to 3 m, the
/// other from 4 to 6 m.
fn two_columns(certified: bool) -> AxiolidGeometry {
    let geometry = column(
        AxiolidGeometry::new(),
        "low",
        [0.0, 0.0],
        [0.0, 3.0],
        certified,
    );
    column(geometry, "high", [1.0, 0.0], [4.0, 6.0], certified)
}

/// The 0.2 m slab `x, y` in [0, 4], its top at z = 0, exact.
fn slab(geometry: AxiolidGeometry, certified: bool) -> AxiolidGeometry {
    let (min, max) = ([0.0, 0.0, -0.2], [4.0, 4.0, 0.0]);
    let geometry = geometry.with_mesh(id("slab"), cuboid(min, max));
    if certified {
        geometry.with_exact_boundary(id("slab"), exact_box(min, max))
    } else {
        geometry
    }
}

/// A 3 m column standing on the slab with its axis at `centre`.
fn column_on_slab(centre: [f64; 2], certified: bool) -> AxiolidGeometry {
    slab(
        column(
            AxiolidGeometry::new(),
            "column",
            centre,
            [0.0, 3.0],
            certified,
        ),
        certified,
    )
}

fn measure(
    geometry: AxiolidGeometry,
    subject: &str,
    counterpart: &str,
    projection: ProximityProjection,
) -> Result<(f64, f64), ProximityError> {
    let request = ProximityRequest::projected(id(subject), id(counterpart), projection).unwrap();
    AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .map(|measured| {
            assert!(
                !measured.evidence().exact,
                "a tessellated pair is never exact"
            );
            measured.interval_metres()
        })
}

fn vertical(offset: f64) -> ProximityProjection {
    ProximityProjection::Vertical {
        footprint_offset_metres: offset,
        direction: VerticalDirection::Either,
        surfaces: VerticalSurfaces::Extents,
    }
}

const OPEN: (f64, f64) = (0.0, f64::INFINITY);
const NONE: (f64, f64) = (f64::INFINITY, f64::INFINITY);

#[test]
fn two_columns_at_different_heights_are_0_6_m_apart_in_plan() {
    let horizontal = ProximityProjection::Horizontal;
    let (lower, upper) = measure(two_columns(false), "low", "high", horizontal).unwrap();
    let deviation = 2.0 * chord_deviation();
    assert!(
        (lower - (0.6 - deviation)).abs() < 1e-9 && (upper - (0.6 + deviation)).abs() < 1e-9,
        "the chords alone: [{lower}, {upper}]"
    );

    let (lower, upper) = measure(two_columns(true), "low", "high", horizontal).unwrap();
    assert!(
        lower <= 0.6 && 0.6 <= upper,
        "[{lower}, {upper}] must hold 0.6"
    );
    assert!(
        upper - lower <= 2.0 * CERTIFIED_ACCURACY_METRES,
        "[{lower}, {upper}] is wider than the accuracy asked"
    );
    assert!((lower - 0.6).abs() < 1e-6 && (upper - 0.6).abs() < 1e-6);
}

#[test]
fn the_footprint_relation_of_a_vertical_distance_is_certified() {
    // The extents are 1 m apart; the footprints 0.6 m. An offset of
    // 0.605 m relates them, 0.595 m does not; both lie inside the chords'
    // plan interval, so the meshes leave either open.
    let gap = 1.0;
    let deviation = 2.0 * chord_deviation();
    for offset in [0.605, 0.595] {
        let (lower, upper) = measure(two_columns(false), "low", "high", vertical(offset)).unwrap();
        assert!((lower - (gap - deviation)).abs() < 1e-9, "{lower}");
        assert!(upper.is_infinite(), "the chords leave {offset} m open");
    }
    let (lower, upper) = measure(two_columns(true), "low", "high", vertical(0.605)).unwrap();
    assert!(
        (lower - (gap - deviation)).abs() < 1e-9 && (upper - (gap + deviation)).abs() < 1e-9,
        "related: [{lower}, {upper}]"
    );
    assert_eq!(
        measure(two_columns(true), "low", "high", vertical(0.595)),
        Ok(NONE),
        "certainly farther than the offset: unrelated"
    );
}

#[test]
fn columns_apart_in_plan_do_not_overlap() {
    // 0.6 m apart: the meshes already deny overlap, and the certificate
    // agrees.
    for certified in [false, true] {
        assert_eq!(
            measure(
                two_columns(certified),
                "low",
                "high",
                ProximityProjection::PlanOverlap
            ),
            Ok(NONE)
        );
    }
}

#[test]
fn a_column_standing_on_a_slab_overlaps_it_in_plan() {
    for certified in [false, true] {
        let geometry = || column_on_slab([2.0, 2.0], certified);
        assert_eq!(
            measure(
                geometry(),
                "column",
                "slab",
                ProximityProjection::PlanOverlap
            ),
            Ok((0.0, 0.0))
        );
        let (lower, upper) = measure(
            geometry(),
            "column",
            "slab",
            ProximityProjection::Horizontal,
        )
        .unwrap();
        assert_eq!(lower, 0.0);
        if certified {
            // Zero, the column standing inside the slab's footprint.
            assert!(upper <= 2.0 * CERTIFIED_ACCURACY_METRES, "{upper}");
        } else {
            assert!((upper - chord_deviation()).abs() < 1e-12, "{upper}");
        }
        // Standing on it: the extents touch, the footprints are related.
        let (lower, upper) = measure(geometry(), "column", "slab", vertical(0.0)).unwrap();
        assert_eq!(lower, 0.0);
        assert!((upper - chord_deviation()).abs() < 1e-12, "{upper}");
    }
}

#[test]
fn a_column_just_off_the_slab_edge_is_certified_apart() {
    // 3 mm off the edge, within the chord deviation of the slab.
    let centre = [4.203, 2.0];
    assert_eq!(
        measure(
            column_on_slab(centre, false),
            "column",
            "slab",
            ProximityProjection::PlanOverlap
        ),
        Ok(OPEN)
    );
    assert_eq!(
        measure(
            column_on_slab(centre, true),
            "column",
            "slab",
            ProximityProjection::PlanOverlap
        ),
        Ok(NONE)
    );
    assert_eq!(
        measure(
            column_on_slab(centre, true),
            "column",
            "slab",
            vertical(0.0)
        ),
        Ok(NONE)
    );
    let (lower, upper) = measure(
        column_on_slab(centre, true),
        "column",
        "slab",
        ProximityProjection::Horizontal,
    )
    .unwrap();
    let expected = centre[0] - RADIUS - 4.0;
    assert!(
        lower <= expected && expected <= upper && upper - lower <= 2.0 * CERTIFIED_ACCURACY_METRES,
        "[{lower}, {upper}] must hold {expected}"
    );
}

#[test]
fn one_boundary_alone_certifies_nothing() {
    let geometry = || {
        column(AxiolidGeometry::new(), "low", [0.0, 0.0], [0.0, 3.0], true).with_tessellated_mesh(
            id("high"),
            column_mesh([1.0, 0.0], [4.0, 6.0]),
            chord_deviation(),
        )
    };
    for projection in [ProximityProjection::Horizontal, vertical(0.605)] {
        assert_eq!(
            measure(geometry(), "low", "high", projection),
            measure(two_columns(false), "low", "high", projection)
        );
    }
}

#[test]
fn an_exact_pair_stays_uncertified() {
    // Square columns meshed exactly: the plan distance is a point already.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("low"), cuboid([-0.2, -0.2, 0.0], [0.2, 0.2, 3.0]))
        .with_mesh(id("high"), cuboid([0.8, -0.2, 4.0], [1.2, 0.2, 6.0]))
        .with_exact_boundary(id("low"), exact_column([0.0, 0.0], [0.0, 3.0]))
        .with_exact_boundary(id("high"), exact_column([1.0, 0.0], [4.0, 6.0]));
    let request =
        ProximityRequest::projected(id("low"), id("high"), ProximityProjection::Horizontal)
            .unwrap();
    let measured = AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .unwrap();
    assert!(measured.evidence().exact);
    let (lower, upper) = measured.interval_metres();
    assert_eq!(lower, upper, "a point");
    assert!((lower - 0.6).abs() < 1e-12, "{lower}");
}

#[test]
fn a_sliver_over_the_slab_edge_stays_open() {
    // The column reaches 5 mm over the slab's edge at x = 4, less than the
    // chord deviation, so the mesh cannot tell a sliver from a gap. The
    // kernel shows overlap only from planar patches it isolates within its
    // step budget, which it does not for a sliver: the relation stays open
    // rather than guessed either way.
    for certified in [false, true] {
        let geometry = || column_on_slab([4.195, 2.0], certified);
        assert_eq!(
            measure(
                geometry(),
                "column",
                "slab",
                ProximityProjection::PlanOverlap
            ),
            Ok(OPEN)
        );
        let (lower, upper) = measure(geometry(), "column", "slab", vertical(0.0)).unwrap();
        assert_eq!(lower, 0.0);
        assert!(upper.is_infinite());
    }
}

#[test]
fn boundaries_that_miss_their_meshes_in_plan_refuse() {
    // The low column's boundary stands a metre away from its mesh.
    let geometry = || {
        column(AxiolidGeometry::new(), "high", [1.0, 0.0], [4.0, 6.0], true)
            .with_tessellated_mesh(
                id("low"),
                column_mesh([0.0, 0.0], [0.0, 3.0]),
                chord_deviation(),
            )
            .with_exact_boundary(id("low"), exact_column([-1.0, 0.0], [0.0, 3.0]))
    };
    assert_eq!(
        measure(geometry(), "low", "high", ProximityProjection::Horizontal),
        Err(ProximityError::InvalidMeasurement)
    );
    // The mesh stands deep inside the slab's footprint, the boundary a
    // metre off its edge: overlap shown on one, a gap on the other.
    let geometry = column_on_slab([2.0, 2.0], false)
        .with_exact_boundary(id("column"), exact_column([5.5, 2.0], [0.0, 3.0]))
        .with_exact_boundary(id("slab"), exact_box([0.0, 0.0, -0.2], [4.0, 4.0, 0.0]));
    assert_eq!(
        measure(geometry, "column", "slab", ProximityProjection::PlanOverlap),
        Err(ProximityError::InvalidMeasurement)
    );
    // The other way round: the mesh a metre off the edge, the boundary
    // standing on the slab.
    let geometry = column_on_slab([5.5, 2.0], false)
        .with_exact_boundary(id("column"), exact_column([3.9, 2.0], [0.0, 3.0]))
        .with_exact_boundary(id("slab"), exact_box([0.0, 0.0, -0.2], [4.0, 4.0, 0.0]));
    assert_eq!(
        measure(geometry, "column", "slab", ProximityProjection::PlanOverlap),
        Err(ProximityError::InvalidMeasurement)
    );
}
