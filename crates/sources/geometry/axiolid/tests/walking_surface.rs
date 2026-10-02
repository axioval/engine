//! Stair flights, ramps and headroom over generated meshes.

use std::collections::BTreeMap;

use axiolid_core::{Point3, Tolerance};
use axiolid_mesh::{TriMesh, audit_mesh, compose};
use axioval_axiolid::{AxiolidGeometry, AxiolidWalkingSurfaceService};
use axioval_engine::{
    ClearWidthEvidence, ClearWidthRequest, ClearanceBelowRequest, HandrailEvidence,
    HandrailRequest, HeadroomRequest, LandingClearWidthRequest, LandingEvidence, LandingRequest,
    MeasuredInterval, MetricDirection, RailMeasurement, RailSide, RiserClosure, Tread, TreadFlight,
    TreadFlightRequest, WalkingEnd, WalkingLine, WalkingStretch, WalkingSurfaceError,
    WalkingSurfaceService,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// Whether `interval` holds `value`, up to the binary rounding of the
/// decimal coordinates it was measured from.
fn holds(interval: MeasuredInterval, value: f64) -> bool {
    interval.lower() - 1e-12 <= value && value <= interval.upper() + 1e-12
}

/// Ear-clipping triangulation of a simple counter-clockwise polygon.
fn triangulate(polygon: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut remaining: Vec<usize> = (0..polygon.len()).collect();
    let mut ears = Vec::new();
    while remaining.len() > 3 {
        let count = remaining.len();
        let ear = (0..count)
            .find(|&i| {
                let (p, c, n) = (
                    remaining[(i + count - 1) % count],
                    remaining[i],
                    remaining[(i + 1) % count],
                );
                if cross(polygon[p], polygon[c], polygon[n]) <= 0.0 {
                    return false;
                }
                remaining.iter().all(|&other| {
                    other == p
                        || other == c
                        || other == n
                        || cross(polygon[p], polygon[c], polygon[other]) < 0.0
                        || cross(polygon[c], polygon[n], polygon[other]) < 0.0
                        || cross(polygon[n], polygon[p], polygon[other]) < 0.0
                })
            })
            .expect("a simple polygon has an ear");
        ears.push([
            remaining[(ear + count - 1) % count],
            remaining[ear],
            remaining[(ear + 1) % count],
        ]);
        remaining.remove(ear);
    }
    ears.push([remaining[0], remaining[1], remaining[2]]);
    ears
}

/// A closed, outward-facing prism: `profile` (counter-clockwise, as
/// `(along, up)`) swept `width` across, turned `angle` radians in plan
/// from the x axis and moved by `offset`.
fn prism(profile: &[[f64; 2]], width: f64, angle: f64, offset: [f64; 3]) -> TriMesh {
    let (sin, cos) = angle.sin_cos();
    let place = |along: f64, across: f64, up: f64| {
        Point3::new(
            offset[0] + along * cos - across * sin,
            offset[1] + along * sin + across * cos,
            offset[2] + up,
        )
    };
    let n = profile.len();
    let mut points: Vec<Point3> = profile.iter().map(|p| place(p[0], 0.0, p[1])).collect();
    points.extend(profile.iter().map(|p| place(p[0], width, p[1])));
    let mut indices = Vec::new();
    for [a, b, c] in triangulate(profile) {
        // The profile's own winding faces away from the sweep.
        indices.extend([a, b, c].map(|i| u32::try_from(i).unwrap()));
        indices.extend([a + n, c + n, b + n].map(|i| u32::try_from(i).unwrap()));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n].map(|k| u32::try_from(k).unwrap()));
    }
    TriMesh::new(points, indices)
}

/// A stair side profile on a flat base: risers of the given heights, each
/// tread `going` deep; the last tread is the flight's top.
fn flight_profile(risers: &[f64], going: f64) -> Vec<[f64; 2]> {
    let steps = risers.len();
    let top: f64 = risers.iter().sum();
    #[allow(clippy::cast_precision_loss)]
    let length = going * steps as f64;
    let mut profile = vec![[0.0, 0.0], [length, 0.0], [length, top]];
    let mut elevation = top;
    for step in (0..steps).rev() {
        #[allow(clippy::cast_precision_loss)]
        let front = going * step as f64;
        profile.push([front, elevation]);
        elevation -= risers[step];
        if step > 0 {
            profile.push([front, elevation]);
        }
    }
    profile
}

fn service(geometry: AxiolidGeometry) -> AxiolidWalkingSurfaceService {
    AxiolidWalkingSurfaceService::new(geometry)
}

/// The flight of `local` walked along its centre line.
fn walk(
    stairs: &AxiolidWalkingSurfaceService,
    local: &str,
) -> Result<TreadFlight, WalkingSurfaceError> {
    stairs.measure_tread_flight(&TreadFlightRequest::new(id(local)))
}

fn direction(flight: &TreadFlight) -> MetricDirection {
    match flight.walking_line() {
        WalkingLine::Straight(direction) => *direction,
        WalkingLine::Turning(_) => panic!("a straight flight turns: {flight:?}"),
    }
}

fn flight(risers: &[f64], angle: f64) -> AxiolidWalkingSurfaceService {
    service(AxiolidGeometry::new().with_mesh(
        id("flight"),
        prism(&flight_profile(risers, 0.28), 1.2, angle, [2.0, 1.0, 0.0]),
    ))
}

#[test]
fn a_straight_flight_measures_its_risers_and_goings_exactly() {
    let measured = walk(&flight(&[0.17, 0.17, 0.21, 0.17], 0.0), "flight").unwrap();
    assert!(measured.is_exact(), "{measured:?}");
    assert!(direction(&measured) == MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap());
    assert_eq!(measured.treads().len(), 4);
    assert!(!measured.ends_in_riser());
    let risers = measured.risers();
    assert_eq!(risers.len(), 4);
    for (riser, expected) in risers.iter().zip([0.17, 0.17, 0.21, 0.17]) {
        assert!(holds(*riser, expected), "{riser:?} {expected}");
    }
    let goings = measured.goings();
    assert_eq!(goings.len(), 3);
    assert!(goings.iter().all(|going| holds(*going, 0.28)), "{goings:?}");
    assert!(holds(measured.rise(), 0.72));
    assert!(holds(measured.treads()[0].depth(), 0.28));
    assert!(measured.nosings().iter().all(|nosing| holds(*nosing, 0.0)));
    // Straight treads 1.2 m wide, every riser closed, no winder.
    for tread in measured.treads() {
        assert!(holds(tread.width().unwrap(), 1.2));
        assert_eq!(tread.riser_below(), RiserClosure::Closed);
    }
    for angle in measured.winder_angles() {
        let angle = angle.unwrap();
        assert!(angle.lower() == 0.0 && angle.upper() < 1e-12, "{angle:?}");
    }
    assert_eq!(
        measured.evidence().locator,
        format!("tread-flight:{}", id("flight"))
    );
}

#[test]
fn a_turned_flight_measures_within_the_rounding_of_its_direction() {
    let measured = walk(&flight(&[0.18; 5], 0.5), "flight").unwrap();
    assert!(!measured.is_exact());
    let [x, y, z] = direction(&measured).components();
    assert!((x - 0.5_f64.cos()).abs() < 1e-12 && (y - 0.5_f64.sin()).abs() < 1e-12);
    assert!(z.abs() < f64::EPSILON);
    assert!(
        measured
            .goings()
            .iter()
            .all(|going| holds(*going, 0.28) && going.upper() - going.lower() < 1e-12)
    );
    assert!(measured.risers().iter().all(|riser| holds(*riser, 0.18)));
}

#[test]
fn a_flight_ending_in_a_riser_counts_it() {
    // Treads at 0.18, 0.36 and 0.54; the last riser climbs to 0.72, the
    // edge of the upper floor, and the back falls steeply to the base.
    let profile = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [0.84, 0.72],
        [0.84, 0.54],
        [0.56, 0.54],
        [0.56, 0.36],
        [0.28, 0.36],
        [0.28, 0.18],
        [0.0, 0.18],
    ];
    let measured = walk(
        &service(
            AxiolidGeometry::new().with_mesh(id("flight"), prism(&profile, 1.0, 0.0, [0.0; 3])),
        ),
        "flight",
    )
    .unwrap();
    assert!(measured.ends_in_riser());
    assert_eq!(measured.treads().len(), 3);
    let risers = measured.risers();
    assert_eq!(risers.len(), 4);
    assert!(risers.iter().all(|riser| holds(*riser, 0.18)), "{risers:?}");
    assert!(holds(measured.rise(), 0.72));
    // The final riser rises from the last tread's back edge.
    assert_eq!(measured.riser_closures(), [RiserClosure::Closed; 4]);
}

#[test]
fn open_meshes_and_pieces_are_refused() {
    let mut open = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    open.indices.truncate(open.indices.len() - 3);
    let open = walk(
        &service(AxiolidGeometry::new().with_mesh(id("flight"), open)),
        "flight",
    );
    assert!(
        matches!(&open, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("closed")),
        "{open:?}"
    );

    let pieces = compose(&[
        prism(&flight_profile(&[0.18; 2], 0.28), 1.2, 0.0, [0.0; 3]),
        prism(
            &flight_profile(&[0.18; 2], 0.28),
            1.2,
            0.0,
            [0.56, 0.0, 0.36],
        ),
    ]);
    let pieces = walk(
        &service(AxiolidGeometry::new().with_mesh(id("flight"), pieces)),
        "flight",
    );
    assert!(
        matches!(&pieces, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("pieces")),
        "{pieces:?}"
    );
}

