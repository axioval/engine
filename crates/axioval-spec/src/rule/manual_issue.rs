use serde::{Deserialize, Serialize};

use crate::rule::assertion::Severity;
use crate::rule::execution::ElementScopeSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualIssueSpec {
    pub category: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ElementScopeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualIssuePlanSpec {
    #[serde(default)]
    pub issues: Vec<ManualIssueSpec>,
    #[serde(default = "default_severity")]
    pub severity: Severity,
}

impl ManualIssuePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (index, issue) in self.issues.iter().enumerate() {
            for (field, value) in [
                ("category", &issue.category),
                ("name", &issue.name),
                ("description", &issue.description),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("issue row {index} has blank `{field}`"));
                }
            }
        }
        Ok(())
    }
}

fn default_severity() -> Severity {
    Severity::Warning
}
