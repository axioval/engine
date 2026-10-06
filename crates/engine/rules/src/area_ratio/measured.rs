//! An object's area as `area-ratio` sums it, as a value: the area a
//! property states, or its plan footprint (an empty one counting zero) or
//! facade area, exactly as the capability read each.

use axioval_engine::{
    MeasuredMemo, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use crate::measured_kinds::{interval, refused};
use crate::plan_area::{Measure, Sum};
use crate::support::PropertyRef;

/// The memo key of one object's area as a ratio reads it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RatioKey;

/// Measures `ratio_area`.
pub(crate) struct RatioMeasures;

const RATIO_AREA: &str = "ratio_area";

impl MeasuredProvider for RatioMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[RATIO_AREA]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        // This side's measure, else the rule's for both, else the footprint.
        let measure = match call.choice("measure").or_else(|| call.choice("otherwise")) {
            Some("facade") => Measure::Facade,
            _ => Measure::Footprint,
        };
        let property = match call.argument("property") {
            Some(MeasuredArgument::Property { set, name }) => Some((set.clone(), name.clone())),
            _ => None,
        };
        // Once per object, measure and property for the run: a storey's
        // ratio reads the areas its neighbours' ratios read too.
        MeasuredMemo::of(
            context.services,
            (RatioKey, object.clone(), measure, property.clone()),
            || {
                let stated = property.as_ref().map(|(set, name)| PropertyRef {
                    set: set.as_deref(),
                    name,
                });
                let area = Sum::measured(context, stated, measure, std::slice::from_ref(object))
                    .map_err(refused(call.name(), object))?;
                Ok(interval(
                    (area.lower, area.upper),
                    Some(QuantityDimension::Area),
                    area.evidence.iter().all(|evidence| evidence.exact),
                    format!("{RATIO_AREA}:{object}"),
                ))
            },
        )
    }
}
