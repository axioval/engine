//! `opening-spaces`: doors, windows and openings relate to the spaces their
//! host wall's exposure calls for.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue};
use axioval_rules::OpeningSpaces;
use common::{Model, findings, id, kind, property, rule, selector, strings, unevaluated};

const ID: &str = "axioval:capability.opening-spaces";
const ADJACENT: &str = "axioval:derived.adjacent-space";

/// Internal wall `wi`, external wall `we`, wall `wu` declaring nothing and
/// spaces `s1`, `s2`. Each door or window fills its own opening `o…`.
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
    let evaluation = model.evaluate(
        &OpeningSpaces,
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
    let evaluation = model.evaluate(
        &OpeningSpaces,
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
    let evaluation = model.evaluate(
        &OpeningSpaces,
        &rule(ID, kind("window"), parameters(&[ADJACENT])),
    );
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
    let evaluation =
        internal_only.evaluate(&OpeningSpaces, &rule(ID, kind("opening"), openings.clone()));
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
    let evaluation = undeclared.evaluate(&OpeningSpaces, &rule(ID, kind("opening"), openings));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn an_element_without_a_host_wall_is_not_evaluated() {
    let model = building().object("d9", "door");
    let evaluation = model.evaluate(
        &OpeningSpaces,
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
        let evaluation = building().evaluate(
            &OpeningSpaces,
            &rule(ID, doors_and_windows(), parameters(&path)),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
            "{path:?}"
        );
    }
}
