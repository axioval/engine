//! Free-floor options: obstacle selection, elevation band and merged spaces.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest,
    CompiledRule, CompletePlacementEvidence, CompleteSupportEvidence, ElevationBand,
    FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle,
    MetricDirection, MetricFrame, MetricPoint, NotEvaluatedReason, PlacementDomain,
    PlacementOutcome, PlacementRequest, RuleCapability,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, ObjectId};
use axioval_rules::{FreeFloorCircle, FreeFloorRectangle};
use common::{Model, id, kind, number, rule, selector, source, string, strings, unevaluated};

/// Finds a placement unless a blocking object is among the obstacles, and
/// records every request.
struct Scripted {
    blocking: BTreeSet<ObjectId>,
    requests: Mutex<Vec<PlacementRequest>>,
}

impl FreeSpaceService for Scripted {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("unused".into()))
    }
    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("unused".into()))
    }
    fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        self.requests.lock().unwrap().push(request.clone());
        let evidence = Evidence::exact(source(), "search");
        if request
            .obstacles()
            .iter()
            .any(|obstacle| self.blocking.contains(obstacle))
        {
            return Ok(PlacementOutcome::NoPlacement(
                CompletePlacementEvidence::try_new(request.clone(), evidence)?,
            ));
        }
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(request.scope().clone(), [0.0, 0.0, 0.0]).unwrap(),
            MetricDirection::try_new([1.0, 0.0, 0.0])?,
            MetricDirection::try_new([0.0, 1.0, 0.0])?,
            MetricDirection::try_new([0.0, 0.0, 1.0])?,
        )?;
        let found = match request.domain() {
            PlacementDomain::Supported(support) => ClearancePlacementEvidence::try_new_supported(
                request.clone(),
                frame.clone(),
                CompleteSupportEvidence::try_new(
                    support.support().clone(),
                    frame,
                    0.0,
                    evidence.clone(),
                )?,
                evidence,
            )?,
            _ => ClearancePlacementEvidence::try_new(request.clone(), frame, evidence)?,
        };
        Ok(PlacementOutcome::Found(found))
    }
}

fn model() -> Model {
    Model::default()
        .object("room", "room")
        .object("alcove", "space")
        .object("wall", "wall")
        .object("cabinet", "furniture")
        .object("crate", "furniture")
        .edge("Groups", "room", "alcove")
        .text("cabinet", "Pset", "Fixed", "yes")
        .text("crate", "Pset", "Fixed", "yes")
}

fn circle(parameters: Vec<(&str, ParameterValue)>) -> CompiledRule {
    let mut all = vec![
        ("diameter_metres", number(1.5)),
        ("height_metres", number(2.0)),
    ];
    all.extend(parameters);
    rule("axioval:capability.free-floor-circle", kind("room"), all)
}

fn run(
    model: Model,
    capability: &dyn RuleCapability,
    rule: &CompiledRule,
    blocking: &[&str],
) -> (CapabilityEvaluation, Vec<PlacementRequest>) {
    let service = Arc::new(Scripted {
        blocking: blocking.iter().map(|local| id(local)).collect(),
        requests: Mutex::new(Vec::new()),
    });
    let handle = service.clone();
    let evaluation = model.evaluate_with(capability, rule, |services| {
        services
            .register(FreeSpaceServiceHandle::new(handle))
            .unwrap();
    });
    let requests = service.requests.lock().unwrap().clone();
    (evaluation, requests)
}

fn ids(locals: &[&str]) -> Vec<ObjectId> {
    locals.iter().map(|local| id(local)).collect()
}

/// Without a selection every other object is an obstacle, as before.
#[test]
fn every_other_object_is_an_obstacle_by_default() {
    let (evaluation, requests) = run(model(), &FreeFloorCircle, &circle(vec![]), &[]);
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    assert_eq!(
        requests[0].obstacles(),
        &ids(&["alcove", "cabinet", "crate", "wall"])[..]
    );
    assert_eq!(requests[0].band(), None);
}

/// A selection sends only its objects: the wall is no obstacle here.
#[test]
fn an_obstacle_selection_chooses_the_obstacles() {
    let rule = circle(vec![("obstacles", selector(kind("furniture")))]);
    let (evaluation, requests) = run(model(), &FreeFloorCircle, &rule, &["cabinet"]);
    assert_eq!(requests[0].obstacles(), &ids(&["cabinet", "crate"])[..]);
    assert_eq!(evaluation.findings().len(), 1);
    assert_eq!(
        evaluation.findings()[0].message,
        "NO_FREE_FLOOR_SPACE_FOR_CIRCLE"
    );
    // The wall blocks, but is not selected.
    let (evaluation, _) = run(model(), &FreeFloorCircle, &rule, &["wall"]);
    assert!(evaluation.findings().is_empty());
}

fn fixed() -> Selector {
    Selector::Property {
        property_set: Some("Pset".into()),
        property: "Fixed".into(),
        operator: ComparisonOperator::Exists,
        value: None,
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    }
}

