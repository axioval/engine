//! End to end: a door or window on another storey than its host wall is
//! found over real IFC.
//!
//! From each door or window, `IfcRelFillsElement` then `IfcRelVoidsElement`
//! backwards reaches the host wall; both climb `IfcRelContainedInSpatialStructure`
//! backwards to their storeys, which must be the same.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// Storeys #101 (EG) and #102 (OG). Wall #10 stands on EG with openings #20
/// and #21; wall #11 on OG with opening #22. Door #30 fills #20 on EG, door
/// #31 fills #21 but is placed on OG, window #32 fills #22 on OG. Door #33
/// fills nothing.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#100=IFCBUILDING('0000000000000000000100',$,'B',$,$,$,$,$,.ELEMENT.,$,$,$);
#101=IFCBUILDINGSTOREY('0000000000000000000101',$,'EG',$,$,$,$,$,.ELEMENT.,0.);
#102=IFCBUILDINGSTOREY('0000000000000000000102',$,'OG',$,$,$,$,$,.ELEMENT.,3.);
#103=IFCRELAGGREGATES('0000000000000000000103',$,$,$,#100,(#101,#102));
#10=IFCWALL('0000000000000000000010',$,'W1',$,$,$,$,$,.STANDARD.);
#11=IFCWALL('0000000000000000000011',$,'W2',$,$,$,$,$,.STANDARD.);
#20=IFCOPENINGELEMENT('0000000000000000000020',$,$,$,$,$,$,$,.OPENING.);
#21=IFCOPENINGELEMENT('0000000000000000000021',$,$,$,$,$,$,$,.OPENING.);
#22=IFCOPENINGELEMENT('0000000000000000000022',$,$,$,$,$,$,$,.OPENING.);
#24=IFCRELVOIDSELEMENT('0000000000000000000024',$,$,$,#10,#20);
#25=IFCRELVOIDSELEMENT('0000000000000000000025',$,$,$,#10,#21);
#26=IFCRELVOIDSELEMENT('0000000000000000000026',$,$,$,#11,#22);
#30=IFCDOOR('0000000000000000000030',$,'D1',$,$,$,$,$,$,$,.DOOR.,$,$);
#31=IFCDOOR('0000000000000000000031',$,'D2',$,$,$,$,$,$,$,.DOOR.,$,$);
#32=IFCWINDOW('0000000000000000000032',$,'F1',$,$,$,$,$,$,$,.WINDOW.,$,$);
#33=IFCDOOR('0000000000000000000033',$,'D3',$,$,$,$,$,$,$,.DOOR.,$,$);
#35=IFCRELFILLSELEMENT('0000000000000000000035',$,$,$,#20,#30);
#36=IFCRELFILLSELEMENT('0000000000000000000036',$,$,$,#21,#31);
#37=IFCRELFILLSELEMENT('0000000000000000000037',$,$,$,#22,#32);
#40=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000040',$,$,$,(#10,#30,#33),#101);
#41=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000041',$,$,$,(#11,#31,#32),#102);
ENDSEC;
END-ISO-10303-21;
";

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

/// The `same-container` definition, its signature taken from the registry.
fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let capability = "axioval:capability.same-container";
    let parameters: serde_json::Map<String, Value> = registry
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
        .collect();
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": {
            "axioval:test.door": concept("axioval:test.door", "IfcDoor"),
            "axioval:test.window": concept("axioval:test.window", "IfcWindow"),
            "axioval:test.storey": concept("axioval:test.storey", "IfcBuildingStorey"),
        },
        "definitions": {
            "axioval:test.same-container": {
                "id": "axioval:test.same-container",
                "name": text("same-container"),
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

fn ruleset() -> RuleSetPackage {
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [{
            "id": "openings-on-their-host-storey",
            "definitionId": "axioval:test.same-container",
            "name": text("openings-on-their-host-storey"),
            "severity": "error",
            "applicability": {
                "kind": "anyOf",
                "operands": [entity("axioval:test.door"), entity("axioval:test.window")],
            },
            "parameters": {
                "counterpart_path": {
                    "type": "stringList",
                    "value": ["IfcRelFillsElement:backward", "IfcRelVoidsElement:backward"],
                },
                "container_selector": {
                    "type": "selector",
                    "value": entity("axioval:test.storey"),
                },
                "relationship": { "type": "string", "value": "IfcRelContainedInSpatialStructure" },
                "direction": { "type": "string", "value": "backward" },
            },
        }]},
    }))
    .unwrap()
}

fn report() -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry);
    let plan = compile(&registry, &[definitions], &ruleset()).unwrap();
    let session = import_ifc_session("model.ifc", IFC.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn a_door_on_another_storey_than_its_host_wall_is_found() {
    let report = report();
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let findings: Vec<(&str, &str)> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.object_id().map_or("", |id| id.local_id.as_str()),
                finding.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        findings,
        [(
            "#31",
            "in #102, but its counterpart via IfcRelFillsElement then IfcRelVoidsElement is \
             not: #10 is in #101"
        )]
    );
    // The host and both storeys are related, and the relationships cited.
    let related: Vec<&str> = report.findings()[0]
        .related
        .iter()
        .map(|object| object.local_id.as_str())
        .collect();
    assert_eq!(related, ["#10", "#101", "#102"]);
    assert!(!report.findings()[0].evidence.is_empty());
}
