//! Semantic capabilities over exact properties and relationships, no geometry.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::{
    ConsistentValue, ManualIssue, NameSequence, RelatedCount, RelativeCount, SelectorConformance,
    UniqueValue, register_builtins,
};
use common::{
    Model, boolean, findings, flagged, integer, kind, property, rule, selector, string, unevaluated,
};

const ATTR: &str = axioval_ir::ATTRIBUTE_SET;

fn matches(set: &str, name: &str, pattern: &str) -> Selector {
    Selector::Property {
        property_set: Some(set.into()),
        property: name.into(),
        operator: ComparisonOperator::Matches,
        value: Some(string(pattern)),
    }
}

fn all_of(operands: Vec<Selector>) -> Selector {
    Selector::AllOf { operands }
}

mod conformance {
    use super::*;

    const ID: &str = "axioval:capability.selector-conformance";

    /// Agreed rows: offices numbered `1xx`, corridors with any number.
    fn agreed() -> ParameterValue {
        selector(Selector::AnyOf {
            operands: vec![
                all_of(vec![
                    matches(ATTR, "LongName", "^(?i)office$"),
                    matches(ATTR, "Name", "^1\\d\\d$"),
                ]),
                matches(ATTR, "LongName", "^(?i)corridor$"),
            ],
        })
    }

    fn spaces() -> Model {
        Model::default()
            .object("s1", "space")
            .object("s2", "space")
            .object("s3", "space")
            .object("s4", "space")
            .text("s1", ATTR, "LongName", "Office")
            .text("s1", ATTR, "Name", "101")
            .text("s2", ATTR, "LongName", "Office")
            .text("s2", ATTR, "Name", "201")
            .text("s3", ATTR, "LongName", "Corridor")
            .text("s4", ATTR, "LongName", "Kitchen")
    }

    #[test]
    fn an_object_passes_when_one_agreed_row_matches_it_whole() {
        let evaluation = spaces().evaluate(
            &SelectorConformance,
            &rule(ID, kind("space"), vec![("requirement", agreed())]),
        );
        assert_eq!(flagged(&evaluation), ["s2", "s4"]);
        // The finding cites the values that were consulted.
        let s2 = &evaluation.findings()[0];
        assert!(
            s2.evidence
                .iter()
                .any(|evidence| evidence.locator.ends_with("Name")),
            "{:?}",
            s2.evidence
        );
    }

    #[test]
    fn an_undecidable_requirement_is_not_a_violation() {
        let evaluation = spaces().unreadable("s4").evaluate(
            &SelectorConformance,
            &rule(
                ID,
                kind("space"),
                vec![
                    ("requirement", agreed()),
                    ("message", string("unknown space")),
                ],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [("s2".into(), "unknown space".into())]
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("s4".to_owned(), NotEvaluatedReason::BackendUnavailable)]
        );
    }
}

mod unique {
    use super::*;

    const ID: &str = "axioval:capability.unique-value";

    fn spaces() -> Model {
        Model::default()
            .object("st1", "storey")
            .object("st2", "storey")
            .object("s1", "space")
            .object("s2", "space")
            .object("s3", "space")
            .object("s4", "space")
            .object("s5", "space")
            .object("s6", "space")
            .edge("aggregates", "st1", "s1")
            .edge("aggregates", "st1", "s2")
            .edge("aggregates", "st2", "s3")
            .edge("aggregates", "st2", "s4")
            .edge("aggregates", "st2", "s5")
            .edge("aggregates", "st2", "s6")
            .text("s1", ATTR, "Name", "101")
            .text("s2", ATTR, "Name", " 101 ")
            .text("s3", ATTR, "Name", "A1")
            .text("s4", ATTR, "Name", "a1")
            .text("s5", ATTR, "Name", "  ")
            .text("s6", ATTR, "Name", "101")
    }

