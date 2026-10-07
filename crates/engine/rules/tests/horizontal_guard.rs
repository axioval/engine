//! Horizontal-guard capability contract tests.
//!
//! ADR 0004: the service measures exposed edges and nearby elements; the
//! capability decides whether an edge is adequately guarded.
//!
//! The capability runs as a template; every evaluation here runs it and the
//! implementation it replaced (`axioval_rules::reference::HorizontalGuard`)
//! on the same services and holds the template to its whole outside
//! contract (`held`).
#![allow(missing_docs)]

mod common;

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardError, GuardEvidence,
    GuardSearch, GuardService, GuardServiceHandle, NotEvaluatedReason, RuleCapability, RuleContext,
    ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::HorizontalGuard;
use axioval_rules::reference::HorizontalGuard as Reference;

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
            include_subtypes: false,
        },
        parameters,
    }
}
fn rule() -> CompiledRule {
    rule_with(&[])
}

/// The template's evaluation of `rule`, its measured values read as a run
/// reads them, held to the replaced implementation's whole contract on the
/// same services.
fn held(
    project: &Project,
    services: &ServiceRegistry,
    rule: &CompiledRule,
) -> axioval_engine::CapabilityEvaluation {
    use axioval_rules::parity::{Observations, Parity};
    let reference = Reference.evaluate(&RuleContext { project, services }, rule);
    let mut measured = services.clone();
    axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut measured, project);
    let values = axioval_engine::MeasuredValues::of(&measured, project);
    measured.register(values).unwrap();
    let template = HorizontalGuard.evaluate(
        &RuleContext {
            project,
            services: &measured,
        },
        rule,
    );
    let parity = Parity::contract().compare(
        ("horizontal-guard", &Observations::of_evaluation(&reference)),
        ("template", &Observations::of_evaluation(&template)),
    );
    assert!(parity.holds(), "{}", parity.diff());
    template
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
    held(&project, &services, rule)
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
            vec![barrier(0.0, 1.2, [0.0, 0.6], None)],
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
    let outcome = held(&project, &services, &rule());
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
    held(&project, &services, &rule())
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
            vec![barrier(0.05, 1.2, [0.0, 0.7], None)],
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
        vec![barrier(0.05, 1.2, [0.0, 0.7], None)],
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
    // Both are reported, whichever edge was measured first; a report
    // orders them, not the evaluation.
    let mut messages: Vec<&str> = outcome
        .findings()
        .iter()
        .map(|finding| finding.message.as_str())
        .collect();
    messages.sort_unstable();
    assert_eq!(messages, ["hole_in_barrier", "missing_barrier"]);
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
    let outcome = held(&project, &services, &rule());
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

/// A stub of railing beside a long open edge is not a barrier with a hole in
/// it -- the edge is simply unguarded. Native gates the whole barrier branch
/// on more than half the edge being reached, and the distinction matters: a
/// reviewer told `hole_in_barrier` looks for a gap to close, while
/// `missing_barrier` says the railing was never built.
#[test]
fn a_barrier_along_less_than_half_the_edge_is_absent_not_holed() {
    let outcome = evaluate(
        Ok(edge(
            vec![barrier(0.05, 1.2, [0.0, 0.4], None)],
            Vec::new(),
            Vec::new(),
        )),
        &rule(),
    );
    assert_eq!(
        message_of(&outcome),
        "missing_barrier",
        "a barrier covering 40% of the edge is not present on it"
    );
}

/// Measures slab-1 only and a stray surface outside the selection, after
/// checking that the rule asked for exactly its selection.
struct SelectionStub;

impl GuardService for SelectionStub {
    fn measure_guard_edges(&self, search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        assert_eq!(search.surfaces(), &[oid("slab-1"), oid("slab-2")]);
        GuardEvidence::try_new(
            vec![
                GuardEdge::new(oid("slab-1"), vec![], vec![], vec![]),
                GuardEdge::new(oid("roof"), vec![], vec![], vec![]),
            ],
            2,
            Evidence::exact(source(), "guard:edges"),
        )
    }
}

