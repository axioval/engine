use serde::{Deserialize, Serialize};

/// Pipeline stage at which a rule instance could not advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    CodecToSpec,
    SpecToRuntime,
}

/// Stable machine-readable diagnostic category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    UnsupportedRule,
    IncompleteConfiguration,
    UnsupportedParameter,
}

/// A structured explanation for a rule that was not executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleDiagnostic {
    /// Source adapter type/class. This is provenance only; semantic compilers
    /// must use `definition_id`, never dispatch on this field.
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
    pub stage: DiagnosticStage,
    pub code: DiagnosticCode,
    pub message: String,
}
