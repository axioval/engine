//! Exact source-neutral property-resolution host-service contracts.

use axioval_ir::{Evidence, ObjectId, Property, PropertyValue};
use std::sync::Arc;
use thiserror::Error;

/// Failure to resolve a property conclusively.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PropertyResolutionError {
    /// The requested property reference is malformed.
    #[error("property request is invalid")]
    InvalidRequest,
    /// Returned data names another object or property.
    #[error("property response does not match its request")]
    ResponseRequestMismatch,
    /// A conclusive answer lacks exact, reviewable provenance.
    #[error("property evidence is not exact and reviewable")]
    InexactEvidence,
    /// A conclusive answer contains a non-finite numeric value.
    #[error("property value is not finite")]
    InvalidValue,
    /// The source cannot currently provide a conclusive answer.
    #[error("property resolution unavailable: {0}")]
    Unavailable(String),
}

/// Request for one property on one source-qualified object.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PropertyRequest {
    object_id: ObjectId,
    property_set: Option<String>,
    property: String,
}
impl PropertyRequest {
    /// Creates a request. An omitted set requests an unambiguous property by name.
    pub fn try_new(
        object_id: ObjectId,
        property_set: Option<String>,
        property: impl Into<String>,
    ) -> Result<Self, PropertyResolutionError> {
        let property = property.into();
        if property.trim().is_empty()
            || property_set
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Ok(Self {
            object_id,
            property_set,
            property,
        })
    }
    /// Requested object.
    pub fn object_id(&self) -> &ObjectId {
        &self.object_id
    }
    /// Optional requested property set.
    pub fn property_set(&self) -> Option<&str> {
        self.property_set.as_deref()
    }
    /// Requested property name.
    pub fn property(&self) -> &str {
        &self.property
    }
    fn matches(&self, property: &Property) -> bool {
        property.name == self.property
            && self
                .property_set
                .as_ref()
                .is_none_or(|set| property.property_set == *set)
    }
}

/// Exact proof that a requested property is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletePropertyAbsenceEvidence {
    request: PropertyRequest,
    evidence: Evidence,
}
impl CompletePropertyAbsenceEvidence {
    /// Creates request-bound exact absence evidence.
    pub fn try_new(
        request: PropertyRequest,
        evidence: Evidence,
    ) -> Result<Self, PropertyResolutionError> {
        if !reviewable(&evidence) {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        Ok(Self { request, evidence })
    }
    /// Bound request.
    pub fn request(&self) -> &PropertyRequest {
        &self.request
    }
    /// Exact reviewable provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Exact property value bound to the request that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProperty {
    request: PropertyRequest,
    property: Property,
}
impl ResolvedProperty {
    /// Creates an exact request-bound property value.
    pub fn try_new(
        request: PropertyRequest,
        property: Property,
    ) -> Result<Self, PropertyResolutionError> {
        if !request.matches(&property) {
            return Err(PropertyResolutionError::ResponseRequestMismatch);
        }
        if !property.evidence.as_ref().is_some_and(reviewable) {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        if !valid_value(&property.value) {
            return Err(PropertyResolutionError::InvalidValue);
        }
        Ok(Self { request, property })
    }
    /// Bound request, including the source-qualified object identity.
    pub fn request(&self) -> &PropertyRequest {
        &self.request
    }
    /// Exact typed property and its reviewable provenance.
    pub fn property(&self) -> &Property {
        &self.property
    }
}

/// Conclusive property result from a trusted source adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyResolution {
    /// The exact request-bound property value and its provenance.
    Present(ResolvedProperty),
    /// Exact proof that the requested property is absent.
    Absent(CompletePropertyAbsenceEvidence),
}

/// Trusted adapter seam for property resolution.
pub trait PropertyResolutionService: Send + Sync {
    /// Resolves one request or reports why it is not conclusive.
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError>;
}

/// Cloneable, type-erased property service registered by the host.
#[derive(Clone)]
pub struct PropertyResolutionServiceHandle(Arc<dyn PropertyResolutionService>);
impl PropertyResolutionServiceHandle {
    /// Wraps a trusted service.
    pub fn new(service: Arc<dyn PropertyResolutionService>) -> Self {
        Self(service)
    }
    /// Resolves and validates request binding and exact provenance.
    pub fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let resolution = self.0.resolve(request)?;
        match &resolution {
            PropertyResolution::Present(resolved) => {
                if resolved.request() != request || !request.matches(resolved.property()) {
                    return Err(PropertyResolutionError::ResponseRequestMismatch);
                }
                if !resolved
                    .property()
                    .evidence
                    .as_ref()
                    .is_some_and(reviewable)
                {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
                if !valid_value(&resolved.property().value) {
                    return Err(PropertyResolutionError::InvalidValue);
                }
            }
            PropertyResolution::Absent(evidence) => {
                if evidence.request() != request {
                    return Err(PropertyResolutionError::ResponseRequestMismatch);
                }
                if !reviewable(evidence.evidence()) {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
            }
        }
        Ok(resolution)
    }
}

fn valid_value(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::Decimal(value) | PropertyValue::Quantity { value, .. } => value.is_finite(),
        PropertyValue::Null
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::String(_) => true,
    }
}

fn reviewable(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
