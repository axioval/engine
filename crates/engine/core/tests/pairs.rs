//! `pairs`: consecutive members of an ordered sequence, `previous` and
//! `next` within a pair, and how an undecided order stays undecided.
#![allow(missing_docs, clippy::float_cmp)]
use std::rc::Rc;

use axioval_engine::expression::{
    ExpressionContext, Interval, Leaf, NotEvaluated, PairRead, Reason, Sequenced, Source, Type,
    TypeEnvironment, Unit, Value, check, evaluate,
};
use axioval_ir::contract::{AggregateSource, Expression, ExpressionError, Selector};
use serde_json::json;

fn expression(value: serde_json::Value) -> Expression {
    let expression: Expression = serde_json::from_value(value).unwrap();
    expression.validate().unwrap();
    expression
}

/// One member: whether it surely belongs, what orders it (`None`: not
/// evaluated), its height in metres (`None`: not evaluated) and its name.
#[derive(Clone)]
struct Riser {
    certain: bool,
    order: Option<Value>,
    height: Option<f64>,
    name: &'static str,
}

fn riser(order: f64, height_mm: f64, name: &'static str) -> Riser {
    Riser {
        certain: true,
        order: Some(Value::number(order)),
        height: Some(height_mm / 1000.0),
        name,
    }
}

/// A flight's risers listed for a `pairs`, a property `h` read of the one
/// in scope; the pair in scope, and the members the evaluation related.
struct Flight {
    risers: Rc<Vec<Riser>>,
    scope: Option<usize>,
    pair: Option<(usize, usize)>,
    chosen: Vec<usize>,
}

impl Flight {
    fn new(risers: Vec<Riser>) -> Self {
        Self {
            risers: Rc::new(risers),
            scope: None,
            pair: None,
            chosen: vec![usize::MAX],
        }
    }

    fn within(&self, scope: usize, pair: Option<(usize, usize)>) -> Self {
        Self {
            risers: self.risers.clone(),
            scope: Some(scope),
            pair,
            chosen: Vec::new(),
        }
    }
}

fn unread(path: &str) -> NotEvaluated {
    NotEvaluated {
        path: path.into(),
        label: None,
        reason: Reason::Unreadable(format!("{path} cannot be read")),
    }
}

impl ExpressionContext for Flight {
    fn property(&mut self, _: Option<&str>, name: &str) -> Leaf {
        let height = self
            .scope
            .and_then(|scope| self.risers[scope].height)
            .filter(|_| name == "h");
        height.map_or_else(
            || Leaf::unreadable(format!("`{name}` is not read here")),
            |height| {
                Leaf::stated(Value::Number {
                    value: Interval::point(height),
                    unit: Unit::of(Some(axioval_ir::QuantityDimension::Length)),
                })
            },
        )
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        Leaf::unreadable(format!("`{name}` is unknown"))
    }

    fn sequenced_members(
        &mut self,
        _: &AggregateSource,
        _: Option<&Selector>,
        (_, by_path): (&Expression, &str),
    ) -> Result<Vec<Sequenced>, String> {
        Ok(self
            .risers
            .iter()
            .map(|riser| Sequenced {
                certain: riser.certain,
                by: riser.order.clone().ok_or_else(|| unread(by_path)),
                evidence: Vec::new(),
                name: riser.name.to_owned(),
            })
            .collect())
    }

    fn pair_value(
        &mut self,
        earlier: usize,
        later: usize,
        (value, path): (&Expression, &str),
    ) -> PairRead {
        let mut scope = self.within(later, Some((earlier, later)));
        (evaluate(value, path, &mut scope).outcome, Vec::new())
    }

    fn neighbour(&mut self, previous: bool, (operand, path): (&Expression, &str)) -> PairRead {
        let Some((earlier, later)) = self.pair else {
            return (Err(unread(path)), Vec::new());
        };
        let mut scope = self.within(if previous { earlier } else { later }, None);
        (evaluate(operand, path, &mut scope).outcome, Vec::new())
    }

    fn chosen(&mut self, chosen: &[usize]) {
        self.chosen = chosen.to_vec();
    }
}

fn height() -> serde_json::Value {
    json!({"kind": "property", "propertySet": "ex:pset-stair-flight-common", "property": "h"})
}

fn millimetres(value: f64) -> serde_json::Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "mm"}})
}

/// Consecutive risers differ by at most 5 mm.
fn within_five() -> serde_json::Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals",
        "left": {"kind": "abs", "operand": {"kind": "subtract", "left": height(),
            "right": {"kind": "previous", "operand": height()}}},
        "right": millimetres(5.0)})
}

