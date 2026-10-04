//! A body's depth along one of its own placement axes as a value, measured
//! exactly as `body-extent` measures it: the object's frame, then the
//! directional extent along the axis, refused for the capability's reasons
//! and cited as exactly as the frame and the extent are.

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, ObjectFrameServiceHandle,
    PropertyResolutionError, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Axis, extent_error, frame_error};
use crate::measured_kinds::{interval, refused};
use crate::support::Unavailable;

/// Measures `body_extent`.
pub(crate) struct ExtentMeasures;

const BODY_EXTENT: &str = "body_extent";

fn along(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
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
    Ok(interval(
        extent.length_metres(),
        Some(QuantityDimension::Length),
        frame.evidence().exact && extent.evidence().exact,
        extent.evidence().locator.clone(),
    ))
}

impl MeasuredProvider for ExtentMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[BODY_EXTENT]
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
