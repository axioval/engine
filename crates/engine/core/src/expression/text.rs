//! The canonical text form of an expression, as people write it in a
//! takeoff column or a rule editor: `area × 42.5 EUR/m²`,
//! `if(class == "office", area * 2, area)`.
//!
//! ```text
//! expression := or
//! or         := and ("or" and)*
//! and        := not ("and" not)*
//! not        := "not" not | comparison
//! comparison := sum (("==" | "!=" | "<" | "<=" | ">" | ">=") sum)?
//! sum        := term (("+" | "-" | "−") term)*
//! term       := factor (("*" | "×" | "·" | "/" | "÷") factor)*
//! factor     := ("-" | "−") factor
//!             | number [unit] | "text" | true | false | null
//!             | function "(" expression ("," expression)* ")"
//!             | name | "(" expression ")"
//! ```
//!
//! A number takes the unit written right after it (`42.5 EUR/m²`, `10 m2`).
//! A name (`area`, `profile_width`) reads the column or parameter of that
//! name. The functions are `min`, `max` (two values or more), `abs`,
//! `sqrt`, `floor`, `ceil`, `round(value, step)`, `if(condition, then,
//! else)` and `coalesce(...)`. Text parses into the expression tree; it
//! never runs as code.

use axioval_ir::contract::{
    Branch, Expression, ExpressionComparison, MAX_EXPRESSION_DEPTH, ScalarValue,
};

use super::unit::parse_unit;

/// The longest text accepted, in characters.
pub const MAX_TEXT_LENGTH: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A number literal and the unit written after it, as written.
    Number(f64, Option<String>),
    Text(String),
    Name(String),
    Plus,
    Minus,
    Times,
    Divide,
    Open,
    Close,
    Comma,
    Compare(ExpressionComparison),
}

/// Whether `character` continues a unit written after a number literal.
fn unit_character(character: char, previous: Option<char>) -> bool {
    character.is_ascii_alphanumeric()
        || "°²³¹⁰⁴⁵⁶⁷⁸⁹⁻·/^".contains(character)
        || (character == '-' && previous == Some('^'))
}

/// Splits `text` into tokens.
#[allow(clippy::too_many_lines)]
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < characters.len() {
        let character = characters[at];
        let next = characters.get(at + 1).copied();
        let two = |token: Token| (token, 2);
        let one = |token: Token| (token, 1);
        let single = match (character, next) {
            ('=', Some('=')) => Some(two(Token::Compare(ExpressionComparison::Equals))),
            ('!', Some('=')) => Some(two(Token::Compare(ExpressionComparison::NotEquals))),
            ('<', Some('=')) => Some(two(Token::Compare(ExpressionComparison::LessThanOrEquals))),
            ('>', Some('=')) => Some(two(Token::Compare(
                ExpressionComparison::GreaterThanOrEquals,
            ))),
            ('<', _) => Some(one(Token::Compare(ExpressionComparison::LessThan))),
            ('>', _) => Some(one(Token::Compare(ExpressionComparison::GreaterThan))),
            ('≤', _) => Some(one(Token::Compare(ExpressionComparison::LessThanOrEquals))),
            ('≥', _) => Some(one(Token::Compare(
                ExpressionComparison::GreaterThanOrEquals,
            ))),
            ('≠', _) => Some(one(Token::Compare(ExpressionComparison::NotEquals))),
            ('+', _) => Some(one(Token::Plus)),
            ('-' | '−', _) => Some(one(Token::Minus)),
            ('*' | '×' | '·', _) => Some(one(Token::Times)),
            ('/' | '÷', _) => Some(one(Token::Divide)),
            ('(', _) => Some(one(Token::Open)),
            (')', _) => Some(one(Token::Close)),
            (',', _) => Some(one(Token::Comma)),
            _ => None,
        };
        if let Some((token, width)) = single {
            tokens.push(token);
            at += width;
        } else if character.is_whitespace() {
            at += 1;
        } else if character == '"' {
            let start = at + 1;
            let end = characters[start..]
                .iter()
                .position(|character| *character == '"')
                .ok_or("a text literal is not closed")?;
            tokens.push(Token::Text(characters[start..start + end].iter().collect()));
            at = start + end + 1;
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
            if !value.is_finite() {
                return Err(format!("`{literal}` is not finite"));
            }
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
            let unit = if at == unit_start {
                None
            } else {
                let unit: String = characters[unit_start..at].iter().collect();
                if parse_unit(&unit).is_ok() {
                    Some(unit)
                } else {
                    // Not a unit (a keyword such as `or`, or a misspelt
                    // unit): read again as what follows the number.
                    at = unit_start;
                    None
                }
            };
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

fn shown(token: &Token) -> String {
    match token {
        Token::Number(value, _) => format!("number `{value}`"),
        Token::Text(text) => format!("text `\"{text}\"`"),
        Token::Name(name) => format!("`{name}`"),
        Token::Plus => "`+`".into(),
        Token::Minus => "`-`".into(),
        Token::Times => "`×`".into(),
        Token::Divide => "`÷`".into(),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
        Token::Compare(_) => "a comparison".into(),
    }
}

/// Parses `text` into an expression tree.
///
/// # Errors
///
/// Why the text is no expression: empty, too long, nested too deeply, a
/// token out of place, a misspelt unit or an unknown function.
pub fn parse_text(text: &str) -> Result<Expression, String> {
    if text.trim().is_empty() {
        return Err("the expression is empty".into());
    }
    if text.chars().count() > MAX_TEXT_LENGTH {
        return Err(format!(
            "the expression is longer than {MAX_TEXT_LENGTH} characters"
        ));
    }
    let tokens = tokens(text)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        depth: 0,
    };
    let expression = parser.or()?;
    if let Some(token) = parser.tokens.get(parser.at) {
        return Err(format!("unexpected {}", shown(token)));
    }
    Ok(expression)
}

struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
    depth: usize,
}

