//! A body's depth along one of its own placement axes as a value, measured
//! exactly as `body-extent` measures it: the object's frame, then the
//! directional extent along the axis, refused for the capability's reasons
//! and cited as exactly as the frame and the extent are. The positions of
//! the extent's two ends along the axis are values too, the magnitudes the
//! template's rounding allowance scales with.

use axioval_engine::{
    DirectionalExtent, MeasuredProvider, Measurement, NotEvaluatedReason, ObjectFrame,
    ObjectFrameServiceHandle, PropertyResolutionError, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Axis, extent_error, frame_error};
use crate::measured_kinds::{interval, refused};
use crate::support::Unavailable;

/// Measures `body_extent` and `body_position`.
pub(crate) struct ExtentMeasures;

const BODY_EXTENT: &str = "body_extent";
const BODY_POSITION: &str = "body_position";

/// The object's frame and its directional extent along the call's axis.
fn measured(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(ObjectFrame, DirectionalExtent), Unavailable> {
    let axis = Axis::parse(call.choice("axis").unwrap_or_default())?;
    let (Some(frames), Some(extents)) = (
        context.services.get::<ObjectFrameServiceHandle>(),
        context.services.get::<VerticalExtentServiceHandle>(),
    ) else {
        return Err((
            NotEvaluatedReason::MissingService,
            "body-extent needs the object-frame and vertical-extent services".into(),
        ));
    };
    let frame = frames
        .object_frame(object)
        .map_err(|error| frame_error(&error))?;
    let extent = extents
        .measure_directional_extent(object, axis.of(&frame))
        .map_err(|error| extent_error(&error))?;
    Ok((frame, extent))
}

fn along(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    let (frame, extent) = measured(call, object, context)?;
    let range = match call.name() {
        BODY_POSITION => {
            let end = match call.choice("end") {
                Some("low") => extent.lower(),
                Some("high") => extent.upper(),
                other => {
                    return Err(crate::support::invalid(format!(
                        "end `{}` is unsupported; use `low` or `high`",
                        other.unwrap_or_default()
                    )));
                }
            };
            (end.lower_metres(), end.upper_metres())
        }
        _ => extent.length_metres(),
    };
    Ok(interval(
        range,
        Some(QuantityDimension::Length),
        frame.evidence().exact && extent.evidence().exact,
        extent.evidence().locator.clone(),
    ))
}

impl MeasuredProvider for ExtentMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[BODY_EXTENT, BODY_POSITION]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        along(call, object, context).map_err(refused(call.name(), object))
    }
}
