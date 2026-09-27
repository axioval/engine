//! Space-boundary coverage over a box space and hand-built boundary surfaces.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidBoundaryCoverageService, AxiolidGeometry};
use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryPlacement,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

const X: f64 = 4.0;
const Y: f64 = 3.0;
const Z: f64 = 2.5;
const SURFACE: f64 = 2.0 * (X * Y + X * Z + Y * Z);

/// A closed, outward-oriented box `X` by `Y` by `Z` from the origin, turned
/// by `turn` radians about the vertical axis.
fn space_body(turn: f64) -> TriMesh {
    let corners = [
        [0.0, 0.0, 0.0],
        [X, 0.0, 0.0],
        [X, Y, 0.0],
        [0.0, Y, 0.0],
        [0.0, 0.0, Z],
        [X, 0.0, Z],
        [X, Y, Z],
        [0.0, Y, Z],
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(corners.iter().map(|p| turned(*p, turn)).collect(), indices)
}

fn turned([x, y, z]: [f64; 3], turn: f64) -> Point3 {
    if turn == 0.0 {
        return Point3::new(x, y, z);
    }
    let (sin, cos) = turn.sin_cos();
    Point3::new(x * cos - y * sin, x * sin + y * cos, z)
}

/// Parallelograms `origin + s·u + t·v` for `s, t` in `[0, 1]`, as one mesh.
/// A parallelogram: its origin and two edge vectors.
type Patch = ([f64; 3], [f64; 3], [f64; 3]);

fn patches(parts: &[Patch], turn: f64) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for (origin, u, v) in parts {
        let at = |s: f64, t: f64| {
            turned(
                [0, 1, 2].map(|axis| origin[axis] + s * u[axis] + t * v[axis]),
                turn,
            )
        };
        let first = u32::try_from(positions.len()).unwrap();
        positions.extend([at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)]);
        indices.extend([0, 1, 2, 0, 2, 3].map(|offset| first + offset));
    }
    TriMesh::new(positions, indices)
}

/// The six faces of the box, each as its own boundary.
fn faces() -> Vec<(&'static str, Patch)> {
    vec![
        ("floor", ([0.0, 0.0, 0.0], [X, 0.0, 0.0], [0.0, Y, 0.0])),
        ("ceiling", ([0.0, 0.0, Z], [X, 0.0, 0.0], [0.0, Y, 0.0])),
        ("south", ([0.0, 0.0, 0.0], [X, 0.0, 0.0], [0.0, 0.0, Z])),
        ("north", ([0.0, Y, 0.0], [X, 0.0, 0.0], [0.0, 0.0, Z])),
        ("west", ([0.0, 0.0, 0.0], [0.0, Y, 0.0], [0.0, 0.0, Z])),
        ("east", ([X, 0.0, 0.0], [0.0, Y, 0.0], [0.0, 0.0, Z])),
    ]
}

fn service(turn: f64) -> AxiolidBoundaryCoverageService {
    AxiolidBoundaryCoverageService::new(
        AxiolidGeometry::new().with_mesh(id("space"), space_body(turn)),
    )
    .with_space(id("space"))
}

fn with_faces(
    mut service: AxiolidBoundaryCoverageService,
    skip: &[&str],
    turn: f64,
) -> AxiolidBoundaryCoverageService {
    for (name, face) in faces() {
        if !skip.contains(&name) {
            service = service.with_boundary(
                id("space"),
                id(name),
                Some(id(&format!("{name}-element"))),
                patches(&[face], turn),
            );
        }
    }
    service
}

fn measure(
    service: &AxiolidBoundaryCoverageService,
    tolerance: f64,
) -> Result<BoundaryCoverage, BoundaryCoverageError> {
    service.measure_boundary_coverage(
        &BoundaryCoverageRequest::try_new(id("space"), tolerance).unwrap(),
    )
}

