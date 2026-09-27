//! Semantic capabilities over exact properties and relationships, no geometry.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::{
    ConsistentValue, ManualIssue, NameSequence, NumberingConsistency, RelatedCount, RelativeCount,
    SelectorConformance, UniqueValue, register_builtins,
};
use common::{
    Model, assert_deviation, boolean, deviation_of, findings, flagged, integer, kind, property,
    rule, selector, string, unevaluated,
};

const ATTR: &str = axioval_ir::ATTRIBUTE_SET;

fn matches(set: &str, name: &str, pattern: &str) -> Selector {
    Selector::Property {
        property_set: Some(set.into()),
        property: name.into(),
        operator: ComparisonOperator::Matches,
        value: Some(string(pattern)),
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
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
            [(
                "s2".into(),
                "unknown space: axioval:attributes.LongName `Office`, axioval:attributes.Name `201`"
                    .into()
            )]
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("s4".to_owned(), NotEvaluatedReason::BackendUnavailable)]
        );
    }

    #[test]
    fn no_value_is_its_own_result_and_unknown_values_are_grouped() {
        let model = spaces()
            .object("s5", "space")
            .object("s6", "space")
            .object("s7", "space")
            .text("s5", ATTR, "LongName", "Kitchen")
            .text("s6", ATTR, "LongName", " ")
            .text("s7", ATTR, "LongName", "kitchen");
        let evaluation = model.evaluate(
            &SelectorConformance,
            &rule(ID, kind("space"), vec![("requirement", agreed())]),
        );
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "s6".into(),
                    "axioval:attributes.LongName, axioval:attributes.Name has no value to \
                     compare with the agreed list"
                        .into()
                ),
                (
                    "s4".into(),
                    "does not match any agreed combination of values: \
                     axioval:attributes.LongName `Kitchen`, axioval:attributes.Name absent"
                        .into()
                ),
                (
                    "s2".into(),
                    "does not match any agreed combination of values: \
                     axioval:attributes.LongName `Office`, axioval:attributes.Name `201`"
                        .into()
                ),
                (
                    "s7".into(),
                    "does not match any agreed combination of values: \
                     axioval:attributes.LongName `kitchen`, axioval:attributes.Name absent"
                        .into()
                ),
            ]
        );
        // One finding per unknown value, relating every object that holds it.
        let kitchen = &evaluation.findings()[1];
        assert_eq!(kitchen.related.len(), 1);
        assert_eq!(kitchen.related[0].local_id, "s5");
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
            .find(|finding| finding.object_id().unwrap().local_id == "s1")
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
        // T1 disagrees; T2 spans two kinds and is not compared; d5 has no
        // mark, but no other unmarked door disagrees with it.
        assert_eq!(flagged(&evaluation), ["d1", "d2", "d3"]);
        let d3 = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id().unwrap().local_id == "d3")
            .unwrap();
        assert_eq!(
            d3.message,
            "Pset.FireRating is `F90` where other objects with Pset.Mark `T1` have `F30`"
        );
        assert_eq!(d3.related.len(), 2);
    }

    #[test]
    fn a_missing_key_is_reported_only_when_unkeyed_objects_conflict() {
        let rule = rule(
            ID,
            Selector::All,
            vec![
                ("key", property(Some("Pset"), "Mark")),
                ("value", property(Some("Pset"), "FireRating")),
            ],
        );
        let agreeing = Model::default()
            .object("d1", "door")
            .object("d2", "door")
            .text("d1", "Pset", "FireRating", "F30")
            .text("d2", "Pset", "Mark", " ")
            .text("d2", "Pset", "FireRating", "F30");
        let evaluation = agreeing.evaluate(&ConsistentValue, &rule);
        assert!(
            evaluation.findings().is_empty(),
            "{:?}",
            findings(&evaluation)
        );
        assert!(evaluation.not_evaluated_outcomes().is_empty());

        let conflicting = Model::default()
            .object("d1", "door")
            .object("d2", "door")
            .object("d3", "door")
            .text("d1", "Pset", "FireRating", "F30")
            .text("d2", "Pset", "FireRating", "F90")
            .text("d3", "Pset", "Mark", "T1")
            .text("d3", "Pset", "FireRating", "F60");
        let evaluation = conflicting.evaluate(&ConsistentValue, &rule);
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "d1".into(),
                    "Pset.Mark has no value, and Pset.FireRating is `F30` where other \
                     objects without Pset.Mark have `F90`"
                        .into()
                ),
                (
                    "d2".into(),
                    "Pset.Mark has no value, and Pset.FireRating is `F90` where other \
                     objects without Pset.Mark have `F30`"
                        .into()
                ),
            ]
        );
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
        assert_deviation(deviation_of(&evaluation, "2 related"), (1.0, 1.0));
        assert_deviation(deviation_of(&evaluation, "0 related"), (1.0, 1.0));
    }

    #[test]
    fn an_undecided_member_matters_only_when_it_could_change_the_verdict() {
        let model = rooms().unreadable("d1").unreadable("d2");
        let fire_doors = Selector::Property {
            property_set: Some("Pset".into()),
            property: "FireRated".into(),
            operator: ComparisonOperator::Equals,
            value: Some(boolean(true)),
            case_sensitive: true,
            trim: false,
            quantifier: None,
            precision: None,
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
                            case_sensitive: true,
                            trim: false,
                            quantifier: None,
                            precision: None,
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

    /// Revolving doors `rv1` (lobby to street) and `rv2` (lobby to hall),
    /// swing doors `s1` (lobby to street) and `s2` (lobby to office), each
    /// door reaching its spaces through `adjacent`.
    fn entrances() -> Model {
        let mut model = Model::default();
        for space in ["lobby", "street", "hall", "office"] {
            model = model.object(space, "space");
        }
        for (door, operation, spaces) in [
            ("rv1", "REVOLVING", ["lobby", "street"]),
            ("rv2", "REVOLVING", ["lobby", "hall"]),
            ("s1", "SWING", ["lobby", "street"]),
            ("s2", "SWING", ["lobby", "office"]),
        ] {
            model = model
                .object(door, "door")
                .text(door, "Pset", "Operation", operation);
            for space in spaces {
                model = model.edge("adjacent", door, space);
            }
        }
        model
    }

    /// With `same_ends`, only a related object between the same spaces
    /// counts: a revolving door whose only swing door leads elsewhere is a
    /// finding.
    #[test]
    fn a_revolving_door_needs_a_swing_door_between_the_same_spaces() {
        let parameters = |ends: bool| {
            let mut parameters = vec![
                (
                    "related_selector",
                    selector(matches("Pset", "Operation", "SWING")),
                ),
                (
                    "path",
                    common::strings(&["adjacent:forward", "adjacent:backward"]),
                ),
                ("minimum", integer(1)),
            ];
            if ends {
                parameters.push(("same_ends", common::strings(&["adjacent:forward"])));
            }
            parameters
        };
        let evaluation = entrances().evaluate(
            &RelatedCount,
            &rule(
                ID,
                matches("Pset", "Operation", "REVOLVING"),
                parameters(true),
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "rv2".into(),
                "0 related object(s) via adjacent then adjacent with the same ends via adjacent; \
                 required at least 1"
                    .into()
            )]
        );
        assert!(unevaluated(&evaluation).is_empty());
        // Without it, any swing door of the lobby would do.
        let evaluation = entrances().evaluate(
            &RelatedCount,
            &rule(
                ID,
                matches("Pset", "Operation", "REVOLVING"),
                parameters(false),
            ),
        );
        assert!(findings(&evaluation).is_empty());
        // A door between the same spaces the selection cannot decide may be
        // the one required.
        let model = entrances()
            .object("s3", "door")
            .edge("adjacent", "s3", "lobby")
            .edge("adjacent", "s3", "hall")
            .unreadable("s3");
        let evaluation = model.evaluate(
            &RelatedCount,
            &rule(
                ID,
                matches("Pset", "Operation", "REVOLVING"),
                parameters(true),
            ),
        );
        assert!(
            findings(&evaluation).is_empty(),
            "{:?}",
            findings(&evaluation)
        );
        // s3 itself may be a revolving door too.
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("s3".to_owned(), NotEvaluatedReason::BackendUnavailable),
                ("rv2".to_owned(), NotEvaluatedReason::IncompleteEvidence)
            ]
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
    fn a_number_below_the_start_is_separate_from_an_order_break() {
        let evaluation = check(building(&[
            ("a", "1", Some(0.0)),
            ("b1", "0", Some(1.0)),
            ("c", "2", Some(2.0)),
            ("d", "1", Some(3.0)),
        ]));
        // `0` does not become the member below `2`, which follows `1`.
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "b1".into(),
                    "axioval:attributes.Name 0 is below the start 1".into()
                ),
                (
                    "d".into(),
                    "axioval:attributes.Name 1 is not above 2, the member below it".into()
                ),
            ]
        );
        assert!(evaluation.findings()[0].related.is_empty());
        assert_eq!(evaluation.findings()[1].related[0].local_id, "c");
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

