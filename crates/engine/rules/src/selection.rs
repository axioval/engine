//! Deterministic, fail-closed selector evaluation.

use std::sync::Arc;

use axioval_engine::MeasuredMemo;
use axioval_engine::comparison::{self as shared, Order, Pattern, Tolerance, Undecided};
use axioval_engine::{
    BindingError, CapabilityEvaluation, ClassificationAssignment, ClassificationError,
    ClassificationServiceHandle, Classifications, ConceptBindings, NameMatch, NamePattern,
    NotEvaluatedReason, PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle, ResourceObjects,
    RuleContext, RuleOutcomes, SourceDisciplines, SourceMetadataIndex, TypeHierarchyError,
    TypeHierarchyServiceHandle,
};
use axioval_ir::contract::{
    ComparisonOperator, ParameterValue, Quantifier, RelatedQuantifier, RuleOutcomeKind, Selector,
};
use axioval_ir::{
    CLASSIFICATION_SET, Date, DateTime, Discipline, Evidence, Object, ObjectId, PropertyValue,
    QuantityDimension, SourceId, TemporalPrecision,
};
use regex::Regex;

pub(crate) use axioval_engine::comparison::TextOptions;

use crate::support::{Traversal, exact_f64, si_quantity, undefined};

/// What a run's shared selections are kept by: the selector, as written.
#[derive(Clone, Hash, PartialEq, Eq)]
struct SharedSelector(Arc<str>);

/// A shared selection: the positions in its population of the objects
/// selected, in order, and the outcomes of the objects and sources it could
/// not decide.
type Shared = Arc<(Vec<usize>, CapabilityEvaluation)>;

/// How many selectors [`shared_as`] keeps before it starts over.
const SHARED_KEPT: usize = 1024;

/// What a shared selection of `selector` is kept by, as written; `None`
/// where it is never shared (it reads a rule's outcomes or evaluates an
/// expression). A pure function of the selector, so written once per
/// process for each selector (at most [`SHARED_KEPT`]), found again by a
/// fingerprint of its shape and compared whole.
fn shared_as(selector: &Selector) -> Option<Arc<str>> {
    type Written = std::collections::HashMap<u64, Vec<(Selector, Option<Arc<str>>)>>;
    static KEPT: std::sync::LazyLock<std::sync::Mutex<(usize, Written)>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new((0, Written::new())));
    let print = {
        use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hasher};
        let mut hasher = BuildHasherDefault::<DefaultHasher>::default().build_hasher();
        fingerprint(selector, &mut hasher);
        hasher.finish()
    };
    if let Ok(kept) = KEPT.lock()
        && let Some((_, written)) = kept
            .1
            .get(&print)
            .and_then(|kept| kept.iter().find(|(known, _)| known == selector))
    {
        return written.clone();
    }
    let written = (selector.rule_references().is_empty() && selector.expressions().is_empty())
        .then(|| serde_json::to_string(selector).ok())
        .flatten()
        .map(Arc::from);
    if let Ok(mut kept) = KEPT.lock() {
        if kept.0 >= SHARED_KEPT {
            *kept = (0, Written::new());
        }
        kept.0 += 1;
        kept.1
            .entry(print)
            .or_default()
            .push((selector.clone(), written.clone()));
    }
    written
}

/// A cheap fingerprint of `selector`'s shape: equal selectors share it.
fn fingerprint(selector: &Selector, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(selector).hash(hasher);
    match selector {
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => {
            object_type.hash(hasher);
            include_subtypes.hash(hasher);
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            operands.len().hash(hasher);
            for operand in operands {
                fingerprint(operand, hasher);
            }
        }
        Selector::Not { operand } => fingerprint(operand, hasher),
        _ => {}
    }
}

