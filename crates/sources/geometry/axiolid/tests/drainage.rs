//! How water runs off a roof to its drains, over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{DrainagePath, DrainageRequest, ProximityError, ProximityService};
use axioval_ir::{ObjectId, SourceId};

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "roof").unwrap(), local).unwrap()
}

/// A roof slab 10 m deep (y 0..10), its top falling 0.15 m from its
/// edges at 0.25 m to the middle of each of `cells` (`x0`, `x1`, the
/// middle's height): four planes around each middle, the valleys from the
/// corners down to it, the slab's underside at -0.3 m. With `open`, only
/// the top, as a sheet.
fn roof(cells: &[(f64, f64, f64)], open: bool) -> TriMesh {
    const EDGE: f64 = 0.25;
    const UNDER: f64 = -0.3;
    let mut xs: Vec<f64> = cells.iter().map(|cell| cell.0).collect();
    xs.push(cells.last().unwrap().1);
    // The border, anticlockwise from above: along y = 0, back along y = 10.
    let mut ring: Vec<[f64; 2]> = xs.iter().map(|x| [*x, 0.0]).collect();
    ring.extend(xs.iter().rev().map(|x| [*x, 10.0]));
    let n = ring.len();
    let mut points: Vec<Point3> = ring
        .iter()
        .map(|[x, y]| Point3::new(*x, *y, EDGE))
        .collect();
    let mut indices: Vec<u32> = Vec::new();
    let at = |x: f64, y: f64| -> u32 {
        u32::try_from(
            ring.iter()
                .position(|p| (p[0] - x).abs() < 1e-12 && (p[1] - y).abs() < 1e-12)
                .unwrap(),
        )
        .unwrap()
    };
    for &(x0, x1, middle) in cells {
        let centre = u32::try_from(points.len()).unwrap();
        points.push(Point3::new(f64::midpoint(x0, x1), 5.0, middle));
        let corners = [at(x0, 0.0), at(x1, 0.0), at(x1, 10.0), at(x0, 10.0)];
        for i in 0..4 {
            indices.extend([centre, corners[i], corners[(i + 1) % 4]]);
        }
    }
    if !open {
        let base = u32::try_from(points.len()).unwrap();
        points.extend(ring.iter().map(|[x, y]| Point3::new(*x, *y, UNDER)));
        let count = u32::try_from(n).unwrap();
        for i in 0..count {
            let j = (i + 1) % count;
            indices.extend([i, base + i, base + j, i, base + j, j]);
        }
        for &(x0, x1, _) in cells {
            let corners = [at(x0, 0.0), at(x1, 0.0), at(x1, 10.0), at(x0, 10.0)].map(|c| base + c);
            indices.extend([corners[0], corners[2], corners[1]]);
            indices.extend([corners[0], corners[3], corners[2]]);
        }
    }
    TriMesh::new(points, indices)
}

/// A drain 0.3 m square in plan around (`x`, `y`), standing in the slab.
fn drain(x: f64, y: f64) -> TriMesh {
    let (min, max) = ([x - 0.15, y - 0.15, -0.2], [x + 0.15, y + 0.15, 0.1]);
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let points = vec![
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

/// Two drains at the low points of two cells, and between them a dished
/// cell 4 m wide whose middle no drain takes.
fn dished() -> AxiolidProximityService {
    AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(
                id("roof"),
                roof(
                    &[(0.0, 10.0, 0.1), (10.0, 14.0, 0.15), (14.0, 24.0, 0.1)],
                    false,
                ),
            )
            .with_mesh(id("drain-1"), drain(5.0, 5.0))
            .with_mesh(id("drain-2"), drain(19.0, 5.0)),
    )
}

/// The same falls without the dish: every point runs to a drain.
fn falling(open: bool) -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(
            id("roof"),
            roof(&[(0.0, 10.0, 0.1), (10.0, 20.0, 0.1)], open),
        )
        .with_mesh(id("drain-1"), drain(5.0, 5.0))
        .with_mesh(id("drain-2"), drain(15.0, 5.0))
}

