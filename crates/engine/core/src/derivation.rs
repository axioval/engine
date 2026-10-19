//! The derivation store of [ADR 0006]: expensive, reusable work stated as
//! typed derivations and built once per key for a whole check.
//!
//! A derivation kind ([`DerivationKind`]) is a key type, a value type and a
//! pure build function that reads only its key and other derivations
//! ([`Inputs`]), never a rule. The [`DerivationStore`] keeps each kind's
//! values by key behind [`Arc`], compares keys field by field (`Eq`, never
//! a formatted string) and hashes them with `foldhash`.
//!
//! **Single flight.** A key is built at most once at a time. A caller asking
//! for a key another thread is building waits for that build and reads its
//! value; the store's lock is never held while building, so a build may read
//! other derivations (re-entrancy). A build that would wait, directly or
//! through other threads' waits, on a key it is itself building is a cycle:
//! it is reported ([`DerivationError::Cycle`]), never waited on. A build that
//! panics leaves nothing behind: its waiters wake and build the key
//! themselves, so they get what a first caller would have got. A value is
//! whatever the build returned, an error of the derivation's own included
//! (`Value = Result<T, E>`), and is shared like any other: a pure build
//! fails alike for every caller.
//!
//! A run installs one empty store ([`crate::Runtime`] replaces any host
//! copy), so nothing built in one check answers another. The engine reaches
//! it through the run's services, and so does a geometry adapter: the store
//! is a service of the [`ServiceRegistry`] like any other, so no core crate
//! depends on an adapter to read or build an adapter's derivations.
//! [`crate::MeasuredMemo`]'s shared table is a view of the same store.
//!
//! **The current store.** A geometry service's methods are handed a request,
//! never the registry, so the runtime also makes the run's store the
//! *current* one of the thread running the plan, and `Parallelism::map_ordered`
//! of every worker it starts ([`current`], [`DerivationStore::enter`]), as it
//! does the check's memory budget. An adapter derives through it
//! ([`DerivationStore::derive_current`]); outside a run there is none, and
//! every read is built afresh.
//!
//! **Keeping nothing.** A store made by [`DerivationStore::forgetful`]
//! builds every read afresh and keeps no value, still one build of a key at
//! a time, so a run shows that no report depends on what was reused: the
//! determinism gate of [#441](https://github.com/axioval/engine/issues/441)
//! runs every check with the store kept and forgetful, and requires the
//! same bytes. [`crate::Runtime::without_reuse`] installs one.
//!
//! **Precision** ([#438](https://github.com/axioval/engine/issues/438)).
//! A geometric kind keys its precision class ([`Precision`]: Exact, or Fast
//! with its budget, [`DerivationKind::precision`]), so the same question
//! asked Exact and Fast is two entries, and an Exact read never finds the
//! Fast one. A Fast value carries its excess ([`DerivationKind::excess`]);
//! every read checks it against the class asked, and a value the class does
//! not hold is refused ([`DerivationError::Precision`]), never handed out.
//!
//! **Releasing** ([#440](https://github.com/axioval/engine/issues/440)).
//! A kept value is memory held on purpose, so the store gives it up in
//! three ways, each only ever costing time: a released value is built
//! again when read again, to the same value, and every budget charges a
//! read as if it built.
//!
//! - After its last reader: a kind names the services it is read through
//!   ([`DerivationKind::read_through`]), and the runtime releases its
//!   values once no rule still to run may ask them
//!   ([`DerivationStore::release_unread`]).
//! - Past a bound: a kind's own ([`DerivationKind::keep_bytes`]) and the
//!   store's ([`DerivationStore::keeping`]), the values read least
//!   recently first, of any kind for the store's.
//! - For the memory budget: kept values never take the check past it
//!   beside the steps it admitted, and a step that does not fit asks the
//!   store to release them before it waits ([`DerivationStore::reclaimer`],
//!   [`crate::MemoryBudget::reclaim_from`]).
//!
//! [ADR 0006]: https://github.com/axioval/engine/blob/main/docs/src/adr-0006.md

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::ServiceRegistry;
use crate::memory::{self, Held, Reclaim};

mod key;
mod precision;

pub use key::{DerivationVersion, Kernel, KeyCompleteness};
pub use precision::{FastBudget, Precision};

