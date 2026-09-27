//! End to end: several IFC models in one session, each with a discipline.
//!
//! Each file is imported as its own source and the sessions are federated.
//! Both files use the same STEP instance ids, so every assertion that names
//! an object names it with its source: `#10` alone would be ambiguous.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{
    CapabilityRegistry, CoordinateSystemError, CoordinateSystemServiceHandle, EvidenceSession,
    EvidenceSessionError, ObjectFrameError, ObjectFrameServiceHandle, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, Runtime,
    SemanticRelationship, SourceIntegrityServiceHandle, SourceSnapshot, TraversalDirection,
    TypeHierarchyServiceHandle, compile,
};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{
    DefinitionPackage, Discipline, NotEvaluatedReason, ObjectId, Report, RuleSetPackage, Scope,
    SourceId,
};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// Wall #10 states its `Reference`; door #30 fills opening #20 in it.
const ARCHITECTURE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#10=IFCWALL('0000000000000000000A10',$,'A-W1',$,$,$,$,$,.STANDARD.);
#11=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('A-1'),$);
#12=IFCPROPERTYSET('0000000000000000000A12',$,'Pset_WallCommon',$,(#11));
#13=IFCRELDEFINESBYPROPERTIES('0000000000000000000A13',$,$,$,(#10),#12);
#20=IFCOPENINGELEMENT('0000000000000000000A20',$,$,$,$,$,$,$,.OPENING.);
#21=IFCRELVOIDSELEMENT('0000000000000000000A21',$,$,$,#10,#20);
#30=IFCDOOR('0000000000000000000A30',$,'A-D1',$,$,$,$,$,$,$,.DOOR.,$,$);
#31=IFCRELFILLSELEMENT('0000000000000000000A31',$,$,$,#20,#30);
ENDSEC;
END-ISO-10303-21;
";

/// Wall #10 states no `Reference`; door #30 fills nothing.
const STRUCTURE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#10=IFCWALL('0000000000000000000S10',$,'S-W1',$,$,$,$,$,.STANDARD.);
#30=IFCDOOR('0000000000000000000S30',$,'S-D1',$,$,$,$,$,$,$,.DOOR.,$,$);
ENDSEC;
END-ISO-10303-21;
";

fn source(document: &str) -> SourceId {
    SourceId::new("ifc-step", document).unwrap()
}

fn id(document: &str, local: &str) -> ObjectId {
    ObjectId::new(source(document), local).unwrap()
}

fn discipline(name: &str) -> Discipline {
    Discipline::new(name).unwrap()
}

/// One session per file, each declaring its discipline when given one.
fn member(document: &str, ifc: &str, role: Option<&str>) -> EvidenceSession {
    let session = import_ifc_session(document, ifc.as_bytes()).unwrap();
    match role {
        Some(role) => session
            .with_discipline(&source(document), discipline(role))
            .unwrap(),
        None => session,
    }
}

fn federation(structure: Option<&str>) -> EvidenceSession {
    EvidenceSession::federate([
        member("arch.ifc", ARCHITECTURE, Some("architecture")),
        member("struct.ifc", STRUCTURE, structure),
    ])
    .unwrap()
}

