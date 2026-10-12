//! Lines of sight between real meshes: hidden by a wall, visible past a
//! column, undecided across a seam.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidSightService};
use axioval_engine::{
    SightError, SightOutcome, SightRequest, SightService, SkyObstruction, SkyObstructionRequest,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
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

/// The eye at (0, 0, 1.5) looks along +x at target `t`, a 0.5 m wide box
/// at x 5 to 5.5. Wall `wall` stands across the view at x 2, column `col`
/// in front of the target's middle at x 2.5, wall halves `left` and
/// `right` meet at y 0 in x 3, and `far` stands beyond the target.
fn scene() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("t"), cuboid([5.0, -0.25, 0.0], [5.5, 0.25, 2.0]))
        .with_mesh(id("wall"), cuboid([2.0, -3.0, 0.0], [2.2, 3.0, 3.0]))
        .with_mesh(id("col"), cuboid([2.5, -0.1, 0.0], [2.7, 0.1, 3.0]))
        .with_mesh(id("left"), cuboid([3.0, -3.0, 0.0], [3.2, 0.0, 3.0]))
        .with_mesh(id("right"), cuboid([3.0, 0.0, 0.0], [3.2, 3.0, 3.0]))
        .with_mesh(id("far"), cuboid([8.0, -3.0, 0.0], [8.2, 3.0, 3.0]))
        .with_no_body(id("zone"))
}

fn look(blockers: &[&str], within: Option<f64>) -> Result<Option<SightOutcome>, SightError> {
    look_in(scene(), blockers, within)
}

fn look_in(
    geometry: AxiolidGeometry,
    blockers: &[&str],
    within: Option<f64>,
) -> Result<Option<SightOutcome>, SightError> {
    let request = SightRequest::try_new(
        [0.0, 0.0, 1.5],
        id("t"),
        blockers.iter().map(|local| id(local)).collect(),
        within,
    )
    .expect("valid request");
    let answer = AxiolidSightService::new(geometry).assess_sight(&request)?;
    let (lower, upper) = answer.distance_metres();
    assert!(
        lower <= 5.0 && 5.0 <= upper && upper - lower < 1e-6,
        "{lower} {upper}"
    );
    Ok(answer.outcome().cloned())
}

#[test]
fn a_target_behind_a_wall_is_hidden_by_it() {
    assert_eq!(
        look(&["wall", "far", "zone"], None).unwrap(),
        Some(SightOutcome::Hidden {
            occluders: vec![id("wall")]
        })
    );
}

#[test]
fn a_target_is_visible_past_a_column_with_a_witness_on_it() {
    let Some(SightOutcome::Visible { through }) = look(&["col", "far"], None).unwrap() else {
        panic!("visible past the column");
    };
    assert!(
        (5.0..=5.5).contains(&through[0]) && (-0.25..=0.25).contains(&through[1]),
        "{through:?}"
    );
    assert!(matches!(
        look(&[], None).unwrap(),
        Some(SightOutcome::Visible { .. })
    ));
}

#[test]
fn a_cover_made_by_two_blockers_across_a_seam_stays_undecided() {
    assert_eq!(
        look(&["left", "right"], None).unwrap(),
        Some(SightOutcome::Undecided)
    );
}

#[test]
fn a_target_surely_beyond_the_range_is_not_looked_at() {
    assert_eq!(look(&["wall"], Some(4.0)).unwrap(), None);
    assert!(look(&["wall"], Some(5.0)).unwrap().is_some());
}

#[test]
fn inexact_or_unmeasured_blockers_and_bodiless_targets_refuse() {
    let tessellated =
        scene().with_tessellated_mesh(id("wall"), cuboid([2.0, -3.0, 0.0], [2.2, 3.0, 3.0]), 1e-3);
    assert!(matches!(
        look_in(tessellated, &["wall"], None),
        Err(SightError::Unavailable(_))
    ));
    let unmeasured = scene().with_unmeasured(id("pillar"), "no body representation");
    assert!(matches!(
        look_in(unmeasured, &["pillar"], None),
        Err(SightError::Unavailable(_))
    ));
    let request = SightRequest::try_new([0.0; 3], id("zone"), vec![], None).unwrap();
    assert!(matches!(
        AxiolidSightService::new(scene()).assess_sight(&request),
        Err(SightError::Unavailable(_))
    ));
    let request = SightRequest::try_new([0.0; 3], id("t"), vec![id("ghost")], None).unwrap();
    assert!(matches!(
        AxiolidSightService::new(scene()).assess_sight(&request),
        Err(SightError::UnknownObject(_))
    ));
}

