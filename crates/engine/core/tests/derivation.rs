//! The derivation store: each key built once, re-entrant builds, panics
//! and cycles (ADR 0006, #437).
#![allow(missing_docs)]

use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use axioval_engine::{
    DerivationError, DerivationKind, DerivationStore, DerivationVersion, Inputs, MeasuredMemo,
    MemoryBudget, Parallelism, ServiceRegistry,
};

/// `work` on a thread of its own, failing the test should it not end: a
/// store that hangs fails rather than stalls the suite.
fn ends<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> R {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let answer = panic::catch_unwind(AssertUnwindSafe(work));
        let _ = sender.send(answer);
    });
    match receiver.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(answer)) => answer,
        Ok(Err(payload)) => panic::resume_unwind(payload),
        Err(error) => panic!("the store hung: {error}"),
    }
}

static SLOW_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A key's square, slowly, counting its builds.
struct SlowSquare;

impl DerivationKind for SlowSquare {
    const NAME: &'static str = "slow-square";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u64;
    type Value = u64;

    fn build(key: &u64, _: &Inputs<'_>) -> Result<u64, DerivationError> {
        SLOW_BUILDS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        Ok(key * key)
    }

    fn size_hint(_: &u64) -> Option<usize> {
        Some(8)
    }
}

#[test]
fn many_threads_asking_for_one_key_build_it_once() {
    let store = DerivationStore::new();
    let threads = 16;
    let start = Arc::new(Barrier::new(threads));
    let answers = ends({
        let store = store.clone();
        move || {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let store = store.clone();
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        *store
                            .derive::<SlowSquare>(&ServiceRegistry::new(), &7)
                            .expect("no cycle")
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("no panic"))
                .collect::<Vec<u64>>()
        }
    });
    assert_eq!(answers, vec![49; threads]);
    assert_eq!(SLOW_BUILDS.load(Ordering::SeqCst), 1);
    let statistics = store.statistics();
    assert_eq!(statistics.len(), 1);
    let square = &statistics[0];
    assert_eq!(square.kind, "slow-square");
    assert_eq!(square.entries, 1);
    assert_eq!(square.builds, 1);
    assert_eq!(square.hits, threads as u64 - 1);
    assert_eq!(square.bytes, 8);
    // The reuse report (#442): the kind's version, the time its one build
    // took (it sleeps 50 ms), and the most it held.
    assert_eq!(square.version, Some(DerivationVersion::new(1, &[])));
    assert!(square.build_time >= Duration::from_millis(50));
    assert_eq!(square.peak_bytes, 8);
    // Released, a key is built again, to the same value.
    assert!(store.release::<SlowSquare>(&7));
    assert!(store.held::<SlowSquare>(&7).is_none());
    assert_eq!(
        *store
            .derive::<SlowSquare>(&ServiceRegistry::new(), &7)
            .expect("no cycle"),
        49
    );
    assert_eq!(SLOW_BUILDS.load(Ordering::SeqCst), 2);
}

static BASE_BUILDS: AtomicUsize = AtomicUsize::new(0);

struct Base;

impl DerivationKind for Base {
    const NAME: &'static str = "base";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u32;
    type Value = u32;

    fn build(key: &u32, _: &Inputs<'_>) -> Result<u32, DerivationError> {
        BASE_BUILDS.fetch_add(1, Ordering::SeqCst);
        Ok(key + 1)
    }
}

/// The sum of the bases up to its key: a build reading other derivations.
struct Sum;

impl DerivationKind for Sum {
    const NAME: &'static str = "sum";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u32;
    type Value = u32;

    fn build(key: &u32, inputs: &Inputs<'_>) -> Result<u32, DerivationError> {
        let mut sum = 0;
        for part in 0..=*key {
            sum += *inputs.derive::<Base>(&part)?;
            // A key read twice within one build is built once.
            sum -= *inputs.derive::<Base>(&part)?;
            sum += *inputs.derive::<Base>(&part)?;
        }
        if *key > 0 {
            // And a build may read its own kind's other keys.
            let below = *inputs.derive::<Sum>(&(key - 1))?;
            assert_eq!(below + key + 1, sum);
        }
        Ok(sum)
    }
}

