//! Exact property-resolution contract tests.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ResolvedProperty, UnreadableValue,
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
fn exact_evidence_from_another_source_is_rejected() {
    let other = SourceId::new("cad", "other-model").unwrap();
    let property = axioval_ir::Property::new(
        "Pset_WallCommon",
        "Reference",
        axioval_ir::PropertyValue::String("EI60".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(other.clone(), "property table"));
    assert_eq!(
        ResolvedProperty::try_new(request(), property).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
    assert_eq!(
        CompletePropertyAbsenceEvidence::try_new(
            request(),
            Evidence::exact(other, "complete property table"),
        )
        .unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
}

#[test]
fn non_finite_present_property_is_rejected() {
    use axioval_ir::PropertyValue;
    let text = || PropertyValue::String("A-WALL".into());
    for value in [
        PropertyValue::Decimal(f64::NAN),
        PropertyValue::Quantity {
            value: f64::INFINITY,
            dimension: axioval_ir::QuantityDimension::Length,
        },
        // A list holds finite scalars only: no null, no nested list.
        PropertyValue::List(vec![text(), PropertyValue::Decimal(f64::NAN)]),
        PropertyValue::List(vec![text(), PropertyValue::Null]),
        PropertyValue::List(vec![PropertyValue::List(vec![text()])]),
        // A range states at least one scalar, all of one kind.
        PropertyValue::Bounded {
            lower: None,
            upper: None,
            set_point: None,
        },
        PropertyValue::Bounded {
            lower: Some(Box::new(PropertyValue::Decimal(1.0))),
            upper: Some(Box::new(text())),
            set_point: None,
        },
        PropertyValue::Bounded {
            lower: Some(Box::new(PropertyValue::Null)),
            upper: None,
            set_point: None,
        },
        // A table has rows, and every cell is a finite scalar.
        PropertyValue::Table(Vec::new()),
        PropertyValue::Table(vec![axioval_ir::PropertyTableRow {
            defining: text(),
            defined: PropertyValue::List(vec![text()]),
        }]),
        PropertyValue::Table(vec![axioval_ir::PropertyTableRow {
            defining: PropertyValue::Decimal(f64::NAN),
            defined: text(),
        }]),
        // A complex property is no value: never inside one.
        PropertyValue::List(vec![PropertyValue::Complex]),
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

/// A complex property holds no value of any type, so it declares none.
#[test]
fn a_complex_property_is_present_and_declares_no_type() {
    let complex = || {
        axioval_ir::Property::new(
            "Pset_WallCommon",
            "Reference",
            axioval_ir::PropertyValue::Complex,
        )
        .unwrap()
        .with_evidence(Evidence::exact(source(), "native property table"))
    };
    let resolved = ResolvedProperty::try_new(request(), complex()).unwrap();
    assert_eq!(
        resolved.property().value,
        axioval_ir::PropertyValue::Complex
    );
    assert_eq!(
        ResolvedProperty::try_new(
            request(),
            complex().with_data_type("IFCLENGTHMEASURE").unwrap()
        )
        .unwrap_err(),
        PropertyResolutionError::InvalidValue
    );
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

struct Unreadable(UnreadableValue);
impl PropertyResolutionService for Unreadable {
    fn resolve(
        &self,
        _request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        Err(PropertyResolutionError::UnreadableValue(Box::new(
            self.0.clone(),
        )))
    }
}

fn unreadable(request: PropertyRequest) -> UnreadableValue {
    UnreadableValue::try_new(
        request,
        "IFCMASSMEASURE",
        Evidence::exact(source(), "property #10"),
        "the unit of a IFCMASSMEASURE cannot be resolved exactly",
    )
    .unwrap()
}

#[test]
fn an_unreadable_value_is_a_request_bound_answer_with_its_declared_type() {
    let handle = PropertyResolutionServiceHandle::new(Arc::new(Unreadable(unreadable(request()))));
    let Err(PropertyResolutionError::UnreadableValue(answer)) = handle.resolve(&request()) else {
        panic!("expected an unreadable value");
    };
    assert_eq!(answer.request(), &request());
    assert_eq!(answer.data_type(), "IFCMASSMEASURE");
    assert_eq!(answer.evidence().locator, "property #10");
    assert_eq!(
        PropertyResolutionError::UnreadableValue(answer).to_string(),
        "the unit of a IFCMASSMEASURE cannot be resolved exactly"
    );
}

#[test]
fn an_unreadable_value_is_bound_and_exact_or_refused() {
    let other = PropertyRequest::try_new(
        ObjectId::new(source(), "other-wall").unwrap(),
        Some("Pset_WallCommon".into()),
        "Reference",
    )
    .unwrap();
    let handle = PropertyResolutionServiceHandle::new(Arc::new(Unreadable(unreadable(other))));
    assert_eq!(
        handle.resolve(&request()).unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );

    let mut inexact = Evidence::exact(source(), "property #10");
    inexact.exact = false;
    let foreign = Evidence::exact(SourceId::new("cad", "other").unwrap(), "property #10");
    for evidence in [inexact, foreign] {
        assert_eq!(
            UnreadableValue::try_new(request(), "IFCMASSMEASURE", evidence, "no unit"),
            Err(PropertyResolutionError::InexactEvidence)
        );
    }
    for (data_type, reason) in [(" ", "no unit"), ("IFCMASSMEASURE", "")] {
        assert_eq!(
            UnreadableValue::try_new(
                request(),
                data_type,
                Evidence::exact(source(), "property #10"),
                reason
            ),
            Err(PropertyResolutionError::InvalidRequest)
        );
    }
}
