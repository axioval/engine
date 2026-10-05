//! The one comparison every rule decides with.
//!
//! The expression evaluator, the property selectors, the generic judges
//! (`property-predicate`, `property-comparison`, …) and the templates'
//! range judge all compare through this module, so one set of
//! comparison, tolerance and interval semantics serves every rule:
//!
//! - **Numbers** are intervals: every value they may take. A comparison
//!   is decided only where every pair of values the two intervals allow
//!   answers it alike ([`numbers`], through [`verdict`]); a straddling one
//!   is undecided, never a pass or a failure. A [`Tolerance`] widens
//!   equality (absolute and relative, or rounding to decimals); exact
//!   comparison takes none.
//! - **Integers** compare exactly ([`integers`]), and with a number only
//!   where their binary value is exact ([`exact_f64`]).
//! - **Text** is compared as stated, or case folded and trimmed as a rule
//!   declares ([`TextOptions`]); `like` and `matches` match the whole value
//!   ([`pattern`]).
//! - **Dates** compare as XML Schema orders them and date-times as
//!   instants ([`temporal_order`]); an order XML Schema leaves
//!   indeterminate decides equality (they differ) and no order
//!   ([`temporal_holds`]).
//! - **Truths** compare only for equality ([`booleans`]).
//!
//! What a comparison is *of* (a stated property, a literal, a measured
//! interval) and how its failure is worded stay with the caller; whether
//! it holds is decided here. `null`, the source stating no value, is
//! never compared: each caller decides what a comparison with nothing
//! means (`null` in an expression, no match in a selector, a failed
//! predicate).

use std::cmp::Ordering;

use axioval_ir::contract::ExpressionComparison;
use axioval_ir::{PropertyValue, TemporalPrecision};
use regex::{Regex, RegexBuilder};

/// An ordered or equality comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Order {
    /// Whether the comparison holds for two values ordered `ordering`.
    #[must_use]
    pub fn holds(self, ordering: Ordering) -> bool {
        match self {
            Self::Equal => ordering.is_eq(),
            Self::NotEqual => !ordering.is_eq(),
            Self::Less => ordering.is_lt(),
            Self::LessOrEqual => ordering.is_le(),
            Self::Greater => ordering.is_gt(),
            Self::GreaterOrEqual => ordering.is_ge(),
        }
    }

    /// Whether it tests equality rather than an order.
    #[must_use]
    pub fn is_equality(self) -> bool {
        matches!(self, Self::Equal | Self::NotEqual)
    }

    /// `Some(negated)` for an equality test (`false` for `Equal`, `true`
    /// for `NotEqual`), `None` for an order: how [`temporal_holds`]
    /// answers an indeterminate pair.
    #[must_use]
    pub fn equality(self) -> Option<bool> {
        match self {
            Self::Equal => Some(false),
            Self::NotEqual => Some(true),
            _ => None,
        }
    }

    /// The order an expression's comparison operator states; `None` for
    /// `like`, `matches` and `contains`.
    #[must_use]
    pub fn of(operator: ExpressionComparison) -> Option<Self> {
        use ExpressionComparison as C;
        Some(match operator {
            C::Equals => Self::Equal,
            C::NotEquals => Self::NotEqual,
            C::LessThan => Self::Less,
            C::LessThanOrEquals => Self::LessOrEqual,
            C::GreaterThan => Self::Greater,
            C::GreaterThanOrEquals => Self::GreaterOrEqual,
            C::Like | C::Matches | C::Contains => return None,
        })
    }
}

/// A declared numeric tolerance: absolute and relative, or rounding to
/// decimals.
///
/// With an absolute and/or a relative tolerance, two numbers are equal
/// when `|a - b| <= absolute + relative * max(|a|, |b|)`, the boundary
/// included. The bound is symmetric but not transitive. Values are
/// decimals as a reviewer reads them, so the comparison allows a few units
/// in the last place for binary rounding: `1.1` and `1.0` are within `0.1`.
///
/// With `decimals`, both numbers are first rounded half away from zero to
/// that many decimal places of their shortest decimal form (`2.345` rounds
/// to `2.35`, as displayed) and then compared exactly. Rounding is
/// transitive; it cannot be combined with a tolerance.
///
/// Quantities are compared in canonical SI units, so a tolerance or
/// rounding on a length is in metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tolerance {
    absolute: f64,
    relative: f64,
    decimals: Option<u32>,
}

