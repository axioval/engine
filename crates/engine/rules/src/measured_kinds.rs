//! What the built-in measured-value providers share: reading the source
//! kinds a measured name selects objects by.

use std::collections::BTreeSet;

use axioval_engine::{PropertyResolutionError, RuleContext, TypeHierarchyServiceHandle};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

/// A provider's check of the rule parameters a measured value names, as
/// the rule states them and keyed by the value's keys: what a template's
/// `Check::Arguments` refuses once per rule.
pub(crate) type ArgumentCheck = fn(
    &std::collections::BTreeMap<String, axioval_ir::contract::ParameterValue>,
) -> Result<(), crate::support::Unavailable>;

/// The argument check the provider of the measured value `name` declares,
/// if any: a declaration only the measurement knows how to read.
pub(crate) fn argument_check(name: &str) -> Option<ArgumentCheck> {
    match name {
        "light_area" | "light_size" | "light_step" => Some(crate::light_area::check_arguments),
        _ => None,
    }
}

/// The project's objects of the `,`-separated source kinds `key` names,
/// `object` left out: subtypes included where the source declares a type
/// hierarchy, kinds matched exactly where it declares none.
pub(crate) fn objects_of_kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<BTreeSet<ObjectId>, PropertyResolutionError> {
    let mut found = every_object_of_kinds(context, call, key)?;
    found.remove(object);
    Ok(found)
}

/// The project's objects of the `,`-separated source kinds `key` names,
/// matched as [`objects_of_kinds`] matches them, the object measured
/// included: the population a capability selects by kind.
pub(crate) fn every_object_of_kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<BTreeSet<ObjectId>, PropertyResolutionError> {
    let Some(MeasuredArgument::SourceKind(kinds)) = call.argument(key) else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let kinds: Vec<&str> = kinds.split(',').map(str::trim).collect();
    let hierarchy = context.services.get::<TypeHierarchyServiceHandle>();
    let mut found = BTreeSet::new();
    for candidate in context.project.objects() {
        let held = candidate.kind();
        for kind in &kinds {
            let matched =
                held.eq_ignore_ascii_case(kind)
                    || match hierarchy {
                        Some(hierarchy) => hierarchy
                            .is_a(&candidate.id.source, held, kind)
                            .map_err(|error| {
                                PropertyResolutionError::Unavailable(format!(
                                    "whether {} is a {kind} is unknown: {error}",
                                    candidate.id
                                ))
                            })?,
                        None => false,
                    };
            if matched {
                found.insert(candidate.id.clone());
                break;
            }
        }
    }
    Ok(found)
}

/// The objects the [`Objects`](axioval_ir::measured::MeasuredParameterKind::Objects)
/// argument `key` names, `None` where the call states none: the objects
/// a rule's selector picked (or the anchor), as bound into the call, or
/// those of the source kinds it names, all surely picked and `leave_out`
/// left out.
///
/// # Errors
///
/// A kind whose objects cannot be told, or an argument of another kind.
pub(crate) fn selection(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    leave_out: Option<&ObjectId>,
) -> Result<Option<axioval_ir::measured::MeasuredSelection>, PropertyResolutionError> {
    Ok(match call.argument(key) {
        None => None,
        Some(MeasuredArgument::Objects(selection)) => Some(selection.clone()),
        Some(MeasuredArgument::SourceKind(_)) => {
            let mut matched = every_object_of_kinds(context, call, key)?;
            if let Some(object) = leave_out {
                matched.remove(object);
            }
            Some(axioval_ir::measured::MeasuredSelection {
                parameter: key.to_owned(),
                matched,
                undecided: BTreeSet::new(),
            })
        }
        Some(_) => return Err(PropertyResolutionError::InvalidRequest),
    })
}

/// [`objects_of_kinds`], `object` included when it is of the kinds too.
pub(crate) fn objects_of_kinds_including(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<BTreeSet<ObjectId>, PropertyResolutionError> {
    let mut found = objects_of_kinds(context, call, key, object)?;
    let Some(MeasuredArgument::SourceKind(kinds)) = call.argument(key) else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let Some(own) = context.project.object(object) else {
        return Ok(found);
    };
    let held = own.kind();
    let hierarchy = context.services.get::<TypeHierarchyServiceHandle>();
    for kind in kinds.split(',').map(str::trim) {
        let matched = held.eq_ignore_ascii_case(kind)
            || match hierarchy {
                Some(hierarchy) => hierarchy
                    .is_a(&object.source, held, kind)
                    .map_err(|error| {
                        PropertyResolutionError::Unavailable(format!(
                            "whether {object} is a {kind} is unknown: {error}"
                        ))
                    })?,
                None => false,
            };
        if matched {
            found.insert(object.clone());
            break;
        }
    }
    Ok(found)
}

/// A capability's refusal as a property-resolution error that maps back to
/// the same reason, so a measured value is left open for the reason the
/// capability would give.
pub(crate) fn resolution_error(
    (reason, message): crate::support::Unavailable,
) -> PropertyResolutionError {
    use axioval_engine::NotEvaluatedReason;
    match reason {
        NotEvaluatedReason::IncompleteEvidence => PropertyResolutionError::Incomplete(message),
        NotEvaluatedReason::MissingService => PropertyResolutionError::MissingService(message),
        NotEvaluatedReason::NotRecorded => PropertyResolutionError::NotRecorded(message),
        NotEvaluatedReason::InvalidEvidence => PropertyResolutionError::Conflicting(message),
        NotEvaluatedReason::InvalidDeclaration => PropertyResolutionError::InvalidArgument(message),
        _ => PropertyResolutionError::Unavailable(message),
    }
}

/// A refusal of `name` of `object` as a property-resolution error that maps
/// back to the capability's reason.
pub(crate) fn refused(
    name: &str,
    object: &ObjectId,
) -> impl Fn(crate::support::Unavailable) -> PropertyResolutionError {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of {object}: {why}")))
}

/// A value sure to lie in `[lower, upper]`, cited as exactly as the
/// measurement it comes from: an interval of an exact measurement holds
/// only rounding, so its evidence stays exact, and an approximate one's is
/// never exact, a point included.
pub(crate) fn interval(
    (lower, upper): (f64, f64),
    dimension: Option<axioval_ir::QuantityDimension>,
    exact: bool,
    locator: String,
) -> axioval_engine::Measurement {
    if exact {
        axioval_engine::Measurement::Rounded {
            lower,
            upper,
            dimension,
            locator,
        }
    } else {
        axioval_engine::Measurement::Cited {
            lower,
            upper,
            dimension,
            locator,
            exact: false,
        }
    }
}
