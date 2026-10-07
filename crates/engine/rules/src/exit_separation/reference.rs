//! `exit-separation` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry. It reaches and measures through the same
//! `Reached` and `Measured` the template's measured `exit_separation`
//! reads.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::{Finding, Object};

use super::{Candidates, Declaration, Measured, NAME, Pair, Pairs, Reached, Standing, upper};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding};

/// Requires each selected space's exits to lie far enough apart for its
/// size, as `exit-separation` judged it before it became a template.
pub struct ExitSeparation;

impl RuleCapability for ExitSeparation {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match super::declaration(&Parameters(rule), true) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let selector = declared
            .exit_selector
            .expect("the selector was read with the declaration");
        let candidates = Candidates::select(context, selector);
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        for space in spaces {
            let found = check(context, rule, &declared, &candidates, space);
            for finding in found.findings {
                evaluation.push_finding(finding);
            }
            if let Some((reason, message)) = found.unevaluated {
                evaluation.push_object_not_evaluated(space.id.clone(), reason, message);
            }
        }
        evaluation
    }
}

/// What checking one space found: findings that stand, and why the rest
/// could not be decided.
#[derive(Default)]
struct Checked {
    findings: Vec<Finding>,
    unevaluated: Option<Unavailable>,
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    candidates: &Candidates<'_>,
    space: &Object,
) -> Checked {
    let mut checked = Checked::default();
    let universe: Vec<axioval_ir::ObjectId> = candidates
        .universe
        .iter()
        .map(|object| object.id.clone())
        .collect();
    let reached = match Reached::of(
        context,
        &declared.exits,
        (&universe, &candidates.undecided),
        space,
    ) {
        Ok(reached) => reached,
        Err(unavailable) => {
            checked.unevaluated = Some(unavailable);
            return checked;
        }
    };
    let (exits, maybe) = (&reached.exits, &reached.maybe);
    let mut evidence = reached.evidence.clone();
    let undecided = || reached.undecided(&candidates.undecided);
    if let Some(minimum) = declared.minimum_exits {
        if exits.len() + maybe.len() < minimum {
            checked.findings.push(finding(
                rule,
                &space.id,
                format!(
                    "has {} exit(s) via {}; at least {minimum} required",
                    exits.len() + maybe.len(),
                    declared.exits.relationship
                ),
                evidence.clone(),
                exits.iter().chain(maybe).cloned().collect(),
            ));
        } else if exits.len() < minimum {
            checked.unevaluated = Some((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} certain exit(s), at least {minimum} required: {}",
                    exits.len(),
                    undecided()
                ),
            ));
            return checked;
        }
    }
    if exits.len() + maybe.len() < 2 {
        return checked;
    }
    let separated = if exits.len() < 2 {
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("fewer than two certain exits: {}", undecided()),
        ))
    } else {
        separation(
            context,
            rule,
            declared,
            space,
            &reached,
            undecided,
            &mut evidence,
        )
    };
    match separated {
        Ok(Some(found)) => checked.findings.push(found),
        // A finding already standing is not withdrawn for want of the rest.
        Err(unavailable) if checked.findings.is_empty() => checked.unevaluated = Some(unavailable),
        Ok(None) | Err(_) => {}
    }
    checked
}

/// Judges the separation of the sure exits, in identity order, against the
/// space's required distance.
fn separation(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    space: &Object,
    reached: &Reached,
    undecided: impl Fn() -> String,
    evidence: &mut Vec<axioval_ir::Evidence>,
) -> Result<Option<Finding>, Unavailable> {
    let (exits, maybe) = (&reached.exits, &reached.maybe);
    let measured = Measured::of(context, declared, space, exits, evidence)?;
    let required = measured.required();
    let pairs = &measured.pairs;
    let mut related = measured.related.clone();
    let standings: Vec<Standing> = pairs.iter().map(|pair| pair.standing(required)).collect();
    let unknown: Vec<&str> = standings
        .iter()
        .filter_map(|standing| match standing {
            Standing::Unknown(why) => Some(why.as_str()),
            _ => None,
        })
        .collect();
    let far_enough = standings
        .iter()
        .any(|standing| matches!(standing, Standing::FarEnough));
    let too_close: Vec<&Pair> = pairs
        .iter()
        .zip(&standings)
        .filter(|(_, standing)| matches!(standing, Standing::TooClose))
        .map(|(pair, _)| pair)
        .collect();
    let requirement = measured.requirement();
    let failing: Vec<&Pair> = match declared.pairs {
        Pairs::Any if far_enough => return Ok(None),
        // Every pair is too close; name the one farthest apart.
        Pairs::Any if unknown.is_empty() && maybe.is_empty() => pairs
            .iter()
            .max_by(|a, b| upper(a).total_cmp(&upper(b)))
            .into_iter()
            .collect(),
        Pairs::All if !too_close.is_empty() => too_close,
        Pairs::All if unknown.is_empty() && maybe.is_empty() => return Ok(None),
        _ => {
            let mut why: Vec<String> = unknown.iter().map(|why| (*why).to_owned()).collect();
            if !maybe.is_empty() {
                why.push(undecided());
            }
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} ({requirement})", why.join("; ")),
            ));
        }
    };
    let described: Vec<String> = failing
        .iter()
        .map(|pair| pair.describe(declared.separation))
        .collect();
    let message = if declared.pairs == Pairs::Any && exits.len() > 2 {
        format!(
            "no two of its {} exits are far enough apart: {}; {requirement}",
            exits.len(),
            described.join("; ")
        )
    } else {
        format!("exits {}; {requirement}", described.join("; "))
    };
    evidence.push(measured.diameter.evidence().clone());
    for pair in &failing {
        related.insert(pair.first.clone());
        related.insert(pair.second.clone());
        if let Ok((_, _, measured)) = &pair.measured {
            evidence.push(measured.clone());
        }
    }
    Ok(Some(finding(
        rule,
        &space.id,
        message,
        evidence.clone(),
        related.into_iter().collect(),
    )))
}