mod numbering {
    use super::*;

    const ID: &str = "axioval:capability.numbering-consistency";

    /// Storey 1: B-101, B-102, B-104, a lobby and a bare `101`; storey 2:
    /// B-201, B-202, B-301.
    fn spaces() -> Model {
        let mut model = Model::default()
            .object("st1", "storey")
            .object("st2", "storey");
        for (storey, space, name) in [
            ("st1", "s1", "B-101"),
            ("st1", "s2", "B-102"),
            ("st1", "s3", "B-104"),
            ("st1", "s4", "B-Lobby"),
            ("st1", "s5", "101"),
            ("st2", "s6", "B-201"),
            ("st2", "s7", "B-202"),
            ("st2", "s8", "B-301"),
        ] {
            model = model
                .object(space, "space")
                .edge("aggregates", storey, space)
                .text(space, ATTR, "Name", name);
        }
        model
    }

    fn check(
        model: Model,
        extra: Vec<(&str, ParameterValue)>,
    ) -> axioval_engine::CapabilityEvaluation {
        let mut parameters = vec![
            ("property", property(Some(ATTR), "Name")),
            ("pattern", string(r"B-(\d+)")),
            ("relationship", string("aggregates")),
            ("direction", string("backward")),
        ];
        parameters.extend(extra);
        model.evaluate(&NumberingConsistency, &rule(ID, kind("space"), parameters))
    }

