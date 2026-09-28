//! Deterministic, fail-closed selector evaluation.

use axioval_engine::{
    BindingError, CapabilityEvaluation, ClassificationAssignment, ClassificationError,
    ClassificationServiceHandle, ConceptBindings, NameMatch, NamePattern, NotEvaluatedReason,
    PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionServiceHandle, RuleContext, RuleOutcomes,
    SourceDisciplines, SourceMetadataIndex, TypeHierarchyError, TypeHierarchyServiceHandle,
};
use axioval_ir::contract::{
    ComparisonOperator, ParameterValue, Quantifier, RelatedQuantifier, RuleOutcomeKind, Selector,
};
use axioval_ir::{
    Date, DateTime, Discipline, Evidence, Object, PropertyValue, QuantityDimension,
    TemporalPrecision,
};
use regex::{Regex, RegexBuilder};
use std::cmp::Ordering;

use crate::support::{Tolerance, Traversal, exact_f64, si_quantity, temporal_order, undefined};

pub(crate) fn select_objects<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let mut selected = Vec::new();
    let mut evaluation = CapabilityEvaluation::default();
    for object in context.project.objects() {
        match selector_matches(context, selector, object, &mut Vec::new()) {
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
pub(crate) enum Selection {
    Match,
    NoMatch,
    NotEvaluated(NotEvaluatedReason, String),
}

/// Whether `object` is selected; property facts consulted are added to `evidence`.
pub(crate) fn selector_matches(
    context: &RuleContext<'_>,
    selector: &Selector,
    object: &Object,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    match selector {
        Selector::All => Selection::Match,
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => entity_type_matches(context, object, object_type, *include_subtypes),
        Selector::Classification {
            system,
            code,
            code_pattern,
            include_descendants,
        } => classification_matches(
            context,
            object,
            system,
            (
                code.as_deref(),
                code_pattern.as_deref(),
                *include_descendants,
            ),
        ),
        Selector::AllOf { operands } => all_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object, evidence)),
        ),
        Selector::AnyOf { operands } => any_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object, evidence)),
        ),
        Selector::Not { operand } => match selector_matches(context, operand, object, evidence) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
        Selector::Property {
            property_set,
            property,
            operator,
            value,
            case_sensitive,
            trim,
            quantifier,
            precision,
        } => property_selector_matches(
            context,
            object,
            property_set.as_deref(),
            property,
            (
                operator,
                value.as_ref(),
                TextOptions {
                    case_sensitive: *case_sensitive,
                    trim: *trim,
                },
                *quantifier,
                *precision,
            ),
            evidence,
        ),
        Selector::PropertyPattern {
            property_set_pattern,
            property_pattern,
            matched,
            operator,
            value,
            case_sensitive,
            trim,
            quantifier,
            precision,
        } => property_pattern_matches(
            context,
            object,
            (property_set_pattern.as_deref(), property_pattern, *matched),
            (
                operator,
                value.as_ref(),
                TextOptions {
                    case_sensitive: *case_sensitive,
                    trim: *trim,
                },
                *quantifier,
                *precision,
            ),
            evidence,
        ),
        Selector::Related {
            path,
            quantifier,
            selector,
        } => related_matches(context, object, path, *quantifier, selector, evidence),
        Selector::Discipline { value } => discipline_matches(context, object, value, evidence),
        source @ Selector::Source { .. } => source_matches(context, object, source),
        Selector::RuleOutcome { rule, outcome } => {
            rule_outcome_matches(context, object, rule, *outcome)
        }
    }
}

/// Whether the rule `rule` judged `object` as `outcome` asks, from the
/// outcomes the runtime recorded.
fn rule_outcome_matches(
    context: &RuleContext<'_>,
    object: &Object,
    rule: &str,
    outcome: RuleOutcomeKind,
) -> Selection {
    let Some(outcomes) = context.services.get::<RuleOutcomes>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "no rule outcomes are available outside a run".into(),
        );
    };
    match outcomes.selects(rule, outcome, object) {
        Ok(matches) => verdict(matches),
        Err((reason, message)) => Selection::NotEvaluated(reason, message),
    }
}

