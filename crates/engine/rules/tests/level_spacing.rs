//! Storey heights from consecutive elevations.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::ParameterValue;
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::LevelSpacing;
use common::{Model, boolean, findings, kind, property, rule, selector, string, unevaluated};

const ID: &str = "axioval:capability.level-spacing";

fn building(levels: &[(&str, Option<f64>)]) -> Model {
    let mut model = Model::default().object("b", "building");
    for (local, elevation) in levels {
        model = model.object(local, "storey").edge("aggregates", "b", local);
        if let Some(elevation) = elevation {
            model = model.value(
                local,
                "Levels",
                "Elevation",
                PropertyValue::Quantity {
                    value: *elevation,
                    dimension: QuantityDimension::Length,
                },
            );
        }
    }
    model
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn check(model: Model, extra: Vec<(&str, ParameterValue)>) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![
        ("member_selector", selector(kind("storey"))),
        ("order", property(Some("Levels"), "Elevation")),
        ("relationship", string("aggregates")),
        ("ignore_highest", boolean(true)),
    ];
    parameters.extend(extra);
    model.evaluate(&LevelSpacing, &rule(ID, kind("building"), parameters))
}

/// Basement at -3, then 0, 3, 7.5 (a 4.5 m storey), roof level 10.5.
fn storeys() -> Model {
    building(&[
        ("ug", Some(-3.0)),
        ("eg", Some(0.0)),
        ("og1", Some(3.0)),
        ("og2", Some(7.5)),
        ("dg", Some(10.5)),
    ])
}

