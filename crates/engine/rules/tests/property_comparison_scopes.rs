//! Property comparison along paths, within spaces and buildings, with text
//! patterns, presence operators, ranges and finding categories.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::PropertyComparison;
use common::{
    Model, boolean, findings, kind, number, property, rule, selector, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.property-comparison";

/// The parameters every rule declares, with `compared_selector` picking `candidates`.
fn base<'a>(
    candidates: &str,
    mode: &str,
    quantifier: &str,
    operator: &str,
) -> Vec<(&'a str, ParameterValue)> {
    vec![
        ("compared_selector", selector(kind(candidates))),
        ("component_mode", string(mode)),
        ("quantifier", string(quantifier)),
        ("operator", string(operator)),
        ("factor", number(1.0)),
    ]
}

fn with<'a>(
    mut parameters: Vec<(&'a str, ParameterValue)>,
    more: Vec<(&'a str, ParameterValue)>,
) -> Vec<(&'a str, ParameterValue)> {
    parameters.extend(more);
    parameters
}

fn fire_walls() -> Selector {
    Selector::property(
        Some("Pset".into()),
        "IsFireWall",
        ComparisonOperator::Exists,
        None,
    )
}

/// Fire wall `w1` has two openings: `o1` holds door `d1` of type `EI30-T1`,
/// `o2` door `d2` of type `t30-rs`. Wall `w2` is no fire wall and holds `d3`.
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
        .text("w1", "Pset", "IsFireWall", "yes")
        .text("d1", "Type", "Name", "EI30-T1")
        .text("d2", "Type", "Name", "t30-rs")
        .text("d3", "Type", "Name", "plain")
}

fn door_types(
    operator: &str,
    more: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    with(
        base("door", "related", "each", operator),
        with(
            vec![
                ("compared_property", property(Some("Type"), "Name")),
                ("path", strings(&["voids:forward", "fills"])),
            ],
            more,
        ),
    )
}

#[test]
fn fire_wall_door_types_are_matched_through_voids_and_fills_against_a_pattern_list() {
    let patterns = || ("target_texts", strings(&["EI??-*", "T30*"]));
    let evaluation = walls().evaluate(
        &PropertyComparison,
        &rule(ID, fire_walls(), door_types("like", vec![patterns()])),
    );
    // `t30-rs` is not `T30*` when case matters; w2 is not a fire wall.
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "candidate test:model/d2 does not satisfy comparison".into()
        )]
    );
    let evaluation = walls().evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types("like", vec![patterns(), ("case_sensitive", boolean(false))]),
        ),
    );
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_single_pattern_matches_the_whole_value() {
    let evaluation = walls().evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types("like", vec![("target_text", string("EI30"))]),
        ),
    );
    // `EI30` is a prefix of `EI30-T1`, not the whole value.
    assert_eq!(evaluation.findings().len(), 2);
}

