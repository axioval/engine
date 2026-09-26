//! XML Schema regular expressions, compiled to `regex`.
//!
//! An XML Schema pattern matches the whole value, has no anchors (`^` and
//! `$` are ordinary characters) and defines `.`, `\s` and `\w` differently
//! from `regex`. The translation keeps XML Schema meaning exactly; a
//! construct it cannot map exactly (character-class subtraction, `\i`/`\c`
//! name escapes, `\p{Is…}` block escapes) is refused rather than
//! approximated.

use regex::Regex;

/// XML Schema `\s`: space, tab, line feed and carriage return only.
const SPACE: &str = r"\x20\t\n\r";
const SPACE_CLASS: &str = r"[\x20\t\n\r]";
const NOT_SPACE_CLASS: &str = r"[^\x20\t\n\r]";
/// XML Schema `\w`: anything but punctuation, separators and "other".
const NOT_WORD: &str = r"\p{P}\p{Z}\p{C}";
const WORD_CLASS: &str = r"[^\p{P}\p{Z}\p{C}]";
const NOT_WORD_CLASS: &str = r"[\p{P}\p{Z}\p{C}]";

/// Compiles an XML Schema pattern into a whole-value matcher.
///
/// # Errors
///
/// Returns a description of the construct that cannot be translated exactly,
/// or of the syntax error.
pub(crate) fn compile(pattern: &str) -> Result<Regex, String> {
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    let mut previous_class_char: Option<char> = None;
    while let Some(c) = chars.next() {
        if in_class {
            match c {
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '\\' => {
                    let escaped = chars.next().ok_or("pattern ends with a backslash")?;
                    match escaped {
                        's' => out.push_str(SPACE),
                        'S' => out.push_str(NOT_SPACE_CLASS),
                        'w' => out.push_str(WORD_CLASS),
                        'W' => out.push_str(NOT_WORD),
                        other => push_escape(&mut out, other, &mut chars)?,
                    }
                }
                '-' if chars.peek() == Some(&'[') => {
                    return Err("character-class subtraction is not supported".into());
                }
                '-' if previous_class_char == Some('-') => {
                    return Err("`--` in a character class is not XML Schema syntax".into());
                }
                '[' => return Err("an unescaped `[` inside a character class".into()),
                // Set operators in `regex` classes; literals in XML Schema.
                '&' | '~' => {
                    out.push('\\');
                    out.push(c);
                }
                other => out.push(other),
            }
            previous_class_char = Some(c);
            continue;
        }
        match c {
            '.' => out.push_str(r"[^\n\r]"),
            '^' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '[' => {
                in_class = true;
                previous_class_char = None;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    chars.next();
                    out.push('^');
                }
            }
            '(' if chars.peek() == Some(&'?') => {
                return Err("`(?` is not XML Schema syntax".into());
            }
            '\\' => {
                let escaped = chars.next().ok_or("pattern ends with a backslash")?;
                match escaped {
                    's' => out.push_str(SPACE_CLASS),
                    'S' => out.push_str(NOT_SPACE_CLASS),
                    'w' => out.push_str(WORD_CLASS),
                    'W' => out.push_str(NOT_WORD_CLASS),
                    other => push_escape(&mut out, other, &mut chars)?,
                }
            }
            other => out.push(other),
        }
    }
    if in_class {
        return Err("unterminated character class".into());
    }
    Regex::new(&format!(r"\A(?:{out})\z")).map_err(|error| error.to_string())
}

/// Escapes that mean the same in both dialects, and `\p{…}` categories.
fn push_escape(
    out: &mut String,
    escaped: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<(), String> {
    match escaped {
        'n' | 'r' | 't' | 'd' | 'D' | '\\' | '|' | '.' | '-' | '^' | '?' | '*' | '+' | '{'
        | '}' | '(' | ')' | '[' | ']' => {
            out.push('\\');
            out.push(escaped);
            Ok(())
        }
        'p' | 'P' => {
            if chars.next() != Some('{') {
                return Err(format!("`\\{escaped}` needs a braced name"));
            }
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(c) => name.push(c),
                    None => return Err("unterminated `\\p{`".into()),
                }
            }
            if name.starts_with("Is") {
                return Err(format!("block escape `\\p{{{name}}}` is not supported"));
            }
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(format!("`{name}` is not a Unicode category"));
            }
            out.push('\\');
            out.push(escaped);
            out.push('{');
            out.push_str(&name);
            out.push('}');
            Ok(())
        }
        'i' | 'I' | 'c' | 'C' => Err(format!("name escape `\\{escaped}` is not supported")),
        other => Err(format!("`\\{other}` is not an XML Schema escape")),
    }
}

#[cfg(test)]
mod tests {
    use super::compile;

    fn matches(pattern: &str, value: &str) -> bool {
        compile(pattern)
            .unwrap_or_else(|error| panic!("{pattern}: {error}"))
            .is_match(value)
    }

    #[test]
    fn patterns_match_the_whole_value() {
        assert!(matches("IFC.*", "IFCWALL"));
        assert!(!matches("WALL", "IFCWALL"));
        assert!(!matches("IFC", "IFCWALL"));
        assert!(matches("", ""));
        assert!(matches("a|b", "b"));
        assert!(!matches("a|b", "ab"));
    }

    #[test]
    fn anchors_are_ordinary_characters() {
        assert!(matches("^a$", "^a$"));
        assert!(!matches("^a$", "a"));
    }

    #[test]
    fn dot_space_and_word_keep_xml_schema_meaning() {
        assert!(!matches(".", "\r"));
        assert!(!matches(".", "\n"));
        assert!(matches(".", "é"));
        // U+00A0 is Unicode whitespace but not XML Schema `\s`.
        assert!(!matches(r"\s", "\u{a0}"));
        assert!(matches(r"\S", "\u{a0}"));
        assert!(matches(r"\s", "\t"));
        // `_` is punctuation, so not an XML Schema word character.
        assert!(!matches(r"\w", "_"));
        assert!(matches(r"\W", "_"));
        assert!(matches(r"[\s_]+", " _\t"));
        assert!(matches(r"[^\s]+", "ab"));
        assert!(!matches(r"[\w]", "-"));
    }

    #[test]
    fn classes_ranges_categories_and_quantifiers() {
        assert!(matches("[A-Z]{2}[0-9]{3}", "EI090"));
        assert!(!matches("[A-Z]{2}[0-9]{3}", "EI90"));
        assert!(matches(r"\p{Lu}+", "ÄB"));
        assert!(matches(r"\d+\.\d+", "4.2"));
        assert!(matches("[a&~]+", "&~a"));
        assert!(matches(r"\^$", "^$"));
    }

    #[test]
    fn untranslatable_constructs_are_refused() {
        for pattern in [
            "[a-z-[aeiou]]",
            r"\i\c*",
            r"\p{IsBasicLatin}",
            "(?i)a",
            "[a--b]",
            "[a[b]]",
            "[ab",
            r"\q",
            "a\\",
            r"\$",
        ] {
            assert!(compile(pattern).is_err(), "{pattern}");
        }
    }
}
