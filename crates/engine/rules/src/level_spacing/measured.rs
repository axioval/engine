//! Level heights and the elevations of a level's spaces as values, measured
//! exactly as `level-spacing` measures them: its levels ordered by their
//! `order` lengths, each level's rise to the next one up (the highest from
//! its contents), the prevailing rise among an anchor's levels, and the
//! prevailing bottom or top elevation among a level's spaces.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension, RuleId};

use super::{
    Config, Height, Level, Reach, extent, extents, heights, levels, ordered, parse, prevailing,
    reached,
};
use crate::measured_kinds::{every_object_of_kinds, interval, refused};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// Measures `level_rise`, `prevailing_rise` and `prevailing_elevation`.
pub(crate) struct LevelMeasures;

const LEVEL_RISE: &str = "level_rise";
const PREVAILING_RISE: &str = "prevailing_rise";
const PREVAILING_ELEVATION: &str = "prevailing_elevation";
const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

fn steps<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c Vec<String>> {
    match call.argument(key) {
        Some(MeasuredArgument::Path(steps)) => Some(steps),
        _ => None,
    }
}

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// The objects of the kinds `key` names, the object measured included, as
/// a selector.
fn kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<Selector, Unavailable> {
    let objects = every_object_of_kinds(context, call, key)
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
    Ok(Selector::Objects { objects })
}

fn selector(value: Selector) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(value),
    }
}

fn path(steps: &[String]) -> ParameterValue {
    ParameterValue::StringList {
        value: steps.to_vec(),
    }
}

/// The `level-spacing` declaration a level call stands for: its levels,
/// order, anchor path, ignored ends, contents and consistency tolerance.
fn declaration(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<CompiledRule, Unavailable> {
    let Some(MeasuredArgument::Property { set, name }) = call.argument("order") else {
        return Err(invalid("`order` is required"));
    };
    let mut parameters = BTreeMap::from([
        (
            "member_selector".to_owned(),
            selector(kinds(context, call, "levels")?),
        ),
        (
            "order".to_owned(),
            ParameterValue::PropertyReference {
                property: name.clone(),
                property_set: set.clone(),
            },
        ),
        // The heights are measured whatever is judged of them.
        (
            "consistent".to_owned(),
            ParameterValue::Boolean { value: true },
        ),
        (
            "tolerance".to_owned(),
            ParameterValue::Quantity {
                value: length(call, "tolerance"),
                unit: "m".into(),
            },
        ),
        (
            "ignore_lowest".to_owned(),
            ParameterValue::Boolean {
                value: call.choice("lowest") == Some("ignored"),
            },
        ),
        (
            "ignore_highest".to_owned(),
            ParameterValue::Boolean {
                value: call.choice("highest") == Some("ignored"),
            },
        ),
    ]);
    if let Some(steps) = steps(call, "anchor") {
        parameters.insert("path".to_owned(), path(steps));
    }
    if let Some(steps) = steps(call, "contents") {
        parameters.insert("content_path".to_owned(), path(steps));
        if call.argument("content_kinds").is_some() {
            parameters.insert(
                "content_selector".to_owned(),
                selector(kinds(context, call, "content_kinds")?),
            );
        }
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-level").expect("a valid rule id"),
        capability: "axioval:capability.level-spacing".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: BTreeSet::from([object.clone()]),
        },
        parameters,
    })
}

/// The one object `steps` reach from `object`; `None` when they reach
/// none.
fn one_reached<'a>(
    context: &RuleContext<'a>,
    traversal: &Traversal,
    object: &ObjectId,
    what: &str,
) -> Result<Option<&'a Object>, Unavailable> {
    let everything: Vec<&Object> = context.project.objects().collect();
    let (found, _) = traversal.related(context, object, &everything)?;
    match found.as_slice() {
        [] => Ok(None),
        [one] => {
            Ok(Some(context.project.object(one).ok_or_else(|| {
                invalid(format!("{one} is not in the project"))
            })?))
        }
        several => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{object} reaches {} {what} via {}, not one",
                several.len(),
                traversal.relationship
            ),
        )),
    }
}

/// The levels `level` is ordered among: its anchor's, or, without an
/// anchor path, every level of its source.
fn peers<'a>(
    context: &RuleContext<'a>,
    call: &MeasuredCall,
    config: &Config<'_>,
    level: &ObjectId,
) -> Result<Option<Vec<Level<'a>>>, Unavailable> {
    if let Some(anchor) = steps(call, "anchor") {
        let back = Traversal::path(anchor)?.reversed();
        let Some(anchor) = one_reached(context, &back, level, "anchors")? else {
            return Ok(None);
        };
        return levels(context, config, anchor).map(Some);
    }
    let (candidates, outcomes) = select_objects(context, config.members);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            outcome.reason().clone(),
            format!("level selection is undecided: {}", outcome.message()),
        ));
    }
    let reached = candidates
        .iter()
        .filter(|candidate| candidate.id.source == level.source)
        .map(|candidate| candidate.id.clone())
        .collect();
    ordered(context, config, reached, &[]).map(Some)
}

