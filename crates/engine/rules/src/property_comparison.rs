//! Exact, source-neutral property-to-property comparison capability.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    AbsentEndPolicy, CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PropertyResolutionServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, RuleCapability, RuleContext,
    SemanticRelationship,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, Severity, TemporalPrecision};

use crate::levels::Levels;
use crate::selection::select_objects;
use regex::{Regex, RegexBuilder};

use crate::support::{
    Parameters, PropertyRef, Tolerance, Traversal, Unavailable, invalid, resolve, temporal,
    temporal_order, undefined,
};

/// Compares a property on relationship-selected candidates with a property on each checked object.
///
/// Numbers and quantities may be compared under a declared `tolerance`,
/// `relative_tolerance` or `decimals`, applied between the compared value
/// and the target after its factor: within the tolerance they are equal, and
/// only beyond it greater or less. A tolerance on a text, text list or
/// boolean constant target is an invalid declaration.
///
/// Dates and date-times compare chronologically with the ordered operators,
/// against another property or a `target_date` or `target_date_time`
/// constant: dates by day, date-times as instants whatever their UTC
/// offsets. `precision` `day` reads every date-time as the calendar day it
/// states, so a date-time compares with a date; without it that pair is not
/// evaluated. A factor other than 1 or a tolerance does not apply to them,
/// and `precision` applies to nothing else.
///
/// Candidates are the checked object itself (`checked`), the members of a
/// group it shares (`shared`), the objects a relationship or `path` reaches
/// from it (`related`), or the objects in the same space or building as it
/// (`same_space`, `same_building`): those whose nearest `container_selector`
/// object, climbed to along the declared relationship steps, is one of its
/// own.
/// With `container_relationship` `axioval:derived.same-level`, a candidate
/// also shares the checked object's container when one of its containers is
/// on one level with one of the object's in another source (an MEP model's
/// storey and the architecture model's at one elevation); an undecided
/// level leaves the object not evaluated.
pub struct PropertyComparison;

enum Mode<'a> {
    Checked,
    Shared(&'a str, AbsentEndPolicy),
    Related(Traversal<'a>),
    /// Candidates sharing a nearest container with the checked object.
    SameContainer {
        traversal: Traversal<'a>,
        containers: &'a Selector,
        /// Containers of other sources matched as one level
        /// (`axioval:derived.same-level`) by a property, when declared.
        levels: Option<(&'a str, PropertyRef<'a>)>,
    },
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
#[derive(Clone, Copy, PartialEq, Eq)]
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
    /// The whole text matches a wildcard pattern (`*`, `?`, `\\` escapes),
    /// as the `like` property selector.
    Like,
    /// The whole text matches a regular expression, as the `matches`
    /// property selector.
    Matches,
    IsDefined,
    IsUndefined,
    /// Within an inclusive range.
    Between,
}

impl Operator {
    /// Whether an ordered operator holds for `ordering`; `None` for any other.
    fn orders(self, ordering: Ordering) -> Option<bool> {
        Some(match self {
            Self::Equals => ordering.is_eq(),
            Self::NotEquals => !ordering.is_eq(),
            Self::Greater => ordering.is_gt(),
            Self::GreaterOrEqual => ordering.is_ge(),
            Self::Less => ordering.is_lt(),
            Self::LessOrEqual => ordering.is_le(),
            _ => return None,
        })
    }

    /// Whether the operator judges presence alone and takes no target.
    fn judges_presence(self) -> bool {
        matches!(self, Self::IsDefined | Self::IsUndefined)
    }

    /// Whether the operator reads text only.
    fn textual(self) -> bool {
        matches!(
            self,
            Self::Contains | Self::OneOf | Self::NoneOf | Self::Like | Self::Matches
        )
    }
}

/// The right-hand side of every comparison.
enum Target<'a> {
    /// No target: `is_defined` and `is_undefined`.
    None,
    /// A property of the checked object.
    Property(PropertyRef<'a>),
    /// A declared constant.
    Value(PropertyValue),
    /// A declared list of texts or patterns; any one of them is enough.
    Texts(&'a [String]),
    /// Declared inclusive bounds, for `between`.
    Range(PropertyValue, PropertyValue),
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
            // Exactly one target: a property of the checked object, a
            // constant, a text list or a range.
            ParameterDescriptor::optional("target_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("target_number", ParameterType::Number),
            ParameterDescriptor::optional("target_quantity", ParameterType::Quantity),
            ParameterDescriptor::optional("target_text", ParameterType::String),
            ParameterDescriptor::optional("target_texts", ParameterType::StringList),
            ParameterDescriptor::optional("target_boolean", ParameterType::Boolean),
            ParameterDescriptor::optional("target_date", ParameterType::Date),
            ParameterDescriptor::optional("target_date_time", ParameterType::DateTime),
            ParameterDescriptor::optional("precision", ParameterType::String),
            ParameterDescriptor::optional("minimum_number", ParameterType::Number),
            ParameterDescriptor::optional("maximum_number", ParameterType::Number),
            ParameterDescriptor::optional("minimum_quantity", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum_quantity", ParameterType::Quantity),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::required("factor", ParameterType::Number),
            ParameterDescriptor::required("component_mode", ParameterType::String),
            ParameterDescriptor::optional("container_selector", ParameterType::Selector),
            ParameterDescriptor::optional("container_relationship", ParameterType::String),
            ParameterDescriptor::optional("level_property", ParameterType::PropertyReference),
            ParameterDescriptor::required("quantifier", ParameterType::String),
            ParameterDescriptor::optional("category_property", ParameterType::PropertyReference),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .chain(crate::support::tolerance_parameters())
        .collect()
    }
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("property-comparison parameters are invalid: {message}"),
                );
            }
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
        let containers = match &config.mode {
            Mode::SameContainer { containers, .. } => {
                let (selected, outcomes) = select_objects(context, containers);
                if !outcomes.not_evaluated_outcomes().is_empty() {
                    // An undecided object may be the space two elements share.
                    for object in checked {
                        evaluation.push_object_not_evaluated(
                            object.id.clone(),
                            NotEvaluatedReason::InvalidEvidence,
                            "container selector was not evaluated conclusively",
                        );
                    }
                    return evaluation;
                }
                selected
                    .into_iter()
                    .map(|object| object.id.clone())
                    .collect()
            }
            _ => BTreeSet::new(),
        };
        let mut judge = Judge {
            context,
            rule,
            config: &config,
            universe: &universe,
            containers: &containers,
            nearest: BTreeMap::new(),
            levels: match &config.mode {
                Mode::SameContainer {
                    levels: Some((relationship, property)),
                    ..
                } => Levels::parse(relationship, *property).ok(),
                _ => None,
            },
        };
        for object in checked {
            let mut outcome = Outcome::default();
            judge.object(object, &mut outcome);
            outcome.flush(context, &config, object, &mut evaluation);
        }
        evaluation
    }
}

