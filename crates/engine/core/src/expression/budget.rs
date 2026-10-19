//! The expression evaluation budget (#447): the nodes one rule may
//! evaluate, charged as if everything it reads were built for it.
//!
//! - **A share per rule.** The runtime gives each rule its own
//!   [`EvaluationBudget`] of the run's size and makes it the thread's
//!   current one ([`EvaluationBudget::enter`]), on every worker the rule
//!   fans out to too. Every node an expression evaluates is spent from the
//!   current budget ([`spend`]), so what a rule is refused depends on its
//!   own request alone, never on the rules before it.
//! - **Shared values charged as if built.** A value built once and read by
//!   many (a ruleset value, a measured value or a selection kept in the
//!   run's memo) is built in a budget of its own, a whole share
//!   ([`built`]), and the nodes it took are kept beside it ([`Cost`]).
//!   Every reader is charged them ([`charge`]), whether it built the value
//!   or found it kept. A reader that cannot afford them has nothing left:
//!   the read is refused and so is every node after it. So a refusal never
//!   depends on what was reused, on which rule read a value first, or on
//!   the store keeping anything (`Runtime::without_reuse`).
//! - **Threads.** One rule's subjects spend one share whatever the
//!   threads. Which subject a spent share refuses depends on the order
//!   they are spent in, so the runtime judges a rule whose share ran out
//!   on several threads again on one, its subjects in their order; a rule
//!   whose share did not run out reports the same on any number of
//!   threads, every node it evaluated having been granted.
//!
//! The run-wide backstop, refusing whole rules in plan order once the
//! rules before them spent the run's size, is the runtime's
//! (`Runtime::with_evaluation_budget`).
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The default number of expression nodes one rule may evaluate, and the
/// rules of one run before the backstop refuses the rest.
pub const DEFAULT_EVALUATION_BUDGET: u64 = 100_000_000;

/// The nodes one rule (or one shared value's build) may still evaluate.
#[derive(Debug)]
pub struct EvaluationBudget {
    remaining: AtomicU64,
    limit: u64,
    /// The share a value built within it is given: a rule's whole one.
    share: u64,
    /// Whether a node or a charge was refused.
    refused: AtomicBool,
}

/// The nodes a shared value took to build, charged to every reader as if
/// built for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cost {
    /// The nodes its build evaluated, those of the shared values it read
    /// included.
    pub nodes: u64,
    /// Whether its build ran out of its share: no reader can afford it.
    pub refused: bool,
}

impl EvaluationBudget {
    /// A budget of `limit` nodes, a value built within it given as many.
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self::within(limit, limit)
    }

    /// A budget of `limit` nodes, a value built within it given `share`:
    /// a rule past the run-wide backstop has none to spend, but the values
    /// it reads are built as for any other.
    #[must_use]
    pub fn within(limit: u64, share: u64) -> Self {
        Self {
            remaining: AtomicU64::new(limit),
            limit,
            share,
            refused: AtomicBool::new(false),
        }
    }

    /// Spends one node; false once the budget is spent.
    pub fn spend(&self) -> bool {
        let spent = self
            .remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if !spent {
            self.refused.store(true, Ordering::Relaxed);
        }
        spent
    }

    /// Charges what a shared value took to build; false, and nothing
    /// left, where it is more than what is left.
    pub fn charge(&self, cost: Cost) -> bool {
        if cost.nodes == 0 && !cost.refused {
            return true;
        }
        let charged = !cost.refused
            && self
                .remaining
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                    left.checked_sub(cost.nodes)
                })
                .is_ok();
        if !charged {
            self.remaining.store(0, Ordering::Relaxed);
            self.refused.store(true, Ordering::Relaxed);
        }
        charged
    }

    /// Whether a node or a charge was refused: the budget ran out while
    /// something was still to be evaluated.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.refused.load(Ordering::Relaxed)
    }

    /// The nodes spent and charged so far.
    #[must_use]
    pub fn spent(&self) -> u64 {
        self.limit - self.remaining.load(Ordering::Relaxed)
    }

    /// The budget's size.
    #[must_use]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// What it took, as a shared value built within it is charged.
    #[must_use]
    pub fn cost(&self) -> Cost {
        Cost {
            nodes: self.spent(),
            refused: self.refused(),
        }
    }

    /// Makes this the calling thread's budget ([`current`]) until the
    /// returned guard is dropped, which puts back the one it replaced.
    pub fn enter(self: &Arc<Self>) -> Entered {
        enter(Some(Arc::clone(self)))
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<EvaluationBudget>>> = const { RefCell::new(None) };
}