#[test]
fn builds_read_other_derivations_each_built_once() {
    let mut services = ServiceRegistry::new();
    let store = DerivationStore::new();
    services.register(store.clone()).expect("one store");
    let sum = ends(move || *DerivationStore::of::<Sum>(&services, &4).expect("no cycle"));
    assert_eq!(sum, 1 + 2 + 3 + 4 + 5);
    assert_eq!(BASE_BUILDS.load(Ordering::SeqCst), 5);
    let kinds: Vec<(&str, u64)> = store
        .statistics()
        .iter()
        .map(|kind| (kind.kind, kind.builds))
        .collect();
    assert_eq!(kinds, [("base", 5), ("sum", 5)]);
}

static FLAKY_BUILDS: AtomicUsize = AtomicUsize::new(0);
static FLAKY_START: std::sync::OnceLock<Barrier> = std::sync::OnceLock::new();

/// Panics on its first build, once every caller is waiting for it.
struct Flaky;

impl DerivationKind for Flaky {
    const NAME: &'static str = "flaky";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = ();
    type Value = &'static str;

    fn build((): &(), _: &Inputs<'_>) -> Result<&'static str, DerivationError> {
        if FLAKY_BUILDS.fetch_add(1, Ordering::SeqCst) == 0 {
            FLAKY_START.get().expect("set").wait();
            // Let the others reach their wait.
            std::thread::sleep(Duration::from_millis(100));
            panic!("the first build fails");
        }
        Ok("built")
    }
}

#[test]
fn a_panicking_build_leaves_no_waiter_hanging() {
    let threads = 8;
    FLAKY_START.get_or_init(|| Barrier::new(threads));
    let store = DerivationStore::new();
    let answers = ends(move || {
        let first = {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .derive::<Flaky>(&ServiceRegistry::new(), &())
                    .map(|value| *value)
            })
        };
        let others: Vec<_> = (1..threads)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    // Ask once the first build is surely in flight.
                    FLAKY_START.get().expect("set").wait();
                    store
                        .derive::<Flaky>(&ServiceRegistry::new(), &())
                        .map(|value| *value)
                })
            })
            .collect();
        let first = first.join();
        let others: Vec<_> = others
            .into_iter()
            .map(|handle| handle.join().expect("a waiter does not panic"))
            .collect();
        (
            first.is_err(),
            others,
            store.held::<Flaky>(&()).map(|value| *value),
        )
    });
    let (first_panicked, others, held) = answers;
    assert!(first_panicked, "the panic reaches the caller that built");
    assert!(others.iter().all(|answer| answer == &Ok("built")));
    assert_eq!(held, Some("built"));
    // The panic held nothing; the key was built once more, not per waiter.
    assert_eq!(FLAKY_BUILDS.load(Ordering::SeqCst), 2);
}

/// A key that needs itself.
struct Selfish;

impl DerivationKind for Selfish {
    const NAME: &'static str = "selfish";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u8;
    type Value = u8;

    fn build(key: &u8, inputs: &Inputs<'_>) -> Result<u8, DerivationError> {
        Ok(*inputs.derive::<Selfish>(key)? + 1)
    }
}

#[test]
fn a_build_reading_its_own_key_is_a_cycle() {
    let answer = ends(|| DerivationStore::of::<Selfish>(&ServiceRegistry::new(), &1));
    assert_eq!(answer, Err(DerivationError::Cycle { kind: "selfish" }));
    assert_eq!(
        answer.expect_err("a cycle").to_string(),
        "derivation cycle: selfish is read while it is being built"
    );
}

static CROSS_START: std::sync::OnceLock<Barrier> = std::sync::OnceLock::new();
static LEFT_FIRST: AtomicBool = AtomicBool::new(true);
static RIGHT_FIRST: AtomicBool = AtomicBool::new(true);

