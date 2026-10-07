//! The pairs of a measured member list put in classes ([`Pairs`]): each
//! candidate pair a provider measured judged by the first class that
//! holds, three-valued, then graded and grouped.
//!
//! The list is read once per rule, of the project, `@selection` the objects
//! the rule selects. An item the provider left open (a body it could not
//! measure, a pair whose tolerances it could not choose) is open on its
//! subject; every other item is a pair. Its classes are tried in order: a
//! class surely holding decides it, one surely not holding hands it on,
//! one that may hold decides only where both readings agree. Open outcomes
//! are reported once per object, reason and message, as a set, the rule's
//! own selections' outcomes among them.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::template::{
    Bound, ClassSeverities, Operand, PairClass, PairGrades, PairGroups, PairOrder, PairTest, Pairs,
    When,
};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, MeasuredMember, Measurement, MemberValue, RuleContext,
};
use axioval_ir::contract::{ParameterValue, ScalarValue};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Severity};

use super::{Constant, Plan, Read, formatted, placeholder, project_object};
use crate::measured_arguments::Arguments;
use crate::selection::select_shared;
use crate::support::finding;

/// Kleene's three truth values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth {
    Yes,
    No,
    Unknown,
}

impl Truth {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }

    fn not(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// One field of an item as its member states it.
enum Field<'m> {
    Number((f64, f64)),
    Truth(bool),
    Text(&'m str),
    Objects(&'m [ObjectId]),
    Null,
    Undecided(&'m str),
    Missing,
}

fn field<'m>(member: &'m MeasuredMember, name: &str) -> Field<'m> {
    match member.fields.get(name) {
        None => Field::Missing,
        Some(MemberValue::Undecided { why }) => Field::Undecided(why),
        Some(MemberValue::Truth { value, .. }) => Field::Truth(*value),
        Some(MemberValue::Text { text }) => Field::Text(text),
        Some(MemberValue::Objects { objects }) => Field::Objects(objects),
        Some(MemberValue::Measured(Measurement::Absent { .. })) => Field::Null,
        Some(MemberValue::Measured(
            Measurement::Value { lower, upper, .. }
            | Measurement::Rounded { lower, upper, .. }
            | Measurement::Cited { lower, upper, .. },
        )) => Field::Number((*lower, *upper)),
    }
}

/// A text field's words; none where it states none.
fn text<'m>(member: &'m MeasuredMember, name: &str) -> &'m str {
    match field(member, name) {
        Field::Text(text) => text,
        _ => "",
    }
}

/// The first object an objects field names.
fn first_object(member: &MeasuredMember, name: &str) -> Option<ObjectId> {
    match field(member, name) {
        Field::Objects(objects) => objects.first().cloned(),
        _ => None,
    }
}

/// A not-evaluated reason as reports spell it (`incomplete_evidence`);
/// incomplete evidence where the words name none.
fn reason(words: &str) -> NotEvaluatedReason {
    serde_json::from_value(serde_json::Value::String(words.to_owned()))
        .unwrap_or(NotEvaluatedReason::IncompleteEvidence)
}

/// A severity as a rule states it.
fn severity_of(words: &str) -> Option<Severity> {
    match words {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        _ => None,
    }
}

