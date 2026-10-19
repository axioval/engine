//! The precision class of a geometric derivation
//! ([#438](https://github.com/axioval/engine/issues/438)): Exact, or Fast
//! within a budget.
//!
//! A kernel's fast mode (a disc morphology filling notches by chords, say)
//! is promised only on the side it was asked for and within the excess it
//! states, not bit for bit across releases. A Fast value therefore never
//! answers an Exact question: the class is a field of the key, so an Exact
//! read never finds a Fast value however alike the rest of their keys, and
//! the store checks every value it hands out against the class asked
//! ([`super::DerivationKind::excess`], [`super::DerivationError::Precision`]).

use std::fmt;

/// How exactly a geometric derivation's value is computed: a field of its
/// key ([`super::DerivationKind::precision`]).
///
/// A Fast value carries its excess: how far, at most the budget, it may lie
/// beyond the exact one on the side it keeps
/// ([`super::DerivationKind::excess`]). An Exact value carries none.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Precision {
    /// Computed exactly: the value carries no excess.
    Exact,
    /// Computed in a kernel's fast mode, with at most this excess.
    Fast(FastBudget),
}

/// The greatest excess a Fast value may carry, in metres, compared and
/// hashed by its bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FastBudget(u64);

impl FastBudget {
    /// The budget in metres.
    #[must_use]
    pub const fn metres(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl Precision {
    /// The Fast class with a budget of `metres` of excess.
    #[must_use]
    pub const fn fast(metres: f64) -> Self {
        Self::Fast(FastBudget(metres.to_bits()))
    }

    /// Whether the class is Exact.
    #[must_use]
    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact)
    }

    /// The Fast class's budget in metres; `None` for Exact.
    #[must_use]
    pub const fn budget_metres(self) -> Option<f64> {
        match self {
            Self::Exact => None,
            Self::Fast(budget) => Some(budget.metres()),
        }
    }

    /// Whether a value carrying `excess` answers a question of this class:
    /// an Exact one only a value with none (not even a Fast one whose
    /// excess is zero, which is promised only up to a release), a Fast one
    /// a value with none or with at most its budget.
    #[must_use]
    pub fn holds(self, excess: Option<f64>) -> bool {
        match (self, excess) {
            (_, None) => true,
            (Self::Exact, Some(_)) => false,
            (Self::Fast(budget), Some(excess)) => excess <= budget.metres(),
        }
    }
}

impl fmt::Debug for Precision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Precision {
    /// `exact`, or `fast within 0.001 m`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => write!(f, "exact"),
            Self::Fast(budget) => write!(f, "fast within {} m", budget.metres()),
        }
    }
}

impl fmt::Debug for FastBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} m", self.metres())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_question_holds_no_excess_and_a_fast_one_its_budget() {
        let fast = Precision::fast(1.0e-3);
        assert!(Precision::Exact.holds(None));
        assert!(!Precision::Exact.holds(Some(0.0)));
        assert!(fast.holds(None) && fast.holds(Some(0.0)) && fast.holds(Some(1.0e-3)));
        assert!(!fast.holds(Some(2.0e-3)) && !fast.holds(Some(f64::NAN)));
        assert_eq!(fast.budget_metres(), Some(1.0e-3));
        assert_eq!(Precision::Exact.budget_metres(), None);
        assert!(Precision::Exact.is_exact() && !fast.is_exact());
        assert_ne!(fast, Precision::Exact);
        assert_ne!(fast, Precision::fast(2.0e-3));
        assert_eq!(fast.to_string(), "fast within 0.001 m");
        assert_eq!(format!("{:?}", Precision::Exact), "exact");
    }
}
