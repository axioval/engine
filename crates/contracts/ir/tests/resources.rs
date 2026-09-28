//! Resource objects a report names, and references between instances.
#![allow(missing_docs)]

use axioval_ir::{
    ExternalId, Finding, IdentityError, Object, ObjectId, Project, PropertyValue, Report, RuleId,
    Severity, SourceId, finding_ids,
};
use serde_json::json;

const STABLE: &str = "stable";

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
}

fn project() -> Project {
    Project::new(vec![
        Object::new(id("#1"), "wall").with_external_id(ExternalId::new(STABLE, "W").unwrap()),
    ])
    .unwrap()
}

fn finding(subject: &str) -> Finding {
    Finding::new(
        RuleId::new("r").unwrap(),
        id(subject),
        Severity::Error,
        "missing",
    )
}

#[test]
fn a_report_without_resources_serializes_as_before() {
    let report = Report {
        findings: vec![finding("#1")],
        ..Report::default()
    };
    let wire = serde_json::to_value(&report).unwrap();
    assert!(wire.get("resources").is_none());
    let read: Report = serde_json::from_value(wire).unwrap();
    assert_eq!(read, report);
}

#[test]
fn a_resource_the_report_names_resolves_through_the_report() {
    let material = Object::new(id("#7"), "material");
    let report = Report {
        findings: vec![finding("#7")],
        resources: vec![material.clone()],
        ..Report::default()
    };
    let project = project();
    assert_eq!(report.resource(&id("#7")), Some(&material));
    assert_eq!(report.object(&project, &id("#7")), Some(&material));
    assert_eq!(
        report.object(&project, &id("#1")),
        project.object(&id("#1"))
    );
    assert_eq!(report.object(&project, &id("#9")), None);
    let read: Report = serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    assert_eq!(read, report);
}

#[test]
fn a_resource_is_keyed_by_its_alias_or_else_its_identity() {
    let project = project();
    let keyed = |resource: Object| {
        let report = Report {
            findings: vec![finding(&resource.id.local_id)],
            resources: vec![resource],
            ..Report::default()
        };
        finding_ids(&report, &project, STABLE).unwrap()[0]
    };
    // An alias survives renumbering; an identity does not.
    let aliased = |local: &str| {
        keyed(
            Object::new(id(local), "relation")
                .with_external_id(ExternalId::new(STABLE, "R").unwrap()),
        )
    };
    assert_eq!(aliased("#7"), aliased("#8"));
    let bare = |local: &str| keyed(Object::new(id(local), "material"));
    assert_ne!(bare("#7"), bare("#8"));
    assert_eq!(bare("#7"), bare("#7"));
}

#[test]
fn a_resource_the_report_does_not_carry_is_unknown() {
    let report = Report {
        findings: vec![finding("#7")],
        ..Report::default()
    };
    assert_eq!(
        finding_ids(&report, &project(), STABLE),
        Err(IdentityError::UnknownObject(id("#7")))
    );
}

#[test]
fn a_reference_names_an_instance_on_the_wire() {
    let value = PropertyValue::Reference(id("#3"));
    let wire = serde_json::to_value(&value).unwrap();
    assert_eq!(
        wire,
        json!({
            "type": "reference",
            "value": {"source": {"system": "test", "document": "model"}, "local_id": "#3"}
        })
    );
    assert!(value.is_scalar());
    assert_eq!(
        serde_json::from_value::<PropertyValue>(wire).unwrap(),
        value
    );
}