/// Whether `object`'s source metadata `field` satisfies the comparison.
///
/// The field is compared as a property's value: one value as a scalar,
/// several as a list (needing `quantifier`), and a field read and found to
/// hold nothing as an absent property, which matches nothing. A field never
/// read is `NotRecorded`, with a message naming the source only, so the
/// runtime reports it once per rule and source.
fn source_matches(context: &RuleContext<'_>, object: &Object, selector: &Selector) -> Selection {
    let Selector::Source {
        field,
        operator,
        value: expected,
        case_sensitive,
        trim,
        quantifier,
    } = selector
    else {
        unreachable!("only `source` selectors are matched here");
    };
    let (field, expected, quantifier) = (*field, expected.as_ref(), *quantifier);
    let options = TextOptions {
        case_sensitive: *case_sensitive,
        trim: *trim,
    };
    let test = match selector_test(operator, expected, options, quantifier, None) {
        Ok(test) => test,
        Err(Selection::NotEvaluated(reason, message)) => {
            return Selection::NotEvaluated(
                reason,
                message.replacen("property selector", "source selector", 1),
            );
        }
        Err(other) => return other,
    };
    let Some(index) = context.services.get::<SourceMetadataIndex>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "source metadata is not available outside an evidence session".into(),
        );
    };
    let source = &object.id.source;
    let Some(values) = index.values(source, field) else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::NotRecorded,
            format!(
                "source `{source}` does not record its {}, so the `source` selector cannot decide",
                field.as_str()
            ),
        );
    };
    let value = match values {
        [] => return Selection::NoMatch,
        [one] => PropertyValue::String(one.clone()),
        several => PropertyValue::List(
            several
                .iter()
                .map(|value| PropertyValue::String(value.clone()))
                .collect(),
        ),
    };
    match test.holds_quantified(&value, quantifier, options) {
        Ok(matches) => verdict(matches),
        Err(message) => Selection::NotEvaluated(
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "source selector on the {} of `{source}`: {message}",
                field.as_str()
            ),
        ),
    }
}

/// Whether `object`'s source plays `discipline`.
///
/// The discipline is the session's declaration about the source. A source
/// that declares none is unknown, reported once per source (`NotRecorded`
/// with a source-only message), never a non-match: otherwise a rule scoped to
/// a discipline would pass over a model nobody classified.
fn discipline_matches(
    context: &RuleContext<'_>,
    object: &Object,
    discipline: &Discipline,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    match discipline_of(context, object, "the `discipline` selector") {
        Ok(declared) if declared == discipline => {
            // A mapped discipline cites the map rule and the value it matched:
            // the host's assignment, not a fact the source states.
            if let Some(locator) = context
                .services
                .get::<SourceDisciplines>()
                .and_then(|disciplines| disciplines.origin(&object.id.source))
                .and_then(axioval_engine::DisciplineOrigin::locator)
            {
                evidence.push(Evidence {
                    source: object.id.source.clone(),
                    locator,
                    exact: false,
                });
            }
            Selection::Match
        }
        Ok(_) => Selection::NoMatch,
        Err((reason, message)) => Selection::NotEvaluated(reason, message),
    }
}

/// The discipline `object`'s source plays, for `reader` (named in the
/// message when none is declared).
///
/// A source that declares none is `NotRecorded`, with a message naming the
/// source only, so the runtime reports it once per rule and source.
pub(crate) fn discipline_of<'c>(
    context: &RuleContext<'c>,
    object: &Object,
    reader: &str,
) -> Result<&'c Discipline, (NotEvaluatedReason, String)> {
    let Some(disciplines) = context.services.get::<SourceDisciplines>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "source disciplines are not available outside an evidence session".into(),
        ));
    };
    disciplines.of(&object.id.source).ok_or_else(|| {
        (
            NotEvaluatedReason::NotRecorded,
            format!(
                "source `{}` declares no discipline, so {reader} cannot decide",
                object.id.source
            ),
        )
    })
}

