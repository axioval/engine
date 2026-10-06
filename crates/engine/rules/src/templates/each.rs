//! Members read and judged one by one ([`Each`]): ordered, against their
//! neighbours and a reference prevailing among them, and over their own
//! nested members.

use std::collections::BTreeMap;

use axioval_engine::expression::{Interval, Unit, Value};
use axioval_engine::template::{
    Applies, Decision, Each, Judgement, Nested, Operand, Prevailing, Reference, UndecidedMembers,
};
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext};
use axioval_ir::contract::{Expression, ScalarValue};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, PropertyValue, QuantityDimension, ReportColumn,
    ReportTable, ReportValue,
};

use super::{
    Constant, Outcome, Plan, Read, Scope, effective_of, holds, read_values, render, within,
};
use crate::counts::{Population, relation_text, tally};
use crate::expression_leaves::ObjectLeaves;
use crate::level_spacing::prevailing;
use crate::plan_area::Verdict;
use crate::selection::{object_by_id, select_objects};
use crate::support::{Parameters, Traversal, finding};

/// An interval of a value read or derived.
type Span = (f64, f64);

/// A member or nested member, with what was read of it.
struct Subject<'a> {
    object: &'a Object,
    values: BTreeMap<&'static str, Span>,
    stated: super::Named<Option<PropertyValue>>,
    evidence: Vec<Evidence>,
    /// The next member up, which a member's rise relates.
    next: Option<ObjectId>,
}

impl Subject<'_> {
    fn new(object: &Object) -> Subject<'_> {
        Subject {
            object,
            values: BTreeMap::new(),
            stated: super::Named::default(),
            evidence: Vec::new(),
            next: None,
        }
    }
}

/// `minuend − subtrahend` in plain binary arithmetic, as the
/// capabilities computed differences of intervals.
fn difference(minuend: Span, subtrahend: Span) -> Span {
    (minuend.0 - subtrahend.1, minuend.1 - subtrahend.0)
}

/// Whether `applies` holds for the rule: every `when` parameter stated (a
/// boolean, or its default, true), one of `any`, and its condition.
fn applies(plan: &Plan<'_>, applies: &Applies) -> bool {
    let declared = |name: &&str| {
        !matches!(
            plan.constants.get(*name),
            None | Some(Constant::Scalar(ScalarValue::Boolean { value: false }))
        )
    };
    applies.when.iter().all(declared)
        && (applies.any.is_empty() || applies.any.iter().any(declared))
        && holds(plan, &Read::default(), applies.condition)
}

/// A boolean parameter (or its default) that holds.
fn flag(plan: &Plan<'_>, name: Option<&str>) -> bool {
    name.is_some_and(|name| {
        matches!(
            plan.constants.get(name),
            Some(Constant::Scalar(ScalarValue::Boolean { value: true }))
        )
    })
}

/// The values a decision reads of its subject.
fn reads(decision: &Decision) -> Vec<&'static str> {
    match decision {
        Decision::Within { value, .. } => vec![*value],
        Decision::Near {
            value, reference, ..
        } => match reference {
            Reference::Value(name) | Reference::Prevailing(Prevailing { value: name, .. }) => {
                vec![*value, *name]
            }
        },
        _ => Vec::new(),
    }
}

/// A length interval as the evaluator reads one.
fn number(span: Span) -> Value {
    Value::Number {
        value: Interval {
            lower: span.0,
            upper: span.1,
        },
        unit: Unit::of(Some(QuantityDimension::Length)),
    }
}

/// What the messages about `subject` read: its values, its member's
/// (`member:<name>`) and the member's identity (`{member}`).
fn read_of(subject: &Subject<'_>, member: Option<&Subject<'_>>) -> Read {
    let mut read = Read::default();
    for (name, span) in &subject.values {
        read.values.insert(name, number(*span));
    }
    read.stated = subject.stated.clone();
    if let Some(member) = member {
        for (name, span) in &member.values {
            read.outer.insert(name, number(*span));
        }
        read.named.insert("member", member.object.id.to_string());
    } else {
        read.named.insert("member", subject.object.id.to_string());
    }
    read
}

