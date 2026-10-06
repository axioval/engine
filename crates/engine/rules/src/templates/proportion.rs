//! Proportions ([`axioval_engine::template::Decision::Proportion`]): two
//! exact counts judged by a ratio with an exception for small counts, or by
//! a table of steps, in integer arithmetic; per anchor (its two member
//! populations) or per group of the counted objects by a stated value.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::template::{ProportionGroups, ProportionParameters};
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext};
use axioval_ir::contract::{ParameterValue, ScalarValue};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue};

use super::{Constant, Outcome, Plan, Read, read_values, render};
use crate::counts::Population;
use crate::expression_leaves::ObjectLeaves;
use crate::support::{Parameters, Unavailable, finding, invalid, undefined, value_key};

/// An order a ratio's two sides stand in.
#[derive(Clone, Copy, Debug)]
pub(super) enum Operator {
    Equal,
    NotEqual,
    Greater,
    AtLeast,
    Less,
    AtMost,
}

impl Operator {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "equal" => Self::Equal,
            "not_equal" => Self::NotEqual,
            "greater" => Self::Greater,
            "at_least" => Self::AtLeast,
            "less" => Self::Less,
            "at_most" => Self::AtMost,
            _ => return None,
        })
    }

    fn holds(self, left: i128, right: i128) -> bool {
        match self {
            Self::Equal => left == right,
            Self::NotEqual => left != right,
            Self::Greater => left > right,
            Self::AtLeast => left >= right,
            Self::Less => left < right,
            Self::AtMost => left <= right,
        }
    }
}

/// A proportion as the rule states it, checked and bound once.
#[derive(Clone, Debug)]
pub(super) enum Mode {
    Ratio {
        provided_unit: i64,
        required_unit: i64,
        operator: Operator,
        word: String,
        /// Required counts from 1 up to but excluding the first are judged
        /// as `provided operator` the second.
        small: Option<(u64, u64)>,
    },
    Table {
        /// `(required from, provided at least)`, sorted by `required from`.
        rows: Vec<(u64, u64)>,
        /// `(additional required, additional provided)`, both positive.
        increment: Option<(u64, u64)>,
    },
}

impl Mode {
    /// Whether the counts pass, and the requirement as a reviewer reads it;
    /// `None` where the mode sets no requirement for this required count.
    pub(super) fn judge(&self, provided: u64, required: u64) -> Option<(bool, String)> {
        match self {
            Self::Ratio {
                operator,
                word,
                small: Some((below, at_least)),
                ..
            } if (1..*below).contains(&required) => Some((
                operator.holds(i128::from(provided), i128::from(*at_least)),
                format!("{provided} {word} {at_least} for fewer than {below} required"),
            )),
            Self::Ratio {
                provided_unit,
                required_unit,
                operator,
                word,
                ..
            } => Some((
                operator.holds(
                    i128::from(provided) * i128::from(*required_unit),
                    i128::from(required) * i128::from(*provided_unit),
                ),
                format!("{provided}/{provided_unit} {word} {required}/{required_unit}"),
            )),
            Self::Table { rows, increment } => {
                if rows.first().is_some_and(|(first, _)| required < *first) {
                    return None;
                }
                let row = rows.iter().rev().find(|(from, _)| required >= *from);
                let minimum = match (row, increment) {
                    (Some((from, at_least)), Some((step, extra))) if row == rows.last() => {
                        at_least.saturating_add(((required - from) / step).saturating_mul(*extra))
                    }
                    (Some((_, at_least)), _) => *at_least,
                    (None, increment) => {
                        increment.map_or(0, |(step, extra)| (required / step).saturating_mul(extra))
                    }
                };
                Some((
                    provided >= minimum,
                    format!("at least {minimum} provided for {required} required"),
                ))
            }
        }
    }
}

