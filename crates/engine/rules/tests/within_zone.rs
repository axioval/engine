//! `count_within`, `sum_within`, `min_within`, `max_within` and
//! `mean_within` over the counterparts lying in a context zone (#360),
//! each located by its footprint or, lacking one of any area, by its
//! placement point.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ContextZones, MetricDirection, MetricFrame, MetricPoint, ObjectFrame, ObjectFrameError,
    ObjectFrameService, ObjectFrameServiceHandle, ObjectFront, PlanArea, PlanAreaError,
    PlanAreaService, PlanAreaServiceHandle, ServiceRegistry, SourceSnapshot, SuppliedZones,
    ZoneArea, ZoneAreaRequest,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::{Model, id, source};

/// Axis-aligned rectangular footprints `[x0, y0, x1, y1]` (one of no width
/// encloses no area), each exact; an object without one has no geometry.
/// Zones are read by their outer ring's bounding box.
struct Footprints(BTreeMap<ObjectId, [f64; 4]>);

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let width = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let depth = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    width * depth
}

impl PlanAreaService for Footprints {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let r = self
            .0
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let area = (r[2] - r[0]) * (r[3] - r[1]);
        PlanArea::try_new(area, area, Evidence::exact(source(), "footprint"))
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }

    fn measure_zone_area(&self, request: &ZoneAreaRequest) -> Result<ZoneArea, PlanAreaError> {
        let [object] = request.objects() else {
            panic!("each counterpart is located alone")
        };
        let rectangle = *self
            .0
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let outer = request.zone()[0].outer();
        let fold = |pick: fn(f64, f64) -> f64, axis: usize, start: f64| {
            outer.iter().map(|point| point[axis]).fold(start, pick)
        };
        let zone = [
            fold(f64::min, 0, f64::INFINITY),
            fold(f64::min, 1, f64::INFINITY),
            fold(f64::max, 0, f64::NEG_INFINITY),
            fold(f64::max, 1, f64::NEG_INFINITY),
        ];
        let a = request.accuracy_metres();
        let sure = [zone[0] + a, zone[1] + a, zone[2] - a, zone[3] - a];
        let possible = [zone[0] - a, zone[1] - a, zone[2] + a, zone[3] + a];
        let area = (rectangle[2] - rectangle[0]) * (rectangle[3] - rectangle[1]);
        let (least, most) = (overlap(rectangle, sure), overlap(rectangle, possible));
        let inside = (least, most);
        let outside = (area - most, area - least);
        #[allow(clippy::float_cmp)]
        let exact = least == most;
        let mut evidence = Evidence::exact(source(), format!("zone-area:{}", object.local_id));
        evidence.exact = exact;
        ZoneArea::try_new(
            vec![object.clone()],
            (area, area),
            inside,
            outside,
            evidence,
        )
    }
}

/// Placement origins `[x, y]`, each stated exactly; an object without one
/// is not placed.
struct Placements(Vec<SourceSnapshot>, BTreeMap<ObjectId, [f64; 2]>);

impl ObjectFrameService for Placements {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let [x, y] = *self
            .1
            .get(object)
            .ok_or_else(|| ObjectFrameError::NotPlaced(object.clone()))?;
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [x, y, 0.0]).unwrap(),
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap();
        let evidence = Evidence::exact(source(), format!("placement:{}", object.local_id));
        ObjectFrame::try_new(object.clone(), frame, ObjectFront::NotStated, evidence)
    }
}

/// Parcel `P1` (x 0..30, y 0..20) and building line `L1` along y = 15,
/// stated within `ACCURACY` metres.
const ZONES: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "axioval:model"}},
  "accuracy": ACCURACY,
  "features": [
    {"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
     "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [30, 0], [30, 20], [0, 20], [0, 0]]]}},
    {"type": "Feature", "id": "L1", "properties": {"kind": "building-line"},
     "geometry": {"type": "LineString", "coordinates": [[0, 15], [30, 15]]}}
  ]}"#;

/// What a tree states and where it stands.
struct Tree {
    local: &'static str,
    value: i64,
    footprint: Option<[f64; 4]>,
    placement: Option<[f64; 2]>,
}