/// The value `name` of `subject`, or of its member (`member:<name>`).
fn value_of(subject: &Subject<'_>, member: Option<&Subject<'_>>, name: &str) -> Option<Span> {
    match (name.strip_prefix("member:"), member) {
        (Some(name), Some(member)) => member.values.get(name).copied(),
        _ => subject.values.get(name).copied(),
    }
}

/// The tolerance a `Near` decision reads.
fn tolerance_of(plan: &Plan<'_>, operand: Operand, subject: &Subject<'_>) -> Option<f64> {
    match operand {
        Operand::Parameter(name) => plan.constants.get(name).and_then(Constant::number),
        Operand::Value(name) => subject.values.get(name).map(|span| span.0),
    }
}

/// Judges `subjects` (members, or the nested members of `member`) by
/// `judgement`, each its own outcome.
#[allow(clippy::too_many_lines)]
fn judge(
    plan: &Plan<'_>,
    rule: &CompiledRule,
    judgement: &Judgement,
    subjects: &[Subject<'_>],
    member: Option<&Subject<'_>>,
    evaluation: &mut CapabilityEvaluation,
) {
    let read_names = reads(&judgement.decision);
    let subjects: Vec<&Subject<'_>> = subjects
        .iter()
        .filter(|subject| {
            read_names
                .iter()
                .all(|name| value_of(subject, member, name).is_some())
        })
        .collect();
    if subjects.len() < judgement.least {
        return;
    }
    let decision = effective_of(plan, &judgement.decision);
    // The reference prevailing among the subjects, where one is asked for.
    let mut prevailing_reference = None;
    if let Decision::Near {
        reference: Reference::Prevailing(Prevailing { value, missing }),
        tolerance,
        ..
    } = &decision
    {
        #[allow(clippy::float_cmp)]
        let exact: Vec<f64> = subjects
            .iter()
            .filter_map(|subject| value_of(subject, member, value))
            .filter(|span| span.0 == span.1)
            .map(|span| span.0)
            .collect();
        let tolerance = subjects
            .first()
            .and_then(|subject| tolerance_of(plan, *tolerance, subject))
            .unwrap_or(0.0);
        if let Some(index) = prevailing(&exact, tolerance) {
            prevailing_reference = Some(exact[index]);
        } else {
            if let Some(missing) = missing {
                for subject in &subjects {
                    let read = read_of(subject, member);
                    evaluation.push_object_not_evaluated(
                        subject.object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        render(plan, &read, missing),
                    );
                }
            }
            return;
        }
    }
    for subject in subjects {
        let mut read = read_of(subject, member);
        let related: Vec<ObjectId> = match member {
            Some(member) => vec![member.object.id.clone()],
            None => subject.next.iter().cloned().collect(),
        };
        let verdict = match &decision {
            Decision::Within { .. } => match within(plan, &read, &decision) {
                Some(judged) => judged.verdict,
                None => continue,
            },
            Decision::Near {
                value,
                reference,
                tolerance,
            } => {
                let (Some((lower, upper)), Some(tolerance)) = (
                    value_of(subject, member, value),
                    tolerance_of(plan, *tolerance, subject),
                ) else {
                    continue;
                };
                let reference = match reference {
                    Reference::Value(name) => value_of(subject, member, name),
                    Reference::Prevailing(_) => {
                        prevailing_reference.map(|reference| (reference, reference))
                    }
                };
                let Some((least, most)) = reference else {
                    continue;
                };
                read.values.insert("reference", number((least, most)));
                if least - upper > tolerance || lower - most > tolerance {
                    Verdict::Fail(String::new())
                } else if most - lower > tolerance || upper - least > tolerance {
                    Verdict::Undecided(String::new())
                } else {
                    Verdict::Pass
                }
            }
            _ => continue,
        };
        match verdict {
            Verdict::Pass => {}
            Verdict::Fail(bound) => {
                read.bound = Some(bound);
                evaluation.push_finding(finding(
                    rule,
                    &subject.object.id,
                    render(plan, &read, judgement.fail),
                    subject.evidence.clone(),
                    related,
                ));
            }
            Verdict::Undecided(bound) => {
                read.bound = Some(bound);
                evaluation.push_object_not_evaluated(
                    subject.object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &read, judgement.undecided),
                );
            }
        }
    }
}