fn request(outlets: &[&str], fall: f64) -> DrainageRequest {
    DrainageRequest::try_new(id("roof"), outlets.iter().map(|o| id(o)).collect(), fall).unwrap()
}

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-6
}

/// The longest run: from a corner down the valley until the drain's
/// footprint, 4.85 m off in x and y, along a valley falling 0.15 m over
/// 5√2 m.
fn longest() -> f64 {
    4.85 * 2.0_f64.sqrt() * (1.0 + (0.15 / (5.0 * 2.0_f64.sqrt())).powi(2)).sqrt()
}

#[test]
fn a_dished_area_between_two_drains_does_not_drain() {
    let service = dished();
    let measured = service
        .measure_drainage(&request(&["drain-1", "drain-2"], 0.0))
        .unwrap();
    let (area_low, area_high) = measured.surface_square_metres();
    assert!(
        near(area_low, 240.0) && near(area_high, 240.0),
        "{measured:?}"
    );
    let (lower, upper) = measured.undrained_square_metres();
    assert!(near(lower, 40.0) && near(upper, 40.0), "{measured:?}");
    assert!(measured.evidence().exact, "{measured:?}");
    let DrainagePath::Bounded {
        lower_metres,
        upper_metres,
    } = measured.path()
    else {
        panic!("{measured:?}");
    };
    assert!(
        lower_metres <= longest() && longest() <= upper_metres,
        "{lower_metres} {upper_metres}"
    );
    assert!(lower_metres > longest() - 1e-6, "{lower_metres}");
    // The valley's last 0.15√2 m lie over the drain.
    assert!(upper_metres < 7.08, "{upper_metres}");
}

#[test]
fn a_correctly_falling_roof_drains() {
    for open in [false, true] {
        let service = AxiolidProximityService::new(falling(open));
        let measured = service
            .measure_drainage(&request(&["drain-1", "drain-2"], 0.02))
            .unwrap();
        assert_eq!(
            measured.undrained_square_metres(),
            (0.0, 0.0),
            "{measured:?}"
        );
        assert!(near(measured.surface_square_metres().1, 200.0));
        assert!(measured.evidence().exact);
        assert!(
            matches!(measured.path(), DrainagePath::Bounded { lower_metres, upper_metres }
                if lower_metres <= longest() && longest() <= upper_metres),
            "{measured:?}"
        );
    }
}

#[test]
fn faces_falling_less_than_the_minimum_hold_water() {
    let service = AxiolidProximityService::new(falling(false));
    // The faces fall 3 %: below a 5 % minimum the roof holds its water, but
    // for the rain falling straight into the drains, 0.09 m² each.
    let measured = service
        .measure_drainage(&request(&["drain-1", "drain-2"], 0.05))
        .unwrap();
    let (lower, upper) = measured.undrained_square_metres();
    assert!(
        (lower - 199.82).abs() < 1e-6 && near(upper, 200.0),
        "{measured:?}"
    );
    // At exactly 3 % whether they shed water is undecided: none of it
    // surely drains, none surely holds.
    let measured = service
        .measure_drainage(&request(&["drain-1", "drain-2"], 0.03))
        .unwrap();
    let (lower, upper) = measured.undrained_square_metres();
    assert!(near(lower, 0.0) && near(upper, 200.0), "{measured:?}");
    assert!(!measured.evidence().exact);
}

#[test]
fn without_the_drain_its_catchment_does_not_drain() {
    let service = AxiolidProximityService::new(falling(false));
    let measured = service
        .measure_drainage(&request(&["drain-2"], 0.0))
        .unwrap();
    let (lower, upper) = measured.undrained_square_metres();
    assert!(near(lower, 100.0) && near(upper, 100.0), "{measured:?}");
    let measured = service.measure_drainage(&request(&[], 0.0)).unwrap();
    assert!(near(measured.undrained_square_metres().0, 200.0));
    assert_eq!(measured.path(), DrainagePath::NoOutlet);
}

