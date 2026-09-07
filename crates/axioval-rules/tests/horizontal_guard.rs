//! Horizontal-guard capability contract tests.
//!
//! ADR 0004: the service measures exposed edges and nearby elements; the
//! capability decides whether an edge is adequately guarded.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardError, GuardEvidence,
    GuardSearch, GuardService, GuardServiceHandle, NotEvaluatedReason, RuleCapability, RuleContext,
    ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::HorizontalGuard;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn rule_with(overrides: &[(&str, ParameterValue)]) -> CompiledRule {
    let number = |value: f64| ParameterValue::Number { value };
    let mut parameters = BTreeMap::from([
        ("minimum_barrier_height_metres".to_string(), number(1.0)),
        ("maximum_barrier_gap_metres".to_string(), number(0.1)),
        ("maximum_platform_gap_metres".to_string(), number(0.1)),
        ("maximum_landing_gap_metres".to_string(), number(0.3)),
        ("maximum_fall_height_metres".to_string(), number(0.5)),
        ("minimum_landing_width_metres".to_string(), number(1.0)),
        ("climbable_barrier_distance_metres".to_string(), number(0.3)),
        ("maximum_climbable_height_metres".to_string(), number(0.6)),
        (
            "minimum_climbable_side_length_metres".to_string(),
            number(0.1),
        ),
        (
            "measure_barrier_from_curb".to_string(),
            ParameterValue::Boolean { value: false },
        ),
    ]);
    for (key, value) in overrides {
        parameters.insert((*key).to_string(), value.clone());
    }
    CompiledRule {
        id: RuleId::new("guard").unwrap(),
        capability: "axioval:capability.horizontal-guard".into(),
        severity: RuleSeverity::Error,
        selector: Selector::EntityType {
            object_type: "slab".into(),
            include_subtypes: true,
        },
        parameters,
    }
}
fn rule() -> CompiledRule {
    rule_with(&[])
}

fn barrier(gap: f64, top: f64, interval: [f64; 2], curb: Option<f64>) -> GuardCandidate {
    GuardCandidate::try_new(oid("rail"), gap, top, interval, 0.0, curb).unwrap()
}
fn landing(gap: f64, top: f64, width: f64) -> GuardCandidate {
    GuardCandidate::try_new(oid("floor"), gap, top, [0.0, 1.0], width, None).unwrap()
}
fn climbable(distance: f64, top: f64, side: f64) -> ClimbableCandidate {
    ClimbableCandidate::try_new(oid("bench"), oid("rail"), distance, top, side).unwrap()
}

struct Stub(Result<GuardEdge, GuardError>);

impl GuardService for Stub {
    fn measure_guard_edges(&self, _search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        let edge = self.0.clone()?;
        GuardEvidence::try_new(vec![edge], 1, Evidence::exact(source(), "guard:edges"))
    }
}

fn evaluate(
    edge: Result<GuardEdge, GuardError>,
    rule: &CompiledRule,
) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(Stub(edge))))
        .unwrap();
    HorizontalGuard.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

fn edge(
    barriers: Vec<GuardCandidate>,
    landings: Vec<GuardCandidate>,
    climbables: Vec<ClimbableCandidate>,
) -> GuardEdge {
    GuardEdge::new(oid("slab-1"), barriers, landings, climbables)
}

#[test]
fn a_tall_close_barrier_covering_the_edge_is_protection() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn a_barrier_below_the_required_height_does_not_protect() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.0, 0.6, [0.0, 1.0], None)],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].message, "barrier_too_low");
}

#[test]
fn a_barrier_too_far_from_the_edge_does_not_protect() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.9, 1.2, [0.0, 1.0], None)],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
}

/// A barrier covering part of an edge leaves the rest unguarded.
#[test]
fn partial_barrier_coverage_is_not_protection() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.0, 1.2, [0.0, 0.5], None)],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].message, "hole_in_barrier");
}