/// The selection is the walking-surface profile. A selected surface with no
/// measured edge is not evaluated rather than "nothing to guard", and an edge
/// outside the selection is not this rule's finding.
#[test]
fn the_selection_is_the_walking_surface_profile() {
    let project = Project::new(vec![
        Object::new(oid("slab-1"), "slab"),
        Object::new(oid("slab-2"), "slab"),
        Object::new(oid("roof"), "roof"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(SelectionStub)))
        .unwrap();
    let outcome = held(&project, &services, &rule());
    assert_eq!(outcome.findings().len(), 1, "{:?}", outcome.findings());
    assert_eq!(outcome.findings()[0].object_id(), Some(&oid("slab-1")));
    let not_evaluated = outcome.not_evaluated_outcomes();
    assert_eq!(not_evaluated.len(), 1);
    assert_eq!(not_evaluated[0].object_id(), Some(&oid("slab-2")));
}

// ---------------------------------------------------------------------------
// Candidate roles.
//
// A mesh does not say whether a body is a railing or a cupboard. The ruleset
// names the objects that may play each role, and nothing else counts.
// ---------------------------------------------------------------------------

fn entity(object_type: &str) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(Selector::EntityType {
            object_type: object_type.into(),
            include_subtypes: false,
        }),
    }
}

/// Reports a cupboard as a full-height barrier along the whole edge, as an
/// adapter that ignored the candidate sets would, after checking that the
/// rule sent the sets it resolved.
struct CupboardStub;

impl GuardService for CupboardStub {
    fn measure_guard_edges(&self, search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        assert_eq!(search.barrier_candidates(), Some(&[oid("rail")][..]));
        assert_eq!(search.landing_candidates(), Some(&[][..]));
        assert_eq!(search.climbable_candidates(), None);
        let cupboard =
            GuardCandidate::try_new(oid("cupboard"), 0.0, 2.0, [0.0, 1.0], 0.0, None).unwrap();
        GuardEvidence::try_new(
            vec![GuardEdge::new(
                oid("slab-1"),
                vec![cupboard],
                vec![],
                vec![],
            )],
            1,
            Evidence::exact(source(), "guard:edges"),
        )
    }
}

fn evaluate_roles(rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![
        Object::new(oid("slab-1"), "slab"),
        Object::new(oid("rail"), "railing"),
        Object::new(oid("cupboard"), "furniture"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(CupboardStub)))
        .unwrap();
    held(&project, &services, rule)
}

/// A cupboard along the edge is not a barrier when the ruleset names only
/// railings as barriers, even if the measurement reports it as one.
#[test]
fn an_object_outside_the_barrier_selection_is_not_protection() {
    let outcome = evaluate_roles(&rule_with(&[
        ("barrier_selector", entity("railing")),
        ("landing_selector", entity("terrace")),
    ]));
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert_eq!(message_of(&outcome), "missing_barrier");
    assert!(outcome.findings()[0].related.is_empty());
}