/// The budget the calling thread evaluates expressions within, if any.
#[must_use]
pub fn current() -> Option<Arc<EvaluationBudget>> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Spends one node of the calling thread's budget; true without one.
#[must_use]
pub fn spend() -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_none_or(|budget| budget.spend())
    })
}

/// Charges `cost` to the calling thread's budget
/// ([`EvaluationBudget::charge`]); true without one.
#[must_use]
pub fn charge(cost: Cost) -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_none_or(|budget| budget.charge(cost))
    })
}

/// Whether the calling thread's budget refused a node or a charge.
#[must_use]
pub fn exhausted() -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_some_and(|budget| budget.refused())
    })
}

/// `build`'s value, built for sharing in a budget of its own, and what it
/// took: a whole share where the calling thread has a budget, unbounded
/// (and only counted) where it has none. Nothing is charged to the
/// calling thread's budget; each reader charges the cost
/// ([`charge`]), whether it built the value or found it kept.
pub fn built<T>(build: impl FnOnce() -> T) -> (T, Cost) {
    let share = current().map_or(u64::MAX, |budget| budget.share);
    let own = Arc::new(EvaluationBudget::within(share, share));
    let value = {
        let _own = own.enter();
        build()
    };
    (value, own.cost())
}

/// `value`, built at `cost` ([`built`]), as its reader reads it: the cost
/// charged to the calling thread's budget, which has nothing left where it
/// cannot afford it.
pub fn read_built<T>(value: T, cost: Cost) -> T {
    let _ = charge(cost);
    value
}

/// The calling thread's budget while held ([`EvaluationBudget::enter`]);
/// the one it replaced is put back when dropped.
#[must_use = "the budget is entered only while this is held"]
#[derive(Debug)]
pub struct Entered {
    outer: Option<Arc<EvaluationBudget>>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let outer = self.outer.take();
        CURRENT.with(|current| *current.borrow_mut() = outer);
    }
}

/// Makes `budget` the calling thread's ([`current`]) until the guard is
/// dropped; `None` makes it have none.
pub fn enter(budget: Option<Arc<EvaluationBudget>>) -> Entered {
    let outer = CURRENT.with(|current| std::mem::replace(&mut *current.borrow_mut(), budget));
    Entered { outer }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_value_is_charged_to_every_reader_as_if_built() {
        let rule = Arc::new(EvaluationBudget::new(10));
        let _rule = rule.enter();
        let (value, cost) = built(|| {
            for _ in 0..4 {
                assert!(spend());
            }
            7
        });
        assert_eq!(
            cost,
            Cost {
                nodes: 4,
                refused: false
            }
        );
        // Nothing is charged by the build itself.
        assert_eq!(rule.spent(), 0);
        assert_eq!(read_built(value, cost), 7);
        assert_eq!(read_built(value, cost), 7);
        assert_eq!(rule.spent(), 8);
        assert!(!rule.refused());
        // A third read cannot be afforded: nothing is left.
        assert!(!charge(cost));
        assert!(rule.refused());
        assert!(!spend());
    }

    #[test]
    fn a_build_is_given_a_whole_share_whatever_its_reader_has_left() {
        let rule = Arc::new(EvaluationBudget::new(5));
        let _rule = rule.enter();
        for _ in 0..4 {
            assert!(spend());
        }
        let ((), cost) = built(|| {
            for _ in 0..5 {
                assert!(spend());
            }
            assert!(!spend());
        });
        assert_eq!(
            cost,
            Cost {
                nodes: 5,
                refused: true
            }
        );
        assert!(!charge(cost));
        assert_eq!(rule.spent(), 5);
    }

    #[test]
    fn without_a_budget_everything_is_granted_and_counted() {
        let _none = enter(None);
        assert!(spend());
        let ((), cost) = built(|| {
            assert!(spend());
            assert!(spend());
        });
        assert_eq!(cost.nodes, 2);
        assert!(charge(cost));
        assert!(!exhausted());
    }
}
