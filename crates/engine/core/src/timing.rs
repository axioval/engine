//! Per-rule timings (`Runtime::with_rule_timings`).
//!
//! The runtime starts a [`RuleClock`] around each rule it evaluates. Host
//! service handles route every call into their host through [`host`], which
//! adds the call's wall time to the running clock under the service's
//! name. Without a running clock `host` only calls through, so a run without
//! timings reads no clock at all.
//!
//! The clock is per thread: a rule's capability calls its services on the
//! thread the runtime evaluates it on. A host call made from inside another
//! (one service answering through another's handle) counts with the outer
//! call only, so no time is counted twice.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    time::{Duration, Instant},
};

use axioval_ir::{RuleTiming, ServiceTiming};

/// What one rule's host calls have cost so far.
#[derive(Default)]
struct Accumulator {
    /// How many host calls are open on this thread.
    depth: u32,
    services: BTreeMap<&'static str, (u64, Duration)>,
}

thread_local! {
    static CLOCK: RefCell<Option<Accumulator>> = const { RefCell::new(None) };
}

/// Closes one open host call when dropped, on every path out of it.
struct Open;

impl Drop for Open {
    fn drop(&mut self) {
        CLOCK.with(|clock| {
            if let Some(accumulator) = clock.borrow_mut().as_mut() {
                accumulator.depth = accumulator.depth.saturating_sub(1);
            }
        });
    }
}

/// Calls `call`, a call into the host's `service`, and counts its wall time
/// against the rule being timed, if any. Never changes what `call` returns.
pub(crate) fn host<T>(service: &'static str, call: impl FnOnce() -> T) -> T {
    let outermost = CLOCK.with(|clock| {
        clock.borrow_mut().as_mut().map(|accumulator| {
            accumulator.depth += 1;
            accumulator.depth == 1
        })
    });
    let Some(outermost) = outermost else {
        return call();
    };
    let _open = Open;
    if !outermost {
        return call();
    }
    let started = Instant::now();
    let answer = call();
    let elapsed = started.elapsed();
    CLOCK.with(|clock| {
        if let Some(accumulator) = clock.borrow_mut().as_mut() {
            let entry = accumulator.services.entry(service).or_default();
            entry.0 += 1;
            entry.1 += elapsed;
        }
    });
    answer
}

/// The clock of one rule, running from [`RuleClock::start`] until
/// [`RuleClock::finish`] or until it is dropped.
pub(crate) struct RuleClock {
    started: Instant,
    /// The clock this one interrupted, restored when it stops.
    outer: Option<Accumulator>,
    /// Whether the clock has stopped.
    stopped: bool,
}

impl RuleClock {
    /// Starts timing a rule on this thread.
    pub(crate) fn start() -> Self {
        let outer = CLOCK.with(|clock| clock.borrow_mut().replace(Accumulator::default()));
        Self {
            started: Instant::now(),
            outer,
            stopped: false,
        }
    }

    /// Stops the clock and returns the rule's timing.
    pub(crate) fn finish(mut self) -> RuleTiming {
        let wall = self.started.elapsed();
        let accumulator = self.stop().unwrap_or_default();
        let host: Duration = accumulator.services.values().map(|(_, time)| *time).sum();
        RuleTiming {
            wall_seconds: wall.as_secs_f64(),
            host_seconds: host.min(wall).as_secs_f64(),
            services: accumulator
                .services
                .into_iter()
                .map(|(service, (calls, time))| ServiceTiming {
                    service: service.to_owned(),
                    calls,
                    seconds: time.as_secs_f64(),
                })
                .collect(),
        }
    }

    /// Puts the interrupted clock back and returns this one's account;
    /// `None` when it was already stopped.
    fn stop(&mut self) -> Option<Accumulator> {
        if std::mem::replace(&mut self.stopped, true) {
            return None;
        }
        let outer = self.outer.take();
        CLOCK.with(|clock| std::mem::replace(&mut *clock.borrow_mut(), outer))
    }
}

impl Drop for RuleClock {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_clock_a_host_call_only_calls_through() {
        assert_eq!(host("proximity", || 7), 7);
        CLOCK.with(|clock| assert!(clock.borrow().is_none()));
    }

    #[test]
    fn a_clock_counts_outermost_host_calls_per_service() {
        let clock = RuleClock::start();
        host("proximity", || host("plan-area", || ()));
        host("proximity", || ());
        host("metric-routing", || ());
        let timing = clock.finish();
        let calls: Vec<(&str, u64)> = timing
            .services
            .iter()
            .map(|service| (service.service.as_str(), service.calls))
            .collect();
        assert_eq!(calls, [("metric-routing", 1), ("proximity", 2)]);
        assert!(timing.host_seconds <= timing.wall_seconds);
        CLOCK.with(|clock| assert!(clock.borrow().is_none()));
    }

    #[test]
    fn a_dropped_clock_stops() {
        drop(RuleClock::start());
        CLOCK.with(|clock| assert!(clock.borrow().is_none()));
    }
}
