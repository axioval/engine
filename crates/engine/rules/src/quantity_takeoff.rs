//! Information takeoff: stated and measured quantities counted and summed
//! per group of objects.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::CategoryLevel;
use axioval_ir::{
    MEASURED_AREA, MEASURED_SET, MEASURED_VOLUME, Object, PropertyValue, QuantityDimension,
    ReportColumn, ReportTable, ReportValue, Scope,
};

use crate::selection::{Selection, selector_matches};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, category_headings, display, exact_f64,
    invalid, resolve,
};

/// The name of the table a takeoff reports.
pub const TAKEOFF_TABLE: &str = "takeoff";
/// How many group keys a takeoff declares at most (`group_1` ...).
const GROUPS: usize = 3;
/// How many quantities a takeoff measures at most (`measure_1` ...).
const MEASURES: usize = 4;

/// Counts the rule's selection per group and aggregates stated or measured
/// quantities of each group into the report table `takeoff`.
///
/// Groups are keyed by up to three properties (`group_1` to `group_3`),
/// each read on the object or, with `group_<n>_path`, on the objects the
/// path reaches (the storey), as a rule's categories read them: distinct
/// values join in one text, no value is `-`. A derived classification is
/// the property `<id>` in `axioval:classification`, and a level of a
/// hierarchical one `<id>;level=<n>`. Each row counts its
/// group (`count`) and aggregates up to four quantities (`measure_1` to
/// `measure_4`) by `sum` (the default), `min`, `max` or `mean`.
///
/// Values are intervals sure to hold the exact value, so aggregates are
/// too. An object whose selection cannot be decided may or may not belong
/// to its group, and one whose group cannot be read may belong to any group
/// of its scope: either widens the count and the aggregates of every group
/// it may belong to, and is reported not evaluated. A member whose quantity
/// is absent or unreadable makes its group's aggregate unknown. The
/// takeoff raises no finding.
pub struct QuantityTakeoff;

impl RuleCapability for QuantityTakeoff {
    fn id(&self) -> &'static str {
        "axioval:capability.quantity-takeoff"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = Vec::new();
        for n in 1..=GROUPS {
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}"),
                ParameterType::PropertyReference,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}_path"),
                ParameterType::StringList,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}_name"),
                ParameterType::String,
            ));
        }
        for n in 1..=MEASURES {
            parameters.push(ParameterDescriptor::optional(
                format!("measure_{n}"),
                ParameterType::PropertyReference,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("measure_{n}_aggregates"),
                ParameterType::StringList,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("measure_{n}_name"),
                ParameterType::String,
            ));
        }
        parameters.push(ParameterDescriptor::optional(
            "across_sources",
            ParameterType::Boolean,
        ));
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declaration = match Declaration::parse(rule) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("quantity-takeoff: {message}"),
                );
            }
        };
        let mut evaluation = CapabilityEvaluation::default();
        let mut scopes: BTreeMap<Scope, Vec<Member>> = BTreeMap::new();
        for object in context.project.objects() {
            let Some(member) = declaration.member(context, rule, object, &mut evaluation) else {
                continue;
            };
            let scope = if declaration.across_sources {
                Scope::Project
            } else {
                Scope::Source(object.id.source.clone())
            };
            scopes.entry(scope).or_default().push(member);
        }
        let kinds = declaration.column_kinds(&scopes, &mut evaluation);
        let table = match declaration.table(rule, &kinds, &scopes) {
            Ok(table) => table,
            Err(error) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidEvidence,
                    format!("quantity-takeoff: {error}"),
                );
            }
        };
        evaluation.push_table(table);
        evaluation
    }
}

/// How a measure's values are combined per group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Aggregate {
    Sum,
    Min,
    Max,
    Mean,
}

impl Aggregate {
    fn parse(name: &str) -> Result<Self, Unavailable> {
        Ok(match name {
            "sum" => Self::Sum,
            "min" => Self::Min,
            "max" => Self::Max,
            "mean" => Self::Mean,
            other => {
                return Err(invalid(format!(
                    "aggregate `{other}` is unsupported; use sum, min, max or mean"
                )));
            }
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Mean => "mean",
        }
    }
}

/// One quantity a takeoff aggregates.
struct Measure<'a> {
    property: PropertyRef<'a>,
    aggregates: Vec<Aggregate>,
    /// The column name after the aggregate: `sum_<name>`.
    name: String,
}

