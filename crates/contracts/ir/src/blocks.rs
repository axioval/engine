//! An editor-neutral block tree for expressions, and the lossless mapping
//! between it and [`Expression`](crate::contract::Expression).
//!
//! A block editor shows an expression as nested blocks. This module states
//! the tree such an editor serialises its workspace to, so every editor
//! implementing the mapping reads and writes the same JSON, and converts it
//! from and to the expression contract. The tree is data: an editor's
//! generator emits the rule JSON (or `.mcs` source) from it, never code.
//!
//! The conversion is driven by the catalogue's field tables
//! ([`EXPRESSION_KINDS`](crate::catalogue::EXPRESSION_KINDS), [`SELECTOR_KINDS`](crate::catalogue::SELECTOR_KINDS), [`AGGREGATE_SOURCES`](crate::catalogue::AGGREGATE_SOURCES)): a
//! field whose [`FieldKind`](crate::catalogue::FieldKind) holds nested nodes becomes an input, every
//! other field a scalar field of the block. A new node kind needs only its
//! catalogue entry.
//!
//! | Block type | Node |
//! | --- | --- |
//! | `expression.<kind>` | an expression node of [`EXPRESSION_KINDS`](crate::catalogue::EXPRESSION_KINDS) |
//! | `measured.<name>` | a `property` read of a [`MEASURED_SET`](crate::MEASURED_SET) value, its parameters as fields |
//! | `selector.<kind>` | a selector of [`SELECTOR_KINDS`](crate::catalogue::SELECTOR_KINDS) |
//! | `source.<kind>` | what an aggregate ranges over, of [`AGGREGATE_SOURCES`](crate::catalogue::AGGREGATE_SOURCES) |
//! | `members.<name>` | a measured member list an aggregate ranges over, its parameters as fields |
//!
//! A measured read (or member list) becomes a `measured.` (`members.`)
//! block only when its text is in canonical form: the registry's name,
//! then each stated parameter as `;key=value` in the registry's parameter
//! order, keys lowercase, values trimmed. Any other spelling stays an
//! `expression.property` (`source.measured`) block holding the text as
//! written, so the mapping never rewrites what an author wrote.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::MEASURED_SET;
use crate::catalogue::{AGGREGATE_SOURCES, EXPRESSION_KINDS, Field, FieldKind, SELECTOR_KINDS};
use crate::contract::{AggregateSource, Expression, Selector};
use crate::measured::{
    MEASURED_MEMBERS, MEASURED_VALUES, MeasuredDescriptor, MeasuredError, parse, parse_members,
};

/// The prefix of an expression node's block type.
pub const EXPRESSION_PREFIX: &str = "expression.";
/// The prefix of a measured value's block type.
pub const MEASURED_PREFIX: &str = "measured.";
/// The prefix of a selector's block type.
pub const SELECTOR_PREFIX: &str = "selector.";
/// The prefix of an aggregate source's block type.
pub const SOURCE_PREFIX: &str = "source.";
/// The prefix of a measured member list's block type.
pub const MEMBERS_PREFIX: &str = "members.";
/// The field of a `measured.` block stating whose property it reads
/// (`of`: `subject`). Parameter keys are lowercase, so it never collides
/// with one.
pub const PROPERTY_OF_FIELD: &str = "propertyOf";

/// One block: a node of an expression, selector or aggregate source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    /// What the block is, such as `expression.compare`,
    /// `selector.property` or `measured.slope`.
    #[serde(rename = "type")]
    pub block_type: String,
    /// The author's label of an expression node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Scalar settings, by field name: operators, names, choices, flags,
    /// literal and parameter values, paths and measured parameters, each
    /// in its JSON form. A field the node leaves unstated is absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Value>,
    /// Nested blocks, by field name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, BlockInput>,
}

/// What an input of a [`Block`] holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BlockInput {
    /// One nested block.
    Block(Box<Block>),
    /// A list of nested blocks, in written order.
    Blocks(Vec<Block>),
    /// Nested blocks by name: a lookup's key per key column.
    Map(BTreeMap<String, Block>),
    /// The `when`/`then` branches of an `if`, in order: the first is the
    /// `if`, the others each an `else if`.
    Branches(Vec<BlockBranch>),
}

/// One `when` → `then` branch of an `expression.if` block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockBranch {
    pub when: Block,
    pub then: Block,
}

