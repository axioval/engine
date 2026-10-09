//! Placement next to tessellated bodies, bracketed by their chord deviation
//! (#302): decided where the deviation cannot tip the verdict, refused where
//! it can.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, FreeSpaceError, FreeSpaceService, MetricDirection, MetricFrame, MetricPoint,
    PlacementDomain, PlacementOrientation, PlacementOutcome, PlacementRequest, PlacementShape,
    SupportedPlacement,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A plan point.
type Xy = (f64, f64);

/// Closed, outward-wound upright prisms over the given plan quads
/// (counter-clockwise corners), as one mesh of separate shells.
fn prisms(quads: &[[Xy; 4]], z0: f64, z1: f64) -> TriMesh {
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
        indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
        indices.extend([t, t + 1, t + 2, t, t + 2, t + 3]);
        for i in 0..4 {
            let j = (i + 1) % 4;
            indices.extend([b + i, b + j, t + j, b + i, t + j, t + i]);
        }
    }
    TriMesh::new(positions, indices)
}

fn rect(x0: f64, x1: f64, y0: f64, y1: f64) -> [Xy; 4] {
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// Sides of the polygon a round hole or column is tessellated with.
const SIDES: u32 = 16;

/// How far a chord of a polygon of [`SIDES`] sides inscribed in a circle of
/// `radius` lies from the arc: the deviation a host declares.
fn sagitta(radius: f64) -> f64 {
    radius * (1.0 - (std::f64::consts::PI / f64::from(SIDES)).cos())
}

/// The quads of a `side` square centred on `centre` less the inscribed
/// [`SIDES`]-gon of a round hole of `radius`: each quad runs from two
/// consecutive polygon corners out along their rays to the square, whose
/// corners lie on the 45° rays.
fn holed_cell(centre: Xy, side: f64, radius: f64) -> Vec<[Xy; 4]> {
    let at = |k: u32| {
        let angle = 2.0 * std::f64::consts::PI * f64::from(k) / f64::from(SIDES);
        let (s, c) = angle.sin_cos();
        let reach = side / 2.0 / c.abs().max(s.abs());
        (
            (centre.0 + radius * c, centre.1 + radius * s),
            (centre.0 + reach * c, centre.1 + reach * s),
        )
    };
    (0..SIDES)
        .map(|k| {
            let (p0, q0) = at(k);
            let (p1, q1) = at(k + 1);
            [p0, q0, q1, p1]
        })
        .collect()
}

/// A round column of `radius` tessellated as quads of a ring around a
/// small square core, centred on `centre`.
fn round_column(centre: Xy, radius: f64) -> Vec<[Xy; 4]> {
    let core = radius / 4.0;
    let at = |k: u32, r: f64| {
        let angle = 2.0 * std::f64::consts::PI * f64::from(k) / f64::from(SIDES);
        let (s, c) = angle.sin_cos();
        (centre.0 + r * c, centre.1 + r * s)
    };
    let mut quads: Vec<[Xy; 4]> = (0..SIDES)
        .map(|k| {
            [
                at(k, core),
                at(k, radius),
                at(k + 1, radius),
                at(k + 1, core),
            ]
        })
        .collect();
    // The core polygon as fan quads about its centre (two triangles of a
    // degenerate quad would repeat a corner, so pair up its sides).
    for k in (0..SIDES).step_by(2) {
        quads.push([centre, at(k, core), at(k + 1, core), at(k + 2, core)]);
    }
    quads
}

/// The synthetic room of #302: a 12 x 8 floor slab under two spaces split
/// by a partition, walls, a ceiling, and a round hole of radius 0.3 m in
/// the slab's corner under space `a`. The slab is tessellated within its
/// chord deviation; `certified` registers the extent its construction
/// gives (the slab's top at the floor), as a host with its exact boundary
/// does.
fn holed_room(certified: bool) -> AxiolidGeometry {
    let radius = 0.3;
    let mut slab: Vec<[Xy; 4]> = vec![
        rect(0.0, 12.0, 1.5, 8.0),
        rect(0.0, 12.0, 0.0, 0.5),
        rect(0.0, 0.5, 0.5, 1.5),
        rect(1.5, 12.0, 0.5, 1.5),
    ];
    slab.extend(holed_cell((1.0, 1.0), 1.0, radius));
    let walls = [
        rect(0.0, 12.0, 0.0, 0.2),
        rect(0.0, 12.0, 7.8, 8.0),
        rect(0.0, 0.2, 0.2, 7.8),
        rect(11.8, 12.0, 0.2, 7.8),
        rect(5.9, 6.1, 0.2, 7.8),
    ];
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("slab"), prisms(&slab, -0.25, 0.0), sagitta(radius))
        .with_mesh(id("walls"), prisms(&walls, 0.0, 2.8))
        .with_mesh(
            id("ceiling"),
            prisms(&[rect(0.0, 12.0, 0.0, 8.0)], 2.8, 3.05),
        )
        .with_mesh(id("a"), prisms(&[rect(0.2, 5.9, 0.2, 7.8)], 0.0, 2.8))
        .with_mesh(id("b"), prisms(&[rect(6.1, 11.8, 0.2, 7.8)], 0.0, 2.8));
    if certified {
        geometry.with_extent_bounds(
            id("slab"),
            ([0.0, 0.0, -0.25], [12.0, 8.0, 0.0]),
            Some(([0.0, 0.0, -0.25], [12.0, 8.0, 0.0])),
        )
    } else {
        geometry
    }
}

