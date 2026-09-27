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
//! **Multiplied sections.** Each row of `sections` names objects (`objects`,
//! a stair, say) on which a walked metre counts `factor` times, at least
//! one; with `shared_by`, only a section that many checked spaces reach
//! along `section_path` multiplies. Metric routing measures the plain walk
//! and not which sections it crosses, so the multiplied travel is
//! bracketed: at least the plain walk's lower bound, at most its upper bound
//! times the largest factor of a section the walk may cross. A walk of at
//! most `U` metres stays within `U` of its start in plan, so a section whose
//! horizontal distance from the start (the space, or the door) surely
//! exceeds `U` is left out; every other one, and one that might be shared,
//! may be crossed.
//!
//! **Passages.** With `passage_selector`, the passages of a checked space
//! are the `passage_selector` objects `passage_path` reaches from it, and
//! the space itself where `passage_selector` picks it. A passage carries the
//! occupants of every checked space it serves, and must be as wide as the
//! `passage_width` of the rows of `widths` for that load. Its width is its
//! `passage_width_property`, a length; without one, only a failure decides:
//! no clear width exceeds the shorter side of the rectangle of least area
//! enclosing its footprint. A space whose load is unknown (no
//! `area_per_occupant`, an undecided use or selection, an unmeasured
//! footprint) leaves every passage it may serve not evaluated.
//!
//! Not checked: which passages a measured walk crosses (the routing answer
//! names no traversed objects), and whether exit doors open in the direction
//! of escape (door leaves are not read yet).

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, FarthestPointOutcome, FarthestPointRequest,
    MetricPoint, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanArea,
    PlanSpanServiceHandle, ProximityProjection, ProximityRequest, ProximityServiceHandle,
    RuleCapability, RuleContext, TableColumn,
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
    TableColumn::optional("passage_width", ColumnKind::Number),
];

const SECTIONS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("objects", ColumnKind::Selector),
    TableColumn::required("factor", ColumnKind::Number),
    TableColumn::optional("shared_by", ColumnKind::Integer),
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

/// One row of `widths`.
struct WidthRow {
    /// The row covers loads up to this many occupants.
    occupants: u64,
    /// The least width of each exit.
    width: f64,
    /// The least width of all exits together.
    total: Option<f64>,
    /// The least clear width of each passage.
    passage: Option<f64>,
}

/// One row of `sections`: objects on which a walked metre counts `factor`
/// times.
struct SectionKind<'a> {
    name: String,
    objects: &'a Selector,
    factor: f64,
    /// Only a section at least this many checked spaces reach multiplies.
    shared_by: Option<usize>,
}

/// The passages a space's occupants rely on.
struct Passages<'a> {
    path: Option<Traversal<'a>>,
    selector: &'a Selector,
    width: Option<PropertyRef<'a>>,
}