/// Why a block tree, or an expression, does not map.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{path}`: {problem}")]
pub struct BlockError {
    /// Where in the block tree: `$` is the root block, `.inputs.<name>`
    /// an input, `[n]` a list entry or branch, `.fields.<name>` a field.
    pub path: String,
    /// What is wrong there.
    pub problem: String,
}

fn error(path: &str, problem: impl Into<String>) -> BlockError {
    BlockError {
        path: path.to_owned(),
        problem: problem.into(),
    }
}

/// The block tree of `expression`.
///
/// # Errors
///
/// Only for an expression no package can state, such as one holding an
/// engine-internal selector.
pub fn to_blocks(expression: &Expression) -> Result<Block, BlockError> {
    let json = serde_json::to_value(expression)
        .map_err(|problem| error("$", format!("the expression is not writable: {problem}")))?;
    expression_block(&json, "$")
}

/// The expression a block tree states.
///
/// # Errors
///
/// A block of an unknown type, a field or input its node does not take or
/// of the wrong shape, a required one missing, an invalid measured
/// parameter, or a node the contract refuses, each named by its path.
pub fn from_blocks(block: &Block) -> Result<Expression, BlockError> {
    let json = expression_json(block, "$")?;
    serde_json::from_value(json).map_err(|problem| error("$", problem.to_string()))
}

/// Which table a block belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Expression,
    Selector,
    Source,
}

impl Family {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Expression => EXPRESSION_PREFIX,
            Self::Selector => SELECTOR_PREFIX,
            Self::Source => SOURCE_PREFIX,
        }
    }

    const fn what(self) -> &'static str {
        match self {
            Self::Expression => "an expression",
            Self::Selector => "a selector",
            Self::Source => "an aggregate source",
        }
    }

    fn fields(self, kind: &str) -> Option<&'static [Field]> {
        match self {
            Self::Expression => EXPRESSION_KINDS
                .iter()
                .find(|node| node.kind == kind)
                .map(|node| node.fields),
            Self::Selector => SELECTOR_KINDS
                .iter()
                .find(|node| node.kind == kind)
                .map(|node| node.fields),
            Self::Source => AGGREGATE_SOURCES
                .iter()
                .find(|node| node.kind == kind)
                .map(|node| node.fields),
        }
    }

    /// Whether `json` is a node the contract accepts, as serde reads it.
    fn check(self, json: &Value) -> Result<(), String> {
        let result = match self {
            Self::Expression => serde_json::from_value::<Expression>(json.clone()).map(drop),
            Self::Selector => serde_json::from_value::<Selector>(json.clone()).map(drop),
            Self::Source => serde_json::from_value::<AggregateSource>(json.clone()).map(drop),
        };
        result.map_err(|problem| problem.to_string())
    }
}

/// How a field of the catalogue maps onto a block.
enum Slot {
    /// A scalar field.
    Field,
    /// One nested node of a family.
    One(Family),
    /// A list of nested nodes of a family.
    List(Family),
    /// Expressions by name.
    Map,
    /// `if` branches.
    Branches,
}

const fn slot(kind: FieldKind) -> Slot {
    match kind {
        FieldKind::Expression { .. } | FieldKind::SelectorExpression { .. } => {
            Slot::One(Family::Expression)
        }
        FieldKind::Expressions { .. } => Slot::List(Family::Expression),
        FieldKind::ExpressionMap => Slot::Map,
        FieldKind::Branches => Slot::Branches,
        FieldKind::Selector => Slot::One(Family::Selector),
        FieldKind::Selectors => Slot::List(Family::Selector),
        FieldKind::AggregateSource => Slot::One(Family::Source),
        FieldKind::Literal
        | FieldKind::Name { .. }
        | FieldKind::Choice { .. }
        | FieldKind::ExpressionOperator
        | FieldKind::SelectorOperator
        | FieldKind::Flag
        | FieldKind::Path
        | FieldKind::Value => Slot::Field,
    }
}

// --- expression JSON → blocks ---------------------------------------------

fn expression_block(json: &Value, path: &str) -> Result<Block, BlockError> {
    if let Some(block) = measured_block(json) {
        return Ok(block);
    }
    node_block(Family::Expression, json, path)
}

