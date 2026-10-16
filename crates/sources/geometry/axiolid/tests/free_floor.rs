//! The free floor a placement request leaves (#391): its area, bracketed,
//! and the obstacles surely or possibly taking it.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    CylinderClearance, FreeSpaceService, PlacementDomain, PlacementOutcome, PlacementRequest,
    PlacementShape, SupportedPlacement,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

type Xy = (f64, f64);

/// Closed, outward-wound upright prisms over the given plan quads
/// (counter-clockwise corners), as one mesh.
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

/// Sides of the polygon the round column is tessellated with.
const SIDES: u32 = 16;

/// A round column of `radius` on `centre`, as quads of a ring around a
/// square core, and the chord deviation of its polygon.
fn round_column(centre: Xy, radius: f64) -> (Vec<[Xy; 4]>, f64) {
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
    for k in (0..SIDES).step_by(2) {
        quads.push([centre, at(k, core), at(k + 1, core), at(k + 2, core)]);
    }
    let deviation = radius * (1.0 - (std::f64::consts::PI / f64::from(SIDES)).cos());
    (quads, deviation)
}

/// Room `room` (x 0..4, y 0..3) between walls, a cabinet of 1 x 0.6 m in
/// its south-west corner, a lamp hanging above a 2 m band, a crate in the
/// next room and the next room itself.
fn furnished() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), prisms(&[rect(0.0, 4.0, 0.0, 3.0)], 0.0, 2.8))
        .with_mesh(
            id("walls"),
            prisms(
                &[
                    rect(-0.2, 4.2, -0.2, 0.0),
                    rect(-0.2, 4.2, 3.0, 3.2),
                    rect(-0.2, 0.0, 0.0, 3.0),
                    rect(4.0, 4.2, 0.0, 3.0),
                ],
                0.0,
                2.8,
            ),
        )
        .with_mesh(id("cabinet"), prisms(&[rect(0.0, 1.0, 0.0, 0.6)], 0.0, 0.9))
        .with_mesh(id("lamp"), prisms(&[rect(1.8, 2.2, 1.3, 1.7)], 2.3, 2.6))
        .with_mesh(id("next"), prisms(&[rect(4.2, 8.0, 0.0, 3.0)], 0.0, 2.8))
        .with_mesh(id("crate"), prisms(&[rect(5.0, 6.0, 1.0, 2.0)], 0.0, 0.5))
}

fn request(obstacles: &[&str]) -> PlacementRequest {
    PlacementRequest::new_in_domain(
        id("room"),
        PlacementShape::Cylinder(CylinderClearance::try_new(0.75, 2.0).unwrap()),
        obstacles.iter().map(|o| id(o)).collect(),
        PlacementDomain::Supported(SupportedPlacement::try_new(id("room"), 0.0).unwrap()),
    )
    .unwrap()
}

/// Only the cabinet takes the floor: the walls and the next room touch it,
/// the lamp hangs above the band, the crate stands elsewhere. With exact
/// bodies the area is one number, the room's less the cabinet's.
#[test]
fn only_what_stands_on_the_floor_within_the_band_takes_it() {
    let service = AxiolidFreeSpaceService::new(furnished(), source());
    let floor = service
        .measure_free_floor(&request(&["walls", "cabinet", "lamp", "next", "crate"]))
        .expect("measurable");
    assert_eq!(floor.obstacles(), [id("cabinet")]);
    assert_eq!(floor.possible(), &[] as &[ObjectId]);
    let (lower, upper) = (
        floor.area().lower_square_metres(),
        floor.area().upper_square_metres(),
    );
    assert!((lower - 11.4).abs() < 1e-9, "{lower}");
    assert!((upper - lower).abs() < 1e-9, "{upper}");
    assert!(floor.evidence().exact);
    assert_eq!(floor.evidence().locator, "axiolid:free-floor:room");
}

/// A tessellated round column surely takes floor, and the area is the
/// bracket its chord deviation leaves: within it of the true disc's.
#[test]
fn a_tessellated_column_takes_floor_within_its_deviation() {
    let radius = 0.3;
    let (quads, deviation) = round_column((3.0, 2.0), radius);
    let geometry = furnished()
        .with_tessellated_mesh(id("column"), prisms(&quads, 0.0, 2.8), deviation)
        .with_extent_bounds(
            id("column"),
            ([2.7, 1.7, 0.0], [3.3, 2.3, 2.8]),
            Some(([2.7, 1.7, 0.0], [3.3, 2.3, 2.8])),
        );
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let floor = service
        .measure_free_floor(&request(&["cabinet", "column"]))
        .expect("measurable");
    assert_eq!(floor.obstacles(), [id("cabinet"), id("column")]);
    let disc = std::f64::consts::PI * radius * radius;
    let (lower, upper) = (
        floor.area().lower_square_metres(),
        floor.area().upper_square_metres(),
    );
    assert!(lower < upper, "bracketed: {lower} {upper}");
    assert!(
        lower <= 11.4 - disc && 11.4 - disc <= upper,
        "{lower} {upper}"
    );
    assert!(upper - lower < 0.1, "{lower} {upper}");
    assert!(
        floor.evidence().locator.contains("column"),
        "{}",
        floor.evidence().locator
    );
}

