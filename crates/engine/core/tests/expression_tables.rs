//! Reading table parameters between their rows: `interpolate`.
#![allow(missing_docs, clippy::float_cmp)]
use std::collections::BTreeMap;

use axioval_engine::expression::{
    ExpressionContext, Interval, Leaf, Reason, Type, TypeEnvironment, TypeErrorKind, Unit, Value,
    check, evaluate,
};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{Expression, ParameterValue, TableRow};
use serde_json::json;

fn expression(value: serde_json::Value) -> Expression {
    let expression: Expression = serde_json::from_value(value).unwrap();
    expression.validate().unwrap();
    expression
}

/// Table parameters and plain parameters by name.
#[derive(Default)]
struct Context {
    tables: BTreeMap<String, Vec<TableRow>>,
    parameters: BTreeMap<String, Value>,
}

impl ExpressionContext for Context {
    fn property(&mut self, _: Option<&str>, name: &str) -> Leaf {
        Leaf::unreadable(format!("`{name}` is not measured"))
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        self.parameters.get(name).cloned().map_or_else(
            || Leaf::unreadable(format!("`{name}` is unknown")),
            Leaf::stated,
        )
    }

    fn table(&mut self, table: &str, read: &mut dyn FnMut(&[TableRow]) -> Leaf) -> Leaf {
        match self.tables.get(table) {
            Some(rows) => read(rows),
            None => Leaf::unreadable(format!("no table `{table}`")),
        }
    }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect()
}

/// `key` → `value` rows of plain numbers.
fn pairs(pairs: &[(f64, f64)]) -> Vec<TableRow> {
    pairs
        .iter()
        .map(|&(key, value)| row(&[("key", number(key)), ("value", number(value))]))
        .collect()
}

fn plain(lower: f64, upper: f64) -> Value {
    Value::Number {
        value: Interval::new(lower, upper).unwrap(),
        unit: Unit::NONE,
    }
}

fn interpolate(key: &serde_json::Value, clamp: bool) -> Expression {
    let mut node = json!({"kind": "interpolate", "table": "t", "keyColumn": "key",
        "key": key, "column": "value"});
    if clamp {
        node["clamp"] = json!(true);
    }
    expression(node)
}

fn key() -> serde_json::Value {
    json!({"kind": "parameter", "name": "k"})
}

/// Interpolates `t` at the parameter `k` = `at`.
fn at(rows: Vec<TableRow>, at: Value, clamp: bool) -> Result<Value, (Reason, String)> {
    let mut context = Context::default();
    context.tables.insert("t".into(), rows);
    context.parameters.insert("k".into(), at);
    evaluate(&interpolate(&key(), clamp), "v", &mut context)
        .outcome
        .map_err(|why| (why.reason.clone(), why.to_string()))
}

fn interval(value: &Value) -> Interval {
    match value {
        Value::Number { value, .. } => *value,
        other => panic!("{other:?} is no number"),
    }
}

/// Whether `value` holds `[lower, upper]` and lies within rounding of it.
fn about(value: &Value, lower: f64, upper: f64) -> bool {
    let value = interval(value);
    value.lower <= lower
        && upper <= value.upper
        && lower - value.lower < 1e-12
        && value.upper - upper < 1e-12
}

#[test]
fn a_key_between_two_rows_is_interpolated_linearly() {
    let rows = pairs(&[(2.0, 10.0), (3.0, 20.0)]);
    let value = at(rows.clone(), Value::number(2.4), false).unwrap();
    assert!(about(&value, 14.0, 14.0), "{value}");
    // A key on a row reads its cell exactly.
    assert_eq!(
        at(rows.clone(), Value::number(3.0), false),
        Ok(Value::number(20.0))
    );
    assert_eq!(at(rows, Value::number(2.0), false), Ok(Value::number(10.0)));
}

