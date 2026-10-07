//! Required members per group, allocated by a maximum matching.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_rules::GroupComposition;

/// `group-composition` as it runs, held to the implementation it replaced
/// on every evaluation.
static HELD: common::Held = common::Held(
    &GroupComposition,
    &axioval_rules::reference::GroupComposition,
);
use common::{Model, findings, id, integer, kind, property, rule, selector, string, unevaluated};

const ID: &str = "axioval:capability.group-composition";

/// A row from `(column, cell)` pairs.
fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, cell)| ((*column).to_owned(), cell.clone()))
        .collect()
}

/// An entry `label` taking `count` members whose type is like `pattern`.
fn entry(label: &str, pattern: &str, count: i64) -> TableRow {
    row(&[
        ("label", string(label)),
        ("key_1", string(pattern)),
        ("count", integer(count)),
    ])
}

fn requirements(rows: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("requirements", ParameterValue::Table { value: rows }),
        ("key_1", property(Some("Pset"), "Type")),
        ("member_selector", selector(kind("space"))),
        ("relationship", string("assigns")),
    ]
}

/// Apartment `a` is assigned spaces of the given types, `a1`, `a2`, ….
fn apartment(types: &[&str]) -> Model {
    let mut model = Model::default().object("a", "zone");
    for (index, space_type) in types.iter().enumerate() {
        let local = format!("a{}", index + 1);
        model = model
            .object(&local, "space")
            .edge("assigns", "a", &local)
            .text(&local, "Pset", "Type", space_type);
    }
    model
}

