//! What holds a derivation's key to all of its inputs
//! ([#439](https://github.com/axioval/engine/issues/439)): the version a
//! kind states ([`DerivationVersion`]) and the key-completeness test every
//! kind has ([`KeyCompleteness`]).
//!
//! A value is a pure function of its key, so an input missing from the key
//! hands one question's answer to another, the one way reuse can change a
//! verdict. Every kind therefore states a version its crate bumps whenever
//! the same key could build another value, naming the kernel crates its
//! build calls, and has a test that changes each input of its key in turn.
//! The architecture gate (`scripts/architecture.py`) refuses a kind
//! without such a test.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{DerivationKind, DerivationStore};
use crate::ServiceRegistry;

/// A kernel crate a derivation's build calls, at the version its results
/// are stated for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Kernel {
    /// The crate's name, as published.
    pub name: &'static str,
    /// The version the derivation's results are stated for.
    pub version: &'static str,
}

/// The version of a derivation kind ([`DerivationKind::VERSION`]): a
/// revision its crate bumps whenever the same key could build another value
/// (the build or anything it calls changing what it returns), and the
/// kernel crates its build calls, each at the version the revision is
/// stated for.
///
/// Within a check every value of a kind is of one version. A value kept
/// across checks (#409) is read only under the version it was built under,
/// so a kernel upgrade, which names another version here, rebuilds it.
/// [`DerivationVersion::unlocked`] holds the kernels named to the versions a
/// workspace locks, so an upgrade that leaves the kind's version alone
/// fails its crate's tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DerivationVersion {
    /// Bumped whenever the same key could build another value.
    pub revision: u32,
    /// The kernel crates the build calls, by name.
    pub kernels: &'static [Kernel],
}

impl DerivationVersion {
    /// Revision `revision` over `kernels`.
    #[must_use]
    pub const fn new(revision: u32, kernels: &'static [Kernel]) -> Self {
        Self { revision, kernels }
    }

    /// The kernels named at another version than the one `lock` (the text
    /// of a `Cargo.lock`) holds for them, or not held at all.
    #[must_use]
    pub fn unlocked(&self, lock: &str) -> Vec<Kernel> {
        let locked = locked_packages(lock);
        self.kernels
            .iter()
            .filter(|kernel| !locked.contains(&(kernel.name, kernel.version)))
            .copied()
            .collect()
    }
}

impl fmt::Display for DerivationVersion {
    /// `3`, or `3 (axiolid-core 0.3.2, axiolid-mesh 0.3.3)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.revision)?;
        for (index, kernel) in self.kernels.iter().enumerate() {
            let open = if index == 0 { " (" } else { ", " };
            write!(f, "{open}{} {}", kernel.name, kernel.version)?;
        }
        if !self.kernels.is_empty() {
            write!(f, ")")?;
        }
        Ok(())
    }
}

/// Each package's name and version in the text of a `Cargo.lock`.
fn locked_packages(lock: &str) -> Vec<(&str, &str)> {
    fn quoted<'a>(line: &'a str, field: &str) -> Option<&'a str> {
        line.strip_prefix(field)
            .and_then(|rest| rest.trim().strip_prefix('='))
            .map(|value| value.trim().trim_matches('"'))
    }
    let mut packages = Vec::new();
    let mut name = None;
    for line in lock.lines().map(str::trim) {
        if line == "[[package]]" {
            name = None;
        } else if let Some(value) = quoted(line, "name ") {
            name = Some(value);
        } else if let (Some(value), Some(package)) = (quoted(line, "version "), name.take()) {
            packages.push((package, value));
        }
    }
    packages
}

