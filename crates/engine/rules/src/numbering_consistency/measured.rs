//! The numbers of a rule's selection as a measured member list
//! (`numbering`), read as `numbering-consistency` reads them: each object's
//! number, the scope it lies in, and per object what the template judges
//! of it: why its number cannot be read, its prefix's lead over the other
//! prefixes of its scope, and its number's step from the next lower one.
//!
//! The selection (`@selection`) is read once per rule; each object's items
//! are taken from that one reading.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, SelectionIdentity};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Collected, Config, Member};
use crate::measured_kinds::{interval, refused};
use crate::support::{Parameters, Unavailable, invalid};

/// Measures the numbers of a rule's selection.
pub(crate) struct NumberingMeasures;

const LIST: &str = "numbering";

/// The rule parameters a bound call holds, as a rule states them, keyed by
/// the call's keys (the rule's names): everything but the selection.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Property { set, name } => ParameterValue::PropertyReference {
                    property: name.clone(),
                    property_set: set.clone(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                #[allow(clippy::cast_possible_truncation)]
                MeasuredArgument::Number(value) => ParameterValue::Integer {
                    value: *value as i64,
                },
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
                MeasuredArgument::Text(value) => ParameterValue::String {
                    value: value.clone(),
                },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect()
}

/// Every object's items, keyed by the selection and the declaration.
type Items = Arc<BTreeMap<ObjectId, Vec<MeasuredMember>>>;

#[derive(Clone, Hash, PartialEq, Eq)]
struct Read(SelectionIdentity, String);

