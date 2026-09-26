//! Relationship paths: several relationship steps walked one after another.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, Selector};
use axioval_rules::RelatedCount;
use common::{Model, findings, integer, kind, rule, selector, string, strings, unevaluated};

const ID: &str = "axioval:capability.related-count";

/// Fire wall `w1` has two openings; `o1` holds an EI30 door, `o2` a plain
/// one. Wall `w2` holds a plain door but is no fire wall.
fn walls() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("o1", "opening")
        .object("o2", "opening")
        .object("o3", "opening")
        .object("d1", "door")
        .object("d2", "door")
        .object("d3", "door")
        .edge("voids", "w1", "o1")
        .edge("voids", "w1", "o2")
        .edge("voids", "w2", "o3")
        .edge("fills", "o1", "d1")
        .edge("fills", "o2", "d2")
        .edge("fills", "o3", "d3")
        .text("d1", "Type", "Name", "EI30-T1")
        .text("d2", "Type", "Name", "T1")
        .text("d3", "Type", "Name", "T1")
}

/// Doors whose type is not a fire-rated one.
fn unrated_doors() -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("door"),
            Selector::Not {
                operand: Box::new(Selector::Property {
                    property_set: Some("Type".into()),
                    property: "Name".into(),
                    operator: ComparisonOperator::Matches,
                    value: Some(string("^EI\\d+")),
                }),
            },
        ],
    }
}

#[test]
fn a_path_reaches_the_doors_filling_a_walls_openings() {
    let evaluation = walls().evaluate(
        &RelatedCount,
        &rule(
            ID,
            Selector::Property {
                property_set: None,
                property: "IsFireWall".into(),
                operator: ComparisonOperator::Exists,
                value: None,
            },
            vec![
                ("related_selector", selector(unrated_doors())),
                ("path", strings(&["voids:forward", "fills"])),
                ("maximum", integer(0)),
            ],
        ),
    );
    // No wall states IsFireWall in this model; nothing is selected.
    assert!(evaluation.findings().is_empty());
    let fire_walls = walls().text("w1", "Pset", "IsFireWall", "yes");
    let evaluation = fire_walls.evaluate(
        &RelatedCount,
        &rule(
            ID,
            Selector::Property {
                property_set: Some("Pset".into()),
                property: "IsFireWall".into(),
                operator: ComparisonOperator::Exists,
                value: None,
            },
            vec![
                ("related_selector", selector(unrated_doors())),
                ("path", strings(&["voids:forward", "fills"])),
                ("maximum", integer(0)),
            ],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "1 related object(s) via voids then fills; required at most 0".into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related[0].local_id, "d2");
    // Both steps' completeness evidence is cited.
    let locators: Vec<&str> = evaluation.findings()[0]
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert_eq!(locators, ["scan:fills", "scan:voids"]);
}

#[test]
fn a_path_walks_backwards_too() {
    // From each door back to the wall it sits in.
    let evaluation = walls().evaluate(
        &RelatedCount,
        &rule(
            ID,
            kind("door"),
            vec![
                ("related_selector", selector(kind("wall"))),
                ("path", strings(&["fills:backward", "voids:backward"])),
                ("minimum", integer(1)),
                ("maximum", integer(1)),
            ],
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
fn a_path_and_a_single_relationship_are_exclusive() {
    for parameters in [
        vec![
            ("path", strings(&["voids"])),
            ("relationship", string("voids")),
            ("maximum", integer(0)),
        ],
        vec![
            ("path", strings(&["voids:sideways"])),
            ("maximum", integer(0)),
        ],
        vec![("path", strings(&[])), ("maximum", integer(0))],
    ] {
        let evaluation = walls().evaluate(&RelatedCount, &rule(ID, kind("wall"), parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
