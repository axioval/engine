//! `wall-spacing` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in
//! the tests (`parity-reference` feature). It is no capability of any
//! registry. It pairs members and measures bands through the same
//! `Storey`, `Bands` and `uncovered` the template's measured
//! `wall_spacing` reads.

use std::collections::BTreeSet;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Bands, Coverage, Found, Members, NAME, Pair, Services, Storey, parse, uncovered};
use crate::orientation::Tri;
use crate::pairs::refuse_all;
use crate::plan_area::shown;
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding};

/// Requires parallel walls or beams on each selected storey to stand far
/// enough apart, as `wall-spacing` judged it before it became a template.
pub struct WallSpacing;

impl RuleCapability for WallSpacing {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (storeys, mut evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, &config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&storeys, evaluation, &reason, &message),
        };
        let (matched, selection) = select_objects(context, config.members);
        let undecided: BTreeSet<ObjectId> = selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();
        let matched: BTreeSet<ObjectId> = matched.iter().map(|object| object.id.clone()).collect();
        let universe: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| matched.contains(&object.id) || undecided.contains(&object.id))
            .collect();
        let members = Members {
            matched: &matched,
            universe: &universe,
        };
        for storey in storeys {
            let judged = Storey {
                context,
                config: &config,
                services: &services,
                object: storey,
            };
            for check in checks(&judged, &members) {
                match check {
                    Ok((message, evidence, related)) => {
                        evaluation
                            .push_finding(finding(rule, &storey.id, message, evidence, related));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(storey.id.clone(), reason, message);
                    }
                }
            }
        }
        evaluation
    }
}

fn checks(storey: &Storey<'_, '_>, members: &Members<'_>) -> Vec<Result<Found, Unavailable>> {
    let (reached, mut evidence) =
        match storey
            .config
            .member_path
            .related(storey.context, &storey.object.id, members.universe)
        {
            Ok(reached) => reached,
            Err(unavailable) => return vec![Err(unavailable)],
        };
    let reach = storey
        .config
        .minimum
        .unwrap_or(0.0)
        .max(storey.config.coverage.as_ref().map_or(0.0, |c| c.maximum));
    let (pairs, blind) = match storey.pairs(&reached, members, reach) {
        Ok(pairs) => pairs,
        Err(unavailable) => return vec![Err(unavailable)],
    };
    for pair in &pairs {
        evidence.extend(pair.evidence.iter().cloned());
    }
    let mut checks = Vec::new();
    if let Some(minimum) = storey.config.minimum {
        checks.extend(judge_minimum(minimum, &pairs, &blind));
    }
    if let Some(coverage) = &storey.config.coverage {
        checks.extend(judge_coverage(storey, coverage, &pairs, &blind, &evidence));
    }
    checks
}

fn judge_minimum(
    minimum: f64,
    pairs: &[Pair],
    blind: &[String],
) -> Vec<Result<Found, Unavailable>> {
    let mut checks = Vec::new();
    let mut unknown: Vec<String> = blind.to_vec();
    for pair in pairs {
        let (low, high) = pair.distance;
        let close = pair.paired.and(Tri::of(high < minimum, low >= minimum));
        match close {
            Tri::Yes => checks.push(Ok((
                format!(
                    "{} and {} are parallel and {} m apart in plan; at least {minimum} m \
                     required",
                    pair.first,
                    pair.second,
                    shown(low, high)
                ),
                pair.evidence.clone(),
                vec![pair.first.clone(), pair.second.clone()],
            ))),
            Tri::Maybe => {
                let mut why = pair.why.clone();
                if why.is_empty() {
                    why.push(format!(
                        "{} and {} may be closer than {minimum} m ({} m)",
                        pair.first,
                        pair.second,
                        shown(low, high)
                    ));
                }
                unknown.extend(why);
            }
            Tri::No => {}
        }
    }
    if !unknown.is_empty() {
        checks.push(Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "minimum spacing: whether every parallel pair stands {minimum} m apart is \
                 unknown: {}",
                unknown.join("; ")
            ),
        )));
    }
    checks
}

fn judge_coverage(
    storey: &Storey<'_, '_>,
    coverage: &Coverage<'_>,
    pairs: &[Pair],
    blind: &[String],
    evidence: &[Evidence],
) -> Vec<Result<Found, Unavailable>> {
    let Some(areas) = storey.services.areas else {
        return Vec::new();
    };
    let maximum = coverage.maximum;
    let Bands {
        least,
        most,
        mut unknown,
        related,
    } = Bands::of(pairs, maximum);
    unknown.splice(0..0, blind.iter().cloned());
    let (matched, selection) = select_objects(storey.context, coverage.footprints);
    if let Some(outcome) = selection.not_evaluated_outcomes().first() {
        return vec![Err((
            outcome.reason().clone(),
            format!(
                "the storey's footprint objects are undecided: {}",
                outcome.message()
            ),
        ))];
    }
    let (footprints, mut cited) =
        match coverage
            .footprint_path
            .related(storey.context, &storey.object.id, &matched)
        {
            Ok(found) => found,
            Err(unavailable) => return vec![Err(unavailable)],
        };
    if footprints.is_empty() {
        return vec![Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} reaches no footprint object, so it has no gross footprint to cover",
                storey.object.id
            ),
        ))];
    }
    cited.extend(evidence.iter().cloned());
    let threshold = coverage.threshold;
    let mut checks = Vec::new();
    for footprint in footprints {
        let (lower, upper) = match uncovered(areas, &footprint, (&least, &most), unknown.is_empty())
        {
            Ok((area, measured)) => {
                cited.extend(measured);
                area
            }
            Err(error) => {
                checks.push(Err(error));
                continue;
            }
        };
        let what = format!(
            "{} m² of {footprint} lies outside every band between parallel members at most \
             {maximum} m apart",
            shown(lower, upper)
        );
        if lower > threshold {
            let mut objects: Vec<ObjectId> = related.iter().cloned().collect();
            objects.push(footprint.clone());
            checks.push(Ok((
                format!("{what}; at most {threshold} m² allowed"),
                cited.clone(),
                objects,
            )));
        } else if upper > threshold {
            let mut message = format!("{what}, which straddles {threshold} m²");
            for reason in &unknown {
                message.push_str("; ");
                message.push_str(reason);
            }
            checks.push(Err((NotEvaluatedReason::IncompleteEvidence, message)));
        }
    }
    checks
}
