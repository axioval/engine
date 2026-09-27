//! `space-connection`: direct access between spaces and to the outside.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_rules::SpaceConnection;
use common::{Model, findings, id, kind, rule, selector, string, strings, unevaluated};

const ID: &str = "axioval:capability.space-connection";
const ADJACENT: &str = "axioval:derived.adjacent-space";

/// An adjacency edge from `element` to `space` found on `side`.
fn edge(element: &str, space: &str, side: char) -> String {
    format!(
        "{ADJACENT};reach=1:{}->{}:side={side}(1.000000,0.000000):entered=0.000000",
        id(element),
        id(space)
    )
}

/// A face of `element` that enters no space.
fn outside(element: &str, side: char) -> String {
    format!(
        "{ADJACENT};reach=1:{}:side={side}(1.000000,0.000000):outside:reach=1",
        id(element)
    )
}

/// `element` joins `first` on its `+` face to `second` (a space, or the
/// outside when `None`) on its `-` face.
fn joins(model: Model, element: &str, first: &str, second: Option<&str>) -> Model {
    let model =
        model
            .edge(ADJACENT, element, first)
            .cite(ADJACENT, element, &edge(element, first, '+'));
    match second {
        Some(second) => model.edge(ADJACENT, element, second).cite(
            ADJACENT,
            element,
            &edge(element, second, '-'),
        ),
        None => model.cite(ADJACENT, element, &outside(element, '-')),
    }
}

/// Kitchen `k`, bedroom `b` and hall `h`. Door `d1` joins the kitchen and
/// the hall, door `d2` the bedroom and the kitchen, door `d3` the hall and
/// the outside, opening `o1` the hall and the bedroom.
fn flat() -> Model {
    let model = Model::default()
        .object("k", "kitchen")
        .object("b", "bedroom")
        .object("h", "hall")
        .object("d1", "door")
        .object("d2", "door")
        .object("d3", "door")
        .object("o1", "opening");
    let model = joins(model, "d1", "k", Some("h"));
    let model = joins(model, "d2", "b", Some("k"));
    let model = joins(model, "d3", "h", None);
    joins(model, "o1", "h", Some("b"))
}

fn spaces() -> Selector {
    Selector::AnyOf {
        operands: vec![kind("kitchen"), kind("bedroom"), kind("hall")],
    }
}

fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect()
}

fn parameters(path: &str, rows: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("connections", ParameterValue::Table { value: rows }),
        ("access_path", strings(&[path])),
        ("door_selector", selector(kind("door"))),
        ("opening_selector", selector(kind("opening"))),
    ]
}

#[test]
fn a_forbidden_connection_and_a_missing_exit_are_found() {
    let evaluation = flat().evaluate(
        &SpaceConnection,
        &rule(
            ID,
            spaces(),
            parameters(
                ADJACENT,
                vec![
                    // No bedroom may open onto the kitchen.
                    row(&[
                        ("label", string("no kitchen")),
                        ("from", selector(kind("bedroom"))),
                        ("to", selector(kind("kitchen"))),
                        ("access", string("forbidden")),
                    ]),
                    // Hall and kitchen each need a door to the outside.
                    row(&[
                        ("from", selector(kind("hall"))),
                        ("access_type", string("doors")),
                        ("exit", string("required")),
                    ]),
                    row(&[
                        ("from", selector(kind("kitchen"))),
                        ("exit", string("required")),
                    ]),
                    // The hall needs a door to the bedroom; it has an opening.
                    row(&[
                        ("from", selector(kind("hall"))),
                        ("to", selector(kind("bedroom"))),
                        ("access", string("required")),
                        ("access_type", string("doors")),
                    ]),
                    // The kitchen needs access to the hall, and has it.
                    row(&[
                        ("from", selector(kind("kitchen"))),
                        ("to", selector(kind("hall"))),
                        ("access", string("required")),
                    ]),
                ],
            ),
        ),
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    assert_eq!(
        findings(&evaluation),
        [
            (
                "b".into(),
                format!(
                    "has direct access to {} through {}, which row 0 (no kitchen) forbids for a \
                     door or opening",
                    id("k"),
                    id("d2")
                )
            ),
            (
                "h".into(),
                format!(
                    "has no direct access through a door to a space row 3 requires (via {ADJACENT})"
                )
            ),
            (
                "k".into(),
                "has no door or opening directly to the outside, which row 2 requires".into()
            ),
        ]
    );
    let forbidden = &evaluation.findings()[0];
    assert!(forbidden.related.contains(&id("d2")) && forbidden.related.contains(&id("k")));
    assert!(
        forbidden
            .evidence
            .iter()
            .any(|item| item.locator == edge("d2", "k", '-')),
        "{:?}",
        forbidden.evidence
    );
}

#[test]
fn a_forbidden_exit_is_found_and_openings_count_as_access() {
    let evaluation = flat().evaluate(
        &SpaceConnection,
        &rule(
            ID,
            spaces(),
            parameters(
                ADJACENT,
                vec![
                    row(&[
                        ("from", selector(kind("hall"))),
                        ("exit", string("forbidden")),
                    ]),
                    row(&[
                        ("from", selector(kind("hall"))),
                        ("to", selector(kind("bedroom"))),
                        ("access", string("required")),
                        ("access_type", string("openings")),
                    ]),
                ],
            ),
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "h".into(),
            format!(
                "opens directly to the outside through {}, which row 0 forbids for a door or \
                 opening",
                id("d3")
            )
        )]
    );
}

