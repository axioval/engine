//! The building envelope as measured values: one derivation of it, around
//! the bounding objects a rule passes, measured once per run, and what it
//! says about the project, each object and each source.
//!
//! - `envelope_size` (of the project): how many objects the derivation
//!   places on the envelope; its refusal says why the derivation cannot be
//!   made.
//! - `on_envelope`, `declared_external`, `bounds_envelope` (of each
//!   object): whether geometry places it on the envelope, whether the model
//!   declares it external (refused where the model states neither, or its
//!   body was not measured), and whether the derivation is derived around
//!   it.
//! - `external_declarations` (of each source): how many of the source's
//!   objects a selection picks the model declares external in any
//!   derivation, up to those whose declaration is unknown in one.

use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, Citation, EnvelopeDerivation, EnvelopeMembershipError,
    EnvelopeMembershipEvidence, EnvelopeMembershipRequest, EnvelopeMembershipServiceHandle,
    MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Object, ObjectId, SourceId};

use crate::measured_kinds::resolution_error;
use crate::support::{Traversal, Unavailable, invalid};

/// Measures the envelope values.
pub(crate) struct EnvelopeMeasures;

const ENVELOPE_SIZE: &str = "envelope_size";
const ON_ENVELOPE: &str = "on_envelope";
const DECLARED_EXTERNAL: &str = "declared_external";
const BOUNDS_ENVELOPE: &str = "bounds_envelope";
const EXTERNAL_DECLARATIONS: &str = "external_declarations";

/// One derivation measured, or why it cannot be.
type Derived = Result<Arc<EnvelopeMembershipEvidence>, Unavailable>;

/// A derivation's key in the run's memo: the derivation and what bounds
/// it, so every value reading it shares one request.
#[derive(Clone, PartialEq, Eq, Hash)]
struct EnvelopeKey(&'static str, ArgumentsKey);

fn derivation_of(name: &str) -> Result<EnvelopeDerivation, Unavailable> {
    match name {
        "all-spaces" => Ok(EnvelopeDerivation::AllSpaces),
        "gross-area-groups" => Ok(EnvelopeDerivation::GrossAreaGroups),
        other => Err(invalid(format!(
            "envelope derivation `{other}` must be 'all-spaces' or 'gross-area-groups'"
        ))),
    }
}

/// The derivation `derivation` of `call`'s bounding arguments, measured once
/// per run.
fn envelope(
    call: &MeasuredCall,
    derivation: EnvelopeDerivation,
    context: &RuleContext<'_>,
) -> Derived {
    let keys: &[&'static str] = match derivation {
        EnvelopeDerivation::GrossAreaGroups => &["groups", "group_path"],
        _ => &["bounding"],
    };
    let key = EnvelopeKey(derivation.as_str(), ArgumentsKey::of_keys(call, keys));
    MeasuredMemo::of(context.services, key, || derive(call, derivation, context))
}

/// Asks the envelope-membership service for `derivation` around the objects
/// `call` bounds it by.
fn derive(
    call: &MeasuredCall,
    derivation: EnvelopeDerivation,
    context: &RuleContext<'_>,
) -> Derived {
    let Some(service) = context.services.get::<EnvelopeMembershipServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "envelope-membership service is not registered".to_owned(),
        ));
    };
    let request = EnvelopeMembershipRequest::new(derivation, bounding(call, derivation, context)?);
    service
        .measure_envelope_membership(&request)
        .map(Arc::new)
        .map_err(|error| {
            (
                match error {
                    EnvelopeMembershipError::InexactEvidence => NotEvaluatedReason::InvalidEvidence,
                    EnvelopeMembershipError::Unavailable
                    | EnvelopeMembershipError::UnsupportedDerivation => {
                        NotEvaluatedReason::IncompleteEvidence
                    }
                },
                error.to_string(),
            )
        })
}

/// The objects the argument `key` names: a rule's selection bound into
/// the call, or every object of the source kinds it names.
fn selection(
    call: &MeasuredCall,
    key: &str,
    context: &RuleContext<'_>,
) -> Result<MeasuredSelection, Unavailable> {
    crate::measured_kinds::selection(context, call, key, None)
        .map_err(crate::selection::property_error)?
        .ok_or_else(|| invalid(format!("`{key}` names no objects")))
}

/// The objects a selection picks, when it decides every object and picks
/// one: an undecided object might bound the envelope, and nothing picked
/// leaves no region to derive around.
fn decided(selection: &MeasuredSelection) -> Result<Vec<ObjectId>, Unavailable> {
    if !selection.undecided.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "`{}` cannot be decided for {} object(s)",
                selection.parameter,
                selection.undecided.len()
            ),
        ));
    }
    if selection.matched.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("`{}` selects no object", selection.parameter),
        ));
    }
    Ok(selection.matched.iter().cloned().collect())
}

