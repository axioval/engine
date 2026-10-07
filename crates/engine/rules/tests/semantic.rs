//! Semantic capabilities over exact properties and relationships, no geometry.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    MetricDirection, MetricFrame, MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService,
    ObjectFrameServiceHandle, ObjectFront, SourceSnapshot,
};
use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use axioval_rules::{
    ConsistentValue, ManualIssue, NameSequence, NumberingConsistency, RelativeCount,
    SelectorConformance, register_builtins,
};
use common::{
    Model, assert_deviation, boolean, deviation_of, findings, flagged, integer, kind, number,
    property, rule, selector, string, unevaluated,
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

    /// `selector-conformance` runs as a template, held on every fixture to
    /// the implementation it replaced.
    const CONFORMANCE: common::Held = common::Held(
        &SelectorConformance,
        &axioval_rules::reference::SelectorConformance,
    );

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
            &CONFORMANCE,
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
            &CONFORMANCE,
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
            &CONFORMANCE,
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

    #[test]
    fn a_requirement_naming_no_property_reports_each_object_alone() {
        let evaluation = spaces().evaluate(
            &CONFORMANCE,
            &rule(
                ID,
                kind("space"),
                vec![("requirement", selector(kind("door")))],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            ["s1", "s2", "s3", "s4"].map(|space| (
                space.to_owned(),
                "does not match any agreed combination of values".to_owned()
            ))
        );
        let evaluation = spaces().evaluate(&CONFORMANCE, &rule(ID, kind("space"), vec![]));
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "selector-conformance: parameter `requirement` is required"
        );
        let evaluation = spaces().evaluate(
            &CONFORMANCE,
            &rule(
                ID,
                kind("space"),
                vec![("requirement", agreed()), ("message", boolean(true))],
            ),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }

    /// Generated spaces stating names and long names of every agreed and
    /// unknown kind, blank, `null` or something unreadable, judged against
    /// agreed lists of rows, negations, related selectors and expressions,
    /// with and without a message. The template is held to the
    /// implementation it replaced on each.
    mod generated {
        use super::*;
        use proptest::prelude::*;

        fn text(kind: u8) -> Option<PropertyValue> {
            Some(match kind {
                0 => PropertyValue::String("Office".into()),
                1 => PropertyValue::String("office".into()),
                2 => PropertyValue::String("Corridor".into()),
                3 => PropertyValue::String("Kitchen".into()),
                4 => PropertyValue::String("101".into()),
                5 => PropertyValue::String("201".into()),
                6 => PropertyValue::String(" ".into()),
                7 => PropertyValue::Null,
                8 => PropertyValue::Integer(101),
                _ => return None,
            })
        }

        fn requirement(which: u8) -> ParameterValue {
            match which {
                0 => agreed(),
                1 => selector(Selector::Not {
                    operand: Box::new(matches(ATTR, "LongName", "^(?i)kitchen$")),
                }),
                2 => selector(all_of(vec![
                    matches(ATTR, "Name", "^\\d+$"),
                    Selector::Related {
                        path: vec!["aggregates:backward".into()],
                        quantifier: axioval_ir::contract::RelatedQuantifier::Any,
                        selector: Box::new(kind("storey")),
                    },
                ])),
                _ => selector(Selector::Expression {
                    expression: Box::new(
                        serde_json::from_value(serde_json::json!({
                            "kind": "compare",
                            "operator": "equals",
                            "left": {"kind": "property", "propertySet": ATTR, "property": "LongName"},
                            "right": {"kind": "property", "propertySet": ATTR, "property": "Name"},
                        }))
                        .unwrap(),
                    ),
                }),
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn generated_spaces_hold_parity(
                spaces in proptest::collection::vec(
                    (0u8..11, 0u8..11, any::<bool>()),
                    0..8,
                ),
                agreed in 0u8..4,
                message in any::<bool>(),
            ) {
                let mut model = Model::default().object("st", "storey");
                for (index, (long, name, contained)) in spaces.iter().enumerate() {
                    let local = format!("s{index}");
                    model = model.object(&local, "space");
                    if *contained {
                        model = model.edge("aggregates", "st", &local);
                    }
                    if let Some(value) = text(*long) {
                        model = model.value(&local, ATTR, "LongName", value);
                    }
                    model = match text(*name) {
                        Some(value) => model.value(&local, ATTR, "Name", value),
                        None if *name == 10 => model.unreadable(&local),
                        None => model,
                    };
                }
                let mut parameters = vec![("requirement", requirement(agreed))];
                if message {
                    parameters.push(("message", string("not agreed")));
                }
                model.evaluate(&CONFORMANCE, &rule(ID, kind("space"), parameters));
            }
        }
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
        spaces().evaluate(
            &common::Held(
                &axioval_rules::UniqueValue,
                &axioval_rules::reference::UniqueValue,
            ),
            &rule(ID, kind("space"), parameters),
        )
    }

    /// A flag stated as another kind is refused, as the capability read
    /// it.
    #[test]
    fn a_flag_of_another_kind_is_refused() {
        let evaluation = check(vec![("trim", string("yes"))]);
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "unique-value: parameter `trim` has the wrong type"
        );
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

    /// A group decision compares objects with each other: an `expression`
    /// rule judges each on its own, so the rule is not forked.
    #[test]
    fn a_group_decision_is_not_forked() {
        use axioval_rules::templates::{ForkError, fork};
        let bound = rule(
            ID,
            kind("space"),
            vec![("property", property(Some(ATTR), "Name"))],
        );
        assert!(matches!(
            fork(&axioval_rules::UniqueValue, &bound),
            Err(ForkError::Inexpressible(_))
        ));
    }

    /// Generated spaces on two storeys in two sources, stating text,
    /// numbers, quantities, nothing, `null` or something unreadable,
    /// compared with every combination of the declared strictness, scope
    /// and tolerance. The template is held to the implementation it
    /// replaced on each.
    mod generated {
        use super::*;
        use axioval_ir::{ObjectId, QuantityDimension, SourceId};
        use proptest::prelude::*;

        fn value(kind: u8) -> Option<PropertyValue> {
            Some(match kind {
                0 => PropertyValue::String("101".into()),
                1 => PropertyValue::String(" 101".into()),
                2 => PropertyValue::String("A1".into()),
                3 => PropertyValue::String("a1".into()),
                4 => PropertyValue::String("  ".into()),
                5 => PropertyValue::Decimal(1.0),
                6 => PropertyValue::Decimal(1.04),
                7 => PropertyValue::Integer(1),
                8 => PropertyValue::Quantity {
                    value: 1.0,
                    dimension: QuantityDimension::Length,
                },
                9 => PropertyValue::Null,
                _ => return None,
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(192))]

            #[test]
            fn generated_groups_hold_parity(
                spaces in proptest::collection::vec((0u8..12, any::<bool>(), any::<bool>()), 0..7),
                trim in proptest::option::of(any::<bool>()),
                case_sensitive in proptest::option::of(any::<bool>()),
                require in proptest::option::of(any::<bool>()),
                across in proptest::option::of(any::<bool>()),
                along in any::<bool>(),
                tolerance in 0u8..4,
            ) {
                let mut model = Model::default()
                    .object("st1", "storey")
                    .object("st2", "storey");
                for (index, (kind, upper, other)) in spaces.iter().enumerate() {
                    let local = format!("s{index}");
                    model = if *other {
                        model.object_in("other", &local, "space")
                    } else {
                        model
                            .object(&local, "space")
                            .edge("aggregates", if *upper { "st2" } else { "st1" }, &local)
                    };
                    let id = if *other {
                        ObjectId::new(SourceId::new("test", "other").unwrap(), &local).unwrap()
                    } else {
                        common::id(&local)
                    };
                    model = match (value(*kind), *other) {
                        (Some(value), false) => model.value(&local, ATTR, "Name", value),
                        (None, _) if *kind == 11 => model.unreadable_object(id),
                        _ => model,
                    };
                }
                let mut parameters = vec![("property", property(Some(ATTR), "Name"))];
                for (name, flag) in [
                    ("trim", trim),
                    ("case_sensitive", case_sensitive),
                    ("require_value", require),
                    ("across_sources", across),
                ] {
                    if let Some(flag) = flag {
                        parameters.push((name, boolean(flag)));
                    }
                }
                if along {
                    parameters.push(("relationship", string("aggregates")));
                    parameters.push(("direction", string("backward")));
                }
                match tolerance {
                    1 => parameters.push(("tolerance", common::number(0.05))),
                    2 => parameters.push(("relative_tolerance", common::number(0.1))),
                    3 => parameters.push(("decimals", integer(1))),
                    _ => {}
                }
                model.evaluate(
                    &common::Held(
                        &axioval_rules::UniqueValue,
                        &axioval_rules::reference::UniqueValue,
                    ),
                    &rule(ID, kind("space"), parameters),
                );
            }
        }
    }
}

mod consistent {
    use super::*;

    const ID: &str = "axioval:capability.consistent-value";

    /// `consistent-value` runs as a template, held on every fixture to the
    /// implementation it replaced.
    const CONSISTENT: common::Held =
        common::Held(&ConsistentValue, &axioval_rules::reference::ConsistentValue);

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
            &CONSISTENT,
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
        let evaluation = agreeing.evaluate(&CONSISTENT, &rule);
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
        let evaluation = conflicting.evaluate(&CONSISTENT, &rule);
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

    fn metres(value: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value,
            dimension: axioval_ir::QuantityDimension::Length,
        }
    }

    fn millimetres(value: f64) -> ParameterValue {
        ParameterValue::Quantity {
            value,
            unit: "mm".into(),
        }
    }

    /// Walls of type `T1` of the given thicknesses, `w1` onwards.
    fn walls(thicknesses: &[PropertyValue]) -> Model {
        let mut model = Model::default();
        for (index, thickness) in thicknesses.iter().enumerate() {
            let local = format!("w{}", index + 1);
            model = model
                .object(&local, "wall")
                .text(&local, "Pset", "Type", "T1")
                .value(&local, "Pset", "Width", thickness.clone());
        }
        model
    }

    fn thickness(
        model: Model,
        tolerance: Option<(&str, ParameterValue)>,
    ) -> axioval_engine::CapabilityEvaluation {
        let mut parameters = vec![
            ("key", property(Some("Pset"), "Type")),
            ("value", property(Some("Pset"), "Width")),
        ];
        parameters.extend(tolerance);
        model.evaluate(&CONSISTENT, &rule(ID, kind("wall"), parameters))
    }

    #[test]
    fn values_within_the_tolerance_agree_and_without_it_disagree() {
        let close = || walls(&[metres(0.24), metres(0.2405)]);
        let within = thickness(close(), Some(("tolerance_quantity", millimetres(1.0))));
        assert!(within.findings().is_empty(), "{:?}", findings(&within));
        assert!(within.not_evaluated_outcomes().is_empty());
        assert_eq!(flagged(&thickness(close(), None)), ["w1", "w2"]);
        // A number tolerance reads quantities in SI units.
        let within = thickness(close(), Some(("tolerance", number(0.001))));
        assert!(within.findings().is_empty(), "{:?}", findings(&within));
        // Plain numbers take a number tolerance.
        let numbers = walls(&[PropertyValue::Decimal(1.0), PropertyValue::Integer(1)]);
        let within = thickness(numbers, Some(("tolerance", number(0.0))));
        assert!(within.findings().is_empty(), "{:?}", findings(&within));
    }

    #[test]
    fn an_outlier_beyond_the_tolerance_of_the_median_is_reported() {
        let model = walls(&[metres(0.24), metres(0.2405), metres(0.26)]);
        let evaluation = thickness(model, Some(("tolerance_quantity", millimetres(1.0))));
        assert_eq!(
            findings(&evaluation),
            [(
                "w3".into(),
                "Pset.Width is 0.26 m, farther than the tolerance 0.001 m from the median \
                 0.2405 m of the objects with Pset.Type `T1`"
                    .into()
            )]
        );
        assert_eq!(evaluation.findings()[0].related.len(), 2);
        assert!(evaluation.not_evaluated_outcomes().is_empty());
    }

    #[test]
    fn a_range_beyond_the_tolerance_with_no_outlier_reports_its_ends() {
        let model = walls(&[
            PropertyValue::Integer(0),
            PropertyValue::Integer(1),
            PropertyValue::Integer(2),
        ]);
        let evaluation = thickness(model, Some(("tolerance", number(1.0))));
        assert_eq!(flagged(&evaluation), ["w1", "w3"]);
        assert_eq!(
            evaluation.findings()[0].message,
            "Pset.Width is 0, at an end of the range 0 to 2 of Pset.Width over the objects \
             with Pset.Type `T1`, which exceeds the tolerance 1"
        );
    }

    #[test]
    fn a_measured_width_counts_whole_and_a_straddling_range_is_not_evaluated() {
        let measured = |lower, upper| PropertyValue::Measured {
            lower,
            upper,
            dimension: Some(axioval_ir::QuantityDimension::Length),
        };
        // 0.2395 to 0.2415 m may lie within 1 mm of 0.24 m, or not.
        let straddling = walls(&[metres(0.24), measured(0.2395, 0.2415)]);
        let evaluation = thickness(straddling, Some(("tolerance_quantity", millimetres(1.0))));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("w1".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("w2".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ]
        );
        // Wholly within, it agrees.
        let narrow = walls(&[metres(0.24), measured(0.2398, 0.2402)]);
        let evaluation = thickness(narrow, Some(("tolerance_quantity", millimetres(1.0))));
        assert!(evaluation.findings().is_empty());
        assert!(evaluation.not_evaluated_outcomes().is_empty());
    }

    #[test]
    fn a_tolerance_that_does_not_fit_is_refused() {
        // A length tolerance does not apply to an area.
        let area = PropertyValue::Quantity {
            value: 0.24,
            dimension: axioval_ir::QuantityDimension::Area,
        };
        let evaluation = thickness(
            walls(&[metres(0.24), area]),
            Some(("tolerance_quantity", millimetres(1.0))),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("w2".to_owned(), NotEvaluatedReason::InvalidEvidence)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: the tolerance 0.001 m does not apply to 0.24 m²"
        );
        let mut both = rule(
            ID,
            kind("wall"),
            vec![
                ("key", property(Some("Pset"), "Type")),
                ("value", property(Some("Pset"), "Width")),
                ("tolerance", number(0.001)),
                ("tolerance_quantity", millimetres(1.0)),
            ],
        );
        let evaluation = walls(&[metres(0.24)]).evaluate(&CONSISTENT, &both);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: declare either `tolerance` or `tolerance_quantity`, not both"
        );
        both.parameters.remove("tolerance_quantity");
        both.parameters.insert("tolerance".into(), number(-1.0));
        let evaluation = walls(&[metres(0.24)]).evaluate(&CONSISTENT, &both);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: the tolerance is negative"
        );
        both.parameters.remove("key");
        let evaluation = walls(&[metres(0.24)]).evaluate(&CONSISTENT, &both);
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: the tolerance is negative"
        );
        both.parameters.insert("tolerance".into(), number(1.0));
        let evaluation = walls(&[metres(0.24)]).evaluate(&CONSISTENT, &both);
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: parameter `key` is required"
        );
    }

    #[test]
    fn straddling_and_undecided_members_are_worded_as_before() {
        let measured = |lower, upper| PropertyValue::Measured {
            lower,
            upper,
            dimension: Some(axioval_ir::QuantityDimension::Length),
        };
        let straddling = walls(&[metres(0.24), measured(0.2395, 0.2415)]);
        let evaluation = thickness(straddling, Some(("tolerance_quantity", millimetres(1.0))));
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: the range of Pset.Width over the objects with Pset.Type `T1` \
             may lie on either side of the tolerance 0.001 m"
        );
        // w4 lies beyond the median's tolerance; w3 may or may not.
        let model = walls(&[
            metres(0.24),
            metres(0.24),
            measured(0.2405, 0.2425),
            metres(0.26),
        ]);
        let evaluation = thickness(model, Some(("tolerance_quantity", millimetres(1.0))));
        assert_eq!(
            findings(&evaluation),
            [(
                "w4".into(),
                "Pset.Width is 0.26 m, farther than the tolerance 0.001 m from the median \
                 0.24025 to 0.24125 m of the objects with Pset.Type `T1`"
                    .into()
            )]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "consistent-value: Pset.Width 0.24 m may lie within the tolerance 0.001 m of the \
             median 0.24025 to 0.24125 m of the objects with Pset.Type `T1` or beyond it"
        );
    }

    /// Generated walls of two types, some without one, in two sources and
    /// on two storeys, stating text, numbers, quantities, measured
    /// intervals, nothing, `null` or something unreadable, judged under
    /// every combination of strictness, scope, kind and tolerance. The
    /// template is held to the implementation it replaced on each.
    mod generated {
        use super::*;
        use axioval_ir::{ObjectId, QuantityDimension, SourceId};
        use proptest::prelude::*;

        fn value(kind: u8) -> Option<PropertyValue> {
            let length = |value| PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            };
            Some(match kind {
                0 => PropertyValue::String("F30".into()),
                1 => PropertyValue::String("f30".into()),
                2 => PropertyValue::String("F90".into()),
                3 => length(0.24),
                4 => length(0.2405),
                5 => length(0.26),
                6 => PropertyValue::Measured {
                    lower: 0.2395,
                    upper: 0.2415,
                    dimension: Some(QuantityDimension::Length),
                },
                7 => PropertyValue::Decimal(0.24),
                8 => PropertyValue::Integer(1),
                9 => PropertyValue::Quantity {
                    value: 0.24,
                    dimension: QuantityDimension::Area,
                },
                10 => PropertyValue::Null,
                11 => PropertyValue::String(" ".into()),
                _ => return None,
            })
        }

        fn key(kind: u8) -> Option<PropertyValue> {
            Some(match kind {
                0 => PropertyValue::String("T1".into()),
                1 => PropertyValue::String("t1".into()),
                2 => PropertyValue::String("T2".into()),
                3 => PropertyValue::String("  ".into()),
                4 => PropertyValue::Null,
                _ => return None,
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn generated_groups_hold_parity(
                walls in proptest::collection::vec(
                    (0u8..6, 0u8..14, any::<bool>(), any::<bool>(), any::<bool>()),
                    0..8,
                ),
                case_sensitive in proptest::option::of(any::<bool>()),
                same_kind in proptest::option::of(any::<bool>()),
                across in proptest::option::of(any::<bool>()),
                along in any::<bool>(),
                tolerance in 0u8..4,
            ) {
                let mut model = Model::default()
                    .object("st1", "storey")
                    .object("st2", "storey");
                for (index, (marked, stated, upper, other, door)) in walls.iter().enumerate() {
                    let local = format!("w{index}");
                    let kind = if *door { "door" } else { "wall" };
                    let id = if *other {
                        ObjectId::new(SourceId::new("test", "other").unwrap(), &local).unwrap()
                    } else {
                        common::id(&local)
                    };
                    model = if *other {
                        model.object_in("other", &local, kind)
                    } else {
                        model
                            .object(&local, kind)
                            .edge("aggregates", if *upper { "st2" } else { "st1" }, &local)
                    };
                    if let Some(key) = key(*marked) {
                        model = model.value_of(id.clone(), "Pset", "Type", key);
                    }
                    model = match value(*stated) {
                        Some(value) => model.value_of(id, "Pset", "Width", value),
                        None if *stated == 13 => model.unreadable_object(id),
                        None => model,
                    };
                }
                let mut parameters = vec![
                    ("key", property(Some("Pset"), "Type")),
                    ("value", property(Some("Pset"), "Width")),
                ];
                for (name, flag) in [
                    ("case_sensitive", case_sensitive),
                    ("same_kind", same_kind),
                    ("across_sources", across),
                ] {
                    if let Some(flag) = flag {
                        parameters.push((name, boolean(flag)));
                    }
                }
                if along {
                    parameters.push(("relationship", string("aggregates")));
                    parameters.push(("direction", string("backward")));
                }
                match tolerance {
                    1 => parameters.push(("tolerance", number(0.001))),
                    2 => parameters.push(("tolerance_quantity", millimetres(1.0))),
                    3 => parameters.push(("tolerance", number(0.0))),
                    _ => {}
                }
                model.evaluate(&CONSISTENT, &rule(ID, Selector::All, parameters));
            }
        }
    }
}