#[test]
fn a_role_selector_of_the_wrong_type_is_refused() {
    let outcome = evaluate(
        Ok(edge(vec![], vec![], vec![])),
        &rule_with(&[(
            "barrier_selector",
            ParameterValue::String {
                value: "railing".into(),
            },
        )]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// Guard edges are measured from exact evidence only: the service contract
/// refuses an approximate measurement, so no tessellated edge is ever
/// listed, and an exact one lists exact members.
#[test]
fn approximate_guard_edges_are_never_measured() {
    use axioval_engine::{CapabilityRegistry, measured_members};
    let tall = || edge(vec![barrier(0.0, 1.2, [0.0, 1.0], None)], vec![], vec![]);
    let approximate = Evidence {
        source: source(),
        locator: "guard:mesh".into(),
        exact: false,
    };
    assert!(matches!(
        GuardEvidence::try_new(vec![tall()], 1, approximate),
        Err(GuardError::InexactEvidence)
    ));
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(GuardServiceHandle::new(Arc::new(Stub(Ok(tall())))))
        .unwrap();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    let edges = measured_members(
        &services,
        &oid("slab-1"),
        "guard_edges;barrier_gap=0.1;platform_gap=0.1;landing_gap=0.3;landing_width=1;\
         climb_distance=0.3;climb_side=0.1",
    )
    .unwrap();
    assert_eq!(edges.len(), 1);
    assert!(edges[0].exact);
}

/// `horizontal-guard`'s decision as an expression over the measured edges:
/// an edge is guarded when its barriers reach the height along all of it and
/// nothing beside them defeats them, or, reached by barriers along at most
/// half of it, when the landings cover it within the fall allowed. The
/// barrier height, the fall and the climbable height are the expression's;
/// the searches and gaps the list's. On every fixture it flags and leaves
/// open what the capability does.
#[test]
#[allow(clippy::too_many_lines)]
fn the_guard_decision_as_an_expression_over_edges_reaches_the_verdicts() {
    use axioval_rules::ExpressionRequirement;
    use serde_json::{Value, json};
    let field =
        |name: &str| json!({"kind": "property", "propertySet": "axioval:member", "property": name});
    let m = |value: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}});
    let compare = |operator: &str, left: Value, right: Value| json!({"kind": "compare", "operator": operator, "left": left, "right": right});
    let rounded = |name: &str| json!({"kind": "round", "operand": field(name), "step": m(1e-6)});
    // A field an edge does not state (no barrier, nothing climbable, no
    // landing) is `null`, which decides nothing: each test states that an
    // absent field does not meet it.
    let stated = |name: &str, test: Value| json!({"kind": "and", "operands": [{"kind": "isDefined", "operand": field(name)}, test]});
    let requirement = |curb: bool| {
        let covered = stated(
            "guarded_height",
            compare("greaterThanOrEquals", rounded("guarded_height"), m(1.0)),
        );
        let climbed = stated(
            "climbable_height",
            compare("lessThanOrEquals", rounded("climbable_height"), m(0.6)),
        );
        let present = compare(
            "greaterThan",
            field("barrier_share"),
            json!({"kind": "literal", "value": {"type": "number", "value": 0.5}}),
        );
        let landed = stated(
            "landing_fall",
            compare("lessThanOrEquals", rounded("landing_fall"), m(0.5)),
        );
        let list = format!(
            "guard_edges;barrier_gap=0.1;platform_gap=0.1;landing_gap=0.3;landing_width=1;\
             climb_distance=0.3;climb_side=0.1;measure_from={}",
            if curb { "curb" } else { "floor" }
        );
        json!({"kind": "aggregate", "function": "all", "over": {"kind": "measured", "name": list},
            "value": {"kind": "if",
                "branches": [{"when": covered, "then": {"kind": "not", "operand": climbed}}],
                "else": {"kind": "and", "operands": [{"kind": "not", "operand": present}, landed]}}})
    };
    let fixtures: Vec<(Vec<GuardEdge>, bool)> = vec![
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 0.6, [0.0, 1.0], None)],
                vec![],
                vec![],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.9, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 0.6], None)],
                vec![],
                vec![],
            )],
            false,
        ),
        (
            vec![edge(
                vec![
                    barrier(0.0, 1.2, [0.0, 0.5], None),
                    barrier(0.0, 1.2, [0.25, 0.5], None),
                ],
                vec![],
                vec![],
            )],
            false,
        ),
        (
            vec![edge(vec![], vec![landing(0.0, -0.4, 1.5)], vec![])],
            false,
        ),
        (
            vec![edge(vec![], vec![landing(0.0, -3.0, 1.5)], vec![])],
            false,
        ),
        (
            vec![edge(vec![], vec![landing(0.0, -0.4, 0.2)], vec![])],
            false,
        ),
        (
            vec![edge(vec![], vec![landing(5.0, -0.2, 2.0)], vec![])],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![climbable(0.2, 0.5, 0.4)],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![climbable(2.0, 0.5, 0.4)],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![climbable(0.2, 5.0, 0.4)],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], None)],
                vec![],
                vec![climbable(0.2, 0.5, 0.01)],
            )],
            false,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], Some(0.5))],
                vec![],
                vec![],
            )],
            true,
        ),
        (
            vec![edge(
                vec![barrier(0.0, 1.2, [0.0, 1.0], Some(0.5))],
                vec![],
                vec![],
            )],
            false,
        ),
        (vec![edge(vec![], vec![], vec![])], false),
        // A short stub beside a long open edge falls through to its landing.
        (
            vec![edge(
                vec![barrier(0.05, 0.4, [0.0, 0.3], None)],
                vec![landing(0.0, -0.4, 1.5)],
                vec![],
            )],
            false,
        ),
        // Reached along most of it by a low rail, the landing does not help.
        (
            vec![edge(
                vec![barrier(0.05, 0.4, [0.0, 1.0], None)],
                vec![landing(0.0, -0.4, 1.5)],
                vec![],
            )],
            false,
        ),
        (
            vec![
                edge(vec![barrier(0.05, 1.2, [0.0, 0.7], None)], vec![], vec![]),
                edge(vec![barrier(0.0, 1.2, [0.0, 1.0], None)], vec![], vec![]),
            ],
            false,
        ),
    ];
    let project = Project::new(vec![Object::new(oid("slab-1"), "slab")]).unwrap();
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    for (index, (edges, curb)) in fixtures.into_iter().enumerate() {
        let curb_rule = rule_with(&[(
            "measure_barrier_from_curb",
            ParameterValue::Boolean { value: curb },
        )]);
        let mut services = ServiceRegistry::new();
        services
            .register(GuardServiceHandle::new(Arc::new(MultiEdge(edges))))
            .unwrap();
        let expected = held(&project, &services, &curb_rule);
        registry.install_measured(&mut services, &project);
        let expression = CompiledRule {
            capability: "axioval:capability.expression".into(),
            parameters: BTreeMap::from([(
                "requirement".to_owned(),
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement(curb)).unwrap(),
                },
            )]),
            ..curb_rule
        };
        let evaluation = ExpressionRequirement.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &expression,
        );
        let parity = axioval_rules::parity::compare_evaluations(
            ("guard", &expected),
            ("expression", &evaluation),
        );
        assert!(parity.holds(), "fixture {index}:\n{}", parity.diff());
    }
    // An unavailable measurement and a missing service leave both open.
    for unavailable in [true, false] {
        let mut services = ServiceRegistry::new();
        if unavailable {
            services
                .register(GuardServiceHandle::new(Arc::new(Stub(Err(
                    GuardError::Unavailable,
                )))))
                .unwrap();
        }
        let expected = held(&project, &services, &rule());
        registry.install_measured(&mut services, &project);
        let expression = CompiledRule {
            capability: "axioval:capability.expression".into(),
            parameters: BTreeMap::from([(
                "requirement".to_owned(),
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement(false)).unwrap(),
                },
            )]),
            ..rule()
        };
        let evaluation = ExpressionRequirement.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &expression,
        );
        let parity = axioval_rules::parity::compare_evaluations(
            ("guard", &expected),
            ("expression", &evaluation),
        );
        // Divergence D4: the capability leaves the whole rule open, the
        // expression each slab, for the same reason. The harness compares
        // the rule-scoped outcome too, so both differ.
        let reason = expected.not_evaluated_outcomes()[0].reason().clone();
        assert_eq!(expected.not_evaluated_outcomes()[0].object_id(), None);
        assert_eq!(
            parity.differences,
            vec![
                axioval_rules::parity::Difference {
                    scope: axioval_ir::Scope::Project,
                    capability: Some(axioval_rules::parity::Outcome::NotEvaluated {
                        reason: reason.clone()
                    }),
                    expression: None,
                    details: vec![],
                },
                axioval_rules::parity::Difference {
                    scope: oid("slab-1").into(),
                    capability: None,
                    expression: Some(axioval_rules::parity::Outcome::NotEvaluated { reason }),
                    details: vec![],
                },
            ]
        );
    }
}

