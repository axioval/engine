//! Clash matrix contract tests: each pair is judged with the tolerance
//! profile and severity of its most specific cell.
//!
//! The proximity stub answers only the pairs a test declares and panics on
//! any other, which also proves that unmatched and switched-off pairs are
//! never measured.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, GeometryFidelity, LengthInterval, NotEvaluatedReason,
    ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, ObjectId, Severity};
use axioval_rules::ClashMatrix;

use common::{
    Model, boolean, findings, kind, number, property, rule, selector, source, string, strings,
    unevaluated,
};

const ID: &str = "axioval:capability.clash-matrix";

/// Unit boxes along x, and the penetration depth of each declared pair.
#[derive(Default)]
struct Stub {
    boxes: BTreeMap<String, f64>,
    depths: BTreeMap<(String, String), f64>,
}

impl Stub {
    fn object(mut self, local: &str, x: f64) -> Self {
        self.boxes.insert(local.into(), x);
        self
    }
    fn overlap(mut self, a: &str, b: &str, depth: f64) -> Self {
        self.depths.insert((a.into(), b.into()), depth);
        self
    }
}

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let x = *self
            .boxes
            .get(&object.local_id)
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let depth = *self
            .depths
            .get(&(a.clone(), b.clone()))
            .or_else(|| self.depths.get(&(b.clone(), a.clone())))
            .unwrap_or_else(|| panic!("{a}/{b} should not have been measured"));
        ProximityEvidence::try_new(
            request.clone(),
            0.0,
            Some(depth),
            0.0,
            None,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("proximity:{a}:{b}")),
        )?
        .with_hausdorff(LengthInterval::try_new(1.0, 1.0).unwrap())
    }
}

/// A duct, a pipe and a beam. The beam overlaps both others; the duct and
/// the pipe are apart. The duct and the pipe carry their trade.
fn model() -> Model {
    Model::default()
        .object("beam", "beam")
        .object("duct", "duct")
        .object("pipe", "pipe")
        .text("duct", "Pset", "Trade", "HVAC")
        .text("pipe", "Pset", "Trade", "Plumbing")
}

fn geometry() -> Stub {
    Stub::default()
        .object("beam", 0.5)
        .object("duct", 0.0)
        .object("pipe", 1.2)
        .overlap("beam", "duct", 0.03)
        .overlap("beam", "pipe", 0.03)
}

/// A cell: `(column, value)` pairs.
fn cell(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect()
}

/// A cell keying the subject by trade and the counterpart as a beam.
fn trade_by_beam(trade: &str, tolerance: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("subject_key_1", string(trade)),
        ("counterpart_selector", selector(kind("beam"))),
        ("penetration_tolerance_metres", number(tolerance)),
    ]
}

/// Every object against every other, keyed by trade, same-system exclusion
/// switched off unless `extra` declares it.
fn matrix(
    cells: Vec<TableRow>,
    extra: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CompiledRule {
    let mut parameters = vec![
        ("counterparts", selector(Selector::All)),
        ("cells", ParameterValue::Table { value: cells }),
        ("key_1", property(Some("Pset"), "Trade")),
        ("exclude_same_system", boolean(false)),
    ];
    for (name, value) in extra {
        parameters.retain(|(existing, _)| *existing != name);
        parameters.push((name, value));
    }
    rule(ID, Selector::All, parameters)
}

fn run(model: Model, stub: Stub, rule: &axioval_engine::CompiledRule) -> CapabilityEvaluation {
    model.evaluate_with(&ClashMatrix, rule, |services| {
        services
            .register(ProximityServiceHandle::new(Arc::new(stub)))
            .unwrap();
    })
}

#[test]
fn two_category_pairs_are_judged_with_their_own_tolerances_in_one_run() {
    let mut plumbing = trade_by_beam("Plumbing", 0.01);
    plumbing.extend([
        ("label", string("plumbing x structure")),
        ("severity", string("warning")),
    ]);
    let rule = matrix(
        vec![cell(&trade_by_beam("HVAC", 0.05)), cell(&plumbing)],
        vec![],
    );
    let evaluation = run(model(), geometry(), &rule);
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );
    // Both pairs penetrate 0.03 m: within the duct cell's 0.05 m, past the
    // pipe cell's 0.01 m. The beam is the lesser identity, so the pair is
    // reported on it, and the symmetric cell covers it all the same.
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", findings(&evaluation));
    };
    assert_eq!(finding.related, vec![common::id("pipe")]);
    assert_eq!(finding.severity, Severity::Warning);
    assert!(
        finding.message.starts_with(
            "hard clash with test:model/pipe: penetration 0.0300 m exceeds tolerance 0.0100 m"
        ),
        "{}",
        finding.message
    );
    assert!(
        finding
            .message
            .ends_with("(clash matrix cell 1 `plumbing x structure`)"),
        "{}",
        finding.message
    );
    // The trade read to pick the cell is evidence too.
    assert!(finding.evidence.len() > 1, "{:?}", finding.evidence);
}

