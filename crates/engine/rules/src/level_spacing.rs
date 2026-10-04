//! The spacing between consecutive levels: storey heights.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, VerticalExtent, VerticalExtentError, VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{
    Evidence, Object, ObjectId, PropertyValue, QuantityDimension, ReportColumn, ReportTable,
    ReportValue, RuleId,
};

mod measured;

pub(crate) use measured::LevelMeasures;

use crate::counts::{Population, tally};
use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, finding, invalid, resolve,
    traversal_parameters,
};

/// Checks the height of each level: the rise of `order` to the next level up.
///
/// Anchors (buildings) and members (storeys) work as in `name-sequence`:
/// members are the objects `member_selector` picks that the traversal
/// reaches from an anchor, or every such object in the anchor's source.
/// `order` must be a length on every member, typically the storey's
/// elevation in SI. A member's height is the difference to the next member
/// up. `ignore_lowest` leaves out the lowest member (a basement or
/// foundation level).
///
/// The highest member has no member above it. With `content_path`, its
/// height is measured from geometry instead: the highest top, through the
/// vertical-extent service, of the objects that path reaches from it
/// (restricted to `content_selector` when declared), less its own `order`.
/// Without it, the highest member is not evaluated unless `ignore_highest`
/// is set.
///
/// `minimum` and `maximum` bound each height, inclusive. With `consistent`,
/// every checked height must equal the prevailing one within
/// `tolerance` (1 mm by default); the prevailing height is the one most
/// members share, the lowest among equally common ones.
///
/// With `space_selector`, the spaces `space_path` reaches from each checked
/// member (a storey's spaces, say) must each be as high as the member,
/// within `space_tolerance`: a space's height is the rise from its bottom to
/// its top, through the vertical-extent service. `space_height: false`
/// leaves that comparison out.
///
/// With `space_elevation` (`bottom`, `top` or `both`), the spaces of each
/// member must share their bottom (or top) elevation within
/// `space_tolerance`: a space whose elevation lies surely beyond the
/// tolerance of the prevailing one (the one most of the member's spaces
/// share, the lowest among equally common ones) is found. Only exact
/// elevations decide the prevailing one.
///
/// A height measured from geometry is an interval; a verdict needs the whole
/// interval on one side of a bound, and one straddling it is not evaluated.
///
/// Every run reports what it measured beside its findings, whether or not
/// it passed: the table `levels` has one row per level with its `elevation`
/// and `height` (unknown for a level not measured), and, with
/// `space_selector`, the table `spaces` one row per measured space with its
/// `level`, its `height` and the `level_height` it was compared with.
pub struct LevelSpacing;

/// The tables a run reports: levels, and spaces when they are compared.
struct Tables {
    levels: ReportTable,
    spaces: ReportTable,
}

impl Tables {
    fn new(rule: &RuleId) -> Self {
        let length = |id| ReportColumn::quantity(id, QuantityDimension::Length);
        Self {
            levels: ReportTable::new(
                rule.clone(),
                "levels",
                vec![length("elevation"), length("height")],
            )
            .expect("the level table's columns are valid"),
            spaces: ReportTable::new(
                rule.clone(),
                "spaces",
                vec![
                    ReportColumn::text("level"),
                    length("height"),
                    length("level_height"),
                ],
            )
            .expect("the space table's columns are valid"),
        }
    }
}

struct Level<'a> {
    object: &'a Object,
    elevation: f64,
    evidence: Vec<Evidence>,
}

/// A level's height as an interval, with what it was measured from.
struct Height<'l, 'a> {
    level: &'l Level<'a>,
    /// The next level up, when the height is the rise to it.
    above: Option<ObjectId>,
    lower: f64,
    upper: f64,
    evidence: Vec<Evidence>,
}

impl Height<'_, '_> {
    fn shown(&self) -> String {
        shown(self.lower, self.upper)
    }

    fn related(&self) -> Vec<ObjectId> {
        self.above.iter().cloned().collect()
    }
}