/// The parcel's trees: `t1` and `t2` stand on it, `t3` is a point placed
/// on it, `t4` straddles its eastern boundary at x = 30, `t5` stands
/// beside it and `t6`, without a body, is placed far from it.
fn parcel_trees() -> Vec<Tree> {
    vec![
        Tree {
            local: "t1",
            value: 2,
            footprint: Some([2.0, 2.0, 4.0, 4.0]),
            placement: Some([0.0, 0.0]),
        },
        Tree {
            local: "t2",
            value: 3,
            footprint: Some([10.0, 10.0, 11.0, 11.0]),
            placement: None,
        },
        Tree {
            local: "t3",
            value: 4,
            footprint: Some([20.0, 15.0, 20.0, 15.0]),
            placement: Some([20.0, 15.0]),
        },
        Tree {
            local: "t4",
            value: 5,
            footprint: Some([29.0, 5.0, 31.0, 7.0]),
            placement: Some([30.0, 6.0]),
        },
        Tree {
            local: "t5",
            value: 7,
            footprint: Some([40.0, 5.0, 41.0, 6.0]),
            placement: Some([40.5, 5.5]),
        },
        Tree {
            local: "t6",
            value: 100,
            footprint: None,
            placement: Some([50.0, 50.0]),
        },
    ]
}

struct Site {
    trees: Vec<Tree>,
    accuracy: f64,
    zones: bool,
    frames: bool,
}

impl Site {
    fn new(trees: Vec<Tree>) -> Self {
        Self {
            trees,
            accuracy: 0.0,
            zones: true,
            frames: true,
        }
    }

    fn session(&self) -> (Project, ServiceRegistry) {
        let mut model = Model::default().object("site", "site");
        for tree in &self.trees {
            model = model.object(tree.local, "tree").value(
                tree.local,
                "Pset_Tree",
                "Value",
                PropertyValue::Integer(tree.value),
            );
        }
        let (project, mut services) = model.services();
        let footprints = self
            .trees
            .iter()
            .filter_map(|tree| tree.footprint.map(|footprint| (id(tree.local), footprint)))
            .collect();
        services
            .register(PlanAreaServiceHandle::new(Arc::new(Footprints(footprints))))
            .unwrap();
        if self.frames {
            let placements = self
                .trees
                .iter()
                .filter_map(|tree| tree.placement.map(|point| (id(tree.local), point)))
                .collect();
            services
                .register(ObjectFrameServiceHandle::new(Arc::new(Placements(
                    vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
                    placements,
                ))))
                .unwrap();
        }
        if self.zones {
            let text = ZONES.replace("ACCURACY", &self.accuracy.to_string());
            let file = ContextZones::read_geojson(text.as_bytes(), "zones.geojson").unwrap();
            services
                .register(SuppliedZones::try_from_files(vec![file]).unwrap())
                .unwrap();
        }
        (project, services)
    }

    fn measured(&self, name: &str) -> Result<Option<common::Cited>, String> {
        let (project, services) = self.session();
        common::measured_cited(&services, &project, &id("site"), name)
    }
}

fn in_parcel(aggregate: &str) -> String {
    if aggregate == "count" {
        "count_within;to=tree;zone=P1".to_owned()
    } else {
        format!("{aggregate}_within;to=tree;value=Pset_Tree/Value;zone=P1")
    }
}

/// The Done-when of #360: a parcel holding three trees, a fourth
/// straddling its boundary, counts 3 to 4 trees and sums their values as
/// an interval, 2 + 3 + 4 and possibly 5.
#[test]
fn trees_in_a_parcel_are_counted_and_summed_with_the_straddling_one_widening() {
    let site = Site::new(parcel_trees());
    assert_eq!(
        site.measured(&in_parcel("count")),
        Ok(Some(((3.0, 4.0), true)))
    );
    assert_eq!(
        site.measured(&in_parcel("sum")),
        Ok(Some(((9.0, 14.0), true)))
    );
    // The straddling tree may raise the greatest, never lower the least.
    assert_eq!(
        site.measured(&in_parcel("min")),
        Ok(Some(((2.0, 2.0), true)))
    );
    assert_eq!(
        site.measured(&in_parcel("max")),
        Ok(Some(((4.0, 5.0), true)))
    );
    let mean = site.measured(&in_parcel("mean")).unwrap_err();
    assert!(mean.contains("may or may not be within"), "{mean}");
}

/// Without the straddling tree every tree is decided: a point.
#[test]
fn trees_wholly_in_or_out_decide_the_values() {
    let trees = parcel_trees()
        .into_iter()
        .filter(|tree| tree.local != "t4")
        .collect();
    let site = Site::new(trees);
    assert_eq!(
        site.measured(&in_parcel("count")),
        Ok(Some(((3.0, 3.0), true)))
    );
    assert_eq!(
        site.measured(&in_parcel("mean")),
        Ok(Some(((3.0, 3.0), true)))
    );
}