/// Whether the objects `path` reaches from `object` satisfy `selector`
/// under `quantifier`.
///
/// A refused relationship answer leaves the object undecided: the objects
/// it would have reached are unknown. So does a reached object `selector`
/// cannot decide, unless the others already settle the verdict.
fn related_matches(
    context: &RuleContext<'_>,
    object: &Object,
    path: &[String],
    quantifier: RelatedQuantifier,
    selector: &Selector,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    let traversal = match Traversal::path(path) {
        Ok(traversal) => traversal,
        Err((reason, message)) => {
            return Selection::NotEvaluated(reason, format!("related selector: {message}"));
        }
    };
    let everything: Vec<&Object> = context.project.objects().collect();
    let (reached, cited) = match traversal.related(context, &object.id, &everything) {
        Ok(found) => found,
        Err((reason, message)) => {
            return Selection::NotEvaluated(
                reason,
                format!("related selector via {}: {message}", traversal.relationship),
            );
        }
    };
    evidence.extend(cited);
    let outcomes = reached.iter().map(|id| match context.project.object(id) {
        Some(target) => selector_matches(context, selector, target, evidence),
        None => Selection::NotEvaluated(
            NotEvaluatedReason::InvalidEvidence,
            format!("related selector reached {id}, which is not in the project"),
        ),
    });
    match quantifier {
        RelatedQuantifier::Any => any_of(outcomes),
        // `all` never holds vacuously.
        RelatedQuantifier::All if reached.is_empty() => Selection::NoMatch,
        RelatedQuantifier::All => all_of(outcomes),
        RelatedQuantifier::None => match any_of(outcomes) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
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
    // A derived set's names are the engine's or the ruleset's own, the same
    // in every source.
    if let Some(derived) = set.filter(|set| axioval_ir::is_derived_set(set)) {
        return PropertyRequest::try_new(object.id.clone(), Some(derived.to_owned()), name)
            .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()));
    }
    let (property_set, property) = match vocabulary(context) {
        Vocabulary::Native => (set.map(ToOwned::to_owned), name),
        Vocabulary::Package(bindings) => {
            let source = &object.id.source;
            let property = bindings
                .property(name, source)
                .map_err(|error| binding_error(&error))?;
            // The attribute sets are engine vocabulary with one meaning in
            // every source, so they bind to themselves.
            let property_set = set
                .map(|set| {
                    if axioval_ir::is_reserved_set(set) {
                        Ok(set.to_owned())
                    } else {
                        bindings.property_set(set, source).map(ToOwned::to_owned)
                    }
                })
                .transpose()
                .map_err(|error| binding_error(&error))?;
            (property_set, property)
        }
    };
    PropertyRequest::try_new(object.id.clone(), property_set, property)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))
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
    (code, pattern, include_descendants): (Option<&str>, Option<&str>, bool),
) -> Selection {
    let test = match CodeTest::parse(code, pattern, include_descendants) {
        Ok(test) => test,
        Err(message) => {
            return Selection::NotEvaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("classification selector: {message}"),
            );
        }
    };
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
        match test.matches(assignment, system) {
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

/// What a classification selector asks of an assignment's code.
enum CodeTest {
    /// Any code: the system alone.
    Any,
    /// One code exactly.
    Exact(String, bool),
    /// Codes matching an XML Schema pattern, whole.
    Pattern(Regex, bool),
}

impl CodeTest {
    fn parse(
        code: Option<&str>,
        pattern: Option<&str>,
        include_descendants: bool,
    ) -> Result<Self, String> {
        match (code, pattern) {
            (Some(_), Some(_)) => Err("`code` and `codePattern` exclude each other".into()),
            (Some(code), None) => Ok(Self::Exact(code.to_owned(), include_descendants)),
            (None, Some(pattern)) => crate::xsd_pattern::compile(pattern)
                .map(|regex| Self::Pattern(regex, include_descendants))
                .map_err(|error| format!("code pattern {pattern:?}: {error}")),
            (None, None) if include_descendants => {
                Err("`includeDescendants` needs a `code` or a `codePattern`".into())
            }
            (None, None) => Ok(Self::Any),
        }
    }

    /// Whether `assignment` meets the test in `system`; `None` when its
    /// system is unknown.
    fn matches(&self, assignment: &ClassificationAssignment, system: &str) -> Option<bool> {
        match self {
            Self::Any => assignment.in_system(system),
            Self::Exact(code, descendants) => assignment.matches(system, code, *descendants),
            Self::Pattern(regex, descendants) => {
                assignment.matches_code(system, |code| regex.is_match(code), *descendants)
            }
        }
    }
}

/// How a property selector compares text: case folding and trimming.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) trim: bool,
}

impl TextOptions {
    fn is_default(self) -> bool {
        self.case_sensitive && !self.trim
    }

    fn fold(self, text: &str) -> String {
        if self.case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    }

    /// The resolved value as compared: trimmed, then folded, as declared.
    fn prepare(self, text: &str) -> String {
        self.fold(if self.trim { text.trim() } else { text })
    }
}

fn property_selector_matches(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<&str>,
    name: &str,
    test: (
        &ComparisonOperator,
        Option<&ParameterValue>,
        TextOptions,
        Option<Quantifier>,
        Option<TemporalPrecision>,
    ),
    evidence: &mut Vec<Evidence>,
) -> Selection {
    let (operator, expected, options, quantifier, precision) = test;
    let test = match selector_test(operator, expected, options, quantifier, precision) {
        Ok(test) => test,
        Err(invalid) => return invalid,
    };
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
        Ok(PropertyResolution::Absent(proof)) => {
            evidence.push(proof.evidence().clone());
            Selection::NoMatch
        }
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            evidence.extend(property.evidence.iter().cloned());
            match test.holds_quantified(&property.value, quantifier, options) {
                Ok(matches) => verdict(matches),
                Err(message) => Selection::NotEvaluated(
                    crate::support::undecided_reason(&[&property.value]),
                    format!("property selector on `{name}`: {message}"),
                ),
            }
        }
        Err(error) => unavailable(error),
    }
}