/// `Left` needs `Right` and `Right` needs `Left`, each built on its own
/// thread once both are in flight.
struct Left;
struct Right;

impl DerivationKind for Left {
    const NAME: &'static str = "left";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = ();
    type Value = ();

    fn build((): &(), inputs: &Inputs<'_>) -> Result<(), DerivationError> {
        if LEFT_FIRST.swap(false, Ordering::SeqCst) {
            CROSS_START.get().expect("set").wait();
        }
        inputs.derive::<Right>(&()).map(|_| ())
    }
}

impl DerivationKind for Right {
    const NAME: &'static str = "right";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = ();
    type Value = ();

    fn build((): &(), inputs: &Inputs<'_>) -> Result<(), DerivationError> {
        if RIGHT_FIRST.swap(false, Ordering::SeqCst) {
            CROSS_START.get().expect("set").wait();
        }
        std::thread::sleep(Duration::from_millis(50));
        inputs.derive::<Left>(&()).map(|_| ())
    }
}

#[test]
fn a_cycle_through_two_threads_is_reported_not_waited_on() {
    CROSS_START.get_or_init(|| Barrier::new(2));
    let store = DerivationStore::new();
    let (left, right) = ends(move || {
        let left = {
            let store = store.clone();
            std::thread::spawn(move || store.derive::<Left>(&ServiceRegistry::new(), &()))
        };
        let right = std::thread::spawn(move || store.derive::<Right>(&ServiceRegistry::new(), &()));
        (
            left.join().expect("no panic").map(|_| ()),
            right.join().expect("no panic").map(|_| ()),
        )
    });
    assert!(matches!(left, Err(DerivationError::Cycle { .. })));
    assert!(matches!(right, Err(DerivationError::Cycle { .. })));
}

/// A key whose build asks for itself on the workers of a parallel map.
struct Fanned;

impl DerivationKind for Fanned {
    const NAME: &'static str = "fanned";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = ();
    type Value = Vec<bool>;

    fn build((): &(), inputs: &Inputs<'_>) -> Result<Vec<bool>, DerivationError> {
        let threads = Parallelism::new(NonZeroUsize::new(4).expect("nonzero"));
        Ok(threads.map_ordered(&[0_u8; 16], |_| {
            matches!(
                inputs.derive::<Fanned>(&()),
                Err(DerivationError::Cycle { .. })
            )
        }))
    }
}

#[test]
fn a_worker_reading_the_key_its_thread_builds_is_a_cycle() {
    let cycles = ends(|| {
        DerivationStore::of::<Fanned>(&ServiceRegistry::new(), &())
            .expect("the outer build ends")
            .to_vec()
    });
    assert_eq!(cycles, vec![true; 16]);
}

static FALLIBLE_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A derivation whose own failure is its value.
struct Fallible;

impl DerivationKind for Fallible {
    const NAME: &'static str = "fallible";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u8;
    type Value = Result<u8, String>;

    fn build(key: &u8, _: &Inputs<'_>) -> Result<Result<u8, String>, DerivationError> {
        FALLIBLE_BUILDS.fetch_add(1, Ordering::SeqCst);
        Ok(Err(format!("no value for {key}")))
    }
}

#[test]
fn a_failure_of_the_derivation_is_kept_and_shared_like_any_value() {
    let store = DerivationStore::new();
    let services = ServiceRegistry::new();
    for _ in 0..3 {
        let value = store.derive::<Fallible>(&services, &3).expect("no cycle");
        assert_eq!(*value, Err("no value for 3".to_owned()));
    }
    assert_eq!(FALLIBLE_BUILDS.load(Ordering::SeqCst), 1);
}

