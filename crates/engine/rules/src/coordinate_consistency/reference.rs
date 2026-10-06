//! The `coordinate-consistency` implementation the template replaced, kept
//! only as the parity reference its template is held to
//! (`parity-reference`). It shares the comparison
//! ([`compare_coordinate_systems`]) and the choice of the reference source
//! ([`reference_source`]) with the measured values the template reads.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CoordinateSystemServiceHandle, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, SourceCoordinateSystem,
};
use axioval_ir::{Finding, Scope, SourceId};

use super::{CoordinateTolerance, compare_coordinate_systems, reference_source};
use crate::pairs::severity;
use crate::support::{Parameters, Unavailable, invalid};

/// Requires every source of the run to share the reference source's
/// coordinate system.
///
/// The reference is the one source of discipline `reference`, or, without
/// it, the first source in identity order. Each other source's world frame,
/// true north, map conversion (target system, offset, rotation and scale)
/// and site placement are compared with the reference's within
/// `length_tolerance` (metres, default 0.001), `angle_tolerance` (degrees,
/// default 0.01) and `scale_tolerance` (default 0); a source that differs is
/// a finding naming it and every difference. A statement only one of the two
/// makes, a map unit that is not known exactly and an unreadable coordinate
/// system leave the source not evaluated. A source without a map conversion
/// is a finding with `require_map_conversion`, and not evaluated otherwise.
pub struct CoordinateConsistencyCheck;

struct Declaration<'a> {
    reference: Option<&'a str>,
    tolerance: CoordinateTolerance,
    require_map: bool,
}

impl<'a> Declaration<'a> {
    fn read(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let length = parameters.number("length_tolerance")?.unwrap_or(0.001);
        let angle = parameters.number("angle_tolerance")?.unwrap_or(0.01);
        let scale = parameters.number("scale_tolerance")?.unwrap_or(0.0);
        let tolerance =
            CoordinateTolerance::try_new(length, angle.to_radians(), scale).map_err(invalid)?;
        Ok(Self {
            reference: parameters.string("reference")?,
            tolerance,
            require_map: parameters
                .boolean("require_map_conversion")?
                .unwrap_or(false),
        })
    }
}

impl RuleCapability for CoordinateConsistencyCheck {
    fn id(&self) -> &'static str {
        "axioval:capability.coordinate-consistency"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("reference", ParameterType::String),
            ParameterDescriptor::optional("length_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("angle_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("scale_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("require_map_conversion", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = Declaration::read(rule).and_then(|declaration| {
            let Some(service) = context.services.get::<CoordinateSystemServiceHandle>() else {
                return Err((
                    NotEvaluatedReason::MissingService,
                    "no coordinate-system service is registered".into(),
                ));
            };
            let sources = reference_source(context, declaration.reference)?;
            Ok((declaration, service, sources))
        });
        let (declaration, service, (reference, others)) = match declared {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("coordinate-consistency: {message}"),
                );
            }
        };
        let mut evaluation = CapabilityEvaluation::default();
        let base = match service.coordinate_system(&reference) {
            Ok(base) => base,
            Err(error) => {
                for source in others {
                    evaluation.push_source_not_evaluated(
                        source,
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "coordinate-consistency: the reference `{reference}`'s coordinate system cannot be read: {error}"
                        ),
                    );
                }
                return evaluation;
            }
        };
        if declaration.require_map && base.map().is_none() {
            evaluation.push_finding(
                Finding::new(
                    rule.id.clone(),
                    Scope::Source(reference.clone()),
                    severity(rule),
                    format!(
                        "`{reference}`, the reference, states no map conversion; the federation requires one"
                    ),
                )
                .with_evidence(vec![base.evidence().clone()]),
            );
        }
        for source in others {
            let system = match service.coordinate_system(&source) {
                Ok(system) => system,
                Err(error) => {
                    evaluation.push_source_not_evaluated(
                        source,
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("coordinate-consistency: {error}"),
                    );
                    continue;
                }
            };
            judge(
                rule,
                &declaration,
                (&reference, &base),
                (&source, &system),
                &mut evaluation,
            );
        }
        evaluation
    }
}

fn judge(
    rule: &CompiledRule,
    declaration: &Declaration<'_>,
    (reference, base): (&SourceId, &SourceCoordinateSystem),
    (source, system): (&SourceId, &SourceCoordinateSystem),
    evaluation: &mut CapabilityEvaluation,
) {
    let consistency = compare_coordinate_systems(base, system, declaration.tolerance);
    let mut differences: Vec<String> = consistency
        .differences
        .iter()
        .map(|(_, difference)| difference.clone())
        .collect();
    let mut unknown: Vec<String> = consistency
        .unknown
        .iter()
        .map(|(_, reason)| reason.clone())
        .collect();
    let mut not_recorded = false;
    match consistency.georeferenced {
        (_, false) if declaration.require_map => {
            differences.push("states no map conversion".into());
        }
        (true, true) => {}
        // The reference's own missing conversion is its own finding.
        (false, true) if declaration.require_map => {}
        (reference_map, source_map) => {
            not_recorded = true;
            unknown.push(format!(
                "{} no map conversion, so whether the georeferences agree is unknown",
                match (reference_map, source_map) {
                    (true, false) => "this source states",
                    (false, true) => "the reference states",
                    _ => "neither source states",
                }
            ));
        }
    }
    if !differences.is_empty() {
        evaluation.push_finding(
            Finding::new(
                rule.id.clone(),
                Scope::Source(source.clone()),
                severity(rule),
                format!(
                    "`{source}` does not share the coordinate system of `{reference}`: {}",
                    differences.join("; ")
                ),
            )
            .with_evidence(vec![base.evidence().clone(), system.evidence().clone()]),
        );
    } else if !unknown.is_empty() {
        let reason = if not_recorded && unknown.len() == 1 {
            NotEvaluatedReason::NotRecorded
        } else {
            NotEvaluatedReason::IncompleteEvidence
        };
        evaluation.push_source_not_evaluated(
            source.clone(),
            reason,
            format!(
                "coordinate-consistency: `{source}` against `{reference}`: {}",
                unknown.join("; ")
            ),
        );
    }
}
