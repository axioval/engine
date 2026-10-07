//! Numbers read from a pattern that must agree within a scope.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};
use regex::Regex;

use crate::support::{
    Parameters, PropertyRef, Unavailable, display, invalid, resolve, scope_key, undefined,
};
use crate::xsd_pattern;

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::NumberingMeasures;

pub(crate) const NAME: &str = "numbering-consistency";

/// Requires the numbers of one scope to share a prefix and leave no gaps.
///
/// The typical use is the space numbers of one storey: `B-101`, `B-102`,
/// `B-104` read through the pattern `B-(\d+)` give 101, 102 and 104. The
/// `pattern` is an XML Schema pattern over the whole value with exactly one
/// group, which must capture ASCII digits. Scopes are formed as in
/// `unique-value`: one source, the project with `across_sources`, or the
/// objects reaching the same related objects through a declared traversal.
///
/// With `prefix_length`, the first that many digits must be the same across
/// the scope; the objects departing from the prefix most objects share are
/// reported, and when no prefix predominates every object of the scope is.
/// With `gap_free`, the distinct numbers, sorted, must step by one; the
/// objects above each gap are reported. At least one check must be declared.
///
/// A value that is absent, blank, not text or a whole number, or does not
/// match the pattern is not a number: its object is not evaluated, never
/// passed. An object whose value or scope cannot be read might fill a gap or
/// shift the predominant prefix, so a gap it could fill is not evaluated
/// rather than reported.
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `numbering` list of each object, its prefix's lead over the
/// other prefixes of its scope and its number's step from the one below,
/// judged by the template.
pub struct NumberingConsistency;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for NumberingConsistency {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("property", ParameterType::PropertyReference),
        ParameterDescriptor::required("pattern", ParameterType::String),
        ParameterDescriptor::optional("prefix_length", ParameterType::Integer),
        ParameterDescriptor::optional("gap_free", ParameterType::Boolean),
        ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
    ]
    .into_iter()
    .chain(crate::support::traversal_parameters())
    .collect()
}

pub(crate) struct Config<'a> {
    pub(crate) property: PropertyRef<'a>,
    pub(crate) pattern: Regex,
    pub(crate) prefix_length: Option<usize>,
    pub(crate) gap_free: bool,
    pub(crate) across_sources: bool,
    pub(crate) traversal: Option<crate::support::Traversal>,
}

impl<'a> Config<'a> {
    pub(crate) fn read(parameters: &Parameters<'a>) -> Result<Self, Unavailable> {
        let source = parameters.required_string("pattern")?;
        let pattern = xsd_pattern::compile(source)
            .map_err(|error| invalid(format!("pattern {source:?}: {error}")))?;
        if pattern.captures_len() != 2 {
            return Err(invalid(format!(
                "pattern {source:?} must have exactly one group, the number"
            )));
        }
        let prefix_length = match parameters.integer("prefix_length")? {
            None => None,
            Some(length) => Some(
                usize::try_from(length)
                    .ok()
                    .filter(|length| *length > 0)
                    .ok_or_else(|| invalid("prefix_length must be positive"))?,
            ),
        };
        let gap_free = parameters.boolean("gap_free")?.unwrap_or(false);
        if prefix_length.is_none() && !gap_free {
            return Err(invalid(
                "declare `prefix_length`, `gap_free` or both; nothing is checked otherwise",
            ));
        }
        Ok(Self {
            property: parameters.required_property("property")?,
            pattern,
            prefix_length,
            gap_free,
            across_sources: parameters.boolean("across_sources")?.unwrap_or(false),
            traversal: parameters.traversal()?,
        })
    }

    /// The digits the pattern captures from a value, or why there are none.
    fn digits(&self, value: Option<&PropertyValue>) -> Result<String, &'static str> {
        if undefined(value) {
            return Err("is not set, so it has no number");
        }
        let text = match value {
            Some(PropertyValue::String(text)) => text.clone(),
            Some(PropertyValue::Integer(number)) => number.to_string(),
            _ => return Err("is neither text nor a whole number"),
        };
        let captured = self
            .pattern
            .captures(&text)
            .ok_or("does not match the pattern")?
            .get(1)
            .ok_or("does not capture a number")?
            .as_str();
        if captured.is_empty() || !captured.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("captures something other than digits");
        }
        Ok(captured.to_owned())
    }
}

