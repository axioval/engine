//! Rule drafts checked as an editor builds them: one rule, or one
//! expression placed into a rule, compiled against a ruleset context, with
//! every refusal reported as a positioned diagnostic. No model is read.
//!
//! A diagnostic names the refusal by a stable `code`, the engine's own
//! message, the expression path the compiler names (`requirement.and[2]`),
//! a JSON pointer into the draft (`/parameters/requirement/value/operands/2`),
//! the line and column of a draft that does not parse, and, where a name
//! is unknown, the nearest name that is known.
//!
//! [`ExpressionTraces`] makes a run record how every expression rule judged
//! every object it selected, passes included: the dry run of a draft.

use std::sync::{Arc, Mutex, PoisonError};

use axioval_ir::catalogue::{EXPRESSION_KINDS, FieldKind};
use axioval_ir::contract::{Expression, ParameterValue, RuleFolder, RuleInstance, RuleSetPackage};
use axioval_ir::{DefinitionPackage, Explanation, ObjectId};
use serde::Serialize;

use crate::{CapabilityRegistry, EngineError, ExecutionPlan, compile_rulesets};

/// One refusal of a draft.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// What kind of refusal: `syntax` and `shape` for a draft that does
    /// not parse, else the engine error's kind (`invalidExpression`,
    /// `unknownConcept`, `unknownParameter`, …).
    pub code: String,
    pub message: String,
    /// The rule it is about, when the engine names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// The parameter it is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    /// The engine's path into an expression, such as
    /// `requirement.and[2].compare.left`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Where in the draft: a JSON pointer into the rule (or, for an
    /// expression draft, into the expression).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// The line of a draft that does not parse, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Its column, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// A fix: the nearest known name for an unknown one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// A draft's diagnostics, and the plan it compiles to when there are none.
#[derive(Clone, Debug)]
pub struct DraftValidation {
    pub diagnostics: Vec<Diagnostic>,
    /// The context's plan with the draft in it, when it compiles.
    pub plan: Option<ExecutionPlan>,
    /// The id of the drafted rule, when the draft parsed.
    pub rule: Option<String>,
}

impl DraftValidation {
    fn refused(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            plan: None,
            rule: None,
        }
    }
}

/// Where a draft goes in its context.
enum Placement {
    /// A whole rule, replacing the context's rule of the same id.
    Rule(Box<RuleInstance>),
    /// An expression, as the `parameter` of the context's rule `rule`.
    Expression {
        rule: String,
        parameter: String,
        expression: Expression,
    },
}

/// Validates the rule `draft` (a rule instance as a ruleset writes it, as
/// JSON) within `rulesets`: it replaces the rule of the same id, or joins
/// the first ruleset's root, and the whole context is compiled as a check
/// would compile it.
#[must_use]
pub fn validate_rule(
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
    rulesets: &[RuleSetPackage],
    draft: &str,
) -> DraftValidation {
    match parse::<RuleInstance>(draft) {
        Ok(rule) => validate(
            registry,
            definitions,
            rulesets,
            Placement::Rule(Box::new(rule)),
        ),
        Err(diagnostic) => DraftValidation::refused(*diagnostic),
    }
}

/// Validates the expression `draft` (JSON) as the `parameter` (by default
/// `requirement`) of the context's rule `rule`; pointers are into the
/// expression.
#[must_use]
pub fn validate_expression(
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
    rulesets: &[RuleSetPackage],
    rule: &str,
    parameter: &str,
    draft: &str,
) -> DraftValidation {
    match parse::<Expression>(draft) {
        Ok(expression) => validate(
            registry,
            definitions,
            rulesets,
            Placement::Expression {
                rule: rule.to_owned(),
                parameter: parameter.to_owned(),
                expression,
            },
        ),
        Err(diagnostic) => DraftValidation::refused(*diagnostic),
    }
}

fn parse<T: serde::de::DeserializeOwned>(draft: &str) -> Result<T, Box<Diagnostic>> {
    let refused = |code: &str, error: &serde_json::Error| {
        Box::new(Diagnostic {
            code: code.into(),
            message: error.to_string(),
            line: Some(error.line()),
            column: Some(error.column()),
            ..Diagnostic::default()
        })
    };
    let value: serde_json::Value =
        serde_json::from_str(draft).map_err(|error| refused("syntax", &error))?;
    serde_json::from_str(draft).map_err(|error| {
        let mut diagnostic = refused("shape", &error);
        // The shape error's position is in the text; a pointer is not known.
        if value.is_null() {
            diagnostic.message = "the draft is `null`".into();
        }
        diagnostic
    })
}

