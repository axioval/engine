//! What `local-circulation` finds of the spaces a rule selects, as the
//! measured member list `circulation_verdicts` of each space: the
//! circulation search (`circulate`) over every selected space, one item per
//! answer it gives about a space or a component (an entrance missing or too
//! narrow, a component not reached or not linked, a path end without its
//! free area, a stretch without a passing space), each met or not, or
//! undecided with why, naming the object it is about (`at`).

use std::collections::BTreeMap;

use axioval_engine::template::scope_stand_in;
use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, Object, ObjectId, Scope};

use super::{Source, circulate, declaration};
use crate::measured_kinds::{refused, refused_field};
use crate::space_access::{AccessDeclaration, Pick};
use crate::support::{Unavailable, invalid};

/// Measures `circulation_verdicts`.
pub(crate) struct CirculationMeasures;

const VERDICTS: &str = "circulation_verdicts";

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

/// Every selected space's items: the answers about it and its components,
/// and, with the first space's, the answers about no one space.
type BySpace = std::sync::Arc<Result<BTreeMap<ObjectId, Vec<MeasuredMember>>, Unavailable>>;

fn by_space(call: &MeasuredCall, context: &RuleContext<'_>) -> BySpace {
    crate::measured_kinds::latest(context, Latest, call, || {
        let items = verdicts(call, context)?;
        let spaces: Vec<ObjectId> = match selection(call, "selection") {
            Some(selected) => context
                .project
                .objects()
                .filter(|object| selected.matched.contains(&object.id))
                .map(|object| object.id.clone())
                .collect(),
            None => Vec::new(),
        };
        let mut by_space: BTreeMap<ObjectId, Vec<MeasuredMember>> = spaces
            .iter()
            .map(|space| (space.clone(), Vec::new()))
            .collect();
        for (at, item) in items {
            let space = if by_space.contains_key(&at) {
                at
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

/// Every answer, with the object it is about.
fn verdicts(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<(ObjectId, MeasuredMember)>, Unavailable> {
    let rule = crate::measured_kinds::stated_rule(call, &[]);
    let mut declared = declaration(&rule)?;
    // The selections are the ones bound into the list.
    if let Some(components) = selection(call, "component_selector") {
        declared.components = Source::Selected(components);
    }
    declared.obstacles = selection(call, "obstacles").map(Source::Selected);
    declared.swings = selection(call, "subtract_door_swings").map(Source::Selected);
    declared.partners = selection(call, "partner_selector").map(Source::Selected);
    declared.exempt = declared.exempt.and_then(|(_, reach)| {
        selection(call, "end_exempt_selector").map(|exempt| (Source::Selected(exempt), reach))
    });
    if let Some(MeasuredArgument::Path(steps)) = call.argument("access_path") {
        declared.access = AccessDeclaration::of(
            steps,
            selection(call, "door_selector").map(Pick::Selected),
            selection(call, "opening_selector").map(Pick::Selected),
            selection(call, "space_selector").map(Pick::Selected),
        )?;
    }
    let spaces = selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
    let spaces: Vec<&Object> = context
        .project
        .objects()
        .filter(|object| spaces.matched.contains(&object.id))
        .collect();
    let answers = circulate(context, &rule, &declared, &spaces);
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
    Ok(items)
}

impl MeasuredProvider for CirculationMeasures {
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
