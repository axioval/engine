//! The leaf test: one condition on one component.
//!
//! A predicate is the smallest selection unit — the neutral form of one
//! `ClassAndPropertyFilter` row. It says nothing about composition (that is
//! [`crate::classification::expr`]) and nothing about naming (that is
//! [`crate::classification::scheme`]).
//!
//! # Vendor-neutral by construction
//!
//! the provider stores a component class as a leaf class name (`SWall`), IFC calls
//! it `IfcWall`, Revit calls it something else again. The IR stores the
//! **canonical** concept and leaves the vendor spelling to the lowering in
//! `codec`. A predicate that mentioned `SWall` would have leaked the vendor into
//! the neutral layer.

use serde::{Deserialize, Serialize};

/// How a [`Predicate`] participates in its containing set.
///
/// 🚨 This is **not** boolean negation — `Ignore` is a third state the provider's
/// `filterState` really carries, and collapsing it to include/exclude changes
/// which components are checked. `Not` composition lives in
/// [`crate::classification::expr::Expr`], deliberately separate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The row contributes matches. the provider's default when unset.
    #[default]
    Include,
    /// The row removes matches contributed by other rows.
    Exclude,
    /// The row is retained but inert — preserved so a round-trip does not
    /// silently delete a disabled row the user expects to re-enable.
    Ignore,
}

/// How a stored value is compared against a component's actual value.
///
/// Kept explicit rather than folded into the value string: `Matches` is a
/// wildcard match and `Equals` is not, and guessing between them from the
/// presence of a `*` would misread a literal asterisk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// Wildcard pattern match (`*` = any).
    Matches,
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    /// Numeric comparisons — meaningful only for numeric properties.
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
    /// The property carries any non-empty value.
    Exists,
    /// The property is absent or empty.
    IsEmpty,
}

/// What a predicate reads off a component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    /// The component's entity class, canonically (`Wall`, `Slab`, `Space`).
    /// Lowered to `SWall` for the provider, `IfcWall` for IFC/IDS.
    ComponentClass,
    /// A built-in identification field (`Name`, `Type`, `Description`,
    /// `GUID`, …). the provider models these as `IdentificationPropertyReference`.
    Identification { field: String },
    /// A property in a named property set (`Pset_WallCommon.LoadBearing`).
    Property { set: String, name: String },
    /// Membership of another named [`crate::classification::scheme::Scheme`].
    ///
    /// This is what makes schemes composable: a scheme may be defined in terms
    /// of another. Corpus-backed — `.classification` has a dedicated
    /// `ClassificationClassificationColumn` for exactly this.
    Classification { scheme: String },
}

/// One leaf condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Predicate {
    pub subject: Subject,
    pub operator: Operator,
    /// The compared-against value. `*` is the native "any" wildcard.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, skip_serializing_if = "is_default_state")]
    pub state: State,
}

fn is_default_state(s: &State) -> bool {
    *s == State::Include
}

impl Predicate {
    /// A component-class test — the most common predicate by far.
    pub fn component_class(name: impl Into<String>) -> Self {
        Predicate {
            subject: Subject::ComponentClass,
            operator: Operator::Equals,
            value: name.into(),
            state: State::Include,
        }
    }

    /// A property-value test.
    pub fn property(
        set: impl Into<String>,
        name: impl Into<String>,
        operator: Operator,
        value: impl Into<String>,
    ) -> Self {
        Predicate {
            subject: Subject::Property {
                set: set.into(),
                name: name.into(),
            },
            operator,
            value: value.into(),
            state: State::Include,
        }
    }

    /// An identification-field test (`Name`, `Type`, …).
    pub fn identification(
        field: impl Into<String>,
        operator: Operator,
        value: impl Into<String>,
    ) -> Self {
        Predicate {
            subject: Subject::Identification {
                field: field.into(),
            },
            operator,
            value: value.into(),
            state: State::Include,
        }
    }

    /// Mark this predicate as removing matches rather than adding them.
    pub fn excluded(mut self) -> Self {
        self.state = State::Exclude;
        self
    }

    /// Mark this predicate inert while keeping it in the document.
    pub fn ignored(mut self) -> Self {
        self.state = State::Ignore;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_defaults_to_include() {
        assert_eq!(State::default(), State::Include);
        assert_eq!(Predicate::component_class("Wall").state, State::Include);
    }

    #[test]
    fn ignore_is_distinct_from_exclude() {
        // The whole point: three states, not two.
        let a = Predicate::component_class("Wall").excluded();
        let b = Predicate::component_class("Wall").ignored();
        assert_ne!(a.state, b.state);
    }

    #[test]
    fn roundtrips_through_json() {
        let p = Predicate::property("Pset_WallCommon", "LoadBearing", Operator::Equals, "true");
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Predicate>(&json).unwrap(), p);
    }
}
