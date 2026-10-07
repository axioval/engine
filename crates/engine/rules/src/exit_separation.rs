//! Exit separation: a space's exits must lie far enough apart for its size.
//!
//! Each selected space reaches its exits through `exit_path` (for example
//! `axioval:derived.adjacent-space:backward`, from the space to the doors on
//! its boundary), filtered by `exit_selector`. Two exits must lie at least
//! `fraction` of the space's longest plan diagonal apart; when `flag`, read
//! on the space or on the objects `flag_path` reaches from it, is `true`,
//! `flagged_fraction` applies instead (a sprinklered storey, say).
//! `flag_sources` reads the flag from several places in order instead (the
//! space, then its storey, then its building): the first that states a
//! value decides, and `flag_default` applies when none does.
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
//! nothing it could change. Only an exact absence moves on to the next
//! source; a source that cannot be read, or states a null, non-boolean or
//! disagreeing value, leaves the flag unknown. Exits whose selection is undecided can only add
//! pairs: a verdict stands only when they cannot change it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlanLength, PlanSpan, PlanSpanError, PlanSpanServiceHandle, ProximityProjection,
    ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::plan_area::shown;
use crate::selection::{Selection, selector_matches};
use crate::support::table::Row;
use crate::support::{
    Parameters, PropertyRef, Resolved, Traversal, Unavailable, display, invalid, resolve, undefined,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::ExitMeasures;

pub(crate) const NAME: &str = "exit-separation";

const FLAG_SOURCE_COLUMNS: &[TableColumn] = &[
    TableColumn::optional("property_set", ColumnKind::String),
    TableColumn::required("property", ColumnKind::String),
    TableColumn::optional("path", ColumnKind::String),
];

/// Requires each selected space's exits to lie far enough apart for its size.
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `exit_separation` list, the count of a space's exits and the
/// separation of their pairs against the share of its diagonal they must
/// lie apart, judged by the rule's minimum and the required separation.
pub struct ExitSeparation;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

/// Between which points of two exits a separation is measured.
#[derive(Clone, Copy)]
pub(crate) enum Separation {
    Closest,
    Span(PlanSpan),
}

impl Separation {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Closest => "between closest points",
            Self::Span(PlanSpan::Centres) => "between centres",
            Self::Span(PlanSpan::Farthest) => "between farthest points",
        }
    }
}

/// Which pairs of exits must lie far enough apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pairs {
    Any,
    All,
}

/// The flag that selects the second fraction, and where it is read.
pub(crate) struct Flag<'a> {
    /// Where the flag is read, in order: the first that states a value
    /// decides.
    sources: Vec<FlagSource<'a>>,
    /// The value when no source states one.
    default: Option<bool>,
    fraction: f64,
}

/// One place the flag is read: a property of the space, or of the objects
/// a path reaches from it.
pub(crate) struct FlagSource<'a> {
    property: PropertyRef<'a>,
    /// The path's steps, validated when declared.
    path: Option<Vec<String>>,
}

impl FlagSource<'_> {
    fn traversal(&self) -> Option<Traversal> {
        self.path
            .as_ref()
            .map(|path| Traversal::path(path).expect("the path was validated when it was declared"))
    }

    fn named(&self) -> String {
        match self.traversal() {
            None => self.property.to_string(),
            Some(path) => format!("{} (via {})", self.property, path.relationship),
        }
    }
}

/// What one flag source states.
enum Stated {
    Value(bool),
    /// Exactly nothing: every object it is read on lacks the property, or
    /// its path reaches none.
    Nothing(String),
    Unknown(String),
}

fn flag_source(
    property: PropertyRef<'_>,
    path: Option<Vec<String>>,
) -> Result<FlagSource<'_>, Unavailable> {
    if let Some(path) = &path {
        Traversal::path(path)?;
    }
    Ok(FlagSource { property, path })
}

