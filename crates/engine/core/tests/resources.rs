//! Resource objects: listed by class on request, never part of the object
//! population, and reached only through a selector naming their class.
#![allow(missing_docs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, EvidenceSession, ParameterDescriptor,
    ParameterType, ResourceError, ResourceObjects, ResourceRequest, ResourceService,
    ResourceServiceHandle, RuleCapability, RuleContext, Runtime, SourceSnapshot, compile,
};
use axioval_ir::contract::Selector;
use axioval_ir::{
    DefinitionPackage, ExternalId, Finding, Object, ObjectId, Project, Property, PropertyValue,
    RuleSetPackage, Severity, SourceId,
};

const TYPE_SYSTEM: &str = "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3";

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source("model"), local).unwrap()
}

fn snapshot(document: &str) -> SourceSnapshot {
    SourceSnapshot::try_new(source(document), "r1", format!("sha256:{document}"))
        .unwrap()
        .with_type_system(TYPE_SYSTEM)
        .unwrap()
}

/// Lists `#7` and `#8` as materials and counts every request.
struct Materials {
    snapshots: Vec<SourceSnapshot>,
    asked: Arc<AtomicUsize>,
    answer: fn(&ResourceRequest) -> Vec<Object>,
}

fn materials(request: &ResourceRequest) -> Vec<Object> {
    if !request.class().eq_ignore_ascii_case("IfcMaterial") {
        return Vec::new();
    }
    vec![
        Object::new(
            ObjectId::new(request.source().clone(), "#7").unwrap(),
            "IFCMATERIAL",
        ),
        Object::new(
            ObjectId::new(request.source().clone(), "#8").unwrap(),
            "IFCMATERIAL",
        )
        .with_external_id(ExternalId::new("ifc-globalid", "M").unwrap()),
    ]
}

impl ResourceService for Materials {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok((self.answer)(request))
    }
}

fn handle(
    answer: fn(&ResourceRequest) -> Vec<Object>,
) -> (ResourceServiceHandle, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let service = Materials {
        snapshots: vec![snapshot("model")],
        asked: asked.clone(),
        answer,
    };
    (ResourceServiceHandle::new(Arc::new(service)), asked)
}

fn request(class: &str) -> ResourceRequest {
    ResourceRequest::try_new(source("model"), class, false).unwrap()
}

#[test]
fn the_handle_refuses_answers_out_of_contract() {
    let (materials, _) = handle(materials);
    assert_eq!(
        materials.resources(&request("IfcMaterial")).unwrap().len(),
        2
    );
    assert_eq!(
        materials
            .resources(&ResourceRequest::try_new(source("other"), "IfcMaterial", false).unwrap()),
        Err(ResourceError::UncoveredSource(source("other")))
    );
    assert!(ResourceRequest::try_new(source("model"), " ", false).is_err());
    let refused = |answer: fn(&ResourceRequest) -> Vec<Object>| {
        let (handle, _) = handle(answer);
        matches!(
            handle.resources(&request("IfcMaterial")),
            Err(ResourceError::InvalidAnswer(_))
        )
    };
    // Another source's instance.
    assert!(refused(|_| vec![Object::new(
        ObjectId::new(source("other"), "#7").unwrap(),
        "IFCMATERIAL"
    )]));
    // Facts of its own, which only the source's services state.
    assert!(refused(|request| vec![
        Object::new(
            ObjectId::new(request.source().clone(), "#7").unwrap(),
            "IFCMATERIAL"
        )
        .with_property(Property::new("set", "name", PropertyValue::Boolean(true)).unwrap())
    ]));
    // Out of order, or twice.
    assert!(refused(|request| {
        let object = Object::new(
            ObjectId::new(request.source().clone(), "#7").unwrap(),
            "IFCMATERIAL",
        );
        vec![object.clone(), object]
    }));
}

