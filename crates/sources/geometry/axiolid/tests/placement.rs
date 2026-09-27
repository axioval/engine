//! Exact placement search: a witness or a proof that none exists.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, CylinderClearance, ElevationBand, FrameOffsetPlacement, FreeSpaceError,
    FreeSpaceService, MetricDirection, MetricFrame, MetricPoint, PlacementDomain,
    PlacementOrientation, PlacementOutcome, PlacementRequest, PlacementShape,
    SignedDistanceInterval, SupportedPlacement,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// Closed, outward-wound upright prisms over the given plan quads
/// (counter-clockwise corners), as one mesh.
fn prisms(quads: &[[(f64, f64); 4]], z0: f64, z1: f64) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for quad in quads {
        let base = u32::try_from(positions.len()).unwrap();
        for z in [z0, z1] {
            for (x, y) in quad {
                positions.push(Point3::new(*x, *y, z));
            }
        }
        let (b, t) = (base, base + 4);
        // Floor facing down, ceiling facing up.
        indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
        indices.extend([t, t + 1, t + 2, t, t + 2, t + 3]);
        for i in 0..4 {
            let j = (i + 1) % 4;
            indices.extend([b + i, b + j, t + j, b + i, t + j, t + i]);
        }
    }
    TriMesh::new(positions, indices)
}

