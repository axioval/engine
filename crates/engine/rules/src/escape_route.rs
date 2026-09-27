//! `escape-route`: travel distance, number of exits and exit widths per
//! space use.
//!
//! Each selected space takes the first row of `uses` whose `spaces`
//! selector picks it. The row states, each optionally:
//!
//! - `maximum_travel`: the longest walk, in metres, from the start to the
//!   nearest exit. With `route_start` `farthest-point` (the default) the
//!   start is every point of the space's walkable area, so the farthest one
//!   counts; with `door` it is each of the space's own doors (`door_path`
//!   among `door_selector` objects). Walking follows the walking line of a
//!   point (`walking_height`, `walking_step`) through the metric-routing
//!   service; exits are targets at their representative points (the centre
//!   of the footprint, inside it, on the exit's floor).
//! - `exits`: how many exits the space needs.
//! - `area_per_occupant`: the plan area, in square metres, per occupant.
//!   The occupant load is the space's measured footprint divided by it,
//!   rounded up, and every exit must be at least as wide as the first row of
//!   `widths` whose `occupants` covers that load (`width`), and all exits
//!   together as wide as its `total_width` where stated. An exit's width is
//!   its `clear_width_property`, a length; where none is stated, the
//!   geometry decides only a failure: a footprint whose longest plan
//!   diagonal is narrower than required bounds the clear width from above.
//!
//! Exits are the `exit_selector` objects `exit_path` reaches from the space.
//! Every measure is an interval: the travel distance is bounded from above
//! through the exits that surely are exits and from below through every one
//! that might be, and a verdict stands only when what is unknown cannot
//! change it. An occupant load straddling two rows of `widths` requires
//! either row's width, so only what both decide stands.
//!
//! Not checked: multipliers for stairs and for route sections shared by
//! several spaces (routes are measured on one level, and a shared section is
//! not a measured quantity), the free width of passages (corridors) between
//! the exits, and whether exit doors open in the direction of escape (door
//! leaves are not read yet).

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, FarthestPointOutcome, FarthestPointRequest,
    MetricPoint, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanArea,
    PlanSpanServiceHandle, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::exit_separation::Candidates;
use crate::plan_area::{footprint, shown};
use crate::selection::{Selection, select_objects, selector_matches};
use crate::space_distance::representative_point;
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, finding, invalid, resolve,
};

const USES: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("spaces", ColumnKind::Selector),
    TableColumn::optional("maximum_travel", ColumnKind::Number),
    TableColumn::optional("route_start", ColumnKind::String),
    TableColumn::optional("exits", ColumnKind::Integer),
    TableColumn::optional("area_per_occupant", ColumnKind::Number),
];

const WIDTHS: &[TableColumn] = &[
    TableColumn::required("occupants", ColumnKind::Integer),
    TableColumn::required("width", ColumnKind::Number),
    TableColumn::optional("total_width", ColumnKind::Number),
];

/// How narrow the farthest-point bracket is asked to become, in metres.
const TOLERANCE: f64 = 0.01;

/// Requires each selected space's escape routes to fit its use.
pub struct EscapeRoute;

/// Where travel is measured from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    FarthestPoint,
    Door,
}

struct Use<'a> {
    name: String,
    spaces: &'a Selector,
    maximum_travel: Option<f64>,
    start: Start,
    exits: Option<usize>,
    area_per_occupant: Option<f64>,
}

struct Declaration<'a> {
    uses: Vec<Use<'a>>,
    /// `(occupants up to, width of each exit, width of all together)`, by
    /// occupants.
    widths: Vec<(u64, f64, Option<f64>)>,
    exits: Traversal<'a>,
    exit_selector: &'a Selector,
    doors: Option<(Traversal<'a>, &'a Selector)>,
    clear_width: Option<PropertyRef<'a>>,
    profile: Option<MobilityProfile>,
}

