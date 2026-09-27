//! Vendor-neutral selection of members in a federated model set.
//!
//! The model-level rule plans that used to live here (required components,
//! spaces in derived groups, space-group containment, fire-compartment area,
//! storey-name sequence) were retired in favour of registered capabilities;
//! see the "Retired rule plans" section of the capability migration page.

use serde::{Deserialize, Serialize};

use crate::rule::execution::ModelDomain;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ModelSelectionSpec {
    Primary,
    All,
    MemberIds {
        ids: Vec<String>,
    },
    Domains {
        domains: Vec<ModelDomain>,
        include_unclassified: bool,
    },
}

impl ModelSelectionSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::MemberIds { ids } => {
                if ids.is_empty() || ids.iter().any(|id| id.trim().is_empty()) {
                    return Err("model member-id selection is empty or contains a blank id".into());
                }
                let unique: std::collections::BTreeSet<_> = ids.iter().collect();
                if unique.len() != ids.len() {
                    return Err("model member-id selection contains duplicates".into());
                }
            }
            Self::Domains { domains, .. } => {
                if domains.is_empty() {
                    return Err("model domain selection is empty".into());
                }
                if domains.contains(&ModelDomain::Any) && domains.len() != 1 {
                    return Err("model domain `any` cannot be combined with named domains".into());
                }
                let unique: std::collections::BTreeSet<_> =
                    domains.iter().map(|d| d.id()).collect();
                if unique.len() != domains.len() {
                    return Err("model domain selection contains duplicates".into());
                }
            }
            Self::Primary | Self::All => {}
        }
        Ok(())
    }
}
