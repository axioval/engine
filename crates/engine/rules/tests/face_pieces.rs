//! The pieces of a face, one by one (`face_pieces`), and the pieces facing
//! a direction (`face=facing`), read by expression rules over a
//! single-body fill: a level crest between two batters, its base and ends.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, EvidenceSession, FaceNormal, FaceNormals, FacePiece, FacePieceSet,
    FacePieces, SurfaceFace, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// Every piece of each object's closed boundary, outward, with its area.
struct Fills(BTreeMap<ObjectId, Vec<(Vec<FaceNormal>, f64)>>);

impl Fills {
    fn pieces(
        &self,
        object: &ObjectId,
        set: FacePieceSet,
    ) -> Result<Vec<FacePiece>, VerticalExtentError> {
        let looks = |normals: &[FaceNormal]| match set {
            FacePieceSet::Top => normals.iter().all(|normal| normal.lower()[2] > 0.0),
            FacePieceSet::Bottom => normals.iter().all(|normal| normal.upper()[2] < 0.0),
            FacePieceSet::Boundary => true,
        };
        self.0
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?
            .iter()
            .filter(|(normals, _)| looks(normals))
            .map(|(normals, area)| FacePiece::try_new(normals.clone(), *area, *area))
            .collect()
    }
}

impl VerticalExtentService for Fills {
    fn measure_vertical_extent(&self, _: &ObjectId) -> Result<VerticalExtent, VerticalExtentError> {
        Err(VerticalExtentError::Unavailable("faces only".into()))
    }

    fn measure_face_normals(
        &self,
        object: &ObjectId,
        face: SurfaceFace,
    ) -> Result<FaceNormals, VerticalExtentError> {
        let normals: Vec<FaceNormal> = self
            .pieces(object, face.into())?
            .iter()
            .flat_map(|piece| piece.normals().to_vec())
            .collect();
        let mut evidence = Evidence::exact(source(), format!("faces:{object}"));
        evidence.exact = normals.iter().all(FaceNormal::is_exact);
        FaceNormals::try_new(object.clone(), face, normals, evidence)
    }

    fn measure_face_pieces(
        &self,
        object: &ObjectId,
        set: FacePieceSet,
    ) -> Result<FacePieces, VerticalExtentError> {
        let pieces = self.pieces(object, set)?;
        let mut evidence = Evidence::exact(source(), format!("pieces:{object}"));
        evidence.exact = pieces
            .iter()
            .all(|piece| piece.normals().iter().all(FaceNormal::is_exact));
        FacePieces::try_new(object.clone(), set, pieces, evidence)
    }
}

fn exact(vector: [f64; 3]) -> FaceNormal {
    FaceNormal::exact(vector).unwrap()
}

/// A fill 10 m long: a 3 m crest between batters falling 1:1.5 to east
/// and west, a 9 m base and two ends, and any `extra` pieces of its top.
fn fill(extra: Vec<(Vec<FaceNormal>, f64)>) -> Vec<(Vec<FaceNormal>, f64)> {
    let batter = 13.0_f64.sqrt() * 10.0;
    let mut pieces = vec![
        (vec![exact([0.0, 0.0, 1.0])], 30.0),
        (vec![exact([1.0, 0.0, 1.5])], batter),
        (vec![exact([-1.0, 0.0, 1.5])], batter),
        (vec![exact([0.0, 0.0, -1.0])], 90.0),
        (vec![exact([0.0, -1.0, 0.0])], 12.0),
        (vec![exact([0.0, 1.0, 0.0])], 12.0),
    ];
    pieces.extend(extra);
    pieces
}

fn fills() -> EvidenceSession {
    let model = ["fill", "berm", "skewed"]
        .iter()
        .fold(Model::default(), |model, local| model.object(local, "fill"));
    let fills = Fills(BTreeMap::from([
        (id("fill"), fill(Vec::new())),
        // A small sliver of one in five, facing east, on the crest's edge.
        (id("berm"), fill(vec![(vec![exact([1.0, 0.0, 0.2])], 0.01)])),
        // A batter whose normal is known only within a box: it leans
        // between about 42° and 66° from east, and falls from 0.45 to 1.1.
        (
            id("skewed"),
            vec![
                (vec![exact([0.0, 0.0, 1.0])], 30.0),
                (
                    vec![FaceNormal::try_new([0.9, 0.0, 1.0], [1.1, 0.0, 2.0]).unwrap()],
                    36.0,
                ),
                (vec![exact([0.0, 0.0, -1.0])], 60.0),
            ],
        ),
    ]));
    session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(fills)),
            &[snapshot()],
        )
        .unwrap()
}

fn measured(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:measured", "property": name})
}

fn member(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:member", "property": name})
}

fn ratio(angle: Value) -> Value {
    json!({"kind": "convertSlope", "operand": angle, "from": "angle", "to": "ratio"})
}

fn number(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "number", "value": value}})
}

fn at_most(left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals", "left": left, "right": right})
}

fn over(function: &str, list: &str, value: Option<Value>) -> Value {
    let mut aggregate = json!({"kind": "aggregate", "function": function,
        "over": {"kind": "measured", "name": list}});
    if let Some(value) = value {
        aggregate["value"] = value;
    }
    aggregate
}

fn requirement(id: &str, requirement: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("fill"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

fn check(rules: Vec<Value>) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["fill"], &[], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    run(registry, plan, &fills(), |runtime| runtime).unwrap()
}

fn found(report: &Report, rule: &str) -> Vec<String> {
    let mut found: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect();
    found.sort();
    found
}

fn open(report: &Report, rule: &str) -> Vec<(String, NotEvaluatedReason, String)> {
    let mut open: Vec<_> = report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == rule)
        .filter_map(|outcome| match &outcome.scope {
            axioval_ir::Scope::Object(object) => Some((
                object.local_id.clone(),
                outcome.reason.clone(),
                outcome.message.clone(),
            )),
            _ => None,
        })
        .collect();
    open.sort_by(|a, b| a.0.cmp(&b.0));
    open
}

