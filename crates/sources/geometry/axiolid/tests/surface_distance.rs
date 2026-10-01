//! Certified surface distance between two revisions of a body.
//!
//! The base and the revised revision live in two sessions, so each has its
//! own service: the revised body's surface is handed out by one and
//! measured against the base body by the other, in world coordinates.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    ProximityError, ProximityService, SurfaceDirection, SurfaceDistanceEvidence,
    SurfaceDistanceRequest,
};
use axioval_ir::{ObjectId, SourceId};

fn base(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "base").unwrap(), local).unwrap()
}

fn revised(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "revised").unwrap(), local).unwrap()
}

/// Quads `[a, b, c, d]` as a triangle mesh, two triangles each; `reversed`
/// lists them, and each quad's corners, the other way round, as a
/// re-export may.
fn mesh(quads: &[[[f64; 3]; 4]], reversed: bool) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut ordered: Vec<[[f64; 3]; 4]> = quads.to_vec();
    if reversed {
        ordered.reverse();
        for quad in &mut ordered {
            quad.reverse();
        }
    }
    for quad in ordered {
        let first = u32::try_from(positions.len()).unwrap();
        positions.extend(quad.iter().map(|[x, y, z]| Point3::new(*x, *y, *z)));
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    TriMesh::new(positions, indices)
}

/// A rectangle in the plane `axis = at`, spanning `u` and `v` along the
/// other two axes in order.
fn rectangle(axis: usize, at: f64, u: [f64; 2], v: [f64; 2]) -> [[f64; 3]; 4] {
    let point = |a: f64, b: f64| {
        let mut point = [0.0; 3];
        point[axis] = at;
        point[(axis + 1) % 3] = a;
        point[(axis + 2) % 3] = b;
        point
    };
    [
        point(u[0], v[0]),
        point(u[1], v[0]),
        point(u[1], v[1]),
        point(u[0], v[1]),
    ]
}

/// A 4 m long, 0.2 m thick, 3 m high wall from `x`, with a door-sized
/// opening through it from `opening` to `opening + 1` along x, 0.5 m to
/// 2.5 m up.
fn wall_with_opening(start: f64, opening: f64, reversed: bool) -> TriMesh {
    let (x0, x1, y1, z1) = (start, start + 4.0, 0.2, 3.0);
    let (left, right, sill, head) = (opening, opening + 1.0, 0.5, 2.5);
    let mut quads = Vec::new();
    // Both faces around the opening (axis 1: u along z, v along x).
    for face in [0.0, y1] {
        quads.push(rectangle(1, face, [0.0, z1], [x0, left]));
        quads.push(rectangle(1, face, [0.0, z1], [right, x1]));
        quads.push(rectangle(1, face, [0.0, sill], [left, right]));
        quads.push(rectangle(1, face, [head, z1], [left, right]));
    }
    // Ends, top and bottom.
    for end in [x0, x1] {
        quads.push(rectangle(0, end, [0.0, y1], [0.0, z1]));
    }
    for level in [0.0, z1] {
        quads.push(rectangle(2, level, [x0, x1], [0.0, y1]));
    }
    // The opening's reveals.
    for side in [left, right] {
        quads.push(rectangle(0, side, [0.0, y1], [sill, head]));
    }
    for level in [sill, head] {
        quads.push(rectangle(2, level, [left, right], [0.0, y1]));
    }
    mesh(&quads, reversed)
}

