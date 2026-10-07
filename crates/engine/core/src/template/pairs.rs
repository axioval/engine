//! Pairs of a measured member list put in classes ([`Pairs`]): the
//! candidate pairs a rule's selected objects form with their counterparts,
//! each measured by a provider (its separation, penetration, containment,
//! overlap and shared volume as intervals), judged by the first class that
//! holds.
//!
//! A form deciding [`Decision::Pairs`](super::Decision) reads the list
//! once per rule, of the project, its `@` references bound (`@selection`
//! the objects the rule selects). An item either is open (the provider
//! could not measure it, or could not tell which tolerances judge it) or
//! is a pair: its classes are tried in order, each a conjunction of
//! three-valued [`PairTest`]s over the item's numbers, truths and the
//! rule's parameters. The first class that surely holds decides the pair:
//! a finding (or, [`PairClass::opens`], an open outcome) where the class is
//! reported, a pass where it is switched off; one that surely does not
//! hold hands the pair to the next class; one that may hold decides only
//! where both readings agree (two findings keep the later class's, a pass
//! and a finding leave the pair open with the class's `undecided`, an open
//! reading stands). Nothing here measures: the provider measured, the
//! template judges the measurements against the tolerances.

use axioval_ir::contract::{
    AggregateFunction, AggregateSource, Expression, ExpressionComparison, ScalarValue,
};
use serde::Serialize;

use super::{Applies, Bound, ItemText, Operand, When};

/// The pairs of a measured member list of the project, each put in a
/// class.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairs {
    /// The measured member list, written as a measured value is, its `@`
    /// references bound once per rule.
    pub list: &'static str,
    /// Selector parameters whose own not-evaluated outcomes the rule
    /// reports beside its selection's, each once: a counterpart whose
    /// selection cannot be decided.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub selections: &'static [&'static str],
    /// The objects field naming the object an item's outcome is on.
    pub subject: &'static str,
    /// The objects field naming the objects a pair's finding relates. A
    /// finding cites what its item was measured from.
    pub related: &'static str,
    /// An item that is no pair to judge: open on its subject where its
    /// message field states words.
    pub open: PairOpen,
    /// The classes, tried in order.
    pub classes: Vec<PairClass>,
    /// A pair that may or may not be left out: where its message field
    /// states words, any outcome but a pass is open with them instead, so
    /// it never reports a finding and never hides one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unless: Option<PairOpen>,
    /// The severity of a finding.
    pub severity: PairSeverity,
    /// Words added to every finding of a pair (the cell judging it):
    /// rendered over the item, after any grade's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<&'static str>,
    /// Named message parts, each rendered where its conditions hold over
    /// the item and the rule; of several of one name the first that holds.
    /// A text field the list declares and an item leaves out renders as no
    /// words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<ItemText>,
    /// Findings grouped into one per group, where the rule declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<PairGroups>,
}

/// An item left open: the text field `message` (open where it states any
/// words) and the text field `reason` naming the not-evaluated reason as
/// reports spell it (`incomplete_evidence`).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairOpen {
    pub message: &'static str,
    pub reason: &'static str,
}

/// One class of pairs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairClass {
    /// The class's name: what a grade and a group name it by.
    pub name: &'static str,
    /// Every test the class needs, three-valued (Kleene's `and`).
    pub holds: Vec<PairTest>,
    /// A test that excuses the pair from the class where it holds: read
    /// last, and only where the class may hold and is reported, so what it
    /// measures is consulted only where it can change the outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excused: Option<PairTest>,
    /// The truth field saying whether the class is reported: a pair of a
    /// class it states false for passes, never reclassified. Reported
    /// where the item states no such field, and always without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported: Option<&'static str>,
    /// Whether the class leaves its pairs open (`fail` its message, as
    /// incomplete evidence) rather than finds them.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opens: bool,
    /// The finding's message.
    pub fail: &'static str,
    /// The message of a pair the class may or may not hold for, where its
    /// two readings disagree.
    pub undecided: &'static str,
}

/// A three-valued test of a pair.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PairTest {
    /// The number field `value` stands in `order` to `bound`: holds where
    /// every value it may take does, fails where none does, may hold
    /// otherwise. A `null` value or bound fails (there is nothing to
    /// compare: no penetration, no clearance declared); an undecided value
    /// may hold. With `zero`, a bound of exactly zero holds whatever the
    /// value: a tolerance of zero does not limit.
    Compare {
        value: &'static str,
        order: PairOrder,
        bound: Bound,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        zero: bool,
    },
    /// The truth field `field` is `value`; an undecided one may be.
    Truth { field: &'static str, value: bool },
    /// The field `field` is stated (not `null`) as `stated` says.
    Stated { field: &'static str, stated: bool },
}

/// How a [`PairTest::Compare`] orders its value against its bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PairOrder {
    Below,
    AtMost,
    Above,
    AtLeast,
}

/// The severity of a pair's finding: a grade where its class is graded,
/// else the item's own stated severity, else its class's from a table,
/// else the rule's.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairSeverity {
    /// A text field naming a severity (`error`, `warning`, `info`) where it
    /// states one: the severity of the cell judging the pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stated: Option<&'static str>,
    /// A table parameter naming a severity per class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_class: Option<ClassSeverities>,
    /// Grades of one class by a measure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grades: Option<Box<PairGrades>>,
}