#[test]
fn two_models_keep_source_qualified_identities_in_one_session() {
    let session = federation(Some("structure"));
    let sources: Vec<&SourceId> = session.snapshots().map(SourceSnapshot::source).collect();
    assert_eq!(sources, [&source("arch.ifc"), &source("struct.ifc")]);
    let walls: Vec<String> = session
        .project()
        .objects()
        .filter(|object| object.kind() == "IFCWALL")
        .map(|object| object.id.to_string())
        .collect();
    assert_eq!(
        walls,
        ["ifc-step:arch.ifc/#10", "ifc-step:struct.ifc/#10"],
        "same local id, two objects"
    );
    assert_eq!(
        session.discipline(&source("struct.ifc")),
        Some(&discipline("structure"))
    );

    // Each semantic answer comes from the object's own file.
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    let reference = |object: ObjectId| {
        properties
            .resolve(
                &PropertyRequest::try_new(object, Some("Pset_WallCommon".into()), "Reference")
                    .unwrap(),
            )
            .unwrap()
    };
    assert!(matches!(
        reference(id("arch.ifc", "#10")),
        PropertyResolution::Present(_)
    ));
    assert!(matches!(
        reference(id("struct.ifc", "#10")),
        PropertyResolution::Absent(_)
    ));
    let hierarchy = session.service::<TypeHierarchyServiceHandle>().unwrap();
    for document in ["arch.ifc", "struct.ifc"] {
        assert_eq!(
            hierarchy.is_a(&source(document), "IFCWALL", "IFCPRODUCT"),
            Ok(true)
        );
    }
    let integrity = session.service::<SourceIntegrityServiceHandle>().unwrap();
    for document in ["arch.ifc", "struct.ifc"] {
        integrity.issues(&source(document)).unwrap();
    }
    // Neither wall is placed: each file answers for its own object rather
    // than the federation refusing the source as uncovered.
    let frames = session.service::<ObjectFrameServiceHandle>().unwrap();
    for document in ["arch.ifc", "struct.ifc"] {
        let wall = id(document, "#10");
        assert_eq!(
            frames.object_frame(&wall).unwrap_err(),
            ObjectFrameError::NotPlaced(wall)
        );
    }
    assert!(matches!(
        frames.object_frame(&id("other.ifc", "#10")),
        Err(ObjectFrameError::UncoveredSource(_))
    ));
    // Each file states its own coordinate system; neither has a model
    // context, so neither states one.
    let systems = session.service::<CoordinateSystemServiceHandle>().unwrap();
    for document in ["arch.ifc", "struct.ifc"] {
        let system = systems.coordinate_system(&source(document)).unwrap();
        assert_eq!(system.source(), &source(document));
        assert!(system.world().is_none());
    }
    assert!(matches!(
        systems.coordinate_system(&source("other.ifc")),
        Err(CoordinateSystemError::UncoveredSource(_))
    ));

    // A relationship request over the whole project is answered by the
    // anchor's file and bound back to the whole request.
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap();
    let everything: Vec<ObjectId> = session.project().objects().map(|o| o.id.clone()).collect();
    let request = RelationshipSelectionRequest::try_new(
        id("arch.ifc", "#20"),
        everything,
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelFillsElement").unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false,
        },
    )
    .unwrap();
    let selection = relationships.select(&request).unwrap();
    assert_eq!(selection.candidates(), [id("arch.ifc", "#30")]);
    assert!(
        selection
            .evidence()
            .iter()
            .all(|evidence| evidence.source == source("arch.ifc"))
    );
}

#[test]
fn federating_refuses_a_source_twice_and_unroutable_services() {
    let twice = EvidenceSession::federate([
        member("arch.ifc", ARCHITECTURE, None),
        member("arch.ifc", STRUCTURE, None),
    ]);
    assert!(matches!(
        twice,
        Err(EvidenceSessionError::DuplicateSource(source)) if source.document == "arch.ifc"
    ));

    let session = member("arch.ifc", ARCHITECTURE, None);
    let snapshots: Vec<_> = session.snapshots().cloned().collect();
    let hosted = session.with_host_service(7_u32, &snapshots).unwrap();
    assert!(matches!(
        EvidenceSession::federate([hosted, member("struct.ifc", STRUCTURE, None)]),
        Err(EvidenceSessionError::UnfederableService)
    ));

    let session = member("arch.ifc", ARCHITECTURE, Some("architecture"));
    assert!(matches!(
        session.with_discipline(&source("arch.ifc"), discipline("structure")),
        Err(EvidenceSessionError::DuplicateDiscipline(_))
    ));
    let session = member("arch.ifc", ARCHITECTURE, None);
    assert!(matches!(
        session.with_discipline(&source("other.ifc"), discipline("structure")),
        Err(EvidenceSessionError::UnknownSource(_))
    ));
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn concept(id: &str, ifc_name: &str) -> Value {
    json!({
        "id": id,
        "name": text(id),
        "externalNames": [{ "typeSystem": IFC4_TYPE_SYSTEM, "name": ifc_name }],
    })
}

/// A definition's parameters, as the registry declares `capability`'s.
fn signature(registry: &CapabilityRegistry, capability: &str) -> serde_json::Map<String, Value> {
    registry
        .get(capability)
        .unwrap()
        .parameters()
        .into_iter()
        .map(|descriptor| {
            (
                descriptor.name.clone(),
                json!({
                    "id": descriptor.name,
                    "name": text(&descriptor.name),
                    "kind": kind(descriptor.parameter_type),
                    "required": descriptor.required,
                }),
            )
        })
        .collect()
}

fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let capability = "axioval:capability.property-required";
    let parameters = signature(registry, capability);
    let mut reference = concept("axioval:test.reference", "Reference");
    reference["valueKind"] = json!("string");
    let mut fire_rating = concept("axioval:test.fire-rating", "FireRating");
    fire_rating["valueKind"] = json!("string");
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": {
            "axioval:test.wall": concept("axioval:test.wall", "IfcWall"),
            "axioval:test.door": concept("axioval:test.door", "IfcDoor"),
            "axioval:test.duct": concept("axioval:test.duct", "IfcDuctSegment"),
        },
        "properties": {
            "axioval:test.reference": reference,
            "axioval:test.fire-rating": fire_rating,
        },
        "propertySets": {
            "axioval:test.wall-common": concept("axioval:test.wall-common", "Pset_WallCommon"),
            "axioval:test.door-common": concept("axioval:test.door-common", "Pset_DoorCommon"),
        },
        "definitions": {
            "axioval:test.property-required": {
                "id": "axioval:test.property-required",
                "name": text("property-required"),
                "capability": capability,
                "parameters": parameters,
            },
        },
    }))
    .unwrap()
}

