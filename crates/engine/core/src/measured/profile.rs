//! Values along a station range of an alignment, read off the profile the
//! alignment service measures ([`crate::station_profile`]): the greatest
//! and least value of a parameter between two stations, with the stations
//! where it may be attained.
//!
//! They are measured on the alignment itself: `of` names the parameter,
//! `from` and `to` the stations (the alignment's start and end where not
//! stated) and `step` the greatest width of a sampled stretch.

use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

use super::{Answer, Measures};
use crate::alignment::{AlignmentError, AlignmentParameter};
use crate::properties::PropertyResolutionError;
use crate::station_profile::{AlignmentProfileRequest, ProfileExtreme};

/// The greatest value along a station range.
pub(super) const MAX_ALONG: &str = "max_along";
/// The least value along a station range.
pub(super) const MIN_ALONG: &str = "min_along";

/// The names measured here.
pub(super) const NAMES: &[&str] = &[MAX_ALONG, MIN_ALONG];

/// At most this many station stretches are named in a value's evidence.
const NAMED: usize = 5;

/// The parameter `of` names.
fn parameter(call: &MeasuredCall) -> Result<AlignmentParameter, PropertyResolutionError> {
    Ok(match call.choice("of") {
        Some("gradient") => AlignmentParameter::Gradient,
        Some("curvature") => AlignmentParameter::Curvature,
        Some("cant") => AlignmentParameter::Cant,
        Some("cant_ratio") => AlignmentParameter::CantRatio,
        _ => return Err(PropertyResolutionError::InvalidRequest),
    })
}

fn length(call: &MeasuredCall, key: &str) -> Result<Option<f64>, PropertyResolutionError> {
    match call.argument(key) {
        None => Ok(None),
        Some(MeasuredArgument::Length(value)) => Ok(Some(*value)),
        Some(_) => Err(PropertyResolutionError::InvalidRequest),
    }
}

/// Station stretches for evidence, the first [`NAMED`].
fn stretches(stations: &[(f64, f64)]) -> String {
    let mut named: Vec<String> = stations
        .iter()
        .take(NAMED)
        .map(|(from, to)| {
            if (to - from).abs() <= 1e-6 * from.abs().max(1.0) {
                format!("station {:.6}", 0.5 * (from + to))
            } else {
                format!("stations {from:.6} to {to:.6}")
            }
        })
        .collect();
    if stations.len() > NAMED {
        named.push(format!("{} more", stations.len() - NAMED));
    }
    named.join(", ")
}

