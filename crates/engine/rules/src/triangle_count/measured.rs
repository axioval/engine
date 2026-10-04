//! The triangles of an object's mesh as a value, counted by the host as
//! `triangle-count` reads them: a count of a tessellation is cited as
//! approximate evidence, as the capability cites it.

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
    TriangleCountServiceHandle,
};
use axioval_ir::ObjectId;
use axioval_ir::measured::MeasuredCall;

use super::count_error;
use crate::measured_kinds::{interval, refused};

/// Measures `triangle_count`.
pub(crate) struct TriangleMeasures;

const TRIANGLE_COUNT: &str = "triangle_count";

impl MeasuredProvider for TriangleMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[TRIANGLE_COUNT]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let refuse = refused(call.name(), object);
        let counts = context
            .services
            .get::<TriangleCountServiceHandle>()
            .ok_or_else(|| {
                refuse((
                    NotEvaluatedReason::MissingService,
                    "triangle-count service is not registered".into(),
                ))
            })?;
        let count = counts
            .count_triangles(object)
            .map_err(|error| refuse(count_error(&error)))?;
        #[allow(clippy::cast_precision_loss)]
        let triangles = count.triangles() as f64;
        Ok(interval(
            (triangles, triangles),
            None,
            count.is_exact(),
            count.evidence().locator.clone(),
        ))
    }
}
