//! IR contract tests.
#![allow(missing_docs)]

use axioval_ir::{
    Classification, Evidence, Object, ObjectId, Project, Property, PropertyValue, Selector,
    SourceId,
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