/// A nested population bound for the rule: its values' expressions, the
/// population its selector picks and the path reaching it.
struct Bound<'t> {
    nested: &'t Nested,
    /// Its values' expressions, bound once with the rule's plan.
    expressions: &'t [Expression],
    population: Population,
    path: Option<Traversal>,
}

impl<'t> Bound<'t> {
    fn of(
        plan: &'t Plan<'t>,
        nested: &'t Nested,
        expressions: &'t [Expression],
        context: &RuleContext<'_>,
        rule: &CompiledRule,
    ) -> Self {
        let selector = nested
            .selector
            .and_then(|name| match plan.constants.get(name) {
                Some(Constant::Other(axioval_ir::contract::ParameterValue::Selector { value })) => {
                    Some(value.as_ref().clone())
                }
                _ => None,
            });
        Self {
            nested,
            expressions,
            population: Population::of(
                context,
                selector
                    .as_ref()
                    .unwrap_or(&axioval_ir::contract::Selector::All),
            ),
            path: Parameters(rule)
                .strings(nested.path)
                .ok()
                .flatten()
                .and_then(|steps| Traversal::path(steps).ok()),
        }
    }

    /// The nested members of `member`, read; `Err` where the member is
    /// left open, `Ok(None)` where too few judge nothing.
    #[allow(clippy::type_complexity)]
    fn read<'a>(
        &self,
        plan: &Plan<'_>,
        context: &RuleContext<'a>,
        rule: &CompiledRule,
        member: &Subject<'_>,
        evaluation: &mut CapabilityEvaluation,
    ) -> Result<Option<Vec<Subject<'a>>>, (NotEvaluatedReason, String)> {
        let nested = self.nested;
        let missing = nested.services.as_ref().filter(|services| {
            !services
                .needs
                .iter()
                .all(|service| service.registered(context.services))
        });
        let refused = |services: &axioval_engine::template::Services| {
            (
                NotEvaluatedReason::MissingService,
                services.message.to_owned(),
            )
        };
        if nested.services_first
            && let Some(services) = missing
        {
            return Err(refused(services));
        }
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let tallied = tally(context, Some(path), member.object, &self.population)?;
        let mut read = Read::default();
        read.named
            .insert("undecided", tallied.undecided.to_string());
        read.named.insert("relation", relation_text(Some(path)));
        if tallied.undecided > 0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, nested.undecided),
            ));
        }
        if tallied.decided.len() < nested.least {
            return match nested.fewer {
                Some(fewer) => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &read, fewer),
                )),
                None => Ok(None),
            };
        }
        if let Some(services) = missing {
            return Err(refused(services));
        }
        let mut subjects = Vec::new();
        for id in &tallied.decided {
            let Some(object) = object_by_id(context, id) else {
                continue;
            };
            let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
            let mut values = Read::default();
            if let Some(outcome) = read_values(
                plan,
                self.nested.values.iter().zip(self.expressions),
                &|_| false,
                context,
                object,
                &mut leaves,
                &mut values,
            ) {
                let (reason, message) = match outcome {
                    Outcome::Open(reason, message) => (reason, message),
                    Outcome::Finding { message, .. } => {
                        (NotEvaluatedReason::IncompleteEvidence, message)
                    }
                    Outcome::Passed => continue,
                };
                if nested.errors_open_member {
                    return Err((reason, message));
                }
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                continue;
            }
            let mut subject = Subject::new(object);
            for (name, value) in values.values.iter() {
                if let Value::Number { value, .. } = value {
                    subject.values.insert(name, (value.lower, value.upper));
                }
            }
            for derived in &nested.differences {
                if let (Some(minuend), Some(subtrahend)) = (
                    subject.values.get(derived.minuend).copied(),
                    subject.values.get(derived.subtrahend).copied(),
                ) {
                    subject
                        .values
                        .insert(derived.name, difference(minuend, subtrahend));
                }
            }
            subject.evidence = values.evidence;
            subject.evidence.extend(tallied.evidence.iter().cloned());
            subject.evidence.extend(member.evidence.iter().cloned());
            subjects.push(subject);
        }
        Ok(Some(subjects))
    }
}

