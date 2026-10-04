//! Units of measure: exponents of the SI base units and the radian, with
//! at most one currency, and their parsing from written symbols.
//!
//! One unit algebra serves every computed value: expressions and takeoff
//! columns check and combine units here.

use axioval_ir::QuantityDimension;

/// The base units an exponent counts: the seven SI base units, then the
/// radian, which SI counts dimensionless but a plane angle is never
/// compared with a plain number.
const BASES: [&str; 8] = ["m", "kg", "s", "A", "K", "mol", "cd", "rad"];
/// The radian's place in [`BASES`].
const RADIAN: usize = 7;

/// A unit: exponents of [`BASES`] and of at most one currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    exponents: [i8; 8],
    currency: Option<(String, i8)>,
}

impl Unit {
    /// A plain number.
    pub const NONE: Self = Self {
        exponents: [0; 8],
        currency: None,
    };

    /// The radian, the unit of a plane angle.
    pub const RADIAN: Self = Self {
        exponents: [0, 0, 0, 0, 0, 0, 0, 1],
        currency: None,
    };

    /// Whether it is a plain number's: no base unit and no currency.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        *self == Self::NONE
    }

    /// The unit whose square this is, if every exponent is even.
    #[must_use]
    pub fn sqrt(&self) -> Option<Self> {
        let mut exponents = self.exponents;
        for exponent in &mut exponents {
            if *exponent % 2 != 0 {
                return None;
            }
            *exponent /= 2;
        }
        let currency = match &self.currency {
            None => None,
            Some((code, exponent)) if exponent % 2 == 0 => Some((code.clone(), exponent / 2)),
            Some(_) => return None,
        };
        Some(Self {
            exponents,
            currency,
        })
    }

    /// The unit of `dimension`, or of a plain number.
    pub fn of(dimension: Option<QuantityDimension>) -> Self {
        let mut exponents = [0; 8];
        match dimension {
            None => {}
            Some(QuantityDimension::Length) => exponents[0] = 1,
            Some(QuantityDimension::Area) => exponents[0] = 2,
            Some(QuantityDimension::Volume) => exponents[0] = 3,
            Some(QuantityDimension::PlaneAngle) => exponents[RADIAN] = 1,
            Some(QuantityDimension::Other { exponents: si }) => {
                exponents[..7].copy_from_slice(&si);
            }
        }
        Self {
            exponents,
            currency: None,
        }
    }

    /// The dimension a quantity of this unit is reported in: `Ok(None)`
    /// for a plain number or an amount of a currency, refused for a mix of
    /// a plane angle and another unit, which no dimension names.
    pub fn dimension(&self) -> Result<Option<QuantityDimension>, String> {
        if self.currency.is_some() {
            return Ok(None);
        }
        let mut si = [0; 7];
        si.copy_from_slice(&self.exponents[..7]);
        match self.exponents[RADIAN] {
            0 => Ok(QuantityDimension::from_exponents(si)),
            1 if si == [0; 7] => Ok(Some(QuantityDimension::PlaneAngle)),
            _ => Err(format!("{self} names no dimension a takeoff reports")),
        }
    }

    /// Whether a plain number is a value of this unit: a dimensionless one,
    /// or one counting a currency.
    pub fn takes_plain_numbers(&self) -> bool {
        self.currency.is_some() || self.exponents == [0; 8]
    }

    /// Whether a quantity of `dimension` is a value of this unit.
    pub fn takes(&self, dimension: QuantityDimension) -> bool {
        self.currency.is_none() && *self == Self::of(Some(dimension))
    }

    /// Whether it counts a currency, so a column of it is a number column
    /// labelled with its unit.
    pub fn counts_currency(&self) -> bool {
        self.currency.is_some()
    }

    /// This unit times `other` raised to `sign` (1 multiplies, -1 divides).
    ///
    /// # Errors
    ///
    /// An exponent overflowing, or two different currencies.
    pub fn times(&self, other: &Self, sign: i8) -> Result<Self, String> {
        let mut exponents = self.exponents;
        for (exponent, added) in exponents.iter_mut().zip(other.exponents) {
            *exponent = added
                .checked_mul(sign)
                .and_then(|added| exponent.checked_add(added))
                .ok_or_else(|| "a unit exponent overflows".to_owned())?;
        }
        let currency = match (&self.currency, &other.currency) {
            (None, None) => None,
            (Some(own), None) => Some(own.clone()),
            (None, Some((code, exponent))) => Some((code.clone(), exponent * sign)),
            (Some((code, own)), Some((other_code, exponent))) if code == other_code => {
                Some((code.clone(), own + exponent * sign))
            }
            (Some((code, _)), Some((other_code, _))) => {
                return Err(format!("it mixes the currencies {code} and {other_code}"));
            }
        };
        Ok(Self {
            exponents,
            currency: currency.filter(|(_, exponent)| *exponent != 0),
        })
    }
}

