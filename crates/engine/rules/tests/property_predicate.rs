//! Text, number, boolean and presence predicates over exact property values.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ParameterValue, Selector};
use common::{Model, boolean, findings, flagged, integer, number, string, strings, unevaluated};

const CAPABILITY: &str = "axioval:capability.property-predicate";

fn model() -> Model {
    Model::default()
        .object("a", "wall")
        .object("b", "wall")
        .object("c", "wall")
        .text("a", "Pset", "FireRating", "F90")
        .text("b", "Pset", "FireRating", "f30")
        .text("c", "Pset", "FireRating", "   ")
        .value("a", "Pset", "Width", PropertyValue::Decimal(0.24))
        .value("b", "Pset", "Width", PropertyValue::Integer(1))
        .value("a", "Pset", "LoadBearing", PropertyValue::Boolean(true))
}

fn check(
    name: &str,
    operator: &str,
    target: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![
        ("property_set", string("Pset")),
        ("property", string(name)),
        ("operator", string(operator)),
    ];
    parameters.extend(target);
    common::predicate(
        model(),
        &common::rule(CAPABILITY, Selector::All, parameters),
    )
}

#[test]
fn text_equality_is_case_sensitive_unless_declared_otherwise() {
    let strict = check("FireRating", "equal", vec![("text", string("F30"))]);
    assert_eq!(flagged(&strict), ["a", "b", "c"]);
    let folded = check(
        "FireRating",
        "equal",
        vec![("text", string("F30")), ("case_sensitive", boolean(false))],
    );
    assert_eq!(flagged(&folded), ["a", "c"]);
}

#[test]
fn matches_is_a_whole_value_regular_expression() {
    let evaluation = check("FireRating", "matches", vec![("text", string("F\\d+"))]);
    // `f30` fails on case, the blank value fails, and `F90` passes whole.
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let partial = check("FireRating", "matches", vec![("text", string("F9"))]);
    assert_eq!(flagged(&partial), ["a", "b", "c"]);
}