struct Config<'a> {
    selector: &'a Selector,
    compared: Option<PropertyRef<'a>>,
    target: Target<'a>,
    operator: Operator,
    /// The operator as declared, for messages.
    operator_name: &'a str,
    case_sensitive: bool,
    /// Declared `like` or `matches` patterns, compiled once; empty when the
    /// pattern is a property of the checked object.
    patterns: Vec<Regex>,
    factor: f64,
    mode: Mode<'a>,
    quantifier: Quantifier,
    tolerance: Tolerance,
    /// How finely dates and date-times compare.
    precision: Option<TemporalPrecision>,
    category: Option<PropertyRef<'a>>,
}

impl<'a> Config<'a> {
    #[allow(clippy::too_many_lines)]
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let selector = parameters.required_selector("compared_selector")?;
        let compared = parameters.property("compared_property")?;
        let operator_name = parameters.required_string("operator")?;
        let operator = match operator_name {
            "equals" => Operator::Equals,
            "not_equals" => Operator::NotEquals,
            "greater" => Operator::Greater,
            "greater_or_equal" => Operator::GreaterOrEqual,
            "less" => Operator::Less,
            "less_or_equal" => Operator::LessOrEqual,
            "contains" => Operator::Contains,
            "one_of" => Operator::OneOf,
            "none_of" => Operator::NoneOf,
            "like" => Operator::Like,
            "matches" => Operator::Matches,
            "is_defined" => Operator::IsDefined,
            "is_undefined" => Operator::IsUndefined,
            "between" => Operator::Between,
            other => return Err(invalid(format!("operator `{other}` is unsupported"))),
        };
        let target = Self::target(&parameters)?;
        let fits = match (&target, operator) {
            (Target::None, op) => op.judges_presence(),
            (_, op) if op.judges_presence() => false,
            (Target::Range(..), op) => op == Operator::Between,
            (_, Operator::Between) => false,
            (Target::Texts(_), op) => matches!(
                op,
                Operator::OneOf
                    | Operator::NoneOf
                    | Operator::Like
                    | Operator::Matches
                    | Operator::Contains
            ),
            (_, Operator::OneOf | Operator::NoneOf) => false,
            (Target::Value(value), Operator::Like | Operator::Matches | Operator::Contains) => {
                matches!(value, PropertyValue::String(_))
            }
            _ => true,
        };
        if !fits {
            return Err(invalid(format!(
                "operator `{operator_name}` does not take the declared target"
            )));
        }
        let factor = parameters
            .number("factor")?
            .ok_or_else(|| invalid("parameter `factor` is required"))?;
        let quantifier = match parameters.required_string("quantifier")? {
            "each" => Quantifier::Each,
            "at_least_one" => Quantifier::AtLeastOne,
            "count" => Quantifier::Count,
            "sum" => Quantifier::Sum,
            other => return Err(invalid(format!("quantifier `{other}` is unsupported"))),
        };
        if compared.is_none() && !matches!(quantifier, Quantifier::Count) {
            return Err(invalid(
                "`compared_property` is required except for `count`",
            ));
        }
        // A count or sum is one number: it takes neither text operators nor presence.
        if matches!(quantifier, Quantifier::Count | Quantifier::Sum)
            && (operator.textual() || operator.judges_presence())
        {
            return Err(invalid(format!(
                "operator `{operator_name}` does not apply to a count or sum"
            )));
        }
        let tolerance = parameters.tolerance()?;
        // A tolerance is numeric; a text or boolean constant cannot take one.
        if !tolerance.is_exact()
            && (operator.textual()
                || operator.judges_presence()
                || matches!(
                    target,
                    Target::Value(
                        PropertyValue::String(_)
                            | PropertyValue::Boolean(_)
                            | PropertyValue::Date(_)
                            | PropertyValue::DateTime(_)
                    )
                ))
        {
            return Err(invalid("a tolerance applies to numbers only"));
        }
        let precision = parameters.precision()?;
        if precision.is_some()
            && (operator.textual()
                || operator.judges_presence()
                || matches!(quantifier, Quantifier::Count | Quantifier::Sum)
                || matches!(&target, Target::Value(value) if !temporal(value))
                || matches!(target, Target::Texts(_) | Target::Range(..)))
        {
            return Err(invalid(
                "`precision` applies to comparing dates and date-times only",
            ));
        }
        let traversal = parameters.traversal()?;
        let container_selector = parameters.selector("container_selector")?;
        let container_relationship = parameters.string("container_relationship")?;
        let levels = crate::levels::declared(
            container_relationship,
            parameters.property("level_property")?,
        )?;
        let mode = match parameters.required_string("component_mode")? {
            "checked" => Mode::Checked,
            "shared" => {
                if parameters.strings("path")?.is_some() {
                    return Err(invalid(
                        "a shared group is one `relationship`, not a `path`",
                    ));
                }
                let relationship = parameters.required_string("relationship")?;
                if relationship.is_empty() {
                    return Err(invalid("`relationship` is empty"));
                }
                let absent_ends =
                    if parameters.boolean("skip_absent_relationship_ends")? == Some(true) {
                        AbsentEndPolicy::Skip
                    } else {
                        AbsentEndPolicy::Refuse
                    };
                Mode::Shared(relationship, absent_ends)
            }
            "related" => Mode::Related(
                traversal.ok_or_else(|| invalid("`related` needs `relationship` or `path`"))?,
            ),
            "same_space" | "same_building" => {
                let traversal = traversal.ok_or_else(|| {
                    invalid("a container mode needs `relationship` or `path` to climb")
                })?;
                if traversal.follows_chain() {
                    return Err(invalid(
                        "a container mode climbs transitively; `follow_chain` does not apply",
                    ));
                }
                Mode::SameContainer {
                    traversal,
                    levels,
                    containers: container_selector
                        .ok_or_else(|| invalid("a container mode needs `container_selector`"))?,
                }
            }
            other => return Err(invalid(format!("component mode `{other}` is unsupported"))),
        };
        if (container_selector.is_some() || container_relationship.is_some())
            && !matches!(mode, Mode::SameContainer { .. })
        {
            return Err(invalid(
                "`container_selector` and `container_relationship` apply to `same_space` and `same_building` only",
            ));
        }
        let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
        let declared: &[String] = match &target {
            Target::Texts(texts) => texts,
            Target::Value(PropertyValue::String(text)) => std::slice::from_ref(text),
            _ => &[],
        };
        let patterns = if matches!(operator, Operator::Like | Operator::Matches) {
            declared
                .iter()
                .map(|pattern| text_pattern(operator, pattern, case_sensitive))
                .collect::<Result<Vec<_>, String>>()
                .map_err(invalid)?
        } else {
            Vec::new()
        };
        Ok(Self {
            selector,
            compared,
            target,
            operator,
            operator_name,
            case_sensitive,
            patterns,
            factor,
            mode,
            quantifier,
            tolerance,
            precision,
            category: parameters.property("category_property")?,
        })
    }

    /// The one declared target; none at all is `Target::None`.
    fn target(parameters: &Parameters<'a>) -> Result<Target<'a>, Unavailable> {
        let mut targets = Vec::new();
        if let Some(property) = parameters.property("target_property")? {
            targets.push(Target::Property(property));
        }
        if let Some(value) = parameters.number("target_number")? {
            targets.push(Target::Value(PropertyValue::Decimal(value)));
        }
        if let Some((value, dimension)) = parameters.quantity("target_quantity")? {
            targets.push(Target::Value(PropertyValue::Quantity { value, dimension }));
        }
        if let Some(text) = parameters.string("target_text")? {
            targets.push(Target::Value(PropertyValue::String(text.to_owned())));
        }
        if let Some(value) = parameters.boolean("target_boolean")? {
            targets.push(Target::Value(PropertyValue::Boolean(value)));
        }
        if let Some(value) = parameters.date("target_date")? {
            targets.push(Target::Value(PropertyValue::Date(value)));
        }
        if let Some(value) = parameters.date_time("target_date_time")? {
            targets.push(Target::Value(PropertyValue::DateTime(value)));
        }
        if let Some(texts) = parameters.strings("target_texts")? {
            if texts.is_empty() {
                return Err(invalid("`target_texts` is empty"));
            }
            targets.push(Target::Texts(texts));
        }
        let numbers = (
            parameters.number("minimum_number")?,
            parameters.number("maximum_number")?,
        );
        let quantities = (
            parameters.quantity("minimum_quantity")?,
            parameters.quantity("maximum_quantity")?,
        );
        let range = match (numbers, quantities) {
            ((None, None), (None, None)) => None,
            ((Some(low), Some(high)), (None, None)) => Some((
                low,
                high,
                PropertyValue::Decimal(low),
                PropertyValue::Decimal(high),
            )),
            ((None, None), (Some((low, from)), Some((high, to)))) if from == to => Some((
                low,
                high,
                PropertyValue::Quantity {
                    value: low,
                    dimension: from,
                },
                PropertyValue::Quantity {
                    value: high,
                    dimension: to,
                },
            )),
            _ => {
                return Err(invalid(
                    "a range needs both bounds, as numbers or as quantities of one dimension",
                ));
            }
        };
        if let Some((low, high, lower, upper)) = range {
            if low > high {
                return Err(invalid("the range minimum is above its maximum"));
            }
            targets.push(Target::Range(lower, upper));
        }
        match <[Target<'a>; 1]>::try_from(targets) {
            Ok([target]) => Ok(target),
            Err(targets) if targets.is_empty() => Ok(Target::None),
            Err(_) => Err(invalid("declare exactly one target")),
        }
    }
}

