//! The voided area of a host as a measured value: the summed section areas
//! of the openings its path reaches, on the host's middle plane, measured
//! as `opening-area` measures them, so `gross_area − net_area` against it
//! is that capability's comparison.

use std::collections::BTreeMap;

use axioval_engine::{
    CompiledRule, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, RuleId};

use crate::counts::Population;
use crate::opening_area::{Openings, voided};
use crate::support::Parameters;

/// The name measured.
pub(crate) const OPENING_AREA: &str = "opening_area";
/// One opening's section area on its host's middle plane.
pub(crate) const OPENING_SECTION_AREA: &str = "opening_section_area";

/// Measures `opening_area`.
pub(crate) struct OpeningMeasures;

impl MeasuredProvider for OpeningMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[OPENING_AREA, OPENING_SECTION_AREA]
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
        Ok(Measurement::Value {
            lower: voided.sum,
            upper: voided.sum,
            dimension: Some(QuantityDimension::Area),
            locator: format!(
                "{OPENING_AREA}:{object}:{}",
                evidence
                    .iter()
                    .map(|evidence| evidence.locator.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        })
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
    let area = crate::opening_area::opening_area(context, openings, &face, opening, &mut evidence)?
        .map_or(0.0, |(area, _)| area);
    Ok(Measurement::Value {
        lower: area,
        upper: area,
        dimension: Some(QuantityDimension::Area),
        locator: format!("{OPENING_SECTION_AREA}:{}:{host}", opening.id),
    })
}
