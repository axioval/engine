//! The block tree: every fixture in `tests/blocks` pairs an expression
//! with its block tree, and both round-trip losslessly.
//!
//! Regenerate the block trees from the expressions with
//! `AXIOVAL_BLESS=1 cargo test -p axioval-ir --test blocks`.
#![allow(missing_docs)]
use axioval_ir::blocks::{Block, BlockError, from_blocks, to_blocks};
use axioval_ir::catalogue::{AGGREGATE_SOURCES, EXPRESSION_KINDS, SELECTOR_KINDS};
use axioval_ir::contract::Expression;
use axioval_ir::measured::{MEASURED_MEMBERS, MEASURED_VALUES};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

/// A fixture as blessed: written from the types, so keys keep their
/// documented order.
#[derive(serde::Serialize)]
struct Pair {
    expression: Expression,
    blocks: Block,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/blocks")
}

/// Every fixture by name, as `(expression, blocks)`, blessed first when
/// `AXIOVAL_BLESS` is set.
fn fixtures() -> Vec<(String, Value, Value)> {
    static FIXTURES: OnceLock<Vec<(String, Value, Value)>> = OnceLock::new();
    FIXTURES.get_or_init(read_fixtures).clone()
}

fn read_fixtures() -> Vec<(String, Value, Value)> {
    let bless = std::env::var_os("AXIOVAL_BLESS").is_some();
    let mut fixtures: Vec<_> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
            let mut pair: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            if bless {
                let expression: Expression = serde_json::from_value(pair["expression"].clone())
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                let blocks = to_blocks(&expression).unwrap();
                let text = serde_json::to_string_pretty(&Pair { expression, blocks }).unwrap();
                std::fs::write(&path, text + "\n").unwrap();
                pair = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            }
            (name, pair["expression"].clone(), pair["blocks"].clone())
        })
        .collect();
    fixtures.sort_by(|a, b| a.0.cmp(&b.0));
    fixtures
}

#[test]
fn every_fixture_round_trips_both_ways() {
    for (name, expression_json, blocks_json) in fixtures() {
        let expression: Expression = serde_json::from_value(expression_json.clone())
            .unwrap_or_else(|error| panic!("{name}: the expression does not parse: {error}"));
        expression
            .validate()
            .unwrap_or_else(|error| panic!("{name}: the expression is refused: {error}"));
        assert_eq!(
            serde_json::to_value(&expression).unwrap(),
            expression_json,
            "{name}: the expression is written canonically"
        );

        // Expression → blocks → expression.
        let blocks = to_blocks(&expression).unwrap();
        assert_eq!(
            serde_json::to_value(&blocks).unwrap(),
            blocks_json,
            "{name}: the block tree"
        );
        assert_eq!(
            serde_json::to_value(from_blocks(&blocks).unwrap()).unwrap(),
            expression_json,
            "{name}: expression → blocks → expression"
        );

        // Blocks → expression → blocks.
        let read: Block = serde_json::from_value(blocks_json.clone())
            .unwrap_or_else(|error| panic!("{name}: the block tree does not parse: {error}"));
        assert_eq!(serde_json::to_value(&read).unwrap(), blocks_json, "{name}");
        let back = from_blocks(&read).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(back, expression, "{name}");
        assert_eq!(
            serde_json::to_value(to_blocks(&back).unwrap()).unwrap(),
            blocks_json,
            "{name}: blocks → expression → blocks"
        );
    }
}

#[test]
fn every_golden_expression_has_a_block_fixture() {
    let blocks: BTreeSet<String> = fixtures().into_iter().map(|(name, ..)| name).collect();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/expression");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        assert!(blocks.contains(&name), "{name} has no block fixture");
        let golden: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let (_, expression, _) = fixtures()
            .into_iter()
            .find(|(fixture, ..)| *fixture == name)
            .unwrap();
        assert_eq!(expression, golden, "{name} pairs the golden expression");
    }
}

fn block_types(block: &Value, types: &mut BTreeSet<String>) {
    match block {
        Value::Object(object) => {
            if let Some(Value::String(block_type)) = object.get("type") {
                types.insert(block_type.clone());
            }
            for (key, value) in object {
                // A field's value (a literal's `{"type": …}`) is no block.
                if key != "fields" {
                    block_types(value, types);
                }
            }
        }
        Value::Array(entries) => entries.iter().for_each(|entry| block_types(entry, types)),
        _ => {}
    }
}

fn covered_types() -> BTreeSet<String> {
    let mut types = BTreeSet::new();
    for (_, _, blocks) in fixtures() {
        block_types(&blocks, &mut types);
    }
    types
}