/// The largest number of decimals a rule may round to.
pub const MAX_DECIMALS: i64 = 15;

impl Tolerance {
    /// Exact comparison: no tolerance and no rounding.
    pub const EXACT: Self = Self {
        absolute: 0.0,
        relative: 0.0,
        decimals: None,
    };

    /// A tolerance of `absolute` plus `relative` times the larger
    /// magnitude, or rounding to `decimals`; the caller has checked the
    /// declaration (non-negative, a relative tolerance below one, not both
    /// kinds).
    #[must_use]
    pub fn new(absolute: f64, relative: f64, decimals: Option<u32>) -> Self {
        Self {
            absolute,
            relative,
            decimals,
        }
    }

    /// Exact up to the binary rounding of one unit conversion.
    ///
    /// A quantity declared in `mm` is scaled to metres before it is
    /// compared with a value the source stated in metres; the product may
    /// differ from the decimal the author meant in the last place. A few
    /// units in the last place are equal, anything more is not.
    #[must_use]
    pub fn unit_conversion() -> Self {
        Self {
            relative: 4.0 * f64::EPSILON,
            ..Self::default()
        }
    }

    /// Whether this is exact comparison: no tolerance and no rounding.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.decimals.is_none() && self.absolute == 0.0 && self.relative == 0.0
    }

    /// Whether this rounds to decimals rather than allowing a distance.
    #[must_use]
    pub fn rounds(&self) -> bool {
        self.decimals.is_some()
    }

    /// `value` rounded as declared; unchanged without `decimals`.
    #[must_use]
    pub fn round(&self, value: f64) -> f64 {
        match self.decimals {
            Some(decimals) => round_decimal(value, decimals),
            None => value,
        }
    }

    /// Whether two finite numbers are equal under this tolerance.
    #[must_use]
    pub fn equal(&self, left: f64, right: f64) -> bool {
        if self.decimals.is_some() {
            return self.round(left).total_cmp(&self.round(right)).is_eq();
        }
        let magnitude = left.abs().max(right.abs());
        let bound = self.absolute + self.relative * magnitude;
        // Binary rounding of decimal inputs and of the bound itself; an
        // exact comparison takes none.
        let slack = if bound > 0.0 {
            4.0 * f64::EPSILON * magnitude.max(bound)
        } else {
            0.0
        };
        (left - right).abs() <= bound + slack
    }

    /// The order of two finite numbers, `Equal` when they are equal under
    /// this tolerance; `None` when either is not finite.
    #[must_use]
    pub fn order(&self, left: f64, right: f64) -> Option<Ordering> {
        if !left.is_finite() || !right.is_finite() {
            return None;
        }
        if self.equal(left, right) {
            Some(Ordering::Equal)
        } else {
            self.round(left).partial_cmp(&self.round(right))
        }
    }

    /// How findings state the tolerance, such as `within tolerance 0.01`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.decimals {
            Some(decimals) => format!("rounded to {decimals} decimal(s)"),
            None if self.relative == 0.0 => format!("within tolerance {}", self.absolute),
            None if self.absolute == 0.0 => {
                format!("within relative tolerance {}", self.relative)
            }
            None => format!(
                "within tolerance {} plus relative tolerance {}",
                self.absolute, self.relative
            ),
        }
    }

    /// ` (<description>)` for a finding message, or nothing when exact.
    #[must_use]
    pub fn suffix(&self) -> String {
        if self.is_exact() {
            String::new()
        } else {
            format!(" ({})", self.describe())
        }
    }
}