/// A zone stated within 0.5 m leaves the trees well inside it surely in,
/// and the straddling one still only possibly: the evidence is inexact.
#[test]
fn a_zone_stated_within_an_accuracy_is_inexact() {
    let mut site = Site::new(parcel_trees());
    site.accuracy = 0.5;
    let ((lower, upper), exact) = site.measured(&in_parcel("sum")).unwrap().unwrap();
    assert_eq!((lower, upper), (9.0, 14.0));
    assert!(!exact);
}

/// A point tree is located by its placement, decided only farther than
/// the zone's accuracy from its boundary; a tree with neither a footprint
/// of area nor a placement may or may not lie in the zone.
#[test]
fn a_point_tree_is_located_by_its_placement() {
    let point = |local, placement| Tree {
        local,
        value: 1,
        footprint: Some([5.0, 5.0, 5.0, 5.0]),
        placement,
    };
    // Placed just inside the boundary, within the zone's accuracy.
    let mut site = Site::new(vec![point("p1", Some([29.8, 10.0]))]);
    site.accuracy = 0.5;
    assert_eq!(
        site.measured(&in_parcel("count"))
            .map(|value| value.map(|(bounds, _)| bounds)),
        Ok(Some((0.0, 1.0)))
    );
    site.accuracy = 0.1;
    assert_eq!(
        site.measured(&in_parcel("count")),
        Ok(Some(((1.0, 1.0), false)))
    );
    // On the boundary of an exact zone.
    let site = Site::new(vec![point("p1", Some([30.0, 10.0]))]);
    assert_eq!(
        site.measured(&in_parcel("count")),
        Ok(Some(((0.0, 1.0), true)))
    );
    // Neither a footprint of area nor a placement.
    let site = Site::new(vec![point("p1", None)]);
    assert_eq!(
        site.measured(&in_parcel("count")),
        Ok(Some(((0.0, 1.0), true)))
    );
    // A point is never read as outside for enclosing no area: without
    // the object-frame service that places it, it is not evaluated.
    let mut site = Site::new(vec![point("p1", Some([5.0, 5.0]))]);
    site.frames = false;
    let why = site.measured(&in_parcel("count")).unwrap_err();
    assert!(why.contains("object-frame service"), "{why}");
}

/// An unsupplied or unknown zone, and a line, are not evaluated, never
/// zero.
#[test]
fn a_zone_that_is_not_supplied_or_encloses_nothing_is_not_evaluated() {
    let site = Site::new(parcel_trees());
    let why = site
        .measured("sum_within;to=tree;value=Pset_Tree/Value;zone=P9")
        .unwrap_err();
    assert!(why.contains("no supplied context zone is `P9`"), "{why}");
    let why = site.measured("count_within;to=tree;zone=L1").unwrap_err();
    assert!(why.contains("encloses no area"), "{why}");
    let mut bare = Site::new(parcel_trees());
    bare.zones = false;
    let why = bare.measured(&in_parcel("count")).unwrap_err();
    assert!(why.contains("no context zones were supplied"), "{why}");
}

/// A zone reaches the counterparts at no distance: exactly one of
/// `radius`, `group` and `zone`, and nothing of a distance beside `zone`.
#[test]
fn a_zone_reach_states_no_distance() {
    use axioval_ir::measured::parse;
    let base = "sum_within;to=tree;value=Pset_Tree/Value";
    for (written, expected) in [
        (
            format!("{base};radius=15;zone=P1"),
            "`sum_within` parameter `zone`: one of `radius`, `group`, `zone` reaches the \
             counterparts",
        ),
        (
            format!("{base};group=parcels;zone=P1"),
            "`sum_within` parameter `zone`: one of `radius`, `group`, `zone` reaches the \
             counterparts",
        ),
        (
            format!("{base};zone=P1;from=2"),
            "`sum_within` parameter `from`: a context `zone` reaches the counterparts at no \
             distance",
        ),
        (
            "count_within;to=tree;zone=P1;radius=3".to_owned(),
            "`count_within` parameter `zone`: one of `radius`, `zone` reaches the counterparts",
        ),
        (
            "count_within;to=tree;zone=P1;projection=horizontal".to_owned(),
            "`count_within` parameter `projection`: a context `zone` reaches the \
             counterparts at no distance",
        ),
        (
            "count_within;to=tree".to_owned(),
            "`count_within` needs `radius`",
        ),
    ] {
        assert_eq!(parse(&written).unwrap_err().to_string(), expected);
    }
    assert!(parse(&in_parcel("mean")).is_ok());
    assert!(parse(&in_parcel("count")).is_ok());
    assert!(parse("count_within;to=tree;radius=3").is_ok());
}
