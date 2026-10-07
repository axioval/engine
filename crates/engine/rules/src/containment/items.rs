//! What `containment` reads of an inner element, as a measured member list
//! (`containment_items`), and of each outer element and the objects its
//! selections and the broad phase could not decide, as one of the project
//! (`containment_counts`), read as the capability reads them: the broad
//! phase once per rule over the bound inner and outer elements, each inner
//! element placed and its cover measured (`assess`), and the inner
//! elements each outer element surely and possibly holds.
//!
//! An inner element's items: whether it lies in none (a finding with the
//! capability's words, or why it is undecided), each cover band's distance
//! to an outer element it lies in with the band's bounds, and why
//! something of it is not checked. An outer element's item: the inner
//! elements it holds, from sure to possible. The template judges the
//! distances against the bands and the counts against the bounds.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PropertyResolutionError, ProximityProjection, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Assessed, Declaration, Item, assess, declaration};
use crate::measured_kinds::{interval, refused};
use crate::pairs::{Unevaluated, prepare_among};
use crate::support::{Unavailable, invalid};

/// Measures what `containment` reads of an inner element, and of the outer
/// elements and the objects it leaves open.
pub(crate) struct ContainmentItems;

const ITEMS: &str = "containment_items";
const COUNTS: &str = "containment_counts";