/// A kind of derivation: a key, a value and the pure function building the
/// value of a key.
///
/// The build reads only its key and other derivations ([`Inputs::derive`]),
/// and the services the check runs with ([`Inputs::services`]), never a
/// rule's parameters or outcomes: whether a value was built or reused can
/// then never be seen in a report. Derivation kinds are engine-owned or
/// adapter-owned; a package never supplies one.
///
/// The key holds every input that can change the value: the
/// source-qualified identities of its objects, the tolerance and frame, the
/// parameters, and for a geometric derivation its precision class
/// ([`Self::precision`]), so a Fast value never answers an Exact question
/// ([#438](https://github.com/axioval/engine/issues/438),
/// [#439](https://github.com/axioval/engine/issues/439)). Keys are compared
/// field by field (`Eq`), never by a formatted string, and every kind has a
/// key-completeness test ([`KeyCompleteness`]), which the architecture gate
/// requires.
pub trait DerivationKind: 'static {
    /// The kind's name, as a run's statistics show it.
    const NAME: &'static str;

    /// The kind's version: a revision its crate bumps whenever the same key
    /// could build another value, and the kernel crates its build calls.
    /// Every value of the kind is keyed by it as by its key (#439), so a
    /// value is never read under another version (#409).
    const VERSION: DerivationVersion;

    /// What a value is a function of, compared field by field.
    type Key: Hash + Eq + Clone + Send + Sync + 'static;

    /// What is built for a key.
    type Value: Send + Sync + 'static;

    /// Builds the value of `key`.
    ///
    /// # Errors
    ///
    /// Only what reading another derivation reports ([`Inputs::derive`]):
    /// an error is not kept, so the next caller builds the key again. A
    /// failure of the derivation's own belongs in its value.
    fn build(key: &Self::Key, inputs: &Inputs<'_>) -> Result<Self::Value, DerivationError>;

    /// The precision class `key` asks for (#438), a field of the key: a
    /// geometric kind states it for every key, Exact or Fast with its
    /// budget; `None`, the default, is a kind that is not geometric (a
    /// measurement's table). The store hands a read only a value whose
    /// [`Self::excess`] the class holds ([`Precision::holds`]), and a
    /// kind's key-completeness test asks for a key in another class
    /// ([`KeyCompleteness::precision`]).
    fn precision(key: &Self::Key) -> Option<Precision> {
        let _ = key;
        None
    }

    /// The excess `value` carries (#438): for a value computed in a
    /// kernel's fast mode, how far beyond the exact one it may lie, at
    /// most its key's budget; `None`, the default, for a value computed
    /// exactly. Read on every read of a key with a [`Self::precision`].
    fn excess(value: &Self::Value) -> Option<f64> {
        let _ = value;
        None
    }

    /// The bytes `value` holds, where the kind can say: summed in the run's
    /// statistics and held against the check's memory budget while the
    /// value is kept ([`crate::MemoryBudget::hold`], #407).
    fn size_hint(value: &Self::Value) -> Option<usize> {
        let _ = value;
        None
    }

    /// The most bytes (as [`Self::size_hint`] states them) the store keeps
    /// of this kind's values at once, read when a value lands; `None`, the
    /// default, bounds the kind only by the store's own bound and the
    /// check's memory budget. Past it, the values read least recently are
    /// released, and built again when read again. Releasing only ever
    /// costs time, never changes a value.
    #[must_use]
    fn keep_bytes() -> Option<u64> {
        None
    }

    /// The host service handles (their types) through which a rule may
    /// come to read this kind's values, so the plan releases them after
    /// the last rule that may ask one of those services
    /// ([`DerivationStore::release_unread`],
    /// [#440](https://github.com/axioval/engine/issues/440)): a geometry
    /// adapter's kind names the handles whose services build or read it.
    /// `None`, the default, is a kind the plan cannot tell the readers of
    /// (a measurement's memo), kept for the whole check within the bounds.
    ///
    /// A handle left out only costs time: a value released before a read
    /// the plan did not foresee is built again, to the same value.
    #[must_use]
    fn read_through() -> Option<Vec<TypeId>> {
        None
    }
}

/// Why a derivation could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DerivationError {
    /// Building the kind's key needs that very key, directly or through
    /// other derivations, on this thread or on threads it waits for.
    #[error("derivation cycle: {kind} is read while it is being built")]
    Cycle {
        /// The kind of the key read while it was being built.
        kind: &'static str,
    },
    /// The value held or built for a key carries an excess its precision
    /// class does not hold ([`DerivationKind::excess`], #438): a Fast value
    /// for an Exact question, or one past the budget asked. Never handed
    /// out; the read fails closed.
    #[error("{kind}: a value carrying an excess past its key's class ({asked}) is not handed out")]
    Precision {
        /// The kind read.
        kind: &'static str,
        /// The class the key asked for.
        asked: Precision,
    },
}

/// What a build may read: other derivations, through the store building it,
/// and the check's services.
pub struct Inputs<'a> {
    store: &'a DerivationStore,
    services: &'a ServiceRegistry,
}

impl Inputs<'_> {
    /// The value of `key` of kind `D`, built once by the same store.
    ///
    /// # Errors
    ///
    /// As [`DerivationStore::derive`].
    pub fn derive<D: DerivationKind>(
        &self,
        key: &D::Key,
    ) -> Result<Arc<D::Value>, DerivationError> {
        self.store.derive::<D>(self.services, key)
    }

    /// The services of the check: sources and geometry services, never a
    /// rule's state.
    #[must_use]
    pub fn services(&self) -> &ServiceRegistry {
        self.services
    }
}

