//! Disciplines assigned from source metadata.
//!
//! A federated check commonly takes each model's discipline from the
//! application that wrote it or its file name. A [`DisciplineMap`] is the
//! host's ordered list of such assignments; applied to a session, it gives a
//! discipline to each source that declares none, and records which entry
//! did so and on which value, so a report can say where a discipline came
//! from. A declared discipline always wins, and a source no entry matches
//! keeps none, so discipline rules over it stay not evaluated.

use std::fmt;

use axioval_ir::contract::SourceField;
use axioval_ir::{Discipline, SourceId};
use regex::Regex;
use thiserror::Error;

use crate::SourceMetadataIndex;

/// A wildcard pattern as an anchored regular expression.
///
/// `*` is any run of characters (none included), `?` exactly one, and a
/// backslash makes the next character literal (`\*`, `\?`, `\\`). Every
/// other character is literal. This is the one translation of the `like`
/// operator's wildcards.
///
/// # Errors
///
/// A pattern ending in a lone backslash.
pub fn wildcard_regex(pattern: &str) -> Result<String, String> {
    let mut out = String::from("(?s)^");
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or("a wildcard pattern ends with a backslash")?;
                out.push_str(&regex::escape(escaped.encode_utf8(&mut [0; 4])));
            }
            other => out.push_str(&regex::escape(other.encode_utf8(&mut [0; 4]))),
        }
    }
    out.push('$');
    Ok(out)
}

/// One assignment: sources whose `field` has a value matching `pattern`
/// play `discipline`.
#[derive(Clone, Debug)]
pub struct DisciplineRule {
    field: SourceField,
    pattern: String,
    regex: Regex,
    discipline: Discipline,
}

impl DisciplineRule {
    /// An assignment by a wildcard pattern over the whole value, compared
    /// case-sensitively, as the `like` operator compares.
    ///
    /// # Errors
    ///
    /// A pattern that is empty or ends in a lone backslash.
    pub fn new(
        field: SourceField,
        pattern: impl Into<String>,
        discipline: Discipline,
    ) -> Result<Self, DisciplineMapError> {
        let pattern = pattern.into();
        if pattern.is_empty() {
            return Err(DisciplineMapError::InvalidPattern(
                pattern,
                "the pattern is empty".into(),
            ));
        }
        let regex = wildcard_regex(&pattern)
            .and_then(|translated| Regex::new(&translated).map_err(|error| error.to_string()))
            .map_err(|message| DisciplineMapError::InvalidPattern(pattern.clone(), message))?;
        Ok(Self {
            field,
            pattern,
            regex,
            discipline,
        })
    }

    /// The metadata field the pattern reads.
    #[must_use]
    pub fn field(&self) -> SourceField {
        self.field
    }

    /// The wildcard pattern as written.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The discipline a matching source plays.
    #[must_use]
    pub fn discipline(&self) -> &Discipline {
        &self.discipline
    }
}

impl fmt::Display for DisciplineRule {
    /// `field:pattern=discipline`, as the CLI takes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}={}",
            self.field.as_str(),
            self.pattern,
            self.discipline
        )
    }
}

/// A host's ordered disciplines by source metadata.
#[derive(Clone, Debug, Default)]
pub struct DisciplineMap {
    rules: Vec<DisciplineRule>,
}

impl DisciplineMap {
    /// A map assigning nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `rule`; earlier rules are tried first.
    #[must_use]
    pub fn with(mut self, rule: DisciplineRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// The rules, in the order they are tried.
    #[must_use]
    pub fn rules(&self) -> &[DisciplineRule] {
        &self.rules
    }

    /// Whether the map assigns nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// What the map decides for `source`: the first rule one of whose field
    /// values matches assigns its discipline. A rule whose field was never
    /// read cannot be passed over, since it might match, so it stops the
    /// search and the source is left without a discipline.
    pub(crate) fn decide(&self, source: &SourceId, metadata: &SourceMetadataIndex) -> Mapping {
        for (index, rule) in self.rules.iter().enumerate() {
            let Some(values) = metadata.values(source, rule.field) else {
                return Mapping::Unread { rule: index };
            };
            if let Some(value) = values.iter().find(|value| rule.regex.is_match(value)) {
                return Mapping::Assigned {
                    rule: index,
                    value: value.clone(),
                };
            }
        }
        Mapping::Unmatched
    }
}

/// What a [`DisciplineMap`] decided for one source, by rule index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Mapping {
    Assigned { rule: usize, value: String },
    Unread { rule: usize },
    Unmatched,
}

