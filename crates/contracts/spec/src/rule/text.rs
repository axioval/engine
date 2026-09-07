//! Localized display text.
//!
//! Some sources store rule/folder labels as `LocalizedString` (a `lang -> text`
//! map); IDS uses plain strings; `OpenBimRL` uses identifiers. The canonical IR
//! keeps a small locale map and lets each backend pick the locale it needs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A piece of human-facing text with optional per-locale variants.
///
/// The empty map is a valid (unlabeled) value. Locale keys are lowercase
/// ISO-639 codes (`"en"`, `"de"`). [`LocalizedText::get`] resolves with a
/// deterministic fallback chain so backends never have to special-case a
/// missing locale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalizedText {
    /// locale code -> text. `BTreeMap` keeps serialization deterministic.
    by_locale: BTreeMap<String, String>,
}

impl LocalizedText {
    /// An empty (unlabeled) value.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from a single `en` string — the common authoring case.
    pub fn en(text: impl Into<String>) -> Self {
        let mut by_locale = BTreeMap::new();
        by_locale.insert("en".to_string(), text.into());
        Self { by_locale }
    }

    /// Insert/replace one locale variant.
    pub fn set(&mut self, locale: impl Into<String>, text: impl Into<String>) -> &mut Self {
        self.by_locale
            .insert(locale.into().to_lowercase(), text.into());
        self
    }

    /// True when no locale variant is present.
    pub fn is_empty(&self) -> bool {
        self.by_locale.is_empty()
    }

    /// Exact lookup for one locale (no fallback).
    pub fn get_exact(&self, locale: &str) -> Option<&str> {
        self.by_locale
            .get(&locale.to_lowercase())
            .map(String::as_str)
    }

    /// Resolve text for a preferred locale with a deterministic fallback:
    /// exact match -> `en` -> first entry (`BTreeMap` order) -> empty string.
    pub fn get(&self, preferred: &str) -> &str {
        if let Some(t) = self.get_exact(preferred) {
            return t;
        }
        if let Some(t) = self.by_locale.get("en") {
            return t;
        }
        self.by_locale.values().next().map_or("", String::as_str)
    }

    /// Iterate `(locale, text)` pairs in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.by_locale.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl From<&str> for LocalizedText {
    fn from(s: &str) -> Self {
        LocalizedText::en(s)
    }
}

impl From<String> for LocalizedText {
    fn from(s: String) -> Self {
        LocalizedText::en(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_chain() {
        let mut t = LocalizedText::en("Wall");
        t.set("de", "Wand");
        assert_eq!(t.get("de"), "Wand");
        assert_eq!(t.get("en"), "Wall");
        // missing locale falls back to en
        assert_eq!(t.get("fr"), "Wall");
    }

    #[test]
    fn empty_is_empty_string() {
        let t = LocalizedText::empty();
        assert!(t.is_empty());
        assert_eq!(t.get("en"), "");
    }

    #[test]
    fn transparent_serde_roundtrip() {
        let mut t = LocalizedText::en("Space");
        t.set("de", "Raum");
        let json = serde_json::to_string(&t).unwrap();
        // transparent => serializes as a plain object, not wrapped
        assert_eq!(json, r#"{"de":"Raum","en":"Space"}"#);
        let back: LocalizedText = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }
}
