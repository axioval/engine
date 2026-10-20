//! Evaluating a rule's subjects, and a check's independent rules, on
//! several threads, the report exactly the serial evaluation's.
//!
//! A host sizes the pool ([`crate::Runtime::with_parallelism`]); the
//! runtime installs it in every run's services, and a capability reads it
//! there ([`Parallelism::of`]). Without one a run is serial.
//!
//! [`Parallelism::map_ordered`] answers in the subjects' order, never in
//! the order the threads finish, so a capability merging its subjects'
//! outcomes merges them as the serial loop pushes them. A panic in one
//! subject surfaces as the serial loop's would: the first subject's in
//! order, after every subject before it was evaluated.
//!
//! A run's threads are one pool (#406): the rules the runtime evaluates at
//! once each hold one of its threads, and a rule's subjects are judged on
//! the threads none holds (`Pool`), so rules and subjects together never
//! keep more threads busy than the run's parallelism, and no pool is ever
//! nested in another.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::ServiceRegistry;
use crate::expression::budget as evaluation;
use crate::measured::provider::RuleTable;
use crate::{derivation, memory, timing};

/// The threads one run may keep busy at once, shared by the rules it
/// evaluates at once and the subjects each judges (#406).
///
/// A thread is taken ([`Pool::take`], [`Pool::try_take`]) for as long as
/// it works and given back when its [`PoolThread`] is dropped. The runtime
/// takes one for each rule it evaluates, waiting for one to be free;
/// [`Parallelism::map_ordered`] starts a worker only on a thread it can
/// take at once, never waiting, so a rule's subjects take the threads the
/// other rules leave free, and more of them as those rules finish.
#[derive(Debug)]
pub(crate) struct Pool {
    spare: Mutex<usize>,
    freed: Condvar,
}

impl Pool {
    /// A pool of `threads` threads.
    pub(crate) fn new(threads: usize) -> Arc<Self> {
        Arc::new(Self {
            spare: Mutex::new(threads),
            freed: Condvar::new(),
        })
    }

    /// Takes a thread, waiting for one to be free.
    pub(crate) fn take(self: &Arc<Self>) -> PoolThread {
        let mut spare = self.spare.lock().unwrap_or_else(PoisonError::into_inner);
        while *spare == 0 {
            spare = self
                .freed
                .wait(spare)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *spare -= 1;
        PoolThread(Arc::clone(self))
    }

    /// Takes a thread if one is free now.
    pub(crate) fn try_take(self: &Arc<Self>) -> Option<PoolThread> {
        let mut spare = self.spare.lock().unwrap_or_else(PoisonError::into_inner);
        (*spare > 0).then(|| {
            *spare -= 1;
            PoolThread(Arc::clone(self))
        })
    }
}

/// One thread taken from a [`Pool`], given back when dropped.
#[derive(Debug)]
pub(crate) struct PoolThread(Arc<Pool>);

impl Drop for PoolThread {
    fn drop(&mut self) {
        let mut spare = self.0.spare.lock().unwrap_or_else(PoisonError::into_inner);
        *spare += 1;
        drop(spare);
        self.0.freed.notify_one();
    }
}

thread_local! {
    /// The pool of the run this thread works for ([`enter_pool`]).
    static POOL: RefCell<Option<Arc<Pool>>> = const { RefCell::new(None) };
}

/// The pool of the run the calling thread works for, if any.
pub(crate) fn current_pool() -> Option<Arc<Pool>> {
    POOL.with(|pool| pool.borrow().clone())
}

/// The calling thread's pool while held ([`enter_pool`]); the one it
/// replaced is put back when dropped.
#[must_use = "the pool is entered only while this is held"]
pub(crate) struct PoolEntered {
    outer: Option<Arc<Pool>>,
}

impl Drop for PoolEntered {
    fn drop(&mut self) {
        let outer = self.outer.take();
        POOL.with(|pool| *pool.borrow_mut() = outer);
    }
}

/// Makes `pool` the calling thread's until the guard is dropped; `None`
/// makes it have none, and then [`Parallelism::map_ordered`] starts its
/// workers without asking any pool.
pub(crate) fn enter_pool(pool: Option<Arc<Pool>>) -> PoolEntered {
    let outer = POOL.with(|current| std::mem::replace(&mut *current.borrow_mut(), pool));
    PoolEntered { outer }
}

/// How many threads a run evaluates a rule's subjects on, the thread
/// running the rule among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Parallelism(NonZeroUsize);

/// The stack of each worker thread: a main thread's usual size, so a
/// subject recursing as deep as it may on the thread running the rule
/// does so on a worker too.
pub(crate) const WORKER_STACK: usize = 8 * 1024 * 1024;

impl Parallelism {
    /// One thread: the rule's own.
    pub const SERIAL: Self = Self(NonZeroUsize::MIN);