/// [`select_objects`], selected once per run for every rule and template
/// selecting the same objects: a selector that reads no rule's outcomes
/// and evaluates no expression (whose evaluation spends the run's budget)
/// selects the same objects whichever rule reads it. Any other selector is
/// selected now.
pub(crate) fn select_shared<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let (Some(written), Some(memo)) = (shared_as(selector), context.services.get::<MeasuredMemo>())
    else {
        return select_objects(context, selector);
    };
    // The population is the same for every rule of the run, so a position
    // in it names the same object.
    let (population, unreadable) = population(context, selector);
    let population: Vec<&'a Object> = population.collect();
    let shared: Shared = memo.get_or_measure(SharedSelector(written), || {
        let decided: Arc<Decided> = memo.get_or_measure(DecidedKey, Arc::default);
        Arc::new(select_in(
            context,
            selector,
            &population,
            unreadable,
            Some(&decided),
        ))
    });
    let selected = shared.0.iter().map(|index| population[*index]).collect();
    (selected, shared.1.clone())
}

pub(crate) fn select_objects<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let (population, unreadable) = population(context, selector);
    let population: Vec<&'a Object> = population.collect();
    let (selected, evaluation) = select_in(context, selector, &population, unreadable, None);
    (
        selected
            .into_iter()
            .map(|index| population[index])
            .collect(),
        evaluation,
    )
}

