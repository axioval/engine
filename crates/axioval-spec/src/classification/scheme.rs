//! A **named, ordered dispatch** over selection expressions — the abstraction.
//!
//! # Why this is not just a filter with a name
//!
//! The corpus settles the containment direction: a `ClassificationDocument`
//! **contains** `filters: list[CsetFilter]`, while a `ClassAndPropertyFilter`
//! contains no classification. So a scheme is not a sibling of a filter, it is
//! the layer above it.
//!
//! The practical payoff: with raw filters, a project that models walls as
//! generic named objects needs a new row in *every rule* that touches walls.
//! With a scheme the exception is absorbed once, in one named artifact with
//! documented provenance, and every rule stays canonical — it asks for `Wall`,
//! not for `IfcWall OR (IfcBuildingElementProxy AND Name~"Wall*")`.

use serde::{Deserialize, Serialize};

use super::expr::Expr;
use crate::LocalizedText;

/// How a component is matched against the ordered [`Scheme::rows`].
///
/// 🚨 **Semantic, not cosmetic.** Only the ordinal is serialized in
/// `.classification`, and the choice changes results:
/// * [`MatchMethod::FirstMatch`] makes **row order load-bearing** — the first
///   matching row wins and later rows never run.
/// * [`MatchMethod::AllMatching`] lets one component carry **several**
///   classifications at once.
///
/// An IR that dropped this, or defaulted it, would silently change which
/// components end up in which class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMethod {
    /// First matching row wins; order matters.
    FirstMatch,
    /// The best-scoring row wins.
    BestMatch,
    /// Every matching row applies; a component may be multi-classified.
    AllMatching,
}

impl MatchMethod {
    /// Declaration order — this IS the `.classification` decode key, since only
    /// the enum ordinal is serialized.
    pub const ORDINALS: [MatchMethod; 3] = [
        MatchMethod::FirstMatch,
        MatchMethod::BestMatch,
        MatchMethod::AllMatching,
    ];

    pub fn from_ordinal(o: usize) -> Option<Self> {
        Self::ORDINALS.get(o).copied()
    }

    pub fn ordinal(self) -> usize {
        Self::ORDINALS.iter().position(|m| *m == self).unwrap_or(0)
    }

    /// True when row order changes the outcome.
    pub fn order_is_load_bearing(self) -> bool {
        matches!(self, MatchMethod::FirstMatch)
    }
}

/// What a matching row assigns.
///
/// 🚨 **Formula rows must survive as formulas.** Native
/// `ClassificationTableModel` treats a leading `=` as an expression over column
/// references (`A`–`Z`, `AA`–`ZZ`) evaluated **per component at check time**.
/// 25 of 707 corpus rows are formulas. Flattening one to a literal name
/// produces confidently wrong output, so the IR makes the distinction a type,
/// not a string convention a later reader has to re-detect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assignment {
    /// A literal class name.
    Name { value: String },
    /// An expression over column references, evaluated per component.
    Formula { expression: String },
}

impl Assignment {
    /// Classify stored text the way the native table model does.
    pub fn parse(text: &str) -> Self {
        match text.strip_prefix('=') {
            Some(rest) => Assignment::Formula {
                expression: rest.to_string(),
            },
            None => Assignment::Name {
                value: text.to_string(),
            },
        }
    }

    /// The stored form, with the `=` sentinel restored for formulas.
    pub fn to_stored(&self) -> String {
        match self {
            Assignment::Name { value } => value.clone(),
            Assignment::Formula { expression } => format!("={expression}"),
        }
    }

    /// Literal name, if this is one. Formulas deliberately return `None` —
    /// their text is not a member of the assignable set.
    pub fn literal(&self) -> Option<&str> {
        match self {
            Assignment::Name { value } => Some(value),
            Assignment::Formula { .. } => None,
        }
    }
}

/// One row: a condition and what it assigns when matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeRow {
    pub condition: Expr,
    pub assigns: Assignment,
    /// Display colour, in the shared `@poing/material:v1:` envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

/// Where a scheme came from — the provenance a bare filter table cannot carry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Issuing body or standard (`"DIN 276"`, `"Uniclass 2015"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Edition or version of that standard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    /// True when the scheme was imported rather than authored in place.
    #[serde(default)]
    pub imported: bool,
}

