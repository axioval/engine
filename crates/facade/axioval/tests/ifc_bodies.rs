//! End to end: a compiled package reads how IFC bodies are modelled.
//!
//! Pins the join no adapter test covers: package property concepts bound to
//! names inside the reserved body set, a column's I-section depth and a
//! wall's extrusion direction, checked by an ordinary property capability.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{BODY_SET, DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// Millimetres. Column #10 is an HEA300 (290 mm deep), column #20 an HEB300
/// (300 mm deep); wall #30 is extruded straight up, wall #40 leans.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#93=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#91=IFCUNITASSIGNMENT((#90,#93));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#3=IFCLOCALPLACEMENT($,#2);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#11=IFCISHAPEPROFILEDEF(.AREA.,'HEA300',$,300.,290.,8.5,14.,27.,$,$);
#12=IFCEXTRUDEDAREASOLID(#11,#2,#4,3000.);
#13=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#12));
#14=IFCPRODUCTDEFINITIONSHAPE($,$,(#13));
#10=IFCCOLUMN('0000000000000000000010',$,'C1',$,$,#3,#14,$,.COLUMN.);
#21=IFCISHAPEPROFILEDEF(.AREA.,'HEB300',$,300.,300.,11.,19.,27.,$,$);
#22=IFCEXTRUDEDAREASOLID(#21,#2,#4,3000.);
#23=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#22));
#24=IFCPRODUCTDEFINITIONSHAPE($,$,(#23));
#20=IFCCOLUMN('0000000000000000000020',$,'C2',$,$,#3,#24,$,.COLUMN.);
#31=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,5000.,200.);
#32=IFCEXTRUDEDAREASOLID(#31,#2,#4,3000.);
#33=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#32));
#34=IFCPRODUCTDEFINITIONSHAPE($,$,(#33));
#30=IFCWALL('0000000000000000000030',$,'W1',$,$,#3,#34,$,.STANDARD.);
#41=IFCDIRECTION((0.,0.2,1.));
#42=IFCEXTRUDEDAREASOLID(#31,#2,#41,3000.);
#43=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#42));
#44=IFCPRODUCTDEFINITIONSHAPE($,$,(#43));
#40=IFCWALL('0000000000000000000040',$,'W2',$,$,#3,#44,$,.STANDARD.);
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
    let mut depth = concept("axioval:test.section-depth", "Profile.OverallDepth");
    depth["valueKind"] = json!("quantity");
    let mut inclination = concept("axioval:test.inclination", "Extrusion.Inclination");
    inclination["valueKind"] = json!("quantity");
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": {
            "axioval:test.column": concept("axioval:test.column", "IfcColumn"),
            "axioval:test.wall": concept("axioval:test.wall", "IfcWall"),
        },
        "properties": {
            "axioval:test.section-depth": depth,
            "axioval:test.inclination": inclination,
        },
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

fn rule(id: &str, object_type: &str, parameters: &Value) -> Value {
    json!({
        "id": id,
        "definitionId": "axioval:test.property-predicate",
        "name": text(id),
        "severity": "error",
        "parameters": parameters,
        "applicability": {
            "kind": "entityType",
            "objectType": object_type,
            "includeSubtypes": false,
        },
    })
}

fn ruleset() -> RuleSetPackage {
    let depth = rule(
        "column-depth",
        "axioval:test.column",
        &json!({
            "property_set": { "type": "string", "value": BODY_SET },
            "property": { "type": "string", "value": "axioval:test.section-depth" },
            "operator": { "type": "string", "value": "greater_or_equal" },
            "quantity": { "type": "quantity", "value": 300.0, "unit": "mm" },
        }),
    );
    let vertical = rule(
        "wall-vertical",
        "axioval:test.wall",
        &json!({
            "property_set": { "type": "string", "value": BODY_SET },
            "property": { "type": "string", "value": "axioval:test.inclination" },
            "operator": { "type": "string", "value": "less_or_equal" },
            "quantity": { "type": "quantity", "value": 0.5, "unit": "deg" },
        }),
    );
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [],
                  "rules": [depth, vertical] },
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
fn a_columns_section_and_a_walls_extrusion_are_checked_by_a_package() {
    let report = report();
    let flagged: Vec<_> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.rule_id.to_string(),
                finding.object_id().map_or("", |id| id.local_id.as_str()),
                finding.evidence[0].locator.rsplit_once(":body:").unwrap().1,
            )
        })
        .collect();
    // The HEA300 is 290 mm deep; the leaning wall is not extruded along z.
    assert_eq!(
        flagged,
        [
            ("column-depth".to_owned(), "#10", "#10:#13:#12:profile:#11"),
            ("wall-vertical".to_owned(), "#40", "#40:#43:#42"),
        ]
    );
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
}
