//! Exact, source-neutral property-to-property comparison capability.

use std::cmp::Ordering;

use axioval_engine::{
    AbsentEndPolicy, CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PropertyResolution, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionServiceHandle,
    RuleCapability, RuleContext, SemanticRelationship, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Finding, Object, PropertyValue, Severity};

use crate::selection::{bound_property_request, property_error, select_objects};

/// Compares a property on relationship-selected candidates with a property on each checked object.
pub struct PropertyComparison;

#[derive(Clone, Copy)]
enum Mode {
    Checked,
    Shared,
    Related,
}
#[derive(Clone, Copy)]
enum Quantifier {
    Each,
    AtLeastOne,
    /// The number of candidates, compared with the target.
    Count,
    /// The sum of the candidates' compared values, compared with the target.
    Sum,
}
#[derive(Clone, Copy)]
enum Operator {
    Equals,
    NotEquals,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Contains,
    OneOf,
    NoneOf,
}

/// The right-hand side of every comparison.
enum Target<'a> {
    /// A property of the checked object.
    Property(Option<&'a str>, &'a str),
    /// A declared constant.
    Value(PropertyValue),
    /// A declared list of allowed texts, for `one_of` and `none_of`.
    Texts(&'a [String]),
}

impl RuleCapability for PropertyComparison {
    fn id(&self) -> &'static str {
        "axioval:capability.property-comparison"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("compared_selector", ParameterType::Selector),
            // Not read by `count`, which compares the number of candidates.
            ParameterDescriptor::optional("compared_property", ParameterType::PropertyReference),
            // Exactly one target: a property of the checked object or a constant.
            ParameterDescriptor::optional("target_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("target_number", ParameterType::Number),
            ParameterDescriptor::optional("target_text", ParameterType::String),
            ParameterDescriptor::optional("target_texts", ParameterType::StringList),
            ParameterDescriptor::optional("target_boolean", ParameterType::Boolean),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::required("factor", ParameterType::Number),
            ParameterDescriptor::required("component_mode", ParameterType::String),
            ParameterDescriptor::optional("relationship", ParameterType::String),
            ParameterDescriptor::optional("direction", ParameterType::String),
            ParameterDescriptor::optional("follow_chain", ParameterType::Boolean),
            // Opt-in: answer from the relationships that exist when the source
            // has instances missing a required end, citing each one.
            ParameterDescriptor::optional("skip_absent_relationship_ends", ParameterType::Boolean),
            ParameterDescriptor::required("quantifier", ParameterType::String),
        ]
    }
    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some(config) = Config::parse(rule) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-comparison parameters are invalid",
            );
        };
        let (checked, mut evaluation) = select_objects(context, &rule.selector);
        let (universe, universe_outcomes) = select_objects(context, config.selector);
        if !universe_outcomes.not_evaluated_outcomes().is_empty() {
            for object in checked {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    "compared selector was not evaluated conclusively",
                );
            }
            return evaluation;
        }
        for object in checked {
            let selected = match config.mode {
                Mode::Checked => Ok((
                    if universe
                        .binary_search_by_key(&&object.id, |candidate| &candidate.id)
                        .is_ok()
                    {
                        vec![object.id.clone()]
                    } else {
                        vec![]
                    },
                    vec![],
                )),
                Mode::Shared | Mode::Related => {
                    relationship_selection(context, object, &universe, &config)
                }
            };
            let (candidates, relation_evidence) = match selected {
                Ok(value) => value,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            if matches!(config.quantifier, Quantifier::Count | Quantifier::Sum) {
                aggregate(
                    context,
                    rule,
                    object,
                    &candidates,
                    &relation_evidence,
                    &config,
                    &mut evaluation,
                );
                continue;
            }
            if candidates.is_empty() {
                if matches!(config.quantifier, Quantifier::AtLeastOne) {
                    evaluation.push_finding(make_finding(
                        rule,
                        object,
                        "no candidate satisfies comparison".into(),
                        relation_evidence,
                    ));
                }
                continue;
            }
            let Some(properties) = context.services.get::<PropertyResolutionServiceHandle>() else {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "property-resolution service is not registered",
                );
                continue;
            };
            let target = match target_side(context, properties, object, &config.target) {
                Ok((Some(value), _)) => value,
                Ok((None, absence_evidence)) => {
                    evaluation.push_finding(make_finding(
                        rule,
                        object,
                        "target property is absent".into(),
                        combined(&relation_evidence, &absence_evidence, &[]),
                    ));
                    continue;
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let mut any_match = false;
            let mut uncertainties = Vec::new();
            let mut mismatches = Vec::new();
            let mut missing = Vec::new();
            for candidate_id in candidates {
                let Some(candidate) = context.project.object(&candidate_id) else {
                    uncertainties.push((
                        NotEvaluatedReason::InvalidEvidence,
                        "relationship candidate is absent from project".into(),
                    ));
                    continue;
                };
                let (compared_set, compared_name) = config
                    .compared
                    .expect("parsing requires a compared property here");
                match resolve(context, properties, candidate, compared_set, compared_name) {
                    Ok((Some(compared), _)) => match compare_side(
                        &compared.0.value,
                        &target,
                        config.factor,
                        config.operator,
                    ) {
                        Ok(true) => any_match = true,
                        Ok(false) => mismatches.push((
                            candidate,
                            combined(&relation_evidence, &compared.1, target.evidence()),
                        )),
                        Err(message) => {
                            uncertainties.push((NotEvaluatedReason::InvalidEvidence, message));
                        }
                    },
                    Ok((None, absence_evidence)) => missing.push((
                        candidate,
                        combined(&relation_evidence, &absence_evidence, target.evidence()),
                    )),
                    Err(error) => uncertainties.push(error),
                }
            }
            let has_missing_information = !missing.is_empty();
            for (candidate, evidence) in missing {
                evaluation.push_finding(make_finding(
                    rule,
                    candidate,
                    "compared property is absent".into(),
                    evidence,
                ));
            }
            match config.quantifier {
                Quantifier::Each => {
                    for (candidate, evidence) in mismatches {
                        evaluation.push_finding(make_finding(
                            rule,
                            object,
                            format!("candidate {} does not satisfy comparison", candidate.id),
                            evidence,
                        ));
                    }
                    for (reason, message) in uncertainties {
                        evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    }
                }
                // Handled by `aggregate` before candidates are compared one by one.
                Quantifier::Count | Quantifier::Sum => unreachable!(),
                Quantifier::AtLeastOne if any_match || has_missing_information => {}
                Quantifier::AtLeastOne => {
                    if uncertainties.is_empty() {
                        let mut evidence = combined(&relation_evidence, target.evidence(), &[]);
                        for (_, mismatch_evidence) in &mismatches {
                            evidence = combined(&evidence, mismatch_evidence, &[]);
                        }
                        evaluation.push_finding(make_finding(
                            rule,
                            object,
                            "no candidate satisfies comparison".into(),
                            evidence,
                        ));
                    } else {
                        for (reason, message) in uncertainties {
                            evaluation.push_object_not_evaluated(
                                object.id.clone(),
                                reason,
                                message,
                            );
                        }
                    }
                }
            }
        }
        evaluation
    }
}

