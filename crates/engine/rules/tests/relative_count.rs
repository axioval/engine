//! relative-count: grouping by a property value and the small-count case.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::RelativeCount;
use common::{
    Model, boolean, findings, flagged, integer, kind, property, rule, selector, string, unevaluated,
};

const ID: &str = "axioval:capability.relative-count";

/// Workplaces and washbasins, each named `<group>-w<n>` / `<group>-b<n>` and
/// carrying `Loc.Code` = the given code.
fn located(groups: &[(&str, &str, usize, usize)]) -> Model {
    let mut model = Model::default();
    for (name, code, workplaces, basins) in groups {
        for (count, letter, what) in [(workplaces, "w", "workplace"), (basins, "b", "washbasin")] {
            for index in 0..*count {
                let id = format!("{name}-{letter}{index}");
                model = model.object(&id, what).text(&id, "Loc", "Code", code);
            }
        }
    }
    model
}

/// One washbasin per four workplaces, at least, grouped by `Loc.Code`.
fn by_code(extra: Vec<(&str, ParameterValue)>) -> Vec<(&str, ParameterValue)> {
    let mut parameters = vec![
        ("provided_selector", selector(kind("washbasin"))),
        ("required_selector", selector(kind("workplace"))),
        ("provided_unit", integer(1)),
        ("required_unit", integer(4)),
        ("operator", string("at_least")),
        ("group_property", property(Some("Loc"), "Code")),
    ];
    parameters.extend(extra);
    parameters
}

fn check(
    model: Model,
    parameters: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CapabilityEvaluation {
    model.evaluate(&RelativeCount, &rule(ID, Selector::All, parameters))
}

#[test]
fn groups_are_formed_by_property_value() {
    // `a` and `a2` share the code once trimmed and folded.
    let evaluation = check(
        located(&[("a", "A", 3, 1), ("a2", " a ", 1, 0), ("b", "B", 5, 1)]),
        by_code(vec![]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "b-w0".into(),
            "group Loc.Code `B`: 1 provided and 5 required object(s); required 1/1 at_least 5/4"
                .into()
        )]
    );
    // The other five members, beside the object the finding is raised against.
    assert_eq!(evaluation.findings()[0].related.len(), 5);
}

#[test]
fn case_sensitive_codes_form_separate_groups() {
    let evaluation = check(
        located(&[("a", "A", 4, 1), ("a2", "a", 1, 0)]),
        by_code(vec![("case_sensitive", boolean(true))]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "a2-w0".into(),
            "group Loc.Code `a`: 1 required object(s) and no provided object; \
             the group is present only in the required set"
                .into()
        )]
    );
}

#[test]
fn a_group_only_in_the_required_set_is_reported_whatever_the_ratio() {
    // `at_most` would hold for no washbasins, yet the group has no counterpart.
    let evaluation = check(
        located(&[("c", "C", 2, 0), ("d", "D", 4, 1)]),
        by_code(vec![("operator", string("at_most"))]),
    );
    assert_eq!(flagged(&evaluation), ["c-w0"]);
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("present only in the required set")
    );
}

#[test]
fn a_counted_object_without_a_group_value_is_reported() {
    let evaluation = check(
        located(&[("a", "A", 4, 1), ("x", "  ", 1, 0)]),
        by_code(vec![]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "x-w0".into(),
            "Loc.Code is `  `, so the object counts in no group".into()
        )]
    );
}

#[test]
fn an_unreadable_group_value_leaves_every_group_of_its_scope_unevaluated() {
    let evaluation = check(
        located(&[("a", "A", 4, 1), ("b", "B", 8, 0), ("u", "U", 1, 0)]).unreadable("u-w0"),
        by_code(vec![]),
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("u-w0".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("a-w0".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("b-w0".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn the_rule_selection_bounds_the_counted_objects() {
    // Selecting everything, both groups fall short; selecting only `A`
    // objects leaves the `B` shortage out of scope.
    let evaluation = check(
        located(&[("a", "A", 5, 1), ("b", "B", 8, 0)]),
        by_code(vec![]),
    );
    assert_eq!(flagged(&evaluation), ["a-w0", "b-w0"]);
    let only_a = located(&[("a", "A", 5, 1), ("b", "B", 8, 0)]).evaluate(
        &RelativeCount,
        &rule(
            ID,
            Selector::Property {
                property_set: Some("Loc".into()),
                property: "Code".into(),
                operator: ComparisonOperator::Equals,
                value: Some(string("A")),
                case_sensitive: true,
                trim: false,
            },
            by_code(vec![]),
        ),
    );
    assert_eq!(flagged(&only_a), ["a-w0"]);
}

#[test]
fn grouping_parameters_are_checked() {
    for parameters in [
        by_code(vec![("relationship", string("contains"))]),
        vec![
            ("provided_selector", selector(kind("washbasin"))),
            ("required_selector", selector(kind("workplace"))),
            ("provided_unit", integer(1)),
            ("required_unit", integer(4)),
            ("operator", string("at_least")),
            ("across_sources", boolean(true)),
        ],
    ] {
        let evaluation = check(located(&[("a", "A", 1, 1)]), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn small_required_counts_use_the_declared_minimum() {
    // With fewer than four workplaces, at least two washbasins, where the
    // ratio alone would ask for one; four workplaces and zero workplaces are
    // judged by the ratio.
    let evaluation = check(
        located(&[
            ("a", "A", 3, 1),
            ("b", "B", 1, 2),
            ("c", "C", 4, 1),
            ("d", "D", 0, 1),
        ]),
        by_code(vec![
            ("small_required_below", integer(4)),
            ("small_provided", integer(2)),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "a-w0".into(),
            "group Loc.Code `A`: 1 provided and 3 required object(s); \
             required 1 at_least 2 for fewer than 4 required"
                .into()
        )]
    );
    // One per five, but below ten workplaces nothing is required: nine
    // workplaces pass with the one washbasin the ratio would find short.
    let exempt = check(
        located(&[("a", "A", 9, 1), ("b", "B", 12, 1)]),
        by_code(vec![
            ("required_unit", integer(5)),
            ("small_required_below", integer(10)),
            ("small_provided", integer(0)),
        ]),
    );
    assert_eq!(
        findings(&exempt),
        [(
            "b-w0".into(),
            "group Loc.Code `B`: 1 provided and 12 required object(s); required 1/1 at_least 12/5"
                .into()
        )]
    );
    // The exception is judged with the declared operator.
    let at_most = check(
        located(&[("a", "A", 2, 2)]),
        by_code(vec![
            ("operator", string("at_most")),
            ("small_required_below", integer(4)),
            ("small_provided", integer(1)),
        ]),
    );
    assert_eq!(
        findings(&at_most),
        [(
            "a-w0".into(),
            "group Loc.Code `A`: 2 provided and 2 required object(s); \
             required 2 at_most 1 for fewer than 4 required"
                .into()
        )]
    );
}

#[test]
fn small_count_parameters_go_together() {
    for extra in [
        vec![("small_required_below", integer(4))],
        vec![("small_provided", integer(1))],
        vec![
            ("small_required_below", integer(0)),
            ("small_provided", integer(1)),
        ],
        vec![
            ("small_required_below", integer(4)),
            ("small_provided", integer(-1)),
        ],
    ] {
        let evaluation = check(located(&[("a", "A", 1, 1)]), by_code(extra));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