#[test]
fn the_memo_measures_a_key_once_however_many_threads_ask() {
    let memo = MeasuredMemo::default();
    let measured = Arc::new(AtomicUsize::new(0));
    let threads = 12;
    let start = Arc::new(Barrier::new(threads));
    let answers = ends({
        let memo = memo.clone();
        let measured = Arc::clone(&measured);
        move || {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let memo = memo.clone();
                    let measured = Arc::clone(&measured);
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        memo.get_or_measure(("wall", 1_u8), || {
                            measured.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(50));
                            // A measurement may read the memo itself.
                            memo.get_or_measure(("slab", 2_u16), || 2_u32) + 40
                        })
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("no panic"))
                .collect::<Vec<u32>>()
        }
    });
    assert_eq!(answers, vec![42; threads]);
    assert_eq!(measured.load(Ordering::SeqCst), 1);
    // The memo's tables are kinds of the store it is a view of.
    assert_eq!(memo.store().statistics().len(), 2);
}

#[test]
#[should_panic(expected = "derivation cycle")]
fn a_memo_measurement_needing_itself_panics_rather_than_hangs() {
    let memo = MeasuredMemo::default();
    let inner = memo.clone();
    memo.get_or_measure(1_u8, || inner.get_or_measure(1_u8, || 1_u8));
}

static FORGOTTEN_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A key's double, slowly, counting its builds.
struct SlowDouble;

impl DerivationKind for SlowDouble {
    const NAME: &'static str = "slow-double";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u64;
    type Value = u64;

    fn build(key: &u64, _: &Inputs<'_>) -> Result<u64, DerivationError> {
        FORGOTTEN_BUILDS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        Ok(key * 2)
    }
}

/// A store that keeps nothing (#441) answers every read as a keeping one
/// does, building each: callers waiting for a build build the key again
/// rather than read a value never kept, and neither the memo's shared nor
/// its per-rule tables hold anything.
#[test]
fn a_forgetful_store_builds_every_read_and_keeps_nothing() {
    let store = DerivationStore::forgetful();
    assert!(!store.keeps() && DerivationStore::new().keeps());
    let threads = 4;
    let start = Arc::new(Barrier::new(threads));
    let answers = ends({
        let store = store.clone();
        move || {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let store = store.clone();
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        *store
                            .derive::<SlowDouble>(&ServiceRegistry::new(), &21)
                            .expect("no cycle")
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("no panic"))
                .collect::<Vec<u64>>()
        }
    });
    assert_eq!(answers, vec![42; threads]);
    assert_eq!(FORGOTTEN_BUILDS.load(Ordering::SeqCst), threads);
    assert!(store.held::<SlowDouble>(&21).is_none());

    let memo = MeasuredMemo::on(store);
    let measured = AtomicUsize::new(0);
    for _ in 0..3 {
        memo.get_or_measure("wall", || measured.fetch_add(1, Ordering::SeqCst));
        memo.get_or_measure_for_rule("wall", || measured.fetch_add(1, Ordering::SeqCst));
    }
    assert_eq!(measured.load(Ordering::SeqCst), 6);
    memo.insert("door", 1_u32);
    memo.update("door", |value: &mut u32| *value += 1);
    assert_eq!(memo.get::<&str, u32>(&"door"), None);
}

static KILOS_BUILT: AtomicUsize = AtomicUsize::new(0);

/// A kilobyte value per key, of which the store keeps at most three.
struct Kilo;

impl DerivationKind for Kilo {
    const NAME: &'static str = "kilo";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u64;
    type Value = u64;

    fn build(key: &u64, _: &Inputs<'_>) -> Result<u64, DerivationError> {
        KILOS_BUILT.fetch_add(1, Ordering::SeqCst);
        Ok(*key)
    }

    fn size_hint(_: &u64) -> Option<usize> {
        Some(1024)
    }

    fn keep_bytes() -> Option<u64> {
        Some(3 * 1024)
    }
}

