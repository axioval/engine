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