#[test]
fn an_interval_key_gives_every_value_its_keys_allow() {
    let rows = pairs(&[(2.0, 10.0), (3.0, 20.0)]);
    let value = at(rows, plain(2.4, 2.6), false).unwrap();
    assert!(about(&value, 14.0, 16.0), "{value}");
    // Across a row whose value is a peak, the peak is in the result.
    let peaked = pairs(&[(1.0, 0.0), (2.0, 10.0), (3.0, 0.0)]);
    let value = at(peaked, plain(1.5, 2.5), false).unwrap();
    assert_eq!(interval(&value), Interval::new(5.0, 10.0).unwrap());
}

#[test]
fn a_key_beyond_the_rows_is_not_evaluated_unless_clamping_is_declared() {
    let rows = pairs(&[(2.0, 10.0), (3.0, 20.0)]);
    let (reason, message) = at(rows.clone(), Value::number(3.5), false).unwrap_err();
    assert!(matches!(reason, Reason::Unreadable(_)), "{reason:?}");
    assert!(
        message.contains("`key` 3.5 lies beyond its last row's, 3, in table `t`")
            && message.contains("`clamp`"),
        "{message}"
    );
    let (_, message) = at(rows.clone(), Value::number(1.0), false).unwrap_err();
    assert!(message.contains("before its first row's, 2"), "{message}");
    // A key interval reaching beyond the rows is not evaluated either.
    assert!(at(rows.clone(), plain(2.5, 3.5), false).is_err());
    // Clamping takes the nearest end row's value.
    assert_eq!(
        at(rows.clone(), Value::number(3.5), true),
        Ok(Value::number(20.0))
    );
    assert_eq!(
        at(rows.clone(), Value::number(1.0), true),
        Ok(Value::number(10.0))
    );
    let value = at(rows, plain(2.5, 3.5), true).unwrap();
    assert!(about(&value, 15.0, 20.0), "{value}");
}

#[test]
fn units_are_checked_and_cells_converted_like_other_arithmetic() {
    // A span table in metres, read at a key stated in millimetres.
    let rows = vec![
        row(&[("key", metres(2.0)), ("value", metres(0.02))]),
        row(&[("key", metres(3.0)), ("value", metres(0.03))]),
    ];
    let span = Value::quantity(Interval::point(2.5), QuantityDimension::Length);
    let value = at(rows.clone(), span, false).unwrap();
    assert!(about(&value, 0.025, 0.025), "{value}");
    let Value::Number { unit, .. } = value else {
        unreachable!()
    };
    assert_eq!(unit, Unit::of(Some(QuantityDimension::Length)));
    // A plain number is no span: a mismatch, never a row.
    let (reason, message) = at(rows, Value::number(2.5), false).unwrap_err();
    assert!(matches!(reason, Reason::Mismatch(_)), "{reason:?}");
    assert!(
        message.contains("`key` is read at 1, but row 1 of table `t` states m"),
        "{message}"
    );
}

#[test]
fn null_gaps_and_repeated_keys_are_told_apart() {
    let rows = pairs(&[(2.0, 10.0), (3.0, 20.0)]);
    assert_eq!(at(rows, Value::Null, false), Ok(Value::Null));
    // A bracketing row stating no value names its key.
    let gap = vec![
        row(&[("key", number(2.0)), ("value", number(10.0))]),
        row(&[("key", number(3.0))]),
    ];
    let (reason, message) = at(gap, Value::number(2.5), false).unwrap_err();
    assert!(matches!(reason, Reason::Unreadable(_)));
    assert!(
        message.contains("table `t` states no `value` for `key` = 3"),
        "{message}"
    );
    // Two rows at one key are no single value.
    let repeated = pairs(&[(2.0, 10.0), (2.0, 11.0), (3.0, 20.0)]);
    let (_, message) = at(repeated, Value::number(2.5), false).unwrap_err();
    assert!(
        message.contains("rows 1, 2 of table `t` state the same keys, `key` = 2"),
        "{message}"
    );
    // A row without a key cannot be placed.
    let keyless = vec![
        row(&[("key", number(2.0)), ("value", number(10.0))]),
        row(&[("value", number(20.0))]),
    ];
    let (_, message) = at(keyless, Value::number(2.0), false).unwrap_err();
    assert!(
        message.contains("row 2 of table `t` states no `key`"),
        "{message}"
    );
}