#[test]
fn a_backslash_makes_a_wildcard_character_literal() {
    let model = || walls().text("d1", "Type", "Name", "T*1");
    let evaluate = |pattern: &str| {
        let evaluation = model().evaluate(
            &PropertyComparison,
            &rule(
                ID,
                fire_walls(),
                door_types(
                    "like",
                    vec![("target_texts", strings(&[pattern, "t30-rs"]))],
                ),
            ),
        );
        findings(&evaluation).len()
    };
    // `T\*1` is exactly `T*1`; `T*1` also takes `TX1`, and `T\*1` does not.
    assert_eq!(evaluate(r"T\*1"), 0);
    let evaluation = walls().text("d1", "Type", "Name", "TX1").evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types(
                "like",
                vec![("target_texts", strings(&[r"T\*1", "t30-rs"]))],
            ),
        ),
    );
    assert_eq!(evaluation.findings().len(), 1);
    // A trailing backslash escapes nothing: an invalid declaration.
    let evaluation = walls().evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types("like", vec![("target_text", string("T\\"))]),
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn matches_is_a_regular_expression_over_the_whole_value() {
    let rule_for = |pattern: &str| {
        rule(
            ID,
            fire_walls(),
            door_types(
                "matches",
                vec![
                    ("target_texts", strings(&[pattern])),
                    ("case_sensitive", boolean(false)),
                ],
            ),
        )
    };
    let evaluation = walls().evaluate(&PropertyComparison, &rule_for(r"(EI|T)\d+-.*"));
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    // Anchored: `EI\d+` does not match `EI30-T1`.
    let evaluation = walls().evaluate(&PropertyComparison, &rule_for(r"EI\d+"));
    assert_eq!(evaluation.findings().len(), 2);
    let evaluation = walls().evaluate(&PropertyComparison, &rule_for("("));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_pattern_may_be_a_property_of_the_checked_object() {
    let model = || walls().text("w1", "Pset", "DoorPattern", "EI*");
    let evaluation = model().evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types(
                "like",
                vec![("target_property", property(Some("Pset"), "DoorPattern"))],
            ),
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "candidate test:model/d2 does not satisfy comparison".into()
        )]
    );
    // A pattern read from the model that does not compile is not evaluated.
    let evaluation = walls().text("w1", "Pset", "DoorPattern", "EI\\").evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types(
                "like",
                vec![("target_property", property(Some("Pset"), "DoorPattern"))],
            ),
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("w1".into(), NotEvaluatedReason::InvalidEvidence),
            ("w1".into(), NotEvaluatedReason::InvalidEvidence),
        ]
    );
}

#[test]
fn contains_takes_a_text_list_and_ignores_case_on_request() {
    let rule_for = |case_sensitive| {
        rule(
            ID,
            fire_walls(),
            door_types(
                "contains",
                vec![
                    ("target_texts", strings(&["T30", "EI30"])),
                    ("case_sensitive", boolean(case_sensitive)),
                ],
            ),
        )
    };
    let evaluation = walls().evaluate(&PropertyComparison, &rule_for(true));
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "candidate test:model/d2 does not satisfy comparison".into()
        )]
    );
    let evaluation = walls().evaluate(&PropertyComparison, &rule_for(false));
    assert!(findings(&evaluation).is_empty());
}

