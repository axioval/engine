//! What a rule may carry that most target formats cannot state.
//!
//! A rule is plain when it is enabled, reports itself, is not gated on
//! another rule, reports with one fixed severity, and applies to one
//! selected population. [`pre_check`] tells a plain rule from the others in
//! that order, so every profile refuses the same rules for the same first
//! reason; a profile that states gates or grading uses the single helpers
//! instead.
//!
//! [`unsupported_expression_node`] does the same for an expression: it
//! names the first node a profile's
//! [`expression_kinds`](crate::ExportProfile::expression_kinds) leave out,
//! by the path the engine names it with.

use std::fmt;

use axioval_ir::contract::{
    Expression, RuleApplicability, RuleFolder, RuleInstance, Selector, Severity,
};

/// Why a rule is not plain. Exhaustive on purpose: a new reason must reach
/// every profile's refusals, never fall through a wildcard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreCheck {
    /// The rule is disabled.
    Disabled,
    /// An auxiliary rule, which reports nothing itself.
    Auxiliary,
    /// The rule, or a folder around it, is gated on another rule.
    Gated,
    /// Severity bands, overrides or categories shape what the rule reports.
    Graded,
    /// A severity other than the one the format states, by its package
    /// spelling.
    Severity(String),
    /// Named target groups instead of one population.
    Groups,
}

impl fmt::Display for PreCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreCheck::Disabled => f.write_str("the rule is disabled"),
            PreCheck::Auxiliary => f.write_str("an auxiliary rule reports nothing itself"),
            PreCheck::Gated => f.write_str("the rule runs only as another rule's outcome allows"),
            PreCheck::Graded => f.write_str(
                "severity bands, severity overrides or categories shape what the rule reports",
            ),
            PreCheck::Severity(severity) => write!(f, "the rule reports with severity {severity}"),
            PreCheck::Groups => f.write_str("the rule applies to named target groups"),
        }
    }
}

/// The selector of a plain rule reporting with `severity`, or the first
/// reason it is not plain: disabled, auxiliary, gated (itself, or `gated`
/// by a folder around it), graded, another severity, target groups.
///
/// # Errors
///
/// The first reason the rule is not plain.
pub fn pre_check<'r>(
    rule: &'r RuleInstance,
    gated: bool,
    severity: &Severity,
) -> Result<&'r Selector, PreCheck> {
    if !rule.enabled {
        return Err(PreCheck::Disabled);
    }
    if rule.auxiliary {
        return Err(PreCheck::Auxiliary);
    }
    if is_gated(rule, gated) {
        return Err(PreCheck::Gated);
    }
    if is_graded(rule) {
        return Err(PreCheck::Graded);
    }
    if rule.severity != *severity {
        return Err(PreCheck::Severity(severity_name(&rule.severity)));
    }
    selector(rule).ok_or(PreCheck::Groups)
}

/// Whether `rule` has a gate, or is `enclosing`-ly gated by a folder
/// around it.
#[must_use]
pub fn is_gated(rule: &RuleInstance, enclosing: bool) -> bool {
    enclosing || rule.gate.is_some()
}

/// Whether `folder` has a gate, or is `enclosing`-ly gated by a folder
/// around it; its rules and subfolders are gated when it is.
#[must_use]
pub fn is_folder_gated(folder: &RuleFolder, enclosing: bool) -> bool {
    enclosing || folder.gate.is_some()
}

/// Whether severity bands, severity overrides or categories shape what
/// `rule` reports.
#[must_use]
pub fn is_graded(rule: &RuleInstance) -> bool {
    !rule.severity_bands.is_empty()
        || !rule.severity_overrides.is_empty()
        || !rule.categories.is_empty()
}

/// A severity as a package spells it, such as `warning`.
#[must_use]
pub fn severity_name(severity: &Severity) -> String {
    serde_json::to_value(severity)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The selector `rule` applies to, `None` when it applies to named target
/// groups.
#[must_use]
pub fn selector(rule: &RuleInstance) -> Option<&Selector> {
    match &rule.applicability {
        RuleApplicability::Selector(selector) => Some(selector),
        RuleApplicability::Groups(_) => None,
    }
}

/// One node of an expression, by the path the engine names it with
/// (`requirement.and[2].compare.left`) and its `kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionNode {
    /// Where the node sits, from the parameter that holds the expression.
    pub path: String,
    /// The node's `kind`, as a package writes it (`aggregate`).
    pub kind: &'static str,
}

impl fmt::Display for ExpressionNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expression node `{}` (`{}`)", self.path, self.kind)
    }
}

/// Every node of `expression`, rooted at `path`, before its operands and
/// in written order, each with the path the engine's type checker and
/// evaluator name it by: the parent's path, its kind, then the field
/// (`requirement.compare.left`) or the operand index
/// (`requirement.and[2]`). The expressions of an aggregate's member filter
/// sit at `<path>.aggregate.where`.
#[must_use]
pub fn expression_nodes<'e>(
    expression: &'e Expression,
    path: &str,
) -> Vec<(String, &'e Expression)> {
    let mut nodes = Vec::new();
    let mut pending = vec![(path.to_owned(), expression)];
    while let Some((path, node)) = pending.pop() {
        let mut children = operands(node, &path);
        children.reverse();
        nodes.push((path, node));
        pending.extend(children);
    }
    nodes
}

