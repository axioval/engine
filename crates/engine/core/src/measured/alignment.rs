//! Positions along an alignment as values: the station, signed offset and
//! height above the gradient line of an object's reference point, and the
//! alignment's curvature, radius, gradient and cant at that station, each
//! as the alignment service measures it ([`crate::alignment`]).
//!
//! The alignment is the one object of the `alignment` kinds that `path`
//! reaches from the object, or, without a path, the one in the object's
//! source. None reached is an exact absence; several are refused, since
//! picking one would measure along a guess.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::alignment::{
    AlignmentError, AlignmentInterval, AlignmentParameter, AlignmentParameterRequest,
    AlignmentRequest,
};
use crate::path::PathSegment;
use crate::properties::PropertyResolutionError;

/// The station along the alignment.
pub(super) const STATION: &str = "station";
/// The signed plan offset from the alignment.
pub(super) const OFFSET: &str = "offset";
/// The height above the gradient line.
pub(super) const HEIGHT: &str = "height_above_gradient";
/// The alignment's cant at the object's station.
pub(super) const CANT: &str = "alignment_cant";
/// The alignment's plan curvature at the object's station.
pub(super) const CURVATURE: &str = "alignment_curvature";
/// The alignment's gradient at the object's station.
pub(super) const GRADIENT: &str = "alignment_gradient";
/// The alignment's plan radius at the object's station.
pub(super) const RADIUS: &str = "alignment_radius";

/// The names measured here.
pub(super) const NAMES: &[&str] = &[CANT, CURVATURE, GRADIENT, RADIUS, HEIGHT, OFFSET, STATION];

impl Measures {
    /// The one alignment `call` selects for `object`, `None` when none is
    /// selected.
    pub(super) fn alignment_of(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Option<ObjectId>, PropertyResolutionError> {
        let name = call.name();
        let mut candidates = self.of_kinds(call, "alignment", object)?;
        if let Some(MeasuredArgument::Path(steps)) = call.argument("path") {
            let steps: Vec<PathSegment> = steps
                .iter()
                .map(|step| PathSegment::parse(step))
                .collect::<Result<_, _>>()
                .map_err(|_| PropertyResolutionError::InvalidRequest)?;
            let (reached, _) = self.reach(name, &steps, object)?;
            candidates.retain(|candidate| reached.contains(candidate));
        } else {
            candidates.retain(|candidate| candidate.source == object.source);
        }
        match candidates.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some(only.clone())),
            [first, second, ..] => Err(Self::unavailable(
                name,
                object,
                &format!(
                    "{} alignments are selected ({first}, {second}, ...); name one with `path`",
                    candidates.len()
                ),
            )),
        }
    }

    /// A position along an alignment, or a parameter of it, of `object`.
    pub(super) fn along_alignment(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let refused = |error: AlignmentError| match error {
            AlignmentError::InvalidMeasurement | AlignmentError::InexactEvidence => {
                PropertyResolutionError::InvalidValue
            }
            other => Self::unavailable(name, object, &other.to_string()),
        };
        let service = self
            .alignments
            .as_ref()
            .ok_or_else(|| Self::missing(name, "alignment"))?;
        let Some(alignment) = self.alignment_of(call, object)? else {
            return Ok(Answer::Absent(format!(
                "no alignment of the stated kinds is selected for {object}"
            )));
        };
        let request = AlignmentRequest::try_new(object.clone(), alignment.clone())
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let position = service
            .measure_alignment_position(&request)
            .map_err(refused)?;
        let exact = position.evidence().exact;
        let locator = position.evidence().locator.clone();
        let length = Some(QuantityDimension::Length);
        let cited = |value: AlignmentInterval, dimension, locator: String, exact| {
            Answer::Cited(value.lower(), value.upper(), dimension, locator, exact)
        };
        let parameter = match name {
            STATION => return Ok(cited(position.station(), length, locator, exact)),
            HEIGHT => return Ok(cited(position.height(), length, locator, exact)),
            OFFSET => {
                let offset = position.offset();
                return Ok(if call.choice("side") == Some("right") {
                    Answer::Cited(-offset.upper(), -offset.lower(), length, locator, exact)
                } else {
                    cited(offset, length, locator, exact)
                });
            }
            CANT => AlignmentParameter::Cant,
            GRADIENT => AlignmentParameter::Gradient,
            _ => AlignmentParameter::Curvature,
        };
        let request =
            AlignmentParameterRequest::try_new(alignment.clone(), parameter, position.distance())
                .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let answer = service
            .measure_alignment_parameter(&request)
            .map_err(refused)?;
        let locator = format!("{locator}; {}", answer.evidence().locator);
        let exact = exact && answer.evidence().exact;
        let Some(value) = answer.value() else {
            return Ok(Answer::Absent(format!(
                "{alignment} states no cant ({locator})"
            )));
        };
        Ok(match name {
            CANT => cited(value, length, locator, exact),
            RADIUS => {
                let (lower, upper) = radius(value).ok_or_else(|| {
                    Self::unavailable(
                        name,
                        object,
                        &format!(
                            "the curvature of {alignment} there lies in [{}, {}] per metre, \
                             which may be straight, so its radius is unbounded",
                            value.lower(),
                            value.upper()
                        ),
                    )
                })?;
                Answer::Cited(lower, upper, length, locator, false)
            }
            _ => cited(value, None, locator, exact),
        })
    }
}

