//! Deterministic, fail-closed selector evaluation.

use std::collections::BTreeMap;

use axioval_engine::{
    BindingError, CapabilityEvaluation, CapabilityRegistry, ClassificationError,
    ClassificationServiceHandle, CompiledRule, ConceptBindings, NotEvaluatedReason,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
    RuleContext, TypeHierarchyError, TypeHierarchyServiceHandle,
};
use axioval_ir::contract::{
    ComparisonOperator, ParameterValue, Selector, Severity as RuleSeverity,
};
use axioval_ir::{Object, Project, Property, PropertyValue, RuleId};
use regex::Regex;

pub(crate) fn select_objects<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let mut selected = Vec::new();
    let mut evaluation = CapabilityEvaluation::default();
    for object in context.project.objects() {
        match selector_matches(context, selector, object) {
            Selection::Match => selected.push(object),
            Selection::NoMatch => {}
            Selection::NotEvaluated(reason, message) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    (selected, evaluation)
}

#[derive(Clone, Debug)]
enum Selection {
    Match,
    NoMatch,
    NotEvaluated(NotEvaluatedReason, String),
}

fn selector_matches(context: &RuleContext<'_>, selector: &Selector, object: &Object) -> Selection {
    match selector {
        Selector::All => Selection::Match,
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => entity_type_matches(context, object, object_type, *include_subtypes),
        Selector::Classification {
            system,
            code,
            include_descendants,
        } => classification_matches(context, object, system, code, *include_descendants),
        Selector::AllOf { operands } => all_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object)),
        ),
        Selector::AnyOf { operands } => any_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object)),
        ),
        Selector::Not { operand } => match selector_matches(context, operand, object) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
        Selector::Meets {
            capability,
            parameters,
        } => meets(context, object, capability, parameters),
        Selector::Property {
            property_set,
            property,
            operator,
            value,
        } => property_selector_matches(
            context,
            object,
            property_set.as_deref(),
            property,
            operator,
            value.as_ref(),
        ),
    }
}

/// Whether `object` meets a selectable capability's requirement on its own.
///
/// The capability is evaluated over a project of just this object: any
/// finding is a non-match, any undecided outcome leaves membership
/// undecided, and only a clean evaluation is a match.
fn meets(
    context: &RuleContext<'_>,
    object: &Object,
    capability: &str,
    parameters: &BTreeMap<String, ParameterValue>,
) -> Selection {
    let Some(registry) = context.services.get::<CapabilityRegistry>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "no capability registry is available to evaluate a meets selector".into(),
        );
    };
    let Some(trusted) = registry
        .get(capability)
        .filter(|trusted| trusted.selectable())
    else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::InvalidDeclaration,
            format!("`{capability}` is not a registered selectable capability"),
        );
    };
    let project = match Project::new(vec![object.clone()]) {
        Ok(project) => project,
        Err(error) => {
            return Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, error.to_string());
        }
    };
    let Ok(id) = RuleId::new("meets") else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::InvalidDeclaration,
            "selector rule id".into(),
        );
    };
    let rule = CompiledRule {
        id,
        capability: capability.to_owned(),
        severity: RuleSeverity::Error,
        selector: Selector::All,
        parameters: parameters.clone(),
    };
    let evaluation = trusted.evaluate(
        &RuleContext {
            project: &project,
            services: context.services,
        },
        &rule,
    );
    if !evaluation.findings().is_empty() || !evaluation.rule_findings().is_empty() {
        Selection::NoMatch
    } else if let Some(outcome) = evaluation.not_evaluated_outcomes().first() {
        Selection::NotEvaluated(outcome.reason().clone(), outcome.message().to_owned())
    } else {
        Selection::Match
    }
}

/// Vocabulary the names in a rule are written in.
///
/// A plan compiled from packages always runs with [`ConceptBindings`]: its
/// names are canonical concepts and must be translated per source. A trusted
/// host that evaluates a capability directly, without bindings, writes rules
/// in its own source vocabulary, and those names are used verbatim.
enum Vocabulary<'a> {
    Package(&'a ConceptBindings),
    Native,
}

fn vocabulary<'a>(context: &RuleContext<'a>) -> Vocabulary<'a> {
    context
        .services
        .get::<ConceptBindings>()
        .map_or(Vocabulary::Native, Vocabulary::Package)
}

fn binding_error(error: &BindingError) -> (NotEvaluatedReason, String) {
    // An unbound concept is a property of the package/source pairing, not of
    // the evidence: the package names nothing this source can express. A
    // concept no package declares is a broken declaration instead.
    let reason = match error {
        BindingError::UnknownConcept { .. } => NotEvaluatedReason::InvalidDeclaration,
        _ => NotEvaluatedReason::UnboundConcept,
    };
    (reason, error.to_string())
}

