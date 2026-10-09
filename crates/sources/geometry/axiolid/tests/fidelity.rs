//! Every service that reports exact evidence must honour tessellation.
//!
//! A mesh registered with `with_tessellated_mesh` approximates curved faces.
//! A service refuses when such a mesh could change its answer -- the subject
//! itself, or a part whose true body could reach the measurement -- and
//! answers as before when the curved part is far away. Refusing everywhere
//! would make one curved column in a model block every check; answering
//! everywhere would present an estimate as fact.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{
    AxiolidContactService, AxiolidEnvelopeMembershipService, AxiolidFreeSpaceService,
    AxiolidGeometry, AxiolidGuardService, AxiolidLinearQuantityService, AxiolidSpaceService,
};
use axioval_engine::{
    BoundaryRequest, BoxClearance, ClearanceRequest, ClearanceShape, ContactError, ContactRequest,
    ContactService, ContactSide, ContactTolerance, EnvelopeDerivation, EnvelopeMembershipError,
    EnvelopeMembershipRequest, EnvelopeMembershipService, FreeAreaRequest, FreeSpaceError,
    FreeSpaceService, GuardError, GuardSearch, GuardService, LinearQuantityKind,
    LinearQuantityRequest, LinearQuantityService, MetricDirection, MetricFrame, MetricPoint,
    MobilityProfile, OverlapRequest, ShelfGeometry, SpaceError, SpaceService,
};
use axioval_ir::{ObjectId, SourceId};

const CHORD: f64 = 0.002;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented box.
fn cuboid(x: [f64; 2], y: [f64; 2], z: [f64; 2]) -> TriMesh {
    let ([x0, x1], [y0, y1], [z0, z1]) = (x, y, z);
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

/// A slab with a wall under it, plus a column either beside the slab or
/// far away; the column is the only curved part.
fn slab_on_wall(column_x: f64) -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid([0.0, 4.0], [0.0, 0.2], [0.0, 3.0]))
        .with_mesh(id("slab"), cuboid([0.0, 4.0], [-1.0, 1.0], [3.0, 3.2]))
        .with_tessellated_mesh(
            id("column"),
            cuboid([column_x, column_x + 0.3], [0.0, 0.3], [0.0, 3.0]),
            CHORD,
        )
}

fn contact_with(
    geometry: AxiolidGeometry,
    subject: &str,
    candidates: &[&str],
) -> Result<f64, ContactError> {
    AxiolidContactService::new(geometry)
        .measure_contact(&ContactRequest::new(
            id(subject),
            candidates.iter().map(|local| id(local)).collect(),
            ContactSide::Below,
            ContactTolerance::try_new(0.01, 0.01, 0.0001).unwrap(),
        ))
        .map(|evidence| evidence.contact_ratio())
}

fn contact(geometry: AxiolidGeometry, subject: &str) -> Result<f64, ContactError> {
    contact_with(geometry, subject, &["wall", "column", "slab"])
}

#[test]
fn contact_refuses_a_curved_part_it_could_touch_but_not_one_far_away() {
    // The column tops out 0.2 m below the slab: the nearest candidate below,
    // and a chord could hide the gap it is measured at.
    assert_eq!(
        contact(slab_on_wall(4.0), "slab"),
        Err(ContactError::InexactEvidence)
    );
    assert!(contact(slab_on_wall(40.0), "slab").is_ok());
}

/// A curved part the rule did not offer as a candidate is never measured, so
/// it cannot make the answer an estimate.
#[test]
fn contact_ignores_a_curved_part_outside_the_candidates() {
    assert!(contact_with(slab_on_wall(4.0), "slab", &["wall"]).is_ok());
}

#[test]
fn contact_refuses_a_curved_subject() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(
            id("slab"),
            cuboid([0.0, 4.0], [0.0, 4.0], [3.0, 3.2]),
            CHORD,
        )
        .with_mesh(id("wall"), cuboid([0.0, 4.0], [0.0, 0.2], [0.0, 3.0]));
    assert_eq!(
        contact(geometry, "slab"),
        Err(ContactError::InexactEvidence)
    );
}