/// The objects a traversal reaches from each level, among a selection.
struct Reach<'a> {
    traversal: Traversal,
    selector: Option<&'a Selector>,
}

struct Config<'a> {
    members: &'a Selector,
    order: PropertyRef<'a>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    consistent: bool,
    tolerance: f64,
    ignore_lowest: bool,
    ignore_highest: bool,
    traversal: Option<Traversal>,
    contents: Option<Reach<'a>>,
    spaces: Option<(Reach<'a>, f64)>,
    space_checks: SpaceChecks,
}

/// What the spaces a level reaches are checked for.
struct SpaceChecks {
    /// Whether each space is compared with its level's height.
    height: bool,
    /// Which elevations the spaces of one level must share.
    elevation: Vec<Side>,
}

/// A space's bottom or top.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Bottom,
    Top,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
        }
    }
}

impl RuleCapability for LevelSpacing {
    fn id(&self) -> &'static str {
        "axioval:capability.level-spacing"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("member_selector", ParameterType::Selector),
            ParameterDescriptor::required("order", ParameterType::PropertyReference),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("consistent", ParameterType::Boolean),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("ignore_lowest", ParameterType::Boolean),
            ParameterDescriptor::optional("ignore_highest", ParameterType::Boolean),
            ParameterDescriptor::optional("content_path", ParameterType::StringList),
            ParameterDescriptor::optional("content_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_path", ParameterType::StringList),
            ParameterDescriptor::optional("space_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("space_height", ParameterType::Boolean),
            ParameterDescriptor::optional("space_elevation", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("level-spacing: {message}"),
                );
            }
        };
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let mut tables = Tables::new(&rule.id);
        for anchor in anchors {
            match levels(context, &config, anchor) {
                Ok(levels) => check(
                    context,
                    rule,
                    &config,
                    &levels,
                    &mut evaluation,
                    &mut tables,
                ),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                }
            }
        }
        evaluation.push_table(tables.levels);
        evaluation.push_table(tables.spaces);
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name| match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
    };
    let (minimum, maximum) = (length("minimum")?, length("maximum")?);
    let consistent = parameters.boolean("consistent")?.unwrap_or(false);
    let contents = match (
        parameters.strings("content_path")?,
        parameters.selector("content_selector")?,
    ) {
        (None, None) => None,
        (None, Some(_)) => return Err(invalid("`content_selector` needs a `content_path`")),
        (Some(path), selector) => Some(Reach {
            traversal: Traversal::path(path)?,
            selector,
        }),
    };
    let spaces = match (
        parameters.selector("space_selector")?,
        parameters.strings("space_path")?,
        length("space_tolerance")?,
    ) {
        (None, None, None) => None,
        (Some(selector), Some(path), Some(tolerance)) => Some((
            Reach {
                traversal: Traversal::path(path)?,
                selector: Some(selector),
            },
            tolerance,
        )),
        _ => {
            return Err(invalid(
                "`space_selector`, `space_path` and `space_tolerance` go together",
            ));
        }
    };
    let space_height = parameters.boolean("space_height")?;
    let space_elevation = match parameters.string("space_elevation")? {
        None => Vec::new(),
        Some("bottom") => vec![Side::Bottom],
        Some("top") => vec![Side::Top],
        Some("both") => vec![Side::Bottom, Side::Top],
        Some(other) => {
            return Err(invalid(format!(
                "`space_elevation` is `{other}`, not `bottom`, `top` or `both`"
            )));
        }
    };
    if spaces.is_none() && (space_height.is_some() || !space_elevation.is_empty()) {
        return Err(invalid(
            "`space_height` and `space_elevation` need `space_selector`, `space_path` and \
             `space_tolerance`",
        ));
    }
    let space_height = space_height.unwrap_or(true);
    if spaces.is_some() && !space_height && space_elevation.is_empty() {
        return Err(invalid(
            "with `space_height` false, the spaces need a `space_elevation` to check",
        ));
    }
    if minimum.is_none() && maximum.is_none() && !consistent && spaces.is_none() {
        return Err(invalid(
            "declare a minimum, a maximum, consistent or a space_selector",
        ));
    }
    if matches!((minimum, maximum), (Some(minimum), Some(maximum)) if minimum > maximum) {
        return Err(invalid("minimum exceeds maximum"));
    }
    Ok(Config {
        members: parameters.required_selector("member_selector")?,
        order: parameters.required_property("order")?,
        minimum,
        maximum,
        consistent,
        tolerance: length("tolerance")?.unwrap_or(1e-3),
        ignore_lowest: parameters.boolean("ignore_lowest")?.unwrap_or(false),
        ignore_highest: parameters.boolean("ignore_highest")?.unwrap_or(false),
        traversal: parameters.traversal()?,
        contents,
        spaces,
        space_checks: SpaceChecks {
            height: space_height,
            elevation: space_elevation,
        },
    })
}

