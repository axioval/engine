//! The voided area of a host as a measured value: the summed section areas
//! of the openings its path reaches, on the host's middle plane, measured
//! as `opening-area` measures them, so `gross_area − net_area` against it
//! is that capability's comparison; and, as `empty-host` compares them, how
//! many openings it counts and the area of the face they void.

use std::collections::BTreeMap;

use axioval_engine::{
    CompiledRule, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, RuleId};

use crate::counts::Population;
use crate::empty_host::face_area;
use crate::opening_area::{Openings, voided};
use crate::opening_zone::face::{FaceAxes, read_host};
use crate::support::Parameters;

/// The name measured.
pub(crate) const OPENING_AREA: &str = "opening_area";
/// One opening's section area on its host's middle plane.
pub(crate) const OPENING_SECTION_AREA: &str = "opening_section_area";

/// How many openings take area from the host's middle plane.
const OPENING_COUNT: &str = "opening_count";
/// The host's face on its middle plane, as `empty-host` measures it.
const MIDDLE_FACE_AREA: &str = "middle_face_area";

/// Measures `opening_area` and what `empty-host` compares.
pub(crate) struct OpeningMeasures;

/// The face `call`'s axes name on the middle plane of `host`.
fn middle_face(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    host: &ObjectId,
) -> Result<Measurement, crate::support::Unavailable> {
    let axes = FaceAxes::parse(
        call.choice("length_axis").unwrap_or("extrusion"),
        call.choice("height_axis").unwrap_or("profile-y"),
    )?;
    let face = read_host(context, host)?;
    let area = face_area(&face, axes)?;
    Ok(crate::measured_kinds::interval(
        (area, area),
        Some(QuantityDimension::Area),
        exact(&face.evidence),
        format!("{MIDDLE_FACE_AREA}:{host}"),
    ))
}

/// Whether every evidence a value was measured from is exact: the body
/// facts are stated, so this holds unless a source cites an estimate.
fn exact(evidence: &[axioval_ir::Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// A sum of `terms` non-negative doubles, widened by the most its
/// rounding may have moved it, so it holds the exact sum of the terms.
fn summed(sum: f64, terms: usize) -> (f64, f64) {
    if terms < 2 || sum == 0.0 {
        return (sum, sum);
    }
    #[allow(clippy::cast_precision_loss)]
    let margin = (terms - 1) as f64 * f64::EPSILON * sum;
    (
        (sum - margin).next_down().max(0.0),
        (sum + margin).next_up(),
    )
}

impl MeasuredProvider for OpeningMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            MIDDLE_FACE_AREA,
            OPENING_AREA,
            OPENING_COUNT,
            OPENING_SECTION_AREA,
        ]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let unavailable = |why: String| {
            PropertyResolutionError::Unavailable(format!("`{OPENING_AREA}` of {object}: {why}"))
        };
        if call.name() == MIDDLE_FACE_AREA {
            return middle_face(call, context, object).map_err(|(reason, why)| {
                crate::measured_kinds::resolution_error((
                    reason,
                    format!("`{MIDDLE_FACE_AREA}` of {object}: {why}"),
                ))
            });
        }
        let text = |value: &str| ParameterValue::String {
            value: value.to_owned(),
        };
        let key = if call.name() == OPENING_SECTION_AREA {
            "host_path"
        } else {
            "path"
        };
        let Some(MeasuredArgument::Path(steps)) = call.argument(key) else {
            return Err(PropertyResolutionError::InvalidRequest);
        };
        let mut parameters = BTreeMap::from([
            (
                "opening_path".to_owned(),
                ParameterValue::StringList {
                    value: steps.clone(),
                },
            ),
            (
                "length_axis".to_owned(),
                text(call.choice("length_axis").unwrap_or("extrusion")),
            ),
            (
                "height_axis".to_owned(),
                text(call.choice("height_axis").unwrap_or("profile-y")),
            ),
        ]);
        if let Some(MeasuredArgument::Length(minimum)) = call.argument("minimum") {
            parameters.insert(
                "minimum_opening_area".to_owned(),
                ParameterValue::Quantity {
                    value: *minimum,
                    unit: "m2".into(),
                },
            );
        }
        let rule = CompiledRule {
            id: RuleId::new("axioval-measured-opening-area").expect("a valid rule id"),
            capability: "axioval:capability.opening-area".into(),
            severity: Severity::Info,
            selector: Selector::All,
            parameters,
        };
        let refused = |(reason, why): crate::support::Unavailable| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{OPENING_AREA}` of {object}: {why}"),
            ))
        };
        let openings = Openings::parse(&Parameters(&rule)).map_err(refused)?;
        let subject = context
            .project
            .object(object)
            .ok_or_else(|| unavailable("it is not in the project".into()))?;
        if call.name() == OPENING_SECTION_AREA {
            return section(context, &openings, subject).map_err(refused);
        }
        let host = subject;
        let population = Population::of(context, openings.selector);
        let mut evidence = Vec::new();
        let voided =
            voided(context, &openings, &population, host, &mut evidence).map_err(refused)?;
        if call.name() == OPENING_COUNT {
            #[allow(clippy::cast_precision_loss)]
            let counted = voided.counted.len() as f64;
            return Ok(crate::measured_kinds::interval(
                (counted, counted),
                None,
                exact(&evidence),
                format!("{OPENING_COUNT}:{object}"),
            ));
        }
        Ok(crate::measured_kinds::interval(
            summed(voided.sum, voided.counted.len()),
            Some(QuantityDimension::Area),
            exact(&evidence),
            format!(
                "{OPENING_AREA}:{object}:{}",
                evidence
                    .iter()
                    .map(|evidence| evidence.locator.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ))
    }
}

/// The section area `opening` takes from the middle plane of the one host
/// `openings`' path reaches from it.
fn section(
    context: &RuleContext<'_>,
    openings: &Openings<'_>,
    opening: &axioval_ir::Object,
) -> Result<Measurement, crate::support::Unavailable> {
    let everything: Vec<&axioval_ir::Object> = context.project.objects().collect();
    let (hosts, _) = openings.path().related(context, &opening.id, &everything)?;
    let [host] = &hosts[..] else {
        return if hosts.is_empty() {
            Ok(Measurement::Absent {
                locator: format!("{OPENING_SECTION_AREA}:{}: it voids no host", opening.id),
            })
        } else {
            Err((
                axioval_engine::NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it voids {} hosts, so its section is ambiguous",
                    hosts.len()
                ),
            ))
        };
    };
    let face = crate::opening_zone::face::read_host(context, host)?;
    let mut evidence = Vec::new();
    // `None` is an opening surely below the declared minimum, which takes
    // nothing from the plane as `opening-area` counts it: a decided zero,
    // not a missing area (an area that cannot be measured is an error).
    let area = crate::opening_area::opening_area(context, openings, &face, opening, &mut evidence)?
        .map_or(0.0, |(area, _)| area);
    evidence.extend(face.evidence.iter().cloned());
    Ok(crate::measured_kinds::interval(
        (area, area),
        Some(QuantityDimension::Area),
        exact(&evidence),
        format!("{OPENING_SECTION_AREA}:{}:{host}", opening.id),
    ))
}