fn positive(name: &str, column: &str, value: Option<f64>) -> Result<Option<f64>, Unavailable> {
    match value {
        Some(value) if value <= 0.0 => Err(invalid(format!("{name}: `{column}` must be positive"))),
        other => Ok(other),
    }
}

#[allow(clippy::too_many_lines)]
fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let table = parameters
        .table("uses")?
        .ok_or_else(|| invalid("parameter `uses` is required"))?;
    let mut uses = Vec::new();
    for (index, row) in table.into_iter().enumerate() {
        let name = match row.text("label")? {
            Some(label) => format!("use {index} ({label})"),
            None => format!("use {index}"),
        };
        let maximum_travel = match row.number("maximum_travel")? {
            Some(value) if value < 0.0 => {
                return Err(invalid(format!(
                    "{name}: `maximum_travel` must not be negative"
                )));
            }
            other => other,
        };
        let route_start = row.text("route_start")?;
        if route_start.is_some() && maximum_travel.is_none() {
            return Err(invalid(format!(
                "{name}: `route_start` needs `maximum_travel`"
            )));
        }
        let start = match route_start {
            None | Some("farthest-point") => Start::FarthestPoint,
            Some("door") => Start::Door,
            Some(other) => {
                return Err(invalid(format!(
                    "{name}: route start `{other}` is unsupported (farthest-point, door)"
                )));
            }
        };
        let exits = row
            .integer("exits")?
            .map(|exits| {
                usize::try_from(exits)
                    .ok()
                    .filter(|exits| *exits > 0)
                    .ok_or_else(|| invalid(format!("{name}: `exits` must be at least one")))
            })
            .transpose()?;
        let area_per_occupant =
            positive(&name, "area_per_occupant", row.number("area_per_occupant")?)?;
        if maximum_travel.is_none() && exits.is_none() && area_per_occupant.is_none() {
            return Err(invalid(format!("{name} states no requirement")));
        }
        uses.push(Use {
            spaces: row
                .selector("spaces")?
                .ok_or_else(|| invalid(format!("{name} has no `spaces`")))?,
            maximum_travel,
            start,
            exits,
            area_per_occupant,
            name,
        });
    }
    let mut widths = Vec::new();
    for (index, row) in parameters
        .table("widths")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = format!("width {index}");
        let occupants = row
            .integer("occupants")?
            .and_then(|occupants| u64::try_from(occupants).ok())
            .filter(|occupants| *occupants > 0)
            .ok_or_else(|| invalid(format!("{name}: `occupants` must be at least one")))?;
        let width = positive(&name, "width", row.number("width")?)?
            .ok_or_else(|| invalid(format!("{name} has no `width`")))?;
        let total = positive(&name, "total_width", row.number("total_width")?)?;
        widths.push((occupants, width, total));
    }
    widths.sort_by_key(|(occupants, _, _)| *occupants);
    if widths.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("two rows of `widths` state the same `occupants`"));
    }
    let loads = uses.iter().any(|use_| use_.area_per_occupant.is_some());
    if loads && widths.is_empty() {
        return Err(invalid("`area_per_occupant` needs `widths`"));
    }
    if !loads && !widths.is_empty() {
        return Err(invalid("`widths` needs a use stating `area_per_occupant`"));
    }
    let doors = match (
        parameters.strings("door_path")?,
        parameters.selector("door_selector")?,
    ) {
        (Some(path), Some(selector)) => Some((Traversal::path(path)?, selector)),
        (None, None) => None,
        _ => return Err(invalid("`door_path` and `door_selector` go together")),
    };
    if doors.is_none() && uses.iter().any(|use_| use_.start == Start::Door) {
        return Err(invalid(
            "route start `door` needs `door_path` and `door_selector`",
        ));
    }
    let profile = match (
        parameters.number("walking_height")?,
        parameters.number("walking_step")?,
    ) {
        (Some(height), Some(step)) => Some(
            MobilityProfile::try_new(0.0, height, step, 0.0)
                .map_err(|error| invalid(error.to_string()))?,
        ),
        (None, None) => None,
        _ => return Err(invalid("`walking_height` and `walking_step` go together")),
    };
    if profile.is_none() && uses.iter().any(|use_| use_.maximum_travel.is_some()) {
        return Err(invalid(
            "`maximum_travel` needs `walking_height` and `walking_step`",
        ));
    }
    Ok(Declaration {
        uses,
        widths,
        exits: Traversal::path(
            parameters
                .strings("exit_path")?
                .ok_or_else(|| invalid("parameter `exit_path` is required"))?,
        )?,
        exit_selector: parameters.required_selector("exit_selector")?,
        doors,
        clear_width: parameters.property("clear_width_property")?,
        profile,
    })
}