/// The unsigned radius `1 / |k|` of every curvature in `curvature`, as an
/// interval rounded outwards; `None` when it may be zero.
fn radius(curvature: AlignmentInterval) -> Option<(f64, f64)> {
    let (low, high) = (curvature.lower(), curvature.upper());
    if low <= 0.0 && high >= 0.0 {
        return None;
    }
    let (least, most) = if low > 0.0 {
        (low, high)
    } else {
        (-high, -low)
    };
    let (lower, upper) = (1.0 / most, 1.0 / least);
    Some((lower.next_down().max(0.0), upper.next_up()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axioval_ir::{Evidence, MEASURED_SET, Object, Project, PropertyValue, SourceId};

    use super::*;
    use crate::alignment::{
        AlignmentParameterValue, AlignmentPosition, AlignmentService, AlignmentServiceHandle,
    };
    use crate::properties::PropertyResolution;
    use crate::{ServiceRegistry, measured_value};

    fn source() -> SourceId {
        SourceId::new("test", "bridge").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn interval(lower: f64, upper: f64) -> AlignmentInterval {
        AlignmentInterval::try_new(lower, upper).unwrap()
    }

    /// A pier 5 m along the axis, 2 m to its left and 1.5 m above the
    /// gradient line; the kerb stands beyond the end. The axis states a
    /// curvature of 1/500 per metre, a gradient of 2 % and no cant.
    struct Axis {
        asked: Mutex<Vec<AlignmentParameter>>,
    }

    fn evidence(locator: &str) -> Evidence {
        let mut evidence = Evidence::exact(source(), locator);
        evidence.exact = false;
        evidence
    }

    impl AlignmentService for Axis {
        fn measure_alignment_position(
            &self,
            request: &AlignmentRequest,
        ) -> Result<AlignmentPosition, AlignmentError> {
            match request.object().local_id.as_str() {
                "pier" => AlignmentPosition::try_new(
                    request.clone(),
                    interval(4.999, 5.001),
                    interval(104.999, 105.001),
                    interval(1.999, 2.001),
                    interval(1.499, 1.501),
                    evidence("along #175"),
                ),
                "kerb" => Err(AlignmentError::OffRange(
                    "it lies 3 m beyond the end of #175".into(),
                )),
                _ => Err(AlignmentError::UnknownObject(request.object().clone())),
            }
        }

        fn measure_alignment_parameter(
            &self,
            request: &AlignmentParameterRequest,
        ) -> Result<AlignmentParameterValue, AlignmentError> {
            self.asked.lock().unwrap().push(request.parameter());
            assert_eq!(request.distance(), interval(4.999, 5.001));
            let value = match request.parameter() {
                AlignmentParameter::Curvature => Some(interval(0.002, 0.002)),
                AlignmentParameter::Gradient => Some(interval(0.0199, 0.0201)),
                AlignmentParameter::Cant => None,
            };
            let mut evidence = evidence("parameters of #175");
            evidence.exact = value.is_none_or(|value| value.is_exact());
            AlignmentParameterValue::try_new(request.clone(), value, evidence)
        }
    }

    fn project(alignments: &[&str]) -> Project {
        let mut objects = vec![
            Object::new(id("pier"), "IfcPier"),
            Object::new(id("kerb"), "IfcKerb"),
        ];
        objects.extend(
            alignments
                .iter()
                .map(|local| Object::new(id(local), "IfcAlignment")),
        );
        Project::new(objects).unwrap()
    }

    fn services() -> ServiceRegistry {
        let mut services = ServiceRegistry::new();
        services
            .register(AlignmentServiceHandle::new(Arc::new(Axis {
                asked: Mutex::new(Vec::new()),
            })))
            .unwrap();
        services
    }

    fn value(object: &str, name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
        measured_value(&services(), &project(&["axis"]), &id(object), name)
    }

    fn interval_of(resolution: &PropertyResolution) -> (f64, f64, Option<QuantityDimension>) {
        let PropertyResolution::Present(resolved) = resolution else {
            panic!("expected a value: {resolution:?}");
        };
        match resolved.property().value {
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            } => (lower, upper, dimension),
            PropertyValue::Quantity { value, dimension } => (value, value, Some(dimension)),
            ref other => panic!("not an interval: {other:?}"),
        }
    }

    #[test]
    fn every_position_is_the_service_interval() {
        let length = Some(QuantityDimension::Length);
        for (name, expected) in [
            ("station;alignment=IfcAlignment", (104.999, 105.001)),
            ("offset;alignment=IfcAlignment", (1.999, 2.001)),
            ("offset;alignment=IfcAlignment;side=left", (1.999, 2.001)),
            ("offset;alignment=IfcAlignment;side=right", (-2.001, -1.999)),
            (
                "height_above_gradient;alignment=IfcAlignment",
                (1.499, 1.501),
            ),
        ] {
            let resolution = value("pier", name).unwrap();
            assert_eq!(
                interval_of(&resolution),
                (expected.0, expected.1, length),
                "{name}"
            );
            let PropertyResolution::Present(resolved) = &resolution else {
                unreachable!()
            };
            let evidence = resolved.property().evidence.as_ref().unwrap();
            assert!(!evidence.exact, "{name}");
            assert!(
                evidence.locator.starts_with(&format!("{MEASURED_SET}/"))
                    && evidence.locator.contains("along #175"),
                "{}",
                evidence.locator
            );
        }
    }

    #[test]
    fn parameters_are_read_at_the_station_interval() {
        let (lower, upper, dimension) =
            interval_of(&value("pier", "alignment_curvature;alignment=IfcAlignment").unwrap());
        assert_eq!((lower, upper, dimension), (0.002, 0.002, None));
        let (lower, upper, dimension) =
            interval_of(&value("pier", "alignment_radius;alignment=IfcAlignment").unwrap());
        assert_eq!(dimension, Some(QuantityDimension::Length));
        assert!(lower <= 500.0 && upper >= 500.0 && upper - lower < 1e-9);
        let (lower, upper, dimension) =
            interval_of(&value("pier", "alignment_gradient;alignment=IfcAlignment").unwrap());
        assert_eq!((lower, upper, dimension), (0.0199, 0.0201, None));
        // No cant stated is an absence, not a zero.
        assert!(matches!(
            value("pier", "alignment_cant;alignment=IfcAlignment").unwrap(),
            PropertyResolution::Absent(_)
        ));
    }

    #[test]
    fn refusals_leave_the_value_not_evaluated_with_the_reason() {
        let Err(PropertyResolutionError::Unavailable(reason)) =
            value("kerb", "station;alignment=IfcAlignment")
        else {
            panic!("off range must be refused");
        };
        assert!(
            reason.contains("off the alignment's range") && reason.contains("beyond the end"),
            "{reason}"
        );
        // Two alignments in the source: none is picked.
        let Err(PropertyResolutionError::Unavailable(reason)) = measured_value(
            &services(),
            &project(&["axis", "second"]),
            &id("pier"),
            "station;alignment=IfcAlignment",
        ) else {
            panic!("several alignments must be refused");
        };
        assert!(reason.contains("2 alignments"), "{reason}");
        // None: an exact absence.
        assert!(matches!(
            measured_value(
                &services(),
                &project(&[]),
                &id("pier"),
                "station;alignment=IfcAlignment"
            )
            .unwrap(),
            PropertyResolution::Absent(_)
        ));
    }

    #[test]
    fn a_radius_where_the_alignment_may_run_straight_is_refused() {
        assert!(radius(interval(-0.001, 0.002)).is_none());
        assert!(radius(interval(0.0, 0.0)).is_none());
        let (lower, upper) = radius(interval(-0.004, -0.002)).unwrap();
        assert!(lower <= 250.0 && upper >= 500.0);
    }

    #[test]
    fn without_a_service_the_value_is_missing_for_the_run() {
        let error = measured_value(
            &ServiceRegistry::new(),
            &project(&["axis"]),
            &id("pier"),
            "station;alignment=IfcAlignment",
        )
        .unwrap_err();
        assert!(
            matches!(error, PropertyResolutionError::MissingService(ref reason) if reason.contains("alignment")),
            "{error:?}"
        );
    }
}