fn entity(concept: &str) -> Value {
    json!({ "kind": "entityType", "objectType": concept, "includeSubtypes": false })
}

fn rule(id: &str, applicability: &Value, set: &str, property: &str) -> Value {
    json!({
        "id": id,
        "definitionId": "axioval:test.property-required",
        "name": text(id),
        "severity": "error",
        "applicability": applicability,
        "parameters": {
            "property": { "type": "propertyReference", "property": property, "propertySet": set },
        },
    })
}

/// Structural walls need a reference; doors in a wall need a fire rating.
fn ruleset() -> RuleSetPackage {
    let structural_walls = json!({
        "kind": "allOf",
        "operands": [entity("axioval:test.wall"), { "kind": "discipline", "value": "structure" }],
    });
    let doors_in_walls = json!({
        "kind": "allOf",
        "operands": [
            entity("axioval:test.door"),
            {
                "kind": "related",
                "path": ["IfcRelFillsElement:backward", "IfcRelVoidsElement:backward"],
                "selector": entity("axioval:test.wall"),
            },
        ],
    });
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [
            rule("structural-walls-have-a-reference", &structural_walls,
                 "axioval:test.wall-common", "axioval:test.reference"),
            rule("doors-in-walls-are-rated", &doors_in_walls,
                 "axioval:test.door-common", "axioval:test.fire-rating"),
        ]},
    }))
    .unwrap()
}

fn run(session: &EvidenceSession) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry);
    let plan = compile(&registry, &[definitions], &ruleset()).unwrap();
    Runtime::new(registry).run_session(session, plan).unwrap()
}