/// Two rails guarding the same half do not add up to a guarded edge.
#[test]
fn overlapping_barriers_do_not_sum_to_full_coverage() {
    let outcome = evaluate(
        Ok(edge(
            vec![
                barrier(0.0, 1.2, [0.0, 0.5], None),
                barrier(0.0, 1.2, [0.25, 0.5], None),
            ],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert_eq!(
        outcome.findings().len(),
        1,
        "overlap must not fake coverage"
    );
}

/// A short fall onto a wide landing is acceptable without any barrier.
#[test]
fn a_short_fall_onto_a_wide_landing_is_protection() {
    let outcome = evaluate(
        Ok(edge(vec![], vec![landing(0.0, -0.4, 1.5)], vec![])),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn a_landing_too_far_below_is_not_protection() {
    let outcome = evaluate(
        Ok(edge(vec![], vec![landing(0.0, -3.0, 1.5)], vec![])),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
}

#[test]
fn a_landing_too_narrow_to_stand_on_is_not_protection() {
    let outcome = evaluate(
        Ok(edge(vec![], vec![landing(0.0, -0.4, 0.2)], vec![])),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
}

/// An adequate barrier is still defeated by something climbable beside it.
#[test]
fn a_climbable_object_defeats_an_otherwise_adequate_barrier() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
            vec![],
            vec![climbable(0.2, 0.5, 0.4)],
        )),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(
        outcome.findings()[0].message,
        "barrier_too_low_due_to_climbable_object"
    );
}

#[test]
fn a_distant_or_tall_or_narrow_object_does_not_defeat_the_barrier() {
    for climbable_candidate in [
        climbable(2.0, 0.5, 0.4),  // too far away
        climbable(0.2, 5.0, 0.4),  // too high to be a step
        climbable(0.2, 0.5, 0.01), // too narrow to stand on
    ] {
        let outcome = evaluate(
            Ok(edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![climbable_candidate],
            )),
            &rule(),
        );
        assert!(outcome.findings().is_empty());
    }
}

/// When the declaration says to measure from the curb, only the barrier's
/// exposed part counts -- a rail on a high curb is shorter than it looks.
#[test]
fn curb_measurement_reduces_effective_barrier_height() {
    let from_curb = [(
        "measure_barrier_from_curb",
        ParameterValue::Boolean { value: true },
    )];
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.0, 1.2, [0.0, 1.0], Some(0.5))],
            vec![],
            vec![],
        )),
        &rule_with(&from_curb),
    );
    assert_eq!(
        outcome.findings().len(),
        1,
        "1.2 m above the floor is only 0.7 m above a 0.5 m curb"
    );

    // The same barrier passes when the declaration measures from the floor.
    let from_floor = evaluate(
        Ok(edge(
            vec![barrier(0.0, 1.2, [0.0, 1.0], Some(0.5))],
            vec![],
            vec![],
        )),
        &rule(),
    );
    assert!(from_floor.findings().is_empty());
}

#[test]
fn unavailable_measurement_is_not_a_pass() {
    let outcome = evaluate(Err(GuardError::Unavailable), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = HorizontalGuard.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn invalid_declaration_is_refused() {
    let outcome = evaluate(
        Ok(edge(vec![], vec![], vec![])),
        &rule_with(&[(
            "minimum_barrier_height_metres",
            ParameterValue::String {
                value: "waist".into(),
            },
        )]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

// ---------------------------------------------------------------------------
// Defect naming and per-surface grouping.
//
// A capability that reports only "not guarded" is not actionable. These pin
// that each defect is named, and that a surface reports its worst one.
// ---------------------------------------------------------------------------

/// Serves several edges of the same surface from one measurement.
struct MultiEdge(Vec<GuardEdge>);

impl GuardService for MultiEdge {
    fn measure_guard_edges(&self, _search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        GuardEvidence::try_new(self.0.clone(), 1, Evidence::exact(source(), "guard:edges"))
    }
}

fn evaluate_edges(edges: Vec<GuardEdge>) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(MultiEdge(edges))))
        .unwrap();
    HorizontalGuard.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    )
}

fn message_of(outcome: &axioval_engine::CapabilityEvaluation) -> String {
    assert_eq!(outcome.findings().len(), 1, "{:?}", outcome.findings());
    outcome.findings()[0].message.clone()
}

#[test]
fn an_edge_with_nothing_near_it_names_a_missing_barrier() {
    let outcome = evaluate(Ok(edge(Vec::new(), Vec::new(), Vec::new())), &rule());
    assert_eq!(message_of(&outcome), "missing_barrier");
}

#[test]
fn a_short_barrier_is_named_too_low_not_merely_unguarded() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 0.4, [0.0, 1.0], None)],
            Vec::new(),
            Vec::new(),
        )),
        &rule(),
    );
    assert_eq!(message_of(&outcome), "barrier_too_low");
}

#[test]
fn a_tall_barrier_covering_part_of_the_edge_is_named_a_hole() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 1.2, [0.0, 0.4], None)],
            Vec::new(),
            Vec::new(),
        )),
        &rule(),
    );
    assert_eq!(message_of(&outcome), "hole_in_barrier");
}

#[test]
fn a_climbable_object_beside_an_adequate_barrier_is_named() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 1.2, [0.0, 1.0], None)],
            Vec::new(),
            vec![climbable(0.1, 0.5, 0.4)],
        )),
        &rule(),
    );
    assert_eq!(
        message_of(&outcome),
        "barrier_too_low_due_to_climbable_object"
    );
}

#[test]
fn a_defect_names_the_element_a_reviewer_must_look_at() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 0.4, [0.0, 1.0], None)],
            Vec::new(),
            Vec::new(),
        )),
        &rule(),
    );
    assert_eq!(outcome.findings()[0].related, vec![oid("rail")]);
}

