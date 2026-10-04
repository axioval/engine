//! The leaves of an expression evaluated for one object of a rule: its
//! properties, derived values, and the rule's parameters and tables.

use std::cell::RefCell;
use std::collections::BTreeMap;

use axioval_engine::expression::{
    EvaluationBudget, ExpressionContext, Interval, Leaf, Member, RuleRead, Unit, Value,
    derived_value, evaluate,
};
use axioval_engine::{
    MeasuredMember, Measurement, MemberValue, ObjectVerdict, RuleContext, RuleOutcomes,
};
use axioval_ir::contract::{
    AggregateSource, Expression, ParameterValue, ScalarValue, Selector, TableRow,
};
use axioval_ir::{Evidence, NotEvaluatedReason, Object};

use crate::selection::{Selection, object_by_id, selector_matches};
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{PropertyRef, Resolved, Traversal, resolve};

/// A candidate member: the object, whether it surely belongs, and the
/// evidence that reached it.
type Candidate<'a> = (&'a Object, bool, Vec<Evidence>);

/// Answers an expression's leaves for one selected object.
pub(crate) struct ObjectLeaves<'a> {
    context: &'a RuleContext<'a>,
    object: &'a Object,
    /// The rule's checked object: `object`, except in an aggregate's
    /// member scope.
    subject: &'a Object,
    /// The rule's parameters; a selector reads none.
    parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    /// Why each unreadable leaf was unreadable, in reading order.
    reasons: RefCell<Vec<NotEvaluatedReason>>,
    /// The measured member in scope, whose fields `axioval:member` reads.
    fields: Option<&'a MeasuredMember>,
    /// The evidence of the measured member list last listed.
    listed: Vec<Evidence>,
}

impl<'a> ObjectLeaves<'a> {
    /// Leaves of `object`, reading the rule's `parameters` where given.
    pub(crate) fn new(
        context: &'a RuleContext<'a>,
        object: &'a Object,
        parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    ) -> Self {
        Self {
            context,
            object,
            subject: object,
            parameters,
            reasons: RefCell::new(Vec::new()),
            fields: None,
            listed: Vec::new(),
        }
    }

    /// The leaves of `member`, in an aggregate's member scope: the same
    /// subject and parameters.
    fn member(&self, member: &'a Object) -> Self {
        Self {
            context: self.context,
            object: member,
            subject: self.subject,
            parameters: self.parameters,
            reasons: RefCell::new(Vec::new()),
            fields: None,
            listed: Vec::new(),
        }
    }

    /// The leaves of a measured member of the object in scope: the same
    /// object, its fields read in `axioval:member`.
    fn measured_member(&self, member: &'a MeasuredMember) -> Self {
        Self {
            context: self.context,
            object: self.object,
            subject: self.subject,
            parameters: self.parameters,
            reasons: RefCell::new(Vec::new()),
            fields: Some(member),
            listed: Vec::new(),
        }
    }

