//! The `opening-area` implementation the template replaced, kept as the
//! template's parity reference.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use super::{Openings, Picks, Voided, voided};
use crate::counts::Population;
use crate::opening_zone::face::ROUNDING;
use crate::selection::select_objects;
use crate::support::{Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve};

/// The `opening-area` capability as it was before it ran as a template.
pub struct OpeningArea;

struct Config<'a> {
    openings: Openings<'a>,
    gross: PropertyRef<'a>,
    net: PropertyRef<'a>,
    tolerance: f64,
}

/// Reads `area_tolerance`, a non-negative area, zero when absent.
pub(crate) fn area_tolerance(parameters: &Parameters<'_>) -> Result<f64, Unavailable> {
    match parameters.quantity("area_tolerance")? {
        None => Ok(0.0),
        Some((value, QuantityDimension::Area)) if value >= 0.0 => Ok(value),
        Some(_) => Err(invalid("`area_tolerance` is not a non-negative area")),
    }
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        Ok(Self {
            openings: Openings::parse(&parameters)?,
            gross: parameters.required_property("gross_area")?,
            net: parameters.required_property("net_area")?,
            tolerance: area_tolerance(&parameters)?,
        })
    }
}

impl RuleCapability for OpeningArea {
    fn id(&self) -> &'static str {
        "axioval:capability.opening-area"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = Openings::parameters();
        parameters.extend([
            ParameterDescriptor::required("gross_area", ParameterType::PropertyReference),
            ParameterDescriptor::required("net_area", ParameterType::PropertyReference),
            ParameterDescriptor::optional("area_tolerance", ParameterType::Quantity),
        ]);
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("opening-area: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let openings = Population::of(context, config.openings.selector);
        for host in selected {
            match check(context, &config, &openings, host) {
                Ok(None) => {}
                Ok(Some((message, evidence, related))) => {
                    evaluation.push_finding(finding(rule, &host.id, message, evidence, related));
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(host.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Mismatch = (String, Vec<Evidence>, Vec<ObjectId>);

/// A stated area, `None` when exactly absent.
fn area(
    context: &RuleContext<'_>,
    host: &Object,
    property: PropertyRef<'_>,
    evidence: &mut Vec<Evidence>,
) -> Result<Option<f64>, Unavailable> {
    let resolved = resolve(context, host, property)
        .map_err(|(reason, message)| (reason, format!("`{property}`: {message}")))?;
    evidence.extend(resolved.evidence());
    match resolved.value() {
        None => Ok(None),
        Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Area,
        }) if value.is_finite() => Ok(Some(*value)),
        Some(other) => Err((
            NotEvaluatedReason::InvalidEvidence,
            format!("`{property}` is {}, not an area", display(Some(other))),
        )),
    }
}

pub(crate) fn square_metres(value: f64) -> String {
    format!("{} m²", (value * 1e6).round() / 1e6)
}

/// Checks one host; `Ok(None)` when it passes or is not checked.
fn check(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    openings: &Population,
    host: &Object,
) -> Result<Option<Mismatch>, Unavailable> {
    let mut evidence = Vec::new();
    let gross = area(context, host, config.gross, &mut evidence)?;
    let net = area(context, host, config.net, &mut evidence)?;
    let (gross, net) = match (gross, net) {
        (None, None) => return Ok(None),
        (Some(gross), Some(net)) => (gross, net),
        (Some(_), None) | (None, Some(_)) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it states only one of `{}` and `{}`",
                    config.gross, config.net
                ),
            ));
        }
    };
    let Voided { sum, reached, .. } = voided(
        context,
        &config.openings,
        Picks::of(openings),
        host,
        &mut evidence,
    )?;
    let expected = gross - net;
    let slack = config.tolerance + ROUNDING * (1.0 + gross.abs() + net.abs() + sum);
    if (sum - expected).abs() <= slack {
        return Ok(None);
    }
    let described = if reached.is_empty() {
        "it has no openings".to_owned()
    } else {
        format!(
            "its openings ({}) cover {} of its face",
            reached
                .iter()
                .map(|id| id.local_id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            square_metres(sum)
        )
    };
    Ok(Some((
        format!(
            "{described}, but its gross side area {} less its net side area {} is {}; they \
             must agree within {}",
            square_metres(gross),
            square_metres(net),
            square_metres(expected),
            square_metres(config.tolerance)
        ),
        evidence,
        reached,
    )))
}