/// From the middle of a window's outer face at (5, -0.3, 1.75), looking
/// south (-y) over a sector reaching `half` either side.
fn obstruction(
    geometry: AxiolidGeometry,
    half: f64,
    obstacles: &[&str],
) -> Result<SkyObstruction, SightError> {
    let request = SkyObstructionRequest::try_new(
        id("window"),
        [5.0, -0.3, 1.75],
        [0.0, -1.0],
        half,
        obstacles.iter().map(|local| id(local)).collect(),
    )
    .expect("valid request");
    AxiolidSightService::new(geometry).measure_sky_obstruction(&request)
}

/// A block 10 m high above the eye, its face 20 m in front of the window,
/// a tower behind the window and another building well to the side.
fn street() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("window"), cuboid([4.0, -0.3, 1.0], [6.0, 0.0, 2.5]))
        .with_mesh(
            id("block"),
            cuboid([-10.0, -30.3, 0.0], [20.0, -20.3, 11.75]),
        )
        .with_mesh(id("tower"), cuboid([0.0, 5.0, 0.0], [10.0, 10.0, 50.0]))
        .with_mesh(id("aside"), cuboid([40.0, -10.0, 0.0], [45.0, -5.0, 30.0]))
        .with_no_body(id("zone"))
}

#[test]
fn a_block_ten_metres_up_twenty_metres_ahead_rises_by_its_arctangent() {
    let expected = 0.5_f64.atan();
    for half in [0.0, 0.3, std::f64::consts::FRAC_PI_2] {
        let answer = obstruction(street(), half, &["block", "tower", "zone"]).unwrap();
        let (lower, upper) = answer.angle_radians();
        assert!(
            lower <= expected && expected <= upper && upper - lower < 1e-8,
            "{half}: {lower} {upper}"
        );
        assert_eq!(answer.highest(), [id("block")]);
        assert!(answer.evidence().exact);
    }
    // 26.57 degrees.
    assert!((expected.to_degrees() - 26.565).abs() < 1e-3);
}

#[test]
fn only_what_stands_in_front_within_the_sector_obstructs() {
    // The tower stands behind the window; the other building lies well to
    // the side of straight ahead.
    let answer = obstruction(street(), 0.3, &["tower", "aside"]).unwrap();
    assert_eq!(answer.angle_radians(), (0.0, 0.0));
    assert!(answer.highest().is_empty());
    let wide = obstruction(street(), 1.4, &["tower", "aside"]).unwrap();
    assert_eq!(wide.highest(), [id("aside")]);
    assert!(wide.angle_radians().0 > 0.0);
    // Below the horizon nothing obstructs.
    let low =
        AxiolidGeometry::new().with_mesh(id("kerb"), cuboid([0.0, -20.0, 0.0], [10.0, -19.0, 0.2]));
    let answer = obstruction(low, 0.0, &["kerb"]).unwrap();
    assert_eq!(answer.angle_radians(), (0.0, 0.0));
}

#[test]
fn a_tessellated_or_unmeasured_obstacle_in_front_refuses() {
    let tessellated = street().with_tessellated_mesh(
        id("block"),
        cuboid([-10.0, -30.3, 0.0], [20.0, -20.3, 11.75]),
        1e-3,
    );
    assert!(matches!(
        obstruction(tessellated, 0.0, &["block"]),
        Err(SightError::Unavailable(_))
    ));
    let unmeasured = street().with_unmeasured(id("ghost"), "failed");
    assert!(matches!(
        obstruction(unmeasured, 0.0, &["ghost"]),
        Err(SightError::Unavailable(_))
    ));
    // Behind the window, a tessellation cannot matter.
    let behind = street().with_tessellated_mesh(
        id("tower"),
        cuboid([0.0, 5.0, 0.0], [10.0, 10.0, 50.0]),
        1e-3,
    );
    assert!(obstruction(behind, 0.0, &["tower"]).is_ok());
}