struct Config<'a> {
    selector: &'a Selector,
    compared: Option<(Option<&'a str>, &'a str)>,
    target: Target<'a>,
    operator: Operator,
    factor: f64,
    mode: Mode,
    relationship: Option<&'a str>,
    direction: TraversalDirection,
    follow_chain: bool,
    absent_ends: AbsentEndPolicy,
    quantifier: Quantifier,
}
impl<'a> Config<'a> {
    #[allow(clippy::too_many_lines)]
    fn parse(rule: &'a CompiledRule) -> Option<Self> {
        let ParameterValue::Selector { value: selector } =
            rule.parameters.get("compared_selector")?
        else {
            return None;
        };
        let property = |name| match rule.parameters.get(name)? {
            ParameterValue::PropertyReference {
                property,
                property_set,
            } => Some((property_set.as_deref(), property.as_str())),
            _ => None,
        };
        let string = |name| match rule.parameters.get(name)? {
            ParameterValue::String { value } => Some(value.as_str()),
            _ => None,
        };
        let compared = match rule.parameters.get("compared_property") {
            None => None,
            Some(_) => Some(property("compared_property")?),
        };
        let mut targets = Vec::new();
        if rule.parameters.contains_key("target_property") {
            let (set, name) = property("target_property")?;
            targets.push(Target::Property(set, name));
        }
        match rule.parameters.get("target_number") {
            None => {}
            Some(ParameterValue::Number { value }) if value.is_finite() => {
                targets.push(Target::Value(PropertyValue::Decimal(*value)));
            }
            Some(_) => return None,
        }
        match rule.parameters.get("target_text") {
            None => {}
            Some(ParameterValue::String { value }) => {
                targets.push(Target::Value(PropertyValue::String(value.clone())));
            }
            Some(_) => return None,
        }
        match rule.parameters.get("target_boolean") {
            None => {}
            Some(ParameterValue::Boolean { value }) => {
                targets.push(Target::Value(PropertyValue::Boolean(*value)));
            }
            Some(_) => return None,
        }
        match rule.parameters.get("target_texts") {
            None => {}
            Some(ParameterValue::StringList { value }) => targets.push(Target::Texts(value)),
            Some(_) => return None,
        }
        let Ok([target]) = <[Target<'a>; 1]>::try_from(targets) else {
            return None;
        };
        let operator = match string("operator")? {
            "equals" => Operator::Equals,
            "not_equals" => Operator::NotEquals,
            "greater" => Operator::Greater,
            "greater_or_equal" => Operator::GreaterOrEqual,
            "less" => Operator::Less,
            "less_or_equal" => Operator::LessOrEqual,
            "contains" => Operator::Contains,
            "one_of" => Operator::OneOf,
            "none_of" => Operator::NoneOf,
            _ => return None,
        };
        // A text list is the target of `one_of`/`none_of` and of nothing else.
        if matches!(operator, Operator::OneOf | Operator::NoneOf)
            != matches!(target, Target::Texts(_))
        {
            return None;
        }
        let factor = match rule.parameters.get("factor")? {
            ParameterValue::Number { value } if value.is_finite() => *value,
            _ => return None,
        };
        let mode = match string("component_mode")? {
            "checked" => Mode::Checked,
            "shared" => Mode::Shared,
            "related" => Mode::Related,
            _ => return None,
        };
        let relationship = string("relationship");
        if !matches!(mode, Mode::Checked) && relationship.is_none_or(str::is_empty) {
            return None;
        }
        let direction = match string("direction") {
            None | Some("forward") => TraversalDirection::Forward,
            Some("backward") => TraversalDirection::Backward,
            Some("either") => TraversalDirection::Either,
            Some(_) => return None,
        };
        let follow_chain = match rule.parameters.get("follow_chain") {
            None => false,
            Some(ParameterValue::Boolean { value }) => *value,
            _ => return None,
        };
        let absent_ends = match rule.parameters.get("skip_absent_relationship_ends") {
            None | Some(ParameterValue::Boolean { value: false }) => AbsentEndPolicy::Refuse,
            Some(ParameterValue::Boolean { value: true }) => AbsentEndPolicy::Skip,
            _ => return None,
        };
        let quantifier = match string("quantifier")? {
            "each" => Quantifier::Each,
            "at_least_one" => Quantifier::AtLeastOne,
            "count" => Quantifier::Count,
            "sum" => Quantifier::Sum,
            _ => return None,
        };
        if compared.is_none() && !matches!(quantifier, Quantifier::Count) {
            return None;
        }
        // A count or sum is one number; a text list cannot be its target.
        if matches!(quantifier, Quantifier::Count | Quantifier::Sum)
            && matches!(target, Target::Texts(_))
        {
            return None;
        }
        Some(Self {
            selector,
            compared,
            target,
            operator,
            factor,
            mode,
            relationship,
            direction,
            follow_chain,
            absent_ends,
            quantifier,
        })
    }
}

