//! Exit separation: a space's exits must lie far enough apart for its size.
//!
//! Each selected space reaches its exits through `exit_path` (for example
//! `axioval:derived.adjacent-space:backward`, from the space to the doors on
//! its boundary), filtered by `exit_selector`. Two exits must lie at least
//! `fraction` of the space's longest plan diagonal apart; when `flag`, read
//! on the space or on the objects `flag_path` reaches from it, is `true`,
//! `flagged_fraction` applies instead (a sprinklered storey, say).
//!
//! Separation is measured in plan between the exits' closest points (the
//! proximity service's `horizontal` distance), their centres or their
//! farthest points (the plan-span service). With `pairs` `any`, the default,
//! some pair of exits must lie that far apart; with `all`, every pair.
//!
//! Every length is an interval. A pair is far enough apart only when its
//! whole interval lies at or above the whole required interval, too close
//! only when it lies wholly below it, and unknown otherwise. An unknown flag
//! widens the required interval to span both fractions, so it decides
//! nothing it could change. Exits whose selection is undecided can only add
//! pairs: a verdict stands only when they cannot change it.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlanSpan, PlanSpanError, PlanSpanServiceHandle, ProximityProjection, ProximityRequest,
    ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue};

use crate::plan_area::shown;
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, finding, invalid, resolve, undefined,
};

/// Requires each selected space's exits to lie far enough apart for its size.
pub struct ExitSeparation;

/// Between which points of two exits a separation is measured.
#[derive(Clone, Copy)]
enum Separation {
    Closest,
    Span(PlanSpan),
}

impl Separation {
    fn describe(self) -> &'static str {
        match self {
            Self::Closest => "between closest points",
            Self::Span(PlanSpan::Centres) => "between centres",
            Self::Span(PlanSpan::Farthest) => "between farthest points",
        }
    }
}

/// Which pairs of exits must lie far enough apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pairs {
    Any,
    All,
}

/// The flag that selects the second fraction, and where it is read.
struct Flag<'a> {
    property: PropertyRef<'a>,
    path: Option<Traversal<'a>>,
    fraction: f64,
}

struct Declaration<'a> {
    exits: Traversal<'a>,
    exit_selector: &'a Selector,
    fraction: f64,
    flag: Option<Flag<'a>>,
    separation: Separation,
    pairs: Pairs,
    minimum_exits: Option<usize>,
}

fn fraction(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.number(name)? {
        Some(value) if !(value.is_finite() && value > 0.0) => {
            Err(invalid(format!("`{name}` must be a positive number")))
        }
        other => Ok(other),
    }
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let exits = Traversal::path(
        parameters
            .strings("exit_path")?
            .ok_or_else(|| invalid("parameter `exit_path` is required"))?,
    )?;
    let exit_selector = parameters.required_selector("exit_selector")?;
    let flag = match (
        parameters.property("flag")?,
        parameters.strings("flag_path")?,
        fraction(&parameters, "flagged_fraction")?,
    ) {
        (None, None, None) => None,
        (Some(property), path, Some(fraction)) => Some(Flag {
            property,
            path: path.map(Traversal::path).transpose()?,
            fraction,
        }),
        (Some(_), _, None) => return Err(invalid("`flag` needs `flagged_fraction`")),
        (None, _, Some(_)) => return Err(invalid("`flagged_fraction` needs `flag`")),
        (None, Some(_), None) => return Err(invalid("`flag_path` is declared without `flag`")),
    };
    let separation = match parameters.string("separation")?.unwrap_or("closest") {
        "closest" => Separation::Closest,
        "centres" => Separation::Span(PlanSpan::Centres),
        "farthest" => Separation::Span(PlanSpan::Farthest),
        other => return Err(invalid(format!("separation `{other}` is unsupported"))),
    };
    let pairs = match parameters.string("pairs")?.unwrap_or("any") {
        "any" => Pairs::Any,
        "all" => Pairs::All,
        other => return Err(invalid(format!("pairs `{other}` is unsupported"))),
    };
    let minimum_exits = parameters
        .integer("minimum_exits")?
        .map(|minimum| {
            usize::try_from(minimum)
                .ok()
                .filter(|minimum| *minimum > 0)
                .ok_or_else(|| invalid("`minimum_exits` must be at least one"))
        })
        .transpose()?;
    Ok(Declaration {
        exits,
        exit_selector,
        fraction: fraction(&parameters, "fraction")?.unwrap_or(0.5),
        flag,
        separation,
        pairs,
        minimum_exits,
    })
}

