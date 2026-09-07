//! Backend target identifiers and per-target support declarations.

use serde::{Deserialize, Serialize};

/// The output formats a rule definition can be compiled to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Vendor `.cset` (Java-serialized constraint-model graph).
    Cset,
    /// buildingSMART IDS XML.
    Ids,
    /// `OpenBimRL` rule specification.
    OpenBimRl,
    /// The in-process `sol-rules` runtime checker.
    Runtime,
}

impl Target {
    pub const ALL: [Target; 4] = [
        Target::Cset,
        Target::Ids,
        Target::OpenBimRl,
        Target::Runtime,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Target::Cset => "cset",
            Target::Ids => "ids",
            Target::OpenBimRl => "openbimrl",
            Target::Runtime => "runtime",
        }
    }

    /// Parse a CLI/token form (`"cset"`, `"ids"`, `"openbimrl"`, `"runtime"`).
    pub fn parse(s: &str) -> Option<Target> {
        match s.trim().to_lowercase().as_str() {
            "cset" => Some(Target::Cset),
            "ids" => Some(Target::Ids),
            "openbimrl" | "open_bim_rl" | "open-bim-rl" => Some(Target::OpenBimRl),
            "runtime" => Some(Target::Runtime),
            _ => None,
        }
    }
}

/// How completely a rule definition can be represented on a given target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "support", rename_all = "snake_case")]
pub enum Fidelity {
    /// Fully representable — no semantic loss.
    Full,
    /// Representable with documented loss (`reason` explains what is dropped).
    Partial { reason: String },
    /// Not representable on this target (`reason` explains why).
    Unsupported { reason: String },
}

impl Fidelity {
    pub fn is_usable(&self) -> bool {
        !matches!(self, Fidelity::Unsupported { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parse_roundtrip() {
        for t in Target::ALL {
            assert_eq!(Target::parse(t.as_str()), Some(t));
        }
        assert_eq!(Target::parse("open-bim-rl"), Some(Target::OpenBimRl));
        assert_eq!(Target::parse("nope"), None);
    }

    #[test]
    fn fidelity_usable() {
        assert!(Fidelity::Full.is_usable());
        assert!(Fidelity::Partial { reason: "x".into() }.is_usable());
        assert!(!Fidelity::Unsupported { reason: "x".into() }.is_usable());
    }
}
