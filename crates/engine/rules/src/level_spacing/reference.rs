//! `level-spacing` as it was implemented before it became a template
//! (#290), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, VerticalExtent,
};
use axioval_ir::{ObjectId, QuantityDimension, ReportColumn, ReportTable, ReportValue, RuleId};

use super::{
    Config, Height, Level, Reach, Side, extent, extents, heights, levels, metres, parse,
    prevailing, reached, shown,
};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, traversal_parameters};

impl Height<'_, '_> {
    fn shown(&self) -> String {
        shown(self.lower, self.upper)
    }

    fn related(&self) -> Vec<ObjectId> {
        self.above.iter().cloned().collect()
    }
}

/// Checks the height of each level, as `level-spacing` did before it ran
/// as a template.
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