fn node_block(family: Family, json: &Value, path: &str) -> Result<Block, BlockError> {
    if family == Family::Source
        && let Some(block) = members_block(json)
    {
        return Ok(block);
    }
    let object = json
        .as_object()
        .ok_or_else(|| error(path, format!("{} is no object", family.what())))?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| error(path, format!("{} has no `kind`", family.what())))?;
    let fields = family
        .fields(kind)
        .ok_or_else(|| error(path, format!("`{kind}` is no catalogued kind")))?;
    let mut block = Block {
        block_type: format!("{}{kind}", family.prefix()),
        label: None,
        fields: BTreeMap::new(),
        inputs: BTreeMap::new(),
    };
    for (name, value) in object {
        match name.as_str() {
            "kind" => continue,
            "label" if family == Family::Expression => {
                block.label = value.as_str().map(str::to_owned);
                continue;
            }
            _ => {}
        }
        // An unstated optional field is written `null` by some selectors.
        if value.is_null() {
            continue;
        }
        let field = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| error(path, format!("`{kind}` has no catalogued field `{name}`")))?;
        let at = format!("{path}.inputs.{name}");
        let input = match slot(field.kind) {
            Slot::Field => {
                block.fields.insert(name.clone(), value.clone());
                continue;
            }
            Slot::One(nested) => BlockInput::Block(Box::new(nested_block(nested, value, &at)?)),
            Slot::List(nested) => BlockInput::Blocks(
                value
                    .as_array()
                    .ok_or_else(|| error(&at, "is no list"))?
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| nested_block(nested, entry, &format!("{at}[{index}]")))
                    .collect::<Result<_, _>>()?,
            ),
            Slot::Map => BlockInput::Map(
                value
                    .as_object()
                    .ok_or_else(|| error(&at, "is no map"))?
                    .iter()
                    .map(|(key, entry)| {
                        expression_block(entry, &format!("{at}.{key}"))
                            .map(|block| (key.clone(), block))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Slot::Branches => BlockInput::Branches(
                value
                    .as_array()
                    .ok_or_else(|| error(&at, "is no list"))?
                    .iter()
                    .enumerate()
                    .map(|(index, branch)| {
                        let at = format!("{at}[{index}]");
                        let part = |part: &str| {
                            branch
                                .get(part)
                                .ok_or_else(|| error(&at, format!("has no `{part}`")))
                                .and_then(|json| expression_block(json, &format!("{at}.{part}")))
                        };
                        Ok(BlockBranch {
                            when: part("when")?,
                            then: part("then")?,
                        })
                    })
                    .collect::<Result<_, BlockError>>()?,
            ),
        };
        block.inputs.insert(name.clone(), input);
    }
    Ok(block)
}

fn nested_block(family: Family, json: &Value, path: &str) -> Result<Block, BlockError> {
    match family {
        Family::Expression => expression_block(json, path),
        Family::Selector | Family::Source => node_block(family, json, path),
    }
}

/// The parameters `text` states, as written, when `text` is the canonical
/// spelling of a name `parse` accepts.
fn canonical_parameters(
    text: &str,
    parse: fn(&str) -> Result<crate::measured::MeasuredCall, MeasuredError>,
) -> Option<(&'static MeasuredDescriptor, BTreeMap<String, Value>)> {
    let call = parse(text).ok()?;
    let mut parts = text.split(';');
    parts.next();
    let stated: BTreeMap<String, Value> = parts
        .map(|part| {
            let (key, value) = part.split_once('=')?;
            // A value is written trimmed, as a block's field states it.
            (value.trim() == value).then(|| (key.to_owned(), Value::String(value.to_owned())))
        })
        .collect::<Option<_>>()?;
    let rewritten = measured_text(call.descriptor, &stated)?;
    (rewritten == text).then_some((call.descriptor, stated))
}

/// `name;key=value…` with the stated parameters in the registry's order,
/// or `None` when a parameter is not the descriptor's or not text.
fn measured_text(
    descriptor: &MeasuredDescriptor,
    parameters: &BTreeMap<String, Value>,
) -> Option<String> {
    let mut text = descriptor.name.to_owned();
    let mut written = 0;
    for parameter in descriptor.parameters {
        if let Some(value) = parameters.get(parameter.key) {
            text.push(';');
            text.push_str(parameter.key);
            text.push('=');
            text.push_str(value.as_str()?);
            written += 1;
        }
    }
    (written == parameters.len()).then_some(text)
}