#[test]
fn bodiless_unmeasured_and_unknown_flights_are_refused() {
    let mesh = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("curved"), mesh, 0.001)
        .with_no_body(id("zone"))
        .with_unmeasured(id("broken"), "no representation");
    let stairs = service(geometry);
    // A tessellated ramp is still refused: its runs' planes are not
    // certified yet.
    assert!(matches!(
        stairs.measure_sloped_runs(&id("curved")),
        Err(WalkingSurfaceError::InexactGeometry(_))
    ));
    assert!(matches!(
        walk(&stairs, "zone"),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert!(matches!(
        stairs.measure_sloped_runs(&id("broken")),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert_eq!(
        walk(&stairs, "missing"),
        Err(WalkingSurfaceError::UnknownObject(id("missing")))
    );
}

/// A ramp rising `rise` over `length` between two 1 m landings, on a base
/// 0.1 m thick.
fn ramp_profile(rise: f64, length: f64) -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [length + 2.0, 0.0],
        [length + 2.0, 0.1 + rise],
        [length + 1.0, 0.1 + rise],
        [1.0, 0.1],
        [0.0, 0.1],
    ]
}

#[test]
fn a_ramp_measures_the_slope_of_its_run() {
    let ramps = service(
        AxiolidGeometry::new()
            .with_mesh(
                id("gentle"),
                prism(&ramp_profile(0.5, 6.0), 1.5, 0.0, [0.0; 3]),
            )
            .with_mesh(
                id("turned"),
                prism(&ramp_profile(0.5, 3.0), 1.5, 2.0, [10.0, 0.0, 0.0]),
            ),
    );
    let gentle = ramps.measure_sloped_runs(&id("gentle")).unwrap();
    assert!(gentle.is_exact());
    assert_eq!(gentle.runs().len(), 1);
    let run = gentle.runs()[0];
    assert!(run.direction() == MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap());
    assert!(holds(run.rise(), 0.5) && holds(run.length(), 6.0));
    assert!(holds(run.slope(), 0.5 / 6.0), "{:?}", run.slope());

    let turned = ramps.measure_sloped_runs(&id("turned")).unwrap();
    assert!(!turned.is_exact());
    let run = turned.runs()[0];
    let [x, y, _] = run.direction().components();
    assert!((x - 2.0_f64.cos()).abs() < 1e-9 && (y - 2.0_f64.sin()).abs() < 1e-9);
    assert!(holds(run.slope(), 0.5 / 3.0), "{:?}", run.slope());
    assert!(run.slope().upper() - run.slope().lower() < 1e-12);
}

#[test]
fn a_ramp_bending_without_a_landing_or_without_a_slope_is_refused() {
    let bent = vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 1.1],
        [9.0, 1.1],
        [5.0, 0.5],
        [1.0, 0.1],
        [0.0, 0.1],
    ];
    let ramps = service(
        AxiolidGeometry::new()
            .with_mesh(id("bent"), prism(&bent, 1.5, 0.0, [0.0; 3]))
            .with_mesh(
                id("flight"),
                prism(&flight_profile(&[0.18; 3], 0.28), 1.2, 0.0, [0.0; 3]),
            ),
    );
    assert!(matches!(
        ramps.measure_sloped_runs(&id("bent")),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("planar")
    ));
    assert!(matches!(
        ramps.measure_sloped_runs(&id("flight")),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("no sloped face")
    ));
    // Two runs separated by a landing are two runs.
    let two = vec![
        [0.0, 0.0],
        [9.0, 0.0],
        [9.0, 0.6],
        [8.0, 0.6],
        [6.0, 0.4],
        [4.0, 0.4],
        [1.0, 0.1],
        [0.0, 0.1],
    ];
    let ramp =
        service(AxiolidGeometry::new().with_mesh(id("two"), prism(&two, 1.5, 0.0, [0.0; 3])))
            .measure_sloped_runs(&id("two"))
            .unwrap();
    assert_eq!(ramp.runs().len(), 2);
    assert!(holds(ramp.runs()[0].slope(), 0.1));
    assert!(holds(ramp.runs()[1].slope(), 0.1));
    assert!(holds(ramp.runs()[1].length(), 2.0));
}

/// A closed, outward box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    prism(
        &[
            [min[0], min[2]],
            [max[0], min[2]],
            [max[0], max[2]],
            [min[0], max[2]],
        ],
        max[1] - min[1],
        0.0,
        [0.0, min[1], 0.0],
    )
}

#[test]
fn headroom_is_the_least_clearance_above_the_walking_surface() {
    // Flight treads at 0.18 .. 0.72 from x = 0 to 1.12, y 0 .. 1.2; a beam
    // at 2.6 over the third tread (x 0.56 .. 0.84), a slab at 3 over all.
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("flight"),
            prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]),
        )
        .with_mesh(id("beam"), cuboid([0.6, -1.0, 2.6], [0.8, 2.0, 2.9]))
        .with_mesh(id("slab"), cuboid([-1.0, -1.0, 3.0], [3.0, 3.0, 3.2]))
        .with_mesh(id("beside"), cuboid([0.0, 2.0, 0.0], [1.0, 3.0, 1.0]))
        .with_mesh(id("floor"), cuboid([-1.0, -1.0, -0.2], [3.0, 3.0, 0.0]))
        .with_mesh(id("post"), cuboid([0.1, 0.1, -0.5], [0.2, 0.2, 1.0]))
        .with_tessellated_mesh(id("duct"), cuboid([0.0, 0.0, 2.0], [1.0, 1.0, 2.2]), 0.01)
        .with_no_body(id("zone"))
        .with_unmeasured(id("broken"), "no representation");
    let stairs = service(geometry);
    let measure = |obstacles: &[&str]| {
        stairs.measure_headroom(&HeadroomRequest::new(
            id("flight"),
            obstacles.iter().map(|local| id(local)),
        ))
    };

    let headroom = measure(&["beam", "slab", "beside", "floor", "zone"]).unwrap();
    let clearance = headroom.clearance().unwrap();
    assert!(holds(clearance, 2.6 - 0.54), "{clearance:?}");
    assert!(clearance.lower() < clearance.upper());
    assert_eq!(headroom.governing(), &[id("beam")]);
    assert!(!headroom.evidence().exact);

    let slab = measure(&["slab"]).unwrap();
    assert!(holds(slab.clearance().unwrap(), 3.0 - 0.72));

    let clear = measure(&["beside", "floor", "zone"]).unwrap();
    assert_eq!(clear.clearance(), None);
    assert!(clear.governing().is_empty());

    assert!(matches!(
        measure(&["post"]),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("crosses")
    ));
    assert!(matches!(
        measure(&["duct"]),
        Err(WalkingSurfaceError::InexactGeometry(_))
    ));
    assert!(matches!(
        measure(&["broken"]),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert_eq!(
        measure(&["missing"]),
        Err(WalkingSurfaceError::UnknownObject(id("missing")))
    );
}

#[test]
fn headroom_above_a_ramp_follows_its_slope() {
    // A run rising 0.5 over x 1 .. 7; a slab underside at 2.5 over x 5 .. 6,
    // where the run lies at 0.1 + 0.5 * (x - 1) / 6, highest at x = 6.
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("ramp"),
            prism(&ramp_profile(0.5, 6.0), 1.5, 0.0, [0.0; 3]),
        )
        .with_mesh(id("slab"), cuboid([5.0, 0.0, 2.5], [6.0, 1.5, 2.7]));
    let headroom = service(geometry)
        .measure_headroom(&HeadroomRequest::new(id("ramp"), [id("slab")]))
        .unwrap();
    let expected = 2.5 - (0.1 + 0.5 * 5.0 / 6.0);
    assert!(holds(headroom.clearance().unwrap(), expected));
}

/// A placement transform leaves a unit in the last place in coordinates: a
/// tread whose corners differ by that much is still a tread, measured at
/// the interval of its corners' elevations and never exactly.
#[test]
fn a_tread_off_level_by_rounding_is_measured_as_an_interval() {
    let mut mesh = prism(&flight_profile(&[0.18; 3], 0.28), 1.2, 0.0, [0.0; 3]);
    let nudged = mesh
        .positions
        .iter()
        .position(|point| (point.z - 0.36).abs() < 1e-12)
        .unwrap();
    let level = mesh.positions[nudged].z;
    mesh.positions[nudged].z = level.next_down();
    let measured = walk(
        &service(AxiolidGeometry::new().with_mesh(id("flight"), mesh)),
        "flight",
    )
    .unwrap();
    assert!(!measured.is_exact());
    let second = measured.treads()[1].elevation();
    assert_eq!(
        (second.lower_metres(), second.upper_metres()),
        (level.next_down(), level)
    );
    assert!(measured.risers().iter().all(|riser| holds(*riser, 0.18)));

    // A real fall is no rounding: the tread slopes and is refused.
    let mut mesh = prism(&flight_profile(&[0.18; 3], 0.28), 1.2, 0.0, [0.0; 3]);
    mesh.positions[nudged].z = 0.359;
    let sloped = walk(
        &service(AxiolidGeometry::new().with_mesh(id("flight"), mesh)),
        "flight",
    );
    assert!(
        matches!(&sloped, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("sloped")),
        "{sloped:?}"
    );
}

