//! The share of an element its counterparts leave uncovered as a measured
//! value, measured exactly as `counterpart-coverage` measures it for one
//! check: in plan, in height or in the element's elevation, the cover from
//! the plan broad phase, axis-compatible counterparts only with an axis
//! tolerance, and a frame's infill in the elevation.
//!
//! The counterparts are the objects of the source kinds `by` names, the
//! frame members those `frame` names. A counterpart whose extent cannot be
//! read may cover anything, so it drops the lower bound to zero, as the
//! capability counts it.

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, Severity};

use super::{Config, Counterparts, Services, Share, Subject};
use crate::measured_kinds::{objects_of_kinds, resolution_error};
use crate::plan_area::footprint;
use crate::support::{Unavailable, invalid};

/// Measures `counterpart_uncovered_share`.
pub(crate) struct CoverageMeasures;

const NAME: &str = "counterpart_uncovered_share";

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

/// The objects of the kinds `key` names, as a selector.
fn kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<Selector, PropertyResolutionError> {
    Ok(Selector::Objects {
        objects: objects_of_kinds(context, call, key, object)?,
    })
}

/// The check `call` names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Plan,
    Height,
    Elevation,
}

/// The share of `object` the check leaves uncovered, as the capability
/// measures it before grading, and whether every counterpart that may
/// cover it was read: one that was not drops the lower bound to zero
/// without evidence of its own.
fn measure(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    check: Check,
    object: &Object,
) -> Result<(Share, bool), Unavailable> {
    let services = Services::of(context, config)?;
    let margin = config.horizontal.unwrap_or(0.0)
        * if config.elevation {
            std::f64::consts::SQRT_2
        } else {
            1.0
        };
    let subjects = [object];
    let counterparts =
        Counterparts::find(context, config.counterparts, margin, &services, &subjects)?;
    if let Some(refused) = counterparts.unbounded.get(&object.id) {
        return Err(refused.clone());
    }
    let frame = config
        .infill
        .map(|(selector, _)| Counterparts::find(context, selector, margin, &services, &subjects))
        .transpose()?;
    let subject = Subject {
        context,
        config,
        services: &services,
        counterparts: &counterparts,
        frame: frame.as_ref(),
        object,
    };
    if check == Check::Elevation {
        return subject
            .elevation_share()
            .map(|(share, cover)| (share, cover.unknown.is_empty()));
    }
    let area = footprint(context, &object.id)?;
    let cover = subject.cover(&area);
    let read = cover.unknown.is_empty();
    match (check, config.horizontal, config.vertical, services.extents) {
        (Check::Height, _, Some(growth), Some(extents)) => {
            subject.height_share(extents, &cover, growth)
        }
        (_, Some(growth), _, _) => subject.plan_share(&area, &cover, growth),
        _ => Err(invalid("the check has no tolerance")),
    }
    .map(|share| (share, read))
}

impl MeasuredProvider for CoverageMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[NAME]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let refused = |(reason, why): Unavailable| {
            resolution_error((reason, format!("`{NAME}` of {object}: {why}")))
        };
        let check = match call.choice("measure") {
            Some("height") => Check::Height,
            Some("elevation") => Check::Elevation,
            _ => Check::Plan,
        };
        let counterparts = kinds(context, call, "by", object)?;
        let frame = match call.argument("frame") {
            Some(_) => Some(kinds(context, call, "frame", object)?),
            None => None,
        };
        let infill_above = length(call, "infill_above").unwrap_or(0.5);
        if !(0.0..1.0).contains(&infill_above) {
            return Err(refused(invalid("infill_above must lie in [0, 1)")));
        }
        if let Some(axis) = length(call, "axis_tolerance")
            && axis >= 45.0
        {
            return Err(refused(invalid(
                "axis_tolerance must lie in [0, 45) degrees",
            )));
        }
        let config = Config {
            counterparts: &counterparts,
            horizontal: Some(length(call, "horizontal").unwrap_or(0.0)),
            vertical: (check != Check::Plan).then(|| length(call, "vertical").unwrap_or(0.0)),
            bands: vec![(0.0, Severity::Info)],
            axis: length(call, "axis_tolerance"),
            elevation: check == Check::Elevation,
            infill: frame
                .as_ref()
                .filter(|_| check == Check::Elevation)
                .map(|frame| (frame, infill_above)),
        };
        let Some(object) = context
            .project
            .objects()
            .find(|candidate| candidate.id == *object)
        else {
            return Err(refused((
                NotEvaluatedReason::IncompleteEvidence,
                "the object is not in the project".into(),
            )));
        };
        let (
            Share {
                interval: (lower, upper),
                evidence,
                ..
            },
            read,
        ) = measure(context, &config, check, object).map_err(refused)?;
        let locator = format!("{NAME}:{}", object.id);
        // Measured from exact evidence, the share is cited exact, as the
        // capability cites it; its interval holds the undecided cover, not
        // only rounding, so it is cited rather than rounded.
        Ok(Measurement::Cited {
            lower,
            upper,
            dimension: None,
            locator,
            exact: read && evidence.iter().all(|cited| cited.exact),
        })
    }
}
