//! End to end: a compiled package checks material layer thickness over IFC.
//!
//! Pins the join no adapter test covers: a package property concept bound to
//! the IFC4 name `TotalThickness` inside the reserved material set, read from
//! a wall's own layer set usage and from its type's layer set.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, MATERIAL_SET, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// Millimetres. #1 is 300 mm thick through its type #40, #2 is 175 mm thick
/// through its own usage of #36, #3 has no material.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,$,#91);
#20=IFCMATERIAL('Concrete',$,$);
#21=IFCMATERIAL('Mineral wool',$,$);
#30=IFCMATERIALLAYER(#20,200.,$,$,$,$,$);
#31=IFCMATERIALLAYER(#21,100.,$,$,$,$,$);
#32=IFCMATERIALLAYER(#20,175.,$,$,$,$,$);
#35=IFCMATERIALLAYERSET((#30,#31),'Exterior',$);
#36=IFCMATERIALLAYERSET((#32),'Interior',$);
#37=IFCMATERIALLAYERSETUSAGE(#36,.AXIS2.,.POSITIVE.,0.,$);
#40=IFCWALLTYPE('0000000000000000000040',$,'Exterior',$,$,$,$,$,$,.STANDARD.);
#41=IFCRELASSOCIATESMATERIAL('0000000000000000000041',$,$,$,(#40),#35);
#42=IFCRELDEFINESBYTYPE('0000000000000000000042',$,$,$,(#1),#40);
#43=IFCRELASSOCIATESMATERIAL('0000000000000000000043',$,$,$,(#2),#37);
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,$,$,$);
#3=IFCWALL('0000000000000000000003',$,'W3',$,$,$,$,$,$);
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

/// `property-predicate`, its signature taken from the registry.
fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let parameters: serde_json::Map<String, Value> = registry
        .get("axioval:capability.property-predicate")
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
    let mut thickness = concept("axioval:test.total-thickness", "TotalThickness");
    thickness["valueKind"] = json!("quantity");
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": { "axioval:test.wall": concept("axioval:test.wall", "IfcWall") },
        "properties": { "axioval:test.total-thickness": thickness },
        "definitions": {
            "axioval:test.property-predicate": {
                "id": "axioval:test.property-predicate",
                "name": text("property-predicate"),
                "capability": "axioval:capability.property-predicate",
                "parameters": parameters,
            },
        },
    }))
    .unwrap()
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
            "id": "wall-thickness",
            "definitionId": "axioval:test.property-predicate",
            "name": text("wall-thickness"),
            "severity": "error",
            "parameters": {
                "property_set": { "type": "string", "value": MATERIAL_SET },
                "property": { "type": "string", "value": "axioval:test.total-thickness" },
                "operator": { "type": "string", "value": "greater_or_equal" },
                "quantity": { "type": "quantity", "value": 240.0, "unit": "mm" },
            },
            "applicability": {
                "kind": "entityType",
                "objectType": "axioval:test.wall",
                "includeSubtypes": false,
            },
        }] },
    }))
    .unwrap()
}

fn report() -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions(&registry)], &ruleset()).unwrap();
    let session = import_ifc_session("model.ifc", IFC.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn layer_set_thickness_is_checked_on_the_occurrence_and_through_the_type() {
    let report = report();
    let flagged: Vec<_> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.object_id().map_or("", |id| id.local_id.as_str()),
                finding.evidence[0].locator.rsplit_once(':').unwrap().1,
            )
        })
        .collect();
    // #1 is 300 mm through its type and passes; #2 is 175 mm and fails on
    // the evidence of its layer set; #3 has exactly no material.
    assert_eq!(flagged, [("#2", "#36"), ("#3", "TotalThickness")]);
    assert!(
        report.findings()[1]
            .message
            .ends_with("actual value is absent")
    );
    assert!(report.not_evaluated().is_empty());
}