    /// `threads` threads.
    #[must_use]
    pub const fn new(threads: NonZeroUsize) -> Self {
        Self(threads)
    }

    /// As many threads as the machine runs at once
    /// ([`std::thread::available_parallelism`]), one where that is unknown.
    #[must_use]
    pub fn available() -> Self {
        std::thread::available_parallelism().map_or(Self::SERIAL, Self)
    }

    /// How many threads.
    #[must_use]
    pub const fn threads(self) -> usize {
        self.0.get()
    }

    /// Whether only the rule's own thread evaluates.
    #[must_use]
    pub const fn is_serial(self) -> bool {
        self.0.get() == 1
    }

    /// The parallelism a run installed in `services`, serial without one.
    #[must_use]
    pub fn of(services: &ServiceRegistry) -> Self {
        services.get::<Self>().copied().unwrap_or(Self::SERIAL)
    }

    /// `map` of each of `items`, in `items`' order.
    ///
    /// The items are taken in order by at most [`Self::threads`] threads,
    /// this one among them, each taking the next item when it is done
    /// with one: no more than that many items are being mapped at once.
    /// Serial, one item, or a pool that cannot start, maps them on this
    /// thread one after the other.
    ///
    /// Within a run, a worker is started only on a thread of the run's
    /// pool that no rule or other worker holds (`Pool`, #406), taken
    /// before this thread maps its next item and given back when the
    /// worker runs out of items: the rules evaluated at once and their
    /// subjects together keep no more threads busy than the run's
    /// parallelism. Which thread maps an item never changes its answer.
    ///
    /// A panic in `map` is raised again here once every item before it
    /// is mapped: the panic of the first item in order that panicked, as
    /// a serial loop raises it. No item after a panicked one is started.
    ///
    /// The host calls the workers make count towards the rule this thread
    /// is timing, as they would on this thread, and the steps they admit
    /// are admitted against this thread's memory budget
    /// ([`memory::current`]), so the memory those steps take together is
    /// bounded whatever the threads. Their expressions spend this thread's
    /// expression evaluation budget ([`evaluation::current`]): the rule's
    /// one share. What they keep for the rule alone is kept in the rule's
    /// table (`MeasuredMemo::of_rule`).
    pub fn map_ordered<T, R>(self, items: &[T], map: impl Fn(&T) -> R + Sync) -> Vec<R>
    where
        T: Sync,
        R: Send,
    {
        let workers = self.threads().min(items.len());
        if workers <= 1 {
            return items.iter().map(map).collect();
        }
        let next = AtomicUsize::new(0);
        // The first item known to have panicked: none after it is started.
        let stop = AtomicUsize::new(usize::MAX);
        // Maps the next item into `done`; false once none is left.
        let take = |done: &mut Vec<(usize, std::thread::Result<R>)>| {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= items.len() || index > stop.load(Ordering::Relaxed) {
                return false;
            }
            let answer = panic::catch_unwind(AssertUnwindSafe(|| map(&items[index])));
            if answer.is_err() {
                stop.fetch_min(index, Ordering::Relaxed);
            }
            done.push((index, answer));
            true
        };
        let work = || {
            let mut done = Vec::new();
            while take(&mut done) {}
            done
        };
        let timed = timing::fanning_out();
        // Workers work within the derivations this thread is building, so
        // one reading a key this thread builds is a cycle, never a wait.
        let lineage = derivation::lineage();
        // The workers admit their steps against this thread's budget, and
        // derive through its store.
        let budget = memory::current();
        let store = derivation::current();
        // And spend the expression evaluation budget of the rule this
        // thread evaluates (#447), keeping what is the rule's own in its
        // table, on the threads of the run's pool (#406).
        let evaluation = evaluation::current();
        let rule = RuleTable::current();
        let pool = current_pool();
        let (work, lineage, budget, store, evaluation, rule, pool) =
            (&work, &lineage, &budget, &store, &evaluation, &rule, &pool);
        let mut slots: Vec<Option<std::thread::Result<R>>> =
            std::iter::repeat_with(|| None).take(items.len()).collect();
        std::thread::scope(|scope| {
            let mut spawned = Vec::new();
            let mut mine = Vec::new();
            loop {
                // Another worker for the items left beside this thread's
                // next one, on a thread the pool has free now (any, outside
                // a run). A thread that cannot be started leaves its items
                // to the others.
                while spawned.len() + 1 < workers
                    && items.len().saturating_sub(next.load(Ordering::Relaxed)) > 1
                    && stop.load(Ordering::Relaxed) == usize::MAX
                {
                    let thread = match pool {
                        Some(pool) => match pool.try_take() {
                            Some(thread) => Some(thread),
                            None => break,
                        },
                        None => None,
                    };
                    let started = std::thread::Builder::new()
                        .name("axioval-subject".into())
                        .stack_size(WORKER_STACK)
                        .spawn_scoped(scope, move || {
                            let _pool = enter_pool(pool.clone());
                            let _budget = memory::enter(budget.clone());
                            let _store = derivation::enter(store.clone());
                            let _evaluation = evaluation::enter(evaluation.clone());
                            let _rule = RuleTable::enter_entries(rule.clone());
                            let done =
                                derivation::within(lineage.clone(), || timing::worker(timed, work));
                            // The pool's thread is given back as soon as
                            // the worker runs out of items.
                            drop(thread);
                            done
                        });
                    match started {
                        Ok(handle) => spawned.push(handle),
                        Err(_) => break,
                    }
                }
                if !take(&mut mine) {
                    break;
                }
            }
            for (index, answer) in mine {
                slots[index] = Some(answer);
            }
            for handle in spawned {
                match handle.join() {
                    Ok((done, account)) => {
                        timing::absorb(account);
                        for (index, answer) in done {
                            slots[index] = Some(answer);
                        }
                    }
                    // The worker's own bookkeeping cannot panic; should it,
                    // it is raised as it is.
                    Err(payload) => panic::resume_unwind(payload),
                }
            }
        });
        slots
            .into_iter()
            .map(|slot| match slot {
                Some(Ok(answer)) => answer,
                Some(Err(payload)) => panic::resume_unwind(payload),
                // Every item before the first that panicked is mapped, so
                // the loop raises that panic before reaching one not mapped.
                None => unreachable!("an item before the first panic is mapped"),
            })
            .collect()
    }
}

impl Default for Parallelism {
    fn default() -> Self {
        Self::SERIAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn threads(count: usize) -> Parallelism {
        Parallelism::new(NonZeroUsize::new(count).expect("nonzero"))
    }

    #[test]
    fn answers_in_the_items_order_whatever_the_threads() {
        let items: Vec<u64> = (0..500).collect();
        let serial = Parallelism::SERIAL.map_ordered(&items, |item| item * item);
        for count in [2, 3, 8, 64] {
            let parallel = threads(count).map_ordered(&items, |item| {
                // Later items finish first.
                std::thread::sleep(std::time::Duration::from_micros(500 - item));
                item * item
            });
            assert_eq!(parallel, serial);
        }
    }

    #[test]
    fn raises_the_first_panic_in_order_after_the_items_before_it() {
        let mapped = AtomicUsize::new(0);
        let raised = panic::catch_unwind(AssertUnwindSafe(|| {
            threads(4).map_ordered(&(0..64).collect::<Vec<usize>>(), |item| {
                assert!(*item != 40, "item 40");
                assert!(*item != 50, "item 50");
                mapped.fetch_add(1, Ordering::Relaxed);
                *item
            })
        }))
        .expect_err("the map panics");
        assert_eq!(raised.downcast_ref::<&str>(), Some(&"item 40"));
        // Every item before the first panic was mapped.
        assert!(mapped.load(Ordering::Relaxed) >= 40);
    }

    #[test]
    fn workers_host_calls_count_towards_the_rule() {
        let clock = timing::RuleClock::start();
        threads(4).map_ordered(&(0..16).collect::<Vec<u32>>(), |item| {
            timing::host("free-space", || *item)
        });
        let timing = clock.finish();
        let calls: Vec<(&str, u64)> = timing
            .services
            .iter()
            .map(|service| (service.service.as_str(), service.calls))
            .collect();
        assert_eq!(calls, [("free-space", 16)]);
    }

    #[test]
    fn workers_take_only_the_threads_the_pool_has_free() {
        use std::sync::atomic::AtomicUsize;
        // A pool of three, one held by the rule mapping and one by another
        // rule: at most two items at once, whatever the parallelism asks.
        let pool = Pool::new(3);
        let _pool = enter_pool(Some(pool.clone()));
        let _rule = pool.take();
        let other = pool.take();
        let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let items: Vec<u64> = (0..24).collect();
        let mapped = threads(8).map_ordered(&items, |item| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(2));
            running.fetch_sub(1, Ordering::SeqCst);
            item * 2
        });
        assert_eq!(
            mapped,
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(most.load(Ordering::SeqCst) <= 2, "{most:?}");
        // Every worker gave its thread back: with none free, this thread
        // maps every item itself.
        let last = pool.try_take().expect("the worker's thread is back");
        assert!(pool.try_take().is_none());
        let on = threads(8).map_ordered(&items, |_| std::thread::current().id());
        assert!(
            on.iter()
                .all(|thread| *thread == std::thread::current().id())
        );
        drop((last, other));
    }

    #[test]
    fn a_run_without_parallelism_is_serial() {
        assert_eq!(
            Parallelism::of(&ServiceRegistry::new()),
            Parallelism::SERIAL
        );
        assert!(Parallelism::available().threads() >= 1);
    }
}
