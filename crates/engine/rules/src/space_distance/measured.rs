//! What `space-distance` judges of a space, as the measured member list
//! `distance_rows`: one item per row of `distances` whose `from` picks the
//! space, in the table's order, each the nearest destination's distance as
//! the search bounds it (`nearest`, from every destination that might
//! qualify to the sure ones, either end infinite where nothing bounds it),
//! the row's bounds, and the words, related objects and evidence of lying
//! beyond its maximum or within its minimum. A row whose destinations
//! cannot be listed is undecided, for its reason; a space whose rows
//! cannot be told is one undecided item.
//!
//! Each run reads the rule's rows, storeys, access and connectors once, and
//! keeps what it measured (targets, storeys, partners, points, lengths)
//! for all its spaces.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use super::{Answer, Search, declaration};
use crate::measured_kinds::{refused, refused_field, stated_reason};
use crate::space_access::{AccessDeclaration, Pick};
use crate::support::{Unavailable, invalid};

/// Measures `distance_rows`.
pub(crate) struct RowMeasures;

const ROWS: &str = "distance_rows";

fn selection<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c MeasuredSelection> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection),
        _ => None,
    }
}

/// The memo of the latest rule's search.
#[derive(Hash, PartialEq, Eq)]
struct Latest;

/// The search the rule `call` stands for, read once for all its spaces.
fn search(call: &MeasuredCall, context: &RuleContext<'_>) -> Arc<Result<Search, Unavailable>> {
    crate::measured_kinds::latest(context, Latest, call, || {
        let rule = crate::measured_kinds::stated_numbers_rule(call);
        let mut declared = declaration(&rule)?;
        // The selections are the ones bound into the call, never the
        // stand-ins the stated rule holds.
        declared.access = match call.argument("access_path") {
            Some(MeasuredArgument::Path(steps)) => Some(AccessDeclaration::of(
                steps,
                selection(call, "door_selector").map(Pick::Selected),
                selection(call, "opening_selector").map(Pick::Selected),
                selection(call, "space_selector").map(Pick::Selected),
            )?),
            _ => None,
        };
        declared.climbing = declared.climbing.map(|climbing| {
            climbing.selected([
                selection(call, "stair_selector"),
                selection(call, "ramp_selector"),
                selection(call, "lift_selector"),
            ])
        });
        Ok(Search::new(
            context,
            declared,
            selection(call, "storey_selector"),
        ))
    })
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn number(value: Option<f64>, locator: String) -> MemberValue {
    MemberValue::Measured(match value {
        Some(value) => Measurement::Rounded {
            lower: value,
            upper: value,
            dimension: Some(QuantityDimension::Length),
            locator,
        },
        None => Measurement::Absent { locator },
    })
}

fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// The item of one row.
fn item(space: &ObjectId, answer: Answer) -> MeasuredMember {
    let Answer { row, measured } = answer;
    let at = |field: &str| format!("{ROWS}:{space}:{}:{field}", row.name);
    let mut fields: BTreeMap<&'static str, MemberValue> = BTreeMap::from([
        ("row", text(row.name.clone())),
        ("minimum", number(row.minimum, at("minimum"))),
        ("maximum", number(row.maximum, at("maximum"))),
    ]);
    let measured = match measured {
        Ok(measured) => measured,
        Err(refusal) => {
            fields.extend(refused_field("nearest", refusal));
            return MeasuredMember {
                certain: true,
                exact: true,
                fields,
                evidence: Vec::new(),
            };
        }
    };
    // A finding cites what bounds the distance beyond the bound it misses:
    // the nearest possible destination beyond a maximum, the nearest sure
    // one within a minimum.
    let beyond = row
        .maximum
        .is_some_and(|maximum| measured.least > maximum)
        .then_some(measured.above.as_ref())
        .flatten();
    let within = row
        .minimum
        .is_some_and(|minimum| measured.most < minimum)
        .then_some(measured.below.as_ref())
        .flatten();
    let evidence = beyond
        .or(within)
        .map(|finding| finding.evidence.clone())
        .unwrap_or_default();
    let (reason, open) = measured.open;
    for (name, finding) in [("above", measured.above), ("below", measured.below)] {
        let (words, related) = match name {
            "above" => ("above_words", "above_related"),
            _ => ("below_words", "below_related"),
        };
        if let Some(finding) = finding {
            fields.insert(words, text(finding.words));
            fields.insert(
                related,
                MemberValue::Objects {
                    objects: finding.related,
                },
            );
        }
    }
    fields.insert("open_words", text(open));
    fields.insert("reason", stated_reason(&reason));
    let sure = exact(&evidence);
    fields.insert(
        "nearest",
        MemberValue::Measured(Measurement::Cited {
            lower: measured.least,
            upper: measured.most,
            dimension: Some(QuantityDimension::Length),
            locator: at("nearest"),
            exact: sure,
        }),
    );
    MeasuredMember {
        certain: true,
        exact: sure,
        fields,
        evidence,
    }
}

/// The items of `object`.
fn rows(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let search = search(call, context);
    let search = search.as_ref().as_ref().map_err(Clone::clone)?;
    let space = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    match search.answer(context, space) {
        Ok(answers) => Ok(answers
            .into_iter()
            .map(|answer| item(object, answer))
            .collect()),
        // The rows that apply cannot be told: one undecided item.
        Err(refusal) => Ok(vec![MeasuredMember {
            certain: true,
            exact: true,
            fields: refused_field("nearest", refusal).into_iter().collect(),
            evidence: Vec::new(),
        }]),
    }
}

impl MeasuredProvider for RowMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ROWS]
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
        rows(call, object, context).map_err(refused(call.name(), object))
    }
}
