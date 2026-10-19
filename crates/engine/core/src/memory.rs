//! A check's memory budget (#407): how much memory its heavy steps may be
//! admitted with at once, whatever the threads.
//!
//! A host sets one budget per check ([`crate::Runtime::with_memory_budget`])
//! or shares one between the checks it runs at once. The runtime enters it
//! on the thread running the plan, and [`crate::Parallelism::map_ordered`]
//! on each worker, so whatever a rule's subjects call reads it there
//! ([`current`]).
//!
//! A step that may take much memory (an adapter's overlay or morphology)
//! asks to be admitted with its estimate before it is built ([`admit`]),
//! and holds the [`Admission`] while it builds. While what is admitted
//! already would pass the budget with it, the step waits for another to
//! finish; with nothing else admitted, it runs alone. Waiting only ever
//! costs time: which steps run, and what they find, is what they would
//! find serially, so reports do not depend on the budget or the threads.
//!
//! The one refusal is a step whose own estimate is more than the whole
//! budget ([`OverBudget`]), which no waiting would admit. It depends on
//! that estimate and the budget alone, never on what else is running.
//!
//! Values kept on purpose between steps (a derivation store's, #440) are
//! counted with [`MemoryBudget::hold`]: never refused and never waiting,
//! they leave less room to the steps admitted beside them, and
//! [`MemoryBudget::headroom`] tells a keeper what it may still keep.
//!
//! A step admitted on a thread that already holds an admission is admitted
//! at once: it is part of the step that thread is running, and waiting
//! for room it holds itself would never end. An admission is held only
//! while its step builds, never across a lock or a wait for another
//! thread, so the steps waiting always wait for ones that finish.

use std::cell::RefCell;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::ThreadId;

/// How much memory a check's heavy steps may be admitted with at once.
#[derive(Debug)]
pub struct MemoryBudget {
    limit: u64,
    state: Mutex<State>,
    freed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    /// The estimates of the steps admitted now.
    admitted: u64,
    /// What is held on purpose between steps ([`MemoryBudget::hold`]).
    held: u64,
    /// The threads running admitted steps, each with how many it holds.
    running: Vec<(ThreadId, usize)>,
    /// The most admitted and held at once.
    most: u64,
    /// How many admissions waited for room.
    waited: u64,
    /// The largest estimate admitted.
    largest: u64,
}

/// A step refused because its estimate alone is more than the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverBudget {
    /// The step's estimate, in bytes.
    pub estimate: u64,
    /// The check's budget, in bytes.
    pub limit: u64,
}

impl fmt::Display for OverBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "its estimated {} is more than the check's memory budget of {}",
            Size(self.estimate),
            Size(self.limit)
        )
    }
}

impl std::error::Error for OverBudget {}

/// What a budget's use has been ([`MemoryBudget::usage`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryUsage {
    /// The budget, in bytes.
    pub limit: u64,
    /// The most admitted and held at once, in bytes.
    pub most: u64,
    /// How many admissions waited for room.
    pub waited: u64,
    /// The largest one step's estimate admitted, in bytes: a budget at
    /// least as large refuses none of the steps admitted.
    pub largest: u64,
}

/// One step's admission, released when dropped.
#[must_use = "a step is admitted only while its admission is held"]
#[derive(Debug)]
pub struct Admission {
    budget: Arc<MemoryBudget>,
    bytes: u64,
    thread: ThreadId,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.admitted = state.admitted.saturating_sub(self.bytes);
        if let Some(index) = state
            .running
            .iter()
            .position(|(thread, _)| *thread == self.thread)
        {
            state.running[index].1 -= 1;
            if state.running[index].1 == 0 {
                state.running.swap_remove(index);
            }
        }
        drop(state);
        self.budget.freed.notify_all();
    }
}

/// Memory held on purpose between steps, released when dropped.
#[must_use = "memory is counted only while it is held"]
#[derive(Debug)]
pub struct Held {
    budget: Arc<MemoryBudget>,
    bytes: u64,
}

impl Held {
    /// The bytes held.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.held = state.held.saturating_sub(self.bytes);
        drop(state);
        self.budget.freed.notify_all();
    }
}