const ELEMENTS: [&str; 3] = ["slab", "walls", "ceiling"];

fn upright(x: f64, y: f64) -> MetricDirection {
    MetricDirection::try_new([x, y, 0.0]).unwrap()
}

/// A rectangle along the plan axis `(x, y)`, or at any orientation.
fn rectangle(width: f64, depth: f64, along: Option<Xy>) -> PlacementShape {
    PlacementShape::Box {
        shape: BoxClearance::try_new(width, depth, 2.0).unwrap(),
        orientation: along.map_or(PlacementOrientation::Any, |(x, y)| {
            PlacementOrientation::Fixed(
                MetricFrame::try_new(
                    MetricPoint::try_new(id("frame"), [0.0, 0.0, 0.0]).unwrap(),
                    upright(x, y),
                    upright(-y, x),
                    MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
                )
                .unwrap(),
            )
        }),
    }
}

fn place(
    geometry: AxiolidGeometry,
    scope: &str,
    shape: PlacementShape,
    obstacles: &[&str],
) -> Result<PlacementOutcome, FreeSpaceError> {
    let request = PlacementRequest::new_in_domain(
        id(scope),
        shape,
        obstacles.iter().map(|o| id(o)).collect(),
        PlacementDomain::Supported(SupportedPlacement::try_new(id(scope), 0.0).unwrap()),
    )
    .unwrap();
    AxiolidFreeSpaceService::new(geometry, source()).find_placement(&request)
}

/// The witness's locator, which names what was bracketed.
fn found(outcome: Result<PlacementOutcome, FreeSpaceError>) -> String {
    match outcome {
        Ok(PlacementOutcome::Found(witness)) => {
            assert!(witness.evidence().exact);
            witness.evidence().locator.clone()
        }
        other => panic!("expected a placement, got {other:?}"),
    }
}

fn nowhere(outcome: Result<PlacementOutcome, FreeSpaceError>) -> String {
    match outcome {
        Ok(PlacementOutcome::NoPlacement(proof)) => {
            assert!(proof.evidence().exact);
            proof.evidence().locator.clone()
        }
        other => panic!("expected no placement, got {other:?}"),
    }
}

/// Both spaces of the room take a 0.9 m square at any orientation, and
/// refute one larger than either space, although the slab under them is
/// tessellated: its certified extent ends at the floor, so it occupies none
/// of the band whatever its curved hole does.
#[test]
fn a_room_over_a_slab_with_a_round_hole_is_decided() {
    for space in ["a", "b"] {
        found(place(
            holed_room(true),
            space,
            rectangle(0.9, 0.9, None),
            &ELEMENTS,
        ));
        nowhere(place(
            holed_room(true),
            space,
            rectangle(6.0, 6.0, Some((1.0, 0.0))),
            &ELEMENTS,
        ));
    }
}

/// A tessellated column standing in space `a` of [`holed_room`] with an
/// open quad beside it at `z`, its bottom at `bottom` and its top 1 m above:
/// one body of a closed piece and an open one.
fn column_with_sheet(bottom: f64) -> TriMesh {
    let mut column = prisms(&[rect(2.9, 3.2, 3.8, 4.2)], 0.0, 2.8);
    let first = u32::try_from(column.positions.len()).unwrap();
    column.positions.extend(
        [
            (1.0, 1.0, bottom),
            (1.2, 1.0, bottom),
            (1.2, 1.0, bottom + 1.0),
            (1.0, 1.0, bottom + 1.0),
        ]
        .map(|(x, y, z)| Point3::new(x, y, z)),
    );
    column
        .indices
        .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
    column
}