/// The evaluated right-hand side of a comparison.
enum Side<'a> {
    Value(PropertyValue, Vec<Evidence>),
    Texts(&'a [String]),
}

impl Side<'_> {
    fn evidence(&self) -> &[Evidence] {
        match self {
            Self::Value(_, evidence) => evidence,
            Self::Texts(_) => &[],
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Value(value, _) => crate::support::display(Some(value)),
            Self::Texts(texts) => format!("[{}]", texts.join(", ")),
        }
    }
}

/// The target for `object`: its own property, or the declared constant.
fn target_side<'a>(
    context: &RuleContext<'_>,
    properties: &PropertyResolutionServiceHandle,
    object: &Object,
    target: &Target<'a>,
) -> Result<(Option<Side<'a>>, Vec<Evidence>), (NotEvaluatedReason, String)> {
    match target {
        Target::Property(set, name) => {
            resolve(context, properties, object, *set, name).map(|(value, absence)| {
                (
                    value.map(|(property, evidence)| Side::Value(property.value, evidence)),
                    absence,
                )
            })
        }
        Target::Value(value) => Ok((Some(Side::Value(value.clone(), Vec::new())), Vec::new())),
        Target::Texts(texts) => Ok((Some(Side::Texts(texts)), Vec::new())),
    }
}