impl MemoryBudget {
    /// A budget of `limit` bytes.
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            state: Mutex::new(State::default()),
            freed: Condvar::new(),
        }
    }

    /// The budget a host takes when it is told none: half the memory the
    /// system reports available ([`available_memory`]), and never less
    /// than [`MemoryBudget::FLOOR`]; `None` where the system does not
    /// say.
    #[must_use]
    pub fn from_available() -> Option<Self> {
        available_memory().map(|available| Self::new((available / 2).max(Self::FLOOR)))
    }

    /// The least budget [`Self::from_available`] gives: more than any one
    /// step the geometry adapters' own per-step budgets admit is
    /// estimated at, so a check under a default budget refuses no step
    /// for memory, on any machine.
    pub const FLOOR: u64 = 6 << 30;

    /// The budget, in bytes.
    #[must_use]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// What is admitted and held now, in bytes.
    #[must_use]
    pub fn in_use(&self) -> u64 {
        let state = self.lock();
        state.admitted.saturating_add(state.held)
    }

    /// What may still be admitted or held before the budget is reached.
    #[must_use]
    pub fn headroom(&self) -> u64 {
        self.limit.saturating_sub(self.in_use())
    }

    /// The most admitted and held at once so far, and how often a step
    /// waited.
    #[must_use]
    pub fn usage(&self) -> MemoryUsage {
        let state = self.lock();
        MemoryUsage {
            limit: self.limit,
            most: state.most,
            waited: state.waited,
            largest: state.largest,
        }
    }

    /// Whether a step estimated at `estimate` bytes is refused by this
    /// budget, whatever else is admitted: [`Self::admit`]'s refusal,
    /// without admitting it. A step taking a value kept from an earlier
    /// build asks this, so it is refused exactly as building it would be.
    ///
    /// # Errors
    ///
    /// [`OverBudget`] where `estimate` is more than the budget.
    pub fn check(&self, estimate: u64) -> Result<(), OverBudget> {
        if estimate > self.limit {
            return Err(OverBudget {
                estimate,
                limit: self.limit,
            });
        }
        Ok(())
    }

    /// Admits a step estimated at `estimate` bytes, waiting while the
    /// steps admitted already would pass the budget with it. A step is
    /// admitted at once when nothing else is, or when this thread holds
    /// an admission already.
    ///
    /// # Errors
    ///
    /// [`OverBudget`] where `estimate` alone is more than the budget.
    pub fn admit(self: &Arc<Self>, estimate: u64) -> Result<Admission, OverBudget> {
        self.check(estimate)?;
        let thread = std::thread::current().id();
        let mut state = self.lock();
        let mut waited = false;
        loop {
            let nested = state.running.iter().any(|(running, _)| *running == thread);
            let fits = state
                .admitted
                .saturating_add(state.held)
                .saturating_add(estimate)
                <= self.limit;
            if nested || fits || state.admitted == 0 {
                break;
            }
            waited = true;
            state = self
                .freed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.admitted = state.admitted.saturating_add(estimate);
        match state
            .running
            .iter_mut()
            .find(|(running, _)| *running == thread)
        {
            Some((_, count)) => *count += 1,
            None => state.running.push((thread, 1)),
        }
        state.waited += u64::from(waited);
        state.largest = state.largest.max(estimate);
        state.most = state.most.max(state.admitted.saturating_add(state.held));
        Ok(Admission {
            budget: Arc::clone(self),
            bytes: estimate,
            thread,
        })
    }

    /// Counts `bytes` held on purpose between steps until the [`Held`] is
    /// dropped: never refused and never waiting, they leave less room to
    /// the steps admitted beside them.
    pub fn hold(self: &Arc<Self>, bytes: u64) -> Held {
        let mut state = self.lock();
        state.held = state.held.saturating_add(bytes);
        state.most = state.most.max(state.admitted.saturating_add(state.held));
        Held {
            budget: Arc::clone(self),
            bytes,
        }
    }

    /// Makes this the budget of the calling thread ([`current`]) until the
    /// returned guard is dropped, which puts back the one it replaced.
    pub fn enter(self: &Arc<Self>) -> Entered {
        enter(Some(Arc::clone(self)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<MemoryBudget>>> = const { RefCell::new(None) };
}

/// The budget of the check this thread is evaluating, if it has one.
#[must_use]
pub fn current() -> Option<Arc<MemoryBudget>> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Admits a step estimated at `estimate` bytes against the budget of the
/// check this thread is evaluating ([`MemoryBudget::admit`]); without one,
/// at once (`None`).
///
/// # Errors
///
/// [`OverBudget`] where `estimate` alone is more than the budget.
pub fn admit(estimate: u64) -> Result<Option<Admission>, OverBudget> {
    current().map(|budget| budget.admit(estimate)).transpose()
}

/// [`MemoryBudget::check`] against the budget of the check this thread is
/// evaluating; without one, nothing is refused.
///
/// # Errors
///
/// [`OverBudget`] where `estimate` is more than the budget.
pub fn check(estimate: u64) -> Result<(), OverBudget> {
    current().map_or(Ok(()), |budget| budget.check(estimate))
}

/// The calling thread's budget while held ([`MemoryBudget::enter`]); the
/// one it replaced is put back when dropped.
#[must_use = "the budget is entered only while this is held"]
#[derive(Debug)]
pub struct Entered {
    outer: Option<Arc<MemoryBudget>>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let outer = self.outer.take();
        CURRENT.with(|current| *current.borrow_mut() = outer);
    }
}

/// Makes `budget` the calling thread's ([`current`]) until the guard is
/// dropped; `None` makes it have none.
pub(crate) fn enter(budget: Option<Arc<MemoryBudget>>) -> Entered {
    let outer = CURRENT.with(|current| std::mem::replace(&mut *current.borrow_mut(), budget));
    Entered { outer }
}

/// The memory the system reports available to start new work without
/// swapping, in bytes: `MemAvailable` of `/proc/meminfo` on Linux, `None`
/// elsewhere or where it cannot be read.
#[must_use]
pub fn available_memory() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo.lines().find_map(|line| {
        let kib = line
            .strip_prefix("MemAvailable:")?
            .trim()
            .strip_suffix("kB")?;
        kib.trim().parse::<u64>().ok()?.checked_mul(1024)
    })
}

/// Reads a size as a host states one: a whole number of bytes, or one
/// with a binary unit `K`, `M`, `G` or `T` (`KiB` … `TiB` and `KB` …
/// `TB` read alike, as powers of 1024), such as `8G` or `512MiB`.
///
/// # Errors
///
/// A text that is no such size, or one past `u64::MAX` bytes.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let refused = || format!("`{text}` is not a size such as 8G, 512M or 1073741824");
    let number: u64 = number.parse().map_err(|_| refused())?;
    let shift = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 0,
        "K" | "KB" | "KIB" => 10,
        "M" | "MB" | "MIB" => 20,
        "G" | "GB" | "GIB" => 30,
        "T" | "TB" | "TIB" => 40,
        _ => return Err(refused()),
    };
    number
        .checked_mul(1 << shift)
        .ok_or_else(|| format!("`{text}` is more bytes than can be counted"))
}

