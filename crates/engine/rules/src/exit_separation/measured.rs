//! A space's exits and their separation as a measured member list
//! (`exit_separation`), measured as `exit-separation` measures them: the
//! exits the path reaches among those the exit selection picks (its
//! undecided objects possible exits), the longest plan diagonal, the share
//! of it the flag selects (an interval over both shares where the flag is
//! unknown, never a default) and every pair of the sure exits.
//!
//! The list holds up to two items: one counting the exits, where the rule
//! declares `minimum_exits`, and one with the separation of the pairs
//! against the separation required, where there are pairs to judge. The
//! template judges both; the words naming the pairs, the requirement and
//! why pairs are open are the capability's.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, SelectionIdentity};
use axioval_ir::{Evidence, ObjectId};

use super::{Measured, Pair, Pairs, Reached, Standing, declaration, upper};
use crate::measured_kinds::{interval, refused};
use crate::support::{Parameters, Unavailable, invalid};

/// Measures a space's exits and their separation.
pub(crate) struct ExitMeasures;

const LIST: &str = "exit_separation";

/// The rule parameters a bound call holds, as a rule states them, keyed by
/// the call's keys (which are the rule's names): everything but the exit
/// selection, which is bound.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Property { set, name } => ParameterValue::PropertyReference {
                    property: name.clone(),
                    property_set: set.clone(),
                },
                MeasuredArgument::Table(rows) => ParameterValue::Table {
                    value: rows.clone(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                #[allow(clippy::cast_possible_truncation)]
                MeasuredArgument::Number(value) if *key == "minimum_exits" => {
                    ParameterValue::Integer {
                        value: *value as i64,
                    }
                }
                MeasuredArgument::Number(value) => ParameterValue::Number { value: *value },
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
                MeasuredArgument::Choice(value) => ParameterValue::String {
                    value: (*value).to_owned(),
                },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect()
}

/// The objects the exit selection picks or leaves undecided, in the
/// project's order, and why each undecided one is: read once per
/// selection.
struct Picked {
    universe: Vec<ObjectId>,
    undecided: BTreeMap<ObjectId, String>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct PickedKey(SelectionIdentity);

fn picked(context: &RuleContext<'_>, call: &MeasuredCall) -> Result<Arc<Picked>, Unavailable> {
    let Some(MeasuredArgument::Objects(selection)) = call.argument("exit_selector") else {
        return Err(invalid("`exit_selector` is required"));
    };
    Ok(MeasuredMemo::of(
        context.services,
        PickedKey(SelectionIdentity(selection.clone())),
        || {
            let universe = context
                .project
                .objects()
                .filter(|object| {
                    selection.matched.contains(&object.id)
                        || selection.undecided.contains(&object.id)
                })
                .map(|object| object.id.clone())
                .collect();
            let undecided = selection
                .undecided
                .iter()
                .map(|object| {
                    let why = selection.reasons.get(object).cloned().unwrap_or_default();
                    (object.clone(), why)
                })
                .collect();
            Arc::new(Picked {
                universe,
                undecided,
            })
        },
    ))
}

fn count(value: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let value = value as f64;
    value
}

fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
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

/// The items of one space.
#[allow(clippy::too_many_lines)]
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let rule = crate::light_area::synthesised(stated(call));
    let declared = declaration(&Parameters(&rule), false)?;
    let picked = picked(context, call)?;
    let space = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    let reached = Reached::of(
        context,
        &declared.exits,
        (&picked.universe, &picked.undecided),
        space,
    )?;
    let (exits, maybe) = (&reached.exits, &reached.maybe);
    let total = exits.len() + maybe.len();
    let at = |field: &str| format!("{LIST}:{object}:{field}");
    let number = |(low, high): (f64, f64), field: &str, exact: bool| {
        MemberValue::Measured(interval((low, high), None, exact, at(field)))
    };
    let mut items = Vec::new();
    let mut short = false;
    if let Some(minimum) = declared.minimum_exits {
        let exact = exact(&reached.evidence);
        items.push(MeasuredMember {
            certain: true,
            exact,
            evidence: Vec::new(),
            fields: BTreeMap::from([
                ("counted", truth(true)),
                (
                    "exits",
                    number((count(exits.len()), count(total)), "exits", true),
                ),
                (
                    "sure",
                    number((count(exits.len()), count(exits.len())), "sure", true),
                ),
                (
                    "possible",
                    number((count(total), count(total)), "possible", true),
                ),
                ("relation", text(declared.exits.relationship.clone())),
                ("undecided", text(reached.undecided(&picked.undecided))),
                (
                    "named",
                    MemberValue::Objects {
                        objects: exits.iter().chain(maybe).cloned().collect(),
                    },
                ),
            ]),
        });
        if total < minimum {
            short = true;
        } else if exits.len() < minimum {
            // The count is open, and nothing else is judged.
            return Ok(items);
        }
    }
    if total < 2 {
        return Ok(items);
    }
    let separated = |fields: Vec<(&'static str, MemberValue)>, exact: bool| {
        let mut all = BTreeMap::from([("separated", truth(true)), ("short", truth(short))]);
        all.extend(fields);
        MeasuredMember {
            certain: true,
            exact,
            evidence: Vec::new(),
            fields: all,
        }
    };
    if exits.len() < 2 {
        // A finding already standing is not withdrawn for want of the rest.
        if !short {
            items.push(separated(
                vec![(
                    "separation",
                    MemberValue::Undecided {
                        why: format!(
                            "fewer than two certain exits: {}",
                            reached.undecided(&picked.undecided)
                        ),
                    },
                )],
                true,
            ));
        }
        return Ok(items);
    }
    let mut evidence = reached.evidence.clone();
    let measured = match Measured::of(context, &declared, space, exits, &mut evidence) {
        Ok(measured) => measured,
        Err(_) if short => return Ok(items),
        Err(unavailable) => return Err(unavailable),
    };
    let required = measured.required();
    let pairs = &measured.pairs;
    let standings: Vec<Standing> = pairs.iter().map(|pair| pair.standing(required)).collect();
    let mut open: Vec<String> = standings
        .iter()
        .filter_map(|standing| match standing {
            Standing::Unknown(why) => Some(why.clone()),
            _ => None,
        })
        .collect();
    if !maybe.is_empty() {
        open.push(reached.undecided(&picked.undecided));
    }
    let unmeasured = pairs.iter().any(|pair| pair.measured.is_err());
    let unsettled = unmeasured || !maybe.is_empty();
    let measured_spans = pairs.iter().filter_map(|pair| match &pair.measured {
        Ok((lower, upper, _)) => Some((*lower, *upper)),
        Err(_) => None,
    });
    // Some pair far enough apart: the greatest separation; every pair: the
    // least.
    let separation = match declared.pairs {
        Pairs::Any => measured_spans.reduce(|(a, b), (c, d)| (a.max(c), b.max(d))),
        Pairs::All => measured_spans.reduce(|(a, b), (c, d)| (a.min(c), b.min(d))),
    };
    let failing: Vec<&Pair> = match declared.pairs {
        Pairs::Any => pairs
            .iter()
            .max_by(|a, b| upper(a).total_cmp(&upper(b)))
            .into_iter()
            .collect(),
        Pairs::All => pairs
            .iter()
            .zip(&standings)
            .filter(|(_, standing)| matches!(standing, Standing::TooClose))
            .map(|(pair, _)| pair)
            .collect(),
    };
    let described: Vec<String> = failing
        .iter()
        .map(|pair| pair.describe(declared.separation))
        .collect();
    let failed = if declared.pairs == Pairs::Any && exits.len() > 2 {
        format!(
            "no two of its {} exits are far enough apart: {}",
            exits.len(),
            described.join("; ")
        )
    } else {
        format!("exits {}", described.join("; "))
    };
    // What a finding cites and relates: the path, the flag, the diagonal
    // and the pairs too close.
    evidence.push(measured.diameter.evidence().clone());
    let mut related: BTreeSet<ObjectId> = measured.related.clone();
    for pair in &failing {
        related.insert(pair.first.clone());
        related.insert(pair.second.clone());
        if let Ok((_, _, cited)) = &pair.measured {
            evidence.push(cited.clone());
        }
    }
    let exact = exact(&evidence);
    let separation = match separation {
        Some(span) => number(span, "separation", exact),
        None => MemberValue::Measured(Measurement::Absent {
            locator: at("separation"),
        }),
    };
    items.push(separated(
        vec![
            ("separation", separation),
            ("required", number(required, "required", exact)),
            (
                "undecided_fail",
                truth(declared.pairs == Pairs::Any && unsettled),
            ),
            (
                "undecided_pass",
                truth(declared.pairs == Pairs::All && unsettled),
            ),
            ("open", text(open.join("; "))),
            ("requirement", text(measured.requirement())),
            ("failed", text(failed)),
            (
                "related",
                MemberValue::Objects {
                    objects: related.into_iter().collect(),
                },
            ),
        ],
        exact,
    ));
    Ok(items)
}

impl MeasuredProvider for ExitMeasures {
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