/// One geometry set may hold several models: evidence about a flight, a
/// ramp or the headroom above it cites the measured object's own source.
#[test]
fn evidence_cites_each_measured_objects_own_source() {
    let other = SourceId::new("cad", "other").expect("valid source");
    let ramp = ObjectId::new(other.clone(), "ramp").expect("valid id");
    let stairs = service(
        AxiolidGeometry::new()
            .with_mesh(
                id("flight"),
                prism(&flight_profile(&[0.18; 3], 0.28), 1.2, 0.0, [0.0; 3]),
            )
            .with_mesh(
                ramp.clone(),
                prism(&ramp_profile(0.5, 6.0), 1.5, 0.0, [0.0, 5.0, 0.0]),
            )
            .with_mesh(id("slab"), cuboid([-1.0, 3.0, 3.0], [9.0, 7.0, 3.2])),
    );
    let flight = walk(&stairs, "flight").unwrap();
    assert_eq!(flight.evidence().source, source());
    let sloped = stairs.measure_sloped_runs(&ramp).unwrap();
    assert_eq!(sloped.evidence().source, other);
    let headroom = stairs
        .measure_headroom(&HeadroomRequest::new(ramp.clone(), [id("slab")]))
        .unwrap();
    assert_eq!(headroom.evidence().source, other);
    assert_eq!(headroom.governing(), &[id("slab")]);
}

#[test]
fn a_rectangular_flight_and_run_measure_their_width() {
    let straight = flight(&[0.17; 4], 0.0)
        .measure_tread_flight(&TreadFlightRequest::new(id("flight")))
        .unwrap();
    assert!(straight.is_exact());
    let width = straight.width().unwrap();
    assert!(width.is_point() && holds(width, 1.2), "{width:?}");
    assert!(
        straight
            .treads()
            .iter()
            .all(|tread| tread.sides().is_some())
    );

    let turned = flight(&[0.18; 5], 0.5)
        .measure_tread_flight(&TreadFlightRequest::new(id("flight")))
        .unwrap();
    let width = turned.width().unwrap();
    assert!(holds(width, 1.2) && !width.is_point(), "{width:?}");
    assert!(width.upper() - width.lower() < 1e-12);

    let ramp = service(AxiolidGeometry::new().with_mesh(
        id("ramp"),
        prism(&ramp_profile(0.5, 6.0), 1.5, 0.0, [0.0; 3]),
    ))
    .measure_sloped_runs(&id("ramp"))
    .unwrap();
    assert!(ramp.is_exact());
    assert!(holds(ramp.runs()[0].width().unwrap(), 1.5));
}

#[test]
fn a_tread_filling_no_rectangle_has_no_width() {
    // Pull one back corner of the top tread in: a trapezoid in plan.
    let mut mesh = prism(&flight_profile(&[0.18; 3], 0.28), 1.2, 0.0, [0.0; 3]);
    #[allow(clippy::float_cmp)]
    for point in &mut mesh.positions {
        if point.y == 1.2 && (point.x - 0.84).abs() < 1e-9 {
            point.y = 1.0;
        }
    }
    let measured = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh))
        .measure_tread_flight(&TreadFlightRequest::new(id("flight")))
        .unwrap();
    assert!(measured.treads()[0].width().is_some());
    assert!(measured.treads()[2].width().is_none());
    assert_eq!(measured.width(), None);
}

/// A flight on the floor rising 0.72 m from x = 0 to 1.12, y 0 .. 1.2, its
/// last tread its top (x 0.84 .. 1.12), with `objects` beside it.
fn flight_with(objects: Vec<(&str, TriMesh)>) -> AxiolidWalkingSurfaceService {
    let mut geometry = AxiolidGeometry::new().with_mesh(
        id("flight"),
        prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]),
    );
    for (local, mesh) in objects {
        geometry = geometry.with_mesh(id(local), mesh);
    }
    service(geometry)
}

fn landing(
    stairs: &AxiolidWalkingSurfaceService,
    end: WalkingEnd,
    candidates: &[&str],
) -> Result<LandingEvidence, WalkingSurfaceError> {
    stairs.measure_landing(&LandingRequest::new(
        id("flight"),
        end,
        candidates.iter().map(|local| id(local)),
    ))
}

#[test]
fn a_flight_measures_the_landings_at_its_ends() {
    let stairs = flight_with(vec![
        ("landing", cuboid([1.12, -0.1, 0.52], [2.12, 1.4, 0.72])),
        ("floor", cuboid([-2.0, -1.0, -0.2], [3.0, 3.0, 0.0])),
        ("beam", cuboid([0.0, 0.0, 2.5], [3.0, 1.0, 2.8])),
    ]);
    let candidates = ["landing", "floor", "beam"];
    // The last tread is the flight's top: the landing counts from its nosing.
    let top = landing(&stairs, WalkingEnd::FlightTop, &candidates).unwrap();
    assert!(top.evidence().exact, "{top:?}");
    assert_eq!(top.landing().unwrap().carrier(), &id("landing"));
    assert!(holds(top.depth().unwrap(), 2.12 - 0.84), "{top:?}");
    assert!(holds(top.width().unwrap(), 1.5));
    assert!(top.direction() == MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap());
    // The floor runs on under the flight; its landing reaches 2 m back.
    let bottom = landing(&stairs, WalkingEnd::FlightBottom, &candidates).unwrap();
    assert_eq!(bottom.landing().unwrap().carrier(), &id("floor"));
    assert!(holds(bottom.depth().unwrap(), 2.0), "{bottom:?}");
    assert!(holds(bottom.width().unwrap(), 4.0));
    assert!(bottom.direction() == MetricDirection::try_new([-1.0, 0.0, 0.0]).unwrap());
    // Without candidates nothing carries a landing: the flight's own top
    // tread is no landing.
    let none = landing(&stairs, WalkingEnd::FlightTop, &[]).unwrap();
    assert!(none.landing().is_none());
    assert_eq!(none.depth(), None);
}

/// A closed, outward slab `thickness` thick under the plan polygon
/// `outline` (counter-clockwise), its top at `top`.
fn slab(outline: &[[f64; 2]], top: f64, thickness: f64) -> TriMesh {
    let n = outline.len();
    let mut points: Vec<Point3> = outline
        .iter()
        .map(|p| Point3::new(p[0], p[1], top - thickness))
        .collect();
    points.extend(outline.iter().map(|p| Point3::new(p[0], p[1], top)));
    let mut indices = Vec::new();
    for [a, b, c] in triangulate(outline) {
        // The bottom looks down, the top up.
        indices.extend([a, c, b].map(|i| u32::try_from(i).unwrap()));
        indices.extend([a + n, b + n, c + n].map(|i| u32::try_from(i).unwrap()));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, j + n, i, j + n, i + n].map(|k| u32::try_from(k).unwrap()));
    }
    TriMesh::new(points, indices)
}

#[test]
fn a_landing_off_level_apart_or_shaped_otherwise_is_told_apart() {
    // Standing 5 cm off the flight, or 2 cm lower, it meets nothing.
    let apart = flight_with(vec![
        ("gap", cuboid([1.17, 0.0, 0.52], [2.17, 1.2, 0.72])),
        ("low", cuboid([1.12, 0.0, 0.5], [2.12, 1.2, 0.7])),
    ]);
    let top = landing(&apart, WalkingEnd::FlightTop, &["gap", "low"]).unwrap();
    assert!(top.landing().is_none(), "{top:?}");
    // An L-shaped landing is found but not measured.
    let shaped = slab(
        &[
            [1.12, 0.0],
            [2.12, 0.0],
            [2.12, 0.2],
            [1.62, 0.2],
            [1.62, 1.2],
            [1.12, 1.2],
        ],
        0.72,
        0.2,
    );
    let stairs = flight_with(vec![("shaped", shaped)]);
    let top = landing(&stairs, WalkingEnd::FlightTop, &["shaped"]).unwrap();
    assert_eq!(top.landing().unwrap().carrier(), &id("shaped"));
    assert!(top.landing().unwrap().extent().is_none(), "{top:?}");
    assert_eq!(top.depth(), None);
    // Two surfaces meeting one end are refused, never merged.
    let two = flight_with(vec![
        ("a", cuboid([1.12, 0.0, 0.52], [2.12, 0.6, 0.72])),
        ("b", cuboid([1.12, 0.6, 0.52], [2.12, 1.2, 0.72])),
    ]);
    assert!(matches!(
        landing(&two, WalkingEnd::FlightTop, &["a", "b"]),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("several")
    ));
}

#[test]
fn unmeasurable_landing_candidates_are_refused() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("flight"),
            prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]),
        )
        .with_tessellated_mesh(
            id("curved"),
            cuboid([1.12, 0.0, 0.52], [2.12, 1.2, 0.72]),
            0.001,
        )
        .with_unmeasured(id("broken"), "no representation")
        .with_no_body(id("zone"));
    let stairs = service(geometry);
    assert!(matches!(
        landing(&stairs, WalkingEnd::FlightTop, &["curved"]),
        Err(WalkingSurfaceError::InexactGeometry(_))
    ));
    assert!(matches!(
        landing(&stairs, WalkingEnd::FlightTop, &["broken"]),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert!(
        landing(&stairs, WalkingEnd::FlightTop, &["zone"])
            .unwrap()
            .landing()
            .is_none()
    );
    // A flight has no ramp runs.
    assert!(matches!(
        landing(&stairs, WalkingEnd::RunTop(0), &[]),
        Err(WalkingSurfaceError::Unsupported(_))
    ));
}

#[test]
fn a_flight_ending_in_a_riser_counts_its_landing_from_that_riser() {
    let profile = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [0.84, 0.72],
        [0.84, 0.54],
        [0.56, 0.54],
        [0.56, 0.36],
        [0.28, 0.36],
        [0.28, 0.18],
        [0.0, 0.18],
    ];
    let stairs = service(
        AxiolidGeometry::new()
            .with_mesh(id("flight"), prism(&profile, 1.0, 0.0, [0.0; 3]))
            .with_mesh(id("upper"), cuboid([0.84, -1.0, 0.52], [3.0, 2.0, 0.72])),
    );
    let top = landing(&stairs, WalkingEnd::FlightTop, &["upper"]).unwrap();
    assert!(holds(top.depth().unwrap(), 3.0 - 0.84), "{top:?}");
    assert!(holds(top.width().unwrap(), 3.0));
}