fn boxed(expression: Expression) -> Box<Expression> {
    Box::new(expression)
}

impl Parser<'_> {
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
        if self.depth > MAX_EXPRESSION_DEPTH {
            return Err(format!(
                "the expression nests deeper than {MAX_EXPRESSION_DEPTH}"
            ));
        }
        Ok(())
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Name(name)) if name == word)
    }

    fn or(&mut self) -> Result<Expression, String> {
        self.deeper()?;
        let mut operands = vec![self.and()?];
        while self.keyword("or") {
            self.at += 1;
            operands.push(self.and()?);
        }
        self.depth -= 1;
        Ok(if operands.len() == 1 {
            operands.remove(0)
        } else {
            Expression::Or {
                operands,
                label: None,
            }
        })
    }

    fn and(&mut self) -> Result<Expression, String> {
        let mut operands = vec![self.not()?];
        while self.keyword("and") {
            self.at += 1;
            operands.push(self.not()?);
        }
        Ok(if operands.len() == 1 {
            operands.remove(0)
        } else {
            Expression::And {
                operands,
                label: None,
            }
        })
    }

    fn not(&mut self) -> Result<Expression, String> {
        if self.keyword("not") {
            self.at += 1;
            self.deeper()?;
            let operand = self.not()?;
            self.depth -= 1;
            return Ok(Expression::Not {
                operand: boxed(operand),
                label: None,
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expression, String> {
        let left = self.sum()?;
        let Some(Token::Compare(operator)) = self.peek().cloned() else {
            return Ok(left);
        };
        self.at += 1;
        let right = self.sum()?;
        Ok(Expression::Compare {
            operator,
            left: boxed(left),
            right: boxed(right),
            case_sensitive: true,
            label: None,
        })
    }

    /// `term (('+' | '-') term)*`
    fn sum(&mut self) -> Result<Expression, String> {
        let mut node = self.term()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Plus | Token::Minus))
            .cloned()
        {
            self.at += 1;
            let right = self.term()?;
            node = if operator == Token::Plus {
                Expression::Add {
                    left: boxed(node),
                    right: boxed(right),
                    label: None,
                }
            } else {
                Expression::Subtract {
                    left: boxed(node),
                    right: boxed(right),
                    label: None,
                }
            };
        }
        Ok(node)
    }

    /// `factor (('×' | '÷') factor)*`
    fn term(&mut self) -> Result<Expression, String> {
        let mut node = self.factor()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Times | Token::Divide))
            .cloned()
        {
            self.at += 1;
            let right = self.factor()?;
            node = if operator == Token::Times {
                Expression::Multiply {
                    left: boxed(node),
                    right: boxed(right),
                    label: None,
                }
            } else {
                Expression::Divide {
                    left: boxed(node),
                    right: boxed(right),
                    label: None,
                }
            };
        }
        Ok(node)
    }

    fn factor(&mut self) -> Result<Expression, String> {
        let literal = |value: ScalarValue| Expression::Literal { value, label: None };
        match self.next() {
            Some(Token::Minus) => {
                self.deeper()?;
                let inner = self.factor()?;
                self.depth -= 1;
                Ok(Expression::Negate {
                    operand: boxed(inner),
                    label: None,
                })
            }
            Some(Token::Number(value, unit)) => Ok(literal(match unit {
                Some(unit) => ScalarValue::Quantity { value, unit },
                None => ScalarValue::Number { value },
            })),
            Some(Token::Text(text)) => Ok(literal(ScalarValue::String { value: text })),
            Some(Token::Open) => {
                let inner = self.or()?;
                self.expect_close()?;
                Ok(inner)
            }
            Some(Token::Name(name)) if self.peek() == Some(&Token::Open) => {
                self.at += 1;
                self.call(&name)
            }
            Some(Token::Name(name)) => Ok(match name.as_str() {
                "true" => literal(ScalarValue::Boolean { value: true }),
                "false" => literal(ScalarValue::Boolean { value: false }),
                "null" => Expression::Null { label: None },
                _ => Expression::Parameter { name, label: None },
            }),
            Some(token) => Err(format!("unexpected {}", shown(&token))),
            None => Err("the expression ends early".into()),
        }
    }

    /// The arguments of a function call, its `(` read.
    fn arguments(&mut self) -> Result<Vec<Expression>, String> {
        let mut arguments = vec![self.or()?];
        while self.peek() == Some(&Token::Comma) {
            self.at += 1;
            arguments.push(self.or()?);
        }
        self.expect_close()?;
        Ok(arguments)
    }

    fn call(&mut self, name: &str) -> Result<Expression, String> {
        let mut arguments = self.arguments()?;
        let count = |wanted: usize| {
            if arguments.len() == wanted {
                Ok(())
            } else {
                Err(format!("`{name}` takes {wanted} value(s)"))
            }
        };
        let label = None;
        Ok(match name {
            "min" | "max" => {
                if arguments.len() < 2 {
                    return Err(format!("`{name}` takes at least two values"));
                }
                if name == "min" {
                    Expression::Min {
                        operands: arguments,
                        label,
                    }
                } else {
                    Expression::Max {
                        operands: arguments,
                        label,
                    }
                }
            }
            "coalesce" => Expression::Coalesce {
                operands: arguments,
                label,
            },
            "abs" | "sqrt" | "floor" | "ceil" => {
                count(1)?;
                let operand = boxed(arguments.remove(0));
                match name {
                    "abs" => Expression::Abs { operand, label },
                    "sqrt" => Expression::Sqrt { operand, label },
                    "floor" => Expression::Floor { operand, label },
                    _ => Expression::Ceil { operand, label },
                }
            }
            "round" => {
                count(2)?;
                let step = boxed(arguments.remove(1));
                Expression::Round {
                    operand: boxed(arguments.remove(0)),
                    step,
                    label,
                }
            }
            "if" => {
                count(3)?;
                let otherwise = boxed(arguments.remove(2));
                let then = arguments.remove(1);
                Expression::If {
                    branches: vec![Branch {
                        when: arguments.remove(0),
                        then,
                    }],
                    otherwise,
                    label,
                }
            }
            other => return Err(format!("`{other}` is no function")),
        })
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
mod tests {
    use super::parse_text;

    #[test]
    fn the_takeoff_grammar_parses_into_the_tree() {
        let parsed = parse_text("area × 42.5 EUR/m²").unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::json!({"kind": "multiply",
                "left": {"kind": "parameter", "name": "area"},
                "right": {"kind": "literal", "value": {"type": "quantity", "value": 42.5, "unit": "EUR/m²"}}})
        );
        for text in [
            "(area - 2 m²) × length ÷ count",
            "max(-length, 1 m, min(length, 3 m))",
        ] {
            assert!(parse_text(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn conditions_logic_and_text_parse() {
        let parsed = parse_text("if(class == \"office\" and not open, area * 2, area)").unwrap();
        assert_eq!(parsed.kind(), "if");
        assert!(parse_text("a <= 3 or b ≥ 2 m").is_ok());
    }

    #[test]
    fn malformed_text_is_refused_with_a_reason() {
        for (text, reason) in [
            ("", "empty"),
            ("area +", "ends early"),
            ("(area", "not closed"),
            ("area)", "unexpected `)`"),
            ("min(area)", "at least two"),
            ("area $ 2", "no part of an expression"),
            ("2 parsec", "parsec"),
            ("frob(1)", "no function"),
            ("\"open", "not closed"),
        ] {
            let error = parse_text(text).unwrap_err();
            assert!(error.contains(reason), "{text}: {error}");
        }
        let deep = format!("{}area{}", "(".repeat(80), ")".repeat(80));
        assert!(parse_text(&deep).unwrap_err().contains("nests"));
    }
}