fn compare_side(
    left: &PropertyValue,
    side: &Side<'_>,
    factor: f64,
    operator: Operator,
) -> Result<bool, String> {
    match side {
        Side::Value(right, _) => compare(left, right, factor, operator),
        Side::Texts(texts) => match left {
            PropertyValue::String(text) => {
                Ok(texts.contains(text) == matches!(operator, Operator::OneOf))
            }
            _ => Err("one_of and none_of compare text values only".into()),
        },
    }
}

/// `count` and `sum`: one number from all candidates, compared once.
///
/// A `sum` over a candidate whose compared property is absent is not a sum
/// of the model: each such candidate gets the same missing-property finding
/// as elsewhere, and no verdict is drawn from the partial total.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn aggregate(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    object: &Object,
    candidates: &[axioval_ir::ObjectId],
    relation_evidence: &[Evidence],
    config: &Config<'_>,
    evaluation: &mut CapabilityEvaluation,
) {
    let Some(properties) = context.services.get::<PropertyResolutionServiceHandle>() else {
        evaluation.push_object_not_evaluated(
            object.id.clone(),
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered",
        );
        return;
    };
    let mut evidence = relation_evidence.to_vec();
    let (label, left) = if let Quantifier::Count = config.quantifier {
        let Ok(count) = i64::try_from(candidates.len()) else {
            evaluation.push_object_not_evaluated(
                object.id.clone(),
                NotEvaluatedReason::ResourceLimit,
                "candidate count exceeds the integer range",
            );
            return;
        };
        (
            "count of compared components",
            PropertyValue::Integer(count),
        )
    } else {
        let (set, name) = config
            .compared
            .expect("parsing requires a compared property for sum");
        let mut values = Vec::new();
        let mut incomplete = false;
        for candidate_id in candidates {
            let Some(candidate) = context.project.object(candidate_id) else {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    "relationship candidate is absent from project",
                );
                return;
            };
            match resolve(context, properties, candidate, set, name) {
                Ok((Some((property, found)), _)) => {
                    values.push(property.value);
                    evidence.extend(found);
                }
                Ok((None, absence)) => {
                    incomplete = true;
                    evaluation.push_finding(make_finding(
                        rule,
                        candidate,
                        "compared property is absent".into(),
                        combined(relation_evidence, &absence, &[]),
                    ));
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    return;
                }
            }
        }
        if incomplete {
            return;
        }
        match sum(&values) {
            Ok(total) => ("sum of compared values", total),
            Err(message) => {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    message,
                );
                return;
            }
        }
    };
    let target = match target_side(context, properties, object, &config.target) {
        Ok((Some(target), _)) => target,
        Ok((None, absence)) => {
            evaluation.push_finding(make_finding(
                rule,
                object,
                "target property is absent".into(),
                combined(relation_evidence, &absence, &[]),
            ));
            return;
        }
        Err((reason, message)) => {
            evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            return;
        }
    };
    match compare_side(&left, &target, config.factor, config.operator) {
        Ok(true) => {}
        Ok(false) => {
            let operator = match rule.parameters.get("operator") {
                Some(ParameterValue::String { value }) => value.as_str(),
                _ => "operator",
            };
            let factor = if exact_one(config.factor) {
                String::new()
            } else {
                format!("{} x ", config.factor)
            };
            evaluation.push_finding(make_finding(
                rule,
                object,
                format!(
                    "{label} is {} and is not {operator} {factor}{}",
                    crate::support::display(Some(&left)),
                    target.describe()
                ),
                combined(&evidence, target.evidence(), &[]),
            ));
        }
        Err(message) => evaluation.push_object_not_evaluated(
            object.id.clone(),
            NotEvaluatedReason::InvalidEvidence,
            message,
        ),
    }
}

