//! Trusted built-in code measuring registered values the engine core does
//! not measure itself: values that reuse a capability's own measurement
//! (a distance in a mode, a clear width with its deductions, a coverage
//! share), so a measured value and the capability never disagree.
//!
//! A provider is registered beside the capabilities, in the
//! [`CapabilityRegistry`](crate::CapabilityRegistry), never by a package.
//! Each name it measures must be in the registry of measured values
//! ([`axioval_ir::measured`]) and measured by nothing else.

use std::sync::Arc;

use axioval_ir::measured::MeasuredCall;
use axioval_ir::{ObjectId, Project, QuantityDimension};

use crate::properties::PropertyResolutionError;
use crate::{RuleContext, ServiceRegistry};

/// What a provider measured.
#[derive(Clone, Debug, PartialEq)]
pub enum Measurement {
    /// A value sure to lie in `[lower, upper]`, a point when exact.
    Value {
        /// The least value it may have.
        lower: f64,
        /// The greatest value it may have.
        upper: f64,
        /// The value's dimension; `None` for a plain number, such as a
        /// count.
        dimension: Option<QuantityDimension>,
        /// Where the measurement came from, for its evidence.
        locator: String,
    },
    /// No value, known exactly: a path reaching nothing, a space with no
    /// obstacle above.
    Absent {
        /// Why there is none, for its evidence.
        locator: String,
    },
}

/// Trusted code measuring registered values.
pub trait MeasuredProvider: Send + Sync + 'static {
    /// The registered names it measures.
    fn names(&self) -> &'static [&'static str];

    /// Measures `call` (one of [`Self::names`]) of `object`.
    ///
    /// # Errors
    ///
    /// As a property resolution: a missing service, an unreadable object, a
    /// measurement that cannot be made.
    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError>;
}

/// The providers of a run, by name, with the run's project and services.
#[derive(Clone)]
pub(crate) struct Providers {
    pub(crate) providers: Arc<Vec<Arc<dyn MeasuredProvider>>>,
    pub(crate) project: Arc<Project>,
}

impl Providers {
    /// The provider measuring `name`.
    pub(crate) fn of(&self, name: &str) -> Option<&Arc<dyn MeasuredProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.names().contains(&name))
    }
}

/// Installs `providers` for a run over `project`, if there are any.
pub(crate) fn install(
    services: &mut ServiceRegistry,
    providers: &[Arc<dyn MeasuredProvider>],
    project: &Project,
) {
    if providers.is_empty() {
        return;
    }
    services.replace(Providers {
        providers: Arc::new(providers.to_vec()),
        project: Arc::new(project.clone()),
    });
}