fn envelope(column_x: f64) -> Result<usize, EnvelopeMembershipError> {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), cuboid([0.0, 10.0], [0.0, 10.0], [0.0, 3.0]))
        .with_mesh(id("wall"), cuboid([0.0, 10.0], [-0.2, 0.2], [0.0, 3.0]))
        .with_tessellated_mesh(
            id("column"),
            cuboid([column_x, column_x + 0.3], [5.0, 5.3], [0.0, 3.0]),
            CHORD,
        );
    AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_internal(id("wall"))
        .measure_envelope_membership(&EnvelopeMembershipRequest::new(
            EnvelopeDerivation::AllSpaces,
            vec![id("space")],
        ))
        .map(|evidence| evidence.derived().len())
}

#[test]
fn envelope_refuses_a_curved_part_reaching_a_space() {
    assert_eq!(envelope(5.0), Err(EnvelopeMembershipError::InexactEvidence));
    assert_eq!(envelope(50.0), Ok(1));
}

fn frame() -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("room"), [1.0, 1.0, 0.0]).unwrap(),
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap()
}

fn clearance(column_x: f64) -> Result<(), FreeSpaceError> {
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        id("column"),
        cuboid([column_x, column_x + 0.3], [0.9, 1.2], [0.0, 3.0]),
        CHORD,
    );
    AxiolidFreeSpaceService::new(geometry, source())
        .assess_clearance(&ClearanceRequest::new(
            frame(),
            ClearanceShape::Box(BoxClearance::try_new(0.8, 0.8, 2.0).unwrap()),
            vec![id("column")],
        ))
        .map(|_| ())
}

fn free_area(column_x: f64) -> Result<(), FreeSpaceError> {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 5.0], [0.0, 5.0], [0.0, 0.1]))
        .with_tessellated_mesh(
            id("column"),
            cuboid([column_x, column_x + 0.3], [1.0, 1.3], [0.0, 3.0]),
            CHORD,
        );
    AxiolidFreeSpaceService::new(geometry, source())
        .measure_free_area(&FreeAreaRequest::new(
            id("room"),
            MobilityProfile::try_new(0.3, 1.8, 0.15, 0.08).unwrap(),
            vec![id("column")],
        ))
        .map(|_| ())
}

#[test]
fn free_space_refuses_a_curved_obstacle_that_could_reach_the_volume() {
    assert_eq!(
        clearance(1.2),
        Err(FreeSpaceError::InexactObstructionEvidence)
    );
    assert!(clearance(20.0).is_ok());
    assert_eq!(free_area(1.0), Err(FreeSpaceError::InexactAreaEvidence));
    assert!(free_area(20.0).is_ok());
}

fn guard(column_x: f64) -> Result<usize, GuardError> {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), cuboid([0.0, 4.0], [0.0, 4.0], [0.0, 0.2]))
        .with_tessellated_mesh(
            id("post"),
            cuboid([column_x, column_x + 0.1], [3.95, 4.05], [0.2, 1.2]),
            CHORD,
        );
    AxiolidGuardService::new(geometry, source())
        .with_walking_surface(id("deck"))
        .measure_guard_edges(GuardSearch::try_new(0.5, 0.1).unwrap())
        .map(|evidence| evidence.edges().len())
}

#[test]
fn guard_refuses_a_curved_candidate_within_reach() {
    assert_eq!(guard(1.0), Err(GuardError::InexactEvidence));
    assert_eq!(guard(30.0), Ok(1));
}

fn spaces(column_x: f64) -> AxiolidSpaceService {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), cuboid([0.0, 4.0], [0.0, 4.0], [0.0, 3.0]))
        .with_tessellated_mesh(
            id("column"),
            cuboid([column_x, column_x + 0.3], [1.0, 1.3], [0.0, 3.0]),
            CHORD,
        );
    AxiolidSpaceService::new(geometry, source()).with_space(id("space"))
}

