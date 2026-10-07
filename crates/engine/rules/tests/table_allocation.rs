//! Exclusive allocation of objects to table rows, per-row counts and areas.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle, SessionSources,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{
    Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension, Scope, SourceId,
};
use axioval_rules::TableAllocation;

/// `table-allocation` as it runs, held to the implementation it replaced on
/// every evaluation.
static HELD: common::Held =
    common::Held(&TableAllocation, &axioval_rules::reference::TableAllocation);
use common::{
    Model, assert_deviation, deviation_of, findings, id, integer, kind, property, rule, selector,
    source, string, unevaluated,
};

const ID: &str = "axioval:capability.table-allocation";

/// Footprint areas in square metres, each with an uncertainty around it.
#[derive(Default)]
struct Areas(BTreeMap<ObjectId, (f64, f64)>);

impl Areas {
    fn with(mut self, local: &str, area: f64, slack: f64) -> Self {
        self.0.insert(id(local), (area, slack));
        self
    }
}

impl PlanAreaService for Areas {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (area, slack) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("footprint:{object}"));
        evidence.exact = slack == 0.0;
        PlanArea::try_new(area - slack, area + slack, evidence)
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not needed".into()))
    }
}

/// A row from `(column, cell)` pairs.
fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, cell)| ((*column).to_owned(), cell.clone()))
        .collect()
}

