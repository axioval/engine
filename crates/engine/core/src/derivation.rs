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
//! [ADR 0006]: https://github.com/axioval/engine/blob/main/docs/src/adr-0006.md

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::ServiceRegistry;

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
/// parameters, and for a geometric derivation its precision class, so a
/// Fast value never answers an Exact question
/// ([#438](https://github.com/axioval/engine/issues/438),
/// [#439](https://github.com/axioval/engine/issues/439)).
pub trait DerivationKind: 'static {
    /// The kind's name, as a run's statistics show it.
    const NAME: &'static str;

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

    /// The bytes `value` holds, where the kind can say: what a kept value
    /// will be charged against the check's memory budget
    /// ([#440](https://github.com/axioval/engine/issues/440)). Today it is
    /// only summed in the run's statistics.
    fn size_hint(value: &Self::Value) -> Option<usize> {
        let _ = value;
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
/// [`crate::MeasuredMemo`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DerivationStatistics {
    /// The kind's name.
    pub kind: &'static str,
    /// Values held now.
    pub entries: usize,
    /// Reads answered by a held value, a waited-for one included.
    pub hits: u64,
    /// Builds started.
    pub builds: u64,
    /// Reads that waited for another caller's build.
    pub waits: u64,
    /// Values released.
    pub released: u64,
    /// The bytes held now, as the kinds' [`DerivationKind::size_hint`]s
    /// state them.
    pub bytes: u64,
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
}

#[derive(Default)]
struct State {
    /// Each kind's table, by the kind's marker, key and value type.
    tables: HashMap<TypeId, Box<dyn Kept>, foldhash::fast::FixedState>,
    /// Each waiting caller (by a number of its own): the build it waits for
    /// and the builds it works within.
    waiting: HashMap<u64, Waiting, foldhash::fast::FixedState>,
}

struct Waiting {
    on: u64,
    within: Vec<u64>,
}

/// A slot of a table: a value held, or a build in flight.
enum Slot<S> {
    Ready { value: S, bytes: u64 },
    Building { flight: u64, waiters: u32 },
}

struct Table<K, S> {
    /// Keys are shared with the build in flight, which finds its slot by
    /// them, so a key need not be cloned.
    entries: HashMap<Arc<K>, Slot<S>, foldhash::fast::RandomState>,
    statistics: DerivationStatistics,
}

trait Kept: Send {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn statistics(&self) -> DerivationStatistics;
}

impl<K: Send + Sync + 'static, S: Send + 'static> Kept for Table<K, S> {
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
}

/// Numbers builds and waiting callers, unique across stores.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
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

/// The marker of a [`DerivationKind`]'s table, apart from any other use of
/// the same key and value types.
struct Typed<D>(PhantomData<D>);

impl DerivationStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The value of `key` of kind `D`, built once: by this caller, or by
    /// the caller already building it, waited for.
    ///
    /// # Errors
    ///
    /// [`DerivationError::Cycle`] when building `key` needs `key` itself,
    /// on this thread or through the threads it would wait for.
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
            D::NAME,
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

    /// The value held for `key` of kind `D`, if one is built.
    #[must_use]
    pub fn held<D: DerivationKind>(&self, key: &D::Key) -> Option<Arc<D::Value>> {
        self.read_held::<Typed<D>, D::Key, Arc<D::Value>, _>(key, Arc::clone)
    }

    /// Releases the value held for `key` of kind `D`, if any: the next read
    /// builds it again, to the same value. Whether it was held.
    pub fn release<D: DerivationKind>(&self, key: &D::Key) -> bool {
        let mut state = self.lock();
        let Some(table) = state.existing::<Typed<D>, D::Key, Arc<D::Value>>() else {
            return false;
        };
        if let Some(Slot::Ready { bytes, .. }) = table.entries.get(key) {
            let bytes = *bytes;
            table.entries.remove(key);
            table.statistics.released += 1;
            table.statistics.bytes -= bytes;
            true
        } else {
            false
        }
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
        // A panic never leaves the state half-changed: builds run unlocked.
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
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
        let table = state.existing::<Marker, K, S>()?;
        match table.entries.get(key)? {
            Slot::Ready { value, .. } => Some(read(value)),
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
        let mut state = self.lock();
        let table = state.table::<Marker, K, S>(kind);
        match table
            .entries
            .insert(Arc::new(key), Slot::Ready { value, bytes: 0 })
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
        let mut state = self.lock();
        let table = state.table::<Marker, K, S>(kind);
        let fresh = || Slot::Ready {
            value: S::default(),
            bytes: 0,
        };
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
        if let Slot::Ready { value, .. } = slot {
            change(value);
        }
    }

    /// The value of `key` in `Marker`'s table, read by `read`: held, built
    /// by the caller already building it and waited for, or built here by
    /// `build` (with the bytes it holds) and held.
    pub(crate) fn get_or_build<Marker, K, S, R>(
        &self,
        kind: &'static str,
        key: Keyed<'_, K>,
        build: impl FnOnce() -> Result<(S, u64), DerivationError>,
        read: impl FnOnce(&S) -> R,
    ) -> Result<R, DerivationError>
    where
        Marker: 'static,
        K: Hash + Eq + Send + Sync + 'static,
        S: Send + 'static,
    {
        let mut state = self.lock();
        let (key, flight) = loop {
            let table = state.table::<Marker, K, S>(kind);
            let building = match table.entries.get(key.get()) {
                Some(Slot::Ready { value, .. }) => {
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
            let table = state.table::<Marker, K, S>(kind);
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
            landed: false,
            marker: PhantomData,
        };
        LINEAGE.with(|lineage| lineage.borrow_mut().push(flight));
        let (value, bytes) = build()?;
        Ok(landing.land(value, bytes, read))
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
    landed: bool,
    marker: PhantomData<fn() -> (Marker, S)>,
}

impl<Marker, K, S> Landing<'_, Marker, K, S>
where
    Marker: 'static,
    K: Hash + Eq + Send + Sync + 'static,
    S: Send + 'static,
{
    fn land<R>(mut self, value: S, bytes: u64, read: impl FnOnce(&S) -> R) -> R {
        // Should reading panic, the build is dropped unlanded.
        let answer = read(&value);
        self.landed = true;
        let mut state = self.store.lock();
        // A value put in its place meanwhile stays: the build keeps its own.
        if let Some(table) = state.existing::<Marker, K, S>()
            && let Some(slot) = table.entries.get_mut(&*self.key)
            && let Slot::Building { flight, waiters } = *slot
            && flight == self.flight
        {
            *slot = Slot::Ready { value, bytes };
            table.statistics.bytes += bytes;
            if waiters > 0 {
                self.store.0.ended.notify_all();
            }
        }
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

    /// `Marker`'s table, made under `kind`'s name when it has none.
    fn table<Marker: 'static, K, S>(&mut self, kind: &'static str) -> &mut Table<K, S>
    where
        K: Send + Sync + 'static,
        S: Send + 'static,
    {
        self.tables
            .entry(TypeId::of::<(Marker, K, S)>())
            .or_insert_with(|| {
                Box::new(Table::<K, S> {
                    entries: HashMap::default(),
                    statistics: DerivationStatistics {
                        kind,
                        ..DerivationStatistics::default()
                    },
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
