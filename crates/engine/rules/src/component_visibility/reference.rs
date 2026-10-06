//! `component-visibility` as it was implemented before it became a
//! template (#283), kept only as the parity reference the template is held
//! to in the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry. Its views are the ones the template's
//! measured `sight_view` reads (`View::of`).

use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::Selector;

use super::{NAME, Picked, Services, View};
use crate::pairs::refuse_all;
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Requires targets within a radius to be in view from an eye point above
/// each selected component, as `component-visibility` judged it before it
/// became a template.
pub struct ComponentVisibility;

/// What the rule requires.
#[derive(Clone, Copy)]
enum Mode {
    AtLeast(u64),
    None,
}

struct Config<'a> {
    targets: &'a Selector,
    blockers: &'a Selector,
    eye_height: f64,
    radius: f64,
    mode: Mode,
}

impl RuleCapability for ComponentVisibility {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (components, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context) {
            Ok(services) => services,
            Err((reason, message)) => {
                return refuse_all(&components, evaluation, &reason, &message);
            }
        };
        let targets = Picked::of(context, config.targets);
        let blockers = Picked::of(context, config.blockers);
        let mut evaluation = evaluation;
        for component in components {
            match View::of(
                &services,
                (config.eye_height, config.radius),
                &targets,
                &blockers,
                component,
            ) {
                Ok(view) => judge(view, &component.id, rule, &config, &mut evaluation),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(component.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => Ok(value),
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
        None => Err(invalid(format!("{name} is required"))),
    };
    let minimum = parameters.integer("minimum")?;
    let mode = match parameters.string("mode")? {
        Some("at-least") => match minimum {
            None => Mode::AtLeast(1),
            Some(minimum) => Mode::AtLeast(
                u64::try_from(minimum)
                    .ok()
                    .filter(|minimum| *minimum > 0)
                    .ok_or_else(|| invalid("minimum must be a positive count"))?,
            ),
        },
        Some("none") if minimum.is_some() => {
            return Err(invalid("minimum applies only to mode `at-least`"));
        }
        Some("none") => Mode::None,
        Some(other) => {
            return Err(invalid(format!(
                "mode `{other}` is unsupported; use `at-least` or `none`"
            )));
        }
        None => return Err(invalid("mode is required")),
    };
    Ok(Config {
        targets: parameters.required_selector("targets")?,
        blockers: parameters.required_selector("blockers")?,
        eye_height: length("eye_height")?,
        radius: length("radius")?,
        mode,
    })
}

fn judge(
    view: View,
    id: &axioval_ir::ObjectId,
    rule: &CompiledRule,
    config: &Config<'_>,
    evaluation: &mut CapabilityEvaluation,
) {
    let sure = view.visible.len() as u64;
    let most = sure + view.unknown.len() as u64;
    let within = format!(
        "within {} m of the eye {} m above the base of {id}",
        config.radius, config.eye_height
    );
    let undecided = || {
        let mut message = format!("{} target(s) {within} are undecided:", view.unknown.len());
        for (target, why) in view.unknown.iter().take(3) {
            let _ = write!(message, " {target} {why};");
        }
        message.pop();
        message
    };
    match config.mode {
        Mode::AtLeast(minimum) => {
            if sure >= minimum {
                return;
            }
            if most >= minimum {
                evaluation.push_object_not_evaluated(
                    id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{sure} target(s) are in view, {minimum} required; {}",
                        undecided()
                    ),
                );
                return;
            }
            let mut message =
                format!("{sure} target(s) {within} are in view; required at least {minimum}");
            if !view.hidden.is_empty() {
                let _ = write!(message, "; {} hidden", view.hidden.len());
            }
            let mut related = view.visible.clone();
            related.extend(view.hidden.iter().cloned());
            evaluation.push_finding(finding(rule, id, message, view.evidence, related));
        }
        Mode::None => {
            if sure > 0 {
                let named = view
                    .visible
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                evaluation.push_finding(finding(
                    rule,
                    id,
                    format!("{sure} target(s) {within} are in view, none allowed: {named}"),
                    view.evidence,
                    view.visible,
                ));
            } else if most > 0 {
                evaluation.push_object_not_evaluated(
                    id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    undecided(),
                );
            }
        }
    }
}