/// How a run reused one derivation kind (or one table of the run's
/// [`crate::MeasuredMemo`]): the reuse report of
/// [#442](https://github.com/axioval/engine/issues/442).
///
/// Every read is a hit or a build (a miss); a hit may have waited for
/// another caller's build. Counts are taken under the store's lock; the
/// build time is read from a clock around each build. None of it reaches a
/// report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DerivationStatistics {
    /// The kind's name.
    pub kind: &'static str,
    /// The kind's version; `None` for a table of the memo, which keeps
    /// nothing past its run.
    pub version: Option<DerivationVersion>,
    /// Values held now.
    pub entries: usize,
    /// Reads answered by a held value, a waited-for one included.
    pub hits: u64,
    /// Builds started: the reads no held value answered.
    pub builds: u64,
    /// Reads that waited for another caller's build.
    pub waits: u64,
    /// The time spent in the builds that ended with a value, summed over
    /// threads.
    pub build_time: Duration,
    /// Values released by name ([`DerivationStore::release`]) or after
    /// the last rule that may read them
    /// ([`DerivationStore::release_unread`], #440).
    pub released: u64,
    /// Values released to keep within a bound: the kind's
    /// ([`DerivationKind::keep_bytes`]), the store's
    /// ([`DerivationStore::keeping`]) or the check's memory budget (#440).
    pub evicted: u64,
    /// The bytes held now, as the kinds' [`DerivationKind::size_hint`]s
    /// state them.
    pub bytes: u64,
    /// The most bytes held at once.
    pub peak_bytes: u64,
}

/// One store of derivations per check, shared by its rules and threads.
///
/// Clones share their values. See the [module](self) for single flight,
/// cycles and panics.
#[derive(Clone, Default)]
pub struct DerivationStore(Arc<Shared>);

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled when a build that others wait for ends, however it ends.
    ended: Condvar,
    /// Whether no value is kept once built ([`DerivationStore::forgetful`]).
    forgets: bool,
    /// The most bytes the store keeps of all its kinds together
    /// ([`DerivationStore::keeping`]).
    keep: Option<u64>,
}

#[derive(Default)]
struct State {
    /// Each kind's table, by the kind's marker, key and value type.
    tables: HashMap<TypeId, Box<dyn Kept>, foldhash::fast::FixedState>,
    /// Each waiting caller (by a number of its own): the build it waits for
    /// and the builds it works within.
    waiting: HashMap<u64, Waiting, foldhash::fast::FixedState>,
    /// Counts reads across every kind, so the value read least recently
    /// is known store-wide.
    clock: u64,
}

struct Waiting {
    on: u64,
    within: Vec<u64>,
}

/// A slot of a table: a value held (its bytes held against the check's
/// memory budget while it is, and when it was last read), or a build in
/// flight.
enum Slot<S> {
    Ready {
        value: S,
        bytes: u64,
        read: u64,
        held: Option<Held>,
    },
    Building {
        flight: u64,
        waiters: u32,
    },
}

impl<S> Slot<S> {
    /// A value held, its bytes held against the calling thread's budget.
    fn ready(value: S, bytes: u64, read: u64) -> Self {
        Self::Ready {
            value,
            bytes,
            read,
            held: hold(bytes),
        }
    }
}

/// `value`, read for `key`, where the precision class `key` asks for holds
/// the excess it carries (#438); else the refusal. A kind keying its class
/// keeps an Exact read from ever finding a Fast value, and this keeps one
/// from being handed out should a build, or what a value keeps after it,
/// state otherwise.
fn held_to<D: DerivationKind>(
    key: &D::Key,
    value: Arc<D::Value>,
) -> Result<Arc<D::Value>, DerivationError> {
    match D::precision(key) {
        Some(asked) if !asked.holds(D::excess(&value)) => Err(DerivationError::Precision {
            kind: D::NAME,
            asked,
        }),
        _ => Ok(value),
    }
}

/// `bytes` held against the memory budget of the calling thread's check,
/// if it has one and they are some.
fn hold(bytes: u64) -> Option<Held> {
    (bytes > 0)
        .then(memory::current)
        .flatten()
        .map(|budget| budget.hold(bytes))
}

/// A released value, dropped once the store is unlocked.
type Released = Box<dyn Any + Send>;

/// How a table is made: its kind's name, what the kind keeps at most, and
/// the services its values are read through.
#[derive(Clone, Copy)]
pub(crate) struct Policy {
    pub(crate) kind: &'static str,
    pub(crate) version: Option<DerivationVersion>,
    pub(crate) keep: Option<u64>,
    pub(crate) through: fn() -> Option<Vec<TypeId>>,
}

impl Policy {
    /// A table of `kind`, bounded only by the store and the budget, whose
    /// readers the plan cannot tell.
    pub(crate) fn unplanned(kind: &'static str) -> Self {
        Self {
            kind,
            version: None,
            keep: None,
            through: || None,
        }
    }
}

struct Table<K, S> {
    /// Keys are shared with the build in flight, which finds its slot by
    /// them, so a key need not be cloned.
    entries: HashMap<Arc<K>, Slot<S>, foldhash::fast::RandomState>,
    statistics: DerivationStatistics,
    /// The services the kind's values are read through
    /// ([`DerivationKind::read_through`]).
    through: Option<Vec<TypeId>>,
}