fn table(rows: Vec<TableRow>) -> ParameterValue {
    ParameterValue::Table { value: rows }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

/// Storey `a` holds `Office 1A` and `Office 2`; storey `b` holds `Lobby`
/// and `Office 3`. Space types are the `Pset.Type` property.
fn storeys() -> Model {
    Model::default()
        .object("a", "storey")
        .object("b", "storey")
        .object("a1", "space")
        .object("a2", "space")
        .object("b1", "space")
        .object("b2", "space")
        .edge("contains", "a", "a1")
        .edge("contains", "a", "a2")
        .edge("contains", "b", "b1")
        .edge("contains", "b", "b2")
        .text("a1", "Pset", "Type", "Office 1A")
        .text("a2", "Pset", "Type", "Office 2")
        .text("b1", "Pset", "Type", "Lobby")
        .text("b2", "Pset", "Type", "Office 3")
}

/// `Office*` twice and `Office 1*` once per storey.
fn per_storey(mode: &str) -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "rows",
            table(vec![
                row(&[("key_1", string("Office*")), ("count", integer(2))]),
                row(&[("key_1", string("Office 1*")), ("count", integer(1))]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("mode", string(mode)),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ]
}

#[test]
fn first_match_assigns_the_earliest_row_and_reports_the_extra_and_the_empty_row() {
    let evaluation = storeys().evaluate(&HELD, &rule(ID, kind("space"), per_storey("first")));
    assert_eq!(
        findings(&evaluation),
        [
            ("b1".into(), "no row matches (Pset.Type is `Lobby`)".into()),
            (
                "a".into(),
                "row 2 (Pset.Type like `Office 1*`) matched no object; required exactly 1".into()
            ),
            (
                "b".into(),
                "row 1 (Pset.Type like `Office*`) has 1 object(s); required exactly 2".into()
            ),
            (
                "b".into(),
                "row 2 (Pset.Type like `Office 1*`) matched no object; required exactly 1".into()
            ),
        ]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    assert_eq!(evaluation.findings()[2].related, [id("b2")]);
}

#[test]
fn most_specific_match_assigns_the_row_with_the_most_literal_characters() {
    let evaluation =
        storeys().evaluate(&HELD, &rule(ID, kind("space"), per_storey("most_specific")));
    assert_eq!(
        findings(&evaluation),
        [
            ("b1".into(), "no row matches (Pset.Type is `Lobby`)".into()),
            (
                "a".into(),
                "row 1 (Pset.Type like `Office*`) has 1 object(s); required exactly 2".into()
            ),
            (
                "b".into(),
                "row 1 (Pset.Type like `Office*`) has 1 object(s); required exactly 2".into()
            ),
            (
                "b".into(),
                "row 2 (Pset.Type like `Office 1*`) matched no object; required exactly 1".into()
            ),
        ]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_tie_leaves_the_object_and_the_rows_it_may_belong_to_not_evaluated() {
    // `Off*` and `*ice` both have three literal characters.
    let model = Model::default()
        .object("s1", "space")
        .object("s2", "space")
        .text("s1", "Pset", "Type", "Office")
        .text("s2", "Pset", "Type", "Office")
        .text("s2", "Pset", "Name", "Corner");
    let parameters = vec![
        (
            "rows",
            table(vec![
                row(&[("key_1", string("Off*")), ("count", integer(1))]),
                row(&[("key_1", string("*ice")), ("count", integer(0))]),
                row(&[
                    ("key_1", string("*ice")),
                    ("key_2", string("Corner")),
                    ("label", string("corner office")),
                    ("count", integer(1)),
                ]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("key_2", property(Some("Pset"), "Name")),
        ("mode", string("most_specific")),
    ];
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    // `s2` goes to the corner row; `s1` ties between rows 1 and 2.
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    let outcomes: Vec<_> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| (outcome.reason().clone(), outcome.message().to_owned()))
        .collect();
    assert_eq!(
        outcomes,
        [
            (
                NotEvaluatedReason::InvalidDeclaration,
                "row 1 (Pset.Type like `Off*`) and row 2 (Pset.Type like `*ice`) match equally specifically".into()
            ),
            (
                NotEvaluatedReason::IncompleteEvidence,
                "table-allocation: row 1 (Pset.Type like `Off*`) has 0 object(s) in source `test:model` and 1 more that may belong to it; required exactly 1".into()
            ),
            (
                NotEvaluatedReason::IncompleteEvidence,
                "table-allocation: row 2 (Pset.Type like `*ice`) has 0 object(s) in source `test:model` and 1 more that may belong to it; required exactly 0".into()
            ),
        ]
    );
}

#[test]
fn an_empty_row_without_a_count_is_found_against_the_source_and_a_zero_count_row_is_not() {
    let model = Model::default()
        .object("s1", "space")
        .text("s1", "Pset", "Type", "Office");
    let parameters = vec![
        (
            "rows",
            table(vec![
                row(&[("key_1", string("Office"))]),
                row(&[("key_1", string("Archive")), ("label", string("archive"))]),
                row(&[("key_1", string("Storage")), ("count", integer(0))]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
    ];
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    assert_eq!(
        findings(&evaluation),
        [(
            "source".into(),
            "row 2 `archive` matched no object in source `test:model`".into()
        )]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_source_without_objects_is_a_group_whose_rows_are_reported() {
    // `other` is in the session but contributes no object.
    let other = SourceId::new("test", "other").unwrap();
    let model = Model::default()
        .object("s1", "space")
        .text("s1", "Pset", "Type", "Office");
    let parameters = vec![
        (
            "rows",
            table(vec![
                row(&[
                    ("key_1", string("Office")),
                    ("label", string("office")),
                    ("count", integer(1)),
                ]),
                row(&[("key_1", string("Storage")), ("count", integer(0))]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
    ];
    let evaluation = model.evaluate_with(&HELD, &rule(ID, kind("space"), parameters), |services| {
        services
            .register(SessionSources::new([source(), other.clone()]))
            .unwrap();
    });
    assert_eq!(
        findings(&evaluation),
        [(
            "source".into(),
            "row 1 `office` matched no object in source `test:other`; required exactly 1".into()
        )]
    );
    assert_eq!(evaluation.findings()[0].scope, Scope::Source(other));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_unreadable_or_absent_key_decides_nothing_or_matches_no_pattern() {
    let model = Model::default()
        .object("s1", "space")
        .object("s2", "space")
        .object("s3", "space")
        .text("s1", "Pset", "Type", "Office")
        .unreadable("s2");
    let parameters = vec![
        (
            "rows",
            table(vec![row(&[("key_1", string("*")), ("count", integer(2))])]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
    ];
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    // `s3` states no type: no pattern matches it, not even `*`.
    assert_eq!(
        findings(&evaluation),
        [("s3".into(), "no row matches (Pset.Type is absent)".into())]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("s2".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("-".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

fn area_rows() -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "rows",
            table(vec![row(&[
                ("key_1", string("Office*")),
                ("area", number(30.0)),
                ("area_tolerance", number(1.0)),
            ])]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ]
}

fn run(
    model: Model,
    areas: Areas,
    parameters: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CapabilityEvaluation {
    model.evaluate_with(&HELD, &rule(ID, kind("space"), parameters), |services| {
        services
            .register(PlanAreaServiceHandle::new(Arc::new(areas)))
            .unwrap();
    })
}

#[test]
fn the_summed_area_of_a_row_lies_within_the_tolerance() {
    // Storey `a`: 20 + 12 = 32 m², too much. Storey `b` holds only the
    // office `b2` of 30.5 m², within 30 ± 1; `b1` is an extra.
    let areas = Areas::default()
        .with("a1", 20.0, 0.0)
        .with("a2", 12.0, 0.0)
        .with("b1", 5.0, 0.0)
        .with("b2", 30.5, 0.0);
    let evaluation = run(storeys(), areas, area_rows());
    assert_eq!(
        findings(&evaluation),
        [
            ("b1".into(), "no row matches (Pset.Type is `Lobby`)".into()),
            (
                "a".into(),
                "row 1 (Pset.Type like `Office*`) sums 32 m²; required 30 ± 1 m²".into()
            ),
        ]
    );
    assert_eq!(evaluation.findings()[1].related, [id("a1"), id("a2")]);
    // 32 m² against at most 31 m².
    assert_deviation(deviation_of(&evaluation, "row 1"), (1.0 / 31.0, 1.0 / 31.0));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

/// The measured list `allocations` cites each row's area exactly as its
/// footprints: summed over a tessellated one, the area finding is
/// inexact, while the row's count, which reads no area, stays exact.
#[test]
fn an_area_summed_over_tessellated_footprints_is_cited_inexactly() {
    let areas = Areas::default()
        .with("a1", 20.0, 0.25)
        .with("a2", 12.0, 0.0)
        .with("b2", 30.5, 0.0);
    let mut parameters = area_rows();
    let Some((_, ParameterValue::Table { value: rows })) = parameters.first_mut() else {
        unreachable!("the rows come first");
    };
    rows[0].insert("count".into(), integer(1));
    let evaluation = run(storeys(), areas, parameters);
    let on_a: Vec<&axioval_ir::Finding> = evaluation
        .findings()
        .iter()
        .filter(|finding| finding.scope == axioval_ir::Scope::Object(id("a")))
        .collect();
    assert_eq!(on_a.len(), 2, "{evaluation:#?}");
    let (count, area): (Vec<_>, Vec<_>) = on_a
        .into_iter()
        .partition(|finding| finding.message.contains("object(s)"));
    assert!(count[0].evidence.iter().all(|item| item.exact));
    assert!(area[0].evidence.iter().any(|item| !item.exact));
}

#[test]
fn a_summed_area_straddling_the_tolerance_is_not_evaluated() {
    let areas = Areas::default()
        .with("a1", 20.0, 0.5)
        .with("a2", 11.0, 0.5)
        .with("b2", 30.0, 0.0);
    let evaluation = run(storeys(), areas, area_rows());
    assert_eq!(findings(&evaluation).len(), 1, "only the extra");
    assert_eq!(
        unevaluated(&evaluation),
        [("a".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn stated_areas_replace_measured_footprints() {
    let area = |value| PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    };
    let model = storeys()
        .value("a1", "Qto", "NetArea", area(15.0))
        .value("a2", "Qto", "NetArea", area(15.0))
        .value("b2", "Qto", "NetArea", area(10.0));
    let mut parameters = area_rows();
    parameters.push(("area_property", property(Some("Qto"), "NetArea")));
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    assert_eq!(
        findings(&evaluation)[1..],
        [(
            "b".into(),
            "row 1 (Pset.Type like `Office*`) sums 10 m²; required 30 ± 1 m²".into()
        )]
    );
}

#[test]
fn a_space_no_storey_contains_is_not_evaluated() {
    let model = storeys()
        .object("loose", "space")
        .text("loose", "Pset", "Type", "Office 9");
    let parameters = vec![
        ("rows", table(vec![row(&[("key_1", string("*"))])])),
        ("key_1", property(Some("Pset"), "Type")),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ];
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("loose".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_key_cell_without_its_property_is_an_invalid_declaration() {
    let parameters = vec![("rows", table(vec![row(&[("key_2", string("Office"))])]))];
    let evaluation = storeys().evaluate(&HELD, &rule(ID, kind("space"), parameters));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "table-allocation: row 1 fills `key_2`, but no `key_2` property is declared"
    );
}

/// `storeys` with storey `a` named `EG` and `b` named `OG`.
fn named_storeys() -> Model {
    storeys()
        .text("a", "Pset", "Name", "EG")
        .text("b", "Pset", "Name", "OG")
}

/// Offices per storey: `eg` in the ground storey, `og` in the upper one.
fn per_named_storey(eg: i64, og: i64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "rows",
            table(vec![
                row(&[
                    ("anchor", string("EG")),
                    ("key_1", string("Office*")),
                    ("count", integer(eg)),
                ]),
                row(&[
                    ("anchor", string("OG")),
                    ("key_1", string("Office*")),
                    ("count", integer(og)),
                ]),
                row(&[("anchor", string("OG")), ("key_1", string("Lobby"))]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("anchor_key", property(Some("Pset"), "Name")),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ]
}

#[test]
fn one_rule_states_different_rows_per_anchor() {
    // Two offices in `EG`, one in `OG`: both rows hold.
    let evaluation =
        named_storeys().evaluate(&HELD, &rule(ID, kind("space"), per_named_storey(2, 1)));
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    // Two offices required in `OG` as well: only `OG` is short, and the
    // `EG` row is never judged there.
    let evaluation =
        named_storeys().evaluate(&HELD, &rule(ID, kind("space"), per_named_storey(2, 2)));
    assert_eq!(
        findings(&evaluation),
        [(
            "b".into(),
            "row 2 (Pset.Type like `Office*`) in anchors like `OG` has 1 object(s); required \
             exactly 2"
                .into()
        )]
    );
}

#[test]
fn an_anchor_whose_key_cannot_be_read_is_not_evaluated() {
    let model = storeys().text("a", "Pset", "Name", "EG").value(
        "b",
        "Pset",
        "Name",
        PropertyValue::Integer(1),
    );
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), per_named_storey(2, 1)));
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("b".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_fourth_key_narrows_a_row() {
    let model = storeys()
        .text("a1", "Pset", "Number", "1.01")
        .text("a2", "Pset", "Number", "2.01");
    let parameters = vec![
        (
            "rows",
            table(vec![row(&[
                ("key_1", string("Office*")),
                ("key_4", string("1.*")),
            ])]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("key_4", property(Some("Pset"), "Number")),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ];
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("space"), parameters));
    let extras: Vec<String> = findings(&evaluation)
        .into_iter()
        .filter(|(_, message)| message.starts_with("no row matches"))
        .map(|(object, _)| object)
        .collect();
    // `b2` states no number, so it matches no pattern.
    assert_eq!(extras, ["a2", "b1", "b2"]);
}

/// Offices of 12 m² ± 10 %, each on its own.
fn office_sizes(mode: &str) -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "rows",
            table(vec![
                row(&[
                    ("key_1", string("Office*")),
                    ("area", number(12.0)),
                    ("area_tolerance_ratio", number(0.1)),
                ]),
                row(&[("key_1", string("Lobby")), ("count", integer(1))]),
            ]),
        ),
        ("key_1", property(Some("Pset"), "Type")),
        ("area_mode", string(mode)),
        ("anchor_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
    ]
}

#[test]
fn a_row_of_relative_tolerance_takes_each_object_within_it() {
    // 12 m² ± 10 % takes 11 m² but not 10 m²; 10.8 ± 0.1 straddles 10.8.
    let areas = Areas::default()
        .with("a1", 11.0, 0.0)
        .with("a2", 10.0, 0.0)
        .with("b1", 40.0, 0.0)
        .with("b2", 10.8, 0.1);
    let evaluation = run(storeys(), areas, office_sizes("each"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "a2".into(),
                "no row matches (Pset.Type is `Office 2`)".into()
            ),
            (
                "a".into(),
                "row 2 (Pset.Type like `Lobby`) matched no object; required exactly 1".into()
            ),
        ]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("b2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles 12 m² ± 10 %"),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );

    // Summed, the same row is judged as 12 m² ± 10 % over both offices.
    let areas = Areas::default()
        .with("a1", 6.0, 0.0)
        .with("a2", 5.0, 0.0)
        .with("b1", 40.0, 0.0)
        .with("b2", 20.0, 0.0);
    let evaluation = run(storeys(), areas, office_sizes("sum"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "a".into(),
                "row 2 (Pset.Type like `Lobby`) matched no object; required exactly 1".into()
            ),
            (
                "b".into(),
                "row 1 (Pset.Type like `Office*`) sums 20 m²; required 12 m² ± 10 %".into()
            ),
        ]
    );
}

#[test]
fn malformed_anchors_tolerances_and_modes_are_invalid_declarations() {
    let base = || {
        vec![
            ("key_1", property(Some("Pset"), "Type")),
            ("anchor_selector", selector(kind("storey"))),
            ("relationship", string("contains")),
        ]
    };
    let cases: Vec<Vec<(&'static str, ParameterValue)>> = vec![
        vec![("rows", table(vec![row(&[("anchor", string("EG"))])]))],
        vec![
            ("rows", table(vec![row(&[("key_1", string("*"))])])),
            ("anchor_key", property(Some("Pset"), "Name")),
        ],
        vec![(
            "rows",
            table(vec![row(&[
                ("area", number(12.0)),
                ("area_tolerance", number(1.0)),
                ("area_tolerance_ratio", number(0.1)),
            ])]),
        )],
        vec![
            ("rows", table(vec![row(&[("key_1", string("*"))])])),
            ("area_mode", string("average")),
        ],
    ];
    for extra in cases {
        let mut parameters = base();
        parameters.extend(extra);
        let evaluation = storeys().evaluate(&HELD, &rule(ID, kind("space"), parameters.clone()));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{parameters:?}"
        );
    }
}

/// Generated storeys of spaces whose types and areas are random, some
/// unreadable, under random rows (patterns, counts, areas and tolerances),
/// modes and groupings, held to the reference.
mod generated {
    use super::*;
    use proptest::prelude::*;

    const TYPES: [&str; 4] = ["Office 1A", "Office 2", "Lobby", "Store"];
    const PATTERNS: [&str; 5] = ["Office*", "Office 1*", "*", "Lobby", "St*"];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_allocations_hold_parity(
            spaces in proptest::collection::vec(
                (0..TYPES.len(), 0.0..40.0f64, prop_oneof![Just(0.0), 0.0..2.0f64], 0..2usize, 0..6u8),
                0..6,
            ),
            rows in proptest::collection::vec(
                (
                    proptest::option::of(0..PATTERNS.len()),
                    proptest::option::of(0..4i64),
                    proptest::option::of((0.0..80.0f64, 0..3u8, 0.0..5.0f64)),
                ),
                1..4,
            ),
            specific in any::<bool>(),
            grouping in 0..3u8,
            each in any::<bool>(),
        ) {
            let mut model = Model::default().object("a", "storey").object("b", "storey");
            let mut areas = Areas::default();
            for (index, (kind, area, slack, storey, fate)) in spaces.iter().enumerate() {
                let local = format!("s{index}");
                model = model
                    .object(&local, "space")
                    .edge("contains", ["a", "b"][*storey], &local);
                // Fate 0 leaves the type unreadable, 1 absent; 2 leaves the
                // area unmeasured.
                model = match fate {
                    0 => model.unreadable_value(&local, "Pset", "Type", "string"),
                    1 => model,
                    _ => model.text(&local, "Pset", "Type", TYPES[*kind]),
                };
                if *fate != 2 {
                    areas = areas.with(&local, *area, *slack);
                }
            }
            let rows: Vec<TableRow> = rows
                .iter()
                .map(|(pattern, count, area)| {
                    let mut cells = Vec::new();
                    if let Some(pattern) = pattern {
                        cells.push(("key_1", string(PATTERNS[*pattern])));
                    }
                    if let Some(count) = count {
                        cells.push(("count", integer(*count)));
                    }
                    if let Some((area, tolerance, value)) = area {
                        cells.push(("area", number(*area)));
                        match tolerance {
                            0 => {}
                            1 => cells.push(("area_tolerance", number(*value))),
                            _ => cells.push(("area_tolerance_ratio", number(value / 10.0))),
                        }
                    }
                    row(&cells)
                })
                .collect();
            let mut parameters = vec![
                ("rows", table(rows)),
                ("key_1", property(Some("Pset"), "Type")),
                ("mode", string(if specific { "most_specific" } else { "first" })),
                ("area_mode", string(if each { "each" } else { "sum" })),
            ];
            match grouping {
                0 => {}
                1 => parameters.push(("across_sources", ParameterValue::Boolean { value: true })),
                _ => {
                    parameters.push(("anchor_selector", selector(kind("storey"))));
                    parameters.push(("relationship", string("contains")));
                }
            }
            model.evaluate_with(&HELD, &rule(ID, kind("space"), parameters), |services| {
                services
                    .register(PlanAreaServiceHandle::new(std::sync::Arc::new(areas)))
                    .unwrap();
            });
        }
    }
}