fn exact<'e>(evidence: impl IntoIterator<Item = &'e Evidence>) -> bool {
    evidence.into_iter().all(|evidence| evidence.exact)
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

fn number(value: f64, locator: String) -> MemberValue {
    MemberValue::Measured(interval((value, value), None, true, locator))
}

fn objects(members: &[&Member<'_>], own: &ObjectId) -> MemberValue {
    let objects: BTreeSet<ObjectId> = members
        .iter()
        .map(|member| member.object.id.clone())
        .filter(|object| object != own)
        .collect();
    MemberValue::Objects {
        objects: objects.into_iter().collect(),
    }
}

/// Every object's items: why its number cannot be read, its prefix's lead
/// and its number's step, as the capability reads the scopes.
#[allow(clippy::too_many_lines)]
fn measure(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    selected: &[&Object],
) -> BTreeMap<ObjectId, Vec<MeasuredMember>> {
    let collected = Collected::of(context, config, selected);
    let mut items: BTreeMap<ObjectId, Vec<MeasuredMember>> = BTreeMap::new();
    let mut push = |object: &ObjectId, exact: bool, fields: Vec<(&'static str, MemberValue)>| {
        items
            .entry(object.clone())
            .or_default()
            .push(MeasuredMember {
                certain: true,
                exact,
                evidence: Vec::new(),
                fields: fields.into_iter().collect(),
            });
    };
    for (object, reason, why) in &collected.open {
        push(
            object,
            true,
            [("unread", truth(true))]
                .into_iter()
                .chain(crate::measured_kinds::refused_field(
                    "number",
                    (reason.clone(), why.clone()),
                ))
                .collect(),
        );
    }
    let property = config.property;
    for scope in collected.scopes.values() {
        let strays = collected.strays(scope);
        if let Some(length) = config.prefix_length {
            let mut by_prefix: BTreeMap<&str, Vec<&Member<'_>>> = BTreeMap::new();
            for member in &scope.members {
                match member.digits.get(..length) {
                    Some(prefix) if member.digits.len() >= length => {
                        by_prefix.entry(prefix).or_default().push(member);
                    }
                    _ => push(
                        &member.object.id,
                        true,
                        vec![
                            ("prefixed", truth(true)),
                            (
                                "lead",
                                MemberValue::Undecided {
                                    why: format!(
                                        "{property} {} has fewer than {length} digit(s), so it \
                                         has no prefix",
                                        member.shown
                                    ),
                                },
                            ),
                        ],
                    ),
                }
            }
            if by_prefix.len() >= 2 {
                let undecided = scope.undecided + strays.len();
                let mut counts: Vec<usize> = by_prefix.values().map(Vec::len).collect();
                counts.sort_unstable_by(|a, b| b.cmp(a));
                let predominant = (counts[0] - counts[1] > undecided)
                    .then(|| {
                        by_prefix
                            .iter()
                            .find(|(_, members)| members.len() == counts[0])
                    })
                    .flatten();
                let prefixes = by_prefix
                    .iter()
                    .map(|(prefix, members)| format!("{prefix} ({})", members.len()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let everyone: Vec<&Member<'_>> = by_prefix.values().flatten().copied().collect();
                for (prefix, members) in &by_prefix {
                    // How far its prefix leads every other, beyond what the
                    // objects not read could shift: at least one, it
                    // predominates.
                    let others = by_prefix
                        .iter()
                        .filter(|(other, _)| other != &prefix)
                        .map(|(_, members)| members.len())
                        .max()
                        .unwrap_or(0);
                    #[allow(clippy::cast_precision_loss)]
                    let lead = members.len() as f64 - others as f64 - undecided as f64;
                    for member in members {
                        let (departs, related): (String, &[&Member<'_>]) = match predominant {
                            Some((held, holders)) => (
                                format!(
                                    "{property} {} does not start with {held}, the prefix of {} \
                                     other object(s)",
                                    member.shown,
                                    holders.len()
                                ),
                                holders,
                            ),
                            None => (
                                format!(
                                    "{property} {}: its scope mixes the prefixes {prefixes}",
                                    member.shown
                                ),
                                &everyone,
                            ),
                        };
                        let exact = exact(
                            member
                                .evidence
                                .iter()
                                .chain(related.iter().flat_map(|other| other.evidence.iter())),
                        );
                        push(
                            &member.object.id,
                            exact,
                            vec![
                                ("prefixed", truth(true)),
                                (
                                    "lead",
                                    number(lead, format!("{LIST}:{}:lead", member.object.id)),
                                ),
                                ("departs", text(departs)),
                                ("related", objects(related, &member.object.id)),
                            ],
                        );
                    }
                }
            }
        }
        if config.gap_free {
            let mut by_number: BTreeMap<u64, Vec<&Member<'_>>> = BTreeMap::new();
            for member in &scope.members {
                by_number.entry(member.number).or_default().push(member);
            }
            let numbers: Vec<(&u64, &Vec<&Member<'_>>)> = by_number.iter().collect();
            for pair in numbers.windows(2) {
                let [(below, under), (above, over)] = pair else {
                    continue;
                };
                let (below, above) = (**below, **above);
                let missing = if above - below == 2 {
                    format!("{} is missing", below + 1)
                } else {
                    format!("{} to {} are missing", below + 1, above.saturating_sub(1))
                };
                // An unread number might be one of the missing ones.
                let fillable = scope.undecided > 0
                    || strays
                        .iter()
                        .any(|number| number.is_none_or(|number| below < number && number < above));
                for member in *over {
                    let exact = exact(
                        member
                            .evidence
                            .iter()
                            .chain(under.iter().flat_map(|other| other.evidence.iter())),
                    );
                    #[allow(clippy::cast_precision_loss)]
                    let step = (above - below) as f64;
                    push(
                        &member.object.id,
                        exact,
                        vec![
                            ("stepped", truth(true)),
                            (
                                "step",
                                number(step, format!("{LIST}:{}:step", member.object.id)),
                            ),
                            ("fillable", truth(fillable)),
                            ("shown", text(member.shown.clone())),
                            ("below", text(below.to_string())),
                            ("missing", text(missing.clone())),
                            ("related", objects(under, &member.object.id)),
                        ],
                    );
                }
            }
        }
    }
    items
}

/// The items of `object`, from the selection's one reading.
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let Some(MeasuredArgument::Objects(selection)) = call.argument("selection") else {
        return Err(invalid("`selection` is required"));
    };
    let stated = stated(call);
    let key = Read(SelectionIdentity(selection.clone()), format!("{stated:?}"));
    let read: Result<Items, Unavailable> = MeasuredMemo::of(context.services, key, || {
        let rule = crate::light_area::synthesised(stated.clone());
        let config = Config::read(&Parameters(&rule))?;
        let selected: Vec<&Object> = selection
            .matched
            .iter()
            .filter_map(|id| crate::selection::object_by_id(context, id))
            .collect();
        Ok(Arc::new(measure(context, &config, &selected)))
    });
    Ok(read?.get(object).cloned().unwrap_or_default())
}

impl MeasuredProvider for NumberingMeasures {
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