    /// The field `name` of the measured member in scope.
    fn member_field(&self, name: &str) -> Leaf {
        let Some(member) = self.fields else {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::InvalidEvidence);
            return Leaf::unreadable(format!(
                "`{}` `{name}` is read only inside an aggregate over measured members",
                axioval_ir::MEMBER_SET
            ));
        };
        let source = self.object.id.source.clone();
        let exact = member.exact;
        let cited = |locator: &str| {
            let locator = format!("{}/{name}:{locator}", axioval_ir::MEMBER_SET);
            vec![Evidence {
                source: source.clone(),
                locator,
                exact,
            }]
        };
        match member.fields.get(name) {
            None => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::InvalidEvidence);
                Leaf::unreadable(format!("the members state no `{name}`"))
            }
            Some(MemberValue::Undecided { why }) => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::IncompleteEvidence);
                Leaf::unreadable(format!("`{name}` is undecided: {why}"))
            }
            Some(MemberValue::Truth { value, locator }) => Leaf {
                value: Ok(Value::Boolean(*value)),
                evidence: cited(locator),
            },
            Some(MemberValue::Measured(Measurement::Absent { locator })) => Leaf {
                value: Ok(Value::Null),
                evidence: cited(locator),
            },
            Some(MemberValue::Measured(
                Measurement::Value {
                    lower,
                    upper,
                    dimension,
                    locator,
                }
                | Measurement::Rounded {
                    lower,
                    upper,
                    dimension,
                    locator,
                }
                | Measurement::Cited {
                    lower,
                    upper,
                    dimension,
                    locator,
                    ..
                },
            )) => {
                let value = Value::from_property(&axioval_ir::PropertyValue::Measured {
                    lower: *lower,
                    upper: *upper,
                    dimension: *dimension,
                });
                if value.is_err() {
                    self.reasons
                        .borrow_mut()
                        .push(NotEvaluatedReason::InvalidEvidence);
                }
                Leaf {
                    value,
                    evidence: cited(locator),
                }
            }
        }
    }

    /// The members of `list` measured of the object in scope, each
    /// evaluated with `value`.
    fn measured_members(
        &mut self,
        list: &str,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        let (measured, listed) =
            axioval_engine::measured_members_cited(self.context.services, &self.object.id, list)
                .map_err(|error| {
                    self.reasons
                        .borrow_mut()
                        .push(crate::selection::property_error(error.clone()).0);
                    format!("`{list}` of {}: {error}", self.object.id)
                })?;
        self.listed = listed;
        let mut members = Vec::new();
        for member in &measured {
            let (value, evidence) = match value {
                None => (Ok(Value::Null), Vec::new()),
                Some(value) => {
                    let mut leaves = self.measured_member(member);
                    let evaluation = evaluate(value, path, &mut leaves);
                    (
                        evaluation.outcome,
                        evaluation
                            .reads
                            .iter()
                            .flat_map(|read| read.leaf.evidence.iter().cloned())
                            .collect(),
                    )
                }
            };
            members.push(Member {
                certain: member.certain,
                value,
                evidence,
            });
        }
        Ok(members)
    }

    /// The candidate members `over` reaches from the object in scope, each
    /// with whether it surely belongs and the evidence that reached it.
    fn candidates(&self, over: &AggregateSource) -> Result<Vec<Candidate<'a>>, String> {
        let context = self.context;
        match over {
            AggregateSource::Path { path } => {
                let traversal = Traversal::path(path).map_err(|(_, why)| why)?;
                let everything: Vec<&Object> = context.project.objects().collect();
                let (reached, cited) = traversal
                    .related(context, &self.object.id, &everything)
                    .map_err(|(_, why)| format!("via {}: {why}", traversal.relationship))?;
                reached
                    .iter()
                    .map(|id| {
                        object_by_id(context, id)
                            .map(|object| (object, true, cited.clone()))
                            .ok_or_else(|| {
                                format!("the path reached {id}, which is not in the run")
                            })
                    })
                    .collect()
            }
            AggregateSource::Group { grouping } => {
                let groups = context
                    .services
                    .get::<std::sync::Arc<axioval_engine::DerivedGroups>>()
                    .ok_or("no derived groups are available outside a run")?;
                let derived = groups
                    .grouping(grouping)
                    .ok_or_else(|| format!("the run derives no grouping `{grouping}`"))?;
                let group = match derived.group(&self.object.id) {
                    Some(group) => group,
                    None => match derived.membership(&self.object.id) {
                        Some(axioval_engine::Membership::Grouped(group, _)) => derived
                            .group(group)
                            .ok_or_else(|| format!("group {group} is not derived"))?,
                        Some(axioval_engine::Membership::Undecided(_, why)) => {
                            return Err(format!(
                                "whether {} belongs to a group of `{grouping}` is undecided: {why}",
                                self.object.id
                            ));
                        }
                        _ => return Ok(Vec::new()),
                    },
                };
                if let Some(why) = group.undecided() {
                    return Err(format!("the members of the group are not all known: {why}"));
                }
                group
                    .members()
                    .iter()
                    .map(|id| {
                        object_by_id(context, id)
                            .map(|object| (object, true, Vec::new()))
                            .ok_or_else(|| format!("member {id} is not in the run"))
                    })
                    .collect()
            }
            AggregateSource::Measured { .. } => Err("measured members are no objects".into()),
            AggregateSource::Selector { selector } => {
                let mut candidates = Vec::new();
                for object in context.project.objects() {
                    let mut evidence = Vec::new();
                    match selector_matches(context, selector, object, &mut evidence) {
                        Selection::Match => candidates.push((object, true, evidence)),
                        Selection::NoMatch => {}
                        Selection::NotEvaluated(..) => candidates.push((object, false, evidence)),
                    }
                }
                Ok(candidates)
            }
        }
    }

    /// Why the first unreadable leaf was unreadable.
    pub(crate) fn first_reason(&self) -> Option<NotEvaluatedReason> {
        self.reasons.borrow().first().cloned()
    }
}

