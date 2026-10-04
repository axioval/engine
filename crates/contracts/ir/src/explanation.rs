//! Explanations of an expression's verdict, carried by the findings and
//! not-evaluated outcomes of expression rules.

use serde::{Deserialize, Serialize};

/// The most entries an explanation keeps besides its deciding path, which it
/// always keeps whole.
pub const MAX_EXPLANATION_ENTRIES: usize = 64;

/// How an expression reached a verdict: one entry per subexpression it evaluated, in
/// evaluation order, with their values, the deciding path marked.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Explanation {
    pub entries: Vec<ExplanationEntry>,
    /// Whether entries beyond [`MAX_EXPLANATION_ENTRIES`] off the deciding path
    /// were left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

/// One evaluated subexpression of an [`Explanation`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationEntry {
    /// Where in the expression, such as `requirement.and[1].compare.left`.
    pub path: String,
    /// The node's kind, such as `compare`.
    pub kind: String,
    /// The author's label, if the node carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Its value, rendered with its unit and interval (`0.045..0.055 m`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Why it was not evaluated, when it was not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_evaluated: Option<String>,
    /// Whether it lies on the path that decided the verdict: the node that
    /// failed or was not evaluated, its ancestors and its operands.
    #[serde(default, skip_serializing_if = "is_false")]
    pub deciding: bool,
}

impl Explanation {
    /// The entries on the deciding path, in evaluation order.
    pub fn deciding(&self) -> impl Iterator<Item = &ExplanationEntry> {
        self.entries.iter().filter(|entry| entry.deciding)
    }
}

impl ExplanationEntry {
    /// The entry as a reviewer reads it: its label or path and kind, and
    /// its value or why it was not evaluated, `requirement.and[1]
    /// (compare) = false`.
    #[must_use]
    pub fn describe(&self) -> String {
        let name = match &self.label {
            Some(label) => format!("{label} ({})", self.path),
            None => format!("{} ({})", self.path, self.kind),
        };
        match (&self.value, &self.not_evaluated) {
            (Some(value), _) => format!("{name} = {value}"),
            (None, Some(why)) => format!("{name}: not evaluated, {why}"),
            (None, None) => name,
        }
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