/// The total of exact values of one kind: integers, numbers, or quantities of one dimension.
fn sum(values: &[PropertyValue]) -> Result<PropertyValue, String> {
    if values
        .iter()
        .all(|value| matches!(value, PropertyValue::Integer(_)))
    {
        return values
            .iter()
            .try_fold(0_i64, |total, value| match value {
                PropertyValue::Integer(value) => total.checked_add(*value),
                _ => None,
            })
            .map(PropertyValue::Integer)
            .ok_or_else(|| "integer sum overflows".into());
    }
    if let Some(PropertyValue::Quantity { dimension, .. }) = values.first() {
        let mut total = 0.0;
        for value in values {
            match value {
                PropertyValue::Quantity {
                    value,
                    dimension: other,
                } if other == dimension => total += value,
                _ => return Err("summed quantities differ in dimension or kind".into()),
            }
        }
        return if total.is_finite() {
            Ok(PropertyValue::Quantity {
                value: total,
                dimension: *dimension,
            })
        } else {
            Err("sum is not finite".into())
        };
    }
    let mut total = 0.0;
    for value in values {
        total += match value {
            PropertyValue::Decimal(value) => *value,
            PropertyValue::Integer(value) => integer_to_f64(*value)?,
            _ => return Err("summed values are not all numbers".into()),
        };
    }
    if total.is_finite() {
        Ok(PropertyValue::Decimal(total))
    } else {
        Err("sum is not finite".into())
    }
}

