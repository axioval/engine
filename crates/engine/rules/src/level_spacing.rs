//! The spacing between consecutive levels: storey heights.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

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
/// up, so the highest member has none: it is not evaluated unless
/// `ignore_highest` is set, since its height needs geometry this rule does
/// not read. `ignore_lowest` leaves out the lowest member (a basement or
/// foundation level).
///
/// `minimum` and `maximum` bound each height, inclusive. With `consistent`,
/// every checked height must equal the prevailing one within
/// `tolerance` (1 mm by default); the prevailing height is the one most
/// members share, the lowest among equally common ones.
pub struct LevelSpacing;

struct Level<'a> {
    object: &'a Object,
    elevation: f64,
    evidence: Vec<Evidence>,
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
    traversal: Option<Traversal<'a>>,
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
        for anchor in anchors {
            match levels(context, &config, anchor) {
                Ok(levels) => check(rule, &config, &levels, &mut evaluation),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                }
            }
        }
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
    if minimum.is_none() && maximum.is_none() && !consistent {
        return Err(invalid("declare a minimum, a maximum or consistent"));
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
        let mut evidence = relation_evidence.clone();
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

fn metres(value: f64) -> String {
    format!("{} m", (value * 1e6).round() / 1e6)
}

fn check(
    rule: &CompiledRule,
    config: &Config<'_>,
    levels: &[Level<'_>],
    evaluation: &mut CapabilityEvaluation,
) {
    let skip = usize::from(config.ignore_lowest);
    let mut heights = Vec::new();
    for (index, level) in levels.iter().enumerate().skip(skip) {
        let Some(above) = levels.get(index + 1) else {
            if !config.ignore_highest {
                evaluation.push_object_not_evaluated(
                    level.object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "the highest level has no level above it; its height needs geometry",
                );
            }
            continue;
        };
        let height = above.elevation - level.elevation;
        let mut evidence = level.evidence.clone();
        evidence.extend(above.evidence.iter().cloned());
        let bound = match (config.minimum, config.maximum) {
            (Some(minimum), _) if height < minimum => Some(format!("at least {}", metres(minimum))),
            (_, Some(maximum)) if height > maximum => Some(format!("at most {}", metres(maximum))),
            _ => None,
        };
        if let Some(bound) = bound {
            evaluation.push_finding(finding(
                rule,
                &level.object.id,
                format!("level height is {}; required {bound}", metres(height)),
                evidence.clone(),
                vec![above.object.id.clone()],
            ));
        }
        heights.push((level, above, height, evidence));
    }
    if !config.consistent || heights.len() < 2 {
        return;
    }
    // Heights within the tolerance of one another count as one.
    let step = config.tolerance.max(f64::EPSILON);
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    #[allow(clippy::cast_possible_truncation)]
    let key = |height: f64| (height / step).round() as i64;
    for (_, _, height, _) in &heights {
        *counts.entry(key(*height)).or_default() += 1;
    }
    let prevailing = counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
        .map(|(key, _)| *key)
        .expect("at least two heights were measured");
    let reference = heights
        .iter()
        .find(|(_, _, height, _)| key(*height) == prevailing)
        .map(|(_, _, height, _)| *height)
        .expect("the prevailing height was measured");
    for (level, above, height, evidence) in &heights {
        if (height - reference).abs() > config.tolerance {
            evaluation.push_finding(finding(
                rule,
                &level.object.id,
                format!(
                    "level height {} differs from the prevailing {}",
                    metres(*height),
                    metres(reference)
                ),
                evidence.clone(),
                vec![above.object.id.clone()],
            ));
        }
    }
}
