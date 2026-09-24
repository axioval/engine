#![allow(missing_docs)]
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedText {
    pub default: String,
    #[serde(default)]
    pub translations: BTreeMap<String, String>,
}

impl LocalizedText {
    /// Text with no translations.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            default: text.into(),
            translations: BTreeMap::new(),
        }
    }
}

/// Package identity and presentation metadata.
///
/// `name` and `description` are localized in the normalized MCS contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub id: String,
    pub name: LocalizedText,
    pub version: String,
    pub description: Option<LocalizedText>,
    pub repository: Option<String>,
    pub license: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}

/// One external-schema name for a canonical concept.
///
/// `type_system` is the release-bound semantic identity the source adapter
/// declares, for example `https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4`.
/// Binding a concept to source data requires an exact match on it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalName {
    #[serde(rename = "typeSystem")]
    pub type_system: String,
    pub name: String,
}

/// A bibliographic source a package may cite. Provenance only: it never changes
/// selection, evidence, or outcomes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub designation: String,
    pub title: LocalizedText,
    pub publisher: Option<String>,
    pub edition: Option<String>,
    pub publication_date: Option<String>,
    pub url: Option<String>,
}

/// One pinpoint within a cited source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    pub kind: String,
    pub value: String,
}

/// A reference to one package source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Citation {
    pub id: String,
    pub source_id: String,
    #[serde(default)]
    pub locators: Vec<Locator>,
    pub note: Option<LocalizedText>,
}