/// What one checked object produced, held back until its category is known.
#[derive(Default)]
struct Outcome {
    findings: Vec<Finding>,
    unevaluated: Vec<Unavailable>,
}

impl Outcome {
    fn finding(&mut self, rule: &CompiledRule, object: &Object, message: String, e: Vec<Evidence>) {
        self.findings.push(make_finding(rule, object, message, e));
    }

    /// Reports the outcome, each finding prefixed with the checked object's
    /// category when one is declared and stated.
    ///
    /// A category that cannot be read leaves the object not evaluated rather
    /// than reporting its findings under the wrong heading.
    fn flush(
        mut self,
        context: &RuleContext<'_>,
        config: &Config<'_>,
        object: &Object,
        evaluation: &mut CapabilityEvaluation,
    ) {
        if let (Some(category), false) = (config.category, self.findings.is_empty()) {
            match crate::support::category_prefix(context, object, category) {
                Ok((prefix, cited)) => {
                    for finding in &mut self.findings {
                        finding.message.insert_str(0, &prefix);
                        finding.evidence = combined(&finding.evidence, &cited, &[]);
                    }
                }
                Err((reason, message)) => {
                    self.findings.clear();
                    self.unevaluated.push((
                        reason,
                        format!("finding category {category} could not be read: {message}"),
                    ));
                }
            }
        }
        for finding in self.findings {
            evaluation.push_finding(finding);
        }
        for (reason, message) in self.unevaluated {
            evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
        }
    }
}

