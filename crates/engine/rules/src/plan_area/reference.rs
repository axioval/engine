//! `plan-area` as it was implemented before it became a template (#282),
//! kept only as the parity reference the template is held to in the rules
//! crate's tests (`parity-reference` feature). It is no capability of any
//! registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::ReportValue;

use super::{
    Measure, Verdict, area_column, deviation, judge, member_areas, own_area, shown, table,
};
use crate::counts::{Population, relation_text};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid, traversal_parameters};

/// Requires measured plan areas to lie within a range, in square metres.
///
/// Without `member_selector`, each selected object's own footprint must lie
/// within `minimum` and `maximum`, inclusive; at least one is required: a
/// space of at least 8 m², a fire compartment of at most 400 m².
///
/// With `member_selector`, each selected object is an anchor, and the summed
/// footprints of the members it reaches (through the declared relationship,
/// or everywhere in its source, as in `related-count`) must lie within the
/// range: the space area of each storey. Footprints are summed, so
/// overlapping members count twice; select members that do not overlap.
///
/// Areas are intervals. A verdict needs the whole interval on one side of a
/// bound; one straddling it is not evaluated. An object with no plan
/// footprint (no body) is not evaluated, and so is an anchor with such a
/// member. An anchor with members whose selection is undecided is judged
/// only when they cannot change the verdict: they can only add area, so a
/// sum already above the maximum stands.
///
/// `measure: facade` bounds the outward-facing surface instead, through the
/// facade-area service: the facade area of each storey, summed over the
/// external walls it contains. A facade area may be zero; a footprint may
/// not, since an empty one means the object has no body.
///
/// Every run reports the table `areas`, one row per subject whose area was
/// measured, passing or not, in the column `plan_area` or, with `measure:
/// facade`, `facade_area`. A subject with undecided members has no row: its
/// area is known only from below.
pub struct PlanAreaRange;

impl RuleCapability for PlanAreaRange {
    fn id(&self) -> &'static str {
        "axioval:capability.plan-area"
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
            ParameterDescriptor::optional("member_selector", ParameterType::Selector),
            ParameterDescriptor::optional("measure", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum")?;
            let maximum = parameters.number("maximum")?;
            if minimum.is_none() && maximum.is_none() {
                return Err(invalid("minimum or maximum is required"));
            }
            if minimum.is_some_and(|value| value < 0.0) || maximum.is_some_and(|value| value < 0.0)
            {
                return Err(invalid("an area bound is negative"));
            }
            if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
                return Err(invalid("minimum exceeds maximum"));
            }
            let members = parameters.selector("member_selector")?;
            let traversal = parameters.traversal()?;
            if members.is_none() && traversal.is_some() {
                return Err(invalid(
                    "a relationship reaches members only with `member_selector`",
                ));
            }
            let measure = Measure::parse(&parameters)?;
            Ok::<_, Unavailable>((minimum, maximum, members, traversal, measure))
        })();
        let (minimum, maximum, members, traversal, measure) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("plan-area: {message}"),
                );
            }
        };
        let members = members.map(|selector| Population::of(context, selector));
        let what = if members.is_some() {
            format!("summed {} of the members", measure.noun())
        } else {
            measure.noun().to_owned()
        };
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        let mut areas = table(&rule.id, "areas", vec![area_column(measure.column())]);
        for subject in subjects {
            let measured = match &members {
                None => own_area(context, measure, &subject.id).map(|sum| (sum, None)),
                Some(population) => {
                    member_areas(context, traversal.as_ref(), subject, population, measure)
                }
            };
            let (sum, reached) = match measured {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                    continue;
                }
            };
            let (related, undecided) = reached.unwrap_or_default();
            if undecided == 0 {
                // Subjects are distinct objects, so rows never collide.
                let _ = areas.push_row(
                    subject.id.clone(),
                    vec![ReportValue::measured(sum.lower, sum.upper)],
                );
            }
            // Undecided members can only add area: only an excess stands.
            if undecided > 0 && !maximum.is_some_and(|maximum| sum.lower > maximum) {
                evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{undecided} member(s) {} cannot be assigned",
                        relation_text(traversal.as_ref())
                    ),
                );
                continue;
            }
            match judge(sum.lower, sum.upper, minimum, maximum) {
                Verdict::Pass => {}
                Verdict::Fail(bound) => evaluation.push_finding_deviating(
                    finding(
                        rule,
                        &subject.id,
                        format!(
                            "{what} is {} m²; required {bound} m²",
                            shown(sum.lower, sum.upper)
                        ),
                        sum.evidence,
                        related,
                    ),
                    deviation(sum.lower, sum.upper, minimum, maximum),
                ),
                Verdict::Undecided(bound) => evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{what} is {} m², which straddles the bound {bound} m²",
                        shown(sum.lower, sum.upper)
                    ),
                ),
            }
        }
        evaluation.push_table(areas);
        evaluation
    }
}
