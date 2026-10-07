//! What `group-composition` judges, as the measured member list
//! `compositions` of the project: the maximum matching of each selected
//! group's members to the rows of `requirements` (`compose`), one item per
//! part every maximum matching leaves short or with a surplus, per group no
//! row matches, per row no group matches, per object no group reaches, and
//! per group or object left open. Each item names where its outcome goes
//! (`at`): the group, the member, the object, or the project's stand-in.

use std::collections::BTreeMap;

use axioval_engine::template::scope_stand_in;
use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId};

use super::{Composed, Selected, compose};
use crate::counts::Population;
use crate::measured_kinds::{refused_field, resolution_error};
use crate::support::{Unavailable, invalid};

/// Measures `compositions`.
pub(crate) struct CompositionMeasures;

const COMPOSITIONS: &str = "compositions";

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: COMPOSITIONS.to_owned(),
    }
}

fn objects(objects: Vec<ObjectId>) -> MemberValue {
    MemberValue::Objects { objects }
}

/// A count, exactly as the matching counted it.
#[allow(clippy::cast_precision_loss)]
fn count(value: usize, locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Rounded {
        lower: value as f64,
        upper: value as f64,
        dimension: None,
        locator: format!("{COMPOSITIONS}:{locator}"),
    })
}

fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// One item.
fn item(composed: Composed) -> MeasuredMember {
    let mut fields = BTreeMap::new();
    let mut evidence = Vec::new();
    match composed {
        Composed::Open { at, why } => {
            fields.insert("at", objects(vec![at]));
            fields.extend(refused_field("open", why));
        }
        Composed::Unmatched {
            group,
            keys,
            evidence: cited,
        } => {
            fields.insert("at", objects(vec![group]));
            fields.insert("unmatched", truth(true));
            fields.insert("keys", text(keys));
            evidence = cited;
        }
        Composed::Absent {
            name,
            evidence: cited,
        } => {
            fields.insert("at", objects(vec![scope_stand_in(None)]));
            fields.insert("absent", truth(true));
            fields.insert("name", text(name));
            evidence = cited;
        }
        Composed::Ungrouped { object, via } => {
            fields.insert("at", objects(vec![object]));
            fields.insert("ungrouped", truth(true));
            fields.insert("via", text(via));
        }
        Composed::Short {
            group,
            subject,
            filled,
            places,
            via,
            related,
            evidence: cited,
        } => {
            fields.insert("at", objects(vec![group]));
            fields.insert("subject", text(subject));
            fields.insert("filled", count(filled, "filled"));
            fields.insert("places", count(places, "places"));
            fields.insert("missing", count(places.saturating_sub(filled), "missing"));
            fields.insert("via", text(via));
            fields.insert("related", objects(related));
            evidence = cited;
        }
        Composed::Surplus {
            group,
            entries,
            keys,
            found,
            places,
            via,
            related,
            evidence: cited,
        } => {
            fields.insert("at", objects(vec![group]));
            fields.insert("unfit", truth(entries.is_none()));
            if let Some((names, verb)) = entries {
                fields.insert("entries", text(names));
                fields.insert("verb", text(verb.to_owned()));
            }
            fields.insert("keys", text(keys));
            fields.insert("found", count(found, "found"));
            fields.insert("places", count(places, "places"));
            fields.insert("surplus", count(found.saturating_sub(places), "surplus"));
            fields.insert("via", text(via));
            fields.insert("related", objects(related));
            evidence = cited;
        }
    }
    MeasuredMember {
        certain: true,
        exact: exact(&evidence),
        fields,
        evidence,
    }
}

/// The items of the project.
fn compositions(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let rule = crate::measured_kinds::stated_numbers_rule(call);
    let selection = |key: &str| match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection.as_ref()),
        _ => None,
    };
    let groups = selection("selection").ok_or_else(|| invalid("`selection` is required"))?;
    // Every object is a member where no `member_selector` narrows them.
    let every = if let Some(picked) = selection("member_selector") {
        Population {
            matched: picked.matched.clone(),
            undecided: picked.undecided.clone(),
            first: None,
        }
    } else {
        Population {
            matched: context
                .project
                .objects()
                .map(|object| object.id.clone())
                .collect(),
            undecided: std::collections::BTreeSet::new(),
            first: None,
        }
    };
    let selected = Selected {
        groups,
        members: &every,
        ungrouped: selection("ungrouped_selector"),
    };
    Ok(compose(context, &rule, &selected)?
        .into_iter()
        .map(item)
        .collect())
}

impl MeasuredProvider for CompositionMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[COMPOSITIONS]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members_of_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        compositions(call, context)
            .map(|members| (members, Vec::new()))
            .map_err(resolution_error)
    }
}