/// An object the selection cannot decide is sent as a candidate, so a
/// witness stands; a proof of absence must hold without it too.
#[test]
fn undecided_obstacles_can_only_keep_a_proof_open() {
    let rule = circle(vec![("obstacles", selector(fixed()))]);

    // Only the undecided crate blocks: not evaluated, never a finding.
    let (evaluation, requests) = run(
        model().unreadable("crate"),
        &FreeFloorCircle,
        &rule,
        &["crate"],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        vec![("room".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(requests[0].obstacles(), &ids(&["cabinet", "crate"])[..]);
    assert_eq!(requests[1].obstacles(), &ids(&["cabinet"])[..]);

    // The cabinet, surely selected, blocks on its own: the finding stands.
    let (evaluation, _) = run(
        model().unreadable("crate"),
        &FreeFloorCircle,
        &rule,
        &["cabinet"],
    );
    assert_eq!(evaluation.findings().len(), 1);
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    // Nothing blocks: the witness with the crate included stands.
    let (evaluation, requests) = run(model().unreadable("crate"), &FreeFloorCircle, &rule, &[]);
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    assert_eq!(requests.len(), 1);
}

/// The band is sent as declared; a missing end defaults to the floor or the
/// shape's height.
#[test]
fn an_elevation_band_is_sent_with_the_request() {
    for (parameters, expected) in [
        (vec![("band_to_metres", number(0.67))], (0.0, 0.67)),
        (vec![("band_from_metres", number(0.1))], (0.1, 2.0)),
        (
            vec![
                ("band_from_metres", number(0.1)),
                ("band_to_metres", number(0.67)),
            ],
            (0.1, 0.67),
        ),
    ] {
        let (_, requests) = run(model(), &FreeFloorCircle, &circle(parameters), &[]);
        assert_eq!(
            requests[0].band(),
            Some(ElevationBand::try_new(expected.0, expected.1).unwrap())
        );
    }
}

#[test]
fn an_invalid_band_is_an_invalid_declaration() {
    for parameters in [
        vec![
            ("band_from_metres", number(1.0)),
            ("band_to_metres", number(0.5)),
        ],
        vec![("band_from_metres", number(-0.1))],
        vec![("band_from_metres", number(2.5))],
        vec![("band_to_metres", string("high"))],
    ] {
        let (evaluation, requests) = run(model(), &FreeFloorCircle, &circle(parameters), &[]);
        assert!(requests.is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            vec![("room".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Spaces the merge path reaches are searched with the space, are never
/// obstacles, and are related to the finding; the search is bounded by
/// their footprints rather than supported by one of them.
#[test]
fn merged_spaces_are_searched_together() {
    let rule = circle(vec![("merge_path", strings(&["Groups:forward"]))]);
    let (evaluation, requests) = run(model(), &FreeFloorCircle, &rule, &["wall"]);
    assert_eq!(requests[0].merged_scopes(), &ids(&["alcove"])[..]);
    assert_eq!(
        requests[0].obstacles(),
        &ids(&["cabinet", "crate", "wall"])[..]
    );
    assert_eq!(requests[0].domain(), &PlacementDomain::Unconstrained);
    let finding = &evaluation.findings()[0];
    assert_eq!(finding.related, ids(&["alcove"]));
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "scan:Groups")
    );

    // Reaching nothing searches the space alone, on its own floor.
    let (_, requests) = run(
        Model::default()
            .object("room", "room")
            .edge("Groups", "other", "room"),
        &FreeFloorCircle,
        &rule,
        &[],
    );
    assert!(requests[0].merged_scopes().is_empty());
    assert!(matches!(
        requests[0].domain(),
        PlacementDomain::Supported(_)
    ));
}

/// A merge path the source cannot answer leaves the space not evaluated.
#[test]
fn an_unanswered_merge_path_is_not_evaluated() {
    let rule = circle(vec![("merge_path", strings(&["Unknown:forward"]))]);
    let (evaluation, requests) = run(model(), &FreeFloorCircle, &rule, &[]);
    assert!(requests.is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        vec![("room".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
}

/// The rectangle takes the same options.
#[test]
fn the_rectangle_takes_the_same_options() {
    let rule = rule(
        "axioval:capability.free-floor-rectangle",
        kind("room"),
        vec![
            ("width_metres", number(1.8)),
            ("length_metres", number(1.5)),
            ("height_metres", number(2.0)),
            ("orientation", string("any")),
            ("obstacles", selector(kind("furniture"))),
            ("band_to_metres", number(0.67)),
            ("merge_path", strings(&["Groups:forward"])),
        ],
    );
    let (evaluation, requests) = run(model(), &FreeFloorRectangle, &rule, &["crate"]);
    assert_eq!(requests[0].obstacles(), &ids(&["cabinet", "crate"])[..]);
    assert_eq!(requests[0].merged_scopes(), &ids(&["alcove"])[..]);
    assert_eq!(
        requests[0].band(),
        Some(ElevationBand::try_new(0.0, 0.67).unwrap())
    );
    assert_eq!(
        evaluation.findings()[0].message,
        "NO_FREE_FLOOR_SPACE_FOR_RECTANGLE"
    );
}