struct Declaration<'a> {
    /// Group column ids, one per level.
    group_ids: Vec<String>,
    levels: Vec<CategoryLevel>,
    measures: Vec<Measure<'a>>,
    across_sources: bool,
}

/// A value's interval and kind: a dimension, or `None` for a plain number.
type Kind = Option<QuantityDimension>;

#[derive(Clone, Copy)]
enum Value {
    Known { lower: f64, upper: f64, kind: Kind },
    Unknown,
}

/// One object the selection may hold.
struct Member {
    /// Its group, or `None` when it cannot be read.
    group: Option<Vec<String>>,
    /// Whether the selection surely holds it.
    certain: bool,
    /// One value per measure.
    values: Vec<Value>,
}

impl<'a> Declaration<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let mut group_ids = Vec::new();
        let mut levels = Vec::new();
        for n in 1..=GROUPS {
            let key = format!("group_{n}");
            let property = parameters.property(&key)?;
            let path = parameters.strings(&format!("{key}_path"))?;
            let name = parameters.string(&format!("{key}_name"))?;
            let Some(property) = property else {
                if path.is_some() || name.is_some() {
                    return Err(invalid(format!(
                        "`{key}_path` or `{key}_name` without `{key}`"
                    )));
                }
                continue;
            };
            if levels.len() + 1 != n {
                return Err(invalid(format!(
                    "`{key}` is declared without `group_{}`",
                    levels.len() + 1
                )));
            }
            let path = match path {
                None => Vec::new(),
                Some(path) => {
                    Traversal::path(path).map_err(|(reason, message)| {
                        (reason, format!("`{key}_path`: {message}"))
                    })?;
                    path.to_vec()
                }
            };
            group_ids.push(name.map_or(key, str::to_owned));
            levels.push(CategoryLevel {
                property_set: property.set.map(str::to_owned),
                property: property.name.to_owned(),
                path,
            });
        }
        let measures = measures(&parameters)?;
        let declaration = Self {
            group_ids,
            levels,
            measures,
            across_sources: parameters.boolean("across_sources")?.unwrap_or(false),
        };
        // Names, lengths and clashes of the columns are refused up front.
        declaration
            .empty_table(rule, &vec![None; declaration.measures.len()])
            .map_err(|error| {
                invalid(format!(
                    "{error}; name the columns with `group_<n>_name` or `measure_<n>_name`"
                ))
            })?;
        Ok(declaration)
    }

    /// The member `object` may be, if the selection may hold it; outcomes
    /// that leave it undecided are pushed to `evaluation`.
    fn member(
        &self,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
        object: &Object,
        evaluation: &mut CapabilityEvaluation,
    ) -> Option<Member> {
        let mut evidence = Vec::new();
        let mut problems: Vec<(NotEvaluatedReason, String)> = Vec::new();
        let certain = match selector_matches(context, &rule.selector, object, &mut evidence) {
            Selection::NoMatch => return None,
            Selection::Match => true,
            Selection::NotEvaluated(reason, message) => {
                problems.push((
                    reason,
                    format!("whether it is selected is undecided ({message}), so it may or may not count"),
                ));
                false
            }
        };
        let group = match category_headings(context, object, &self.levels) {
            Ok((headings, _)) => Some(headings),
            Err((reason, message)) => {
                problems.push((
                    reason,
                    format!("its group cannot be read ({message}), so it may count in any group"),
                ));
                None
            }
        };
        let values = self
            .measures
            .iter()
            .map(|measure| match resolve(context, object, measure.property) {
                Ok(resolved) => match numeric(resolved.value()) {
                    Ok(value) => value,
                    Err((reason, why)) => {
                        problems.push((
                            reason,
                            format!(
                                "`{}` {why}, so its group's `{}` is unknown",
                                measure.property, measure.name
                            ),
                        ));
                        Value::Unknown
                    }
                },
                Err((reason, message)) => {
                    problems.push((
                        reason,
                        format!(
                            "`{}` cannot be read ({message}), so its group's `{}` is unknown",
                            measure.property, measure.name
                        ),
                    ));
                    Value::Unknown
                }
            })
            .collect();
        if let Some((reason, _)) = problems.first() {
            let reason = reason.clone();
            let message = problems
                .into_iter()
                .map(|(_, message)| message)
                .collect::<Vec<_>>()
                .join("; ");
            evaluation.push_object_not_evaluated(
                object.id.clone(),
                reason,
                format!("quantity-takeoff: {message}"),
            );
        }
        Some(Member {
            group,
            certain,
            values,
        })
    }

    /// The column kind of each measure: the one kind its values state, or
    /// the kind of its measured name when none is read. Values of several
    /// kinds leave the measure unknown (`None`) and the rule not evaluated.
    fn column_kinds(
        &self,
        scopes: &BTreeMap<Scope, Vec<Member>>,
        evaluation: &mut CapabilityEvaluation,
    ) -> Vec<Option<Kind>> {
        self.measures
            .iter()
            .enumerate()
            .map(|(index, measure)| {
                let mut kinds: Vec<Kind> = Vec::new();
                for member in scopes.values().flatten() {
                    if let Value::Known { kind, .. } = member.values[index]
                        && !kinds.contains(&kind)
                    {
                        kinds.push(kind);
                    }
                }
                let mut kinds = kinds.into_iter();
                match (kinds.next(), kinds.next()) {
                    (None, _) => Some(measured_kind(measure.property)),
                    (Some(kind), None) => Some(kind),
                    (Some(first), Some(second)) => {
                        evaluation.push_not_evaluated(
                            NotEvaluatedReason::InvalidEvidence,
                            format!(
                                "quantity-takeoff: `{}` is stated as {} and as {}, so no `{}` is aggregated",
                                measure.property,
                                kind_text(first),
                                kind_text(second),
                                measure.name
                            ),
                        );
                        None
                    }
                }
            })
            .collect()
    }

    fn empty_table(
        &self,
        rule: &CompiledRule,
        kinds: &[Option<Kind>],
    ) -> Result<ReportTable, axioval_ir::ReportTableError> {
        let mut columns = vec![ReportColumn::number("count")];
        for (measure, kind) in self.measures.iter().zip(kinds) {
            for aggregate in &measure.aggregates {
                let id = format!("{}_{}", aggregate.name(), measure.name);
                columns.push(match kind.flatten() {
                    Some(dimension) => ReportColumn::quantity(id, dimension),
                    None => ReportColumn::number(id),
                });
            }
        }
        ReportTable::grouped(
            rule.id.clone(),
            TAKEOFF_TABLE,
            self.group_ids.clone(),
            columns,
        )
    }

    /// One row per group of each scope.
    fn table(
        &self,
        rule: &CompiledRule,
        kinds: &[Option<Kind>],
        scopes: &BTreeMap<Scope, Vec<Member>>,
    ) -> Result<ReportTable, axioval_ir::ReportTableError> {
        let mut table = self.empty_table(rule, kinds)?;
        for (scope, members) in scopes {
            let groups: BTreeSet<&Vec<String>> = members
                .iter()
                .filter_map(|member| member.group.as_ref())
                .collect();
            for group in groups {
                let (sure, maybe): (Vec<&Member>, Vec<&Member>) = members
                    .iter()
                    .filter(|member| member.group.as_ref().is_none_or(|own| own == group))
                    .partition(|member| member.certain && member.group.is_some());
                #[allow(clippy::cast_precision_loss)]
                let mut values = vec![ReportValue::measured(
                    sure.len() as f64,
                    (sure.len() + maybe.len()) as f64,
                )];
                for (index, (measure, kind)) in self.measures.iter().zip(kinds).enumerate() {
                    let read = |members: &[&Member]| -> Option<Vec<(f64, f64)>> {
                        members
                            .iter()
                            .map(|member| match member.values[index] {
                                Value::Known { lower, upper, .. } => Some((lower, upper)),
                                Value::Unknown => None,
                            })
                            .collect()
                    };
                    let bounds = kind.and(read(&sure)).zip(read(&maybe));
                    for aggregate in &measure.aggregates {
                        values.push(match &bounds {
                            Some((sure, maybe)) => aggregated(*aggregate, sure, maybe),
                            None => ReportValue::Unknown,
                        });
                    }
                }
                table.push_group_row(scope.clone(), group.clone(), values)?;
            }
        }
        Ok(table)
    }
}