#[test]
fn a_ramp_carries_its_own_landings() {
    let two = vec![
        [0.0, 0.0],
        [9.0, 0.0],
        [9.0, 0.6],
        [8.0, 0.6],
        [6.0, 0.4],
        [4.0, 0.4],
        [1.0, 0.1],
        [0.0, 0.1],
    ];
    let ramps =
        service(AxiolidGeometry::new().with_mesh(id("ramp"), prism(&two, 1.5, 0.0, [0.0; 3])));
    for (end, depth) in [
        (WalkingEnd::RunBottom(0), 1.0),
        (WalkingEnd::RunTop(0), 2.0),
        (WalkingEnd::RunBottom(1), 2.0),
        (WalkingEnd::RunTop(1), 1.0),
    ] {
        let measured = ramps
            .measure_landing(&LandingRequest::new(id("ramp"), end, []))
            .unwrap();
        assert_eq!(measured.landing().unwrap().carrier(), &id("ramp"));
        assert!(
            holds(measured.depth().unwrap(), depth),
            "{end:?} {measured:?}"
        );
        assert!(holds(measured.width().unwrap(), 1.5));
        assert!(measured.evidence().exact);
    }
}

/// A flight whose underside slopes from its foot at x = 0 to 0.4 m under
/// its back at x = 1.12, 1.2 m wide, with its foot at `z`.
fn soffit_flight(z: f64) -> TriMesh {
    prism(
        &[
            [0.0, 0.0],
            [1.12, 0.4],
            [1.12, 0.72],
            [0.84, 0.72],
            [0.84, 0.54],
            [0.56, 0.54],
            [0.56, 0.36],
            [0.28, 0.36],
            [0.28, 0.18],
            [0.0, 0.18],
        ],
        1.2,
        0.0,
        [0.0, 0.0, z],
    )
}

#[test]
fn the_clearance_below_a_flight_is_measured_over_space_floors() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("raised"), soffit_flight(1.5))
        .with_mesh(id("grounded"), soffit_flight(0.0))
        .with_mesh(id("crossing"), soffit_flight(-0.1))
        .with_mesh(
            id("block"),
            prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]),
        )
        .with_mesh(id("hall"), cuboid([-1.0, -1.0, 0.0], [3.0, 3.0, 2.5]))
        .with_mesh(id("elsewhere"), cuboid([5.0, 5.0, 0.0], [6.0, 6.0, 2.5]))
        .with_tessellated_mesh(
            id("round"),
            cuboid([-1.0, -1.0, 0.0], [3.0, 3.0, 2.5]),
            0.01,
        )
        .with_unmeasured(id("broken"), "no representation")
        .with_no_body(id("zone"));
    let stairs = service(geometry);
    let below = |subject: &str, spaces: &[&str]| {
        stairs.measure_clearance_below(&ClearanceBelowRequest::new(
            id(subject),
            spaces.iter().map(|local| id(local)),
        ))
    };
    let raised = below("raised", &["hall", "elsewhere", "zone"]).unwrap();
    assert!(holds(raised.clearance().unwrap(), 1.5), "{raised:?}");
    assert_eq!(raised.governing(), &[id("hall")]);
    assert!(!raised.evidence().exact);
    // Its underside meets the floor at its foot: nothing is higher than zero.
    let grounded = below("grounded", &["hall"]).unwrap();
    assert!(holds(grounded.clearance().unwrap(), 0.0), "{grounded:?}");
    // A solid flight rests on the floor all along: nobody stands under it.
    let block = below("block", &["hall"]).unwrap();
    assert_eq!(block.clearance(), None, "{block:?}");
    assert!(matches!(
        below("crossing", &["hall"]),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("crosses")
    ));
    assert!(matches!(
        below("raised", &["round"]),
        Err(WalkingSurfaceError::InexactGeometry(_))
    ));
    assert!(matches!(
        below("raised", &["broken"]),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert_eq!(
        below("raised", &["missing"]),
        Err(WalkingSurfaceError::UnknownObject(id("missing")))
    );
}

/// A rail `thickness` deep under the plan polyline of its top, given as
/// `(along x, top)` points, swept `width` across from `y`.
fn rail(top: &[[f64; 2]], thickness: f64, y: f64, width: f64) -> TriMesh {
    let mut profile: Vec<[f64; 2]> = top.iter().map(|[x, z]| [*x, z - thickness]).collect();
    profile.extend(top.iter().rev());
    prism(&profile, width, 0.0, [0.0, y, 0.0])
}

/// The top of a rail `height` above the nosing line of the four-riser
/// flight (nosings at x = 0 .. 0.84, 0.18 .. 0.72 m), held level `bottom`
/// before the first and `top` beyond the last.
fn along_flight(height: f64, bottom: f64, top: f64) -> Vec<[f64; 2]> {
    vec![
        [-bottom, 0.18 + height],
        [0.0, 0.18 + height],
        [0.84, 0.72 + height],
        [0.84 + top, 0.72 + height],
    ]
}

fn handrails(
    stairs: &AxiolidWalkingSurfaceService,
    stretch: WalkingStretch,
    subject: &str,
    rails: &[&str],
) -> Result<HandrailEvidence, WalkingSurfaceError> {
    stairs.measure_handrails(
        &HandrailRequest::try_new(
            id(subject),
            stretch,
            rails.iter().map(|local| id(local)),
            (0.2, 1.5),
            0.3,
        )
        .unwrap(),
    )
}

fn rail_of<'a>(evidence: &'a HandrailEvidence, local: &str) -> &'a RailMeasurement {
    &evidence
        .rails()
        .iter()
        .find(|(rail, _)| *rail == id(local))
        .unwrap_or_else(|| panic!("{local} is not along: {evidence:?}"))
        .1
}

#[test]
fn handrails_along_a_flight_measure_height_extension_and_side() {
    // The flight is 1.2 m wide, y 0 .. 1.2: the climber's left at y 1.2.
    let stairs = flight_with(vec![
        ("left", rail(&along_flight(0.9, 0.3, 0.3), 0.05, 1.25, 0.05)),
        (
            "right",
            rail(&along_flight(0.75, 0.1, 0.3), 0.05, -0.1, 0.05),
        ),
        // Beyond the reach across, and a storey above.
        ("far", rail(&along_flight(0.9, 0.3, 0.3), 0.05, 2.0, 0.05)),
        (
            "above",
            rail(&along_flight(3.9, 0.3, 0.3), 0.05, 1.25, 0.05),
        ),
    ]);
    let measured = handrails(
        &stairs,
        WalkingStretch::Flight,
        "flight",
        &["left", "right", "far", "above"],
    )
    .unwrap();
    assert!(!measured.evidence().exact);
    let names: Vec<_> = measured
        .rails()
        .iter()
        .map(|(rail, _)| rail.clone())
        .collect();
    assert_eq!(names, [id("left"), id("right")]);

    let left = rail_of(&measured, "left");
    assert_eq!(measured.side(left), Some(RailSide::Left));
    assert!(
        holds(left.lowest(), 0.9) && holds(left.highest(), 0.9),
        "{left:?}"
    );
    assert!(left.highest().upper() - left.lowest().lower() < 1e-9);
    assert!(holds(measured.bottom_extension(left).unwrap(), 0.3));
    assert!(holds(measured.top_extension(left).unwrap(), 0.3));
    // Level extensions of an exact rail rise by next to nothing.
    for rise in [left.bottom_rise().unwrap(), left.top_rise().unwrap()] {
        assert!(rise.lower() == 0.0 && rise.upper() < 1e-12, "{rise:?}");
    }

    let right = rail_of(&measured, "right");
    assert_eq!(measured.side(right), Some(RailSide::Right));
    assert!(holds(right.lowest(), 0.75), "{right:?}");
    assert!(holds(measured.bottom_extension(right).unwrap(), 0.1));
    // It stops 0.1 m before the flight: short of the 0.3 m measured.
    assert_eq!(right.bottom_rise(), None);
}