impl ExpressionContext for ObjectLeaves<'_> {
    fn spend(&mut self) -> bool {
        let spent = self
            .context
            .services
            .get::<std::sync::Arc<EvaluationBudget>>()
            .is_none_or(|budget| budget.spend());
        if !spent {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::ResourceLimit);
        }
        spent
    }

    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        if set == Some(axioval_ir::MEMBER_SET) {
            return self.member_field(name);
        }
        if set == Some(axioval_ir::VALUE_SET) {
            return self.derived(name);
        }
        match resolve(self.context, self.object, PropertyRef { set, name }) {
            Ok(resolved) => {
                let evidence = resolved.evidence();
                let value = match &resolved {
                    // A stated absence is `null`, never a value not read.
                    Resolved::Absent(_) => Ok(Value::Null),
                    Resolved::Present(property) => Value::from_property(&property.value),
                };
                if value.is_err() {
                    self.reasons
                        .borrow_mut()
                        .push(NotEvaluatedReason::InvalidEvidence);
                }
                Leaf { value, evidence }
            }
            Err((reason, message)) => {
                self.reasons.borrow_mut().push(reason);
                Leaf::unreadable(message)
            }
        }
    }

    fn derived(&mut self, name: &str) -> Leaf {
        let leaf = derived_value(self.context.services, &self.object.id, name);
        if leaf.value.is_err() {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::IncompleteEvidence);
        }
        leaf
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        let Some(parameters) = self.parameters else {
            return Leaf::unreadable(format!("a selector reads no rule parameter, not `{name}`"));
        };
        match parameters.get(name).cloned().map(ScalarValue::try_from) {
            Some(Ok(scalar)) => match Value::from_literal(&scalar) {
                Ok(value) => Leaf::stated(value),
                Err(why) => Leaf::unreadable(why),
            },
            Some(Err(_)) => Leaf::unreadable(format!("parameter `{name}` is no single value")),
            None => Leaf::unreadable(format!("the rule has no parameter `{name}`")),
        }
    }

    fn subject_property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        let object = self.object;
        self.object = self.subject;
        let leaf = self.property(set, name);
        self.object = object;
        leaf
    }

    fn rule(&mut self, rule: &str, read: RuleRead) -> Leaf {
        let Some(outcomes) = self.context.services.get::<RuleOutcomes>() else {
            return Leaf::unreadable("no rule outcomes are available outside a run");
        };
        let Some(record) = outcomes.get(rule) else {
            return Leaf::unreadable(format!("rule `{rule}` has not run"));
        };
        let verdict = record.object(self.object);
        let (count, deviation) = record.findings_about(&self.object.id);
        let value = match (read, verdict) {
            (_, ObjectVerdict::Undecided(why)) => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::IncompleteEvidence);
                return Leaf::unreadable(format!("rule `{rule}` left it open: {why}"));
            }
            (RuleRead::Outcome, ObjectVerdict::Passed) => Value::Boolean(true),
            (RuleRead::Outcome, ObjectVerdict::Failed) => Value::Boolean(false),
            (RuleRead::Outcome, ObjectVerdict::NotSelected) => Value::Null,
            (RuleRead::FindingCount, _) => Value::integer(i64::try_from(count).unwrap_or(i64::MAX)),
            (RuleRead::Deviation, _) => match deviation {
                Some((lower, upper)) => Value::Number {
                    value: Interval { lower, upper },
                    unit: Unit::NONE,
                },
                None => Value::Null,
            },
        };
        Leaf {
            value: Ok(value),
            evidence: vec![Evidence::exact(
                self.object.id.source.clone(),
                format!("rule:{rule}#{}", self.object.id.local_id),
            )],
        }
    }

    fn members(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        if let AggregateSource::Measured { name } = over {
            return self.measured_members(name, value, path);
        }
        let mut members = Vec::new();
        for (object, mut certain, mut evidence) in self.candidates(over)? {
            if let Some(filter) = filter {
                match selector_matches(self.context, filter, object, &mut evidence) {
                    Selection::Match => {}
                    Selection::NoMatch => continue,
                    Selection::NotEvaluated(..) => certain = false,
                }
            }
            let value = match value {
                None => Ok(Value::Null),
                Some(value) => {
                    let mut leaves = self.member(object);
                    let evaluation = evaluate(value, path, &mut leaves);
                    evidence.extend(
                        evaluation
                            .reads
                            .iter()
                            .flat_map(|read| read.leaf.evidence.iter().cloned()),
                    );
                    evaluation.outcome
                }
            };
            members.push(Member {
                certain,
                value,
                evidence,
            });
        }
        Ok(members)
    }

    fn listing_evidence(&mut self) -> Vec<Evidence> {
        std::mem::take(&mut self.listed)
    }

    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        let Some(ParameterValue::Table { value: rows }) =
            self.parameters.and_then(|parameters| parameters.get(table))
        else {
            return Leaf::unreadable(format!("the rule has no table `{table}`"));
        };
        lookup(rows, keys, column).map_or_else(Leaf::unreadable, Leaf::stated)
    }
}