type Containers = Result<(BTreeSet<ObjectId>, Vec<Evidence>), Unavailable>;

/// Judges checked objects one by one, remembering each object's containers.
struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    config: &'r Config<'r>,
    universe: &'r [&'c Object],
    containers: &'r BTreeSet<ObjectId>,
    nearest: BTreeMap<ObjectId, Containers>,
    /// The level match of `container_relationship`, when declared.
    levels: Option<Levels<'r>>,
}

impl Judge<'_, '_> {
    fn object(&mut self, object: &Object, outcome: &mut Outcome) {
        let (candidates, relation_evidence) = match self.candidates(object) {
            Ok(value) => value,
            Err(unavailable) => {
                outcome.unevaluated.push(unavailable);
                return;
            }
        };
        if matches!(self.config.quantifier, Quantifier::Count | Quantifier::Sum) {
            aggregate(
                self.context,
                self.rule,
                object,
                &candidates,
                &relation_evidence,
                self.config,
                outcome,
            );
        } else {
            each(
                self.context,
                self.rule,
                object,
                candidates,
                &relation_evidence,
                self.config,
                outcome,
            );
        }
    }

    fn candidates(
        &mut self,
        object: &Object,
    ) -> Result<(Vec<ObjectId>, Vec<Evidence>), Unavailable> {
        match &self.config.mode {
            Mode::Checked => Ok((
                if self
                    .universe
                    .binary_search_by_key(&&object.id, |candidate| &candidate.id)
                    .is_ok()
                {
                    vec![object.id.clone()]
                } else {
                    vec![]
                },
                vec![],
            )),
            Mode::Shared(relationship, absent_ends) => shared_group(
                self.context,
                object,
                self.universe,
                relationship,
                *absent_ends,
            ),
            Mode::Related(traversal) => traversal.related(self.context, &object.id, self.universe),
            Mode::SameContainer { .. } => {
                let (mine, mut evidence) = self.containers_of(&object.id)?;
                let mut chosen = Vec::new();
                if mine.is_empty() {
                    return Ok((chosen, evidence));
                }
                for candidate in self.universe {
                    if candidate.id == object.id {
                        continue;
                    }
                    let (theirs, cited) = self.containers_of(&candidate.id)?;
                    let shared = match &mut self.levels {
                        None => !theirs.is_disjoint(&mine),
                        Some(levels) => {
                            let (shared, matched) = levels.overlap(self.context, &mine, &theirs)?;
                            if shared {
                                evidence.extend(matched);
                            }
                            shared
                        }
                    };
                    if shared {
                        chosen.push(candidate.id.clone());
                        evidence.extend(cited);
                    }
                }
                Ok((chosen, combined(&evidence, &[], &[])))
            }
        }
    }

