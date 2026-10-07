//! An anchor's members in order as a measured member list
//! (`name_sequence`), read as `name-sequence` reads them: the members the
//! bound member selection picks that the traversal reaches (or every one
//! of the anchor's source), ordered by their order value, each with its
//! number and the number the sequence expects of it.
//!
//! The sequence runs on from the last member whose number counted (stated,
//! whole and not below `first`), so a member without one does not
//! interrupt it, as the capability read it; the template judges each
//! number against what is expected.

use std::collections::BTreeMap;

use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, SelectionIdentity};
use axioval_ir::{Evidence, ObjectId};

use super::{Config, Member, Numbered, members, numbered};
use crate::measured_kinds::{interval, refused};
use crate::support::{PropertyRef, Unavailable, invalid};

/// Measures an anchor's numbered members.
pub(crate) struct SequenceMeasures;

const LIST: &str = "name_sequence";

/// The memo key of a member selection's candidates.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Listed(SelectionIdentity);

/// The declaration a bound call holds: what the rule declared, checked by
/// the template's declaration (`check_arguments`) before any list is read.
fn declared(call: &MeasuredCall) -> Result<Config<'_>, Unavailable> {
    let property = |key: &str| match call.argument(key) {
        Some(MeasuredArgument::Property { set, name }) => Ok(PropertyRef {
            set: set.as_deref(),
            name,
        }),
        _ => Err(invalid(format!("`{key}` is required"))),
    };
    #[allow(clippy::cast_possible_truncation)]
    let integer = |key: &str| match call.argument(key) {
        Some(MeasuredArgument::Number(value)) => Some(*value as i64),
        _ => None,
    };
    Ok(Config {
        #[cfg(feature = "parity-reference")]
        members: None,
        name: property("name")?,
        order: property("order")?,
        first: integer("first").unwrap_or(1),
        increment: integer("increment").unwrap_or(1),
        traversal: crate::measured_kinds::traversal(call)?,
        placement_fallback: call.argument("order_fallback").is_some(),
    })
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

/// The items of one anchor: its members in order.
#[allow(clippy::too_many_lines)]
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let config = declared(call)?;
    let Some(MeasuredArgument::Objects(selection)) = call.argument("member_selector") else {
        return Err(invalid("`member_selector` is required"));
    };
    let anchor = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    // The candidates in the project's order, listed once per selection.
    let candidates = MeasuredMemo::of(
        context.services,
        Listed(SelectionIdentity(selection.clone())),
        || Arc::new(selection.matched.iter().cloned().collect::<Vec<ObjectId>>()),
    );
    let ordered = members(
        context,
        &config,
        anchor,
        (&candidates, selection.first_undecided.clone()),
    )?;
    #[allow(clippy::cast_precision_loss)]
    let number = |value: i64, field: &str| {
        MemberValue::Measured(interval(
            (value as f64, value as f64),
            None,
            true,
            format!("{LIST}:{object}:{field}"),
        ))
    };
    let absent = |field: &str| {
        MemberValue::Measured(Measurement::Absent {
            locator: format!("{LIST}:{object}:{field}"),
        })
    };
    let mut items = Vec::new();
    let mut previous: Option<(i64, &Member<'_>)> = None;
    for member in &ordered {
        let stated = numbered(member.name.as_ref());
        let value = match stated {
            Numbered::Number(value) => Some(value),
            Numbered::Unset | Numbered::Word => None,
        };
        // A number that counts carries the sequence on; only its order
        // against the member below cites that member too.
        let counts = value.is_some_and(|value| value >= config.first);
        let below = previous.filter(|_| counts).map(|(_, below)| below);
        let expected = match previous {
            None => config.first,
            Some((before, _)) => before.saturating_add(config.increment),
        };
        let mut evidence = member.evidence.clone();
        if let Some(below) = below {
            evidence.extend(below.evidence.iter().cloned());
        }
        items.push(MeasuredMember {
            certain: true,
            exact: exact(&evidence),
            evidence: Vec::new(),
            fields: BTreeMap::from([
                (
                    "member",
                    MemberValue::Objects {
                        objects: vec![member.object.id.clone()],
                    },
                ),
                (
                    "shown",
                    MemberValue::Text {
                        text: crate::support::display(member.name.as_ref()),
                    },
                ),
                ("set", truth(!matches!(stated, Numbered::Unset))),
                ("whole", truth(value.is_some())),
                (
                    "value",
                    value.map_or_else(|| absent("value"), |value| number(value, "value")),
                ),
                (
                    "previous",
                    previous.map_or_else(
                        || absent("previous"),
                        |(before, _)| number(before, "previous"),
                    ),
                ),
                ("expected", number(expected, "expected")),
                (
                    "above",
                    truth(
                        value
                            .zip(previous)
                            .is_some_and(|(value, (before, _))| value > before),
                    ),
                ),
                (
                    "below",
                    MemberValue::Objects {
                        objects: below
                            .map(|below| below.object.id.clone())
                            .into_iter()
                            .collect(),
                    },
                ),
            ]),
        });
        if let Some(value) = value.filter(|_| counts) {
            previous = Some((value, member));
        }
    }
    Ok(items)
}

impl MeasuredProvider for SequenceMeasures {
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
