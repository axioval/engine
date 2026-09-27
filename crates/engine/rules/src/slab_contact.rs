//! Exact source-neutral slab-contact capability.
//!
//! ADR 0004: contact areas are measured by a [`ContactServiceHandle`]; whether
//! enough of the face is in contact -- and how serious a shortfall is -- is
//! decided here, in source-neutral policy.
//!
//! The severity bands are policy and were previously computed in the host rule
//! alongside the measurement it consumed. They are reproduced exactly:
//! a shortfall is graded by how far the measured ratio falls beneath the
//! declared minimum, and a total absence of contact by how far away the
//! nearest candidate is.
//!
//! Scope is policy too. Which objects the face may rest on is the
//! `counterparts` selector, resolved here and carried in the request, so the
//! adapter measures exactly those. Leaving out the top or bottom storey is
//! decided here from each storey's `Elevation` attribute.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ContactError, ContactEvidence, ContactRequest,
    ContactServiceHandle, ContactSide, ContactTolerance, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{
    ATTRIBUTE_SET, Finding, Object, ObjectId, PropertyValue, QuantityDimension, Severity, SourceId,
};

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, invalid, resolve, traversal_parameters,
};

/// Requires a minimum fraction of a face to rest on another element.
///
/// `counterparts` selects what the face may rest on; without it, every other
/// project object is a candidate. `skip_top_storey` and `skip_bottom_storey`
/// leave out subjects on the highest or lowest `storey_selector` object of
/// their source, ordered by the `Elevation` attribute. The subject's storey is
/// the one the declared traversal (`relationship` or `path`) reaches from it.
pub struct SlabContact;

/// The attribute storeys are ordered by.
const ELEVATION: PropertyRef<'static> = PropertyRef {
    set: Some(ATTRIBUTE_SET),
    name: "Elevation",
};

struct Declaration<'a> {
    minimum_ratio: f64,
    side: ContactSide,
    tolerance: ContactTolerance,
    counterparts: Option<&'a Selector>,
    skip: Option<StoreySkip<'a>>,
}

struct StoreySkip<'a> {
    top: bool,
    bottom: bool,
    storeys: &'a Selector,
    traversal: Traversal<'a>,
}

/// Every selected storey's elevation, and the lowest and highest per source.
struct Storeys<'a> {
    universe: Vec<&'a Object>,
    elevations: BTreeMap<ObjectId, f64>,
    extremes: BTreeMap<SourceId, (f64, f64)>,
}

impl RuleCapability for SlabContact {
    fn id(&self) -> &'static str {
        "axioval:capability.slab-contact"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("minimum_contact_ratio", ParameterType::Number),
            ParameterDescriptor::required("contact_side", ParameterType::String),
            ParameterDescriptor::required("maximum_gap_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_intersection_metres", ParameterType::Number),
            ParameterDescriptor::required(
                "minimum_polygon_area_square_metres",
                ParameterType::Number,
            ),
            ParameterDescriptor::optional("counterparts", ParameterType::Selector),
            ParameterDescriptor::optional("skip_top_storey", ParameterType::Boolean),
            ParameterDescriptor::optional("skip_bottom_storey", ParameterType::Boolean),
            ParameterDescriptor::optional("storey_selector", ParameterType::Selector),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, evaluation) = select_objects(context, &rule.selector);

        let declaration = match declaration(&Parameters(rule)) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                return refuse_all(
                    &selected,
                    evaluation,
                    &reason,
                    &format!("slab-contact declaration is invalid: {message}"),
                );
            }
        };

        let Some(service) = context.services.get::<ContactServiceHandle>() else {
            return refuse_all(
                &selected,
                evaluation,
                &NotEvaluatedReason::MissingService,
                "contact service is not registered",
            );
        };

        let (candidates, undecided) = match counterparts(context, declaration.counterparts) {
            Ok(counterparts) => counterparts,
            Err((reason, message)) => {
                return refuse_all(&selected, evaluation, &reason, &message);
            }
        };

        let storeys = match &declaration.skip {
            None => None,
            Some(skip) => match storeys(context, skip) {
                Ok(storeys) => Some((skip, storeys)),
                Err((reason, message)) => {
                    return refuse_all(
                        &selected,
                        evaluation,
                        &reason,
                        &format!("storeys cannot be ordered: {message}"),
                    );
                }
            },
        };

        let mut evaluation = evaluation;
        for object in selected {
            if let Some((skip, storeys)) = &storeys {
                match skipped(context, skip, storeys, object) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                        continue;
                    }
                }
            }
            let request = ContactRequest::new(
                object.id.clone(),
                candidates.clone(),
                declaration.side,
                declaration.tolerance,
            );
            judge(
                rule,
                &declaration,
                &undecided,
                &object.id,
                service.measure_contact(&request),
                &mut evaluation,
            );
        }
        evaluation
    }
}