struct Declaration<'a> {
    uses: Vec<Use<'a>>,
    /// By occupants.
    widths: Vec<WidthRow>,
    sections: Vec<SectionKind<'a>>,
    section_path: Option<Traversal<'a>>,
    passages: Option<Passages<'a>>,
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
        let passage = positive(&name, "passage_width", row.number("passage_width")?)?;
        widths.push(WidthRow {
            occupants,
            width,
            total,
            passage,
        });
    }
    widths.sort_by_key(|row| row.occupants);
    if widths
        .windows(2)
        .any(|pair| pair[0].occupants == pair[1].occupants)
    {
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
    let (sections, section_path) = sections(&parameters, &uses)?;
    let passages = passages(&parameters, &widths)?;
    Ok(Declaration {
        uses,
        widths,
        sections,
        section_path,
        passages,
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

type Sections<'a> = (Vec<SectionKind<'a>>, Option<Traversal<'a>>);

fn sections<'a>(
    parameters: &Parameters<'a>,
    uses: &[Use<'_>],
) -> Result<Sections<'a>, Unavailable> {
    let mut sections = Vec::new();
    for (index, row) in parameters
        .table("sections")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = match row.text("label")? {
            Some(label) => format!("section {index} ({label})"),
            None => format!("section {index}"),
        };
        let objects = row
            .selector("objects")?
            .ok_or_else(|| invalid(format!("{name} has no `objects`")))?;
        let factor = row
            .number("factor")?
            .filter(|factor| factor.is_finite() && *factor >= 1.0)
            .ok_or_else(|| invalid(format!("{name}: `factor` must be at least 1")))?;
        let shared_by = row
            .integer("shared_by")?
            .map(|count| {
                usize::try_from(count)
                    .ok()
                    .filter(|count| *count >= 2)
                    .ok_or_else(|| invalid(format!("{name}: `shared_by` must be at least 2")))
            })
            .transpose()?;
        sections.push(SectionKind {
            name,
            objects,
            factor,
            shared_by,
        });
    }
    let path = parameters
        .strings("section_path")?
        .map(Traversal::path)
        .transpose()?;
    let shared = sections.iter().any(|kind| kind.shared_by.is_some());
    if shared && path.is_none() {
        return Err(invalid("`shared_by` needs `section_path`"));
    }
    if !shared && path.is_some() {
        return Err(invalid(
            "`section_path` needs a section stating `shared_by`",
        ));
    }
    if !sections.is_empty() && uses.iter().all(|use_| use_.maximum_travel.is_none()) {
        return Err(invalid("`sections` needs a use stating `maximum_travel`"));
    }
    Ok((sections, path))
}

fn passages<'a>(
    parameters: &Parameters<'a>,
    widths: &[WidthRow],
) -> Result<Option<Passages<'a>>, Unavailable> {
    let path = parameters.strings("passage_path")?;
    let width = parameters.property("passage_width_property")?;
    let Some(selector) = parameters.selector("passage_selector")? else {
        if path.is_some() || width.is_some() || widths.iter().any(|row| row.passage.is_some()) {
            return Err(invalid(
                "`passage_path`, `passage_width_property` and `passage_width` need \
                 `passage_selector`",
            ));
        }
        return Ok(None);
    };
    if widths.is_empty() || widths.iter().any(|row| row.passage.is_none()) {
        return Err(invalid(
            "`passage_selector` needs every row of `widths` to state `passage_width`",
        ));
    }
    Ok(Some(Passages {
        path: path.map(Traversal::path).transpose()?,
        selector,
        width,
    }))
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
            ParameterDescriptor::optional("sections", ParameterType::Table(SECTIONS)),
            ParameterDescriptor::optional("section_path", ParameterType::StringList),
            ParameterDescriptor::optional("passage_path", ParameterType::StringList),
            ParameterDescriptor::optional("passage_selector", ParameterType::Selector),
            ParameterDescriptor::optional(
                "passage_width_property",
                ParameterType::PropertyReference,
            ),
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
        let passages = declared
            .passages
            .as_ref()
            .map(|passages| Candidates::select(context, passages.selector));
        let judge = Judge {
            context,
            rule,
            declared: &declared,
            exits,
            doors,
            sections: possible_sections(context, rule, &declared),
            passages,
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        let mut served = Served::default();
        let mut results: Vec<(ObjectId, String, Checked)> = Vec::new();
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
                    judge.serve(&space.id, Err("no row of `uses` picks it"), &mut served);
                    continue;
                }
                Matched::Undecided | Matched::Ambiguous(_) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "escape-route: whether a row of `uses` picks this space is undecided",
                    );
                    judge.serve(
                        &space.id,
                        Err("whether a row of `uses` picks it is undecided"),
                        &mut served,
                    );
                    continue;
                }
            };
            let mut checked = Checked::default();
            let load = judge.space(space, use_, &mut checked);
            let load = match &load {
                Some(Ok(load)) => Ok(load),
                Some(Err(_)) => Err("its footprint is not measured"),
                None => Err("its use states no `area_per_occupant`"),
            };
            if let Some(doubt) = judge.serve(&space.id, load, &mut served) {
                checked.doubts.push(doubt);
            }
            results.push((
                space.id.clone(),
                format!("escape-route {}", use_.name),
                checked,
            ));
        }
        if judge.passages.is_some() {
            // A space the rule may select brings occupants nobody counted.
            for space in Candidates::select(context, &rule.selector).undecided.keys() {
                judge.serve(
                    space,
                    Err("whether the rule selects it is undecided"),
                    &mut served,
                );
            }
            judge.judge_passages(served, &mut results);
        }
        for (subject, prefix, checked) in results {
            emit(&mut evaluation, subject, &prefix, checked);
        }
        evaluation
    }
}