/// Rounds `value` half away from zero to `decimals` places of its shortest
/// decimal form.
///
/// # Panics
///
/// Never: the rounded digits always form a decimal literal.
#[must_use]
pub fn round_decimal(value: f64, decimals: u32) -> f64 {
    if !value.is_finite() {
        return value;
    }
    // `{:e}` prints the shortest digits that read back as `value`.
    let text = format!("{:e}", value.abs());
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return value;
    };
    let Ok(exponent) = exponent.parse::<i64>() else {
        return value;
    };
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    // Digits kept: those before the point plus `decimals` after it.
    let Ok(keep) = usize::try_from(exponent + 1 + i64::from(decimals)) else {
        // Every digit lies below half a unit of the last kept place.
        return 0.0;
    };
    if keep >= digits.len() {
        return value;
    }
    let kept = digits[..keep]
        .iter()
        .fold(0_u64, |total, digit| total * 10 + u64::from(digit - b'0'));
    let units = kept + u64::from(digits[keep] >= b'5');
    let rounded: f64 = format!("{units}e-{decimals}")
        .parse()
        .expect("a decimal literal parses");
    // `+ 0.0` turns a negative zero into zero, so it keys like zero.
    value.signum() * rounded + 0.0
}

/// Whether a comparison holds for every value of an interval whose least
/// and greatest ends order as `least` and `greatest` against the bound:
/// `Some` when every ordering between them gives one answer, `None` when
/// the interval straddles the bound. An ordering is monotone in the value,
/// so the orderings between the ends are every one the interval can take.
#[must_use]
pub fn verdict(
    least: Ordering,
    greatest: Ordering,
    holds: impl Fn(Ordering) -> bool,
) -> Option<bool> {
    let verdicts: Vec<bool> = [Ordering::Less, Ordering::Equal, Ordering::Greater]
        .into_iter()
        .filter(|ordering| least <= *ordering && *ordering <= greatest)
        .map(holds)
        .collect();
    let first = *verdicts.first()?;
    verdicts
        .iter()
        .all(|verdict| *verdict == first)
        .then_some(first)
}

/// Why two numbers are not compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undecided {
    /// The intervals allow values answering the comparison both ways.
    Straddles,
    /// A bound is not a finite number (under a tolerance), or not a number.
    NotFinite,
}

/// `left order right` for two numbers, each every value of an interval
/// `(lower, upper)`: decided only where every pair of values the two
/// allow answers alike. The least ordering is the left's least against
/// the right's greatest, the greatest the left's greatest against the
/// right's least; a point is an interval of one value.
///
/// Exact comparison orders by IEEE comparison (so `-0` equals `0`, and an
/// infinite bound orders); under a tolerance a bound that is not finite
/// leaves the comparison undecided.
///
/// # Errors
///
/// [`Undecided`] where the intervals straddle, or a bound is no number.
pub fn numbers(
    order: Order,
    left: (f64, f64),
    right: (f64, f64),
    tolerance: &Tolerance,
) -> Result<bool, Undecided> {
    let ordering = |a: f64, b: f64| {
        if tolerance.is_exact() {
            a.partial_cmp(&b)
        } else {
            tolerance.order(a, b)
        }
    };
    let (Some(least), Some(greatest)) = (ordering(left.0, right.1), ordering(left.1, right.0))
    else {
        return Err(Undecided::NotFinite);
    };
    verdict(least, greatest, |ordering| order.holds(ordering)).ok_or(Undecided::Straddles)
}

/// `left order right` for two integers, exactly.
#[must_use]
pub fn integers(order: Order, left: i64, right: i64) -> bool {
    order.holds(left.cmp(&right))
}

/// `value` as a float, when the conversion is exact (magnitude up to 2^53).
#[must_use]
pub fn exact_f64(value: i64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    (value.unsigned_abs() <= 1 << 53).then_some(value as f64)
}

/// `left order right` for two truths: `None` for an order, which truths
/// do not have.
#[must_use]
pub fn booleans(order: Order, left: bool, right: bool) -> Option<bool> {
    order.is_equality().then(|| order.holds(left.cmp(&right)))
}

/// How text is compared: case folded unless `case_sensitive`, and the
/// value trimmed of surrounding whitespace where `trim`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextOptions {
    pub case_sensitive: bool,
    pub trim: bool,
}

impl TextOptions {
    /// Text as stated: case-sensitive, untrimmed.
    pub const STATED: Self = Self {
        case_sensitive: true,
        trim: false,
    };

