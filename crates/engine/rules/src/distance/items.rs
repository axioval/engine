//! What `distance` judges of a subject, as a measured member list
//! (`distance_items`), and what it leaves open of the objects its
//! selections and the broad phase could not decide, as one of the project
//! (`distance_open`), measured as the capability measures them: the
//! broad phase once per rule over the bound subjects and counterparts,
//! each subject's counterparts in scope, measured in the declared
//! projection, and what keeping them apart and having them within reach
//! come to (`verdicts`).
//!
//! A subject's one item holds, for keeping apart, the nearest violating
//! counterpart's distance, and for lying within, the nearest counterpart's
//! distance or the counterparts counted, each with the capability's words;
//! the template judges the distances against the bounds and the counts
//! against `count`, and joins the findings into one.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PropertyResolutionError, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Caches, Declaration, Judged, Kinds, Pairs, Scope, Verdict, declaration, verdicts};
use crate::measured_kinds::{interval, refused};
use crate::pairs::Unevaluated;
use crate::support::{Unavailable, invalid};

/// Measures what `distance` judges of a subject, and what it leaves open
/// of the objects its selections and the broad phase could not decide.
pub(crate) struct DistanceItems;

const ITEMS: &str = "distance_items";
const OPEN: &str = "distance_open";

/// The rule parameters a bound call holds, as a rule states them, keyed by
/// the call's keys (the rule's names); a bound selection as any selector,
/// since the objects are read from the bound selections.
fn stated(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter(|(key, _)| **key != "selection")
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Length(value) => ParameterValue::Number { value: *value },
                #[allow(clippy::cast_possible_truncation)]
                MeasuredArgument::Number(value) => ParameterValue::Integer {
                    value: *value as i64,
                },
                MeasuredArgument::Text(value) => ParameterValue::String {
                    value: value.clone(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                MeasuredArgument::Truth(value) => ParameterValue::Boolean { value: *value },
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

/// What a rule's subjects share: the declaration, the pairs prepared
/// once, the containers' kinds, and what each object reaches and each
/// counterpart's heights.
struct Judging {
    declared: Declaration,
    pairs: Pairs,
    kinds: Option<Kinds>,
    caches: Mutex<Caches>,
}

/// The memo of the latest rule's reading.
#[derive(Hash, PartialEq, Eq)]
struct Latest;

/// What the rule `call` stands for reads once for all its subjects; why
/// every subject is refused, where the declaration, a service or the
/// broad phase is unusable.
fn judging(call: &MeasuredCall, context: &RuleContext<'_>) -> Arc<Result<Judging, Unavailable>> {
    crate::measured_kinds::latest(context, Latest, call, || {
        let rule = crate::light_area::synthesised(stated(call));
        let declared = declaration(&rule)?;
        if let Some(missing) = heights_missing(context, declared.elevation) {
            return Err(missing);
        }
        let subjects =
            selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
        let counterparts =
            selection(call, "counterparts").ok_or_else(|| invalid("`counterparts` is required"))?;
        let objects = |selection: &MeasuredSelection| -> Vec<&Object> {
            context
                .project
                .objects()
                .filter(|object| selection.matched.contains(&object.id))
                .collect()
        };
        let mut unevaluated = Unevaluated::default();
        for chosen in [subjects, counterparts] {
            for (object, (reason, message)) in &chosen.reasons {
                unevaluated.push(object.clone(), reason.clone(), message.clone());
            }
        }
        let pairs = Pairs::among(
            context,
            &declared,
            (&objects(subjects), &objects(counterparts)),
            unevaluated,
        )?;
        let kinds = selection(call, "container_selector").map(|containers| Kinds {
            sure: containers.matched.clone(),
            undecided: containers.undecided.clone(),
        });
        Ok(Judging {
            declared,
            pairs,
            kinds,
            caches: Mutex::new(Caches::default()),
        })
    })
}

/// Why `elevation_overlap` cannot be judged: the vertical-extent service
/// is not registered.
fn heights_missing(context: &RuleContext<'_>, elevation: Option<f64>) -> Option<Unavailable> {
    (elevation.is_some()
        && context
            .services
            .get::<VerticalExtentServiceHandle>()
            .is_none())
    .then(|| {
        (
            NotEvaluatedReason::MissingService,
            "distance: `elevation_overlap` needs the vertical-extent service, which is not \
             registered"
                .to_owned(),
        )
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

/// A `null` number: nothing named against the bound.
fn absent(name: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Absent {
        locator: format!("{ITEMS}:{name}"),
    })
}

fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// One check's fields, named after `name`: its verdict's number (a
/// distance or a count), its words and the objects it relates; where it is
/// open, its number refused for why.
fn fields(
    name: &'static str,
    verdict: Option<Verdict>,
    (at, scope_evidence): (&str, &[Evidence]),
    fields: &mut Vec<(&'static str, MemberValue)>,
) -> bool {
    let words: &'static str = match name {
        "apart" => "apart_words",
        "reach" => "reach_words",
        _ => "count_words",
    };
    let related: &'static str = match name {
        "apart" => "apart_related",
        "reach" => "reach_related",
        _ => "count_related",
    };
    let checked: &'static str = match name {
        "apart" => "apart_checked",
        "reach" => "reach_checked",
        _ => "count_checked",
    };
    let Some(verdict) = verdict else {
        fields.push((checked, truth(false)));
        return true;
    };
    fields.push((checked, truth(true)));
    let number = |judged: Judged, exact: bool| match judged {
        Judged::Distance(lower, upper) => Some(MemberValue::Measured(interval(
            (lower, upper),
            Some(axioval_ir::QuantityDimension::Length),
            exact,
            format!("{ITEMS}:{at}:{name}"),
        ))),
        #[allow(clippy::cast_precision_loss)]
        Judged::Count(surely, possibly) => Some(MemberValue::Measured(interval(
            (surely as f64, possibly as f64),
            None,
            exact,
            format!("{ITEMS}:{at}:{name}"),
        ))),
        Judged::Nothing => None,
    };
    match verdict {
        Verdict::Finding {
            message,
            related: named,
            evidence,
            judged,
        } => {
            let sure = exact(&evidence) && exact(scope_evidence);
            if let Some(value) = number(judged, sure) {
                fields.push((name, value));
            } else {
                // Nothing within reach: a finding of its own.
                fields.push((name, absent(name)));
                fields.push(("none_within", truth(true)));
            }
            fields.push((words, text(message)));
            fields.push((related, MemberValue::Objects { objects: named }));
            sure
        }
        Verdict::NotEvaluated((reason, why)) => {
            let [field, stated] = crate::measured_kinds::refused_field(name, (reason, why));
            fields.push(field);
            // The item is open for its first open check's reason.
            if !fields.iter().any(|(name, _)| *name == "reason") {
                fields.push(stated);
            }
            true
        }
        // A distance that passes is not named; the counterparts counted
        // are.
        Verdict::Pass(judged @ Judged::Count(..)) if name == "count" => {
            fields.push((name, number(judged, true).unwrap_or_else(|| absent(name))));
            true
        }
        Verdict::Pass(_) => {
            fields.push((name, absent(name)));
            true
        }
    }
}

/// The item of one subject.
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let judging = judging(call, context);
    let Judging {
        declared,
        pairs,
        kinds,
        caches,
    } = judging.as_ref().as_ref().map_err(Clone::clone)?;
    if pairs.unmeasurable_subjects().contains(object) {
        // Its extent could not be read: open as the broad phase left it.
        let (_, reason, message) = pairs
            .unevaluated()
            .iter()
            .find(|(open, _, message)| open == object && unmeasured(message))
            .unwrap_or_else(|| unreachable!("an unmeasurable subject is left open"));
        return Err((reason.clone(), message.clone()));
    }
    let everything = crate::support::everything(context);
    let mut caches = caches.lock().unwrap_or_else(PoisonError::into_inner);
    let mut scope = Scope::new(
        declared,
        kinds.as_ref(),
        (context, &everything),
        &mut caches,
    );
    let judged = verdicts((context, declared), pairs, &mut scope, object)?;
    let at = object.to_string();
    let mut held = Vec::new();
    let nearest = matches!(declared.mode, super::Mode::Nearest);
    let mut sure = fields(
        "apart",
        judged.apart,
        (&at, &judged.scope_evidence),
        &mut held,
    );
    let (reach, count) = if nearest {
        (judged.within, None)
    } else {
        (None, judged.within)
    };
    sure &= fields("reach", reach, (&at, &judged.scope_evidence), &mut held);
    sure &= fields("count", count, (&at, &judged.scope_evidence), &mut held);
    if !held.iter().any(|(name, _)| *name == "none_within") {
        held.push(("none_within", truth(false)));
    }
    Ok(vec![MeasuredMember {
        certain: true,
        exact: sure,
        evidence: Vec::new(),
        fields: held.into_iter().collect(),
    }])
}

/// Whether `message` is the broad phase's word that an object's extent
/// could not be read.
fn unmeasured(message: &str) -> bool {
    message.ends_with("; pairs involving this object were not checked")
        || message.ends_with("; its distances were not checked")
}

/// The objects the rule's selections and the broad phase left open, beyond
/// what the rule's own selection and each subject report: one item each.
fn open(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let judging = judging(call, context);
    // A rule the capability refused whole leaves nothing else open.
    let Ok(Judging { pairs, .. }) = judging.as_ref() else {
        return Ok(Vec::new());
    };
    let subjects =
        selection(call, "selection").ok_or_else(|| invalid("`selection` is required"))?;
    Ok(pairs
        .unevaluated()
        .iter()
        .filter(|(object, reason, message)| {
            // The rule's selection reports its own undecided objects.
            let selected = subjects
                .reasons
                .get(object)
                .is_some_and(|(own, words)| own == reason && words == message);
            // A subject whose extent could not be read reports it itself.
            let subject = pairs.unmeasurable_subjects().contains(object) && unmeasured(message);
            !(selected || subject)
        })
        .map(|(object, reason, message)| MeasuredMember {
            certain: true,
            exact: true,
            evidence: Vec::new(),
            fields: [(
                "object",
                MemberValue::Objects {
                    objects: vec![object.clone()],
                },
            )]
            .into_iter()
            .chain(crate::measured_kinds::refused_field(
                "open",
                (reason.clone(), message.clone()),
            ))
            .collect(),
        })
        .collect())
}

impl MeasuredProvider for DistanceItems {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ITEMS, OPEN]
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
        open(call, context)
            .map(|members| (members, Vec::new()))
            .map_err(crate::measured_kinds::resolution_error)
    }
}