impl<K: Hash + Eq, S: Send + 'static> Table<K, S> {
    /// Counts `bytes` more held.
    fn grew(&mut self, bytes: u64) {
        self.statistics.bytes += bytes;
        self.statistics.peak_bytes = self.statistics.peak_bytes.max(self.statistics.bytes);
    }

    /// The value read least recently, read before `before`.
    fn oldest_key(&self, before: u64) -> Option<(u64, Arc<K>)> {
        self.entries
            .iter()
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { read, .. } if *read < before => Some((*read, Arc::clone(key))),
                _ => None,
            })
            .min_by_key(|(read, _)| *read)
    }

    /// Releases the value held for `key`, its bytes returned to the budget
    /// at once; the value, to be dropped once the store is unlocked.
    fn remove(&mut self, key: &K) -> Option<S> {
        if !matches!(self.entries.get(key), Some(Slot::Ready { .. })) {
            return None;
        }
        let Some(Slot::Ready {
            value, bytes, held, ..
        }) = self.entries.remove(key)
        else {
            return None;
        };
        drop(held);
        self.statistics.bytes -= bytes;
        Some(value)
    }

    /// Releases the value held for `key` to keep within a bound.
    fn evict_key(&mut self, key: &K) -> Option<S> {
        let value = self.remove(key)?;
        self.statistics.evicted += 1;
        Some(value)
    }

    /// Releases the values read least recently, all read before `before`,
    /// until the table holds at most `limit` bytes.
    fn evict(&mut self, limit: u64, before: u64, released: &mut Vec<Released>) {
        while self.statistics.bytes > limit {
            let Some((_, key)) = self.oldest_key(before) else {
                break;
            };
            if let Some(value) = self.evict_key(&key) {
                released.push(Box::new(value));
            }
        }
    }
}

trait Kept: Send {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn statistics(&self) -> DerivationStatistics;
    /// The bytes held now.
    fn bytes(&self) -> u64;
    /// When the value read least recently, read before `before`, was read.
    fn oldest(&self, before: u64) -> Option<u64>;
    /// Releases the value read least recently, read before `before`, and
    /// tells its bytes.
    fn evict_oldest(&mut self, before: u64) -> Option<(u64, Released)>;
    /// The services the kind's values are read through.
    fn through(&self) -> Option<&[TypeId]>;
    /// Releases every value held, after the last rule reading it.
    fn retire(&mut self, released: &mut Vec<Released>);
}

impl<K: Hash + Eq + Send + Sync + 'static, S: Send + 'static> Kept for Table<K, S> {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn statistics(&self) -> DerivationStatistics {
        DerivationStatistics {
            entries: self
                .entries
                .values()
                .filter(|slot| matches!(slot, Slot::Ready { .. }))
                .count(),
            ..self.statistics.clone()
        }
    }

    fn bytes(&self) -> u64 {
        self.statistics.bytes
    }

    fn oldest(&self, before: u64) -> Option<u64> {
        self.oldest_key(before).map(|(read, _)| read)
    }

    fn evict_oldest(&mut self, before: u64) -> Option<(u64, Released)> {
        let (_, key) = self.oldest_key(before)?;
        let bytes = match self.entries.get(&*key) {
            Some(Slot::Ready { bytes, .. }) => *bytes,
            _ => 0,
        };
        let value = self.evict_key(&key)?;
        Some((bytes, Box::new(value)))
    }

    fn through(&self) -> Option<&[TypeId]> {
        self.through.as_deref()
    }

    fn retire(&mut self, released: &mut Vec<Released>) {
        let keys: Vec<Arc<K>> = self
            .entries
            .iter()
            .filter(|(_, slot)| matches!(slot, Slot::Ready { .. }))
            .map(|(key, _)| Arc::clone(key))
            .collect();
        for key in keys {
            if let Some(value) = self.remove(&key) {
                self.statistics.released += 1;
                released.push(Box::new(value));
            }
        }
    }
}

/// Numbers builds and waiting callers, unique across stores.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    /// The store of the check this thread evaluates ([`current`]).
    static CURRENT: RefCell<Option<DerivationStore>> = const { RefCell::new(None) };
    /// The builds this thread works within: its own, innermost last, after
    /// those of the thread that started it to work for them.
    static LINEAGE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// The builds the calling thread works within, for a thread it starts to
/// work for them ([`within`]).
pub(crate) fn lineage() -> Vec<u64> {
    LINEAGE.with(|lineage| lineage.borrow().clone())
}

/// Runs `work` on this thread as working within `lineage`'s builds: a
/// worker reading a key the thread that started it is building is a cycle,
/// not a wait.
pub(crate) fn within<R>(lineage: Vec<u64>, work: impl FnOnce() -> R) -> R {
    let outer = LINEAGE.with(|current| std::mem::replace(&mut *current.borrow_mut(), lineage));
    let restore = Restore(Some(outer));
    let answer = work();
    drop(restore);
    answer
}

struct Restore(Option<Vec<u64>>);

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(outer) = self.0.take() {
            LINEAGE.with(|current| *current.borrow_mut() = outer);
        }
    }
}

/// The store of the check this thread evaluates, if it evaluates one: the
/// run's, entered by the runtime on the thread running the plan and by
/// `Parallelism::map_ordered` on its workers ([`DerivationStore::enter`]).
#[must_use]
pub fn current() -> Option<DerivationStore> {
    CURRENT.with(|current| current.borrow().clone())
}

