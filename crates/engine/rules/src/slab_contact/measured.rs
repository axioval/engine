//! Where an object's storey stands among its source's storeys, as
//! `slab-contact` decides which subjects lie on the top or bottom storey:
//! storeys ordered by their `Elevation` attribute, the object on the one
//! storey its path reaches.

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId};

use super::{StoreySkip, storeys};
use crate::measured_kinds::{every_object_of_kinds, refused};
use crate::support::{Traversal, Unavailable, invalid};

/// Measures `levels_above` and `levels_below`.
pub(crate) struct StoreyMeasures;

const LEVELS_ABOVE: &str = "levels_above";
const LEVELS_BELOW: &str = "levels_below";

fn count(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
        return Err(invalid("`path` is required"));
    };
    let selector = Selector::Objects {
        objects: every_object_of_kinds(context, call, "levels")
            .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?,
    };
    let skip = StoreySkip {
        top: true,
        bottom: true,
        storeys: &selector,
        traversal: Traversal::path(steps)?,
    };
    let storeys = storeys(context, &skip)?;
    let (reached, _) = skip.traversal.related(context, object, &storeys.universe)?;
    let [storey] = reached.as_slice() else {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{object} reaches {} storeys through {}, so where it lies is unknown",
                reached.len(),
                skip.traversal.relationship
            ),
        ));
    };
    let own = storeys.elevations[storey];
    let above = call.name() == LEVELS_ABOVE;
    let counted = storeys
        .universe
        .iter()
        .map(|candidate: &&Object| &candidate.id)
        .filter(|candidate| candidate.source == storey.source)
        .filter(|candidate| {
            let elevation = storeys.elevations[*candidate];
            if above {
                elevation.total_cmp(&own).is_gt()
            } else {
                elevation.total_cmp(&own).is_lt()
            }
        })
        .count();
    #[allow(clippy::cast_precision_loss)]
    let counted = counted as f64;
    Ok(Measurement::Value {
        lower: counted,
        upper: counted,
        dimension: None,
        locator: format!("{}:{object}:{storey}", call.name()),
    })
}

impl MeasuredProvider for StoreyMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[LEVELS_ABOVE, LEVELS_BELOW]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        count(call, object, context).map_err(refused(call.name(), object))
    }
}