/// The positions in `population` of the objects `selector` selects, in
/// order, and the outcomes of those it cannot decide and of the sources
/// whose resource objects are `unreadable`.
fn select_in(
    context: &RuleContext<'_>,
    selector: &Selector,
    population: &[&Object],
    unreadable: Vec<(SourceId, String)>,
    decided: Option<&Decided>,
) -> (Vec<usize>, CapabilityEvaluation) {
    let mut selected = Vec::new();
    let mut evaluation = CapabilityEvaluation::default();
    for (source, why) in unreadable {
        evaluation.push_source_not_evaluated(
            source,
            NotEvaluatedReason::IncompleteEvidence,
            format!("its resource objects cannot be listed: {why}"),
        );
    }
    // An entity type selects by the object's source and kind alone: each
    // pair is decided once, however many objects share it.
    let mut kinds: Vec<(&SourceId, &str, Selection)> = Vec::new();
    // Entity types combined, in a shared selection: each decided once per
    // run (one alone is decided once per source and kind here).
    let decided =
        decided.filter(|_| !matches!(selector, Selector::EntityType { .. }) && of_kinds(selector));
    for (index, object) in population.iter().copied().enumerate() {
        let selection = match (selector, decided) {
            (
                Selector::EntityType {
                    object_type,
                    include_subtypes,
                },
                None,
            ) => {
                let (source, kind) = (&object.id.source, object.kind());
                let decided = kinds
                    .iter()
                    .find(|(decided, decided_kind, _)| {
                        *decided_kind == kind
                            && (std::ptr::eq(*decided, source) || *decided == source)
                    })
                    .map(|(_, _, selection)| selection.clone());
                decided.unwrap_or_else(|| {
                    let selection =
                        entity_type_matches(context, object, object_type, *include_subtypes);
                    kinds.push((source, kind, selection.clone()));
                    selection
                })
            }
            // Decided once per source and kind.
            (_, Some(decided)) => {
                let (source, kind) = (&object.id.source, object.kind());
                let known = kinds
                    .iter()
                    .find(|(known, known_kind, _)| {
                        *known_kind == kind && (std::ptr::eq(*known, source) || *known == source)
                    })
                    .map(|(_, _, selection)| selection.clone());
                known.unwrap_or_else(|| {
                    let selection = of_kind(context, selector, object, decided);
                    kinds.push((source, kind, selection.clone()));
                    selection
                })
            }
            _ => selector_matches(context, selector, object, &mut Vec::new()),
        };
        match selection {
            Selection::Match => selected.push(index),
            Selection::NoMatch => {}
            Selection::NotEvaluated(reason, message) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    (selected, evaluation)
}

/// What the entity types a run decided are kept by.
#[derive(Hash, PartialEq, Eq)]
struct DecidedKey;

/// The entity types a run decided for its shared selections: whether an
/// object is of a type is a fact of the run, whichever selector asks.
#[derive(Default)]
pub(crate) struct Decided(std::sync::Mutex<DecidedTable>);

#[derive(Default)]
struct DecidedTable {
    /// The entity types asked, as written, each with whether subtypes
    /// count.
    types: Vec<(Box<str>, bool)>,
    /// Each object's decision of a type, by the object's place in memory
    /// (the run's objects stay where they are for the run) and the type's
    /// place in `types`: matched (0), not (1), or undecided, `unsure`'s
    /// entry at the code less two. Small, since a run keeps it whole.
    decided: std::collections::HashMap<(usize, u32), u32>,
    /// The undecided outcomes, each with its reason and message.
    unsure: Vec<Selection>,
}

impl DecidedTable {
    fn get(&self, key: (usize, u32)) -> Option<Selection> {
        Some(match *self.decided.get(&key)? {
            0 => Selection::Match,
            1 => Selection::NoMatch,
            code => self.unsure.get(usize::try_from(code - 2).ok()?)?.clone(),
        })
    }

    fn insert(&mut self, key: (usize, u32), selection: &Selection) {
        let code = match selection {
            Selection::Match => 0,
            Selection::NoMatch => 1,
            Selection::NotEvaluated(..) => {
                let Ok(code) = u32::try_from(self.unsure.len() + 2) else {
                    return;
                };
                self.unsure.push(selection.clone());
                code
            }
        };
        self.decided.insert(key, code);
    }
}

impl Decided {
    /// Whether `object` is of `object_type` (with subtypes where
    /// `include_subtypes`), decided once per run.
    fn entity_type(
        &self,
        context: &RuleContext<'_>,
        object: &Object,
        object_type: &str,
        include_subtypes: bool,
    ) -> Selection {
        let place = std::ptr::from_ref(object) as usize;
        let key = self.0.lock().ok().map(|mut table| {
            let known = table.types.iter().position(|(known, subtypes)| {
                **known == *object_type && *subtypes == include_subtypes
            });
            let index = if let Some(index) = known {
                index
            } else {
                table.types.push((object_type.into(), include_subtypes));
                table.types.len() - 1
            };
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            (place, index, table.get((place, index)))
        });
        if let Some((_, _, Some(selection))) = key {
            return selection;
        }
        let selection = entity_type_matches(context, object, object_type, include_subtypes);
        if let Some((place, index, None)) = key
            && let Ok(mut table) = self.0.lock()
        {
            table.insert((place, index), &selection);
        }
        selection
    }
}

/// Whether `selector` selects by entity types alone: by an object's source
/// and kind, nothing else of it.
fn of_kinds(selector: &Selector) -> bool {
    match selector {
        Selector::EntityType { .. } => true,
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            operands.iter().all(of_kinds)
        }
        Selector::Not { operand } => of_kinds(operand),
        _ => false,
    }
}

/// [`selector_matches`] of a selector [`of_kinds`], each entity type
/// decided once per run.
fn of_kind(
    context: &RuleContext<'_>,
    selector: &Selector,
    object: &Object,
    decided: &Decided,
) -> Selection {
    match selector {
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => decided.entity_type(context, object, object_type, *include_subtypes),
        Selector::AllOf { operands } => all_of(
            operands
                .iter()
                .map(|item| of_kind(context, item, object, decided)),
        ),
        Selector::AnyOf { operands } => any_of(
            operands
                .iter()
                .map(|item| of_kind(context, item, object, decided)),
        ),
        Selector::Not { operand } => match of_kind(context, operand, object, decided) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
        other => selector_matches(context, other, object, &mut Vec::new()),
    }
}

/// Every object `selector` may select: the project's objects, then the
/// resource objects it reaches ([`ResourceObjects::reached`]), and the
/// sources whose reached resource objects could not be listed.
pub(crate) fn population<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (
    impl Iterator<Item = &'a Object> + use<'a>,
    Vec<(SourceId, String)>,
) {
    let services: &'a axioval_engine::ServiceRegistry = context.services;
    let reached = services
        .get::<ResourceObjects>()
        .map(|resources| resources.reached(selector, services.get::<RuleOutcomes>()))
        .unwrap_or_default();
    (
        context.project.objects().chain(reached.objects),
        reached.unreadable,
    )
}

/// The object or resource object `id` of the run.
pub(crate) fn object_by_id<'a>(context: &RuleContext<'a>, id: &ObjectId) -> Option<&'a Object> {
    let services: &'a axioval_engine::ServiceRegistry = context.services;
    context.project.object(id).or_else(|| {
        services
            .get::<ResourceObjects>()
            .and_then(|resources| resources.object(id))
    })
}