    fn check(extra: Vec<(&str, ParameterValue)>) -> axioval_engine::CapabilityEvaluation {
        let mut parameters = vec![("property", property(Some(ATTR), "Name"))];
        parameters.extend(extra);
        spaces().evaluate(&UniqueValue, &rule(ID, kind("space"), parameters))
    }

    #[test]
    fn values_repeat_after_trimming_and_without_regard_to_case() {
        let evaluation = check(vec![]);
        assert_eq!(flagged(&evaluation), ["s1", "s2", "s3", "s4", "s5", "s6"]);
        let s1 = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id.local_id == "s1")
            .unwrap();
        assert_eq!(
            s1.message,
            "axioval:attributes.Name `101` is also used by 2 other object(s)"
        );
        assert_eq!(s1.related.len(), 2);
    }

    #[test]
    fn declared_strictness_and_optional_values_are_honoured() {
        let evaluation = check(vec![
            ("trim", boolean(false)),
            ("case_sensitive", boolean(true)),
            ("require_value", boolean(false)),
        ]);
        assert_eq!(flagged(&evaluation), ["s1", "s6"]);
    }

    #[test]
    fn a_relationship_scopes_uniqueness_to_one_storey() {
        let evaluation = check(vec![
            ("relationship", string("aggregates")),
            ("direction", string("backward")),
        ]);
        // 101 repeats on st1, A1/a1 on st2; s6's 101 is on another storey.
        assert_eq!(flagged(&evaluation), ["s1", "s2", "s3", "s4", "s5"]);
    }
}

mod consistent {
    use super::*;

    const ID: &str = "axioval:capability.consistent-value";

    #[test]
    fn members_of_one_key_that_disagree_are_each_reported() {
        let model = Model::default()
            .object("d1", "door")
            .object("d2", "door")
            .object("d3", "door")
            .object("d4", "door")
            .object("d5", "door")
            .object("w1", "window")
            .text("d1", "Pset", "Mark", "T1")
            .text("d2", "Pset", "Mark", "t1")
            .text("d3", "Pset", "Mark", "T1")
            .text("d4", "Pset", "Mark", "T2")
            .text("w1", "Pset", "Mark", "T2")
            .text("d1", "Pset", "FireRating", "F30")
            .text("d2", "Pset", "FireRating", "F30")
            .text("d3", "Pset", "FireRating", "F90")
            .text("d4", "Pset", "FireRating", "F30")
            .text("w1", "Pset", "FireRating", "F90");
        let evaluation = model.evaluate(
            &ConsistentValue,
            &rule(
                ID,
                Selector::All,
                vec![
                    ("key", property(Some("Pset"), "Mark")),
                    ("value", property(Some("Pset"), "FireRating")),
                ],
            ),
        );
        // T1 disagrees; T2 spans two kinds and is not compared; d5 has no mark.
        assert_eq!(flagged(&evaluation), ["d1", "d2", "d3", "d5"]);
        let d3 = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id.local_id == "d3")
            .unwrap();
        assert_eq!(
            d3.message,
            "Pset.FireRating is `F90` where other objects with Pset.Mark `T1` have `F30`"
        );
        assert_eq!(d3.related.len(), 2);
    }
}

mod related_count {
    use super::*;

    const ID: &str = "axioval:capability.related-count";

    fn rooms() -> Model {
        Model::default()
            .object("r1", "room")
            .object("r2", "room")
            .object("r3", "room")
            .object("d1", "door")
            .object("d2", "door")
            .object("d3", "door")
            .object("w1", "window")
            .edge("bounds", "r1", "d1")
            .edge("bounds", "r2", "d2")
            .edge("bounds", "r2", "d3")
            .edge("bounds", "r2", "w1")
    }

    fn doors(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
        let mut parameters = vec![
            ("related_selector", selector(kind("door"))),
            ("relationship", string("bounds")),
        ];
        parameters.extend(extra);
        parameters
    }