#[test]
fn a_member_fitting_two_entries_goes_where_the_other_member_cannot() {
    // Taking members in order and each into its first fitting entry puts
    // the bedroom `a1` into `room` and leaves the living room `a2` nowhere,
    // since it is no bedroom. The maximum matching fills both.
    let parameters = requirements(vec![entry("room", "*room", 1), entry("bedroom", "Bed*", 1)]);
    let evaluation =
        apartment(&["Bedroom", "Living room"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn missing_and_surplus_members_are_reported_per_entry() {
    let parameters = requirements(vec![
        entry("bedroom", "Bed*", 2),
        entry("kitchen", "Kitchen", 1),
        entry("bathroom", "Bath*", 1),
    ]);
    let evaluation = apartment(&["Bedroom", "Bedroom", "Kitchen", "Lobby", "Bedroom"])
        .evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "a".into(),
                "row 3 `bathroom` has 0 of 1 required member(s) via assigns; 1 missing".into()
            ),
            (
                "a".into(),
                "row 1 `bedroom` takes 2 member(s), but 3 fit via assigns; 1 surplus".into()
            ),
            (
                "a".into(),
                "surplus member via assigns: no entry fits it (Pset.Type is `Lobby`)".into()
            ),
        ]
    );
    assert_eq!(
        evaluation.findings()[1].related,
        [id("a1"), id("a2"), id("a5")]
    );
    assert_eq!(evaluation.findings()[2].related, [id("a4")]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn entries_competing_for_one_member_miss_it_together() {
    // The bedroom fits both entries; which one stays empty is no fact of
    // the model, so neither is blamed alone.
    let parameters = requirements(vec![
        entry("bedroom", "Bed*", 1),
        entry("sleeping room", "*room", 1),
    ]);
    let evaluation = apartment(&["Bedroom"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        findings(&evaluation),
        [(
            "a".into(),
            "row 1 `bedroom` and row 2 `sleeping room` together have 1 of 2 required member(s) via assigns; 1 missing"
                .into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("a1")]);
}

#[test]
fn an_unreadable_key_leaves_the_group_and_the_member_not_evaluated() {
    let parameters = requirements(vec![entry("bedroom", "Bed*", 2)]);
    let model = apartment(&["Bedroom", "Bedroom"]).unreadable("a2");
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("a2".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("a".to_owned(), NotEvaluatedReason::BackendUnavailable),
        ]
    );
}

#[test]
fn an_undecided_membership_leaves_the_group_not_evaluated() {
    // Only spaces stating a type are members; `a2`'s cannot be read.
    let mut parameters = requirements(vec![entry("bedroom", "Bed*", 1)]);
    parameters[2] = (
        "member_selector",
        selector(Selector::AllOf {
            operands: vec![
                kind("space"),
                Selector::Property {
                    property_set: Some("Pset".into()),
                    property: "Type".into(),
                    operator: ComparisonOperator::Exists,
                    value: None,
                    case_sensitive: true,
                    trim: false,
                    quantifier: None,
                    precision: None,
                },
            ],
        }),
    );
    let model = apartment(&["Bedroom", "Bedroom"]).unreadable("a2");
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("a".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "group-composition: 1 object(s) it reaches via assigns may be members"
    );
}

#[test]
fn group_rows_apply_by_the_groups_key_and_a_group_no_row_matches_is_found() {
    // `a` is a two-room apartment, `b` a studio no row is written for.
    let model = apartment(&["Bedroom", "Living room"])
        .text("a", "Pset", "Kind", "2-room")
        .object("b", "zone")
        .object("b1", "space")
        .edge("assigns", "b", "b1")
        .text("b1", "Pset", "Type", "Living room")
        .text("b", "Pset", "Kind", "Studio");
    let mut parameters = requirements(vec![
        row(&[
            ("group", string("2-room")),
            ("label", string("bedroom")),
            ("key_1", string("Bed*")),
            ("count", integer(1)),
        ]),
        entry("living room", "Living*", 1),
        row(&[
            ("group", string("3-room")),
            ("label", string("second bedroom")),
            ("key_1", string("Bed*")),
            ("count", integer(1)),
        ]),
    ]);
    parameters.push(("group_key", property(Some("Pset"), "Kind")));
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        findings(&evaluation),
        [(
            "b".into(),
            "no requirement row matches the group (Pset.Kind is `Studio`)".into()
        )]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn objects_no_group_reaches_are_found_with_an_ungrouped_selector() {
    let model = apartment(&["Bedroom"])
        .object("loose", "space")
        .text("loose", "Pset", "Type", "Bedroom");
    let mut parameters = requirements(vec![entry("bedroom", "Bed*", 1)]);
    parameters.push(("ungrouped_selector", selector(kind("space"))));
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        findings(&evaluation),
        [("loose".into(), "in no group via assigns".into())]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_ungrouped_object_is_not_found_when_a_group_cannot_be_walked() {
    // The second relationship is unknown to the source, so no group can be
    // walked and nothing proves the space is in none.
    let model = apartment(&["Bedroom"]).object("loose", "space");
    let mut parameters = requirements(vec![entry("bedroom", "Bed*", 1)]);
    parameters[3] = ("path", common::strings(&["assigns", "unknown"]));
    parameters.push(("ungrouped_selector", selector(kind("space"))));
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("a".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("a1".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("loose".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn a_declaration_without_a_relationship_or_a_count_is_invalid() {
    let mut parameters = requirements(vec![entry("bedroom", "Bed*", 1)]);
    parameters.pop();
    let evaluation = apartment(&["Bedroom"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "group-composition: a group reaches its members only through `relationship` or `path`"
    );

    let parameters = requirements(vec![row(&[("key_1", string("Bed*"))])]);
    let evaluation = apartment(&["Bedroom"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "group-composition: row 1 has no count"
    );
}

/// Apartment `a` of type `A`, number `1`, with a bedroom; the rows ask for
/// types `A` (number `1`) and `B`.
fn typed_apartments(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(String, String)> {
    let model = apartment(&["Bedroom"])
        .text("a", "Pset", "Kind", "A")
        .text("a", "Pset", "Number", "1");
    let mut parameters = requirements(vec![
        row(&[
            ("group", string("A")),
            ("group_3", string("1")),
            ("key_1", string("Bed*")),
            ("count", integer(1)),
        ]),
        row(&[
            ("group", string("B")),
            ("key_1", string("Bed*")),
            ("count", integer(2)),
        ]),
    ]);
    parameters.push(("group_key_1", property(Some("Pset"), "Kind")));
    parameters.push(("group_key_3", property(Some("Pset"), "Number")));
    parameters.extend(extra);
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );
    findings(&evaluation)
}

#[test]
fn a_required_group_missing_from_the_model_is_a_project_finding() {
    // Without the switch, the unmatched row is silently unused.
    assert!(typed_apartments(Vec::new()).is_empty());
    assert_eq!(
        typed_apartments(vec![(
            "report_absent_groups",
            ParameterValue::Boolean { value: true }
        )]),
        [(
            "project".into(),
            "not in model: no group matches row 2 (Pset.Kind like `B`)".into()
        )]
    );
}

#[test]
fn a_group_whose_key_cannot_be_read_may_be_the_missing_one() {
    let model =
        apartment(&["Bedroom"]).value("a", "Pset", "Kind", axioval_ir::PropertyValue::Integer(2));
    let mut parameters = requirements(vec![row(&[
        ("group", string("B")),
        ("key_1", string("Bed*")),
        ("count", integer(1)),
    ])]);
    parameters.push(("group_key", property(Some("Pset"), "Kind")));
    parameters.push((
        "report_absent_groups",
        ParameterValue::Boolean { value: true },
    ));
    let evaluation = model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("-".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("a".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn group_keys_must_be_declared_once_and_used() {
    for extra in [
        vec![
            ("group_key", property(Some("Pset"), "Kind")),
            ("group_key_1", property(Some("Pset"), "Kind")),
        ],
        vec![("group_key_2", property(Some("Pset"), "Name"))],
    ] {
        let mut parameters = requirements(vec![row(&[
            ("group", string("A")),
            ("group_3", string("1")),
            ("key_1", string("Bed*")),
            ("count", integer(1)),
        ])]);
        parameters.push(("group_key_3", property(Some("Pset"), "Number")));
        parameters.extend(extra);
        let evaluation =
            apartment(&["Bedroom"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// The measured list `compositions` never cites an answer measured
/// approximately: a group reached through one is left open rather than
/// found short on it, while an exact reach cites only exact evidence.
#[test]
fn a_composition_never_cites_an_approximate_answer() {
    let parameters = requirements(vec![entry("bedroom", "Bed*", 2)]);
    let exact =
        apartment(&["Bedroom"]).evaluate(&HELD, &rule(ID, kind("zone"), parameters.clone()));
    assert_eq!(exact.findings().len(), 1);
    assert!(exact.findings()[0].evidence.iter().all(|item| item.exact));
    let approximate = apartment(&["Bedroom"])
        .cite_approximate("assigns", "a", "derived:assigns")
        .evaluate(&HELD, &rule(ID, kind("zone"), parameters));
    assert!(approximate.findings().is_empty(), "{approximate:#?}");
    assert_eq!(
        unevaluated(&approximate),
        [("a".to_owned(), NotEvaluatedReason::InvalidEvidence)]
    );
}

/// Generated groups of members whose types are random, some unreadable or
/// undecided, some groups keyed, under random entries and switches, held to
/// the reference.
mod generated {
    use super::*;
    use proptest::prelude::*;

    const TYPES: [&str; 5] = ["Bedroom", "Living room", "Kitchen", "Bath", "Hall"];
    const PATTERNS: [&str; 6] = ["Bed*", "*room", "Kitchen", "B*", "*", "Hall"];
    const KINDS: [&str; 2] = ["flat", "house"];

    fn typed() -> Selector {
        Selector::Property {
            property_set: Some("Pset".into()),
            property: "Type".into(),
            operator: ComparisonOperator::Exists,
            value: None,
            case_sensitive: true,
            trim: false,
            quantifier: None,
            precision: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_compositions_hold_parity(
            groups in proptest::collection::vec(
                (
                    0..KINDS.len(),
                    any::<bool>(),
                    proptest::collection::vec((0..TYPES.len(), 0..8u8), 0..5),
                ),
                0..3,
            ),
            loose in proptest::collection::vec(0..TYPES.len(), 0..2),
            entries in proptest::collection::vec(
                (0..PATTERNS.len(), 0..3i64, proptest::option::of(0..KINDS.len())),
                1..4,
            ),
            keyed in any::<bool>(),
            absent in any::<bool>(),
            ungrouped in any::<bool>(),
        ) {
            let mut model = Model::default();
            for (index, (kind_of, unreadable, members)) in groups.iter().enumerate() {
                let group = format!("g{index}");
                model = model.object(&group, "zone");
                model = if *unreadable {
                    model.unreadable_value(&group, "Pset", "Kind", "string")
                } else {
                    model.text(&group, "Pset", "Kind", KINDS[*kind_of])
                };
                for (number, (space_type, fate)) in members.iter().enumerate() {
                    let member = format!("{group}m{number}");
                    model = model.object(&member, "space").edge("assigns", &group, &member);
                    // Fate 0 leaves the type unreadable, 1 its object
                    // unreadable (its membership undecided), 2 the type
                    // absent.
                    model = match fate {
                        0 => model.unreadable_value(&member, "Pset", "Type", "string"),
                        1 => model.unreadable(&member),
                        2 => model,
                        _ => model.text(&member, "Pset", "Type", TYPES[*space_type]),
                    };
                }
            }
            for (number, space_type) in loose.iter().enumerate() {
                let member = format!("loose{number}");
                model = model
                    .object(&member, "space")
                    .text(&member, "Pset", "Type", TYPES[*space_type]);
            }
            let rows: Vec<TableRow> = entries
                .iter()
                .map(|(pattern, count, group)| {
                    let mut cells = vec![
                        ("key_1", string(PATTERNS[*pattern])),
                        ("count", integer(*count)),
                    ];
                    if keyed && let Some(group) = group {
                        cells.push(("group", string(KINDS[*group])));
                    }
                    row(&cells)
                })
                .collect();
            let any_group_cell = keyed && entries.iter().any(|(_, _, group)| group.is_some());
            let mut parameters = requirements(rows);
            parameters[2] = ("member_selector", selector(typed()));
            if any_group_cell {
                parameters.push(("group_key", property(Some("Pset"), "Kind")));
            }
            if absent {
                parameters.push((
                    "report_absent_groups",
                    ParameterValue::Boolean { value: true },
                ));
            }
            if ungrouped {
                parameters.push(("ungrouped_selector", selector(kind("space"))));
            }
            model.evaluate(&HELD, &rule(ID, kind("zone"), parameters));
        }
    }
}
