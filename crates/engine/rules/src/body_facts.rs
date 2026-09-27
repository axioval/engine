//! Reads the reserved body set (`axioval:body`) of one object.
//!
//! The body set is engine vocabulary: every source names its facts alike, so
//! they are asked for by the engine's own names, never through package
//! concept binding (as `clash` asks for the presentation layer). Every fact
//! read is cited, present or absent.

use axioval_engine::{
    NotEvaluatedReason, PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle,
    RuleContext,
};
use axioval_ir::{BODY_SET, Evidence, Object, PropertyValue, QuantityDimension};

use crate::selection::property_error;
use crate::support::{Unavailable, display, invalid};

/// One object's body facts, read on demand and cited as they are read.
pub(crate) struct BodyFacts<'a> {
    service: &'a PropertyResolutionServiceHandle,
    object: &'a Object,
    evidence: Vec<Evidence>,
}

impl<'a> BodyFacts<'a> {
    pub(crate) fn of(context: &RuleContext<'a>, object: &'a Object) -> Result<Self, Unavailable> {
        let service = context
            .services
            .get::<PropertyResolutionServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "property-resolution service is not registered".to_owned(),
                )
            })?;
        Ok(Self {
            service,
            object,
            evidence: Vec::new(),
        })
    }

    /// The facts cited so far.
    pub(crate) fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    /// Takes the facts cited so far.
    pub(crate) fn into_evidence(self) -> Vec<Evidence> {
        self.evidence
    }

    /// `name`'s value, `None` when exactly absent.
    pub(crate) fn value(&mut self, name: &str) -> Result<Option<PropertyValue>, Unavailable> {
        let request =
            PropertyRequest::try_new(self.object.id.clone(), Some(BODY_SET.to_owned()), name)
                .map_err(|error| invalid(error.to_string()))?;
        match self.service.resolve(&request).map_err(|error| {
            let (reason, message) = property_error(error);
            (reason, format!("`{BODY_SET}.{name}`: {message}"))
        })? {
            PropertyResolution::Present(resolved) => {
                let property = resolved.property();
                self.evidence.extend(property.evidence.iter().cloned());
                Ok(Some(property.value.clone()))
            }
            PropertyResolution::Absent(proof) => {
                self.evidence.push(proof.evidence().clone());
                Ok(None)
            }
        }
    }

    fn wrong(name: &str, value: &PropertyValue, expected: &str) -> Unavailable {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "`{BODY_SET}.{name}` is {}, not {expected}",
                display(Some(value))
            ),
        )
    }

    pub(crate) fn text(&mut self, name: &str) -> Result<Option<String>, Unavailable> {
        match self.value(name)? {
            None => Ok(None),
            Some(PropertyValue::String(text)) => Ok(Some(text)),
            Some(other) => Err(Self::wrong(name, &other, "text")),
        }
    }

    pub(crate) fn integer(&mut self, name: &str) -> Result<Option<i64>, Unavailable> {
        match self.value(name)? {
            None => Ok(None),
            Some(PropertyValue::Integer(value)) => Ok(Some(value)),
            Some(other) => Err(Self::wrong(name, &other, "an integer")),
        }
    }

    fn quantity(
        &mut self,
        name: &str,
        dimension: QuantityDimension,
        expected: &str,
    ) -> Result<Option<f64>, Unavailable> {
        match self.value(name)? {
            None => Ok(None),
            Some(PropertyValue::Quantity {
                value,
                dimension: stated,
            }) if stated == dimension && value.is_finite() => Ok(Some(value)),
            Some(other) => Err(Self::wrong(name, &other, expected)),
        }
    }

    pub(crate) fn length(&mut self, name: &str) -> Result<Option<f64>, Unavailable> {
        self.quantity(name, QuantityDimension::Length, "a length")
    }
}

/// The family names of profiles that are not parameterised sections.
pub(crate) const ARBITRARY_FAMILIES: &[&str] = &[
    "arbitrary-closed",
    "arbitrary-with-voids",
    "center-line",
    "composite",
];
