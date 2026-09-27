//! Shared set-up for capabilities that check subjects against counterparts.
//!
//! Both groups are selected, every selected object's extent is requested, and
//! the engine's broad phase proposes the pairs worth measuring. An object whose
//! extent cannot be read is reported, never silently dropped: a clash against
//! it would otherwise be indistinguishable from no clash.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CandidatePair, CapabilityEvaluation, CompiledRule, GeometryFidelity, NotEvaluatedReason,
    ObjectBounds, ProximityError, ProximityProjection, ProximityServiceHandle, RuleContext,
    projected_candidate_pairs,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Object, ObjectId, Severity};

use crate::selection::select_objects;

/// Not-evaluated outcomes, deduplicated: one object may be selected by both
/// groups and fail the same way twice.
#[derive(Default)]
pub(crate) struct Unevaluated(BTreeSet<(ObjectId, NotEvaluatedReason, String)>);

impl Unevaluated {
    pub(crate) fn push(&mut self, object: ObjectId, reason: NotEvaluatedReason, message: String) {
        self.0.insert((object, reason, message));
    }
    pub(crate) fn drain_into(self, evaluation: &mut CapabilityEvaluation) {
        for (object, reason, message) in self.0 {
            evaluation.push_object_not_evaluated(object, reason, message);
        }
    }
}

/// Pairs ready for narrow-phase measurement.
pub(crate) struct Prepared<'a> {
    pub(crate) service: &'a ProximityServiceHandle,
    pub(crate) subjects: Vec<ObjectId>,
    /// Selected subjects whose extent could not be read.
    pub(crate) unmeasurable_subjects: BTreeSet<ObjectId>,
    /// Counterparts with a readable extent, in identity order.
    pub(crate) counterparts: BTreeSet<ObjectId>,
    /// Selected counterparts whose extent could not be read.
    pub(crate) unmeasurable_counterparts: BTreeSet<ObjectId>,
    pub(crate) pairs: Vec<CandidatePair>,
    pub(crate) unevaluated: Unevaluated,
}

pub(crate) fn severity(rule: &CompiledRule) -> Severity {
    match rule.severity {
        axioval_ir::contract::Severity::Error => Severity::Error,
        axioval_ir::contract::Severity::Warning => Severity::Warning,
        axioval_ir::contract::Severity::Info => Severity::Info,
    }
}

pub(crate) fn counterpart_selector(rule: &CompiledRule) -> Option<&Selector> {
    match rule.parameters.get("counterparts")? {
        ParameterValue::Selector { value } => Some(value),
        _ => None,
    }
}

pub(crate) fn reason(error: ProximityError) -> NotEvaluatedReason {
    match error {
        ProximityError::Unavailable | ProximityError::NoBody => {
            NotEvaluatedReason::IncompleteEvidence
        }
        ProximityError::UnsupportedProjection => NotEvaluatedReason::BackendUnavailable,
        ProximityError::InvalidMeasurement
        | ProximityError::EvidenceFidelityMismatch
        | ProximityError::SameObject => NotEvaluatedReason::InvalidEvidence,
    }
}

/// Refuses every selected subject because the declaration is unusable.
pub(crate) fn refuse_declaration(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    message: &str,
) -> CapabilityEvaluation {
    let (subjects, evaluation) = select_objects(context, &rule.selector);
    refuse_all(
        &subjects,
        evaluation,
        &NotEvaluatedReason::InvalidDeclaration,
        message,
    )
}

/// Marks every subject not evaluated for one rule-wide reason.
pub(crate) fn refuse_all(
    subjects: &[&Object],
    mut evaluation: CapabilityEvaluation,
    reason: &NotEvaluatedReason,
    message: &str,
) -> CapabilityEvaluation {
    for object in subjects {
        evaluation.push_object_not_evaluated(object.id.clone(), reason.clone(), message);
    }
    evaluation
}

