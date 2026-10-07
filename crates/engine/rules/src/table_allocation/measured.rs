//! What `table-allocation` judges, as the measured member list
//! `allocations` of the project: the allocation of the rule's selection
//! (`selection`) to the rows of `rows` (`allocate`), one item per outcome
//! of an object of its own (an extra, an object left open), per group whose
//! rows cannot be told, and per row of a group: its objects counted, and
//! their summed area where the row states one. Each item names where its
//! outcomes go (`at`): the object, the anchor, or the source's or the
//! project's stand-in.

use std::collections::BTreeMap;

use axioval_engine::template::scope_stand_in;
use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, Scope};

use super::{Allocated, Counted, allocate};
use crate::counts::Population;
use crate::measured_kinds::{refused_field, resolution_error};
use crate::support::{Unavailable, invalid};

/// Measures `allocations`.
pub(crate) struct AllocationMeasures;

const ALLOCATIONS: &str = "allocations";

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: ALLOCATIONS.to_owned(),
    }
}

fn objects(objects: Vec<ObjectId>) -> MemberValue {
    MemberValue::Objects { objects }
}

fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// A number counted or stated, a point.
#[allow(clippy::cast_precision_loss)]
fn counted(value: usize, locator: String) -> MemberValue {
    MemberValue::Measured(Measurement::Rounded {
        lower: value as f64,
        upper: value as f64,
        dimension: None,
        locator,
    })
}

/// Where outcomes about `scope` go: the object, or the stand-in of the
/// source or the project.
fn at(scope: &Scope) -> MemberValue {
    objects(vec![match scope {
        Scope::Object(object) => object.clone(),
        Scope::Source(source) => scope_stand_in(Some(source)),
        Scope::Project => scope_stand_in(None),
    }])
}

/// What every item of one row of one group states.
fn row_fields(row: &Counted, fields: &mut BTreeMap<&'static str, MemberValue>) {
    fields.insert("at", at(&row.scope));
    fields.insert("name", text(row.name.clone()));
    fields.insert("place", text(row.place.clone()));
    fields.insert("related", objects(row.assigned.clone()));
    fields.insert(
        "open_count",
        counted(row.open, format!("{ALLOCATIONS}:{}:open", row.name)),
    );
}

/// One item.
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn item(allocated: Allocated) -> MeasuredMember {
    let mut fields = BTreeMap::new();
    let mut evidence = Vec::new();
    match allocated {
        Allocated::Extra {
            object,
            keys,
            evidence: cited,
        } => {
            fields.insert("at", objects(vec![object]));
            fields.insert("extra", truth(true));
            fields.insert("keys", text(keys));
            evidence = cited;
        }
        Allocated::Open { object, why } => {
            fields.insert("at", objects(vec![object]));
            fields.extend(refused_field("open", why));
        }
        Allocated::GroupOpen {
            scope,
            why: (reason, message),
        } => {
            fields.insert("at", at(&scope));
            fields.extend(refused_field(
                "open",
                (reason, format!("table-allocation: {message}")),
            ));
        }
        Allocated::Count(row) => {
            row_fields(&row, &mut fields);
            let locator = format!("{ALLOCATIONS}:{}:count", row.name);
            let found = row.assigned.len() as f64;
            fields.insert(
                "found",
                MemberValue::Measured(Measurement::Cited {
                    lower: found,
                    upper: found + row.open as f64,
                    dimension: None,
                    locator: locator.clone(),
                    exact: exact(&row.evidence),
                }),
            );
            fields.insert(
                "count",
                MemberValue::Measured(match row.count {
                    Some(count) => Measurement::Rounded {
                        lower: count as f64,
                        upper: count as f64,
                        dimension: None,
                        locator,
                    },
                    None => Measurement::Absent { locator },
                }),
            );
            fields.insert("empty", truth(row.empty()));
            evidence = row.evidence;
        }
        Allocated::Summed {
            counted: row,
            area,
            sum,
        } => {
            row_fields(&row, &mut fields);
            let (low, high) = area.bounds();
            let locator = |bound: &str| format!("{ALLOCATIONS}:{}:{bound}", row.name);
            let bound = |value: f64, name: &str| {
                MemberValue::Measured(Measurement::Rounded {
                    lower: value,
                    upper: value,
                    dimension: Some(QuantityDimension::Area),
                    locator: locator(name),
                })
            };
            fields.insert("low", bound(low, "low"));
            fields.insert("high", bound(high, "high"));
            fields.insert("required", text(area.required()));
            fields.insert("more", truth(row.open > 0));
            evidence = row.evidence;
            match sum {
                Ok(sum) => {
                    evidence.extend(sum.evidence);
                    fields.insert(
                        "sum",
                        MemberValue::Measured(Measurement::Cited {
                            lower: sum.lower,
                            upper: sum.upper,
                            dimension: Some(QuantityDimension::Area),
                            locator: locator("sum"),
                            exact: exact(&evidence),
                        }),
                    );
                }
                Err((reason, message)) => fields.extend(refused_field(
                    "sum",
                    (
                        reason,
                        format!("table-allocation: {}{}: {message}", row.name, row.place),
                    ),
                )),
            }
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
fn allocations(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let rule = crate::measured_kinds::stated_numbers_rule(call);
    let Some(MeasuredArgument::Objects(selection)) = call.argument("selection") else {
        return Err(invalid("`selection` is required"));
    };
    let members = Population {
        matched: selection.matched.clone(),
        undecided: selection.undecided.clone(),
        first: None,
    };
    let anchors = match call.argument("anchor_selector") {
        Some(MeasuredArgument::Objects(anchors)) => Some(anchors.as_ref()),
        _ => None,
    };
    Ok(allocate(context, &rule, &members, anchors)?
        .into_iter()
        .map(item)
        .collect())
}

impl MeasuredProvider for AllocationMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ALLOCATIONS]
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
        allocations(call, context)
            .map(|members| (members, Vec::new()))
            .map_err(resolution_error)
    }
}