/// A named, ordered, methoded classification scheme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheme {
    /// Stable id rules reference by name — never inlined at the use site.
    pub id: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub title: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub description: LocalizedText,
    pub method: MatchMethod,
    /// Ordered rows. Order is load-bearing under [`MatchMethod::FirstMatch`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<SchemeRow>,
    /// Explicit rows from the provider's classification-name/defaultData table.
    /// They may exist without a literal rule assignment in formula-based schemes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<SchemeName>,
    /// Persisted native classification behavior flags.
    #[serde(default)]
    pub settings: SchemeSettings,
    /// Component scope — which components the scheme considers at all.
    #[serde(default = "any_expr", skip_serializing_if = "is_any")]
    pub scope: Expr,
    #[serde(default)]
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemeName {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchemeSettings {
    pub locked: bool,
    pub allow_multiple_names: bool,
    pub use_dates_as_names: bool,
    pub show_unclassified: bool,
    pub use_model_colors: bool,
    pub show_spaces_as_surfaces: bool,
}

fn any_expr() -> Expr {
    Expr::Any
}

fn is_any(e: &Expr) -> bool {
    matches!(e, Expr::Any)
}

impl Scheme {
    pub fn new(id: impl Into<String>, method: MatchMethod) -> Self {
        Scheme {
            id: id.into(),
            title: LocalizedText::empty(),
            description: LocalizedText::empty(),
            method,
            rows: Vec::new(),
            names: Vec::new(),
            settings: SchemeSettings::default(),
            scope: Expr::Any,
            provenance: Provenance::default(),
        }
    }

    pub fn with_row(mut self, row: SchemeRow) -> Self {
        self.rows.push(row);
        self
    }

    /// Distinct literal class names, in table order.
    ///
    /// Formula rows are excluded — their text is an expression, not a name.
    /// This mirrors `codec`'s `Document::classifications`, which is the
    /// behaviour the corpus validated.
    pub fn class_names(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for row in &self.rows {
            if let Some(name) = row.assigns.literal() {
                if !name.is_empty() && !seen.contains(&name) {
                    seen.push(name);
                }
            }
        }
        seen
    }

    /// True when any row assigns via a formula.
    pub fn has_formula_rows(&self) -> bool {
        self.rows
            .iter()
            .any(|r| matches!(r.assigns, Assignment::Formula { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::predicate::Predicate;

    fn row(name: &str) -> SchemeRow {
        SchemeRow {
            condition: Expr::leaf(Predicate::component_class(name)),
            assigns: Assignment::Name {
                value: name.to_string(),
            },
            material: None,
        }
    }

    #[test]
    fn method_ordinals_match_declaration_order() {
        // This list IS the .classification decode key — only the ordinal is
        // serialized, so a reorder silently remaps every stored document.
        assert_eq!(MatchMethod::from_ordinal(0), Some(MatchMethod::FirstMatch));
        assert_eq!(MatchMethod::from_ordinal(1), Some(MatchMethod::BestMatch));
        assert_eq!(MatchMethod::from_ordinal(2), Some(MatchMethod::AllMatching));
        assert_eq!(MatchMethod::from_ordinal(3), None);
        for m in MatchMethod::ORDINALS {
            assert_eq!(MatchMethod::from_ordinal(m.ordinal()), Some(m));
        }
    }

    #[test]
    fn only_first_match_makes_order_load_bearing() {
        assert!(MatchMethod::FirstMatch.order_is_load_bearing());
        assert!(!MatchMethod::AllMatching.order_is_load_bearing());
    }

    #[test]
    fn formula_rows_survive_as_formulas() {
        let f = Assignment::parse("=A&\"-\"&B");
        assert!(matches!(f, Assignment::Formula { .. }));
        assert_eq!(f.literal(), None, "a formula is not an assignable name");
        assert_eq!(
            f.to_stored(),
            "=A&\"-\"&B",
            "the = sentinel must round-trip"
        );

        let n = Assignment::parse("Wall");
        assert_eq!(n.literal(), Some("Wall"));
        assert_eq!(n.to_stored(), "Wall");
    }

    #[test]
    fn class_names_exclude_formulas() {
        let s = Scheme::new("x", MatchMethod::FirstMatch)
            .with_row(row("Wall"))
            .with_row(SchemeRow {
                condition: Expr::Any,
                assigns: Assignment::parse("=A"),
                material: None,
            })
            .with_row(row("Slab"));
        assert_eq!(s.class_names(), vec!["Wall", "Slab"]);
        assert!(s.has_formula_rows());
    }

    #[test]
    fn roundtrips_through_json() {
        let s = Scheme::new("din276", MatchMethod::AllMatching).with_row(row("Wall"));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Scheme>(&json).unwrap(), s);
    }
}
