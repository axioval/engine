//! Items of a measured member list judged one by one ([`Items`]): each
//! failing test its own outcome on the checked object, or every item in
//! one outcome ([`Together`]).
//!
//! The list is measured once per object (`ObjectLeaves::bound_members`),
//! its `@` references bound to the rule's parameters; each item's fields
//! are read as the provider states them, and judged here against the
//! rule's parameters in plain binary arithmetic, as the capabilities
//! judged them: a verdict needs the whole interval on one side of a bound
//! widened by its allowance, and one straddling it is open.

use axioval_engine::Deviation;
use axioval_engine::template::{
    Allowance, Alternatives, Any, Applies, Bound, Combined, Count, Effect, End, Every, Group,
    Guard, ItemCheck, ItemTest, ItemUnit, Items, Judge, Least, On, OnNull, OpenItems, Operand,
    Requirement, Rows, Spread, TogetherJudge, Truths, When,
};
use axioval_engine::{MeasuredMember, Measurement, MemberValue};
use axioval_ir::contract::{ParameterValue, ScalarValue};
use axioval_ir::{Evidence, NotEvaluatedReason, Object, ObjectId};

use super::{Constant, Outcome, Plan, Read, placeholder};
use crate::expression_leaves::ObjectLeaves;
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, deviation, judge};

/// An interval of an item's number.
type Span = (f64, f64);

/// One field of an item as the member states it.
enum Field<'m> {
    Number(Span),
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

/// What a check's messages read of one item, beside the plan.
struct Scope<'s, 'p, 't> {
    plan: &'s Plan<'t>,
    items: &'s Items,
    read: &'s Read,
    leaves: &'s ObjectLeaves<'p>,
    member: Option<&'s MeasuredMember>,
    /// The item's place among those judged together, and their number.
    place: Option<(usize, usize)>,
    /// Placeholders a judgement states: `{bound}`, `{why}`, `{failing}`.
    named: Vec<(&'static str, String)>,
    /// The operands requirements chose, by their names.
    chosen: Vec<(&'static str, Span)>,
    /// The unit of the judgement's value, which `{bound}` is shown in.
    unit: ItemUnit,
}

impl<'p, 't> Scope<'_, 'p, 't> {
    fn with<'a>(&'a self, member: Option<&'a MeasuredMember>) -> Scope<'a, 'p, 't> {
        Scope {
            plan: self.plan,
            items: self.items,
            read: self.read,
            leaves: self.leaves,
            member,
            place: self.place,
            named: Vec::new(),
            chosen: Vec::new(),
            unit: self.unit,
        }
    }

    fn named(&mut self, name: &'static str, text: String) {
        self.named.retain(|(key, _)| *key != name);
        self.named.push((name, text));
    }

    /// Whether every condition holds for the rule and the item.
    fn holds(&self, when: &[When]) -> bool {
        when.iter().all(|condition| self.holds_one(*condition))
    }

    fn holds_one(&self, condition: When) -> bool {
        let field_of = |name| {
            self.member
                .map_or(Field::Missing, |member| field(member, name))
        };
        match condition {
            When::Field { field, value } => {
                matches!(field_of(field), Field::Truth(held) if held == value)
            }
            When::Null { field } => matches!(field_of(field), Field::Null),
            When::Stated { field } => !matches!(field_of(field), Field::Null | Field::Missing),
            When::Declared { parameter } => declared(self.plan, parameter),
            When::Undeclared { parameter } => !declared(self.plan, parameter),
            When::Undecided { selector } => self.leaves.leaves_undecided(selector),
            When::Equals { parameter, value } => matches!(
                self.plan.constants.get(parameter),
                Some(Constant::Text(stated)) if stated == value
            ),
            // An exact number, such as a count: compared exactly.
            #[allow(clippy::float_cmp)]
            When::Is { field, value } => {
                matches!(field_of(field), Field::Number((low, high)) if low == value && high == value)
            }
            When::Below { field, value } => {
                matches!(field_of(field), Field::Number((_, high)) if high < value)
            }
            When::Empty { field } => {
                matches!(field_of(field), Field::Objects(objects) if objects.is_empty())
            }
            When::Unknown { field } => matches!(field_of(field), Field::Undecided(_)),
        }
    }

    /// `template` with its placeholders rendered.
    fn render(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len() * 2);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            out.push_str(&rest[..open]);
            let key = &rest[open + 1..open + close];
            match self.placeholder(key) {
                Some(text) => out.push_str(&text),
                None => out.push_str(&rest[open..=open + close]),
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    }

    fn placeholder(&self, key: &str) -> Option<String> {
        // A placeholder a judgement states with its format (`bound:plain`).
        if let Some((_, text)) = self.named.iter().find(|(named, _)| *named == key) {
            return Some(text.clone());
        }
        let (name, format) = key.rsplit_once(':').unwrap_or((key, ""));
        if format.is_empty() {
            if let Some((_, text)) = self.named.iter().find(|(named, _)| *named == name) {
                return Some(text.clone());
            }
            match (name, self.place) {
                ("index", Some((index, _))) => return Some(index.to_string()),
                ("count", Some((_, count))) => return Some(count.to_string()),
                _ => {}
            }
            let mut texts = self
                .items
                .texts
                .iter()
                .filter(|text| text.name == name)
                .peekable();
            if texts.peek().is_some() {
                return Some(
                    texts
                        .find(|text| self.holds(&text.when))
                        .map(|text| self.render(text.text))
                        .unwrap_or_default(),
                );
            }
        }
        let unit = unit_of(format);
        // A number with a fixed number of decimals, or a hundredfold.
        if let Some(member) = self.member
            && let Field::Number(span) = field(member, name)
            && let Some(shown) = super::formatted(format, span)
        {
            return Some(shown);
        }
        if let Some((_, span)) = self.chosen.iter().find(|(chosen, _)| *chosen == name) {
            return Some(show(*span, unit.unwrap_or(self.unit)));
        }
        if let Some(member) = self.member {
            match field(member, name) {
                Field::Text(text) => return Some(text.to_owned()),
                Field::Objects(objects) => {
                    return Some(
                        objects
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(if format == "and" { " and " } else { ", " }),
                    );
                }
                Field::Number(span) => return Some(show(span, unit.unwrap_or(ItemUnit::Length))),
                Field::Truth(value) => return Some(value.to_string()),
                _ => {}
            }
        }
        if let (Some(unit), Some(value)) = (
            unit,
            self.plan.constants.get(name).and_then(Constant::number),
        ) {
            return Some(point(value, unit));
        }
        placeholder(self.plan, self.read, key)
    }
}

/// The unit a placeholder's format names.
fn unit_of(format: &str) -> Option<ItemUnit> {
    match format {
        "length" => Some(ItemUnit::Length),
        "degrees" => Some(ItemUnit::Degrees),
        "ratio" => Some(ItemUnit::Ratio),
        "count" => Some(ItemUnit::Count),
        "area" => Some(ItemUnit::Area),
        _ => None,
    }
}

/// Whether the rule states the parameter (a boolean, or its default,
/// true).
fn declared(plan: &Plan<'_>, parameter: &str) -> bool {
    !matches!(
        plan.constants.get(parameter),
        None | Some(Constant::Scalar(ScalarValue::Boolean { value: false }))
    )
}