/// The comparison a property selector states, or why it is invalid.
fn selector_test(
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
    options: TextOptions,
    quantifier: Option<Quantifier>,
    precision: Option<TemporalPrecision>,
) -> Result<Test, Selection> {
    Test::parse(operator, expected, options, precision)
        .and_then(|test| {
            if quantifier.is_some() && test.judges_presence() {
                Err("`quantifier` applies to value comparisons, not to presence".into())
            } else {
                Ok(test)
            }
        })
        .map_err(|message| {
            Selection::NotEvaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("property selector: {message}"),
            )
        })
}

/// A `propertyPattern` selector: the properties whose set and name match,
/// enumerated exactly, compared under `matched`.
fn property_pattern_matches(
    context: &RuleContext<'_>,
    object: &Object,
    patterns: (Option<&str>, &str, Quantifier),
    test: (
        &ComparisonOperator,
        Option<&ParameterValue>,
        TextOptions,
        Option<Quantifier>,
        Option<TemporalPrecision>,
    ),
    evidence: &mut Vec<Evidence>,
) -> Selection {
    let (set_pattern, name_pattern, matched) = patterns;
    let (operator, expected, options, quantifier, precision) = test;
    let test = match selector_test(operator, expected, options, quantifier, precision) {
        Ok(test) => test,
        Err(invalid) => return invalid,
    };
    let compile = |pattern: &str| {
        xsd_name_pattern(pattern).map_err(|message| {
            Selection::NotEvaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("property selector: name pattern {pattern:?}: {message}"),
            )
        })
    };
    let set = match set_pattern.map(compile).transpose() {
        Ok(set) => set,
        Err(invalid) => return invalid,
    };
    let name = match compile(name_pattern) {
        Ok(name) => name,
        Err(invalid) => return invalid,
    };
    let set = set.as_ref().map_or(NameSpec::Any, NameSpec::Pattern);
    let enumeration = match enumerate(context, object, set, NameSpec::Pattern(&name)) {
        Ok(enumeration) => enumeration,
        Err((reason, message)) => return Selection::NotEvaluated(reason, message),
    };
    evidence.push(enumeration.evidence().clone());
    if enumeration.properties().is_empty() {
        return Selection::NoMatch;
    }
    let decisive = matches!(matched, Quantifier::Any);
    let mut undecided = None;
    for property in enumeration.properties() {
        evidence.extend(property.evidence.iter().cloned());
        match test.holds_quantified(&property.value, quantifier, options) {
            Ok(held) if held == decisive => return verdict(decisive),
            Ok(_) => {}
            Err(message) => {
                undecided.get_or_insert(format!(
                    "property selector on `{}.{}`: {message}",
                    property.property_set, property.name
                ));
            }
        }
    }
    match undecided {
        Some(message) => Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, message),
        None => verdict(!decisive),
    }
}

/// How a rule names a property set or a property for an enumeration.
#[derive(Clone, Copy, Debug)]
pub(crate) enum NameSpec<'a> {
    /// Every name.
    Any,
    /// A concept (or, natively, a source name), bound through the package
    /// vocabulary like a property request's names.
    Exact(&'a str),
    /// A pattern over the source's own names; never bound.
    Pattern(&'a NamePattern),
}

impl std::fmt::Display for NameSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::Exact(name) => f.write_str(name),
            Self::Pattern(pattern) => write!(f, "/{}/", pattern.as_str()),
        }
    }
}

/// A whole-name matcher for an XML Schema pattern, as IDS names property
/// sets and properties.
pub(crate) fn xsd_name_pattern(pattern: &str) -> Result<NamePattern, String> {
    let translated = crate::xsd_pattern::translate(pattern)?;
    NamePattern::new(translated).map_err(|error| error.to_string())
}

/// A request enumerating `object`'s properties, exact names bound through the
/// package vocabulary as in [`bound_property_request`]. A pattern names the
/// source's own names and is never bound.
pub(crate) fn bound_enumeration_request(
    context: &RuleContext<'_>,
    object: &Object,
    set: NameSpec<'_>,
    property: NameSpec<'_>,
) -> Result<PropertyEnumerationRequest, (NotEvaluatedReason, String)> {
    let bind =
        |spec: NameSpec<'_>, is_set: bool| -> Result<NameMatch, (NotEvaluatedReason, String)> {
            Ok(match spec {
                NameSpec::Any => NameMatch::Any,
                NameSpec::Pattern(pattern) => NameMatch::Pattern(pattern.clone()),
                NameSpec::Exact(name) => NameMatch::Exact(match vocabulary(context) {
                    Vocabulary::Native => name.to_owned(),
                    Vocabulary::Package(_) if is_set && axioval_ir::is_reserved_set(name) => {
                        name.to_owned()
                    }
                    Vocabulary::Package(bindings) => {
                        let source = &object.id.source;
                        let bound = if is_set {
                            bindings.property_set(name, source)
                        } else {
                            bindings.property(name, source)
                        };
                        bound.map_err(|error| binding_error(&error))?.to_owned()
                    }
                }),
            })
        };
    PropertyEnumerationRequest::try_new(object.id.clone(), bind(set, true)?, bind(property, false)?)
        .map_err(|_| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                format!(
                    "properties {set}.{property} cannot be enumerated: a name is blank or a reserved set"
                ),
            )
        })
}