impl RuleCapability for EscapeRoute {
    fn id(&self) -> &'static str {
        "axioval:capability.escape-route"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("uses", ParameterType::Table(USES)),
            ParameterDescriptor::optional("widths", ParameterType::Table(WIDTHS)),
            ParameterDescriptor::required("exit_path", ParameterType::StringList),
            ParameterDescriptor::required("exit_selector", ParameterType::Selector),
            ParameterDescriptor::optional("door_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("clear_width_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("walking_height", ParameterType::Number),
            ParameterDescriptor::optional("walking_step", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("escape-route: {message}"),
                );
            }
        };
        let exits = Candidates::select(context, declared.exit_selector);
        let doors = declared
            .doors
            .as_ref()
            .map(|(_, selector)| Candidates::select(context, selector));
        let judge = Judge {
            context,
            rule,
            declared: &declared,
            exits,
            doors,
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        for space in spaces {
            let matched =
                match_rows(
                    &declared.uses,
                    RowSelection::First,
                    |use_| match selector_matches(context, use_.spaces, space, &mut Vec::new()) {
                        Selection::Match => RowTest::Match(0),
                        Selection::NoMatch => RowTest::NoMatch,
                        Selection::NotEvaluated(..) => RowTest::Undecided,
                    },
                );
            let use_ = match matched {
                Matched::Rows(rows) if !rows.is_empty() => rows[0].1,
                Matched::Rows(_) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "escape-route: no row of `uses` picks this space",
                    );
                    continue;
                }
                Matched::Undecided | Matched::Ambiguous(_) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "escape-route: whether a row of `uses` picks this space is undecided",
                    );
                    continue;
                }
            };
            let mut checked = Checked::default();
            judge.space(space, use_, &mut checked);
            for found in checked.findings {
                evaluation.push_finding(found);
            }
            if !checked.doubts.is_empty() {
                let reason = if checked
                    .doubts
                    .iter()
                    .any(|(why, _)| *why == NotEvaluatedReason::MissingService)
                {
                    NotEvaluatedReason::MissingService
                } else {
                    checked.doubts[0].0.clone()
                };
                let mut messages: Vec<String> = checked
                    .doubts
                    .into_iter()
                    .map(|(_, message)| message)
                    .collect();
                messages.dedup();
                evaluation.push_object_not_evaluated(
                    space.id.clone(),
                    reason,
                    format!("escape-route {}: {}", use_.name, messages.join("; ")),
                );
            }
        }
        evaluation
    }
}

/// What checking one space found: findings that stand, and what could not
/// be decided.
#[derive(Default)]
struct Checked {
    findings: Vec<Finding>,
    doubts: Vec<Unavailable>,
}

fn incomplete(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}

fn missing(service: &str) -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        format!("{service} service is not registered"),
    )
}

