//! What a rule may carry that most target formats cannot state.
//!
//! A rule is plain when it is enabled, reports itself, is not gated on
//! another rule, reports with one fixed severity, and applies to one
//! selected population. [`pre_check`] tells a plain rule from the others in
//! that order, so every profile refuses the same rules for the same first
//! reason; a profile that states gates or grading uses the single helpers
//! instead.

use std::fmt;

use axioval_ir::contract::{RuleApplicability, RuleFolder, RuleInstance, Selector, Severity};

/// Why a rule is not plain. Exhaustive on purpose: a new reason must reach
/// every profile's refusals, never fall through a wildcard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreCheck {
    /// The rule is disabled.
    Disabled,
    /// An auxiliary rule, which reports nothing itself.
    Auxiliary,
    /// The rule, or a folder around it, is gated on another rule.
    Gated,
    /// Severity bands, overrides or categories shape what the rule reports.
    Graded,
    /// A severity other than the one the format states, by its package
    /// spelling.
    Severity(String),
    /// Named target groups instead of one population.
    Groups,
}

impl fmt::Display for PreCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreCheck::Disabled => f.write_str("the rule is disabled"),
            PreCheck::Auxiliary => f.write_str("an auxiliary rule reports nothing itself"),
            PreCheck::Gated => f.write_str("the rule runs only as another rule's outcome allows"),
            PreCheck::Graded => f.write_str(
                "severity bands, severity overrides or categories shape what the rule reports",
            ),
            PreCheck::Severity(severity) => write!(f, "the rule reports with severity {severity}"),
            PreCheck::Groups => f.write_str("the rule applies to named target groups"),
        }
    }
}

/// The selector of a plain rule reporting with `severity`, or the first
/// reason it is not plain: disabled, auxiliary, gated (itself, or `gated`
/// by a folder around it), graded, another severity, target groups.
///
/// # Errors
///
/// The first reason the rule is not plain.
pub fn pre_check<'r>(
    rule: &'r RuleInstance,
    gated: bool,
    severity: &Severity,
) -> Result<&'r Selector, PreCheck> {
    if !rule.enabled {
        return Err(PreCheck::Disabled);
    }
    if rule.auxiliary {
        return Err(PreCheck::Auxiliary);
    }
    if is_gated(rule, gated) {
        return Err(PreCheck::Gated);
    }
    if is_graded(rule) {
        return Err(PreCheck::Graded);
    }
    if rule.severity != *severity {
        return Err(PreCheck::Severity(severity_name(&rule.severity)));
    }
    selector(rule).ok_or(PreCheck::Groups)
}

/// Whether `rule` has a gate, or is `enclosing`-ly gated by a folder
/// around it.
#[must_use]
pub fn is_gated(rule: &RuleInstance, enclosing: bool) -> bool {
    enclosing || rule.gate.is_some()
}

/// Whether `folder` has a gate, or is `enclosing`-ly gated by a folder
/// around it; its rules and subfolders are gated when it is.
#[must_use]
pub fn is_folder_gated(folder: &RuleFolder, enclosing: bool) -> bool {
    enclosing || folder.gate.is_some()
}

/// Whether severity bands, severity overrides or categories shape what
/// `rule` reports.
#[must_use]
pub fn is_graded(rule: &RuleInstance) -> bool {
    !rule.severity_bands.is_empty()
        || !rule.severity_overrides.is_empty()
        || !rule.categories.is_empty()
}

/// A severity as a package spells it, such as `warning`.
#[must_use]
pub fn severity_name(severity: &Severity) -> String {
    serde_json::to_value(severity)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The selector `rule` applies to, `None` when it applies to named target
/// groups.
#[must_use]
pub fn selector(rule: &RuleInstance) -> Option<&Selector> {
    match &rule.applicability {
        RuleApplicability::Selector(selector) => Some(selector),
        RuleApplicability::Groups(_) => None,
    }
}