#[test]
fn a_kind_past_its_bound_releases_the_values_read_least_recently() {
    let store = DerivationStore::new();
    let services = ServiceRegistry::new();
    let read = |key: u64| *store.derive::<Kilo>(&services, &key).expect("no cycle");
    for key in 1..=3 {
        assert_eq!(read(key), key);
    }
    // Reading 1 again makes 2 the value read least recently.
    assert_eq!(read(1), 1);
    assert_eq!(read(4), 4);
    assert!(store.held::<Kilo>(&2).is_none());
    for key in [1, 3, 4] {
        assert!(store.held::<Kilo>(&key).is_some(), "{key} is kept");
    }
    let kilo = store
        .statistics()
        .into_iter()
        .find(|kind| kind.kind == "kilo")
        .expect("counted");
    assert_eq!(
        (kilo.builds, kilo.hits, kilo.evicted, kilo.released),
        (4, 1, 1, 0)
    );
    assert_eq!((kilo.entries, kilo.bytes), (3, 3 * 1024));
    // The fourth value was held beside the three before one was evicted.
    assert_eq!(kilo.peak_bytes, 4 * 1024);
    // A released value is built again, to the same value.
    assert_eq!(read(2), 2);
    assert_eq!(KILOS_BUILT.load(Ordering::SeqCst), 5);
}

/// An eight-byte value per key.
struct Eight;

impl DerivationKind for Eight {
    const NAME: &'static str = "eight";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u64;
    type Value = u64;

    fn build(key: &u64, _: &Inputs<'_>) -> Result<u64, DerivationError> {
        Ok(*key)
    }

    fn size_hint(_: &u64) -> Option<usize> {
        Some(8)
    }
}

#[test]
fn kept_values_are_held_against_the_check_memory_budget_until_released() {
    let budget = Arc::new(MemoryBudget::new(1 << 20));
    let _entered = budget.enter();
    let store = DerivationStore::new();
    let services = ServiceRegistry::new();
    store.derive::<Eight>(&services, &3).expect("no cycle");
    assert_eq!(budget.in_use(), 8);
    // A value grown after it was built states its bytes again.
    assert!(store.resize::<Eight>(&3, 100));
    assert_eq!(budget.in_use(), 100);
    let square = store
        .statistics()
        .into_iter()
        .find(|kind| kind.kind == "eight")
        .expect("counted");
    assert_eq!(square.bytes, 100);
    assert!(store.release::<Eight>(&3));
    assert_eq!(budget.in_use(), 0);
    assert!(!store.resize::<Eight>(&3, 100));
    let eight = store
        .statistics()
        .into_iter()
        .find(|kind| kind.kind == "eight")
        .expect("counted");
    // Released by name, not evicted; the peak outlives the value.
    assert_eq!(
        (eight.released, eight.evicted, eight.bytes, eight.peak_bytes),
        (1, 0, 0, 100)
    );
}

static TRIPLES_BUILT: AtomicUsize = AtomicUsize::new(0);

/// A key's triple, counting its builds.
struct Triple;

impl DerivationKind for Triple {
    const NAME: &'static str = "triple";
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = u64;
    type Value = u64;

    fn build(key: &u64, _: &Inputs<'_>) -> Result<u64, DerivationError> {
        TRIPLES_BUILT.fetch_add(1, Ordering::SeqCst);
        Ok(key * 3)
    }
}

#[test]
fn an_adapter_derives_through_the_current_store_on_every_worker() {
    // Outside a check there is no current store: every read is built.
    assert!(axioval_engine::derivation::current().is_none());
    for _ in 0..2 {
        assert_eq!(
            *DerivationStore::derive_current::<Triple>(&1).expect("no cycle"),
            3
        );
    }
    assert_eq!(TRIPLES_BUILT.load(Ordering::SeqCst), 2);

    let store = DerivationStore::new();
    let entered = store.enter();
    let keys: Vec<u64> = (0..64).map(|key| key % 4 + 10).collect();
    let parallelism = Parallelism::new(NonZeroUsize::new(4).expect("nonzero"));
    let tripled = parallelism.map_ordered(&keys, |key| {
        *DerivationStore::derive_current::<Triple>(key).expect("no cycle")
    });
    assert_eq!(tripled, keys.iter().map(|key| key * 3).collect::<Vec<_>>());
    // Four keys, each built once by whichever worker asked first.
    assert_eq!(TRIPLES_BUILT.load(Ordering::SeqCst), 2 + 4);
    drop(entered);
    assert!(axioval_engine::derivation::current().is_none());
}