/// The proportion `names` states of `rule`, refused as the capabilities
/// refused it, in their order.
pub(super) fn parse(
    rule: &CompiledRule,
    names: &ProportionParameters,
) -> Result<Mode, Unavailable> {
    let parameters = Parameters(rule);
    let Some(table) = parameters.strings(names.table)? else {
        let unit = |name: &str| match parameters.integer(name)? {
            Some(value) if value > 0 => Ok(value),
            _ => Err(invalid(format!("{name} must be a positive integer"))),
        };
        let word = parameters.required_string(names.operator)?;
        let small = match (
            parameters.integer(names.small_below)?,
            parameters.integer(names.small_provided)?,
        ) {
            (None, None) => None,
            (Some(below), Some(provided)) if below > 0 && provided >= 0 => {
                Some((below.unsigned_abs(), provided.unsigned_abs()))
            }
            (Some(_), Some(_)) => {
                return Err(invalid(format!(
                    "{} must be positive and {} not negative",
                    names.small_below, names.small_provided
                )));
            }
            _ => {
                return Err(invalid(format!(
                    "{} and {} go together",
                    names.small_below, names.small_provided
                )));
            }
        };
        return Ok(Mode::Ratio {
            provided_unit: unit(names.provided_unit)?,
            required_unit: unit(names.required_unit)?,
            operator: Operator::parse(word)
                .ok_or_else(|| invalid(format!("operator `{word}` is unsupported")))?,
            word: word.to_owned(),
            small,
        });
    };
    for ratio in [
        names.provided_unit,
        names.required_unit,
        names.operator,
        names.small_below,
        names.small_provided,
    ] {
        if rule.parameters.contains_key(ratio) {
            return Err(invalid(format!("`{ratio}` does not apply in table mode")));
        }
    }
    let count = |text: &str| {
        let text = text.trim();
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse::<u64>().ok()
    };
    let mut rows = table
        .iter()
        .map(|row| {
            row.split_once(':')
                .and_then(|(from, at_least)| Some((count(from)?, count(at_least)?)))
                .ok_or_else(|| invalid(format!("table row `{row}` is not `required:provided`")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_unstable();
    if rows.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("two table rows start at the same required count"));
    }
    let positive = |name: &str| match parameters.integer(name)? {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value.unsigned_abs())),
        Some(_) => Err(invalid(format!("{name} must be positive"))),
    };
    let increment = match (
        positive(names.additional_required)?,
        positive(names.additional_provided)?,
    ) {
        (Some(step), Some(extra)) => Some((step, extra)),
        (None, None) => None,
        _ => {
            return Err(invalid(format!(
                "{} and {} go together",
                names.additional_required, names.additional_provided
            )));
        }
    };
    if rows.is_empty() && increment.is_none() {
        return Err(invalid("the table needs rows or increments"));
    }
    Ok(Mode::Table { rows, increment })
}

/// The proportion's verdict on an anchor's two counts, read as exact
/// numbers of at least zero: passed, or a finding worded `fail` with
/// `{requirement}`; `None` where a count is no such number.
pub(super) fn judged(
    plan: &Plan<'_>,
    mut read: Read,
    (provided, required): (&str, &str),
    related: Vec<ObjectId>,
) -> Option<Outcome> {
    let mode = plan.proportion.as_ref()?;
    let count = |name: &str| match read.values.get(name) {
        Some(axioval_engine::expression::Value::Number { value, .. })
            if value.lower.to_bits() == value.upper.to_bits()
                && value.lower >= 0.0
                && value.lower.fract() == 0.0 =>
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(value.lower as u64)
        }
        _ => None,
    };
    let (provided, required) = (count(provided)?, count(required)?);
    Some(match mode.judge(provided, required) {
        Some((false, requirement)) => {
            read.named.insert("requirement", requirement);
            Outcome::Finding {
                severity: None,
                message: render(plan, &read, plan.form.fail),
                evidence: read.evidence,
                related,
                deviation: None,
            }
        }
        _ => Outcome::Passed,
    })
}

