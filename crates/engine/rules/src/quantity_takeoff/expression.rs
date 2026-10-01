//! Computed takeoff columns: a small arithmetic expression over the other
//! columns of a member, parsed and unit-checked when the rule is bound and
//! evaluated over intervals, never run as code.

use axioval_ir::QuantityDimension;

/// The longest expression accepted, in characters.
const MAX_LENGTH: usize = 1000;
/// How deeply an expression may nest.
const MAX_DEPTH: usize = 32;

/// The base units an exponent counts: the seven SI base units, then the
/// radian, which SI counts dimensionless but a plane angle is never
/// compared with a plain number.
const BASES: [&str; 8] = ["m", "kg", "s", "A", "K", "mol", "cd", "rad"];
/// The radian's place in [`BASES`].
const RADIAN: usize = 7;

/// A unit: exponents of [`BASES`] and of at most one currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Unit {
    exponents: [i8; 8],
    currency: Option<(String, i8)>,
}

impl Unit {
    const NONE: Self = Self {
        exponents: [0; 8],
        currency: None,
    };

    /// The unit of `dimension`, or of a plain number.
    pub(super) fn of(dimension: Option<QuantityDimension>) -> Self {
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
    pub(super) fn dimension(&self) -> Result<Option<QuantityDimension>, String> {
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
    pub(super) fn takes_plain_numbers(&self) -> bool {
        self.currency.is_some() || self.exponents == [0; 8]
    }

    /// Whether a quantity of `dimension` is a value of this unit.
    pub(super) fn takes(&self, dimension: QuantityDimension) -> bool {
        self.currency.is_none() && *self == Self::of(Some(dimension))
    }

    /// Whether it counts a currency, so a column of it is a number column
    /// labelled with its unit.
    pub(super) fn counts_currency(&self) -> bool {
        self.currency.is_some()
    }

    fn times(&self, other: &Self, sign: i8) -> Result<Self, String> {
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
pub(super) fn parse_unit(text: &str) -> Result<(f64, Unit), String> {
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
    fn pow(&self, exponent: i8) -> Result<Self, String> {
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

/// How binding resolves a column name: its input number and unit, or why
/// it names none.
pub(super) type Resolve<'a> = dyn FnMut(&str) -> Result<(usize, Unit), String> + 'a;

/// A bound expression: its tree, over inputs numbered in the order the
/// resolver named them, and the unit of its value.
#[derive(Debug)]
pub(super) struct Expression {
    root: Node,
    unit: Unit,
}

#[derive(Debug)]
enum Node {
    Literal(f64),
    Input(usize),
    Negate(Box<Node>),
    Add(Box<Node>, Box<Node>),
    Subtract(Box<Node>, Box<Node>),
    Multiply(Box<Node>, Box<Node>),
    Divide(Box<Node>, Box<Node>),
    Min(Vec<Node>),
    Max(Vec<Node>),
}

/// A closed interval of a value in coherent units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Interval {
    pub(super) lower: f64,
    pub(super) upper: f64,
}

/// Why an expression has no value for one member.
#[derive(Debug, PartialEq)]
pub(super) enum Failure {
    /// A divisor's interval holds zero.
    ZeroDivisor,
    /// A bound is not finite.
    Overflow,
}

impl Expression {
    /// The unit of the expression's value.
    pub(super) fn unit(&self) -> &Unit {
        &self.unit
    }

    /// The value for `inputs`, one interval per input in resolver order:
    /// an interval sure to hold every value the inputs allow, up to
    /// binary rounding.
    pub(super) fn evaluate(&self, inputs: &[Interval]) -> Result<Interval, Failure> {
        let value = evaluate(&self.root, inputs)?;
        if value.lower.is_finite() && value.upper.is_finite() {
            Ok(value)
        } else {
            Err(Failure::Overflow)
        }
    }
}

fn evaluate(node: &Node, inputs: &[Interval]) -> Result<Interval, Failure> {
    let pair = |left: &Node, right: &Node| -> Result<(Interval, Interval), Failure> {
        Ok((evaluate(left, inputs)?, evaluate(right, inputs)?))
    };
    Ok(match node {
        Node::Literal(value) => Interval {
            lower: *value,
            upper: *value,
        },
        Node::Input(index) => inputs[*index],
        Node::Negate(inner) => {
            let inner = evaluate(inner, inputs)?;
            Interval {
                lower: -inner.upper,
                upper: -inner.lower,
            }
        }
        Node::Add(left, right) => {
            let (left, right) = pair(left, right)?;
            Interval {
                lower: left.lower + right.lower,
                upper: left.upper + right.upper,
            }
        }
        Node::Subtract(left, right) => {
            let (left, right) = pair(left, right)?;
            Interval {
                lower: left.lower - right.upper,
                upper: left.upper - right.lower,
            }
        }
        Node::Multiply(left, right) => {
            let (left, right) = pair(left, right)?;
            corners(left, right, |a, b| a * b)
        }
        Node::Divide(left, right) => {
            let (left, right) = pair(left, right)?;
            if right.lower <= 0.0 && right.upper >= 0.0 {
                return Err(Failure::ZeroDivisor);
            }
            corners(left, right, |a, b| a / b)
        }
        Node::Min(operands) | Node::Max(operands) => {
            let least = matches!(node, Node::Min(_));
            let mut value: Option<Interval> = None;
            for operand in operands {
                let next = evaluate(operand, inputs)?;
                let Some(current) = value.as_mut() else {
                    value = Some(next);
                    continue;
                };
                *current = if least {
                    Interval {
                        lower: current.lower.min(next.lower),
                        upper: current.upper.min(next.upper),
                    }
                } else {
                    Interval {
                        lower: current.lower.max(next.lower),
                        upper: current.upper.max(next.upper),
                    }
                };
            }
            // Binding requires two operands at least.
            value.ok_or(Failure::Overflow)?
        }
    })
}

/// The hull of `operation` over the four corners of two intervals: the
/// bounds of a product or of a quotient by an interval without zero.
fn corners(left: Interval, right: Interval, operation: impl Fn(f64, f64) -> f64) -> Interval {
    let values = [
        operation(left.lower, right.lower),
        operation(left.lower, right.upper),
        operation(left.upper, right.lower),
        operation(left.upper, right.upper),
    ];
    Interval {
        lower: values.iter().copied().fold(f64::INFINITY, f64::min),
        upper: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A number literal in coherent units, and its unit.
    Number(f64, Unit),
    Name(String),
    Plus,
    Minus,
    Times,
    Divide,
    Open,
    Close,
    Comma,
}

/// Splits `text` into tokens; a number literal takes the unit written right
/// after it (`42.5 EUR/m²`), up to the next space or operator.
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < characters.len() {
        let character = characters[at];
        let single = match character {
            '+' => Some(Token::Plus),
            '-' | '−' => Some(Token::Minus),
            '*' | '×' | '·' => Some(Token::Times),
            '/' | '÷' => Some(Token::Divide),
            '(' => Some(Token::Open),
            ')' => Some(Token::Close),
            ',' => Some(Token::Comma),
            _ => None,
        };
        if let Some(token) = single {
            tokens.push(token);
            at += 1;
        } else if character.is_whitespace() {
            at += 1;
        } else if character.is_ascii_digit() || character == '.' {
            let start = at;
            while at < characters.len()
                && (characters[at].is_ascii_digit()
                    || characters[at] == '.'
                    || ((characters[at] == 'e' || characters[at] == 'E')
                        && characters.get(at + 1).is_some_and(|next| {
                            next.is_ascii_digit() || *next == '-' || *next == '+'
                        }))
                    || ((characters[at] == '-' || characters[at] == '+')
                        && matches!(characters[at - 1], 'e' | 'E')))
            {
                at += 1;
            }
            let literal: String = characters[start..at].iter().collect();
            let value: f64 = literal
                .parse()
                .map_err(|_| format!("`{literal}` is no number"))?;
            while at < characters.len() && characters[at] == ' ' {
                at += 1;
            }
            let unit_start = at;
            if at < characters.len()
                && (characters[at].is_ascii_alphabetic() || characters[at] == '°')
            {
                while at < characters.len()
                    && unit_character(characters[at], characters.get(at.wrapping_sub(1)).copied())
                {
                    at += 1;
                }
            }
            let (scale, unit) = if at == unit_start {
                (1.0, Unit::NONE)
            } else {
                parse_unit(&characters[unit_start..at].iter().collect::<String>())?
            };
            let value = value * scale;
            if !value.is_finite() {
                return Err(format!("`{literal}` is not finite"));
            }
            tokens.push(Token::Number(value, unit));
        } else if character.is_ascii_alphabetic() || character == '_' {
            let start = at;
            while at < characters.len()
                && (characters[at].is_ascii_alphanumeric() || characters[at] == '_')
            {
                at += 1;
            }
            tokens.push(Token::Name(characters[start..at].iter().collect()));
        } else {
            return Err(format!("`{character}` is no part of an expression"));
        }
    }
    Ok(tokens)
}

/// Whether `character` continues a unit written after a number literal.
fn unit_character(character: char, previous: Option<char>) -> bool {
    character.is_ascii_alphanumeric()
        || "°²³¹⁰⁴⁵⁶⁷⁸⁹⁻·/^".contains(character)
        || (character == '-' && previous == Some('^'))
}

/// Parses and binds `text`: `resolve` gives each column name the expression
/// uses its input number and unit, or why it cannot be used. Units are
/// checked here: `+`, `-`, `min` and `max` need one unit, `×` and `÷`
/// combine them.
pub(super) fn bind(text: &str, resolve: &mut Resolve<'_>) -> Result<Expression, String> {
    if text.trim().is_empty() {
        return Err("the expression is empty".into());
    }
    if text.chars().count() > MAX_LENGTH {
        return Err(format!(
            "the expression is longer than {MAX_LENGTH} characters"
        ));
    }
    let tokens = tokens(text)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        resolve,
        depth: 0,
    };
    let (root, unit) = parser.sum()?;
    if let Some(token) = parser.tokens.get(parser.at) {
        return Err(format!("unexpected {}", shown(token)));
    }
    Ok(Expression { root, unit })
}

fn shown(token: &Token) -> String {
    match token {
        Token::Number(value, _) => format!("number `{value}`"),
        Token::Name(name) => format!("`{name}`"),
        Token::Plus => "`+`".into(),
        Token::Minus => "`-`".into(),
        Token::Times => "`×`".into(),
        Token::Divide => "`÷`".into(),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
    }
}

struct Parser<'t, 'r, 'f> {
    tokens: &'t [Token],
    at: usize,
    resolve: &'r mut Resolve<'f>,
    depth: usize,
}

impl Parser<'_, '_, '_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn deeper(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("the expression nests deeper than {MAX_DEPTH}"));
        }
        Ok(())
    }

