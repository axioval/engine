//! Per-phase timings on stderr ([`Options::timings`](crate::Options::timings)).
//!
//! Each phase is printed as soon as it ends, so a run stopped by a time
//! limit still says where its time went. Timings never reach the result:
//! the result of a run with timings is the result of one without. The line
//! format is the one the `axioval` command line prints for its own phases.

use std::time::Instant;

/// Times consecutive phases; silent unless enabled.
#[derive(Debug)]
pub(crate) struct Stopwatch {
    /// When the current phase began; `None` when timings are off.
    last: Option<Instant>,
}

impl Stopwatch {
    /// A stopwatch started now, printing only when `on`.
    pub(crate) fn new(on: bool) -> Self {
        Self {
            last: on.then(Instant::now),
        }
    }

    /// Ends the phase `name` (begun at the previous lap) and starts the next.
    pub(crate) fn lap(&mut self, name: &str) {
        if let Some(last) = self.last.as_mut() {
            eprintln!("timing: {:>10.3} s {name}", last.elapsed().as_secs_f64());
            *last = Instant::now();
        }
    }
}