/// Whether `id` is one of the run's resource objects.
pub(crate) fn is_resource(context: &RuleContext<'_>, id: &ObjectId) -> bool {
    context
        .services
        .get::<ResourceObjects>()
        .is_some_and(|resources| resources.object(id).is_some())
}

#[derive(Clone, Debug)]
pub(crate) enum Selection {
    Match,
    NoMatch,
    NotEvaluated(NotEvaluatedReason, String),
}

/// Whether `object` is selected; property facts consulted are added to `evidence`.
#[allow(clippy::too_many_lines)] // one arm per selector kind
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
        derived @ Selector::DerivedClass { .. } => {
            derived_class(context, object, derived, evidence)
        }
        Selector::DerivedGroup { grouping } => derived_group(context, object, grouping),
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
        Selector::Expression { expression } => {
            expression_matches(context, object, expression, evidence)
        }
        Selector::Objects { objects } => verdict(objects.contains(&object.id)),
    }
}

/// Whether `expression` holds for `object`: true selects, false and `null`
/// do not, and an expression that cannot be decided leaves it not
/// evaluated with the reason and the subexpression's path.
fn expression_matches(
    context: &RuleContext<'_>,
    object: &Object,
    expression: &axioval_ir::contract::Expression,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    use axioval_engine::expression::{Reason, Value, evaluate};
    let mut leaves = crate::expression_leaves::ObjectLeaves::new(context, object, None);
    let evaluation = evaluate(expression, "selector.expression", &mut leaves);
    evidence.extend(
        evaluation
            .reads
            .iter()
            .flat_map(|read| read.leaf.evidence.iter().cloned()),
    );
    match evaluation.outcome {
        Ok(Value::Boolean(holds)) => verdict(holds),
        Ok(Value::Null) => Selection::NoMatch,
        Ok(other) => Selection::NotEvaluated(
            NotEvaluatedReason::InvalidDeclaration,
            format!("the selector's expression is {}, not a truth", other.kind()),
        ),
        Err(why) => Selection::NotEvaluated(
            leaves
                .first_reason()
                .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                .unwrap_or_else(|| crate::expression_requirement::reason_of(&why)),
            format!("the selector's expression: {why}"),
        ),
    }
}

/// Whether a classification of the run assigned `object` the class, or a
/// class within it, from the classes the runtime derived before any rule.
fn derived_class(
    context: &RuleContext<'_>,
    object: &Object,
    selector: &Selector,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    let Selector::DerivedClass {
        classification,
        class,
        include_descendants,
    } = selector
    else {
        unreachable!("only `derivedClass` selectors are matched here");
    };
    let Some(classifications) = context.services.get::<std::sync::Arc<Classifications>>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "no derived classes are available outside a run".into(),
        );
    };
    match classifications.selects(classification, class, *include_descendants, &object.id) {
        Ok(matches) => {
            evidence.push(Evidence::exact(
                object.id.source.clone(),
                format!("{CLASSIFICATION_SET}/{classification}#class={class}"),
            ));
            verdict(matches)
        }
        Err((reason, message)) => Selection::NotEvaluated(reason, message),
    }
}