fn pairs(function: &str, value: Option<serde_json::Value>) -> Expression {
    expression(pairs_node(function, value))
}

fn pairs_node(function: &str, value: Option<serde_json::Value>) -> serde_json::Value {
    let mut node = json!({"kind": "pairs", "function": function,
        "over": {"kind": "path", "path": ["ex:rel-aggregates"]},
        "by": {"kind": "property", "propertySet": "ex:pset-stair-flight-common",
            "property": "order"}});
    if let Some(value) = value {
        node["value"] = value;
    }
    node
}

/// `function` over `risers`: its outcome, the members related, and the
/// pairs named.
fn over(
    function: &str,
    value: Option<serde_json::Value>,
    risers: Vec<Riser>,
) -> (Result<Value, Reason>, Vec<usize>, Vec<String>) {
    let mut context = Flight::new(risers);
    let evaluation = evaluate(&pairs(function, value), "e", &mut context);
    let named = evaluation
        .reads
        .iter()
        .find_map(|read| match &read.source {
            Source::Pairs { chosen, .. } => Some(chosen.clone()),
            _ => None,
        })
        .unwrap();
    (
        evaluation.outcome.map_err(|why| why.reason),
        context.chosen,
        named,
    )
}

/// #350: risers of 172, 175 and 181 mm with a 5 mm limit on consecutive
/// differences fail at the pair 175 → 181, which names both members; the
/// listing order is not the sequence's.
#[test]
fn consecutive_risers_fail_at_the_pair_differing_by_more_than_the_limit() {
    let risers = vec![
        riser(3.0, 181.0, "r3"),
        riser(1.0, 172.0, "r1"),
        riser(2.0, 175.0, "r2"),
    ];
    let (outcome, chosen, named) = over("all", Some(within_five()), risers.clone());
    assert_eq!(outcome, Ok(Value::Boolean(false)));
    assert_eq!(chosen, vec![0, 2]);
    assert_eq!(named, vec!["r2 → r3"]);
    // Every pair differing by at most 6.5 mm holds.
    let limit = |mm: f64| {
        let mut value = within_five();
        value["right"] = millimetres(mm);
        value
    };
    let (outcome, chosen, named) = over("all", Some(limit(6.5)), risers.clone());
    assert_eq!(outcome, Ok(Value::Boolean(true)));
    assert!(chosen.is_empty() && named.is_empty());
    // `any` names the pair that holds, `none` the pair that fails it.
    let (outcome, _, named) = over("any", Some(within_five()), risers.clone());
    assert_eq!(
        (outcome, named),
        (Ok(Value::Boolean(true)), vec!["r1 → r2".to_owned()])
    );
    let (outcome, _, named) = over("none", Some(within_five()), risers.clone());
    assert_eq!(
        (outcome, named),
        (Ok(Value::Boolean(false)), vec!["r1 → r2".to_owned()])
    );
    // The largest difference, its pair named; a count of pairs.
    let difference = json!({"kind": "subtract", "left": height(),
        "right": {"kind": "previous", "operand": height()}});
    let (outcome, _, named) = over("max", Some(difference.clone()), risers.clone());
    let Ok(Value::Number { value, unit }) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(unit, Unit::of(Some(axioval_ir::QuantityDimension::Length)));
    assert!((value.lower - 0.006).abs() < 1e-12 && value.upper - value.lower < 1e-12);
    assert_eq!(named, vec!["r2 → r3"]);
    let (outcome, _, _) = over("count", None, risers.clone());
    assert_eq!(outcome, Ok(Value::number(2.0)));
    // `next` reads the later member, as the pair's own reads do.
    let symmetric = json!({"kind": "subtract",
        "left": {"kind": "next", "operand": height()},
        "right": {"kind": "previous", "operand": height()}});
    let (outcome, _, _) = over("min", Some(symmetric), risers);
    let Ok(Value::Number { value, .. }) = outcome else {
        panic!("{outcome:?}");
    };
    assert!((value.lower - 0.003).abs() < 1e-12 && value.upper - value.lower < 1e-12);
}

fn ordered(lower: f64, upper: f64) -> Value {
    Value::Number {
        value: Interval::new(lower, upper).unwrap(),
        unit: Unit::NONE,
    }
}

fn at(order: Value, height_mm: f64, name: &'static str) -> Riser {
    Riser {
        certain: true,
        order: Some(order),
        height: Some(height_mm / 1000.0),
        name,
    }
}

