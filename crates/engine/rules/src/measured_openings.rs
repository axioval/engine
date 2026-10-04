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

/// Measures `opening_area`.
pub(crate) struct OpeningMeasures;

impl MeasuredProvider for OpeningMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[OPENING_AREA]
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
        let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
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
        let host = context
            .project
            .object(object)
            .ok_or_else(|| unavailable("it is not in the project".into()))?;
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