/// The anchor's levels, lowest first, or why they cannot be ordered.
fn levels<'a>(
    context: &RuleContext<'a>,
    config: &Config<'_>,
    anchor: &Object,
) -> Result<Vec<Level<'a>>, Unavailable> {
    let (candidates, outcomes) = select_objects(context, config.members);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            outcome.reason().clone(),
            format!("member selection is undecided: {}", outcome.message()),
        ));
    }
    let (reached, relation_evidence): (Vec<ObjectId>, Vec<Evidence>) = match &config.traversal {
        Some(traversal) => traversal.related(context, &anchor.id, &candidates)?,
        None => (
            candidates
                .iter()
                .filter(|member| member.id.source == anchor.id.source && member.id != anchor.id)
                .map(|member| member.id.clone())
                .collect(),
            Vec::new(),
        ),
    };
    ordered(context, config, reached, &relation_evidence)
}

/// The levels `reached`, ordered lowest first by their `order` lengths, or
/// why they cannot be ordered.
fn ordered<'a>(
    context: &RuleContext<'a>,
    config: &Config<'_>,
    reached: Vec<ObjectId>,
    relation_evidence: &[Evidence],
) -> Result<Vec<Level<'a>>, Unavailable> {
    let mut levels = Vec::new();
    for id in reached {
        let object = context
            .project
            .object(&id)
            .ok_or_else(|| invalid(format!("member {id} is not in the project")))?;
        let order = resolve(context, object, config.order)?;
        let Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }) = order.value()
        else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{id} has no length {} ({}), so the levels cannot be measured",
                    config.order,
                    crate::support::display(order.value())
                ),
            ));
        };
        let mut evidence = relation_evidence.to_vec();
        evidence.extend(order.evidence());
        levels.push(Level {
            object,
            elevation: *value,
            evidence,
        });
    }
    levels.sort_by(|left, right| {
        left.elevation
            .total_cmp(&right.elevation)
            .then_with(|| left.object.id.cmp(&right.object.id))
    });
    Ok(levels)
}

pub(crate) fn metres(value: f64) -> String {
    format!("{} m", (value * 1e6).round() / 1e6)
}

/// A length interval as a reviewer reads it.
pub(crate) fn shown(lower: f64, upper: f64) -> String {
    let (low, high) = (metres(lower), metres(upper));
    if low == high {
        low
    } else {
        format!("between {low} and {high}")
    }
}

pub(crate) fn extents<'a>(
    context: &RuleContext<'a>,
) -> Result<&'a VerticalExtentServiceHandle, Unavailable> {
    context
        .services
        .get::<VerticalExtentServiceHandle>()
        .ok_or((
            NotEvaluatedReason::MissingService,
            "vertical-extent service is not registered".into(),
        ))
}

pub(crate) fn extent(
    service: &VerticalExtentServiceHandle,
    object: &ObjectId,
) -> Result<VerticalExtent, Unavailable> {
    service
        .measure_vertical_extent(object)
        .map_err(|error| match error {
            VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
                (NotEvaluatedReason::BackendUnavailable, error.to_string())
            }
            VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
                (NotEvaluatedReason::InvalidEvidence, error.to_string())
            }
        })
}

