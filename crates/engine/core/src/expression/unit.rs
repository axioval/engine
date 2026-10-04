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

/// A unit: exponents of the SI base units and the radian, and of at most
/// one currency.
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

/// One unit symbol a written unit may use: its scale to the coherent unit
/// and its exponents of the SI base units and the radian.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitSymbol {
    pub symbol: &'static str,
    /// Other spellings of the same symbol.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub aliases: &'static [&'static str],
    /// The factor to the coherent unit (`mm` is `0.001` m).
    pub scale: f64,
    /// Exponents of `m`, `kg`, `s`, `A`, `K`, `mol`, `cd` and `rad`.
    pub exponents: [i8; 8],
    pub label: &'static [axioval_ir::measured::LocalizedText],
}

const fn e(m: i8, kg: i8, s: i8, a: i8, k: i8, rad: i8) -> [i8; 8] {
    [m, kg, s, a, k, 0, 0, rad]
}

/// Every unit symbol a written unit may use, in a stable order; a factor
/// may raise it to a power, and a currency (three capital letters) or `1`
/// stands beside them.
pub const UNIT_SYMBOLS: &[UnitSymbol] = &[
    UnitSymbol {
        symbol: "m",
        aliases: &[],
        scale: 1.0,
        exponents: e(1, 0, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("metre", "Meter"),
    },
    UnitSymbol {
        symbol: "cm",
        aliases: &[],
        scale: 1e-2,
        exponents: e(1, 0, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("centimetre", "Zentimeter"),
    },
    UnitSymbol {
        symbol: "mm",
        aliases: &[],
        scale: 1e-3,
        exponents: e(1, 0, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("millimetre", "Millimeter"),
    },
    UnitSymbol {
        symbol: "km",
        aliases: &[],
        scale: 1e3,
        exponents: e(1, 0, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilometre", "Kilometer"),
    },
    UnitSymbol {
        symbol: "l",
        aliases: &["L"],
        scale: 1e-3,
        exponents: e(3, 0, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("litre", "Liter"),
    },
    UnitSymbol {
        symbol: "g",
        aliases: &[],
        scale: 1e-3,
        exponents: e(0, 1, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("gram", "Gramm"),
    },
    UnitSymbol {
        symbol: "kg",
        aliases: &[],
        scale: 1.0,
        exponents: e(0, 1, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilogram", "Kilogramm"),
    },
    UnitSymbol {
        symbol: "t",
        aliases: &[],
        scale: 1e3,
        exponents: e(0, 1, 0, 0, 0, 0),
        label: &axioval_ir::measured::en_de("tonne", "Tonne"),
    },
    UnitSymbol {
        symbol: "s",
        aliases: &[],
        scale: 1.0,
        exponents: e(0, 0, 1, 0, 0, 0),
        label: &axioval_ir::measured::en_de("second", "Sekunde"),
    },
    UnitSymbol {
        symbol: "min",
        aliases: &[],
        scale: 60.0,
        exponents: e(0, 0, 1, 0, 0, 0),
        label: &axioval_ir::measured::en_de("minute", "Minute"),
    },
    UnitSymbol {
        symbol: "h",
        aliases: &[],
        scale: 3600.0,
        exponents: e(0, 0, 1, 0, 0, 0),
        label: &axioval_ir::measured::en_de("hour", "Stunde"),
    },
    UnitSymbol {
        symbol: "A",
        aliases: &[],
        scale: 1.0,
        exponents: e(0, 0, 0, 1, 0, 0),
        label: &axioval_ir::measured::en_de("ampere", "Ampere"),
    },
    UnitSymbol {
        symbol: "K",
        aliases: &[],
        scale: 1.0,
        exponents: e(0, 0, 0, 0, 1, 0),
        label: &axioval_ir::measured::en_de("kelvin", "Kelvin"),
    },
    UnitSymbol {
        symbol: "mol",
        aliases: &[],
        scale: 1.0,
        exponents: [0, 0, 0, 0, 0, 1, 0, 0],
        label: &axioval_ir::measured::en_de("mole", "Mol"),
    },
    UnitSymbol {
        symbol: "cd",
        aliases: &[],
        scale: 1.0,
        exponents: [0, 0, 0, 0, 0, 0, 1, 0],
        label: &axioval_ir::measured::en_de("candela", "Candela"),
    },
    UnitSymbol {
        symbol: "rad",
        aliases: &[],
        scale: 1.0,
        exponents: e(0, 0, 0, 0, 0, 1),
        label: &axioval_ir::measured::en_de("radian", "Radiant"),
    },
    UnitSymbol {
        symbol: "deg",
        aliases: &["°"],
        scale: std::f64::consts::PI / 180.0,
        exponents: e(0, 0, 0, 0, 0, 1),
        label: &axioval_ir::measured::en_de("degree", "Grad"),
    },
    UnitSymbol {
        symbol: "N",
        aliases: &[],
        scale: 1.0,
        exponents: e(1, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("newton", "Newton"),
    },
    UnitSymbol {
        symbol: "kN",
        aliases: &[],
        scale: 1e3,
        exponents: e(1, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilonewton", "Kilonewton"),
    },
    UnitSymbol {
        symbol: "Pa",
        aliases: &[],
        scale: 1.0,
        exponents: e(-1, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("pascal", "Pascal"),
    },
    UnitSymbol {
        symbol: "kPa",
        aliases: &[],
        scale: 1e3,
        exponents: e(-1, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilopascal", "Kilopascal"),
    },
    UnitSymbol {
        symbol: "MPa",
        aliases: &[],
        scale: 1e6,
        exponents: e(-1, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("megapascal", "Megapascal"),
    },
    UnitSymbol {
        symbol: "J",
        aliases: &[],
        scale: 1.0,
        exponents: e(2, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("joule", "Joule"),
    },
    UnitSymbol {
        symbol: "kWh",
        aliases: &[],
        scale: 3.6e6,
        exponents: e(2, 1, -2, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilowatt-hour", "Kilowattstunde"),
    },
    UnitSymbol {
        symbol: "W",
        aliases: &[],
        scale: 1.0,
        exponents: e(2, 1, -3, 0, 0, 0),
        label: &axioval_ir::measured::en_de("watt", "Watt"),
    },
    UnitSymbol {
        symbol: "kW",
        aliases: &[],
        scale: 1e3,
        exponents: e(2, 1, -3, 0, 0, 0),
        label: &axioval_ir::measured::en_de("kilowatt", "Kilowatt"),
    },
];

/// A unit symbol's scale to the coherent unit and its exponents.
fn symbol(name: &str) -> Option<(f64, [i8; 8])> {
    UNIT_SYMBOLS
        .iter()
        .find(|unit| unit.symbol == name || unit.aliases.contains(&name))
        .map(|unit| (unit.scale, unit.exponents))
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

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::parse_unit;
    use axioval_ir::QuantityDimension;

    #[test]
    fn units_parse_with_scales_exponents_and_currencies() {
        let (scale, unit) = parse_unit("EUR/m²").unwrap();
        assert_eq!(scale, 1.0);
        assert_eq!(unit.to_string(), "EUR·m⁻²");
        assert!(unit.dimension().unwrap().is_none());
        let (scale, unit) = parse_unit("cm2").unwrap();
        assert!((scale - 1e-4).abs() < 1e-18);
        assert_eq!(unit.dimension().unwrap(), Some(QuantityDimension::Area));
        let (_, unit) = parse_unit("W/m^2·K").unwrap();
        assert_eq!(unit.to_string(), "kg·s⁻³·K⁻¹");
        assert_eq!(parse_unit("m⁻¹").unwrap().1.to_string(), "m⁻¹");
        assert_eq!(parse_unit("1").unwrap().1.to_string(), "1");
        assert!(parse_unit("parsec").is_err());
        assert!(parse_unit("m/s/s").is_err());
        assert!(parse_unit("EUR·USD").is_err());
        assert!(parse_unit("rad·m").unwrap().1.dimension().is_err());
    }
}