#[test]
fn a_cell_without_its_own_severity_takes_the_rules() {
    let rule = matrix(vec![cell(&trade_by_beam("*", 0.01))], vec![]);
    let evaluation = run(model(), geometry(), &rule);
    let severities: Vec<_> = evaluation
        .findings()
        .iter()
        .map(|finding| finding.severity.clone())
        .collect();
    assert_eq!(severities, [Severity::Error, Severity::Error]);
}

#[test]
fn the_most_specific_cell_wins_and_a_tie_is_not_evaluated() {
    // `HV*` and `*AC` both key the trade with two literals: a tie for the
    // duct, never broken by declaration order.
    let tied = vec![
        cell(&trade_by_beam("HV*", 0.05)),
        cell(&trade_by_beam("*AC", 0.01)),
    ];
    let evaluation = run(
        model(),
        Stub::default()
            .object("beam", 0.5)
            .object("duct", 0.0)
            .object("pipe", 10.0)
            .overlap("beam", "duct", 0.03),
        &matrix(tied.clone(), vec![]),
    );
    assert!(evaluation.findings().is_empty());
    let [outcome] = evaluation.not_evaluated_outcomes() else {
        panic!("one open pair expected: {:?}", unevaluated(&evaluation));
    };
    assert_eq!(outcome.reason(), &NotEvaluatedReason::InvalidDeclaration);
    assert!(
        outcome.message()
            == "clash matrix cell 0 and cell 1 cover the pair with test:model/duct equally",
        "{}",
        outcome.message()
    );

    // A cell with more literals is more specific and decides.
    let mut decided = tied;
    decided.push(cell(&trade_by_beam("HVAC", 0.01)));
    let evaluation = run(
        model(),
        Stub::default()
            .object("beam", 0.5)
            .object("duct", 0.0)
            .object("pipe", 10.0)
            .overlap("beam", "duct", 0.03),
        &matrix(decided, vec![]),
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", findings(&evaluation));
    };
    assert!(
        finding.message.ends_with("(clash matrix cell 2)"),
        "{}",
        finding.message
    );

    // Keying one more category outweighs any number of literals: a blank
    // trade with a beam counterpart loses to `*` with one.
    let evaluation = run(
        model(),
        Stub::default()
            .object("beam", 0.5)
            .object("duct", 0.0)
            .object("pipe", 10.0)
            .overlap("beam", "duct", 0.03),
        &matrix(
            vec![
                cell(&[
                    ("subject_key_1", string("HVAC")),
                    ("penetration_tolerance_metres", number(0.01)),
                ]),
                cell(&trade_by_beam("*", 0.05)),
            ],
            vec![],
        ),
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_unmatched_pair_is_ignored_unless_reported() {
    // Only the plumbing cell exists: the duct/beam pair is not covered.
    let cells = vec![cell(&trade_by_beam("Plumbing", 0.05))];
    let evaluation = run(model(), geometry(), &matrix(cells.clone(), vec![]));
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    let evaluation = run(
        model(),
        geometry(),
        &matrix(cells, vec![("report_unmatched", boolean(true))]),
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", findings(&evaluation));
    };
    assert_eq!(finding.related, vec![common::id("duct")]);
    assert_eq!(
        finding.message,
        "no clash matrix cell covers test:model/beam (Pset.Trade absent) against \
         test:model/duct (Pset.Trade `HVAC`)"
    );
}

#[test]
fn an_ordered_matrix_covers_a_pair_only_the_declared_way_round() {
    // The pair is oriented beam -> pipe; the cell keys the pipe as subject.
    let cells = vec![cell(&trade_by_beam("Plumbing", 0.01))];
    let evaluation = run(
        model(),
        geometry(),
        &matrix(
            cells,
            vec![
                ("symmetric", boolean(false)),
                ("report_unmatched", boolean(true)),
            ],
        ),
    );
    let messages: Vec<String> = findings(&evaluation)
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages
            .iter()
            .all(|message| message.starts_with("no clash matrix cell covers")),
        "{messages:?}"
    );
}

#[test]
fn a_switched_off_cell_covers_its_pairs_without_measuring_them() {
    let mut off = trade_by_beam("HVAC", 0.0);
    off.extend([
        ("report_duplicates", boolean(false)),
        ("report_containment", boolean(false)),
        ("report_intersections", boolean(false)),
    ]);
    let evaluation = run(
        model(),
        Stub::default()
            .object("beam", 0.5)
            .object("duct", 0.0)
            .object("pipe", 10.0),
        &matrix(vec![cell(&off)], vec![("report_unmatched", boolean(true))]),
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_unreadable_category_leaves_the_pair_open() {
    let evaluation = run(
        model().unreadable("duct"),
        geometry(),
        &matrix(vec![cell(&trade_by_beam("HVAC", 0.05))], vec![]),
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    let open = unevaluated(&evaluation);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].0, "beam");
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("clash matrix cell for the pair with test:model/duct cannot be chosen"),
        "{}",
        evaluation.not_evaluated_outcomes()[0].message()
    );
}

#[test]
fn a_discipline_outside_a_session_is_unknown_never_a_non_match() {
    let evaluation = run(
        model(),
        geometry(),
        &matrix(
            vec![cell(&[
                ("subject_discipline", string("*")),
                ("penetration_tolerance_metres", number(0.0)),
            ])],
            vec![],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::MissingService),
        "{:?}",
        unevaluated(&evaluation)
    );
    assert!(!evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn same_system_exclusion_is_on_by_default_and_needs_its_path() {
    let cells = vec![cell(&trade_by_beam("*", 0.01))];
    let mut rule = matrix(cells.clone(), vec![]);
    rule.parameters.remove("exclude_same_system");
    let evaluation = run(model(), geometry(), &rule);
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration),
        "{:?}",
        unevaluated(&evaluation)
    );

    // The beam and the pipe share a system: only the duct pair is checked.
    let mut rule = matrix(cells, vec![("system_path", string("groups:backward"))]);
    rule.parameters.remove("exclude_same_system");
    let evaluation = run(
        model()
            .object("system", "system")
            .edge("groups", "system", "beam")
            .edge("groups", "system", "pipe"),
        geometry().object("system", 50.0),
        &rule,
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", findings(&evaluation));
    };
    assert_eq!(finding.related, vec![common::id("duct")]);
}

#[test]
fn invalid_cells_refuse_the_rule() {
    for cells in [
        vec![],
        // A key cell without its key property.
        vec![cell(&[
            ("subject_key_2", string("HVAC")),
            ("penetration_tolerance_metres", number(0.0)),
        ])],
        vec![cell(&[
            ("severity", string("fatal")),
            ("penetration_tolerance_metres", number(0.0)),
        ])],
        vec![cell(&[("penetration_tolerance_metres", number(-1.0))])],
    ] {
        let evaluation = run(model(), geometry(), &matrix(cells, vec![]));
        assert!(evaluation.findings().is_empty());
        assert!(
            !evaluation.not_evaluated_outcomes().is_empty()
                && unevaluated(&evaluation)
                    .iter()
                    .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration),
            "{:?}",
            unevaluated(&evaluation)
        );
    }
    let evaluation = run(
        model(),
        geometry(),
        &matrix(
            vec![cell(&trade_by_beam("*", 0.0))],
            vec![("exclude_paths", strings(&[" "]))],
        ),
    );
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration)
    );
}