#[test]
fn space_measurements_refuse_a_curved_part_touching_the_space() {
    let near = spaces(1.0);
    assert_eq!(
        near.measure_overlaps(&id("space"), &OverlapRequest::new())
            .unwrap_err(),
        SpaceError::InexactEvidence
    );
    // The column stands inside, farther than its deviation from every
    // boundary point, so it surely covers none of them (#305).
    assert!(
        near.measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
            .is_ok()
    );
    // One whose outline passes within its deviation of a boundary
    // point (the west edge's midpoint) may cover it or not.
    let touching = AxiolidSpaceService::new(
        AxiolidGeometry::new()
            .with_mesh(id("space"), cuboid([0.0, 4.0], [0.0, 4.0], [0.0, 3.0]))
            .with_tessellated_mesh(
                id("column"),
                cuboid([-0.3, 0.001], [1.9, 2.1], [0.0, 3.0]),
                CHORD,
            ),
        source(),
    )
    .with_space(id("space"));
    assert_eq!(
        touching
            .measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
            .unwrap_err(),
        SpaceError::InexactEvidence
    );
    // Clear height reads the space alone, which is exact.
    assert!(near.measure_clear_height(&id("space")).is_ok());

    let far = spaces(40.0);
    assert!(
        far.measure_overlaps(&id("space"), &OverlapRequest::new())
            .is_ok()
    );
    assert!(
        far.measure_boundary_gaps(&id("space"), &BoundaryRequest::new())
            .is_ok()
    );
}

/// A curved space's clear height is the interval its chord deviation
/// leaves it in, never a point; with no other space it has no duplicate
/// whatever its deviation (#305). Its boundary is chords, so its gaps are
/// refused.
#[test]
fn a_curved_space_is_measured_within_its_deviation() {
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        id("vault"),
        cuboid([0.0, 4.0], [0.0, 4.0], [0.0, 3.0]),
        CHORD,
    );
    let service = AxiolidSpaceService::new(geometry, source()).with_space(id("vault"));
    let height = service.measure_clear_height(&id("vault")).unwrap();
    let (lower, upper) = height.bounds_metres();
    assert!((lower - (3.0 - 2.0 * CHORD)).abs() < 1e-12, "{lower}");
    assert!((upper - (3.0 + 2.0 * CHORD)).abs() < 1e-12, "{upper}");
    assert!(height.evidence().exact);
    assert!(
        height.evidence().locator.contains("within-chord-deviation"),
        "{}",
        height.evidence().locator
    );
    assert!(service.measure_duplicates(&id("vault")).unwrap().is_empty());
    assert_eq!(
        service
            .measure_boundary_gaps(&id("vault"), &BoundaryRequest::new())
            .unwrap_err(),
        SpaceError::InexactEvidence
    );
}

/// Shelf length is an interval, so a curved room widens it rather than
/// refusing: its true boundary may lie up to the chord deviation either side
/// of the mesh, and every cross-section keeps that much more margin. The
/// wider interval still holds the exact room's length.
#[test]
fn shelf_length_widens_rather_than_refuses() {
    let shelf = ShelfGeometry::try_new(0.3, 1.0, 0.4, 0.0, 2.0, 0.9).unwrap();
    let measure = |geometry: AxiolidGeometry| {
        let evidence = AxiolidLinearQuantityService::new(geometry)
            .measure_linear_quantity(&LinearQuantityRequest::new(
                id("room"),
                LinearQuantityKind::ShelfRunningLength(shelf),
            ))
            .unwrap();
        (
            evidence.measured().lower_metres(),
            evidence.measured().upper_metres(),
            evidence.clear_height().unwrap(),
        )
    };
    let room = || cuboid([0.0, 3.99], [0.0, 3.99], [0.0, 3.0]);
    let (exact_low, exact_high, exact_height) =
        measure(AxiolidGeometry::new().with_mesh(id("room"), room()));
    let (curved_low, curved_high, curved_height) =
        measure(AxiolidGeometry::new().with_tessellated_mesh(id("room"), room(), 0.01));
    assert!(exact_low > 0.0);
    assert!(
        curved_low < exact_low,
        "{curved_low} must be below {exact_low}"
    );
    assert!(
        curved_high >= exact_high,
        "{curved_high} must reach {exact_high}"
    );
    assert!(exact_height.is_exact());
    assert!(!curved_height.is_exact());
}
