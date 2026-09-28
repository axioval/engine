//! Relative counts: provided objects against required ones, per anchor or
//! per property-value group.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, ObjectId};

use crate::counts::{Population, relation_text, tally};
use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, finding, invalid, resolve,
    traversal_parameters, undefined, value_key,
};

#[derive(Clone, Copy)]
enum Operator {
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

/// Requires enough provided objects for the required ones at each anchor or
/// in each group.
///
/// **Ratio mode.** With `provided_unit` p and `required_unit` r, the anchor
/// passes when `provided / p` stands in `operator` (`equal`, `not_equal`,
/// `greater`, `at_least`, `less`, `at_most`) to `required / r`: "one
/// washbasin (provided, p = 1) per four workplaces (required, r = 4), at
/// least" is `washbasins * 4 >= workplaces * 1`. Integer arithmetic keeps it
/// exact.
///
/// **Small counts.** With `small_required_below` n and `small_provided` k,
/// both declared together, a required count from 1 up to but excluding n is
/// judged as `provided operator k` instead of by the ratio: "with fewer than
/// four workplaces, at least one washbasin" is n = 4, k = 1; "below ten
/// workplaces nothing is required" is n = 10, k = 0 with `at_least`. A
/// required count of zero is always judged by the ratio.
///
/// **Table mode.** `table` lists rows `R:P`, "from R required objects on, at
/// least P provided". The row with the largest R not above the required
/// count applies. Beyond the last row, each further `additional_required`
/// required objects need `additional_provided` more. Below the first row the
/// table sets no requirement, so the anchor or group is skipped rather than
/// extrapolated; a table of increments alone applies them from zero.
/// Parameters have no table type, so a row is written as text and a
/// malformed one is a declaration error.
///
/// **Anchors.** By default the rule's selection names the anchors, and the
/// relationship works as in `related-count`; with no relationship an
/// anchor's whole source is counted, so selecting the building checks the
/// whole model and selecting storeys checks each storey. An anchor with any
/// undecided member is not evaluated.
///
/// **Groups.** With `group_property`, the rule's selection is instead the set
/// of objects counted, and each is counted in the group of its
/// `group_property` value, within one source unless `across_sources`. Text
/// values are trimmed and compared ignoring case unless `case_sensitive`.
/// Relationship parameters do not apply. A group that has required objects
/// and no provided object is reported as present only in the required set,
/// whatever the mode would say. A group's finding is raised against its
/// lowest required object (its lowest provided object when it has none) and
/// names every member. A counted object with no group value gets a finding
/// of its own; one whose value cannot be read leaves every group of its
/// scope not evaluated, since it could belong to any of them.
pub struct RelativeCount;

/// The small-count exception of ratio mode.
struct SmallCount {
    /// Required counts from 1 up to but excluding this use the exception.
    below: u64,
    /// The right-hand side `provided` is compared to instead of the ratio.
    provided: u64,
}

enum Mode<'a> {
    Ratio {
        provided_unit: i64,
        required_unit: i64,
        operator: Operator,
        word: &'a str,
        small: Option<SmallCount>,
    },
    Table {
        /// `(required from, provided at least)`, sorted by `required from`.
        rows: Vec<(u64, u64)>,
        /// `(additional required, additional provided)`, both positive.
        increment: Option<(u64, u64)>,
    },
}