fn open_objects(report: &Report, rule: &str) -> Vec<String> {
    open(report, rule)
        .into_iter()
        .map(|(object, _, _)| object)
        .collect()
}

const LIMIT: f64 = 0.7;

#[test]
fn the_steepest_piece_decides_where_the_hull_over_the_top_does_not() {
    let report = check(vec![
        requirement("hull", at_most(ratio(measured("slope")), number(LIMIT))),
        requirement(
            "steepest",
            at_most(
                over("max", "face_pieces", Some(ratio(member("slope")))),
                number(LIMIT),
            ),
        ),
    ]);
    // Over the whole top, the level crest widens every fill's slope down
    // to zero: 0 to 1:1.5 is within 0.7 and passes, but `berm`'s sliver
    // and `skewed`'s box straddle it.
    assert!(found(&report, "hull").is_empty());
    assert_eq!(open_objects(&report, "hull"), ["berm", "skewed"]);
    // Piece by piece, the sliver decides `berm`; `skewed`'s batter still
    // straddles the limit.
    assert_eq!(found(&report, "steepest"), ["berm"]);
    assert_eq!(open_objects(&report, "steepest"), ["skewed"]);
}

#[test]
fn every_batter_is_judged_and_small_pieces_are_filtered_by_area() {
    // Every piece larger than 1 m² falls no steeper than 1:1.5 (and a
    // little): the berm's sliver is too small to count.
    let report = check(vec![requirement(
        "batters",
        over(
            "all",
            "face_pieces",
            Some(json!({"kind": "implies",
                "antecedent": {"kind": "compare", "operator": "greaterThan",
                    "left": member("area"),
                    "right": {"kind": "literal",
                        "value": {"type": "quantity", "value": 1.0, "unit": "m2"}}},
                "consequent": at_most(ratio(member("slope")), number(LIMIT))})),
        ),
    )]);
    assert!(found(&report, "batters").is_empty());
    assert_eq!(open_objects(&report, "batters"), ["skewed"]);
}

#[test]
fn a_level_crest_descends_nowhere() {
    let report = check(vec![
        // Each fill has a level piece, whose direction of descent is
        // `null`: not every piece descends somewhere.
        requirement(
            "descends",
            over(
                "all",
                "face_pieces",
                Some(json!({"kind": "isDefined", "operand": member("gradient_direction")})),
            ),
        ),
        // One piece descends east, a bearing of a quarter turn.
        requirement(
            "east",
            over(
                "any",
                "face_pieces",
                Some(
                    json!({"kind": "between", "operand": member("gradient_direction"),
                    "low": {"kind": "literal",
                        "value": {"type": "quantity", "value": 89.0, "unit": "deg"}},
                    "high": {"kind": "literal",
                        "value": {"type": "quantity", "value": 91.0, "unit": "deg"}}}),
                ),
            ),
        ),
    ]);
    assert_eq!(found(&report, "descends"), ["berm", "fill", "skewed"]);
    assert!(found(&report, "east").is_empty());
    assert!(open_objects(&report, "east").is_empty());
}

#[test]
fn a_face_facing_a_direction_reads_only_its_pieces() {
    let east = "face=facing;direction=1,0,0;tolerance=60";
    let report = check(vec![
        // The east batter leans 56.3° from east; the crest, the base and
        // the ends lean 90°, the west batter 123.7°.
        requirement(
            "east-batter",
            at_most(ratio(measured(&format!("slope;{east}"))), number(LIMIT)),
        ),
        requirement(
            "facing-east",
            json!({"kind": "compare", "operator": "equals",
                "left": over("count", &format!("face_pieces;{east}"), None),
                "right": {"kind": "literal", "value": {"type": "integer", "value": 1}}}),
        ),
    ]);
    // `fill`'s east batter alone is read: the crest beside it leaves
    // nothing open. `berm`'s steep sliver faces east too, so its east face
    // spans 1:1.5 to 5 and straddles the limit, and whether `skewed`'s
    // batter faces east within 60° cannot be decided.
    assert!(found(&report, "east-batter").is_empty());
    let open = open(&report, "east-batter");
    assert_eq!(
        open.iter()
            .map(|(object, _, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["berm", "skewed"]
    );
    // A straddle is incomplete evidence; a face that cannot be told
    // apart is a measurement the backend could not make, as a face that
    // may stand vertical is.
    assert_eq!(open[0].1, NotEvaluatedReason::IncompleteEvidence);
    assert_eq!(open[1].1, NotEvaluatedReason::BackendUnavailable);
    assert!(
        open[1].2.contains("faces the direction cannot be decided"),
        "{}",
        open[1].2
    );
    // Counted, it is a possible member: the count straddles one.
    assert_eq!(found(&report, "facing-east"), ["berm"]);
    assert_eq!(open_objects(&report, "facing-east"), ["skewed"]);
}

#[test]
fn a_facing_face_needs_its_direction_and_tolerance() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["fill"], &[], &[]);
    for name in [
        "slope;face=facing;direction=1,0,0",
        "slope;direction=1,0,0;tolerance=10",
    ] {
        let refused = plan(
            &registry,
            &definitions,
            vec![requirement("r", at_most(measured(name), number(1.0)))],
        );
        assert!(refused.is_err(), "{name}");
    }
    let refused = plan(
        &registry,
        &definitions,
        vec![requirement(
            "r",
            over(
                "count",
                "face_pieces;face=facing;direction=0,0,1;tolerance=200",
                None,
            ),
        )],
    );
    assert!(refused.is_err());
}
