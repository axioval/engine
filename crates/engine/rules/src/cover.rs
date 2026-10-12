//! The cover of an embedded object to its host's surface, `cover`: the
//! least distance from the object's whole body to the chosen faces of the
//! one host a relationship path reaches.
//!
//! The proximity service measures the signed distance from the body to one
//! class of the host's faces (`measure_face_distance`), positive inside the
//! host; the faces the object is meant to pass through are left out by
//! naming the others (`faces`). Several classes measure to their union: the
//! least of each class's distance. Only a body proven wholly inside its host
//! has a cover; one reaching out of it, or whose side the service cannot
//! decide, is not evaluated, never a negative cover.

use axioval_engine::{
    Citation, FaceClass, FaceDistanceRequest, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, ProximityServiceHandle, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::containment::face_reason;
use crate::measured_kinds::{interval, refused};
use crate::support::{Traversal, Unavailable};

/// Measures `cover`.
pub(crate) struct CoverMeasures;

const COVER: &str = "cover";

impl MeasuredProvider for CoverMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[COVER]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        cover(call, object, context).map_err(refused(call.name(), object))
    }
}

/// The face classes `faces` names: every face where it names all three.
fn classes(call: &MeasuredCall) -> Vec<FaceClass> {
    let named: Vec<FaceClass> = match call.argument("faces") {
        Some(MeasuredArgument::Choices(names)) => names
            .iter()
            .filter_map(|name| FaceClass::parse(name))
            .collect(),
        _ => Vec::new(),
    };
    if named.is_empty() || named.len() == 3 {
        vec![FaceClass::Any]
    } else {
        named
    }
}

fn cover(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let service = context
        .services
        .get::<ProximityServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "proximity service is not registered".to_owned(),
            )
        })?;
    let Some(MeasuredArgument::Path(steps)) = call.argument("host_path") else {
        return Err(crate::support::invalid("`host_path` is required"));
    };
    let everything: Vec<&Object> = context.project.objects().collect();
    let (hosts, reached) = Traversal::path(steps)?.related(context, object, &everything)?;
    let host = match hosts.as_slice() {
        [] => {
            return Ok((
                Measurement::Absent {
                    locator: format!("{COVER}:{object}: the path reaches no host"),
                },
                Citation {
                    evidence: reached,
                    ..Citation::default()
                },
            ));
        }
        [host] => host.clone(),
        several => {
            let named: Vec<String> = several.iter().map(ToString::to_string).collect();
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "the path reaches several hosts ({}), so the one it is embedded in is \
                     unknown",
                    named.join(", ")
                ),
            ));
        }
    };
    let (lower, upper, exact, mut evidence) = distances(service, call, object, &host)?;
    let metres = |lower: f64, upper: f64| {
        #[allow(clippy::float_cmp)]
        if lower == upper {
            format!("{lower:.4} m")
        } else {
            format!("between {lower:.4} and {upper:.4} m")
        }
    };
    if upper < 0.0 {
        return Err((
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "it is not wholly inside its host {host}: it reaches {} past the faces \
                 measured to, so it has no cover",
                metres(-upper, -lower)
            ),
        ));
    }
    if lower < 0.0 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "whether it lies wholly inside its host {host} cannot be decided: its signed \
                 distance to the faces measured to is {}",
                metres(lower, upper)
            ),
        ));
    }
    let locator = std::iter::once(format!("{COVER}:{object}->{host}"))
        .chain(evidence.iter().map(|item| item.locator.clone()))
        .collect::<Vec<_>>()
        .join(";");
    evidence.extend(reached);
    Ok((
        interval(
            (lower, upper),
            Some(QuantityDimension::Length),
            exact,
            locator,
        ),
        Citation {
            related: vec![host],
            evidence,
            ..Citation::default()
        },
    ))
}

/// The least signed distance from `object` to each class of `host`'s faces
/// `faces` names, `(lower, upper, exact, evidence)`.
fn distances(
    service: &ProximityServiceHandle,
    call: &MeasuredCall,
    object: &ObjectId,
    host: &ObjectId,
) -> Result<(f64, f64, bool, Vec<Evidence>), Unavailable> {
    let (mut lower, mut upper) = (f64::INFINITY, f64::INFINITY);
    let mut exact = true;
    let mut evidence: Vec<Evidence> = Vec::new();
    for class in classes(call) {
        let measured = FaceDistanceRequest::try_new(object.clone(), host.clone(), class)
            .and_then(|request| service.measure_face_distance(&request))
            .map_err(|error| {
                (
                    face_reason(error),
                    format!(
                        "its distance to the {} faces of its host {host} could not be measured: \
                         {error}",
                        class.name()
                    ),
                )
            })?;
        let signed = measured.signed();
        lower = lower.min(signed.lower_metres());
        upper = upper.min(signed.upper_metres());
        exact &= measured.evidence().exact;
        evidence.push(measured.evidence().clone());
    }
    Ok((lower, upper, exact, evidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> MeasuredCall {
        axioval_ir::measured::parse(name).unwrap()
    }

    #[test]
    fn faces_name_one_class_each_and_all_three_every_face() {
        assert_eq!(classes(&call("cover;host_path=r")), vec![FaceClass::Any]);
        assert_eq!(
            classes(&call("cover;host_path=r;faces=bottom,top")),
            vec![FaceClass::Bottom, FaceClass::Top]
        );
        assert_eq!(
            classes(&call("cover;host_path=r;faces=side,top,bottom")),
            vec![FaceClass::Any]
        );
    }
}
