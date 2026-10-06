//! `area-ratio` as it was implemented before it became a template (#282),
//! kept only as the parity reference the template is held to in the rules
//! crate's tests (`parity-reference` feature). It is no capability of any
//! registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, ObjectId, ReportColumn, ReportValue};

use crate::counts::{Population, relation_text, tally};
use crate::light_area::LightArea;
use crate::plan_area::{Measure, Sum, Verdict, area_column, deviation, judge, shown, table};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Requires the plan area of one population to stand in a ratio to another,
/// as `area-ratio` judged it before it became a template.
pub struct AreaRatio;

impl RuleCapability for AreaRatio {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum")?;
            let maximum = parameters.number("maximum")?;
            if minimum.is_none() && maximum.is_none() {
                return Err(invalid("minimum or maximum is required"));
            }
            if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
                return Err(invalid("minimum exceeds maximum"));
            }
            let top_area = parameters.property("numerator_property")?;
            Ok::<_, Unavailable>((
                parameters.required_selector("numerator_selector")?,
                parameters.selector("denominator_selector")?,
                top_area,
                LightArea::parse(&parameters, top_area)?,
                parameters
                    .boolean("empty_numerator_finding")?
                    .unwrap_or(false),
                parameters.property("denominator_property")?,
                (minimum, maximum),
                Measure::sides(&parameters)?,
                parameters.traversal()?,
            ))
        })();
        let (
            numerator,
            denominator,
            top_area,
            light,
            report_empty,
            bottom_area,
            (minimum, maximum),
            (top_measure, bottom_measure),
            traversal,
        ) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("area-ratio: {message}"),
                );
            }
        };
        if light.is_some() && (top_measure == Measure::Facade || bottom_measure == Measure::Facade)
        {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "area-ratio: a `facade` measure does not combine with `numerator_derivation` \
                 `light-area`"
                    .to_owned(),
            );
        }
        let noun = if top_measure == bottom_measure {
            top_measure.noun().to_owned()
        } else {
            format!("{} to {}", top_measure.noun(), bottom_measure.noun())
        };
        let numerator = Population::of(context, numerator);
        // Members already reported, so one reached by several anchors is
        // reported once.
        let mut reported = std::collections::BTreeSet::new();
        let denominator = denominator.map(|selector| Population::of(context, selector));
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let via = relation_text(traversal.as_ref());
        let mut ratios = table(
            &rule.id,
            "ratios",
            vec![
                area_column("numerator_area"),
                area_column("denominator_area"),
                ReportColumn::number("ratio"),
            ],
        );
        for anchor in anchors {
            let judged = (|| {
                let over = tally(context, traversal.as_ref(), anchor, &numerator)?;
                let under = match &denominator {
                    Some(population) => {
                        Some(tally(context, traversal.as_ref(), anchor, population)?)
                    }
                    None => None,
                };
                let undecided = over.undecided + under.as_ref().map_or(0, |under| under.undecided);
                if undecided > 0 {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{undecided} related object(s) {via} cannot be assigned"),
                    ));
                }
                if report_empty && over.decided.is_empty() {
                    return Ok(Judged::Empty(over.evidence));
                }
                let (mut top, provenance) = match &light {
                    None => (
                        Sum::measured(context, top_area, top_measure, &over.decided)?,
                        String::new(),
                    ),
                    Some(light) => {
                        let summed = light.sum(context, &over.decided);
                        for (member, message, evidence) in &summed.oversized {
                            if reported.insert(member.clone()) {
                                evaluation.push_finding(finding(
                                    rule,
                                    member,
                                    message.clone(),
                                    evidence.clone(),
                                    vec![anchor.id.clone()],
                                ));
                            }
                        }
                        for (member, message) in &summed.unchecked {
                            if reported.insert(member.clone()) {
                                evaluation.push_object_not_evaluated(
                                    member.clone(),
                                    NotEvaluatedReason::IncompleteEvidence,
                                    message.clone(),
                                );
                            }
                        }
                        if let Some(failure) = summed.failure {
                            return Err(failure);
                        }
                        if let Some((member, _, _)) = summed.oversized.first() {
                            return Err((
                                NotEvaluatedReason::InvalidEvidence,
                                format!(
                                    "{} member(s), first {member}, state a light area larger \
                                     than the element",
                                    summed.oversized.len()
                                ),
                            ));
                        }
                        let provenance = summed.provenance();
                        (
                            Sum {
                                lower: summed.lower,
                                upper: summed.upper,
                                evidence: summed.evidence,
                            },
                            provenance,
                        )
                    }
                };
                top.evidence.extend(over.evidence);
                let mut bottom = match &under {
                    Some(under) => {
                        Sum::measured(context, bottom_area, bottom_measure, &under.decided)?
                    }
                    None => Sum::measured(
                        context,
                        bottom_area,
                        bottom_measure,
                        std::slice::from_ref(&anchor.id),
                    )?,
                };
                if let Some(under) = under {
                    bottom.evidence.extend(under.evidence);
                }
                if bottom.upper <= 0.0 {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("the denominator has no {}", bottom_measure.noun()),
                    ));
                }
                let lower = top.lower / bottom.upper;
                let upper = if bottom.lower > 0.0 {
                    top.upper / bottom.lower
                } else {
                    f64::INFINITY
                };
                let mut evidence = top.evidence;
                evidence.extend(bottom.evidence);
                Ok(Judged::Ratio {
                    lower,
                    upper,
                    numerator: (top.lower, top.upper),
                    denominator: (bottom.lower, bottom.upper),
                    provenance,
                    evidence,
                    members: over.decided,
                })
            })();
            let (lower, upper, area, of, provenance, evidence, members) = match judged {
                Ok(Judged::Ratio {
                    lower,
                    upper,
                    numerator,
                    denominator,
                    provenance,
                    evidence,
                    members,
                }) => {
                    // Anchors are distinct objects, so rows never collide.
                    let _ = ratios.push_row(
                        anchor.id.clone(),
                        vec![
                            ReportValue::measured(numerator.0, numerator.1),
                            ReportValue::measured(denominator.0, denominator.1),
                            ReportValue::measured(lower, upper),
                        ],
                    );
                    (
                        lower,
                        upper,
                        numerator.0,
                        denominator.0,
                        provenance,
                        evidence,
                        members,
                    )
                }
                Ok(Judged::Empty(evidence)) => {
                    evaluation.push_finding(finding(
                        rule,
                        &anchor.id,
                        format!("no numerator object is reached {via}; the ratio is 0"),
                        evidence,
                        Vec::new(),
                    ));
                    continue;
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                    continue;
                }
            };
            match judge(lower, upper, minimum, maximum) {
                Verdict::Pass => {}
                Verdict::Fail(bound) => evaluation.push_finding_deviating(
                    finding(
                        rule,
                        &anchor.id,
                        format!(
                            "{noun} ratio is {} ({} m² of {} m²); required {bound}{provenance}",
                            shown(lower, upper),
                            (area * 100.0).round() / 100.0,
                            (of * 100.0).round() / 100.0,
                        ),
                        evidence,
                        members,
                    ),
                    deviation(lower, upper, minimum, maximum),
                ),
                Verdict::Undecided(bound) => evaluation.push_object_not_evaluated(
                    anchor.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{noun} ratio is {}, which straddles the bound {bound}",
                        shown(lower, upper)
                    ),
                ),
            }
        }
        evaluation.push_table(ratios);
        evaluation
    }
}

/// What one anchor of `area-ratio` comes to before the bounds are applied.
enum Judged {
    Ratio {
        lower: f64,
        upper: f64,
        /// The summed numerator and denominator areas, as intervals.
        numerator: (f64, f64),
        denominator: (f64, f64),
        /// Which light-area steps produced the numerator, for the message.
        provenance: String,
        evidence: Vec<Evidence>,
        members: Vec<ObjectId>,
    },
    /// The anchor reaches no numerator object, and that is reported.
    Empty(Vec<Evidence>),
}