    /// The nearest containers of `object`, climbed once per evaluation.
    fn containers_of(&mut self, object: &ObjectId) -> Containers {
        let Mode::SameContainer { traversal, .. } = &self.config.mode else {
            unreachable!("only container modes climb");
        };
        self.nearest
            .entry(object.clone())
            .or_insert_with(|| traversal.nearest_containers(self.context, object, self.containers))
            .clone()
    }
}

/// `each` and `at_least_one`: every candidate compared on its own.
#[allow(clippy::too_many_lines)]
fn each(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    object: &Object,
    candidates: Vec<ObjectId>,
    relation_evidence: &[Evidence],
    config: &Config<'_>,
    outcome: &mut Outcome,
) {
    if candidates.is_empty() {
        if matches!(config.quantifier, Quantifier::AtLeastOne) {
            outcome.finding(
                rule,
                object,
                "no candidate satisfies comparison".into(),
                relation_evidence.to_vec(),
            );
        }
        return;
    }
    if context
        .services
        .get::<PropertyResolutionServiceHandle>()
        .is_none()
    {
        outcome.unevaluated.push((
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        ));
        return;
    }
    let target = match target_side(context, object, &config.target) {
        Ok((Some(value), _)) => value,
        Ok((None, absence_evidence)) => {
            outcome.finding(
                rule,
                object,
                "target property is absent".into(),
                combined(relation_evidence, &absence_evidence, &[]),
            );
            return;
        }
        Err(unavailable) => {
            outcome.unevaluated.push(unavailable);
            return;
        }
    };
    let compared = config
        .compared
        .expect("parsing requires a compared property here");
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
        let resolved = match resolve(context, candidate, compared) {
            Ok(resolved) => resolved,
            Err(error) => {
                uncertainties.push(error);
                continue;
            }
        };
        let evidence = combined(relation_evidence, &resolved.evidence(), target.evidence());
        let verdict = match (config.operator, resolved.value()) {
            (Operator::IsDefined, value) => Ok(!undefined(value)),
            (Operator::IsUndefined, value) => Ok(undefined(value)),
            (_, None) => {
                missing.push((candidate, evidence));
                continue;
            }
            (_, Some(value)) => compare_side(value, &target, config),
        };
        match verdict {
            Ok(true) => any_match = true,
            Ok(false) => mismatches.push((candidate, evidence)),
            Err(message) => uncertainties.push((NotEvaluatedReason::InvalidEvidence, message)),
        }
    }
    let has_missing_information = !missing.is_empty();
    for (candidate, evidence) in missing {
        outcome.finding(
            rule,
            candidate,
            "compared property is absent".into(),
            evidence,
        );
    }
    match config.quantifier {
        Quantifier::Each => {
            for (candidate, evidence) in mismatches {
                outcome.finding(
                    rule,
                    object,
                    format!(
                        "candidate {} does not satisfy comparison{}",
                        candidate.id,
                        config.tolerance.suffix()
                    ),
                    evidence,
                );
            }
            outcome.unevaluated.extend(uncertainties);
        }
        // Handled by `aggregate`.
        Quantifier::Count | Quantifier::Sum => unreachable!(),
        Quantifier::AtLeastOne if any_match || has_missing_information => {}
        Quantifier::AtLeastOne => {
            if uncertainties.is_empty() {
                let mut evidence = combined(relation_evidence, target.evidence(), &[]);
                for (_, mismatch_evidence) in &mismatches {
                    evidence = combined(&evidence, mismatch_evidence, &[]);
                }
                outcome.finding(
                    rule,
                    object,
                    format!(
                        "no candidate satisfies comparison{}",
                        config.tolerance.suffix()
                    ),
                    evidence,
                );
            } else {
                outcome.unevaluated.extend(uncertainties);
            }
        }
    }
}

