//! Trusted built-in code measuring registered values the engine core does
//! not measure itself: values that reuse a capability's own measurement
//! (a distance in a mode, a clear width with its deductions, a coverage
//! share), so a measured value and the capability never disagree.
//!
//! A provider is registered beside the capabilities, in the
//! [`CapabilityRegistry`](crate::CapabilityRegistry), never by a package.
//! Each name it measures must be in the registry of measured values
//! ([`axioval_ir::measured`]) and measured by nothing else.

use std::collections::BTreeMap;
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

/// One field of a measured member.
#[derive(Clone, Debug, PartialEq)]
pub enum MemberValue {
    /// A number, or `null` ([`Measurement::Absent`]).
    Measured(Measurement),
    /// A truth, stated exactly.
    Truth {
        /// The truth.
        value: bool,
        /// Where it came from, for its evidence.
        locator: String,
    },
    /// A field the measurement could not decide; reading it leaves the
    /// expression not evaluated.
    Undecided {
        /// Why, in plain words.
        why: String,
    },
}

/// One measured member: a flight's step, a ramp's run.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredMember {
    /// Whether it surely is a member; `false` when the measurement could
    /// not decide it.
    pub certain: bool,
    /// Whether the measurement it comes from is exact: its fields' evidence
    /// is then exact, an interval holding only the rounding of exact
    /// arithmetic.
    pub exact: bool,
    /// Every field its list declares, by name.
    pub fields: BTreeMap<&'static str, MemberValue>,
}

/// Trusted code measuring registered values.
pub trait MeasuredProvider: Send + Sync + 'static {
    /// The registered names it measures.
    fn names(&self) -> &'static [&'static str];

    /// The registered member lists it measures.
    fn member_lists(&self) -> &'static [&'static str] {
        &[]
    }

    /// Measures the members `call` (one of [`Self::member_lists`]) lists
    /// of `object`, in the list's order.
    ///
    /// # Errors
    ///
    /// As [`Self::measure`].
    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let _ = (object, context);
        Err(PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}`",
            call.name()
        )))
    }

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

/// The members of `object` the measured member list `name` lists, as a run
/// measures them.
///
/// # Errors
///
/// An unknown or malformed list, a list no registered code measures, or a
/// measurement that cannot be made.
pub fn measured_members(
    services: &ServiceRegistry,
    object: &ObjectId,
    name: &str,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let call = axioval_ir::measured::parse_members(name)
        .map_err(|error| PropertyResolutionError::Unavailable(error.to_string()))?;
    let providers = services.get::<Providers>().ok_or_else(|| {
        PropertyResolutionError::MissingService(format!(
            "no built-in code measures `{}` outside a run",
            call.name()
        ))
    })?;
    let provider = providers
        .providers
        .iter()
        .find(|provider| provider.member_lists().contains(&call.name()))
        .ok_or_else(|| {
            PropertyResolutionError::MissingService(format!(
                "no built-in code measures `{}`",
                call.name()
            ))
        })?;
    let context = RuleContext {
        project: &providers.project,
        services,
    };
    provider.members(&call, object, &context)
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
