//! `opening-spaces`: doors, windows and openings relate to the spaces their
//! host wall's exposure calls for.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue};
use axioval_rules::OpeningSpaces;
use common::{Model, findings, id, kind, property, rule, selector, strings, unevaluated};

const ID: &str = "axioval:capability.opening-spaces";
const ADJACENT: &str = "axioval:derived.adjacent-space";

/// Internal wall `wi`, external wall `we`, wall `wu` declaring nothing and
/// spaces `s1`, `s2`. Each door or window fills its own opening `o…`.
/// The template's evaluation of `rule` over `model`, held to the
/// implementation it replaced under `Parity::contract()`.
fn held(model: Model, rule: &axioval_engine::CompiledRule) -> axioval_engine::CapabilityEvaluation {
    model.holding_contract(
        &OpeningSpaces,
        &axioval_rules::reference::OpeningSpaces,
        rule,
        |_| {},
        &[],
        0.0,
    )
}

fn building() -> Model {
    let mut model = Model::default()
        .object("wi", "wall")
        .object("we", "wall")
        .object("wu", "wall")
        .object("s1", "space")
        .object("s2", "space")
        .value("wi", "Pset", "IsExternal", PropertyValue::Boolean(false))
        .value("we", "Pset", "IsExternal", PropertyValue::Boolean(true));
    for (element, kind, wall) in [
        ("d1", "door", "wi"),
        ("d2", "door", "wi"),
        ("d3", "door", "wu"),
        ("w1", "window", "we"),
        ("w2", "window", "we"),
    ] {
        let opening = format!("o{element}");
        model = model
            .object(&opening, "opening")
            .object(element, kind)
            .edge("voids", wall, &opening)
            .edge("fills", &opening, element);
    }
    model
}

fn parameters(space_path: &[&str]) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("host_path", strings(&["fills:backward", "voids:backward"])),
        ("host_selector", selector(kind("wall"))),
        ("external_property", property(Some("Pset"), "IsExternal")),
        ("space_path", strings(space_path)),
        ("space_selector", selector(kind("space"))),
    ]
}

fn doors_and_windows() -> Selector {
    Selector::AnyOf {
        operands: vec![kind("door"), kind("window")],
    }
}