/// Every refusal is the rule's, after it selected, worded as the capability
/// worded it; a rule selecting nothing judges nothing, its declaration
/// included.
#[test]
#[allow(clippy::too_many_lines)]
fn refusals_are_the_rules_and_worded_as_before() {
    let project = Project::new(vec![
        Object::new(oid("slab-1"), "slab"),
        Object::new(oid("slab-2"), "slab"),
    ])
    .unwrap();
    let edges = || {
        let mut services = ServiceRegistry::new();
        services
            .register(GuardServiceHandle::new(Arc::new(SelectionStub)))
            .unwrap();
        services
    };
    let opened = |outcome: &axioval_engine::CapabilityEvaluation| {
        outcome
            .not_evaluated_outcomes()
            .iter()
            .map(|outcome| {
                (
                    outcome.object_id().map(|object| object.local_id.clone()),
                    outcome.reason().clone(),
                    outcome.message().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let number = |value: f64| ParameterValue::Number { value };
    let zero_gaps = rule_with(&[
        ("maximum_barrier_gap_metres", number(0.0)),
        ("maximum_platform_gap_metres", number(0.0)),
        ("maximum_landing_gap_metres", number(0.0)),
        ("climbable_barrier_distance_metres", number(0.0)),
    ]);
    for (services, rule, reason, message) in [
        (
            edges(),
            rule_with(&[("maximum_fall_height_metres", number(-0.5))]),
            NotEvaluatedReason::InvalidDeclaration,
            "horizontal-guard declaration is missing or not realisable",
        ),
        (
            edges(),
            rule_with(&[(
                "measure_barrier_from_curb",
                ParameterValue::String {
                    value: "yes".into(),
                },
            )]),
            NotEvaluatedReason::InvalidDeclaration,
            "horizontal-guard declaration is missing or not realisable",
        ),
        (
            ServiceRegistry::new(),
            rule(),
            NotEvaluatedReason::MissingService,
            "guard service is not registered",
        ),
        (
            edges(),
            zero_gaps,
            NotEvaluatedReason::InvalidDeclaration,
            "horizontal-guard thresholds do not define a usable search",
        ),
    ] {
        let outcome = held(&project, &services, &rule);
        assert_eq!(opened(&outcome), [(None, reason, message.to_owned())]);
    }
    let unavailable = evaluate(Err(GuardError::Unavailable), &rule());
    assert_eq!(
        opened(&unavailable),
        [(
            None,
            NotEvaluatedReason::IncompleteEvidence,
            "guard measurement is unavailable".to_owned()
        )]
    );
    let wrong = evaluate(
        Ok(edge(vec![], vec![], vec![])),
        &rule_with(&[("climbable_selector", ParameterValue::Number { value: 1.0 })]),
    );
    assert!(
        wrong.not_evaluated_outcomes()[0]
            .message()
            .starts_with("horizontal-guard: "),
        "{:?}",
        opened(&wrong)
    );
    // A surface without a measured edge is open on its own.
    let outcome = held(&project, &edges(), &rule());
    assert_eq!(
        opened(&outcome),
        [(
            Some("slab-2".to_owned()),
            NotEvaluatedReason::IncompleteEvidence,
            "no edge was measured for this walking surface; it has no measurable body".to_owned()
        )]
    );
    // Nothing selected, nothing judged, the declaration included.
    let nothing = Project::new(vec![Object::new(oid("roof"), "roof")]).unwrap();
    let outcome = held(
        &nothing,
        &ServiceRegistry::new(),
        &rule_with(&[("maximum_fall_height_metres", number(-0.5))]),
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert!(outcome.findings().is_empty());
}

/// A role selection that cannot decide an object leaves the rule open,
/// naming the selector and how many objects it leaves undecided.
#[test]
fn an_undecided_role_selection_leaves_the_rule_open() {
    let (project, mut services) = common::Model::default()
        .object("slab-1", "slab")
        .object("rail-1", "railing")
        .unreadable("rail-1")
        .services();
    services
        .register(GuardServiceHandle::new(Arc::new(Stub(Ok(GuardEdge::new(
            common::id("slab-1"),
            vec![],
            vec![],
            vec![],
        ))))))
        .unwrap();
    let barrier = common::selector(Selector::Property {
        property_set: Some("Pset".into()),
        property: "Barrier".into(),
        operator: axioval_ir::contract::ComparisonOperator::Equals,
        value: Some(ParameterValue::Boolean { value: true }),
        case_sensitive: false,
        trim: false,
        quantifier: None,
        precision: None,
    });
    let outcome = held(
        &project,
        &services,
        &rule_with(&[("barrier_selector", barrier)]),
    );
    assert!(outcome.findings().is_empty());
    let open = outcome.not_evaluated_outcomes();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].object_id(), None);
    assert_eq!(open[0].reason(), &NotEvaluatedReason::IncompleteEvidence);
    assert_eq!(
        open[0].message(),
        "horizontal-guard: `barrier_selector` cannot be decided for 2 object(s)"
    );
}

mod generated {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;

    /// A candidate's gap, top, interval along the edge, curb, width and
    /// element.
    type Raw = (u8, i8, (u8, u8), Option<u8>, u8, u8);

    fn raw() -> impl Strategy<Value = Raw> {
        (
            0u8..8,
            -30i8..30,
            (0u8..10, 0u8..11),
            proptest::option::weighted(0.3, 0u8..8),
            0u8..20,
            0u8..4,
        )
    }

    fn interval((start, length): (u8, u8)) -> [f64; 2] {
        let start = f64::from(start) / 10.0;
        [start, (start + f64::from(length) / 10.0).min(1.0)]
    }

    fn barrier((gap, top, along, curb, _, which): Raw) -> GuardCandidate {
        GuardCandidate::try_new(
            oid(&format!("rail-{which}")),
            f64::from(gap) / 20.0,
            f64::from(top.unsigned_abs()) / 20.0,
            interval(along),
            0.0,
            curb.map(|curb| f64::from(curb) / 10.0),
        )
        .unwrap()
    }

    fn landing((gap, top, along, _, width, which): Raw) -> GuardCandidate {
        GuardCandidate::try_new(
            oid(&format!("floor-{which}")),
            f64::from(gap) / 10.0,
            -f64::from(top.unsigned_abs()) / 20.0,
            interval(along),
            f64::from(width) / 10.0,
            None,
        )
        .unwrap()
    }

    fn climbable((gap, top, _, _, width, which): Raw) -> ClimbableCandidate {
        ClimbableCandidate::try_new(
            oid(&format!("bench-{which}")),
            oid(&format!("rail-{}", which % 2 * 2)),
            f64::from(gap) / 10.0,
            f64::from(top.unsigned_abs()) / 20.0,
            f64::from(width) / 20.0,
        )
        .unwrap()
    }

    type RawEdge = (Vec<Raw>, Vec<Raw>, Vec<Raw>);

    fn raw_edge() -> impl Strategy<Value = RawEdge> {
        (vec(raw(), 0..4), vec(raw(), 0..3), vec(raw(), 0..2))
    }

    fn parameters() -> impl Strategy<Value = Vec<(&'static str, ParameterValue)>> {
        (
            (1u8..30, 0u8..4, 0u8..4, 0u8..6),
            (0u8..20, 0u8..20, 0u8..6, 0u8..20, 0u8..6),
            any::<bool>(),
            (any::<bool>(), any::<bool>(), any::<bool>()),
        )
            .prop_map(
                |(
                    (height, barrier_gap, platform_gap, landing_gap),
                    (fall, width, distance, climb, side),
                    curb,
                    (barriers, landings, climbables),
                )| {
                    let number = |value: f64| ParameterValue::Number { value };
                    let mut parameters = vec![
                        (
                            "minimum_barrier_height_metres",
                            number(f64::from(height) / 20.0),
                        ),
                        (
                            "maximum_barrier_gap_metres",
                            number(f64::from(barrier_gap) / 20.0),
                        ),
                        (
                            "maximum_platform_gap_metres",
                            number(f64::from(platform_gap) / 20.0),
                        ),
                        (
                            "maximum_landing_gap_metres",
                            number(f64::from(landing_gap) / 10.0),
                        ),
                        ("maximum_fall_height_metres", number(f64::from(fall) / 20.0)),
                        (
                            "minimum_landing_width_metres",
                            number(f64::from(width) / 10.0),
                        ),
                        (
                            "climbable_barrier_distance_metres",
                            number(f64::from(distance) / 10.0),
                        ),
                        (
                            "maximum_climbable_height_metres",
                            number(f64::from(climb) / 20.0),
                        ),
                        (
                            "minimum_climbable_side_length_metres",
                            number(f64::from(side) / 20.0),
                        ),
                        (
                            "measure_barrier_from_curb",
                            ParameterValue::Boolean { value: curb },
                        ),
                    ];
                    if barriers {
                        parameters.push(("barrier_selector", entity("railing")));
                    }
                    if landings {
                        parameters.push(("landing_selector", entity("floor")));
                    }
                    if climbables {
                        parameters.push(("climbable_selector", entity("bench")));
                    }
                    parameters
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(160))]

        #[test]
        fn generated_edges_hold_parity(
            surfaces in vec(vec(raw_edge(), 0..4), 1..4),
            parameters in parameters(),
        ) {
            // Odd rails are not railings, so a role selection leaves them out.
            let mut objects: Vec<Object> = (0..4)
                .flat_map(|which| {
                    [
                        Object::new(
                            oid(&format!("rail-{which}")),
                            if which % 2 == 0 { "railing" } else { "wall" },
                        ),
                        Object::new(oid(&format!("floor-{which}")), "floor"),
                        Object::new(oid(&format!("bench-{which}")), "bench"),
                    ]
                })
                .collect();
            let mut edges = Vec::new();
            for (index, surface) in surfaces.iter().enumerate() {
                let id = format!("slab-{index}");
                objects.push(Object::new(oid(&id), "slab"));
                for (barriers, landings, climbables) in surface {
                    edges.push(GuardEdge::new(
                        oid(&id),
                        barriers.iter().copied().map(barrier).collect(),
                        landings.iter().copied().map(landing).collect(),
                        climbables.iter().copied().map(climbable).collect(),
                    ));
                }
            }
            let project = Project::new(objects).unwrap();
            let mut services = ServiceRegistry::new();
            services
                .register(GuardServiceHandle::new(Arc::new(MultiEdge(edges))))
                .unwrap();
            held(&project, &services, &rule_with(&parameters));
        }
    }
}
