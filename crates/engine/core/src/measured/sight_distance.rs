//! The available sight distance along an alignment as a value, as the
//! alignment service measures it ([`crate::sight_distance`]).
//!
//! It is measured on the alignment itself: `station` names where the eye
//! stands, `eye_height`, `eye_offset`, `target_height` and
//! `target_offset` place the eye and the target in the section, `range`
//! and `step` how far ahead and how finely targets are looked at, and the
//! bodies of the `obstacles` kinds in the alignment's source are what the
//! sight line must clear besides the gradient line.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::alignment::AlignmentError;
use crate::properties::PropertyResolutionError;
use crate::sight_distance::{SightDistanceRequest, SightObstruction};

/// The available sight distance at a station.
pub(super) const AVAILABLE: &str = "available_sight_distance";

/// The names measured here.
pub(super) const NAMES: &[&str] = &[AVAILABLE];

/// At most this many obstructions are named in a value's evidence.
const NAMED: usize = 5;

impl Measures {
    /// The available sight distance along `alignment`.
    pub(super) fn sight_distance(
        &self,
        call: &MeasuredCall,
        alignment: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let service = self
            .alignments
            .as_ref()
            .ok_or_else(|| Self::missing(name, "alignment"))?;
        let length = |key: &str| match call.argument(key) {
            Some(MeasuredArgument::Length(value)) => Ok(*value),
            _ => Err(PropertyResolutionError::InvalidRequest),
        };
        let obstacles = if call.argument("obstacles").is_some() {
            let mut bodies = self.of_kinds(call, "obstacles", alignment)?;
            bodies.retain(|body| body.source == alignment.source);
            bodies
        } else {
            Vec::new()
        };
        let request = SightDistanceRequest::try_new(
            alignment.clone(),
            length("station")?,
            [length("eye_offset")?, length("eye_height")?],
            [length("target_offset")?, length("target_height")?],
            (length("range")?, length("step")?),
            obstacles,
        )
        .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let sight = service
            .measure_sight_distance(&request)
            .map_err(|error| match error {
                AlignmentError::InvalidMeasurement | AlignmentError::InexactEvidence => {
                    PropertyResolutionError::InvalidValue
                }
                other => Self::unavailable(name, alignment, &other.to_string()),
            })?;
        let distance = sight.distance();
        let mut locator = sight.evidence().locator.clone();
        let named: Vec<String> = sight
            .blocked_by()
            .iter()
            .map(|obstruction| match obstruction {
                SightObstruction::GradientLine => "the gradient line".to_owned(),
                SightObstruction::Body(body) => body.to_string(),
            })
            .collect();
        if !named.is_empty() {
            locator.push_str("; blocked at ");
            locator.push_str(&distance.upper().to_string());
            locator.push_str(" m by ");
            locator.push_str(&named[..named.len().min(NAMED)].join(", "));
            if named.len() > NAMED {
                locator.push_str(" and ");
                locator.push_str(&(named.len() - NAMED).to_string());
                locator.push_str(" more");
            }
        }
        Ok(Answer::Value(
            distance.lower(),
            distance.upper(),
            Some(QuantityDimension::Length),
            locator,
            sight.evidence().exact,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::sync::Arc;

    use axioval_ir::{Evidence, Object, Project, PropertyValue, SourceId};

    use super::*;
    use crate::alignment::{
        AlignmentInterval, AlignmentPosition, AlignmentRequest, AlignmentService,
        AlignmentServiceHandle,
    };
    use crate::properties::PropertyResolution;
    use crate::sight_distance::SightDistance;
    use crate::{ServiceRegistry, measured_value};

    fn source() -> SourceId {
        SourceId::new("test", "road").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    /// Clear to the range without obstacles; with them the wall blocks at
    /// 66.1 m and the sight line is proven clear to 66 m; beyond station
    /// 2000 the eye is off the alignment.
    struct Axis;

    impl AlignmentService for Axis {
        fn measure_alignment_position(
            &self,
            request: &AlignmentRequest,
        ) -> Result<AlignmentPosition, AlignmentError> {
            Err(AlignmentError::UnknownObject(request.object().clone()))
        }

        fn measure_sight_distance(
            &self,
            request: &SightDistanceRequest,
        ) -> Result<SightDistance, AlignmentError> {
            if request.station() > 2000.0 {
                return Err(AlignmentError::OffRange(
                    "station 2500 lies beyond the end".into(),
                ));
            }
            assert_eq!(request.eye(), [0.0, 1.0]);
            assert_eq!(request.target(), [-1.5, 0.15]);
            let (distance, blocked_by, exact) = if request.obstacles().is_empty() {
                (AlignmentInterval::exact(request.range())?, Vec::new(), true)
            } else {
                (
                    AlignmentInterval::try_new(66.0, 66.1)?,
                    vec![SightObstruction::Body(id("wall"))],
                    false,
                )
            };
            let mut evidence = Evidence::exact(source(), "sight along #9");
            evidence.exact = exact;
            SightDistance::try_new(request.clone(), distance, blocked_by, evidence)
        }
    }

    fn value(name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
        let mut services = ServiceRegistry::new();
        services
            .register(AlignmentServiceHandle::new(Arc::new(Axis)))
            .unwrap();
        let project = Project::new(vec![
            Object::new(id("#9"), "IfcAlignment"),
            Object::new(id("wall"), "IfcWall"),
        ])
        .unwrap();
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
            PropertyValue::Quantity { value, .. } => (value, value),
            ref other => panic!("not a length: {other:?}"),
        };
        (lower, upper, evidence.locator.clone(), evidence.exact)
    }

    const CALL: &str = "available_sight_distance;station=1000;eye_height=1;target_height=0.15;\
                        target_offset=-1.5;range=250";

    #[test]
    fn a_clear_view_is_the_range_and_a_blocked_one_names_the_obstacle() {
        let (lower, upper, _, exact) = read(&value(CALL).unwrap());
        assert_eq!((lower, upper), (250.0, 250.0));
        assert!(exact);
        let (lower, upper, locator, exact) =
            read(&value(&format!("{CALL};obstacles=IfcWall")).unwrap());
        assert_eq!((lower, upper), (66.0, 66.1));
        assert!(!exact);
        assert!(
            locator.contains("blocked at 66.1 m by test:road/wall"),
            "{locator}"
        );
    }

    #[test]
    fn an_eye_off_the_alignment_is_refused_with_the_reason() {
        let Err(PropertyResolutionError::Unavailable(reason)) =
            value(&CALL.replace("station=1000", "station=2500"))
        else {
            panic!("an eye beyond the end must be refused");
        };
        assert!(reason.contains("beyond the end"), "{reason}");
        assert!(value(&CALL.replace("eye_height=1", "eye_height=0")).is_err());
    }
}