/// A byte count as a host reads it: in GiB, MiB or KiB, to one decimal.
#[derive(Debug, Clone, Copy)]
pub struct Size(pub u64);

impl fmt::Display for Size {
    #[allow(clippy::cast_precision_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0 as f64;
        for (shift, unit) in [(30, "GiB"), (20, "MiB"), (10, "KiB")] {
            let scale = f64::from(1_u32 << shift);
            if bytes >= scale {
                return write!(f, "{:.1} {unit}", bytes / scale);
            }
        }
        write!(f, "{} bytes", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[test]
    fn a_step_past_the_whole_budget_is_refused_whatever_runs() {
        let budget = Arc::new(MemoryBudget::new(1000));
        let refused = budget.admit(1001).unwrap_err();
        assert_eq!(
            refused,
            OverBudget {
                estimate: 1001,
                limit: 1000
            }
        );
        assert_eq!(
            refused.to_string(),
            "its estimated 1001 bytes is more than the check's memory budget of 1000 bytes"
        );
        assert_eq!(budget.check(1001), Err(refused));
        // The same step with others running is refused alike, and one
        // within the budget is not, whatever is held.
        let _held = budget.hold(900);
        let _running = budget.admit(10).unwrap();
        assert_eq!(budget.admit(1001).unwrap_err(), refused);
        assert!(budget.check(1000).is_ok());
    }

    #[test]
    fn steps_wait_for_room_and_never_run_past_the_budget_together() {
        let budget = Arc::new(MemoryBudget::new(100));
        let admitted = AtomicU64::new(0);
        let most = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..10 {
                        let _step = budget.admit(40).unwrap();
                        let now = admitted.fetch_add(40, Ordering::SeqCst) + 40;
                        most.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_micros(200));
                        admitted.fetch_sub(40, Ordering::SeqCst);
                    }
                });
            }
        });
        // Two steps of 40 fit in 100, a third does not.
        assert!(most.load(Ordering::SeqCst) <= 80);
        let usage = budget.usage();
        assert!(usage.most <= 80, "{usage:?}");
        assert!(usage.waited > 0, "{usage:?}");
        assert_eq!(usage.largest, 40);
        assert_eq!(budget.in_use(), 0);
    }

    #[test]
    fn a_step_runs_alone_when_nothing_else_is_admitted() {
        let budget = Arc::new(MemoryBudget::new(100));
        // Held values past the budget never stop a step running alone.
        let held = budget.hold(150);
        assert_eq!(budget.headroom(), 0);
        let step = budget.admit(100).unwrap();
        assert_eq!(budget.in_use(), 250);
        drop((step, held));
        assert_eq!(budget.in_use(), 0);
        assert_eq!(budget.headroom(), 100);
    }

    #[test]
    fn a_thread_holding_an_admission_is_admitted_at_once() {
        let budget = Arc::new(MemoryBudget::new(100));
        let outer = budget.admit(80).unwrap();
        // Nested on the same thread: no wait, though it does not fit.
        let inner = budget.admit(80).unwrap();
        assert_eq!(budget.in_use(), 160);
        drop(inner);
        drop(outer);
        assert_eq!(budget.in_use(), 0);
    }

    #[test]
    fn a_waiting_step_is_admitted_once_room_is_freed() {
        let budget = Arc::new(MemoryBudget::new(100));
        let first = budget.admit(60).unwrap();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let _second = budget.admit(60).unwrap();
                budget.in_use()
            });
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(budget.in_use(), 60, "the second step waits");
            drop(first);
            assert_eq!(waiter.join().unwrap(), 60);
        });
        assert_eq!(budget.usage().waited, 1);
    }

    #[test]
    fn the_entered_budget_is_the_threads_until_released() {
        assert!(current().is_none());
        assert!(admit(u64::MAX).unwrap().is_none(), "no budget, no refusal");
        let budget = Arc::new(MemoryBudget::new(10));
        {
            let _entered = budget.enter();
            assert!(Arc::ptr_eq(&current().unwrap(), &budget));
            assert!(admit(11).is_err());
            assert!(check(11).is_err());
            let inner = Arc::new(MemoryBudget::new(20));
            {
                let _inner = inner.enter();
                assert_eq!(current().unwrap().limit(), 20);
            }
            assert_eq!(current().unwrap().limit(), 10);
            let _none = enter(None);
            assert!(current().is_none());
        }
        assert!(current().is_none());
    }

    #[test]
    fn sizes_read_as_hosts_state_them() {
        assert_eq!(parse_size("8G"), Ok(8 << 30));
        assert_eq!(parse_size("512MiB"), Ok(512 << 20));
        assert_eq!(parse_size(" 3 kb "), Ok(3 << 10));
        assert_eq!(parse_size("1073741824"), Ok(1 << 30));
        assert_eq!(parse_size("2T"), Ok(2 << 40));
        assert!(parse_size("eight").is_err());
        assert!(parse_size("8X").is_err());
        assert!(parse_size("-1G").is_err());
        assert!(parse_size("99999999999T").is_err());
        assert_eq!(Size(8 << 30).to_string(), "8.0 GiB");
        assert_eq!(Size(1536 << 10).to_string(), "1.5 MiB");
        assert_eq!(Size(12).to_string(), "12 bytes");
    }

    #[test]
    fn the_default_is_never_below_the_floor() {
        if let Some(budget) = MemoryBudget::from_available() {
            assert!(budget.limit() >= MemoryBudget::FLOOR);
        }
    }
}