mod related_count {
    use super::*;

    const ID: &str = "axioval:capability.related-count";

    /// `related-count`, held to the implementation it replaced on every fixture.
    const RELATED_COUNT: common::Held = common::Held(
        &axioval_rules::RelatedCount,
        &axioval_rules::reference::RelatedCount,
    );

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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
            &RELATED_COUNT,
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
        let evaluation = rooms().evaluate(&RELATED_COUNT, &rule(ID, kind("room"), doors(vec![])));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }

    /// Doors that may be fire-rated: rated, not rated, or unreadable.
    fn rated(model: Model, ratings: &[u8]) -> Model {
        ratings
            .iter()
            .zip(["d1", "d2", "d3"])
            .fold(model, |model, (rating, door)| match rating {
                1 | 2 => model.value(
                    door,
                    "Pset",
                    "FireRated",
                    PropertyValue::Boolean(*rating == 1),
                ),
                3 => model.unreadable(door),
                _ => model,
            })
    }

    fn fire_rated() -> Selector {
        Selector::property(
            Some("Pset".into()),
            "FireRated",
            ComparisonOperator::Equals,
            Some(common::boolean(true)),
        )
    }

    /// The rule forked from the template, an `expression` rule, reaches its
    /// verdicts: undecided members widen the count in both. Members
    /// everywhere in the anchor's source and shared ends have no
    /// aggregate form, and such a rule is not forked.
    #[test]
    fn the_forked_rule_reaches_the_templates_verdicts() {
        use axioval_rules::templates::{Fork, ForkError, fork};
        for ratings in [[0, 0, 0], [1, 2, 3], [3, 3, 1], [1, 1, 2]] {
            for (minimum, maximum) in [(Some(1), None), (None, Some(1)), (Some(1), Some(2))] {
                for filter in [None, Some(fire_rated())] {
                    let mut parameters = vec![("relationship", string("bounds"))];
                    if let Some(filter) = filter {
                        parameters.push(("related_selector", selector(filter)));
                    }
                    if let Some(minimum) = minimum {
                        parameters.push(("minimum", integer(minimum)));
                    }
                    if let Some(maximum) = maximum {
                        parameters.push(("maximum", integer(maximum)));
                    }
                    let bound = rule(ID, kind("room"), parameters);
                    let template = rated(rooms(), &ratings).evaluate(&RELATED_COUNT, &bound);
                    let forked = fork(&axioval_rules::RelatedCount, &bound).unwrap();
                    let mut expression_rule = bound.clone();
                    expression_rule.capability = Fork::CAPABILITY.into();
                    expression_rule.parameters = forked.parameters();
                    let forked = rated(rooms(), &ratings)
                        .evaluate(&axioval_rules::ExpressionRequirement, &expression_rule);
                    let parity = axioval_rules::parity::compare_evaluations(
                        ("template", &template),
                        ("fork", &forked),
                    );
                    assert!(
                        parity.holds(),
                        "{ratings:?} {minimum:?} {maximum:?}\n{}",
                        parity.diff()
                    );
                }
            }
        }
        for parameters in [
            vec![("minimum", integer(1))],
            vec![
                ("minimum", integer(1)),
                ("relationship", string("bounds")),
                ("same_ends", common::strings(&["bounds:backward"])),
            ],
        ] {
            assert!(matches!(
                fork(
                    &axioval_rules::RelatedCount,
                    &rule(ID, kind("room"), parameters)
                ),
                Err(ForkError::Inexpressible(_))
            ));
        }
    }

    /// Generated rooms and doors: doors bounding random rooms, some
    /// fire-rated, some unreadable, counted along the relationship or in
    /// the whole source, with or without a filter, shared ends and bounds.
    /// The template is held to the implementation it replaced on each.
    mod generated {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(128))]

            #[test]
            fn generated_counts_hold_parity(
                bounds in proptest::collection::vec((0usize..3, 0usize..3), 0..6),
                ratings in [0u8..4, 0u8..4, 0u8..4],
                minimum in proptest::option::of(0i64..4),
                maximum in proptest::option::of(0i64..4),
                along in any::<bool>(),
                filter in 0u8..3,
                ends in any::<bool>(),
                anchors_are_doors in any::<bool>(),
            ) {
                let rooms = ["r1", "r2", "r3"];
                let doors = ["d1", "d2", "d3"];
                let model = bounds.iter().fold(
                    Model::default()
                        .object("r1", "room")
                        .object("r2", "room")
                        .object("r3", "room")
                        .object("d1", "door")
                        .object("d2", "door")
                        .object("d3", "door"),
                    |model, (room, door)| model.edge("bounds", rooms[*room], doors[*door]),
                );
                let model = rated(model, &ratings);
                let mut parameters = Vec::new();
                if along {
                    parameters.push(("relationship", string("bounds")));
                    if anchors_are_doors {
                        parameters.push(("direction", string("backward")));
                    }
                }
                match filter {
                    1 => parameters.push(("related_selector", selector(kind("door")))),
                    2 => parameters.push(("related_selector", selector(fire_rated()))),
                    _ => {}
                }
                if ends {
                    parameters.push(("same_ends", common::strings(&["bounds:backward"])));
                }
                if let Some(minimum) = minimum {
                    parameters.push(("minimum", integer(minimum)));
                }
                if let Some(maximum) = maximum {
                    parameters.push(("maximum", integer(maximum)));
                }
                let anchors = if anchors_are_doors { "door" } else { "room" };
                model.evaluate(&RELATED_COUNT, &rule(ID, kind(anchors), parameters));
            }
        }
    }
}