#[test]
fn a_rail_in_pieces_is_measured_piece_by_piece_and_put_in_order() {
    // The left rail of `handrails_along_a_flight_…` cut at x 0.4 and 0.5.
    let at = |x: f64| 1.08 + x * 0.54 / 0.84;
    let lower = vec![[-0.3, 1.08], [0.0, 1.08], [0.4, at(0.4)]];
    let upper = vec![[0.5, at(0.5)], [0.84, 1.62], [1.14, 1.62]];
    let stairs = flight_with(vec![
        ("upper", rail(&upper, 0.05, 1.25, 0.05)),
        ("lower", rail(&lower, 0.05, 1.25, 0.05)),
    ]);
    let measured = handrails(
        &stairs,
        WalkingStretch::Flight,
        "flight",
        &["upper", "lower"],
    )
    .unwrap();
    for local in ["lower", "upper"] {
        let piece = rail_of(&measured, local);
        assert_eq!(measured.side(piece), Some(RailSide::Left));
        assert!(
            holds(piece.lowest(), 0.9) && holds(piece.highest(), 0.9),
            "{piece:?}"
        );
    }
    let pieces = measured.side_rail(RailSide::Left).unwrap();
    let names: Vec<&ObjectId> = pieces.iter().map(|(rail, _)| rail).collect();
    assert_eq!(names, [&id("lower"), &id("upper")]);
    // The first piece reaches beyond the bottom, level; the last beyond the
    // top.
    let (first, last) = (&pieces[0].1, &pieces[1].1);
    assert!(holds(measured.bottom_extension(first).unwrap(), 0.3));
    assert!(first.bottom_rise().unwrap().upper() < 1e-12);
    assert!(holds(measured.top_extension(last).unwrap(), 0.3));
    assert!(last.top_rise().unwrap().upper() < 1e-12);
    let gap = measured.gap(first, last).unwrap();
    assert!(
        holds(gap, 0.1) && gap.upper() - gap.lower() < 1e-9,
        "{gap:?}"
    );
}

#[test]
fn a_rail_sloping_on_past_the_flight_rises_over_its_extension() {
    // The rail keeps climbing 0.3 m past the last nosing, then stops.
    let top = vec![[0.0, 1.08], [1.14, 1.08 + 1.14 * 0.54 / 0.84]];
    let stairs = flight_with(vec![("rail", rail(&top, 0.05, 1.25, 0.05))]);
    let measured = handrails(&stairs, WalkingStretch::Flight, "flight", &["rail"]).unwrap();
    let rail = rail_of(&measured, "rail");
    assert!(
        holds(rail.lowest(), 0.9) && holds(rail.highest(), 0.9),
        "{rail:?}"
    );
    let rise = rail.top_rise().unwrap();
    assert!(holds(rise, 0.3 * 0.54 / 0.84), "{rise:?}");
    assert_eq!(rail.bottom_rise(), None);
    assert!(holds(measured.bottom_extension(rail).unwrap(), 0.0));
}

#[test]
fn rails_not_parallel_or_unmeasurable_are_refused() {
    // Turned in plan: its plan is no rectangle along x.
    let mut turned = rail(&along_flight(0.9, 0.3, 0.3), 0.05, 1.25, 0.05);
    for point in &mut turned.positions {
        point.y += 0.1 * point.x;
    }
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("flight"),
            prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]),
        )
        .with_mesh(id("turned"), turned)
        .with_tessellated_mesh(
            id("round"),
            rail(&along_flight(0.9, 0.3, 0.3), 0.05, 1.25, 0.05),
            0.002,
        )
        .with_unmeasured(id("broken"), "no representation")
        .with_no_body(id("zone"));
    let stairs = service(geometry);
    let measure = |rails: &[&str]| handrails(&stairs, WalkingStretch::Flight, "flight", rails);
    assert!(matches!(
        measure(&["turned"]),
        Err(WalkingSurfaceError::Unsupported(m)) if m.contains("rectangle")
    ));
    assert!(matches!(
        measure(&["broken"]),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert_eq!(
        measure(&["missing"]),
        Err(WalkingSurfaceError::UnknownObject(id("missing")))
    );
    assert!(measure(&["zone"]).unwrap().rails().is_empty());
    // A tessellated rail is measured, widened by its chord deviation.
    let round = measure(&["round"]).unwrap();
    let rail = rail_of(&round, "round");
    assert!(holds(rail.lowest(), 0.9) && rail.lowest().lower() <= 0.9 - 0.002);
    assert!(rail.top_rise().unwrap().upper() >= 0.004);
    // A flight without rails, and a ramp's run on a flight, are told apart.
    assert!(measure(&[]).unwrap().rails().is_empty());
    assert!(matches!(
        handrails(&stairs, WalkingStretch::Run(0), "flight", &[]),
        Err(WalkingSurfaceError::Unsupported(_))
    ));
}

#[test]
fn a_handrail_along_a_ramp_follows_its_surface() {
    // The run rises 0.5 m over x 1 .. 7 from 0.1 m, 1.5 m wide.
    let top = vec![[0.7, 1.0], [1.0, 1.0], [7.0, 1.5], [7.3, 1.5]];
    let ramps = service(
        AxiolidGeometry::new()
            .with_mesh(
                id("ramp"),
                prism(&ramp_profile(0.5, 6.0), 1.5, 0.0, [0.0; 3]),
            )
            .with_mesh(id("rail"), rail(&top, 0.05, -0.1, 0.05)),
    );
    let measured = handrails(&ramps, WalkingStretch::Run(0), "ramp", &["rail"]).unwrap();
    let rail = rail_of(&measured, "rail");
    assert_eq!(measured.side(rail), Some(RailSide::Right));
    assert!(
        holds(rail.lowest(), 0.9) && holds(rail.highest(), 0.9),
        "{rail:?}"
    );
    assert!(holds(measured.bottom_extension(rail).unwrap(), 0.3));
    assert!(holds(measured.top_extension(rail).unwrap(), 0.3));
    assert!(rail.bottom_rise().unwrap().upper() < 1e-12);
}

#[test]
fn a_handrail_along_a_turned_flight_measures_within_rounding() {
    let angle: f64 = 0.5;
    let (sin, cos) = angle.sin_cos();
    let top = along_flight(0.9, 0.3, 0.3);
    let mut profile: Vec<[f64; 2]> = top.iter().map(|[x, z]| [*x, z - 0.05]).collect();
    profile.extend(top.iter().rev());
    let stairs = service(
        AxiolidGeometry::new()
            .with_mesh(
                id("flight"),
                prism(&flight_profile(&[0.18; 4], 0.28), 1.2, angle, [0.0; 3]),
            )
            .with_mesh(
                id("rail"),
                prism(&profile, 0.05, angle, [-1.25 * sin, 1.25 * cos, 0.0]),
            ),
    );
    let measured = handrails(&stairs, WalkingStretch::Flight, "flight", &["rail"]).unwrap();
    let rail = rail_of(&measured, "rail");
    assert_eq!(measured.side(rail), Some(RailSide::Left));
    assert!(
        holds(rail.lowest(), 0.9) && holds(rail.highest(), 0.9),
        "{rail:?}"
    );
    assert!(rail.lowest().upper() - rail.lowest().lower() < 1e-9);
    assert!(holds(measured.top_extension(rail).unwrap(), 0.3));
    assert!(rail.top_rise().unwrap().upper() < 1e-9);
}

/// A closed, outward solid standing on `z = 0`: every cell, a convex
/// counter-clockwise polygon of `points` (corners shared by index, no
/// corner of one cell on another's edge), rises to its height. Where a
/// cell meets a lower one or the outside, a wall falls from its height to
/// the other's, split at every height meeting its ends so the mesh is a
/// closed two-manifold.
fn stepped(points: &[[f64; 2]], cells: &[(Vec<usize>, f64)]) -> TriMesh {
    let mut heights: Vec<Vec<f64>> = vec![vec![0.0]; points.len()];
    for (corners, height) in cells {
        for corner in corners {
            heights[*corner].push(*height);
        }
    }
    let mut positions = Vec::new();
    let mut first = Vec::with_capacity(points.len());
    for (point, levels) in points.iter().zip(&mut heights) {
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        first.push(u32::try_from(positions.len()).unwrap());
        positions.extend(levels.iter().map(|z| Point3::new(point[0], point[1], *z)));
    }
    let vertex = |point: usize, height: f64| -> u32 {
        let level = heights[point]
            .iter()
            .position(|z| z.total_cmp(&height).is_eq())
            .unwrap();
        first[point] + u32::try_from(level).unwrap()
    };
    let mut owner: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (corners, height) in cells {
        for k in 0..corners.len() {
            owner.insert((corners[k], corners[(k + 1) % corners.len()]), *height);
        }
    }
    let mut indices = Vec::new();
    for (corners, height) in cells {
        for k in 1..corners.len() - 1 {
            indices.extend([
                vertex(corners[0], *height),
                vertex(corners[k], *height),
                vertex(corners[k + 1], *height),
            ]);
            indices.extend([
                vertex(corners[0], 0.0),
                vertex(corners[k + 1], 0.0),
                vertex(corners[k], 0.0),
            ]);
        }
        for k in 0..corners.len() {
            let (a, b) = (corners[k], corners[(k + 1) % corners.len()]);
            let other = owner.get(&(b, a)).copied().unwrap_or(0.0);
            if other >= *height {
                continue;
            }
            let side = |point: usize| -> Vec<u32> {
                heights[point]
                    .iter()
                    .filter(|z| **z >= other && **z <= *height)
                    .map(|z| vertex(point, *z))
                    .collect()
            };
            // Outward is to the right of a -> b: wind (a, lo), (b, lo),
            // (b, hi), (a, hi), zipping up both sides.
            let (left, right) = (side(a), side(b));
            let level = |index: u32| positions[index as usize].z;
            let (mut i, mut j) = (0, 0);
            while i + 1 < left.len() || j + 1 < right.len() {
                if j + 1 < right.len()
                    && (i + 1 == left.len() || level(right[j + 1]) <= level(left[i + 1]))
                {
                    indices.extend([left[i], right[j], right[j + 1]]);
                    j += 1;
                } else {
                    indices.extend([left[i], right[j], left[i + 1]]);
                    i += 1;
                }
            }
        }
    }
    TriMesh::new(positions, indices)
}

/// A quarter-turn flight 1 m wide, 0.18 m risers: three straight treads
/// 0.28 m deep climbing along +x, three winders turning left through the
/// square x 0.84..1.84, y 0..1 about its inner corner (0.84, 1), their
/// nosings 30° apart, and three straight treads climbing along +y. The
/// last tread is the top.
fn quarter_turn() -> TriMesh {
    let slope = 1.0 / 3.0_f64.sqrt();
    let points = vec![
        [0.0, 0.0],          // 0
        [0.0, 1.0],          // 1
        [0.28, 0.0],         // 2
        [0.28, 1.0],         // 3
        [0.56, 0.0],         // 4
        [0.56, 1.0],         // 5
        [0.84, 0.0],         // 6
        [0.84, 1.0],         // 7: the inner corner
        [0.84 + slope, 0.0], // 8: the -60° nosing
        [1.84, 0.0],         // 9
        [1.84, 1.0 - slope], // 10: the -30° nosing
        [1.84, 1.0],         // 11
        [0.84, 1.28],        // 12
        [1.84, 1.28],        // 13
        [0.84, 1.56],        // 14
        [1.84, 1.56],        // 15
        [0.84, 1.84],        // 16
        [1.84, 1.84],        // 17
    ];
    let cells: Vec<Vec<usize>> = vec![
        vec![0, 2, 3, 1],
        vec![2, 4, 5, 3],
        vec![4, 6, 7, 5],
        vec![6, 8, 7],
        vec![8, 9, 10, 7],
        vec![10, 11, 7],
        vec![7, 11, 13, 12],
        vec![12, 13, 15, 14],
        vec![14, 15, 17, 16],
    ];
    let cells: Vec<(Vec<usize>, f64)> = cells
        .into_iter()
        .zip(1_u32..)
        .map(|(cell, step)| (cell, 0.18 * f64::from(step)))
        .collect();
    stepped(&points, &cells)
}

fn is_closed(mesh: &TriMesh) -> bool {
    audit_mesh(mesh, Tolerance::new(1e-9, 1e-9).unwrap()).is_closed_two_manifold()
}

#[test]
fn a_quarter_turn_flight_is_walked_along_its_walking_line() {
    let mesh = quarter_turn();
    assert!(is_closed(&mesh));
    let stairs = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh));
    let centre = walk(&stairs, "flight").unwrap();
    assert!(!centre.is_exact());
    let WalkingLine::Turning(vertices) = centre.walking_line() else {
        panic!("the flight turns: {centre:?}");
    };
    assert_eq!(vertices.len(), 9);
    // The centre line runs midway across the straight treads.
    assert!((vertices[0][1] - 0.5).abs() < 1e-9 && (vertices[8][0] - 1.34).abs() < 1e-9);
    let risers = centre.risers();
    assert_eq!(risers.len(), 9);
    assert!(risers.iter().all(|riser| holds(*riser, 0.18)), "{risers:?}");
    assert!(!centre.ends_in_riser());
    let goings = centre.goings();
    assert_eq!(goings.len(), 8);
    // Nosing to nosing along the line: 0.28 m where it runs square to the
    // nosings, more where it crosses one obliquely on its way round.
    for going in [goings[0], goings[1], goings[7]] {
        assert!(holds(going, 0.28), "{goings:?}");
        assert!(going.upper() - going.lower() < 1e-8);
    }
    for going in &goings[2..7] {
        assert!(going.lower() > 0.28 && going.upper() < 0.42, "{goings:?}");
    }
    // The nosings turn 30° over each winder and not at all elsewhere.
    let angles: Vec<MeasuredInterval> = centre
        .winder_angles()
        .into_iter()
        .map(Option::unwrap)
        .collect();
    for (angle, degrees) in angles
        .iter()
        .zip([0.0, 0.0, 0.0, 30.0, 30.0, 30.0, 0.0, 0.0])
    {
        let expected: f64 = f64::to_radians(degrees);
        assert!(holds(*angle, expected), "{angle:?} {degrees}");
        assert!(angle.upper() - angle.lower() < 1e-12);
    }
    // Every straight tread is 1 m wide along its nosing; a winder tapers
    // and has no width of its own.
    let widths: Vec<Option<MeasuredInterval>> = centre.treads().iter().map(Tread::width).collect();
    for (index, width) in widths.iter().enumerate() {
        if (3..6).contains(&index) {
            assert_eq!(*width, None, "{widths:?}");
        } else {
            assert!(holds(width.unwrap(), 1.0), "{widths:?}");
        }
    }
    assert!(
        centre
            .treads()
            .iter()
            .all(|tread| tread.riser_below() == RiserClosure::Closed)
    );

    // Nearer the inner side, the winders' goings shrink and the straight
    // ones stay.
    let inner = stairs
        .measure_tread_flight(&TreadFlightRequest::from_inner_side(id("flight"), 0.3).unwrap())
        .unwrap();
    let near = inner.goings();
    for index in [0, 7] {
        assert!(holds(near[index], 0.28), "{near:?}");
    }
    for index in 3..5 {
        assert!(
            near[index].upper() < goings[index].lower(),
            "{near:?} {goings:?}"
        );
    }
    // A line wider than the treads leaves them.
    let outside = stairs
        .measure_tread_flight(&TreadFlightRequest::from_inner_side(id("flight"), 1.5).unwrap());
    assert!(
        matches!(&outside, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("outside")),
        "{outside:?}"
    );
}