fn refuse_all(
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

/// The decided candidates, and the objects the selector could not decide.
///
/// Without a selector every project object is a candidate; the request drops
/// the subject itself.
fn counterparts(
    context: &RuleContext<'_>,
    selector: Option<&Selector>,
) -> Result<(Vec<ObjectId>, BTreeSet<ObjectId>), Unavailable> {
    let Some(selector) = selector else {
        return Ok((
            context
                .project
                .objects()
                .map(|object| object.id.clone())
                .collect(),
            BTreeSet::new(),
        ));
    };
    let (objects, outcomes) = select_objects(context, selector);
    let mut undecided = BTreeSet::new();
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => {
                undecided.insert(object.clone());
            }
            // Nothing is known about which objects count.
            None => {
                return Err((
                    outcome.reason().clone(),
                    format!("counterpart selection is undecided: {}", outcome.message()),
                ));
            }
        }
    }
    Ok((
        objects.iter().map(|object| object.id.clone()).collect(),
        undecided,
    ))
}

/// Every storey's elevation. Any undecided storey or unknown elevation fails
/// closed: without it, which storey is highest or lowest is not known.
fn storeys<'a>(
    context: &RuleContext<'a>,
    skip: &StoreySkip<'_>,
) -> Result<Storeys<'a>, Unavailable> {
    let (universe, outcomes) = select_objects(context, skip.storeys);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            outcome.reason().clone(),
            format!("storey selection is undecided: {}", outcome.message()),
        ));
    }
    let mut elevations = BTreeMap::new();
    let mut extremes: BTreeMap<SourceId, (f64, f64)> = BTreeMap::new();
    for storey in &universe {
        let resolved = resolve(context, storey, ELEVATION)?;
        let Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }) = resolved.value()
        else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} has no length Elevation ({})",
                    storey.id,
                    crate::support::display(resolved.value())
                ),
            ));
        };
        if !value.is_finite() {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!("{} has a non-finite Elevation", storey.id),
            ));
        }
        elevations.insert(storey.id.clone(), *value);
        extremes
            .entry(storey.id.source.clone())
            .and_modify(|(low, high)| {
                *low = low.min(*value);
                *high = high.max(*value);
            })
            .or_insert((*value, *value));
    }
    Ok(Storeys {
        universe,
        elevations,
        extremes,
    })
}

/// Whether `subject` lies on a storey the rule leaves out.
///
/// The subject must reach exactly one storey: none or several leave its
/// position in the building unknown, which is not evaluated rather than
/// guessed.
fn skipped(
    context: &RuleContext<'_>,
    skip: &StoreySkip<'_>,
    storeys: &Storeys<'_>,
    subject: &Object,
) -> Result<bool, Unavailable> {
    let (reached, _) = skip
        .traversal
        .related(context, &subject.id, &storeys.universe)?;
    let [storey] = reached.as_slice() else {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} reaches {} storeys through {}, so whether it is on the top or bottom \
                 storey is unknown",
                subject.id,
                reached.len(),
                skip.traversal.relationship
            ),
        ));
    };
    let (Some(elevation), Some((low, high))) = (
        storeys.elevations.get(storey),
        storeys.extremes.get(&storey.source),
    ) else {
        return Err(invalid(format!("storey {storey} was not selected")));
    };
    Ok((skip.top && elevation.total_cmp(high).is_eq())
        || (skip.bottom && elevation.total_cmp(low).is_eq()))
}

/// Grades a total absence of contact by the gap to the nearest candidate.
///
/// An unknown distance is the most serious case: nothing was found to rest on
/// at all.
fn absent_severity(nearest_distance_metres: Option<f64>) -> Severity {
    match nearest_distance_metres {
        None => Severity::Error,
        Some(distance) if distance < 0.1 => Severity::Info,
        Some(distance) if distance > 0.5 => Severity::Error,
        Some(_) => Severity::Warning,
    }
}

/// Grades a partial contact by how far short of the requirement it falls.
fn shortfall_severity(ratio: f64, minimum_ratio: f64) -> Severity {
    // `minimum_ratio` is validated positive in `declaration`.
    let relative = ratio / minimum_ratio;
    if relative > 0.9 {
        Severity::Info
    } else if relative < 0.3 {
        Severity::Error
    } else {
        Severity::Warning
    }
}