/// Where a source's discipline came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisciplineOrigin {
    /// The host declared it for the source.
    Declared,
    /// A discipline map assigned it.
    Mapped {
        /// The assigning rule, `field:pattern=discipline`.
        rule: String,
        /// The field it read.
        field: SourceField,
        /// The value its pattern matched.
        value: String,
    },
}

impl DisciplineOrigin {
    /// A reviewable locator for evidence citing the assignment:
    /// `discipline-map:field:pattern=discipline@value`.
    #[must_use]
    pub fn locator(&self) -> Option<String> {
        match self {
            Self::Declared => None,
            Self::Mapped { rule, value, .. } => Some(format!("discipline-map:{rule}@{value}")),
        }
    }
}

/// Why a source kept no discipline under a map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnmappedReason {
    /// No rule matched any value.
    NoMatch,
    /// The rule (`field:pattern=discipline`) reads a field the source never
    /// stated, so neither it nor a later rule could decide.
    Unread(String),
}

/// An invalid discipline map.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DisciplineMapError {
    /// A pattern cannot be matched.
    #[error("discipline map pattern `{0}`: {1}")]
    InvalidPattern(String, String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceMetadata;

    fn source(document: &str) -> SourceId {
        SourceId::new("test", document).unwrap()
    }

    fn rule(field: SourceField, pattern: &str, discipline: &str) -> DisciplineRule {
        DisciplineRule::new(field, pattern, Discipline::new(discipline).unwrap()).unwrap()
    }

    #[test]
    fn the_first_matching_rule_assigns_and_an_unread_field_stops_the_search() {
        let map = DisciplineMap::new()
            .with(rule(
                SourceField::Application,
                "*Architecture*",
                "architecture",
            ))
            .with(rule(SourceField::FileName, "*_STR*", "structure"));
        let index = SourceMetadataIndex::new([
            (
                source("a"),
                SourceMetadata::new()
                    .with(SourceField::Application, ["Tool Architecture"])
                    .with(SourceField::FileName, ["a_STR.ifc"]),
            ),
            (
                source("b"),
                SourceMetadata::new()
                    .with(SourceField::Application, ["Other"])
                    .with(SourceField::FileName, ["b_STR.ifc"]),
            ),
            (
                source("c"),
                SourceMetadata::new().with(SourceField::FileName, ["c_STR.ifc"]),
            ),
            (
                source("d"),
                SourceMetadata::new()
                    .with(SourceField::Application, ["Other"])
                    .with(SourceField::FileName, ["d.ifc"]),
            ),
        ]);
        assert_eq!(
            map.decide(&source("a"), &index),
            Mapping::Assigned {
                rule: 0,
                value: "Tool Architecture".into()
            }
        );
        assert_eq!(
            map.decide(&source("b"), &index),
            Mapping::Assigned {
                rule: 1,
                value: "b_STR.ifc".into()
            }
        );
        assert_eq!(
            map.decide(&source("c"), &index),
            Mapping::Unread { rule: 0 }
        );
        assert_eq!(map.decide(&source("d"), &index), Mapping::Unmatched);
    }

    #[test]
    fn patterns_are_wildcards_over_the_whole_value() {
        assert_eq!(wildcard_regex(r"W?-*.1").unwrap(), r"(?s)^W.\-.*\.1$");
        assert!(wildcard_regex("a\\").is_err());
        assert!(matches!(
            DisciplineRule::new(SourceField::Application, "", Discipline::new("x").unwrap()),
            Err(DisciplineMapError::InvalidPattern(..))
        ));
        assert_eq!(
            rule(SourceField::FileName, "*.ifc", "mep").to_string(),
            "fileName:*.ifc=mep"
        );
    }
}