#[test]
fn every_catalogued_kind_has_a_fixture() {
    let types = covered_types();
    let missing: Vec<String> = EXPRESSION_KINDS
        .iter()
        .map(|kind| format!("expression.{}", kind.kind))
        .chain(
            SELECTOR_KINDS
                .iter()
                .map(|kind| format!("selector.{}", kind.kind)),
        )
        .chain(
            AGGREGATE_SOURCES
                .iter()
                .map(|kind| format!("source.{}", kind.kind)),
        )
        .filter(|block_type| !types.contains(block_type))
        .collect();
    assert!(missing.is_empty(), "no fixture holds {missing:?}");
    assert!(
        types
            .iter()
            .any(|block_type| block_type.starts_with("measured."))
    );
    assert!(
        types
            .iter()
            .any(|block_type| block_type.starts_with("members."))
    );
}

#[test]
fn measured_values_become_blocks_with_their_parameters_as_fields() {
    let (_, _, blocks) = fixtures()
        .into_iter()
        .find(|(name, ..)| name == "measured-parameters")
        .unwrap();
    let compare = &blocks["inputs"]["value"]["block"];
    assert_eq!(
        compare["inputs"]["left"]["block"],
        json!({
            "type": "measured.distance",
            "label": "distance to the nearest door",
            "fields": {"to": "IfcDoor", "projection": "horizontal"},
        })
    );
    assert_eq!(
        compare["inputs"]["right"]["block"],
        json!({
            "type": "measured.bottom_above_level",
            "fields": {
                "path": "IfcRelContainedInSpatialStructure:backward",
                "propertyOf": "subject",
            },
        })
    );
    let (_, _, blocks) = fixtures()
        .into_iter()
        .find(|(name, ..)| name == "members")
        .unwrap();
    assert_eq!(
        blocks["inputs"]["over"]["block"],
        json!({"type": "members.runs", "fields": {"landing": "IfcSlab"}})
    );
}

#[test]
fn a_measured_name_not_written_canonically_is_kept_as_written() {
    let (_, _, blocks) = fixtures()
        .into_iter()
        .find(|(name, ..)| name == "measured-noncanonical")
        .unwrap();
    for side in ["left", "right"] {
        assert_eq!(
            blocks["inputs"][side]["block"]["type"], "expression.property",
            "{side}"
        );
    }
    // A value written with surrounding whitespace is no field value.
    let spaced: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": "axioval:measured",
        "property": "distance;to= IfcDoor",
    }))
    .unwrap();
    let blocks = to_blocks(&spaced).unwrap();
    assert_eq!(blocks.block_type, "expression.property");
    assert_eq!(from_blocks(&blocks).unwrap(), spaced);
    let (_, _, blocks) = fixtures()
        .into_iter()
        .find(|(name, ..)| name == "members-noncanonical")
        .unwrap();
    assert_eq!(
        blocks["inputs"]["over"]["block"],
        json!({"type": "source.measured", "fields": {"name": "runs; landing = IfcSlab"}})
    );
}

#[test]
fn measured_parameter_keys_never_collide_with_the_scope_field() {
    for descriptor in MEASURED_VALUES
        .iter()
        .chain(MEASURED_MEMBERS.iter().map(|members| &members.list))
    {
        assert_eq!(
            descriptor.name,
            descriptor.name.to_ascii_lowercase(),
            "{}",
            descriptor.name
        );
        for parameter in descriptor.parameters {
            assert_eq!(
                parameter.key,
                parameter.key.to_ascii_lowercase(),
                "{}",
                descriptor.name
            );
            assert_ne!(parameter.key, axioval_ir::blocks::PROPERTY_OF_FIELD);
        }
    }
}

/// A measured value naming a rule parameter (`@door_selector`) or the
/// anchor keeps the reference as its field's text, both ways.
#[test]
fn a_measured_reference_is_a_field_as_written() {
    let name = "shelf_length;depth=@shelf_depth_metres;horizontal=0.3;vertical=0.35;\
                bottom=0.1;top=2;clearance=0.9;access=@access_path;doors=@door_selector;\
                openings=@anchor";
    let expression: Expression = serde_json::from_value(json!({
        "kind": "property", "propertySet": "axioval:measured", "property": name,
    }))
    .unwrap();
    let blocks = to_blocks(&expression).unwrap();
    let json = serde_json::to_value(&blocks).unwrap();
    assert_eq!(json["type"], "measured.shelf_length");
    assert_eq!(json["fields"]["doors"], "@door_selector");
    assert_eq!(json["fields"]["openings"], "@anchor");
    assert_eq!(json["fields"]["depth"], "@shelf_depth_metres");
    assert_eq!(from_blocks(&blocks).unwrap(), expression);
    // A reference where the parameter takes none is refused.
    let refused = refused(json!({"type": "measured.slope",
        "fields": {"face": "facing", "direction": "@axis", "tolerance": "10"}}));
    assert!(refused.to_string().contains("@axis"), "{refused}");
}

#[test]
fn a_block_without_defaults_and_one_stating_them_read_alike() {
    let explicit: Block = serde_json::from_value(json!({
        "type": "expression.compare",
        "fields": {"operator": "equals", "caseSensitive": true},
        "inputs": {
            "left": {"block": {"type": "expression.parameter", "fields": {"name": "a"}}},
            "right": {"block": {"type": "expression.null"}},
        },
    }))
    .unwrap();
    let expression = from_blocks(&explicit).unwrap();
    assert_eq!(
        serde_json::to_value(to_blocks(&expression).unwrap()).unwrap()["fields"],
        json!({"operator": "equals"}),
        "the canonical tree omits a field at its default"
    );
}