/// The calling thread's current store while held
/// ([`DerivationStore::enter`]); the one it replaced is put back when
/// dropped.
#[must_use = "the store is current only while this is held"]
pub struct Entered {
    outer: Option<DerivationStore>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let outer = self.outer.take();
        CURRENT.with(|current| *current.borrow_mut() = outer);
    }
}

/// Makes `store` the calling thread's current one until the guard is
/// dropped; `None` makes it have none.
pub(crate) fn enter(store: Option<DerivationStore>) -> Entered {
    let outer = CURRENT.with(|current| std::mem::replace(&mut *current.borrow_mut(), store));
    Entered { outer }
}

/// The marker of a [`DerivationKind`]'s table, apart from any other use of
/// the same key and value types.
struct Typed<D>(PhantomData<D>);

impl DerivationStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that keeps nothing: every read builds its key afresh (one
    /// build of a key at a time, its waiters then building it again), and
    /// whatever is put in it is dropped. Since a value is a pure function
    /// of its key, every answer is what a keeping store gives, only built
    /// more often: the determinism gate (#441) compares the two.
    #[must_use]
    pub fn forgetful() -> Self {
        Self(Arc::new(Shared {
            forgets: true,
            ..Shared::default()
        }))
    }

    /// A store that keeps at most `bytes` of all its kinds' values
    /// together (as their [`DerivationKind::size_hint`]s state them):
    /// past it, the values read least recently, of any kind, are released
    /// and built again when read again. The value just built is always
    /// kept until another lands, so a bound of one byte releases nearly
    /// everything: the determinism gate (#440, #441) runs a check so,
    /// to show that releasing only ever costs time.
    #[must_use]
    pub fn keeping(bytes: u64) -> Self {
        Self(Arc::new(Shared {
            keep: Some(bytes),
            ..Shared::default()
        }))
    }

    /// Whether the store keeps the values it builds (not
    /// [`Self::forgetful`]).
    #[must_use]
    pub fn keeps(&self) -> bool {
        !self.0.forgets
    }

    /// The store as a keeper the check's memory budget may ask to release
    /// what it holds ([`crate::MemoryBudget::reclaim_from`]): a step that
    /// does not fit beside the kept values then releases the values read
    /// least recently before it waits.
    #[must_use]
    pub fn reclaimer(&self) -> Weak<dyn Reclaim> {
        let shared: Arc<dyn Reclaim> = self.0.clone();
        Arc::downgrade(&shared)
    }

    /// The value of `key` of kind `D`, built once: by this caller, or by
    /// the caller already building it, waited for.
    ///
    /// # Errors
    ///
    /// [`DerivationError::Cycle`] when building `key` needs `key` itself,
    /// on this thread or through the threads it would wait for;
    /// [`DerivationError::Precision`] when the value carries an excess the
    /// precision class `key` asks for does not hold (#438).
    ///
    /// # Panics
    ///
    /// When `D`'s build panics, the panic is raised again here; whoever
    /// waited for the build then builds the key itself.
    pub fn derive<D: DerivationKind>(
        &self,
        services: &ServiceRegistry,
        key: &D::Key,
    ) -> Result<Arc<D::Value>, DerivationError> {
        self.get_or_build::<Typed<D>, D::Key, Arc<D::Value>, _>(
            Policy {
                kind: D::NAME,
                version: Some(D::VERSION),
                keep: D::keep_bytes(),
                through: D::read_through,
            },
            Keyed::Borrowed(key, D::Key::clone),
            || {
                let value = D::build(
                    key,
                    &Inputs {
                        store: self,
                        services,
                    },
                )?;
                let bytes = D::size_hint(&value).map_or(0, |bytes| bytes as u64);
                Ok((Arc::new(value), bytes))
            },
            Arc::clone,
        )
        .and_then(|value| held_to::<D>(key, value))
    }

    /// [`Self::derive`] in the store `services` holds; without one (outside
    /// a run), in a store of this call alone, so a build still reads each
    /// other derivation once and a cycle is still reported.
    ///
    /// # Errors
    ///
    /// As [`Self::derive`].
    pub fn of<D: DerivationKind>(
        services: &ServiceRegistry,
        key: &D::Key,
    ) -> Result<Arc<D::Value>, DerivationError> {
        match services.get::<Self>() {
            Some(store) => store.derive::<D>(services, key),
            None => Self::new().derive::<D>(services, key),
        }
    }

    /// Makes this the calling thread's current store ([`current`]) until
    /// the returned guard is dropped, which puts back the one it replaced.
    pub fn enter(&self) -> Entered {
        enter(Some(self.clone()))
    }

    /// [`Self::derive`] in the calling thread's current store ([`current`]),
    /// for a build that reads no service: a geometry adapter's kind, whose
    /// key names the adapter state it is built over. Outside a run there is
    /// no current store, and the value is built afresh (in a store of this
    /// call alone).
    ///
    /// # Errors
    ///
    /// As [`Self::derive`].
    pub fn derive_current<D: DerivationKind>(
        key: &D::Key,
    ) -> Result<Arc<D::Value>, DerivationError> {
        let services = ServiceRegistry::new();
        match current() {
            Some(store) => store.derive::<D>(&services, key),
            None => Self::new().derive::<D>(&services, key),
        }
    }

    /// States again the bytes the value held for `key` of kind `D` holds,
    /// where it has grown since it was built (a memo it keeps), so the
    /// statistics and the budget count what is held. Past the kind's, the
    /// store's or the budget's bound, other values are released, the ones
    /// read least recently first. Whether a value is held.
    pub fn resize<D: DerivationKind>(&self, key: &D::Key, bytes: usize) -> bool {
        let mut released = Vec::new();
        let mut state = self.lock();
        let now = state.tick();
        let Some(table) = state.existing::<Typed<D>, D::Key, Arc<D::Value>>() else {
            return false;
        };
        let Some(Slot::Ready {
            bytes: counted,
            held,
            read,
            ..
        }) = table.entries.get_mut(key)
        else {
            return false;
        };
        let bytes = bytes as u64;
        let before = *counted;
        *counted = bytes;
        *read = now;
        // The bytes held before are returned as the new ones are held.
        *held = hold(bytes);
        table.statistics.bytes -= before;
        table.grew(bytes);
        if let Some(limit) = D::keep_bytes() {
            table.evict(limit, now, &mut released);
        }
        state.enforce(self.0.keep, now, &mut released);
        drop(state);
        drop(released);
        true
    }

    /// The value held for `key` of kind `D`, if one is built.
    #[must_use]
    pub fn held<D: DerivationKind>(&self, key: &D::Key) -> Option<Arc<D::Value>> {
        self.read_held::<Typed<D>, D::Key, Arc<D::Value>, _>(key, Arc::clone)
            .and_then(|value| held_to::<D>(key, value).ok())
    }

    /// Releases the value held for `key` of kind `D`, if any: the next read
    /// builds it again, to the same value. Whether it was held.
    pub fn release<D: DerivationKind>(&self, key: &D::Key) -> bool {
        let mut state = self.lock();
        let released = state
            .existing::<Typed<D>, D::Key, Arc<D::Value>>()
            .and_then(|table| {
                let value = table.remove(key)?;
                table.statistics.released += 1;
                Some(value)
            });
        drop(state);
        released.is_some()
    }

    /// Releases every value of every kind whose readers the plan can tell
    /// ([`DerivationKind::read_through`]) and that no rule still to run
    /// reads: every service it is read through is `unread`. The runtime
    /// calls it before each rule and after the last (#440). A kind whose
    /// readers the plan cannot tell is kept. The number of values
    /// released.
    pub fn release_unread(&self, unread: impl Fn(TypeId) -> bool) -> u64 {
        let mut released = Vec::new();
        let mut state = self.lock();
        for table in state.tables.values_mut() {
            if table
                .through()
                .is_some_and(|through| through.iter().all(|service| unread(*service)))
            {
                table.retire(&mut released);
            }
        }
        drop(state);
        released.len() as u64
    }

    /// How each kind was reused so far, by kind name.
    #[must_use]
    pub fn statistics(&self) -> Vec<DerivationStatistics> {
        let state = self.lock();
        let mut statistics: Vec<DerivationStatistics> = state
            .tables
            .values()
            .map(|table| table.statistics())
            .collect();
        statistics.sort_by(|a, b| a.kind.cmp(b.kind));
        statistics
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock()
    }

    /// What `read` reads of the value held for `key` in `Marker`'s table,
    /// if it is built; a key being built is not waited for.
    pub(crate) fn read_held<Marker, K, S, R>(
        &self,
        key: &K,
        read: impl FnOnce(&S) -> R,
    ) -> Option<R>
    where
        Marker: 'static,
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        let mut state = self.lock();
        let now = state.tick();
        let table = state.existing::<Marker, K, S>()?;
        match table.entries.get_mut(key)? {
            Slot::Ready {
                value, read: last, ..
            } => {
                *last = now;
                Some(read(value))
            }
            Slot::Building { .. } => None,
        }
    }

    /// Holds `value` for `key` in `Marker`'s table, replacing what was held
    /// or is being built (whose build then keeps its value to itself).
    pub(crate) fn put<Marker, K, S>(&self, kind: &'static str, key: K, value: S)
    where
        Marker: 'static,
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        if self.0.forgets {
            return;
        }
        let mut state = self.lock();
        let now = state.tick();
        let table = state.table::<Marker, K, S>(Policy::unplanned(kind));
        match table
            .entries
            .insert(Arc::new(key), Slot::ready(value, 0, now))
        {
            Some(Slot::Ready { bytes, .. }) => table.statistics.bytes -= bytes,
            // Whoever waited for that build reads the value now held.
            Some(Slot::Building { .. }) => self.0.ended.notify_all(),
            None => {}
        }
    }

    /// Changes the value held for `key` in `Marker`'s table (starting from
    /// the default, also in place of a build in flight) by `change`, with
    /// the store locked.
    pub(crate) fn change<Marker, K, S>(
        &self,
        kind: &'static str,
        key: K,
        change: impl FnOnce(&mut S),
    ) where
        Marker: 'static,
        K: Hash + Eq + Send + Sync + 'static,
        S: Default + Send + 'static,
    {
        if self.0.forgets {
            return;
        }
        let mut state = self.lock();
        let now = state.tick();
        let table = state.table::<Marker, K, S>(Policy::unplanned(kind));
        let fresh = || Slot::ready(S::default(), 0, now);
        let slot = match table.entries.get_mut(&key) {
            Some(slot) => {
                if matches!(slot, Slot::Building { .. }) {
                    *slot = fresh();
                    // Whoever waited for that build reads the value now held.
                    self.0.ended.notify_all();
                }
                slot
            }
            None => table.entries.entry(Arc::new(key)).or_insert_with(fresh),
        };
        if let Slot::Ready { value, read, .. } = slot {
            *read = now;
            change(value);
        }
    }

    /// The value of `key` in `Marker`'s table, read by `read`: held, built
    /// by the caller already building it and waited for, or built here by
    /// `build` (with the bytes it holds) and held.
    ///
    /// Past the kind's, the store's or the budget's bound, the values read
    /// least recently are released when this one lands.
    pub(crate) fn get_or_build<Marker, K, S, R>(
        &self,
        policy: Policy,
        key: Keyed<'_, K>,
        build: impl FnOnce() -> Result<(S, u64), DerivationError>,
        read: impl FnOnce(&S) -> R,
    ) -> Result<R, DerivationError>
    where
        Marker: 'static,
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        let kind = policy.kind;
        let mut state = self.lock();
        let (key, flight) = loop {
            let now = state.tick();
            let table = state.table::<Marker, K, S>(policy);
            let building = match table.entries.get_mut(key.get()) {
                Some(Slot::Ready {
                    value, read: last, ..
                }) => {
                    *last = now;
                    table.statistics.hits += 1;
                    return Ok(read(value));
                }
                Some(Slot::Building { flight, .. }) => *flight,
                None => {
                    let flight = next();
                    let key = Arc::new(key.into_owned());
                    table
                        .entries
                        .insert(Arc::clone(&key), Slot::Building { flight, waiters: 0 });
                    table.statistics.builds += 1;
                    break (key, flight);
                }
            };
            let within = lineage();
            if state.blocks(building, &within) {
                return Err(DerivationError::Cycle { kind });
            }
            let table = state.table::<Marker, K, S>(policy);
            table.statistics.waits += 1;
            if let Some(Slot::Building { waiters, .. }) = table.entries.get_mut(key.get()) {
                *waiters += 1;
            }
            let waiter = next();
            state.waiting.insert(
                waiter,
                Waiting {
                    on: building,
                    within,
                },
            );
            while state.building::<Marker, K, S>(key.get(), building) {
                state = self
                    .0
                    .ended
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            state.waiting.remove(&waiter);
        };
        drop(state);
        let landing = Landing::<Marker, K, S> {
            store: self,
            key,
            flight,
            keep: policy.keep,
            landed: false,
            marker: PhantomData,
        };
        LINEAGE.with(|lineage| lineage.borrow_mut().push(flight));
        let started = Instant::now();
        let (value, bytes) = build()?;
        Ok(landing.land(value, bytes, started.elapsed(), read))
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic never leaves the state half-changed: builds run unlocked.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The check's memory budget asks the store to release what it keeps when
/// a step needs the room (#440): the values read least recently, of any
/// kind, first. Each is built again when read again, to the same value.
impl Reclaim for Shared {
    fn reclaim(&self, bytes: u64) -> u64 {
        let mut released = Vec::new();
        let mut freed = 0;
        let mut state = self.lock();
        while freed < bytes {
            let Some(bytes) = state.evict_oldest(u64::MAX, &mut released) else {
                break;
            };
            freed += bytes;
        }
        drop(state);
        drop(released);
        freed
    }
}

/// A key as a caller has it: its own, or borrowed with the way to copy it
/// should the store need its own.
pub(crate) enum Keyed<'a, K> {
    Owned(K),
    Borrowed(&'a K, fn(&K) -> K),
}

impl<K> Keyed<'_, K> {
    fn get(&self) -> &K {
        match self {
            Self::Owned(key) => key,
            Self::Borrowed(key, _) => key,
        }
    }

    fn into_owned(self) -> K {
        match self {
            Self::Owned(key) => key,
            Self::Borrowed(key, copy) => copy(key),
        }
    }
}