/// Every property of `object` in the sets `set` names whose names `property`
/// names, with the evidence that there are no others.
pub(crate) fn enumerate(
    context: &RuleContext<'_>,
    object: &Object,
    set: NameSpec<'_>,
    property: NameSpec<'_>,
) -> Result<PropertyEnumeration, (NotEvaluatedReason, String)> {
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        ));
    };
    let request = bound_enumeration_request(context, object, set, property)?;
    service.enumerate(&request).map_err(property_error)
}

/// The sets `set` names in which `matched` found no property, by name and
/// sorted, with the evidence of their enumeration: a rule naming its sets
/// requires a match in each of them, as IDS does. A set the object carries
/// without any member is one of them (`PropertyEnumeration::empty_sets`).
pub(crate) fn sets_without_match(
    context: &RuleContext<'_>,
    object: &Object,
    set: NameSpec<'_>,
    matched: &PropertyEnumeration,
) -> Result<(Vec<String>, Evidence), (NotEvaluatedReason, String)> {
    let every = enumerate(context, object, set, NameSpec::Any)?;
    let missing: std::collections::BTreeSet<String> = every
        .properties()
        .iter()
        .map(|property| property.property_set.clone())
        .chain(every.empty_sets().iter().cloned())
        .filter(|name| {
            !matched
                .properties()
                .iter()
                .any(|found| found.property_set == *name)
        })
        .collect();
    Ok((missing.into_iter().collect(), every.evidence().clone()))
}

pub(crate) fn property_error(error: PropertyResolutionError) -> (NotEvaluatedReason, String) {
    match error {
        PropertyResolutionError::Unavailable(message) => {
            (NotEvaluatedReason::BackendUnavailable, message)
        }
        PropertyResolutionError::Incomplete(message) => {
            (NotEvaluatedReason::IncompleteEvidence, message)
        }
        PropertyResolutionError::NotRecorded(message) => (NotEvaluatedReason::NotRecorded, message),
        PropertyResolutionError::MissingService(message) => {
            (NotEvaluatedReason::MissingService, message)
        }
        error => (NotEvaluatedReason::InvalidEvidence, error.to_string()),
    }
}

fn unavailable(error: PropertyResolutionError) -> Selection {
    let (reason, message) = property_error(error);
    Selection::NotEvaluated(reason, message)
}

#[derive(Clone, Copy, Debug)]
enum Order {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Order {
    fn of(operator: &ComparisonOperator) -> Option<Self> {
        Some(match operator {
            ComparisonOperator::Equals => Self::Equal,
            ComparisonOperator::NotEquals => Self::NotEqual,
            ComparisonOperator::LessThan => Self::Less,
            ComparisonOperator::LessThanOrEquals => Self::LessOrEqual,
            ComparisonOperator::GreaterThan => Self::Greater,
            ComparisonOperator::GreaterThanOrEquals => Self::GreaterOrEqual,
            _ => return None,
        })
    }

    fn is_equality(self) -> bool {
        matches!(self, Self::Equal | Self::NotEqual)
    }

    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Self::Equal => ordering.is_eq(),
            Self::NotEqual => !ordering.is_eq(),
            Self::Less => ordering.is_lt(),
            Self::LessOrEqual => ordering.is_le(),
            Self::Greater => ordering.is_gt(),
            Self::GreaterOrEqual => ordering.is_ge(),
        }
    }
}

/// The declared side of an ordered comparison, in canonical form.
#[derive(Debug)]
enum Expected {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    /// In SI, with its dimension.
    Quantity(f64, QuantityDimension),
    /// Folded as declared.
    Text(String),
    Date(Date),
    DateTime(DateTime),
}

/// A property selector's comparison, validated against its declaration.
#[derive(Debug)]
enum Test {
    Exists,
    /// Present but null, blank or a list of nothing else.
    Empty,
    /// Present with a value.
    NotEmpty,
    /// With the declared precision of a date or date-time comparison.
    Compare(Order, Expected, Option<TemporalPrecision>),
    /// Folded as declared.
    Contains(String),
    /// `matches` or `like`, anchored to the whole value.
    Pattern(Regex),
    /// Folded as declared; `none` for `noneOf`.
    Member {
        texts: Vec<String>,
        none: bool,
    },
}