/// A closed, outward union of grid cells, `filled` saying which cell
/// between consecutive `xs`, `ys` and `zs` is solid. Cells may meet only
/// face to face, never along an edge alone.
fn voxels(
    xs: &[f64],
    ys: &[f64],
    zs: &[f64],
    filled: impl Fn(usize, usize, usize) -> bool,
) -> TriMesh {
    let (nx, ny, nz) = (xs.len() - 1, ys.len() - 1, zs.len() - 1);
    let solid = |i: isize, j: isize, k: isize| {
        let (Ok(i), Ok(j), Ok(k)) = (usize::try_from(i), usize::try_from(j), usize::try_from(k))
        else {
            return false;
        };
        i < nx && j < ny && k < nz && filled(i, j, k)
    };
    let mut positions = Vec::new();
    let mut known: BTreeMap<[usize; 3], u32> = BTreeMap::new();
    let mut vertex = |corner: [usize; 3]| -> u32 {
        *known.entry(corner).or_insert_with(|| {
            positions.push(Point3::new(xs[corner[0]], ys[corner[1]], zs[corner[2]]));
            u32::try_from(positions.len() - 1).unwrap()
        })
    };
    let mut indices = Vec::new();
    for i in 0..nx {
        for j in 0..ny {
            for k in 0..nz {
                if !filled(i, j, k) {
                    continue;
                }
                let cell = [i, j, k];
                for axis in 0..3 {
                    for positive in [false, true] {
                        let mut next = cell.map(|c| isize::try_from(c).unwrap());
                        next[axis] += if positive { 1 } else { -1 };
                        if solid(next[0], next[1], next[2]) {
                            continue;
                        }
                        let (first, second) = ((axis + 1) % 3, (axis + 2) % 3);
                        let corner = |along_first: usize, along_second: usize| {
                            let mut corner = cell;
                            corner[axis] += usize::from(positive);
                            corner[first] += along_first;
                            corner[second] += along_second;
                            corner
                        };
                        // (u, v, axis) is right-handed: counter-clockwise
                        // in (u, v) faces +axis.
                        let mut quad = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)];
                        if !positive {
                            quad.reverse();
                        }
                        let [a, b, c, d] = quad.map(&mut vertex);
                        indices.extend([a, b, c, a, c, d]);
                    }
                }
            }
        }
    }
    TriMesh::new(positions, indices)
}

/// Four open-riser treads, plates 0.04 m thick at 0.18 m rises and 0.28 m
/// goings, 1 m wide, joined by a 0.1 m spine under their middle that
/// stands on the floor.
fn open_risers() -> TriMesh {
    let (rise, thickness, spine) = (0.18, 0.04, 0.23);
    let tread = |k: usize| rise * f64::from(u32::try_from(k + 1).unwrap());
    let mut zs = vec![0.0];
    for k in 0..4 {
        zs.extend([
            tread(k),
            tread(k) - thickness,
            (tread(k) - thickness - spine).max(0.0),
        ]);
    }
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let xs = [0.0, 0.28, 0.56, 0.84, 1.12];
    let ys = [0.0, 0.45, 0.55, 1.0];
    let cells = zs.clone();
    voxels(&xs, &ys, &zs, move |i, j, k| {
        let (bottom, top) = (cells[k], cells[k + 1]);
        let within = |low: f64, high: f64| bottom >= low - 1e-12 && top <= high + 1e-12;
        let plate = within(tread(i) - thickness, tread(i));
        let spine = j == 1
            && within(
                (tread(i) - thickness - spine).max(0.0),
                tread(i) - thickness,
            );
        plate || spine
    })
}