/// Whether `applies` holds for the rule.
fn applies(plan: &Plan<'_>, applies: &Applies) -> bool {
    applies.when.iter().all(|name| declared(plan, name))
        && (applies.any.is_empty() || applies.any.iter().any(|name| declared(plan, name)))
        && super::holds(plan, &Read::default(), applies.condition)
}

/// A number in `unit`, as the capabilities showed one.
fn point(value: f64, unit: ItemUnit) -> String {
    match unit {
        ItemUnit::Length => metres(value),
        ItemUnit::Degrees => format!("{}°", (value.to_degrees() * 1e4).round() / 1e4),
        ItemUnit::Ratio => format!("{}", (value * 1e6).round() / 1e6),
        #[allow(clippy::cast_possible_truncation)]
        ItemUnit::Count => format!("{}", value.round() as i64),
        ItemUnit::Area => format!("{}", (value * 1e4).round() / 1e4),
    }
}

/// An interval in `unit`: one number where both ends show alike.
fn show((lower, upper): Span, unit: ItemUnit) -> String {
    if unit == ItemUnit::Length {
        return shown(lower, upper);
    }
    let (low, high) = (point(lower, unit), point(upper, unit));
    if low == high {
        low
    } else {
        format!("between {low} and {high}")
    }
}

/// Bounds as the capabilities worded them: `0.15 m to 0.19 m`, `at least
/// 0.26 m`, `at most 3`.
fn bound_words(minimum: Option<f64>, maximum: Option<f64>, unit: ItemUnit) -> String {
    match (minimum, maximum) {
        (Some(minimum), Some(maximum)) => {
            format!("{} to {}", point(minimum, unit), point(maximum, unit))
        }
        (Some(minimum), None) => format!("at least {}", point(minimum, unit)),
        (None, Some(maximum)) => format!("at most {}", point(maximum, unit)),
        (None, None) => String::new(),
    }
}

/// The bound a range failed or straddled, its number as declared, as a
/// range judge words it (`at least 4`): the lower bound where the value
/// fails below it or reaches below it, the upper otherwise; the lenient end
/// of a bound that is an interval.
fn plain_bound(
    (lower, _): Span,
    minimum: Option<Span>,
    maximum: Option<Span>,
    below: Option<bool>,
) -> String {
    let below = below.unwrap_or_else(|| minimum.is_some_and(|(low, _)| lower < low));
    match (minimum, maximum) {
        (Some((low, _)), _) if below || maximum.is_none() => format!("at least {low}"),
        (_, Some((_, high))) => format!("at most {high}"),
        _ => String::new(),
    }
}

/// Eight units in the last place of a magnitude (at least one): the
/// binary rounding of decimal coordinates, as the capabilities allowed it.
fn slack(scale: f64) -> f64 {
    8.0 * f64::EPSILON * scale.abs().max(1.0)
}

/// An operand's interval for an item: a stated parameter, or the item's
/// number.
fn operand(plan: &Plan<'_>, member: Option<&MeasuredMember>, operand: Operand) -> Option<Span> {
    match operand {
        Operand::Parameter(name) => plan
            .constants
            .get(name)
            .and_then(Constant::number)
            .map(|value| (value, value)),
        Operand::Value(name) => match member.map(|member| field(member, name)) {
            Some(Field::Number(span)) => Some(span),
            _ => None,
        },
    }
}

/// The allowance a bound is widened by, over `members` (one, or every
/// item judged together: the largest).
fn allowance(plan: &Plan<'_>, members: &[&MeasuredMember], allowance: Allowance) -> f64 {
    match allowance {
        // A raised value is moved, never its bound.
        Allowance::None | Allowance::Raised { .. } => 0.0,
        Allowance::Fixed { value } => value,
        Allowance::Slack { times, magnitude } => {
            let end = |span: Span| match magnitude.end {
                End::Lower => span.0,
                End::Upper => span.1,
            };
            let scale = match magnitude.operand {
                Operand::Parameter(_) => operand(plan, None, magnitude.operand).map_or(0.0, end),
                Operand::Value(_) => members
                    .iter()
                    .filter_map(|member| operand(plan, Some(member), magnitude.operand))
                    .map(end)
                    .fold(0.0_f64, |most, value| most.max(value.abs())),
            };
            times * slack(scale)
        }
        Allowance::Stated { times, value } => {
            times
                * members
                    .iter()
                    .filter_map(|member| match field(member, value) {
                        Field::Number((_, upper)) => Some(upper),
                        _ => None,
                    })
                    .fold(0.0_f64, f64::max)
        }
    }
}

/// The requirements' bound for an item: the largest (`largest`) or
/// smallest of the operands each chose, as an interval, and the operands
/// chosen by their names.
fn bound(
    scope: &Scope<'_, '_, '_>,
    requirements: &[Requirement],
    largest: bool,
) -> (Option<Span>, Vec<(&'static str, Span)>) {
    let mut chosen = Vec::new();
    let mut bound: Option<Span> = None;
    for requirement in requirements {
        let picked = requirement.options.iter().find_map(|choice| {
            if !scope.holds(&choice.when) {
                return None;
            }
            match choice.bound {
                Bound::Operand(bound) => operand(scope.plan, scope.member, bound),
                Bound::Literal(value) => Some((value, value)),
            }
        });
        let Some(span) = picked else {
            continue;
        };
        chosen.push((requirement.name, span));
        bound = Some(match bound {
            None => span,
            Some((low, high)) if largest => (low.max(span.0), high.max(span.1)),
            Some((low, high)) => (low.min(span.0), high.min(span.1)),
        });
    }
    (bound, chosen)
}

/// What a range judgement came to.
enum Ranged {
    Pass,
    Fail {
        below: bool,
        deviation: Option<Deviation>,
    },
    Undecided,
}

/// A value against bounds widened by `allowance`: a point bound decides
/// as the capabilities' `judge` did; an interval bound fails where the
/// value fails its most lenient end and passes where it passes its
/// strictest.
fn ranged(
    (lower, upper): Span,
    minimum: Option<Span>,
    maximum: Option<Span>,
    allowance: f64,
) -> Ranged {
    // A bound known as one number decides as the capabilities' `judge`.
    #[allow(clippy::float_cmp)]
    let points = minimum.is_none_or(|(low, high)| low == high)
        && maximum.is_none_or(|(low, high)| low == high);
    if points {
        let (minimum, maximum) = (minimum.map(|span| span.0), maximum.map(|span| span.0));
        return match judge(
            lower,
            upper,
            minimum.map(|bound| bound - allowance),
            maximum.map(|bound| bound + allowance),
        ) {
            Verdict::Pass => Ranged::Pass,
            Verdict::Fail(_) => Ranged::Fail {
                below: minimum.is_some_and(|bound| upper < bound - allowance),
                deviation: deviation(lower, upper, minimum, maximum),
            },
            Verdict::Undecided(_) => Ranged::Undecided,
        };
    }
    // The most lenient bounds, then the strictest.
    let lenient = judge(
        lower,
        upper,
        minimum.map(|span| span.0 - allowance),
        maximum.map(|span| span.1 + allowance),
    );
    if let Verdict::Fail(_) = lenient {
        let below = minimum.is_some_and(|span| upper < span.0 - allowance);
        let deviation = if below {
            minimum.and_then(|(low, high)| {
                Deviation::try_new(
                    Deviation::below(low, lower, upper).lower(),
                    Deviation::below(high, lower, upper).upper(),
                )
            })
        } else {
            maximum.and_then(|(low, high)| {
                Deviation::try_new(
                    Deviation::above(high, lower, upper).lower(),
                    Deviation::above(low, lower, upper).upper(),
                )
            })
        };
        return Ranged::Fail { below, deviation };
    }
    match judge(
        lower,
        upper,
        minimum.map(|span| span.1 - allowance),
        maximum.map(|span| span.0 + allowance),
    ) {
        Verdict::Pass => Ranged::Pass,
        _ => Ranged::Undecided,
    }
}

/// A value against bounds, raised by `raised` against the lower one and
/// lowered by it against the upper one ([`Allowance::Raised`]): failing
/// either fails, straddling either is open.
fn raised_range(
    (lower, upper): Span,
    minimum: Option<Span>,
    maximum: Option<Span>,
    raised: f64,
) -> Ranged {
    let at_least =
        minimum.map(|bound| ranged((lower + raised, upper + raised), Some(bound), None, 0.0));
    let at_most =
        maximum.map(|bound| ranged((lower - raised, upper - raised), None, Some(bound), 0.0));
    let mut verdict = Ranged::Pass;
    for judged in [at_least, at_most].into_iter().flatten() {
        match judged {
            Ranged::Fail { .. } => return judged,
            Ranged::Undecided => verdict = Ranged::Undecided,
            Ranged::Pass => {}
        }
    }
    verdict
}

/// The worse of two optional deviations.
fn worse(one: Option<Deviation>, other: Option<Deviation>) -> Option<Deviation> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.worst(other)),
        (one, other) => one.or(other),
    }
}