impl Test {
    /// Whether the test judges a present value as a whole, never compared.
    fn judges_presence(&self) -> bool {
        matches!(self, Self::Exists | Self::Empty | Self::NotEmpty)
    }

    fn parse(
        operator: &ComparisonOperator,
        expected: Option<&ParameterValue>,
        options: TextOptions,
        precision: Option<TemporalPrecision>,
    ) -> Result<Self, String> {
        let temporal = matches!(
            expected,
            Some(ParameterValue::Date { .. } | ParameterValue::DateTime { .. })
        );
        if precision.is_some() && !temporal {
            return Err("`precision` applies to a date or date-time comparison only".into());
        }
        let Some(expected) = expected else {
            let presence = match operator {
                ComparisonOperator::Exists => Some(Self::Exists),
                ComparisonOperator::IsEmpty => Some(Self::Empty),
                ComparisonOperator::IsNotEmpty => Some(Self::NotEmpty),
                _ => None,
            };
            return if let Some(presence) = presence {
                if options.is_default() {
                    Ok(presence)
                } else {
                    Err("`caseSensitive` and `trim` apply to text comparisons only".into())
                }
            } else {
                Err("comparison has no expected value".into())
            };
        };
        let string = |kind: &str| match expected {
            ParameterValue::String { value } => Ok(value.as_str()),
            _ => Err(format!("`{}` takes {kind}", operator_name(operator))),
        };
        let test = match operator {
            ComparisonOperator::Exists
            | ComparisonOperator::IsEmpty
            | ComparisonOperator::IsNotEmpty => {
                return Err(format!(
                    "the {} operator must not have a value",
                    operator_name(operator)
                ));
            }
            ComparisonOperator::Matches => Self::Pattern(
                RegexBuilder::new(&format!("^(?:{})$", string("a string")?))
                    .case_insensitive(!options.case_sensitive)
                    .build()
                    .map_err(|error| format!("invalid regular expression: {error}"))?,
            ),
            ComparisonOperator::Like => Self::Pattern(
                RegexBuilder::new(&wildcard(string("a string")?)?)
                    .case_insensitive(!options.case_sensitive)
                    .build()
                    .map_err(|error| format!("invalid wildcard pattern: {error}"))?,
            ),
            ComparisonOperator::Contains => Self::Contains(options.fold(string("a string")?)),
            ComparisonOperator::OneOf | ComparisonOperator::NoneOf => match expected {
                ParameterValue::StringList { value } => Self::Member {
                    texts: value.iter().map(|text| options.fold(text)).collect(),
                    none: matches!(operator, ComparisonOperator::NoneOf),
                },
                _ => {
                    return Err(format!("`{}` takes a string list", operator_name(operator)));
                }
            },
            ordered => {
                let order = Order::of(ordered).expect("every other operator is ordered");
                let value = match expected {
                    ParameterValue::Boolean { value } if order.is_equality() => {
                        Expected::Boolean(*value)
                    }
                    ParameterValue::Integer { value } => Expected::Integer(*value),
                    ParameterValue::Number { value } if value.is_finite() => {
                        Expected::Number(*value)
                    }
                    ParameterValue::Quantity { value, unit } => {
                        let (value, dimension) =
                            si_quantity(*value, unit).map_err(|(_, message)| message)?;
                        Expected::Quantity(value, dimension)
                    }
                    ParameterValue::String { value } => Expected::Text(options.fold(value)),
                    ParameterValue::Date { value } => Expected::Date(*value),
                    ParameterValue::DateTime { value } => Expected::DateTime(*value),
                    ParameterValue::Enum { value } | ParameterValue::Reference { value }
                        if order.is_equality() =>
                    {
                        Expected::Text(options.fold(value))
                    }
                    _ => {
                        return Err(format!(
                            "`{}` does not apply to this value",
                            operator_name(operator)
                        ));
                    }
                };
                if !matches!(value, Expected::Text(_)) && !options.is_default() {
                    return Err("`caseSensitive` and `trim` apply to text comparisons only".into());
                }
                Self::Compare(order, value, precision)
            }
        };
        Ok(test)
    }