#[test]
fn a_selector_reaches_resources_only_by_naming_their_class() {
    let population = ResourceObjects::new().with_class(
        source("model"),
        "IfcMaterial",
        false,
        Ok(materials(&request("IfcMaterial"))),
    );
    let entity = Selector::EntityType {
        object_type: "IfcMaterial".into(),
        include_subtypes: false,
    };
    let reached = |selector: &Selector| {
        population
            .reached(selector, None)
            .objects
            .iter()
            .map(|object| object.id.local_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(reached(&entity), ["#7", "#8"]);
    assert_eq!(
        reached(&Selector::AllOf {
            operands: vec![Selector::All, entity.clone()]
        }),
        ["#7", "#8"]
    );
    assert!(reached(&Selector::All).is_empty());
    assert!(
        reached(&Selector::Not {
            operand: Box::new(entity.clone())
        })
        .is_empty()
    );
    // Another subtype flag is another answer, never asked here.
    assert!(
        reached(&Selector::EntityType {
            object_type: "IfcMaterial".into(),
            include_subtypes: true,
        })
        .is_empty()
    );
    let unread = ResourceObjects::new().with_class(
        source("model"),
        "IfcMaterial",
        false,
        Err("broken".into()),
    );
    assert_eq!(
        unread.reached(&entity, None).unreadable,
        [(source("model"), "broken".to_owned())]
    );
}

/// Reports every resource object its selector reaches.
struct Reached;
impl RuleCapability for Reached {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let reached = context
            .services
            .get::<ResourceObjects>()
            .map(|resources| {
                resources
                    .reached(&rule.selector, None)
                    .objects
                    .iter()
                    .map(|object| {
                        Finding::new(
                            rule.id.clone(),
                            object.id.clone(),
                            Severity::Error,
                            "reached",
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        CapabilityEvaluation::evaluated(reached)
    }
}

/// The fixture packages, the rule selecting `object_type` (a material
/// concept bound to `IfcMaterial` is added).
fn packages(object_type: &str) -> (DefinitionPackage, RuleSetPackage) {
    let mut definitions: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    definitions["objectTypes"]["axioval:example.ifc.material"] = serde_json::json!({
        "id": "axioval:example.ifc.material",
        "name": {"default": "Material", "translations": {}},
        "externalNames": [{"typeSystem": TYPE_SYSTEM, "name": "IfcMaterial"}],
        "citations": []
    });
    let mut rules: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    rules["root"]["rules"][0]["applicability"]["groups"]["walls"]["selector"] = serde_json::json!({
        "kind": "entityType", "objectType": object_type, "includeSubtypes": false
    });
    (
        serde_json::from_value(definitions).unwrap(),
        serde_json::from_value(rules).unwrap(),
    )
}

fn run(object_type: &str) -> (axioval_ir::Report, usize) {
    let (definitions, rules) = packages(object_type);
    let registry = CapabilityRegistry::new().register(Reached).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let (materials, asked) = handle(materials);
    let project = Project::new(vec![Object::new(id("#1"), "IFCWALL")]).unwrap();
    let session = EvidenceSession::try_new(project, [snapshot("model")])
        .unwrap()
        .with_service(materials)
        .unwrap();
    let report = Runtime::new(registry).run_session(&session, plan).unwrap();
    (report, asked.load(Ordering::SeqCst))
}

#[test]
fn a_run_lists_the_classes_its_rules_name_and_reports_the_resources_it_names() {
    let (report, asked) = run("axioval:example.ifc.material");
    assert_eq!(asked, 1);
    let named: Vec<&str> = report
        .findings()
        .iter()
        .filter_map(|finding| finding.object_id())
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(named, ["#7", "#8"]);
    let carried: Vec<&str> = report
        .resources
        .iter()
        .map(|object| object.id.local_id.as_str())
        .collect();
    assert_eq!(carried, ["#7", "#8"]);
    assert_eq!(
        report
            .resource(&id("#8"))
            .and_then(|object| object.external_id("ifc-globalid")),
        Some("M")
    );
}

#[test]
fn an_object_class_reaches_no_resource_and_the_report_carries_none() {
    let (report, asked) = run("axioval:example.ifc.wall");
    // Asked, and answered with nothing: the class selects objects only.
    assert_eq!(asked, 1);
    assert!(report.findings().is_empty());
    assert!(report.resources.is_empty());
    assert!(
        serde_json::to_value(&report)
            .unwrap()
            .get("resources")
            .is_none()
    );
}

#[test]
fn a_resource_service_is_routed_by_source_in_a_federation() {
    let member = |document: &str| {
        let asked = Arc::new(AtomicUsize::new(0));
        let service = Materials {
            snapshots: vec![snapshot(document)],
            asked,
            answer: materials,
        };
        EvidenceSession::try_new(Project::new(vec![]).unwrap(), [snapshot(document)])
            .unwrap()
            .with_service(ResourceServiceHandle::new(Arc::new(service)))
            .unwrap()
    };
    let federated = EvidenceSession::federate([member("model"), member("other")]).unwrap();
    let router = federated.services().get::<ResourceServiceHandle>().unwrap();
    let listed = router
        .resources(&ResourceRequest::try_new(source("other"), "IfcMaterial", false).unwrap())
        .unwrap();
    assert!(
        listed
            .iter()
            .all(|object| object.id.source == source("other"))
    );
    assert_eq!(
        router.resources(&ResourceRequest::try_new(source("third"), "IfcMaterial", false).unwrap()),
        Err(ResourceError::UncoveredSource(source("third")))
    );
}