/// The declared measures, `measure_1` onwards.
fn measures<'a>(parameters: &Parameters<'a>) -> Result<Vec<Measure<'a>>, Unavailable> {
    let mut measures = Vec::new();
    for n in 1..=MEASURES {
        let key = format!("measure_{n}");
        let property = parameters.property(&key)?;
        let aggregates = parameters.strings(&format!("{key}_aggregates"))?;
        let name = parameters.string(&format!("{key}_name"))?;
        let Some(property) = property else {
            if aggregates.is_some() || name.is_some() {
                return Err(invalid(format!(
                    "`{key}_aggregates` or `{key}_name` without `{key}`"
                )));
            }
            continue;
        };
        if measures.len() + 1 != n {
            return Err(invalid(format!(
                "`{key}` is declared without `measure_{}`",
                measures.len() + 1
            )));
        }
        let aggregates = match aggregates {
            None => vec![Aggregate::Sum],
            Some([]) => return Err(invalid(format!("`{key}_aggregates` is empty"))),
            Some(names) => {
                let parsed = names
                    .iter()
                    .map(|name| Aggregate::parse(name.trim()))
                    .collect::<Result<Vec<_>, _>>()?;
                if parsed.iter().collect::<BTreeSet<_>>().len() != parsed.len() {
                    return Err(invalid(format!("`{key}_aggregates` repeats an aggregate")));
                }
                parsed
            }
        };
        let name = match name {
            Some(name) => name.to_owned(),
            None => column_name(property.name),
        };
        if name.is_empty() {
            return Err(invalid(format!(
                "`{key}` gives no column name; declare `{key}_name`"
            )));
        }
        measures.push(Measure {
            property,
            aggregates,
            name,
        });
    }
    Ok(measures)
}