/// An order the intervals leave undecided is never guessed: the pairs it
/// touches are not evaluated, which an `all` decides only where a decided
/// pair fails, and never as a pass.
#[test]
fn an_undecided_order_leaves_its_pairs_undecided() {
    // `b` and `c` may come either way round: every pair touching them is
    // undecided, and so is the `all`, though each listed pair holds.
    let undecided = vec![
        at(ordered(1.0, 1.0), 172.0, "a"),
        at(ordered(2.0, 3.0), 175.0, "b"),
        at(ordered(2.5, 3.5), 176.0, "c"),
    ];
    let (outcome, chosen, named) = over("all", Some(within_five()), undecided.clone());
    assert!(
        matches!(&outcome, Err(Reason::UndecidedOrder(earlier, later)) if earlier == "a" && later == "b"),
        "{outcome:?}"
    );
    assert!(chosen.is_empty() && named.is_empty());
    let (outcome, _, _) = over("max", Some(height()), undecided.clone());
    assert!(
        matches!(outcome, Err(Reason::UndecidedOrder(..))),
        "{outcome:?}"
    );
    // Their count is known whatever the order.
    let (outcome, _, _) = over("count", None, undecided.clone());
    assert_eq!(outcome, Ok(Value::number(2.0)));
    // `d` follows `b` or `c`: that pair is undecided too, though it fails
    // either way. A decided pair failing decides the `all`.
    let mut failing = undecided.clone();
    failing.push(at(ordered(10.0, 10.0), 190.0, "d"));
    let (outcome, _, _) = over("all", Some(within_five()), failing.clone());
    assert!(
        matches!(outcome, Err(Reason::UndecidedOrder(..))),
        "{outcome:?}"
    );
    failing.push(at(ordered(11.0, 11.0), 172.0, "e"));
    let (outcome, _, named) = over("all", Some(within_five()), failing);
    assert_eq!(
        (outcome, named),
        (Ok(Value::Boolean(false)), vec!["d → e".to_owned()])
    );
    // Equal points are no order either.
    let tied = vec![
        at(ordered(1.0, 1.0), 172.0, "a"),
        at(ordered(1.0, 1.0), 172.0, "b"),
    ];
    let (outcome, _, _) = over("all", Some(within_five()), tied);
    assert!(
        matches!(outcome, Err(Reason::UndecidedOrder(..))),
        "{outcome:?}"
    );
    // Touching intervals may share a point; separated ones decide.
    let touching = vec![
        at(ordered(1.0, 2.0), 172.0, "a"),
        at(ordered(2.0, 3.0), 175.0, "b"),
    ];
    let (outcome, _, _) = over("all", Some(within_five()), touching);
    assert!(
        matches!(outcome, Err(Reason::UndecidedOrder(..))),
        "{outcome:?}"
    );
    let apart = vec![
        at(ordered(1.0, 1.9), 172.0, "a"),
        at(ordered(2.0, 3.0), 175.0, "b"),
    ];
    assert_eq!(
        over("all", Some(within_five()), apart).0,
        Ok(Value::Boolean(true))
    );
}