/// The evidence an item's outcome cites: the form's, the list's, and the
/// item's own exactness.
fn cited(scope: &Scope<'_, '_, '_>, listed: &[Evidence], object: &Object) -> Vec<Evidence> {
    let mut evidence = scope.read.evidence.clone();
    evidence.extend(listed.iter().cloned());
    if let Some(member) = scope.member {
        evidence.extend(member.evidence.iter().cloned());
        evidence.push(Evidence {
            source: object.id.source.clone(),
            locator: format!(
                "{}#{}",
                scope.items.list,
                scope.place.map_or(0, |(index, _)| index)
            ),
            exact: member.exact,
        });
    }
    evidence
}

fn open(message: String) -> Outcome {
    Outcome::Open(NotEvaluatedReason::IncompleteEvidence, message)
}

/// The objects an item's objects field names.
fn objects(member: Option<&MeasuredMember>, name: Option<&str>) -> Vec<ObjectId> {
    match (member, name) {
        (Some(member), Some(name)) => match field(member, name) {
            Field::Objects(objects) => objects.to_vec(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The first effect turning `on` open for the item, worded.
fn effect(scope: &Scope<'_, '_, '_>, effects: &[Effect], on: &[On]) -> Option<Outcome> {
    effects
        .iter()
        .find(|effect| on.contains(&effect.on) && scope.holds(&effect.when))
        .map(|effect| open(scope.render(effect.message)))
}

/// Judges `items` of `object`: the outcomes of every item's tests, or of
/// the items together.
#[allow(clippy::too_many_lines)]
pub(super) fn judge_items(
    plan: &Plan<'_>,
    items: &Items,
    read: &Read,
    quiet: bool,
    object: &Object,
    leaves: &ObjectLeaves<'_>,
) -> Vec<Outcome> {
    if !applies(plan, &items.applies) {
        return Vec::new();
    }
    let scope = Scope {
        plan,
        items,
        read,
        leaves,
        member: None,
        place: None,
        named: Vec::new(),
        chosen: Vec::new(),
        unit: ItemUnit::Length,
    };
    let list = leaves.written_list(items.list, || {
        super::unstated_dropped_list(items.list, &plan.constants)
    });
    let listed = leaves.bound_members(&list);
    let (members, evidence) = match listed.as_ref() {
        Ok((members, evidence)) => (members, evidence),
        Err((reason, why)) => {
            return match items.refused {
                // The object is open already: a refusal adds nothing.
                Some(_) if quiet => Vec::new(),
                Some(message) => {
                    let mut scope = scope;
                    scope.named("why", why.clone());
                    vec![Outcome::Open(reason.clone(), scope.render(message))]
                }
                None => Vec::new(),
            };
        }
    };
    if let Some(together) = &items.together {
        let present: Vec<&MeasuredMember> = members
            .iter()
            .filter(|member| {
                together
                    .present
                    .is_none_or(|name| !matches!(field(member, name), Field::Null))
                    && scope.with(Some(member)).holds(&together.when)
            })
            .collect();
        return vec![match &together.judge {
            TogetherJudge::Every(every) => {
                judge_every(&scope, every, together.name, &present, evidence, object)
            }
            TogetherJudge::Truths(truths) => {
                judge_truths(&scope, truths, together.name, &present, evidence, object)
            }
            TogetherJudge::Spread(spread) => {
                judge_spread(&scope, spread, &present, evidence, object)
            }
            TogetherJudge::Count(count) => judge_count(&scope, count, &present, evidence, object),
            TogetherJudge::Least(least) => judge_least(&scope, least, &present, evidence, object),
            TogetherJudge::Any(any) => judge_any(&scope, any, &present, evidence, object),
        }];
    }
    let mut outcomes = Vec::new();
    // The key of each item's outcomes, for grouped passing.
    let mut keyed: Vec<(Option<f64>, bool)> = Vec::new();
    let count = members.len();
    let key = items
        .passing
        .as_ref()
        .and_then(|passing| passing.groups.as_ref())
        .map(|groups| groups.key);
    // Where each item's outcomes lie, for combining them.
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(count);
    for (index, member) in members.iter().enumerate() {
        let mut item = scope.with(Some(member));
        item.place = Some((index + 1, count));
        let before = outcomes.len();
        for check in &items.checks {
            check_item(&item, check, evidence, object, &mut outcomes);
        }
        // An item stating why it is open: its open outcomes give that
        // reason.
        if let Some(reason) = items.reason.and_then(|name| stated_reason(member, name)) {
            for outcome in &mut outcomes[before..] {
                if let Outcome::Open(open, _) = outcome {
                    *open = reason.clone();
                }
            }
        }
        // An item's outcomes on the object it names, where it names one.
        if let Some(at) = items.at
            && let Field::Objects([placed, ..]) = field(member, at)
        {
            for outcome in &mut outcomes[before..] {
                if !matches!(outcome, Outcome::Passed) {
                    let held = std::mem::replace(outcome, Outcome::Passed);
                    *outcome = Outcome::Placed(placed.clone(), Box::new(held));
                }
            }
        }
        let key = key.and_then(|key| match field(member, key) {
            Field::Number((low, _)) => Some(low),
            _ => None,
        });
        let failed = outcomes[before..]
            .iter()
            .any(|outcome| !matches!(outcome, Outcome::Passed));
        keyed.push((key, failed));
        ranges.push((before, outcomes.len()));
    }
    if let Some(combined) = &items.combined {
        return combine(&scope, combined, members, &ranges, outcomes);
    }
    if items.merged {
        outcomes = merged(outcomes);
    }
    if let Some(passing) = &items.passing {
        match &passing.groups {
            None => {
                if outcomes
                    .iter()
                    .all(|outcome| matches!(outcome, Outcome::Passed))
                    && scope.holds(&passing.when)
                {
                    outcomes.push(open(scope.render(passing.message)));
                }
            }
            Some(groups) => {
                let list = super::unstated_dropped_list(groups.list, &plan.constants);
                if let Ok((groups_listed, _)) = leaves.bound_members(&list).as_ref() {
                    for group in groups_listed {
                        let Field::Number((key, _)) = field(group, groups.key) else {
                            continue;
                        };
                        let failed = keyed
                            .iter()
                            .any(|(item, failed)| *item == Some(key) && *failed);
                        let worded = scope.with(Some(group));
                        if !failed && worded.holds(&passing.when) {
                            outcomes.push(open(worded.render(passing.message)));
                        }
                    }
                }
            }
        }
    }
    outcomes
}

/// The reason the item's text field `name` states, as a report writes it
/// (`missing_service`); none where it states none or another word.
fn stated_reason(member: &MeasuredMember, name: &str) -> Option<NotEvaluatedReason> {
    match field(member, name) {
        Field::Text(text) => {
            serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
        }
        _ => None,
    }
}

/// The items' outcomes as one ([`Combined`]): `outcomes` lie item by item
/// in `ranges`.
fn combine(
    scope: &Scope<'_, '_, '_>,
    combined: &Combined,
    members: &[MeasuredMember],
    ranges: &[(usize, usize)],
    outcomes: Vec<Outcome>,
) -> Vec<Outcome> {
    let mut per_item: Vec<Vec<Outcome>> = ranges.iter().map(|_| Vec::new()).collect();
    for (index, outcome) in outcomes.into_iter().enumerate() {
        if let Some(item) = ranges
            .iter()
            .position(|(from, to)| (*from..*to).contains(&index))
        {
            per_item[item].push(outcome);
        }
    }
    let kept: Vec<Outcome> = match &combined.alternatives {
        None => per_item.into_iter().flatten().collect(),
        Some(alternatives) => alternative(scope, alternatives, members, per_item),
    };
    let mut messages = Vec::new();
    let mut cited: Vec<Evidence> = Vec::new();
    let mut related: Vec<ObjectId> = Vec::new();
    let mut worst: Option<Deviation> = None;
    let mut opened: Vec<(NotEvaluatedReason, String)> = Vec::new();
    for outcome in kept {
        match outcome {
            Outcome::Finding {
                message,
                evidence,
                related: objects,
                deviation,
                ..
            } => {
                messages.push(message);
                cited.extend(evidence);
                related.extend(objects);
                worst = match (worst, deviation) {
                    (Some(worst), Some(missed)) => Some(worst.worst(missed)),
                    (worst, missed) => worst.or(missed),
                };
            }
            Outcome::Open(reason, message) => opened.push((reason, message)),
            Outcome::Passed | Outcome::Placed(..) => {}
        }
    }
    // Worded over the first item: what every item shares (its row).
    let mut whole = scope.with(members.first());
    if !messages.is_empty() {
        whole.named("findings", messages.join(combined.separator));
        related.sort();
        related.dedup();
        return vec![Outcome::Finding {
            message: whole.render(combined.fail),
            evidence: cited,
            related,
            deviation: worst,
            severity: None,
        }];
    }
    let Some((reason, _)) = opened.first() else {
        return Vec::new();
    };
    let reason = reason.clone();
    whole.named(
        "opens",
        opened
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>()
            .join(combined.separator),
    );
    vec![Outcome::Open(reason, whole.render(combined.open))]
}

/// Whether `outcomes` hold a finding.
fn fails(outcomes: &[Outcome]) -> bool {
    outcomes
        .iter()
        .any(|outcome| matches!(outcome, Outcome::Finding { .. }))
}

/// Whether `outcomes` hold an open outcome.
fn opens(outcomes: &[Outcome]) -> bool {
    outcomes
        .iter()
        .any(|outcome| matches!(outcome, Outcome::Open(..)))
}

/// The outcomes each group of items keeps as alternatives
/// ([`Alternatives`]), `per_item` the items' outcomes in order.
fn alternative(
    scope: &Scope<'_, '_, '_>,
    alternatives: &Alternatives,
    members: &[MeasuredMember],
    mut per_item: Vec<Vec<Outcome>>,
) -> Vec<Outcome> {
    // The groups in the order their first items come.
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let key = match field(member, alternatives.group) {
            Field::Text(text) => text,
            _ => "",
        };
        match groups.iter_mut().find(|(group, _)| *group == key) {
            Some((_, items)) => items.push(index),
            None => groups.push((key, vec![index])),
        }
    }
    let mut kept = Vec::new();
    for (_, group) in groups {
        let sure: Vec<usize> = group
            .iter()
            .copied()
            .filter(|index| {
                matches!(
                    field(&members[*index], alternatives.sure),
                    Field::Truth(true)
                )
            })
            .collect();
        let sure_failing: Vec<usize> = sure
            .iter()
            .copied()
            .filter(|index| fails(&per_item[*index]))
            .collect();
        let every_fails = sure.is_empty()
            && group
                .iter()
                .all(|index| fails(&per_item[*index]) && !opens(&per_item[*index]));
        if !sure_failing.is_empty() || every_fails {
            let chosen = if sure_failing.is_empty() {
                group
            } else {
                sure_failing
            };
            for index in chosen {
                kept.extend(
                    std::mem::take(&mut per_item[index])
                        .into_iter()
                        .filter(|outcome| matches!(outcome, Outcome::Finding { .. })),
                );
            }
        } else if group
            .iter()
            .any(|index| fails(&per_item[*index]) || opens(&per_item[*index]))
        {
            let mut open_ones = Vec::new();
            for index in &group {
                open_ones.extend(
                    std::mem::take(&mut per_item[*index])
                        .into_iter()
                        .filter(|outcome| matches!(outcome, Outcome::Open(..))),
                );
            }
            if open_ones.is_empty() {
                open_ones.push(open(
                    scope
                        .with(Some(&members[group[0]]))
                        .render(alternatives.open),
                ));
            }
            kept.extend(open_ones);
        }
    }
    kept
}

/// `outcomes` with the findings worded alike made one, in the place of the
/// first: relating the objects each related, sorted and each once, citing
/// what each cited, graded by the worst.
fn merged(outcomes: Vec<Outcome>) -> Vec<Outcome> {
    let mut merged: Vec<Outcome> = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let Outcome::Finding {
            message,
            evidence,
            related,
            deviation,
            severity,
        } = outcome
        else {
            merged.push(outcome);
            continue;
        };
        let alike = merged.iter_mut().find_map(|kept| match kept {
            Outcome::Finding {
                message: kept_message,
                evidence: kept_evidence,
                related: kept_related,
                deviation: kept_deviation,
                ..
            } if *kept_message == message => Some((kept_evidence, kept_related, kept_deviation)),
            _ => None,
        });
        if let Some((kept_evidence, kept_related, kept_deviation)) = alike {
            {
                for cited in evidence {
                    if !kept_evidence.contains(&cited) {
                        kept_evidence.push(cited);
                    }
                }
                kept_related.extend(related);
                kept_related.sort();
                kept_related.dedup();
                *kept_deviation = worse(kept_deviation.take(), deviation);
            }
        } else {
            {
                let mut related = related;
                related.sort();
                related.dedup();
                merged.push(Outcome::Finding {
                    message,
                    evidence,
                    related,
                    deviation,
                    severity,
                });
            }
        }
    }
    merged
}

fn check_item(
    scope: &Scope<'_, '_, '_>,
    check: &ItemCheck,
    listed: &[Evidence],
    object: &Object,
    outcomes: &mut Vec<Outcome>,
) {
    match check {
        ItemCheck::Group(group) => check_group(scope, group, listed, object, outcomes),
        ItemCheck::Test(test) => {
            if applies(scope.plan, &test.applies) && scope.holds(&test.when) {
                outcomes.push(test_item(scope, test, listed, object));
            }
        }
    }
}

fn check_group(
    scope: &Scope<'_, '_, '_>,
    group: &Group,
    listed: &[Evidence],
    object: &Object,
    outcomes: &mut Vec<Outcome>,
) {
    if !applies(scope.plan, &group.applies) || !scope.holds(&group.when) {
        return;
    }
    for guard in &group.guards {
        if !scope.holds(&guard.when) {
            continue;
        }
        if let Stop::Here(outcome) = guarded(scope, guard, listed, object) {
            outcomes.extend(outcome);
            return;
        }
    }
    for check in &group.checks {
        check_item(scope, check, listed, object, outcomes);
    }
}

/// What a guard or a `null` makes of an item: judged on, or no further,
/// with an outcome or none.
enum Stop {
    On,
    Here(Option<Outcome>),
}

/// What a guard stops the group with.
fn guarded(scope: &Scope<'_, '_, '_>, guard: &Guard, listed: &[Evidence], object: &Object) -> Stop {
    let Some(member) = scope.member else {
        return Stop::On;
    };
    match field(member, guard.field) {
        Field::Undecided(why) => {
            let message = match guard.undecided {
                Some(message) => {
                    let mut worded = scope.with(Some(member));
                    worded.named("why", why.to_owned());
                    worded.render(message)
                }
                None => why.to_owned(),
            };
            Stop::Here(Some(open(message)))
        }
        Field::Missing => Stop::Here(Some(Outcome::Open(
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "the members of `{}` state no `{}`",
                scope.items.list, guard.field
            ),
        ))),
        Field::Null => null(scope, guard.null, listed, object),
        _ => Stop::On,
    }
}

/// What a `null` comes to.
fn null(scope: &Scope<'_, '_, '_>, on: OnNull, listed: &[Evidence], object: &Object) -> Stop {
    match on {
        OnNull::Judge | OnNull::Unmet => Stop::On,
        OnNull::Skip => Stop::Here(None),
        OnNull::Open(message) => Stop::Here(Some(open(scope.render(message)))),
        OnNull::Fail(message) => Stop::Here(Some(Outcome::Finding {
            severity: None,
            message: scope.render(message),
            evidence: cited(scope, listed, object),
            related: Vec::new(),
            deviation: None,
        })),
    }
}

/// One test of one item: its finding, opening or pass, or a further
/// test in its place.
#[allow(clippy::too_many_lines)]
fn test_item(
    scope: &Scope<'_, '_, '_>,
    test: &ItemTest,
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let Some(member) = scope.member else {
        return Outcome::Passed;
    };
    let mut scope = scope.with(Some(member));
    // A further test in this one's place, where it applies.
    let instead = |scope: &Scope<'_, '_, '_>, next: Option<&ItemTest>| {
        next.filter(|next| applies(scope.plan, &next.applies) && scope.holds(&next.when))
            .map(|next| test_item(scope, next, listed, object))
    };
    // A finding worded by `message`, or what an effect or a further test
    // makes of it; its words are `{failed}` to an effect.
    let worded =
        |scope: &Scope<'_, '_, '_>, message: &str, deviation: Option<Deviation>, below: bool| {
            let on: &[On] = if below {
                &[On::Fail, On::FailBelow]
            } else {
                &[On::Fail]
            };
            let message = scope.render(message);
            let mut failed = scope.with(scope.member);
            failed.chosen.clone_from(&scope.chosen);
            failed.named.clone_from(&scope.named);
            failed.unit = scope.unit;
            failed.named("failed", message.clone());
            if let Some(outcome) = effect(&failed, &test.effects, on) {
                return outcome;
            }
            if let Some(outcome) = instead(scope, test.otherwise.as_deref()) {
                return outcome;
            }
            Outcome::Finding {
                severity: None,
                message,
                evidence: cited(scope, listed, object),
                related: objects(scope.member, test.related),
                deviation,
            }
        };
    let finding = |scope: &Scope<'_, '_, '_>, deviation: Option<Deviation>, below: bool| {
        worded(scope, test.fail, deviation, below)
    };
    let passed = |scope: &Scope<'_, '_, '_>| {
        if let Some(outcome) = effect(scope, &test.effects, &[On::Pass]) {
            return outcome;
        }
        instead(scope, test.then.as_deref()).unwrap_or(Outcome::Passed)
    };
    let straddles = |scope: &Scope<'_, '_, '_>| {
        instead(scope, test.straddled.as_deref())
            .unwrap_or_else(|| open(scope.render(test.undecided)))
    };
    match &test.judge {
        Judge::Truth {
            value,
            finding: fails,
        } => match field(member, value) {
            Field::Truth(held) if held == *fails => finding(&scope, None, false),
            Field::Truth(_) => passed(&scope),
            Field::Null => effect(&scope, &test.effects, &[On::Null]).unwrap_or(Outcome::Passed),
            Field::Undecided(why) => {
                scope.named("why", why.to_owned());
                straddles(&scope)
            }
            _ => Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "`{value}` of the members of `{}` is no truth",
                    scope.items.list
                ),
            ),
        },
        Judge::Range(range) => {
            scope.unit = range.unit;
            let (minimum, mut chosen) = bound(&scope, &range.at_least, true);
            let (maximum, more) = bound(&scope, &range.at_most, false);
            chosen.extend(more);
            let words: Vec<&'static str> = range
                .at_least
                .iter()
                .chain(&range.at_most)
                .filter(|requirement| chosen.iter().any(|(name, _)| *name == requirement.name))
                .map(|requirement| requirement.words)
                .collect();
            scope.chosen = chosen;
            let requirements = words
                .iter()
                .map(|words| scope.render(words))
                .collect::<Vec<_>>()
                .join(" and ");
            scope.named("requirements", requirements);
            scope.named(
                "bound",
                bound_words(
                    minimum.map(|span| span.0),
                    maximum.map(|span| span.0),
                    range.unit,
                ),
            );
            let value = match field(member, range.value) {
                Field::Number(span) => span,
                Field::Null => {
                    if let Some(outcome) = effect(&scope, &test.effects, &[On::Null]) {
                        return outcome;
                    }
                    return match range.null {
                        // A value that does not reach the bound misses only
                        // a lower bound the rule states.
                        OnNull::Fail(_) if minimum.is_none() => Outcome::Passed,
                        OnNull::Fail(message) => worded(&scope, message, None, true),
                        OnNull::Unmet => worded(&scope, test.fail, None, false),
                        on => match null(&scope, on, listed, object) {
                            Stop::Here(Some(outcome)) => outcome,
                            _ => Outcome::Passed,
                        },
                    };
                }
                Field::Undecided(why) => {
                    scope.named("why", why.to_owned());
                    return open(scope.render(range.unmeasured.unwrap_or("{why}")));
                }
                _ => {
                    return Outcome::Open(
                        NotEvaluatedReason::InvalidEvidence,
                        format!(
                            "`{}` of the members of `{}` is no number",
                            range.value, scope.items.list
                        ),
                    );
                }
            };
            let verdict = match range.allowance {
                Allowance::Raised { value: raised } => {
                    raised_range(value, minimum, maximum, raised)
                }
                widened => ranged(
                    value,
                    minimum,
                    maximum,
                    allowance(scope.plan, &[member], widened),
                ),
            };
            match verdict {
                Ranged::Pass => passed(&scope),
                Ranged::Fail { below, deviation } => {
                    scope.named(
                        "bound:plain",
                        plain_bound(value, minimum, maximum, Some(below)),
                    );
                    let graded = scope.plan.template.grades && range.grade;
                    finding(&scope, deviation.filter(|_| graded), below)
                }
                Ranged::Undecided => {
                    scope.named("bound:plain", plain_bound(value, minimum, maximum, None));
                    straddles(&scope)
                }
            }
        }
        Judge::Rows(rows) => judge_rows(&scope, rows, test, listed, object),
        Judge::Fails => finding(&scope, None, false),
    }
}