impl Measures {
    /// The greatest or least value of a parameter of the alignment
    /// `alignment` along a station range.
    pub(super) fn along_profile(
        &self,
        call: &MeasuredCall,
        alignment: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let service = self
            .alignments
            .as_ref()
            .ok_or_else(|| Self::missing(name, "alignment"))?;
        let parameter = parameter(call)?;
        let (from, to) = (length(call, "from")?, length(call, "to")?);
        if let (Some(from), Some(to)) = (from, to)
            && from > to
        {
            return Err(Self::unavailable(
                name,
                alignment,
                &format!("the range runs backwards, from station {from} to station {to}"),
            ));
        }
        let request = AlignmentProfileRequest::try_new(
            alignment.clone(),
            parameter,
            from,
            to,
            length(call, "step")?,
        )
        .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let profile = service
            .measure_alignment_profile(&request)
            .map_err(|error| match error {
                AlignmentError::InvalidMeasurement | AlignmentError::InexactEvidence => {
                    PropertyResolutionError::InvalidValue
                }
                other => Self::unavailable(name, alignment, &other.to_string()),
            })?;
        let locator = profile.evidence().locator.clone();
        let which = if name == MAX_ALONG {
            ProfileExtreme::Greatest
        } else {
            ProfileExtreme::Least
        };
        let Some(extreme) = profile.extreme(which) else {
            return Ok(Answer::Absent(format!(
                "{alignment} states no cant ({locator})"
            )));
        };
        let word = if which == ProfileExtreme::Greatest {
            "greatest"
        } else {
            "least"
        };
        let value = extreme.value();
        Ok(Answer::Value(
            value.lower(),
            value.upper(),
            None,
            format!("{locator}; {word} at {}", stretches(extreme.stations())),
            extreme.is_exact(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::sync::Arc;

    use axioval_ir::{Evidence, Object, Project, PropertyValue, SourceId};

    use super::*;
    use crate::alignment::{AlignmentInterval, AlignmentService, AlignmentServiceHandle};
    use crate::properties::PropertyResolution;
    use crate::station_profile::{AlignmentProfile, ProfilePiece};
    use crate::{AlignmentPosition, AlignmentRequest, ServiceRegistry, measured_value};

    fn source() -> SourceId {
        SourceId::new("test", "road").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn point(value: f64) -> AlignmentInterval {
        AlignmentInterval::exact(value).unwrap()
    }

    /// An axis stationed from 1000 whose gradient rises from 2 % over a
    /// vertical curve to a peak of 6.2 % at 160 m, falls to 3 % and holds
    /// it to 300 m; it states no cant, and refuses its curvature.
    struct Axis;

    impl AlignmentService for Axis {
        fn measure_alignment_position(
            &self,
            request: &AlignmentRequest,
        ) -> Result<AlignmentPosition, AlignmentError> {
            Err(AlignmentError::UnknownObject(request.object().clone()))
        }

        fn measure_alignment_profile(
            &self,
            request: &AlignmentProfileRequest,
        ) -> Result<AlignmentProfile, AlignmentError> {
            let evidence = |exact| {
                let mut evidence = Evidence::exact(source(), "gradient of #9");
                evidence.exact = exact;
                evidence
            };
            match request.parameter() {
                AlignmentParameter::Cant => {
                    return AlignmentProfile::try_new(request.clone(), None, evidence(true));
                }
                AlignmentParameter::Curvature => {
                    return Err(AlignmentError::Unavailable(
                        "a sinusoidal spiral has no certified bound here".into(),
                    ));
                }
                _ => {}
            }
            let from = request.from().unwrap_or(1000.0) - 1000.0;
            let to = request.to().unwrap_or(1300.0) - 1000.0;
            let grades = [
                (0.0, 100.0, 0.02, 0.02),
                (100.0, 160.0, 0.02, 0.062),
                (160.0, 240.0, 0.062, 0.03),
                (240.0, 300.0, 0.03, 0.03),
            ];
            let mut pieces = Vec::new();
            for (a, b, start, end) in grades {
                let (a, b, start, end): (f64, f64, f64, f64) = (a, b, start, end);
                let (low, high) = (a.max(from), b.min(to));
                if low >= high {
                    continue;
                }
                let rate = (end - start) / (b - a);
                let at = |x: f64| start + rate * (x - a);
                let value = |x: f64| {
                    if x == a {
                        point(start)
                    } else if x == b {
                        point(end)
                    } else {
                        point(at(x))
                    }
                };
                pieces.push(ProfilePiece::try_new(
                    AlignmentInterval::try_new(low, high)?,
                    1000.0 + low,
                    value(low),
                    value(high),
                    point(rate),
                )?);
            }
            AlignmentProfile::try_new(request.clone(), Some(pieces), evidence(true))
        }
    }

    fn value(name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
        let mut services = ServiceRegistry::new();
        services
            .register(AlignmentServiceHandle::new(Arc::new(Axis)))
            .unwrap();
        let project = Project::new(vec![Object::new(id("#9"), "IfcAlignment")]).unwrap();
        measured_value(&services, &project, &id("#9"), name)
    }

    fn read(resolution: &PropertyResolution) -> (f64, f64, String, bool) {
        let PropertyResolution::Present(resolved) = resolution else {
            panic!("expected a value: {resolution:?}");
        };
        let property = resolved.property();
        let evidence = property.evidence.as_ref().unwrap();
        let (lower, upper) = match property.value {
            PropertyValue::Measured { lower, upper, .. } => (lower, upper),
            PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => (value, value),
            ref other => panic!("not a number: {other:?}"),
        };
        (lower, upper, evidence.locator.clone(), evidence.exact)
    }

    #[test]
    fn the_steepest_gradient_is_its_peak_with_the_peak_station() {
        let (lower, upper, locator, exact) = read(&value("max_along;of=gradient").unwrap());
        assert_eq!((lower, upper), (0.062, 0.062));
        assert!(exact);
        assert!(
            locator.contains("greatest at station 1160.0000"),
            "{locator}"
        );
        // Over the first 150 m the curve is still rising.
        let (lower, upper, locator, _) =
            read(&value("max_along;of=gradient;from=1000;to=1130").unwrap());
        assert!((lower - 0.041).abs() < 1e-12 && (upper - 0.041).abs() < 1e-12);
        assert!(locator.contains("station 1130"), "{locator}");
        let (lower, _, locator, _) = read(&value("min_along;of=gradient").unwrap());
        assert_eq!(lower, 0.02);
        assert!(
            locator.contains("least at stations 1000.000000 to 1100.0000"),
            "{locator}"
        );
    }

    #[test]
    fn no_cant_is_an_absence_and_refusals_keep_their_reason() {
        assert!(matches!(
            value("max_along;of=cant").unwrap(),
            PropertyResolution::Absent(_)
        ));
        let Err(PropertyResolutionError::Unavailable(reason)) = value("max_along;of=curvature")
        else {
            panic!("an unbounded law must be refused");
        };
        assert!(reason.contains("sinusoidal spiral"), "{reason}");
        let Err(PropertyResolutionError::Unavailable(reason)) =
            value("min_along;of=gradient;from=1200;to=1100")
        else {
            panic!("a backward range must be refused");
        };
        assert!(reason.contains("runs backwards"), "{reason}");
    }
}
