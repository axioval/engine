//! Closed intervals of real values and sound arithmetic over them.
//!
//! Every operation returns an interval holding every value the operands
//! allow. Where a floating-point result rounds, the bound moves one step
//! outward, so the exact value is never lost; an exact operation on points
//! stays a point.

/// A closed interval `[lower, upper]` of a value in coherent units; a point
/// when both bounds are equal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    /// The least value it allows.
    pub lower: f64,
    /// The greatest value it allows.
    pub upper: f64,
}

/// Why an operation over intervals has no interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntervalFailure {
    /// A divisor's interval holds zero.
    ZeroDivisor,
    /// A bound is not finite.
    Overflow,
    /// The operand lies partly or wholly outside the function's domain:
    /// a negative square root, a tangent at a pole, an angle of the origin.
    Domain,
}

/// The result of an operation over intervals.
pub type IntervalResult = Result<Interval, IntervalFailure>;

/// Two ulps either way for functions the platform does not round
/// correctly.
fn loose(value: f64) -> Interval {
    Interval {
        lower: value.next_down().next_down(),
        upper: value.next_up().next_up(),
    }
}

/// The interval around a rounded `result` whose exact value is
/// `result + error`.
fn rounded(result: f64, error: f64) -> Interval {
    if error > 0.0 {
        Interval {
            lower: result,
            upper: result.next_up(),
        }
    } else if error < 0.0 {
        Interval {
            lower: result.next_down(),
            upper: result,
        }
    } else {
        Interval::point(result)
    }
}

/// The exact error of `a + b` (Knuth's two-sum).
fn sum_error(a: f64, b: f64, sum: f64) -> f64 {
    let b_virtual = sum - a;
    let a_virtual = sum - b_virtual;
    (a - a_virtual) + (b - b_virtual)
}

fn add_bounds(a: f64, b: f64) -> Interval {
    let sum = a + b;
    if !sum.is_finite() {
        return Interval::point(sum);
    }
    rounded(sum, sum_error(a, b, sum))
}

fn mul_bounds(a: f64, b: f64) -> Interval {
    let product = a * b;
    if !product.is_finite() {
        return Interval::point(product);
    }
    rounded(product, a.mul_add(b, -product))
}

fn div_bounds(a: f64, b: f64) -> Interval {
    let quotient = a / b;
    if !quotient.is_finite() || b == 0.0 {
        return Interval::point(quotient);
    }
    // a = q·b + r exactly, so a/b = q + r/b.
    let remainder = (-quotient).mul_add(b, a);
    rounded(quotient, remainder / b)
}

impl Interval {
    /// The interval holding only `value`.
    #[must_use]
    pub const fn point(value: f64) -> Self {
        Self {
            lower: value,
            upper: value,
        }
    }

    /// The interval `[lower, upper]`, if both bounds are finite and ordered.
    #[must_use]
    pub fn new(lower: f64, upper: f64) -> Option<Self> {
        (lower.is_finite() && upper.is_finite() && lower <= upper).then_some(Self { lower, upper })
    }

    /// Whether it holds one value only.
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn is_point(self) -> bool {
        self.lower == self.upper
    }