/// Records what checking one object found.
fn emit(evaluation: &mut CapabilityEvaluation, subject: ObjectId, prefix: &str, checked: Checked) {
    for found in checked.findings {
        evaluation.push_finding(found);
    }
    if checked.doubts.is_empty() {
        return;
    }
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
        subject,
        reason,
        format!("{prefix}: {}", messages.join("; ")),
    );
}

/// A section object the walk may cross, and its factor.
struct Section {
    object: ObjectId,
    factor: f64,
    /// Its row of `sections`.
    kind: usize,
}

/// Every object that may be a multiplying section: picked (or perhaps
/// picked) by a row of `sections`, and, with `shared_by`, reached from
/// enough checked spaces that it may be shared. A factor of one changes
/// nothing and is left out.
fn possible_sections(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
) -> Vec<Section> {
    if declared.sections.is_empty() {
        return Vec::new();
    }
    let candidates: Vec<Candidates<'_>> = declared
        .sections
        .iter()
        .map(|kind| Candidates::select(context, kind.objects))
        .collect();
    // How many checked spaces may reach each shared section; a space whose
    // sections cannot be read may reach any.
    let mut reaching: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let mut unread = 0;
    if let Some(path) = &declared.section_path {
        let mut universe: Vec<&Object> = Vec::new();
        for (kind, candidates) in declared.sections.iter().zip(&candidates) {
            if kind.shared_by.is_some() {
                for object in &candidates.universe {
                    if !universe.iter().any(|known| known.id == object.id) {
                        universe.push(object);
                    }
                }
            }
        }
        for space in Candidates::select(context, &rule.selector).universe {
            match path.related(context, &space.id, &universe) {
                Ok((reached, _)) => {
                    for object in reached {
                        *reaching.entry(object).or_default() += 1;
                    }
                }
                Err(_) => unread += 1,
            }
        }
    }
    let mut sections = Vec::new();
    for (index, (kind, candidates)) in declared.sections.iter().zip(&candidates).enumerate() {
        if kind.factor <= 1.0 {
            continue;
        }
        for object in &candidates.universe {
            if let Some(needed) = kind.shared_by
                && reaching.get(&object.id).copied().unwrap_or(0) + unread < needed
            {
                continue;
            }
            sections.push(Section {
                object: object.id.clone(),
                factor: kind.factor,
                kind: index,
            });
        }
    }
    sections
}

/// A space's occupant load, as an interval, and the area it comes from.
struct Load {
    least: u64,
    most: u64,
    area: PlanArea,
}

/// The occupants relying on one passage.
#[derive(Default)]
struct Reliance {
    least: u64,
    most: u64,
    /// Why more occupants may rely on it than `most`.
    unbounded: Vec<String>,
    /// The checked spaces it serves.
    spaces: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

/// The occupants relying on every passage reached.
#[derive(Default)]
struct Served {
    passages: BTreeMap<ObjectId, Reliance>,
    /// Why any passage may serve more occupants: a space whose passages
    /// cannot be read.
    anywhere: Vec<String>,
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

/// The least and the largest of `values`.
fn span(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, 0.0_f64), |(low, high), value| {
        (low.min(value), high.max(value))
    })
}

/// An occupant load for a message.
fn occupants(least: u64, most: u64) -> String {
    if least == most {
        format!("{least} occupant(s)")
    } else {
        format!("between {least} and {most} occupants")
    }
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
    sections: Vec<Section>,
    passages: Option<Candidates<'c>>,
}

/// What bounds a clear width from above where none is stated.
#[derive(Clone, Copy)]
enum Bound {
    /// The footprint's longest plan diagonal.
    Diagonal,
    /// The shorter side of the rectangle of least area enclosing the
    /// footprint.
    ShortSide,
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

