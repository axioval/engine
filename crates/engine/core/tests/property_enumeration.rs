//! Exact property-enumeration contract tests.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    NameMatch, NamePattern, PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle,
};
use axioval_ir::{Evidence, ObjectId, Property, PropertyValue, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn object(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn common() -> NameMatch {
    NameMatch::Pattern(NamePattern::new("Pset_.*Common").unwrap())
}

fn request() -> PropertyEnumerationRequest {
    PropertyEnumerationRequest::try_new(object("wall"), common(), NameMatch::Any).unwrap()
}

fn property(set: &str, name: &str) -> Property {
    Property::new(set, name, PropertyValue::String("x".into()))
        .unwrap()
        .with_evidence(Evidence::exact(source(), format!("{set}.{name}")))
}

fn complete() -> Evidence {
    Evidence::exact(source(), "complete property table")
}

#[test]
fn a_pattern_matches_whole_names_only() {
    let pattern = NamePattern::new("Foo.*").unwrap();
    assert!(pattern.is_match("FooBar"));
    assert!(!pattern.is_match("XFooBar"));
    assert_eq!(pattern.as_str(), "Foo.*");
    assert_eq!(
        NamePattern::new("(").unwrap_err(),
        PropertyResolutionError::InvalidRequest
    );
    assert!(NameMatch::Any.matches("anything"));
    assert!(!NameMatch::Exact("A".into()).matches("a"));
}

#[test]
fn a_request_refuses_blank_names_and_reserved_sets() {
    for (set, property) in [
        (NameMatch::Exact(" ".into()), NameMatch::Any),
        (NameMatch::Any, NameMatch::Exact(String::new())),
        (
            NameMatch::Exact("axioval:attributes".into()),
            NameMatch::Any,
        ),
    ] {
        assert_eq!(
            PropertyEnumerationRequest::try_new(object("wall"), set, property),
            Err(PropertyResolutionError::InvalidRequest)
        );
    }
}

#[test]
fn an_enumeration_is_sorted_and_holds_only_selected_exact_distinct_properties() {
    let enumeration = PropertyEnumeration::try_new(
        request(),
        vec![
            property("Pset_WallCommon", "Status"),
            property("Pset_DoorCommon", "FireRating"),
        ],
        complete(),
    )
    .unwrap();
    assert_eq!(enumeration.properties()[0].property_set, "Pset_DoorCommon");
    // Not selected by the set pattern.
    assert_eq!(
        PropertyEnumeration::try_new(request(), vec![property("Other", "Status")], complete())
            .unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );
    // A reserved set is never enumerated, whatever the pattern says.
    let everything =
        PropertyEnumerationRequest::try_new(object("wall"), NameMatch::Any, NameMatch::Any)
            .unwrap();
    assert_eq!(
        PropertyEnumeration::try_new(
            everything,
            vec![property("axioval:attributes", "Name")],
            complete()
        )
        .unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );
    assert!(matches!(
        PropertyEnumeration::try_new(
            request(),
            vec![
                property("Pset_WallCommon", "Status"),
                property("Pset_WallCommon", "Status")
            ],
            complete()
        ),
        Err(PropertyResolutionError::Conflicting(_))
    ));
    let inexact = Property::new("Pset_WallCommon", "Status", PropertyValue::Null).unwrap();
    assert_eq!(
        PropertyEnumeration::try_new(request(), vec![inexact], complete()).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
    let mut guessed = complete();
    guessed.exact = false;
    assert_eq!(
        PropertyEnumeration::try_new(request(), Vec::new(), guessed).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
    let other = Evidence::exact(SourceId::new("cad", "other").unwrap(), "table");
    assert_eq!(
        PropertyEnumeration::try_new(request(), Vec::new(), other).unwrap_err(),
        PropertyResolutionError::InexactEvidence
    );
}

/// Answers every enumeration about object `other`, whatever was asked.
struct Misbound;
impl PropertyResolutionService for Misbound {
    fn resolve(&self, _: &PropertyRequest) -> Result<PropertyResolution, PropertyResolutionError> {
        Err(PropertyResolutionError::Unavailable("resolution".into()))
    }
    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        let other = PropertyEnumerationRequest::try_new(
            object("other"),
            request.property_set().clone(),
            request.property().clone(),
        )?;
        PropertyEnumeration::try_new(other, Vec::new(), complete())
    }
}

/// Cannot list properties.
struct NamesOnly;
impl PropertyResolutionService for NamesOnly {
    fn resolve(&self, _: &PropertyRequest) -> Result<PropertyResolution, PropertyResolutionError> {
        Err(PropertyResolutionError::Unavailable("resolution".into()))
    }
}

#[test]
fn the_handle_binds_the_answer_and_the_default_refuses() {
    let misbound = PropertyResolutionServiceHandle::new(Arc::new(Misbound));
    assert_eq!(
        misbound.enumerate(&request()).unwrap_err(),
        PropertyResolutionError::ResponseRequestMismatch
    );
    let names_only = PropertyResolutionServiceHandle::new(Arc::new(NamesOnly));
    assert!(matches!(
        names_only.enumerate(&request()),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}

#[test]
fn empty_sets_are_selected_sorted_and_hold_no_property() {
    let enumeration = PropertyEnumeration::try_new(
        request(),
        vec![property("Pset_WallCommon", "IsExternal")],
        complete(),
    )
    .unwrap();
    let with = enumeration
        .clone()
        .with_empty_sets([
            "Pset_SlabCommon".to_owned(),
            "Pset_BeamCommon".to_owned(),
            "Pset_SlabCommon".to_owned(),
        ])
        .unwrap();
    assert_eq!(with.empty_sets(), ["Pset_BeamCommon", "Pset_SlabCommon"]);
    assert!(enumeration.empty_sets().is_empty());
    // A set the request does not select, or a reserved one, is not an answer.
    for set in ["Other", "axioval:attributes"] {
        assert_eq!(
            enumeration.clone().with_empty_sets([set.to_owned()]),
            Err(PropertyResolutionError::ResponseRequestMismatch),
            "{set}"
        );
    }
    // A set holding an enumerated property is not empty.
    assert!(matches!(
        enumeration.with_empty_sets(["Pset_WallCommon".to_owned()]),
        Err(PropertyResolutionError::Conflicting(_))
    ));
}