/// The base wall in its session and the revised one in another, measured
/// as a comparison measures them.
fn measure(before: TriMesh, after: TriMesh) -> SurfaceDistanceEvidence {
    let base_service =
        AxiolidProximityService::new(AxiolidGeometry::new().with_mesh(base("#1"), before));
    let revised_service =
        AxiolidProximityService::new(AxiolidGeometry::new().with_mesh(revised("#9"), after));
    let surface = revised_service.body_surface(&revised("#9")).unwrap();
    let request = SurfaceDistanceRequest::try_new(base("#1"), Arc::new(surface), 1e-4).unwrap();
    base_service.measure_surface_distance(&request).unwrap()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The opening moved 0.5 m along the wall: the bounds stay put, but the
/// base opening's far reveal now stands in the middle of the revised
/// opening, 0.5 m from either of its reveals.
#[test]
fn an_opening_moved_within_unchanged_bounds_is_as_far_as_it_moved() {
    let (before, after) = (
        wall_with_opening(0.0, 1.0, false),
        wall_with_opening(0.0, 1.5, false),
    );
    let bounds = |mesh: TriMesh, object: ObjectId| {
        AxiolidProximityService::new(AxiolidGeometry::new().with_mesh(object.clone(), mesh))
            .bounds(&object)
            .unwrap()
            .bounds()
    };
    assert_eq!(
        bounds(before.clone(), base("#1")),
        bounds(after.clone(), revised("#9"))
    );
    let measured = measure(before, after);
    let interval = measured.distance();
    assert!(
        interval.lower_metres() <= 0.5 + 1e-12 && interval.upper_metres() >= 0.5 - 1e-12,
        "{interval:?}"
    );
    assert!(
        interval.upper_metres() - interval.lower_metres() <= 1e-4 + 1e-12,
        "{interval:?}"
    );
    assert!(measured.evidence().exact);
    // The witness is a point of one wall that far from the other.
    let (_, witness) = measured.witness();
    assert!(
        (distance(witness.from(), witness.to()) - 0.5).abs() < 1e-3,
        "{witness:?}"
    );
}

/// A re-export lists the same faces in another order: the surfaces are one
/// and the distance is zero up to rounding.
#[test]
fn a_re_exported_identical_wall_is_no_distance_away() {
    let measured = measure(
        wall_with_opening(0.0, 1.0, false),
        wall_with_opening(0.0, 1.0, true),
    );
    let interval = measured.distance();
    assert!(interval.upper_metres() < 1e-9, "{interval:?}");
    assert!(interval.lower_metres() <= 0.0, "{interval:?}");
}

/// A wall moved 0.25 m along itself is exactly 0.25 m away at its ends; the
/// certified interval holds that value, so a tolerance of 0.25 m is
/// straddled.
#[test]
fn a_shift_by_the_tolerance_is_held_by_the_interval() {
    let measured = measure(
        wall_with_opening(0.0, 1.0, false),
        wall_with_opening(0.25, 1.25, false),
    );
    let interval = measured.distance();
    assert!(
        interval.lower_metres() <= 0.25 && interval.upper_metres() >= 0.25,
        "{interval:?}"
    );
    // Both sides stray as far: the base's start and the revision's end.
    assert!(measured.forward().interval().lower_metres() > 0.249);
    assert!(measured.backward().interval().lower_metres() > 0.249);
    assert!(matches!(
        measured.witness().0,
        SurfaceDirection::FromSubject | SurfaceDirection::FromCounterpart
    ));
}

/// A tessellation's mesh may stray from its true surface by more than its
/// chord deviation says, so no distance to it is certified.
#[test]
fn a_tessellated_body_is_refused() {
    let wall = wall_with_opening(0.0, 1.0, false);
    let tessellated = AxiolidProximityService::new(AxiolidGeometry::new().with_tessellated_mesh(
        revised("#9"),
        wall.clone(),
        0.001,
    ));
    let surface = tessellated.body_surface(&revised("#9")).unwrap();
    assert!(!surface.fidelity().is_exact());
    let exact =
        AxiolidProximityService::new(AxiolidGeometry::new().with_mesh(base("#1"), wall.clone()));
    let request = SurfaceDistanceRequest::try_new(base("#1"), Arc::new(surface), 1e-4).unwrap();
    assert_eq!(
        exact.measure_surface_distance(&request),
        Err(ProximityError::EvidenceFidelityMismatch)
    );

    // Nor the other way round: a tessellated subject.
    let curved = AxiolidProximityService::new(AxiolidGeometry::new().with_tessellated_mesh(
        base("#1"),
        wall.clone(),
        0.001,
    ));
    let straight =
        AxiolidProximityService::new(AxiolidGeometry::new().with_mesh(revised("#9"), wall));
    let request = SurfaceDistanceRequest::try_new(
        base("#1"),
        Arc::new(straight.body_surface(&revised("#9")).unwrap()),
        1e-4,
    )
    .unwrap();
    assert_eq!(
        curved.measure_surface_distance(&request),
        Err(ProximityError::EvidenceFidelityMismatch)
    );
}

#[test]
fn a_bodiless_object_has_no_surface() {
    let service = AxiolidProximityService::new(AxiolidGeometry::new().with_no_body(base("storey")));
    assert_eq!(
        service.body_surface(&base("storey")),
        Err(ProximityError::NoBody)
    );
    assert_eq!(
        service.body_surface(&base("unknown")),
        Err(ProximityError::Unavailable)
    );
}
