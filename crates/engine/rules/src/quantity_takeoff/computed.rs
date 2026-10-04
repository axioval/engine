//! Computed takeoff columns: an expression over the other columns of a
//! member, written as text (`area × 42.5 EUR/m²`) or as the expression
//! tree, type and unit checked when the rule is bound and evaluated by the
//! engine's evaluator over intervals, never run as code.
//!
//! A name in the expression reads the column of that name (a profile's
//! dimension as `<column>_<dimension>`); the expression may also read the
//! member's properties, derived values and aggregates, and branch on them.

use std::cell::RefCell;
use std::collections::BTreeMap;

use axioval_engine::expression::{
    ExpressionContext, Leaf, Member, Reason, RuleRead, Type, TypeEnvironment, Unit, Value, check,
    evaluate, parse_text,
};
use axioval_engine::{CompiledRule, NotEvaluatedReason, RuleContext};
use axioval_ir::Object;
use axioval_ir::contract::{AggregateSource, Expression, ParameterValue, Selector};

use crate::expression_leaves::ObjectLeaves;

/// A computed column as declared: its text, or the expression itself.
#[derive(Clone, Copy)]
pub(super) enum Written<'a> {
    Text(&'a str),
    Tree(&'a Expression),
}

impl Written<'_> {
    /// How errors quote it.
    pub(super) fn shown(self) -> String {
        match self {
            Self::Text(text) => format!("`{text}`"),
            Self::Tree(_) => "(expression)".into(),
        }
    }

    /// The expression tree.
    fn tree(self) -> Result<Expression, String> {
        let tree = match self {
            Self::Text(text) => parse_text(text)?,
            Self::Tree(tree) => tree.clone(),
        };
        tree.validate().map_err(|error| error.to_string())?;
        Ok(tree)
    }
}

/// The written column a parameter declares: text, or an expression.
pub(super) fn written(value: Option<&ParameterValue>) -> Option<Written<'_>> {
    match value? {
        ParameterValue::String { value } => Some(Written::Text(value)),
        ParameterValue::Expression { value } => Some(Written::Tree(value)),
        _ => None,
    }
}

/// The unit of the column an expression reads by name, or why it names
/// none.
pub(super) type Resolve<'r> = dyn Fn(&str) -> Result<Unit, String> + 'r;

/// Types a computed column's names through `resolve`: each a number of its
/// column's unit. Properties and derived values are typed when read.
struct Columns<'r> {
    resolve: &'r Resolve<'r>,
    read: RefCell<Vec<String>>,
}

impl TypeEnvironment for Columns<'_> {
    fn property(&self, _: Option<&str>, _: &str) -> Result<Type, String> {
        Ok(Type::Any)
    }

    fn parameter(&self, name: &str) -> Result<Type, String> {
        let unit = (self.resolve)(name)?;
        let mut read = self.read.borrow_mut();
        if !read.iter().any(|seen| seen == name) {
            read.push(name.to_owned());
        }
        Ok(Type::Number(unit))
    }

    fn derived(&self, _: &str) -> Result<Type, String> {
        Ok(Type::Any)
    }
}

/// Binds `written`: its tree, the names of the columns it reads in reading
/// order, and the unit of its value.
pub(super) fn bind(
    written: Written<'_>,
    resolve: &Resolve<'_>,
) -> Result<(Expression, Vec<String>, Unit), String> {
    let tree = written.tree()?;
    let columns = Columns {
        resolve,
        read: RefCell::new(Vec::new()),
    };
    let found = check(&tree, "column", &columns).map_err(|error| error.kind.to_string())?;
    let unit = match found {
        Type::Number(unit) => unit,
        Type::Integer => Unit::NONE,
        other => return Err(format!("it computes {other}, not a number")),
    };
    Ok((tree, columns.read.into_inner(), unit))
}

/// A column's value for one member: a number in its unit, `null` when it
/// states none, or why it cannot be read.
pub(super) type ColumnValue = Result<Value, (NotEvaluatedReason, String)>;

/// Evaluates a bound column for `object`: its names read `columns`, the
/// rest the member and the rule.
pub(super) fn evaluate_column(
    tree: &Expression,
    columns: BTreeMap<String, ColumnValue>,
    context: &RuleContext<'_>,
    object: &Object,
    rule: &CompiledRule,
) -> ColumnValue {
    let mut leaves = ColumnLeaves {
        columns,
        leaves: ObjectLeaves::new(context, object, Some(&rule.parameters)),
        failure: None,
    };
    let evaluation = evaluate(tree, "column", &mut leaves);
    match evaluation.outcome {
        Ok(value) => Ok(value),
        Err(why) => Err(match (&why.reason, leaves.failure) {
            (Reason::Unreadable(_), Some(failure)) => failure,
            (Reason::ZeroDivisor, _) => (
                NotEvaluatedReason::IncompleteEvidence,
                "cannot be computed: it divides by an interval that holds zero".into(),
            ),
            (Reason::Overflow, _) => (NotEvaluatedReason::InvalidEvidence, "is not finite".into()),
            (reason, _) => (
                leaves
                    .leaves
                    .first_reason()
                    .unwrap_or_else(|| crate::expression_requirement::reason_of(&why)),
                format!("cannot be computed: {reason}"),
            ),
        }),
    }
}

/// The leaves of a computed column: the member's columns by name, then the
/// member itself.
struct ColumnLeaves<'a> {
    columns: BTreeMap<String, ColumnValue>,
    leaves: ObjectLeaves<'a>,
    /// Why the first column read could not be read.
    failure: Option<(NotEvaluatedReason, String)>,
}

impl ExpressionContext for ColumnLeaves<'_> {
    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        self.leaves.property(set, name)
    }

    fn subject_property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        self.leaves.subject_property(set, name)
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        match self.columns.get(name) {
            Some(Ok(value)) => Leaf::stated(value.clone()),
            Some(Err((reason, why))) => {
                self.failure.get_or_insert_with(|| {
                    (
                        reason.clone(),
                        format!("cannot be computed: `{name}` {why}"),
                    )
                });
                Leaf::unreadable(why.clone())
            }
            None => self.leaves.parameter(name),
        }
    }

    fn derived(&mut self, name: &str) -> Leaf {
        self.leaves.derived(name)
    }

    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        self.leaves.lookup(table, keys, column)
    }

    fn rule(&mut self, rule: &str, read: RuleRead) -> Leaf {
        self.leaves.rule(rule, read)
    }

    fn members(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        self.leaves.members(over, filter, value, path)
    }
}