mod relative_count {
    use super::*;

    const ID: &str = "axioval:capability.relative-count";
    /// `relative-count` runs as a template, held on every fixture to the
    /// implementation it replaced.
    const RELATIVE: common::Held =
        common::Held(&RelativeCount, &axioval_rules::reference::RelativeCount);

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
            &RELATIVE,
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

    /// The template, held to the implementation it replaced on every
    /// evaluation.
    static HELD: common::Held =
        common::Held(&NameSequence, &axioval_rules::reference::NameSequence);

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
        model.evaluate_measured(
            &HELD,
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
            |_| {},
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

    /// Placement origins by local id; any other object is not placed.
    struct Heights(BTreeMap<&'static str, f64>, Vec<SourceSnapshot>);

    impl ObjectFrameService for Heights {
        fn source_snapshots(&self) -> &[SourceSnapshot] {
            &self.1
        }

        fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
            let height = *self
                .0
                .get(object.local_id.as_str())
                .ok_or_else(|| ObjectFrameError::NotPlaced(object.clone()))?;
            let axis = |vector| MetricDirection::try_new(vector).unwrap();
            let frame = MetricFrame::try_new(
                MetricPoint::try_new(object.clone(), [0.0, 0.0, height]).unwrap(),
                axis([1.0, 0.0, 0.0]),
                axis([0.0, 1.0, 0.0]),
                axis([0.0, 0.0, 1.0]),
            )
            .unwrap();
            ObjectFrame::try_new(
                object.clone(),
                frame,
                ObjectFront::NotStated,
                Evidence::exact(common::source(), format!("placement:{object}")),
            )
        }
    }

    /// Checks with `order_fallback` `placement_height` and, when given, the
    /// object-frame service placing members at `heights`.
    fn check_placed(
        model: Model,
        heights: Option<&[(&'static str, f64)]>,
    ) -> axioval_engine::CapabilityEvaluation {
        let rule = rule(
            ID,
            kind("building"),
            vec![
                ("member_selector", selector(kind("storey"))),
                ("name", property(Some(ATTR), "Name")),
                ("order", property(Some("Levels"), "Elevation")),
                ("relationship", string("aggregates")),
                ("order_fallback", string("placement_height")),
            ],
        );
        model.evaluate_measured(&HELD, &rule, |services| {
            if let Some(heights) = heights {
                let snapshot = SourceSnapshot::try_new(common::source(), "r1", "sha256:1").unwrap();
                services
                    .register(ObjectFrameServiceHandle::new(Arc::new(Heights(
                        heights.iter().copied().collect(),
                        vec![snapshot],
                    ))))
                    .unwrap();
            }
        })
    }

    #[test]
    fn members_without_an_order_value_are_ordered_by_their_placement_height() {
        // No storey states an elevation; they stand at 0 m, 3 m and 6 m.
        let storeys = building(&[("g", "1", None), ("u", "3", None), ("m", "2", None)]);
        let heights = [("g", 0.0), ("m", 3.0), ("u", 6.0)];
        let evaluation = check_placed(storeys, Some(&heights));
        assert!(
            evaluation.findings().is_empty() && evaluation.not_evaluated_outcomes().is_empty(),
            "{:?}",
            findings(&evaluation)
        );
        // The one at 3 m misnamed `4` is reported against the one below it.
        let misnamed = building(&[("g", "1", None), ("u", "3", None), ("m", "4", None)]);
        let evaluation = check_placed(misnamed, Some(&heights));
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "m".into(),
                    "axioval:attributes.Name 4 does not follow 1; expected 2".into()
                ),
                (
                    "u".into(),
                    "axioval:attributes.Name 3 is not above 4, the member below it".into()
                ),
            ]
        );
        // Cited as exactly as the placements it was ordered by.
        assert!(
            evaluation.findings()[0]
                .evidence
                .iter()
                .all(|evidence| evidence.exact)
        );
        // A stated elevation still orders its member: 7 m is above 6 m.
        let mixed = building(&[("g", "1", None), ("m", "2", None), ("u", "4", Some(7.0))])
            .object("x", "storey")
            .edge("aggregates", "b", "x")
            .text("x", ATTR, "Name", "3");
        let heights = [("g", 0.0), ("m", 3.0), ("x", 6.0), ("u", 100.0)];
        let evaluation = check_placed(mixed, Some(&heights));
        assert!(
            evaluation.findings().is_empty(),
            "{:?}",
            findings(&evaluation)
        );
    }