    /// Case folded unless `case_sensitive`, untrimmed.
    #[must_use]
    pub const fn case(case_sensitive: bool) -> Self {
        Self {
            case_sensitive,
            trim: false,
        }
    }

    /// Whether these are the defaults: case-sensitive, untrimmed.
    #[must_use]
    pub fn is_default(self) -> bool {
        self == Self::STATED
    }

    /// `text` folded as declared (a declared side: never trimmed).
    #[must_use]
    pub fn fold(self, text: &str) -> String {
        if self.case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    }

    /// A compared value: trimmed, then folded, as declared.
    #[must_use]
    pub fn prepare(self, text: &str) -> String {
        self.fold(if self.trim { text.trim() } else { text })
    }
}

/// `value order expected` for text, `value` prepared and `expected`
/// already folded as `options` declare ([`TextOptions::fold`]).
#[must_use]
pub fn texts(order: Order, value: &str, expected: &str, options: TextOptions) -> bool {
    order.holds(options.prepare(value).as_str().cmp(expected))
}

/// Whether `value`, prepared as `options` declare, contains `needle`
/// (already folded).
#[must_use]
pub fn contains(value: &str, needle: &str, options: TextOptions) -> bool {
    options.prepare(value).contains(needle)
}

/// Whether `value`, prepared as `options` declare, is one of `members`
/// (each already folded).
#[must_use]
pub fn member<S: AsRef<str>>(value: &str, members: &[S], options: TextOptions) -> bool {
    let value = options.prepare(value);
    members.iter().any(|member| member.as_ref() == value)
}

/// What a text pattern is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// A wildcard pattern: `*` any run of characters, `?` one, `\` makes
    /// the next character literal.
    Like,
    /// A regular expression.
    Matches,
}

impl Pattern {
    /// How a message names the pattern's kind.
    #[must_use]
    pub fn kind(self) -> &'static str {
        match self {
            Self::Like => "wildcard pattern",
            Self::Matches => "regular expression",
        }
    }
}

/// Why a pattern does not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    /// A wildcard pattern is malformed (it ends with a lone backslash).
    Wildcard(String),
    /// The regular expression does not compile.
    Compile(String),
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wildcard(why) | Self::Compile(why) => f.write_str(why),
        }
    }
}

/// `source` compiled as a `like` or `matches` pattern matching the whole
/// value, case-insensitive unless `case_sensitive`. A pattern is matched
/// against the value as stated (trimmed where a rule declares it), never
/// against a case-folded copy: folding is the regular expression's own.
///
/// # Errors
///
/// Why the pattern does not compile.
pub fn pattern(kind: Pattern, source: &str, case_sensitive: bool) -> Result<Regex, PatternError> {
    let anchored = match kind {
        Pattern::Like => crate::wildcard_regex(source).map_err(PatternError::Wildcard)?,
        Pattern::Matches => format!("^(?:{source})$"),
    };
    RegexBuilder::new(&anchored)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| PatternError::Compile(error.to_string()))
}

/// The order of two dates or date-times; `None` when either is neither.
///
/// Dates order as XML Schema orders them ([`axioval_ir::Date::cmp_timeline`])
/// and date-times as instants, whatever their offsets. `Ok(None)` is XML
/// Schema's indeterminate order: a date stating a time zone and one stating
/// none lie within 14 hours of each other, so neither precedes, equals nor
/// follows the other. Equality is then decided (they differ) and an order
/// is not (see [`temporal_holds`]). A date-time and a date compare only
/// at `day` precision, which reads every date-time and every date as the
/// calendar day it states, its time zone aside; exactly, a date-time
/// neither precedes nor follows the day it falls on, so the pair is an
/// error, never a verdict.
#[must_use]
pub fn temporal_order(
    left: &PropertyValue,
    right: &PropertyValue,
    precision: Option<TemporalPrecision>,
) -> Option<Result<Option<Ordering>, String>> {
    let day = |value: &PropertyValue| match value {
        PropertyValue::Date(date) => Some(date.calendar_day()),
        PropertyValue::DateTime(instant) => Some(instant.date()),
        _ => None,
    };
    let (left_day, right_day) = (day(left)?, day(right)?);
    Some(match (left, right, precision) {
        (_, _, Some(TemporalPrecision::Day)) => Ok(Some(left_day.cmp(&right_day))),
        (PropertyValue::Date(left), PropertyValue::Date(right), None) => {
            Ok(left.cmp_timeline(*right))
        }
        (PropertyValue::DateTime(left), PropertyValue::DateTime(right), None) => {
            Ok(Some(left.cmp_instant(*right)))
        }
        _ => Err(
            "a date-time compares with a date only at day precision; declare precision `day`"
                .into(),
        ),
    })
}

