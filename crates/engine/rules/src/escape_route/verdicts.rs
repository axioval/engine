//! What `escape-route` finds of the spaces a rule selects, as the measured
//! member list `escape_verdicts` of each space: the escape search
//! (`search`) over every selected space, one item per answer it gives
//! about a space, a passage, a door or a source (travel, exits, widths,
//! door directions, clear heights, loads), each met or not, or undecided
//! with why, naming what it is about (`at`).

use std::collections::BTreeMap;

use axioval_engine::template::scope_stand_in;
use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, Object, ObjectId, Scope};

use super::{Bindings, Selected, declaration, search};
use crate::measured_kinds::{refused, refused_field};
use crate::support::{Unavailable, invalid};

/// Measures `escape_verdicts`.
pub(crate) struct EscapeMeasures;

const VERDICTS: &str = "escape_verdicts";

fn selection<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c MeasuredSelection> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection),
        _ => None,
    }
}

fn at(scope: &Scope) -> MemberValue {
    MemberValue::Objects {
        objects: vec![match scope {
            Scope::Object(object) => object.clone(),
            Scope::Source(source) => scope_stand_in(Some(source)),
            Scope::Project => scope_stand_in(None),
        }],
    }
}

/// The memo of the latest rule's answers.
#[derive(Hash, PartialEq, Eq)]
struct Latest;

/// Every selected space's items: the answers about it, and, with the first
/// space's, the answers about no one selected space (a passage, a door, a
/// source).
type BySpace = std::sync::Arc<Result<BTreeMap<ObjectId, Vec<MeasuredMember>>, Unavailable>>;

fn by_space(call: &MeasuredCall, context: &RuleContext<'_>) -> BySpace {
    crate::measured_kinds::latest(context, Latest, call, || {
        let (spaces, items) = verdicts(call, context)?;
        let mut by_space: BTreeMap<ObjectId, Vec<MeasuredMember>> = spaces
            .iter()
            .map(|space| (space.clone(), Vec::new()))
            .collect();
        for (about, item) in items {
            let space = if by_space.contains_key(&about) {
                about
            } else {
                match spaces.first() {
                    Some(first) => first.clone(),
                    None => continue,
                }
            };
            by_space.entry(space).or_default().push(item);
        }
        Ok(by_space)
    })
}

/// The spaces surely selected, and every answer with what it is about.
type Answers = (Vec<ObjectId>, Vec<(ObjectId, MeasuredMember)>);

fn verdicts(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Answers, Unavailable> {
    let rule = crate::measured_kinds::stated_rule(call, &[]);
    let mut declared = declaration(&rule)?;
    // The connectors are the ones bound into the list.
    declared.climbing = declared.climbing.map(|climbing| {
        climbing.selected([
            selection(call, "stair_selector"),
            selection(call, "ramp_selector"),
            selection(call, "lift_selector"),
        ])
    });
    let bound = Bindings {
        exits: selection(call, "exit_selector"),
        doors: selection(call, "door_selector"),
        passages: selection(call, "passage_selector"),
        no_escape: selection(call, "no_escape_selector"),
        route_doors: selection(call, "route_door_selector"),
        compartments: selection(call, "compartment_selector"),
    };
    let picked = selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
    let mut spaces: Vec<&Object> = Vec::new();
    let mut checked: Vec<&Object> = Vec::new();
    for object in context.project.objects() {
        if picked.matched.contains(&object.id) {
            spaces.push(object);
            checked.push(object);
        } else if picked.undecided.contains(&object.id) {
            checked.push(object);
        }
    }
    let undecided: Vec<ObjectId> = picked.undecided.iter().cloned().collect();
    let answers = search(
        context,
        &rule,
        &declared,
        &bound,
        &Selected {
            spaces: &spaces,
            undecided: &undecided,
            checked: &checked,
        },
    );
    Ok((
        spaces.iter().map(|space| space.id.clone()).collect(),
        items(&answers),
    ))
}

/// The search's answers as items, each with what it is about.
fn items(answers: &axioval_engine::CapabilityEvaluation) -> Vec<(ObjectId, MeasuredMember)> {
    let mut items = Vec::new();
    for found in answers.findings() {
        let evidence: Vec<Evidence> = found.evidence.clone();
        let about = match &found.scope {
            Scope::Object(object) => object.clone(),
            _ => scope_stand_in(None),
        };
        items.push((
            about,
            MeasuredMember {
                certain: true,
                exact: evidence.iter().all(|evidence| evidence.exact),
                fields: BTreeMap::from([
                    ("at", at(&found.scope)),
                    (
                        "met",
                        MemberValue::Truth {
                            value: false,
                            locator: VERDICTS.to_owned(),
                        },
                    ),
                    (
                        "words",
                        MemberValue::Text {
                            text: found.message.clone(),
                        },
                    ),
                    (
                        "related",
                        MemberValue::Objects {
                            objects: found.related.clone(),
                        },
                    ),
                ]),
                evidence,
            },
        ));
    }
    for open in answers.not_evaluated_outcomes() {
        let scope = match open.object_id() {
            Some(object) => Scope::Object(object.clone()),
            None => Scope::Project,
        };
        let mut fields = BTreeMap::from([("at", at(&scope))]);
        fields.extend(refused_field(
            "met",
            (open.reason().clone(), open.message().to_owned()),
        ));
        items.push((
            match open.object_id() {
                Some(object) => object.clone(),
                None => scope_stand_in(None),
            },
            MeasuredMember {
                certain: true,
                exact: true,
                fields,
                evidence: Vec::new(),
            },
        ));
    }
    items
}

impl MeasuredProvider for EscapeMeasures {
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
        let by_space = by_space(call, context);
        match by_space.as_ref() {
            Ok(by_space) => Ok(by_space.get(object).cloned().unwrap_or_default()),
            Err(refusal) => Err(refused(call.name(), object)(refusal.clone())),
        }
    }
}