    #[test]
    fn a_member_with_neither_an_order_value_nor_a_placement_is_not_evaluated() {
        let storeys = || building(&[("g", "1", Some(0.0)), ("m", "2", None)]);
        let evaluation = check_placed(storeys(), Some(&[("g", 0.0)]));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("b".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
        // Without the object-frame service the fallback is a missing service.
        let evaluation = check_placed(storeys(), None);
        assert_eq!(
            unevaluated(&evaluation),
            [("b".to_owned(), NotEvaluatedReason::MissingService)]
        );
    }

    #[test]
    fn an_unknown_order_fallback_is_an_invalid_declaration() {
        let evaluation = building(&[("g", "1", Some(0.0))]).evaluate_measured(
            &HELD,
            &rule(
                ID,
                kind("building"),
                vec![
                    ("member_selector", selector(kind("storey"))),
                    ("name", property(Some(ATTR), "Name")),
                    ("order", property(Some("Levels"), "Elevation")),
                    ("order_fallback", string("bounding_box")),
                ],
            ),
            |_| {},
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "name-sequence: order_fallback `bounding_box` is unsupported; use `placement_height`"
        );
        // An increment that does not count up is refused alike.
        let evaluation = building(&[("g", "1", Some(0.0))]).evaluate_measured(
            &HELD,
            &rule(
                ID,
                kind("building"),
                vec![
                    ("member_selector", selector(kind("storey"))),
                    ("name", property(Some(ATTR), "Name")),
                    ("order", property(Some("Levels"), "Elevation")),
                    ("increment", integer(0)),
                ],
            ),
            |_| {},
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "name-sequence: increment must be positive"
        );
    }

    /// Generated buildings: storeys named by numbers, words, blanks or
    /// nothing, at stated, missing or placed elevations (some tied), one
    /// storey's name unreadable at times, numbered from a random start by
    /// a random increment, along the relationship or in the whole source;
    /// each held to the implementation the template replaced.
    #[test]
    fn generated_buildings_hold_parity() {
        let names = ["1", "2", "3", "4", "5", "0", "-1", "EG", "", "+2", "07"];
        let mut judged = 0;
        for storeys in 0..6_usize {
            for pattern in 0..40_usize {
                let mut model = Model::default().object("b", "building");
                let mut heights: Vec<(&'static str, f64)> = Vec::new();
                for storey in 0..storeys {
                    let local: &'static str = ["s0", "s1", "s2", "s3", "s4", "s5"][storey];
                    model = model.object(local, "storey");
                    if (pattern + storey) % 7 != 3 {
                        model = model.edge("aggregates", "b", local);
                    }
                    let name = names[(pattern * 3 + storey * 5) % names.len()];
                    model = match (pattern + 2 * storey) % 9 {
                        0 => model,
                        1 => model.unreadable_value(local, ATTR, "Name", "IfcLabel"),
                        2 => model.value(local, ATTR, "Name", PropertyValue::Integer(4)),
                        _ => model.text(local, ATTR, "Name", name),
                    };
                    #[allow(clippy::cast_precision_loss)]
                    let elevation = ((pattern + storey * 3) % 5) as f64 * 3.0;
                    match (pattern / 2 + storey) % 6 {
                        0 => heights.push((local, elevation)),
                        1 => {}
                        _ => {
                            model = model.value(
                                local,
                                "Levels",
                                "Elevation",
                                PropertyValue::Decimal(elevation),
                            );
                        }
                    }
                }
                let mut parameters = vec![
                    ("member_selector", selector(kind("storey"))),
                    ("name", property(Some(ATTR), "Name")),
                    ("order", property(Some("Levels"), "Elevation")),
                    ("order_fallback", string("placement_height")),
                ];
                if pattern % 2 == 0 {
                    parameters.push(("relationship", string("aggregates")));
                }
                if pattern % 3 == 1 {
                    parameters.push(("first", integer(0)));
                }
                if pattern % 5 == 2 {
                    parameters.push(("increment", integer(2)));
                }
                let rule = rule(ID, kind("building"), parameters);
                let heights = heights.clone();
                let evaluation = model.evaluate_measured(&HELD, &rule, move |services| {
                    let snapshot =
                        SourceSnapshot::try_new(common::source(), "r1", "sha256:1").unwrap();
                    services
                        .register(ObjectFrameServiceHandle::new(Arc::new(Heights(
                            heights.iter().copied().collect(),
                            vec![snapshot],
                        ))))
                        .unwrap();
                });
                judged += evaluation.findings().len() + evaluation.not_evaluated_outcomes().len();
            }
        }
        assert!(judged > 0);
    }
}

/// Model architecture: whether a site has geometry is whether it states a
/// body count in the reserved body set.
mod site_geometry {
    use super::*;
    use axioval_ir::BODY_SET;
    use axioval_rules::PropertyRequired;

    const ID: &str = "axioval:capability.property-required";

    #[test]
    fn a_site_without_a_body_is_a_finding_and_an_unreadable_one_is_not_evaluated() {
        let model = Model::default()
            .object("built", "site")
            .object("bare", "site")
            .object("unread", "site")
            .value("built", BODY_SET, "Count", PropertyValue::Integer(1))
            .unreadable("unread");
        let evaluation = model.evaluate(
            &PropertyRequired,
            &rule(
                ID,
                kind("site"),
                vec![("property", property(Some(BODY_SET), "Count"))],
            ),
        );
        assert_eq!(flagged(&evaluation), ["bare"]);
        assert_eq!(
            unevaluated(&evaluation),
            [("unread".to_owned(), NotEvaluatedReason::BackendUnavailable)]
        );
    }
}

mod numbering {
    use super::*;

    const ID: &str = "axioval:capability.numbering-consistency";

    /// The template, held to the implementation it replaced on every
    /// evaluation.
    static HELD: common::Held = common::Held(
        &NumberingConsistency,
        &axioval_rules::reference::NumberingConsistency,
    );

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
        model.evaluate_measured(&HELD, &rule(ID, kind("space"), parameters), |_| {})
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
        // Cited as exactly as the stated numbers they rest on.
        assert!(
            evaluation
                .findings()
                .iter()
                .all(|finding| finding.evidence.iter().all(|evidence| evidence.exact))
        );
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
                ("s3".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("s4".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("s5".to_owned(), NotEvaluatedReason::IncompleteEvidence),
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
        // Worded as the capability worded them.
        let refused = |extra| {
            check(spaces(), extra).not_evaluated_outcomes()[0]
                .message()
                .to_owned()
        };
        assert_eq!(
            refused(vec![]),
            "numbering-consistency: declare `prefix_length`, `gap_free` or both; nothing is \
             checked otherwise"
        );
        assert_eq!(
            refused(vec![
                ("gap_free", boolean(true)),
                ("pattern", string(r"B-\d+"))
            ]),
            r#"numbering-consistency: pattern "B-\\d+" must have exactly one group, the number"#
        );
        assert_eq!(
            refused(vec![("prefix_length", integer(0))]),
            "numbering-consistency: prefix_length must be positive"
        );
    }

    /// A pattern is matched exactly as stated: its spaces match spaces.
    #[test]
    fn a_pattern_keeps_its_spaces() {
        let evaluation = check(
            spaces(),
            vec![
                ("gap_free", boolean(true)),
                ("pattern", string(r" B-(\d+)")),
            ],
        );
        // No name starts with a space: none is numbered.
        assert!(evaluation.findings().is_empty());
        assert_eq!(unevaluated(&evaluation).len(), 8);
    }

    /// Generated storeys of spaces named by numbers with and without the
    /// pattern's prefix, of fewer digits, too large, blank, integers or
    /// unreadable, some on no storey, some in another source, checked for
    /// prefixes and gaps per storey, per source or across sources; each held
    /// to the implementation the template replaced.
    #[test]
    fn generated_storeys_hold_parity() {
        let names = [
            "B-101",
            "B-102",
            "B-104",
            "B-201",
            "B-202",
            "B-301",
            "B-9",
            "B-1001",
            "B-",
            "",
            "Lobby",
            "B-99999999999999999999999",
        ];
        let mut judged = 0;
        for spaces in 0..7_usize {
            for pattern in 0..30_usize {
                let mut model = Model::default()
                    .object("st1", "storey")
                    .object("st2", "storey")
                    .object_in("other", "x1", "space")
                    .value_of(
                        axioval_ir::ObjectId::new(
                            axioval_ir::SourceId::new("test", "other").unwrap(),
                            "x1",
                        )
                        .unwrap(),
                        ATTR,
                        "Name",
                        PropertyValue::String("B-103".into()),
                    );
                for space in 0..spaces {
                    let local: &'static str = ["s0", "s1", "s2", "s3", "s4", "s5", "s6"][space];
                    model = model.object(local, "space");
                    match (pattern + space) % 5 {
                        0 => {}
                        1 | 2 => model = model.edge("aggregates", "st1", local),
                        _ => model = model.edge("aggregates", "st2", local),
                    }
                    model = match (pattern * 3 + space * 7) % 13 {
                        12 => model.unreadable(local),
                        11 => model.value(local, ATTR, "Name", PropertyValue::Integer(105)),
                        index => model.text(local, ATTR, "Name", names[index % names.len()]),
                    };
                }
                let mut extra = vec![("pattern", string(r"B-(\d+)"))];
                match pattern % 3 {
                    0 => extra.push(("prefix_length", integer(1))),
                    1 => extra.push(("gap_free", boolean(true))),
                    _ => {
                        extra.push(("prefix_length", integer(2)));
                        extra.push(("gap_free", boolean(true)));
                    }
                }
                if pattern % 4 == 1 {
                    extra.push(("across_sources", boolean(true)));
                }
                let mut parameters = vec![("property", property(Some(ATTR), "Name"))];
                if pattern % 2 == 0 {
                    parameters.push(("relationship", string("aggregates")));
                    parameters.push(("direction", string("backward")));
                }
                parameters.extend(extra);
                let evaluation =
                    model.evaluate_measured(&HELD, &rule(ID, kind("space"), parameters), |_| {});
                judged += evaluation.findings().len() + evaluation.not_evaluated_outcomes().len();
            }
        }
        assert!(judged > 0);
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
