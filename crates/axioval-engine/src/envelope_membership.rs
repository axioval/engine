//! Source-neutral envelope-membership evidence.
//!
//! ADR 0004: a service returns what was *measured*; a capability decides what
//! it means. Here the measurement is two sets of objects -- those a model
//! *declares* to be on the building envelope, and those geometry says *are* --
//! and the decision is whether they agree.
//!
//! Two things deliberately do not cross this seam:
//!
//! - **Applicability.** The source provider decided whether a model was worth
//!   checking at all by inspecting its industry domain, and returned an
//!   "irrelevant" flag that the rule then had to interpret. Whether a rule
//!   applies to a model is policy; a service that is asked a question answers
//!   it or reports that it cannot.
//! - **Derivation ambiguity.** The source returned every derivation at once,
//!   each behind an `Option`, leaving the rule to discover that the branch it
//!   wanted was missing. One request now names one derivation, so an
//!   unavailable derivation is an error rather than a silent `None`.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::services::reviewable_exact_evidence;

/// Why envelope membership could not be measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeMembershipError {
    /// The evidence backing the measurement was not exact and reviewable.
    #[error("envelope membership evidence must be exact and reviewable")]
    InexactEvidence,
    /// The adapter cannot derive membership for the requested scope.
    #[error("envelope membership is unavailable for the requested derivation")]
    Unavailable,
    /// The requested derivation is not supported by this source.
    #[error("requested envelope derivation is not supported by this source")]
    UnsupportedDerivation,
}

/// Which spatial extent the envelope is derived from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EnvelopeDerivation {
    /// Every space in the model bounds the envelope.
    AllSpaces,
    /// Only spaces belonging to gross-area groups bound the envelope.
    GrossAreaGroups,
}

impl EnvelopeDerivation {
    pub fn as_str(self) -> &'static str {
        match self {
            EnvelopeDerivation::AllSpaces => "all-spaces",
            EnvelopeDerivation::GrossAreaGroups => "gross-area-groups",
        }
    }
}

/// A request for one envelope derivation over one model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeMembershipRequest {
    derivation: EnvelopeDerivation,
}

impl EnvelopeMembershipRequest {
    pub fn new(derivation: EnvelopeDerivation) -> Self {
        Self { derivation }
    }
    pub fn derivation(&self) -> EnvelopeDerivation {
        self.derivation
    }
}

/// Declared and derived envelope membership, with supporting evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeMembershipEvidence {
    request: EnvelopeMembershipRequest,
    declared: Vec<ObjectId>,
    derived: Vec<ObjectId>,
    evaluated_objects: usize,
    evidence: Evidence,
}

impl EnvelopeMembershipEvidence {
    /// Both sets are sorted and deduplicated so agreement is decided by
    /// content, never by the order an adapter happened to walk the model.
    pub fn try_new(
        request: EnvelopeMembershipRequest,
        mut declared: Vec<ObjectId>,
        mut derived: Vec<ObjectId>,
        evaluated_objects: usize,
        evidence: Evidence,
    ) -> Result<Self, EnvelopeMembershipError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(EnvelopeMembershipError::InexactEvidence);
        }
        declared.sort();
        declared.dedup();
        derived.sort();
        derived.dedup();
        Ok(Self {
            request,
            declared,
            derived,
            evaluated_objects,
            evidence,
        })
    }

    pub fn request(&self) -> EnvelopeMembershipRequest {
        self.request
    }
    /// Objects the model states are on the envelope.
    pub fn declared(&self) -> &[ObjectId] {
        &self.declared
    }
    /// Objects geometry places on the envelope.
    pub fn derived(&self) -> &[ObjectId] {
        &self.derived
    }
    /// How many objects the derivation considered.
    pub fn evaluated_objects(&self) -> usize {
        self.evaluated_objects
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// Whether the two sets hold exactly the same objects.
    pub fn agrees(&self) -> bool {
        self.declared == self.derived
    }

    /// Declared on the envelope but not derived there.
    pub fn declared_only(&self) -> Vec<ObjectId> {
        difference(&self.declared, &self.derived)
    }

    /// Derived on the envelope but not declared there.
    pub fn derived_only(&self) -> Vec<ObjectId> {
        difference(&self.derived, &self.declared)
    }
}

/// Both inputs are sorted and deduplicated, so a linear merge suffices.
fn difference(left: &[ObjectId], right: &[ObjectId]) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < left.len() {
        match right.get(j) {
            Some(candidate) if candidate < &left[i] => j += 1,
            Some(candidate) if candidate == &left[i] => {
                i += 1;
                j += 1;
            }
            _ => {
                out.push(left[i].clone());
                i += 1;
            }
        }
    }
    out
}

/// Measures which objects form a model's building envelope.
///
/// ADR 0004: every method returns a measurement. None returns a finding.
pub trait EnvelopeMembershipService: Send + Sync + 'static {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError>;
}

/// Registry handle for an [`EnvelopeMembershipService`].
#[derive(Clone)]
pub struct EnvelopeMembershipServiceHandle(Arc<dyn EnvelopeMembershipService>);

impl EnvelopeMembershipServiceHandle {
    pub fn new(service: Arc<dyn EnvelopeMembershipService>) -> Self {
        Self(service)
    }
    pub fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        self.0.measure_envelope_membership(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }
    fn oid(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }
    fn build(declared: &[&str], derived: &[&str]) -> EnvelopeMembershipEvidence {
        EnvelopeMembershipEvidence::try_new(
            EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces),
            declared.iter().map(|s| oid(s)).collect(),
            derived.iter().map(|s| oid(s)).collect(),
            3,
            Evidence::exact(source(), "envelope:all-spaces"),
        )
        .unwrap()
    }

    /// Agreement is a set property. An adapter that reports the same walls in
    /// a different order, or twice, has not found a discrepancy.
    #[test]
    fn agreement_ignores_order_and_duplicates() {
        assert!(build(&["w2", "w1", "w2"], &["w1", "w2"]).agrees());
    }

    #[test]
    fn differences_are_reported_in_both_directions() {
        let measured = build(&["w1", "w2"], &["w2", "w3"]);
        assert!(!measured.agrees());
        assert_eq!(measured.declared_only(), vec![oid("w1")]);
        assert_eq!(measured.derived_only(), vec![oid("w3")]);
    }

    #[test]
    fn empty_sets_agree_and_have_no_differences() {
        let measured = build(&[], &[]);
        assert!(measured.agrees());
        assert!(measured.declared_only().is_empty());
        assert!(measured.derived_only().is_empty());
    }

    /// A model declaring nothing external while geometry finds walls is a real
    /// discrepancy, not an empty comparison.
    #[test]
    fn nothing_declared_against_derived_walls_is_a_difference() {
        let measured = build(&[], &["w1", "w2"]);
        assert!(!measured.agrees());
        assert_eq!(measured.derived_only(), vec![oid("w1"), oid("w2")]);
        assert!(measured.declared_only().is_empty());
    }

    #[test]
    fn inexact_evidence_is_refused() {
        let result = EnvelopeMembershipEvidence::try_new(
            EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces),
            Vec::new(),
            Vec::new(),
            0,
            Evidence {
                source: source(),
                locator: "envelope:estimate".into(),
                exact: false,
            },
        );
        assert_eq!(result, Err(EnvelopeMembershipError::InexactEvidence));
    }
}
