//! The member list `alignment_elements` ([`AlignmentElementMeasures`]):
//! the elements of one layout of an alignment, in order along it, each
//! with its kind, stations, length, the curvature and radius at both ends,
//! its one radius, a clothoid's parameter and a vertical element's
//! gradients, as the alignment service reads them
//! ([`crate::alignment_element`]).
//!
//! Radii, curvatures and clothoid parameters are derived from what each
//! element states, never accepted from the service. A straight end has
//! zero curvature and an unbounded radius: its radius is undecided, never
//! a number, as `alignment_radius` refuses one on a straight. An end the
//! service cannot read leaves every field derived from it undecided, with
//! the reason; a value the element does not have (the clothoid parameter
//! of an arc, the gradient of a plan element, the one radius of a
//! transition) is `null`.

use std::collections::BTreeMap;

use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Evidence, MEASURED_SET, ObjectId, QuantityDimension};

use super::provider::{MeasuredMember, MeasuredProvider, Measurement, MemberValue};
use crate::RuleContext;
use crate::alignment::{AlignmentError, AlignmentInterval, AlignmentServiceHandle};
use crate::alignment_element::{AlignmentElementsRequest, AlignmentLayout, ElementEnd};
use crate::properties::PropertyResolutionError;

/// The member list measured here.
const ALIGNMENT_ELEMENTS: &str = "alignment_elements";

/// Measures the member list `alignment_elements` over the alignment
/// service.
#[derive(Clone, Copy, Debug, Default)]
pub struct AlignmentElementMeasures;

impl AlignmentElementMeasures {
    #[allow(clippy::too_many_lines)] // One field after another.
    fn elements(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let refused = |why: String| {
            PropertyResolutionError::Unavailable(format!(
                "`{MEASURED_SET}` member list `{ALIGNMENT_ELEMENTS}` of {object}: {why}"
            ))
        };
        let service = context
            .services
            .get::<AlignmentServiceHandle>()
            .ok_or_else(|| {
                PropertyResolutionError::MissingService(format!(
                    "no alignment service is registered, so `{ALIGNMENT_ELEMENTS}` cannot be \
                     measured"
                ))
            })?;
        let layout = match call.choice("layout") {
            Some("horizontal") => AlignmentLayout::Horizontal,
            Some("vertical") => AlignmentLayout::Vertical,
            _ => return Err(PropertyResolutionError::InvalidRequest),
        };
        let request = AlignmentElementsRequest::new(object.clone(), layout);
        let read = service
            .measure_alignment_elements(&request)
            .map_err(|error| match error {
                AlignmentError::InvalidMeasurement | AlignmentError::InexactEvidence => {
                    PropertyResolutionError::InvalidValue
                }
                other => refused(other.to_string()),
            })?;
        let exact = read.evidence().exact;
        let list = &read.evidence().locator;
        let length = Some(QuantityDimension::Length);
        let mut members = Vec::with_capacity(read.elements().len());
        for (index, element) in read.elements().iter().enumerate() {
            let at = |field: &str| format!("{list}#element={}:{field}", index + 1);
            let number = |(lower, upper): (f64, f64), dimension, field: &str| {
                MemberValue::Measured(Measurement::Rounded {
                    lower,
                    upper,
                    dimension,
                    locator: at(field),
                })
            };
            let interval = |value: AlignmentInterval, dimension, field: &str| {
                number((value.lower(), value.upper()), dimension, field)
            };
            let null =
                |field: &str| MemberValue::Measured(Measurement::Absent { locator: at(field) });
            let undecided = |why: String| MemberValue::Undecided {
                why: format!("{}: {why}", element.locator()),
            };
            let curvature = |end: &ElementEnd, field: &str| match end.curvature() {
                Ok(value) => interval(value, None, field),
                Err(why) => undecided(why),
            };
            let radius = |end: &ElementEnd, field: &str| match end.radius() {
                Ok(value) => number((value, value), length, field),
                Err(why) => undecided(why),
            };
            let one_radius = match element.radius() {
                Ok(Some(value)) => number((value, value), length, "radius"),
                Ok(None) => null("radius"),
                Err(why) => undecided(why),
            };
            let clothoid = match element.clothoid_parameter() {
                Ok(Some(value)) => interval(value, length, "clothoid_parameter"),
                Ok(None) => null("clothoid_parameter"),
                Err(why) => undecided(why),
            };
            let (start_gradient, end_gradient) = match element.gradients() {
                Some((start, end)) => (
                    number((start, start), None, "start_gradient"),
                    number((end, end), None, "end_gradient"),
                ),
                None => (null("start_gradient"), null("end_gradient")),
            };
            #[allow(clippy::cast_precision_loss)] // A layout's element count.
            let position = (index + 1) as f64;
            members.push(MeasuredMember {
                certain: true,
                exact,
                fields: BTreeMap::from([
                    ("index", number((position, position), None, "index")),
                    (
                        "kind",
                        MemberValue::Text {
                            text: element.kind().name().to_owned(),
                        },
                    ),
                    (
                        "type",
                        MemberValue::Text {
                            text: element.type_name().to_owned(),
                        },
                    ),
                    (
                        "element",
                        MemberValue::Text {
                            text: element.locator().to_owned(),
                        },
                    ),
                    ("from", interval(element.from(), length, "from")),
                    ("to", interval(element.to(), length, "to")),
                    (
                        "length",
                        number((element.length(), element.length()), length, "length"),
                    ),
                    (
                        "start_curvature",
                        curvature(element.start(), "start_curvature"),
                    ),
                    ("end_curvature", curvature(element.end(), "end_curvature")),
                    ("start_radius", radius(element.start(), "start_radius")),
                    ("end_radius", radius(element.end(), "end_radius")),
                    ("radius", one_radius),
                    ("clothoid_parameter", clothoid),
                    ("start_gradient", start_gradient),
                    ("end_gradient", end_gradient),
                ]),
                evidence: Vec::new(),
            });
        }
        Ok((members, vec![read.evidence().clone()]))
    }
}

impl MeasuredProvider for AlignmentElementMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ALIGNMENT_ELEMENTS]
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        Self::elements(call, object, context).map(|(members, _)| members)
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        Self::elements(call, object, context)
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::MissingService(format!(
            "`{}` of {object} is a member list, not a measured value",
            call.name()
        )))
    }
}