/// Whether `object` is a group the run's grouping `grouping` derived.
fn derived_group(context: &RuleContext<'_>, object: &Object, grouping: &str) -> Selection {
    let Some(groups) = context
        .services
        .get::<std::sync::Arc<axioval_engine::DerivedGroups>>()
    else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "no derived groups are available outside a run".into(),
        );
    };
    match groups.grouping(grouping) {
        Some(derived) => verdict(derived.group(&object.id).is_some()),
        None => Selection::NotEvaluated(
            NotEvaluatedReason::InvalidDeclaration,
            format!("the run derives no grouping `{grouping}`"),
        ),
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
    axioval_engine::binding_reason(error)
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
    axioval_engine::bound_property_request(context.services, &object.id, set, name)
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
        PropertyResolutionError::InvalidArgument(message) => {
            (NotEvaluatedReason::InvalidDeclaration, message)
        }
        PropertyResolutionError::MissingService(message) => {
            (NotEvaluatedReason::MissingService, message)
        }
        // Present, of a known type: only the value is unknown.
        PropertyResolutionError::UnreadableValue(unreadable) => (
            NotEvaluatedReason::IncompleteEvidence,
            unreadable.reason().to_owned(),
        ),
        error => (NotEvaluatedReason::InvalidEvidence, error.to_string()),
    }
}

fn unavailable(error: PropertyResolutionError) -> Selection {
    let (reason, message) = property_error(error);
    Selection::NotEvaluated(reason, message)
}

/// The order a selector's comparison operator states; `None` for every
/// operator that is no order.
fn order_of(operator: &ComparisonOperator) -> Option<Order> {
    Some(match operator {
        ComparisonOperator::Equals => Order::Equal,
        ComparisonOperator::NotEquals => Order::NotEqual,
        ComparisonOperator::LessThan => Order::Less,
        ComparisonOperator::LessThanOrEquals => Order::LessOrEqual,
        ComparisonOperator::GreaterThan => Order::Greater,
        ComparisonOperator::GreaterThanOrEquals => Order::GreaterOrEqual,
        _ => return None,
    })
}