    /// Whether `actual` satisfies the test under `quantifier`.
    ///
    /// A list, a bounded value or a table is compared only value by value
    /// (its stated values, see `PropertyValue::stated_values`), and only
    /// when the selector states how; a scalar under a quantifier is a list
    /// of one.
    /// `all` needs at least one element, so an empty list satisfies neither
    /// quantifier. An element that cannot be compared decides the outcome
    /// only when the others leave it open.
    fn holds_quantified(
        &self,
        actual: &PropertyValue,
        quantifier: Option<Quantifier>,
        options: TextOptions,
    ) -> Result<bool, String> {
        let Some(quantifier) = quantifier else {
            if actual.stated_values().is_some() && !self.judges_presence() {
                return Err(format!(
                    "the value is {}; state `quantifier` `any` or `all` to compare its values",
                    kind(actual)
                ));
            }
            return self.holds(actual, options);
        };
        let elements = actual.stated_values().unwrap_or_else(|| vec![actual]);
        let (decisive, mut undecided) = match quantifier {
            Quantifier::Any => (true, None),
            Quantifier::All => (false, None),
        };
        for element in &elements {
            if !element.is_scalar() {
                return Err(format!(
                    "{} nested in a value cannot be compared",
                    kind(element)
                ));
            }
            match self.holds(element, options) {
                Ok(held) if held == decisive => return Ok(decisive),
                Ok(_) => {}
                Err(message) => {
                    undecided.get_or_insert(message);
                }
            }
        }
        match undecided {
            Some(message) => Err(message),
            None => Ok(!decisive && !elements.is_empty()),
        }
    }

    /// Whether every value of a measured interval satisfies the test, or
    /// none does; `Err` when it straddles the bound or compares with
    /// another kind of value.
    fn holds_measured(
        &self,
        (lower, upper): (f64, f64),
        held: QuantityDimension,
    ) -> Result<bool, String> {
        match self {
            Self::Exists | Self::NotEmpty => Ok(true),
            Self::Empty => Ok(false),
            Self::Compare(order, Expected::Quantity(expected, dimension), _) => {
                if held != *dimension {
                    return Err(format!(
                        "a quantity in {} cannot be compared with one in {}",
                        held.unit_symbol(),
                        dimension.unit_symbol()
                    ));
                }
                let tolerance = Tolerance::unit_conversion();
                let (Some(least), Some(greatest)) = (
                    tolerance.order(lower, *expected),
                    tolerance.order(upper, *expected),
                ) else {
                    return Err("the measured value is not finite".into());
                };
                crate::support::interval_verdict(least, greatest, |ordering| order.holds(ordering))
                    .ok_or_else(|| {
                        format!(
                            "the measured value lies between {lower} and {upper} {}, which \
                             straddles the bound",
                            held.unit_symbol()
                        )
                    })
            }
            _ => Err(format!(
                "the value is a measured quantity in {} but the selector compares another kind",
                held.unit_symbol()
            )),
        }
    }

    /// Whether `actual` satisfies the test; `Err` when the value's type
    /// cannot be compared with the declared one, so the object is not
    /// evaluated rather than silently left out.
    fn holds(&self, actual: &PropertyValue, options: TextOptions) -> Result<bool, String> {
        // A comparison presupposes a value: null is no more a match than an
        // absent property.
        if matches!(actual, PropertyValue::Null) {
            return Ok(matches!(self, Self::Exists | Self::Empty));
        }
        if let PropertyValue::Measured {
            lower,
            upper,
            dimension,
        } = actual
        {
            return self.holds_measured((*lower, *upper), *dimension);
        }
        let mismatch = |declared: &str| {
            Err(format!(
                "the value is {} but the selector compares {declared}",
                kind(actual)
            ))
        };
        match self {
            Self::Exists => Ok(true),
            // Emptiness has the meaning `property-requirements` gives presence.
            Self::Empty => Ok(undefined(Some(actual))),
            Self::NotEmpty => Ok(!undefined(Some(actual))),
            Self::Compare(order, expected, precision) => {
                let ordering = match (expected, actual) {
                    (Expected::Date(expected), _) => {
                        match temporal_order(actual, &PropertyValue::Date(*expected), *precision) {
                            Some(ordering) => ordering?,
                            None => return mismatch("a date"),
                        }
                    }
                    (Expected::DateTime(expected), _) => {
                        let expected = PropertyValue::DateTime(*expected);
                        match temporal_order(actual, &expected, *precision) {
                            Some(ordering) => ordering?,
                            None => return mismatch("a date-time"),
                        }
                    }
                    (Expected::Boolean(expected), PropertyValue::Boolean(actual)) => {
                        actual.cmp(expected)
                    }
                    (Expected::Integer(expected), PropertyValue::Integer(actual)) => {
                        actual.cmp(expected)
                    }
                    (Expected::Integer(expected), PropertyValue::Decimal(actual)) => {
                        numeric(*actual, exact_f64(*expected))?
                    }
                    (Expected::Number(expected), PropertyValue::Decimal(actual)) => {
                        numeric(*actual, Some(*expected))?
                    }
                    (Expected::Number(expected), PropertyValue::Integer(actual)) => {
                        numeric_integer(*actual, *expected)?
                    }
                    (
                        Expected::Quantity(expected, dimension),
                        PropertyValue::Quantity {
                            value,
                            dimension: held,
                        },
                    ) => {
                        if held != dimension {
                            return Err(format!(
                                "a quantity in {} cannot be compared with one in {}",
                                held.unit_symbol(),
                                dimension.unit_symbol()
                            ));
                        }
                        Tolerance::unit_conversion()
                            .order(*value, *expected)
                            .ok_or("the quantity is not finite")?
                    }
                    (Expected::Text(expected), PropertyValue::String(actual)) => {
                        options.prepare(actual).as_str().cmp(expected.as_str())
                    }
                    (Expected::Boolean(_), _) => return mismatch("a boolean"),
                    (Expected::Integer(_) | Expected::Number(_), _) => {
                        return mismatch("a unit-less number");
                    }
                    (Expected::Quantity(_, dimension), _) => {
                        return mismatch(&format!("a quantity in {}", dimension.unit_symbol()));
                    }
                    (Expected::Text(_), _) => return mismatch("text"),
                };
                Ok(order.holds(ordering))
            }
            Self::Contains(text) => match actual {
                PropertyValue::String(actual) => Ok(options.prepare(actual).contains(text)),
                _ => mismatch("text"),
            },
            Self::Pattern(pattern) => match actual {
                PropertyValue::String(actual) => {
                    Ok(pattern.is_match(if options.trim { actual.trim() } else { actual }))
                }
                _ => mismatch("text"),
            },
            Self::Member { texts, none } => match actual {
                PropertyValue::String(actual) => {
                    Ok(texts.contains(&options.prepare(actual)) != *none)
                }
                _ => mismatch("text"),
            },
        }
    }
}