    #[test]
    fn a_gap_and_a_different_prefix_are_reported_per_storey() {
        let evaluation = check(
            spaces(),
            vec![("prefix_length", integer(1)), ("gap_free", boolean(true))],
        );
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "s3".into(),
                    "axioval:attributes.Name `B-104` follows 102; 103 is missing".into()
                ),
                (
                    "s8".into(),
                    "axioval:attributes.Name `B-301` does not start with 2, the prefix of 2 other object(s)"
                        .into()
                ),
                (
                    "s8".into(),
                    "axioval:attributes.Name `B-301` follows 202; 203 to 300 are missing".into()
                ),
            ]
        );
        // The gap names the object below it.
        assert_eq!(evaluation.findings()[0].related[0].local_id, "s2");
        // Values the pattern does not number are not evaluated, never passed.
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("s4".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("s5".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ]
        );
    }

    #[test]
    fn each_check_runs_only_when_declared() {
        let prefix = check(spaces(), vec![("prefix_length", integer(1))]);
        assert_eq!(flagged(&prefix), ["s8"]);
        let gaps = check(spaces(), vec![("gap_free", boolean(true))]);
        assert_eq!(flagged(&gaps), ["s3", "s8"]);
    }

    #[test]
    fn without_a_predominant_prefix_every_object_is_reported() {
        let model = spaces()
            .object("s9", "space")
            .edge("aggregates", "st2", "s9")
            .text("s9", ATTR, "Name", "B-302");
        let evaluation = check(model, vec![("prefix_length", integer(1))]);
        assert_eq!(flagged(&evaluation), ["s6", "s7", "s8", "s9"]);
        assert!(evaluation.findings()[0].message.contains("2 (2), 3 (2)"));
    }

    #[test]
    fn an_unreadable_number_withholds_the_gap_it_could_fill() {
        let evaluation = check(spaces().unreadable("s2"), vec![("gap_free", boolean(true))]);
        assert!(
            evaluation
                .findings()
                .iter()
                .all(|finding| finding.object_id().unwrap().local_id != "s3"),
            "{:?}",
            findings(&evaluation)
        );
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("s2".to_owned(), NotEvaluatedReason::BackendUnavailable),
                ("s4".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("s5".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("s3".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ]
        );
        // Storey 2 is unaffected.
        assert_eq!(flagged(&evaluation), ["s8"]);
    }

    #[test]
    fn a_missing_service_leaves_every_object_unevaluated() {
        use axioval_engine::RuleCapability;
        let project = axioval_ir::Project::new(vec![
            axioval_ir::Object::new(common::id("s1"), "space"),
            axioval_ir::Object::new(common::id("s2"), "space"),
        ])
        .unwrap();
        let services = axioval_engine::ServiceRegistry::new();
        let evaluation = NumberingConsistency.evaluate(
            &axioval_engine::RuleContext {
                project: &project,
                services: &services,
            },
            &rule(
                ID,
                kind("space"),
                vec![
                    ("property", property(Some(ATTR), "Name")),
                    ("pattern", string(r"B-(\d+)")),
                    ("gap_free", boolean(true)),
                ],
            ),
        );
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("s1".to_owned(), NotEvaluatedReason::MissingService),
                ("s2".to_owned(), NotEvaluatedReason::MissingService),
            ]
        );
    }

    #[test]
    fn declarations_that_check_nothing_or_capture_no_number_are_refused() {
        for extra in [
            vec![],
            vec![("gap_free", boolean(false))],
            vec![("gap_free", boolean(true)), ("pattern", string(r"B-\d+"))],
            vec![
                ("gap_free", boolean(true)),
                ("pattern", string(r"(B)-(\d+)")),
            ],
            vec![("prefix_length", integer(0))],
            vec![("gap_free", boolean(true)), ("pattern", string("["))],
        ] {
            let evaluation = check(spaces(), extra);
            assert!(evaluation.findings().is_empty());
            assert_eq!(
                unevaluated(&evaluation),
                [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
            );
        }
    }
}

mod manual {
    use super::*;

    #[test]
    fn one_finding_names_every_selected_object() {
        let model = Model::default()
            .object("x", "stair")
            .object("y", "wall")
            .object("z", "stair");
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
        assert_eq!(evaluation.findings()[0].related.len(), 1);
        assert_eq!(evaluation.findings()[0].related[0].local_id, "z");
    }

    /// The check is owed even when nothing matches: it is raised once for
    /// the project, never silently dropped.
    #[test]
    fn an_empty_selection_still_owes_the_check_at_project_level() {
        let model = Model::default().object("y", "wall");
        let evaluation = model.evaluate(
            &ManualIssue,
            &rule(
                "axioval:capability.manual-issue",
                kind("stair"),
                vec![("title", string("Check handrail height"))],
            ),
        );
        assert_eq!(evaluation.findings().len(), 1);
        let finding = &evaluation.findings()[0];
        assert_eq!(finding.scope, axioval_ir::Scope::Project);
        assert_eq!(
            finding.message,
            "Check handrail height (no object matches the selection)"
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
        "numbering-consistency",
        "manual-issue",
    ] {
        assert!(
            registry.get(&format!("axioval:capability.{id}")).is_some(),
            "{id}"
        );
    }
}