fn flagged(report: &Report) -> Vec<(String, String)> {
    report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.rule_id.to_string(),
                finding.object_id().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn a_discipline_selector_scopes_a_rule_to_one_models_objects() {
    let report = run(&federation(Some("structure")));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // The architectural wall lacks nothing and is not structural anyway;
    // the structural wall lacks its reference. Only the architectural door
    // fills an opening in a wall, and relationships are read in its file.
    assert_eq!(
        flagged(&report),
        [
            (
                "doors-in-walls-are-rated".to_owned(),
                "ifc-step:arch.ifc/#30".to_owned()
            ),
            (
                "structural-walls-have-a-reference".to_owned(),
                "ifc-step:struct.ifc/#10".to_owned()
            ),
        ]
    );
}

#[test]
fn a_source_without_a_discipline_is_not_evaluated_once_never_skipped() {
    let report = run(&federation(None));
    assert_eq!(
        flagged(&report),
        [(
            "doors-in-walls-are-rated".to_owned(),
            "ifc-step:arch.ifc/#30".to_owned()
        )]
    );
    let [outcome] = report.not_evaluated() else {
        panic!("{:?}", report.not_evaluated());
    };
    assert_eq!(
        outcome.rule_id.to_string(),
        "structural-walls-have-a-reference"
    );
    assert_eq!(outcome.scope, Scope::Source(source("struct.ifc")));
    assert_eq!(outcome.reason, NotEvaluatedReason::NotRecorded);
    assert!(
        outcome.message.contains("declares no discipline"),
        "{}",
        outcome.message
    );
}

#[test]
fn a_discipline_selector_round_trips_and_refuses_invalid_names() {
    use axioval::ir::contract::Selector;
    let selector: Selector =
        serde_json::from_value(json!({ "kind": "discipline", "value": "structure" })).unwrap();
    assert_eq!(
        selector,
        Selector::Discipline {
            value: discipline("structure")
        }
    );
    assert_eq!(
        serde_json::to_value(&selector).unwrap(),
        json!({ "kind": "discipline", "value": "structure" })
    );
    for invalid in ["", "Structure", " structure", "-mep", "a b"] {
        assert!(
            serde_json::from_value::<Selector>(json!({ "kind": "discipline", "value": invalid }))
                .is_err(),
            "{invalid:?}"
        );
    }
}

/// A model with no objects: a presentation layer holding one point.
const EMPTY: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCPRESENTATIONLAYERWITHSTYLE('Foo',$,(#1),$,.T.,.F.,.F.,());
ENDSEC;
END-ISO-10303-21;
";

/// "Every model contains a wall", per source.
fn run_wall_count(session: &EvidenceSession) -> Report {
    run_count(session, &entity("axioval:test.wall"), &json!({}))
}

/// An `object-count` rule over `applicability` with `parameters`.
fn run_count(session: &EvidenceSession, applicability: &Value, parameters: &Value) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let capability = "axioval:capability.object-count";
    let mut definitions = serde_json::to_value(definitions(&registry)).unwrap();
    definitions["definitions"]["axioval:test.object-count"] = json!({
        "id": "axioval:test.object-count",
        "name": text("object-count"),
        "capability": capability,
        "parameters": signature(&registry, capability),
    });
    let definitions: DefinitionPackage = serde_json::from_value(definitions).unwrap();
    let ruleset: RuleSetPackage = serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [{
            "id": "a-wall-exists",
            "definitionId": "axioval:test.object-count",
            "name": text("a-wall-exists"),
            "severity": "error",
            "applicability": applicability,
            "parameters": parameters,
        }]},
    }))
    .unwrap();
    let plan = compile(&registry, &[definitions], &ruleset).unwrap();
    Runtime::new(registry).run_session(session, plan).unwrap()
}

#[test]
fn a_member_without_objects_stays_in_the_federation_as_an_empty_source() {
    let session = EvidenceSession::federate([
        member("arch.ifc", ARCHITECTURE, Some("architecture")),
        member("empty.ifc", EMPTY, Some("structure")),
    ])
    .unwrap();
    let sources: Vec<&SourceId> = session.snapshots().map(SourceSnapshot::source).collect();
    assert_eq!(sources, [&source("arch.ifc"), &source("empty.ifc")]);
    assert!(
        session
            .project()
            .objects()
            .all(|object| object.id.source == source("arch.ifc"))
    );
    assert_eq!(
        session.discipline(&source("empty.ifc")),
        Some(&discipline("structure"))
    );
    // Its services still answer for it rather than refusing it as uncovered.
    let integrity = session.service::<SourceIntegrityServiceHandle>().unwrap();
    integrity.issues(&source("empty.ifc")).unwrap();
    let systems = session.service::<CoordinateSystemServiceHandle>().unwrap();
    let system = systems.coordinate_system(&source("empty.ifc")).unwrap();
    assert!(system.world().is_none());

    // Object rules find nothing to check in it, and nothing is left undecided.
    let report = run(&session);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(
        flagged(&report),
        [(
            "doors-in-walls-are-rated".to_owned(),
            "ifc-step:arch.ifc/#30".to_owned()
        )]
    );

    // A count per source reports that it holds no wall.
    let report = run_wall_count(&session);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let [finding] = report.findings() else {
        panic!("{:?}", report.findings());
    };
    assert_eq!(finding.scope, Scope::Source(source("empty.ifc")));
    assert_eq!(
        finding.message,
        "no object matches the selection in source `ifc-step:empty.ifc`; required at least 1"
    );
}

/// The file names as the CLI states them, then `map`.
fn mapped(map: &axioval::engine::DisciplineMap, structure: Option<&str>) -> EvidenceSession {
    use axioval::engine::SourceMetadata;
    use axioval::ir::contract::SourceField;
    let named = |document: &str, ifc: &str, role: Option<&str>| {
        member(document, ifc, role)
            .with_source_metadata(
                &source(document),
                SourceMetadata::new().with(SourceField::FileName, [document]),
            )
            .unwrap()
    };
    EvidenceSession::federate([
        named("arch.ifc", ARCHITECTURE, Some("architecture")),
        named("struct.ifc", STRUCTURE, structure),
    ])
    .unwrap()
    .with_discipline_map(map)
}