/// A table row's number in a column, in coherent units.
fn cell(row: &axioval_ir::contract::TableRow, column: &str) -> Option<f64> {
    match row.get(column)? {
        ParameterValue::Number { value } => Some(*value),
        #[allow(clippy::cast_precision_loss)]
        ParameterValue::Integer { value } => Some(*value as f64),
        ParameterValue::Quantity { value, unit } => crate::support::si_quantity(*value, unit)
            .ok()
            .map(|(value, _)| value),
        _ => None,
    }
}

/// Some row of the table holds for the item.
fn judge_rows(
    scope: &Scope<'_, '_, '_>,
    rows: &Rows,
    test: &ItemTest,
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let Some(Constant::Other(ParameterValue::Table { value: table })) =
        scope.plan.constants.get(rows.table)
    else {
        return Outcome::Passed;
    };
    if table.is_empty() {
        return Outcome::Passed;
    }
    let Some(member) = scope.member else {
        return Outcome::Passed;
    };
    let mut verdicts = Vec::new();
    let mut nearest: Option<Option<Deviation>> = None;
    let mut words = Vec::new();
    for row in table {
        let mut described = String::new();
        let mut row_verdicts = Vec::new();
        let mut missed: Option<Deviation> = None;
        for column in &rows.columns {
            let Some(limit) = cell(row, column.column) else {
                continue;
            };
            described.push_str(&column.words.replace("{}", &point(limit, column.unit)));
            let Field::Number((lower, upper)) = field(member, column.value) else {
                return Outcome::Open(
                    NotEvaluatedReason::InvalidEvidence,
                    format!(
                        "`{}` of the members of `{}` is no number",
                        column.value, scope.items.list
                    ),
                );
            };
            let allowance = allowance(scope.plan, &[member], column.allowance);
            row_verdicts.push(judge(lower, upper, None, Some(limit + allowance)));
            missed = worse(missed, deviation(lower, upper, None, Some(limit)));
        }
        words.push(described);
        let fails = row_verdicts
            .iter()
            .any(|verdict| matches!(verdict, Verdict::Fail(_)));
        let holds = row_verdicts
            .iter()
            .all(|verdict| matches!(verdict, Verdict::Pass));
        verdicts.push(if fails {
            Some(false)
        } else if holds {
            Some(true)
        } else {
            None
        });
        // Any row would do, so a failing item misses by as little as the
        // nearest row; a row failing only within its allowance grades
        // nothing.
        nearest = Some(match (nearest, missed) {
            (None, missed) => missed,
            (Some(Some(nearest)), Some(missed)) => Some(nearest.least(missed)),
            (Some(_), _) => None,
        });
    }
    if verdicts.contains(&Some(true)) {
        return Outcome::Passed;
    }
    let mut scope = scope.with(Some(member));
    scope.named("rows", words.join(rows.joiner));
    if verdicts.iter().all(|verdict| *verdict == Some(false)) {
        let graded = scope.plan.template.grades;
        Outcome::Finding {
            severity: None,
            message: scope.render(test.fail),
            evidence: cited(&scope, listed, object),
            related: objects(scope.member, test.related),
            deviation: nearest.flatten().filter(|_| graded),
        }
    } else {
        open(scope.render(test.undecided))
    }
}

