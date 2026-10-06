//! A body's depth along one of its own placement axes as a value, measured
//! exactly as `body-extent` measures it: the object's frame, then the
//! directional extent along the axis, refused for the capability's reasons
//! and cited as exactly as the frame and the extent are. The positions of
//! the extent's two ends along the axis are values too, the magnitudes the
//! template's rounding allowance scales with.

use std::sync::Arc;

use axioval_engine::{
    ElevationInterval, MeasuredMemo, MeasuredProvider, Measurement, MetricDirection,
    NotEvaluatedReason, ObjectFrameServiceHandle, PropertyResolutionError, RuleContext,
    VerticalExtentServiceHandle,
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

/// What `body_extent` and `body_position` read of one object's
/// directional extent along one axis: the length and both ends, cited as
/// exactly as the frame and the extent are.
#[derive(Clone)]
struct Extent {
    length: (f64, f64),
    low: (f64, f64),
    high: (f64, f64),
    exact: bool,
    locator: Arc<str>,
}

/// What the axes of an object's frame are, and whether it was measured
/// exactly.
#[derive(Clone, Copy)]
struct Frame {
    axes: [MetricDirection; 3],
    exact: bool,
}

/// Everything measured of one object's body for the run: its frame once,
/// and its extent along each axis a value read. Kept small, because a run
/// keeps one per object ([`MeasuredMemo`], keyed by the object; no other
/// provider memoizes a `Body`).
#[derive(Clone, Default)]
struct Body {
    frame: Option<Result<Frame, Unavailable>>,
    extents: [Option<Result<Extent, Unavailable>>; 3],
}

/// The object's directional extent along `axis`, measured once per object
/// and axis for the run, on the object's frame measured once per object:
/// `body_extent` and both ends of `body_position`, along any axis and in
/// every rule reading them, share one frame and one extent per axis. The
/// memo keeps one [`Body`] per object, keyed by the object.
fn measured(
    axis: Axis,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Extent, Unavailable> {
    let Some(memo) = context.services.get::<MeasuredMemo>() else {
        return take(axis, None, object, context).1;
    };
    let index = axis.index();
    let known = memo.get_with(object, |body: &Body| {
        (body.extents[index].clone(), body.frame.clone())
    });
    let frame = match known {
        Some((Some(extent), _)) => return extent,
        Some((None, frame)) => frame,
        None => None,
    };
    let (frame, extent) = take(axis, frame, object, context);
    let mut body: Body = memo.get(object).unwrap_or_default();
    body.frame = frame;
    body.extents[index] = Some(extent.clone());
    memo.insert(object.clone(), body);
    extent
}

/// The object's frame (`frame`, where measured already), then its
/// directional extent along `axis`; the frame is returned to keep, unless
/// the services are missing.
fn take(
    axis: Axis,
    frame: Option<Result<Frame, Unavailable>>,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> (
    Option<Result<Frame, Unavailable>>,
    Result<Extent, Unavailable>,
) {
    let (Some(frames), Some(extents)) = (
        context.services.get::<ObjectFrameServiceHandle>(),
        context.services.get::<VerticalExtentServiceHandle>(),
    ) else {
        return (
            frame,
            Err((
                NotEvaluatedReason::MissingService,
                "body-extent needs the object-frame and vertical-extent services".into(),
            )),
        );
    };
    let frame = frame.unwrap_or_else(|| {
        frames
            .object_frame(object)
            .map(|frame| Frame {
                axes: [Axis::Right, Axis::Forward, Axis::Up].map(|axis| axis.of(&frame)),
                exact: frame.evidence().exact,
            })
            .map_err(|error| frame_error(&error))
    });
    let extent = frame.clone().and_then(|measured| {
        let extent = extents
            .measure_directional_extent(object, measured.axes[axis.index()])
            .map_err(|error| extent_error(&error))?;
        let end = |end: ElevationInterval| (end.lower_metres(), end.upper_metres());
        Ok(Extent {
            length: extent.length_metres(),
            low: end(extent.lower()),
            high: end(extent.upper()),
            exact: measured.exact && extent.evidence().exact,
            locator: Arc::from(extent.evidence().locator.as_str()),
        })
    });
    (Some(frame), extent)
}

/// What a call reads along an axis: the extent's length, or the end it
/// names.
#[derive(Clone, Copy)]
struct Read<'c> {
    axis: Axis,
    value: Value<'c>,
}

#[derive(Clone, Copy)]
enum Value<'c> {
    /// `body_extent`.
    Length,
    /// `body_position` at the `end` the call names, if any.
    End(Option<&'c str>),
}

impl<'c> Read<'c> {
    fn of(call: &'c MeasuredCall) -> Result<Self, Unavailable> {
        Ok(Self {
            axis: Axis::parse(call.choice("axis").unwrap_or_default())?,
            value: if call.name() == BODY_POSITION {
                Value::End(call.choice("end"))
            } else {
                Value::Length
            },
        })
    }

    /// The value of `object`; an end the call cannot name is refused once
    /// the extent is measured, as the capability refused it.
    fn measure(
        self,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let extent = measured(self.axis, object, context)?;
        let range = match self.value {
            Value::Length => extent.length,
            Value::End(Some("low")) => extent.low,
            Value::End(Some("high")) => extent.high,
            Value::End(other) => {
                return Err(crate::support::invalid(format!(
                    "end `{}` is unsupported; use `low` or `high`",
                    other.unwrap_or_default()
                )));
            }
        };
        Ok(interval(
            range,
            Some(QuantityDimension::Length),
            extent.exact,
            extent.locator.to_string(),
        ))
    }
}

fn along(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    Read::of(call)?.measure(object, context)
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

    fn measure_batch(
        &self,
        call: &MeasuredCall,
        objects: &[&ObjectId],
        context: &RuleContext<'_>,
    ) -> Vec<Result<Measurement, PropertyResolutionError>> {
        // The call's axis and end once for every object.
        let read = Read::of(call);
        objects
            .iter()
            .map(|object| {
                read.clone()
                    .and_then(|read| read.measure(object, context))
                    .map_err(refused(call.name(), object))
            })
            .collect()
    }
}
