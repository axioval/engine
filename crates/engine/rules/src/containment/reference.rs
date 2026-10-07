//! `containment` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in
//! the tests (`parity-reference` feature). It is no capability of any
//! registry. It places and reads the elements through the same `assess`
//! the template's measured `containment_items` reads.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ProximityProjection, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, ObjectId, Scope};

use super::{Declaration, Held, Item, assess, declaration};
use crate::pairs::{Unevaluated, prepare, refuse_declaration, severity};

/// Checks that inner elements lie in outer elements, keep their cover to
/// the outer element's faces, and are held in the declared numbers, as
/// `containment` judged it before it became a template.
pub struct Containment;

struct Run<'r> {
    rule: &'r CompiledRule,
    declared: &'r Declaration,
    evaluation: CapabilityEvaluation,
    unevaluated: Unevaluated,
}

impl Run<'_> {
    fn finding(
        &mut self,
        scope: &ObjectId,
        message: String,
        related: Vec<ObjectId>,
        evidence: Vec<Evidence>,
    ) {
        self.evaluation.push_finding(
            Finding {
                explanation: None,
                id: None,
                decision: None,
                rule_id: self.rule.id.clone(),
                scope: Scope::Object(scope.clone()),
                severity: severity(self.rule),
                message,
                related: Vec::new(),
                evidence,
                location: None,
                categories: Vec::new(),
            }
            .with_related(related),
        );
    }

    /// Judges one item of an inner element.
    fn item(&mut self, inner: &ObjectId, item: Item) {
        match item {
            Item::Orphan(Ok((message, evidence))) => {
                self.finding(inner, message, Vec::new(), evidence);
            }
            Item::Orphan(Err((reason, message))) | Item::Open((reason, message)) => {
                self.unevaluated.push(inner.clone(), reason, message);
            }
            Item::Band {
                measured: Err((reason, message)),
                ..
            } => self.unevaluated.push(inner.clone(), reason, message),
            Item::Band {
                band,
                outer,
                measured: Ok(measured),
                link,
            } => {
                let band = &self.declared.bands[band];
                let banded = band.read(&measured, &outer);
                let below = band
                    .minimum
                    .map(|minimum| (banded.upper < minimum, banded.lower < minimum));
                let above = band
                    .maximum
                    .map(|maximum| (banded.lower > maximum, banded.upper > maximum));
                let message = if below.is_some_and(|(surely, _)| surely) {
                    banded.below
                } else if above.is_some_and(|(surely, _)| surely) {
                    banded.above
                } else {
                    if below.is_some_and(|(_, possibly)| possibly)
                        || above.is_some_and(|(_, possibly)| possibly)
                    {
                        self.unevaluated.push(
                            inner.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            banded.straddles,
                        );
                    }
                    return;
                };
                let mut evidence = vec![measured.evidence().clone()];
                evidence.extend(link);
                self.finding(inner, message, vec![outer], evidence);
            }
        }
    }

    /// Judges one outer element's count.
    fn count(&mut self, outer: &ObjectId, held: &Held) {
        let (sure, possible) = held.counts();
        let mut undecided = false;
        if let Some(minimum) = self.declared.minimum_count {
            if possible < minimum {
                self.finding(
                    outer,
                    format!("holds {sure} inner elements, fewer than the minimum {minimum}"),
                    held.sure.iter().cloned().collect(),
                    Vec::new(),
                );
                return;
            }
            undecided |= sure < minimum;
        }
        if let Some(maximum) = self.declared.maximum_count {
            if sure > maximum {
                self.finding(
                    outer,
                    format!("holds {sure} inner elements, more than the maximum {maximum}"),
                    held.sure.iter().cloned().collect(),
                    Vec::new(),
                );
                return;
            }
            undecided |= possible > maximum;
        }
        if undecided {
            self.unevaluated.push(
                outer.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "holds between {sure} and {possible} inner elements, so its count cannot be judged"
                ),
            );
        }
    }
}

impl RuleCapability for Containment {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let prepared = match prepare(context, rule, Some(0.0), ProximityProjection::Minimum3d) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let assessed = assess(&declared, prepared);
        let mut run = Run {
            rule,
            declared: &declared,
            evaluation: CapabilityEvaluation::default(),
            unevaluated: assessed.unevaluated,
        };
        for (inner, items) in assessed.inner {
            for item in items {
                run.item(&inner, item);
            }
        }
        if declared.minimum_count.is_some() || declared.maximum_count.is_some() {
            for (outer, count) in &assessed.held {
                run.count(outer, count);
            }
        }
        let Run {
            mut evaluation,
            unevaluated,
            ..
        } = run;
        unevaluated.drain_into(&mut evaluation);
        evaluation
    }
}
