//! An object's own area as a value, measured exactly as `plan-area` measures
//! it: the plan footprint, refused when empty because the object then has
//! no body, or the outward-facing facade area.

use axioval_engine::{
    MeasuredMemo, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Measure, own_area};
use crate::measured_kinds::{interval, refused};

/// The memo key of one object's own area.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AreaKey;

/// Measures `plan_area`.
pub(crate) struct AreaMeasures;

const PLAN_AREA: &str = "plan_area";

impl MeasuredProvider for AreaMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[PLAN_AREA]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let measure = if call.choice("measure") == Some("facade") {
            Measure::Facade
        } else {
            Measure::Footprint
        };
        // Once per object and measure for the run: a storey's rules read
        // its members' areas, which the members' own rules read too.
        MeasuredMemo::of(context.services, (AreaKey, object.clone(), measure), || {
            let area = own_area(context, measure, object).map_err(refused(call.name(), object))?;
            Ok(interval(
                (area.lower, area.upper),
                Some(QuantityDimension::Area),
                area.evidence.iter().all(|evidence| evidence.exact),
                format!("{PLAN_AREA}:{object}"),
            ))
        })
    }
}
