//! Stair flights, ramps and headroom over generated meshes.

use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, compose};
use axioval_axiolid::{AxiolidGeometry, AxiolidWalkingSurfaceService};
use axioval_engine::{
    ClearanceBelowRequest, HandrailEvidence, HandrailRequest, HeadroomRequest, LandingEvidence,
    LandingRequest, MeasuredInterval, MetricDirection, RailMeasurement, RailSide, WalkingEnd,
    WalkingStretch, WalkingSurfaceError, WalkingSurfaceService,
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

fn flight(risers: &[f64], angle: f64) -> AxiolidWalkingSurfaceService {
    service(AxiolidGeometry::new().with_mesh(
        id("flight"),
        prism(&flight_profile(risers, 0.28), 1.2, angle, [2.0, 1.0, 0.0]),
    ))
}

#[test]
fn a_straight_flight_measures_its_risers_and_goings_exactly() {
    let measured = flight(&[0.17, 0.17, 0.21, 0.17], 0.0)
        .measure_tread_flight(&id("flight"))
        .unwrap();
    assert!(measured.is_exact(), "{measured:?}");
    assert!(measured.direction() == MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap());
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
    assert_eq!(
        measured.evidence().locator,
        format!("tread-flight:{}", id("flight"))
    );
}

#[test]
fn a_turned_flight_measures_within_the_rounding_of_its_direction() {
    let measured = flight(&[0.18; 5], 0.5)
        .measure_tread_flight(&id("flight"))
        .unwrap();
    assert!(!measured.is_exact());
    let [x, y, z] = measured.direction().components();
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
    let measured = service(
        AxiolidGeometry::new().with_mesh(id("flight"), prism(&profile, 1.0, 0.0, [0.0; 3])),
    )
    .measure_tread_flight(&id("flight"))
    .unwrap();
    assert!(measured.ends_in_riser());
    assert_eq!(measured.treads().len(), 3);
    let risers = measured.risers();
    assert_eq!(risers.len(), 4);
    assert!(risers.iter().all(|riser| holds(*riser, 0.18)), "{risers:?}");
    assert!(holds(measured.rise(), 0.72));
}

#[test]
fn turning_flights_open_meshes_and_pieces_are_refused() {
    // Move the top tread sideways: the treads no longer follow a line.
    let mut mesh = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    #[allow(clippy::float_cmp)]
    for point in &mut mesh.positions {
        if point.z == 0.72 {
            point.y += 0.5;
        }
    }
    let turning = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh))
        .measure_tread_flight(&id("flight"));
    assert!(
        matches!(&turning, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("straight")),
        "{turning:?}"
    );

    let mut open = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    open.indices.truncate(open.indices.len() - 3);
    let open = service(AxiolidGeometry::new().with_mesh(id("flight"), open))
        .measure_tread_flight(&id("flight"));
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
    let pieces = service(AxiolidGeometry::new().with_mesh(id("flight"), pieces))
        .measure_tread_flight(&id("flight"));
    assert!(
        matches!(&pieces, Err(WalkingSurfaceError::Unsupported(m)) if m.contains("pieces")),
        "{pieces:?}"
    );
}

#[test]
fn tessellated_bodiless_unmeasured_and_unknown_flights_are_refused() {
    let mesh = prism(&flight_profile(&[0.18; 4], 0.28), 1.2, 0.0, [0.0; 3]);
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("curved"), mesh, 0.001)
        .with_no_body(id("zone"))
        .with_unmeasured(id("broken"), "no representation");
    let stairs = service(geometry);
    assert!(matches!(
        stairs.measure_tread_flight(&id("curved")),
        Err(WalkingSurfaceError::InexactGeometry(_))
    ));
    assert!(matches!(
        stairs.measure_tread_flight(&id("zone")),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert!(matches!(
        stairs.measure_sloped_runs(&id("broken")),
        Err(WalkingSurfaceError::Unavailable(_))
    ));
    assert_eq!(
        stairs.measure_tread_flight(&id("missing")),
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
    let measured = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh))
        .measure_tread_flight(&id("flight"))
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
    let sloped = service(AxiolidGeometry::new().with_mesh(id("flight"), mesh))
        .measure_tread_flight(&id("flight"));
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
    let flight = stairs.measure_tread_flight(&id("flight")).unwrap();
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
        .measure_tread_flight(&id("flight"))
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
        .measure_tread_flight(&id("flight"))
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
        .measure_tread_flight(&id("flight"))
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
    assert!(holds(measured.bottom_extension(left), 0.3));
    assert!(holds(measured.top_extension(left), 0.3));
    // Level extensions of an exact rail rise by next to nothing.
    for rise in [left.bottom_rise().unwrap(), left.top_rise().unwrap()] {
        assert!(rise.lower() == 0.0 && rise.upper() < 1e-12, "{rise:?}");
    }

    let right = rail_of(&measured, "right");
    assert_eq!(measured.side(right), Some(RailSide::Right));
    assert!(holds(right.lowest(), 0.75), "{right:?}");
    assert!(holds(measured.bottom_extension(right), 0.1));
    // It stops 0.1 m before the flight: short of the 0.3 m measured.
    assert_eq!(right.bottom_rise(), None);
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
    assert!(holds(measured.bottom_extension(rail), 0.0));
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
    assert!(holds(measured.bottom_extension(rail), 0.3));
    assert!(holds(measured.top_extension(rail), 0.3));
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
    assert!(holds(measured.top_extension(rail), 0.3));
    assert!(rail.top_rise().unwrap().upper() < 1e-9);
}
