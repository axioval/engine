//! What `accessible-route` walks to a destination, as the measured member
//! list `route_verdicts`: one item, whether some start reaches it for the
//! rule's body with its passing spaces (`reached`, undecided with why where
//! the walk cannot tell), and, where it is cut off or lacks passing spaces,
//! the words naming what blocks it, the objects that do and the evidence.
//!
//! Each run takes the rule's scene and walkability snapshot once, for every
//! destination the rule selects.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId};

use super::{Verdict, Walked, bound, declaration, walked};
use crate::measured_kinds::{refused, refused_field};
use crate::support::{Unavailable, invalid};

/// Measures `route_verdicts`.
pub(crate) struct RouteMeasures;

const VERDICTS: &str = "route_verdicts";

/// What a rule reads once: its stated declaration and its walk.
struct Ruled {
    rule: axioval_engine::CompiledRule,
    walked: Result<Walked, Unavailable>,
}

#[derive(Hash, PartialEq, Eq)]
struct Latest;

fn ruled(call: &MeasuredCall, context: &RuleContext<'_>) -> Arc<Result<Ruled, Unavailable>> {
    crate::measured_kinds::latest(context, Latest, call, || {
        let rule = crate::measured_kinds::stated_numbers_rule(call);
        let Some(MeasuredArgument::Objects(selection)) = call.argument("selection") else {
            return Err(invalid("`selection` is required"));
        };
        let destinations: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| selection.matched.contains(&object.id))
            .collect();
        let walked = {
            let declared = bound(declaration(&rule)?, call);
            walked(context, &declared, &destinations)
        };
        Ok(Ruled { rule, walked })
    })
}

fn verdicts(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let ruled = ruled(call, context);
    let ruled = ruled.as_ref().as_ref().map_err(Clone::clone)?;
    let walked = ruled.walked.as_ref().map_err(Clone::clone)?;
    let declared = bound(declaration(&ruled.rule)?, call);
    let locator = format!("{VERDICTS}:{object}");
    let reached = |value: bool| MemberValue::Truth {
        value,
        locator: locator.clone(),
    };
    let judge = walked.judge(context, &declared);
    let found = match judge.destination(object) {
        Verdict::Reachable => None,
        Verdict::Blocked(blocked) => Some(judge.finding(&ruled.rule, object, blocked)),
        Verdict::Crowded(missed) => Some(judge.crowded(&ruled.rule, object, missed)),
        Verdict::Undecided(reason, why) => {
            return Ok(vec![MeasuredMember {
                certain: true,
                exact: true,
                fields: refused_field("reached", (reason, why))
                    .into_iter()
                    .collect(),
                evidence: Vec::new(),
            }]);
        }
    };
    let mut fields = BTreeMap::new();
    let mut evidence = Vec::new();
    match found {
        None => {
            fields.insert("reached", reached(true));
        }
        Some(found) => {
            fields.insert("reached", reached(false));
            fields.insert(
                "words",
                MemberValue::Text {
                    text: found.message,
                },
            );
            fields.insert(
                "related",
                MemberValue::Objects {
                    objects: found.related,
                },
            );
            evidence = found.evidence;
        }
    }
    Ok(vec![MeasuredMember {
        certain: true,
        exact: evidence.iter().all(|evidence| evidence.exact),
        fields,
        evidence,
    }])
}

impl MeasuredProvider for RouteMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[VERDICTS]
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
        verdicts(call, object, context).map_err(refused(call.name(), object))
    }
}