fn rules_mut(node: &mut RuleFolder) -> Vec<&mut RuleInstance> {
    let mut found: Vec<&mut RuleInstance> = node.rules.iter_mut().collect();
    for child in &mut node.folders {
        found.extend(rules_mut(child));
    }
    found
}

fn validate(
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
    rulesets: &[RuleSetPackage],
    placement: Placement,
) -> DraftValidation {
    let mut context = rulesets.to_vec();
    let (id, draft_json, expression_root) = match placement {
        Placement::Rule(rule) => {
            let id = rule.id.clone();
            let json = serde_json::to_value(&rule).unwrap_or_default();
            let mut placed = false;
            for ruleset in &mut context {
                for existing in rules_mut(&mut ruleset.root) {
                    if existing.id == id {
                        *existing = (*rule).clone();
                        placed = true;
                    }
                }
            }
            if !placed {
                match context.first_mut() {
                    Some(first) => first.root.rules.push(*rule),
                    None => {
                        return DraftValidation::refused(Diagnostic {
                            code: "noRuleSet".into(),
                            message: "a draft needs a ruleset to compile in".into(),
                            ..Diagnostic::default()
                        });
                    }
                }
            }
            (id, json, None)
        }
        Placement::Expression {
            rule,
            parameter,
            expression,
        } => {
            let json = serde_json::to_value(&expression).unwrap_or_default();
            let mut placed = false;
            for ruleset in &mut context {
                for existing in rules_mut(&mut ruleset.root) {
                    if existing.id == rule {
                        existing.parameters.insert(
                            parameter.clone(),
                            ParameterValue::Expression {
                                value: Box::new(expression.clone()),
                            },
                        );
                        placed = true;
                    }
                }
            }
            if !placed {
                let known: Vec<String> = context
                    .iter_mut()
                    .flat_map(|ruleset| rules_mut(&mut ruleset.root))
                    .map(|existing| existing.id.clone())
                    .collect();
                return DraftValidation::refused(Diagnostic {
                    code: "unknownRule".into(),
                    message: format!("the context has no rule `{rule}` to place the expression in"),
                    rule: Some(rule.clone()),
                    suggestion: closest(&rule, known.iter().map(String::as_str)),
                    ..Diagnostic::default()
                });
            }
            (rule, json, Some(parameter))
        }
    };
    match compile_rulesets(registry, definitions, &context) {
        Ok(plan) => DraftValidation {
            diagnostics: Vec::new(),
            plan: Some(plan),
            rule: Some(id),
        },
        Err(error) => DraftValidation {
            diagnostics: vec![diagnose(
                &error,
                &Drafted {
                    id: &id,
                    json: &draft_json,
                    expression: expression_root.as_deref(),
                },
                registry,
                definitions,
            )],
            plan: None,
            rule: Some(id),
        },
    }
}

/// The draft a diagnostic positions itself in.
struct Drafted<'a> {
    id: &'a str,
    json: &'a serde_json::Value,
    /// For an expression draft, the parameter it was placed as.
    expression: Option<&'a str>,
}

/// The engine error's kind, from its variant: `InvalidExpression` is
/// `invalidExpression`.
fn code(error: &EngineError) -> String {
    let debug = format!("{error:?}");
    let name: String = debug
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_lowercase().to_string() + chars.as_str()
    })
}

