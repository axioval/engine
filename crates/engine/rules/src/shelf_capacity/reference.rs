//! `shelf-capacity` as it was implemented before it became a template
//! (#289), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, LinearInterval, LinearQuantityServiceHandle,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
    ShelfGeometry,
};
use axioval_ir::contract::ParameterValue;

use super::{Shelving, measure};
use crate::level_spacing::shown;
use crate::selection::select_objects;
use crate::space_access::AccessDeclaration;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Minimum running metres of shelving a space must provide.
pub struct ShelfCapacity;

impl RuleCapability for ShelfCapacity {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("minimum_running_metres", ParameterType::Number),
            ParameterDescriptor::required("shelf_depth_metres", ParameterType::Number),
            ParameterDescriptor::required("horizontal_spacing_metres", ParameterType::Number),
            ParameterDescriptor::required("vertical_spacing_metres", ParameterType::Number),
            ParameterDescriptor::required("bottom_elevation_metres", ParameterType::Number),
            ParameterDescriptor::required("top_elevation_metres", ParameterType::Number),
            ParameterDescriptor::required("door_clearance_metres", ParameterType::Number),
            ParameterDescriptor::required("access_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, mut evaluation) = select_objects(context, &rule.selector);

        let parameters = Parameters(rule);
        let (minimum, geometry, access) = match declaration(rule, &parameters) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                for object in selected {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason.clone(),
                        message.clone(),
                    );
                }
                return evaluation;
            }
        };

        let Some(service) = context.services.get::<LinearQuantityServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "linear-quantity service is not registered",
                );
            }
            return evaluation;
        };

        let index = access.index(context);
        for object in selected {
            let Shelving {
                measured,
                doors,
                evidence,
            } = match measure(service, &index, geometry, &object.id) {
                Ok(shelving) => shelving,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            judge_height(
                &mut evaluation,
                rule,
                &object.id,
                measured.clear_height(),
                geometry.top_elevation_metres(),
                &evidence,
            );
            judge_length(
                &mut evaluation,
                rule,
                &object.id,
                measured.measured(),
                minimum,
                evidence,
                doors,
            );
        }
        evaluation
    }
}

/// The minimum, the arrangement and where the doors are read from.
fn declaration<'a>(
    rule: &CompiledRule,
    parameters: &Parameters<'a>,
) -> Result<(f64, ShelfGeometry, AccessDeclaration<'a>), Unavailable> {
    let minimum = minimum_running_metres(rule)
        .ok_or_else(|| invalid("shelf capacity minimum must be a finite, non-negative number"))?;
    let geometry = shelf_geometry(rule).ok_or_else(|| {
        invalid("shelf geometry parameters are missing or not physically realisable")
    })?;
    let access = AccessDeclaration::parse(parameters)
        .and_then(|access| access.ok_or_else(|| invalid("parameter `access_path` is required")))
        .map_err(|(reason, message)| (reason, format!("shelf-capacity: {message}")))?;
    Ok((minimum, geometry, access))
}

/// The space is too low for the shelving when its clear height lies wholly
/// below the shelving's top elevation.
fn judge_height(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    object: &axioval_ir::ObjectId,
    height: Option<LinearInterval>,
    top: f64,
    evidence: &[axioval_ir::Evidence],
) {
    match height {
        Some(height) if height.definitely_below(top) => evaluation.push_finding(finding(
            rule,
            object,
            format!(
                "space too low for the shelving: clear height {} below the shelving's top \
                 elevation {}",
                shown(height.lower_metres(), height.upper_metres()),
                shown(top, top)
            ),
            evidence.to_vec(),
            Vec::new(),
        )),
        Some(height) if height.definitely_at_least(top) => {}
        Some(height) => evaluation.push_object_not_evaluated(
            object.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "clear height {} may or may not reach the shelving's top elevation {}",
                shown(height.lower_metres(), height.upper_metres()),
                shown(top, top)
            ),
        ),
        None => evaluation.push_object_not_evaluated(
            object.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            "the clear height of the space was not measured",
        ),
    }
}

fn judge_length(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    object: &axioval_ir::ObjectId,
    interval: LinearInterval,
    minimum: f64,
    evidence: Vec<axioval_ir::Evidence>,
    doors: Vec<axioval_ir::ObjectId>,
) {
    if interval.definitely_at_least(minimum) {
        return;
    }
    // Fail closed on ambiguity: an interval that straddles the minimum has
    // not been shown to fail, so reporting a violation would assert more
    // than was measured.
    if !interval.definitely_below(minimum) {
        evaluation.push_object_not_evaluated(
            object.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "measured shelf length {} spans the required minimum {minimum:.3}",
                shown(interval.lower_metres(), interval.upper_metres())
            ),
        );
        return;
    }
    evaluation.push_finding(finding(
        rule,
        object,
        format!(
            "shelf running metres {:.3} below required {minimum:.3}",
            interval.upper_metres()
        ),
        evidence,
        doors,
    ));
}

fn minimum_running_metres(rule: &CompiledRule) -> Option<f64> {
    number(rule, "minimum_running_metres").filter(|value| *value >= 0.0)
}

fn number(rule: &CompiledRule, key: &str) -> Option<f64> {
    match rule.parameters.get(key)? {
        ParameterValue::Number { value } if value.is_finite() => Some(*value),
        _ => None,
    }
}

/// The measured arrangement, taken from the declaration.
///
/// Validity is decided by `ShelfGeometry`, so an impossible arrangement is
/// rejected once rather than by each adapter.
fn shelf_geometry(rule: &CompiledRule) -> Option<ShelfGeometry> {
    ShelfGeometry::try_new(
        number(rule, "shelf_depth_metres")?,
        number(rule, "horizontal_spacing_metres")?,
        number(rule, "vertical_spacing_metres")?,
        number(rule, "bottom_elevation_metres")?,
        number(rule, "top_elevation_metres")?,
        number(rule, "door_clearance_metres")?,
    )
    .ok()
}