/// The direct operands of `node` with their paths, in written order.
#[allow(clippy::too_many_lines)] // One arm per node kind, kept exhaustive.
fn operands<'e>(node: &'e Expression, path: &str) -> Vec<(String, &'e Expression)> {
    let kind = node.kind();
    let field = |name: &str| format!("{path}.{kind}.{name}");
    let item = |index: usize| format!("{path}.{kind}[{index}]");
    match node {
        Expression::Literal { .. }
        | Expression::Null { .. }
        | Expression::Property { .. }
        | Expression::Parameter { .. }
        | Expression::Derived { .. }
        | Expression::RuleOutcome { .. }
        | Expression::FindingCount { .. }
        | Expression::Deviation { .. } => Vec::new(),
        Expression::Lookup { keys, .. } => keys
            .iter()
            .map(|(key, value)| (format!("{path}.lookup.keys[{key}]"), value))
            .collect(),
        Expression::Not { operand, .. }
        | Expression::IsDefined { operand, .. }
        | Expression::IsUndefined { operand, .. }
        | Expression::Negate { operand, .. }
        | Expression::Abs { operand, .. }
        | Expression::Floor { operand, .. }
        | Expression::Ceil { operand, .. }
        | Expression::Sqrt { operand, .. }
        | Expression::Sin { operand, .. }
        | Expression::Cos { operand, .. }
        | Expression::Tan { operand, .. }
        | Expression::ConvertSlope { operand, .. }
        | Expression::Length { operand, .. }
        | Expression::Lower { operand, .. }
        | Expression::Upper { operand, .. }
        | Expression::Trim { operand, .. } => vec![(field("operand"), operand.as_ref())],
        Expression::And { operands, .. }
        | Expression::Or { operands, .. }
        | Expression::Coalesce { operands, .. }
        | Expression::Min { operands, .. }
        | Expression::Max { operands, .. }
        | Expression::Concat { operands, .. } => operands
            .iter()
            .enumerate()
            .map(|(index, operand)| (item(index), operand))
            .collect(),
        Expression::Implies {
            antecedent,
            consequent,
            ..
        } => vec![
            (field("antecedent"), antecedent.as_ref()),
            (field("consequent"), consequent.as_ref()),
        ],
        Expression::Xor { left, right, .. }
        | Expression::Compare { left, right, .. }
        | Expression::Add { left, right, .. }
        | Expression::Subtract { left, right, .. }
        | Expression::Multiply { left, right, .. }
        | Expression::Divide { left, right, .. } => vec![
            (field("left"), left.as_ref()),
            (field("right"), right.as_ref()),
        ],
        Expression::Between {
            operand, low, high, ..
        } => vec![
            (field("operand"), operand.as_ref()),
            (field("low"), low.as_ref()),
            (field("high"), high.as_ref()),
        ],
        Expression::OneOf {
            operand, values, ..
        }
        | Expression::NoneOf {
            operand, values, ..
        } => std::iter::once((field("operand"), operand.as_ref()))
            .chain(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (format!("{path}.{kind}.values[{index}]"), value)),
            )
            .collect(),
        Expression::If {
            branches,
            otherwise,
            ..
        } => branches
            .iter()
            .enumerate()
            .flat_map(|(index, branch)| {
                [
                    (format!("{path}.if.branches[{index}].when"), &branch.when),
                    (format!("{path}.if.branches[{index}].then"), &branch.then),
                ]
            })
            .chain(std::iter::once((
                format!("{path}.if.else"),
                otherwise.as_ref(),
            )))
            .collect(),
        Expression::Round { operand, step, .. } => vec![
            (field("operand"), operand.as_ref()),
            (field("step"), step.as_ref()),
        ],
        Expression::Atan2 { y, x, .. } => {
            vec![(field("y"), y.as_ref()), (field("x"), x.as_ref())]
        }
        Expression::Aggregate { filter, value, .. } => filter
            .iter()
            .flat_map(|filter| filter.expressions())
            .map(|nested| (field("where"), nested))
            .chain(value.iter().map(|value| (field("value"), value.as_ref())))
            .collect(),
    }
}

/// The first node of `expression`, rooted at `path`, whose kind is not
/// among `supported`, in the order of [`expression_nodes`]; `None` when
/// every node's is.
///
/// A profile passes its
/// [`expression_kinds`](crate::ExportProfile::expression_kinds) and
/// refuses the rule naming the node: a kind it does not support is a
/// computation its format cannot state, so the rule is
/// [refused](crate::LossKind::Refused), never degraded.
#[must_use]
pub fn unsupported_expression_node(
    expression: &Expression,
    path: &str,
    supported: &[&str],
) -> Option<ExpressionNode> {
    expression_nodes(expression, path)
        .into_iter()
        .find(|(_, node)| !supported.contains(&node.kind()))
        .map(|(path, node)| ExpressionNode {
            path,
            kind: node.kind(),
        })
}
