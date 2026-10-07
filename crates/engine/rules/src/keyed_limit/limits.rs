//! What `keyed-limit` reads, as measured values of an object:
//!
//! - `limit_row` (members): one item, the row of `limits` the object's keys
//!   select (the single most specific matching row): whether one matches,
//!   its index, the keys as a message describes them and the objects a path
//!   key was read on; refused where the keys cannot decide the row or rows
//!   tie, as the capability refused it. A list, read with the run's
//!   resolver, so a key a classification derives is read as derived.
//! - `limited_values` (members): what the row bounds, with the row and its
//!   bounds (none where no row with a bound applies, or the row cannot be
//!   selected, which `limit_row` reports): one
//!   item for a quantity of the object (`value`, `what`, `unit`), one per
//!   floor for a sill height (`what`), one per floor a side of a door may
//!   step onto for a threshold step (`named`, grouped by `side`, `sure`
//!   where the side surely steps onto it). A clear width, clear height,
//!   glazing ratio or measured value, and a step, are read as the decimals
//!   they display: an end within a few units in the last place of a bound
//!   is the bound. A member area some reached objects may add to is known
//!   only from below, and so undecided, unless it already exceeds the
//!   maximum. A quantity that cannot be measured refuses the list, worded
//!   as the capability left the object open.
//!
//! The rule the arguments state and its compiled rows are read once per
//! run, and each object's row once per run.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, CompiledRule, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement,
    MemberValue, PropertyResolutionError, RuleContext,
};

use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Evidence, ObjectId};

use super::defaults::DoorDefaults;
use super::{
    Limit, Measuring, Quantity, Selected, declared, limits, quantity, selected, sills, snapped,
};
use crate::measured_kinds::{interval, resolution_error};
use crate::support::{Parameters, Unavailable, invalid};

const LIMIT_ROW: &str = "limit_row";
const LIMITED_VALUES: &str = "limited_values";

/// Measures what `keyed-limit` reads.
pub(crate) struct LimitMeasures;

/// The rule the call's arguments state, its rows compiled and its
/// door-type defaults read: what every object of the run reads alike.
struct Prepared {
    rule: CompiledRule,
    limits: Arc<Vec<Limit>>,
    defaults: Option<Arc<DoorDefaults>>,
    /// The arguments an object's row depends on, kept once so each
    /// object's row is keyed without copying the rows.
    rows: ArgumentsKey,
}

#[derive(Hash, PartialEq, Eq)]
struct PreparedKey(ArgumentsKey);