type Resolved = (axioval_ir::Property, Vec<Evidence>);
fn resolve(
    context: &RuleContext<'_>,
    service: &PropertyResolutionServiceHandle,
    object: &Object,
    set: Option<&str>,
    name: &str,
) -> Result<(Option<Resolved>, Vec<Evidence>), (NotEvaluatedReason, String)> {
    let request = bound_property_request(context, object, set, name)?;
    match service.resolve(&request) {
        Ok(PropertyResolution::Present(value)) => {
            let property = value.property().clone();
            Ok((
                Some((property.clone(), property.evidence.into_iter().collect())),
                Vec::new(),
            ))
        }
        Ok(PropertyResolution::Absent(proof)) => Ok((None, vec![proof.evidence().clone()])),
        Err(error) => Err(property_error(error)),
    }
}
fn relationship_selection(
    context: &RuleContext<'_>,
    object: &Object,
    universe: &[&Object],
    config: &Config<'_>,
) -> Result<(Vec<axioval_ir::ObjectId>, Vec<Evidence>), (NotEvaluatedReason, String)> {
    let Some(service) = context.services.get::<RelationshipSelectionServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "relationship-selection service is not registered".into(),
        ));
    };
    let relationship = SemanticRelationship::try_new(config.relationship.unwrap_or_default())
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    let query = match config.mode {
        Mode::Shared => RelationshipQuery::SharedGroup { relationship },
        Mode::Related => RelationshipQuery::Related {
            relationship,
            direction: config.direction,
            follow_chain: config.follow_chain,
        },
        Mode::Checked => unreachable!(),
    };
    let request = RelationshipSelectionRequest::try_new(
        object.id.clone(),
        universe.iter().map(|item| item.id.clone()).collect(),
        query,
    )
    .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?
    .with_absent_ends(config.absent_ends);
    service
        .select(&request)
        .map(|selection| {
            (
                selection.candidates().to_vec(),
                selection.evidence().to_vec(),
            )
        })
        .map_err(|error| match error {
            RelationshipSelectionError::Unavailable(message) => {
                (NotEvaluatedReason::BackendUnavailable, message)
            }
            other => (NotEvaluatedReason::InvalidEvidence, other.to_string()),
        })
}
fn compare(
    left: &PropertyValue,
    right: &PropertyValue,
    factor: f64,
    operator: Operator,
) -> Result<bool, String> {
    let equal = |ord: Ordering| match operator {
        Operator::Equals => ord.is_eq(),
        Operator::NotEquals => !ord.is_eq(),
        Operator::Greater => ord.is_gt(),
        Operator::GreaterOrEqual => ord.is_ge(),
        Operator::Less => ord.is_lt(),
        Operator::LessOrEqual => ord.is_le(),
        Operator::Contains | Operator::OneOf | Operator::NoneOf => false,
    };
    match (left, right) {
        (PropertyValue::Boolean(a), PropertyValue::Boolean(b)) if exact_one(factor) => {
            match operator {
                Operator::Equals => Ok(a == b),
                Operator::NotEquals => Ok(a != b),
                _ => Err("boolean comparison operator is invalid".into()),
            }
        }
        (PropertyValue::String(a), PropertyValue::String(b)) if exact_one(factor) => match operator
        {
            Operator::Equals => Ok(a == b),
            Operator::NotEquals => Ok(a != b),
            Operator::Contains => Ok(a.contains(b)),
            _ => Err("string comparison operator is invalid".into()),
        },
        (PropertyValue::Integer(a), PropertyValue::Integer(b)) if exact_one(factor) => {
            Ok(equal(a.cmp(b)))
        }
        (
            PropertyValue::Quantity {
                value: a,
                dimension: da,
            },
            PropertyValue::Quantity {
                value: b,
                dimension: db,
            },
        ) if da == db => {
            if a.is_finite() && b.is_finite() {
                numeric(*a, *b, factor, equal)
            } else {
                Err("quantity value is non-finite".into())
            }
        }
        (PropertyValue::Integer(a), PropertyValue::Decimal(b))
            if (*a).unsigned_abs() <= (1_u64 << 53) && b.is_finite() =>
        {
            numeric(integer_to_f64(*a)?, *b, factor, equal)
        }
        (PropertyValue::Decimal(a), PropertyValue::Integer(b))
            if (*b).unsigned_abs() <= (1_u64 << 53) && a.is_finite() =>
        {
            numeric(*a, integer_to_f64(*b)?, factor, equal)
        }
        (PropertyValue::Decimal(a), PropertyValue::Decimal(b))
            if a.is_finite() && b.is_finite() =>
        {
            numeric(*a, *b, factor, equal)
        }
        (PropertyValue::Integer(_), PropertyValue::Integer(_)) => {
            Err("integer factor cannot be represented exactly".into())
        }
        _ => Err("property values have incompatible types or dimensions".into()),
    }
}
fn numeric(
    left: f64,
    right: f64,
    factor: f64,
    predicate: impl FnOnce(Ordering) -> bool,
) -> Result<bool, String> {
    let scaled = right * factor;
    if scaled.is_finite() {
        Ok(predicate(left.total_cmp(&scaled)))
    } else {
        Err("scaled target is non-finite".into())
    }
}
fn exact_one(value: f64) -> bool {
    value.to_bits() == 1.0_f64.to_bits()
}
fn integer_to_f64(value: i64) -> Result<f64, String> {
    if value.unsigned_abs() > (1_u64 << 53) {
        return Err("integer cannot be represented exactly as a decimal".into());
    }
    #[allow(clippy::cast_precision_loss)]
    Ok(value as f64)
}
fn combined(parts: &[Evidence], left: &[Evidence], right: &[Evidence]) -> Vec<Evidence> {
    let mut values = parts
        .iter()
        .chain(left)
        .chain(right)
        .cloned()
        .collect::<Vec<_>>();
    values.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
    values.dedup();
    values
}
fn make_finding(
    rule: &CompiledRule,
    object: &Object,
    message: String,
    evidence: Vec<Evidence>,
) -> Finding {
    Finding {
        rule_id: rule.id.clone(),
        object_id: object.id.clone(),
        related: Vec::new(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
    }
}