/// Selects both groups and runs the broad phase within `margin_metres` in
/// `projection`.
///
/// Returns the evaluation early, with every subject refused, when the
/// declaration or the service is unusable.
#[allow(clippy::too_many_lines)]
pub(crate) fn prepare<'a>(
    context: &RuleContext<'a>,
    rule: &CompiledRule,
    margin_metres: Option<f64>,
    projection: ProximityProjection,
) -> Result<Prepared<'a>, CapabilityEvaluation> {
    let (subjects, evaluation) = select_objects(context, &rule.selector);
    let (Some(selector), Some(margin)) = (counterpart_selector(rule), margin_metres) else {
        return Err(refuse_all(
            &subjects,
            evaluation,
            &NotEvaluatedReason::InvalidDeclaration,
            "pairwise declaration is missing or not physically realisable",
        ));
    };
    let Some(service) = context.services.get::<ProximityServiceHandle>() else {
        return Err(refuse_all(
            &subjects,
            evaluation,
            &NotEvaluatedReason::MissingService,
            "proximity service is not registered",
        ));
    };
    let (counterparts, counterpart_selection) = select_objects(context, selector);

    let mut unevaluated = Unevaluated::default();
    for outcome in evaluation
        .not_evaluated_outcomes()
        .iter()
        .chain(counterpart_selection.not_evaluated_outcomes())
    {
        if let Some(object) = outcome.object_id() {
            unevaluated.push(
                object.clone(),
                outcome.reason().clone(),
                outcome.message().to_owned(),
            );
        }
    }

    let mut bounds: BTreeMap<&ObjectId, ObjectBounds> = BTreeMap::new();
    let mut unmeasurable = BTreeSet::new();
    for object in subjects.iter().chain(&counterparts) {
        if bounds.contains_key(&object.id) || unmeasurable.contains(&object.id) {
            continue;
        }
        match service.bounds(&object.id) {
            Ok(extent) if extent.object() == &object.id => {
                bounds.insert(&object.id, extent);
            }
            outcome => {
                let (reason, message) = match outcome {
                    Err(error) => (reason(error), error.to_string()),
                    // An extent naming another object answers a different question.
                    Ok(_) => (
                        NotEvaluatedReason::InvalidEvidence,
                        "proximity bounds name a different object".to_owned(),
                    ),
                };
                unevaluated.push(
                    object.id.clone(),
                    reason,
                    format!("{message}; pairs involving this object were not checked"),
                );
                unmeasurable.insert(object.id.clone());
            }
        }
    }

    let group = |objects: &[&Object]| -> Vec<ObjectBounds> {
        objects
            .iter()
            .filter_map(|object| bounds.get(&object.id).cloned())
            .collect()
    };
    let pairs = match projected_candidate_pairs(
        &group(&subjects),
        &group(&counterparts),
        projection,
        margin,
    ) {
        Ok(pairs) => pairs,
        Err(error) => {
            return Err(refuse_all(
                &subjects,
                evaluation,
                &NotEvaluatedReason::InvalidEvidence,
                &error.to_string(),
            ));
        }
    };
    Ok(Prepared {
        service,
        subjects: subjects
            .iter()
            .filter(|object| bounds.contains_key(&object.id))
            .map(|object| object.id.clone())
            .collect(),
        unmeasurable_subjects: subjects
            .iter()
            .filter(|object| unmeasurable.contains(&object.id))
            .map(|object| object.id.clone())
            .collect(),
        counterparts: counterparts
            .iter()
            .filter(|object| bounds.contains_key(&object.id))
            .map(|object| object.id.clone())
            .collect(),
        unmeasurable_counterparts: counterparts
            .iter()
            .filter(|object| unmeasurable.contains(&object.id))
            .map(|object| object.id.clone())
            .collect(),
        pairs,
        unevaluated,
    })
}

/// Human-readable suffix for measurements on tessellated geometry.
pub(crate) fn fidelity_note(fidelity: GeometryFidelity) -> String {
    if fidelity.is_exact() {
        String::new()
    } else {
        format!(
            " (approximate: tessellated geometry, true surfaces within {:.4} m)",
            fidelity.deviation_metres()
        )
    }
}
