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
        "space_connections" => Some(crate::space_connection::check_arguments),
        "limited_values" => Some(crate::keyed_limit::check_arguments),
        "zone_checks" => Some(crate::opening_zone::check_arguments),
        "connected_spaces" => Some(crate::opening_spaces::check_arguments),
        "effective_reaching" | "effective_area" | "effective_covered" | "effective_share"
        | "effective_capacity" | "effective_unread" | "effective_missing" => {
            Some(crate::effective_coverage::check_arguments)
        }
        "recesses" => Some(crate::recess_width::check_arguments),
        "exit_separation" => Some(crate::exit_separation::check_arguments),
        "name_sequence" => Some(crate::name_sequence::check_arguments),
        "numbering" => Some(crate::numbering_consistency::check_arguments),
        "wall_spacing" => Some(crate::wall_spacing::check_arguments),
        "parking_bay" => Some(crate::parking_bay::check_arguments),
        "distance_items" => Some(crate::distance::check_arguments),
        "containment_items" => Some(crate::containment::check_arguments),
        "clash_pairs" => Some(crate::clash::check_arguments),
        "clash_matrix_pairs" => Some(crate::clash_matrix::check_arguments),
        "free_floor_fit" => Some(crate::free_floor::check_arguments),
        "well_requirements" => Some(crate::light_well::check_arguments),
        "opening_area" | "opening_count" => Some(crate::measured_openings::check_arguments),
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

/// [`selection`], borrowed from the call where a selection is bound into
/// it: a provider reading it for every object copies nothing.
///
/// # Errors
///
/// As [`selection`].
pub(crate) fn selection_cow<'c>(
    context: &RuleContext<'_>,
    call: &'c MeasuredCall,
    key: &str,
) -> Result<
    Option<std::borrow::Cow<'c, axioval_ir::measured::MeasuredSelection>>,
    PropertyResolutionError,
> {
    if let Some(MeasuredArgument::Objects(selection)) = call.argument(key) {
        return Ok(Some(std::borrow::Cow::Borrowed(selection)));
    }
    selection(context, call, key, None).map(|selection| selection.map(std::borrow::Cow::Owned))
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
        Some(MeasuredArgument::Objects(selection)) => Some(selection.as_ref().clone()),
        Some(MeasuredArgument::SourceKind(_)) => {
            let mut matched = every_object_of_kinds(context, call, key)?;
            if let Some(object) = leave_out {
                matched.remove(object);
            }
            Some(axioval_ir::measured::MeasuredSelection {
                parameter: key.to_owned(),
                matched,
                undecided: BTreeSet::new(),
                first_undecided: None,
                reasons: std::collections::BTreeMap::new(),
            })
        }
        Some(_) => return Err(PropertyResolutionError::InvalidRequest),
    })
}

/// The argument `argument` as the rule parameter it was bound from. A
/// selection stands in as every object: only its presence is read from the
/// rule, its objects from the call ([`population`]).
fn rule_parameter(argument: &MeasuredArgument) -> Option<axioval_ir::contract::ParameterValue> {
    use axioval_ir::contract::{ParameterValue, Selector};
    Some(match argument {
        MeasuredArgument::Table(rows) => ParameterValue::Table {
            value: rows.clone(),
        },
        MeasuredArgument::Property { set, name } => ParameterValue::PropertyReference {
            property_set: set.clone(),
            property: name.clone(),
        },
        MeasuredArgument::Path(steps) => ParameterValue::StringList {
            value: steps.clone(),
        },
        MeasuredArgument::Text(text) => ParameterValue::String {
            value: text.clone(),
        },
        MeasuredArgument::Choice(choice) => ParameterValue::String {
            value: (*choice).to_owned(),
        },
        MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
        MeasuredArgument::Length(value) => ParameterValue::Quantity {
            value: *value,
            unit: "m".into(),
        },
        MeasuredArgument::Number(value) => ParameterValue::Number { value: *value },
        MeasuredArgument::Objects(_) => ParameterValue::Selector {
            value: Box::new(Selector::All),
        },
        _ => return None,
    })
}

/// The rule the arguments of `call` state, each by its key: a capability's
/// own parameters, read back from the measured list a template names them
/// in. `areas` are the keys of areas, bound as plain numbers of square
/// metres.
pub(crate) fn stated_rule(call: &MeasuredCall, areas: &[&str]) -> axioval_engine::CompiledRule {
    use axioval_ir::contract::ParameterValue;
    let stated = call
        .descriptor
        .parameters
        .iter()
        .filter_map(|declared| {
            let value = match (call.argument(declared.key)?, areas.contains(&declared.key)) {
                (MeasuredArgument::Number(value), true) => ParameterValue::Quantity {
                    value: *value,
                    unit: "m2".into(),
                },
                (argument, _) => rule_parameter(argument)?,
            };
            Some((declared.key.to_owned(), value))
        })
        .collect();
    crate::light_area::synthesised(stated)
}

/// The objects the selection argument `key` binds, every object without
/// one.
///
/// # Errors
///
/// As [`selection`], for the reason a capability would give.
pub(crate) fn population(
    call: &MeasuredCall,
    key: &str,
    context: &RuleContext<'_>,
) -> Result<std::sync::Arc<crate::counts::Population>, crate::support::Unavailable> {
    match selection(context, call, key, None).map_err(crate::selection::property_error)? {
        Some(picked) => Ok(std::sync::Arc::new(crate::counts::Population {
            matched: picked.matched,
            undecided: picked.undecided,
            first: None,
        })),
        None => Ok(crate::counts::every_object(context)),
    }
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
    move |(reason, why)| resolution_error((reason, refusal_of(name, object, &why)))
}