impl Mode<'_> {
    /// Whether the counts pass, and the requirement as a reviewer reads it;
    /// `None` when the mode sets no requirement for this required count.
    fn judge(&self, provided: u64, required: u64) -> Option<(bool, String)> {
        match self {
            Self::Ratio {
                operator,
                word,
                small: Some(small),
                ..
            } if (1..small.below).contains(&required) => Some((
                operator.holds(i128::from(provided), i128::from(small.provided)),
                format!(
                    "{provided} {word} {} for fewer than {} required",
                    small.provided, small.below
                ),
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

const RATIO_ONLY: [&str; 5] = [
    "provided_unit",
    "required_unit",
    "operator",
    "small_required_below",
    "small_provided",
];

fn parse_mode<'a>(parameters: &Parameters<'a>) -> Result<Mode<'a>, Unavailable> {
    let Some(table) = parameters.strings("table")? else {
        let unit = |name| match parameters.integer(name)? {
            Some(value) if value > 0 => Ok(value),
            _ => Err(invalid(format!("{name} must be a positive integer"))),
        };
        let word = parameters.required_string("operator")?;
        let small = match (
            parameters.integer("small_required_below")?,
            parameters.integer("small_provided")?,
        ) {
            (None, None) => None,
            (Some(below), Some(provided)) if below > 0 && provided >= 0 => Some(SmallCount {
                below: below.unsigned_abs(),
                provided: provided.unsigned_abs(),
            }),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "small_required_below must be positive and small_provided not negative",
                ));
            }
            _ => {
                return Err(invalid(
                    "small_required_below and small_provided go together",
                ));
            }
        };
        return Ok(Mode::Ratio {
            provided_unit: unit("provided_unit")?,
            required_unit: unit("required_unit")?,
            operator: Operator::parse(word)
                .ok_or_else(|| invalid(format!("operator `{word}` is unsupported")))?,
            word,
            small,
        });
    };
    for ratio in RATIO_ONLY {
        if parameters.0.parameters.contains_key(ratio) {
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
    let positive = |name| match parameters.integer(name)? {
        None => Ok(None),
        Some(value) if value > 0 => Ok(Some(value.unsigned_abs())),
        Some(_) => Err(invalid(format!("{name} must be positive"))),
    };
    let increment = match (
        positive("additional_required")?,
        positive("additional_provided")?,
    ) {
        (Some(step), Some(extra)) => Some((step, extra)),
        (None, None) => None,
        _ => {
            return Err(invalid(
                "additional_required and additional_provided go together",
            ));
        }
    };
    if rows.is_empty() && increment.is_none() {
        return Err(invalid("the table needs rows or increments"));
    }
    Ok(Mode::Table { rows, increment })
}

/// What the counts are taken over.
enum Grouping<'a> {
    /// The rule's selection, each anchor with its related objects.
    Anchors(Option<Traversal>),
    /// The rule's selection, grouped by a property value.
    Property {
        property: PropertyRef<'a>,
        across_sources: bool,
        case_sensitive: bool,
    },
}

const GROUP_ONLY: [&str; 2] = ["across_sources", "case_sensitive"];

fn parse_grouping<'a>(parameters: &Parameters<'a>) -> Result<Grouping<'a>, Unavailable> {
    let declared = |name: &str| parameters.0.parameters.contains_key(name);
    let Some(property) = parameters.property("group_property")? else {
        if let Some(name) = GROUP_ONLY.into_iter().find(|name| declared(name)) {
            return Err(invalid(format!(
                "`{name}` applies only with group_property"
            )));
        }
        return Ok(Grouping::Anchors(parameters.traversal()?));
    };
    if let Some(descriptor) = traversal_parameters()
        .into_iter()
        .find(|descriptor| declared(&descriptor.name))
    {
        return Err(invalid(format!(
            "`{}` does not apply with group_property",
            descriptor.name
        )));
    }
    Ok(Grouping::Property {
        property,
        across_sources: parameters.boolean("across_sources")?.unwrap_or(false),
        case_sensitive: parameters.boolean("case_sensitive")?.unwrap_or(false),
    })
}

impl RuleCapability for RelativeCount {
    fn id(&self) -> &'static str {
        "axioval:capability.relative-count"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("provided_selector", ParameterType::Selector),
            ParameterDescriptor::required("required_selector", ParameterType::Selector),
            ParameterDescriptor::optional("provided_unit", ParameterType::Integer),
            ParameterDescriptor::optional("required_unit", ParameterType::Integer),
            ParameterDescriptor::optional("operator", ParameterType::String),
            ParameterDescriptor::optional("small_required_below", ParameterType::Integer),
            ParameterDescriptor::optional("small_provided", ParameterType::Integer),
            ParameterDescriptor::optional("table", ParameterType::StringList),
            ParameterDescriptor::optional("additional_required", ParameterType::Integer),
            ParameterDescriptor::optional("additional_provided", ParameterType::Integer),
            ParameterDescriptor::optional("group_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            Ok::<_, Unavailable>((
                parameters.required_selector("provided_selector")?,
                parameters.required_selector("required_selector")?,
                parse_mode(&parameters)?,
                parse_grouping(&parameters)?,
            ))
        })();
        let (provided, required, mode, grouping) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("relative-count: {message}"),
                );
            }
        };
        let provided = Population::of(context, provided);
        let required = Population::of(context, required);
        match grouping {
            Grouping::Anchors(traversal) => by_anchor(
                context,
                rule,
                &mode,
                traversal.as_ref(),
                &provided,
                &required,
            ),
            Grouping::Property {
                property,
                across_sources,
                case_sensitive,
            } => by_group(
                context,
                rule,
                &mode,
                GroupBy {
                    property,
                    across_sources,
                    case_sensitive,
                },
                &provided,
                &required,
            ),
        }
    }
}