/// `values` combined by `aggregate`, where `sure` surely belong to the group
/// and `maybe` may: an interval sure to hold the aggregate of every
/// membership and every value the intervals allow.
fn aggregated(aggregate: Aggregate, sure: &[(f64, f64)], maybe: &[(f64, f64)]) -> ReportValue {
    match aggregate {
        Aggregate::Sum => {
            let lower = sure.iter().map(|value| value.0).sum::<f64>()
                + maybe.iter().map(|value| value.0.min(0.0)).sum::<f64>();
            let upper = sure.iter().map(|value| value.1).sum::<f64>()
                + maybe.iter().map(|value| value.1.max(0.0)).sum::<f64>();
            ReportValue::measured(lower, upper)
        }
        // With no sure member the group may hold no value at all.
        _ if sure.is_empty() => ReportValue::Unknown,
        Aggregate::Min => {
            let lower = sure
                .iter()
                .chain(maybe)
                .map(|value| value.0)
                .fold(f64::INFINITY, f64::min);
            let upper = sure
                .iter()
                .map(|value| value.1)
                .fold(f64::INFINITY, f64::min);
            ReportValue::measured(lower, upper)
        }
        Aggregate::Max => {
            let lower = sure
                .iter()
                .map(|value| value.0)
                .fold(f64::NEG_INFINITY, f64::max);
            let upper = sure
                .iter()
                .chain(maybe)
                .map(|value| value.1)
                .fold(f64::NEG_INFINITY, f64::max);
            ReportValue::measured(lower, upper)
        }
        Aggregate::Mean => {
            let lower = extreme_mean(
                sure.iter().map(|value| value.0),
                maybe.iter().map(|value| value.0),
                true,
            );
            let upper = extreme_mean(
                sure.iter().map(|value| value.1),
                maybe.iter().map(|value| value.1),
                false,
            );
            ReportValue::measured(lower, upper)
        }
    }
}

/// The least (`least`) or greatest mean of every `sure` value together
/// with any choice of `maybe` values: the optional values below (above)
/// the running mean, smallest (largest) first, each lowers (raises) it.
fn extreme_mean(
    sure: impl Iterator<Item = f64>,
    maybe: impl Iterator<Item = f64>,
    least: bool,
) -> f64 {
    let (mut total, mut count) = sure.fold((0.0, 0.0), |(total, count), value| {
        (total + value, count + 1.0)
    });
    let mut maybe: Vec<f64> = maybe.collect();
    maybe.sort_by(f64::total_cmp);
    if !least {
        maybe.reverse();
    }
    for value in maybe {
        let mean = total / count;
        if (least && value < mean) || (!least && value > mean) {
            total += value;
            count += 1.0;
        } else {
            break;
        }
    }
    total / count
}