#[test]
fn each_height_is_bounded_inclusively() {
    let evaluation = check(
        storeys(),
        vec![
            (
                "minimum",
                ParameterValue::Quantity {
                    value: 2500.0,
                    unit: "mm".into(),
                },
            ),
            ("maximum", metres(4.0)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "og1".into(),
            "level height is 4.5 m; required at most 4 m".into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related[0].local_id, "og2");
}

#[test]
fn consistency_flags_heights_away_from_the_prevailing_one() {
    let evaluation = check(storeys(), vec![("consistent", boolean(true))]);
    assert_eq!(
        findings(&evaluation),
        [(
            "og1".into(),
            "level height 4.5 m differs from the prevailing 3 m".into()
        )]
    );
    // Leaving the basement out does not change which height prevails here.
    let without_basement = check(
        storeys(),
        vec![
            ("consistent", boolean(true)),
            ("ignore_lowest", boolean(true)),
        ],
    );
    assert_eq!(findings(&without_basement).len(), 1);
}

#[test]
fn the_highest_level_is_not_evaluated_unless_ignored() {
    let evaluation = check(
        storeys(),
        vec![
            ("maximum", metres(10.0)),
            ("ignore_highest", boolean(false)),
        ],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("dg".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_level_without_an_elevation_leaves_the_building_unmeasured() {
    let evaluation = check(
        building(&[("eg", Some(0.0)), ("og", None)]),
        vec![("maximum", metres(4.0))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("b".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn bounds_must_be_lengths() {
    let evaluation = check(
        storeys(),
        vec![(
            "maximum",
            ParameterValue::Quantity {
                value: 4.0,
                unit: "m2".into(),
            },
        )],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// Each level's `level_rise` within the bounds, and against the
/// `prevailing_rise`, reaches `level-spacing`'s verdicts on its storeys.
mod as_expressions {
    use super::*;
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{
        abs, assert_parity, at_most, between, differences, m, measured, rule as expression,
        subtract, unless_null,
    };
    use serde_json::Value;

    fn level(name: &str, options: &str) -> Value {
        measured(&format!(
            "{name};levels=storey;order=Levels/Elevation;anchor=aggregates{options}"
        ))
    }

    fn rewrite(model: Model, requirement: &Value) -> axioval_engine::CapabilityEvaluation {
        model.evaluate_measured(
            &ExpressionRequirement,
            &expression(kind("storey"), requirement),
            |_| {},
        )
    }

    fn bounded(options: &str, minimum: f64, maximum: f64) -> Value {
        let rise = level("level_rise", options);
        unless_null(&rise, between(rise.clone(), m(minimum), m(maximum)))
    }

    fn consistent(options: &str) -> Value {
        let rise = level("level_rise", options);
        let reference = level("prevailing_rise", options);
        unless_null(
            &reference,
            unless_null(
                &rise,
                at_most(abs(subtract(rise.clone(), reference.clone())), m(0.001)),
            ),
        )
    }

    #[test]
    fn bounded_heights_reach_the_verdicts() {
        for (minimum, maximum) in [(2.5, 4.0), (3.0, 4.5), (3.5, 10.0)] {
            let found = check(
                storeys(),
                vec![("minimum", metres(minimum)), ("maximum", metres(maximum))],
            );
            let rewritten = rewrite(storeys(), &bounded(";highest=ignored", minimum, maximum));
            assert_parity(ID, &found, &rewritten);
            let found = check(
                storeys(),
                vec![
                    ("minimum", metres(minimum)),
                    ("maximum", metres(maximum)),
                    ("ignore_lowest", boolean(true)),
                ],
            );
            let rewritten = rewrite(
                storeys(),
                &bounded(";highest=ignored;lowest=ignored", minimum, maximum),
            );
            assert_parity(ID, &found, &rewritten);
        }
        // The highest level, not ignored, has no rise to judge.
        let found = check(
            storeys(),
            vec![
                ("maximum", metres(10.0)),
                ("ignore_highest", boolean(false)),
            ],
        );
        let rewritten = rewrite(storeys(), &bounded("", 0.0, 10.0));
        assert_parity(ID, &found, &rewritten);
    }

    #[test]
    fn consistent_heights_reach_the_verdicts() {
        let found = check(storeys(), vec![("consistent", boolean(true))]);
        let rewritten = rewrite(storeys(), &consistent(";highest=ignored"));
        assert_parity(ID, &found, &rewritten);
        let found = check(
            storeys(),
            vec![
                ("consistent", boolean(true)),
                ("ignore_lowest", boolean(true)),
            ],
        );
        let rewritten = rewrite(storeys(), &consistent(";highest=ignored;lowest=ignored"));
        assert_parity(ID, &found, &rewritten);
        // Two storeys, each its own prevailing candidate: the lower prevails.
        let two = || building(&[("eg", Some(0.0)), ("og", Some(3.0)), ("dg", Some(7.0))]);
        let found = check(two(), vec![("consistent", boolean(true))]);
        let rewritten = rewrite(two(), &consistent(";highest=ignored"));
        assert_parity(ID, &found, &rewritten);
    }

    /// A storey without an elevation: the capability leaves the building
    /// open, unable to order its storeys; the rewrite, judging storeys,
    /// leaves each storey of that building open.
    #[test]
    fn an_unordered_building_leaves_its_storeys_open() {
        let model = || building(&[("eg", Some(0.0)), ("og", None)]);
        let found = check(model(), vec![("maximum", metres(4.0))]);
        let rewritten = rewrite(model(), &bounded(";highest=ignored", 0.0, 4.0));
        assert_eq!(
            differences(ID, &found, &rewritten),
            [
                "test:model/b: capability not evaluated (IncompleteEvidence), \
                 expression reported nothing",
                "test:model/eg: capability reported nothing, \
                 expression not evaluated (IncompleteEvidence)",
                "test:model/og: capability reported nothing, \
                 expression not evaluated (IncompleteEvidence)",
            ]
        );
    }

    /// Generated buildings: storeys at millimetre elevations, a range of
    /// storey heights, the lowest storey judged or left out. The rewrite
    /// reaches the capability's verdicts, and the rise it reads is the
    /// height the capability reports in its `levels` table.
    mod generated {
        use super::*;
        use axioval_rules::parity::{Observations, Parity};
        use proptest::collection::vec;
        use proptest::prelude::*;

        fn storeys(base: i32, rises: &[i32]) -> Vec<(String, Option<f64>)> {
            let mut elevation = base;
            let mut storeys = vec![("s0".to_owned(), Some(f64::from(elevation) / 1000.0))];
            for (index, rise) in rises.iter().enumerate() {
                elevation += rise;
                storeys.push((
                    format!("s{}", index + 1),
                    Some(f64::from(elevation) / 1000.0),
                ));
            }
            storeys
        }

        fn model(storeys: &[(String, Option<f64>)]) -> Model {
            let levels: Vec<(&str, Option<f64>)> = storeys
                .iter()
                .map(|(local, elevation)| (local.as_str(), *elevation))
                .collect();
            building(&levels)
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 64,
                failure_persistence: None,
                ..ProptestConfig::default()
            })]

            #[test]
            fn generated_buildings_hold_parity_with_their_heights(
                base in -6000i32..3000,
                rises in vec(2000i32..6000, 2..7),
                minimum in 2000i32..4000,
                spread in 0i32..3000,
                lowest in any::<bool>(),
            ) {
                let storeys = storeys(base, &rises);
                let (minimum, maximum) = (f64::from(minimum) / 1000.0, f64::from(minimum + spread) / 1000.0);
                let mut parameters = vec![("minimum", metres(minimum)), ("maximum", metres(maximum))];
                let mut options = ";highest=ignored".to_owned();
                if lowest {
                    parameters.push(("ignore_lowest", boolean(true)));
                    options.push_str(";lowest=ignored");
                }
                let found = check(model(&storeys), parameters);
                let rewritten = rewrite(model(&storeys), &bounded(&options, minimum, maximum));
                let objects: Vec<_> = storeys.iter().map(|(local, _)| common::id(local)).collect();
                // The rise of every level the check judges (divergence D16:
                // the table reports a level left out as unmeasured, where
                // the measured value leaving it out states it absent).
                let judged = |scope: &axioval_ir::Scope| {
                    let last = format!("s{}", storeys.len() - 1);
                    scope.object().is_some_and(|object| {
                        object.local_id != last && !(lowest && object.local_id == "s0")
                    })
                };
                let rises = model(&storeys).measure(
                    &format!("level_rise;levels=storey;order=Levels/Elevation;anchor=aggregates{options}"),
                    &objects,
                    |_| {},
                );
                let rewrite = rises
                    .into_iter()
                    .fold(Observations::of_evaluation(&rewritten), |observed, (object, rise)| {
                        observed.with_value(object, "levels.height", rise)
                    })
                    .retain_values(|scope, _| judged(scope));
                let capability =
                    Observations::of_evaluation(&found).retain_values(|scope, _| judged(scope));
                let parity = Parity::outcomes()
                    .value("levels.height", 0.0)
                    .compare((ID, &capability), ("expression", &rewrite));
                prop_assert!(parity.holds(), "{}", parity.diff());
                prop_assert!(parity.values > 0);
            }
        }
    }
}
