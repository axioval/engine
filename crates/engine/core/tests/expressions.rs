//! The expression evaluator and type checker.
#![allow(missing_docs, clippy::float_cmp)]
use std::collections::BTreeMap;

use axioval_engine::expression::{
    ExpressionContext, Interval, Leaf, Reason, Type, TypeEnvironment, TypeErrorKind, Unit, Value,
    check, check_as, evaluate,
};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::Expression;
use serde_json::json;

fn expression(value: serde_json::Value) -> Expression {
    let expression: Expression = serde_json::from_value(value).unwrap();
    expression.validate().unwrap();
    expression
}

/// Parameters by name: a value, or unreadable when absent.
#[derive(Default)]
struct Context {
    parameters: BTreeMap<String, Value>,
    properties: BTreeMap<String, Value>,
}

impl ExpressionContext for Context {
    fn property(&mut self, _: Option<&str>, name: &str) -> Leaf {
        self.properties.get(name).cloned().map_or_else(
            || Leaf::unreadable(format!("`{name}` is not measured")),
            Leaf::stated,
        )
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        self.parameters.get(name).cloned().map_or_else(
            || Leaf::unreadable(format!("`{name}` is unknown")),
            Leaf::stated,
        )
    }
}

fn parameter(name: &str) -> serde_json::Value {
    json!({"kind": "parameter", "name": name})
}