    /// Whether it holds `value`.
    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        self.lower <= value && value <= self.upper
    }

    /// The least interval holding both.
    #[must_use]
    pub fn hull(self, other: Self) -> Self {
        Self {
            lower: self.lower.min(other.lower),
            upper: self.upper.max(other.upper),
        }
    }

    fn finite(self) -> IntervalResult {
        if self.lower.is_finite() && self.upper.is_finite() {
            Ok(self)
        } else {
            Err(IntervalFailure::Overflow)
        }
    }

    /// `-self`.
    #[must_use]
    pub fn negate(self) -> Self {
        Self {
            lower: -self.upper,
            upper: -self.lower,
        }
    }

    /// `self + other`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Overflow`] when a bound is not finite.
    pub fn plus(self, other: Self) -> IntervalResult {
        Self {
            lower: add_bounds(self.lower, other.lower).lower,
            upper: add_bounds(self.upper, other.upper).upper,
        }
        .finite()
    }

    /// `self - other`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Overflow`] when a bound is not finite.
    pub fn minus(self, other: Self) -> IntervalResult {
        self.plus(other.negate())
    }

    /// `self × other`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Overflow`] when a bound is not finite.
    pub fn times(self, other: Self) -> IntervalResult {
        Self::corners(self, other, mul_bounds).finite()
    }

    /// `self ÷ other`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::ZeroDivisor`] when `other` holds zero,
    /// [`IntervalFailure::Overflow`] when a bound is not finite.
    pub fn divided_by(self, other: Self) -> IntervalResult {
        if other.contains(0.0) {
            return Err(IntervalFailure::ZeroDivisor);
        }
        Self::corners(self, other, div_bounds).finite()
    }

    /// The hull of `operation` over the four corners of two intervals: the
    /// bounds of a product, or of a quotient by an interval without zero.
    fn corners(left: Self, right: Self, operation: fn(f64, f64) -> Self) -> Self {
        [
            operation(left.lower, right.lower),
            operation(left.lower, right.upper),
            operation(left.upper, right.lower),
            operation(left.upper, right.upper),
        ]
        .into_iter()
        .reduce(Self::hull)
        .unwrap_or(left)
    }

    /// `|self|`.
    #[must_use]
    pub fn abs(self) -> Self {
        if self.lower >= 0.0 {
            self
        } else if self.upper <= 0.0 {
            self.negate()
        } else {
            Self {
                lower: 0.0,
                upper: self.upper.max(-self.lower),
            }
        }
    }

    /// The least of two values, bound by bound.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self {
            lower: self.lower.min(other.lower),
            upper: self.upper.min(other.upper),
        }
    }

    /// The greatest of two values, bound by bound.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self {
            lower: self.lower.max(other.lower),
            upper: self.upper.max(other.upper),
        }
    }

    /// The nearest multiple of `step`, halves away from zero.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Domain`] unless `step` is one positive value,
    /// [`IntervalFailure::Overflow`] when a bound is not finite.
    pub fn round_to(self, step: Self) -> IntervalResult {
        if !step.is_point() || step.lower <= 0.0 {
            return Err(IntervalFailure::Domain);
        }
        // Rounding is monotone, so the bounds round to the bounds; the
        // quotients are widened first so a bound at a half rounds both ways.
        let lower = self.divided_by(step)?.lower.round();
        let upper = self.divided_by(step)?.upper.round();
        Self { lower, upper }.times(step)
    }

    /// The greatest integer not above, bound by bound.
    #[must_use]
    pub fn floor(self) -> Self {
        Self {
            lower: self.lower.floor(),
            upper: self.upper.floor(),
        }
    }

    /// The least integer not below, bound by bound.
    #[must_use]
    pub fn ceil(self) -> Self {
        Self {
            lower: self.lower.ceil(),
            upper: self.upper.ceil(),
        }
    }

    /// `√self`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Domain`] when it holds a negative value.
    pub fn sqrt(self) -> IntervalResult {
        if self.lower < 0.0 {
            return Err(IntervalFailure::Domain);
        }
        let bound = |value: f64| {
            let root = value.sqrt();
            // value = root² + error exactly.
            rounded(root, (-root).mul_add(root, value))
        };
        Ok(Self {
            lower: bound(self.lower).lower,
            upper: bound(self.upper).upper,
        })
    }

    /// The sine of an angle in radians.
    #[must_use]
    pub fn sin(self) -> Self {
        self.periodic(f64::sin, std::f64::consts::FRAC_PI_2)
    }

    /// The cosine of an angle in radians.
    #[must_use]
    pub fn cos(self) -> Self {
        self.periodic(f64::cos, 0.0)
    }

    /// The range of a sine-like function over the interval: the hull of its
    /// values at the bounds, extended to ±1 where a maximum (at `peak` plus
    /// a multiple of 2π) or a minimum (half a turn later) lies within.
    fn periodic(self, function: fn(f64) -> f64, peak: f64) -> Self {
        use std::f64::consts::{PI, TAU};
        let unit = Self {
            lower: -1.0,
            upper: 1.0,
        };
        // Half a turn of slack for the rounding of the turn count.
        if self.upper - self.lower >= TAU - 1e-9 {
            return unit;
        }
        let at = |value: f64| loose(function(value));
        let mut range = at(self.lower).hull(at(self.upper));
        let reaches = |extreme: f64| {
            // The first extreme at or above the lower bound, with slack: an
            // extreme within the slack of a bound counts as inside.
            let turns = ((self.lower - extreme) / TAU).floor();
            let mut candidate = extreme + turns * TAU;
            while candidate < self.lower - 1e-9 {
                candidate += TAU;
            }
            candidate <= self.upper + 1e-9
        };
        if reaches(peak) {
            range.upper = 1.0;
        }
        if reaches(peak + PI) {
            range.lower = -1.0;
        }
        Self {
            lower: range.lower.max(-1.0),
            upper: range.upper.min(1.0),
        }
    }

    /// The tangent of an angle in radians.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Domain`] when the interval reaches a pole.
    pub fn tan(self) -> IntervalResult {
        use std::f64::consts::{FRAC_PI_2, PI};
        // A pole at π/2 + kπ; slack for the rounding of the count.
        let pole = ((self.lower - FRAC_PI_2) / PI).ceil() * PI + FRAC_PI_2;
        if pole <= self.upper + 1e-9 || pole - PI >= self.lower - 1e-9 {
            return Err(IntervalFailure::Domain);
        }
        Self {
            lower: loose(self.lower.tan()).lower,
            upper: loose(self.upper.tan()).upper,
        }
        .finite()
    }

    /// The arctangent, an angle in radians in `(-π/2, π/2)`.
    #[must_use]
    pub fn atan(self) -> Self {
        Self {
            lower: loose(self.lower.atan()).lower,
            upper: loose(self.upper.atan()).upper,
        }
    }

    /// The angle of the vector `(x, y)` in radians, in `(-π, π]`.
    ///
    /// # Errors
    ///
    /// [`IntervalFailure::Domain`] when the box of vectors holds the origin
    /// or crosses the negative x axis, where the angle jumps.
    pub fn atan2(y: Self, x: Self) -> IntervalResult {
        if (y.contains(0.0) && x.lower <= 0.0) || (x.contains(0.0) && y.contains(0.0)) {
            return Err(IntervalFailure::Domain);
        }
        // Away from the origin and the cut, the angle's extremes over a box
        // lie at its corners.
        Ok([
            (y.lower, x.lower),
            (y.lower, x.upper),
            (y.upper, x.lower),
            (y.upper, x.upper),
        ]
        .into_iter()
        .map(|(y, x)| loose(y.atan2(x)))
        .reduce(Self::hull)
        .unwrap_or(y))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Interval, IntervalFailure};

    fn interval(lower: f64, upper: f64) -> Interval {
        Interval::new(lower, upper).unwrap()
    }

    #[test]
    fn exact_operations_on_points_stay_points() {
        let two = Interval::point(2.0);
        assert_eq!(two.plus(two), Ok(Interval::point(4.0)));
        assert_eq!(two.times(two), Ok(Interval::point(4.0)));
        assert_eq!(Interval::point(4.0).sqrt(), Ok(two));
        assert_eq!(
            Interval::point(1.0).divided_by(Interval::point(4.0)),
            Ok(Interval::point(0.25))
        );
    }

    #[test]
    fn rounded_results_widen_to_hold_the_exact_value() {
        let sum = Interval::point(0.1).plus(Interval::point(0.2)).unwrap();
        assert!(!sum.is_point());
        assert!(sum.contains(0.1 + 0.2));
        let third = Interval::point(1.0)
            .divided_by(Interval::point(3.0))
            .unwrap();
        assert!(third.lower < third.upper);
        assert!(third.contains(1.0 / 3.0));
    }

    #[test]
    fn failures_are_named() {
        assert_eq!(
            Interval::point(1.0).divided_by(interval(-1.0, 1.0)),
            Err(IntervalFailure::ZeroDivisor)
        );
        assert_eq!(interval(-1.0, 1.0).sqrt(), Err(IntervalFailure::Domain));
        assert_eq!(
            interval(1.0, 2.0).tan(),
            Err(IntervalFailure::Domain),
            "π/2 lies within"
        );
        assert_eq!(
            Interval::point(f64::MAX).plus(Interval::point(f64::MAX)),
            Err(IntervalFailure::Overflow)
        );
        assert_eq!(
            Interval::atan2(interval(-1.0, 1.0), Interval::point(-1.0)),
            Err(IntervalFailure::Domain)
        );
    }

    #[test]
    fn periodic_functions_reach_their_extremes_within() {
        let sine = interval(0.0, 3.0).sin();
        assert_eq!(sine.upper, 1.0);
        assert!(sine.lower <= 0.0 && sine.lower > -1e-12);
        let cosine = interval(3.0, 3.5).cos();
        assert_eq!(cosine.lower, -1.0);
        assert_eq!(interval(0.0, 7.0).sin(), interval(-1.0, 1.0));
    }

    #[test]
    fn rounding_to_a_step_is_monotone() {
        assert_eq!(
            interval(0.012, 0.018).round_to(Interval::point(0.005)),
            Ok(interval(0.01, 0.02))
        );
        assert_eq!(
            Interval::point(1.0).round_to(Interval::point(0.0)),
            Err(IntervalFailure::Domain)
        );
    }
}
