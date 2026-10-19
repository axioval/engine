//! What the adapter's derivations share (ADR 0006): the identity of the
//! state a value is built over, and how much of a kind the check's store
//! keeps.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axioval_engine::memory;

/// A value's state, compared and hashed by its address: two keys over one
/// service's state are equal in this field, keys over two services never,
/// whatever those hold. The key holds the state, so its address is not
/// reused while the key lives.
pub(crate) struct Over<T>(pub(crate) Arc<T>);

impl<T> Clone for Over<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for Over<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for Over<T> {}

impl<T> Hash for Over<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// What the store keeps of one of the adapter's kinds at most without a
/// memory budget: 1 GiB.
const KEEP_WITHOUT_BUDGET: u64 = 1 << 30;

/// What the store keeps of one of the adapter's kinds at most: a quarter
/// of the check's memory budget, or 1 GiB without one. Past it, the values
/// read least recently are released and built again when read again.
pub(crate) fn keep_bytes() -> u64 {
    memory::current().map_or(KEEP_WITHOUT_BUDGET, |budget| budget.limit() / 4)
}