/// A table parameter whose row naming a class in `class` gives its
/// severity in `severity`.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassSeverities {
    pub table: &'static str,
    pub class: &'static str,
    pub severity: &'static str,
}

/// A class graded by a measure: the severity of the highest grade the
/// measure exceeds (the table parameter's rows, each a bound `above` and a
/// `severity`), the class's own where it exceeds none. A measure that is
/// an interval takes the most severe grade any value in it may reach, one
/// not measured the most severe of all, so a pair is never graded milder
/// than it may be. The finding's message says how it was graded.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairGrades {
    /// The class graded.
    pub class: &'static str,
    /// The string parameter choosing the measure.
    pub by: &'static str,
    /// The measures it may choose.
    pub measures: Vec<GradeMeasure>,
    /// The table parameter of grades.
    pub table: &'static str,
    pub above: &'static str,
    pub severity: &'static str,
    /// The words of a grade every value of the measure reaches:
    /// `{severity}`, `{measure}` (its words) and `{value}` (the measure).
    pub graded: &'static str,
    /// The words of the most severe grade an interval may reach.
    pub reaches: &'static str,
    /// The words of a measure not measured.
    pub unmeasured: &'static str,
}

/// A measure a [`PairGrades`] may choose: the option naming it, the
/// item's number field, and how it is worded (`smallest extent`, shown to
/// `digits` decimals in `unit`).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeMeasure {
    pub option: &'static str,
    pub field: &'static str,
    pub words: &'static str,
    pub digits: usize,
    pub unit: &'static str,
}

/// Findings grouped by a key into one finding per group: on the object
/// most of its pairs involve (the first in identity order of those
/// involved most), relating every other, at the most severe of their
/// severities, citing what each cited. A group of one is its pair's own
/// finding. A pair whose key is undecided is reported alone, saying why.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairGroups {
    /// Where the rule groups at all.
    pub applies: Applies,
    /// The text field keying the group; undecided where it cannot be read.
    pub key: &'static str,
    /// Where these hold, the class is part of the key too.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub classed: Vec<When>,
    /// The group's finding: `{count}` its pairs, `{class}` their class,
    /// `{parts}` each pair's `part` joined `; `, and the first pair's
    /// fields and texts.
    pub message: &'static str,
    /// One pair in a group's finding: `{message}` its finding's words.
    pub part: &'static str,
    /// Added to a pair's finding reported alone: `{why}` the key's reason.
    pub alone: &'static str,
}

impl Pairs {
    /// The decision as a block editor shows it: no pair of the list falls
    /// in a reported class, each class the conjunction of its tests. It
    /// states what the template judges, not how it words, grades or groups
    /// it, nor what an undecided reading comes to; a rule is never forked
    /// from it.
    #[must_use]
    pub fn expression(&self) -> Expression {
        let field = |name: &str| Expression::Property {
            property_set: Some(axioval_ir::MEMBER_SET.to_owned()),
            property: name.to_owned(),
            of: None,
            label: None,
        };
        let test = |test: &PairTest| match *test {
            PairTest::Compare {
                value,
                order,
                bound,
                ..
            } => Expression::Compare {
                operator: match order {
                    PairOrder::Below => ExpressionComparison::LessThan,
                    PairOrder::AtMost => ExpressionComparison::LessThanOrEquals,
                    PairOrder::Above => ExpressionComparison::GreaterThan,
                    PairOrder::AtLeast => ExpressionComparison::GreaterThanOrEquals,
                },
                left: Box::new(field(value)),
                right: Box::new(match bound {
                    Bound::Operand(Operand::Value(name)) => field(name),
                    Bound::Operand(Operand::Parameter(name)) => Expression::Parameter {
                        name: name.to_owned(),
                        label: None,
                    },
                    Bound::Literal(value) => Expression::Literal {
                        value: ScalarValue::Number { value },
                        label: None,
                    },
                }),
                case_sensitive: true,
                label: None,
            },
            PairTest::Truth { field: name, value } => {
                if value {
                    field(name)
                } else {
                    Expression::Not {
                        operand: Box::new(field(name)),
                        label: None,
                    }
                }
            }
            PairTest::Stated {
                field: name,
                stated,
            } => {
                let operand = Box::new(field(name));
                if stated {
                    Expression::IsDefined {
                        operand,
                        label: None,
                    }
                } else {
                    Expression::IsUndefined {
                        operand,
                        label: None,
                    }
                }
            }
        };
        let classes = self
            .classes
            .iter()
            .filter(|class| !class.opens)
            .map(|class| {
                let mut operands: Vec<Expression> = class.holds.iter().map(test).collect();
                if let Some(reported) = class.reported {
                    operands.push(field(reported));
                }
                Expression::And {
                    operands,
                    label: Some(class.name.to_owned()),
                }
            })
            .collect();
        Expression::Aggregate {
            function: AggregateFunction::None,
            over: AggregateSource::Measured {
                name: self.list.to_owned(),
            },
            filter: None,
            value: Some(Box::new(Expression::Or {
                operands: classes,
                label: None,
            })),
            label: Some("no pair is reported".into()),
        }
    }
}