    /// Checks one space, and answers its occupant load where its use
    /// states one.
    fn space(
        &self,
        space: &Object,
        use_: &Use<'_>,
        checked: &mut Checked,
    ) -> Option<Result<Load, Unavailable>> {
        let load = use_
            .area_per_occupant
            .map(|per_occupant| Self::load(self.context, &space.id, per_occupant));
        let exits = match self.reached(&self.declared.exits, &self.exits, &space.id) {
            Ok(exits) => exits,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return load;
            }
        };
        if let Some(required) = use_.exits {
            self.count(space, use_, required, &exits, checked);
        }
        if let (Some(per_occupant), Some(load)) = (use_.area_per_occupant, &load) {
            match load {
                Ok(load) => self.widths(space, use_, per_occupant, load, &exits, checked),
                Err(unavailable) => checked.doubts.push(unavailable.clone()),
            }
        }
        if let Some(maximum) = use_.maximum_travel {
            self.travel(space, use_, maximum, &exits, checked);
        }
        load
    }

    /// The footprint over the area per occupant, rounded up at both ends.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn load(
        context: &RuleContext<'_>,
        space: &ObjectId,
        per_occupant: f64,
    ) -> Result<Load, Unavailable> {
        let area = footprint(context, space)?;
        let load = |area: f64| (area / per_occupant).ceil().max(1.0) as u64;
        Ok(Load {
            least: load(area.lower_square_metres()),
            most: load(area.upper_square_metres()),
            area,
        })
    }

    /// Adds `space`'s occupants to every passage it reaches; answers why its
    /// passages cannot be read.
    fn serve(
        &self,
        space: &ObjectId,
        load: Result<&Load, &str>,
        served: &mut Served,
    ) -> Option<Unavailable> {
        let (Some(declared), Some(candidates)) =
            (self.declared.passages.as_ref(), self.passages.as_ref())
        else {
            return None;
        };
        let mut reached = Vec::new();
        let mut evidence = Vec::new();
        if candidates.universe.iter().any(|object| object.id == *space) {
            reached.push(space.clone());
        }
        if let Some(path) = &declared.path {
            match path.related(self.context, space, &candidates.universe) {
                Ok((found, cited)) => {
                    reached.extend(found);
                    evidence = cited;
                }
                Err((_, message)) => {
                    served
                        .anywhere
                        .push(format!("the passages of {space} cannot be read: {message}"));
                    return Some(incomplete(format!(
                        "its passages cannot be read: {message}"
                    )));
                }
            }
        }
        for passage in reached {
            let reliance = served.passages.entry(passage).or_default();
            match load {
                Ok(load) => {
                    reliance.least += load.least;
                    reliance.most += load.most;
                    reliance.evidence.push(load.area.evidence().clone());
                }
                Err(why) => reliance.unbounded.push(format!("{space}: {why}")),
            }
            reliance.spaces.push(space.clone());
            reliance.evidence.extend(evidence.iter().cloned());
        }
        None
    }

    /// Judges every passage a checked space reaches, adding the outcome to
    /// that of the passage where it is a checked space itself.
    fn judge_passages(&self, served: Served, results: &mut Vec<(ObjectId, String, Checked)>) {
        for (passage, reliance) in served.passages {
            let mut checked = Checked::default();
            self.passage(&passage, &reliance, &served.anywhere, &mut checked);
            if let Some((_, _, own)) = results.iter_mut().find(|(id, _, _)| *id == passage) {
                own.findings.extend(checked.findings);
                own.doubts.extend(checked.doubts);
            } else {
                results.push((passage, "escape-route passage".to_owned(), checked));
            }
        }
    }

    fn passage(
        &self,
        passage: &ObjectId,
        reliance: &Reliance,
        anywhere: &[String],
        checked: &mut Checked,
    ) {
        let (Some(declared), Some(candidates)) =
            (self.declared.passages.as_ref(), self.passages.as_ref())
        else {
            return;
        };
        let unknown: Vec<&str> = reliance
            .unbounded
            .iter()
            .chain(anywhere)
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            checked.doubts.push(incomplete(format!(
                "the occupants relying on passage {passage} are unknown: {}",
                unknown.join("; ")
            )));
            return;
        }
        let (least, most) = (reliance.least, reliance.most);
        let required = self.rows(least, most).and_then(|rows| {
            rows.iter()
                .map(|row| row.passage)
                .collect::<Option<Vec<f64>>>()
                .map(|widths| span(widths.into_iter()))
                .ok_or_else(|| "a row of `widths` states no `passage_width`".to_owned())
        });
        let (low, high) = match required {
            Ok(required) => required,
            Err(why) => {
                checked.doubts.push(incomplete(why));
                return;
            }
        };
        let spaces: Vec<String> = reliance.spaces.iter().map(ToString::to_string).collect();
        let basis = format!(
            "{} relying on it (from {}) require at least {} m",
            occupants(least, most),
            spaces.join(", "),
            shown(low, high)
        );
        let (what, cited) = match self.width(passage, declared.width, Bound::ShortSide) {
            Width::Stated(width, cited) if width < low => (
                format!("passage {passage} is {width} m wide (stated clear width)"),
                cited,
            ),
            Width::Stated(width, _) if width >= high => return,
            Width::Stated(width, _) => {
                checked.doubts.push(incomplete(format!(
                    "passage {passage} is {width} m wide, and {basis}"
                )));
                return;
            }
            Width::AtMost(bound, cited) if bound < low => (
                format!(
                    "passage {passage} is at most {} m wide (the shorter side of the rectangle \
                     enclosing its footprint)",
                    shown(bound, bound)
                ),
                vec![cited],
            ),
            Width::AtMost(..) => {
                checked.doubts.push(incomplete(format!(
                    "the clear width of passage {passage} is not stated, and {basis}"
                )));
                return;
            }
            Width::Unknown(why) => {
                checked.doubts.push(incomplete(format!(
                    "the width of passage {passage} is unknown: {why}"
                )));
                return;
            }
        };
        if let Some(why) = candidates.undecided.get(passage) {
            checked.doubts.push(incomplete(format!(
                "whether {passage} is a passage is undecided ({why}), and it may be too narrow"
            )));
            return;
        }
        let mut evidence = reliance.evidence.clone();
        evidence.extend(cited);
        checked.findings.push(finding(
            self.rule,
            passage,
            format!("{what}; {basis}"),
            evidence,
            reliance.spaces.clone(),
        ));
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

    /// An exit's or passage's clear width: stated in `property` (and the
    /// parameter declaring it), else bounded by its footprint.
    fn width(&self, exit: &ObjectId, property: Option<PropertyRef<'_>>, bound: Bound) -> Width {
        let mut why = match (property, bound) {
            (Some(_), _) => String::new(),
            (None, Bound::Diagonal) => "no `clear_width_property` is declared".to_owned(),
            (None, Bound::ShortSide) => "no `passage_width_property` is declared".to_owned(),
        };
        if let Some(property) = property {
            match self.context.project.object(exit) {
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
        match bound {
            Bound::Diagonal => match spans.measure_diameter(exit) {
                Ok(diameter) => Width::AtMost(diameter.upper_metres(), diameter.evidence().clone()),
                Err(error) => Width::Unknown(format!("{why}, and its footprint: {error}")),
            },
            // A body that passes stands on a disc of its width inside the
            // footprint, so the footprint is at least that wide across in
            // every direction, and so is any rectangle enclosing it.
            Bound::ShortSide => match spans.measure_rectangle(exit) {
                Ok(rectangle) => match rectangle.width_and_length() {
                    Ok([(_, width), _]) => Width::AtMost(width, rectangle.evidence().clone()),
                    Err(reason) => Width::Unknown(format!("{why}, and {reason}")),
                },
                Err(error) => Width::Unknown(format!("{why}, and its footprint: {error}")),
            },
        }
    }

    /// The rows of `widths` that may apply to a load between `least` and
    /// `most` occupants.
    fn rows(&self, least: u64, most: u64) -> Result<&[WidthRow], String> {
        let rows = &self.declared.widths;
        let from = rows
            .iter()
            .position(|row| row.occupants >= least)
            .ok_or_else(|| format!("no row of `widths` covers {least} occupant(s)"))?;
        let to = rows
            .iter()
            .position(|row| row.occupants >= most)
            .ok_or_else(|| format!("no row of `widths` covers {most} occupant(s)"))?;
        Ok(&rows[from..=to])
    }

    /// The widths `widths` requires for any load between `least` and
    /// `most` occupants: of each exit, and of all together where the rows
    /// state it.
    fn required_width(&self, least: u64, most: u64) -> Result<Required, String> {
        let rows = self.rows(least, most)?;
        let totals: Vec<f64> = rows.iter().filter_map(|row| row.total).collect();
        let total = if totals.is_empty() {
            None
        } else if totals.len() == rows.len() {
            Some(span(totals.into_iter()))
        } else {
            return Err(format!(
                "the rows of `widths` for {least} to {most} occupants state `total_width` \
                 only in part"
            ));
        };
        Ok(Required {
            each: span(rows.iter().map(|row| row.width)),
            total,
        })
    }

    fn widths(
        &self,
        space: &Object,
        use_: &Use<'_>,
        per_occupant: f64,
        load: &Load,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let Load { least, most, area } = load;
        let (least, most) = (*least, *most);
        let occupants = occupants(least, most);
        let required = match self.required_width(least, most) {
            Ok(required) => required,
            Err(why) => {
                checked.doubts.push(incomplete(why));
                return;
            }
        };
        let (low, high) = required.each;
        let width = |exit| self.width(exit, self.declared.clear_width, Bound::Diagonal);
        let sure: Vec<Width> = exits.sure.iter().map(width).collect();
        let maybe: Vec<Width> = exits.maybe.iter().map(width).collect();
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
            self.total_width(space, use_, &occupants, area, total, exits, widths, checked);
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
        // Sections multiply the walk by at least one, so the plain walk's
        // lower bound stands; its upper bound grows by the largest factor
        // of a section it may cross.
        let mut most = 0.0_f64;
        let mut multiplied = 1.0_f64;
        let mut crossed = BTreeSet::new();
        let mut worst: Option<(Option<ObjectId>, Travel)> = None;
        for (start, sure_start, [lower, upper]) in measured {
            match upper {
                Ok(upper) => {
                    let from = start.as_ref().unwrap_or(&space.id);
                    let (factor, kinds) = self.factor(from, upper.upper, maximum);
                    most = most.max(upper.upper * factor);
                    if !kinds.is_empty() {
                        multiplied = multiplied.max(factor);
                        crossed.extend(kinds);
                    }
                }
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
        let counted = if crossed.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = crossed
                .iter()
                .map(|kind: &usize| self.declared.sections[*kind].name.as_str())
                .collect();
            format!(
                ", counting the walk on {} up to {multiplied} times",
                names.join(", ")
            )
        };
        checked.doubts.extend(doubts);
        checked.doubts.push(incomplete(format!(
            "the longest travel to the nearest exit is {} m walking{counted}, and {allows}",
            if most.is_finite() {
                shown(least, most)
            } else {
                format!("at least {}", shown(least, least))
            }
        )));
    }

    /// The largest factor a walk of at most `reach` metres from `from` may
    /// count its metres by, and the rows of `sections` it may cross.
    ///
    /// Such a walk stays within `reach` of `from` in plan, so a section
    /// surely farther away than that is not crossed. A section that cannot
    /// be measured may be.
    fn factor(&self, from: &ObjectId, reach: f64, maximum: f64) -> (f64, BTreeSet<usize>) {
        let largest = self
            .sections
            .iter()
            .map(|section| section.factor)
            .fold(1.0_f64, f64::max);
        if !reach.is_finite() || reach * largest <= maximum {
            // Unbounded either way, or within the maximum at any factor.
            return (largest, BTreeSet::new());
        }
        if reach > maximum {
            // No factor makes this start pass: every section may count.
            return (
                largest,
                self.sections.iter().map(|section| section.kind).collect(),
            );
        }
        let proximity = self.context.services.get::<ProximityServiceHandle>();
        let mut factor = 1.0_f64;
        let mut kinds = BTreeSet::new();
        for section in &self.sections {
            let far = section.object != *from
                && proximity.is_some_and(|proximity| {
                    ProximityRequest::projected(
                        from.clone(),
                        section.object.clone(),
                        ProximityProjection::Horizontal,
                    )
                    .and_then(|request| proximity.measure_distance(&request))
                    .is_ok_and(|distance| distance.interval_metres().0 > reach)
                });
            if !far {
                factor = factor.max(section.factor);
                kinds.insert(section.kind);
            }
        }
        (factor, kinds)
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