fn measured_block(json: &Value) -> Option<Block> {
    let object = json.as_object()?;
    if object.get("kind")?.as_str()? != "property"
        || object.get("propertySet")?.as_str()? != MEASURED_SET
    {
        return None;
    }
    let (descriptor, mut fields) = canonical_parameters(object.get("property")?.as_str()?, parse)?;
    for (name, value) in object {
        match name.as_str() {
            "kind" | "propertySet" | "property" | "label" => {}
            "of" => {
                fields.insert(PROPERTY_OF_FIELD.to_owned(), value.clone());
            }
            _ => return None,
        }
    }
    Some(Block {
        block_type: format!("{MEASURED_PREFIX}{}", descriptor.name),
        label: object
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned),
        fields,
        inputs: BTreeMap::new(),
    })
}

fn members_block(json: &Value) -> Option<Block> {
    let object = json.as_object()?;
    if object.get("kind")?.as_str()? != "measured" || object.len() != 2 {
        return None;
    }
    let (descriptor, fields) = canonical_parameters(object.get("name")?.as_str()?, parse_members)?;
    Some(Block {
        block_type: format!("{MEMBERS_PREFIX}{}", descriptor.name),
        label: None,
        fields,
        inputs: BTreeMap::new(),
    })
}

// --- blocks → expression JSON ---------------------------------------------

fn expression_json(block: &Block, path: &str) -> Result<Value, BlockError> {
    if let Some(name) = block.block_type.strip_prefix(MEASURED_PREFIX) {
        return measured_json(block, name, path);
    }
    node_json(Family::Expression, block, path)
}

fn nested_json(family: Family, block: &Block, path: &str) -> Result<Value, BlockError> {
    match family {
        Family::Expression => expression_json(block, path),
        Family::Selector | Family::Source => node_json(family, block, path),
    }
}

fn node_json(family: Family, block: &Block, path: &str) -> Result<Value, BlockError> {
    if family == Family::Source
        && let Some(name) = block.block_type.strip_prefix(MEMBERS_PREFIX)
    {
        return members_json(block, name, path);
    }
    let kind = block
        .block_type
        .strip_prefix(family.prefix())
        .ok_or_else(|| {
            error(
                path,
                format!(
                    "`{}` is not {} block; expected `{}…`",
                    block.block_type,
                    family.what(),
                    family.prefix()
                ),
            )
        })?;
    let fields = family.fields(kind).ok_or_else(|| {
        error(
            path,
            format!("`{}` names no catalogued kind", block.block_type),
        )
    })?;
    if block.label.is_some() && family != Family::Expression {
        return Err(error(path, format!("{} takes no label", family.what())));
    }
    let mut object = Map::new();
    object.insert("kind".into(), Value::String(kind.to_owned()));
    for (name, value) in &block.fields {
        let at = format!("{path}.fields.{name}");
        let field = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| error(&at, format!("`{}` has no field `{name}`", block.block_type)))?;
        if !matches!(slot(field.kind), Slot::Field) {
            return Err(error(&at, "is an input, not a field"));
        }
        object.insert(name.clone(), value.clone());
    }
    for (name, input) in &block.inputs {
        let at = format!("{path}.inputs.{name}");
        let field = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| error(&at, format!("`{}` has no input `{name}`", block.block_type)))?;
        let value = input_json(slot(field.kind), input, &at)?;
        object.insert(name.clone(), value);
    }
    if let Some(field) = fields
        .iter()
        .find(|field| field.required && !object.contains_key(field.name))
    {
        let side = if matches!(slot(field.kind), Slot::Field) {
            "field"
        } else {
            "input"
        };
        return Err(error(
            path,
            format!("`{}` needs the {side} `{}`", block.block_type, field.name),
        ));
    }
    if let Some(label) = &block.label {
        object.insert("label".into(), Value::String(label.clone()));
    }
    let json = Value::Object(object);
    family
        .check(&json)
        .map_err(|problem| error(path, problem))?;
    Ok(json)
}