/// The rule and rows `call` states, read once per run.
fn prepared(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Arc<Prepared>, Unavailable> {
    MeasuredMemo::of(
        context.services,
        PreparedKey(ArgumentsKey::of(call)),
        || {
            let rule = crate::measured_kinds::stated_rule(call, &[]);
            let rows = ArgumentsKey::of_keys(call, ROW_KEYS);
            let limits = compiled(&rule, &rows, context)?;
            let parameters = Parameters(&rule);
            let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
            let defaults = DoorDefaults::parse(&parameters, case_sensitive)?.map(Arc::new);
            Ok(Arc::new(Prepared {
                rule,
                limits,
                defaults,
                rows,
            }))
        },
    )
}

#[derive(Hash, PartialEq, Eq)]
struct RowsKey(ArgumentsKey);

/// The rows the keys select among, compiled once per run for the row and
/// for what it bounds alike.
fn compiled(
    rule: &CompiledRule,
    rows: &ArgumentsKey,
    context: &RuleContext<'_>,
) -> Result<Arc<Vec<Limit>>, Unavailable> {
    MeasuredMemo::of(context.services, RowsKey(rows.clone()), || {
        let parameters = Parameters(rule);
        let declared = declared(&parameters)?;
        limits(&parameters, &declared).map(Arc::new)
    })
}

/// The key of an object's row in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct RowKey(ArgumentsKey, ObjectId);

/// The row `object`'s keys select, read once per run for the keys and
/// rows the call states.
fn row(
    prepared: &Prepared,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Arc<Selected>, Unavailable> {
    let key = RowKey(prepared.rows.clone(), object.clone());
    MeasuredMemo::of(context.services, key, || {
        // A derived group is a resource object of the run, not the
        // project's.
        let subject = crate::selection::object_by_id(context, object)
            .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
        let declared = declared(&Parameters(&prepared.rule))?;
        selected(context, &declared, &prepared.limits, subject).map(Arc::new)
    })
}

/// The arguments the row depends on.
const ROW_KEYS: &[&str] = &[
    "limits",
    "key_1",
    "key_1_path",
    "key_2",
    "key_2_path",
    "key_3",
    "key_3_path",
    "key_4",
    "key_4_path",
    "pair_key",
    "case_sensitive",
];

fn number(value: Option<f64>, locator: &str) -> MemberValue {
    match value {
        Some(value) => {
            MemberValue::Measured(interval((value, value), None, true, locator.to_owned()))
        }
        None => MemberValue::Measured(Measurement::Absent {
            locator: locator.to_owned(),
        }),
    }
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

impl LimitMeasures {
    /// The row the object's keys select, as one item: whether a row
    /// matches (`listed`), its index (`row`), the keys as a message
    /// describes them (`keys`) and the objects a key was read on
    /// (`related`).
    fn row(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let prepared = prepared(call, context)?;
        let row = row(&prepared, object, context)?;
        let locator = format!("{LIMIT_ROW}:{object}");
        #[allow(clippy::cast_precision_loss)]
        let index = row.index.map(|index| index as f64);
        let item = MeasuredMember {
            certain: true,
            exact: true,
            fields: [
                (
                    "listed",
                    MemberValue::Truth {
                        value: row.index.is_some(),
                        locator: locator.clone(),
                    },
                ),
                ("row", number(index, &locator)),
                ("keys", text(row.described.clone())),
                (
                    "related",
                    MemberValue::Objects {
                        objects: row.sources.clone(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            evidence: Vec::new(),
        };
        Ok((vec![item], row.evidence.clone()))
    }

    /// What the row bounds, item by item, and what it was read from: one
    /// arm per form of quantity, kept together.
    #[allow(clippy::too_many_lines)]
    fn values(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let prepared = prepared(call, context)?;
        // A row that cannot be selected is the row list's to report.
        let Ok(row) = row(&prepared, object, context) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let Some(index) = row.index else {
            return Ok((Vec::new(), Vec::new()));
        };
        let limit = &prepared.limits[index];
        let (minimum, maximum) = (limit.minimum, limit.maximum);
        if minimum.is_none() && maximum.is_none() {
            return Ok((Vec::new(), Vec::new()));
        }
        // A derived group is a resource object of the run, not the
        // project's.
        let subject = crate::selection::object_by_id(context, object)
            .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
        let parameters = Parameters(&prepared.rule);
        let quantity = quantity(&parameters, || Ok(prepared.defaults.clone()))?;
        let locator = format!("{LIMITED_VALUES}:{object}");
        let item = |fields: Vec<(&'static str, MemberValue)>,
                    related: Vec<ObjectId>,
                    exact,
                    own: Vec<Evidence>| {
            let mut related_objects = row.sources.clone();
            related_objects.extend(related);
            related_objects.sort();
            related_objects.dedup();
            let mut fields: BTreeMap<&'static str, MemberValue> = fields.into_iter().collect();
            #[allow(clippy::cast_precision_loss)]
            fields.insert("row", number(Some(index as f64), &locator));
            fields.insert("keys", text(row.described.clone()));
            fields.insert("minimum", number(minimum, &locator));
            fields.insert("maximum", number(maximum, &locator));
            fields.insert(
                "related",
                MemberValue::Objects {
                    objects: related_objects,
                },
            );
            MeasuredMember {
                certain: true,
                exact,
                fields,
                evidence: own,
            }
        };
        let span = |(lower, upper): (f64, f64), exact: bool| {
            MemberValue::Measured(interval((lower, upper), None, exact, locator.clone()))
        };
        let mut evidence = row.evidence.clone();
        let items = match quantity {
            Quantity::SillHeight(path) => {
                let sills = sills(context, &path, subject)?;
                evidence.push(sills.window.clone());
                evidence.extend(sills.cited);
                sills
                    .floors
                    .into_iter()
                    .map(|(floor, measured)| {
                        let what = text(format!("sill height above the floor of {floor}"));
                        match measured {
                            Ok((height, cited)) => {
                                let exact = cited.exact && sills.window.exact;
                                item(
                                    vec![("value", span(height, exact)), ("what", what)],
                                    vec![floor],
                                    exact,
                                    vec![cited],
                                )
                            }
                            Err(why) => item(
                                vec![("value", MemberValue::Undecided { why }), ("what", what)],
                                vec![floor],
                                true,
                                Vec::new(),
                            ),
                        }
                    })
                    .collect()
            }
            Quantity::ThresholdStep(step) => {
                let step = match call.argument("ramp_selector") {
                    Some(_) => step.with_ramps(crate::measured_kinds::population(
                        call,
                        "ramp_selector",
                        context,
                    )?),
                    None => step,
                };
                let (alternatives, cited) =
                    step.alternatives(context, subject, (minimum, maximum))?;
                evidence.extend(cited);
                alternatives
                    .into_iter()
                    .map(|alternative| {
                        let sure = MemberValue::Truth {
                            value: alternative.sure,
                            locator: locator.clone(),
                        };
                        let side = text(alternative.side);
                        match alternative.step {
                            Ok((value, named)) => item(
                                vec![
                                    ("value", span(value, alternative.exact)),
                                    ("named", text(named)),
                                    ("side", side),
                                    ("sure", sure),
                                ],
                                vec![alternative.related],
                                alternative.exact,
                                alternative.evidence,
                            ),
                            Err(why) => item(
                                vec![
                                    ("value", MemberValue::Undecided { why }),
                                    ("named", text(String::new())),
                                    ("side", side),
                                    ("sure", sure),
                                ],
                                vec![alternative.related],
                                true,
                                alternative.evidence,
                            ),
                        }
                    })
                    .collect()
            }
            quantity => {
                let quantity = &quantity;
                let members = match quantity {
                    Quantity::MemberPlanArea { .. } => Some(crate::measured_kinds::population(
                        call,
                        "member_selector",
                        context,
                    )?),
                    _ => None,
                };
                let measuring = Measuring {
                    quantity,
                    members: members.as_deref(),
                };
                let (measured, related, undecided) = measuring.measure(context, subject)?;
                let exact = measured.evidence.iter().all(|evidence| evidence.exact);
                evidence.extend(measured.evidence);
                // Undecided members can only add area: with any, only a sum
                // already above the maximum stands.
                let value =
                    if undecided > 0 && !maximum.is_some_and(|maximum| measured.lower > maximum) {
                        MemberValue::Undecided {
                            why: format!(
                                "{undecided} reached object(s) may be members, so the {} is known \
                             only from below (limit row {index})",
                                measured.what
                            ),
                        }
                    } else {
                        let displayed = matches!(
                            quantity,
                            Quantity::ClearWidth(_)
                                | Quantity::ClearHeight(_)
                                | Quantity::GlazingRatio(_)
                                | Quantity::Measured(_)
                        );
                        let value = if displayed {
                            snapped(measured.lower, measured.upper, minimum, maximum)
                        } else {
                            (measured.lower, measured.upper)
                        };
                        span(value, exact)
                    };
                vec![item(
                    vec![
                        ("value", value),
                        ("what", text(measured.what)),
                        ("unit", text(measured.unit)),
                    ],
                    related,
                    exact,
                    Vec::new(),
                )]
            }
        };
        Ok((items, evidence))
    }
}

impl MeasuredProvider for LimitMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LIMIT_ROW, LIMITED_VALUES]
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
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    /// The items' findings cite the keys' and the quantity's evidence.
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let measured = if call.name() == LIMIT_ROW {
            Self::row(call, object, context)
        } else {
            Self::values(call, object, context)
        };
        measured.map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