fn by_anchor(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    mode: &Mode<'_>,
    traversal: Option<&Traversal>,
    provided: &Population,
    required: &Population,
) -> CapabilityEvaluation {
    let (anchors, mut evaluation) = select_objects(context, &rule.selector);
    let via = relation_text(traversal);
    for anchor in anchors {
        let tallies = tally(context, traversal, anchor, provided)
            .and_then(|p| Ok((p, tally(context, traversal, anchor, required)?)));
        let (provided, required) = match tallies {
            Ok(tallies) => tallies,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                continue;
            }
        };
        if provided.undecided + required.undecided > 0 {
            evaluation.push_object_not_evaluated(
                anchor.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} related object(s) {via} cannot be assigned to either population",
                    provided.undecided + required.undecided
                ),
            );
            continue;
        }
        let (n_provided, n_required) = (provided.decided.len(), required.decided.len());
        let Some((false, requirement)) = mode.judge(n_provided as u64, n_required as u64) else {
            continue;
        };
        let mut evidence = provided.evidence;
        evidence.extend(required.evidence);
        evaluation.push_finding(finding(
            rule,
            &anchor.id,
            format!(
                "{n_provided} provided and {n_required} required object(s) {via}; \
                 required {requirement}"
            ),
            evidence,
            provided
                .decided
                .into_iter()
                .chain(required.decided)
                .collect(),
        ));
    }
    evaluation
}

#[derive(Clone, Copy)]
struct GroupBy<'a> {
    property: PropertyRef<'a>,
    across_sources: bool,
    case_sensitive: bool,
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

#[derive(Default)]
struct Group {
    shown: String,
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

#[allow(clippy::too_many_lines)]
fn by_group(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    mode: &Mode<'_>,
    by: GroupBy<'_>,
    provided: &Population,
    required: &Population,
) -> CapabilityEvaluation {
    let property = by.property;
    let selection = Population::of(context, &rule.selector);
    let mut evaluation = CapabilityEvaluation::default();
    let mut groups: BTreeMap<(String, String), Group> = BTreeMap::new();
    // Scopes holding an object whose group value could not be read.
    let mut unreadable: BTreeSet<String> = BTreeSet::new();
    for object in context.project.objects() {
        let as_provided = Membership::of(&object.id, &selection, provided);
        let as_required = Membership::of(&object.id, &selection, required);
        if as_provided == Membership::No && as_required == Membership::No {
            continue;
        }
        let scope = if by.across_sources {
            String::new()
        } else {
            object.id.source.to_string()
        };
        let resolved = match resolve(context, object, property) {
            Ok(resolved) => resolved,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                unreadable.insert(scope);
                continue;
            }
        };
        let value = match resolved.value() {
            Some(value) if !undefined(Some(value)) => value,
            value => {
                if as_provided == Membership::Yes || as_required == Membership::Yes {
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!(
                            "{property} is {}, so the object counts in no group",
                            display(value)
                        ),
                        resolved.evidence(),
                        vec![],
                    ));
                } else {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "{property} is {}, and whether the object is counted is undecided",
                            display(value)
                        ),
                    );
                }
                continue;
            }
        };
        let key = value_key(value, true, by.case_sensitive);
        let group = groups.entry((scope, key)).or_insert_with(|| Group {
            shown: display(Some(value)),
            ..Group::default()
        });
        group.evidence.extend(resolved.evidence());
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
    for ((scope, _), group) in groups {
        let representative = group.representative().clone();
        let label = format!("group {property} {}", group.shown);
        if !group.undecided.is_empty() {
            evaluation.push_object_not_evaluated(
                representative,
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{label}: {} object(s) cannot be assigned to either population",
                    group.undecided.len()
                ),
            );
            continue;
        }
        if unreadable.contains(&scope) {
            evaluation.push_object_not_evaluated(
                representative,
                NotEvaluatedReason::IncompleteEvidence,
                format!("{label}: an object whose {property} could not be read may belong here"),
            );
            continue;
        }
        let (n_provided, n_required) = (group.provided.len(), group.required.len());
        let message = if n_provided == 0 {
            if n_required == 0 {
                continue;
            }
            format!(
                "{label}: {n_required} required object(s) and no provided object; \
                 the group is present only in the required set"
            )
        } else {
            let Some((false, requirement)) = mode.judge(n_provided as u64, n_required as u64)
            else {
                continue;
            };
            format!(
                "{label}: {n_provided} provided and {n_required} required object(s); \
                 required {requirement}"
            )
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