/// A height as a value, cited as exactly as what it was measured from.
fn rise(height: &Height<'_, '_>, locator: String) -> Measurement {
    interval(
        (height.lower, height.upper),
        LENGTH,
        height.evidence.iter().all(|evidence| evidence.exact),
        locator,
    )
}

impl LevelMeasures {
    fn level(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let locator = format!("{}:{object}", call.name());
        let absent = |why: &str| Measurement::Absent {
            locator: format!("{locator}: {why}"),
        };
        let level = match steps(call, "path") {
            None => object.clone(),
            Some(steps) => {
                match one_reached(context, &Traversal::path(steps)?, object, "levels")? {
                    Some(level) => level.id.clone(),
                    None => return Ok(absent("the path reaches no level")),
                }
            }
        };
        let rule = declaration(call, object, context)?;
        let config = parse(&Parameters(&rule))?;
        let Some(levels) = peers(context, call, &config, &level)? else {
            return Ok(absent("no anchor reaches the level"));
        };
        if !levels.iter().any(|peer| peer.object.id == level) {
            return Ok(absent("the object is no level of its anchor"));
        }
        let mut open = CapabilityEvaluation::default();
        let heights = heights(context, &config, &levels, &mut open);
        if call.name() == PREVAILING_RISE {
            if heights.len() < 2 {
                return Ok(absent("fewer than two levels have a height"));
            }
            #[allow(clippy::float_cmp)]
            let exact: Vec<f64> = heights
                .iter()
                .filter(|height| height.lower == height.upper)
                .map(|height| height.lower)
                .collect();
            return Ok(match prevailing(&exact, config.tolerance) {
                Some(index) => interval((exact[index], exact[index]), LENGTH, true, locator),
                None => absent("no level has an exact height"),
            });
        }
        if let Some(height) = heights
            .iter()
            .find(|height| height.level.object.id == level)
        {
            return Ok(rise(height, locator));
        }
        if let Some(outcome) = open
            .not_evaluated_outcomes()
            .iter()
            .find(|outcome| outcome.object_id() == Some(&level))
        {
            return Err((outcome.reason().clone(), outcome.message().to_owned()));
        }
        Ok(absent("the level is not checked"))
    }

    /// The prevailing exact bottom or top elevation among the spaces of the
    /// space's level, as `level-spacing`'s `space_elevation` compares them.
    fn elevation(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let locator = format!("{}:{object}", call.name());
        let absent = |why: &str| Measurement::Absent {
            locator: format!("{locator}: {why}"),
        };
        let Some(spaces) = steps(call, "spaces") else {
            return Err(invalid("`spaces` is required"));
        };
        let traversal = Traversal::path(spaces)?;
        let Some(level) = one_reached(context, &traversal.reversed(), object, "levels")? else {
            return Ok(absent("no level reaches the space"));
        };
        let selector = match call.argument("kinds") {
            Some(_) => Some(kinds(context, call, "kinds")?),
            None => None,
        };
        let reach = Reach {
            traversal,
            selector: selector.as_ref(),
        };
        let (members, _) = reached(context, &reach, level, "space(s)")?;
        if members.len() < 2 {
            return Ok(absent("the level has fewer than two spaces"));
        }
        let service = extents(context)?;
        let top = call.choice("side") == Some("top");
        let mut exact = Vec::new();
        for space in &members {
            match extent(service, space) {
                Ok(extent) => {
                    let elevation = if top { extent.top() } else { extent.bottom() };
                    #[allow(clippy::float_cmp)]
                    if elevation.lower_metres() == elevation.upper_metres() {
                        exact.push(elevation.lower_metres());
                    }
                }
                Err(refusal) if space == object => return Err(refusal),
                Err(_) => {}
            }
        }
        match prevailing(&exact, length(call, "tolerance")) {
            Some(index) => Ok(interval(
                (exact[index], exact[index]),
                LENGTH,
                true,
                locator,
            )),
            None => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "no space of level {} has an exact {} elevation to compare with",
                    level.id,
                    if top { "top" } else { "bottom" }
                ),
            )),
        }
    }
}

impl MeasuredProvider for LevelMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[LEVEL_RISE, PREVAILING_ELEVATION, PREVAILING_RISE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        match call.name() {
            LEVEL_RISE | PREVAILING_RISE => Self::level(call, object, context),
            PREVAILING_ELEVATION => Self::elevation(call, object, context),
            _ => return Err(PropertyResolutionError::InvalidRequest),
        }
        .map_err(refused(call.name(), object))
    }
}