/// Objects reached from a space, split by whether their selection is sure.
struct Reached {
    sure: Vec<ObjectId>,
    maybe: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

impl Reached {
    fn doubts(&self, candidates: &Candidates<'_>, what: &str) -> Vec<String> {
        self.maybe
            .iter()
            .map(|object| {
                format!(
                    "whether {object} is {what} is undecided: {}",
                    candidates.undecided[object]
                )
            })
            .collect()
    }
}

/// Widths required of each exit and of all together, as intervals.
struct Required {
    each: (f64, f64),
    total: Option<(f64, f64)>,
}

/// A stated or bounded exit width.
#[derive(Clone)]
enum Width {
    /// A stated clear width, and its evidence.
    Stated(f64, Vec<Evidence>),
    /// The footprint's longest plan diagonal, which no clear width exceeds.
    AtMost(f64, Evidence),
    Unknown(String),
}

/// A start (a door, or none for the farthest point), whether it surely is
/// one, and its travel bounded from below and from above.
type Measured = (Option<ObjectId>, bool, [Result<Travel, Unavailable>; 2]);

/// A bound on the travel from one start, and what supports it.
#[derive(Clone)]
struct Travel {
    lower: f64,
    upper: f64,
    /// Where the lower bound is attained, for a finding's message.
    at: Option<[f64; 2]>,
    evidence: Vec<Evidence>,
}

impl Travel {
    /// Nothing measured: the travel is at least `lower`, perhaps unbounded.
    fn unbounded(lower: f64) -> Self {
        Self {
            lower,
            upper: f64::INFINITY,
            at: None,
            evidence: Vec::new(),
        }
    }
}

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    declared: &'r Declaration<'r>,
    exits: Candidates<'c>,
    doors: Option<Candidates<'c>>,
}

impl Judge<'_, '_> {
    fn reached(
        &self,
        traversal: &Traversal<'_>,
        candidates: &Candidates<'_>,
        space: &ObjectId,
    ) -> Result<Reached, Unavailable> {
        let (reached, evidence) = traversal.related(self.context, space, &candidates.universe)?;
        let (maybe, sure) = reached
            .into_iter()
            .partition(|object| candidates.undecided.contains_key(object));
        Ok(Reached {
            sure,
            maybe,
            evidence,
        })
    }

    fn space(&self, space: &Object, use_: &Use<'_>, checked: &mut Checked) {
        let exits = match self.reached(&self.declared.exits, &self.exits, &space.id) {
            Ok(exits) => exits,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        if let Some(required) = use_.exits {
            self.count(space, use_, required, &exits, checked);
        }
        if let Some(area) = use_.area_per_occupant {
            self.widths(space, use_, area, &exits, checked);
        }
        if let Some(maximum) = use_.maximum_travel {
            self.travel(space, use_, maximum, &exits, checked);
        }
    }

    fn count(
        &self,
        space: &Object,
        use_: &Use<'_>,
        required: usize,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let (sure, maybe) = (exits.sure.len(), exits.maybe.len());
        if sure + maybe < required {
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "has {} exit(s) via {}; {} requires at least {required}",
                    sure + maybe,
                    self.declared.exits.relationship,
                    use_.name
                ),
                exits.evidence.clone(),
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
        } else if sure < required {
            checked.doubts.push(incomplete(format!(
                "{sure} certain exit(s), at least {required} required: {}",
                exits.doubts(&self.exits, "an exit").join("; ")
            )));
        }
    }

    /// An exit's clear width: stated, else bounded by its footprint.
    fn width(&self, exit: &ObjectId) -> Width {
        let objects: BTreeMap<&ObjectId, &Object> = self
            .context
            .project
            .objects()
            .map(|object| (&object.id, object))
            .collect();
        let mut why = "no `clear_width_property` is declared".to_owned();
        if let Some(property) = self.declared.clear_width {
            match objects.get(exit) {
                None => why = format!("{exit} is not in the project"),
                Some(object) => match resolve(self.context, object, property) {
                    Ok(resolved) => match resolved.value() {
                        None => why = format!("{property} of {exit} is not stated"),
                        Some(PropertyValue::Quantity {
                            value,
                            dimension: QuantityDimension::Length,
                        }) if value.is_finite() && *value > 0.0 => {
                            return Width::Stated(*value, resolved.evidence());
                        }
                        Some(other) => {
                            return Width::Unknown(format!(
                                "{property} of {exit} is {}, not a positive length",
                                display(Some(other))
                            ));
                        }
                    },
                    Err((_, message)) => {
                        return Width::Unknown(format!(
                            "{property} of {exit} cannot be read: {message}"
                        ));
                    }
                },
            }
        }
        let Some(spans) = self.context.services.get::<PlanSpanServiceHandle>() else {
            return Width::Unknown(format!(
                "{why}, and the plan-span service is not registered"
            ));
        };
        match spans.measure_diameter(exit) {
            Ok(diameter) => Width::AtMost(diameter.upper_metres(), diameter.evidence().clone()),
            Err(error) => Width::Unknown(format!("{why}, and its footprint: {error}")),
        }
    }

