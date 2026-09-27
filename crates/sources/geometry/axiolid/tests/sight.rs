//! Lines of sight between real meshes: hidden by a wall, visible past a
//! column, undecided across a seam.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidSightService};
use axioval_engine::{SightError, SightOutcome, SightRequest, SightService};
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
