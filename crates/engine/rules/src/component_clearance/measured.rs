//! What `component-clearance` checks of a component, as the measured member
//! list `clearance_checks`: each side's questions in the capability's order
//! (the volume free less its tolerance, each larger volume obstructed, the
//! volume inside its spaces, its base supported), each the search's answer
//! (`met`, undecided where the positions or selections leave it open), or,
//! with `quantifier` `any`, one answer for every side. The support coverage
//! is measured here, through `FreeSpaceService::assess_support_coverage`.
//!
//! Each run reads the rule's obstacles, walls and supports once, as bound
//! into the list.

use std::collections::BTreeMap;

use axioval_engine::{
    FreeSpaceServiceHandle, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, ObjectFrameServiceHandle, PlanSpanServiceHandle, PropertyResolutionError,
    RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};

use super::{Config, Obstacles, Services, Split, judged};
use crate::measured_kinds::{refused, refused_field};
use crate::support::{Unavailable, invalid};
use crate::wall_sides::Walls;

/// Measures `clearance_checks`.
pub(crate) struct ClearanceMeasures;

const CHECKS: &str = "clearance_checks";

/// What a rule reads once for all its components: its stated declaration
/// and the selections bound into the list.
struct Ruled {
    rule: axioval_engine::CompiledRule,
    obstacles: Obstacles,
    walls: Option<Walls>,
    supports: Option<Split>,
}

#[derive(Hash, PartialEq, Eq)]
struct Latest;

fn selection<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c MeasuredSelection> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection),
        _ => None,
    }
}

fn ruled(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> std::sync::Arc<Result<Ruled, Unavailable>> {
    crate::measured_kinds::latest(context, Latest, call, || {
        let obstacles =
            selection(call, "obstacles").ok_or_else(|| invalid("`obstacles` is required"))?;
        Ok(Ruled {
            rule: crate::measured_kinds::stated_rule(call, &[]),
            obstacles: Obstacles::of(obstacles, selection(call, "allowed_intruders")),
            walls: selection(call, "wall_selector")
                .map(|walls| Walls::possible(walls.matched.clone(), walls.undecided.clone())),
            supports: selection(call, "support_selector").map(Split::of),
        })
    })
}

fn checks(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let ruled = ruled(call, context);
    let ruled = ruled.as_ref().as_ref().map_err(Clone::clone)?;
    let config = Config::parse(&ruled.rule)?;
    let missing = |message: &str| (NotEvaluatedReason::MissingService, message.to_owned());
    let (Some(frames), Some(extents), Some(free_space)) = (
        context.services.get::<ObjectFrameServiceHandle>(),
        context.services.get::<VerticalExtentServiceHandle>(),
        context.services.get::<FreeSpaceServiceHandle>(),
    ) else {
        return Err(missing(
            "component-clearance needs the object-frame, vertical-extent and free-space services",
        ));
    };
    let spans = context.services.get::<PlanSpanServiceHandle>();
    if config.walls.is_some() && spans.is_none() {
        return Err(missing(
            "component-clearance with `front_axis` `against-wall` needs the plan-span service",
        ));
    }
    let services = Services {
        frames,
        extents,
        free_space,
        spans,
        walls: config.walls.and(ruled.walls.as_ref()),
        supports: config.support.and(ruled.supports.as_ref()),
    };
    let component = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    Ok(
        judged(context, &config, &services, &ruled.obstacles, component)
            .into_iter()
            .map(|(label, result)| item(object, label, result))
            .collect(),
    )
}

fn item(object: &ObjectId, label: String, result: super::Judged) -> MeasuredMember {
    let locator = format!("{CHECKS}:{object}:{label}");
    let mut fields = BTreeMap::from([("label", MemberValue::Text { text: label })]);
    let mut evidence = Vec::new();
    match result {
        Ok(None) => {
            fields.insert(
                "met",
                MemberValue::Truth {
                    value: true,
                    locator,
                },
            );
        }
        Ok(Some((words, cited, related))) => {
            fields.insert(
                "met",
                MemberValue::Truth {
                    value: false,
                    locator,
                },
            );
            fields.insert("words", MemberValue::Text { text: words });
            fields.insert("related", MemberValue::Objects { objects: related });
            evidence = cited;
        }
        Err(refusal) => fields.extend(refused_field("met", refusal)),
    }
    MeasuredMember {
        certain: true,
        exact: evidence.iter().all(|evidence| evidence.exact),
        fields,
        evidence,
    }
}

impl MeasuredProvider for ClearanceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[CHECKS]
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
        checks(call, object, context).map_err(refused(call.name(), object))
    }
}