/// A value as a number or quantity interval, or why it is none.
fn numeric(value: Option<&PropertyValue>) -> Result<Value, (NotEvaluatedReason, String)> {
    let known = |lower: f64, upper: f64, kind: Kind| {
        if lower.is_finite() && upper.is_finite() && lower <= upper {
            Ok(Value::Known { lower, upper, kind })
        } else {
            Err((
                NotEvaluatedReason::InvalidEvidence,
                "is not finite".to_owned(),
            ))
        }
    };
    match value {
        None | Some(PropertyValue::Null) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            "has no value".to_owned(),
        )),
        Some(PropertyValue::Integer(value)) => match exact_f64(*value) {
            Some(value) => known(value, value, None),
            None => Err((
                NotEvaluatedReason::InvalidEvidence,
                "is too large to add exactly".to_owned(),
            )),
        },
        Some(PropertyValue::Decimal(value)) => known(*value, *value, None),
        Some(PropertyValue::Quantity { value, dimension }) => {
            known(*value, *value, Some(*dimension))
        }
        Some(PropertyValue::Measured {
            lower,
            upper,
            dimension,
        }) => known(*lower, *upper, Some(*dimension)),
        Some(other) => Err((
            NotEvaluatedReason::InvalidEvidence,
            format!("states {}, not a number or quantity", display(Some(other))),
        )),
    }
}

/// The kind a measured name answers in, a length unless an area or volume;
/// a plain number for any other property.
fn measured_kind(property: PropertyRef<'_>) -> Kind {
    if property.set != Some(MEASURED_SET) {
        return None;
    }
    let name = property.name.to_ascii_lowercase();
    Some(match name.as_str() {
        MEASURED_AREA => QuantityDimension::Area,
        MEASURED_VOLUME => QuantityDimension::Volume,
        _ => QuantityDimension::Length,
    })
}

fn kind_text(kind: Kind) -> String {
    match kind {
        Some(dimension) => format!("a quantity in {}", dimension.unit_symbol()),
        None => "a plain number".to_owned(),
    }
}

/// A property name as a column name: the part after its last `.`, words
/// split at case changes and joined by `_`, lowercase (`t.NetSideArea` is
/// `net_side_area`).
fn column_name(property: &str) -> String {
    let base = property.rsplit('.').next().unwrap_or(property);
    let mut name = String::new();
    let mut after_lower = false;
    for character in base.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && after_lower {
                name.push('_');
            }
            after_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
            name.push(character.to_ascii_lowercase());
        } else {
            if !name.is_empty() && !name.ends_with('_') {
                name.push('_');
            }
            after_lower = false;
        }
    }
    name.trim_end_matches('_').to_owned()
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Aggregate, aggregated, column_name};
    use axioval_ir::ReportValue;

    #[test]
    fn column_names_follow_the_property_name() {
        assert_eq!(column_name("t.NetSideArea"), "net_side_area");
        assert_eq!(column_name("extent_z"), "extent_z");
        assert_eq!(column_name("Qto_WallBaseQuantities.Length"), "length");
        assert_eq!(column_name("GrossArea2"), "gross_area2");
        assert_eq!(column_name("Fläche"), "fl_che");
        assert_eq!(column_name("ÄÖ"), "");
    }

    #[test]
    fn sums_widen_by_what_possible_members_may_add() {
        let sure = [(10.0, 10.0), (12.0, 13.0)];
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[]),
            ReportValue::measured(22.0, 23.0)
        );
        // A possible member adds nothing or its value.
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[(5.0, 6.0)]),
            ReportValue::measured(22.0, 29.0)
        );
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[(-2.0, 1.0)]),
            ReportValue::measured(20.0, 24.0)
        );
        assert_eq!(
            aggregated(Aggregate::Sum, &[], &[(5.0, 5.0)]),
            ReportValue::measured(0.0, 5.0)
        );
    }

    #[test]
    fn extremes_and_means_bound_every_membership() {
        let sure = [(4.0, 4.0), (6.0, 6.0)];
        let maybe = [(1.0, 1.0), (9.0, 9.0), (5.0, 5.0)];
        assert_eq!(
            aggregated(Aggregate::Min, &sure, &maybe),
            ReportValue::measured(1.0, 4.0)
        );
        assert_eq!(
            aggregated(Aggregate::Max, &sure, &maybe),
            ReportValue::measured(6.0, 9.0)
        );
        // Least mean: 4, 6 and 1 (11 / 3); greatest: 4, 6 and 9 (19 / 3).
        assert_eq!(
            aggregated(Aggregate::Mean, &sure, &maybe),
            ReportValue::measured(11.0 / 3.0, 19.0 / 3.0)
        );
        assert_eq!(
            aggregated(Aggregate::Mean, &sure, &[]),
            ReportValue::exact(5.0)
        );
        // A group that may be empty has no sure extreme or mean.
        for aggregate in [Aggregate::Min, Aggregate::Max, Aggregate::Mean] {
            assert_eq!(aggregated(aggregate, &[], &maybe), ReportValue::Unknown);
        }
    }
}