/// Runs a form judging an anchor's members one by one.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    plan: &Plan<'_>,
    each: &Each,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let scope = match Scope::of(plan, context, rule) {
        Ok(Some(scope)) => scope,
        Ok(None) => {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("{}: the form judges no members", plan.template.name),
            );
        }
        Err((reason, message)) => {
            return CapabilityEvaluation::not_evaluated(
                reason,
                format!("{}: {message}", plan.template.name),
            );
        }
    };
    let nested: Vec<Bound<'_>> = each
        .nested
        .iter()
        .zip(plan.each.iter().skip(1))
        .filter(|(nested, _)| applies(plan, &nested.applies))
        .map(|(nested, expressions)| Bound::of(plan, nested, expressions, context, rule))
        .collect();
    let checks: Vec<&Judgement> = each
        .checks
        .iter()
        .filter(|judgement| applies(plan, &judgement.applies))
        .collect();
    // A rise is read only where something judged reads it.
    let rise_read = checks
        .iter()
        .copied()
        .chain(nested.iter().flat_map(|bound| &bound.nested.checks))
        .any(|judgement| reads(&judgement.decision).contains(&each.rise.name))
        || nested
            .iter()
            .any(|bound| bound.nested.members_with == Some(each.rise.name));
    let mut table = each.table.as_ref().and_then(|table| {
        ReportTable::new(
            rule.id.clone(),
            table.name,
            table
                .columns
                .iter()
                .map(|column| ReportColumn::quantity(column.id, column.dimension))
                .collect(),
        )
        .ok()
    });
    let mut nested_tables: Vec<Option<ReportTable>> =
        each.nested
            .iter()
            .map(|nested| {
                nested.table.as_ref().and_then(|table| {
                    ReportTable::new(
                        rule.id.clone(),
                        table.name,
                        std::iter::once(ReportColumn::text(table.member))
                            .chain(
                                table.columns.iter().map(|column| {
                                    ReportColumn::quantity(column.id, column.dimension)
                                }),
                            )
                            .collect(),
                    )
                    .ok()
                })
            })
            .collect();
    let refuse = match &scope.members.undecided {
        UndecidedMembers::Refuse { message } => {
            scope.population.first.clone().map(|(reason, why)| {
                let read = Read {
                    why: Some(why),
                    ..Read::default()
                };
                (reason, render(plan, &read, message))
            })
        }
        _ => None,
    };
    let (anchors, mut evaluation) = select_objects(context, &rule.selector);
    for anchor in anchors {
        if let Some((reason, message)) = &refuse {
            evaluation.push_object_not_evaluated(
                anchor.id.clone(),
                reason.clone(),
                message.clone(),
            );
            continue;
        }
        let tallied = match scope.tally(context, anchor) {
            Ok(tallied) => tallied,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                continue;
            }
        };
        // Read each member's values; a member whose order cannot be read
        // leaves the anchor open, since the members cannot be ordered.
        let mut members = Vec::new();
        let mut unordered = None;
        for id in &tallied.decided {
            let Some(object) = object_by_id(context, id) else {
                continue;
            };
            let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
            let mut read = Read::default();
            read.named.insert("member", object.id.to_string());
            let opened = read_values(
                plan,
                each.values.iter().zip(&plan.each[0]),
                &|name| name == each.order,
                context,
                object,
                &mut leaves,
                &mut read,
            );
            if let Some(outcome) = opened {
                unordered = Some(match outcome {
                    Outcome::Open(reason, message) => (reason, message),
                    _ => (
                        NotEvaluatedReason::IncompleteEvidence,
                        render(plan, &read, each.unordered),
                    ),
                });
                break;
            }
            let mut subject = Subject::new(object);
            let Some(PropertyValue::Quantity {
                value: order,
                dimension: QuantityDimension::Length,
            }) = read.stated.get(each.order).cloned().flatten()
            else {
                unordered = Some((
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &read, each.unordered),
                ));
                break;
            };
            subject.values.insert(each.order, (order, order));
            for (name, value) in read.values.iter() {
                if let Value::Number { value, .. } = value {
                    subject
                        .values
                        .entry(name)
                        .or_insert((value.lower, value.upper));
                }
            }
            subject.stated = read.stated;
            subject.evidence = read.evidence;
            subject.evidence.extend(tallied.evidence.iter().cloned());
            members.push(subject);
        }
        if let Some((reason, message)) = unordered {
            evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
            continue;
        }
        members.sort_by(|left, right| {
            left.values[each.order]
                .0
                .total_cmp(&right.values[each.order].0)
                .then_with(|| left.object.id.cmp(&right.object.id))
        });
        if rise_read {
            let skip = usize::from(flag(plan, each.skip_first));
            let count = members.len();
            for index in skip..count {
                let own = members[index].values[each.order];
                if let Some(next) = members.get(index + 1) {
                    let rise = difference(next.values[each.order], own);
                    let next_id = next.object.id.clone();
                    let next_evidence = next.evidence.clone();
                    let member = &mut members[index];
                    member.values.insert(each.rise.name, rise);
                    member.next = Some(next_id);
                    member.evidence.extend(next_evidence);
                    continue;
                }
                if flag(plan, each.skip_last) {
                    continue;
                }
                let last = each.rise.last.and_then(|highest| {
                    nested
                        .iter()
                        .find(|bound| bound.nested.name == highest.nested)
                        .map(|bound| (bound, highest.value))
                });
                let Some((bound, value)) = last else {
                    evaluation.push_object_not_evaluated(
                        members[index].object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        each.rise.open,
                    );
                    continue;
                };
                match bound.read(plan, context, rule, &members[index], &mut evaluation) {
                    Ok(Some(contents)) => {
                        let (mut lower, mut upper) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
                        let mut evidence = Vec::new();
                        for content in &contents {
                            if let Some(span) = content.values.get(value) {
                                lower = lower.max(span.0);
                                upper = upper.max(span.1);
                            }
                            evidence.extend(content.evidence.iter().cloned());
                        }
                        let member = &mut members[index];
                        member
                            .values
                            .insert(each.rise.name, (lower - own.1, upper - own.0));
                        member.evidence.extend(evidence);
                    }
                    Ok(None) => {}
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(
                            members[index].object.id.clone(),
                            reason,
                            message,
                        );
                    }
                }
            }
        }
        if let (Some(table), Some(form)) = (&mut table, &each.table) {
            for member in &members {
                let row = form
                    .columns
                    .iter()
                    .map(|column| match member.values.get(column.value) {
                        Some((lower, upper)) => ReportValue::measured(*lower, *upper),
                        None => ReportValue::Unknown,
                    })
                    .collect();
                // A member two anchors reach keeps the row of the first.
                let _ = table.push_row(member.object.id.clone(), row);
            }
        }
        for judgement in &checks {
            judge(plan, rule, judgement, &members, None, &mut evaluation);
        }
        for bound in &nested {
            let nested = bound.nested;
            if nested.checks.is_empty() && nested.table.is_none() {
                continue;
            }
            let position = each
                .nested
                .iter()
                .position(|candidate| std::ptr::eq(candidate, nested))
                .unwrap_or_default();
            for member in &members {
                if nested
                    .members_with
                    .is_some_and(|name| !member.values.contains_key(name))
                {
                    continue;
                }
                let subjects = match bound.read(plan, context, rule, member, &mut evaluation) {
                    Ok(Some(subjects)) => subjects,
                    Ok(None) => continue,
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(
                            member.object.id.clone(),
                            reason,
                            message,
                        );
                        continue;
                    }
                };
                if let (Some(Some(table)), Some(form)) =
                    (nested_tables.get_mut(position), &nested.table)
                {
                    for subject in &subjects {
                        let row = std::iter::once(ReportValue::text(member.object.id.to_string()))
                            .chain(form.columns.iter().map(|column| {
                                match value_of(subject, Some(member), column.value) {
                                    Some((lower, upper)) => ReportValue::measured(lower, upper),
                                    None => ReportValue::Unknown,
                                }
                            }))
                            .collect();
                        // A nested member two members reach keeps the row
                        // of the first.
                        let _ = table.push_row(subject.object.id.clone(), row);
                    }
                }
                for judgement in nested
                    .checks
                    .iter()
                    .filter(|judgement| applies(plan, &judgement.applies))
                {
                    judge(
                        plan,
                        rule,
                        judgement,
                        &subjects,
                        Some(member),
                        &mut evaluation,
                    );
                }
            }
        }
    }
    if let Some(table) = table {
        evaluation.push_table(table);
    }
    for table in nested_tables.into_iter().flatten() {
        evaluation.push_table(table);
    }
    evaluation
}
