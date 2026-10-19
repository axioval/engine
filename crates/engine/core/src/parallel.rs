//! Evaluating a rule's subjects on several threads, its report exactly the
//! serial evaluation's.
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

use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ServiceRegistry;
use crate::{derivation, timing};

/// How many threads a run evaluates a rule's subjects on, the thread
/// running the rule among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Parallelism(NonZeroUsize);

/// The stack of each worker thread: a main thread's usual size, so a
/// subject recursing as deep as it may on the thread running the rule
/// does so on a worker too.
const WORKER_STACK: usize = 8 * 1024 * 1024;

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
    /// A panic in `map` is raised again here once every item before it
    /// is mapped: the panic of the first item in order that panicked, as
    /// a serial loop raises it. No item after a panicked one is started.
    ///
    /// The host calls the workers make count towards the rule this thread
    /// is timing, as they would on this thread.
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
        let work = || {
            let mut done = Vec::new();
            loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= items.len() || index > stop.load(Ordering::Relaxed) {
                    break;
                }
                let answer = panic::catch_unwind(AssertUnwindSafe(|| map(&items[index])));
                if answer.is_err() {
                    stop.fetch_min(index, Ordering::Relaxed);
                }
                done.push((index, answer));
            }
            done
        };
        let timed = timing::fanning_out();
        // Workers work within the derivations this thread is building, so
        // one reading a key this thread builds is a cycle, never a wait.
        let lineage = derivation::lineage();
        let mut slots: Vec<Option<std::thread::Result<R>>> =
            std::iter::repeat_with(|| None).take(items.len()).collect();
        std::thread::scope(|scope| {
            // A thread that cannot be started leaves its items to the others.
            let spawned: Vec<_> = (1..workers)
                .filter_map(|_| {
                    std::thread::Builder::new()
                        .name("axioval-subject".into())
                        .stack_size(WORKER_STACK)
                        .spawn_scoped(scope, || {
                            derivation::within(lineage.clone(), || timing::worker(timed, work))
                        })
                        .ok()
                })
                .collect();
            for (index, answer) in work() {
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
    fn a_run_without_parallelism_is_serial() {
        assert_eq!(
            Parallelism::of(&ServiceRegistry::new()),
            Parallelism::SERIAL
        );
        assert!(Parallelism::available().threads() >= 1);
    }
}