/// A key-completeness test of kind `D` (#439): from a key of known inputs,
/// each input is changed in turn, and the key has to change with it; where
/// the input affects the value, the value has to change too. Values are
/// compared by a fingerprint the test chooses (what of a value can be
/// compared), each built in a store of its own, so none is reused.
///
/// Every kind on a store has one: the architecture gate refuses a
/// `DerivationKind` impl whose crate never names it as
/// `KeyCompleteness::<Kind`.
///
/// ```
/// use axioval_engine::{
///     DerivationError, DerivationKind, DerivationVersion, Inputs, KeyCompleteness,
/// };
///
/// struct Scaled;
///
/// impl DerivationKind for Scaled {
///     const NAME: &'static str = "scaled";
///     const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
///     /// A value and its factor's bits.
///     type Key = (u32, u64);
///     type Value = f64;
///
///     fn build(&(value, factor): &(u32, u64), _: &Inputs<'_>) -> Result<f64, DerivationError> {
///         Ok(f64::from(value) * f64::from_bits(factor))
///     }
/// }
///
/// KeyCompleteness::<Scaled, _>::new((2, 1.5_f64.to_bits()), |value| value.to_bits())
///     .same("the same inputs", (2, 1.5_f64.to_bits()))
///     .changes_value("the value", (3, 1.5_f64.to_bits()))
///     .changes_value("the factor", (2, 2.5_f64.to_bits()))
///     .check();
/// ```
pub struct KeyCompleteness<D: DerivationKind, F> {
    base: D::Key,
    services: ServiceRegistry,
    fingerprint: fn(&D::Value) -> F,
    inputs: usize,
    /// Whether a key in another precision class was asked ([`Self::precision`]).
    classes: bool,
    failures: Vec<String>,
}

// A test states each changed key once, in place: taking it by value reads
// as the list of inputs it is.
#[allow(clippy::needless_pass_by_value)]
impl<D, F> KeyCompleteness<D, F>
where
    D: DerivationKind,
    F: PartialEq + fmt::Debug,
{
    /// A test of the keys of `D` against `base`, values compared by
    /// `fingerprint`; builds read no service unless
    /// [`Self::with_services`] gives some.
    #[must_use]
    pub fn new(base: D::Key, fingerprint: fn(&D::Value) -> F) -> Self {
        let copy = base.clone();
        let mut failures = Vec::new();
        if copy != base || hash(&copy) != hash(&base) {
            failures.push("a copy of the key differs from it or hashes otherwise".to_owned());
        }
        Self {
            base,
            services: ServiceRegistry::new(),
            fingerprint,
            inputs: 0,
            classes: false,
            failures,
        }
    }

    /// The services every build of the test reads.
    #[must_use]
    pub fn with_services(mut self, services: ServiceRegistry) -> Self {
        self.services = services;
        self
    }

    /// `key`, built from the same inputs as the base key, is equal to it,
    /// hashes alike and builds the same value.
    #[must_use]
    pub fn same(mut self, what: &str, key: D::Key) -> Self {
        if key != self.base {
            self.failures
                .push(format!("{what}: the key differs from the base key"));
        } else if hash(&key) != hash(&self.base) {
            self.failures.push(format!(
                "{what}: the key equals the base key but hashes otherwise"
            ));
        } else if self.value(&key) != self.value(&self.base) {
            self.failures
                .push(format!("{what}: an equal key builds another value"));
        }
        self
    }

    /// `key`, the base key with only the input `what` changed, differs from
    /// the base key. Its value is not compared: for an input that does not
    /// change the value the test's keys build (an identity of equal
    /// content, an order the build sorts), or that no fixture can show.
    #[must_use]
    pub fn input(mut self, what: &str, key: D::Key) -> Self {
        self.inputs += 1;
        if key == self.base {
            self.failures
                .push(format!("changing {what} leaves the key alike"));
        }
        self
    }

    /// As [`Self::input`], and the value `key` builds differs from the base
    /// key's too: the input affects the value.
    #[must_use]
    pub fn changes_value(mut self, what: &str, key: D::Key) -> Self {
        self.inputs += 1;
        if key == self.base {
            self.failures
                .push(format!("changing {what} leaves the key alike"));
        } else if self.value(&key) == self.value(&self.base) {
            self.failures.push(format!(
                "changing {what} changes the key but not the value: the test does not show \
                 what the input affects"
            ));
        }
        self
    }

    /// `key`, the base key in another precision class (#438), is another
    /// entry of one store: asked first, its value never answers the base
    /// key, whose read builds a value of its own, and each key reads its
    /// own value again. Its value is not compared: a Fast value may equal
    /// the Exact one where the fast mode has nothing to fill.
    ///
    /// # Panics
    ///
    /// When a build fails with an error of the store's (a cycle, or a value
    /// its class does not hold).
    #[must_use]
    pub fn precision(mut self, what: &str, key: D::Key) -> Self {
        self.inputs += 1;
        self.classes = true;
        let (asked, base) = (D::precision(&key), D::precision(&self.base));
        if key == self.base {
            self.failures
                .push(format!("changing {what} leaves the key alike"));
            return self;
        }
        if asked.is_none() || asked == base {
            self.failures.push(format!(
                "changing {what} leaves the precision class alike ({asked:?}, the base key's \
                 {base:?})"
            ));
            return self;
        }
        let store = DerivationStore::new();
        let read = |key: &D::Key| {
            store
                .derive::<D>(&self.services, key)
                .unwrap_or_else(|error| panic!("building a key of `{}`: {error}", D::NAME))
        };
        let other = read(&key);
        if store.held::<D>(&self.base).is_some() {
            self.failures.push(format!(
                "{what}: the base key finds the value built for the other class"
            ));
        }
        let first = read(&self.base);
        let entries: usize = store
            .statistics()
            .iter()
            .filter(|statistics| statistics.kind == D::NAME)
            .map(|statistics| statistics.entries)
            .sum();
        if Arc::ptr_eq(&first, &other) || entries != 2 {
            self.failures.push(format!(
                "{what}: the base key is answered by the other class's value ({entries} entries)"
            ));
        }
        if !Arc::ptr_eq(&read(&self.base), &first) || !Arc::ptr_eq(&read(&key), &other) {
            self.failures.push(format!(
                "{what}: a key does not read its own class's value again"
            ));
        }
        self
    }

    /// Fails, naming every input that failed, when one did or when no input
    /// was changed, or when the kind keys a precision class
    /// ([`DerivationKind::precision`]) and no key in another class was
    /// asked ([`Self::precision`]); and when the kind names a kernel crate at another
    /// version than the `Cargo.lock` above the working directory (the
    /// workspace's, under `cargo test`) locks, or no lock is found.
    ///
    /// # Panics
    ///
    /// On any failure.
    pub fn check(mut self) {
        assert!(
            self.inputs > 0,
            "the key-completeness test of `{}` changes no input",
            D::NAME
        );
        if D::precision(&self.base).is_some() && !self.classes {
            self.failures.push(
                "it keys a precision class, and no key in another class is asked \
                 (`KeyCompleteness::precision`)"
                    .to_owned(),
            );
        }
        if !D::VERSION.kernels.is_empty() {
            match workspace_lock() {
                Some(lock) => {
                    for kernel in D::VERSION.unlocked(&lock) {
                        self.failures.push(format!(
                            "its version names {} {}, which the workspace does not lock: bump \
                             the revision if its results can change, and name the locked version",
                            kernel.name, kernel.version
                        ));
                    }
                }
                None => self
                    .failures
                    .push("it names kernel crates, and no Cargo.lock is found".to_owned()),
            }
        }
        assert!(
            self.failures.is_empty(),
            "the key of `{}` is incomplete:\n- {}",
            D::NAME,
            self.failures.join("\n- ")
        );
    }

    /// The fingerprint of `key`'s value, built in a store of its own.
    fn value(&self, key: &D::Key) -> F {
        let value = DerivationStore::new()
            .derive::<D>(&self.services, key)
            .unwrap_or_else(|error| panic!("building a key of `{}`: {error}", D::NAME));
        (self.fingerprint)(&value)
    }
}

