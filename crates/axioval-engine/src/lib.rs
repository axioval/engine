//! Trusted capability compilation and deterministic runtime.
#![forbid(unsafe_code)]
#![allow(missing_docs, clippy::missing_errors_doc)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_ir::contract as schema;
use axioval_ir::{Finding, Project, Report};
use thiserror::Error;

/// Errors while compiling untrusted declarations into a trusted execution plan.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    /// A package declares a schema version this compiler does not implement.
    #[error(
        "unsupported schema version `{version}` for {package_kind} `{package_id}`; supported: {supported}"
    )]
    UnsupportedSchemaVersion {
        package_kind: &'static str,
        package_id: String,
        version: String,
        supported: &'static str,
    },
    /// Multiple supplied definition packages declared the same package identity.
    #[error("duplicate definition package `{0}`")]
    DuplicateDefinitionPackage(String),
    /// A capability was not registered by the host.
    #[error("unknown capability `{0}`")]
    UnknownCapability(String),
    /// Two trusted implementations claimed an ID.
    #[error("duplicate capability `{0}`")]
    DuplicateCapability(String),
    /// A package supplied a non-declared parameter.
    #[error("capability `{capability}` does not declare parameter `{parameter}`")]
    UnknownParameter {
        capability: String,
        parameter: String,
    },
    /// A required parameter was absent.
    #[error("capability `{capability}` requires parameter `{parameter}`")]
    MissingParameter {
        capability: String,
        parameter: String,
    },
    /// A binding type did not conform to its descriptor.
    #[error("capability `{capability}` parameter `{parameter}` has invalid type")]
    InvalidParameterType {
        capability: String,
        parameter: String,
    },
    /// A rule binds a parameter more than once.
    #[error("rule has duplicate parameter binding `{0}`")]
    DuplicateBinding(String),
    /// A rule references no loaded definition.
    #[error("unknown rule definition `{0}`")]
    UnknownDefinition(String),
    /// A ruleset references a definition package that was not supplied.
    #[error("missing definition package `{0}`")]
    MissingDefinitionPackage(String),
    /// A trusted capability descriptor conflicts with its portable definition.
    #[error("definition `{definition}` conflicts with capability `{capability}`: {detail}")]
    CapabilityContract {
        definition: String,
        capability: String,
        detail: String,
    },
    /// Rule IDs must be unique throughout the recursive folder tree.
    #[error("duplicate rule id `{0}`")]
    DuplicateRule(String),
}

/// Supported declarative parameter types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterType {
    Boolean,
    Integer,
    Number,
    String,
    Quantity,
    Enum,
    Reference,
    ObjectTypeReference,
    PropertyReference,
    StringList,
    ReferenceList,
}
impl ParameterType {
    fn accepts(self, value: &schema::ParameterValue) -> bool {
        matches!(
            (self, value),
            (Self::Boolean, schema::ParameterValue::Boolean { .. })
                | (Self::Integer, schema::ParameterValue::Integer { .. })
                | (Self::Number, schema::ParameterValue::Number { .. })
                | (Self::String, schema::ParameterValue::String { .. })
                | (Self::Quantity, schema::ParameterValue::Quantity { .. })
                | (Self::Enum, schema::ParameterValue::Enum { .. })
                | (Self::Reference, schema::ParameterValue::Reference { .. })
                | (
                    Self::ObjectTypeReference,
                    schema::ParameterValue::ObjectTypeReference { .. }
                )
                | (
                    Self::PropertyReference,
                    schema::ParameterValue::PropertyReference { .. }
                )
                | (Self::StringList, schema::ParameterValue::StringList { .. })
                | (
                    Self::ReferenceList,
                    schema::ParameterValue::ReferenceList { .. }
                )
        )
    }
}
/// Trusted capability parameter descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterDescriptor {
    pub name: String,
    pub parameter_type: ParameterType,
    pub required: bool,
}
impl ParameterDescriptor {
    /// Required parameter descriptor.
    pub fn required(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: true,
        }
    }
    /// Optional parameter descriptor.
    pub fn optional(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: false,
        }
    }
}

