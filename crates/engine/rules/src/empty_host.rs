//! `empty-host`: a host (a wall) whose face its openings void wholly.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

use crate::opening_zone::face::{Axis, FaceAxes, Host};
use crate::support::Unavailable;

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
///
/// It runs as a template ([`axioval_engine::template`]): the truth that a
/// host with an opening on its middle plane (`opening_count`) keeps more
/// of its face (`middle_face_area`) than its openings take
/// (`opening_area`) and the tolerance, judged by the truth judge.
pub struct EmptyHost;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for EmptyHost {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The area of the host's face on its middle plane.
pub(crate) fn face_area(host: &Host, axes: FaceAxes) -> Result<f64, Unavailable> {
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