#[test]
fn water_running_off_the_edge_is_lost() {
    // One plane falling 3 % towards y = 0, with a drain in its middle: the
    // drain takes only what runs over it, the rest leaves over the edge.
    let points = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(10.0, 0.0, 0.0),
        Point3::new(10.0, 10.0, 0.3),
        Point3::new(0.0, 10.0, 0.3),
    ];
    let sheet = TriMesh::new(points, vec![0, 1, 2, 0, 2, 3]);
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("roof"), sheet)
            .with_mesh(id("drain-1"), drain(5.0, 5.0)),
    );
    let measured = service
        .measure_drainage(&request(&["drain-1"], 0.0))
        .unwrap();
    let (lower, upper) = measured.undrained_square_metres();
    // Neither face surely drains. The lower face runs off the edge but for
    // the drain and what runs over it, half its 0.3 m square; the upper
    // face runs into the lower one.
    assert!(
        (lower - (50.0 - 0.045)).abs() < 1e-6 && near(upper, 100.0),
        "{measured:?}"
    );
    // With a gutter along the low edge it all drains.
    let gutter = TriMesh::new(
        vec![
            Point3::new(-0.1, -0.2, -0.2),
            Point3::new(10.1, -0.2, -0.2),
            Point3::new(10.1, 0.1, -0.2),
            Point3::new(-0.1, 0.1, -0.2),
            Point3::new(-0.1, -0.2, 0.0),
            Point3::new(10.1, -0.2, 0.0),
            Point3::new(10.1, 0.1, 0.0),
            Point3::new(-0.1, 0.1, 0.0),
        ],
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ],
    );
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(
                id("roof"),
                TriMesh::new(
                    vec![
                        Point3::new(0.0, 0.0, 0.0),
                        Point3::new(10.0, 0.0, 0.0),
                        Point3::new(10.0, 10.0, 0.3),
                        Point3::new(0.0, 10.0, 0.3),
                    ],
                    vec![0, 1, 2, 0, 2, 3],
                ),
            )
            .with_mesh(id("gutter"), gutter),
    );
    let measured = service
        .measure_drainage(&request(&["gutter"], 0.0))
        .unwrap();
    assert_eq!(
        measured.undrained_square_metres(),
        (0.0, 0.0),
        "{measured:?}"
    );
    // The longest run is down the whole plane from its top edge to the
    // gutter's inner edge, 9.9 m in plan.
    let longest = 9.9 * (1.0_f64 + 0.03 * 0.03).sqrt();
    assert!(
        matches!(measured.path(), DrainagePath::Bounded { lower_metres, upper_metres }
            if (lower_metres - longest).abs() < 1e-6 && longest <= upper_metres),
        "{measured:?}"
    );
}

#[test]
fn a_tessellated_roof_never_surely_drains() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(
            id("roof"),
            roof(&[(0.0, 10.0, 0.1), (10.0, 20.0, 0.1)], false),
            0.001,
        )
        .with_mesh(id("drain-1"), drain(5.0, 5.0))
        .with_mesh(id("drain-2"), drain(15.0, 5.0));
    let measured = AxiolidProximityService::new(geometry)
        .measure_drainage(&request(&["drain-1", "drain-2"], 0.0))
        .unwrap();
    let (lower, upper) = measured.undrained_square_metres();
    assert!(near(lower, 0.0) && near(upper, 200.0), "{measured:?}");
    assert_eq!(measured.path(), DrainagePath::Unbounded);
    assert!(!measured.evidence().exact);
}

#[test]
fn a_bodiless_outlet_or_surface_refuses() {
    let geometry = falling(false).with_no_body(id("drain-3"));
    let service = AxiolidProximityService::new(geometry);
    assert_eq!(
        service.measure_drainage(&request(&["drain-3"], 0.0)),
        Err(ProximityError::NoBody)
    );
}