/// Every item's number within the range, in one outcome.
#[allow(clippy::too_many_lines)]
fn judge_every(
    scope: &Scope<'_, '_, '_>,
    every: &Every,
    name: &str,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let range = &every.range;
    let mut scope = scope.with(None);
    scope.unit = range.unit;
    let count = present.len();
    let allowance = allowance(scope.plan, present, range.allowance);
    let (minimum, _) = bound(&scope, &range.at_least, true);
    let (maximum, _) = bound(&scope, &range.at_most, false);
    let (minimum, maximum) = (minimum.map(|span| span.0), maximum.map(|span| span.0));
    let (low, high) = (
        minimum.map(|bound| bound - allowance),
        maximum.map(|bound| bound + allowance),
    );
    scope.named("bound", bound_words(minimum, maximum, range.unit));
    let mut failing = Vec::new();
    let mut missed = None;
    let mut straddling = Vec::new();
    let mut unmeasured = Vec::new();
    // Open items in item order: (straddling, worded).
    let mut open_items: Vec<String> = Vec::new();
    for (index, member) in present.iter().enumerate() {
        let mut item = scope.with(Some(member));
        item.place = Some((index + 1, count));
        item.unit = range.unit;
        let named = item.render(name);
        item.named("name", named.clone());
        let span = match field(member, range.value) {
            Field::Number(span) => span,
            Field::Undecided(_) | Field::Null => {
                if let Some(message) = every.unmeasured_any {
                    return open(scope.render(message));
                }
                unmeasured.push(named.clone());
                if let OpenItems::Each { unmeasured, .. } = &every.open {
                    open_items.push(item.render(unmeasured));
                }
                continue;
            }
            _ => {
                return Outcome::Open(
                    NotEvaluatedReason::InvalidEvidence,
                    format!(
                        "`{}` of the members of `{}` is no number",
                        range.value, scope.items.list
                    ),
                );
            }
        };
        let (lower, upper) = span;
        let verdict = match every.zero {
            Some(zero) => {
                if upper <= zero || low.is_none_or(|low| lower >= low) {
                    Verdict::Pass
                } else if lower > zero && low.is_some_and(|low| upper < low) {
                    Verdict::Fail(String::new())
                } else {
                    Verdict::Undecided(String::new())
                }
            }
            None => judge(lower, upper, low, high),
        };
        match verdict {
            Verdict::Pass => {}
            Verdict::Fail(_) => {
                failing.push(item.render(every.item));
                missed = worse(missed, deviation(lower, upper, minimum, maximum));
            }
            Verdict::Undecided(_) => {
                let worded = item.render(every.item);
                if let OpenItems::Each { straddling, .. } = &every.open {
                    item.named("item", worded.clone());
                    open_items.push(item.render(straddling));
                }
                straddling.push(worded);
            }
        }
    }
    if !failing.is_empty() {
        scope.named("failing", failing.join(", "));
        let graded = scope.plan.template.grades && range.grade;
        return Outcome::Finding {
            severity: None,
            message: scope.render(every.fail),
            evidence: cited(&scope, listed, object),
            related: Vec::new(),
            deviation: missed.filter(|_| graded),
        };
    }
    if straddling.is_empty() && unmeasured.is_empty() {
        return Outcome::Passed;
    }
    match &every.open {
        OpenItems::Grouped {
            straddling: straddles,
            unmeasured: unmeasured_words,
        } => {
            let mut parts = Vec::new();
            if !straddling.is_empty() {
                scope.named("items", straddling.join(", "));
                parts.push(scope.render(straddles));
            }
            if !unmeasured.is_empty() {
                scope.named("items", unmeasured.join(", "));
                parts.push(scope.render(unmeasured_words));
            }
            open(parts.join("; "))
        }
        OpenItems::Each { .. } => open(open_items.join("; ")),
    }
}