/// The objects `derivation` is derived around.
fn bounding(
    call: &MeasuredCall,
    derivation: EnvelopeDerivation,
    context: &RuleContext<'_>,
) -> Result<Vec<ObjectId>, Unavailable> {
    if derivation != EnvelopeDerivation::GrossAreaGroups {
        return decided(&selection(call, "bounding", context)?);
    }
    let groups = decided(&selection(call, "groups", context)?)?;
    let Some(MeasuredArgument::Path(steps)) = call.argument("group_path") else {
        return Err(invalid("`group_path` states no relationship path"));
    };
    let path = Traversal::path(steps)?;
    // A group's members may be of any kind; the path, not the rule, says
    // what belongs to it.
    let universe: Vec<&Object> = context.project.objects().collect();
    let mut members = BTreeSet::new();
    for group in &groups {
        let (reached, _) = path.related(context, group, &universe)?;
        members.extend(reached);
    }
    if members.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the {} selected gross-area group(s) have no member along `{}`",
                groups.len(),
                call.bound_from("group_path").unwrap_or("group_path")
            ),
        ));
    }
    Ok(members.into_iter().collect())
}

/// `object` among the sorted `objects`.
fn holds(objects: &[ObjectId], object: &ObjectId) -> bool {
    objects.binary_search(object).is_ok()
}

/// A truth as a measured number, cited as exactly as the envelope's
/// evidence.
fn flag(truth: bool, evidence: &EnvelopeMembershipEvidence) -> Measurement {
    let value = if truth { 1.0 } else { 0.0 };
    crate::measured_kinds::interval(
        (value, value),
        None,
        evidence.evidence().exact,
        evidence.evidence().locator.clone(),
    )
}

/// The envelope's evidence, cited beside a value read from it.
fn cited(evidence: &EnvelopeMembershipEvidence) -> Citation {
    Citation {
        evidence: vec![evidence.evidence().clone()],
        ..Citation::default()
    }
}

impl EnvelopeMeasures {
    /// The derivation `call` names, measured.
    fn named(call: &MeasuredCall, context: &RuleContext<'_>) -> Derived {
        let derivation = derivation_of(call.choice("derivation").unwrap_or_default())?;
        envelope(call, derivation, context)
    }

    fn of_object(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let measured = Self::named(call, context)?;
        let truth = match call.name() {
            ON_ENVELOPE => holds(measured.on_envelope(), object),
            BOUNDS_ENVELOPE => measured.request().bounding().contains(object),
            _ => {
                if holds(measured.undeclared(), object) {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "not compared with the {} envelope: the model states neither \
                             external nor internal, or its body could not be measured",
                            measured.request().derivation().as_str()
                        ),
                    ));
                }
                holds(measured.declared(), object)
            }
        };
        Ok((flag(truth, &measured), cited(&measured)))
    }

    fn of_project(
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let measured = Self::named(call, context)?;
        #[allow(clippy::cast_precision_loss)]
        let size = measured.on_envelope().len() as f64;
        Ok((
            Measurement::Cited {
                lower: size,
                upper: size,
                dimension: None,
                locator: measured.evidence().locator.clone(),
                exact: measured.evidence().exact,
            },
            cited(&measured),
        ))
    }

    fn of_source(
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let Some(MeasuredArgument::Choices(derivations)) = call.argument("derivations") else {
            return Err(invalid("`derivations` names no derivation"));
        };
        let mut measured = Vec::new();
        for name in derivations {
            // A derivation that cannot be made leaves the rule open on its
            // own; the declarations are read from those that can.
            if let Ok(evidence) = envelope(call, derivation_of(name)?, context) {
                measured.push(evidence);
            }
        }
        if measured.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "no derivation of the envelope can be made".to_owned(),
            ));
        }
        let objects = selection(call, "objects", context)?;
        let (mut declared, mut unknown) = (0_usize, 0_usize);
        for object in objects
            .matched
            .iter()
            .filter(|object| object.source == *source)
        {
            if measured
                .iter()
                .any(|evidence| holds(evidence.declared(), object))
            {
                declared += 1;
            } else if measured
                .iter()
                .any(|evidence| holds(evidence.undeclared(), object))
            {
                unknown += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let (lower, upper) = (declared as f64, (declared + unknown) as f64);
        Ok((
            Measurement::Cited {
                lower,
                upper,
                dimension: None,
                locator: format!("{EXTERNAL_DECLARATIONS}: {source}"),
                exact: measured.iter().all(|evidence| evidence.evidence().exact),
            },
            Citation {
                evidence: measured
                    .iter()
                    .map(|evidence| evidence.evidence().clone())
                    .collect(),
                ..Citation::default()
            },
        ))
    }
}

/// A refusal of `name` of `subject`, as a property resolution states it.
fn refused(
    name: &str,
    subject: impl std::fmt::Display,
) -> impl Fn(Unavailable) -> PropertyResolutionError {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of {subject}: {why}")))
}

impl MeasuredProvider for EnvelopeMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            BOUNDS_ENVELOPE,
            DECLARED_EXTERNAL,
            ENVELOPE_SIZE,
            EXTERNAL_DECLARATIONS,
            ON_ENVELOPE,
        ]
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
        Self::of_object(call, object, context).map_err(refused(call.name(), object))
    }

    fn measure_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::of_source(call, source, context).map_err(refused(call.name(), source))
    }

    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::of_project(call, context).map_err(refused(call.name(), "the project"))
    }
}