/// The JSON of the input `input` of a field mapping to `slot`.
fn input_json(slot: Slot, input: &BlockInput, at: &str) -> Result<Value, BlockError> {
    Ok(match (slot, input) {
        (Slot::One(nested), BlockInput::Block(child)) => nested_json(nested, child, at)?,
        (Slot::List(nested), BlockInput::Blocks(children)) => Value::Array(
            children
                .iter()
                .enumerate()
                .map(|(index, child)| nested_json(nested, child, &format!("{at}[{index}]")))
                .collect::<Result<_, _>>()?,
        ),
        (Slot::Map, BlockInput::Map(children)) => Value::Object(
            children
                .iter()
                .map(|(key, child)| {
                    expression_json(child, &format!("{at}.{key}")).map(|json| (key.clone(), json))
                })
                .collect::<Result<_, _>>()?,
        ),
        (Slot::Branches, BlockInput::Branches(branches)) => Value::Array(
            branches
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    let at = format!("{at}[{index}]");
                    let mut object = Map::new();
                    object.insert(
                        "when".into(),
                        expression_json(&branch.when, &format!("{at}.when"))?,
                    );
                    object.insert(
                        "then".into(),
                        expression_json(&branch.then, &format!("{at}.then"))?,
                    );
                    Ok(Value::Object(object))
                })
                .collect::<Result<_, BlockError>>()?,
        ),
        (Slot::Field, _) => return Err(error(at, "is a field, not an input")),
        (Slot::One(_), _) => return Err(error(at, "takes one block (`block`)")),
        (Slot::List(_), _) => return Err(error(at, "takes a list of blocks (`blocks`)")),
        (Slot::Map, _) => return Err(error(at, "takes blocks by name (`map`)")),
        (Slot::Branches, _) => return Err(error(at, "takes branches (`branches`)")),
    })
}

/// The text of a `measured.`/`members.` block's parameters, checked by
/// the registry's parser.
fn parameters_text(
    block: &Block,
    descriptor: Option<&'static MeasuredDescriptor>,
    parse: fn(&str) -> Result<crate::measured::MeasuredCall, MeasuredError>,
    reserved: &[&str],
    path: &str,
) -> Result<String, BlockError> {
    let descriptor = descriptor.ok_or_else(|| {
        error(
            path,
            format!("`{}` names no registered name", block.block_type),
        )
    })?;
    if !block.inputs.is_empty() {
        return Err(error(
            path,
            format!("`{}` takes no inputs", block.block_type),
        ));
    }
    let mut parameters = BTreeMap::new();
    for (key, value) in &block.fields {
        if reserved.contains(&key.as_str()) {
            continue;
        }
        let at = format!("{path}.fields.{key}");
        if descriptor.parameter(key).is_none() {
            let accepted: Vec<&str> = descriptor
                .parameters
                .iter()
                .map(|parameter| parameter.key)
                .collect();
            return Err(error(
                &at,
                format!(
                    "`{}` takes no parameter `{key}`; it takes {}",
                    descriptor.name,
                    if accepted.is_empty() {
                        "none".to_owned()
                    } else {
                        accepted.join(", ")
                    }
                ),
            ));
        }
        let text = value
            .as_str()
            .ok_or_else(|| error(&at, "a parameter is written as text"))?;
        if text.is_empty() || text.trim() != text || text.contains(';') {
            return Err(error(
                &at,
                "a parameter is non-empty text without surrounding whitespace or `;`",
            ));
        }
        parameters.insert(key.clone(), value.clone());
    }
    let text = measured_text(descriptor, &parameters)
        .ok_or_else(|| error(path, "a parameter is not the registry's"))?;
    parse(&text).map_err(|problem| error(path, problem.to_string()))?;
    Ok(text)
}

fn measured_json(block: &Block, name: &str, path: &str) -> Result<Value, BlockError> {
    let text = parameters_text(
        block,
        MEASURED_VALUES
            .iter()
            .find(|descriptor| descriptor.name == name),
        parse,
        &[PROPERTY_OF_FIELD],
        path,
    )?;
    let mut object = Map::new();
    object.insert("kind".into(), Value::String("property".into()));
    object.insert("propertySet".into(), Value::String(MEASURED_SET.into()));
    object.insert("property".into(), Value::String(text));
    if let Some(of) = block.fields.get(PROPERTY_OF_FIELD) {
        object.insert("of".into(), of.clone());
    }
    if let Some(label) = &block.label {
        object.insert("label".into(), Value::String(label.clone()));
    }
    let json = Value::Object(object);
    Family::Expression
        .check(&json)
        .map_err(|problem| error(path, problem))?;
    Ok(json)
}

fn members_json(block: &Block, name: &str, path: &str) -> Result<Value, BlockError> {
    if block.label.is_some() {
        return Err(error(path, "an aggregate source takes no label"));
    }
    let text = parameters_text(
        block,
        MEASURED_MEMBERS
            .iter()
            .map(|members| &members.list)
            .find(|descriptor| descriptor.name == name),
        parse_members,
        &[],
        path,
    )?;
    let mut object = Map::new();
    object.insert("kind".into(), Value::String("measured".into()));
    object.insert("name".into(), Value::String(text));
    Ok(Value::Object(object))
}
