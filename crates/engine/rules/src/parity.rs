//! A differential parity harness: a capability rule and its rewrite as an
//! expression rule, run over the same models, must judge every object
//! alike before the capability's decision moves to expressions.
//!
//! [`outcomes`] reads how one rule of a report judged each object;
//! [`compare`] lines two rules up object by object and lists every
//! difference in verdict, severity (the graded measure's band) and evidence
//! exactness. [`ParityEvidence`] is what a migration ledger records as
//! proof that a rewrite was checked.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axioval_ir::{NotEvaluatedReason, ObjectId, Report, Scope, Severity};
use serde::Serialize;

/// How one rule judged one object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum Outcome {
    /// Found: how severe, and whether every finding's evidence is exact.
    Finding {
        /// The most severe finding's severity.
        severity: Severity,
        /// Whether every finding's evidence is exact.
        exact: bool,
    },
    /// Left open, and why.
    NotEvaluated {
        /// Why the rule could not decide.
        reason: NotEvaluatedReason,
    },
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finding { severity, exact } => write!(
                f,
                "finding ({severity:?}, {} evidence)",
                if *exact { "exact" } else { "inexact" }
            ),
            Self::NotEvaluated { reason } => write!(f, "not evaluated ({reason:?})"),
        }
    }
}

/// How `rule` judged each object it reported about. An object it reported
/// nothing about passed or was not selected, which parity reads as one.
/// Several findings about one object read as the most severe, exact only
/// when every one is; a finding outranks an open outcome.
#[must_use]
pub fn outcomes(report: &Report, rule: &str) -> BTreeMap<ObjectId, Outcome> {
    let mut outcomes = BTreeMap::new();
    for finding in report.findings() {
        let Scope::Object(object) = &finding.scope else {
            continue;
        };
        if finding.rule_id.to_string() != rule {
            continue;
        }
        let exact = finding.evidence.iter().all(|evidence| evidence.exact);
        let merged = match outcomes.remove(object) {
            Some(Outcome::Finding {
                severity,
                exact: own,
            }) => Outcome::Finding {
                // `Severity` orders the most severe first.
                severity: severity.min(finding.severity.clone()),
                exact: own && exact,
            },
            _ => Outcome::Finding {
                severity: finding.severity.clone(),
                exact,
            },
        };
        outcomes.insert(object.clone(), merged);
    }
    for outcome in &report.not_evaluated {
        if outcome.rule_id.to_string() != rule {
            continue;
        }
        if let Scope::Object(object) = &outcome.scope {
            outcomes
                .entry(object.clone())
                .or_insert_with(|| Outcome::NotEvaluated {
                    reason: outcome.reason.clone(),
                });
        }
    }
    outcomes
}

/// One object two rules judged differently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    /// The object judged differently.
    pub object: ObjectId,
    /// The capability rule's outcome; `None` when it passed or did not
    /// select the object.
    pub capability: Option<Outcome>,
    /// The expression rule's outcome.
    pub expression: Option<Outcome>,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = |outcome: &Option<Outcome>| {
            outcome
                .as_ref()
                .map_or_else(|| "passed".to_owned(), ToString::to_string)
        };
        write!(
            f,
            "{}: capability {}, expression {}",
            self.object,
            shown(&self.capability),
            shown(&self.expression)
        )
    }
}

/// What one parity check compared and found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityEvidence {
    /// The capability rule's id.
    pub capability: String,
    /// The expression rule's id.
    pub expression: String,
    /// How many objects either rule reported about.
    pub objects: usize,
    /// How many objects the capability rule found.
    pub found: usize,
    /// How many objects the capability rule left open.
    pub open: usize,
    /// Every object judged differently; empty at parity.
    pub differences: Vec<Difference>,
}

impl ParityEvidence {
    /// Whether the two rules judged every object alike.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.differences.is_empty()
    }

    /// The differences, one line each, for a failing test.
    #[must_use]
    pub fn diff(&self) -> String {
        self.differences
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Compares how the rule `capability` and the rule `expression` of
/// `report` judged every object.
#[must_use]
pub fn compare(report: &Report, capability: &str, expression: &str) -> ParityEvidence {
    let left = outcomes(report, capability);
    let right = outcomes(report, expression);
    let objects: BTreeSet<&ObjectId> = left.keys().chain(right.keys()).collect();
    let differences = objects
        .iter()
        .filter(|object| left.get(**object) != right.get(**object))
        .map(|object| Difference {
            object: (*object).clone(),
            capability: left.get(*object).cloned(),
            expression: right.get(*object).cloned(),
        })
        .collect();
    let count = |kind: fn(&Outcome) -> bool| left.values().filter(|outcome| kind(outcome)).count();
    ParityEvidence {
        capability: capability.to_owned(),
        expression: expression.to_owned(),
        objects: objects.len(),
        found: count(|outcome| matches!(outcome, Outcome::Finding { .. })),
        open: count(|outcome| matches!(outcome, Outcome::NotEvaluated { .. })),
        differences,
    }
}