#[test]
fn open_risers_are_found_between_treads() {
    let mesh = open_risers();
    assert!(is_closed(&mesh));
    let stairs = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh));
    let measured = walk(&stairs, "flight").unwrap();
    assert!(measured.is_exact(), "{measured:?}");
    assert_eq!(measured.treads().len(), 4);
    assert!(measured.risers().iter().all(|riser| holds(*riser, 0.18)));
    assert!(measured.goings().iter().all(|going| holds(*going, 0.28)));
    let closures: Vec<RiserClosure> = measured.treads().iter().map(Tread::riser_below).collect();
    // The strip under the first nosing is open but for the spine; whether
    // a riser stands set back behind it is not measured.
    assert_eq!(
        closures,
        [
            RiserClosure::NotMeasured,
            RiserClosure::Open,
            RiserClosure::Open,
            RiserClosure::Open
        ]
    );
}

/// A small deterministic displacement in `[-1, 1]`.
fn wobble(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    #[allow(clippy::cast_precision_loss)]
    let unit = (*state >> 11) as f64 / (1_u64 << 53) as f64;
    2.0 * unit - 1.0
}

#[test]
fn a_tessellated_flight_is_measured_within_its_chord_deviation() {
    let deviation = 0.001;
    let mut mesh = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    let mut state = 85;
    for point in &mut mesh.positions {
        point.x += 0.4 * deviation * wobble(&mut state);
        point.y += 0.4 * deviation * wobble(&mut state);
        point.z += 0.4 * deviation * wobble(&mut state);
    }
    let stairs =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("flight"), mesh, deviation));
    let measured = walk(&stairs, "flight").unwrap();
    assert!(!measured.is_exact());
    assert!(!measured.walking_line().is_turning());
    let risers = measured.risers();
    assert_eq!(risers.len(), 4);
    for riser in &risers {
        assert!(holds(*riser, 0.18), "{risers:?}");
        assert!(
            riser.upper() - riser.lower() <= 6.0 * deviation,
            "{risers:?}"
        );
    }
    for going in measured.goings() {
        assert!(holds(going, 0.28), "{going:?}");
    }
    for tread in measured.treads() {
        assert!(holds(tread.width().unwrap(), 1.2));
        assert_eq!(tread.riser_below(), RiserClosure::Closed);
    }
    for angle in measured.winder_angles() {
        assert!(holds(angle.unwrap(), 0.0));
    }

    // The same flight tessellated but not displaced is measured as an
    // interval all the same.
    let mesh = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    let stairs =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("flight"), mesh, deviation));
    let measured = walk(&stairs, "flight").unwrap();
    assert!(!measured.is_exact());
    let first = measured.risers()[0];
    assert!(holds(first, 0.18) && first.upper() - first.lower() >= 4.0 * deviation - 1e-12);
}

#[test]
fn a_flight_turning_both_ways_has_no_inner_side() {
    // Square treads 1 m wide climbing along +y, turning right to +x,
    // then left to +y again.
    let points = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [1.0, 1.0],
        [0.0, 2.0],
        [1.0, 2.0],
        [2.0, 1.0],
        [2.0, 2.0],
        [1.0, 3.0],
        [2.0, 3.0],
        [1.0, 4.0],
        [2.0, 4.0],
    ];
    let cells = vec![
        (vec![0, 1, 3, 2], 0.18),
        (vec![2, 3, 5, 4], 0.36),
        (vec![3, 6, 7, 5], 0.54),
        (vec![5, 7, 9, 8], 0.72),
        (vec![8, 9, 11, 10], 0.90),
    ];
    let mesh = stepped(&points, &cells);
    let stairs = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh));
    let inner = stairs
        .measure_tread_flight(&TreadFlightRequest::from_inner_side(id("flight"), 0.3).unwrap());
    assert!(
        matches!(&inner, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("both ways")),
        "{inner:?}"
    );
}

/// The quarter turn raised 2 m above a hall's floor, a floor slab before
/// its foot (x -1.5 .. 0, y -0.2 .. 1.2, top at 2 m) and the upper floor
/// beyond its top tread (x 0.84 .. 1.84, y 1.84 .. 3, top at 3.62 m).
fn raised_quarter_turn(objects: Vec<(&str, TriMesh)>) -> AxiolidWalkingSurfaceService {
    let mut mesh = quarter_turn();
    for point in &mut mesh.positions {
        point.z += 2.0;
    }
    let mut geometry = AxiolidGeometry::new()
        .with_mesh(id("flight"), mesh)
        .with_mesh(id("hall"), cuboid([-1.0, -1.0, 0.0], [3.0, 3.0, 2.5]))
        .with_mesh(id("floor"), cuboid([-1.5, -0.2, 1.8], [0.0, 1.2, 2.0]))
        .with_mesh(id("upper"), cuboid([0.84, 1.84, 3.42], [1.84, 3.0, 3.62]));
    for (local, mesh) in objects {
        geometry = geometry.with_mesh(id(local), mesh);
    }
    service(geometry)
}

/// `mesh` turned a quarter anticlockwise in plan about the origin and moved
/// by `offset`: exactly, coordinates swapped.
fn turned(mut mesh: TriMesh, offset: [f64; 2]) -> TriMesh {
    for point in &mut mesh.positions {
        let (x, y) = (point.x, point.y);
        point.x = offset[0] - y;
        point.y = offset[1] + x;
    }
    mesh
}

#[test]
fn a_quarter_turn_flight_measures_headroom_below_landings_and_width() {
    let stairs = raised_quarter_turn(vec![]);
    let flight = walk(&stairs, "flight").unwrap();
    assert!(flight.walking_line().is_turning());
    // Its straight treads fill rectangles, its winders taper: no width.
    assert_eq!(flight.width(), None);
    assert!(flight.treads()[0].width().is_some());
    assert!(flight.treads()[8].width().is_some());
    // Headroom below needs no walking direction.
    let below = stairs
        .measure_clearance_below(&ClearanceBelowRequest::new(id("flight"), [id("hall")]))
        .unwrap();
    assert!(holds(below.clearance().unwrap(), 2.0), "{below:?}");
    // Each end is placed along its own tread's direction, square to its
    // nosing: back along -x at the foot, on along +y at the top, where the
    // top tread counts towards the landing.
    let bottom = landing(
        &stairs,
        WalkingEnd::FlightBottom,
        &["floor", "upper", "hall"],
    )
    .unwrap();
    assert!(bottom.direction() == MetricDirection::try_new([-1.0, 0.0, 0.0]).unwrap());
    assert_eq!(bottom.landing().unwrap().carrier(), &id("floor"));
    assert!(holds(bottom.depth().unwrap(), 1.5), "{bottom:?}");
    assert!(holds(bottom.width().unwrap(), 1.4), "{bottom:?}");
    let top = landing(&stairs, WalkingEnd::FlightTop, &["floor", "upper", "hall"]).unwrap();
    assert!(top.direction() == MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap());
    assert_eq!(top.landing().unwrap().carrier(), &id("upper"));
    assert!(holds(top.depth().unwrap(), 3.0 - 1.56), "{top:?}");
    assert!(holds(top.width().unwrap(), 1.0), "{top:?}");
    // Along axes, the positions are exact.
    assert!(top.evidence().exact);
}

#[test]
fn handrails_along_a_quarter_turn_are_measured_part_by_part() {
    let slope = 1.0 / 3.0_f64.sqrt();
    // The outer rail in two pieces meeting at the corner (1.84, 0), its top
    // 0.9 m above the nosings' outer ends: along y = 0 over the lower
    // straight treads and the first winder (x 0 .. 0.84 + slope), along
    // x = 1.84 from the second winder (y 1 - slope) on, level 0.3 m beyond
    // either end and rising around the corner.
    let lower = vec![
        [-0.3, 3.08],
        [0.0, 3.08],
        [0.84, 3.62],
        [0.84 + slope, 3.80],
        [1.89, 3.98],
    ];
    let upper = vec![
        [-0.1, 3.98],
        [1.0 - slope, 3.98],
        [1.0, 4.16],
        [1.28, 4.34],
        [1.56, 4.52],
        [1.86, 4.52],
    ];
    let stairs = raised_quarter_turn(vec![
        ("lower", rail(&lower, 0.05, -0.1, 0.05)),
        (
            "upper_rail",
            turned(rail(&upper, 0.05, 0.0, 0.05), [1.89, 0.0]),
        ),
    ]);
    let measured = handrails(
        &stairs,
        WalkingStretch::Flight,
        "flight",
        &["lower", "upper_rail"],
    )
    .unwrap();
    assert_eq!(measured.parts().len(), 2, "{measured:?}");
    assert!(measured.parts()[1].direction() == MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap());
    let (first, last) = (
        rail_of(&measured, "lower"),
        rail_of(&measured, "upper_rail"),
    );
    assert_eq!((first.part(), last.part()), (0, 1));
    assert_eq!(measured.side(first), Some(RailSide::Right));
    assert_eq!(measured.side(last), Some(RailSide::Right));
    let pieces = measured.side_rail(RailSide::Right).unwrap();
    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0].0, id("lower"));
    // They meet at the corner.
    let gap = measured.gap(first, last).unwrap();
    assert!(gap.lower() == 0.0 && gap.upper() < 1e-9, "{gap:?}");
    // Where the nosings end on its side the rail runs 0.9 m above them;
    // around the corner, between two nosings on different walls, the pitch
    // line may lie anywhere between their elevations, one riser apart.
    assert!(
        holds(first.lowest(), 0.72) && holds(first.lowest(), 0.9),
        "{first:?}"
    );
    assert!(first.lowest().upper() < 0.9 + 1e-9, "{first:?}");
    assert!(
        holds(first.highest(), 0.9) && first.highest().upper() > 1.0,
        "{first:?}"
    );
    assert!(
        holds(last.lowest(), 0.9) && last.lowest().upper() - last.lowest().lower() < 1e-9,
        "{last:?}"
    );
    assert!(
        holds(last.highest(), 1.08) && last.highest().upper() < 1.08 + 1e-9,
        "{last:?}"
    );
    // The first piece reaches level beyond the foot, the last beyond the
    // top; neither is measured beyond the other end.
    assert!(holds(measured.bottom_extension(first).unwrap(), 0.3));
    assert!(first.bottom_rise().unwrap().upper() < 1e-12);
    assert_eq!(measured.top_extension(first), None);
    assert!(holds(measured.top_extension(last).unwrap(), 0.3));
    assert!(last.top_rise().unwrap().upper() < 1e-12);
    assert_eq!(measured.bottom_extension(last), None);
}