/// `` `name` of object: why ``, as `format!` words it, joined without the
/// formatting machinery: a refusal is worded for every object refused.
fn refusal_of(name: &str, object: &ObjectId, why: &str) -> String {
    let source = &object.source;
    let mut message = String::with_capacity(
        name.len()
            + source.system.len()
            + source.document.len()
            + object.local_id.len()
            + why.len()
            + 10,
    );
    for piece in [
        "`",
        name,
        "` of ",
        &source.system,
        ":",
        &source.document,
        "/",
        &object.local_id,
        ": ",
        why,
    ] {
        message.push_str(piece);
    }
    message
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

/// A field left undecided for a reason of its own, and the item's
/// `reason` field stating that reason as a report writes it
/// (`missing_service`), read through [`axioval_engine::template::Items::reason`]:
/// an item a capability left open for a missing service or invalid evidence.
pub(crate) fn refused_field(
    name: &'static str,
    (reason, why): crate::support::Unavailable,
) -> [(&'static str, axioval_engine::MemberValue); 2] {
    [
        (name, axioval_engine::MemberValue::Undecided { why }),
        ("reason", stated_reason(&reason)),
    ]
}

/// `reason` as a report writes it, as an item's text field.
pub(crate) fn stated_reason(
    reason: &axioval_ir::NotEvaluatedReason,
) -> axioval_engine::MemberValue {
    axioval_engine::MemberValue::Text {
        text: match serde_json::to_value(reason) {
            Ok(serde_json::Value::String(reason)) => reason,
            _ => String::new(),
        },
    }
}

/// The traversal the call's arguments state, read as a rule states it:
/// each traversal parameter under its own name.
pub(crate) fn traversal(
    call: &MeasuredCall,
) -> Result<Option<crate::support::Traversal>, crate::support::Unavailable> {
    use axioval_ir::contract::ParameterValue;
    let parameters: std::collections::BTreeMap<String, ParameterValue> = call
        .arguments
        .iter()
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Text(text) => ParameterValue::String {
                    value: text.clone(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect();
    let rule = axioval_engine::CompiledRule {
        id: axioval_ir::RuleId::new("axioval-measured-traversal").expect("a valid rule id"),
        capability: "axioval:measured".into(),
        severity: axioval_ir::contract::Severity::Info,
        selector: axioval_ir::contract::Selector::All,
        parameters,
    };
    crate::support::Parameters(&rule).traversal()
}

/// Measures `undecided_count`.
pub(crate) struct SelectionMeasures;

const UNDECIDED_COUNT: &str = "undecided_count";

impl axioval_engine::MeasuredProvider for SelectionMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[UNDECIDED_COUNT]
    }

    /// How many objects other than the one measured `objects` cannot
    /// decide: counted over the selection bound into the call, exactly.
    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<axioval_engine::Measurement, PropertyResolutionError> {
        let counted = match call.argument("objects") {
            Some(MeasuredArgument::Objects(counted)) => {
                Some(std::borrow::Cow::Borrowed(&**counted))
            }
            _ => selection(context, call, "objects", None)?.map(std::borrow::Cow::Owned),
        };
        let Some(counted) = counted else {
            return Ok(axioval_engine::Measurement::Rounded {
                lower: 0.0,
                upper: 0.0,
                dimension: None,
                locator: format!("{UNDECIDED_COUNT}:{object}: no objects named"),
            });
        };
        #[allow(clippy::cast_precision_loss)]
        let count = counted
            .undecided
            .iter()
            .filter(|candidate| *candidate != object)
            .count() as f64;
        Ok(axioval_engine::Measurement::Rounded {
            lower: count,
            upper: count,
            dimension: None,
            locator: format!("{UNDECIDED_COUNT}:{object}:{}", counted.parameter),
        })
    }
}

/// What one rule reads once for all its objects, kept for that rule alone
/// ([`axioval_engine::MeasuredMemo::of_rule`], dropped once the rule is
/// evaluated), by provider (`marker`) and the call's arguments: a run keeps
/// one rule's reading at a time, as a capability kept its own only while
/// it ran.
pub(crate) fn latest<M, T>(
    context: &axioval_engine::RuleContext<'_>,
    marker: M,
    call: &MeasuredCall,
    read: impl FnOnce() -> T,
) -> std::sync::Arc<T>
where
    M: std::hash::Hash + Eq + Send + 'static,
    T: Send + Sync + 'static,
{
    // One slot per provider for the rule, holding the latest call's
    // arguments: each object's call is compared to them, never copied.
    type Slot<T> = std::sync::Arc<
        std::sync::Mutex<
            Option<(
                std::collections::BTreeMap<&'static str, MeasuredArgument>,
                std::sync::Arc<T>,
            )>,
        >,
    >;
    let slot: Slot<T> = axioval_engine::MeasuredMemo::of_rule(context.services, marker, || {
        std::sync::Arc::new(std::sync::Mutex::new(None))
    });
    let mut held = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((arguments, value)) = held.as_ref()
        && *arguments == call.arguments
    {
        return value.clone();
    }
    *held = None;
    let value = std::sync::Arc::new(read());
    *held = Some((call.arguments.clone(), value.clone()));
    value
}