/// Undecided membership, an order not evaluated and `null` follow the
/// aggregates: a member that may be absent leaves the pairs around it and
/// the pair bridging it possible, an order not evaluated may lie anywhere,
/// and `null` never mixes with not evaluated.
#[test]
fn pairs_over_undecided_unread_and_null_members() {
    let mut maybe = riser(2.0, 200.0, "maybe");
    maybe.certain = false;
    // With the possible member the pairs around it fail; without it the
    // bridging pair holds: undecided.
    let bridged = vec![
        riser(1.0, 172.0, "a"),
        maybe.clone(),
        riser(3.0, 175.0, "c"),
    ];
    let (outcome, _, _) = over("all", Some(within_five()), bridged);
    assert_eq!(outcome, Err(Reason::UndecidedMembers(1)));
    // A possible member agreeing with both sides cannot change the result.
    maybe.height = Some(0.173);
    let agreeing = vec![riser(1.0, 172.0, "a"), maybe, riser(3.0, 175.0, "c")];
    assert_eq!(
        over("all", Some(within_five()), agreeing.clone()).0,
        Ok(Value::Boolean(true))
    );
    // The count of pairs widens over it.
    let (outcome, _, _) = over("count", None, agreeing);
    assert_eq!(outcome, Ok(ordered(1.0, 2.0)));
    // An order not evaluated may lie anywhere: no pair is decided.
    let mut unread_order = riser(2.0, 175.0, "b");
    unread_order.order = None;
    let (outcome, _, _) = over(
        "all",
        Some(within_five()),
        vec![riser(1.0, 172.0, "a"), unread_order, riser(3.0, 181.0, "c")],
    );
    assert!(
        matches!(&outcome, Err(Reason::Unreadable(why)) if why.contains("e.pairs.by")),
        "{outcome:?}"
    );
    // A sure member stating no order makes it `null`; an undecided one
    // leaves it not evaluated.
    let mut unordered = riser(2.0, 175.0, "b");
    unordered.order = Some(Value::Null);
    let members = vec![riser(1.0, 172.0, "a"), unordered.clone()];
    assert_eq!(over("all", Some(within_five()), members).0, Ok(Value::Null));
    unordered.certain = false;
    let members = vec![riser(1.0, 172.0, "a"), unordered];
    assert_eq!(
        over("all", Some(within_five()), members).0,
        Err(Reason::UndecidedMembers(1))
    );
    // A height not evaluated leaves its pair open: an `all` holds nowhere.
    let mut unread_height = riser(2.0, 175.0, "b");
    unread_height.height = None;
    let (outcome, _, _) = over(
        "all",
        Some(within_five()),
        vec![riser(1.0, 172.0, "a"), unread_height],
    );
    assert!(matches!(outcome, Err(Reason::Unreadable(_))), "{outcome:?}");
    // One member has no pair: `all` over none fails, `any` too.
    let one = vec![riser(1.0, 172.0, "a")];
    assert_eq!(
        over("all", Some(within_five()), one.clone()).0,
        Ok(Value::Boolean(false))
    );
    assert_eq!(over("count", None, one).0, Ok(Value::number(0.0)));
    // An order that is no number.
    let mut named_order = riser(1.0, 172.0, "a");
    named_order.order = Some(Value::Text("first".into()));
    let (outcome, _, _) = over("all", Some(within_five()), vec![named_order]);
    assert!(matches!(outcome, Err(Reason::Mismatch(_))), "{outcome:?}");
}

struct Environment;

impl TypeEnvironment for Environment {
    fn property(&self, _: Option<&str>, name: &str) -> Result<Type, String> {
        Ok(match name {
            "h" | "order" => Type::Number(Unit::of(Some(axioval_ir::QuantityDimension::Length))),
            _ => Type::Text,
        })
    }

    fn parameter(&self, name: &str) -> Result<Type, String> {
        Err(format!("`{name}` is unknown"))
    }
}

#[test]
fn pairs_are_typed_by_their_function_and_previous_only_within_their_value() {
    assert_eq!(
        check(&pairs("all", Some(within_five())), "v", &Environment).unwrap(),
        Type::Boolean
    );
    let difference = json!({"kind": "subtract", "left": height(),
        "right": {"kind": "previous", "operand": height()}});
    assert_eq!(
        check(&pairs("max", Some(difference)), "v", &Environment).unwrap(),
        Type::Number(Unit::of(Some(axioval_ir::QuantityDimension::Length)))
    );
    assert_eq!(
        check(&pairs("count", None), "v", &Environment).unwrap(),
        Type::Integer
    );
    // The order is a number.
    let mut node = serde_json::to_value(pairs("count", None)).unwrap();
    node["by"] = json!({"kind": "property", "property": "class"});
    let error = check(&expression(node), "v", &Environment).unwrap_err();
    assert_eq!(error.path, "v.pairs.by");
    // `previous` and `next` read a pair only within its value: not
    // outside one, in its order, or in a member scope nested within it.
    let previous = json!({"kind": "previous", "operand": height()});
    let refused = |value: serde_json::Value| {
        serde_json::from_value::<Expression>(value)
            .unwrap()
            .validate()
    };
    assert_eq!(
        refused(previous.clone()),
        Err(ExpressionError::OutsidePairs { kind: "previous" })
    );
    let mut node = serde_json::to_value(pairs("count", None)).unwrap();
    node["by"] = previous.clone();
    assert_eq!(
        refused(node),
        Err(ExpressionError::OutsidePairs { kind: "previous" })
    );
    let nested = json!({"kind": "aggregate", "function": "max",
        "over": {"kind": "path", "path": ["ex:rel-aggregates"]}, "value": previous});
    assert_eq!(
        refused(pairs_node("max", Some(nested))),
        Err(ExpressionError::OutsidePairs { kind: "previous" })
    );
    let next = json!({"kind": "next", "operand": height()});
    assert_eq!(
        refused(next),
        Err(ExpressionError::OutsidePairs { kind: "next" })
    );
    // A count takes no value, any other function one.
    assert!(
        serde_json::from_value::<Expression>(json!({"kind": "pairs", "function": "all",
            "over": {"kind": "path", "path": ["ex:rel-aggregates"]}, "by": height()}))
        .unwrap()
        .validate()
        .is_err()
    );
}