/// The objects `reach` finds from `level`, all decided, with the evidence.
fn reached(
    context: &RuleContext<'_>,
    reach: &Reach<'_>,
    level: &Object,
    what: &str,
) -> Result<(Vec<ObjectId>, Vec<Evidence>), Unavailable> {
    let population = Population::of(context, reach.selector.unwrap_or(&Selector::All));
    let tally = tally(context, Some(&reach.traversal), level, &population)?;
    if tally.undecided > 0 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} {what} via {} cannot be assigned",
                tally.undecided, reach.traversal.relationship
            ),
        ));
    }
    Ok((tally.decided, tally.evidence))
}

/// The highest level's height: the highest top of its contents, less its
/// elevation.
fn measured_height<'l, 'a>(
    context: &RuleContext<'_>,
    contents: &Reach<'_>,
    level: &'l Level<'a>,
) -> Result<Height<'l, 'a>, Unavailable> {
    let service = extents(context)?;
    let (objects, mut evidence) = reached(context, contents, level.object, "content(s)")?;
    if objects.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the highest level reaches no contents via {}, so its height cannot be measured",
                contents.traversal.relationship
            ),
        ));
    }
    let (mut lower, mut upper) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for object in &objects {
        let extent = extent(service, object)?;
        lower = lower.max(extent.top().lower_metres());
        upper = upper.max(extent.top().upper_metres());
        evidence.push(extent.evidence().clone());
    }
    evidence.extend(level.evidence.iter().cloned());
    Ok(Height {
        level,
        above: None,
        lower: lower - level.elevation,
        upper: upper - level.elevation,
        evidence,
    })
}

/// Every checked level's height, reporting the levels whose height is unknown.
fn heights<'l, 'a>(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    levels: &'l [Level<'a>],
    evaluation: &mut CapabilityEvaluation,
) -> Vec<Height<'l, 'a>> {
    let skip = usize::from(config.ignore_lowest);
    let mut heights = Vec::new();
    for (index, level) in levels.iter().enumerate().skip(skip) {
        if let Some(above) = levels.get(index + 1) {
            let height = above.elevation - level.elevation;
            let mut evidence = level.evidence.clone();
            evidence.extend(above.evidence.iter().cloned());
            heights.push(Height {
                level,
                above: Some(above.object.id.clone()),
                lower: height,
                upper: height,
                evidence,
            });
            continue;
        }
        if config.ignore_highest {
            continue;
        }
        let Some(contents) = &config.contents else {
            evaluation.push_object_not_evaluated(
                level.object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                "the highest level has no level above it; its height needs geometry",
            );
            continue;
        };
        match measured_height(context, contents, level) {
            Ok(height) => heights.push(height),
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(level.object.id.clone(), reason, message);
            }
        }
    }
    heights
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    config: &Config<'_>,
    levels: &[Level<'_>],
    evaluation: &mut CapabilityEvaluation,
    tables: &mut Tables,
) {
    // Only the spaces' shared elevations need no level height.
    let needs_heights = config.minimum.is_some()
        || config.maximum.is_some()
        || config.consistent
        || (config.spaces.is_some() && config.space_checks.height);
    let heights = if needs_heights {
        heights(context, config, levels, evaluation)
    } else {
        Vec::new()
    };
    for level in levels {
        let height = heights
            .iter()
            .find(|height| height.level.object.id == level.object.id)
            .map_or(ReportValue::Unknown, |height| {
                ReportValue::measured(height.lower, height.upper)
            });
        // A level two anchors reach keeps the row of the first.
        let _ = tables.levels.push_row(
            level.object.id.clone(),
            vec![ReportValue::exact(level.elevation), height],
        );
    }
    for height in &heights {
        let (fail, open) = match (config.minimum, config.maximum) {
            (Some(minimum), _) if height.upper < minimum => {
                (Some(format!("at least {}", metres(minimum))), None)
            }
            (_, Some(maximum)) if height.lower > maximum => {
                (Some(format!("at most {}", metres(maximum))), None)
            }
            (Some(minimum), _) if height.lower < minimum => {
                (None, Some(format!("at least {}", metres(minimum))))
            }
            (_, Some(maximum)) if height.upper > maximum => {
                (None, Some(format!("at most {}", metres(maximum))))
            }
            _ => (None, None),
        };
        if let Some(bound) = fail {
            evaluation.push_finding(finding(
                rule,
                &height.level.object.id,
                format!("level height is {}; required {bound}", height.shown()),
                height.evidence.clone(),
                height.related(),
            ));
        } else if let Some(bound) = open {
            evaluation.push_object_not_evaluated(
                height.level.object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "level height is {}, which straddles the bound {bound}",
                    height.shown()
                ),
            );
        }
    }
    if config.consistent {
        consistency(rule, config, &heights, evaluation);
    }
    if let Some((spaces, tolerance)) = &config.spaces
        && !config.space_checks.elevation.is_empty()
    {
        for level in levels {
            if let Err((reason, message)) = space_elevations(
                context,
                rule,
                (spaces, *tolerance),
                &config.space_checks.elevation,
                level,
                evaluation,
            ) {
                evaluation.push_object_not_evaluated(level.object.id.clone(), reason, message);
            }
        }
    }
    if let Some((spaces, tolerance)) = &config.spaces
        && config.space_checks.height
    {
        for height in &heights {
            match space_heights(
                context,
                rule,
                (spaces, *tolerance),
                height,
                evaluation,
                &mut tables.spaces,
            ) {
                Ok(()) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(
                        height.level.object.id.clone(),
                        reason,
                        message,
                    );
                }
            }
        }
    }
}