    #[test]
    fn bounds_are_inclusive_and_only_selected_objects_count() {
        let evaluation = rooms().evaluate(
            &RelatedCount,
            &rule(
                ID,
                kind("room"),
                doors(vec![("minimum", integer(1)), ("maximum", integer(1))]),
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "r2".into(),
                    "2 related object(s) via bounds; required between 1 and 1".into()
                ),
                (
                    "r3".into(),
                    "0 related object(s) via bounds; required between 1 and 1".into()
                ),
            ]
        );
        assert_eq!(evaluation.findings()[0].related.len(), 2);
    }

    #[test]
    fn an_undecided_member_matters_only_when_it_could_change_the_verdict() {
        let model = rooms().unreadable("d1").unreadable("d2");
        let fire_doors = Selector::Property {
            property_set: Some("Pset".into()),
            property: "FireRated".into(),
            operator: ComparisonOperator::Equals,
            value: Some(boolean(true)),
        };
        let evaluation = model.evaluate(
            &RelatedCount,
            &rule(
                ID,
                kind("room"),
                vec![
                    ("related_selector", selector(fire_doors)),
                    ("relationship", string("bounds")),
                    ("maximum", integer(1)),
                ],
            ),
        );
        // r2 holds d2 (unknown) and d3 (not fire rated): at most 1 either way.
        // r1 holds only d1 (unknown): at most 1 either way. Nothing to report.
        assert!(evaluation.findings().is_empty());
        assert!(evaluation.not_evaluated_outcomes().is_empty());
        let minimum = rooms().unreadable("d1").evaluate(
            &RelatedCount,
            &rule(
                ID,
                kind("room"),
                vec![
                    (
                        "related_selector",
                        selector(Selector::Property {
                            property_set: Some("Pset".into()),
                            property: "FireRated".into(),
                            operator: ComparisonOperator::Exists,
                            value: None,
                        }),
                    ),
                    ("relationship", string("bounds")),
                    ("minimum", integer(1)),
                ],
            ),
        );
        // r1's only candidate is unknown: it might be the one required.
        assert_eq!(flagged(&minimum), ["r2", "r3"]);
        assert_eq!(
            unevaluated(&minimum),
            [("r1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }

    #[test]
    fn without_a_relationship_the_anchor_counts_its_whole_source() {
        let model = Model::default()
            .object("b", "building")
            .object("s1", "slab");
        let evaluation = model.evaluate(
            &RelatedCount,
            &rule(
                ID,
                kind("building"),
                vec![
                    ("related_selector", selector(kind("wall"))),
                    ("minimum", integer(1)),
                ],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "b".into(),
                "0 related object(s) in the same source; required at least 1".into()
            )]
        );
    }

    #[test]
    fn a_bound_is_required() {
        let evaluation = rooms().evaluate(&RelatedCount, &rule(ID, kind("room"), doors(vec![])));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

mod relative_count {
    use super::*;

    const ID: &str = "axioval:capability.relative-count";

    #[test]
    fn a_ratio_is_checked_per_anchor_in_exact_integers() {
        // One table per four chairs, at least.
        let mut model = Model::default()
            .object("st1", "storey")
            .object("st2", "storey");
        for (storey, chairs, tables) in [("st1", 8, 2), ("st2", 9, 2)] {
            for index in 0..chairs {
                let chair = format!("{storey}-c{index}");
                model = model
                    .object(&chair, "chair")
                    .edge("contains", storey, &chair);
            }
            for index in 0..tables {
                let table = format!("{storey}-t{index}");
                model = model
                    .object(&table, "table")
                    .edge("contains", storey, &table);
            }
        }
        let evaluation = model.evaluate(
            &RelativeCount,
            &rule(
                ID,
                kind("storey"),
                vec![
                    ("provided_selector", selector(kind("table"))),
                    ("required_selector", selector(kind("chair"))),
                    ("provided_unit", integer(1)),
                    ("required_unit", integer(4)),
                    ("operator", string("at_least")),
                    ("relationship", string("contains")),
                ],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "st2".into(),
                "2 provided and 9 required object(s) via contains; required 2/1 at_least 9/4"
                    .into()
            )]
        );
    }
}

mod name_sequence {
    use super::*;

    const ID: &str = "axioval:capability.name-sequence";

    fn building(storeys: &[(&str, &str, Option<f64>)]) -> Model {
        let mut model = Model::default().object("b", "building");
        for (local, name, elevation) in storeys {
            model = model
                .object(local, "storey")
                .edge("aggregates", "b", local)
                .text(local, ATTR, "Name", name);
            if let Some(elevation) = elevation {
                model = model.value(
                    local,
                    "Levels",
                    "Elevation",
                    PropertyValue::Decimal(*elevation),
                );
            }
        }
        model
    }

    fn check(model: Model) -> axioval_engine::CapabilityEvaluation {
        model.evaluate(
            &NameSequence,
            &rule(
                ID,
                kind("building"),
                vec![
                    ("member_selector", selector(kind("storey"))),
                    ("name", property(Some(ATTR), "Name")),
                    ("order", property(Some("Levels"), "Elevation")),
                    ("relationship", string("aggregates")),
                ],
            ),
        )
    }

    #[test]
    fn names_count_up_from_the_lowest_member() {
        let evaluation = check(building(&[
            ("g", "1", Some(0.0)),
            ("u", "3", Some(6.0)),
            ("m", "2", Some(3.0)),
        ]));
        assert!(
            evaluation.findings().is_empty(),
            "{:?}",
            findings(&evaluation)
        );
    }

    #[test]
    fn each_break_in_the_sequence_is_named() {
        let evaluation = check(building(&[
            ("a", "2", Some(0.0)),
            ("b1", "EG", Some(1.0)),
            ("c", "3", Some(2.0)),
            ("d", "3", Some(3.0)),
            ("e", "5", Some(4.0)),
            ("f", " 6", Some(5.0)),
            ("g", "", Some(6.0)),
        ]));
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "a".into(),
                    "axioval:attributes.Name of the first member is 2; expected 1".into()
                ),
                (
                    "b1".into(),
                    "axioval:attributes.Name `EG` is not a whole number".into()
                ),
                (
                    "d".into(),
                    "axioval:attributes.Name 3 is not above 3, the member below it".into()
                ),
                (
                    "e".into(),
                    "axioval:attributes.Name 5 does not follow 3; expected 4".into()
                ),
                (
                    "f".into(),
                    "axioval:attributes.Name ` 6` is not a whole number".into()
                ),
                ("g".into(), "axioval:attributes.Name is not set".into()),
            ]
        );
        // The sequence break names the member below.
        assert_eq!(evaluation.findings()[3].related[0].local_id, "d");
    }

    #[test]
    fn a_member_without_an_order_value_leaves_the_anchor_unordered() {
        let evaluation = check(building(&[("a", "1", Some(0.0)), ("b1", "2", None)]));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("b".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }
}

mod manual {
    use super::*;

    #[test]
    fn every_selected_object_carries_the_instruction() {
        let model = Model::default().object("x", "stair").object("y", "wall");
        let evaluation = model.evaluate(
            &ManualIssue,
            &rule(
                "axioval:capability.manual-issue",
                kind("stair"),
                vec![
                    ("title", string("Check handrail height")),
                    ("category", string("Safety")),
                    ("description", string("Measure on site.")),
                ],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "x".into(),
                "Safety: Check handrail height: Measure on site.".into()
            )]
        );
    }
}

#[test]
fn every_new_capability_is_a_builtin() {
    let registry = register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    for id in [
        "selector-conformance",
        "unique-value",
        "consistent-value",
        "related-count",
        "relative-count",
        "name-sequence",
        "manual-issue",
    ] {
        assert!(
            registry.get(&format!("axioval:capability.{id}")).is_some(),
            "{id}"
        );
    }
}