/// A build in flight. Landing it holds its value and wakes whoever waits
/// for it; dropping it unlanded (a panic or an error) removes its slot and
/// wakes them, so they build the key themselves. Either way its thread
/// leaves the build.
struct Landing<'a, Marker, K, S>
where
    Marker: 'static,
    K: Hash + Eq + Send + Sync + 'static,
    S: Send + 'static,
{
    store: &'a DerivationStore,
    key: Arc<K>,
    flight: u64,
    keep: Option<u64>,
    landed: bool,
    marker: PhantomData<fn() -> (Marker, S)>,
}

impl<Marker, K, S> Landing<'_, Marker, K, S>
where
    Marker: 'static,
    K: Hash + Eq + Send + Sync + 'static,
    S: Send + 'static,
{
    fn land<R>(mut self, value: S, bytes: u64, took: Duration, read: impl FnOnce(&S) -> R) -> R {
        // Should reading panic, the build is dropped unlanded.
        let answer = read(&value);
        self.landed = true;
        // Released values are dropped after the store is unlocked.
        let mut released = Vec::new();
        let mut state = self.store.lock();
        let now = state.tick();
        // A value put in its place meanwhile stays: the build keeps its own.
        if let Some(table) = state.existing::<Marker, K, S>()
            && let Some(slot) = table.entries.get_mut(&*self.key)
            && let Slot::Building { flight, waiters } = *slot
            && flight == self.flight
        {
            table.statistics.build_time += took;
            if self.store.0.forgets {
                // Nothing is kept: whoever waited builds the key again.
                table.entries.remove(&*self.key);
                if waiters > 0 {
                    self.store.0.ended.notify_all();
                }
                return answer;
            }
            *slot = Slot::ready(value, bytes, now);
            table.grew(bytes);
            if let Some(limit) = self.keep {
                table.evict(limit, now, &mut released);
            }
            if waiters > 0 {
                self.store.0.ended.notify_all();
            }
            state.enforce(self.store.0.keep, now, &mut released);
        }
        drop(state);
        drop(released);
        answer
    }
}