/// The text of the first `Cargo.lock` in the working directory or above it.
fn workspace_lock() -> Option<String> {
    let directory = std::env::current_dir().ok()?;
    directory
        .ancestors()
        .find_map(|directory| std::fs::read_to_string(directory.join("Cargo.lock")).ok())
}

fn hash<K: Hash>(key: &K) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_names_its_kernels_and_finds_those_not_locked() {
        const KERNELS: &[Kernel] = &[
            Kernel {
                name: "kernel-a",
                version: "0.3.2",
            },
            Kernel {
                name: "kernel-b",
                version: "1.0.0",
            },
        ];
        let version = DerivationVersion::new(3, KERNELS);
        assert_eq!(version.to_string(), "3 (kernel-a 0.3.2, kernel-b 1.0.0)");
        assert_eq!(DerivationVersion::new(1, &[]).to_string(), "1");
        let lock = "version = 4\n\n[[package]]\nname = \"kernel-a\"\nversion = \"0.3.2\"\n\
                    source = \"registry\"\n\n[[package]]\nname = \"kernel-b\"\n\
                    version = \"1.0.1\"\n";
        assert_eq!(version.unlocked(lock), vec![KERNELS[1]]);
        assert_eq!(version.unlocked(""), KERNELS.to_vec());
    }
}