/// Whether an object belongs to a population the rule counts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Membership {
    Yes,
    Undecided,
    No,
}

impl Membership {
    fn of(id: &ObjectId, selection: &Population, population: &Population) -> Self {
        if selection.matched.contains(id) && population.matched.contains(id) {
            Self::Yes
        } else if selection.contains(id) && population.contains(id) {
            Self::Undecided
        } else {
            Self::No
        }
    }
}

/// One group of counted objects.
#[derive(Default)]
struct Group {
    value: Option<PropertyValue>,
    provided: BTreeSet<ObjectId>,
    required: BTreeSet<ObjectId>,
    undecided: BTreeSet<ObjectId>,
    evidence: Vec<Evidence>,
}

impl Group {
    /// The object a group's outcome is raised against.
    fn representative(&self) -> &ObjectId {
        self.required
            .first()
            .or_else(|| self.provided.first())
            .or_else(|| self.undecided.first())
            .expect("a group has a member")
    }
}

/// A boolean parameter of the plan (or its default), false where unstated.
fn flag(plan: &Plan<'_>, name: &str) -> bool {
    matches!(
        plan.constants.get(name),
        Some(Constant::Scalar(ScalarValue::Boolean { value: true }))
    )
}

/// The population a selector parameter of the plan picks; none where
/// unstated (the declaration requires it).
fn population(plan: &Plan<'_>, context: &RuleContext<'_>, name: &str) -> Population {
    match plan.constants.get(name) {
        Some(Constant::Other(ParameterValue::Selector { value })) => Population::of(context, value),
        _ => Population {
            matched: BTreeSet::new(),
            undecided: BTreeSet::new(),
            first: None,
        },
    }
}

/// Runs a form judging the proportion per group of the counted objects.
#[allow(clippy::too_many_lines)]
pub(super) fn groups(
    plan: &Plan<'_>,
    groups: &ProportionGroups,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let Some(mode) = plan.proportion.as_ref() else {
        return CapabilityEvaluation::default();
    };
    let decision = &plan.form.decision;
    let across = flag(plan, groups.across);
    let case_sensitive = flag(plan, groups.case_sensitive);
    let provided = population(plan, context, groups.provided);
    let required = population(plan, context, groups.required);
    let selection = Population::of(context, &rule.selector);
    let mut evaluation = CapabilityEvaluation::default();
    let mut grouped: BTreeMap<(String, String), Group> = BTreeMap::new();
    // Scopes holding an object whose group value could not be read.
    let mut unreadable: BTreeSet<String> = BTreeSet::new();
    for object in context.project.objects() {
        let as_provided = Membership::of(&object.id, &selection, &provided);
        let as_required = Membership::of(&object.id, &selection, &required);
        if as_provided == Membership::No && as_required == Membership::No {
            continue;
        }
        let scope = if across {
            String::new()
        } else {
            object.id.source.to_string()
        };
        let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
        let mut read = Read::default();
        match read_values(
            plan,
            plan.values(),
            &|name| super::judges_stated(decision, name),
            context,
            object,
            &mut leaves,
            &mut read,
        ) {
            None => {}
            Some(Outcome::Open(reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                unreadable.insert(scope);
                continue;
            }
            // A value read as stated is neither found nor passed.
            Some(_) => continue,
        }
        let stated = read.stated.get(groups.value).cloned().flatten();
        let value = match stated {
            Some(value) if !undefined(Some(&value)) => value,
            value => {
                read.stated.insert(groups.value, value);
                if as_provided == Membership::Yes || as_required == Membership::Yes {
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        render(plan, &read, groups.ungrouped),
                        read.evidence,
                        vec![],
                    ));
                } else {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        render(plan, &read, groups.undecided_ungrouped),
                    );
                }
                continue;
            }
        };
        let key = value_key(&value, true, case_sensitive);
        let group = grouped.entry((scope, key)).or_insert_with(|| Group {
            value: Some(value),
            ..Group::default()
        });
        group.evidence.extend(read.evidence);
        for (membership, members) in [
            (as_provided, &mut group.provided),
            (as_required, &mut group.required),
        ] {
            match membership {
                Membership::Yes => {
                    members.insert(object.id.clone());
                }
                Membership::Undecided => {
                    group.undecided.insert(object.id.clone());
                }
                Membership::No => {}
            }
        }
    }
    for ((scope, _), group) in grouped {
        let representative = group.representative().clone();
        let mut read = Read::default();
        read.stated.insert(groups.value, group.value.clone());
        let label = render(plan, &read, groups.label);
        read.named.insert("label", label);
        read.named
            .insert("undecided", group.undecided.len().to_string());
        read.named
            .insert("provided", group.provided.len().to_string());
        read.named
            .insert("required", group.required.len().to_string());
        if !group.undecided.is_empty() {
            evaluation.push_object_not_evaluated(
                representative,
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, groups.undecided),
            );
            continue;
        }
        if unreadable.contains(&scope) {
            evaluation.push_object_not_evaluated(
                representative,
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, groups.unreadable),
            );
            continue;
        }
        let (n_provided, n_required) = (group.provided.len(), group.required.len());
        let message = if n_provided == 0 {
            if n_required == 0 {
                continue;
            }
            render(plan, &read, groups.only_required)
        } else {
            let Some((false, requirement)) = mode.judge(n_provided as u64, n_required as u64)
            else {
                continue;
            };
            read.named.insert("requirement", requirement);
            render(plan, &read, groups.fail)
        };
        evaluation.push_finding(finding(
            rule,
            &representative,
            message,
            group.evidence,
            group.provided.into_iter().chain(group.required).collect(),
        ));
    }
    evaluation
}