/// The evaluated right-hand side of a comparison.
enum Side<'a> {
    None,
    Value(PropertyValue, Vec<Evidence>),
    Texts(&'a [String]),
    Range(PropertyValue, PropertyValue),
}

impl Side<'_> {
    fn evidence(&self) -> &[Evidence] {
        match self {
            Self::Value(_, evidence) => evidence,
            Self::None | Self::Texts(_) | Self::Range(..) => &[],
        }
    }

    fn describe(&self, factor: &str) -> String {
        match self {
            Self::None => String::new(),
            Self::Value(value, _) => format!("{factor}{}", crate::support::display(Some(value))),
            Self::Texts(texts) => format!("[{}]", texts.join(", ")),
            Self::Range(lower, upper) => format!(
                "{factor}{} and {factor}{}",
                crate::support::display(Some(lower)),
                crate::support::display(Some(upper))
            ),
        }
    }
}

/// The target for `object`: its own property, or the declared constant.
fn target_side<'a>(
    context: &RuleContext<'_>,
    object: &Object,
    target: &Target<'a>,
) -> Result<(Option<Side<'a>>, Vec<Evidence>), Unavailable> {
    match target {
        Target::None => Ok((Some(Side::None), Vec::new())),
        Target::Property(property) => {
            let resolved = resolve(context, object, *property)?;
            Ok(match resolved.value() {
                Some(value) => (
                    Some(Side::Value(value.clone(), resolved.evidence())),
                    vec![],
                ),
                None => (None, resolved.evidence()),
            })
        }
        Target::Value(value) => Ok((Some(Side::Value(value.clone(), Vec::new())), Vec::new())),
        Target::Texts(texts) => Ok((Some(Side::Texts(texts)), Vec::new())),
        Target::Range(lower, upper) => {
            Ok((Some(Side::Range(lower.clone(), upper.clone())), Vec::new()))
        }
    }
}

