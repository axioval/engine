//! Exact source-neutral slab-contact capability.
//!
//! ADR 0004: contact areas are measured by a [`ContactServiceHandle`]; whether
//! enough of the face is in contact -- and how serious a shortfall is -- is
//! decided here, in source-neutral policy.
//!
//! The severity bands are policy and were previously computed in the host rule
//! alongside the measurement it consumed. They are reproduced exactly:
//! a shortfall is graded by how far the measured ratio falls beneath the
//! declared minimum, and a total absence of contact by how far away the
//! nearest candidate is.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ContactError, ContactRequest, ContactServiceHandle,
    ContactSide, ContactTolerance, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Finding, Severity};

use crate::selection::select_objects;

/// Requires a minimum fraction of a face to rest on another element.
pub struct SlabContact;

impl RuleCapability for SlabContact {
    fn id(&self) -> &'static str {
        "axioval:capability.slab-contact"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("minimum_contact_ratio", ParameterType::Number),
            ParameterDescriptor::required("contact_side", ParameterType::String),
            ParameterDescriptor::required("maximum_gap_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_intersection_metres", ParameterType::Number),
            ParameterDescriptor::required(
                "minimum_polygon_area_square_metres",
                ParameterType::Number,
            ),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, mut evaluation) = select_objects(context, &rule.selector);

        let Some((minimum_ratio, side, tolerance)) = declaration(rule) else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidDeclaration,
                    "slab-contact declaration is missing or not physically realisable",
                );
            }
            return evaluation;
        };

        let Some(service) = context.services.get::<ContactServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "contact service is not registered",
                );
            }
            return evaluation;
        };

        for object in selected {
            let request = ContactRequest::new(object.id.clone(), side, tolerance);
            match service.measure_contact(&request) {
                Ok(measured) => {
                    let ratio = measured.contact_ratio();
                    if ratio >= minimum_ratio {
                        continue;
                    }
                    let mut elements = vec![object.id.clone()];
                    elements.extend(measured.touching().iter().cloned());
                    let (severity, message) = if measured.contact_area_square_metres() == 0.0 {
                        (
                            absent_severity(measured.nearest_distance_metres()),
                            "no contact".to_string(),
                        )
                    } else {
                        (
                            shortfall_severity(ratio, minimum_ratio),
                            format!("contact ratio {ratio:.4} below required {minimum_ratio:.4}"),
                        )
                    };
                    evaluation.push_finding(Finding {
                        rule_id: rule.id.clone(),
                        object_id: object.id.clone(),
                        severity,
                        message,
                        evidence: vec![measured.evidence().clone()],
                    });
                }
                Err(error) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    match error {
                        // The adapter could not orient the body, so it never
                        // measured anything; that is missing evidence, not a
                        // clean face.
                        ContactError::Unavailable | ContactError::UncheckableOrientation => {
                            NotEvaluatedReason::IncompleteEvidence
                        }
                        ContactError::InexactEvidence | ContactError::InvalidAreas => {
                            NotEvaluatedReason::InvalidEvidence
                        }
                    },
                    error.to_string(),
                ),
            }
        }
        evaluation
    }
}

/// Grades a total absence of contact by the gap to the nearest candidate.
///
/// An unknown distance is the most serious case: nothing was found to rest on
/// at all.
fn absent_severity(nearest_distance_metres: Option<f64>) -> Severity {
    match nearest_distance_metres {
        None => Severity::Error,
        Some(distance) if distance < 0.1 => Severity::Info,
        Some(distance) if distance > 0.5 => Severity::Error,
        Some(_) => Severity::Warning,
    }
}

/// Grades a partial contact by how far short of the requirement it falls.
fn shortfall_severity(ratio: f64, minimum_ratio: f64) -> Severity {
    // `minimum_ratio` is validated positive in `declaration`.
    let relative = ratio / minimum_ratio;
    if relative > 0.9 {
        Severity::Info
    } else if relative < 0.3 {
        Severity::Error
    } else {
        Severity::Warning
    }
}

fn number(rule: &CompiledRule, key: &str) -> Option<f64> {
    match rule.parameters.get(key)? {
        ParameterValue::Number { value } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn declaration(rule: &CompiledRule) -> Option<(f64, ContactSide, ContactTolerance)> {
    // A non-positive minimum would make every measurement pass and make the
    // shortfall grading divide by zero.
    let minimum_ratio =
        number(rule, "minimum_contact_ratio").filter(|ratio| *ratio > 0.0 && *ratio <= 1.0)?;
    let side = match rule.parameters.get("contact_side")? {
        ParameterValue::String { value } if value == "above" => ContactSide::Above,
        ParameterValue::String { value } if value == "below" => ContactSide::Below,
        _ => return None,
    };
    let tolerance = ContactTolerance::try_new(
        number(rule, "maximum_gap_metres")?,
        number(rule, "maximum_intersection_metres")?,
        number(rule, "minimum_polygon_area_square_metres")?,
    )
    .ok()?;
    Some((minimum_ratio, side, tolerance))
}
