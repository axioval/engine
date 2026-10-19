//! Per-phase timings on stderr (`check --timings`).
//!
//! Each phase is printed as soon as it ends, so a run stopped by a time
//! limit still says where its time went. Timings never reach the result:
//! the result of a run with `--timings` is the result of one without. How
//! the run reused its derivations is `--stats`' ([`crate::stats`]).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axioval::engine::memory::Size;
use axioval::engine::{MemoryBudget, RuleObserver, RuleProgress};

/// Times consecutive phases; silent unless enabled.
#[derive(Debug)]
pub struct Stopwatch {
    /// When the stopwatch and the current phase began; `None` when timings
    /// are off.
    times: Option<(Instant, Instant)>,
}

impl Stopwatch {
    /// A stopwatch started now, printing only when `on`.
    pub fn new(on: bool) -> Self {
        Self {
            times: on.then(|| (Instant::now(), Instant::now())),
        }
    }

    /// Whether timings are printed.
    pub fn is_on(&self) -> bool {
        self.times.is_some()
    }

    /// Ends the phase `name` (begun at the previous lap) and starts the next.
    pub fn lap(&mut self, name: &str) {
        if let Some((_, last)) = self.times.as_mut() {
            print_phase(name, last.elapsed().as_secs_f64());
            *last = Instant::now();
        }
    }

    /// Ends the last phase, `name`, and prints the time since the start.
    pub fn finish(&mut self, name: &str) {
        self.lap(name);
        if let Some((start, _)) = self.times {
            print_phase("total", start.elapsed().as_secs_f64());
        }
    }

    /// Restarts the current phase without printing, after time another
    /// clock has already accounted for.
    pub fn skip(&mut self) {
        if let Some((_, last)) = self.times.as_mut() {
            *last = Instant::now();
        }
    }

    /// Prints how the check's memory budget was used, if timings are on
    /// and there is one: `memory: budget …, at most … admitted at once, the
    /// largest step …, N steps waited, … kept released for them`.
    pub fn memory(&self, budget: Option<&MemoryBudget>) {
        if let (true, Some(budget)) = (self.is_on(), budget) {
            let usage = budget.usage();
            eprintln!(
                "memory: budget {}, at most {} admitted at once, the largest step {}, {} steps \
                 waited, {} kept released for them",
                Size(usage.limit),
                Size(usage.most),
                Size(usage.largest),
                usage.waited,
                Size(usage.reclaimed)
            );
        }
    }

    /// A rule observer printing each rule's time as `rule <id>`, or `None`
    /// when timings are off.
    pub fn rules(&self) -> Option<RuleObserver> {
        self.is_on().then(|| {
            let started: Mutex<Option<Instant>> = Mutex::new(None);
            let observer: RuleObserver = Arc::new(move |rule, progress| {
                let mut started = started
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match progress {
                    RuleProgress::Started => *started = Some(Instant::now()),
                    RuleProgress::Finished => {
                        if let Some(start) = started.take() {
                            print_phase(&format!("rule {rule}"), start.elapsed().as_secs_f64());
                        }
                    }
                }
            });
            observer
        })
    }
}

/// One line per phase: `timing: <seconds> s <phase>`.
fn print_phase(name: &str, seconds: f64) {
    eprintln!("timing: {seconds:>10.3} s {name}");
}
