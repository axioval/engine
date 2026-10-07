//! What `parking-bay` judges of a bay, as a measured member list
//! (`parking_bay`), measured as the capability measures it: the aisles,
//! neighbours and obstacles near each bay from one plan broad phase over
//! the rule's bays, and each bay's steps (`Bay::steps`): its sizes, the
//! obstacles counted within it and at its ends and sides, its orientation
//! to an aisle, and in filter mode the states it may be in.
//!
//! Its items: a size against its bounds (worded as a filter leaves it), a
//! count against what is allowed, and a search's own answer; the template
//! judges each.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Bay, Filtering, Matter, Nearby, Reference, Services, parse};
use crate::measured_kinds::{interval, refused};
use crate::orientation::Tri;
use crate::plan_area::shown;
use crate::support::{Parameters, Unavailable, invalid};

/// Measures what `parking-bay` judges of a bay.
pub(crate) struct BayItems;

const LIST: &str = "parking_bay";

/// The rule parameters a bound call holds, as a rule states them, keyed by
/// the call's keys (the rule's names); a bound selection as any selector,
/// since the bay's neighbours are read from the bound objects.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter(|(key, _)| **key != "selection")
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Length(value) => ParameterValue::Quantity {
                    value: *value,
                    unit: "m".into(),
                },
                // The tolerance is read in degrees, as bound.
                MeasuredArgument::Number(value) => ParameterValue::Quantity {
                    value: *value,
                    unit: "deg".into(),
                },
                MeasuredArgument::Text(value) => ParameterValue::String {
                    value: value.clone(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                MeasuredArgument::Objects(_) => ParameterValue::Selector {
                    value: Box::new(Selector::All),
                },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect()
}

fn selection<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c Arc<MeasuredSelection>> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection),
        _ => None,
    }
}

/// The memo key of the objects near the rule's bays: the bays, the
/// objects looked for and the reach.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Near(SelectionIdentity, SelectionIdentity, u64);

/// The objects of `objects` near each of the rule's bays (`bays`), from one
/// broad phase per rule.
fn near(
    context: &RuleContext<'_>,
    services: &Services<'_>,
    (bays, objects): (&Arc<MeasuredSelection>, &Arc<MeasuredSelection>),
    reach: f64,
) -> Arc<Result<Nearby, Unavailable>> {
    MeasuredMemo::of(
        context.services,
        Near(
            SelectionIdentity(bays.clone()),
            SelectionIdentity(objects.clone()),
            reach.to_bits(),
        ),
        || {
            let Some(proximity) = services.proximity else {
                return Arc::new(Err((
                    NotEvaluatedReason::MissingService,
                    "proximity service is not registered".to_owned(),
                )));
            };
            let selected: Vec<&Object> = context
                .project
                .objects()
                .filter(|object| bays.matched.contains(&object.id))
                .collect();
            Arc::new(
                Nearby::of(
                    proximity,
                    objects.matched.clone(),
                    &objects.undecided,
                    &selected,
                    reach,
                )
                .map(Nearby::kept),
            )
        },
    )
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: LIST.to_owned(),
    }
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn exact<'e>(evidence: impl IntoIterator<Item = &'e Evidence>) -> bool {
    evidence.into_iter().all(|evidence| evidence.exact)
}