impl<Marker, K, S> Drop for Landing<'_, Marker, K, S>
where
    Marker: 'static,
    K: Hash + Eq + Send + Sync + 'static,
    S: Send + 'static,
{
    fn drop(&mut self) {
        LINEAGE.with(|lineage| {
            let mut lineage = lineage.borrow_mut();
            if lineage.last() == Some(&self.flight) {
                lineage.pop();
            }
        });
        if self.landed {
            return;
        }
        let mut state = self.store.lock();
        if state.building::<Marker, K, S>(&*self.key, self.flight) {
            if let Some(table) = state.existing::<Marker, K, S>() {
                table.entries.remove(&*self.key);
            }
            self.store.0.ended.notify_all();
        }
    }
}

impl State {
    /// The next read's time, store-wide.
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Releases the value read least recently, of any kind, read before
    /// `before`; its bytes.
    fn evict_oldest(&mut self, before: u64, released: &mut Vec<Released>) -> Option<u64> {
        let table = self
            .tables
            .values_mut()
            .filter_map(|table| table.oldest(before).map(|read| (read, table)))
            .min_by_key(|(read, _)| *read)
            .map(|(_, table)| table)?;
        let (bytes, value) = table.evict_oldest(before)?;
        released.push(value);
        Some(bytes)
    }

