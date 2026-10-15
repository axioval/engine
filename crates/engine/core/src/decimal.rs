//! Stated decimals: the decimal a source or a rule writes, read back from
//! the double it parsed to, and carried exactly through a change of unit.
//!
//! A source stating `950 mm` and a rule stating `0.95 m` state the same
//! length, but `950.0 * 0.001` is a rounding step above the double `0.95`
//! parses to. [`coherent`] converts a stated value by a decimal prefix as
//! the decimal it states, to the double nearest the converted decimal, so
//! both read as the same double and compare as the decimals do. Nothing is
//! read as a decimal that is not the double nearest one, so no value near a
//! bound is ever taken for it.

/// One more than the largest integer of fifteen digits.
const DECIMAL_LIMIT: f64 = 1e15;

/// The largest power of ten a double holds exactly.
const EXACT_POWER: u32 = 22;

/// The decimal `value` is the double nearest to, as integer digits (fewer
/// than sixteen) over `10^places` (at most fifteen), the fewest places
/// first; `None` for any other value (a non-finite one, or one only near a
/// short decimal, such as `1.0.next_down()`).
///
/// Distinct decimals of at most fifteen significant digits are distinct
/// doubles, so the decimal found is the only one of that length the value
/// can have been read from.
#[must_use]
#[allow(clippy::float_cmp, clippy::cast_possible_truncation)]
pub fn stated(value: f64) -> Option<(i64, u32)> {
    (0..=15_u32).find_map(|places| {
        let scale = 10_f64.powi(places.cast_signed());
        let digits = (value * scale).round();
        // Digits and scale are exact in a double, so the quotient is the
        // double nearest the decimal.
        (digits.abs() < DECIMAL_LIMIT && digits / scale == value).then_some((digits as i64, places))
    })
}

/// The double nearest `digits × 10^exponent`; `None` where the power is
/// not exact in a double, so the quotient or product would round twice.
#[allow(clippy::cast_precision_loss)]
fn nearest(digits: i64, exponent: i32) -> Option<f64> {
    // Fewer than sixteen digits, so exact in a double.
    let digits = digits as f64;
    let power = 10_f64.powi(exponent.checked_abs()?);
    (exponent.unsigned_abs() <= EXACT_POWER).then(|| {
        // One correctly rounded operation on exact operands.
        if exponent < 0 {
            digits / power
        } else {
            digits * power
        }
    })
}

/// The power of ten `scale` is (`-3` for `0.001`, the scale of `mm`),
/// within a rounding step of it, if it is one.
#[allow(clippy::float_cmp)]
fn decimal_exponent(scale: f64) -> Option<i32> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let exponent = scale.log10().round();
    if exponent.abs() > f64::from(EXACT_POWER) {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let exponent = exponent as i32;
    let power = 10_f64.powi(exponent.abs());
    let exact = if exponent < 0 { 1.0 / power } else { power };
    ((exact - scale).abs() <= f64::EPSILON * scale).then_some(exponent)
}

/// `value` stated in a unit `scale` coherent units large, in coherent
/// units. A decimal prefix (`mm`, `cm`, `km`) converts the decimal `value`
/// states ([`stated`]) exactly, to the double nearest the converted
/// decimal, so `950 mm` is the same double as `0.95 m`; any other value or
/// scale is multiplied, rounding once.
#[must_use]
pub fn coherent(value: f64, scale: f64) -> f64 {
    decimal_exponent(scale)
        .and_then(|exponent| shifted(value, exponent))
        .unwrap_or(value * scale)
}

/// `value` in coherent units as written in a unit `scale` coherent units
/// large, the inverse of [`coherent`]: `0.95` metres is `950` millimetres
/// exactly. Any other value or scale is divided, rounding once.
#[must_use]
pub fn in_unit(value: f64, scale: f64) -> f64 {
    decimal_exponent(scale)
        .and_then(|exponent| shifted(value, exponent.checked_neg()?))
        .unwrap_or(value / scale)
}

/// The double nearest the decimal `value` states times `10^exponent`.
fn shifted(value: f64, exponent: i32) -> Option<f64> {
    let (digits, places) = stated(value)?;
    nearest(digits, exponent.checked_sub(places.cast_signed())?)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{coherent, in_unit, stated};

    #[test]
    fn a_stated_decimal_reads_back_from_its_double() {
        assert_eq!(stated(0.9), Some((9, 1)));
        assert_eq!(stated(950.0), Some((950, 0)));
        assert_eq!(stated(-0.05), Some((-5, 2)));
        assert_eq!(stated(1.0_f64.next_down()), None);
        assert_eq!(stated(1.0 / 3.0), None);
        assert_eq!(stated(f64::NAN), None);
        assert_eq!(stated(f64::INFINITY), None);
    }

    #[test]
    fn a_decimal_prefix_converts_the_stated_decimal() {
        // 950 × 0.001 rounds a step above 0.95.
        assert_ne!(950.0 * 0.001, 0.95);
        assert_eq!(coherent(950.0, 0.001), 0.95);
        assert_eq!(coherent(1400.0, 1e-3), 1.4);
        assert_eq!(coherent(2010.0, 1e-3), 2.01);
        assert_eq!(coherent(12.5, 1e-3), 0.0125);
        assert_eq!(coherent(0.1, 1e-3), 0.0001);
        assert_eq!(coherent(95.0, 1e-2), 0.95);
        assert_eq!(coherent(1.2, 1e3), 1200.0);
        assert_eq!(coherent(0.9, 1.0), 0.9);
        assert_eq!(in_unit(0.95, 1e-3), 950.0);
        assert_eq!(in_unit(0.0125, 1e-3), 12.5);
        // Every whole number of millimetres reads as its decimal in metres.
        for millimetres in 0..20_000_u32 {
            let decimal: f64 = format!("{}.{:03}", millimetres / 1000, millimetres % 1000)
                .parse()
                .unwrap();
            assert_eq!(coherent(f64::from(millimetres), 1e-3), decimal);
        }
    }

    #[test]
    fn any_other_value_or_scale_is_multiplied() {
        let near = 1.0_f64.next_down();
        assert_eq!(coherent(near, 1e-3), near * 1e-3);
        let degrees = std::f64::consts::PI / 180.0;
        assert_eq!(coherent(90.0, degrees), 90.0 * degrees);
        assert_eq!(coherent(1.0, 0.3048), 0.3048);
    }
}