/// Builds a property request in the checked object's own source vocabulary.
///
/// Package rules reference canonical concepts, which are bound through the
/// object's declared type system first, for both the property and its
/// optional set qualifier. An unbindable concept is never passed through
/// verbatim: a source asked for a name it does not use would answer "absent"
/// and turn a vocabulary gap into a violation.
pub(crate) fn bound_property_request(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<&str>,
    name: &str,
) -> Result<PropertyRequest, (NotEvaluatedReason, String)> {
    let (property_set, property) = match vocabulary(context) {
        Vocabulary::Native => (set.map(ToOwned::to_owned), name),
        Vocabulary::Package(bindings) => {
            let source = &object.id.source;
            let property = bindings
                .property(name, source)
                .map_err(|error| binding_error(&error))?;
            let property_set = set
                .map(|set| bindings.property_set(set, source).map(ToOwned::to_owned))
                .transpose()
                .map_err(|error| binding_error(&error))?;
            (property_set, property)
        }
    };
    PropertyRequest::try_new(object.id.clone(), property_set, property)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))
}

/// A property concept's name in the checked object's source vocabulary.
///
/// Used for names that are not looked up in property sets, such as direct
/// attributes. As for property requests, an unbindable concept is never
/// passed through verbatim.
pub(crate) fn bound_name(
    context: &RuleContext<'_>,
    object: &Object,
    concept: &str,
) -> Result<String, (NotEvaluatedReason, String)> {
    match vocabulary(context) {
        Vocabulary::Native => Ok(concept.to_owned()),
        Vocabulary::Package(bindings) => bindings
            .property(concept, &object.id.source)
            .map(ToOwned::to_owned)
            .map_err(|error| binding_error(&error)),
    }
}

/// Whether an object is an instance of an object type.
///
/// A kind always matches itself. Beyond that, `include_subtypes` needs the
/// source's own type hierarchy; the engine knows none, so without that service
/// a different kind is unknown rather than "no". Before this, subtypes were
/// ignored and every subtype instance was silently left out of scope.
fn entity_type_matches(
    context: &RuleContext<'_>,
    object: &Object,
    object_type: &str,
    include_subtypes: bool,
) -> Selection {
    let name = match vocabulary(context) {
        Vocabulary::Native => object_type,
        Vocabulary::Package(bindings) => match bindings.object_type(object_type, &object.id.source)
        {
            Ok(name) => name,
            Err(error) => {
                let (reason, message) = binding_error(&error);
                return Selection::NotEvaluated(reason, message);
            }
        },
    };
    // Source kinds are compared case-insensitively: STEP writes `IFCWALL` for
    // the schema's `IfcWall`, and both name the same entity.
    if object.kind().eq_ignore_ascii_case(name) {
        return Selection::Match;
    }
    if !include_subtypes {
        return Selection::NoMatch;
    }
    let Some(hierarchy) = context.services.get::<TypeHierarchyServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "type-hierarchy service is not registered; subtype membership is unknown".into(),
        );
    };
    match hierarchy.is_a(&object.id.source, object.kind(), name) {
        Ok(matches) => verdict(matches),
        Err(TypeHierarchyError::UnknownType(message)) => {
            Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, message)
        }
        Err(error) => {
            Selection::NotEvaluated(NotEvaluatedReason::BackendUnavailable, error.to_string())
        }
    }
}

fn verdict(value: bool) -> Selection {
    if value {
        Selection::Match
    } else {
        Selection::NoMatch
    }
}

fn all_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::NoMatch => return Selection::NoMatch,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::Match)
}

fn any_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::Match => return Selection::Match,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::NoMatch)
}

/// Whether `object` carries `code` in `system`, as its source states.
///
/// The project's inline classification list is not consulted: an empty list
/// cannot tell "unclassified" from "never read", and treating the second as
/// the first made every classification rule over an IFC model select nothing
/// and pass. A source with no classification service is not evaluated.
fn classification_matches(
    context: &RuleContext<'_>,
    object: &Object,
    system: &str,
    code: &str,
    include_descendants: bool,
) -> Selection {
    let Some(service) = context.services.get::<ClassificationServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "classification service is not registered; classifications are unknown".into(),
        );
    };
    let assignments = match service.classifications(&object.id) {
        Ok(assignments) => assignments,
        Err(error @ ClassificationError::Unreadable(_)) => {
            return Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, error.to_string());
        }
        Err(error) => {
            return Selection::NotEvaluated(
                NotEvaluatedReason::BackendUnavailable,
                error.to_string(),
            );
        }
    };
    let mut undecided = false;
    for assignment in &assignments {
        match assignment.matches(system, code, include_descendants) {
            Some(true) => return Selection::Match,
            Some(false) => {}
            None => undecided = true,
        }
    }
    if undecided {
        Selection::NotEvaluated(
            NotEvaluatedReason::IncompleteEvidence,
            "a classification of this object is not linked to a system".into(),
        )
    } else {
        Selection::NoMatch
    }
}