/// One validated portable rule bound to trusted executable capability code.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledRule {
    /// Package-local stable rule ID.
    pub id: String,
    /// Registered capability ID.
    pub capability: String,
    /// Rule severity.
    pub severity: schema::Severity,
    /// Source-neutral applicability selector.
    pub selector: schema::Selector,
    /// Strictly validated parameter bindings.
    pub parameters: BTreeMap<String, schema::ParameterValue>,
}

/// Source-neutral data and typed host services visible during one rule evaluation.
pub struct RuleContext<'a> {
    /// Immutable composed project view.
    pub project: &'a Project,
    /// Adapter-provided semantic and computational capabilities.
    pub services: &'a ServiceRegistry,
}

/// Trusted code selected by a package capability ID; packages never supply executable code.
pub trait RuleCapability: Send + Sync {
    /// Stable trusted capability ID.
    fn id(&self) -> &'static str;
    /// Strict accepted parameters.
    fn parameters(&self) -> Vec<ParameterDescriptor>;
    /// Evaluates an already-validated rule request.
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> Vec<Finding>;
}

/// Host-controlled registry of trusted capabilities.
#[derive(Clone, Default)]
pub struct CapabilityRegistry {
    capabilities: BTreeMap<String, Arc<dyn RuleCapability>>,
}
impl CapabilityRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers a capability; duplicate IDs are rejected.
    pub fn register<C: RuleCapability + 'static>(
        mut self,
        capability: C,
    ) -> Result<Self, EngineError> {
        let id = capability.id().to_owned();
        if self
            .capabilities
            .insert(id.clone(), Arc::new(capability))
            .is_some()
        {
            return Err(EngineError::DuplicateCapability(id));
        }
        Ok(self)
    }
    /// Gets trusted code by exact ID.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn RuleCapability>> {
        self.capabilities.get(id)
    }
}

/// Validated, deterministic request plan.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    rules: Vec<CompiledRule>,
}
impl ExecutionPlan {
    /// Rules ordered by stable rule ID.
    pub fn rules(&self) -> &[CompiledRule] {
        &self.rules
    }
}

mod compiler;
mod metric_routing;
mod services;
mod topology;
pub use compiler::{SUPPORTED_SCHEMA_VERSION, compile};
pub use metric_routing::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, LengthInterval, MetricPoint,
    MetricRouteEvidence, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingService, MetricRoutingServiceHandle, MobilityProfile, ThresholdVerdict,
};
pub use services::{ServiceRegistry, ServiceRegistryError};
pub use topology::{
    CompleteTopologyEvidence, ConnectivityGraph, RouteOutcome, TopologyError, VerifiedConnection,
};

/// Deterministic runtime that invokes only registered trusted capabilities.
pub struct Runtime {
    registry: CapabilityRegistry,
    services: ServiceRegistry,
}
impl Runtime {
    /// Creates a runtime from a host-controlled registry.
    pub fn new(registry: CapabilityRegistry) -> Self {
        Self {
            registry,
            services: ServiceRegistry::new(),
        }
    }
    /// Adds adapter-provided host services to subsequent evaluations.
    #[must_use]
    pub fn with_services(mut self, services: ServiceRegistry) -> Self {
        self.services = services;
        self
    }
    /// Executes a plan and returns deterministically sorted findings.
    ///
    /// Execution fails closed if the host registry no longer contains any capability
    /// that was present when the plan was compiled.
    pub fn run(&self, project: &Project, plan: ExecutionPlan) -> Result<Report, EngineError> {
        let context = RuleContext {
            project,
            services: &self.services,
        };
        let mut findings = Vec::new();
        for rule in plan.rules {
            let capability = self
                .registry
                .get(&rule.capability)
                .ok_or_else(|| EngineError::UnknownCapability(rule.capability.clone()))?;
            findings.extend(capability.evaluate(&context, &rule));
        }
        findings.sort_by(|a, b| {
            a.rule_id
                .cmp(&b.rule_id)
                .then_with(|| a.object_id.cmp(&b.object_id))
                .then_with(|| a.message.cmp(&b.message))
        });
        Ok(Report { findings })
    }
}