/// Flags heights away from the prevailing one. Only exact heights decide
/// which one prevails; a measured interval is judged against it.
fn consistency(
    rule: &CompiledRule,
    config: &Config<'_>,
    heights: &[Height<'_, '_>],
    evaluation: &mut CapabilityEvaluation,
) {
    #[allow(clippy::float_cmp)]
    let exact: Vec<f64> = heights
        .iter()
        .filter(|height| height.lower == height.upper)
        .map(|height| height.lower)
        .collect();
    if heights.len() < 2 {
        return;
    }
    let Some(reference) = prevailing(&exact, config.tolerance).map(|index| exact[index]) else {
        return;
    };
    for height in heights {
        let (below, above) = (reference - height.upper, height.lower - reference);
        if below > config.tolerance || above > config.tolerance {
            evaluation.push_finding(finding(
                rule,
                &height.level.object.id,
                format!(
                    "level height {} differs from the prevailing {}",
                    height.shown(),
                    metres(reference)
                ),
                height.evidence.clone(),
                height.related(),
            ));
        } else if reference - height.lower > config.tolerance
            || height.upper - reference > config.tolerance
        {
            evaluation.push_object_not_evaluated(
                height.level.object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "level height is {}, which may or may not match the prevailing {}",
                    height.shown(),
                    metres(reference)
                ),
            );
        }
    }
}