impl RuleCapability for ExitSeparation {
    fn id(&self) -> &'static str {
        "axioval:capability.exit-separation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("exit_path", ParameterType::StringList),
            ParameterDescriptor::required("exit_selector", ParameterType::Selector),
            ParameterDescriptor::optional("fraction", ParameterType::Number),
            ParameterDescriptor::optional("flag", ParameterType::PropertyReference),
            ParameterDescriptor::optional("flag_path", ParameterType::StringList),
            ParameterDescriptor::optional("flagged_fraction", ParameterType::Number),
            ParameterDescriptor::optional("separation", ParameterType::String),
            ParameterDescriptor::optional("pairs", ParameterType::String),
            ParameterDescriptor::optional("minimum_exits", ParameterType::Integer),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("exit-separation: {message}"),
                );
            }
        };
        let candidates = Candidates::select(context, declared.exit_selector);
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        for space in spaces {
            let found = check(context, rule, &declared, &candidates, space);
            for finding in found.findings {
                evaluation.push_finding(finding);
            }
            if let Some((reason, message)) = found.unevaluated {
                evaluation.push_object_not_evaluated(space.id.clone(), reason, message);
            }
        }
        evaluation
    }
}

/// The objects `exit_selector` picks, and those it cannot decide. Shared
/// with `escape-route`.
pub(crate) struct Candidates<'a> {
    pub(crate) universe: Vec<&'a Object>,
    pub(crate) undecided: BTreeMap<ObjectId, String>,
}

impl<'a> Candidates<'a> {
    pub(crate) fn select(context: &RuleContext<'a>, selector: &Selector) -> Self {
        let mut universe = Vec::new();
        let mut undecided = BTreeMap::new();
        for object in context.project.objects() {
            match selector_matches(context, selector, object, &mut Vec::new()) {
                Selection::Match => universe.push(object),
                Selection::NoMatch => {}
                Selection::NotEvaluated(_, message) => {
                    undecided.insert(object.id.clone(), message);
                    universe.push(object);
                }
            }
        }
        Self {
            universe,
            undecided,
        }
    }
}

/// What checking one space found: findings that stand, and why the rest
/// could not be decided.
#[derive(Default)]
struct Checked {
    findings: Vec<Finding>,
    unevaluated: Option<Unavailable>,
}

/// A pair of exits and how its separation stands against the requirement.
struct Pair {
    first: ObjectId,
    second: ObjectId,
    measured: Result<(f64, f64, Evidence), String>,
}

enum Standing {
    FarEnough,
    TooClose,
    Unknown(String),
}

impl Pair {
    fn standing(&self, required: (f64, f64)) -> Standing {
        match &self.measured {
            Err(why) => Standing::Unknown(format!(
                "the separation of {} and {} cannot be measured: {why}",
                self.first, self.second
            )),
            Ok((lower, _, _)) if *lower >= required.1 => Standing::FarEnough,
            Ok((_, upper, _)) if *upper < required.0 => Standing::TooClose,
            Ok((lower, upper, _)) => Standing::Unknown(format!(
                "{} and {} are {} m apart, which straddles the required {} m",
                self.first,
                self.second,
                shown(*lower, *upper),
                shown(required.0, required.1)
            )),
        }
    }

    fn describe(&self, separation: Separation) -> String {
        let apart = match &self.measured {
            Ok((lower, upper, _)) => shown(*lower, *upper),
            Err(_) => "unknown".into(),
        };
        format!(
            "{} and {} are {apart} m apart {}",
            self.first,
            self.second,
            separation.describe()
        )
    }
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    candidates: &Candidates<'_>,
    space: &Object,
) -> Checked {
    let mut checked = Checked::default();
    let (reached, mut evidence) =
        match declared
            .exits
            .related(context, &space.id, &candidates.universe)
        {
            Ok(reached) => reached,
            Err(unavailable) => {
                checked.unevaluated = Some(unavailable);
                return checked;
            }
        };
    let (maybe, exits): (Vec<ObjectId>, Vec<ObjectId>) = reached
        .into_iter()
        .partition(|exit| candidates.undecided.contains_key(exit));
    let undecided = || {
        maybe
            .iter()
            .map(|exit| {
                format!(
                    "whether {exit} is an exit is undecided: {}",
                    candidates.undecided[exit]
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    if let Some(minimum) = declared.minimum_exits {
        if exits.len() + maybe.len() < minimum {
            checked.findings.push(finding(
                rule,
                &space.id,
                format!(
                    "has {} exit(s) via {}; at least {minimum} required",
                    exits.len() + maybe.len(),
                    declared.exits.relationship
                ),
                evidence.clone(),
                exits.iter().chain(&maybe).cloned().collect(),
            ));
        } else if exits.len() < minimum {
            checked.unevaluated = Some((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} certain exit(s), at least {minimum} required: {}",
                    exits.len(),
                    undecided()
                ),
            ));
            return checked;
        }
    }
    if exits.len() + maybe.len() < 2 {
        return checked;
    }
    let separated = if exits.len() < 2 {
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("fewer than two certain exits: {}", undecided()),
        ))
    } else {
        separation(
            context,
            rule,
            declared,
            space,
            &exits,
            &maybe,
            undecided,
            &mut evidence,
        )
    };
    match separated {
        Ok(Some(found)) => checked.findings.push(found),
        // A finding already standing is not withdrawn for want of the rest.
        Err(unavailable) if checked.findings.is_empty() => checked.unevaluated = Some(unavailable),
        Ok(None) | Err(_) => {}
    }
    checked
}

