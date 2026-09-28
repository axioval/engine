//! Rule-level refinement of a capability's outcomes: severities graded by
//! how far a value misses its bound.
//!
//! A capability decides what is found; a rule instance may refine how it is
//! reported. The runtime applies the refinement after the capability ran, so
//! every capability that reports what refinement needs gets it without a
//! parameter of its own.

use axioval_ir::contract::{
    self as schema, CategoryLevel, Selector, SeverityBand, SeverityOverride,
};
use axioval_ir::{NotEvaluatedReason, Object, Severity};

use crate::{CapabilityEvaluation, CompiledRule, RuleContext, SelectorVerdict};

/// How far a measured value misses the bound it fails, relative to that
/// bound, as an interval sure to hold the exact relative deviation.
///
/// Both ends are at least zero; the upper end may be infinite (a bound of
/// zero, or a value known only from one side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deviation {
    lower: f64,
    upper: f64,
}

impl Deviation {
    /// A relative deviation in `[lower, upper]`; `None` unless
    /// `0 <= lower <= upper` (NaN refused).
    #[must_use]
    pub fn try_new(lower: f64, upper: f64) -> Option<Self> {
        (lower >= 0.0 && lower <= upper).then_some(Self { lower, upper })
    }

    /// The shortfall of a value in `[lower, upper]` below `minimum`, relative
    /// to `minimum`: `(minimum - value) / |minimum|`, widened outwards so it
    /// holds the exact quotient. A bound of zero makes any shortfall
    /// infinitely large.
    #[must_use]
    pub fn below(minimum: f64, lower: f64, upper: f64) -> Self {
        Self::relative(minimum - upper, minimum - lower, minimum)
    }

    /// The excess of a value in `[lower, upper]` over `maximum`, relative to
    /// `maximum`: `(value - maximum) / |maximum|`, widened outwards.
    #[must_use]
    pub fn above(maximum: f64, lower: f64, upper: f64) -> Self {
        Self::relative(lower - maximum, upper - maximum, maximum)
    }

    fn relative(least: f64, most: f64, bound: f64) -> Self {
        let scale = bound.abs();
        let (lower, upper) = if scale == 0.0 {
            let infinite = |miss: f64| if miss > 0.0 { f64::INFINITY } else { 0.0 };
            (infinite(least), infinite(most))
        } else {
            // One rounding in the difference and one in the quotient: a
            // step outwards on each end keeps the exact value inside.
            (
                (least / scale).next_down().next_down(),
                (most / scale).next_up().next_up(),
            )
        };
        let lower = if lower.is_nan() { 0.0 } else { lower.max(0.0) };
        let upper = if upper.is_nan() {
            f64::INFINITY
        } else {
            upper.max(lower)
        };
        Self { lower, upper }
    }

    /// The larger of two deviations, as a finding naming several failing
    /// values is graded by the one missing most: an interval sure to hold
    /// the larger exact value.
    #[must_use]
    pub fn worst(self, other: Self) -> Self {
        Self {
            lower: self.lower.max(other.lower),
            upper: self.upper.max(other.upper),
        }
    }

    /// The smaller of two deviations, as a value that may meet any of
    /// several alternative limits misses by as little as the nearest one.
    #[must_use]
    pub fn least(self, other: Self) -> Self {
        Self {
            lower: self.lower.min(other.lower),
            upper: self.upper.min(other.upper),
        }
    }

    /// The least the exact deviation may be.
    #[must_use]
    pub fn lower(&self) -> f64 {
        self.lower
    }

    /// The most the exact deviation may be.
    #[must_use]
    pub fn upper(&self) -> f64 {
        self.upper
    }
}

/// What a rule instance asks of its outcomes beyond the capability's
/// verdicts. Empty for a rule that declares nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleRefinement {
    /// Severity bands over the relative deviation, ascending.
    pub severity_bands: Vec<SeverityBand>,
    /// Severities chosen by the objects a finding involves, first match
    /// first.
    pub severity_overrides: Vec<SeverityOverride>,
    /// Nested categories headed before findings, outermost first.
    pub categories: Vec<CategoryLevel>,
}