/// The largest disc of the furnished room's floor is estimated where the
/// searches then decide it (#400): the room is 3 m deep, and the cabinet
/// keeps no disc of 1.5 m from its east end, so the largest has radius
/// 1.5. A disc of the radius estimated to fit is placed, one of the
/// radius estimated to exceed proven absent, and the two lie within a few
/// millimetres of it.
#[test]
fn the_largest_disc_is_estimated_where_the_searches_decide_it() {
    let service = AxiolidFreeSpaceService::new(furnished(), source());
    let obstacles = ["walls", "cabinet", "lamp", "next", "crate"];
    let estimate = service
        .estimate_largest_disc(&request(&obstacles))
        .expect("estimated");
    let (fits, exceeds) = (
        estimate.fits_radius_metres(),
        estimate.exceeds_radius_metres(),
    );
    assert!(fits <= 1.5 && 1.5 <= exceeds, "{fits} {exceeds}");
    assert!(exceeds - fits < 0.005, "{fits} {exceeds}");
    let sized = |radius: f64| {
        PlacementRequest::new_in_domain(
            id("room"),
            PlacementShape::Cylinder(CylinderClearance::try_new(radius, 2.0).unwrap()),
            obstacles.iter().map(|o| id(o)).collect(),
            PlacementDomain::Supported(SupportedPlacement::try_new(id("room"), 0.0).unwrap()),
        )
        .unwrap()
    };
    assert!(matches!(
        service.find_placement(&sized(fits)),
        Ok(PlacementOutcome::Found(_))
    ));
    assert!(matches!(
        service.find_placement(&sized(exceeds)),
        Ok(PlacementOutcome::NoPlacement(_))
    ));
}

/// Space footprints of axiolid/kernel#331, synthetic: a wedge whose apex
/// is three corners along a side of the 64-gon the disc morphology works
/// with, the middle one off the line through the others by an ulp --
/// outwards in the first two (the second along another side), inwards in
/// the third. The coordinates are those of the kernel's own test.
const PARALLEL_WALLS: [[Xy; 5]; 3] = [
    [
        (1.000_147_203_022_991_2, 1.997_003_613_631_385),
        (1.0, 2.0),
        (0.999_901_864_651_339_1, 2.001_997_590_912_41),
        (-0.023_427_428_718_893_285, 2.452_327_644_684_508),
        (0.001_351_746_817_966_015_3, 1.947_935_939_300_970_5),
    ],
    [
        (0.998_212_902_086_515_5, 2.002_409_622_594_436_6),
        (1.0, 2.0),
        (1.001_191_398_608_989_8, 1.998_393_584_937_042_2),
        (2.102_248_582_335_269_2, 2.192_489_123_692_456),
        (1.801_420_433_565_373_6, 2.598_108_927_089_279),
    ],
    [
        (1.000_147_203_022_973_2, 1.997_003_613_631_384),
        (0.999_999_999_999_999_9, 2.0),
        (0.999_901_864_651_351_1, 2.001_997_590_912_411),
        (-0.023_427_428_716_179_297, 2.452_327_644_690_649),
        (0.001_351_746_817_653_598_5, 1.947_935_939_306_962_4),
    ],
];

/// A closed, outward-wound upright prism over a counter-clockwise ring
/// whose first corner sees every other, fanned from that corner.
fn prism_over(ring: &[Xy], z0: f64, z1: f64) -> TriMesh {
    let n = u32::try_from(ring.len()).unwrap();
    let positions = [z0, z1]
        .into_iter()
        .flat_map(|z| ring.iter().map(move |(x, y)| Point3::new(*x, *y, z)))
        .collect();
    let mut indices = Vec::new();
    for k in 1..n - 1 {
        indices.extend([0, k + 1, k, n, n + k, n + k + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    TriMesh::new(positions, indices)
}

/// A tessellated space whose footprint has a corner an ulp off the chord
/// across it is measured, and a turning circle in it decided, at each
/// deviation (#415). axiolid-overlay 0.3.16 refused seven of these nine
/// footprints (`bracketed footprint: ZeroArea`); 0.3.15 and 0.3.17
/// measure them all. Each wedge has 0.255 m² of floor, which the bracket
/// holds within its deviation along the 2.6 m outline.
#[test]
fn a_footprint_with_a_corner_an_ulp_off_its_chord_is_measured() {
    for wall in PARALLEL_WALLS {
        // Counter-clockwise from the wedge's far corner, which sees the rest.
        let mut ring = wall.to_vec();
        ring.rotate_left(3);
        for deviation in [0.0005, 0.001, 0.002] {
            let geometry = AxiolidGeometry::new()
                .with_tessellated_mesh(id("space"), prism_over(&ring, 0.0, 2.5), deviation)
                .with_extent_bounds(
                    id("space"),
                    ([-1.0, 1.0, -deviation], [3.0, 3.0, 2.5 + deviation]),
                    None,
                );
            let service = AxiolidFreeSpaceService::new(geometry, source());
            let request = PlacementRequest::new_in_domain(
                id("space"),
                PlacementShape::Cylinder(CylinderClearance::try_new(0.05, 2.0).unwrap()),
                vec![],
                PlacementDomain::Supported(SupportedPlacement::try_new(id("space"), 0.0).unwrap()),
            )
            .unwrap();
            let floor = service
                .measure_free_floor(&request)
                .unwrap_or_else(|e| panic!("{wall:?} at {deviation}: {e:?}"));
            let (lower, upper) = (
                floor.area().lower_square_metres(),
                floor.area().upper_square_metres(),
            );
            assert!(
                lower < 0.255 && 0.255 < upper && upper - lower < 6.0 * deviation,
                "{wall:?} at {deviation}: {lower} {upper}"
            );
            assert!(
                matches!(
                    service.find_placement(&request),
                    Ok(PlacementOutcome::Found(_))
                ),
                "{wall:?} at {deviation}"
            );
        }
    }
}