/// Judges the separation of `exits`, in identity order, against the space's
/// required distance.
#[allow(clippy::too_many_arguments)]
fn separation(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    space: &Object,
    exits: &[ObjectId],
    maybe: &[ObjectId],
    undecided: impl Fn() -> String,
    evidence: &mut Vec<Evidence>,
) -> Result<Option<Finding>, Unavailable> {
    let spans = context
        .services
        .get::<PlanSpanServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "plan-span service is not registered".to_owned(),
            )
        })?;
    let diameter = spans
        .measure_diameter(&space.id)
        .map_err(|error| span_unavailable(&error))?;
    let mut related = BTreeSet::new();
    let (fractions, flag_note) = fractions(context, declared, space, evidence, &mut related)?;
    let required = (
        fractions.0 * diameter.lower_metres(),
        fractions.1 * diameter.upper_metres(),
    );
    let pairs = measure_pairs(context, spans, declared.separation, exits)?;
    let standings: Vec<Standing> = pairs.iter().map(|pair| pair.standing(required)).collect();
    let unknown: Vec<&str> = standings
        .iter()
        .filter_map(|standing| match standing {
            Standing::Unknown(why) => Some(why.as_str()),
            _ => None,
        })
        .collect();
    let far_enough = standings
        .iter()
        .any(|standing| matches!(standing, Standing::FarEnough));
    let too_close: Vec<&Pair> = pairs
        .iter()
        .zip(&standings)
        .filter(|(_, standing)| matches!(standing, Standing::TooClose))
        .map(|(pair, _)| pair)
        .collect();
    let requirement = format!(
        "required at least {} m ({} × the longest plan diagonal of {} m{flag_note})",
        shown(required.0, required.1),
        shown(fractions.0, fractions.1),
        shown(diameter.lower_metres(), diameter.upper_metres()),
    );
    let failing: Vec<&Pair> = match declared.pairs {
        Pairs::Any if far_enough => return Ok(None),
        // Every pair is too close; name the one farthest apart.
        Pairs::Any if unknown.is_empty() && maybe.is_empty() => pairs
            .iter()
            .max_by(|a, b| upper(a).total_cmp(&upper(b)))
            .into_iter()
            .collect(),
        Pairs::All if !too_close.is_empty() => too_close,
        Pairs::All if unknown.is_empty() && maybe.is_empty() => return Ok(None),
        _ => {
            let mut why: Vec<String> = unknown.iter().map(|why| (*why).to_owned()).collect();
            if !maybe.is_empty() {
                why.push(undecided());
            }
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} ({requirement})", why.join("; ")),
            ));
        }
    };
    let described: Vec<String> = failing
        .iter()
        .map(|pair| pair.describe(declared.separation))
        .collect();
    let message = if declared.pairs == Pairs::Any && exits.len() > 2 {
        format!(
            "no two of its {} exits are far enough apart: {}; {requirement}",
            exits.len(),
            described.join("; ")
        )
    } else {
        format!("exits {}; {requirement}", described.join("; "))
    };
    evidence.push(diameter.evidence().clone());
    for pair in &failing {
        related.insert(pair.first.clone());
        related.insert(pair.second.clone());
        if let Ok((_, _, measured)) = &pair.measured {
            evidence.push(measured.clone());
        }
    }
    Ok(Some(finding(
        rule,
        &space.id,
        message,
        evidence.clone(),
        related.into_iter().collect(),
    )))
}

fn upper(pair: &Pair) -> f64 {
    pair.measured
        .as_ref()
        .map_or(f64::NEG_INFINITY, |(_, upper, _)| *upper)
}