/// The `column` cell of the most specific row whose key cells match `keys`:
/// a text cell is a wildcard pattern matched against the key's text, any
/// other cell must equal the key, and a blank cell accepts any key. No
/// matching row, or one leaving `column` blank, is `null`; tied or undecided
/// rows have no value.
fn lookup(
    rows: &[TableRow],
    keys: &BTreeMap<String, Value>,
    column: &str,
) -> Result<Value, String> {
    let mut problem = None;
    let matched = match_rows(rows, RowSelection::MostSpecific, |row| {
        let mut verdict = RowTest::Match(0);
        for (key, value) in keys {
            let Some(cell) = row.get(key) else {
                continue;
            };
            let outcome = match (cell, value) {
                (_, Value::Null) => RowTest::NoMatch,
                (ParameterValue::String { value: pattern }, value) => match key_text(value) {
                    Some(text) => match TextPattern::new(pattern, true) {
                        Ok(pattern) => pattern.test(&text),
                        Err(why) => {
                            problem.get_or_insert(why);
                            RowTest::Undecided
                        }
                    },
                    None => RowTest::NoMatch,
                },
                (cell, value) => match ScalarValue::try_from(cell.clone())
                    .ok()
                    .and_then(|cell| Value::from_literal(&cell).ok())
                {
                    Some(cell) => equal(&cell, value),
                    None => RowTest::NoMatch,
                },
            };
            verdict = verdict.and(outcome);
        }
        verdict
    });
    match matched {
        Matched::Rows(found) => Ok(match found.first() {
            None => Value::Null,
            Some((_, row)) => match row.get(column) {
                None => Value::Null,
                Some(cell) => {
                    let scalar = ScalarValue::try_from(cell.clone())
                        .map_err(|_| format!("column `{column}` holds no single value"))?;
                    Value::from_literal(&scalar)?
                }
            },
        }),
        Matched::Undecided => Err(problem.unwrap_or_else(|| "a row cannot be decided".into())),
        Matched::Ambiguous(tied) => Err(format!(
            "rows {} tie for the most specific",
            tied.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A key read as text, as `keyed-limit` reads keys.
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) | Value::Enum(text) => Some(text.clone()),
        Value::Boolean(value) => Some(value.to_string()),
        Value::Number { value, unit } if unit.is_plain() && value.is_point() => {
            Some(value.lower.to_string())
        }
        _ => None,
    }
}

fn equal(cell: &Value, value: &Value) -> RowTest {
    match (cell, value) {
        (
            Value::Number {
                value: cell,
                unit: cell_unit,
            },
            Value::Number { value, unit },
        ) if cell_unit == unit => {
            if cell.upper < value.lower || cell.lower > value.upper {
                RowTest::NoMatch
            } else if cell.is_point() && value.is_point() {
                RowTest::Match(1)
            } else {
                RowTest::Undecided
            }
        }
        (cell, value) if cell == value => RowTest::Match(1),
        _ => RowTest::NoMatch,
    }
}
