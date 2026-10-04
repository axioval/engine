//! Naming the expression nodes a profile cannot state.
#![allow(missing_docs)]

use axioval_export::precheck::{ExpressionNode, expression_nodes, unsupported_expression_node};
use axioval_export::{ExportOutcome, ExportProfile};
use axioval_ir::DefinitionPackage;
use axioval_ir::RuleSetPackage;
use axioval_ir::contract::Expression;
use serde_json::json;

fn expression(value: serde_json::Value) -> Expression {
    serde_json::from_value(value).expect("an expression")
}

fn property(name: &str) -> serde_json::Value {
    json!({"kind": "property", "propertySet": "P", "property": name})
}

fn number(value: f64) -> serde_json::Value {
    json!({"kind": "literal", "value": {"type": "number", "value": value}})
}

/// `and(isDefined(A), A > 1, sum over openings of their area <= B * 0.4)`.
fn openings() -> Expression {
    expression(json!({"kind": "and", "operands": [
        {"kind": "isDefined", "operand": property("A")},
        {"kind": "compare", "operator": "greaterThan", "left": property("A"), "right": number(1.0)},
        {"kind": "compare", "operator": "lessThanOrEquals",
         "left": {"kind": "aggregate", "function": "sum",
                  "over": {"kind": "path", "path": ["IfcRelVoidsElement:forward"]},
                  "value": property("Area")},
         "right": {"kind": "multiply", "left": property("B"), "right": number(0.4)}}
    ]}))
}

#[test]
fn nodes_are_named_by_the_engines_paths_in_written_order() {
    let tree = openings();
    let nodes: Vec<(String, &str)> = expression_nodes(&tree, "requirement")
        .into_iter()
        .map(|(path, node)| (path, node.kind()))
        .collect();
    let expected = [
        ("requirement", "and"),
        ("requirement.and[0]", "isDefined"),
        ("requirement.and[0].isDefined.operand", "property"),
        ("requirement.and[1]", "compare"),
        ("requirement.and[1].compare.left", "property"),
        ("requirement.and[1].compare.right", "literal"),
        ("requirement.and[2]", "compare"),
        ("requirement.and[2].compare.left", "aggregate"),
        (
            "requirement.and[2].compare.left.aggregate.value",
            "property",
        ),
        ("requirement.and[2].compare.right", "multiply"),
        ("requirement.and[2].compare.right.multiply.left", "property"),
        ("requirement.and[2].compare.right.multiply.right", "literal"),
    ];
    let expected: Vec<(String, &str)> = expected
        .iter()
        .map(|(path, kind)| ((*path).to_owned(), *kind))
        .collect();
    assert_eq!(nodes, expected);
}

#[test]
fn the_first_unsupported_node_is_named() {
    let tree = openings();
    let plain = ["and", "isDefined", "property", "compare", "literal"];
    let node = unsupported_expression_node(&tree, "requirement", &plain).expect("a node");
    assert_eq!(
        node,
        ExpressionNode {
            path: "requirement.and[2].compare.left".to_owned(),
            kind: "aggregate",
        }
    );
    assert_eq!(
        node.to_string(),
        "expression node `requirement.and[2].compare.left` (`aggregate`)"
    );
    // With aggregates, the arithmetic is next.
    let with_aggregates = [
        "and",
        "isDefined",
        "property",
        "compare",
        "literal",
        "aggregate",
    ];
    assert_eq!(
        unsupported_expression_node(&tree, "requirement", &with_aggregates)
            .expect("a node")
            .path,
        "requirement.and[2].compare.right"
    );
    // No kind at all: the root.
    assert_eq!(
        unsupported_expression_node(&tree, "requirement", &[])
            .expect("a node")
            .path,
        "requirement"
    );
    let every = [
        "and",
        "isDefined",
        "property",
        "compare",
        "literal",
        "aggregate",
        "multiply",
    ];
    assert_eq!(
        unsupported_expression_node(&tree, "requirement", &every),
        None
    );
}

#[test]
fn branches_values_and_member_filters_have_paths() {
    let tree = expression(json!({"kind": "if",
        "branches": [{"when": {"kind": "oneOf", "operand": property("A"),
                               "values": [number(1.0), number(2.0)]},
                      "then": {"kind": "aggregate", "function": "count",
                               "over": {"kind": "path", "path": ["IfcRelAggregates:forward"]},
                               "where": {"kind": "expression",
                                         "expression": {"kind": "isDefined", "operand": property("C")}}}}],
        "else": {"kind": "null"}}));
    let paths: Vec<String> = expression_nodes(&tree, "value")
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        paths,
        [
            "value",
            "value.if.branches[0].when",
            "value.if.branches[0].when.oneOf.operand",
            "value.if.branches[0].when.oneOf.values[0]",
            "value.if.branches[0].when.oneOf.values[1]",
            "value.if.branches[0].then",
            "value.if.branches[0].then.aggregate.where",
            "value.if.branches[0].then.aggregate.where.isDefined.operand",
            "value.if.else",
        ]
    );
}

struct Plain;

impl ExportProfile for Plain {
    fn id(&self) -> &'static str {
        "plain"
    }

    fn export(&self, _: &[DefinitionPackage], _: &RuleSetPackage) -> ExportOutcome {
        ExportOutcome::default()
    }
}

#[test]
fn a_profile_states_no_expression_unless_it_declares_its_nodes() {
    assert!(Plain.expression_kinds().is_empty());
    let node = unsupported_expression_node(&openings(), "requirement", Plain.expression_kinds());
    assert_eq!(node.map(|node| node.kind), Some("and"));
}