#[test]
fn stated_boundaries_are_counted_by_the_host_walls_exposure() {
    let model = building()
        // d1 in the internal wall has one space, d2 two.
        .edge("boundary", "s1", "d1")
        .edge("boundary", "s1", "d2")
        .edge("boundary", "s2", "d2")
        // w1 in the external wall has none, w2 one.
        .edge("boundary", "s2", "w2")
        .edge("boundary", "s1", "d3");
    let evaluation = held(
        model,
        &rule(ID, doors_and_windows(), parameters(&["boundary:backward"])),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d1".into(),
                "relates to 1 space(s) via boundary; in an internal wall (wi) it needs two \
                 spaces, one on each side"
                    .into()
            ),
            (
                "w1".into(),
                "relates to 0 space(s) via boundary; in an external wall (we) it needs one \
                 space, the other side outside"
                    .into()
            ),
        ]
    );
    // An undeclared `IsExternal` is not evaluated, never taken as internal.
    assert_eq!(
        unevaluated(&evaluation),
        [("d3".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let d1 = &evaluation.findings()[0];
    assert!(d1.related.contains(&id("wi")) && d1.related.contains(&id("s1")));
    assert!(
        d1.evidence
            .iter()
            .any(|item| item.locator.ends_with("Pset.IsExternal")),
        "{:?}",
        d1.evidence
    );
}

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

fn adjacent(model: Model, element: &str, space: &str, side: char) -> Model {
    model
        .edge(ADJACENT, element, space)
        .cite(ADJACENT, element, &edge(element, space, side))
}

#[test]
fn derived_adjacency_needs_the_spaces_on_opposite_sides() {
    let mut model = building();
    // d1 connects s1 and s2 across the wall; d2 has both on the same side.
    model = adjacent(model, "d1", "s1", '+');
    model = adjacent(model, "d1", "s2", '-');
    model = adjacent(model, "d2", "s1", '+');
    model = adjacent(model, "d2", "s2", '+');
    // w1 opens from s1 to the outside; w2 has s1 on both faces.
    model = adjacent(model, "w1", "s1", '-').cite(ADJACENT, "w1", &outside("w1", '+'));
    model = adjacent(model, "w2", "s1", '+').cite(ADJACENT, "w2", &edge("w2", "s1", '-'));
    let evaluation = held(
        model,
        &rule(ID, doors_and_windows(), parameters(&[ADJACENT])),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d2".into(),
                "its spaces are not on opposite sides (s1 on side +, s2 on side +); in an \
                 internal wall (wi) it needs two spaces, one on each side"
                    .into()
            ),
            (
                "w2".into(),
                "its one space lies on both sides (s1 on side + and -); in an external wall \
                 (we) it needs one space, the other side outside"
                    .into()
            ),
        ]
    );
    // d3's wall declares nothing and it has no adjacency at all.
    assert_eq!(
        unevaluated(&evaluation),
        [("d3".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_space_the_evidence_places_on_no_side_is_not_evaluated() {
    let model = building().edge(ADJACENT, "w1", "s1");
    let evaluation = held(model, &rule(ID, kind("window"), parameters(&[ADJACENT])));
    assert!(
        findings(&evaluation)
            .iter()
            .all(|(object, _)| object != "w1")
    );
    assert!(unevaluated(&evaluation).contains(&("w1".into(), NotEvaluatedReason::InvalidEvidence)));
}

#[test]
fn a_source_without_an_external_wall_is_reported_against_the_source() {
    // Only the internal wall hosts anything.
    let internal_only = Model::default()
        .object("wi", "wall")
        .object("o", "opening")
        .object("s1", "space")
        .object("s2", "space")
        .value("wi", "Pset", "IsExternal", PropertyValue::Boolean(false))
        .edge("voids", "wi", "o")
        .edge("boundary", "s1", "o")
        .edge("boundary", "s2", "o");
    let mut openings = parameters(&["boundary:backward"]);
    openings[0].1 = strings(&["voids:backward"]);
    let evaluation = held(internal_only, &rule(ID, kind("opening"), openings.clone()));
    assert_eq!(
        findings(&evaluation),
        [(
            "source".into(),
            "none of the 1 host wall(s) in source `test:model` is declared external".into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());

    // A wall that declares nothing might be the external one.
    let undeclared = Model::default()
        .object("wi", "wall")
        .object("wu", "wall")
        .object("o", "opening")
        .object("s1", "space")
        .object("s2", "space")
        .value("wi", "Pset", "IsExternal", PropertyValue::Boolean(false))
        .edge("voids", "wi", "o")
        .edge("boundary", "s1", "o")
        .edge("boundary", "s2", "o");
    let evaluation = held(undeclared, &rule(ID, kind("opening"), openings));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn an_element_without_a_host_wall_is_not_evaluated() {
    let model = building().object("d9", "door");
    let evaluation = held(
        model,
        &rule(ID, kind("door"), parameters(&["boundary:backward"])),
    );
    assert!(
        unevaluated(&evaluation).contains(&("d9".into(), NotEvaluatedReason::IncompleteEvidence))
    );
}

#[test]
fn the_derived_adjacency_must_be_the_only_forward_step() {
    for path in [
        vec!["fills:backward", ADJACENT],
        vec!["axioval:derived.adjacent-space;reach=2:backward"],
    ] {
        let evaluation = held(
            building(),
            &rule(ID, doors_and_windows(), parameters(&path)),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
            "{path:?}"
        );
    }
}

/// An element's spaces are never read from adjacency measured on a
/// tessellation: such evidence is refused, which leaves the element open,
/// never an exact verdict; from exact adjacency its finding cites exact
/// evidence.
#[test]
fn connected_spaces_read_from_approximate_adjacency_are_never_exact() {
    let model = || {
        let model = adjacent(building(), "d2", "s1", '+');
        adjacent(model, "d2", "s2", '+')
    };
    let approximate = model().cite_approximate(ADJACENT, "d2", "mesh:d2");
    let evaluation = held(
        approximate,
        &rule(ID, kind("door"), parameters(&[ADJACENT])),
    );
    assert!(
        findings(&evaluation)
            .iter()
            .all(|(object, _)| object != "d2"),
        "{evaluation:?}"
    );
    assert!(
        unevaluated(&evaluation)
            .iter()
            .any(|(object, _)| object == "d2"),
        "{evaluation:?}"
    );
    let evaluation = held(model(), &rule(ID, kind("door"), parameters(&[ADJACENT])));
    let found = evaluation
        .findings()
        .iter()
        .find(|finding| {
            finding
                .message
                .starts_with("its spaces are not on opposite sides")
        })
        .expect("d2's spaces on one side");
    assert!(found.evidence.iter().all(|evidence| evidence.exact));
}

mod generated {
    use super::*;
    use proptest::prelude::*;

    /// How a wall declares its exposure: true, false, `null`, a text, or
    /// not at all.
    fn exposure() -> impl Strategy<Value = u8> {
        0u8..5
    }

    /// Spaces by kind, or by a use some of them state unreadably.
    fn used_as(use_: &str) -> ParameterValue {
        selector(
            serde_json::from_value(serde_json::json!({
                "kind": "property", "propertySet": "Pset", "property": "Use",
                "operator": "equals", "value": {"type": "string", "value": use_}}))
            .unwrap(),
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_elements_hold_parity(
            walls in proptest::collection::vec(exposure(), 1..4),
            elements in proptest::collection::vec((0usize..4, proptest::collection::vec((0usize..3, 0u8..3), 0..4)), 1..5),
            uses in proptest::collection::vec(0u8..3, 3),
            by_use in any::<bool>(),
            sided in any::<bool>(),
            undecided_wall in any::<bool>(),
        ) {
            let mut model = Model::default();
            for (index, declared) in walls.iter().enumerate() {
                let wall = format!("w{index}");
                model = model.object(&wall, "wall");
                model = match declared {
                    0 => model.value(&wall, "Pset", "IsExternal", PropertyValue::Boolean(true)),
                    1 => model.value(&wall, "Pset", "IsExternal", PropertyValue::Boolean(false)),
                    2 => model.value(&wall, "Pset", "IsExternal", PropertyValue::Null),
                    3 => model.value(&wall, "Pset", "IsExternal", PropertyValue::String("yes".into())),
                    _ => model,
                };
            }
            for ((space, stated), _) in ["s0", "s1", "s2"].into_iter().zip(&uses).zip(0..) {
                model = model.object(space, "space");
                model = match stated {
                    0 => model.text(space, "Pset", "Use", "room"),
                    1 => model.unreadable_value(space, "Pset", "Use", "IFCLABEL"),
                    _ => model,
                };
            }
            for (index, (host, spaces)) in elements.iter().enumerate() {
                let element = format!("e{index}");
                model = model.object(&element, "opening");
                if *host < walls.len() {
                    model = model.edge("voids", &format!("w{host}"), &element);
                }
                for (space, side) in spaces {
                    let space = format!("s{space}");
                    if sided {
                        let face = ['+', '-', '+'][usize::from(*side)];
                        model = adjacent(model, &element, &space, face);
                        if *side == 2 {
                            model = model.cite(ADJACENT, &element, &outside(&element, '-'));
                        }
                    } else {
                        model = model.edge("boundary", &space, &element);
                    }
                }
            }
            let mut parameters = vec![
                ("host_path", strings(&["voids:backward"])),
                ("host_selector", selector(kind("wall"))),
                ("external_property", property(Some("Pset"), "IsExternal")),
                (
                    "space_path",
                    strings(&[if sided { ADJACENT } else { "boundary:backward" }]),
                ),
                (
                    "space_selector",
                    if by_use { used_as("room") } else { selector(kind("space")) },
                ),
            ];
            if undecided_wall {
                // A thing the host selector cannot decide, hosting the
                // elements that name no wall.
                model = model
                    .object("x", "thing")
                    .unreadable_value("x", "Pset", "Kind", "IFCLABEL");
                for (index, (host, _)) in elements.iter().enumerate() {
                    if *host >= walls.len() {
                        model = model.edge("voids", "x", &format!("e{index}"));
                    }
                }
                parameters[1] = (
                    "host_selector",
                    selector(Selector::AnyOf {
                        operands: vec![
                            kind("wall"),
                            Selector::AllOf {
                                operands: vec![
                                    kind("thing"),
                                    Selector::property(
                                        Some("Pset".into()),
                                        "Kind",
                                        ComparisonOperator::Exists,
                                        None,
                                    ),
                                ],
                            },
                        ],
                    }),
                );
            }
            held(model, &rule(ID, kind("opening"), parameters));
        }
    }
}