impl RuleRefinement {
    /// Whether the rule declares nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.severity_bands.is_empty()
            && self.severity_overrides.is_empty()
            && self.categories.is_empty()
    }

    /// Whether applying the rule's declarations needs an [`OutcomeRefiner`]:
    /// everything but severity bands reads the model.
    #[must_use]
    pub fn needs_refiner(&self) -> bool {
        !self.severity_overrides.is_empty() || !self.categories.is_empty()
    }
}

/// Trusted code that applies what a rule instance declares about its
/// outcomes and that needs the model to apply: selectors, properties and
/// relationships (severity overrides, categories).
///
/// The runtime calls it after the capability ran and after severity bands
/// were applied, for every rule the plan refines. The host installs it with
/// the capabilities ([`crate::CapabilityRegistry::with_refiner`]); a rule
/// needing one compiles only against a registry that has one, so no
/// declaration is ever silently ignored.
pub trait OutcomeRefiner: Send + Sync {
    /// Refines one rule's outcomes in place. What cannot be decided becomes
    /// a not-evaluated outcome, never a default.
    fn refine(
        &self,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
        refining: &Refining<'_>,
        evaluation: &mut CapabilityEvaluation,
    );

    /// How many objects `rule`'s applicability selector surely selects;
    /// objects it cannot decide are not counted.
    fn selected(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> usize;

    /// Whether `selector` selects `object`, for the runtime's own reads of
    /// the model: the selection of a rule whose outcomes another rule reads
    /// per object.
    ///
    /// The default decides nothing, so a refiner that does not evaluate
    /// selectors leaves every such object undecided rather than selected or
    /// not.
    fn evaluate_selector(
        &self,
        context: &RuleContext<'_>,
        selector: &Selector,
        object: &Object,
    ) -> SelectorVerdict {
        let _ = (context, selector, object);
        SelectorVerdict::Undecided(
            NotEvaluatedReason::MissingService,
            "the host's outcome refiner evaluates no selectors".into(),
        )
    }
}

/// What one call of an [`OutcomeRefiner`] applies: the rule's declarations
/// and the host's location policy.
#[derive(Clone, Copy, Debug)]
pub struct Refining<'a> {
    /// What the rule instance declares; empty for a rule declaring nothing.
    pub refinement: &'a RuleRefinement,
    /// How the host locates outcomes; `None` when it does not.
    pub locations: Option<&'a LocationPolicy>,
}

/// How outcomes are located by storey and space; a host's choice, never a
/// package's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocationMethod {
    /// The storeys each object lies in, climbed to along the containment
    /// path; no spaces.
    Storeys,
    /// The nearest storeys and spaces each object lies in, climbed to along
    /// the containment path.
    Containers,
    /// Storeys as for [`Self::Containers`]; spaces are those whose body
    /// contains or meets each object, through the geometry-derived
    /// `axioval:derived.contained-in-space` relationship.
    Geometry,
}

/// A host's location policy: the method, and what storeys and spaces are in
/// its sources' own vocabulary.
///
/// Storeys and spaces are objects of the named kinds (compared ignoring
/// case). The containment path's steps are climbed in any order and any
/// number of times, stopping at each storey or space reached, as a
/// `related` selector's steps read. An object that is itself a storey or
/// space is located in itself. `name` is the property (set, name) whose
/// value names a place, read natively.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocationPolicy {
    pub method: LocationMethod,
    pub storey_kinds: Vec<String>,
    pub space_kinds: Vec<String>,
    pub containment: Vec<String>,
    pub name: Option<(String, String)>,
}

/// Checks a rule's bands: each threshold finite and positive, strictly
/// ascending, at least one band.
pub(crate) fn validate_bands(bands: &[SeverityBand]) -> Result<(), String> {
    let mut previous = 0.0;
    for (index, band) in bands.iter().enumerate() {
        if !band.below.is_finite() || band.below <= previous {
            return Err(format!(
                "severity band {index} must lie below a finite threshold above {previous}"
            ));
        }
        previous = band.below;
    }
    Ok(())
}