impl std::fmt::Display for Unit {
    /// `m²`, `EUR·m⁻²`, `1` for a plain number.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if let Some((code, exponent)) = &self.currency {
            parts.push(power(code, *exponent));
        }
        for (base, exponent) in BASES.iter().zip(self.exponents) {
            if exponent != 0 {
                parts.push(power(base, exponent));
            }
        }
        if parts.is_empty() {
            f.write_str("1")
        } else {
            f.write_str(&parts.join("·"))
        }
    }
}

fn power(base: &str, exponent: i8) -> String {
    if exponent == 1 {
        return base.to_owned();
    }
    let digits: String = exponent
        .to_string()
        .chars()
        .map(|digit| match digit {
            '-' => '⁻',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            _ => '⁰',
        })
        .collect();
    format!("{base}{digits}")
}

/// A unit symbol's scale to the coherent unit and its exponents.
fn symbol(name: &str) -> Option<(f64, [i8; 8])> {
    const fn e(m: i8, kg: i8, s: i8, a: i8, k: i8, rad: i8) -> [i8; 8] {
        [m, kg, s, a, k, 0, 0, rad]
    }
    Some(match name {
        "m" => (1.0, e(1, 0, 0, 0, 0, 0)),
        "cm" => (1e-2, e(1, 0, 0, 0, 0, 0)),
        "mm" => (1e-3, e(1, 0, 0, 0, 0, 0)),
        "km" => (1e3, e(1, 0, 0, 0, 0, 0)),
        "l" | "L" => (1e-3, e(3, 0, 0, 0, 0, 0)),
        "g" => (1e-3, e(0, 1, 0, 0, 0, 0)),
        "kg" => (1.0, e(0, 1, 0, 0, 0, 0)),
        "t" => (1e3, e(0, 1, 0, 0, 0, 0)),
        "s" => (1.0, e(0, 0, 1, 0, 0, 0)),
        "min" => (60.0, e(0, 0, 1, 0, 0, 0)),
        "h" => (3600.0, e(0, 0, 1, 0, 0, 0)),
        "A" => (1.0, e(0, 0, 0, 1, 0, 0)),
        "K" => (1.0, e(0, 0, 0, 0, 1, 0)),
        "mol" => (1.0, [0, 0, 0, 0, 0, 1, 0, 0]),
        "cd" => (1.0, [0, 0, 0, 0, 0, 0, 1, 0]),
        "rad" => (1.0, e(0, 0, 0, 0, 0, 1)),
        "deg" | "°" => (std::f64::consts::PI / 180.0, e(0, 0, 0, 0, 0, 1)),
        "N" => (1.0, e(1, 1, -2, 0, 0, 0)),
        "kN" => (1e3, e(1, 1, -2, 0, 0, 0)),
        "Pa" => (1.0, e(-1, 1, -2, 0, 0, 0)),
        "kPa" => (1e3, e(-1, 1, -2, 0, 0, 0)),
        "MPa" => (1e6, e(-1, 1, -2, 0, 0, 0)),
        "J" => (1.0, e(2, 1, -2, 0, 0, 0)),
        "kWh" => (3.6e6, e(2, 1, -2, 0, 0, 0)),
        "W" => (1.0, e(2, 1, -3, 0, 0, 0)),
        "kW" => (1e3, e(2, 1, -3, 0, 0, 0)),
        _ => return None,
    })
}