/// Every item's truth, in one outcome.
fn judge_truths(
    scope: &Scope<'_, '_, '_>,
    truths: &Truths,
    name: &str,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let mut scope = scope.with(None);
    let count = present.len();
    let mut failing = Vec::new();
    let mut unknown = Vec::new();
    for (index, member) in present.iter().enumerate() {
        let mut item = scope.with(Some(member));
        item.place = Some((index + 1, count));
        let named = item.render(name);
        item.named("name", named.clone());
        match field(member, truths.value) {
            Field::Truth(held) if held == truths.finding => failing.push(item.render(truths.item)),
            Field::Truth(_) | Field::Null => {}
            _ => unknown.push(named),
        }
    }
    if !failing.is_empty() {
        scope.named("failing", failing.join(", "));
        return Outcome::Finding {
            severity: None,
            message: scope.render(truths.fail),
            evidence: cited(&scope, listed, object),
            related: Vec::new(),
            deviation: None,
        };
    }
    if unknown.is_empty() {
        return Outcome::Passed;
    }
    scope.named("items", unknown.join(truths.joiner));
    open(scope.render(truths.undecided))
}

/// At least one item meets the requirement: one holding passes; else the
/// first item left open opens the check; else the failing items are one
/// finding.
fn judge_any(
    scope: &Scope<'_, '_, '_>,
    any: &Any,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let mut scope = scope.with(None);
    let count = present.len();
    let mut failing = Vec::new();
    let mut related = Vec::new();
    let mut opened = None;
    for (index, member) in present.iter().enumerate() {
        let mut item = scope.with(Some(member));
        item.place = Some((index + 1, count));
        if item.holds(&any.holds) {
            return Outcome::Passed;
        }
        if opened.is_some() {
            continue;
        }
        if let Some(case) = any.open.iter().find(|case| item.holds(&case.when)) {
            if let Some(Field::Undecided(why)) = case.why.map(|name| field(member, name)) {
                item.named("why", why.to_owned());
            }
            opened = Some(item.render(case.message));
            continue;
        }
        if item.holds(&any.fails) {
            failing.push(item.render(any.item));
            for id in objects(Some(member), any.related) {
                if !related.contains(&id) {
                    related.push(id);
                }
            }
        }
    }
    if let Some(message) = opened {
        return open(message);
    }
    if failing.is_empty() {
        return Outcome::Passed;
    }
    scope.named("failing", failing.join(", "));
    Outcome::Finding {
        message: scope.render(any.fail),
        evidence: cited(&scope, listed, object),
        related,
        deviation: None,
        severity: None,
    }
}