fn diagnose(
    error: &EngineError,
    draft: &Drafted<'_>,
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
) -> Diagnostic {
    let mut diagnostic = Diagnostic {
        code: code(error),
        message: error.to_string(),
        ..Diagnostic::default()
    };
    let parameter_pointer = |parameter: &str| -> Option<String> {
        draft
            .expression
            .is_none()
            .then(|| format!("/parameters/{}", escape(parameter)))
    };
    match error {
        EngineError::InvalidRuleId(rule) | EngineError::DuplicateRule(rule) => {
            diagnostic.rule = Some(rule.clone());
            diagnostic.pointer = draft.expression.is_none().then(|| "/id".into());
        }
        EngineError::UnknownDefinition(definition) => {
            diagnostic.pointer = draft.expression.is_none().then(|| "/definitionId".into());
            diagnostic.suggestion = closest(
                definition,
                definitions
                    .iter()
                    .flat_map(|package| package.definitions.keys().map(String::as_str)),
            );
        }
        EngineError::UnknownCapability(capability) => {
            diagnostic.suggestion = closest(capability, registry.ids());
        }
        EngineError::UnknownParameter {
            capability,
            parameter,
        } => {
            diagnostic.parameter = Some(parameter.clone());
            diagnostic.pointer = parameter_pointer(parameter);
            if let Some(found) = registry.get(capability) {
                let names: Vec<String> = found
                    .parameters()
                    .into_iter()
                    .map(|descriptor| descriptor.name)
                    .collect();
                diagnostic.suggestion = closest(parameter, names.iter().map(String::as_str));
            }
        }
        EngineError::MissingParameter { parameter, .. } => {
            diagnostic.parameter = Some(parameter.clone());
            diagnostic.pointer = draft.expression.is_none().then(|| "/parameters".into());
            diagnostic.suggestion = Some(format!("add the parameter `{parameter}`"));
        }
        EngineError::InvalidParameterType { parameter, .. }
        | EngineError::InvalidTableRow { parameter, .. }
        | EngineError::InvalidTableFile { parameter, .. } => {
            diagnostic.parameter = Some(parameter.clone());
            diagnostic.pointer = parameter_pointer(parameter);
        }
        EngineError::InvalidExpression {
            rule,
            parameter,
            path,
            detail,
        } => {
            diagnostic.suggestion =
                unknown_concept(detail).and_then(|concept| closest(concept, concepts(definitions)));
            diagnostic.rule = Some(rule.clone());
            diagnostic.parameter = Some(parameter.clone());
            diagnostic.path = Some(path.clone());
            if rule == draft.id {
                diagnostic.pointer =
                    expression_pointer(path).map(|within| match draft.expression {
                        Some(_) => within,
                        None => format!("/parameters/{}/value{within}", escape(parameter)),
                    });
            }
        }
        EngineError::UnknownConcept { .. } | EngineError::InvalidMeasured { .. } => {
            name_diagnostic(error, draft, definitions, &mut diagnostic);
        }
        _ => {}
    }
    diagnostic
}

/// Positions an unknown concept or measured name in the draft and suggests
/// the nearest known one.
fn name_diagnostic(
    error: &EngineError,
    draft: &Drafted<'_>,
    definitions: &[DefinitionPackage],
    diagnostic: &mut Diagnostic,
) {
    match error {
        EngineError::UnknownConcept { rule, concept, .. } => {
            diagnostic.rule = Some(rule.clone());
            if rule == draft.id {
                diagnostic.pointer = find(draft.json, concept);
            }
            diagnostic.suggestion = closest(concept, concepts(definitions));
        }
        EngineError::InvalidMeasured { rule, property, .. } => {
            diagnostic.rule = Some(rule.clone());
            if rule == draft.id {
                diagnostic.pointer = find(draft.json, property);
            }
            let name = property.split(';').next().unwrap_or_default();
            diagnostic.suggestion = closest(
                name,
                axioval_ir::measured::MEASURED_VALUES
                    .iter()
                    .map(|descriptor| descriptor.name)
                    .chain(
                        axioval_ir::measured::MEASURED_MEMBERS
                            .iter()
                            .map(|members| members.list.name),
                    ),
            );
        }
        _ => {}
    }
}

/// Every concept id the packages declare.
fn concepts(definitions: &[DefinitionPackage]) -> impl Iterator<Item = &str> {
    definitions.iter().flat_map(|package| {
        package
            .object_types
            .keys()
            .chain(package.properties.keys())
            .chain(package.property_sets.keys())
            .map(String::as_str)
    })
}

/// The concept an expression's binding refusal names as unknown.
fn unknown_concept(detail: &str) -> Option<&str> {
    let (_, rest) = detail.split_once(" concept `")?;
    rest.split_once('`').map(|(concept, _)| concept)
}

/// A JSON pointer token.
fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// The pointer of the first string in `value` equal to `text`, in
/// document order.
fn find(value: &serde_json::Value, text: &str) -> Option<String> {
    match value {
        serde_json::Value::String(found) if found == text => Some(String::new()),
        serde_json::Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| find(item, text).map(|rest| format!("/{index}{rest}"))),
        serde_json::Value::Object(fields) => fields
            .iter()
            .find_map(|(key, item)| find(item, text).map(|rest| format!("/{}{rest}", escape(key)))),
        _ => None,
    }
}

/// The list field a `kind[index]` step indexes: the node's one list of
/// expressions.
fn list_field(kind: &str) -> Option<&'static str> {
    EXPRESSION_KINDS
        .iter()
        .find(|node| node.kind == kind)?
        .fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Expressions { .. }))
        .map(|field| field.name)
}

fn is_kind(token: &str) -> bool {
    EXPRESSION_KINDS.iter().any(|node| node.kind == token)
}