/// The rows of `flag_sources`: a property (with its set) and the path the
/// objects it is read on are reached along, its steps separated by spaces.
fn flag_sources(rows: Vec<Row<'_>>) -> Result<Vec<FlagSource<'_>>, Unavailable> {
    if rows.is_empty() {
        return Err(invalid("`flag_sources` has no rows"));
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let property = row
                .text("property")?
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| invalid(format!("flag source {index} has no `property`")))?;
            let set = row.text("property_set")?;
            if set.is_some_and(|set| set.trim().is_empty()) {
                return Err(invalid(format!(
                    "flag source {index} has a blank `property_set`"
                )));
            }
            let path = row
                .text("path")?
                .map(|path| path.split_whitespace().map(str::to_owned).collect());
            flag_source(
                PropertyRef {
                    set,
                    name: property,
                },
                path,
            )
            .map_err(|(reason, why)| (reason, format!("flag source {index}: {why}")))
        })
        .collect()
}

fn flag<'a>(parameters: &Parameters<'a>) -> Result<Option<Flag<'a>>, Unavailable> {
    let sources = match (
        parameters.property("flag")?,
        parameters.strings("flag_path")?,
        parameters.table("flag_sources")?,
    ) {
        (None, None, None) => None,
        (Some(property), path, None) => {
            Some(vec![flag_source(property, path.map(<[String]>::to_vec))?])
        }
        (None, None, Some(rows)) => Some(flag_sources(rows)?),
        (None, Some(_), None) => return Err(invalid("`flag_path` is declared without `flag`")),
        (_, _, Some(_)) => {
            return Err(invalid(
                "declare either `flag` (with `flag_path`) or `flag_sources`, not both",
            ));
        }
    };
    let default = parameters.boolean("flag_default")?;
    match (sources, fraction(parameters, "flagged_fraction")?) {
        (None, None) if default.is_some() => {
            Err(invalid("`flag_default` needs `flag` or `flag_sources`"))
        }
        (None, None) => Ok(None),
        (Some(sources), Some(fraction)) => Ok(Some(Flag {
            sources,
            default,
            fraction,
        })),
        (Some(_), None) => Err(invalid("`flag` needs `flagged_fraction`")),
        (None, Some(_)) => Err(invalid("`flagged_fraction` needs `flag` or `flag_sources`")),
    }
}

pub(crate) struct Declaration<'a> {
    pub(crate) exits: Traversal,
    /// The exit selector, where the declaration was read from a rule.
    pub(crate) exit_selector: Option<&'a Selector>,
    pub(crate) fraction: f64,
    pub(crate) flag: Option<Flag<'a>>,
    pub(crate) separation: Separation,
    pub(crate) pairs: Pairs,
    pub(crate) minimum_exits: Option<usize>,
}

fn fraction(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.number(name)? {
        Some(value) if !(value.is_finite() && value > 0.0) => {
            Err(invalid(format!("`{name}` must be a positive number")))
        }
        other => Ok(other),
    }
}

/// The rule's declaration, read and refused as the capability always read
/// it; the exit selector too where `selector` holds (a measured list is
/// handed the objects it picks instead).
pub(crate) fn declaration<'a>(
    parameters: &Parameters<'a>,
    selector: bool,
) -> Result<Declaration<'a>, Unavailable> {
    let exits = Traversal::path(
        parameters
            .strings("exit_path")?
            .ok_or_else(|| invalid("parameter `exit_path` is required"))?,
    )?;
    let exit_selector = if selector {
        Some(parameters.required_selector("exit_selector")?)
    } else {
        None
    };
    let flag = flag(parameters)?;
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
        fraction: fraction(parameters, "fraction")?.unwrap_or(0.5),
        flag,
        separation,
        pairs,
        minimum_exits,
    })
}

/// Checks the rule parameters the measured `exit_separation` is handed, as
/// the rule states them: the capability's declaration, in its order and
/// words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    declaration(&Parameters(&rule), true).map(|_| ())
}

impl RuleCapability for ExitSeparation {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("exit_path", ParameterType::StringList),
        ParameterDescriptor::required("exit_selector", ParameterType::Selector),
        ParameterDescriptor::optional("fraction", ParameterType::Number),
        ParameterDescriptor::optional("flag", ParameterType::PropertyReference),
        ParameterDescriptor::optional("flag_path", ParameterType::StringList),
        ParameterDescriptor::optional("flagged_fraction", ParameterType::Number),
        ParameterDescriptor::optional("flag_sources", ParameterType::Table(FLAG_SOURCE_COLUMNS)),
        ParameterDescriptor::optional("flag_default", ParameterType::Boolean),
        ParameterDescriptor::optional("separation", ParameterType::String),
        ParameterDescriptor::optional("pairs", ParameterType::String),
        ParameterDescriptor::optional("minimum_exits", ParameterType::Integer),
    ]
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