/// The sure core of a tessellated obstacle (`solid_column`) needs only the
/// pieces standing across its column to be closed: the column proves that a
/// 5 m square fits nowhere in space `a`, beside an open quad hanging in the
/// band. A quad standing on the floor leaves its piece's inside undecided,
/// and the request refuses rather than prove an absence on it.
#[test]
fn a_tessellated_column_beside_an_open_piece_is_measured() {
    let square = || rectangle(5.0, 5.0, Some((1.0, 0.0)));
    let obstacles = ["slab", "walls", "ceiling", "column"];
    found(place(holed_room(true), "a", square(), &ELEMENTS));
    let hanging =
        holed_room(true).with_tessellated_mesh(id("column"), column_with_sheet(1.0), 0.001);
    nowhere(place(hanging, "a", square(), &obstacles));
    let standing =
        holed_room(true).with_tessellated_mesh(id("column"), column_with_sheet(0.0), 0.001);
    let refused =
        place(standing, "a", square(), &obstacles).expect_err("an open piece on the floor");
    assert!(
        refused.to_string().contains("is not a closed solid"),
        "{refused}"
    );
}

/// Without a certified extent the slab's top may lie its chord deviation
/// above the floor, inside the band, wherever it is: no witness holds for
/// every such slab, so a fit is refused rather than passed.
#[test]
fn a_slab_that_may_rise_into_the_band_refuses_a_fit() {
    assert_eq!(
        place(
            holed_room(false),
            "a",
            rectangle(0.9, 0.9, Some((1.0, 0.0))),
            &ELEMENTS
        ),
        Err(FreeSpaceError::InexactPlacementEvidence)
    );
    // The scope alone still refutes what fits nowhere in it.
    nowhere(place(
        holed_room(false),
        "a",
        rectangle(6.0, 6.0, Some((1.0, 0.0))),
        &ELEMENTS,
    ));
}

/// A 4 x 1.2 room whose far part a tessellated body fills from `x = gap`.
/// A 1.0 m wide rectangle along x fits only beside it.
fn alcove(gap: f64, deviation: f64) -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 1.2)], 0.0, 3.0))
        .with_tessellated_mesh(
            id("body"),
            prisms(&[rect(gap, 4.0, -0.5, 1.7)], 0.0, 3.0),
            deviation,
        )
}

/// A fit wider than the rectangle by more than the deviation is found, one
/// narrower by more than it is refuted, and one within it of the
/// rectangle's width is refused: the body's true face may lie on either
/// side of it.
#[test]
fn a_fit_within_the_deviation_is_refused_not_decided() {
    let deviation = 0.01;
    let shape = || rectangle(1.0, 1.0, Some((1.0, 0.0)));
    let locator = found(place(alcove(1.03, deviation), "room", shape(), &["body"]));
    assert!(
        locator.contains(":within-chord-deviation=") && locator.contains("body"),
        "{locator}"
    );
    nowhere(place(alcove(0.97, deviation), "room", shape(), &["body"]));
    for gap in [0.995, 1.0, 1.005] {
        assert_eq!(
            place(alcove(gap, deviation), "room", shape(), &["body"]),
            Err(FreeSpaceError::InexactPlacementEvidence),
            "gap {gap}"
        );
    }
    // Exact, the same body decides both sides of the width.
    let exact = |gap: f64| {
        AxiolidGeometry::new()
            .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 1.2)], 0.0, 3.0))
            .with_mesh(id("body"), prisms(&[rect(gap, 4.0, -0.5, 1.7)], 0.0, 3.0))
    };
    let locator = found(place(exact(1.005), "room", shape(), &["body"]));
    assert!(!locator.contains("within-chord-deviation"), "{locator}");
    nowhere(place(exact(0.995), "room", shape(), &["body"]));
}

/// A tessellated scope is searched eroded by its deviation for a witness
/// and grown by it for a proof of absence.
#[test]
fn a_tessellated_scope_is_bracketed_both_ways() {
    let room = |side: f64| {
        AxiolidGeometry::new().with_tessellated_mesh(
            id("room"),
            prisms(&[rect(0.0, side, 0.0, side)], 0.0, 3.0),
            0.01,
        )
    };
    let shape = || rectangle(1.0, 1.0, Some((1.0, 0.0)));
    found(place(room(1.05), "room", shape(), &[]));
    nowhere(place(room(0.95), "room", shape(), &[]));
    assert_eq!(
        place(room(1.0), "room", shape(), &[]),
        Err(FreeSpaceError::InexactPlacementEvidence)
    );
}

