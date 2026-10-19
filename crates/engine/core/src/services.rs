//! Type-safe host service registration.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;

/// Service registration failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ServiceRegistryError {
    #[error("a service of this type is already registered")]
    Duplicate,
}

/// Immutable type-indexed services supplied by an application or adapter.
#[derive(Clone, Default)]
pub struct ServiceRegistry {
    entries: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// The service types a host watches for ([`Self::watch`]), shared by
    /// every copy of the registry.
    watch: Option<ServiceWatch>,
}
impl ServiceRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers one service without allowing silent replacement.
    pub fn register<T: Any + Send + Sync>(
        &mut self,
        service: T,
    ) -> Result<(), ServiceRegistryError> {
        if self.entries.contains_key(&TypeId::of::<T>()) {
            return Err(ServiceRegistryError::Duplicate);
        }
        self.entries.insert(TypeId::of::<T>(), Arc::new(service));
        Ok(())
    }
    /// Registers a service, replacing any existing one of the same type.
    ///
    /// Crate-private: only the engine may overwrite, and only for services it
    /// derives per run and must not accept from a host.
    pub(crate) fn replace<T: Any + Send + Sync>(&mut self, service: T) {
        self.entries.insert(TypeId::of::<T>(), Arc::new(service));
    }
    /// Number of registered services.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    /// Looks up a service by its concrete interface type.
    ///
    /// A type the registry watches ([`Self::watch`]) is noted as asked for,
    /// whether or not a service of it is registered.
    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        if let Some(watch) = &self.watch {
            watch.note(id);
        }
        self.entries.get(&id)?.downcast_ref()
    }
    /// Looks up a service without noting it on a watch ([`Self::watch`]):
    /// for code that holds every service it may use and notes a watch when
    /// it uses one ([`Self::watching`]).
    pub(crate) fn peek<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.entries.get(&TypeId::of::<T>())?.downcast_ref()
    }
    /// The watch on this registry, if any.
    pub(crate) fn watching(&self) -> Option<&ServiceWatch> {
        self.watch.as_ref()
    }
    /// Watches `types`: every later look-up of one ([`Self::get`]), on this
    /// registry or any copy of it, is noted on the returned watch, found or
    /// not. A host that left services out of a run because the plan should
    /// never ask for them reads the watch afterwards to learn whether some
    /// rule did, and runs again with them. Watching anew replaces the
    /// earlier watch.
    pub fn watch(&mut self, types: Vec<(TypeId, &'static str)>) -> ServiceWatch {
        let watch = ServiceWatch(
            types
                .into_iter()
                .map(|(id, name)| (id, name, AtomicBool::new(false)))
                .collect(),
        );
        self.watch = Some(watch.clone());
        watch
    }
}

/// Service types a host watches on a registry
/// ([`ServiceRegistry::watch`]), and which of them were asked for.
#[derive(Clone)]
pub struct ServiceWatch(Arc<[(TypeId, &'static str, AtomicBool)]>);

impl ServiceWatch {
    /// The watched type `T` with its name, for [`ServiceRegistry::watch`].
    #[must_use]
    pub fn of<T: Any>() -> (TypeId, &'static str) {
        (TypeId::of::<T>(), std::any::type_name::<T>())
    }

    fn note(&self, id: TypeId) {
        if let Some((_, _, asked)) = self.0.iter().find(|(watched, _, _)| *watched == id) {
            asked.store(true, Ordering::Relaxed);
        }
    }

    /// Notes every watched type as asked for: what code that holds services
    /// looked up without noting ([`ServiceRegistry::peek`]) does when it may
    /// use any of them.
    pub(crate) fn note_every(&self) {
        for (_, _, asked) in self.0.iter() {
            asked.store(true, Ordering::Relaxed);
        }
    }

    /// The names of the watched types asked for so far, in watch order.
    #[must_use]
    pub fn asked(&self) -> Vec<&'static str> {
        self.0
            .iter()
            .filter(|(_, _, asked)| asked.load(Ordering::Relaxed))
            .map(|(_, name, _)| *name)
            .collect()
    }
}

impl std::fmt::Debug for ServiceWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(_, name, _)| name))
            .finish()
    }
}

/// Evidence an adapter may present as fact.
///
/// Exact, and carrying a locator a human can follow back to the source. Both
/// halves matter: an exact measurement nobody can audit is not reviewable, and
/// a well-located estimate is not exact. Owned here so every service applies
/// the same admission test.
pub(crate) fn reviewable_exact_evidence(evidence: &axioval_ir::Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::{ServiceRegistry, ServiceWatch};

    struct Watched;
    struct Other;

    #[test]
    fn a_watch_notes_every_look_up_of_its_types_on_every_copy() {
        let mut registry = ServiceRegistry::new();
        registry.register(Other).unwrap();
        let watch = registry.watch(vec![ServiceWatch::of::<Watched>()]);
        let copy = registry.clone();
        assert!(copy.get::<Other>().is_some());
        assert!(registry.peek::<Watched>().is_none());
        assert!(watch.asked().is_empty());
        // Not registered, and asked for all the same.
        assert!(copy.get::<Watched>().is_none());
        assert_eq!(watch.asked(), [std::any::type_name::<Watched>()]);
    }
}