/// A unit as written, such as `m2`, `m²`, `EUR/m²` or `W/m²·K`, and its
/// scale to the coherent unit: factors joined by `·` or `*`, each a
/// symbol with an optional exponent (`2`, `²`, `^-1`, `⁻¹`), every
/// factor after a `/` dividing. A currency is three capital letters
/// (`EUR`); `1` is a plain number.
pub fn parse_unit(text: &str) -> Result<(f64, Unit), String> {
    let text = text.trim();
    if text == "1" {
        return Ok((1.0, Unit::NONE));
    }
    let mut scale = 1.0;
    let mut unit = Unit::NONE;
    let (numerator, denominator) = match text.split_once('/') {
        Some((numerator, denominator)) => (numerator, Some(denominator)),
        None => (text, None),
    };
    if denominator.is_some_and(|denominator| denominator.contains('/')) {
        return Err(format!(
            "unit `{text}` divides twice; write every factor after one `/`"
        ));
    }
    for (part, sign) in [(numerator, 1), (denominator.unwrap_or(""), -1)] {
        if sign == -1 && denominator.is_none() {
            continue;
        }
        if part.trim() == "1" && sign == 1 {
            continue;
        }
        for factor in part.split(['·', '*']) {
            let (factor_scale, factor_unit) =
                factor_unit(factor.trim()).map_err(|why| format!("unit `{text}`: {why}"))?;
            scale = if sign == 1 {
                scale * factor_scale
            } else {
                scale / factor_scale
            };
            unit = unit
                .times(&factor_unit, sign)
                .map_err(|why| format!("unit `{text}`: {why}"))?;
        }
    }
    Ok((scale, unit))
}

/// One factor of a unit: a symbol and its exponent.
fn factor_unit(factor: &str) -> Result<(f64, Unit), String> {
    let split = factor
        .char_indices()
        .find(|(_, character)| !(character.is_ascii_alphabetic() || *character == '°'))
        .map_or(factor.len(), |(index, _)| index);
    let (name, exponent) = factor.split_at(split);
    if name.is_empty() {
        return Err(format!("`{factor}` names no unit"));
    }
    let exponent = exponent_of(exponent).ok_or_else(|| format!("`{factor}` has no exponent"))?;
    let (scale, exponents) = match symbol(name) {
        Some(found) => found,
        None if name.len() == 3 && name.bytes().all(|byte| byte.is_ascii_uppercase()) => {
            let unit = Unit {
                exponents: [0; 8],
                currency: Some((name.to_owned(), 1)),
            };
            return Ok((1.0, unit.pow(exponent)?));
        }
        None => return Err(format!("`{name}` is no unit symbol or currency code")),
    };
    let unit = Unit {
        exponents,
        currency: None,
    }
    .pow(exponent)?;
    Ok((scale.powi(i32::from(exponent)), unit))
}

impl Unit {
    /// This unit raised to `exponent`.
    ///
    /// # Errors
    ///
    /// An exponent overflowing.
    pub fn pow(&self, exponent: i8) -> Result<Self, String> {
        let overflow = || "a unit exponent overflows".to_owned();
        let mut exponents = self.exponents;
        for value in &mut exponents {
            *value = value.checked_mul(exponent).ok_or_else(overflow)?;
        }
        let currency = match &self.currency {
            Some((code, own)) => Some((
                code.clone(),
                own.checked_mul(exponent).ok_or_else(overflow)?,
            )),
            None => None,
        };
        Ok(Self {
            exponents,
            currency,
        })
    }
}

/// An exponent as written after a symbol: nothing (1), digits, `^` and
/// digits with an optional `-`, or superscripts.
fn exponent_of(text: &str) -> Option<i8> {
    if text.is_empty() {
        return Some(1);
    }
    let plain: String = text
        .trim_start_matches('^')
        .chars()
        .map(|character| match character {
            '⁻' => '-',
            '⁰' => '0',
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            '⁴' => '4',
            '⁵' => '5',
            '⁶' => '6',
            '⁷' => '7',
            '⁸' => '8',
            '⁹' => '9',
            other => other,
        })
        .collect();
    plain.parse::<i8>().ok().filter(|exponent| *exponent != 0)
}