/// A deviation that bounds nothing refuses as before.
#[test]
fn a_deviation_that_bounds_nothing_refuses() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 4.0)], 0.0, 3.0))
        .with_tessellated_mesh(
            id("column"),
            prisms(&round_column((3.0, 3.0), 0.2), 0.0, 3.0),
            f64::NAN,
        );
    assert_eq!(
        place(geometry, "room", rectangle(0.9, 0.9, None), &["column"]),
        Err(FreeSpaceError::InexactPlacementEvidence)
    );
}

/// The room of #304's coordinates: a 5 x 4 room at (600 000, 5 600 000),
/// turned by 2.3°, with a round tessellated column in it. Band footprints
/// there need a snapping tolerance coarser than `ON_SURFACE` (eight units in
/// the last place of 5.6e6 are 1e-8 m); the verdicts are those of the room
/// at the origin.
#[test]
fn a_room_far_from_the_origin_is_decided() {
    let (s, c) = 2.3_f64.to_radians().sin_cos();
    let at = |(x, y): Xy| (600_000.0 + x * c - y * s, 5_600_000.0 + x * s + y * c);
    let quad = |q: [Xy; 4]| q.map(at);
    let column: Vec<[Xy; 4]> = round_column((3.8, 2.0), 0.3)
        .into_iter()
        .map(quad)
        .collect();
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("room"),
            prisms(&[quad(rect(0.0, 5.0, 0.0, 4.0))], 0.0, 3.0),
        )
        .with_mesh(
            id("wall"),
            prisms(&[quad(rect(5.0, 5.2, -0.2, 4.2))], 0.0, 3.0),
        )
        .with_tessellated_mesh(id("column"), prisms(&column, 0.0, 3.0), sagitta(0.3));
    let obstacles = ["wall", "column"];
    let locator = found(place(
        geometry.clone(),
        "room",
        rectangle(0.9, 0.9, None),
        &obstacles,
    ));
    assert!(locator.contains("column"), "{locator}");
    nowhere(place(
        geometry,
        "room",
        rectangle(4.5, 4.5, Some((c, s))),
        &obstacles,
    ));
}

/// A tessellated body elsewhere on the floor, its plan box apart from the
/// room's, is left out of the room's search: the fit is found as without
/// it, and the evidence names no body it was bracketed by.
#[test]
fn a_tessellated_body_elsewhere_is_left_out() {
    let geometry = || {
        AxiolidGeometry::new()
            .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 1.2)], 0.0, 3.0))
            .with_tessellated_mesh(
                id("far"),
                prisms(&[rect(10.0, 12.0, 0.0, 1.2)], 0.0, 3.0),
                0.01,
            )
    };
    let shape = || rectangle(1.0, 1.0, Some((1.0, 0.0)));
    let locator = found(place(geometry(), "room", shape(), &["far"]));
    assert!(!locator.contains("within-chord-deviation"), "{locator}");
    nowhere(place(
        geometry(),
        "room",
        rectangle(6.0, 6.0, Some((1.0, 0.0))),
        &["far"],
    ));
}

/// An unmeasured obstacle refuses a search it may reach, but one the host
/// bounds apart from the room (`with_unmeasured_bound`) is left out.
#[test]
fn an_unmeasured_obstacle_bounded_elsewhere_is_left_out() {
    let room = || {
        AxiolidGeometry::new()
            .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 1.2)], 0.0, 3.0))
            .with_unmeasured(id("lost"), "not meshed")
    };
    let shape = || rectangle(1.0, 1.0, Some((1.0, 0.0)));
    assert!(matches!(
        place(room(), "room", shape(), &["lost"]),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
    let far = room().with_unmeasured_bound(id("lost"), [10.0, 0.0, 0.0], [12.0, 1.2, 3.0]);
    found(place(far, "room", shape(), &["lost"]));
    let near = room().with_unmeasured_bound(id("lost"), [3.0, 0.0, 0.0], [5.0, 1.2, 3.0]);
    assert!(matches!(
        place(near, "room", shape(), &["lost"]),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
}