/// A closed body whose `(x, z)` profile (counter-clockwise, x right, z up)
/// is extruded along y from `y0` to `y1`. `fan` holds the profile's
/// triangulation as index triples.
fn extruded(profile: &[(f64, f64)], fan: &[[u32; 3]], y0: f64, y1: f64) -> TriMesh {
    let n = u32::try_from(profile.len()).unwrap();
    let mut positions = Vec::new();
    for y in [y0, y1] {
        positions.extend(profile.iter().map(|(x, z)| Point3::new(*x, y, *z)));
    }
    let mut indices = Vec::new();
    for [a, b, c] in fan {
        // Counter-clockwise in (x, z) faces -y, which is outward at y0.
        indices.extend([*a, *b, *c, n + a, n + c, n + b]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, n + i, n + j, i, n + j, j]);
    }
    TriMesh::new(positions, indices)
}

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> [(f64, f64); 4] {
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// A `side` square centred on `(cx, cy)`, turned by `degrees`.
fn turned_square(cx: f64, cy: f64, side: f64, degrees: f64) -> [(f64, f64); 4] {
    let (s, c) = degrees.to_radians().sin_cos();
    let h = side / 2.0;
    [(-h, -h), (h, -h), (h, h), (-h, h)].map(|(x, y)| (cx + x * c - y * s, cy + x * s + y * c))
}

fn room(quads: &[[(f64, f64); 4]]) -> AxiolidGeometry {
    AxiolidGeometry::new().with_mesh(id("room"), prisms(quads, 0.0, 3.0))
}

fn axis(x: f64, y: f64) -> MetricDirection {
    MetricDirection::try_new([x, y, 0.0]).unwrap()
}

/// A frame whose right axis points along `(x, y)` in plan.
fn along(x: f64, y: f64) -> PlacementOrientation {
    PlacementOrientation::Fixed(
        MetricFrame::try_new(
            MetricPoint::try_new(id("door"), [0.0, 0.0, 0.0]).unwrap(),
            axis(x, y),
            axis(-y, x),
            MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
        )
        .unwrap(),
    )
}

fn rectangle(width: f64, depth: f64, orientation: PlacementOrientation) -> PlacementShape {
    PlacementShape::Box {
        shape: BoxClearance::try_new(width, depth, 2.0).unwrap(),
        orientation,
    }
}

fn place(
    geometry: AxiolidGeometry,
    shape: PlacementShape,
    obstacles: &[&str],
) -> Result<PlacementOutcome, FreeSpaceError> {
    let request = PlacementRequest::new_in_domain(
        id("room"),
        shape,
        obstacles.iter().map(|o| id(o)).collect(),
        PlacementDomain::Supported(SupportedPlacement::try_new(id("room"), 0.0).unwrap()),
    )
    .unwrap();
    AxiolidFreeSpaceService::new(geometry, source()).find_placement(&request)
}

/// The witness centre and the angle of its width axis, in degrees.
fn found(outcome: Result<PlacementOutcome, FreeSpaceError>) -> ([f64; 3], f64) {
    match outcome {
        Ok(PlacementOutcome::Found(witness)) => {
            let frame = witness.frame();
            let [x, y, _] = frame.right().components();
            (frame.origin().coordinates_metres(), y.atan2(x).to_degrees())
        }
        other => panic!("expected a placement, got {other:?}"),
    }
}

fn nowhere(outcome: Result<PlacementOutcome, FreeSpaceError>) {
    match outcome {
        Ok(PlacementOutcome::NoPlacement(_)) => {}
        other => panic!("expected no placement, got {other:?}"),
    }
}

/// An L-shaped room takes a 1.5 m × 1.0 m area along x only in its long arm.
#[test]
fn a_rectangle_fits_in_one_arm_of_an_l_shaped_room_only() {
    let geometry = room(&[rect(0.0, 4.0, 0.0, 1.2), rect(0.0, 1.2, 1.2, 4.0)]);
    let ([x, y, z], _) = found(place(geometry, rectangle(1.5, 1.0, along(1.0, 0.0)), &[]));
    assert!(y <= 0.7 && (0.75..=3.25).contains(&x), "centre ({x}, {y})");
    assert!(z.abs() < 1e-12, "the witness stands on the floor, got {z}");
}

/// The orientation is part of the answer: a corridor takes the rectangle
/// along its length but not across it.
#[test]
fn a_fixed_orientation_answers_only_for_its_own_axes() {
    let corridor = || room(&[rect(0.0, 4.0, 0.0, 1.2)]);
    found(place(corridor(), rectangle(1.5, 1.0, along(1.0, 0.0)), &[]));
    nowhere(place(corridor(), rectangle(1.5, 1.0, along(0.0, 1.0)), &[]));
}

/// A column in the middle of a 3 m room leaves no 1.5 m square anywhere.
/// Without the column's dilation the square would fit in a corner, so this
/// case fails if the obstacles are not grown by the shape.
#[test]
fn an_obstacle_closes_the_last_gap() {
    let geometry = || room(&[rect(0.0, 3.0, 0.0, 3.0)]);
    let square = || rectangle(1.5, 1.5, along(1.0, 0.0));
    found(place(geometry(), square(), &[]));
    let with_column =
        geometry().with_mesh(id("column"), prisms(&[rect(1.4, 1.6, 1.4, 1.6)], 0.0, 3.0));
    nowhere(place(with_column, square(), &["column"]));
}

/// An obstacle above the height band leaves the floor free.
#[test]
fn an_obstacle_above_the_band_does_not_block() {
    let geometry = room(&[rect(0.0, 3.0, 0.0, 3.0)])
        .with_mesh(id("duct"), prisms(&[rect(0.0, 3.0, 0.0, 3.0)], 2.5, 2.8));
    found(place(
        geometry,
        rectangle(1.5, 1.5, along(1.0, 0.0)),
        &["duct"],
    ));
}

/// An obstacle counts only by the part of its solid inside the height band.
///
/// An L-shaped body stands in the room: a 0.3 m column along one wall and an
/// arm overhead at 2.5 m reaching across the whole room. Its height range
/// overlaps the 2 m band and its plan outline covers the whole floor, yet
/// inside the band it occupies only the column's strip, so a 2 m square fits
/// beside it. Projecting the whole body would prove no placement instead.
#[test]
fn an_obstacle_counts_only_by_its_part_inside_the_band() {
    let profile = [
        (0.0, 0.0),
        (0.3, 0.0),
        (0.3, 2.5),
        (3.0, 2.5),
        (3.0, 2.8),
        (0.0, 2.8),
    ];
    let fan = [[2, 3, 4], [2, 4, 5], [2, 5, 0], [2, 0, 1]];
    let geometry = room(&[rect(0.0, 3.0, 0.0, 3.0)])
        .with_mesh(id("gantry"), extruded(&profile, &fan, 0.0, 3.0));
    let ([x, _, _], _) = found(place(
        geometry,
        rectangle(2.0, 2.0, along(1.0, 0.0)),
        &["gantry"],
    ));
    assert!(
        x >= 1.3 - 1e-9,
        "the square stands clear of the column, got x = {x}"
    );

    // The column alone, 0.8 m wide, leaves too little room.
    let wide = [
        (0.0, 0.0),
        (1.2, 0.0),
        (1.2, 2.5),
        (3.0, 2.5),
        (3.0, 2.8),
        (0.0, 2.8),
    ];
    let geometry =
        room(&[rect(0.0, 3.0, 0.0, 3.0)]).with_mesh(id("gantry"), extruded(&wide, &fan, 0.0, 3.0));
    nowhere(place(
        geometry,
        rectangle(2.0, 2.0, along(1.0, 0.0)),
        &["gantry"],
    ));
}

/// Placement is evidence about the scope, so it cites the scope's source,
/// not the source the service was built with.
#[test]
fn placement_evidence_cites_the_scope_source() {
    let service = AxiolidFreeSpaceService::new(
        room(&[rect(0.0, 3.0, 0.0, 3.0)]),
        SourceId::new("other", "set").unwrap(),
    );
    let request = PlacementRequest::new(id("room"), turning_circle(), Vec::new());
    match service.find_placement(&request) {
        Ok(PlacementOutcome::Found(witness)) => {
            assert_eq!(witness.evidence().source, source());
            assert!(witness.evidence().exact);
        }
        other => panic!("expected a placement, got {other:?}"),
    }
    let request =
        PlacementRequest::new(id("room"), rectangle(4.0, 1.0, along(1.0, 0.0)), Vec::new());
    match service.find_placement(&request) {
        Ok(PlacementOutcome::NoPlacement(proof)) => {
            assert_eq!(proof.evidence().source, source());
        }
        other => panic!("expected no placement, got {other:?}"),
    }
}

/// A rectangle exactly as wide as the room fits only by contact. Rounding
/// could hide or invent that fit, so the search refuses rather than risk an
/// inverted verdict.
#[test]
fn a_knife_edge_fit_is_refused_not_decided() {
    let geometry = room(&[rect(0.0, 4.0, 0.0, 1.5)]);
    assert!(matches!(
        place(geometry, rectangle(1.0, 1.5, along(1.0, 0.0)), &[]),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

fn turning_circle() -> PlacementShape {
    PlacementShape::Cylinder(CylinderClearance::try_new(0.75, 2.0).unwrap())
}

/// A 1.50 m turning circle with 1 mm to spare on each side is found, and one
/// missing by 1 mm is proven absent. Neither is inverted.
#[test]
fn a_turning_circle_is_decided_on_both_sides_of_a_millimetre() {
    found(place(
        room(&[rect(0.0, 1.502, 0.0, 1.502)]),
        turning_circle(),
        &[],
    ));
    nowhere(place(
        room(&[rect(0.0, 1.498, 0.0, 1.498)]),
        turning_circle(),
        &[],
    ));
}

/// A circle that fits only by contact lies inside the approximation band and
/// is refused. Its free region is a single point, which must not read as no
/// placement.
#[test]
fn a_circle_within_the_approximation_band_is_refused() {
    assert!(matches!(
        place(room(&[rect(0.0, 1.5, 0.0, 1.5)]), turning_circle(), &[]),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

/// A 2.10 m × 0.60 m stretcher fits in a 2 m square room only diagonally.
#[test]
fn a_stretcher_fits_only_diagonally() {
    let geometry = || room(&[rect(0.0, 2.0, 0.0, 2.0)]);
    nowhere(place(geometry(), rectangle(2.1, 0.6, along(1.0, 0.0)), &[]));
    nowhere(place(geometry(), rectangle(2.1, 0.6, along(0.0, 1.0)), &[]));
    let (_, angle) = found(place(
        geometry(),
        rectangle(2.1, 0.6, PlacementOrientation::Any),
        &[],
    ));
    let folded = angle.rem_euclid(90.0);
    assert!((20.0..=70.0).contains(&folded), "angle {angle}");
}

/// A 3 m rectangle fits a 2 m square room at no angle: its diagonal is only
/// 2.83 m. The search must prove every angle interval empty.
#[test]
fn a_rectangle_that_fits_at_no_angle_is_proven_absent() {
    nowhere(place(
        room(&[rect(0.0, 2.0, 0.0, 2.0)]),
        rectangle(3.0, 0.6, PlacementOrientation::Any),
        &[],
    ));
}

/// A square only slightly smaller than a room turned by 30° fits only when
/// turned with it.
#[test]
fn a_square_fits_a_turned_room_only_turned_with_it() {
    let geometry = || room(&[turned_square(5.0, 5.0, 2.0, 30.0)]);
    nowhere(place(geometry(), rectangle(1.9, 1.9, along(1.0, 0.0)), &[]));
    let (_, angle) = found(place(
        geometry(),
        rectangle(1.9, 1.9, PlacementOrientation::Any),
        &[],
    ));
    let folded = angle.rem_euclid(90.0);
    assert!((folded - 30.0).abs() <= 3.0, "angle {angle}");
}

/// An obstacle without geometry could stand anywhere, so no verdict is
/// complete.
#[test]
fn an_unmeasured_obstacle_refuses() {
    let geometry = room(&[rect(0.0, 3.0, 0.0, 3.0)]).with_unmeasured(id("cabinet"), "no body");
    assert!(matches!(
        place(geometry, rectangle(1.5, 1.5, along(1.0, 0.0)), &["cabinet"]),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
}

/// Placement is supported by the scope's own floor; any other support is
/// refused rather than approximated.
#[test]
fn other_domains_are_refused() {
    let request = PlacementRequest::new_in_domain(
        id("room"),
        turning_circle(),
        Vec::new(),
        PlacementDomain::Supported(SupportedPlacement::try_new(id("slab"), 0.0).unwrap()),
    )
    .unwrap();
    let service = AxiolidFreeSpaceService::new(room(&[rect(0.0, 3.0, 0.0, 3.0)]), source());
    assert!(matches!(
        service.find_placement(&request),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

fn search(
    geometry: AxiolidGeometry,
    request: &PlacementRequest,
) -> Result<PlacementOutcome, FreeSpaceError> {
    AxiolidFreeSpaceService::new(geometry, source()).find_placement(request)
}

/// An elevation band decides which part of an obstacle counts: a table top
/// from 0.70 m to 0.75 m blocks the default band (the floor up by the shape's
/// height) but not a band ending at 0.67 m, the knee room under it; a low
/// plinth below a band starting at 0.10 m does not count either.
#[test]
fn an_elevation_band_chooses_which_part_of_an_obstacle_counts() {
    let geometry = || {
        room(&[rect(0.0, 2.0, 0.0, 2.0)])
            .with_mesh(id("table"), prisms(&[rect(0.0, 2.0, 0.0, 1.5)], 0.70, 0.75))
            .with_mesh(id("plinth"), prisms(&[rect(0.0, 2.0, 1.4, 2.0)], 0.0, 0.05))
    };
    let request = || {
        PlacementRequest::new(
            id("room"),
            turning_circle(),
            vec![id("plinth"), id("table")],
        )
    };
    nowhere(search(geometry(), &request()));
    let knee_room = request().with_band(ElevationBand::try_new(0.10, 0.67).unwrap());
    found(search(geometry(), &knee_room));
    // The plinth alone, counted from the floor, still leaves no room.
    let from_floor = request().with_band(ElevationBand::try_new(0.0, 0.67).unwrap());
    nowhere(search(geometry(), &from_floor));
}

/// Two 1.0 m wide spaces side by side take a 1.50 m turning circle only
/// together: the search runs on the union of the scope and its merged
/// scopes, and the witness stays grounded on the scope.
#[test]
fn merged_scopes_are_searched_as_one_floor() {
    let geometry = || {
        AxiolidGeometry::new()
            .with_mesh(id("room"), prisms(&[rect(0.0, 1.0, 0.0, 2.0)], 0.0, 3.0))
            .with_mesh(id("alcove"), prisms(&[rect(1.0, 2.0, 0.0, 2.0)], 0.0, 3.0))
    };
    let alone = PlacementRequest::new(id("room"), turning_circle(), Vec::new());
    nowhere(search(geometry(), &alone));
    let merged = alone
        .clone()
        .with_merged_scopes(vec![id("alcove")])
        .unwrap();
    match search(geometry(), &merged) {
        Ok(PlacementOutcome::Found(witness)) => {
            assert_eq!(witness.frame().origin().subject(), &id("room"));
            assert!(witness.evidence().locator.contains("alcove"));
        }
        other => panic!("expected a placement, got {other:?}"),
    }

    // A merged space one step up is another floor, not a larger one.
    let raised = AxiolidGeometry::new()
        .with_mesh(id("room"), prisms(&[rect(0.0, 1.0, 0.0, 2.0)], 0.0, 3.0))
        .with_mesh(id("alcove"), prisms(&[rect(1.0, 2.0, 0.0, 2.0)], 0.2, 3.0));
    assert!(matches!(
        search(raised, &merged),
        Err(FreeSpaceError::Unavailable(_))
    ));

    // No single support holds a base spanning both spaces.
    let supported = PlacementRequest::new_in_domain(
        id("room"),
        turning_circle(),
        Vec::new(),
        PlacementDomain::Supported(SupportedPlacement::try_new(id("room"), 0.0).unwrap()),
    )
    .unwrap()
    .with_merged_scopes(vec![id("alcove")])
    .unwrap();
    assert!(matches!(
        search(geometry(), &supported),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

/// A door frame at `(2, 0)` on the room's south wall, facing north into it.
fn door() -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("door"), [2.0, 0.0, 0.0]).unwrap(),
        axis(1.0, 0.0),
        axis(0.0, 1.0),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap()
}

fn interval(low: f64, high: f64) -> SignedDistanceInterval {
    SignedDistanceInterval::try_new(low, high).unwrap()
}

fn in_front_of_door(
    shape: PlacementShape,
    across: (f64, f64),
    ahead: (f64, f64),
    obstacles: &[&str],
) -> PlacementRequest {
    PlacementRequest::new_in_domain(
        id("room"),
        shape,
        obstacles.iter().map(|o| id(o)).collect(),
        PlacementDomain::SupportedFrameOffsets {
            support: SupportedPlacement::try_new(id("room"), 0.0).unwrap(),
            offsets: FrameOffsetPlacement::new(
                door(),
                interval(across.0, across.1),
                interval(ahead.0, ahead.1),
                SignedDistanceInterval::exact(0.0).unwrap(),
            ),
        },
    )
    .unwrap()
}

fn door_box() -> PlacementShape {
    rectangle(1.5, 1.5, PlacementOrientation::Fixed(door()))
}

/// A 1.50 m square in front of a door: its centre may slide 0.2 m either
/// side of the door's axis and 0.75 m to 1.0 m ahead. The witness keeps the
/// door's axes and lies within the offsets.
#[test]
fn a_box_is_found_within_the_door_offsets() {
    let geometry = room(&[rect(0.0, 4.0, 0.0, 4.0)]);
    let request = in_front_of_door(door_box(), (-0.2, 0.2), (0.75, 1.0), &[]);
    let ([x, y, z], angle) = found(search(geometry, &request));
    assert!(
        (1.8..=2.2).contains(&x) && (0.75..=1.0).contains(&y),
        "({x}, {y})"
    );
    assert!(z.abs() < 1e-12 && angle.abs() < 1e-12);
}

/// A column right in front of the door blocks every offset the domain
/// admits, although the room has space elsewhere: the configuration space is
/// intersected with the offset box, not searched as a whole. Letting the box
/// slide 1 m sideways admits a centre beside the column again.
#[test]
fn an_obstacle_in_front_of_the_door_is_decided_within_the_offsets() {
    let geometry = || {
        room(&[rect(0.0, 4.0, 0.0, 4.0)])
            .with_mesh(id("column"), prisms(&[rect(1.9, 2.1, 1.3, 1.5)], 0.0, 3.0))
    };
    found(search(
        geometry(),
        &PlacementRequest::new(id("room"), door_box(), vec![id("column")]),
    ));
    nowhere(search(
        geometry(),
        &in_front_of_door(door_box(), (-0.2, 0.2), (0.75, 1.0), &["column"]),
    ));
    let ([x, _, _], _) = found(search(
        geometry(),
        &in_front_of_door(door_box(), (-1.2, 1.2), (0.75, 1.0), &["column"]),
    ));
    assert!(
        (x - 2.0).abs() >= 0.85 - 1e-9,
        "the box clears the column, x = {x}"
    );
}

/// A box pinned to one offset has no area to search: the single centre is
/// checked directly, and its absence is still proven with the grown box.
#[test]
fn a_pinned_offset_is_decided_both_ways() {
    let geometry = || {
        room(&[rect(0.0, 4.0, 0.0, 4.0)])
            .with_mesh(id("column"), prisms(&[rect(3.5, 3.7, 0.5, 0.7)], 0.0, 3.0))
    };
    let pinned = |ahead: f64| in_front_of_door(door_box(), (0.0, 0.0), (ahead, ahead), &["column"]);
    let ([x, y, _], _) = found(search(geometry(), &pinned(0.75)));
    assert!((x - 2.0).abs() < 1e-12 && (y - 0.75).abs() < 1e-12);
    // Ahead by 0.5 m the box would stick out through the wall.
    nowhere(search(geometry(), &pinned(0.5)));
}

/// A turning circle in front of a door keeps the door's axes in its witness.
#[test]
fn a_circle_is_found_and_proven_absent_within_the_offsets() {
    let request = in_front_of_door(turning_circle(), (-0.2, 0.2), (0.0, 1.0), &[]);
    match search(room(&[rect(0.0, 4.0, 0.0, 4.0)]), &request) {
        Ok(PlacementOutcome::Found(witness)) => {
            assert_eq!(witness.frame().right(), door().right());
            let [x, y, _] = witness.frame().origin().coordinates_metres();
            assert!(
                (1.8..=2.2).contains(&x) && (0.75..=1.0).contains(&y),
                "({x}, {y})"
            );
        }
        other => panic!("expected a placement, got {other:?}"),
    }
    // Only 0.7 m ahead of the door is admitted: the circle meets the wall.
    let close = in_front_of_door(turning_circle(), (-0.2, 0.2), (0.0, 0.7), &[]);
    nowhere(search(room(&[rect(0.0, 4.0, 0.0, 4.0)]), &close));
}

/// A tilted anchor or a floor outside the vertical offsets is refused, not
/// approximated or proven empty.
#[test]
fn unusable_offset_domains_refuse() {
    let geometry = || room(&[rect(0.0, 4.0, 0.0, 4.0)]);
    let raised = PlacementRequest::new_in_domain(
        id("room"),
        turning_circle(),
        Vec::new(),
        PlacementDomain::FrameOffsets(FrameOffsetPlacement::new(
            door(),
            interval(-0.2, 0.2),
            interval(0.0, 1.0),
            interval(0.5, 1.0),
        )),
    )
    .unwrap();
    assert!(matches!(
        search(geometry(), &raised),
        Err(FreeSpaceError::Unavailable(_))
    ));
    let (s, c) = 0.1_f64.sin_cos();
    let tilted = MetricFrame::try_new(
        MetricPoint::try_new(id("door"), [2.0, 0.0, 0.0]).unwrap(),
        axis(1.0, 0.0),
        MetricDirection::try_new([0.0, c, s]).unwrap(),
        MetricDirection::try_new([0.0, -s, c]).unwrap(),
    )
    .unwrap();
    let request = PlacementRequest::new_in_domain(
        id("room"),
        turning_circle(),
        Vec::new(),
        PlacementDomain::FrameOffsets(FrameOffsetPlacement::new(
            tilted,
            interval(-0.2, 0.2),
            interval(0.0, 1.0),
            interval(-1.0, 1.0),
        )),
    )
    .unwrap();
    assert!(matches!(
        search(geometry(), &request),
        Err(FreeSpaceError::Unavailable(_))
    ));
}