fn compare_side(
    left: &PropertyValue,
    side: &Side<'_>,
    config: &Config<'_>,
) -> Result<bool, String> {
    let at = |right: &PropertyValue, operator: Operator| {
        if let Some(ordering) = temporal_order(left, right, config.precision) {
            if !exact_one(config.factor) {
                return Err("a factor does not apply to dates".into());
            }
            return operator
                .orders(ordering?)
                .ok_or_else(|| format!("`{}` does not compare dates", config.operator_name));
        }
        if config.precision.is_some() {
            return Err("`precision` applies to comparing dates and date-times only".into());
        }
        compare(
            left,
            right,
            config.factor,
            operator,
            &config.tolerance,
            config.case_sensitive,
        )
    };
    match side {
        Side::None => Err("the operator takes no target".into()),
        Side::Value(right, _) if matches!(config.operator, Operator::Like | Operator::Matches) => {
            let (PropertyValue::String(text), PropertyValue::String(pattern)) = (left, right)
            else {
                return Err(format!(
                    "`{}` compares text values only",
                    config.operator_name
                ));
            };
            if config.patterns.is_empty() {
                // The pattern is the checked object's own value.
                Ok(text_pattern(config.operator, pattern, config.case_sensitive)?.is_match(text))
            } else {
                Ok(config.patterns.iter().any(|regex| regex.is_match(text)))
            }
        }
        Side::Value(right, _) => at(right, config.operator),
        Side::Range(lower, upper) => {
            Ok(at(lower, Operator::GreaterOrEqual)? && at(upper, Operator::LessOrEqual)?)
        }
        Side::Texts(texts) => {
            let PropertyValue::String(text) = left else {
                return Err(format!(
                    "`{}` compares text values only",
                    config.operator_name
                ));
            };
            let fold = |value: &str| {
                if config.case_sensitive {
                    value.to_owned()
                } else {
                    value.to_lowercase()
                }
            };
            if matches!(config.operator, Operator::Like | Operator::Matches) {
                return Ok(config.patterns.iter().any(|regex| regex.is_match(text)));
            }
            let text = fold(text);
            let any = |test: &dyn Fn(&str) -> bool| texts.iter().any(|item| test(item));
            Ok(match config.operator {
                Operator::OneOf => any(&|item| fold(item) == text),
                Operator::NoneOf => !any(&|item| fold(item) == text),
                Operator::Contains => any(&|item| text.contains(&fold(item))),
                _ => {
                    return Err(
                        "a text list takes one_of, none_of, contains, like or matches".into(),
                    );
                }
            })
        }
    }
}

/// A `like` or `matches` pattern compiled as the property selectors compile
/// it: anchored to the whole value, case folded unless `case_sensitive`.
fn text_pattern(operator: Operator, pattern: &str, case_sensitive: bool) -> Result<Regex, String> {
    let (source, kind) = if let Operator::Like = operator {
        (crate::selection::wildcard(pattern)?, "wildcard pattern")
    } else {
        (format!("^(?:{pattern})$"), "regular expression")
    };
    RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| format!("invalid {kind}: {error}"))
}