impl Expected {
    /// A date or date-time literal as a value, and what it is.
    fn temporal(&self) -> Option<(PropertyValue, &'static str)> {
        match self {
            Self::Date(date) => Some((PropertyValue::Date(*date), "a date")),
            Self::DateTime(instant) => Some((PropertyValue::DateTime(*instant), "a date-time")),
            _ => None,
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

    #[allow(clippy::too_many_lines)]
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
            ComparisonOperator::Matches | ComparisonOperator::Like => {
                let kind = if matches!(operator, ComparisonOperator::Like) {
                    Pattern::Like
                } else {
                    Pattern::Matches
                };
                Self::Pattern(
                    shared::pattern(kind, string("a string")?, options.case_sensitive).map_err(
                        |error| match error {
                            shared::PatternError::Wildcard(why) => why,
                            shared::PatternError::Compile(why) => {
                                format!("invalid {}: {why}", kind.kind())
                            }
                        },
                    )?,
                )
            }
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
                let order = order_of(ordered).expect("every other operator is ordered");
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
        held: Option<QuantityDimension>,
    ) -> Result<bool, String> {
        let unit = |dimension: Option<QuantityDimension>| {
            dimension.map_or_else(
                || "a plain number".to_owned(),
                QuantityDimension::unit_symbol,
            )
        };
        match self {
            Self::Exists | Self::NotEmpty => Ok(true),
            Self::Empty => Ok(false),
            Self::Compare(order, wanted @ (Expected::Quantity(..) | Expected::Number(_)), _) => {
                let (expected, dimension) = match wanted {
                    Expected::Quantity(expected, dimension) => (expected, Some(*dimension)),
                    Expected::Number(expected) => (expected, None),
                    _ => unreachable!("matched above"),
                };
                if held != dimension {
                    return Err(format!(
                        "a measured value in {} cannot be compared with one in {}",
                        unit(held),
                        unit(dimension)
                    ));
                }
                shared::numbers(
                    *order,
                    (lower, upper),
                    (*expected, *expected),
                    &Tolerance::unit_conversion(),
                )
                .map_err(|undecided| match undecided {
                    Undecided::NotFinite => "the measured value is not finite".into(),
                    Undecided::Straddles => format!(
                        "the measured value lies between {lower} and {upper} ({}), which \
                         straddles the bound",
                        unit(held)
                    ),
                })
            }
            _ => Err(format!(
                "the value is measured in {} but the selector compares another kind",
                unit(held)
            )),
        }
    }

    /// Whether `actual` satisfies the test; `Err` when the value's type
    /// cannot be compared with the declared one, so the object is not
    /// evaluated rather than silently left out.
    #[allow(clippy::too_many_lines)]
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
                if let Some((expected, declared)) = expected.temporal() {
                    return match shared::temporal(*order, actual, &expected, *precision) {
                        Some(holds) => holds,
                        None => mismatch(declared),
                    };
                }
                let exact = |actual: f64, expected: f64| {
                    shared::numbers(
                        *order,
                        (actual, actual),
                        (expected, expected),
                        &Tolerance::EXACT,
                    )
                    .map_err(|_| "the value is not a finite number".to_owned())
                };
                let beyond =
                    || "an integer beyond 2^53 cannot be compared with a number".to_owned();
                match (expected, actual) {
                    (Expected::Boolean(expected), PropertyValue::Boolean(actual)) => {
                        shared::booleans(*order, *actual, *expected)
                            .ok_or_else(|| "booleans have no order".to_owned())
                    }
                    (Expected::Integer(expected), PropertyValue::Integer(actual)) => {
                        Ok(shared::integers(*order, *actual, *expected))
                    }
                    (Expected::Integer(expected), PropertyValue::Decimal(actual)) => {
                        exact(*actual, exact_f64(*expected).ok_or_else(beyond)?)
                    }
                    (Expected::Number(expected), PropertyValue::Decimal(actual)) => {
                        exact(*actual, *expected)
                    }
                    (Expected::Number(expected), PropertyValue::Integer(actual)) => {
                        exact(exact_f64(*actual).ok_or_else(beyond)?, *expected)
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
                        shared::numbers(
                            *order,
                            (*value, *value),
                            (*expected, *expected),
                            &Tolerance::unit_conversion(),
                        )
                        .map_err(|_| "the quantity is not finite".to_owned())
                    }
                    (Expected::Text(expected), PropertyValue::String(actual)) => {
                        Ok(shared::texts(*order, actual, expected, options))
                    }
                    (Expected::Boolean(_), _) => mismatch("a boolean"),
                    (Expected::Integer(_) | Expected::Number(_), _) => {
                        mismatch("a unit-less number")
                    }
                    (Expected::Quantity(_, dimension), _) => {
                        mismatch(&format!("a quantity in {}", dimension.unit_symbol()))
                    }
                    (Expected::Text(_), _) => mismatch("text"),
                    (Expected::Date(_), _) => mismatch("a date"),
                    (Expected::DateTime(_), _) => mismatch("a date-time"),
                }
            }
            Self::Contains(text) => match actual {
                PropertyValue::String(actual) => Ok(shared::contains(actual, text, options)),
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
                    Ok(shared::member(actual, texts, options) != *none)
                }
                _ => mismatch("text"),
            },
        }
    }
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
        PropertyValue::Reference(_) => "a reference".into(),
        PropertyValue::List(_) => "a list".into(),
        PropertyValue::Bounded { .. } => "a bounded value".into(),
        PropertyValue::Table(_) => "a table".into(),
        PropertyValue::Complex => "a complex property".into(),
        PropertyValue::Measured { dimension, .. } => match dimension {
            Some(dimension) => format!("a measured quantity in {}", dimension.unit_symbol()),
            None => "a measured number".into(),
        },
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