/// Why an order of two dates XML Schema leaves indeterminate is not decided.
pub const INCOMPARABLE_DATES: &str = "a date stating a time zone and one stating none \
     lie within 14 hours of each other, so neither precedes the other";

/// Whether an equality or order holds between two dates or date-times
/// whose order may be indeterminate (see [`temporal_order`]).
///
/// `is` judges a decided ordering. `equality` is `Some(false)` for an
/// equality test and `Some(true)` for an inequality test, which an
/// indeterminate pair answers exactly (they differ), and `None` for an
/// order, which it cannot answer.
///
/// # Errors
///
/// [`INCOMPARABLE_DATES`] for an order of an indeterminate pair.
pub fn temporal_holds(
    ordering: Option<Ordering>,
    equality: Option<bool>,
    is: impl FnOnce(Ordering) -> bool,
) -> Result<bool, String> {
    match (ordering, equality) {
        (Some(ordering), _) => Ok(is(ordering)),
        (None, Some(negated)) => Ok(negated),
        (None, None) => Err(INCOMPARABLE_DATES.into()),
    }
}

/// `left order right` for two dates or date-times at `precision`; `None`
/// when either is neither.
///
/// # Errors
///
/// A date-time against a date without `day` precision, or an order XML
/// Schema leaves indeterminate.
#[must_use]
pub fn temporal(
    order: Order,
    left: &PropertyValue,
    right: &PropertyValue,
    precision: Option<TemporalPrecision>,
) -> Option<Result<bool, String>> {
    temporal_order(left, right, precision).map(|ordering| {
        temporal_holds(ordering?, order.equality(), |ordering| {
            order.holds(ordering)
        })
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Order, Tolerance, Undecided, numbers, round_decimal, verdict};
    use std::cmp::Ordering;

    const ORDERS: [Order; 6] = [
        Order::Equal,
        Order::NotEqual,
        Order::Less,
        Order::LessOrEqual,
        Order::Greater,
        Order::GreaterOrEqual,
    ];

    /// The evaluator's interval comparison before it moved here, kept to
    /// prove the shared one decides every pair of intervals alike.
    fn decide(order: Order, left: (f64, f64), right: (f64, f64)) -> Option<bool> {
        let below = left.1 < right.0;
        let above = left.0 > right.1;
        let equal = left.0 == left.1 && right.0 == right.1 && left == right;
        let at_most = left.1 <= right.0;
        let at_least = left.0 >= right.1;
        match order {
            Order::Equal => (equal || below || above).then_some(equal),
            Order::NotEqual => (equal || below || above).then_some(!equal),
            Order::Less => (below || at_least).then_some(below),
            Order::LessOrEqual => (at_most || above).then_some(at_most),
            Order::Greater => (above || at_most).then_some(above),
            Order::GreaterOrEqual => (at_least || below).then_some(at_least),
        }
    }

    /// The range judge's bound check before it moved here.
    fn judged(lower: f64, upper: f64, minimum: f64, exclusive: bool) -> Option<bool> {
        let below = |measured: f64| measured < minimum || (exclusive && measured <= minimum);
        if below(upper) {
            Some(false)
        } else if below(lower) {
            None
        } else {
            Some(true)
        }
    }

    fn intervals() -> Vec<(f64, f64)> {
        let ends = [
            -2.0,
            -1.0,
            -0.0,
            0.0,
            0.5,
            1.0,
            1.0 + f64::EPSILON,
            2.0,
            3.0,
        ];
        let mut intervals = Vec::new();
        for lower in ends {
            for upper in ends {
                if lower <= upper {
                    intervals.push((lower, upper));
                }
            }
        }
        intervals
    }

    #[test]
    fn exact_interval_comparison_is_the_evaluators() {
        for order in ORDERS {
            for left in intervals() {
                for right in intervals() {
                    assert_eq!(
                        numbers(order, left, right, &Tolerance::EXACT).ok(),
                        decide(order, left, right),
                        "{order:?} {left:?} {right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_bound_check_is_the_range_judges() {
        for (lower, upper) in intervals() {
            for minimum in [-1.0, 0.0, 0.5, 1.0, 2.0] {
                for (exclusive, order) in [(false, Order::GreaterOrEqual), (true, Order::Greater)] {
                    assert_eq!(
                        numbers(order, (lower, upper), (minimum, minimum), &Tolerance::EXACT).ok(),
                        judged(lower, upper, minimum, exclusive),
                    );
                }
            }
        }
    }

    #[test]
    fn a_tolerance_widens_equality_and_refuses_what_is_not_finite() {
        let unit = Tolerance::unit_conversion();
        // `9 mm` scaled to metres is not the double nearest `0.009`.
        let scaled = 9.0 * 1e-3;
        assert_ne!(scaled, 0.009);
        assert_eq!(
            numbers(Order::Equal, (0.009, 0.009), (scaled, scaled), &unit),
            Ok(true)
        );
        assert_eq!(
            numbers(
                Order::Equal,
                (0.009, 0.009),
                (scaled, scaled),
                &Tolerance::EXACT
            ),
            Ok(false)
        );
        assert_eq!(
            numbers(Order::Less, (0.0, 2.0), (1.0, 1.0), &unit),
            Err(Undecided::Straddles)
        );
        assert_eq!(
            numbers(Order::Less, (0.0, f64::INFINITY), (1.0, 1.0), &unit),
            Err(Undecided::NotFinite)
        );
        assert_eq!(
            numbers(
                Order::Less,
                (0.0, 0.5),
                (f64::INFINITY, f64::INFINITY),
                &Tolerance::EXACT
            ),
            Ok(true)
        );
        assert_eq!(
            numbers(Order::Less, (f64::NAN, 0.5), (1.0, 1.0), &Tolerance::EXACT),
            Err(Undecided::NotFinite)
        );
    }

    #[test]
    fn a_verdict_needs_every_ordering_between_the_ends_to_agree() {
        let holds = |ordering: Ordering| ordering.is_le();
        assert_eq!(verdict(Ordering::Less, Ordering::Equal, holds), Some(true));
        assert_eq!(verdict(Ordering::Less, Ordering::Greater, holds), None);
        assert_eq!(
            verdict(Ordering::Greater, Ordering::Greater, holds),
            Some(false)
        );
    }

    #[test]
    fn rounding_reads_the_shortest_decimal_form_half_away_from_zero() {
        assert_eq!(round_decimal(2.345, 2), 2.35);
        assert_eq!(round_decimal(1.005, 2), 1.01);
        assert_eq!(round_decimal(2.344_999, 2), 2.34);
        assert_eq!(round_decimal(-2.345, 2), -2.35);
        assert_eq!(round_decimal(0.6, 0), 1.0);
        assert_eq!(round_decimal(0.4, 0), 0.0);
        assert_eq!(round_decimal(0.000_4, 2), 0.0);
        assert_eq!(round_decimal(9.999, 2), 10.0);
        assert_eq!(round_decimal(123.0, 2), 123.0);
        assert_eq!(round_decimal(1e300, 2), 1e300);
        assert!(round_decimal(-0.001, 2).is_sign_positive());
    }

    #[test]
    fn a_tolerance_includes_its_boundary_as_written_in_decimal() {
        let absolute = Tolerance::new(0.1, 0.0, None);
        assert!(absolute.equal(1.0, 1.1));
        assert!(absolute.equal(1.1, 1.0));
        assert!(!absolute.equal(1.0, 1.100_001));
        let relative = Tolerance::new(0.0, 0.25, None);
        assert!(relative.equal(3.0, 4.0));
        assert!(!relative.equal(2.9, 4.0));
        assert!(Tolerance::default().is_exact());
        assert!(!Tolerance::default().equal(1.0, 1.0 + f64::EPSILON * 8.0));
    }
}