/// The exits a space reaches: those surely exits, those whose selection is
/// undecided, and the evidence of the path reaching them.
pub(crate) struct Reached {
    pub(crate) exits: Vec<ObjectId>,
    pub(crate) maybe: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
}

impl Reached {
    /// The exits `space` reaches along the declared path among
    /// `universe`, those in `undecided` possible exits.
    pub(crate) fn of(
        context: &RuleContext<'_>,
        exits: &Traversal,
        (universe, undecided): (&[ObjectId], &BTreeMap<ObjectId, String>),
        space: &Object,
    ) -> Result<Self, Unavailable> {
        let (reached, evidence) = exits.related_ids(context, &space.id, universe)?;
        let (maybe, exits): (Vec<ObjectId>, Vec<ObjectId>) = reached
            .into_iter()
            .partition(|exit| undecided.contains_key(exit));
        Ok(Self {
            exits,
            maybe,
            evidence,
        })
    }

    /// Why each possible exit is undecided, joined `; `.
    pub(crate) fn undecided(&self, why: &BTreeMap<ObjectId, String>) -> String {
        self.maybe
            .iter()
            .map(|exit| format!("whether {exit} is an exit is undecided: {}", why[exit]))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// What a space's separation is judged on: its longest plan diagonal, the
/// share of it that applies (an interval over both shares where the flag
/// is unknown) with the note naming the flag, the objects the flag was read
/// on, and every pair of its sure exits with its separation.
pub(crate) struct Measured {
    pub(crate) diameter: PlanLength,
    pub(crate) fractions: (f64, f64),
    pub(crate) flag_note: String,
    pub(crate) related: BTreeSet<ObjectId>,
    pub(crate) pairs: Vec<Pair>,
}

impl Measured {
    /// Measures the separation of `exits`, in identity order, as the
    /// capability measured it; the flag's evidence joins `evidence`.
    pub(crate) fn of(
        context: &RuleContext<'_>,
        declared: &Declaration<'_>,
        space: &Object,
        exits: &[ObjectId],
        evidence: &mut Vec<Evidence>,
    ) -> Result<Self, Unavailable> {
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
        let pairs = measure_pairs(context, spans, declared.separation, exits)?;
        Ok(Self {
            diameter,
            fractions,
            flag_note,
            related,
            pairs,
        })
    }

    /// The separation required: the share of the diagonal, an interval.
    pub(crate) fn required(&self) -> (f64, f64) {
        (
            self.fractions.0 * self.diameter.lower_metres(),
            self.fractions.1 * self.diameter.upper_metres(),
        )
    }

    /// The requirement as findings word it.
    pub(crate) fn requirement(&self) -> String {
        let required = self.required();
        format!(
            "required at least {} m ({} × the longest plan diagonal of {} m{})",
            shown(required.0, required.1),
            shown(self.fractions.0, self.fractions.1),
            shown(self.diameter.lower_metres(), self.diameter.upper_metres()),
            self.flag_note,
        )
    }
}

/// A pair of exits and how its separation stands against the requirement.
pub(crate) struct Pair {
    pub(crate) first: ObjectId,
    pub(crate) second: ObjectId,
    pub(crate) measured: Result<(f64, f64, Evidence), String>,
}

pub(crate) enum Standing {
    FarEnough,
    TooClose,
    Unknown(String),
}

impl Pair {
    pub(crate) fn standing(&self, required: (f64, f64)) -> Standing {
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

    pub(crate) fn describe(&self, separation: Separation) -> String {
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

pub(crate) fn upper(pair: &Pair) -> f64 {
    pair.measured
        .as_ref()
        .map_or(f64::NEG_INFINITY, |(_, upper, _)| *upper)
}

pub(crate) fn span_unavailable(error: &PlanSpanError) -> Unavailable {
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
pub(crate) fn measure_pairs(
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
pub(crate) fn fractions(
    context: &RuleContext<'_>,
    declared: &Declaration<'_>,
    space: &Object,
    evidence: &mut Vec<Evidence>,
    related: &mut BTreeSet<ObjectId>,
) -> Result<((f64, f64), String), Unavailable> {
    let Some(flag) = &declared.flag else {
        return Ok(((declared.fraction, declared.fraction), String::new()));
    };
    let applying = |value: bool| {
        let fraction = if value {
            flag.fraction
        } else {
            declared.fraction
        };
        (fraction, fraction)
    };
    let both = (
        flag.fraction.min(declared.fraction),
        flag.fraction.max(declared.fraction),
    );
    let mut silent = Vec::new();
    for source in &flag.sources {
        let named = source.named();
        match read_flag(context, source, space, evidence, related)? {
            Stated::Value(value) => return Ok((applying(value), format!(", {named} {value}"))),
            Stated::Unknown(why) => return Ok((both, format!(", {named} unknown: {why}"))),
            Stated::Nothing(why) => silent.push((named, why)),
        }
    }
    let names: Vec<&str> = silent.iter().map(|(named, _)| named.as_str()).collect();
    if let Some(value) = flag.default {
        return Ok((
            applying(value),
            format!(", {} not stated, default {value}", names.join(" nor ")),
        ));
    }
    let why: Vec<&str> = silent.iter().map(|(_, why)| why.as_str()).collect();
    Ok((
        both,
        format!(", {} unknown: {}", names.join(" nor "), why.join("; ")),
    ))
}

/// What one source states about the flag: the value on the space or the
/// objects its path reaches, which must agree, or exactly nothing when every
/// one of them lacks it.
fn read_flag(
    context: &RuleContext<'_>,
    source: &FlagSource<'_>,
    space: &Object,
    evidence: &mut Vec<Evidence>,
    related: &mut BTreeSet<ObjectId>,
) -> Result<Stated, Unavailable> {
    let property = source.property;
    let holders = match source.traversal() {
        None => vec![space.id.clone()],
        Some(path) => {
            let everything: Vec<&Object> = context.project.objects().collect();
            let (reached, cited) = match path.related(context, &space.id, &everything) {
                Ok(reached) => reached,
                Err((_, why)) => return Ok(Stated::Unknown(why)),
            };
            evidence.extend(cited);
            if reached.is_empty() {
                return Ok(Stated::Nothing(format!(
                    "{} reaches no object",
                    path.relationship
                )));
            }
            reached
        }
    };
    let mut found: Option<(bool, ObjectId)> = None;
    let mut lacking: Option<ObjectId> = None;
    for holder in holders {
        let Some(object) = context.project.object(&holder) else {
            return Err(invalid(format!("{holder} is not in the project")));
        };
        let resolved = match resolve(context, object, property) {
            Ok(resolved) => resolved,
            Err((_, why)) => return Ok(Stated::Unknown(format!("{property} of {holder}: {why}"))),
        };
        evidence.extend(resolved.evidence());
        if holder != space.id {
            related.insert(holder.clone());
        }
        let value = match &resolved {
            Resolved::Absent(_) => {
                lacking.get_or_insert(holder);
                continue;
            }
            Resolved::Present(stated) => match &stated.value {
                PropertyValue::Boolean(value) => *value,
                // Stated empty is not an absence: never move on from it.
                other if undefined(Some(other)) => {
                    return Ok(Stated::Unknown(format!(
                        "{property} of {holder} is {}",
                        display(Some(other))
                    )));
                }
                other => {
                    return Ok(Stated::Unknown(format!(
                        "{property} of {holder} is {}, not a boolean",
                        display(Some(other))
                    )));
                }
            },
        };
        match &found {
            Some((held, first)) if *held != value => {
                return Ok(Stated::Unknown(format!(
                    "{property} differs between {first} ({held}) and {holder} ({value})"
                )));
            }
            Some(_) => {}
            None => found = Some((value, holder)),
        }
    }
    Ok(match (found, lacking) {
        (Some((value, _)), None) => Stated::Value(value),
        (Some((value, first)), Some(without)) => Stated::Unknown(format!(
            "{first} states {property} {value}, {without} states nothing"
        )),
        (None, _) => Stated::Nothing(format!("{property} has no value")),
    })
}