/// The rule parameters a bound call holds, as a rule states them, keyed by
/// the call's keys (the rule's names); a bound selection as any selector,
/// since the objects are read from the bound selections.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter(|(key, _)| **key != "selection")
        .filter_map(|(key, argument)| {
            let value = match argument {
                #[allow(clippy::cast_possible_truncation)]
                MeasuredArgument::Number(value) if key.ends_with("_count") => {
                    ParameterValue::Integer {
                        value: *value as i64,
                    }
                }
                MeasuredArgument::Number(value) => ParameterValue::Number { value: *value },
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
                MeasuredArgument::Table(rows) => ParameterValue::Table {
                    value: rows.clone(),
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

/// What a rule reads once for all its inner elements: the declaration
/// and the elements placed and read.
struct Reading {
    declared: Declaration,
    assessed: Assessed,
}

/// The memo of the latest rule's reading.
#[derive(Hash, PartialEq, Eq)]
struct Latest;

/// What the rule `call` stands for reads, once per rule; why every inner
/// element is refused, where the declaration or the service is unusable or
/// the broad phase cannot run.
fn reading(call: &MeasuredCall, context: &RuleContext<'_>) -> Arc<Result<Reading, Unavailable>> {
    crate::measured_kinds::latest(context, Latest, call, || {
        let rule = crate::light_area::synthesised(stated(call));
        let declared = declaration(&rule)?;
        let inner =
            selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
        let outer =
            selection(call, "counterparts").ok_or_else(|| invalid("`counterparts` is required"))?;
        let objects = |selection: &MeasuredSelection| -> Vec<&Object> {
            context
                .project
                .objects()
                .filter(|object| selection.matched.contains(&object.id))
                .collect()
        };
        let mut unevaluated = Unevaluated::default();
        for chosen in [inner, outer] {
            for (object, (reason, message)) in &chosen.reasons {
                unevaluated.push(object.clone(), reason.clone(), message.clone());
            }
        }
        let prepared = prepare_among(
            context,
            (&objects(inner), &objects(outer)),
            unevaluated,
            (0.0, ProximityProjection::Minimum3d),
        )?;
        let assessed = assess(&declared, prepared);
        Ok(Reading { declared, assessed })
    })
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: ITEMS.to_owned(),
    }
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn exact<'e>(evidence: impl IntoIterator<Item = &'e Evidence>) -> bool {
    evidence.into_iter().all(|evidence| evidence.exact)
}

fn length(value: (f64, f64), exact: bool, locator: String) -> MemberValue {
    MemberValue::Measured(interval(
        value,
        Some(axioval_ir::QuantityDimension::Length),
        exact,
        locator,
    ))
}

fn member(exact: bool, fields: Vec<(&'static str, MemberValue)>) -> MeasuredMember {
    MeasuredMember {
        certain: true,
        exact,
        evidence: Vec::new(),
        fields: fields.into_iter().collect(),
    }
}

/// Whether `message` is the broad phase's word that an object's extent
/// could not be read.
fn unmeasured(message: &str) -> bool {
    message.ends_with("; pairs involving this object were not checked")
}

/// The open outcome an item may come to, by which an inner element's items
/// are ordered as the capability ordered what it left open.
fn open_key(item: &Item, declared: &super::Declaration) -> Option<Unavailable> {
    match item {
        Item::Orphan(Err(open))
        | Item::Open(open)
        | Item::Band {
            measured: Err(open),
            ..
        } => Some(open.clone()),
        Item::Orphan(Ok(_)) => None,
        Item::Band {
            band,
            outer,
            measured: Ok(measured),
            ..
        } => Some((
            NotEvaluatedReason::IncompleteEvidence,
            declared.bands[*band].read(measured, outer).straddles,
        )),
    }
}

/// The items of one inner element.
#[allow(clippy::too_many_lines)]
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let reading = reading(call, context);
    let Reading { declared, assessed } = reading.as_ref().as_ref().map_err(Clone::clone)?;
    if assessed.unmeasurable_subjects.contains(object) {
        // Its extent could not be read: open as the broad phase left it.
        let (_, reason, message) = assessed
            .unevaluated
            .iter()
            .find(|(open, _, message)| open == object && unmeasured(message))
            .unwrap_or_else(|| unreachable!("an unmeasurable inner element is left open"));
        return Err((reason.clone(), message.clone()));
    }
    let Some((_, found)) = assessed.inner.iter().find(|(inner, _)| inner == object) else {
        return Ok(Vec::new());
    };
    // What the capability left open it reported once each, in order.
    let mut ordered: Vec<(Option<Unavailable>, &Item)> = found
        .iter()
        .map(|item| (open_key(item, declared), item))
        .collect();
    ordered.sort_by(|(a, _), (b, _)| a.cmp(b));
    ordered.dedup_by(|(a, later), (b, _)| {
        a == b
            && matches!(
                later,
                Item::Open(_)
                    | Item::Orphan(Err(_))
                    | Item::Band {
                        measured: Err(_),
                        ..
                    }
            )
    });
    let at = object.to_string();
    Ok(ordered
        .into_iter()
        .enumerate()
        .map(|(index, (_, item))| match item {
            Item::Orphan(Ok((message, evidence))) => member(
                exact(evidence),
                vec![
                    ("orphan_checked", truth(true)),
                    ("orphan", truth(true)),
                    ("orphan_words", text(message.clone())),
                ],
            ),
            Item::Orphan(Err((reason, why))) => member(
                true,
                vec![
                    ("orphan_checked", truth(true)),
                    ("orphan", MemberValue::Undecided { why: why.clone() }),
                    ("reason", crate::measured_kinds::stated_reason(reason)),
                ],
            ),
            Item::Open((reason, why)) => member(
                true,
                vec![
                    ("open_checked", truth(true)),
                    ("open", MemberValue::Undecided { why: why.clone() }),
                    ("reason", crate::measured_kinds::stated_reason(reason)),
                ],
            ),
            Item::Band {
                band,
                outer,
                measured,
                link,
            } => {
                let bounds = &declared.bands[*band];
                let mut fields = vec![
                    ("band_checked", truth(true)),
                    (
                        "related",
                        MemberValue::Objects {
                            objects: vec![outer.clone()],
                        },
                    ),
                ];
                let sure = match measured {
                    Err((reason, why)) => {
                        fields.extend(crate::measured_kinds::refused_field(
                            "distance",
                            (reason.clone(), why.clone()),
                        ));
                        true
                    }
                    Ok(measured) => {
                        let banded = bounds.read(measured, outer);
                        let sure = measured.evidence().exact && exact(link);
                        fields.extend([
                            (
                                "distance",
                                length(
                                    (banded.lower, banded.upper),
                                    sure,
                                    format!("{ITEMS}:{at}:{index}"),
                                ),
                            ),
                            ("below_words", text(banded.below)),
                            ("above_words", text(banded.above)),
                            ("straddle_words", text(banded.straddles)),
                        ]);
                        sure
                    }
                };
                if let Some(minimum) = bounds.minimum {
                    fields.push((
                        "low",
                        length((minimum, minimum), true, format!("{ITEMS}:low")),
                    ));
                }
                if let Some(maximum) = bounds.maximum {
                    fields.push((
                        "high",
                        length((maximum, maximum), true, format!("{ITEMS}:high")),
                    ));
                }
                member(sure, fields)
            }
        })
        .collect())
}

/// Each outer element's count, where the rule bounds counts, and the
/// objects the selections and the broad phase left open beyond what the
/// rule's own selection and each inner element report.
fn counts(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let reading = reading(call, context);
    // A rule the capability refused whole leaves nothing else open.
    let Ok(Reading { declared, assessed }) = reading.as_ref() else {
        return Ok(Vec::new());
    };
    let inner = selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
    let objects = |object: &ObjectId| MemberValue::Objects {
        objects: vec![object.clone()],
    };
    let mut members = Vec::new();
    if declared.minimum_count.is_some() || declared.maximum_count.is_some() {
        for (outer, held) in &assessed.held {
            let (sure, possible) = held.counts();
            // Holding nothing, surely, it meets any maximum: only a minimum
            // judges it.
            if possible == 0 && declared.minimum_count.is_none() {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let number = |(low, high): (usize, usize)| {
                MemberValue::Measured(interval(
                    (low as f64, high as f64),
                    None,
                    true,
                    format!("{COUNTS}:{outer}"),
                ))
            };
            members.push(member(
                true,
                vec![
                    ("count_checked", truth(true)),
                    ("object", objects(outer)),
                    ("count", number((sure, possible))),
                    ("held", number((sure, sure))),
                    ("may", number((possible, possible))),
                    (
                        "related",
                        MemberValue::Objects {
                            objects: held.sure.iter().cloned().collect(),
                        },
                    ),
                ],
            ));
        }
    }
    members.extend(
        assessed
            .unevaluated
            .iter()
            .filter(|(object, reason, message)| {
                // The rule's selection reports its own undecided objects.
                let selected = inner
                    .reasons
                    .get(object)
                    .is_some_and(|(own, words)| own == reason && words == message);
                // An inner element whose extent could not be read reports
                // it itself.
                let subject =
                    assessed.unmeasurable_subjects.contains(object) && unmeasured(message);
                !(selected || subject)
            })
            .map(|(object, reason, message)| {
                member(
                    true,
                    vec![
                        ("open_checked", truth(true)),
                        ("object", objects(object)),
                        (
                            "open",
                            MemberValue::Undecided {
                                why: message.clone(),
                            },
                        ),
                        ("reason", crate::measured_kinds::stated_reason(reason)),
                    ],
                )
            }),
    );
    Ok(members)
}

impl MeasuredProvider for ContainmentItems {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ITEMS, COUNTS]
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

    fn members_of_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        counts(call, context)
            .map(|members| (members, Vec::new()))
            .map_err(crate::measured_kinds::resolution_error)
    }
}