fn span_unavailable(error: &PlanSpanError) -> Unavailable {
    let reason = match error {
        PlanSpanError::UnknownObject(_) | PlanSpanError::Unavailable(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        PlanSpanError::InvalidMeasurement | PlanSpanError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (
        reason,
        format!("the longest plan diagonal cannot be measured: {error}"),
    )
}

/// Every pair of `exits` with its measured separation.
fn measure_pairs(
    context: &RuleContext<'_>,
    spans: &PlanSpanServiceHandle,
    separation: Separation,
    exits: &[ObjectId],
) -> Result<Vec<Pair>, Unavailable> {
    let proximity = match separation {
        Separation::Closest => Some(
            context
                .services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::MissingService,
                        "proximity service is not registered".to_owned(),
                    )
                })?,
        ),
        Separation::Span(_) => None,
    };
    let mut pairs = Vec::new();
    for (index, first) in exits.iter().enumerate() {
        for second in &exits[index + 1..] {
            let measured = match (separation, proximity) {
                (Separation::Closest, Some(proximity)) => ProximityRequest::projected(
                    first.clone(),
                    second.clone(),
                    ProximityProjection::Horizontal,
                )
                .and_then(|request| proximity.measure_distance(&request))
                .map(|distance| {
                    let (lower, upper) = distance.interval_metres();
                    (lower, upper, distance.evidence().clone())
                })
                .map_err(|error| error.to_string()),
                (Separation::Span(between), _) => spans
                    .measure_span(first, second, between)
                    .map(|length| {
                        (
                            length.lower_metres(),
                            length.upper_metres(),
                            length.evidence().clone(),
                        )
                    })
                    .map_err(|error| error.to_string()),
                (Separation::Closest, None) => unreachable!("the proximity service was required"),
            };
            pairs.push(Pair {
                first: first.clone(),
                second: second.clone(),
                measured,
            });
        }
    }
    Ok(pairs)
}

/// The fraction of the diagonal that applies, as an interval spanning both
/// fractions when the flag is unknown, and a note naming the flag.
fn fractions(
    context: &RuleContext<'_>,
    declared: &Declaration<'_>,
    space: &Object,
    evidence: &mut Vec<Evidence>,
    related: &mut BTreeSet<ObjectId>,
) -> Result<((f64, f64), String), Unavailable> {
    let Some(flag) = &declared.flag else {
        return Ok(((declared.fraction, declared.fraction), String::new()));
    };
    let named = match &flag.path {
        None => flag.property.to_string(),
        Some(path) => format!("{} (via {})", flag.property, path.relationship),
    };
    Ok(match read_flag(context, flag, space, evidence, related)? {
        Ok(true) => ((flag.fraction, flag.fraction), format!(", {named} true")),
        Ok(false) => (
            (declared.fraction, declared.fraction),
            format!(", {named} false"),
        ),
        Err(why) => (
            (
                flag.fraction.min(declared.fraction),
                flag.fraction.max(declared.fraction),
            ),
            format!(", {named} unknown: {why}"),
        ),
    })
}

/// The flag's value on the space or the objects its path reaches, which must
/// agree; `Err` says why it is unknown.
fn read_flag(
    context: &RuleContext<'_>,
    flag: &Flag<'_>,
    space: &Object,
    evidence: &mut Vec<Evidence>,
    related: &mut BTreeSet<ObjectId>,
) -> Result<Result<bool, String>, Unavailable> {
    let holders = match &flag.path {
        None => vec![space.id.clone()],
        Some(path) => {
            let everything: Vec<&Object> = context.project.objects().collect();
            let (reached, cited) = match path.related(context, &space.id, &everything) {
                Ok(reached) => reached,
                Err((_, why)) => return Ok(Err(why)),
            };
            evidence.extend(cited);
            if reached.is_empty() {
                return Ok(Err(format!("{} reaches no object", path.relationship)));
            }
            reached
        }
    };
    let mut found: Option<(bool, ObjectId)> = None;
    for holder in holders {
        let Some(object) = context.project.object(&holder) else {
            return Err(invalid(format!("{holder} is not in the project")));
        };
        let resolved = match resolve(context, object, flag.property) {
            Ok(resolved) => resolved,
            Err((_, why)) => return Ok(Err(format!("{} of {holder}: {why}", flag.property))),
        };
        evidence.extend(resolved.evidence());
        if holder != space.id {
            related.insert(holder.clone());
        }
        let value = match resolved.value() {
            Some(PropertyValue::Boolean(value)) => *value,
            other if undefined(other) => {
                return Ok(Err(format!(
                    "{} of {holder} is {}",
                    flag.property,
                    display(other)
                )));
            }
            other => {
                return Ok(Err(format!(
                    "{} of {holder} is {}, not a boolean",
                    flag.property,
                    display(other)
                )));
            }
        };
        match &found {
            Some((held, first)) if *held != value => {
                return Ok(Err(format!(
                    "{} differs between {first} ({held}) and {holder} ({value})",
                    flag.property
                )));
            }
            Some(_) => {}
            None => found = Some((value, holder)),
        }
    }
    Ok(found.map_or_else(
        || Err(format!("{} has no value", flag.property)),
        |(value, _)| Ok(value),
    ))
}
