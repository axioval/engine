//! Exact, source-neutral property-to-property comparison capability.

use std::cmp::Ordering;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionServiceHandle,
    RuleCapability, RuleContext, SemanticRelationship, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Finding, Object, PropertyValue, Severity};

use crate::selection::{property_error, select_objects};

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
}

impl RuleCapability for PropertyComparison {
    fn id(&self) -> &'static str {
        "axioval:capability.property-comparison"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("compared_selector", ParameterType::Selector),
            ParameterDescriptor::required("compared_property", ParameterType::PropertyReference),
            ParameterDescriptor::required("target_property", ParameterType::PropertyReference),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::required("factor", ParameterType::Number),
            ParameterDescriptor::required("component_mode", ParameterType::String),
            ParameterDescriptor::optional("relationship", ParameterType::String),
            ParameterDescriptor::optional("direction", ParameterType::String),
            ParameterDescriptor::optional("follow_chain", ParameterType::Boolean),
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
            let target = resolve(properties, object, config.target_set, config.target_name);
            let target = match target {
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
                match resolve(
                    properties,
                    candidate,
                    config.compared_set,
                    config.compared_name,
                ) {
                    Ok((Some(compared), _)) => match compare(
                        &compared.0.value,
                        &target.0.value,
                        config.factor,
                        config.operator,
                    ) {
                        Ok(true) => any_match = true,
                        Ok(false) => mismatches.push((
                            candidate,
                            combined(&relation_evidence, &compared.1, &target.1),
                        )),
                        Err(message) => {
                            uncertainties.push((NotEvaluatedReason::InvalidEvidence, message));
                        }
                    },
                    Ok((None, absence_evidence)) => missing.push((
                        candidate,
                        combined(&relation_evidence, &absence_evidence, &target.1),
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
                Quantifier::AtLeastOne if any_match || has_missing_information => {}
                Quantifier::AtLeastOne => {
                    if uncertainties.is_empty() {
                        let mut evidence = combined(&relation_evidence, &target.1, &[]);
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
    compared_set: Option<&'a str>,
    compared_name: &'a str,
    target_set: Option<&'a str>,
    target_name: &'a str,
    operator: Operator,
    factor: f64,
    mode: Mode,
    relationship: Option<&'a str>,
    direction: TraversalDirection,
    follow_chain: bool,
    quantifier: Quantifier,
}
impl<'a> Config<'a> {
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
        let (compared_set, compared_name) = property("compared_property")?;
        let (target_set, target_name) = property("target_property")?;
        let string = |name| match rule.parameters.get(name)? {
            ParameterValue::String { value } => Some(value.as_str()),
            _ => None,
        };
        let operator = match string("operator")? {
            "equals" => Operator::Equals,
            "not_equals" => Operator::NotEquals,
            "greater" => Operator::Greater,
            "greater_or_equal" => Operator::GreaterOrEqual,
            "less" => Operator::Less,
            "less_or_equal" => Operator::LessOrEqual,
            "contains" => Operator::Contains,
            _ => return None,
        };
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
        let quantifier = match string("quantifier")? {
            "each" => Quantifier::Each,
            "at_least_one" => Quantifier::AtLeastOne,
            _ => return None,
        };
        Some(Self {
            selector,
            compared_set,
            compared_name,
            target_set,
            target_name,
            operator,
            factor,
            mode,
            relationship,
            direction,
            follow_chain,
            quantifier,
        })
    }
}

type Resolved = (axioval_ir::Property, Vec<Evidence>);
fn resolve(
    service: &PropertyResolutionServiceHandle,
    object: &Object,
    set: Option<&str>,
    name: &str,
) -> Result<(Option<Resolved>, Vec<Evidence>), (NotEvaluatedReason, String)> {
    let request = PropertyRequest::try_new(object.id.clone(), set.map(str::to_owned), name)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
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
    .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
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
        Operator::Contains => false,
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
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
    }
}