fn severity_words(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// What a pair comes to.
enum Judged {
    Pass,
    /// A finding of the class at the index, worded.
    Finding(usize, String),
    Open(NotEvaluatedReason, String),
}

/// The outcome where a class may or may not hold: it stands only where
/// both readings agree; two findings keep the later class's.
fn either(yes: Judged, no: Judged, undecided: impl FnOnce() -> String) -> Judged {
    match (yes, no) {
        (Judged::Pass, Judged::Pass) => Judged::Pass,
        (Judged::Finding(..), Judged::Finding(class, message)) => Judged::Finding(class, message),
        (_, Judged::Open(reason, message)) | (Judged::Open(reason, message), _) => {
            Judged::Open(reason, message)
        }
        _ => Judged::Open(NotEvaluatedReason::IncompleteEvidence, undecided()),
    }
}

/// One finding before grouping.
struct Reported {
    subject: ObjectId,
    related: Vec<ObjectId>,
    class: &'static str,
    message: String,
    severity: Severity,
    evidence: Vec<Evidence>,
}

/// The judge of one rule: its plan and the decision's data.
struct Judge<'p, 't> {
    plan: &'p Plan<'t>,
    pairs: &'p Pairs,
    /// The rule's own severity.
    rule: Severity,
    /// The list's descriptor: which fields its items may leave out.
    listed: Option<&'static axioval_ir::measured::MemberDescriptor>,
}

impl Judge<'_, '_> {
    /// Whether `when` holds for the rule and the item.
    fn holds(&self, member: &MeasuredMember, when: &[When]) -> bool {
        when.iter().all(|condition| match *condition {
            When::Field { field: name, value } => {
                matches!(field(member, name), Field::Truth(held) if held == value)
            }
            When::Null { field: name } => matches!(field(member, name), Field::Null),
            When::Stated { field: name } => {
                !matches!(field(member, name), Field::Null | Field::Missing)
            }
            When::Declared { parameter } => declared(self.plan, parameter),
            When::Undeclared { parameter } => !declared(self.plan, parameter),
            When::Equals { parameter, value } => matches!(
                self.plan.constants.get(parameter),
                Some(Constant::Text(stated)) if stated == value
            ),
            #[allow(clippy::float_cmp)]
            When::Is { field: name, value } => {
                matches!(field(member, name), Field::Number((low, high)) if low == value && high == value)
            }
            When::Below { field: name, value } => {
                matches!(field(member, name), Field::Number((_, high)) if high < value)
            }
            When::Empty { field: name } => {
                matches!(field(member, name), Field::Objects(objects) if objects.is_empty())
            }
            When::Unknown { field: name } => matches!(field(member, name), Field::Undecided(_)),
            When::Undecided { .. } => false,
        })
    }

    /// `template` with its placeholders rendered over the item: the words
    /// the judgement names, the texts, the item's fields (a number with a
    /// numeric format, `{penetration:fixed4}`), then the rule's
    /// parameters.
    fn render(&self, member: &MeasuredMember, template: &str, named: &[(&str, String)]) -> String {
        let mut out = String::with_capacity(template.len() * 2);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            out.push_str(&rest[..open]);
            let key = &rest[open + 1..open + close];
            match self.placeholder(member, key, named) {
                Some(text) => out.push_str(&text),
                None => out.push_str(&rest[open..=open + close]),
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    }

    fn placeholder(
        &self,
        member: &MeasuredMember,
        key: &str,
        named: &[(&str, String)],
    ) -> Option<String> {
        if let Some((_, text)) = named.iter().find(|(name, _)| *name == key) {
            return Some(text.clone());
        }
        let (name, format) = key.rsplit_once(':').unwrap_or((key, ""));
        if format.is_empty() {
            let mut texts = self
                .pairs
                .texts
                .iter()
                .filter(|text| text.name == name)
                .peekable();
            if texts.peek().is_some() {
                return Some(
                    texts
                        .find(|text| self.holds(member, &text.when))
                        .map(|text| self.render(member, text.text, named))
                        .unwrap_or_default(),
                );
            }
        }
        match field(member, name) {
            Field::Number(span) => {
                if let Some(shown) = formatted(format, span) {
                    return Some(shown);
                }
            }
            Field::Text(text) if format.is_empty() => return Some(text.to_owned()),
            Field::Objects(objects) if format.is_empty() => {
                return Some(
                    objects
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            Field::Truth(value) if format.is_empty() => return Some(value.to_string()),
            // A text field the item leaves out states no words.
            Field::Missing
                if format.is_empty()
                    && self
                        .listed
                        .and_then(|listed| listed.field(name))
                        .is_some_and(|declared| {
                            declared.kind == axioval_ir::measured::MemberFieldKind::Text
                        }) =>
            {
                return Some(String::new());
            }
            _ => {}
        }
        placeholder(self.plan, &Read::default(), key)
    }

    /// A bound's interval: an item's number, a parameter, a literal; `None`
    /// where there is none to compare with, and `Err` where it is
    /// undecided.
    fn bound(&self, member: &MeasuredMember, bound: Bound) -> Result<Option<(f64, f64)>, ()> {
        match bound {
            Bound::Literal(value) => Ok(Some((value, value))),
            Bound::Operand(Operand::Parameter(name)) => Ok(self
                .plan
                .constants
                .get(name)
                .and_then(Constant::number)
                .map(|value| (value, value))),
            Bound::Operand(Operand::Value(name)) => match field(member, name) {
                Field::Number(span) => Ok(Some(span)),
                Field::Undecided(_) => Err(()),
                _ => Ok(None),
            },
        }
    }

    /// A test of the item, three-valued.
    fn test(&self, member: &MeasuredMember, test: &PairTest) -> Truth {
        match *test {
            PairTest::Truth { field: name, value } => match field(member, name) {
                Field::Truth(held) if held == value => Truth::Yes,
                Field::Undecided(_) => Truth::Unknown,
                _ => Truth::No,
            },
            PairTest::Stated {
                field: name,
                stated,
            } => {
                let held = !matches!(field(member, name), Field::Null | Field::Missing);
                if held == stated {
                    Truth::Yes
                } else {
                    Truth::No
                }
            }
            PairTest::Compare {
                value,
                order,
                bound,
                zero,
            } => {
                let (low, high) = match self.bound(member, bound) {
                    Ok(Some(span)) => span,
                    Ok(None) => return Truth::No,
                    Err(()) => return Truth::Unknown,
                };
                #[allow(clippy::float_cmp)]
                if zero && low == 0.0 && high == 0.0 {
                    return Truth::Yes;
                }
                match field(member, value) {
                    Field::Number(span) => ordered(span, order, (low, high)),
                    Field::Undecided(_) => Truth::Unknown,
                    _ => Truth::No,
                }
            }
        }
    }

    /// The pair judged by its classes from `index` on.
    fn classify(&self, member: &MeasuredMember, index: usize) -> Judged {
        let Some(class) = self.pairs.classes.get(index) else {
            return Judged::Pass;
        };
        let mut holds = Truth::Yes;
        for test in &class.holds {
            holds = holds.and(self.test(member, test));
            if holds == Truth::No {
                return self.classify(member, index + 1);
            }
        }
        // Switched off only where the item says so.
        let reported = class
            .reported
            .is_none_or(|name| !matches!(field(member, name), Field::Truth(false)));
        // An excuse is consulted only where it can change the outcome.
        if reported && let Some(excused) = &class.excused {
            holds = holds.and(self.test(member, excused).not());
        }
        let own = || self.own(member, index, class, reported);
        match holds {
            Truth::Yes => own(),
            Truth::No => self.classify(member, index + 1),
            Truth::Unknown => either(own(), self.classify(member, index + 1), || {
                self.render(member, class.undecided, &[])
            }),
        }
    }

    /// What a pair of the class comes to where the class holds.
    fn own(
        &self,
        member: &MeasuredMember,
        index: usize,
        class: &PairClass,
        reported: bool,
    ) -> Judged {
        if !reported {
            Judged::Pass
        } else if class.opens {
            Judged::Open(
                NotEvaluatedReason::IncompleteEvidence,
                self.render(member, class.fail, &[]),
            )
        } else {
            Judged::Finding(index, self.render(member, class.fail, &[]))
        }
    }

    /// A finding's severity and the words saying how it was graded.
    fn severity(&self, member: &MeasuredMember, class: &str) -> (Severity, String) {
        let rule = self.rule.clone();
        let severity = &self.pairs.severity;
        let stated = severity
            .stated
            .and_then(|name| severity_of(text(member, name)));
        let fixed = stated
            .or_else(|| {
                severity
                    .by_class
                    .and_then(|by_class| self.class_severity(by_class, class))
            })
            .unwrap_or(rule);
        match &severity.grades {
            Some(grades) if grades.class == class => self.graded(member, grades, fixed),
            _ => (fixed, String::new()),
        }
    }

    /// The severity a table parameter names for `class`.
    fn class_severity(&self, by_class: ClassSeverities, class: &str) -> Option<Severity> {
        let Some(Constant::Other(ParameterValue::Table { value: rows })) =
            self.plan.constants.get(by_class.table)
        else {
            return None;
        };
        rows.iter()
            .find(|row| {
                matches!(row.get(by_class.class), Some(ParameterValue::String { value }) if value == class)
            })
            .and_then(|row| match row.get(by_class.severity) {
                Some(ParameterValue::String { value }) => severity_of(value),
                _ => None,
            })
    }

    /// A graded class's severity: the most severe grade its measure may
    /// reach.
    fn graded(
        &self,
        member: &MeasuredMember,
        grades: &PairGrades,
        fixed: Severity,
    ) -> (Severity, String) {
        let Some(measure) = (match self.plan.constants.get(grades.by) {
            Some(Constant::Text(option)) => grades
                .measures
                .iter()
                .find(|measure| measure.option == option),
            _ => None,
        }) else {
            return (fixed, String::new());
        };
        let Some(Constant::Other(ParameterValue::Table { value: rows })) =
            self.plan.constants.get(grades.table)
        else {
            return (fixed, String::new());
        };
        let mut steps: Vec<(f64, Severity)> = rows
            .iter()
            .filter_map(|row| {
                let above = match row.get(grades.above) {
                    Some(ParameterValue::Number { value }) => *value,
                    #[allow(clippy::cast_precision_loss)]
                    Some(ParameterValue::Integer { value }) => *value as f64,
                    _ => return None,
                };
                let severity = match row.get(grades.severity) {
                    Some(ParameterValue::String { value }) => severity_of(value)?,
                    _ => return None,
                };
                Some((above, severity))
            })
            .collect();
        steps.sort_by(|a, b| a.0.total_cmp(&b.0));
        let grade_of = |value: f64| {
            steps
                .iter()
                .rev()
                .find(|(above, _)| value > *above)
                .map_or_else(|| fixed.clone(), |(_, severity)| severity.clone())
        };
        let Field::Number((lower, upper)) = field(member, measure.field) else {
            // Not measured: it may fall in any grade.
            let worst = steps
                .iter()
                .map(|(_, severity)| severity.clone())
                .chain([fixed.clone()])
                .min()
                .unwrap_or(Severity::Error);
            let words = self.render(
                member,
                grades.unmeasured,
                &[
                    ("severity", severity_words(&worst).to_owned()),
                    ("measure", measure.words.to_owned()),
                ],
            );
            return (worst, words);
        };
        let worst = steps
            .iter()
            .filter(|(above, _)| *above >= lower && *above < upper)
            .map(|(_, severity)| severity.clone())
            .chain([grade_of(lower)])
            .min()
            .unwrap_or_else(|| grade_of(lower));
        let digits = measure.digits;
        let value = if lower.total_cmp(&upper).is_eq() {
            format!("{lower:.digits$} {}", measure.unit)
        } else {
            format!("{lower:.digits$} to {upper:.digits$} {}", measure.unit)
        };
        let words = if worst == grade_of(upper) && worst == grade_of(lower) {
            grades.graded
        } else {
            grades.reaches
        };
        let words = self.render(
            member,
            words,
            &[
                ("severity", severity_words(&worst).to_owned()),
                ("measure", measure.words.to_owned()),
                ("value", value),
            ],
        );
        (worst, words)
    }
}

/// Whether every value of `(lower, upper)` stands in `order` to every value
/// of the bound `(low, high)` (yes), none does (no), or some may (unknown).
fn ordered((lower, upper): (f64, f64), order: PairOrder, (low, high): (f64, f64)) -> Truth {
    let (yes, no) = match order {
        PairOrder::Below => (upper < low, lower >= high),
        PairOrder::AtMost => (upper <= low, lower > high),
        PairOrder::Above => (lower > high, upper <= low),
        PairOrder::AtLeast => (lower >= high, upper < low),
    };
    if yes {
        Truth::Yes
    } else if no {
        Truth::No
    } else {
        Truth::Unknown
    }
}

/// Whether the rule states `parameter` (a boolean, or its default, true).
fn declared(plan: &Plan<'_>, parameter: &str) -> bool {
    !matches!(
        plan.constants.get(parameter),
        None | Some(Constant::Scalar(ScalarValue::Boolean { value: false }))
    )
}

/// The pairs `pairs`' list states for the rule: parsed once per rule as
/// the plan bound it, its references bound and measured once per run, of
/// the project; why not, as the list refuses it.
fn list(
    plan: &Plan<'_>,
    pairs: &Pairs,
    context: &RuleContext<'_>,
    arguments: &Arguments,
    project: &axioval_ir::Object,
) -> Result<Vec<MeasuredMember>, (NotEvaluatedReason, String)> {
    // Written once per rule, as the plan keeps it, and parsed once.
    let written = plan.bound.written_list(pairs.list);
    let parsed = crate::measured_arguments::parsed(&written, true)
        .map_or_else(|| axioval_ir::measured::parse_members(&written), Ok);
    let mut call =
        parsed.map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    crate::measured_arguments::bind(
        context,
        Some(&plan.bound.parameters),
        Some(arguments),
        &project.id,
        &mut call,
    )?;
    axioval_engine::measured_members_bound(context.services, &project.id, &call)
        .map(|(members, _)| members)
        .map_err(|error| {
            let (reason, message) = crate::selection::property_error(error);
            let message = message
                .strip_prefix("property evidence conflicts: ")
                .map_or_else(|| message.clone(), ToOwned::to_owned);
            (reason, message)
        })
}

/// Judges the pairs of `pairs`' list for `rule`.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    plan: &Plan<'_>,
    pairs: &Pairs,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let (selected, mut evaluation) = select_shared(context, &rule.selector);
    let arguments = Arguments::of_rule(rule).selected(&selected, &evaluation);
    let project = project_object();
    let listed = list(plan, pairs, context, &arguments, &project);
    let members = match listed {
        Ok(members) => members,
        Err((reason, why)) => {
            // A refused list refuses every selected object.
            for object in &selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    reason.clone(),
                    why.clone(),
                );
            }
            return evaluation;
        }
    };
    // Open outcomes, each once: the selections' and the items'.
    let mut opened: BTreeSet<(ObjectId, NotEvaluatedReason, String)> = BTreeSet::new();
    let mut note = |evaluation: &CapabilityEvaluation| {
        for outcome in evaluation.not_evaluated_outcomes() {
            if let Some(object) = outcome.object_id() {
                opened.insert((
                    object.clone(),
                    outcome.reason().clone(),
                    outcome.message().to_owned(),
                ));
            }
        }
    };
    note(&evaluation);
    for name in pairs.selections {
        if let Some(Constant::Other(ParameterValue::Selector { value: selector })) =
            plan.constants.get(*name)
        {
            note(&select_shared(context, selector).1);
        }
    }
    let judge = Judge {
        plan,
        pairs,
        rule: crate::pairs::severity(rule),
        listed: pairs
            .list
            .split(';')
            .next()
            .and_then(axioval_ir::measured::member_descriptor),
    };
    let grouping = pairs
        .groups
        .as_ref()
        .filter(|groups| super::each::applies(plan, &groups.applies));
    let mut result = CapabilityEvaluation::default();
    // Each item judged and let go: only a grouped finding keeps its item.
    let mut reported: Vec<(Reported, MeasuredMember)> = Vec::new();
    for (index, mut member) in members.into_iter().enumerate() {
        let member_ref = &member;
        let Some(subject) = first_object(member_ref, pairs.subject) else {
            continue;
        };
        let judged = judged(&judge, pairs, member_ref);
        let judged = match judged {
            Ok(judged) => judged,
            Err(open) => {
                opened.insert((subject, open.0, open.1));
                continue;
            }
        };
        match judged {
            Judged::Pass => {}
            Judged::Open(reason, message) => {
                opened.insert((subject, reason, message));
            }
            Judged::Finding(class, message) => {
                let class = pairs.classes[class].name;
                let (severity, graded) = judge.severity(&member, class);
                let mut message = message;
                message.push_str(&graded);
                if let Some(suffix) = pairs.suffix {
                    message.push_str(&judge.render(&member, suffix, &[]));
                }
                let related = match field(&member, pairs.related) {
                    Field::Objects(objects) => objects.to_vec(),
                    _ => Vec::new(),
                };
                let evidence = cited(&mut member, &subject, pairs.list, index);
                let found = Reported {
                    subject,
                    related,
                    class,
                    message,
                    severity,
                    evidence,
                };
                if grouping.is_some() {
                    reported.push((found, member));
                } else {
                    push(&mut result, rule, found);
                }
            }
        }
    }
    if let Some(groups) = grouping {
        group(&judge, groups, rule, reported, &mut result);
    }
    for (object, reason, message) in opened {
        result.push_object_not_evaluated(object, reason, message);
    }
    result
}

/// What an item comes to: open as it states, or judged by the classes,
/// an undecided exclusion opening anything but a pass.
fn judged(
    judge: &Judge<'_, '_>,
    pairs: &Pairs,
    member: &MeasuredMember,
) -> Result<Judged, (NotEvaluatedReason, String)> {
    let open = text(member, pairs.open.message);
    if !open.is_empty() {
        return Err((reason(text(member, pairs.open.reason)), open.to_owned()));
    }
    let mut judged = judge.classify(member, 0);
    if let Some(unless) = &pairs.unless {
        let message = text(member, unless.message);
        if !message.is_empty() && !matches!(judged, Judged::Pass) {
            judged = Judged::Open(reason(text(member, unless.reason)), message.to_owned());
        }
    }
    Ok(judged)
}

/// What an item's finding cites: what the item was measured from, or,
/// where it states nothing, its place in the list, as exact as it is.
fn cited(
    member: &mut MeasuredMember,
    subject: &ObjectId,
    list: &str,
    index: usize,
) -> Vec<Evidence> {
    if member.evidence.is_empty() {
        vec![Evidence {
            source: subject.source.clone(),
            locator: format!("{list}#{index}"),
            exact: member.exact,
        }]
    } else {
        std::mem::take(&mut member.evidence)
    }
}

fn push(evaluation: &mut CapabilityEvaluation, rule: &CompiledRule, found: Reported) {
    let mut finding = finding(
        rule,
        &found.subject,
        found.message,
        found.evidence,
        Vec::new(),
    )
    .with_related(found.related);
    finding.severity = found.severity;
    evaluation.push_finding(finding);
}

/// Groups the findings by their key: one finding per group, on the object
/// most of its pairs involve; a pair whose key cannot be read alone.
fn group(
    judge: &Judge<'_, '_>,
    groups: &PairGroups,
    rule: &CompiledRule,
    reported: Vec<(Reported, MeasuredMember)>,
    evaluation: &mut CapabilityEvaluation,
) {
    let mut grouped: BTreeMap<String, Vec<(Reported, MeasuredMember)>> = BTreeMap::new();
    let mut alone: Vec<Reported> = Vec::new();
    for (mut found, member) in reported {
        match field(&member, groups.key) {
            Field::Text(key) => {
                let mut key = key.to_owned();
                if judge.holds(&member, &groups.classed) {
                    key.push('\u{1f}');
                    key.push_str(found.class);
                }
                grouped.entry(key).or_default().push((found, member));
            }
            other => {
                let why = match other {
                    Field::Undecided(why) => why.to_owned(),
                    _ => format!("the members state no `{}`", groups.key),
                };
                let words = judge.render(&member, groups.alone, &[("why", why)]);
                found.message.push_str(&words);
                alone.push(found);
            }
        }
    }
    for (_, mut members) in grouped {
        if members.len() == 1 {
            if let Some((found, _)) = members.pop() {
                push(evaluation, rule, found);
            }
            continue;
        }
        let class = members[0].0.class;
        let mut counts: BTreeMap<ObjectId, usize> = BTreeMap::new();
        for (found, _) in &members {
            *counts.entry(found.subject.clone()).or_default() += 1;
            for related in &found.related {
                *counts.entry(related.clone()).or_default() += 1;
            }
        }
        let most = counts.values().copied().max().unwrap_or(0);
        let Some(hub) = counts
            .iter()
            .find_map(|(object, count)| (*count == most).then(|| object.clone()))
        else {
            continue;
        };
        let severity = members
            .iter()
            .map(|(found, _)| found.severity.clone())
            .min()
            .unwrap_or(Severity::Error);
        let count = members.len();
        let mut evidence: Vec<Evidence> = Vec::new();
        let mut parts = Vec::with_capacity(count);
        for (found, member) in &mut members {
            parts.push(judge.render(
                member,
                groups.part,
                &[
                    ("subject", found.subject.to_string()),
                    ("message", std::mem::take(&mut found.message)),
                ],
            ));
            for cited in std::mem::take(&mut found.evidence) {
                if !evidence.contains(&cited) {
                    evidence.push(cited);
                }
            }
        }
        let message = judge.render(
            &members[0].1,
            groups.message,
            &[
                ("count", count.to_string()),
                ("class", class.to_owned()),
                ("parts", parts.join("; ")),
            ],
        );
        push(
            evaluation,
            rule,
            Reported {
                subject: hub,
                related: counts.into_keys().collect(),
                class,
                message,
                severity,
                evidence,
            },
        );
    }
    for found in alone {
        push(evaluation, rule, found);
    }
}

#[cfg(test)]
mod tests {
    use super::{Judged, NotEvaluatedReason, PairOrder, Truth, either, ordered, reason};

    /// A value stands in an order to a bound where every value it may take
    /// does, fails where none does, and may otherwise: a duplicate within
    /// its tolerance, a penetration past it, a separation below a
    /// clearance.
    #[test]
    fn an_interval_is_ordered_three_valued() {
        // At most: the upper end within the bound holds, the lower end past
        // it fails, the bound itself holds.
        assert_eq!(
            ordered((0.0, 0.0), PairOrder::AtMost, (0.0, 0.0)),
            Truth::Yes
        );
        assert_eq!(
            ordered((0.2, 0.3), PairOrder::AtMost, (0.1, 0.1)),
            Truth::No
        );
        assert_eq!(
            ordered((0.05, 0.2), PairOrder::AtMost, (0.1, 0.1)),
            Truth::Unknown
        );
        // Above: strictly past the bound.
        assert_eq!(ordered((0.1, 0.1), PairOrder::Above, (0.1, 0.1)), Truth::No);
        assert_eq!(
            ordered((0.11, 0.2), PairOrder::Above, (0.1, 0.1)),
            Truth::Yes
        );
        assert_eq!(
            ordered((0.05, 0.2), PairOrder::Above, (0.1, 0.1)),
            Truth::Unknown
        );
        // Below: strictly short of the bound; at it fails.
        assert_eq!(
            ordered((0.01, 0.04), PairOrder::Below, (0.05, 0.05)),
            Truth::Yes
        );
        assert_eq!(
            ordered((0.05, 0.06), PairOrder::Below, (0.05, 0.05)),
            Truth::No
        );
        assert_eq!(
            ordered((0.04, 0.06), PairOrder::Below, (0.05, 0.05)),
            Truth::Unknown
        );
        // At least, against a bound that is an interval: every value of
        // the bound must be reached.
        assert_eq!(
            ordered((0.3, 0.4), PairOrder::AtLeast, (0.1, 0.2)),
            Truth::Yes
        );
        assert_eq!(
            ordered((0.15, 0.4), PairOrder::AtLeast, (0.1, 0.2)),
            Truth::Unknown
        );
        assert_eq!(
            ordered((0.0, 0.05), PairOrder::AtLeast, (0.1, 0.2)),
            Truth::No
        );
    }

    /// Kleene's `and` fails on any failure and holds only where both hold.
    #[test]
    fn tests_combine_three_valued() {
        assert_eq!(Truth::Yes.and(Truth::Unknown), Truth::Unknown);
        assert_eq!(Truth::Unknown.and(Truth::No), Truth::No);
        assert_eq!(Truth::Yes.and(Truth::Yes), Truth::Yes);
        assert_eq!(Truth::Unknown.not(), Truth::Unknown);
    }

    /// A class that may hold decides only where both readings agree: two
    /// findings keep the later class's, two passes pass, an open reading
    /// stands, and a pass against a finding is open with the class's
    /// words.
    #[test]
    fn an_undecided_class_stands_only_where_both_readings_agree() {
        let words = || "undecided".to_owned();
        let finding = |class: usize| Judged::Finding(class, format!("class {class}"));
        assert!(matches!(
            either(finding(0), finding(1), words),
            Judged::Finding(1, _)
        ));
        assert!(matches!(
            either(Judged::Pass, Judged::Pass, words),
            Judged::Pass
        ));
        assert!(matches!(
            either(finding(0), Judged::Pass, words),
            Judged::Open(NotEvaluatedReason::IncompleteEvidence, message) if message == "undecided"
        ));
        assert!(matches!(
            either(
                Judged::Pass,
                Judged::Open(NotEvaluatedReason::InvalidEvidence, "later".into()),
                words
            ),
            Judged::Open(NotEvaluatedReason::InvalidEvidence, message) if message == "later"
        ));
    }

    /// A reason is read as reports spell it, and fails closed as
    /// incomplete evidence where it is not one.
    #[test]
    fn a_reason_is_read_as_reports_spell_it() {
        assert_eq!(reason("not_recorded"), NotEvaluatedReason::NotRecorded);
        assert_eq!(
            reason("invalid_declaration"),
            NotEvaluatedReason::InvalidDeclaration
        );
        assert_eq!(reason("anything"), NotEvaluatedReason::IncompleteEvidence);
    }
}