/// Requires the spaces of a level to share their bottom (or top) elevation:
/// each is judged against the prevailing exact elevation among them.
fn space_elevations(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    (spaces, tolerance): (&Reach<'_>, f64),
    sides: &[Side],
    level: &Level<'_>,
    evaluation: &mut CapabilityEvaluation,
) -> Result<(), Unavailable> {
    let (members, relation) = reached(context, spaces, level.object, "space(s)")?;
    if members.len() < 2 {
        return Ok(());
    }
    let service = extents(context)?;
    let mut measured = Vec::new();
    for space in members {
        match extent(service, &space) {
            Ok(extent) => measured.push((space, extent)),
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(space, reason, message);
            }
        }
    }
    for &side in sides {
        let interval = |extent: &VerticalExtent| {
            let elevation = match side {
                Side::Bottom => extent.bottom(),
                Side::Top => extent.top(),
            };
            (elevation.lower_metres(), elevation.upper_metres())
        };
        #[allow(clippy::float_cmp)]
        let exact: Vec<f64> = measured
            .iter()
            .map(|(_, extent)| interval(extent))
            .filter(|(lower, upper)| lower == upper)
            .map(|(lower, _)| lower)
            .collect();
        let Some(reference) = prevailing(&exact, tolerance).map(|index| exact[index]) else {
            for (space, _) in &measured {
                evaluation.push_object_not_evaluated(
                    space.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "no space of level {} has an exact {} elevation to compare with",
                        level.object.id,
                        side.name()
                    ),
                );
            }
            continue;
        };
        for (space, extent) in &measured {
            let (lower, upper) = interval(extent);
            let (below, above) = (reference - upper, lower - reference);
            let differs = format!(
                "space {} elevation is {}, and the prevailing {} elevation of the spaces of \
                 level {} is {}",
                side.name(),
                shown(lower, upper),
                side.name(),
                level.object.id,
                metres(reference)
            );
            if below > tolerance || above > tolerance {
                let mut evidence = level.evidence.clone();
                evidence.extend(relation.iter().cloned());
                evidence.push(extent.evidence().clone());
                evaluation.push_finding(finding(
                    rule,
                    space,
                    format!(
                        "{differs}; they may differ by at most {}",
                        metres(tolerance)
                    ),
                    evidence,
                    vec![level.object.id.clone()],
                ));
            } else if reference - lower > tolerance || upper - reference > tolerance {
                evaluation.push_object_not_evaluated(
                    space.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{differs}, which straddles the tolerance of {}",
                        metres(tolerance)
                    ),
                );
            }
        }
    }
    Ok(())
}

/// Requires each space of a level to be as high as the level.
fn space_heights(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    (spaces, tolerance): (&Reach<'_>, f64),
    height: &Height<'_, '_>,
    evaluation: &mut CapabilityEvaluation,
    table: &mut ReportTable,
) -> Result<(), Unavailable> {
    let (members, relation) = reached(context, spaces, height.level.object, "space(s)")?;
    if members.is_empty() {
        return Ok(());
    }
    let service = extents(context)?;
    for space in members {
        let extent = match extent(service, &space) {
            Ok(extent) => extent,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(space, reason, message);
                continue;
            }
        };
        let lower = extent.top().lower_metres() - extent.bottom().upper_metres();
        let upper = extent.top().upper_metres() - extent.bottom().lower_metres();
        // A space two levels reach keeps the row of the first.
        let _ = table.push_row(
            space.clone(),
            vec![
                ReportValue::text(height.level.object.id.to_string()),
                ReportValue::measured(lower, upper),
                ReportValue::measured(height.lower, height.upper),
            ],
        );
        let (least, most) = (lower - height.upper, upper - height.lower);
        let differs = format!(
            "space height is {} and its level's height {}",
            shown(lower, upper),
            height.shown()
        );
        if least > tolerance || most < -tolerance {
            let mut evidence = height.evidence.clone();
            evidence.extend(relation.iter().cloned());
            evidence.push(extent.evidence().clone());
            evaluation.push_finding(finding(
                rule,
                &space,
                format!(
                    "{differs}; they may differ by at most {}",
                    metres(tolerance)
                ),
                evidence,
                vec![height.level.object.id.clone()],
            ));
        } else if least < -tolerance || most > tolerance {
            evaluation.push_object_not_evaluated(
                space,
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{differs}, which straddles the tolerance of {}",
                    metres(tolerance)
                ),
            );
        }
    }
    Ok(())
}

/// The index of a value with the prevailing magnitude: the one most values
/// share, counting values within `tolerance` of one another as one, and the
/// lowest among equally common ones. `None` for no values.
pub(crate) fn prevailing(values: &[f64], tolerance: f64) -> Option<usize> {
    let step = tolerance.max(f64::EPSILON);
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    #[allow(clippy::cast_possible_truncation)]
    let key = |value: f64| (value / step).round() as i64;
    for value in values {
        *counts.entry(key(*value)).or_default() += 1;
    }
    let prevailing = counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
        .map(|(key, _)| *key)?;
    values.iter().position(|value| key(*value) == prevailing)
}