/// `SplitMix64`, so every run tests the same cases.
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
}

#[test]
fn interpolated_intervals_hold_the_value_at_every_key_they_allow() {
    let mut random = Random(0x7AB1E);
    for _ in 0..1000 {
        let mut keys: Vec<f64> = (0..5).map(|_| random.unit() * 10.0).collect();
        keys.sort_by(f64::total_cmp);
        keys.dedup();
        let values: Vec<f64> = keys.iter().map(|_| random.unit() * 100.0 - 50.0).collect();
        let rows = pairs(
            &keys
                .iter()
                .copied()
                .zip(values.iter().copied())
                .collect::<Vec<_>>(),
        );
        let (first, last) = (keys[0], keys[keys.len() - 1]);
        let lower = first + (last - first) * random.unit();
        let upper = lower + (last - lower) * random.unit();
        let value = interval(&at(rows, plain(lower, upper), false).unwrap());
        for _ in 0..8 {
            let point = lower + (upper - lower) * random.unit();
            let segment = keys
                .windows(2)
                .position(|pair| pair[0] <= point && point <= pair[1]);
            let exact = match segment {
                Some(index) => {
                    let (k0, k1) = (keys[index], keys[index + 1]);
                    let (v0, v1) = (values[index], values[index + 1]);
                    v0 + (point - k0) / (k1 - k0) * (v1 - v0)
                }
                None => values[0],
            };
            assert!(
                value.lower - 1e-9 <= exact && exact <= value.upper + 1e-9,
                "{exact} at {point} outside {value:?}"
            );
        }
    }
}

/// The table `t`: `key` a length, `value` a plain number, `class` text.
struct Environment;

impl TypeEnvironment for Environment {
    fn property(&self, _: Option<&str>, name: &str) -> Result<Type, String> {
        Err(format!("no property `{name}`"))
    }

    fn parameter(&self, name: &str) -> Result<Type, String> {
        match name {
            "span" => Ok(Type::Number(Unit::of(Some(QuantityDimension::Length)))),
            "count" => Ok(Type::Integer),
            other => Err(format!("no parameter `{other}`")),
        }
    }

    fn lookup(&self, table: &str, column: &str) -> Result<(BTreeMap<String, Type>, Type), String> {
        if table != "t" {
            return Err(format!("no table `{table}`"));
        }
        let columns: BTreeMap<String, Type> = [
            (
                "key",
                Type::Number(Unit::of(Some(QuantityDimension::Length))),
            ),
            ("value", Type::Integer),
            ("class", Type::Text),
        ]
        .into_iter()
        .map(|(id, kind)| (id.to_owned(), kind))
        .collect();
        let found = columns
            .get(column)
            .cloned()
            .ok_or_else(|| format!("no column `{column}`"))?;
        Ok((columns, found))
    }
}

#[test]
fn interpolation_is_typed_by_its_columns_and_key() {
    let typed = |node: serde_json::Value| check(&expression(node), "v", &Environment);
    let node = |key_column: &str, key: &str, column: &str| {
        json!({"kind": "interpolate", "table": "t", "keyColumn": key_column,
            "key": {"kind": "parameter", "name": key}, "column": column})
    };
    // Whole numbers interpolate to numbers.
    assert_eq!(typed(node("key", "span", "value")), Ok(Type::NUMBER));
    let error = typed(node("key", "count", "value")).unwrap_err();
    assert_eq!(error.path, "v.interpolate.key");
    assert!(matches!(error.kind, TypeErrorKind::UnitMismatch { .. }));
    let error = typed(node("class", "span", "value")).unwrap_err();
    assert!(matches!(error.kind, TypeErrorKind::NotNumeric(Type::Text)));
    let error = typed(node("key", "span", "class")).unwrap_err();
    assert!(matches!(error.kind, TypeErrorKind::NotNumeric(Type::Text)));
    let error = typed(node("width", "span", "value")).unwrap_err();
    assert!(matches!(error.kind, TypeErrorKind::Unknown(_)));
}