fn member(exact: bool, fields: Vec<(&'static str, MemberValue)>) -> MeasuredMember {
    MeasuredMember {
        certain: true,
        exact,
        evidence: Vec::new(),
        fields: fields.into_iter().collect(),
    }
}

fn number(value: (f64, f64), exact: bool, locator: String) -> MemberValue {
    MemberValue::Measured(interval(value, None, exact, locator))
}

/// The item of one step, if any.
#[allow(clippy::too_many_lines)]
fn item(step: Matter, filtering: Option<&Filtering>, at: &str) -> Option<MeasuredMember> {
    Some(match step {
        Matter::Judged(Ok(None)) => return None,
        Matter::Judged(Ok(Some((message, evidence, related)))) => member(
            exact(&evidence),
            vec![
                ("judged", truth(true)),
                ("found", truth(true)),
                ("message", text(message)),
                ("related", MemberValue::Objects { objects: related }),
            ],
        ),
        Matter::Judged(Err((reason, why))) => member(
            true,
            [("judged", truth(true))]
                .into_iter()
                .chain(crate::measured_kinds::refused_field("found", (reason, why)))
                .collect(),
        ),
        Matter::Count(counting) => {
            #[allow(clippy::cast_precision_loss)]
            let (surely, most, allowed) = (
                counting.counted.0 as f64,
                counting.counted.1 as f64,
                counting.allowed as f64,
            );
            member(
                exact(&counting.evidence),
                vec![
                    ("counting", truth(true)),
                    (
                        "count",
                        number((surely, most), true, format!("{LIST}:{at}:count")),
                    ),
                    (
                        "allowed",
                        number((allowed, allowed), true, format!("{LIST}:{at}:allowed")),
                    ),
                    ("found_words", text(counting.found)),
                    ("open_words", text(counting.open)),
                    (
                        "related",
                        MemberValue::Objects {
                            objects: counting.related,
                        },
                    ),
                ],
            )
        }
        Matter::Size(sized) => {
            let applies = filtering.map_or(Tri::Yes, |filtering| filtering.applies);
            if applies == Tri::No {
                return None;
            }
            let why = filtering.map_or_else(String::new, |filtering| filtering.why.clone());
            let doubtful = applies == Tri::Maybe;
            let (what, measured, mut evidence) = match sized.measured {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    let why = if doubtful {
                        format!("{message}; {why}")
                    } else {
                        message
                    };
                    return Some(member(
                        true,
                        [("sized", truth(true))]
                            .into_iter()
                            .chain(crate::measured_kinds::refused_field("size", (reason, why)))
                            .collect(),
                    ));
                }
            };
            let suffix = match filtering {
                Some(filtering) if applies == Tri::Yes => {
                    evidence.extend(filtering.evidence.iter().cloned());
                    format!(" (a bay with {})", filtering.states)
                }
                _ => String::new(),
            };
            let exact = exact(&evidence);
            let mut fields = vec![
                ("sized", truth(true)),
                (
                    "size",
                    number(measured, exact, format!("{LIST}:{at}:{what}")),
                ),
                (
                    "measured",
                    text(format!("{what} is {} m", shown(measured.0, measured.1))),
                ),
                ("suffix", text(suffix)),
                ("doubtful", truth(doubtful)),
                ("applies_why", text(why.clone())),
                (
                    "later",
                    text(if doubtful {
                        format!("; {why}")
                    } else {
                        String::new()
                    }),
                ),
            ];
            if let Some(low) = sized.bounds.0 {
                fields.push(("low", number((low, low), true, format!("{LIST}:low"))));
            }
            if let Some(high) = sized.bounds.1 {
                fields.push(("high", number((high, high), true, format!("{LIST}:high"))));
            }
            member(exact, fields)
        }
    })
}

/// The items of one bay.
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let rule = crate::light_area::synthesised(stated(call));
    let mut config = parse(&Parameters(&rule))?;
    // The tolerance in degrees exactly as bound.
    if let (Some(orientation), Some(MeasuredArgument::Number(degrees))) = (
        config.orientation.as_mut(),
        call.argument("angle_tolerance"),
    ) {
        orientation.tolerance = *degrees;
    }
    let services = Services::of(context, &config)?;
    let bays = selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
    let references =
        match config
            .orientation
            .as_ref()
            .map(|orientation| match &orientation.reference {
                Reference::Aisles { reach, .. } => selection(call, "aisles")
                    .map(|aisles| near(context, &services, (bays, aisles), *reach)),
                Reference::Neighbours { reach } => {
                    Some(near(context, &services, (bays, bays), *reach))
                }
            }) {
            Some(Some(found)) => Some(found),
            Some(None) => return Err(invalid("`aisles` is required")),
            None => None,
        };
    if let Some(Err(unavailable)) = references.as_deref() {
        return Err(unavailable.clone());
    }
    let obstacles = match config.obstructions.as_ref() {
        Some(obstructions) => Some(
            selection(call, "obstacles")
                .map(|obstacles| near(context, &services, (bays, obstacles), obstructions.reach))
                .ok_or_else(|| invalid("`obstacles` is required"))?,
        ),
        None => None,
    };
    if let Some(Err(unavailable)) = obstacles.as_deref() {
        return Err(unavailable.clone());
    }
    let bay_object = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    let bay = Bay {
        config: &config,
        services: &services,
        object: bay_object,
    };
    let (steps, filtering) = bay.steps(
        references.as_deref().and_then(|found| found.as_ref().ok()),
        obstacles.as_deref().and_then(|found| found.as_ref().ok()),
    );
    let at = object.to_string();
    Ok(steps
        .into_iter()
        .filter_map(|step| item(step, filtering.as_ref(), &at))
        .collect())
}

impl MeasuredProvider for BayItems {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LIST]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        items(call, object, context).map_err(refused(call.name(), object))
    }
}