fn refused(blocks: Value) -> BlockError {
    let block: Block = serde_json::from_value(blocks).unwrap();
    from_blocks(&block).unwrap_err()
}

/// Malformed block trees, each with the path and the problem it is refused
/// with.
#[allow(clippy::too_many_lines)]
fn malformed() -> [(Value, &'static str, &'static str); 17] {
    let parameter = json!({"type": "expression.parameter", "fields": {"name": "a"}});
    [
        (
            json!({"type": "expression.frobnicate"}),
            "$",
            "names no catalogued kind",
        ),
        (
            json!({"type": "selector.all"}),
            "$",
            "is not an expression block",
        ),
        (
            json!({"type": "expression.and", "inputs": {"operands": {"blocks": [
                parameter,
                {"type": "expression.not"},
            ]}}}),
            "$.inputs.operands[1]",
            "needs the input `operand`",
        ),
        (
            json!({"type": "expression.not", "inputs": {"operand": {"blocks": [parameter]}}}),
            "$.inputs.operand",
            "takes one block",
        ),
        (
            json!({"type": "expression.compare", "fields": {"operator": "equals", "left": 1},
                "inputs": {"right": {"block": parameter}}}),
            "$.fields.left",
            "is an input, not a field",
        ),
        (
            json!({"type": "expression.compare", "fields": {"operator": "about"},
                "inputs": {"left": {"block": parameter}, "right": {"block": parameter}}}),
            "$",
            "unknown variant `about`",
        ),
        (
            json!({"type": "expression.parameter", "fields": {"name": "a", "colour": "red"}}),
            "$.fields.colour",
            "has no field `colour`",
        ),
        (
            json!({"type": "expression.aggregate", "fields": {"function": "count"},
            "inputs": {
                "over": {"block": {"type": "source.path", "fields": {"path": ["x"]}}},
                "where": {"block": {"type": "expression.null"}},
            }}),
            "$.inputs.where",
            "is not a selector block",
        ),
        (
            json!({"type": "expression.aggregate", "fields": {"function": "count"},
                "inputs": {"over": {"block": {"type": "selector.allOf", "inputs": {
                    "operands": {"blocks": [{"type": "selector.entityType"}]}}}}}}),
            "$.inputs.over",
            "is not an aggregate source block",
        ),
        (
            json!({"type": "expression.if",
            "inputs": {
                "branches": {"branches": [{"when": parameter, "then": {"type": "expression.null", "fields": {"value": 1}}}]},
                "else": {"block": parameter},
            }}),
            "$.inputs.branches[0].then.fields.value",
            "has no field `value`",
        ),
        (
            json!({"type": "measured.distance", "fields": {"projection": "horizontal"}}),
            "$",
            "needs `to`",
        ),
        (
            json!({"type": "measured.distance", "fields": {"to": "IfcDoor", "colour": "red"}}),
            "$.fields.colour",
            "takes no parameter `colour`",
        ),
        (
            json!({"type": "measured.distance", "fields": {"to": " IfcDoor"}}),
            "$.fields.to",
            "without surrounding whitespace",
        ),
        (
            json!({"type": "measured.distance", "fields": {"to": 3}}),
            "$.fields.to",
            "written as text",
        ),
        (
            json!({"type": "measured.nonsense"}),
            "$",
            "names no registered name",
        ),
        (
            json!({"type": "expression.aggregate", "fields": {"function": "count"},
                "inputs": {"over": {"block": {"type": "members.nonsense"}}}}),
            "$.inputs.over",
            "names no registered name",
        ),
        (
            json!({"type": "expression.lookup", "fields": {"table": "t", "column": "c"},
                "inputs": {"keys": {"map": {"k": {"type": "selector.all"}}}}}),
            "$.inputs.keys.k",
            "is not an expression block",
        ),
    ]
}

#[test]
fn malformed_block_trees_are_refused_with_a_named_path() {
    for (blocks, path, problem) in malformed() {
        let error = refused(blocks.clone());
        assert_eq!(error.path, path, "{blocks}: {error}");
        assert!(error.problem.contains(problem), "{blocks}: {error}");
    }
}

#[test]
fn block_trees_with_unknown_members_do_not_parse() {
    for blocks in [
        json!({"type": "expression.null", "colour": "red"}),
        json!({"type": "expression.not", "inputs": {"operand": {"single": {"type": "expression.null"}}}}),
        json!({"type": "expression.if", "inputs": {"branches": {"branches": [{"when": {"type": "expression.null"}}]}}}),
    ] {
        assert!(
            serde_json::from_value::<Block>(blocks.clone()).is_err(),
            "{blocks}"
        );
    }
}
