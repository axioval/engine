use serde::{Deserialize, Serialize};

use super::assertion::Severity;
use super::execution::ElementScopeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessPolicy {
    Allowed,
    Required,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    DoorOrOpening,
    Door,
    Opening,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceConnectionPlanSpec {
    pub spaces_a: ElementScopeSpec,
    pub spaces_b: ElementScopeSpec,
    pub between_policy: AccessPolicy,
    pub between_kind: AccessKind,
    pub outside_policy: AccessPolicy,
    pub outside_kind: AccessKind,
    #[serde(default = "default_severity")]
    pub severity: Severity,
}

impl SpaceConnectionPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_space_scope("spaces_a", &self.spaces_a)?;
        validate_space_scope("spaces_b", &self.spaces_b)?;
        Ok(())
    }
}

fn validate_space_scope(label: &str, scope: &ElementScopeSpec) -> Result<(), String> {
    if scope.candidate_types.is_empty() {
        return Err(format!("{label} must select at least one IFC space type"));
    }
    if scope
        .candidate_types
        .iter()
        .any(|value| !value.trim().eq_ignore_ascii_case("IFCSPACE"))
    {
        return Err(format!(
            "{label} must contain only IFCSPACE candidate types"
        ));
    }
    Ok(())
}

fn default_severity() -> Severity {
    Severity::Warning
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_plan_rejects_non_space_candidate_types() {
        let plan = SpaceConnectionPlanSpec {
            spaces_a: ElementScopeSpec {
                candidate_types: vec!["IFCWALL".into()],
                ..Default::default()
            },
            spaces_b: ElementScopeSpec {
                candidate_types: vec!["IFCSPACE".into()],
                ..Default::default()
            },
            between_policy: AccessPolicy::Allowed,
            between_kind: AccessKind::DoorOrOpening,
            outside_policy: AccessPolicy::Allowed,
            outside_kind: AccessKind::DoorOrOpening,
            severity: Severity::Warning,
        };
        assert!(plan.validate().is_err());
    }
}
