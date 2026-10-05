//! `shelf-capacity`: the running metres of shelving a space holds, and
//! whether it is tall enough for the shelving.
//!
//! ADR 0004: the measurement (running metres of shelving) comes from a
//! [`LinearQuantityServiceHandle`]; the decision -- whether that clears the
//! declared minimum, and whether the space is tall enough for the shelving
//! -- is made in source-neutral policy.
//!
//! The doors and openings whose clearances carry no shelving are the rule's
//! selection: each space's are the elements `access_path` reaches it from,
//! read as `space-connection` reads them, and they travel in the request.
//! An element whose spaces cannot be read, or whose type is undecided and
//! that reaches the space, leaves the space not evaluated: its clearance
//! could take shelving away.
//!
//! It runs as a template ([`axioval_engine::template`]): the measured
//! `shelf_clear_height` against the shelving's top, then `shelf_length`
//! against the minimum, both measured with the rule's selectors bound into
//! them (`doors=@door_selector`).

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, LinearQuantityError, LinearQuantityEvidence,
    LinearQuantityKind, LinearQuantityRequest, LinearQuantityServiceHandle, NotEvaluatedReason,
    ParameterDescriptor, RuleCapability, RuleContext, ShelfGeometry,
};
use axioval_ir::{Evidence, ObjectId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::ShelfMeasures;

use crate::space_access::AccessIndex;
use crate::support::Unavailable;

/// Minimum running metres of shelving a space must provide.
pub struct ShelfCapacity;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ShelfCapacity {
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

/// What the service measured of one space's shelving, with the doors and
/// openings sent and the evidence of both.
#[derive(Clone)]
struct Shelving {
    measured: LinearQuantityEvidence,
    doors: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

/// Measures the shelving of `space`, its doors and openings sent in the
/// request; refused when they are unknown or the service cannot answer.
fn measure(
    service: &LinearQuantityServiceHandle,
    index: &AccessIndex,
    geometry: ShelfGeometry,
    space: &ObjectId,
) -> Result<Shelving, Unavailable> {
    let (doors, door_evidence) = index.reaching(space).map_err(|why| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("the doors and openings of {space} are unknown: {why}"),
        )
    })?;
    let request = LinearQuantityRequest::new(
        space.clone(),
        LinearQuantityKind::ShelfRunningLength(geometry),
    )
    .with_doors(doors.clone());
    let measured = match service.measure_linear_quantity(&request) {
        Ok(measured) if measured.request() == &request => measured,
        Ok(_) => {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                "the shelf length answers another request".into(),
            ));
        }
        Err(error) => return Err((reason(error), error.to_string())),
    };
    let mut evidence = vec![measured.evidence().clone()];
    evidence.extend(door_evidence);
    Ok(Shelving {
        measured,
        doors,
        evidence,
    })
}

fn reason(error: LinearQuantityError) -> NotEvaluatedReason {
    match error {
        LinearQuantityError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
        // Both mean the adapter produced something it cannot stand behind,
        // which is an evidence defect, not a missing measurement.
        LinearQuantityError::InexactEvidence | LinearQuantityError::InvalidInterval => {
            NotEvaluatedReason::InvalidEvidence
        }
        // The arrangement was validated before the request, so reaching here
        // means the declaration, not the model.
        LinearQuantityError::InvalidGeometry => NotEvaluatedReason::InvalidDeclaration,
    }
}