fn property_selector_matches(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<&str>,
    name: &str,
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
) -> Selection {
    if let Some(message) = selector_declaration_error(operator, expected) {
        return Selection::NotEvaluated(NotEvaluatedReason::InvalidDeclaration, message);
    }
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        );
    };
    let request = match bound_property_request(context, object, set, name) {
        Ok(request) => request,
        Err((reason, message)) => return Selection::NotEvaluated(reason, message),
    };
    match service.resolve(&request) {
        Ok(PropertyResolution::Absent(_)) => Selection::NoMatch,
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            if matches!(operator, ComparisonOperator::Exists) {
                Selection::Match
            } else if let Some(expected) = expected {
                match compare_property(property, operator, expected) {
                    Ok(matches) => verdict(matches),
                    Err(message) => {
                        Selection::NotEvaluated(NotEvaluatedReason::InvalidDeclaration, message)
                    }
                }
            } else {
                Selection::NotEvaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    "property selector comparison has no expected value".into(),
                )
            }
        }
        Err(error) => unavailable(error),
    }
}

fn selector_declaration_error(
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
) -> Option<String> {
    match (operator, expected) {
        (ComparisonOperator::Exists, Some(_)) => {
            Some("property selector exists operator must not have a value".into())
        }
        (ComparisonOperator::Exists, None)
        | (ComparisonOperator::Matches, Some(ParameterValue::String { .. })) => None,
        (_, None) => Some("property selector comparison has no expected value".into()),
        (ComparisonOperator::Matches, Some(_)) => {
            Some("property selector regex must be a string".into())
        }
        (_, Some(_)) => None,
    }
}

pub(crate) fn property_error(error: PropertyResolutionError) -> (NotEvaluatedReason, String) {
    match error {
        PropertyResolutionError::Unavailable(message) => {
            (NotEvaluatedReason::BackendUnavailable, message)
        }
        PropertyResolutionError::Incomplete(message) => {
            (NotEvaluatedReason::IncompleteEvidence, message)
        }
        error => (NotEvaluatedReason::InvalidEvidence, error.to_string()),
    }
}

fn unavailable(error: PropertyResolutionError) -> Selection {
    let (reason, message) = property_error(error);
    Selection::NotEvaluated(reason, message)
}

fn compare_property(
    property: &Property,
    operator: &ComparisonOperator,
    expected: &ParameterValue,
) -> Result<bool, String> {
    Ok(match operator {
        ComparisonOperator::Equals => values_equal(&property.value, expected).unwrap_or(false),
        ComparisonOperator::NotEquals => {
            values_equal(&property.value, expected).is_some_and(|equal| !equal)
        }
        ComparisonOperator::LessThan => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_lt)
        }
        ComparisonOperator::LessThanOrEquals => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_le)
        }
        ComparisonOperator::GreaterThan => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_gt)
        }
        ComparisonOperator::GreaterThanOrEquals => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_ge)
        }
        ComparisonOperator::Matches => match (&property.value, expected) {
            (PropertyValue::String(actual), ParameterValue::String { value }) => Regex::new(value)
                .map_err(|error| format!("invalid property selector regex: {error}"))?
                .is_match(actual),
            _ => false,
        },
        ComparisonOperator::Exists => true,
    })
}

fn values_equal(actual: &PropertyValue, expected: &ParameterValue) -> Option<bool> {
    match (actual, expected) {
        (PropertyValue::Boolean(actual), ParameterValue::Boolean { value }) => {
            Some(actual == value)
        }
        (PropertyValue::Integer(actual), ParameterValue::Integer { value }) => {
            Some(actual == value)
        }
        (PropertyValue::Decimal(actual), ParameterValue::Number { value }) => {
            Some(actual.total_cmp(value).is_eq())
        }
        (
            PropertyValue::String(actual),
            ParameterValue::String { value }
            | ParameterValue::Enum { value }
            | ParameterValue::Reference { value },
        ) => Some(actual == value),
        _ => None,
    }
}

fn ordered(actual: &PropertyValue, expected: &ParameterValue) -> Option<std::cmp::Ordering> {
    match (actual, expected) {
        (PropertyValue::Integer(actual), ParameterValue::Integer { value }) => {
            Some(actual.cmp(value))
        }
        (PropertyValue::Decimal(actual), ParameterValue::Number { value }) => {
            actual.partial_cmp(value)
        }
        (PropertyValue::String(actual), ParameterValue::String { value }) => {
            Some(actual.cmp(value))
        }
        _ => None,
    }
}
