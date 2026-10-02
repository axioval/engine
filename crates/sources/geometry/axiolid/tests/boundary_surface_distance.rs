//! Certified surface distance between two revisions' exact boundaries.
//!
//! A round column (radius 0.2 m, 3 m high) is meshed as a 16-gon, a
//! tessellation, so no distance between two revisions' meshes is certified.
//! With each revision's exact boundary registered beside its mesh, the
//! kernel's `boundary_hausdorff_distance` measures between the boundaries:
//! an identical copy is zero away up to rounding and a copy moved by a
//! millimetre a millimetre, both decided where the meshes decide nothing.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use axiolid_construct::boolean_exact::{ArcPrism, boolean_arc_prisms_exact};
use axiolid_core::{BooleanOperator, Point2, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_overlay::ArcRing;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    ExactBoundaryHandle, ProximityError, ProximityService, SurfaceBasis, SurfaceDistanceEvidence,
    SurfaceDistanceRequest,
};
use axioval_ir::{ObjectId, SourceId};

const RADIUS: f64 = 0.2;
const SIDES: u32 = 16;
const HEIGHT: f64 = 3.0;

fn base(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "base").unwrap(), local).unwrap()
}

fn revised(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "revised").unwrap(), local).unwrap()
}

/// How far the column's chords fall inside its circle, declared a little
/// above.
fn deviation() -> f64 {
    RADIUS * (1.0 - (PI / f64::from(SIDES)).cos()) * 1.01
}

/// A closed prism over `ring` from 0 to `HEIGHT`, as the kernel builds it
/// from an arc section (the intersection with a copy scaled about the
/// centre is the section itself).
fn exact_prism(section: impl Fn(f64) -> ArcRing) -> axiolid_brep::ExactBRep {
    let prism = |scale: f64| ArcPrism {
        section: section(scale),
        bottom: 0.0,
        top: HEIGHT,
    };
    boolean_arc_prisms_exact(
        &prism(1.0),
        &prism(2.0),
        BooleanOperator::Intersection,
        Tolerance::METRE,
    )
    .expect("an exact prism")
}

fn exact_column(centre: [f64; 2]) -> axiolid_brep::ExactBRep {
    exact_prism(|scale| ArcRing::circle(Point2::new(centre[0], centre[1]), RADIUS * scale))
}

/// The column's chord mesh, closed and outward.
fn column_mesh(centre: [f64; 2]) -> TriMesh {
    let mut positions = Vec::new();
    for level in [0.0, HEIGHT] {
        for side in 0..SIDES {
            let angle = TAU * f64::from(side) / f64::from(SIDES);
            positions.push(Point3::new(
                centre[0] + RADIUS * angle.cos(),
                centre[1] + RADIUS * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(centre[0], centre[1], 0.0));
    positions.push(Point3::new(centre[0], centre[1], HEIGHT));
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

/// One revision's column: its tessellated mesh at `centre`, and its exact
/// boundary when given.
fn session(
    object: &ObjectId,
    centre: [f64; 2],
    boundary: Option<axiolid_brep::ExactBRep>,
) -> AxiolidProximityService {
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        object.clone(),
        column_mesh(centre),
        deviation(),
    );
    AxiolidProximityService::new(match boundary {
        Some(boundary) => geometry.with_exact_boundary(object.clone(), boundary),
        None => geometry,
    })
}

/// The base column measured against the revised one's surface, as a
/// comparison measures them.
fn measure(
    before: &AxiolidProximityService,
    after: &AxiolidProximityService,
    accuracy: f64,
) -> Result<SurfaceDistanceEvidence, ProximityError> {
    let surface = after.body_surface(&revised("#9")).unwrap();
    let request = SurfaceDistanceRequest::try_new(base("#1"), Arc::new(surface), accuracy).unwrap();
    before.measure_surface_distance(&request)
}

#[test]
fn identical_exact_boundaries_are_zero_apart() {
    let before = session(&base("#1"), [0.0, 0.0], Some(exact_column([0.0, 0.0])));
    let after = session(&revised("#9"), [0.0, 0.0], Some(exact_column([0.0, 0.0])));
    assert!(
        after
            .body_surface(&revised("#9"))
            .unwrap()
            .exact_boundary()
            .is_some()
    );
    let measured = measure(&before, &after, 1e-5).unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::ExactBoundary);
    // Zero up to the rounding of the matched surfaces, a few picometres.
    let distance = measured.distance();
    assert!(distance.lower_metres() <= 0.0, "{measured:?}");
    assert!(distance.upper_metres() < 1e-9, "{measured:?}");
    assert!(measured.evidence().exact);
    assert!(
        measured
            .evidence()
            .locator
            .starts_with("axiolid:boundary-hausdorff:"),
        "{measured:?}"
    );
}

/// A millimetre's move against the 16-gon's chord deviation of about four
/// millimetres: the meshes certify nothing, the boundaries the move.
///
/// The moved copy's faces are trimmed in world coordinates, so the kernel
/// cannot match them and closes the upper bound only at first order: within
/// its refinement budget the interval ends a little over half a millimetre
/// above the true distance. It is sound, and wider than asked; judging its
/// width is the comparison's.
#[test]
fn a_millimetre_move_is_measured_between_the_boundaries_only() {
    let moved = [0.001, 0.0];
    let meshes = measure(
        &session(&base("#1"), [0.0, 0.0], None),
        &session(&revised("#9"), moved, None),
        1e-4,
    );
    assert_eq!(meshes, Err(ProximityError::EvidenceFidelityMismatch));

    // One side's boundary is not enough either.
    let one_sided = measure(
        &session(&base("#1"), [0.0, 0.0], Some(exact_column([0.0, 0.0]))),
        &session(&revised("#9"), moved, None),
        1e-4,
    );
    assert_eq!(one_sided, Err(ProximityError::EvidenceFidelityMismatch));

    let measured = measure(
        &session(&base("#1"), [0.0, 0.0], Some(exact_column([0.0, 0.0]))),
        &session(&revised("#9"), moved, Some(exact_column(moved))),
        1e-4,
    )
    .unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::ExactBoundary);
    let distance = measured.distance();
    assert!(
        distance.lower_metres() <= 0.001 && distance.upper_metres() >= 0.001,
        "{distance:?}"
    );
    // The witness lies on the boundary, a millimetre from the other.
    assert!(distance.lower_metres() > 0.000_99, "{distance:?}");
    assert!(distance.upper_metres() < 0.002, "{distance:?}");
}

/// A boundary this kernel cannot read is no boundary: the mesh distance
/// then refuses the tessellation.
#[test]
fn a_foreign_boundary_falls_back_to_the_mesh() {
    let before = session(&base("#1"), [0.0, 0.0], Some(exact_column([0.0, 0.0])));
    let surface = session(&revised("#9"), [0.0, 0.0], None)
        .body_surface(&revised("#9"))
        .unwrap()
        .with_exact_boundary(ExactBoundaryHandle::new(Arc::new("not a boundary")));
    let request = SurfaceDistanceRequest::try_new(base("#1"), Arc::new(surface), 1e-5).unwrap();
    assert_eq!(
        before.measure_surface_distance(&request),
        Err(ProximityError::EvidenceFidelityMismatch)
    );
}