fn numeric(actual: f64, expected: Option<f64>) -> Result<Ordering, String> {
    match expected {
        Some(expected) => actual
            .partial_cmp(&expected)
            .ok_or_else(|| "the value is not a finite number".into()),
        None => Err("an integer beyond 2^53 cannot be compared with a number".into()),
    }
}

fn numeric_integer(actual: i64, expected: f64) -> Result<Ordering, String> {
    let actual = exact_f64(actual)
        .ok_or_else(|| "an integer beyond 2^53 cannot be compared with a number".to_owned())?;
    numeric(actual, Some(expected))
}

fn kind(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".into(),
        PropertyValue::Boolean(_) => "a boolean".into(),
        PropertyValue::Integer(_) => "an integer".into(),
        PropertyValue::Decimal(_) => "a number".into(),
        PropertyValue::Quantity { dimension, .. } => {
            format!("a quantity in {}", dimension.unit_symbol())
        }
        PropertyValue::String(_) => "text".into(),
        PropertyValue::Date(_) => "a date".into(),
        PropertyValue::DateTime(_) => "a date-time".into(),
        PropertyValue::List(_) => "a list".into(),
        PropertyValue::Bounded { .. } => "a bounded value".into(),
        PropertyValue::Table(_) => "a table".into(),
        PropertyValue::Measured { dimension, .. } => {
            format!("a measured quantity in {}", dimension.unit_symbol())
        }
    }
}

fn operator_name(operator: &ComparisonOperator) -> &'static str {
    match operator {
        ComparisonOperator::Equals => "equals",
        ComparisonOperator::NotEquals => "notEquals",
        ComparisonOperator::LessThan => "lessThan",
        ComparisonOperator::LessThanOrEquals => "lessThanOrEquals",
        ComparisonOperator::GreaterThan => "greaterThan",
        ComparisonOperator::GreaterThanOrEquals => "greaterThanOrEquals",
        ComparisonOperator::Matches => "matches",
        ComparisonOperator::Like => "like",
        ComparisonOperator::Contains => "contains",
        ComparisonOperator::OneOf => "oneOf",
        ComparisonOperator::NoneOf => "noneOf",
        ComparisonOperator::Exists => "exists",
        ComparisonOperator::IsEmpty => "isEmpty",
        ComparisonOperator::IsNotEmpty => "isNotEmpty",
    }
}

/// A wildcard pattern as an anchored regular expression.
///
/// `*` is any run of characters (none included), `?` exactly one, and a
/// backslash makes the next character literal (`\*`, `\?`, `\\`). Every
/// other character is literal.
pub(crate) fn wildcard(pattern: &str) -> Result<String, String> {
    axioval_engine::wildcard_regex(pattern)
}

#[cfg(test)]
mod tests {
    use super::wildcard;

    #[test]
    fn wildcards_translate_to_anchored_literals() {
        assert_eq!(wildcard("W?-*.1").unwrap(), r"(?s)^W.\-.*\.1$");
        assert_eq!(wildcard(r"a\*b").unwrap(), r"(?s)^a\*b$");
        assert!(wildcard("a\\").is_err());
    }
}