#[test]
fn spaces_on_one_face_of_a_door_are_not_connected_through_it() {
    // d4 has the bedroom and the hall on the same face.
    let model = flat()
        .object("d4", "door")
        .edge(ADJACENT, "d4", "b")
        .cite(ADJACENT, "d4", &edge("d4", "b", '+'))
        .edge(ADJACENT, "d4", "h")
        .cite(ADJACENT, "d4", &edge("d4", "h", '+'));
    let evaluation = model.evaluate(
        &SpaceConnection,
        &rule(
            ID,
            kind("hall"),
            parameters(
                ADJACENT,
                vec![row(&[
                    ("from", selector(kind("hall"))),
                    ("to", selector(kind("bedroom"))),
                    ("access", string("required")),
                    ("access_type", string("doors")),
                ])],
            ),
        ),
    );
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
}

/// Doors that carry `Door.Kind`; the source cannot read `d1`'s.
fn declared_door() -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("door"),
            Selector::property(
                Some("Door".into()),
                "Kind",
                ComparisonOperator::Exists,
                None,
            ),
        ],
    }
}

#[test]
fn an_undecided_door_leaves_what_it_could_change_not_evaluated() {
    let model = flat()
        .value("d2", "Door", "Kind", PropertyValue::String("swing".into()))
        .value("d3", "Door", "Kind", PropertyValue::String("swing".into()))
        .unreadable("d1");
    let mut parameters = parameters(
        ADJACENT,
        vec![
            // Only d1 could join the kitchen to the hall.
            row(&[
                ("from", selector(kind("kitchen"))),
                ("to", selector(kind("hall"))),
                ("access", string("forbidden")),
                ("access_type", string("doors")),
            ]),
            // d2 surely joins the bedroom to the kitchen.
            row(&[
                ("from", selector(kind("bedroom"))),
                ("to", selector(kind("kitchen"))),
                ("access", string("forbidden")),
                ("access_type", string("doors")),
            ]),
        ],
    );
    parameters[2] = ("door_selector", selector(declared_door()));
    let evaluation = model.evaluate(&SpaceConnection, &rule(ID, spaces(), parameters));
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(space, _)| space)
            .collect::<Vec<_>>(),
        ["b"]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("k".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn stated_boundaries_connect_spaces_but_cannot_show_the_outside() {
    let boundaries = || {
        Model::default()
            .object("k", "kitchen")
            .object("b", "bedroom")
            .object("d2", "door")
            .edge("boundary", "k", "d2")
            .edge("boundary", "b", "d2")
    };
    let forbidden = row(&[
        ("from", selector(kind("bedroom"))),
        ("to", selector(kind("kitchen"))),
        ("access", string("forbidden")),
    ]);
    let evaluation = boundaries().evaluate(
        &SpaceConnection,
        &rule(
            ID,
            spaces(),
            parameters("boundary:backward", vec![forbidden.clone()]),
        ),
    );
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(space, _)| space)
            .collect::<Vec<_>>(),
        ["b"]
    );
    let exit = row(&[
        ("from", selector(kind("bedroom"))),
        ("exit", string("required")),
    ]);
    let evaluation = boundaries().evaluate(
        &SpaceConnection,
        &rule(
            ID,
            spaces(),
            parameters("boundary:backward", vec![forbidden, exit]),
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("only `axioval:derived.adjacent-space` records"),
        "{evaluation:?}"
    );
}

#[test]
fn declarations_that_cannot_be_judged_are_refused() {
    for rows in [
        // Access without a destination.
        vec![row(&[
            ("from", selector(kind("bedroom"))),
            ("access", string("forbidden")),
        ])],
        vec![row(&[
            ("from", selector(kind("bedroom"))),
            ("exit", string("sometimes")),
        ])],
        vec![row(&[
            ("from", selector(kind("bedroom"))),
            ("access_type", string("windows")),
        ])],
    ] {
        let evaluation = flat().evaluate(
            &SpaceConnection,
            &rule(ID, spaces(), parameters(ADJACENT, rows)),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    // Doors cannot be told apart without a door selector.
    let evaluation = flat().evaluate(
        &SpaceConnection,
        &rule(
            ID,
            spaces(),
            vec![
                (
                    "connections",
                    ParameterValue::Table {
                        value: vec![row(&[
                            ("from", selector(kind("hall"))),
                            ("exit", string("required")),
                            ("access_type", string("doors")),
                        ])],
                    },
                ),
                ("access_path", strings(&[ADJACENT])),
                ("opening_selector", selector(kind("opening"))),
            ],
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