    /// Releases the values read least recently, all read before `before`,
    /// while the store holds more than `keep` bytes, or while the memory
    /// budget of the calling thread's check is passed by what is admitted
    /// and held: what is kept never takes the room of a step, nor takes
    /// the check past its budget.
    fn enforce(&mut self, keep: Option<u64>, before: u64, released: &mut Vec<Released>) {
        let budget = memory::current();
        loop {
            let over_keep = keep.is_some_and(|keep| {
                self.tables.values().map(|table| table.bytes()).sum::<u64>() > keep
            });
            let over_budget = budget
                .as_ref()
                .is_some_and(|budget| budget.in_use() > budget.limit());
            if !(over_keep || over_budget) || self.evict_oldest(before, released).is_none() {
                break;
            }
        }
    }

    fn existing<Marker: 'static, K, S>(&mut self) -> Option<&mut Table<K, S>>
    where
        K: Send + Sync + 'static,
        S: Send + 'static,
    {
        self.tables
            .get_mut(&TypeId::of::<(Marker, K, S)>())?
            .as_any_mut()
            .downcast_mut::<Table<K, S>>()
    }

    /// `Marker`'s table, made by `policy` when it has none.
    fn table<Marker: 'static, K, S>(&mut self, policy: Policy) -> &mut Table<K, S>
    where
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        self.tables
            .entry(TypeId::of::<(Marker, K, S)>())
            .or_insert_with(|| {
                Box::new(Table::<K, S> {
                    entries: HashMap::default(),
                    statistics: DerivationStatistics {
                        kind: policy.kind,
                        version: policy.version,
                        ..DerivationStatistics::default()
                    },
                    through: (policy.through)(),
                })
            })
            .as_any_mut()
            .downcast_mut::<Table<K, S>>()
            .expect("a table is kept under its own type")
    }

    /// Whether `key` of `Marker`'s table is still being built by `flight`.
    fn building<Marker: 'static, K, S>(&mut self, key: &K, flight: u64) -> bool
    where
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        self.existing::<Marker, K, S>()
            .and_then(|table| table.entries.get(key))
            .is_some_and(
                |slot| matches!(slot, Slot::Building { flight: building, .. } if *building == flight),
            )
    }

    /// Whether waiting for the build `flight` would never end for a caller
    /// working within the builds `within`: `flight` is one of them, or a
    /// caller working within `flight` waits, directly or through others'
    /// waits, for one of them.
    fn blocks(&self, flight: u64, within: &[u64]) -> bool {
        let mut reached = vec![flight];
        let mut next = 0;
        while let Some(&build) = reached.get(next) {
            next += 1;
            if within.contains(&build) {
                return true;
            }
            for waiting in self.waiting.values() {
                if waiting.within.contains(&build) && !reached.contains(&waiting.on) {
                    reached.push(waiting.on);
                }
            }
        }
        false
    }
}