/// `count` and `sum`: one number from all candidates, compared once.
///
/// A `sum` over a candidate whose compared property is absent is not a sum
/// of the model: each such candidate gets the same missing-property finding
/// as elsewhere, and no verdict is drawn from the partial total.
#[allow(clippy::too_many_lines)]
fn aggregate(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    object: &Object,
    candidates: &[ObjectId],
    relation_evidence: &[Evidence],
    config: &Config<'_>,
    outcome: &mut Outcome,
) {
    if context
        .services
        .get::<PropertyResolutionServiceHandle>()
        .is_none()
    {
        outcome.unevaluated.push((
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        ));
        return;
    }
    let mut evidence = relation_evidence.to_vec();
    let (label, left) = if let Quantifier::Count = config.quantifier {
        let Ok(count) = i64::try_from(candidates.len()) else {
            outcome.unevaluated.push((
                NotEvaluatedReason::ResourceLimit,
                "candidate count exceeds the integer range".into(),
            ));
            return;
        };
        (
            "count of compared components",
            PropertyValue::Integer(count),
        )
    } else {
        let compared = config
            .compared
            .expect("parsing requires a compared property for sum");
        let mut values = Vec::new();
        let mut incomplete = false;
        for candidate_id in candidates {
            let Some(candidate) = context.project.object(candidate_id) else {
                outcome.unevaluated.push((
                    NotEvaluatedReason::InvalidEvidence,
                    "relationship candidate is absent from project".into(),
                ));
                return;
            };
            match resolve(context, candidate, compared) {
                Ok(resolved) => {
                    if let Some(value) = resolved.value() {
                        values.push(value.clone());
                        evidence.extend(resolved.evidence());
                    } else {
                        incomplete = true;
                        outcome.finding(
                            rule,
                            candidate,
                            "compared property is absent".into(),
                            combined(relation_evidence, &resolved.evidence(), &[]),
                        );
                    }
                }
                Err(unavailable) => {
                    outcome.unevaluated.push(unavailable);
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
                outcome
                    .unevaluated
                    .push((NotEvaluatedReason::InvalidEvidence, message));
                return;
            }
        }
    };
    let target = match target_side(context, object, &config.target) {
        Ok((Some(target), _)) => target,
        Ok((None, absence)) => {
            outcome.finding(
                rule,
                object,
                "target property is absent".into(),
                combined(relation_evidence, &absence, &[]),
            );
            return;
        }
        Err(unavailable) => {
            outcome.unevaluated.push(unavailable);
            return;
        }
    };
    match compare_side(&left, &target, config) {
        Ok(true) => {}
        Ok(false) => {
            let factor = if exact_one(config.factor) {
                String::new()
            } else {
                format!("{} x ", config.factor)
            };
            outcome.finding(
                rule,
                object,
                format!(
                    "{label} is {} and is not {} {}{}",
                    crate::support::display(Some(&left)),
                    config.operator_name,
                    target.describe(&factor),
                    config.tolerance.suffix()
                ),
                combined(&evidence, target.evidence(), &[]),
            );
        }
        Err(message) => outcome
            .unevaluated
            .push((NotEvaluatedReason::InvalidEvidence, message)),
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

/// Members of a group the checked object shares, through one relationship.
fn shared_group(
    context: &RuleContext<'_>,
    object: &Object,
    universe: &[&Object],
    relationship: &str,
    absent_ends: AbsentEndPolicy,
) -> Result<(Vec<ObjectId>, Vec<Evidence>), Unavailable> {
    let Some(service) = context.services.get::<RelationshipSelectionServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "relationship-selection service is not registered".into(),
        ));
    };
    let relationship =
        SemanticRelationship::try_new(relationship).map_err(|error| invalid(error.to_string()))?;
    let request = RelationshipSelectionRequest::try_new(
        object.id.clone(),
        universe.iter().map(|item| item.id.clone()).collect(),
        RelationshipQuery::SharedGroup { relationship },
    )
    .map_err(|error| invalid(error.to_string()))?
    .with_absent_ends(absent_ends);
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
    tolerance: &Tolerance,
    case_sensitive: bool,
) -> Result<bool, String> {
    let equal = |ord: Ordering| operator.orders(ord).unwrap_or(false);
    match (left, right) {
        (PropertyValue::Boolean(a), PropertyValue::Boolean(b)) if exact_one(factor) => {
            match operator {
                Operator::Equals => Ok(a == b),
                Operator::NotEquals => Ok(a != b),
                _ => Err("boolean comparison operator is invalid".into()),
            }
        }
        (PropertyValue::String(a), PropertyValue::String(b)) if exact_one(factor) => {
            let (a, b) = if case_sensitive {
                (a.clone(), b.clone())
            } else {
                (a.to_lowercase(), b.to_lowercase())
            };
            match operator {
                Operator::Equals => Ok(a == b),
                Operator::NotEquals => Ok(a != b),
                Operator::Contains => Ok(a.contains(&b)),
                _ => Err("string comparison operator is invalid".into()),
            }
        }
        (PropertyValue::Integer(a), PropertyValue::Integer(b))
            if exact_one(factor) && tolerance.is_exact() =>
        {
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
                numeric(*a, *b, factor, tolerance, equal)
            } else {
                Err("quantity value is non-finite".into())
            }
        }
        (PropertyValue::Integer(a), PropertyValue::Decimal(b))
            if (*a).unsigned_abs() <= (1_u64 << 53) && b.is_finite() =>
        {
            numeric(integer_to_f64(*a)?, *b, factor, tolerance, equal)
        }
        (PropertyValue::Decimal(a), PropertyValue::Integer(b))
            if (*b).unsigned_abs() <= (1_u64 << 53) && a.is_finite() =>
        {
            numeric(*a, integer_to_f64(*b)?, factor, tolerance, equal)
        }
        (PropertyValue::Decimal(a), PropertyValue::Decimal(b))
            if a.is_finite() && b.is_finite() =>
        {
            numeric(*a, *b, factor, tolerance, equal)
        }
        (PropertyValue::Integer(a), PropertyValue::Integer(b)) if !tolerance.is_exact() => numeric(
            integer_to_f64(*a)?,
            integer_to_f64(*b)?,
            factor,
            tolerance,
            equal,
        ),
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
    tolerance: &Tolerance,
    predicate: impl FnOnce(Ordering) -> bool,
) -> Result<bool, String> {
    let scaled = right * factor;
    if scaled.is_finite() {
        let ordering = if tolerance.is_exact() {
            left.total_cmp(&scaled)
        } else {
            tolerance
                .order(left, scaled)
                .ok_or("compared value is non-finite")?
        };
        Ok(predicate(ordering))
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
        scope: axioval_ir::Scope::Object(object.id.clone()),
        related: Vec::new(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
        location: None,
    }
}
