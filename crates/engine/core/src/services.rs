//! Type-safe host service registration.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
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
    /// Looks up a service by its concrete interface type.
    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.entries.get(&TypeId::of::<T>())?.downcast_ref()
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