/// Checks the rule parameters the measured `numbering` is handed, as the
/// rule states them: the capability's declaration, in its order and words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    Config::read(&Parameters(&rule)).map(|_| ())
}

pub(crate) struct Member<'a> {
    pub(crate) object: &'a Object,
    pub(crate) shown: String,
    pub(crate) digits: String,
    pub(crate) number: u64,
    pub(crate) evidence: Vec<Evidence>,
}

pub(crate) struct Scope<'a> {
    pub(crate) source: String,
    pub(crate) members: Vec<Member<'a>>,
    /// Objects of this scope whose number could not be read.
    pub(crate) undecided: usize,
}

impl Scope<'_> {
    fn new(source: String) -> Self {
        Self {
            source,
            members: Vec::new(),
            undecided: 0,
        }
    }
}

/// The numbers of the selected objects, read as the capability reads them:
/// each scope's members, the objects in no known scope by source key (with
/// their number where read), and why each object whose number or scope
/// cannot be read is left open.
pub(crate) struct Collected<'a> {
    pub(crate) scopes: BTreeMap<String, Scope<'a>>,
    pub(crate) stray: Vec<(String, Option<u64>)>,
    pub(crate) open: Vec<(ObjectId, NotEvaluatedReason, String)>,
}

impl<'a> Collected<'a> {
    /// Reads `selected`, in order.
    pub(crate) fn of(
        context: &RuleContext<'_>,
        config: &Config<'_>,
        selected: &[&'a Object],
    ) -> Self {
        let mut collected = Self {
            scopes: BTreeMap::new(),
            stray: Vec::new(),
            open: Vec::new(),
        };
        for object in selected {
            let object: &'a Object = object;
            let source = if config.across_sources {
                String::new()
            } else {
                object.id.source.to_string()
            };
            let scope = |context: &RuleContext<'_>| {
                scope_key(
                    context,
                    config.traversal.as_ref(),
                    config.across_sources,
                    object,
                )
            };
            let resolved = match resolve(context, object, config.property) {
                Ok(resolved) => resolved,
                Err((reason, message)) => {
                    collected.open.push((object.id.clone(), reason, message));
                    match scope(context) {
                        Ok((key, _)) => {
                            collected
                                .scopes
                                .entry(key)
                                .or_insert_with(|| Scope::new(source))
                                .undecided += 1;
                        }
                        Err(_) => collected.stray.push((source, None)),
                    }
                    continue;
                }
            };
            let value = resolved.value();
            let digits = match config.digits(value) {
                Ok(digits) => digits,
                Err(message) => {
                    collected.open.push((
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{} {} {message}", config.property, display(value)),
                    ));
                    continue;
                }
            };
            let Ok(number) = digits.parse::<u64>() else {
                collected.open.push((
                    object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} {} holds a number too large to compare",
                        config.property,
                        display(value)
                    ),
                ));
                continue;
            };
            let (key, mut evidence) = match scope(context) {
                Ok(scope) => scope,
                Err((reason, message)) => {
                    collected.open.push((object.id.clone(), reason, message));
                    collected.stray.push((source, Some(number)));
                    continue;
                }
            };
            evidence.extend(resolved.evidence());
            collected
                .scopes
                .entry(key)
                .or_insert_with(|| Scope::new(source))
                .members
                .push(Member {
                    object,
                    shown: display(value),
                    digits,
                    number,
                    evidence,
                });
        }
        collected
    }

    /// The strays of `scope`'s source.
    pub(crate) fn strays(&self, scope: &Scope<'_>) -> Vec<Option<u64>> {
        self.stray
            .iter()
            .filter(|(source, _)| *source == scope.source)
            .map(|(_, number)| *number)
            .collect()
    }
}