/// `SplitMix64`: a small seeded generator, so every run tests the same cases.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[allow(clippy::cast_precision_loss)]
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random arithmetic tree over the parameters `a`, `b`, `c`.
fn tree(random: &mut Random, depth: u32) -> serde_json::Value {
    if depth == 0 || random.below(4) == 0 {
        return match random.below(4) {
            0 => {
                json!({"kind": "literal", "value": {"type": "number", "value": random.unit() * 10.0 - 5.0}})
            }
            1 => parameter("a"),
            2 => parameter("b"),
            _ => parameter("c"),
        };
    }
    let mut next = || tree(random, depth - 1);
    let (left, right) = (next(), next());
    match random.below(11) {
        0 => json!({"kind": "add", "left": left, "right": right}),
        1 => json!({"kind": "subtract", "left": left, "right": right}),
        2 => json!({"kind": "multiply", "left": left, "right": right}),
        3 => json!({"kind": "divide", "left": left, "right": right}),
        4 => json!({"kind": "min", "operands": [left, right]}),
        5 => json!({"kind": "max", "operands": [left, right]}),
        6 => json!({"kind": "negate", "operand": left}),
        7 => json!({"kind": "abs", "operand": left}),
        8 => json!({"kind": "sqrt", "operand": {"kind": "abs", "operand": left}}),
        9 => json!({"kind": "sin", "operand": {"kind": "multiply", "left": left,
            "right": {"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "rad"}}}}),
        _ => json!({"kind": "convertSlope", "operand": left, "from": "ratio", "to": "percent"}),
    }
}

/// The same tree over points, in plain floating point.
fn reference(expression: &Expression, points: &BTreeMap<&str, f64>) -> Option<f64> {
    use axioval_ir::contract::ScalarValue;
    let value = |operand: &Expression| reference(operand, points);
    Some(match expression {
        Expression::Literal {
            value: ScalarValue::Number { value } | ScalarValue::Quantity { value, .. },
            ..
        } => *value,
        Expression::Parameter { name, .. } => points[name.as_str()],
        Expression::Add { left, right, .. } => value(left)? + value(right)?,
        Expression::Subtract { left, right, .. } => value(left)? - value(right)?,
        Expression::Multiply { left, right, .. } => value(left)? * value(right)?,
        Expression::Divide { left, right, .. } => value(left)? / value(right)?,
        Expression::Min { operands, .. } => value(&operands[0])?.min(value(&operands[1])?),
        Expression::Max { operands, .. } => value(&operands[0])?.max(value(&operands[1])?),
        Expression::Negate { operand, .. } => -value(operand)?,
        Expression::Abs { operand, .. } => value(operand)?.abs(),
        Expression::Sqrt { operand, .. } => value(operand)?.sqrt(),
        Expression::Sin { operand, .. } => value(operand)?.sin(),
        Expression::ConvertSlope { operand, .. } => value(operand)? * 100.0,
        _ => return None,
    })
}

#[test]
fn intervals_hold_the_value_of_every_point_they_allow() {
    let mut random = Random(0x5EED);
    let mut checked = 0;
    for _ in 0..4000 {
        let expression = expression(tree(&mut random, 5));
        let mut context = Context::default();
        let mut points = BTreeMap::new();
        for name in ["a", "b", "c"] {
            let lower = random.unit() * 20.0 - 10.0;
            let width = if random.below(2) == 0 {
                0.0
            } else {
                random.unit()
            };
            let point = lower + width * random.unit();
            points.insert(name, point);
            context.parameters.insert(
                name.into(),
                Value::Number {
                    value: Interval::new(lower, lower + width).unwrap(),
                    unit: Unit::NONE,
                },
            );
        }
        let Ok(Value::Number { value, .. }) = evaluate(&expression, "value", &mut context).outcome
        else {
            // Division by an interval around zero, or similar: not evaluated.
            continue;
        };
        let exact = reference(&expression, &points).unwrap();
        assert!(
            value.contains(exact),
            "{exact} is outside {value:?} for {}",
            serde_json::to_string(&expression).unwrap()
        );
        checked += 1;
    }
    assert!(checked > 2000, "only {checked} trees were evaluated");
}

/// A truth leaf: true, false, `null`, or not evaluated.
fn truth(state: &str) -> serde_json::Value {
    match state {
        "T" => json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
        "F" => json!({"kind": "literal", "value": {"type": "boolean", "value": false}}),
        "N" => json!({"kind": "null"}),
        _ => parameter("unknown"),
    }
}

fn kleene(value: serde_json::Value) -> &'static str {
    match evaluate(&expression(value), "r", &mut Context::default()).outcome {
        Ok(Value::Boolean(true)) => "T",
        Ok(Value::Boolean(false)) => "F",
        Ok(other) => panic!("{other:?}"),
        Err(_) => "U",
    }
}

#[test]
fn logic_follows_kleene_truth_tables() {
    // Rows: left T F U; columns: right T F U.
    let states = ["T", "F", "U"];
    let tables = [
        ("and", ["TFU", "FFF", "UFU"]),
        ("or", ["TTT", "TFU", "TUU"]),
        ("implies", ["TFU", "TTT", "TUU"]),
        ("xor", ["FTU", "TFU", "UUU"]),
    ];
    for (kind, rows) in tables {
        for (left, row) in states.iter().zip(rows) {
            for (right, expected) in states.iter().zip(row.chars()) {
                let value = match kind {
                    "implies" => {
                        json!({"kind": kind, "antecedent": truth(left), "consequent": truth(right)})
                    }
                    "xor" => json!({"kind": kind, "left": truth(left), "right": truth(right)}),
                    _ => json!({"kind": kind, "operands": [truth(left), truth(right)]}),
                };
                assert_eq!(kleene(value), expected.to_string(), "{left} {kind} {right}");
            }
        }
    }
    for (operand, expected) in [("T", "F"), ("F", "T"), ("U", "U"), ("N", "T")] {
        assert_eq!(
            kleene(json!({"kind": "not", "operand": truth(operand)})),
            expected
        );
    }
}

#[test]
fn null_is_a_stated_absence_and_never_not_evaluated() {
    let three = json!({"kind": "literal", "value": {"type": "integer", "value": 3}});
    let absent = json!({"kind": "null"});
    let unknown = parameter("unknown");
    let run = |value: serde_json::Value| {
        evaluate(&expression(value), "r", &mut Context::default()).outcome
    };
    for operator in ["equals", "notEquals", "lessThan", "greaterThanOrEquals"] {
        assert_eq!(
            run(json!({"kind": "compare", "operator": operator, "left": absent, "right": three})),
            Ok(Value::Boolean(false)),
            "{operator}"
        );
    }
    assert_eq!(
        run(json!({"kind": "isUndefined", "operand": absent})),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        run(json!({"kind": "isDefined", "operand": absent})),
        Ok(Value::Boolean(false))
    );
    assert!(run(json!({"kind": "isDefined", "operand": unknown})).is_err());
    assert_eq!(
        run(json!({"kind": "add", "left": absent, "right": three})),
        Ok(Value::Null)
    );
    assert_eq!(
        run(json!({"kind": "add", "left": unknown, "right": absent})),
        Ok(Value::Null)
    );
    assert_eq!(
        run(json!({"kind": "coalesce", "operands": [absent, three]})),
        Ok(Value::integer(3))
    );
    assert!(run(json!({"kind": "coalesce", "operands": [unknown, three]})).is_err());
    assert_eq!(
        run(json!({"kind": "coalesce", "operands": [absent, absent]})),
        Ok(Value::Null)
    );
    assert_eq!(
        run(json!({"kind": "oneOf", "operand": absent, "values": [three]})),
        Ok(Value::Boolean(false))
    );
    assert_eq!(
        run(
            json!({"kind": "concat", "operands": [absent, {"kind": "literal", "value": {"type": "string", "value": "a"}}]})
        ),
        Ok(Value::Null)
    );
}

fn measured(lower: f64, upper: f64) -> Value {
    Value::quantity(
        Interval::new(lower, upper).unwrap(),
        QuantityDimension::Length,
    )
}

#[test]
fn a_straddling_comparison_is_not_evaluated_and_named() {
    let mut context = Context::default();
    context
        .properties
        .insert("extent_z".into(), measured(0.045, 0.055));
    let height = json!({"kind": "property", "propertySet": "axioval:measured", "property": "extent_z", "label": "pipe height"});
    let limit = |mm: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": mm, "unit": "mm"}});
    let rule = expression(json!({"kind": "and", "operands": [
        {"kind": "literal", "value": {"type": "boolean", "value": true}},
        {"kind": "compare", "operator": "lessThan", "left": height, "right": limit(50.0)}
    ]}));
    let evaluation = evaluate(&rule, "requirement", &mut context);
    let why = evaluation.outcome.unwrap_err();
    assert_eq!(why.path, "requirement.and[1]");
    assert!(matches!(why.reason, Reason::Straddles { .. }), "{why}");
    assert!(why.to_string().contains("0.045..0.055 m"), "{why}");
    assert_eq!(evaluation.reads.len(), 1);
    assert_eq!(evaluation.reads[0].path, "requirement.and[1].compare.left");
    // A 40 mm pipe is decided.
    context
        .properties
        .insert("extent_z".into(), measured(0.039, 0.041));
    let decided = evaluate(&rule, "requirement", &mut context);
    assert_eq!(decided.outcome, Ok(Value::Boolean(true)));
}

#[test]
fn an_undecided_if_is_not_evaluated_naming_its_condition() {
    let cover = |mm: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": mm, "unit": "mm"}});
    let condition = json!({"kind": "compare", "operator": "equals",
        "left": {"kind": "property", "property": "ExposureClass"},
        "right": {"kind": "literal", "value": {"type": "string", "value": "XC4"}}});
    let required = expression(json!({"kind": "if",
        "branches": [{"when": condition, "then": cover(40.0)}], "else": cover(25.0)}));
    let why = evaluate(&required, "required", &mut Context::default())
        .outcome
        .unwrap_err();
    assert_eq!(why.path, "required");
    let Reason::UndecidedCondition(inner) = &why.reason else {
        panic!("{why}");
    };
    assert_eq!(inner.path, "required.if.branches[0].when.compare.left");
    // Branches that agree decide even when the condition is not evaluated.
    let agreeing = expression(json!({"kind": "if",
        "branches": [{"when": condition, "then": cover(25.0)}], "else": cover(25.0)}));
    assert!(
        evaluate(&agreeing, "r", &mut Context::default())
            .outcome
            .is_ok()
    );
    // A stated class decides.
    let mut context = Context::default();
    context
        .properties
        .insert("ExposureClass".into(), Value::Text("XC4".into()));
    let Ok(Value::Number { value, .. }) = evaluate(&required, "r", &mut context).outcome else {
        panic!()
    };
    assert!(value.contains(0.04));
}

#[test]
fn arithmetic_failures_are_named() {
    let mut context = Context::default();
    context.parameters.insert(
        "zero".into(),
        Value::Number {
            value: Interval::new(-1.0, 1.0).unwrap(),
            unit: Unit::NONE,
        },
    );
    let one = json!({"kind": "literal", "value": {"type": "number", "value": 1.0}});
    let why = evaluate(
        &expression(json!({"kind": "divide", "left": one, "right": parameter("zero")})),
        "v",
        &mut context,
    )
    .outcome
    .unwrap_err();
    assert_eq!(why.reason, Reason::ZeroDivisor);
    let huge = json!({"kind": "literal", "value": {"type": "number", "value": f64::MAX}});
    let why = evaluate(
        &expression(json!({"kind": "multiply", "left": huge, "right": huge})),
        "v",
        &mut context,
    )
    .outcome
    .unwrap_err();
    assert_eq!(why.reason, Reason::Overflow);
}

#[test]
fn units_combine_and_slopes_convert() {
    let mut context = Context::default();
    let length =
        |m: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": m, "unit": "m"}});
    let area = evaluate(
        &expression(json!({"kind": "multiply", "left": length(2.0), "right": length(3.0)})),
        "v",
        &mut context,
    )
    .outcome
    .unwrap();
    assert_eq!(
        area,
        Value::quantity(Interval::point(6.0), QuantityDimension::Area)
    );
    let slope = expression(
        json!({"kind": "convertSlope", "from": "ratio", "to": "angle",
        "operand": {"kind": "divide", "left": length(1.0), "right": length(12.0)}}),
    );
    let Ok(Value::Number { value, unit }) = evaluate(&slope, "v", &mut context).outcome else {
        panic!()
    };
    assert_eq!(unit, Unit::RADIAN);
    assert!(value.contains((1.0f64 / 12.0).atan()));
    let mixed = expression(json!({"kind": "add", "left": length(1.0),
        "right": {"kind": "literal", "value": {"type": "number", "value": 1.0}}}));
    let why = evaluate(&mixed, "v", &mut context).outcome.unwrap_err();
    assert!(matches!(why.reason, Reason::Mismatch(_)), "{why}");
}

/// Declared types: `Height` a length, `Class` text, `Load` unknown until read.
struct Environment;

impl TypeEnvironment for Environment {
    fn property(&self, set: Option<&str>, name: &str) -> Result<Type, String> {
        if set == Some(axioval_ir::MEASURED_SET) {
            return axioval_engine::expression::measured_type(name);
        }
        match name {
            "Height" => Ok(Type::Number(Unit::of(Some(QuantityDimension::Length)))),
            "Class" => Ok(Type::Text),
            "Load" => Ok(Type::Any),
            "External" => Ok(Type::Boolean),
            "Opened" => Ok(Type::Date),
            other => Err(format!("no property `{other}` is declared")),
        }
    }

    fn parameter(&self, name: &str) -> Result<Type, String> {
        match name {
            "count" => Ok(Type::Integer),
            "angle" => Ok(Type::Number(Unit::RADIAN)),
            other => Err(format!("no parameter `{other}` is declared")),
        }
    }
}

fn type_error(value: serde_json::Value) -> (String, TypeErrorKind) {
    let error = check(&expression(value), "requirement", &Environment).unwrap_err();
    (error.path, error.kind)
}

fn property(name: &str) -> serde_json::Value {
    json!({"kind": "property", "property": name})
}

#[test]
fn every_type_error_names_its_path() {
    let metre =
        json!({"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "m"}});
    let number = json!({"kind": "literal", "value": {"type": "number", "value": 1.0}});
    let text = json!({"kind": "literal", "value": {"type": "string", "value": "XC4"}});
    let ok = json!({"kind": "literal", "value": {"type": "boolean", "value": true}});
    let cases = [
        (
            json!({"kind": "and", "operands": [ok, ok, {"kind": "compare", "operator": "lessThan", "left": property("Height"), "right": number}]}),
            "requirement.and[2]",
            "UnitMismatch",
        ),
        (
            json!({"kind": "and", "operands": [ok, property("Nope")]}),
            "requirement.and[1]",
            "Unknown",
        ),
        (
            json!({"kind": "compare", "operator": "lessThan",
                "left": {"kind": "property", "propertySet": "axioval:measured", "property": "height"}, "right": metre}),
            "requirement.compare.left",
            "Unknown",
        ),
        (
            json!({"kind": "not", "operand": number}),
            "requirement.not.operand",
            "NotBoolean",
        ),
        (
            json!({"kind": "add", "left": text, "right": number}),
            "requirement.add.left",
            "NotNumeric",
        ),
        (
            json!({"kind": "length", "operand": number}),
            "requirement.length.operand",
            "NotText",
        ),
        (
            json!({"kind": "sin", "operand": metre}),
            "requirement.sin.operand",
            "NotAngle",
        ),
        (
            json!({"kind": "compare", "operator": "equals", "left": property("External"), "right": property("Opened")}),
            "requirement",
            "Mismatch",
        ),
        (
            json!({"kind": "sqrt", "operand": {"kind": "multiply", "left": metre, "right": {"kind": "multiply", "left": metre, "right": metre}}}),
            "requirement",
            "NoSquareRoot",
        ),
        (
            json!({"kind": "add", "left": {"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "parsec"}}, "right": metre}),
            "requirement.add.left",
            "InvalidUnit",
        ),
        (
            json!({"kind": "compare", "operator": "matches", "left": property("Class"), "right": {"kind": "literal", "value": {"type": "string", "value": "("}}}),
            "requirement.compare.right",
            "InvalidPattern",
        ),
        (
            json!({"kind": "if", "branches": [{"when": ok, "then": metre}], "else": number}),
            "requirement.if.else",
            "UnitMismatch",
        ),
        (
            json!({"kind": "coalesce", "operands": [text, ok]}),
            "requirement.coalesce[1]",
            "Mismatch",
        ),
    ];
    for (value, path, kind) in cases {
        let (found_path, found) = type_error(value.clone());
        assert_eq!(found_path, path, "{value}");
        assert!(format!("{found:?}").starts_with(kind), "{value}: {found:?}");
    }
    let requirement = expression(metre);
    let error = check_as(&requirement, "requirement", &Type::Boolean, &Environment).unwrap_err();
    assert_eq!(error.path, "requirement");
    assert!(matches!(error.kind, TypeErrorKind::Expected { .. }));
}

#[test]
fn well_typed_expressions_infer_their_types() {
    let infer = |value: serde_json::Value| check(&expression(value), "v", &Environment).unwrap();
    let metre =
        json!({"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "m"}});
    let area = Type::Number(Unit::of(Some(QuantityDimension::Area)));
    assert_eq!(
        infer(json!({"kind": "multiply", "left": property("Height"), "right": metre})),
        area
    );
    assert_eq!(
        infer(json!({"kind": "divide", "left": property("Height"), "right": metre})),
        Type::NUMBER
    );
    assert_eq!(
        infer(json!({"kind": "property", "propertySet": "axioval:measured", "property": "area"})),
        area
    );
    // A property of unknown type passes and is checked when read.
    assert_eq!(
        infer(json!({"kind": "add", "left": property("Load"), "right": metre})),
        Type::Number(Unit::of(Some(QuantityDimension::Length)))
    );
    assert_eq!(
        infer(json!({"kind": "add", "left": parameter("count"), "right": parameter("count")})),
        Type::Integer
    );
    assert_eq!(
        infer(json!({"kind": "tan", "operand": parameter("angle")})),
        Type::NUMBER
    );
    assert_eq!(
        infer(
            json!({"kind": "if", "branches": [{"when": property("External"), "then": metre}], "else": {"kind": "null"}})
        ),
        Type::Number(Unit::of(Some(QuantityDimension::Length)))
    );
}

#[test]
fn measured_member_fields_are_typed_inside_their_aggregate_only() {
    let field =
        |name: &str| json!({"kind": "property", "propertySet": "axioval:member", "property": name});
    let over = |value: serde_json::Value| {
        json!({"kind": "aggregate", "function": "max", "over": {"kind": "measured", "name": "steps"},
            "value": value})
    };
    let length = Type::Number(Unit::of(Some(QuantityDimension::Length)));
    let infer = |value: serde_json::Value| check(&expression(value), "v", &Environment);
    assert_eq!(infer(over(field("riser"))).unwrap(), length);
    let truth = json!({"kind": "aggregate", "function": "none",
        "over": {"kind": "measured", "name": "steps"}, "value": field("open_riser")});
    assert_eq!(infer(truth).unwrap(), Type::Boolean);
    // Outside the aggregate, and a field the list does not state.
    let error = infer(field("riser")).unwrap_err();
    assert!(
        format!("{:?}", error.kind).contains("only inside an aggregate"),
        "{error:?}"
    );
    let error = infer(over(field("slope"))).unwrap_err();
    assert_eq!(error.path, "v.aggregate.value");
    assert!(format!("{:?}", error.kind).contains("`steps` members state no `slope`"));
    // A nested aggregate over objects reads no member.
    let nested = over(json!({"kind": "aggregate", "function": "max",
        "over": {"kind": "path", "path": ["Hosts"]}, "value": field("riser")}));
    assert!(infer(nested).is_err());
    // Measured members take no `where`, and only a known list.
    let filtered: Result<Expression, _> = serde_json::from_value(json!({"kind": "aggregate",
        "function": "count", "over": {"kind": "measured", "name": "steps"},
        "where": {"kind": "all"}}));
    let error = filtered.unwrap().validate().unwrap_err().to_string();
    assert!(error.contains("takes no `where`"), "{error}");
    let unknown: Expression = serde_json::from_value(json!({"kind": "aggregate",
        "function": "count", "over": {"kind": "measured", "name": "treads"}}))
    .unwrap();
    assert!(
        unknown
            .validate()
            .unwrap_err()
            .to_string()
            .contains("no measured member list")
    );
}

/// Members handed to an aggregate as stated: membership and value.
struct Members(Vec<(bool, Value)>);

impl ExpressionContext for Members {
    fn property(&mut self, _: Option<&str>, name: &str) -> Leaf {
        Leaf::unreadable(format!("`{name}` is not read here"))
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        Leaf::unreadable(format!("`{name}` is unknown"))
    }

    fn members(
        &mut self,
        _: &axioval_ir::contract::AggregateSource,
        _: Option<&axioval_ir::contract::Selector>,
        _: Option<&Expression>,
        _: &str,
    ) -> Result<Vec<axioval_engine::expression::Member>, String> {
        Ok(self
            .0
            .iter()
            .map(|(certain, value)| axioval_engine::expression::Member {
                certain: *certain,
                value: Ok(value.clone()),
                evidence: Vec::new(),
            })
            .collect())
    }
}

fn aggregate(function: &str, members: Vec<(bool, Value)>) -> Result<Value, Reason> {
    let mut node = json!({"kind": "aggregate", "function": function,
        "over": {"kind": "path", "path": ["Hosts"]}});
    if function != "count" {
        node["value"] = json!({"kind": "null"});
    }
    evaluate(&expression(node), "a", &mut Members(members))
        .outcome
        .map_err(|why| why.reason)
}

#[test]
fn aggregates_widen_over_undecided_members_and_follow_kleene_logic() {
    let number = |value: f64| Value::number(value);
    let interval = |lower: f64, upper: f64| Value::Number {
        value: Interval::new(lower, upper).unwrap(),
        unit: Unit::NONE,
    };
    let members = || {
        vec![
            (true, number(2.0)),
            (true, number(5.0)),
            (false, number(3.0)),
        ]
    };
    assert_eq!(aggregate("count", members()), Ok(interval(2.0, 3.0)));
    assert_eq!(aggregate("sum", members()), Ok(interval(7.0, 10.0)));
    assert_eq!(aggregate("min", members()), Ok(number(2.0)));
    assert_eq!(aggregate("max", members()), Ok(number(5.0)));
    assert_eq!(
        aggregate("average", members()),
        Err(Reason::UndecidedMembers(1))
    );
    let lower = vec![(true, number(2.0)), (false, number(1.0))];
    assert_eq!(aggregate("min", lower), Ok(interval(1.0, 2.0)));
    assert_eq!(
        aggregate("distinctCount", members()),
        Ok(interval(2.0, 3.0))
    );
    assert_eq!(
        aggregate("average", vec![(true, number(2.0)), (true, number(4.0))]),
        Ok(number(3.0))
    );
    assert_eq!(aggregate("sum", Vec::new()), Ok(number(0.0)));
    assert_eq!(aggregate("max", Vec::new()), Ok(Value::Null));
    // Kleene logic over membership and truth.
    let truth = Value::Boolean;
    let cases = [
        (
            "any",
            vec![(true, truth(false)), (false, truth(true))],
            Err(Reason::UndecidedMembers(1)),
        ),
        (
            "any",
            vec![(true, truth(true)), (false, truth(false))],
            Ok(truth(true)),
        ),
        (
            "all",
            vec![(true, truth(true)), (false, truth(false))],
            Err(Reason::UndecidedMembers(1)),
        ),
        (
            "all",
            vec![(true, truth(true)), (false, truth(true))],
            Ok(truth(true)),
        ),
        ("all", vec![(false, truth(false))], Ok(truth(false))),
        ("all", Vec::new(), Ok(truth(false))),
        ("none", Vec::new(), Ok(truth(true))),
        ("none", vec![(false, truth(false))], Ok(truth(true))),
    ];
    for (function, members, expected) in cases {
        assert_eq!(
            aggregate(function, members.clone()),
            expected,
            "{function} {members:?}"
        );
    }
}

#[test]
fn an_explanation_keeps_the_deciding_path_and_bounds_the_rest() {
    let mut context = Context::default();
    context.parameters.insert("a".into(), Value::number(1.0));
    // Seventy-odd steps that hold, then one that fails.
    let mut operands: Vec<serde_json::Value> = (0..70)
        .map(|_| json!({"kind": "isDefined", "operand": parameter("a")}))
        .collect();
    operands.push(json!({"kind": "compare", "operator": "greaterThan",
        "left": parameter("a"), "right": {"kind": "literal", "value": {"type": "number", "value": 2.0}}}));
    let evaluation = evaluate(
        &expression(json!({"kind": "and", "operands": operands})),
        "r",
        &mut context,
    );
    assert_eq!(evaluation.outcome, Ok(Value::Boolean(false)));
    let explanation = evaluation.explain("r.and[70]");
    assert!(explanation.truncated);
    assert_eq!(
        explanation.entries.len(),
        axioval_ir::MAX_EXPLANATION_ENTRIES
    );
    let deciding: Vec<&str> = explanation
        .deciding()
        .map(|step| step.path.as_str())
        .collect();
    assert_eq!(
        deciding,
        [
            "r.and[70].compare.left",
            "r.and[70].compare.right",
            "r.and[70]",
            "r"
        ]
    );
}

/// A random tree over most kinds, its leaves of mixed types, so most trees
/// are ill-typed somewhere.
fn any_tree(random: &mut Random, depth: u32) -> serde_json::Value {
    let leaves = [
        json!({"kind": "literal", "value": {"type": "number", "value": 2.5}}),
        json!({"kind": "literal", "value": {"type": "integer", "value": -3}}),
        json!({"kind": "literal", "value": {"type": "quantity", "value": 4.0, "unit": "m"}}),
        json!({"kind": "literal", "value": {"type": "string", "value": "XC4"}}),
        json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
        json!({"kind": "null"}),
        parameter("a"),
        parameter("unknown"),
        json!({"kind": "property", "property": "Height"}),
    ];
    if depth == 0 || random.below(3) == 0 {
        return leaves[usize::try_from(random.below(leaves.len() as u64)).unwrap()].clone();
    }
    let mut next = || any_tree(random, depth - 1);
    let (a, b, c) = (next(), next(), next());
    let comparison = [
        "equals",
        "notEquals",
        "lessThan",
        "greaterThanOrEquals",
        "like",
        "matches",
        "contains",
    ];
    match random.below(22) {
        0 => json!({"kind": "and", "operands": [a, b]}),
        1 => json!({"kind": "or", "operands": [a, b, c]}),
        2 => json!({"kind": "not", "operand": a}),
        3 => json!({"kind": "implies", "antecedent": a, "consequent": b}),
        4 => {
            json!({"kind": "compare", "operator": comparison[usize::try_from(random.below(7)).unwrap()], "left": a, "right": b})
        }
        5 => json!({"kind": "between", "operand": a, "low": b, "high": c}),
        6 => json!({"kind": "oneOf", "operand": a, "values": [b, c]}),
        7 => json!({"kind": "if", "branches": [{"when": a, "then": b}], "else": c}),
        8 => json!({"kind": "coalesce", "operands": [a, b]}),
        9 => json!({"kind": "add", "left": a, "right": b}),
        10 => json!({"kind": "divide", "left": a, "right": b}),
        11 => json!({"kind": "multiply", "left": a, "right": b}),
        12 => json!({"kind": "min", "operands": [a, b]}),
        13 => json!({"kind": "round", "operand": a, "step": b}),
        14 => json!({"kind": "sqrt", "operand": a}),
        15 => json!({"kind": "tan", "operand": a}),
        16 => json!({"kind": "atan2", "y": a, "x": b}),
        17 => json!({"kind": "convertSlope", "operand": a, "from": "angle", "to": "percent"}),
        18 => json!({"kind": "concat", "operands": [a, b]}),
        19 => json!({"kind": "length", "operand": a}),
        20 => json!({"kind": "isDefined", "operand": a}),
        _ => json!({"kind": "xor", "left": a, "right": b}),
    }
}

#[test]
fn fuzzing_trees_never_panics_and_never_yields_an_unsound_interval() {
    let mut random = Random(0xF022);
    for _ in 0..3000 {
        let tree: Expression = serde_json::from_value(any_tree(&mut random, 4)).unwrap();
        let _ = tree.validate();
        let _ = check(&tree, "fuzz", &Environment);
        let mut context = Context::default();
        context.parameters.insert(
            "a".into(),
            Value::Number {
                value: Interval::new(-1.0, 2.0).unwrap(),
                unit: Unit::NONE,
            },
        );
        context
            .properties
            .insert("Height".into(), measured(0.5, 0.75));
        let evaluation = evaluate(&tree, "fuzz", &mut context);
        if let Ok(Value::Number { value, .. }) = evaluation.outcome {
            assert!(value.lower <= value.upper, "{value:?}");
            assert!(
                value.lower.is_finite() && value.upper.is_finite(),
                "{value:?}"
            );
        }
        // Every explanation keeps its deciding path whole.
        let explanation = evaluation.explain("fuzz");
        assert!(explanation.entries.len() <= evaluation.trace.len());
    }
}

#[test]
fn fuzzing_text_never_panics() {
    use axioval_engine::expression::parse_text;
    let pieces = [
        "area", " ", "+", "-", "×", "÷", "(", ")", ",", "2", "3.5", " m", " EUR/m²", "e", "min",
        "max", "if", "and", "or", "not", "==", "<=", "\"", "office", "≥", "round", "^", "²", "·",
    ];
    let mut random = Random(0x7E87);
    let mut parsed = 0;
    for _ in 0..5000 {
        let text: String = (0..random.below(14))
            .map(|_| pieces[usize::try_from(random.below(pieces.len() as u64)).unwrap()])
            .collect();
        if let Ok(tree) = parse_text(&text) {
            parsed += 1;
            let _ = tree.validate();
            let _ = check(&tree, "text", &Environment);
            let _ = evaluate(&tree, "text", &mut Context::default());
        }
    }
    assert!(parsed > 100, "only {parsed} texts parsed");
}
