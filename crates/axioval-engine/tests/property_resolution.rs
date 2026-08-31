//! Exact property-resolution contract tests.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ResolvedProperty,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn request() -> PropertyRequest {
    PropertyRequest::try_new(
        ObjectId::new(source(), "wall").unwrap(),
        Some("Pset_WallCommon".into()),
        "Reference",
    )
    .unwrap()
}

struct ExactAbsent;
impl PropertyResolutionService for ExactAbsent {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        Ok(PropertyResolution::Absent(
            CompletePropertyAbsenceEvidence::try_new(
                request.clone(),
                Evidence::exact(source(), "complete native property lookup"),
            )
            .unwrap(),
        ))
    }
}

#[test]
fn exact_absence_is_request_bound_and_conclusive() {
    let handle = PropertyResolutionServiceHandle::new(Arc::new(ExactAbsent));
    let resolution = handle.resolve(&request()).unwrap();
    let PropertyResolution::Absent(absence) = resolution else {
        panic!("expected exact absence")
    };
    assert_eq!(absence.request(), &request());
}

struct InexactAbsent;
impl PropertyResolutionService for InexactAbsent {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let mut evidence = Evidence::exact(source(), "partial lookup");
        evidence.exact = false;
        let absence = CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence);
        assert_eq!(
            absence.unwrap_err(),
            PropertyResolutionError::InexactEvidence
        );
        Err(PropertyResolutionError::InexactEvidence)
    }
}

#[test]
fn inexact_absence_cannot_become_conclusive() {
    let handle = PropertyResolutionServiceHandle::new(Arc::new(InexactAbsent));
    assert_eq!(
        handle.resolve(&request()).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
}

struct Stub(PropertyResolution);
impl PropertyResolutionService for Stub {
    fn resolve(
        &self,
        _request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        Ok(self.0.clone())
    }
}

#[test]
fn mismatched_present_property_is_rejected_by_the_trusted_constructor() {
    let property = axioval_ir::Property::new(
        "Pset_WallCommon",
        "FireRating",
        axioval_ir::PropertyValue::String("EI60".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(source(), "native property table"));
    assert_eq!(
        ResolvedProperty::try_new(request(), property).unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );
}

#[test]
fn inexact_present_property_is_rejected_by_the_trusted_constructor() {
    let mut evidence = Evidence::exact(source(), "heuristic property");
    evidence.exact = false;
    let property = axioval_ir::Property::new(
        "Pset_WallCommon",
        "Reference",
        axioval_ir::PropertyValue::String("EI60".into()),
    )
    .unwrap()
    .with_evidence(evidence);
    assert_eq!(
        ResolvedProperty::try_new(request(), property).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
}

#[test]
fn non_finite_present_property_is_rejected() {
    for value in [
        axioval_ir::PropertyValue::Decimal(f64::NAN),
        axioval_ir::PropertyValue::Quantity {
            value: f64::INFINITY,
            dimension: axioval_ir::QuantityDimension::Length,
        },
    ] {
        let property = axioval_ir::Property::new("Pset_WallCommon", "Reference", value)
            .unwrap()
            .with_evidence(Evidence::exact(source(), "native property table"));
        assert_eq!(
            ResolvedProperty::try_new(request(), property).unwrap_err(),
            PropertyResolutionError::InvalidValue
        );
    }
}

#[test]
fn exact_present_property_bound_to_another_object_is_rejected() {
    let requested = request();
    let other_request = PropertyRequest::try_new(
        ObjectId::new(source(), "other-wall").unwrap(),
        Some("Pset_WallCommon".into()),
        "Reference",
    )
    .unwrap();
    let property = axioval_ir::Property::new(
        "Pset_WallCommon",
        "Reference",
        axioval_ir::PropertyValue::String("OTHER".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(source(), "other wall property table"));
    let resolved = ResolvedProperty::try_new(other_request, property).unwrap();
    let handle =
        PropertyResolutionServiceHandle::new(Arc::new(Stub(PropertyResolution::Present(resolved))));

    assert_eq!(
        handle.resolve(&requested).unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );
}
