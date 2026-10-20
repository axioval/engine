//! What the adapter's derivations share (ADR 0006): the identity of the
//! state a value is built over, the kernel crates their versions name
//! (#439), and how much of a kind the check's store keeps.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axioval_engine::{Kernel, memory};

use crate::AxiolidGeometry;

/// The kernel crates the adapter's derivations call, at the versions their
/// versions are stated for: the ones the workspace locks, which every
/// kind's key-completeness test holds them to. Upgrading one means
/// stating it here, and bumping the revision of each kind naming it whose
/// results can change.
pub(crate) const AXIOLID_CORE: Kernel = Kernel {
    name: "axiolid-core",
    version: "0.3.2",
};
pub(crate) const AXIOLID_INSPECT: Kernel = Kernel {
    name: "axiolid-inspect",
    version: "0.3.6",
};
pub(crate) const AXIOLID_MEASURE: Kernel = Kernel {
    name: "axiolid-measure",
    version: "0.3.10",
};
pub(crate) const AXIOLID_MESH: Kernel = Kernel {
    name: "axiolid-mesh",
    version: "0.3.3",
};
pub(crate) const AXIOLID_OVERLAY: Kernel = Kernel {
    name: "axiolid-overlay",
    version: "0.3.18",
};
pub(crate) const AXIOLID_RAY_MESH: Kernel = Kernel {
    name: "axiolid-ray-mesh",
    version: "0.4.0",
};
pub(crate) const AXIOLID_SPATIAL: Kernel = Kernel {
    name: "axiolid-spatial",
    version: "0.3.3",
};

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

/// A geometry set in a key, compared and hashed by the identity of what
/// its clones share (#450): keys over one set, through whichever service
/// holds a clone of it, are equal in this field; keys over two sets never,
/// whatever those hold, a clone changed after cloning included (it then
/// holds a copy of its own). The key holds the set, so the address is not
/// reused, nor what it holds changed, while the key lives.
#[derive(Clone)]
pub(crate) struct OverGeometry(pub(crate) AxiolidGeometry);

impl PartialEq for OverGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.0.same(&other.0)
    }
}

impl Eq for OverGeometry {}

impl Hash for OverGeometry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.address().hash(state);
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
