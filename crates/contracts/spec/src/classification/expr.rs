//! Boolean composition over [`Predicate`]s.
//!
//! One source format composes filter rows into `OrFilter` / `AndFilter` / `NotFilter`
//! trees; this is the neutral form of that tree. It is deliberately a *separate*
//! layer from [`crate::classification::predicate::State`]: include/exclude/ignore is
//! a property of a row within a set, while And/Or/Not is a property of the
//! structure. Conflating them is how `Ignore` gets lost.

use serde::{Deserialize, Serialize};

use super::predicate::Predicate;

/// A boolean expression over predicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Expr {
    /// A single leaf test.
    Leaf { predicate: Predicate },
    /// True when every child is true. Empty `And` is vacuously true.
    And { children: Vec<Expr> },
    /// True when any child is true. Empty `Or` is vacuously false.
    Or { children: Vec<Expr> },
    /// Logical negation of the child.
    Not { child: Box<Expr> },
    /// Matches every component — the explicit "no restriction" value.
    ///
    /// Having this as a variant rather than as an empty `And` means "everything"
    /// and "nothing specified yet" are distinguishable, which matters when
    /// deciding whether a lowering may omit the filter entirely.
    Any,
}

impl Expr {
    pub fn leaf(predicate: Predicate) -> Self {
        Expr::Leaf { predicate }
    }

    pub fn and(children: impl IntoIterator<Item = Expr>) -> Self {
        Expr::And {
            children: children.into_iter().collect(),
        }
    }

    pub fn or(children: impl IntoIterator<Item = Expr>) -> Self {
        Expr::Or {
            children: children.into_iter().collect(),
        }
    }

    pub fn negate(child: Expr) -> Self {
        !child
    }

    /// Every predicate in the tree, depth-first, left to right.
    ///
    /// Order is stable so a lowering that flattens to a row table produces the
    /// same table every run — the same determinism concern as the model's
    /// hash-order bug.
    pub fn predicates(&self) -> Vec<&Predicate> {
        let mut out = Vec::new();
        self.walk(&mut |p| out.push(p));
        out
    }

    fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Predicate)) {
        match self {
            Expr::Leaf { predicate } => f(predicate),
            Expr::And { children } | Expr::Or { children } => {
                for c in children {
                    c.walk(f);
                }
            }
            Expr::Not { child } => child.walk(f),
            Expr::Any => {}
        }
    }

    /// Depth of the tree — 0 for a leaf or `Any`.
    ///
    /// A target whose filter model is a flat row table (a `.filter` table)
    /// can represent depth ≤ 1 of a single connective; deeper trees need the
    /// lowering to say so rather than silently flatten. See R12.
    pub fn depth(&self) -> usize {
        match self {
            Expr::Leaf { .. } | Expr::Any => 0,
            Expr::And { children } | Expr::Or { children } => {
                1 + children.iter().map(Expr::depth).max().unwrap_or(0)
            }
            Expr::Not { child } => 1 + child.depth(),
        }
    }

    /// True when the tree contains no predicates at all.
    pub fn is_empty(&self) -> bool {
        matches!(self, Expr::Any) || self.predicates().is_empty()
    }
}

impl std::ops::Not for Expr {
    type Output = Self;

    fn not(self) -> Self::Output {
        Expr::Not {
            child: Box::new(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::predicate::{Operator, Predicate};

    fn wall() -> Predicate {
        Predicate::component_class("Wall")
    }

    #[test]
    fn collects_predicates_in_stable_order() {
        let e = Expr::or([
            Expr::leaf(wall()),
            Expr::and([
                Expr::leaf(Predicate::component_class("Proxy")),
                Expr::leaf(Predicate::identification(
                    "Name",
                    Operator::Matches,
                    "Wall*",
                )),
            ]),
        ]);
        let names: Vec<_> = e.predicates().iter().map(|p| p.value.clone()).collect();
        assert_eq!(names, vec!["Wall", "Proxy", "Wall*"]);
        // Repeated calls must not reorder.
        assert_eq!(e.predicates().len(), 3);
    }

    #[test]
    fn depth_distinguishes_flat_from_nested() {
        assert_eq!(Expr::leaf(wall()).depth(), 0);
        assert_eq!(Expr::or([Expr::leaf(wall())]).depth(), 1);
        assert_eq!(
            Expr::or([Expr::and([Expr::leaf(wall())])]).depth(),
            2,
            "a nested connective must be visible to a flat-table lowering"
        );
    }

    #[test]
    fn any_is_not_the_same_as_empty_and() {
        assert!(Expr::Any.is_empty());
        assert!(Expr::and([]).is_empty());
        assert_ne!(Expr::Any, Expr::and([]));
    }
}