/// The severity `deviation` grades to under `bands`, with `beyond` for a
/// deviation at or past the last band, and whether the interval reached
/// bands of different severities.
///
/// Band `i` holds deviations in `[below(i-1), below(i))` (from zero for the
/// first). An interval takes the most severe band it may reach.
pub(crate) fn grade(
    bands: &[SeverityBand],
    beyond: &Severity,
    deviation: Deviation,
) -> (Severity, bool) {
    let mut reached: Vec<Severity> = Vec::new();
    let mut from = 0.0;
    for band in bands {
        if deviation.lower < band.below && deviation.upper >= from {
            reached.push(report_severity(&band.severity));
        }
        from = band.below;
    }
    if deviation.upper >= from {
        reached.push(beyond.clone());
    }
    // `Severity` orders the most severe first.
    let worst = reached
        .iter()
        .min()
        .cloned()
        .unwrap_or_else(|| beyond.clone());
    let mixed = reached.iter().any(|severity| *severity != worst);
    (worst, mixed)
}

/// A package severity as a report severity.
#[must_use]
pub fn report_severity(severity: &schema::Severity) -> Severity {
    match severity {
        schema::Severity::Error => Severity::Error,
        schema::Severity::Warning => Severity::Warning,
        schema::Severity::Info => Severity::Info,
    }
}

/// A severity as messages spell it.
pub(crate) fn label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bands() -> Vec<SeverityBand> {
        vec![
            SeverityBand {
                below: 0.05,
                severity: schema::Severity::Info,
            },
            SeverityBand {
                below: 0.2,
                severity: schema::Severity::Warning,
            },
        ]
    }

    #[test]
    fn a_deviation_takes_its_band_and_the_rule_severity_beyond() {
        let grade = |lower, upper| grade(&bands(), &Severity::Error, Deviation { lower, upper }).0;
        assert_eq!(grade(0.03, 0.03), Severity::Info);
        assert_eq!(grade(0.1, 0.1), Severity::Warning);
        assert_eq!(grade(0.3, 0.3), Severity::Error);
        assert_eq!(grade(0.05, 0.05), Severity::Warning);
        assert_eq!(grade(0.2, f64::INFINITY), Severity::Error);
    }

    #[test]
    fn a_straddling_deviation_takes_its_most_severe_band() {
        let (severity, mixed) = grade(
            &bands(),
            &Severity::Error,
            Deviation {
                lower: 0.03,
                upper: 0.1,
            },
        );
        assert_eq!(severity, Severity::Warning);
        assert!(mixed);
        let (severity, mixed) = grade(
            &bands(),
            &Severity::Error,
            Deviation {
                lower: 0.1,
                upper: 0.25,
            },
        );
        assert_eq!(severity, Severity::Error);
        assert!(mixed);
    }

    #[test]
    fn a_relative_deviation_holds_the_exact_quotient() {
        let shortfall = Deviation::below(10.0, 7.0, 7.0);
        assert!(shortfall.lower() <= 0.3 && 0.3 <= shortfall.upper());
        let excess = Deviation::above(2.0, 2.5, 3.0);
        assert!(excess.lower() <= 0.25 && 0.5 <= excess.upper());
        assert!(Deviation::below(0.0, -1.0, -1.0).upper().is_infinite());
        assert!(
            Deviation::above(1.0, 2.0, f64::INFINITY)
                .upper()
                .is_infinite()
        );
    }

    #[test]
    fn bands_must_ascend_from_zero() {
        assert!(validate_bands(&bands()).is_ok());
        let mut descending = bands();
        descending.reverse();
        assert!(validate_bands(&descending).is_err());
        assert!(
            validate_bands(&[SeverityBand {
                below: 0.0,
                severity: schema::Severity::Info
            }])
            .is_err()
        );
    }
}