#[test]
fn a_pattern_operator_rejects_a_number_target() {
    let evaluation = walls().evaluate(
        &PropertyComparison,
        &rule(
            ID,
            fire_walls(),
            door_types("like", vec![("target_number", number(3.0))]),
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn is_undefined_and_is_defined_take_no_target_and_read_absence() {
    // d1 states nothing, d2 a blank rating, d3 is not on a fire wall.
    let model = || {
        walls()
            .object("o4", "opening")
            .object("d4", "door")
            .edge("voids", "w1", "o4")
            .edge("fills", "o4", "d4")
            .text("d2", "Pset", "FireRating", "  ")
            .text("d4", "Pset", "FireRating", "EI30")
    };
    let rule_for = |operator| {
        rule(
            ID,
            fire_walls(),
            with(
                base("door", "related", "each", operator),
                vec![
                    ("compared_property", property(Some("Pset"), "FireRating")),
                    ("path", strings(&["voids", "fills"])),
                ],
            ),
        )
    };
    let evaluation = model().evaluate(&PropertyComparison, &rule_for("is_undefined"));
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "candidate test:model/d4 does not satisfy comparison".into()
        )]
    );
    let evaluation = model().evaluate(&PropertyComparison, &rule_for("is_defined"));
    let mut flagged: Vec<_> = findings(&evaluation).into_iter().map(|(_, m)| m).collect();
    flagged.sort();
    assert_eq!(
        flagged,
        [
            "candidate test:model/d1 does not satisfy comparison",
            "candidate test:model/d2 does not satisfy comparison",
        ]
    );
    // A presence operator takes no target.
    let mut with_target = rule_for("is_defined");
    with_target
        .parameters
        .insert("target_text".into(), string("EI30"));
    let evaluation = model().evaluate(&PropertyComparison, &with_target);
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// Storey `st` holds spaces `s1` and `s2`; `s1` holds doors `d1`, `d2` and
/// sensor `x1`, `s2` holds door `d3` and sensor `x2`. Sensor `x3` stands on
/// the storey itself, in no space.
fn spaces() -> Model {
    Model::default()
        .object("st", "storey")
        .object("s1", "space")
        .object("s2", "space")
        .object("d1", "door")
        .object("d2", "door")
        .object("d3", "door")
        .object("x1", "sensor")
        .object("x2", "sensor")
        .object("x3", "sensor")
        .edge("aggregates", "st", "s1")
        .edge("aggregates", "st", "s2")
        .edge("contained", "s1", "d1")
        .edge("contained", "s1", "d2")
        .edge("contained", "s1", "x1")
        .edge("contained", "s2", "d3")
        .edge("contained", "s2", "x2")
        .edge("contained", "st", "x3")
}

fn doors_per_space(
    more: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    with(
        base("door", "same_space", "count", "between"),
        with(
            vec![
                ("container_selector", selector(kind("space"))),
                ("relationship", string("contained")),
                ("direction", string("backward")),
                ("minimum_number", number(1.0)),
                ("maximum_number", number(1.0)),
            ],
            more,
        ),
    )
}

#[test]
fn same_space_counts_the_doors_sharing_the_checked_objects_space_against_a_range() {
    let evaluation = spaces().evaluate(
        &PropertyComparison,
        &rule(ID, kind("sensor"), doors_per_space(vec![])),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "x1".into(),
                "count of compared components is 2 and is not between 1 and 1".into()
            ),
            // In no space, so no door shares one with it.
            (
                "x3".into(),
                "count of compared components is 0 and is not between 1 and 1".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_container_mode_needs_a_container_selector_and_no_chain() {
    let mut without = rule(ID, kind("sensor"), doors_per_space(vec![]));
    without.parameters.remove("container_selector");
    let chained = rule(
        ID,
        kind("sensor"),
        doors_per_space(vec![("follow_chain", boolean(true))]),
    );
    for invalid in [without, chained] {
        let evaluation = spaces().evaluate(&PropertyComparison, &invalid);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn an_undecided_container_leaves_every_checked_object_not_evaluated() {
    let rule = rule(
        ID,
        kind("sensor"),
        doors_per_space(vec![(
            "container_selector",
            selector(Selector::property(
                Some("Pset".into()),
                "IsRoom",
                ComparisonOperator::Exists,
                None,
            )),
        )]),
    );
    let evaluation = spaces()
        .unreadable("s2")
        .evaluate(&PropertyComparison, &rule);
    assert!(evaluation.findings().is_empty());
    assert_eq!(unevaluated(&evaluation).len(), 3);
}

/// Buildings `b1` (storeys `f1`, `f2`) and `b2` (storey `f3`); each storey
/// holds one alarm, and `f1`'s alarm sits in space `s1` on that storey.
fn buildings() -> Model {
    Model::default()
        .object("b1", "building")
        .object("b2", "building")
        .object("f1", "storey")
        .object("f2", "storey")
        .object("f3", "storey")
        .object("s1", "space")
        .object("a1", "alarm")
        .object("a2", "alarm")
        .object("a3", "alarm")
        .edge("aggregates", "b1", "f1")
        .edge("aggregates", "b1", "f2")
        .edge("aggregates", "b2", "f3")
        .edge("aggregates", "f1", "s1")
        .edge("contained", "s1", "a1")
        .edge("contained", "f2", "a2")
        .edge("contained", "f3", "a3")
        .text("a1", "Pset", "System", "A")
        .text("a2", "Pset", "System", "A")
        .text("a3", "Pset", "System", "B")
}

#[test]
fn same_building_climbs_any_number_of_steps_to_the_shared_ancestor() {
    let evaluation = buildings().text("a2", "Pset", "System", "C").evaluate(
        &PropertyComparison,
        &rule(
            ID,
            kind("alarm"),
            with(
                base("alarm", "same_building", "each", "equals"),
                vec![
                    ("container_selector", selector(kind("building"))),
                    (
                        "path",
                        strings(&["contained:backward", "aggregates:backward"]),
                    ),
                    ("compared_property", property(Some("Pset"), "System")),
                    ("target_property", property(Some("Pset"), "System")),
                ],
            ),
        ),
    );
    // a1 and a2 share b1 across storeys and a space; a3 is alone in b2.
    let mut found = findings(&evaluation);
    found.sort();
    assert_eq!(
        found,
        [
            (
                "a1".into(),
                "candidate test:model/a2 does not satisfy comparison".into()
            ),
            (
                "a2".into(),
                "candidate test:model/a1 does not satisfy comparison".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_sum_is_judged_against_both_bounds_of_a_quantity_range() {
    let area = |value| PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    };
    let model = spaces()
        .value("d1", "Q", "Area", area(2.0))
        .value("d2", "Q", "Area", area(2.5))
        .value("d3", "Q", "Area", area(0.5));
    let evaluation = model.evaluate(
        &PropertyComparison,
        &rule(
            ID,
            kind("space"),
            with(
                base("door", "related", "sum", "between"),
                vec![
                    ("relationship", string("contained")),
                    ("compared_property", property(Some("Q"), "Area")),
                    (
                        "minimum_quantity",
                        ParameterValue::Quantity {
                            value: 1.0,
                            unit: "m2".into(),
                        },
                    ),
                    (
                        "maximum_quantity",
                        ParameterValue::Quantity {
                            value: 40_000.0,
                            unit: "cm2".into(),
                        },
                    ),
                ],
            ),
        ),
    );
    // s1 sums to 4.5 m² (above 4 m²), s2 to 0.5 m² (below 1 m²).
    assert_eq!(
        findings(&evaluation),
        [
            (
                "s1".into(),
                "sum of compared values is 4.5 m² and is not between 1 m² and 4 m²".into()
            ),
            (
                "s2".into(),
                "sum of compared values is 0.5 m² and is not between 1 m² and 4 m²".into()
            ),
        ]
    );
}

#[test]
fn a_range_needs_both_bounds_in_order_and_only_between_takes_it() {
    let cases: [Vec<(&str, ParameterValue)>; 3] = [
        vec![("maximum_number", number(0.0))],
        vec![("minimum_number", number(3.0))],
        vec![("operator", string("less"))],
    ];
    for case in cases {
        let mut declared = rule(ID, kind("sensor"), doors_per_space(vec![]));
        declared.parameters.remove("maximum_number");
        declared
            .parameters
            .insert("maximum_number".into(), number(2.0));
        for (name, value) in case {
            declared.parameters.insert(name.into(), value);
        }
        let evaluation = spaces().evaluate(&PropertyComparison, &declared);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
        );
    }
    // Only one bound: a range needs both.
    let mut one_bound = rule(ID, kind("sensor"), doors_per_space(vec![]));
    one_bound.parameters.remove("maximum_number");
    let evaluation = spaces().evaluate(&PropertyComparison, &one_bound);
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn findings_are_categorised_by_a_property_of_the_checked_object() {
    let rule = rule(
        ID,
        kind("sensor"),
        doors_per_space(vec![(
            "category_property",
            property(Some("Pset"), "Discipline"),
        )]),
    );
    let evaluation = spaces()
        .text("x1", "Pset", "Discipline", " Fire ")
        .evaluate(&PropertyComparison, &rule);
    // x3 states no category, so its finding has none.
    assert_eq!(
        findings(&evaluation),
        [
            (
                "x1".into(),
                "[Fire] count of compared components is 2 and is not between 1 and 1".into()
            ),
            (
                "x3".into(),
                "count of compared components is 0 and is not between 1 and 1".into()
            ),
        ]
    );
    let categorised = &evaluation.findings()[0];
    assert!(
        categorised
            .evidence
            .iter()
            .any(|evidence| evidence.locator.ends_with("Pset.Discipline")),
        "{:?}",
        categorised.evidence
    );
    // A category that cannot be read leaves the object not evaluated; one
    // with no finding never reads it.
    let evaluation = spaces()
        .unreadable("x1")
        .unreadable("x2")
        .evaluate(&PropertyComparison, &rule);
    assert_eq!(
        findings(&evaluation),
        [(
            "x3".into(),
            "count of compared components is 0 and is not between 1 and 1".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("x1".into(), NotEvaluatedReason::BackendUnavailable)]
    );
}
