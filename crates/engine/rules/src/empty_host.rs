//! `empty-host`: a host (a wall) whose face its openings void wholly.

use crate::counts::Population;
use crate::opening_area::{Openings, area_tolerance, square_metres, voided};
use crate::opening_zone::face::{Axis, FaceAxes, Host, ROUNDING};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};

/// Reports each selected host whose openings void its whole face, less
/// `area_tolerance`: a wall that is one opening, or openings side by side
/// with no wall left between them.
///
/// The openings are what `opening_path` reaches from the host among the
/// `opening_selector` objects, and each is placed and summed as
/// `opening-area` sums them: on the host's middle plane, from the reserved
/// body set, clear of each other and within the face, openings below
/// `minimum_opening_area` left out. The face is the host's section across
/// its middle plane: the length and height of its box, or where its outline
/// is free, its outline's chord along the face at the middle of the host's
/// thickness times its height (the outline's area when the face is the
/// section itself).
///
/// A host with no openings is not empty. An opening it cannot place, or
/// openings that may overlap, leave the host not evaluated.
pub struct EmptyHost;

impl RuleCapability for EmptyHost {
    fn id(&self) -> &'static str {
        "axioval:capability.empty-host"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = Openings::parameters();
        parameters.push(ParameterDescriptor::optional(
            "area_tolerance",
            ParameterType::Quantity,
        ));
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parsed = (|| {
            let parameters = Parameters(rule);
            let openings = Openings::parse(&parameters)?;
            Ok::<_, Unavailable>((openings, area_tolerance(&parameters)?))
        })();
        let (openings, tolerance) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("empty-host: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let population = Population::of(context, openings.selector);
        for host in selected {
            let judged = (|| {
                let mut evidence = Vec::new();
                let void = voided(context, &openings, &population, host, &mut evidence)?;
                if void.counted.is_empty() {
                    return Ok(None);
                }
                let face = face_area(&void.face, openings.axes)?;
                let slack = tolerance + ROUNDING * (1.0 + face + void.sum);
                if void.sum < face - slack {
                    return Ok(None);
                }
                Ok(Some((
                    format!(
                        "host is empty: its openings ({}) void {} of its {} face",
                        void.counted
                            .iter()
                            .map(|id| id.local_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        square_metres(void.sum),
                        square_metres(face)
                    ),
                    evidence,
                    void.counted,
                )))
            })();
            match judged {
                Ok(None) => {}
                Ok(Some((message, evidence, related))) => {
                    evaluation.push_finding(finding(rule, &host.id, message, evidence, related));
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(host.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

/// The area of the host's face on its middle plane.
fn face_area(host: &Host, axes: FaceAxes) -> Result<f64, Unavailable> {
    let (_, length) = host.axis(axes.length);
    let (_, height) = host.axis(axes.height);
    let through = axes.through();
    let Some(outline) = &host.outline else {
        return Ok((length.1 - length.0) * (height.1 - height.0));
    };
    if through == Axis::Extrusion {
        return Ok(outline.area());
    }
    // The face runs along the other profile axis and the extrusion.
    let (along, depth) = if axes.length == Axis::Extrusion {
        (axes.height, length)
    } else {
        (axes.length, height)
    };
    let (_, across) = host.axis(through);
    let at = f64::midpoint(across.0, across.1);
    let chord = outline
        .chord(usize::from(along == Axis::ProfileY), at)
        .ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                "its outline's edge runs along its middle plane, so its face is undecided"
                    .to_owned(),
            )
        })?;
    Ok(chord * (depth.1 - depth.0))
}