/// The spread of the items' numbers against a tolerance.
fn judge_spread(
    scope: &Scope<'_, '_, '_>,
    spread: &Spread,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let mut scope = scope.with(None);
    scope.unit = spread.unit;
    let mut values = Vec::new();
    for member in present {
        match field(member, spread.value) {
            Field::Number(span) => values.push(span),
            Field::Undecided(why) => return open(why.to_owned()),
            _ => {}
        }
    }
    let Some(tolerance) = operand(scope.plan, None, spread.tolerance).map(|span| span.0) else {
        return Outcome::Passed;
    };
    let (Some(most_low), Some(most_high), Some(least_low), Some(least_high)) = (
        values.iter().map(|span| span.0).reduce(f64::max),
        values.iter().map(|span| span.1).reduce(f64::max),
        values.iter().map(|span| span.0).reduce(f64::min),
        values.iter().map(|span| span.1).reduce(f64::min),
    ) else {
        return Outcome::Passed;
    };
    let lower = (most_low - least_high).next_down().max(0.0);
    let upper = (most_high - least_low).next_up().max(lower);
    let unit = spread.unit;
    let listed_values = values
        .iter()
        .map(|&(low, high)| {
            if point(low, unit) == point(high, unit) {
                point(low, unit)
            } else {
                format!("{}..{}", point(low, unit), point(high, unit))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    scope.named("values", listed_values);
    scope.named("spread", show((lower, upper), unit));
    scope.named("tolerance", point(tolerance, unit));
    let allowance = allowance(scope.plan, present, spread.allowance);
    match judge(lower, upper, None, Some(tolerance + allowance)) {
        Verdict::Pass => Outcome::Passed,
        Verdict::Fail(_) => Outcome::Finding {
            severity: None,
            message: scope.render(spread.fail),
            evidence: cited(&scope, listed, object),
            related: Vec::new(),
            deviation: scope
                .plan
                .template
                .grades
                .then(|| Deviation::above(tolerance, lower, upper)),
        },
        Verdict::Undecided(_) => open(scope.render(spread.undecided)),
    }
}

/// How many items there are, within bounds; never open.
fn judge_count(
    scope: &Scope<'_, '_, '_>,
    count: &Count,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let mut scope = scope.with(None);
    scope.unit = ItemUnit::Count;
    #[allow(clippy::cast_precision_loss)]
    let number = present.len() as f64;
    let (minimum, _) = bound(&scope, &count.at_least, true);
    let (maximum, _) = bound(&scope, &count.at_most, false);
    let (minimum, maximum) = (minimum.map(|span| span.0), maximum.map(|span| span.0));
    let deviation = if minimum.is_some_and(|minimum| number < minimum) {
        minimum.map(|minimum| Deviation::below(minimum, number, number))
    } else if maximum.is_some_and(|maximum| number > maximum) {
        maximum.map(|maximum| Deviation::above(maximum, number, number))
    } else {
        return Outcome::Passed;
    };
    scope.named("count", present.len().to_string());
    scope.named("bound", bound_words(minimum, maximum, ItemUnit::Count));
    Outcome::Finding {
        severity: None,
        message: scope.render(count.fail),
        evidence: cited(&scope, listed, object),
        related: Vec::new(),
        deviation: deviation.filter(|_| scope.plan.template.grades),
    }
}

/// The least of the items' numbers at least a bound.
fn judge_least(
    scope: &Scope<'_, '_, '_>,
    least: &Least,
    present: &[&MeasuredMember],
    listed: &[Evidence],
    object: &Object,
) -> Outcome {
    let mut scope = scope.with(None);
    scope.unit = least.unit;
    let mut unknown: Vec<String> = Vec::new();
    let mut known: Vec<(&MeasuredMember, Span)> = Vec::new();
    for member in present {
        match field(member, least.value) {
            Field::Number(span) => known.push((member, span)),
            Field::Undecided(why) => unknown.push(why.to_owned()),
            _ => {}
        }
    }
    let mut evidence = scope.read.evidence.clone();
    evidence.extend(listed.iter().cloned());
    for (member, _) in &known {
        evidence.push(Evidence {
            source: object.id.source.clone(),
            locator: scope.items.list.to_owned(),
            exact: member.exact,
        });
    }
    scope.named("unknown", unknown.join("; "));
    scope.named(
        "some",
        if unknown.len() == 1 {
            "a width"
        } else {
            "widths"
        }
        .to_owned(),
    );
    // The narrowest is the item of least upper end; the least lies between
    // the least lower and the least upper end.
    let Some((narrowest, (_, upper))) =
        known.iter().copied().min_by(|a, b| a.1.1.total_cmp(&b.1.1))
    else {
        return open(scope.render(least.unmeasured));
    };
    let lower = known
        .iter()
        .map(|(_, span)| span.0)
        .fold(f64::INFINITY, f64::min);
    let (lower, upper) = if lower <= upper.max(lower) {
        (lower, upper.max(lower))
    } else {
        (lower, upper)
    };
    let mut related: Vec<ObjectId> = Vec::new();
    for name in &least.related {
        for found in objects(Some(narrowest), Some(name)) {
            if !related.contains(&found) {
                related.push(found);
            }
        }
    }
    let at = scope.with(Some(narrowest)).render(least.at);
    scope.named("at", at);
    scope.named("least", show((lower, upper), least.unit));
    let Some((minimum, _)) = bound(&scope, std::slice::from_ref(&least.at_least), true).0 else {
        return Outcome::Passed;
    };
    let allowance = least.times * slack(upper);
    match judge(lower, upper, Some(minimum - allowance), None) {
        Verdict::Fail(_) => Outcome::Finding {
            severity: None,
            message: scope.render(least.fail),
            evidence,
            related,
            deviation: scope
                .plan
                .template
                .grades
                .then(|| Deviation::below(minimum, lower, upper)),
        },
        Verdict::Pass => {
            if scope.holds(&least.pending.when) {
                open(scope.render(least.pending.message))
            } else if unknown.is_empty() {
                Outcome::Passed
            } else {
                open(scope.render(least.partial))
            }
        }
        Verdict::Undecided(_) => open(scope.render(least.undecided)),
    }
}

#[cfg(test)]
mod tests {
    use super::{ItemUnit, plain_bound, point, show};

    /// An area is shown as the area capabilities showed one: rounded to
    /// 1e-4, an interval where its ends differ when rounded.
    #[test]
    fn an_area_is_shown_rounded_to_a_ten_thousandth() {
        assert_eq!(point(26.000_04, ItemUnit::Area), "26");
        assert_eq!(point(0.123_456, ItemUnit::Area), "0.1235");
        assert_eq!(show((24.0, 26.0), ItemUnit::Area), "between 24 and 26");
        assert_eq!(show((5.000_01, 5.000_02), ItemUnit::Area), "5");
    }

    /// The plain bound is the one a range judge words: the bound failed or
    /// straddled, its number as declared and without a unit.
    #[test]
    fn the_plain_bound_is_the_one_failed_or_straddled() {
        let four = Some((4.0, 4.0));
        let six = Some((6.25, 6.25));
        assert_eq!(
            plain_bound((3.0, 3.5), four, None, Some(true)),
            "at least 4"
        );
        assert_eq!(
            plain_bound((7.0, 7.0), four, six, Some(false)),
            "at most 6.25"
        );
        // Open: the bound the value reaches past.
        assert_eq!(plain_bound((3.9, 4.1), four, six, None), "at least 4");
        assert_eq!(plain_bound((6.0, 6.5), four, six, None), "at most 6.25");
        assert_eq!(plain_bound((6.0, 6.5), None, six, None), "at most 6.25");
        // A bound that is an interval is worded by its lenient end.
        assert_eq!(
            plain_bound((0.5, 0.6), Some((1.0, 2.0)), None, Some(true)),
            "at least 1"
        );
    }
}

#[cfg(test)]
mod raised_and_merged {
    use super::*;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(axioval_ir::SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    /// A raised value meets a lower bound where the value and the allowance
    /// together reach it, as `value + allowance >= bound` does, and an
    /// upper bound where the value less it stays within.
    #[test]
    fn a_raised_value_is_moved_toward_passing() {
        let step = 1.0e-6;
        let below = 1.0 - 2.0 * step;
        assert!(matches!(
            raised_range((below, below), Some((1.0, 1.0)), None, step),
            Ranged::Fail { .. }
        ));
        let within = 1.0 - step / 2.0;
        assert!(matches!(
            raised_range((within, within), Some((1.0, 1.0)), None, step),
            Ranged::Pass
        ));
        assert!(matches!(
            raised_range(
                (1.0 + step / 2.0, 1.0 + step / 2.0),
                None,
                Some((1.0, 1.0)),
                step
            ),
            Ranged::Pass
        ));
        assert!(matches!(
            raised_range((0.5, 2.0), Some((1.0, 1.0)), None, step),
            Ranged::Undecided
        ));
        // Failing either bound fails.
        assert!(matches!(
            raised_range((3.0, 3.0), Some((1.0, 1.0)), Some((2.0, 2.0)), step),
            Ranged::Fail { .. }
        ));
    }

    /// Findings worded alike are one, relating every object each related,
    /// sorted and each once; other outcomes stay as they are.
    #[test]
    fn findings_worded_alike_merge() {
        let finding = |message: &str, related: &[&str]| Outcome::Finding {
            message: message.to_owned(),
            evidence: Vec::new(),
            related: related.iter().map(|local| id(local)).collect(),
            deviation: None,
            severity: None,
        };
        let merged = merged(vec![
            finding("hole", &["z"]),
            Outcome::Passed,
            finding("missing", &[]),
            finding("hole", &["a", "z"]),
        ]);
        let shown: Vec<(String, Vec<String>)> = merged
            .iter()
            .filter_map(|outcome| match outcome {
                Outcome::Finding {
                    message, related, ..
                } => Some((
                    message.clone(),
                    related.iter().map(|id| id.local_id.clone()).collect(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("hole".to_owned(), vec!["a".to_owned(), "z".to_owned()]),
                ("missing".to_owned(), Vec::new()),
            ]
        );
        assert_eq!(merged.len(), 3);
    }
}