#[cfg(test)]
mod tests {
    use super::{Mode, Operator};

    fn ratio(operator: Operator, word: &str, small: Option<(u64, u64)>) -> Mode {
        Mode::Ratio {
            provided_unit: 1,
            required_unit: 4,
            operator,
            word: word.to_owned(),
            small,
        }
    }

    /// One provided per four required, cross-multiplied in integers; the
    /// small-count exception judged against its own minimum, never at zero.
    #[test]
    fn a_ratio_cross_multiplies_and_small_counts_take_their_minimum() {
        let mode = ratio(Operator::AtLeast, "at_least", None);
        assert_eq!(
            mode.judge(1, 4),
            Some((true, "1/1 at_least 4/4".to_owned()))
        );
        assert_eq!(mode.judge(1, 5).map(|(holds, _)| holds), Some(false));
        let small = ratio(Operator::AtLeast, "at_least", Some((4, 2)));
        assert_eq!(
            small.judge(1, 3),
            Some((false, "1 at_least 2 for fewer than 4 required".to_owned()))
        );
        assert_eq!(small.judge(0, 0).map(|(holds, _)| holds), Some(true));
    }

    /// The row with the largest start not above the required count
    /// applies, increments beyond the last, nothing below the first.
    #[test]
    fn a_table_steps_and_extrapolates_but_never_below_its_first_row() {
        let mode = Mode::Table {
            rows: vec![(1, 1), (10, 2), (25, 3)],
            increment: Some((15, 1)),
        };
        assert_eq!(mode.judge(1, 0), None);
        assert_eq!(
            mode.judge(1, 12),
            Some((false, "at least 2 provided for 12 required".to_owned()))
        );
        assert_eq!(
            mode.judge(4, 40),
            Some((true, "at least 4 provided for 40 required".to_owned()))
        );
        let increments = Mode::Table {
            rows: Vec::new(),
            increment: Some((5, 1)),
        };
        assert_eq!(
            increments.judge(0, 4),
            Some((true, "at least 0 provided for 4 required".to_owned()))
        );
    }
}