fn point(interval: axioval_engine::SurfaceAreaInterval) -> f64 {
    assert!(interval.is_point(), "{interval:?}");
    interval.lower_square_metres()
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn a_fully_bounded_box_is_covered_exactly() {
    let coverage = measure(&with_faces(service(0.0), &[], 0.0), 0.001).unwrap();
    assert!(coverage.is_exact());
    close(point(coverage.surface_area()), SURFACE);
    close(point(coverage.covered_area()), SURFACE);
    close(point(coverage.uncovered_area()), 0.0);
    close(point(coverage.overlap_area()), 0.0);
    close(coverage.covered_share().lower(), 1.0);
    assert!(coverage.overlaps().is_empty());
    assert_eq!(coverage.boundaries().len(), 6);
    assert_eq!(coverage.off_surface().count(), 0);
    let floor = coverage
        .boundaries()
        .iter()
        .find(|boundary| boundary.boundary() == &id("floor"))
        .unwrap();
    assert_eq!(floor.element(), Some(&id("floor-element")));
    let BoundaryPlacement::OnSurface { area } = floor.placement() else {
        panic!("the floor lies on the surface");
    };
    close(point(area), X * Y);
    assert_eq!(
        coverage.evidence().locator,
        format!("axiolid:boundary-coverage:{}", id("space"))
    );
}

#[test]
fn a_missing_wall_boundary_leaves_its_wall_uncovered() {
    let coverage = measure(&with_faces(service(0.0), &["east"], 0.0), 0.001).unwrap();
    close(point(coverage.uncovered_area()), Y * Z);
    close(point(coverage.covered_area()), SURFACE - Y * Z);
    close(
        coverage.covered_share().upper(),
        (SURFACE - Y * Z) / SURFACE,
    );
}

#[test]
fn a_space_without_boundaries_is_uncovered() {
    let coverage = measure(&service(0.0), 0.001).unwrap();
    close(point(coverage.uncovered_area()), SURFACE);
    close(coverage.covered_share().upper(), 0.0);
}

#[test]
fn overlapping_boundaries_report_their_common_area_and_pair() {
    let service = with_faces(service(0.0), &["floor"], 0.0)
        .with_boundary(
            id("space"),
            id("floor-a"),
            None,
            patches(&[([0.0, 0.0, 0.0], [2.5, 0.0, 0.0], [0.0, Y, 0.0])], 0.0),
        )
        .with_boundary(
            id("space"),
            id("floor-b"),
            None,
            patches(&[([1.5, 0.0, 0.0], [2.5, 0.0, 0.0], [0.0, Y, 0.0])], 0.0),
        );
    let coverage = measure(&service, 0.001).unwrap();
    close(point(coverage.uncovered_area()), 0.0);
    close(point(coverage.overlap_area()), Y);
    assert_eq!(coverage.overlaps().len(), 1);
    let pair = &coverage.overlaps()[0];
    assert_eq!(
        (pair.first(), pair.second()),
        (&id("floor-a"), &id("floor-b"))
    );
    close(point(pair.area()), Y);
}

#[test]
fn three_overlapping_boundaries_count_their_common_part_once() {
    let strip = |from: f64| patches(&[([from, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, Y, 0.0])], 0.0);
    let service = with_faces(service(0.0), &["floor"], 0.0)
        .with_boundary(id("space"), id("a"), None, strip(0.0))
        .with_boundary(id("space"), id("b"), None, strip(1.0))
        .with_boundary(id("space"), id("c"), None, strip(2.0));
    let coverage = measure(&service, 0.001).unwrap();
    // x 1..3 is covered twice or more; the pair a, c only touches.
    close(point(coverage.overlap_area()), 2.0 * Y);
    assert_eq!(coverage.overlaps().len(), 2);
}

/// A plane bounded by an outer and an inner curve: the hole stays uncovered.
#[test]
fn a_boundary_with_a_hole_leaves_the_hole_uncovered() {
    let ring = patches(
        &[
            ([0.0, 0.0, 0.0], [X, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 2.0, 0.0], [X, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([2.0, 1.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ],
        0.0,
    );
    let service = with_faces(service(0.0), &["floor"], 0.0).with_boundary(
        id("space"),
        id("floor"),
        None,
        ring,
    );
    let coverage = measure(&service, 0.001).unwrap();
    close(point(coverage.uncovered_area()), 1.0);
    close(point(coverage.overlap_area()), 0.0);
}

#[test]
fn a_boundary_on_no_face_plane_is_reported_and_covers_nothing() {
    let service = with_faces(service(0.0), &[], 0.0).with_boundary(
        id("space"),
        id("mid-air"),
        None,
        patches(&[([0.0, 0.0, 1.0], [X, 0.0, 0.0], [0.0, Y, 0.0])], 0.0),
    );
    let coverage = measure(&service, 0.001).unwrap();
    let off: Vec<_> = coverage
        .off_surface()
        .map(|b| b.boundary().clone())
        .collect();
    assert_eq!(off, vec![id("mid-air")]);
    close(point(coverage.overlap_area()), 0.0);
    close(point(coverage.uncovered_area()), 0.0);
}

#[test]
fn a_boundary_within_the_tolerance_counts_but_is_not_exact() {
    let lifted = patches(&[([0.0, 0.0, 0.0005], [X, 0.0, 0.0], [0.0, Y, 0.0])], 0.0);
    let service = with_faces(service(0.0), &["floor"], 0.0).with_boundary(
        id("space"),
        id("floor"),
        None,
        lifted,
    );
    let coverage = measure(&service, 0.001).unwrap();
    assert!(!coverage.is_exact());
    close(coverage.uncovered_area().upper_square_metres(), 0.0);
    let strict = measure(&service, 0.0).unwrap();
    assert_eq!(strict.off_surface().count(), 1);
    close(strict.uncovered_area().lower_square_metres(), X * Y);
}

#[test]
fn a_turned_space_is_measured_within_rounding_bounds() {
    let turn = 30f64.to_radians();
    let coverage = measure(&with_faces(service(turn), &["east"], turn), 0.001).unwrap();
    assert!(!coverage.is_exact());
    let surface = coverage.surface_area();
    assert!(surface.lower_square_metres() <= SURFACE && SURFACE <= surface.upper_square_metres());
    let uncovered = coverage.uncovered_area();
    assert!(uncovered.lower_square_metres() <= Y * Z + 1e-9);
    assert!(Y * Z - 1e-9 <= uncovered.upper_square_metres());
    assert!(uncovered.upper_square_metres() - uncovered.lower_square_metres() < 1e-3);
}

#[test]
fn a_tessellated_boundary_widens_the_areas() {
    let mut service = with_faces(service(0.0), &["floor"], 0.0);
    service = service.with_tessellated_boundary(
        id("space"),
        id("floor"),
        None,
        patches(&[([0.0, 0.0, 0.0], [X, 0.0, 0.0], [0.0, Y, 0.0])], 0.0),
        0.001,
    );
    let coverage = measure(&service, 0.001).unwrap();
    assert!(!coverage.is_exact());
    let uncovered = coverage.uncovered_area();
    assert!(uncovered.lower_square_metres() <= 0.0 + 1e-12);
    // A band of 1 mm along the floor's 14 m perimeter.
    assert!(uncovered.upper_square_metres() >= 2.0 * 14.0 * 0.001);
    assert!(coverage.covered_share().lower() < 1.0);
}

#[test]
fn unreadable_boundaries_and_bodies_refuse() {
    let unmeasured = with_faces(service(0.0), &[], 0.0).with_unmeasured_boundary(
        id("space"),
        id("face-surface"),
        None,
        "face surfaces are not lowered",
    );
    assert!(matches!(
        measure(&unmeasured, 0.001),
        Err(BoundaryCoverageError::Unavailable(message)) if message.contains("face surfaces")
    ));

    let unknown = AxiolidBoundaryCoverageService::new(
        AxiolidGeometry::new().with_mesh(id("space"), space_body(0.0)),
    );
    assert_eq!(
        measure(&unknown, 0.001),
        Err(BoundaryCoverageError::UnknownSpace(id("space")))
    );

    let bodiless =
        AxiolidBoundaryCoverageService::new(AxiolidGeometry::new().with_no_body(id("space")))
            .with_space(id("space"));
    assert_eq!(
        measure(&bodiless, 0.001),
        Err(BoundaryCoverageError::NoBody(id("space")))
    );

    let curved = AxiolidBoundaryCoverageService::new(AxiolidGeometry::new().with_tessellated_mesh(
        id("space"),
        space_body(0.0),
        0.001,
    ))
    .with_space(id("space"));
    assert!(matches!(
        measure(&curved, 0.001),
        Err(BoundaryCoverageError::Unavailable(_))
    ));

    let unmeshed = AxiolidBoundaryCoverageService::new(
        AxiolidGeometry::new().with_unmeasured(id("space"), "refused"),
    )
    .with_space(id("space"));
    assert!(matches!(
        measure(&unmeshed, 0.001),
        Err(BoundaryCoverageError::Unavailable(_))
    ));
}