/// The JSON pointer, relative to the expression, of an engine path such as
/// `requirement.and[2].compare.left`: its first step names the parameter,
/// then node kinds alternate with their fields, a kind indexed
/// (`and[2]`) naming an item of its list of operands.
#[must_use]
pub fn expression_pointer(path: &str) -> Option<String> {
    let mut steps = path.split('.');
    steps.next()?;
    let mut pointer = String::new();
    let mut expect_kind = true;
    for step in steps {
        let (name, index) = match step.split_once('[') {
            Some((name, rest)) => (name, Some(rest.strip_suffix(']')?)),
            None => (step, None),
        };
        if expect_kind && is_kind(name) {
            if let Some(index) = index {
                pointer.push('/');
                pointer.push_str(list_field(name)?);
                pointer.push('/');
                pointer.push_str(&escape(index));
            } else {
                expect_kind = false;
            }
            continue;
        }
        pointer.push('/');
        pointer.push_str(&escape(name));
        if let Some(index) = index {
            pointer.push('/');
            pointer.push_str(&escape(index));
        }
        // A branch's `when` and `then` follow its index.
        expect_kind = name != "branches";
    }
    Some(pointer)
}

/// The known name nearest `unknown` by edit distance, ignoring case, when
/// it is close enough to be a likely typo.
fn closest<'a>(unknown: &str, known: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let unknown_lower = unknown.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for candidate in known {
        let distance = edit_distance(&unknown_lower, &candidate.to_lowercase());
        if best
            .is_none_or(|(least, name)| distance < least || (distance == least && candidate < name))
        {
            best = Some((distance, candidate));
        }
    }
    let (distance, name) = best?;
    (distance > 0 && distance <= (unknown.chars().count() / 3).max(2))
        .then(|| format!("did you mean `{name}`?"))
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, a) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, b) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(a != *b);
            current.push(
                substitution
                    .min(previous[column + 1] + 1)
                    .min(current[column] + 1),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

/// How an expression rule judged one object in a traced run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectTrace {
    pub rule: String,
    pub object: ObjectId,
    /// `passed`, `failed` or `notEvaluated`.
    pub verdict: &'static str,
    /// Every subexpression evaluated, in order, with its value.
    pub trace: Explanation,
}

/// A host service that makes the expression capability record every
/// object's trace, passes included. Register it for a dry run; runs
/// without it record nothing and are unchanged.
#[derive(Clone, Debug, Default)]
pub struct ExpressionTraces(Arc<Mutex<Vec<ObjectTrace>>>);

impl ExpressionTraces {
    /// An empty recorder; clones share what it records.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records how `rule` judged `object`.
    pub fn record(&self, trace: ObjectTrace) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(trace);
    }

    /// Everything recorded, sorted by rule and object.
    #[must_use]
    pub fn take(&self) -> Vec<ObjectTrace> {
        let mut traces =
            std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
        traces.sort_by(|left, right| (&left.rule, &left.object).cmp(&(&right.rule, &right.object)));
        traces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_paths_become_pointers() {
        assert_eq!(expression_pointer("requirement").as_deref(), Some(""));
        assert_eq!(
            expression_pointer("requirement.and[2].compare.left").as_deref(),
            Some("/operands/2/left")
        );
        assert_eq!(
            expression_pointer("requirement.implies.consequent.oneOf.values[1]").as_deref(),
            Some("/consequent/values/1")
        );
        assert_eq!(
            expression_pointer("requirement.if.branches[0].when.not.operand").as_deref(),
            Some("/branches/0/when/operand")
        );
        assert_eq!(
            expression_pointer("requirement.lookup.keys[class]").as_deref(),
            Some("/keys/class")
        );
        assert_eq!(
            expression_pointer("requirement.aggregate.value.min[0]").as_deref(),
            Some("/value/operands/0")
        );
    }

    #[test]
    fn near_names_are_suggested() {
        assert_eq!(
            closest("rizer", ["riser", "going", "slope"]).as_deref(),
            Some("did you mean `riser`?")
        );
        assert_eq!(closest("riser", ["riser"]), None);
        assert_eq!(closest("completely_else", ["riser"]), None);
    }

    #[test]
    fn engine_errors_have_codes() {
        assert_eq!(
            code(&EngineError::UnknownDefinition("x".into())),
            "unknownDefinition"
        );
        assert_eq!(
            code(&EngineError::InvalidExpression {
                rule: "r".into(),
                parameter: "p".into(),
                path: "p".into(),
                detail: "d".into(),
            }),
            "invalidExpression"
        );
    }

    #[test]
    fn a_draft_that_does_not_parse_is_positioned() {
        let registry = CapabilityRegistry::new();
        let validation = validate_rule(&registry, &[], &[], "{\n  \"id\": ");
        assert_eq!(validation.diagnostics[0].code, "syntax");
        assert_eq!(validation.diagnostics[0].line, Some(2));
        let validation = validate_rule(&registry, &[], &[], "{\"id\": 3}");
        assert_eq!(validation.diagnostics[0].code, "shape");
        assert!(validation.diagnostics[0].column.is_some());
    }
}