    /// `term (('+' | '-') term)*`
    fn sum(&mut self) -> Result<(Node, Unit), String> {
        self.deeper()?;
        let (mut node, unit) = self.term()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Plus | Token::Minus))
            .cloned()
        {
            self.at += 1;
            let (right, right_unit) = self.term()?;
            if right_unit != unit {
                let verb = if operator == Token::Plus {
                    "adds"
                } else {
                    "subtracts"
                };
                return Err(format!("it {verb} {right_unit} and {unit}, which differ"));
            }
            node = if operator == Token::Plus {
                Node::Add(Box::new(node), Box::new(right))
            } else {
                Node::Subtract(Box::new(node), Box::new(right))
            };
        }
        self.depth -= 1;
        Ok((node, unit))
    }

    /// `factor (('×' | '÷') factor)*`
    fn term(&mut self) -> Result<(Node, Unit), String> {
        let (mut node, mut unit) = self.factor()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Times | Token::Divide))
            .cloned()
        {
            self.at += 1;
            let (right, right_unit) = self.factor()?;
            if operator == Token::Times {
                unit = unit.times(&right_unit, 1)?;
                node = Node::Multiply(Box::new(node), Box::new(right));
            } else {
                unit = unit.times(&right_unit, -1)?;
                node = Node::Divide(Box::new(node), Box::new(right));
            }
        }
        Ok((node, unit))
    }

    /// `'-' factor | number | name | ('min' | 'max') '(' sum (',' sum)+ ')' | '(' sum ')'`
    fn factor(&mut self) -> Result<(Node, Unit), String> {
        match self.next() {
            Some(Token::Minus) => {
                self.deeper()?;
                let (inner, unit) = self.factor()?;
                self.depth -= 1;
                Ok((Node::Negate(Box::new(inner)), unit))
            }
            Some(Token::Number(value, unit)) => Ok((Node::Literal(value), unit)),
            Some(Token::Open) => {
                let inner = self.sum()?;
                self.expect_close()?;
                Ok(inner)
            }
            Some(Token::Name(name))
                if (name == "min" || name == "max") && self.peek() == Some(&Token::Open) =>
            {
                self.at += 1;
                let (first, unit) = self.sum()?;
                let mut operands = vec![first];
                while self.peek() == Some(&Token::Comma) {
                    self.at += 1;
                    let (operand, operand_unit) = self.sum()?;
                    if operand_unit != unit {
                        return Err(format!(
                            "`{name}` compares {operand_unit} and {unit}, which differ"
                        ));
                    }
                    operands.push(operand);
                }
                self.expect_close()?;
                if operands.len() < 2 {
                    return Err(format!("`{name}` takes at least two values"));
                }
                Ok((
                    if name == "min" {
                        Node::Min(operands)
                    } else {
                        Node::Max(operands)
                    },
                    unit,
                ))
            }
            Some(Token::Name(name)) => {
                let (index, unit) = (self.resolve)(&name)?;
                Ok((Node::Input(index), unit))
            }
            Some(token) => Err(format!("unexpected {}", shown(&token))),
            None => Err("the expression ends early".into()),
        }
    }

    fn expect_close(&mut self) -> Result<(), String> {
        match self.next() {
            Some(Token::Close) => Ok(()),
            Some(token) => Err(format!("expected `)`, not {}", shown(&token))),
            None => Err("a `(` is not closed".into()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Failure, Interval, Unit, bind, parse_unit};
    use axioval_ir::QuantityDimension;

    fn columns(name: &str) -> Result<(usize, Unit), String> {
        match name {
            "area" => Ok((0, Unit::of(Some(QuantityDimension::Area)))),
            "length" => Ok((1, Unit::of(Some(QuantityDimension::Length)))),
            "count" => Ok((2, Unit::of(None))),
            other => Err(format!("`{other}` names no column")),
        }
    }

    fn bound(text: &str) -> Result<super::Expression, String> {
        bind(text, &mut columns)
    }

    fn point(value: f64) -> Interval {
        Interval {
            lower: value,
            upper: value,
        }
    }

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

    #[test]
    fn a_unit_cost_times_an_area_is_an_amount() {
        let expression = bound("area × 42.5 EUR/m²").unwrap();
        assert_eq!(expression.unit().to_string(), "EUR");
        assert_eq!(
            expression.evaluate(&[point(10.0), point(0.0), point(0.0)]),
            Ok(point(425.0))
        );
        let expression = bound("area * 425 EUR / (10 m2)").unwrap();
        assert_eq!(expression.unit().to_string(), "EUR");
    }

    #[test]
    fn mixing_dimensions_is_refused_when_bound() {
        let error = bound("area + length").unwrap_err();
        assert!(error.contains("adds m and m²"), "{error}");
        assert!(bound("min(area, 2 m)").unwrap_err().contains("differ"));
        assert!(bound("area - 1").unwrap_err().contains("differ"));
        assert!(bound("area + 2 m2 - length × length").is_ok());
        assert!(bound("width").unwrap_err().contains("names no column"));
        for malformed in [
            "",
            "area +",
            "(area",
            "area)",
            "min(area)",
            "area $ 2",
            "2 area",
        ] {
            assert!(bound(malformed).is_err(), "{malformed}");
        }
        let deep = format!("{}area{}", "(".repeat(40), ")".repeat(40));
        assert!(bound(&deep).unwrap_err().contains("nests"));
    }

    #[test]
    fn intervals_bound_every_value_and_a_divisor_around_zero_has_none() {
        let expression = bound("(area - 2 m²) × length ÷ count").unwrap();
        let value = expression
            .evaluate(&[
                Interval {
                    lower: 9.0,
                    upper: 10.0,
                },
                Interval {
                    lower: -1.0,
                    upper: 2.0,
                },
                point(2.0),
            ])
            .unwrap();
        // (7..8) × (-1..2) = -8..16, halved.
        assert_eq!(
            value,
            Interval {
                lower: -4.0,
                upper: 8.0
            }
        );
        let ratio = bound("area / length").unwrap();
        assert_eq!(
            ratio.evaluate(&[
                point(1.0),
                Interval {
                    lower: -1.0,
                    upper: 1.0
                },
                point(0.0)
            ]),
            Err(Failure::ZeroDivisor)
        );
        let least = bound("max(-length, 1 m, min(length, 3 m))").unwrap();
        assert_eq!(
            least.evaluate(&[
                point(0.0),
                Interval {
                    lower: 0.5,
                    upper: 4.0
                },
                point(0.0)
            ]),
            Ok(Interval {
                lower: 1.0,
                upper: 3.0
            })
        );
    }
}