#[test]
fn a_surface_reports_each_distinct_defect_once_not_one_finding_per_edge() {
    // Same surface, two edges: one merely has a hole, the other has nothing.
    let holed = edge(
        vec![barrier(0.05, 1.2, [0.0, 0.4], None)],
        Vec::new(),
        Vec::new(),
    );
    let bare = edge(Vec::new(), Vec::new(), Vec::new());
    let outcome = evaluate_edges(vec![holed, bare]);
    assert_eq!(
        outcome.findings().len(),
        2,
        "two different defects on one surface must both be reported: {:?}",
        outcome.findings()
    );
    assert_eq!(
        outcome.findings()[0].message,
        "missing_barrier",
        "the worst defect must win, not the first measured"
    );
}

#[test]
fn worst_defect_selection_is_independent_of_measurement_order() {
    let holed = edge(
        vec![barrier(0.05, 1.2, [0.0, 0.4], None)],
        Vec::new(),
        Vec::new(),
    );
    let bare = edge(Vec::new(), Vec::new(), Vec::new());
    let forward = evaluate_edges(vec![holed.clone(), bare.clone()]);
    let reversed = evaluate_edges(vec![bare, holed]);
    assert_eq!(
        forward.findings()[0].message,
        reversed.findings()[0].message
    );
}

#[test]
fn a_barrier_tall_only_from_its_curb_is_named_distinctly() {
    // Tall enough measured from the floor, but it stands on a curb: the
    // exposed part is what stops a fall. This must not read as a plain
    // "barrier_too_low" -- the remedy is different.
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 1.2, [0.0, 1.0], Some(0.9))],
            Vec::new(),
            Vec::new(),
        )),
        &rule_with(&[(
            "measure_barrier_from_curb",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert_eq!(
        message_of(&outcome),
        "barrier_too_low_due_to_curb",
        "a curb-lowered barrier needs its own diagnosis"
    );
}

#[test]
fn each_landing_shortfall_is_named_for_its_own_cause() {
    // Same edge, same missing protection, three different reasons -- a
    // reviewer needs to know which one to fix.
    let too_far = evaluate(
        Ok(edge(Vec::new(), vec![landing(5.0, -0.2, 2.0)], Vec::new())),
        &rule(),
    );
    assert_eq!(message_of(&too_far), "landing_too_far_away");

    let too_low = evaluate(
        Ok(edge(Vec::new(), vec![landing(0.05, -3.0, 2.0)], Vec::new())),
        &rule(),
    );
    assert_eq!(message_of(&too_low), "landing_too_low");

    let too_narrow = evaluate(
        Ok(edge(
            Vec::new(),
            vec![landing(0.05, -0.2, 0.05)],
            Vec::new(),
        )),
        &rule(),
    );
    assert_eq!(message_of(&too_narrow), "landings_too_small");
}

/// Related elements are reported in a stable order regardless of the order the
/// service measured them, so a finding does not churn between runs.
#[test]
fn related_elements_are_ordered_independently_of_measurement_order() {
    let far = GuardCandidate::try_new(oid("z-rail"), 0.05, 0.4, [0.0, 0.5], 0.0, None).unwrap();
    let near = GuardCandidate::try_new(oid("a-rail"), 0.05, 0.4, [0.5, 1.0], 0.0, None).unwrap();
    let outcome = evaluate(Ok(edge(vec![far, near], Vec::new(), Vec::new())), &rule());
    // Both halves are short, so both rails are named across the two sampled
    // edges. Whatever the set, it must be sorted.
    let related: Vec<&str> = outcome
        .findings()
        .iter()
        .flat_map(|finding| finding.related.iter())
        .map(|id| id.local_id.as_str())
        .collect();
    let mut sorted = related.clone();
    sorted.sort_unstable();
    assert_eq!(
        related, sorted,
        "related elements must be sorted, not left in measurement order"
    );
    assert!(!related.is_empty(), "the responsible rail must be named");
}

/// Two edges of one surface, each naming a different rail for the same defect,
/// merge into one finding whose related elements stay sorted.
#[test]
fn related_elements_merged_across_edges_stay_sorted() {
    let z = GuardCandidate::try_new(oid("z-rail"), 0.05, 0.4, [0.0, 1.0], 0.0, None).unwrap();
    let a = GuardCandidate::try_new(oid("a-rail"), 0.05, 0.4, [0.0, 1.0], 0.0, None).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(TwoEdges(
            edge(vec![z], Vec::new(), Vec::new()),
            edge(vec![a], Vec::new(), Vec::new()),
        ))))
        .unwrap();
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let outcome = HorizontalGuard.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    let related: Vec<&str> = outcome.findings()[0]
        .related
        .iter()
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(
        related,
        vec!["a-rail", "z-rail"],
        "merged set must be sorted"
    );
}

struct TwoEdges(GuardEdge, GuardEdge);
impl GuardService for TwoEdges {
    fn measure_guard_edges(&self, _search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        GuardEvidence::try_new(
            vec![self.0.clone(), self.1.clone()],
            1,
            Evidence::exact(source(), "guard:edges"),
        )
    }
}
