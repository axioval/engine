//! IR contract tests.
#![allow(missing_docs)]

use axioval_ir::{
    Classification, Evidence, ExternalId, ExternalIdClash, IrError, Object, ObjectId, Project,
    Property, PropertyValue, Selector, SourceId,
};

#[test]
fn project_object_ids_are_source_qualified_and_ordered() {
    let source = SourceId::new("ifc", "model-a").unwrap();
    let id = ObjectId::new(source.clone(), "Wall-7").unwrap();
    let project = Project::new(vec![Object::new(id.clone(), "Wall")]).unwrap();
    assert_eq!(project.object(&id).unwrap().kind(), "Wall");
    assert!(id.to_string().contains("ifc:model-a"));
}

#[test]
fn selector_filters_by_kind_and_classification() {
    let source = SourceId::new("source", "a").unwrap();
    let object = Object::new(ObjectId::new(source, "1").unwrap(), "Wall")
        .with_classification(Classification::new("Uniclass", "EF_25").unwrap());
    let selector = Selector::by_kind("Wall").with_classification("Uniclass", "EF_25");
    assert!(selector.matches(&object));
}

#[test]
fn semantic_property_values_preserve_provenance() {
    let source = SourceId::new("source", "a").unwrap();
    let owner = ObjectId::new(source.clone(), "1").unwrap();
    let property = Property::new(
        "Pset_WallCommon",
        "FireRating",
        PropertyValue::String("60".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(source, "property:1"));
    let object = Object::new(owner, "Wall").with_property(property);
    assert_eq!(
        object
            .property("Pset_WallCommon", "FireRating")
            .unwrap()
            .value(),
        &PropertyValue::String("60".into())
    );
}

#[test]
fn legacy_report_without_not_evaluated_field_remains_readable() {
    let report: axioval_ir::Report = serde_json::from_str(r#"{"findings":[]}"#).unwrap();
    assert!(report.not_evaluated().is_empty());
}

fn object(source: &SourceId, local: &str) -> Object {
    Object::new(ObjectId::new(source.clone(), local).unwrap(), "Wall")
}

#[test]
fn external_ids_are_aliases_looked_up_by_scheme() {
    let source = SourceId::new("source", "a").unwrap();
    let wall = object(&source, "1")
        .with_external_id(ExternalId::new("scheme-b", "B").unwrap())
        .with_external_id(ExternalId::new("scheme-a", "A").unwrap());
    let schemes: Vec<_> = wall.external_ids.iter().map(|id| &*id.scheme).collect();
    assert_eq!(schemes, ["scheme-a", "scheme-b"]);
    assert_eq!(wall.external_id("scheme-b"), Some("B"));
    assert_eq!(wall.external_id("scheme-c"), None);
    assert!(ExternalId::new(" ", "A").is_err());
    assert!(ExternalId::new("scheme-a", "").is_err());
}

#[test]
fn an_object_with_two_ids_in_one_scheme_is_rejected() {
    let source = SourceId::new("source", "a").unwrap();
    let wall = object(&source, "1")
        .with_external_id(ExternalId::new("scheme", "A").unwrap())
        .with_external_id(ExternalId::new("scheme", "B").unwrap());
    assert_eq!(
        Project::new(vec![wall]),
        Err(IrError::ConflictingExternalId {
            object: ObjectId::new(source, "1").unwrap(),
            scheme: "scheme".into(),
        })
    );
    // Deserialized objects need not be sorted; the check must not rely on order.
    let unsorted: Object = serde_json::from_str(
        r#"{"id":{"source":{"system":"source","document":"a"},"local_id":"1"},"kind":"Wall",
            "external_ids":[{"scheme":"x","value":"A"},{"scheme":"y","value":"B"},
                            {"scheme":"x","value":"C"}],
            "properties":[],"classifications":[],"relationships":{}}"#,
    )
    .unwrap();
    assert!(matches!(
        Project::new(vec![unsorted]),
        Err(IrError::ConflictingExternalId { .. })
    ));
}

#[test]
fn a_shared_external_id_is_rejected_within_a_source_only() {
    let a = SourceId::new("source", "a").unwrap();
    let b = SourceId::new("source", "b").unwrap();
    let id = ExternalId::new("scheme", "same").unwrap();
    let duplicated = Project::new(vec![
        object(&a, "2").with_external_id(id.clone()),
        object(&a, "1").with_external_id(id.clone()),
    ]);
    assert_eq!(
        duplicated,
        Err(IrError::DuplicateExternalId(Box::new(ExternalIdClash {
            id: id.clone(),
            first: ObjectId::new(a.clone(), "1").unwrap(),
            second: ObjectId::new(a.clone(), "2").unwrap(),
        })))
    );
    // Two revisions of one model legitimately carry the same alias.
    assert!(
        Project::new(vec![
            object(&a, "1").with_external_id(id.clone()),
            object(&b, "1").with_external_id(id),
        ])
        .is_ok()
    );
}

#[test]
fn objects_without_external_ids_keep_their_serialized_shape() {
    let source = SourceId::new("source", "a").unwrap();
    let json = serde_json::to_string(&object(&source, "1")).unwrap();
    assert!(!json.contains("external_ids"));
    let back: Object = serde_json::from_str(&json).unwrap();
    assert!(back.external_ids.is_empty());
}

#[test]
fn a_property_carries_its_declared_data_type_only_when_reported() {
    let untyped = Property::new("Pset", "Code", PropertyValue::String("A".into())).unwrap();
    let json = serde_json::to_string(&untyped).unwrap();
    // Properties without a reported type keep their serialized shape.
    assert!(!json.contains("data_type"), "{json}");
    assert_eq!(untyped.data_type(), None);

    let typed = untyped.with_data_type("IFCLABEL").unwrap();
    let back: Property = serde_json::from_str(&serde_json::to_string(&typed).unwrap()).unwrap();
    assert_eq!(back.data_type(), Some("IFCLABEL"));
    assert_eq!(back, typed);

    let blank = Property::new("Pset", "Code", PropertyValue::Null)
        .unwrap()
        .with_data_type(" ");
    assert!(matches!(blank, Err(IrError::Blank { .. })));
}