#[test]
fn a_discipline_map_assigns_undeclared_sources_and_is_cited() {
    use axioval::engine::{DisciplineMap, DisciplineOrigin, DisciplineRule, UnmappedReason};
    use axioval::ir::contract::SourceField;
    let rule = |field, pattern: &str, role: &str| {
        DisciplineRule::new(field, pattern, discipline(role)).unwrap()
    };
    // The architectural file is declared: the first rule would match it
    // too, and does not replace its discipline.
    let map = DisciplineMap::new()
        .with(rule(SourceField::FileName, "*.ifc", "mep"))
        .with(rule(SourceField::FileName, "struct*", "structure"));
    let session = mapped(&map, None);
    assert_eq!(
        session.discipline(&source("arch.ifc")),
        Some(&discipline("architecture"))
    );
    assert_eq!(
        session.discipline_origin(&source("arch.ifc")),
        Some(&DisciplineOrigin::Declared)
    );
    assert_eq!(
        session.discipline(&source("struct.ifc")),
        Some(&discipline("mep"))
    );

    let map = DisciplineMap::new().with(rule(SourceField::FileName, "struct*", "structure"));
    let session = mapped(&map, None);
    assert_eq!(
        session.discipline_origin(&source("struct.ifc")),
        Some(&DisciplineOrigin::Mapped {
            rule: "fileName:struct*=structure".into(),
            field: SourceField::FileName,
            value: "struct.ifc".into(),
        })
    );
    // The discipline rule now evaluates the mapped model.
    let report = run(&session);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert!(
        flagged(&report).contains(&(
            "structural-walls-have-a-reference".to_owned(),
            "ifc-step:struct.ifc/#10".to_owned()
        )),
        "{:?}",
        report.findings()
    );
    // A selection resting on the mapped discipline cites the assignment.
    let structural_walls = json!({
        "kind": "allOf",
        "operands": [entity("axioval:test.wall"), { "kind": "discipline", "value": "structure" }],
    });
    let report = run_count(
        &session,
        &structural_walls,
        &json!({ "maximum": { "type": "integer", "value": 0 } }),
    );
    let [finding] = report.findings() else {
        panic!("{:?}", report.findings());
    };
    assert!(
        finding.evidence.iter().any(|evidence| !evidence.exact
            && evidence.locator == "discipline-map:fileName:struct*=structure@struct.ifc"),
        "{:?}",
        finding.evidence
    );

    // A field the source never stated stops the map: unmapped, as without one.
    let map = DisciplineMap::new()
        .with(rule(SourceField::Application, "*", "structure"))
        .with(rule(SourceField::FileName, "struct*", "structure"));
    let session = EvidenceSession::federate([
        member("arch.ifc", ARCHITECTURE, Some("architecture")),
        member("struct.ifc", STRUCTURE, None),
    ])
    .unwrap()
    .with_discipline_map(&map);
    // Without owner histories the file states no application, exactly:
    // the first rule matches nothing and the second reads an unstated
    // file name.
    assert_eq!(session.discipline(&source("struct.ifc")), None);
    assert_eq!(
        session.unmapped(&source("struct.ifc")),
        Some(&UnmappedReason::Unread("fileName:struct*=structure".into()))
    );
}

/// An MEP model holding one duct segment.
const DUCTS: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#10=IFCDUCTSEGMENT('0000000000000000000M10',$,'M-D1',$,$,$,$,$,.RIGIDSEGMENT.);
ENDSEC;
END-ISO-10303-21;
";

#[test]
fn a_duct_count_limited_to_mep_judges_only_the_mep_models() {
    let session = EvidenceSession::federate([
        member("arch.ifc", ARCHITECTURE, Some("architecture")),
        member("mep-1.ifc", DUCTS, Some("mep")),
        member("mep-2.ifc", STRUCTURE, Some("mep")),
    ])
    .unwrap();
    let report = run_count(
        &session,
        &entity("axioval:test.duct"),
        &json!({ "disciplines": { "type": "stringList", "value": ["mep"] } }),
    );
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let [finding] = report.findings() else {
        panic!("{:?}", report.findings());
    };
    assert_eq!(finding.scope, Scope::Source(source("mep-2.ifc")));
}