fn required_number(parameters: &Parameters<'_>, name: &str) -> Result<f64, Unavailable> {
    parameters
        .number(name)?
        .ok_or_else(|| invalid(format!("parameter `{name}` is required")))
}

fn declaration<'a>(parameters: &Parameters<'a>) -> Result<Declaration<'a>, Unavailable> {
    // A non-positive minimum would make every measurement pass and make the
    // shortfall grading divide by zero.
    let minimum_ratio = required_number(parameters, "minimum_contact_ratio")?;
    if !(minimum_ratio > 0.0 && minimum_ratio <= 1.0) {
        return Err(invalid("minimum_contact_ratio must lie in (0, 1]"));
    }
    let side = match parameters.required_string("contact_side")? {
        "above" => ContactSide::Above,
        "below" => ContactSide::Below,
        other => return Err(invalid(format!("contact_side `{other}` is unsupported"))),
    };
    let tolerance = ContactTolerance::try_new(
        required_number(parameters, "maximum_gap_metres")?,
        required_number(parameters, "maximum_intersection_metres")?,
        required_number(parameters, "minimum_polygon_area_square_metres")?,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let top = parameters.boolean("skip_top_storey")?.unwrap_or(false);
    let bottom = parameters.boolean("skip_bottom_storey")?.unwrap_or(false);
    let skip = if top || bottom {
        let storeys = parameters.selector("storey_selector")?.ok_or_else(|| {
            invalid("skipping a storey needs `storey_selector` to say what a storey is")
        })?;
        let traversal = parameters.traversal()?.ok_or_else(|| {
            invalid("skipping a storey needs a `relationship` or `path` to each subject's storey")
        })?;
        Some(StoreySkip {
            top,
            bottom,
            storeys,
            traversal,
        })
    } else {
        None
    };
    Ok(Declaration {
        minimum_ratio,
        side,
        tolerance,
        counterparts: parameters.selector("counterparts")?,
        skip,
    })
}

/// Judges one measurement: a pass, a graded finding, or not evaluated.
fn judge(
    rule: &CompiledRule,
    declaration: &Declaration<'_>,
    undecided: &BTreeSet<ObjectId>,
    subject: &ObjectId,
    measured: Result<ContactEvidence, ContactError>,
    evaluation: &mut CapabilityEvaluation,
) {
    match measured {
        Ok(measured) => {
            let ratio = measured.contact_ratio();
            // More candidates can only add contact, so a pass over the
            // decided ones holds whatever the undecided ones turn out to be.
            if ratio >= declaration.minimum_ratio {
                return;
            }
            let open = undecided.iter().filter(|id| *id != subject).count();
            if open > 0 {
                evaluation.push_object_not_evaluated(
                    subject.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "contact ratio {ratio:.4} is below required {:.4}, but the \
                         counterpart selection is undecided for {open} object(s) \
                         that could support the face",
                        declaration.minimum_ratio
                    ),
                );
                return;
            }
            let (severity, message) = if measured.contact_area_square_metres() == 0.0 {
                (
                    absent_severity(measured.nearest_distance_metres()),
                    "no contact".to_string(),
                )
            } else {
                (
                    shortfall_severity(ratio, declaration.minimum_ratio),
                    format!(
                        "contact ratio {ratio:.4} below required {:.4}",
                        declaration.minimum_ratio
                    ),
                )
            };
            evaluation.push_finding(
                Finding {
                    rule_id: rule.id.clone(),
                    scope: axioval_ir::Scope::Object(subject.clone()),
                    severity,
                    message,
                    related: Vec::new(),
                    evidence: vec![measured.evidence().clone()],
                    location: None,
                }
                // What the face rests on, so a reviewer can open it.
                .with_related(measured.touching().iter().cloned()),
            );
        }
        Err(error) => evaluation.push_object_not_evaluated(
            subject.clone(),
            match error {
                // The adapter could not orient the body, so it never
                // measured anything; that is missing evidence, not a
                // clean face.
                ContactError::Unavailable | ContactError::UncheckableOrientation => {
                    NotEvaluatedReason::IncompleteEvidence
                }
                ContactError::InexactEvidence
                | ContactError::InvalidAreas
                | ContactError::UnrequestedCandidate => NotEvaluatedReason::InvalidEvidence,
            },
            error.to_string(),
        ),
    }
}