    /// The widths `widths` requires for any load between `least` and
    /// `most` occupants: of each exit, and of all together where the rows
    /// state it.
    fn required_width(&self, least: u64, most: u64) -> Result<Required, String> {
        let rows = &self.declared.widths;
        let from = rows
            .iter()
            .position(|(occupants, _, _)| *occupants >= least)
            .ok_or_else(|| format!("no row of `widths` covers {least} occupant(s)"))?;
        let to = rows
            .iter()
            .position(|(occupants, _, _)| *occupants >= most)
            .ok_or_else(|| format!("no row of `widths` covers {most} occupant(s)"))?;
        let span = |values: &mut dyn Iterator<Item = f64>| {
            values.fold((f64::INFINITY, 0.0_f64), |(low, high), value| {
                (low.min(value), high.max(value))
            })
        };
        let rows = &rows[from..=to];
        let totals: Vec<f64> = rows.iter().filter_map(|(_, _, total)| *total).collect();
        let total = if totals.is_empty() {
            None
        } else if totals.len() == rows.len() {
            Some(span(&mut totals.into_iter()))
        } else {
            return Err(format!(
                "the rows of `widths` for {least} to {most} occupants state `total_width` \
                 only in part"
            ));
        };
        Ok(Required {
            each: span(&mut rows.iter().map(|(_, width, _)| *width)),
            total,
        })
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn widths(
        &self,
        space: &Object,
        use_: &Use<'_>,
        per_occupant: f64,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let area = match footprint(self.context, &space.id) {
            Ok(area) => area,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        let load = |area: f64| (area / per_occupant).ceil().max(1.0) as u64;
        let (least, most) = (
            load(area.lower_square_metres()),
            load(area.upper_square_metres()),
        );
        let occupants = if least == most {
            format!("{least} occupant(s)")
        } else {
            format!("between {least} and {most} occupants")
        };
        let required = match self.required_width(least, most) {
            Ok(required) => required,
            Err(why) => {
                checked.doubts.push(incomplete(why));
                return;
            }
        };
        let (low, high) = required.each;
        let sure: Vec<Width> = exits.sure.iter().map(|exit| self.width(exit)).collect();
        let maybe: Vec<Width> = exits.maybe.iter().map(|exit| self.width(exit)).collect();
        let basis = format!(
            "{occupants} ({} m² at {per_occupant} m² each) require at least {} m ({})",
            shown(area.lower_square_metres(), area.upper_square_metres()),
            shown(low, high),
            use_.name
        );
        for (exit, width) in exits.sure.iter().zip(&sure) {
            let mut evidence = exits.evidence.clone();
            evidence.push(area.evidence().clone());
            match width.clone() {
                Width::Stated(width, cited) if width < low => {
                    evidence.extend(cited);
                    checked.findings.push(finding(
                        self.rule,
                        &space.id,
                        format!("exit {exit} is {width} m wide (stated clear width); {basis}"),
                        evidence,
                        vec![exit.clone()],
                    ));
                }
                Width::Stated(width, _) if width >= high => {}
                Width::Stated(width, _) => checked.doubts.push(incomplete(format!(
                    "exit {exit} is {width} m wide, and {basis}"
                ))),
                Width::AtMost(bound, cited) if bound < low => {
                    evidence.push(cited);
                    checked.findings.push(finding(
                        self.rule,
                        &space.id,
                        format!(
                            "exit {exit} is at most {} m wide (its whole footprint's longest \
                             plan diagonal); {basis}",
                            shown(bound, bound)
                        ),
                        evidence,
                        vec![exit.clone()],
                    ));
                }
                Width::AtMost(..) => checked.doubts.push(incomplete(format!(
                    "the clear width of exit {exit} is not stated, and {basis}"
                ))),
                Width::Unknown(why) => checked.doubts.push(incomplete(format!(
                    "the width of exit {exit} is unknown: {why}"
                ))),
            }
        }
        // An undecided exit needs a width only if it is one.
        for (exit, width) in exits.maybe.iter().zip(&maybe) {
            if !matches!(width, Width::Stated(width, _) if *width >= high) {
                checked.doubts.push(incomplete(format!(
                    "whether {exit} is an exit is undecided ({}), and it may be too narrow",
                    self.exits.undecided[exit]
                )));
            }
        }
        if let Some(total) = required.total {
            let widths = (sure.as_slice(), maybe.as_slice());
            self.total_width(
                space, use_, &occupants, &area, total, exits, widths, checked,
            );
        }
    }

    /// Judges the exits' widths together against `total`.
    #[allow(clippy::too_many_arguments)]
    fn total_width(
        &self,
        space: &Object,
        use_: &Use<'_>,
        occupants: &str,
        area: &PlanArea,
        (least_total, most_total): (f64, f64),
        exits: &Reached,
        (sure, maybe): (&[Width], &[Width]),
        checked: &mut Checked,
    ) {
        // Together the sure exits are at least as wide as their stated
        // widths; all that might be exits at most as wide as their bounds.
        let stated: f64 = sure
            .iter()
            .map(|width| match width {
                Width::Stated(width, _) => *width,
                _ => 0.0,
            })
            .sum();
        let bound: f64 = sure
            .iter()
            .chain(maybe)
            .map(|width| match width {
                Width::Stated(width, _) | Width::AtMost(width, _) => *width,
                Width::Unknown(_) => f64::INFINITY,
            })
            .sum();
        let together = format!(
            "{occupants} require at least {} m of exit width together ({})",
            shown(least_total, most_total),
            use_.name
        );
        if bound < least_total {
            let mut evidence = exits.evidence.clone();
            evidence.push(area.evidence().clone());
            for width in sure.iter().chain(maybe) {
                match width {
                    Width::Stated(_, cited) => evidence.extend(cited.iter().cloned()),
                    Width::AtMost(_, cited) => evidence.push(cited.clone()),
                    Width::Unknown(_) => {}
                }
            }
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "its {} exit(s) are at most {} m wide together; {together}",
                    sure.len() + maybe.len(),
                    shown(bound, bound)
                ),
                evidence,
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
        } else if stated < most_total {
            checked.doubts.push(incomplete(format!(
                "its exits are {} m wide together by their stated widths, and {together}",
                shown(stated, stated)
            )));
        }
    }

    #[allow(clippy::too_many_lines)]
    fn travel(
        &self,
        space: &Object,
        use_: &Use<'_>,
        maximum: f64,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let allows = format!("{} allows at most {maximum} m of travel", use_.name);
        if exits.sure.is_empty() && exits.maybe.is_empty() {
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "has no exit via {} to walk to; {allows}",
                    self.declared.exits.relationship
                ),
                exits.evidence.clone(),
                Vec::new(),
            ));
            return;
        }
        let Some(routes) = self.context.services.get::<MetricRoutingServiceHandle>() else {
            checked.doubts.push(missing("metric-routing"));
            return;
        };
        let Some(profile) = self.declared.profile else {
            checked
                .doubts
                .push(invalid("`maximum_travel` needs a walking profile"));
            return;
        };
        // What could change the verdict, reported only if it stays open.
        let mut doubts: Vec<Unavailable> = Vec::new();
        let mut sure: Vec<(ObjectId, MetricPoint)> = Vec::new();
        let mut all: Vec<(ObjectId, MetricPoint)> = Vec::new();
        let mut placed = true;
        for exit in exits.sure.iter().chain(&exits.maybe) {
            match representative_point(self.context, exit) {
                Ok((point, _)) => {
                    if exits.sure.contains(exit) {
                        sure.push((exit.clone(), point.clone()));
                    }
                    all.push((exit.clone(), point));
                }
                Err(unavailable) => {
                    placed = false;
                    doubts.push(unavailable);
                }
            }
        }
        doubts.extend(
            exits
                .doubts(&self.exits, "an exit")
                .into_iter()
                .map(incomplete),
        );
        let bounds = |door: Option<&ObjectId>| -> [Result<Travel, Unavailable>; 2] {
            let measure = |targets: &[(ObjectId, MetricPoint)]| {
                if targets.is_empty() {
                    return Ok(Travel::unbounded(f64::INFINITY));
                }
                let targets = targets.iter().map(|(_, point)| point.clone()).collect();
                match door {
                    None => Self::farthest(routes, &space.id, targets, profile),
                    Some(door) => self.nearest(routes, door, targets, profile),
                }
            };
            let upper = measure(&sure);
            let lower = if !placed {
                // An exit without a point might lie anywhere.
                Ok(Travel::unbounded(0.0))
            } else if sure.len() == all.len() {
                upper.clone()
            } else {
                measure(&all)
            };
            [lower, upper]
        };
        let measured: Vec<Measured> = match use_.start {
            Start::FarthestPoint => vec![(None, true, bounds(None))],
            Start::Door => {
                let (traversal, _) = self
                    .declared
                    .doors
                    .as_ref()
                    .expect("a door start is declared with doors");
                let candidates = self.doors.as_ref().expect("door candidates are selected");
                let doors = match self.reached(traversal, candidates, &space.id) {
                    Ok(doors) => doors,
                    Err(unavailable) => {
                        checked.doubts.push(unavailable);
                        return;
                    }
                };
                if doors.sure.is_empty() && doors.maybe.is_empty() {
                    checked.doubts.push(incomplete(format!(
                        "{} reaches no door via {} to start from",
                        space.id, traversal.relationship
                    )));
                    return;
                }
                doubts.extend(
                    doors
                        .doubts(candidates, "a door of it")
                        .into_iter()
                        .map(incomplete),
                );
                doors
                    .sure
                    .iter()
                    .map(|door| (door, true))
                    .chain(doors.maybe.iter().map(|door| (door, false)))
                    .map(|(door, sure_start)| (Some(door.clone()), sure_start, bounds(Some(door))))
                    .collect()
            }
        };
        // Travel is judged at its worst start: a finding needs one sure
        // start whose every route is too long, a pass every possible start
        // within the maximum.
        let mut most = 0.0_f64;
        let mut worst: Option<(Option<ObjectId>, Travel)> = None;
        for (start, sure_start, [lower, upper]) in measured {
            match upper {
                Ok(upper) => most = most.max(upper.upper),
                Err(unavailable) => {
                    most = f64::INFINITY;
                    doubts.push(unavailable);
                }
            }
            match lower {
                Ok(lower) if sure_start => {
                    if worst
                        .as_ref()
                        .is_none_or(|(_, known)| lower.lower > known.lower)
                    {
                        worst = Some((start, lower));
                    }
                }
                Ok(_) => {}
                Err(unavailable) => doubts.push(unavailable),
            }
        }
        let from = |start: &Option<ObjectId>| match start {
            Some(door) => format!("door {door}"),
            None => "its farthest point".to_owned(),
        };
        if let Some((start, travel)) = &worst
            && travel.lower > maximum
        {
            let place = travel
                .at
                .map(|[x, y]| format!(", around ({x:.2}, {y:.2}),"))
                .unwrap_or_default();
            let message = if travel.lower.is_infinite() {
                if start.is_none() {
                    format!("part of it{place} reaches no exit walking; {allows}")
                } else {
                    format!("{} reaches no exit walking; {allows}", from(start))
                }
            } else {
                format!(
                    "{}{place} lies {} m from the nearest exit walking; {allows}",
                    from(start),
                    if travel.upper.is_finite() {
                        shown(travel.lower, travel.upper)
                    } else {
                        format!("at least {}", shown(travel.lower, travel.lower))
                    }
                )
            };
            let mut evidence = exits.evidence.clone();
            evidence.extend(travel.evidence.iter().cloned());
            checked.findings.push(finding(
                self.rule,
                &space.id,
                message,
                evidence,
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
            // A travel finding stands; what is undecided cannot withdraw it.
            return;
        }
        if most <= maximum {
            return;
        }
        let least = worst.map_or(0.0, |(_, travel)| travel.lower);
        checked.doubts.extend(doubts);
        checked.doubts.push(incomplete(format!(
            "the longest travel to the nearest exit is {} m walking, and {allows}",
            if most.is_finite() {
                shown(least, most)
            } else {
                format!("at least {}", shown(least, least))
            }
        )));
    }

    /// The farthest point of `space` from the nearest of `targets`.
    fn farthest(
        routes: &MetricRoutingServiceHandle,
        space: &ObjectId,
        targets: Vec<MetricPoint>,
        profile: MobilityProfile,
    ) -> Result<Travel, Unavailable> {
        let request = FarthestPointRequest::try_new(space.clone(), targets, profile, TOLERANCE)
            .map_err(|error| incomplete(error.to_string()))?;
        match routes.farthest_point(&request) {
            Ok(FarthestPointOutcome::Bounded(bounded)) => {
                let [x, y, _] = bounded.witness().coordinates_metres();
                Ok(Travel {
                    lower: bounded.distance().lower_metres(),
                    upper: bounded.distance().upper_metres(),
                    at: Some([x, y]),
                    evidence: vec![bounded.evidence().clone()],
                })
            }
            Ok(FarthestPointOutcome::Unreachable(cut_off)) => {
                let [x, y, _] = cut_off.witness().coordinates_metres();
                Ok(Travel {
                    lower: f64::INFINITY,
                    upper: f64::INFINITY,
                    at: Some([x, y]),
                    evidence: vec![cut_off.completeness().evidence().clone()],
                })
            }
            Err(error) => Err(incomplete(format!(
                "the farthest point of {space} from an exit: {error}"
            ))),
        }
    }

    /// The walk from `door` to the nearest of `targets`.
    fn nearest(
        &self,
        routes: &MetricRoutingServiceHandle,
        door: &ObjectId,
        targets: Vec<MetricPoint>,
        profile: MobilityProfile,
    ) -> Result<Travel, Unavailable> {
        let (origin, cited) = representative_point(self.context, door)?;
        let request = NearestTargetRequest::try_new(origin, targets, profile)
            .map_err(|error| incomplete(error.to_string()))?;
        match routes.nearest_target(&request) {
            Ok(NearestTargetOutcome::Reached(reached)) => {
                let mut evidence = cited;
                evidence.push(reached.evidence().clone());
                Ok(Travel {
                    lower: reached.shortest_distance().lower_metres(),
                    upper: reached.shortest_distance().upper_metres(),
                    at: None,
                    evidence,
                })
            }
            Ok(NearestTargetOutcome::Unreachable(unreachable)) => {
                let mut evidence = cited;
                evidence.push(unreachable.completeness().evidence().clone());
                Ok(Travel {
                    lower: f64::INFINITY,
                    upper: f64::INFINITY,
                    at: None,
                    evidence,
                })
            }
            Err(error) => Err(incomplete(format!(
                "walking from door {door} to an exit: {error}"
            ))),
        }
    }
}