#[test]
fn rails_a_quarter_turn_cannot_place_are_refused() {
    let top = vec![[0.0, 3.08], [0.84, 3.62]];
    // Down the middle of the lower straight treads, whose pitch line
    // differs from side to side.
    let middle = raised_quarter_turn(vec![("rail", rail(&top, 0.05, 0.45, 0.1))]);
    let refused = handrails(&middle, WalkingStretch::Flight, "flight", &["rail"]);
    assert!(
        matches!(&refused, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("middle")),
        "{refused:?}"
    );
    // At an angle to both straight parts.
    let mut slanted = rail(&top, 0.05, -0.1, 0.05);
    for point in &mut slanted.positions {
        point.y -= 0.1 * point.x;
    }
    let slanted = raised_quarter_turn(vec![("rail", slanted)]);
    let refused = handrails(&slanted, WalkingStretch::Flight, "flight", &["rail"]);
    assert!(
        matches!(&refused, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("straight")),
        "{refused:?}"
    );
    // Inside the turn, beside the flight but along neither straight part:
    // it might be a piece of a handrail, so it is not dropped.
    let short = [[0.0, 3.08], [0.6, 3.46]];
    let inside = raised_quarter_turn(vec![("rail", rail(&short, 0.05, 1.5, 0.05))]);
    let refused = handrails(&inside, WalkingStretch::Flight, "flight", &["rail"]);
    assert!(
        matches!(&refused, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("along none")),
        "{refused:?}"
    );
    // Far from the flight, a rail is not along it.
    let far = raised_quarter_turn(vec![("rail", rail(&top, 0.05, -2.0, 0.05))]);
    let measured = handrails(&far, WalkingStretch::Flight, "flight", &["rail"]).unwrap();
    assert!(measured.rails().is_empty());

    // A tessellated quarter turn's straight parts climb along directions
    // its chords turn: a rail straight along the wall runs along none of
    // them exactly, and is refused rather than measured in a frame it
    // does not fill.
    let deviation = 0.001;
    let mut mesh = quarter_turn();
    let mut state = 131;
    for point in &mut mesh.positions {
        point.x += 0.4 * deviation * wobble(&mut state);
        point.y += 0.4 * deviation * wobble(&mut state);
    }
    let stairs = service(
        AxiolidGeometry::new()
            .with_tessellated_mesh(id("flight"), mesh, deviation)
            .with_mesh(
                id("rail"),
                rail(&[[0.0, 1.08], [0.84, 1.62]], 0.05, -0.1, 0.05),
            ),
    );
    assert!(walk(&stairs, "flight").unwrap().walking_line().is_turning());
    let refused = handrails(&stairs, WalkingStretch::Flight, "flight", &["rail"]);
    assert!(
        matches!(&refused, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("straight")),
        "{refused:?}"
    );
}

fn clear_width(
    stairs: &AxiolidWalkingSurfaceService,
    obstacles: &[&str],
) -> Result<ClearWidthEvidence, WalkingSurfaceError> {
    stairs.measure_clear_width(
        &ClearWidthRequest::try_new(
            id("flight"),
            WalkingStretch::Flight,
            obstacles.iter().map(|local| id(local)),
            (0.5, 1.5),
        )
        .unwrap(),
    )
}

#[test]
fn the_clear_width_is_the_narrowest_free_width_between_rails_and_walls() {
    // The flight is 1.2 m wide, y 0 .. 1.2. Rails 0.1 m wide stand inside
    // it along both sides 0.85 to 0.9 m above the nosing line; a wall runs
    // along its right side; a rail lower than the band stands in its
    // middle; a post stands over the middle.
    let stairs = flight_with(vec![
        ("right", rail(&along_flight(0.9, 0.3, 0.3), 0.05, 0.0, 0.1)),
        ("left", rail(&along_flight(0.9, 0.3, 0.3), 0.05, 1.1, 0.1)),
        ("wall", cuboid([-1.0, -0.2, 0.0], [2.0, 0.0, 3.0])),
        ("low", rail(&along_flight(0.2, 0.3, 0.3), 0.05, 0.5, 0.1)),
        ("post", cuboid([0.3, 0.55, 0.0], [0.5, 0.65, 3.0])),
    ]);
    let both = clear_width(&stairs, &["right", "left", "wall", "low"]).unwrap();
    assert!(!both.evidence().exact);
    assert!(holds(both.width(), 1.0), "{both:?}");
    assert!(
        both.width().upper() - both.width().lower() < 1e-6,
        "{both:?}"
    );
    assert_eq!(both.governing(), [id("left"), id("right")]);
    let one = clear_width(&stairs, &["right", "wall"]).unwrap();
    assert!(holds(one.width(), 1.1), "{one:?}");
    assert_eq!(one.governing(), [id("right")]);
    // Nothing reaching in: the walking surface's own width.
    let none = clear_width(&stairs, &["wall", "low"]).unwrap();
    assert!(holds(none.width(), 1.2), "{none:?}");
    assert!(none.governing().is_empty());
    assert!(matches!(
        clear_width(&stairs, &["post"]),
        Err(WalkingSurfaceError::Unsupported(_))
    ));
}

#[test]
fn the_clear_width_along_a_ramp_follows_its_surface() {
    // A run rising 0.5 m over 5 m between 1 m landings, 1.5 m wide from y 0;
    // a rail inside its left side 0.9 m above the surface.
    let ramp = prism(&ramp_profile(0.5, 5.0), 1.5, 0.0, [0.0; 3]);
    let top = [[1.0, 0.1 + 0.9], [6.0, 0.6 + 0.9]];
    let stairs = service(
        AxiolidGeometry::new()
            .with_mesh(id("ramp"), ramp)
            .with_mesh(id("rail"), rail(&top, 0.05, 1.35, 0.15)),
    );
    let measured = stairs
        .measure_clear_width(
            &ClearWidthRequest::try_new(
                id("ramp"),
                WalkingStretch::Run(0),
                [id("rail")],
                (0.5, 1.5),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(holds(measured.width(), 1.35), "{measured:?}");
    assert_eq!(measured.governing(), [id("rail")]);
}

fn landing_clear_width(
    stairs: &AxiolidWalkingSurfaceService,
    candidates: &[&str],
    obstacles: &[&str],
) -> Result<axioval_engine::LandingClearWidthEvidence, WalkingSurfaceError> {
    stairs.measure_landing_clear_width(
        &LandingClearWidthRequest::try_new(
            LandingRequest::new(
                id("flight"),
                WalkingEnd::FlightTop,
                candidates.iter().map(|local| id(local)),
            ),
            obstacles.iter().map(|local| id(local)),
            (0.5, 1.5),
        )
        .unwrap(),
    )
}

#[test]
fn a_landings_clear_width_lies_between_the_walls_bounding_it() {
    // A 1.2 m flight (y 0 .. 1.2) arrives at a 1.5 m slab (y -0.1 .. 1.4)
    // with walls standing on it 1 m apart (y 0.1 and 1.1), and a wall too
    // low for the band.
    let stairs = flight_with(vec![
        ("landing", cuboid([1.12, -0.1, 0.52], [2.12, 1.4, 0.72])),
        ("right", cuboid([1.12, -0.1, 0.72], [2.12, 0.1, 3.0])),
        ("left", cuboid([1.12, 1.1, 0.72], [2.12, 1.4, 3.0])),
        ("kerb", cuboid([1.12, 1.0, 0.72], [2.12, 1.1, 0.82])),
    ]);
    let measured = landing_clear_width(&stairs, &["landing"], &["right", "left", "kerb"]).unwrap();
    assert!(!measured.evidence().exact);
    let landing = measured.landing().unwrap();
    assert_eq!(landing.carrier(), &id("landing"));
    assert!(holds(landing.width(), 1.0), "{landing:?}");
    assert!(landing.width().upper() - landing.width().lower() < 1e-6);
    assert_eq!(landing.governing(), [id("left"), id("right")]);
    assert_eq!(landing.bounds(), (&[id("right")][..], &[id("left")][..]));
    // One wall bounds one side only; the slab's own edge bounds the other.
    let one = landing_clear_width(&stairs, &["landing"], &["right"]).unwrap();
    let one = one.landing().unwrap();
    assert!(holds(one.width(), 1.3), "{one:?}");
    assert_eq!(one.bounds(), (&[id("right")][..], &[][..]));
    // Without a candidate there is no landing.
    let none = landing_clear_width(&stairs, &[], &["right", "left"]).unwrap();
    assert!(none.landing().is_none());
}