#[test]
fn an_invalid_regular_expression_is_a_declaration_error() {
    let evaluation = check("FireRating", "matches", vec![("text", string("F("))]);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn one_of_and_none_of_take_a_list() {
    let one = check(
        "FireRating",
        "one_of",
        vec![
            ("texts", strings(&["F30", "F90"])),
            ("case_sensitive", boolean(false)),
        ],
    );
    assert_eq!(flagged(&one), ["c"]);
    let none = check("FireRating", "none_of", vec![("texts", strings(&["F90"]))]);
    assert_eq!(flagged(&none), ["a"]);
}

#[test]
fn contains_looks_inside_the_value() {
    let evaluation = check("FireRating", "contains", vec![("text", string("9"))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
}

#[test]
fn presence_treats_blank_text_as_undefined() {
    let defined = check("FireRating", "is_defined", vec![]);
    assert_eq!(flagged(&defined), ["c"]);
    let undefined = check("Width", "is_undefined", vec![]);
    // `c` has no width at all, which is what is_undefined asks for.
    assert_eq!(flagged(&undefined), ["a", "b"]);
}

#[test]
fn numbers_compare_integers_and_decimals_but_absence_fails() {
    let evaluation = check("Width", "less_or_equal", vec![("number", number(0.3))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let messages = findings(&evaluation);
    assert!(messages[0].1.contains("actual value is 1"), "{messages:?}");
    assert!(
        messages[1].1.contains("actual value is absent"),
        "{messages:?}"
    );
}

#[test]
fn booleans_compare_only_with_booleans() {
    let evaluation = check("LoadBearing", "equal", vec![("boolean", boolean(true))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let ordered = check(
        "LoadBearing",
        "greater_than",
        vec![("boolean", boolean(true))],
    );
    assert_eq!(
        unevaluated(&ordered),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_target_is_required_exactly_once() {
    for target in [vec![], vec![("text", string("F30")), ("value", integer(3))]] {
        let evaluation = check("FireRating", "equal", target);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    let presence = check("FireRating", "is_defined", vec![("text", string("x"))]);
    assert_eq!(
        unevaluated(&presence),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_quantity_is_never_compared_with_a_bare_number() {
    let model = Model::default().object("q", "slab").value(
        "q",
        "Pset",
        "Depth",
        PropertyValue::Quantity {
            value: 0.2,
            dimension: axioval_ir::QuantityDimension::Length,
        },
    );
    let evaluation = common::predicate(
        model,
        &common::rule(
            CAPABILITY,
            Selector::All,
            vec![
                ("property_set", string("Pset")),
                ("property", string("Depth")),
                ("operator", string("less_than")),
                ("number", number(1.0)),
            ],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("q".to_owned(), NotEvaluatedReason::InvalidEvidence)]
    );
}

/// A bound computed per object: load-bearing walls at most 0.3, others 2.
/// An unstated flag is `null`, which decides no branch, so the bound
/// states that a wall not stated load-bearing is one of the others.
#[test]
fn a_bound_computed_per_object_judges_each_object_by_its_own() {
    let bound = common::expression(serde_json::json!({"kind": "if",
        "branches": [{"when": {"kind": "coalesce", "operands": [
                          {"kind": "property", "propertySet": "Pset", "property": "LoadBearing"},
                          {"kind": "literal", "value": {"type": "boolean", "value": false}}]},
                      "then": {"kind": "literal", "value": {"type": "number", "value": 0.3}}}],
        "else": {"kind": "literal", "value": {"type": "number", "value": 2.0}}}));
    let evaluation = check("Width", "less_or_equal", vec![("number", bound)]);
    // `b` is 1 wide but not load-bearing; `c` states no width.
    assert_eq!(flagged(&evaluation), ["c"]);
    // A bound computed as `null` is no bound, and `number` is the only
    // target: the declaration is incomplete, never a pass.
    let absent = common::expression(serde_json::json!(
        {"kind": "property", "propertySet": "Pset", "property": "Missing"}));
    let evaluation = check("Width", "less_or_equal", vec![("number", absent)]);
    assert!(evaluation.findings().is_empty());
    assert!(!evaluation.not_evaluated_outcomes().is_empty());
}

/// `(object or "-", message)` of every not-evaluated outcome.
fn open(evaluation: &axioval_engine::CapabilityEvaluation) -> Vec<(String, String)> {
    evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map_or_else(|| "-".to_owned(), |object| object.local_id.clone()),
                outcome.message().to_owned(),
            )
        })
        .collect()
}

/// The capability's outside contract, word for word: its findings, its
/// objects left open and its refused declarations.
#[test]
#[allow(clippy::too_many_lines)]
fn messages_are_worded_as_the_capability_worded_them() {
    let pair = |object: &str, message: &str| (object.to_owned(), message.to_owned());
    assert_eq!(
        findings(&check(
            "Width",
            "less_or_equal",
            vec![("number", number(0.3))]
        )),
        [
            pair(
                "b",
                "property Pset.Width does not satisfy less_or_equal 0.3; actual value is 1"
            ),
            pair(
                "c",
                "property Pset.Width does not satisfy less_or_equal 0.3; actual value is absent"
            ),
        ]
    );
    assert_eq!(
        findings(&check(
            "FireRating",
            "one_of",
            vec![("texts", strings(&["F30", "F90"]))]
        )),
        [
            pair(
                "b",
                "property Pset.FireRating does not satisfy one_of [F30, F90]; actual value is `f30`"
            ),
            pair(
                "c",
                "property Pset.FireRating does not satisfy one_of [F30, F90]; actual value is `   `"
            ),
        ]
    );
    assert_eq!(
        findings(&check("FireRating", "is_defined", vec![])),
        [pair(
            "c",
            "property Pset.FireRating does not satisfy is_defined; actual value is `   `"
        )]
    );
    assert_eq!(
        findings(&check(
            "Width",
            "equal",
            vec![("number", number(0.2)), ("tolerance", number(0.05))]
        )),
        [
            pair(
                "b",
                "property Pset.Width does not satisfy equal 0.2 (within tolerance 0.05); \
                 actual value is 1"
            ),
            pair(
                "c",
                "property Pset.Width does not satisfy equal 0.2 (within tolerance 0.05); \
                 actual value is absent"
            ),
        ]
    );
    let model = Model::default().object("q", "slab").value(
        "q",
        "Pset",
        "Depth",
        PropertyValue::Quantity {
            value: 0.2,
            dimension: axioval_ir::QuantityDimension::Length,
        },
    );
    let rule = |target: (&str, ParameterValue)| {
        common::rule(
            CAPABILITY,
            Selector::All,
            vec![
                ("property_set", string("Pset")),
                ("property", string("Depth")),
                ("operator", string("less_than")),
                target,
            ],
        )
    };
    assert_eq!(
        open(&common::predicate(
            model.clone(),
            &rule(("number", number(1.0)))
        )),
        [pair(
            "q",
            "a quantity cannot be compared with a unit-less target"
        )]
    );
    let quantity = ParameterValue::Quantity {
        value: 150.0,
        unit: "mm".into(),
    };
    assert_eq!(
        findings(&common::predicate(model, &rule(("quantity", quantity)))),
        [pair(
            "q",
            "property Pset.Depth does not satisfy less_than 150 mm; actual value is 0.2 m"
        )]
    );
    for (operator, target, message) in [
        (
            "equal",
            vec![],
            "property-predicate: operator `equal` takes 1 target value(s); 0 given",
        ),
        (
            "greater_than",
            vec![("boolean", boolean(true))],
            "property-predicate: operator `greater_than` does not apply to a boolean",
        ),
        (
            "contains",
            vec![("number", number(1.0))],
            "property-predicate: operator `contains` does not apply to a number",
        ),
        (
            "equal",
            vec![("text", string("x")), ("precision", string("day"))],
            "property-predicate: `precision` applies to a date or date_time target only",
        ),
        (
            "equal",
            vec![("text", string("x")), ("tolerance", number(0.1))],
            "property-predicate: a tolerance applies to a numeric target only",
        ),
    ] {
        assert_eq!(
            open(&check("FireRating", operator, target)),
            [pair("-", message)],
            "{operator}"
        );
    }
    let invalid = open(&check(
        "FireRating",
        "matches",
        vec![("text", string("F("))],
    ));
    assert!(
        invalid[0]
            .1
            .starts_with("property-predicate: invalid regular expression: "),
        "{invalid:?}"
    );
}

/// A target as a rule's parameters state it.
type Target = Vec<(&'static str, ParameterValue)>;

/// The rule forked from the template, an `expression` rule evaluated by the
/// expression capability, reaches the template's verdicts wherever the
/// values are of the target's kind or absent: the same stated values read
/// through the same evaluator, decided by the same comparison. (A value of
/// another kind, which the template fails, leaves the fork open; blank
/// text is defined to the fork.)
#[test]
fn the_forked_rule_reaches_the_templates_verdicts() {
    use axioval_rules::templates::{Fork, fork};
    let typed = Model::default()
        .object("a", "wall")
        .object("b", "wall")
        .object("c", "wall")
        .object("d", "wall")
        .text("a", "Pset", "Rating", "F90")
        .text("b", "Pset", "Rating", "f30")
        .text("c", "Pset", "Rating", "F30")
        .value("a", "Pset", "Width", PropertyValue::Decimal(0.24))
        .value("b", "Pset", "Width", PropertyValue::Decimal(0.3))
        .value("c", "Pset", "Width", PropertyValue::Decimal(0.31))
        .value("a", "Pset", "Count", PropertyValue::Integer(3))
        .value("b", "Pset", "Count", PropertyValue::Integer(4))
        .value("a", "Pset", "Bearing", PropertyValue::Boolean(true))
        .value("b", "Pset", "Bearing", PropertyValue::Boolean(false));
    let declarations: Vec<(&str, &str, Target)> = vec![
        ("Width", "less_or_equal", vec![("number", number(0.3))]),
        ("Width", "greater_than", vec![("number", number(0.24))]),
        ("Width", "not_equal", vec![("number", number(0.3))]),
        ("Count", "equal", vec![("value", integer(4))]),
        ("Bearing", "equal", vec![("boolean", boolean(true))]),
        ("Rating", "equal", vec![("text", string("F30"))]),
        (
            "Rating",
            "equal",
            vec![("text", string("F30")), ("case_sensitive", boolean(false))],
        ),
        ("Rating", "contains", vec![("text", string("3"))]),
        ("Rating", "matches", vec![("text", string("F\\d+"))]),
        (
            "Rating",
            "none_of",
            vec![
                ("texts", strings(&["F90", "f30"])),
                ("case_sensitive", boolean(false)),
            ],
        ),
        ("Rating", "one_of", vec![("texts", strings(&["F90"]))]),
        ("Rating", "is_defined", vec![]),
        ("Width", "is_undefined", vec![]),
    ];
    for (property, operator, target) in declarations {
        let mut parameters = vec![
            ("property_set", string("Pset")),
            ("property", string(property)),
            ("operator", string(operator)),
        ];
        parameters.extend(target);
        let bound = common::rule(CAPABILITY, Selector::All, parameters);
        let forked = fork(&axioval_rules::PropertyPredicate, &bound).unwrap();
        let mut expression_rule = bound.clone();
        expression_rule.capability = Fork::CAPABILITY.into();
        expression_rule.parameters = forked.parameters();
        let template = common::predicate(typed.clone(), &bound);
        let forked = typed
            .clone()
            .evaluate(&axioval_rules::ExpressionRequirement, &expression_rule);
        let parity =
            axioval_rules::parity::compare_evaluations(("template", &template), ("fork", &forked));
        assert!(parity.holds(), "{property} {operator}\n{}", parity.diff());
    }
    // A tolerance has no expression form: the template refuses to fork it.
    let tolerant = common::rule(
        CAPABILITY,
        Selector::All,
        vec![
            ("property_set", string("Pset")),
            ("property", string("Width")),
            ("operator", string("equal")),
            ("number", number(0.3)),
            ("tolerance", number(0.01)),
        ],
    );
    assert_eq!(
        fork(&axioval_rules::PropertyPredicate, &tolerant)
            .unwrap_err()
            .to_string(),
        "the rule has no expression form: no expression states a tolerance"
    );
}

/// Generated predicates over generated values: every operator, target
/// kind, text option, tolerance and precision against values of every
/// kind (absent, `null`, lists, measured intervals, complex properties
/// included). The template holds the whole outside contract of the
/// implementation it replaced on every one (`common::predicate`).
mod generated {
    use super::*;
    use proptest::prelude::*;

    fn value() -> impl Strategy<Value = Option<PropertyValue>> {
        let length = |value: f64| PropertyValue::Quantity {
            value,
            dimension: axioval_ir::QuantityDimension::Length,
        };
        let date = |text: &str| PropertyValue::Date(text.parse().unwrap());
        let instant = |text: &str| PropertyValue::DateTime(text.parse().unwrap());
        prop_oneof![
            1 => Just(None),
            1 => Just(Some(PropertyValue::Null)),
            1 => (-3i64..4).prop_map(|value| Some(PropertyValue::Integer(value))),
            1 => Just(Some(PropertyValue::Integer(i64::MAX))),
            1 => prop::sample::select(vec![0.3, 0.300_000_000_000_000_04, -0.0, 0.0, 2.5, 0.009])
                .prop_map(|value| Some(PropertyValue::Decimal(value))),
            1 => prop::sample::select(vec![0.15, 0.009, 9.0 * 1e-3, 0.3])
                .prop_map(move |value| Some(length(value))),
            1 => Just(Some(PropertyValue::Quantity {
                value: 0.3,
                dimension: axioval_ir::QuantityDimension::Area,
            })),
            4 => prop::sample::select(vec!["F90", "f90", " F90 ", "", "   ", "a*b", "F30"])
                .prop_map(|text| Some(PropertyValue::String(text.to_owned()))),
            1 => any::<bool>().prop_map(|value| Some(PropertyValue::Boolean(value))),
            1 => prop::sample::select(vec!["2022-01-01", "2022-01-01Z", "2021-12-31+12:00"])
                .prop_map(move |text| Some(date(text))),
            1 => prop::sample::select(vec!["2022-01-01T00:00:00Z", "2021-12-31T23:00:00-02:00"])
                .prop_map(move |text| Some(instant(text))),
            1 => Just(Some(PropertyValue::List(vec![
                PropertyValue::String("F90".into()),
                PropertyValue::String(String::new()),
            ]))),
            1 => Just(Some(PropertyValue::List(vec![PropertyValue::String(
                "  ".into()
            )]))),
            1 => Just(Some(PropertyValue::Measured {
                lower: 0.2,
                upper: 0.4,
                dimension: Some(axioval_ir::QuantityDimension::Length),
            })),
            1 => Just(Some(PropertyValue::Complex)),
        ]
    }

    fn target() -> impl Strategy<Value = Vec<(&'static str, ParameterValue)>> {
        let quantity = |value: f64, unit: &str| ParameterValue::Quantity {
            value,
            unit: unit.into(),
        };
        prop_oneof![
            1 => Just(vec![]),
            1 => (-3i64..4).prop_map(|value| vec![("value", integer(value))]),
            1 => prop::sample::select(vec![0.3, 0.0, 2.5, 0.009])
                .prop_map(|value| vec![("number", number(value))]),
            1 => prop::sample::select(vec![(150.0, "mm"), (9.0, "mm"), (0.3, "m"), (0.3, "m2")])
                .prop_map(move |(value, unit)| vec![("quantity", quantity(value, unit))]),
            1 => Just(vec![("quantity", quantity(1.0, "furlong"))]),
            4 => prop::sample::select(vec!["F90", "f90", "", "F\\d+", "a*", "F("])
                .prop_map(|text| vec![("text", string(text))]),
            1 => Just(vec![("texts", strings(&["F90", "f30", ""]))]),
            1 => any::<bool>().prop_map(|value| vec![("boolean", boolean(value))]),
            1 => prop::sample::select(vec!["2022-01-01", "2022-01-01Z"]).prop_map(|text| vec![(
                "date",
                ParameterValue::Date {
                    value: text.parse().unwrap()
                }
            )]),
            1 => Just(vec![(
                "date_time",
                ParameterValue::DateTime {
                    value: "2022-01-01T00:00:00Z".parse().unwrap()
                }
            )]),
            1 => Just(vec![("number", number(1.0)), ("text", string("x"))]),
        ]
    }

    fn options() -> impl Strategy<Value = Vec<(&'static str, ParameterValue)>> {
        (
            prop::option::of(any::<bool>()),
            prop_oneof![
                10 => Just(vec![]),
                2 => Just(vec![("tolerance", number(0.1))]),
                2 => Just(vec![("relative_tolerance", number(0.25))]),
                2 => Just(vec![("decimals", integer(1))]),
                1 => Just(vec![("tolerance", number(-1.0))]),
                1 => Just(vec![("decimals", integer(1)), ("tolerance", number(0.1))]),
            ],
            prop_oneof![
                10 => Just(None),
                3 => Just(Some("day")),
                1 => Just(Some("hour")),
            ],
        )
            .prop_map(|(case_sensitive, mut tolerance, precision)| {
                if let Some(case_sensitive) = case_sensitive {
                    tolerance.push(("case_sensitive", boolean(case_sensitive)));
                }
                if let Some(precision) = precision {
                    tolerance.push(("precision", string(precision)));
                }
                tolerance
            })
    }

    const OPERATORS: &[&str] = &[
        "equal",
        "not_equal",
        "greater_than",
        "greater_or_equal",
        "less_than",
        "less_or_equal",
        "contains",
        "matches",
        "one_of",
        "none_of",
        "is_defined",
        "is_undefined",
        "approximately",
    ];

    /// The operator words that fit a target, mostly, and any word else.
    fn statement() -> impl Strategy<Value = (&'static str, Vec<(&'static str, ParameterValue)>)> {
        target().prop_flat_map(|target| {
            let fitting: &'static [&'static str] = match target.first().map(|(name, _)| *name) {
                None => &["is_defined", "is_undefined"],
                Some("text") => &["equal", "not_equal", "contains", "matches"],
                Some("texts") => &["one_of", "none_of"],
                Some("boolean") => &["equal", "not_equal"],
                Some(_) => &OPERATORS[..6],
            };
            (
                prop_oneof![
                    4 => prop::sample::select(fitting),
                    1 => prop::sample::select(OPERATORS),
                ],
                Just(target),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 2048,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn generated_predicates_hold_the_contract(
            values in prop::collection::vec(value(), 1..8),
            (operator, target) in statement(),
            options in options(),
        ) {
            let mut model = Model::default();
            for (index, value) in values.iter().enumerate() {
                let local = format!("o{index}");
                model = model.object(&local, "wall");
                if let Some(value) = value {
                    model = model.value(&local, "Pset", "Value", value.clone());
                }
            }
            let mut parameters = vec![
                ("property_set", string("Pset")),
                ("property", string("Value")),
                ("operator", string(operator)),
            ];
            parameters.extend(target);
            parameters.extend(options);
            common::predicate(model, &common::rule(CAPABILITY, Selector::All, parameters));
        }
    }
}
