//! End to end: a blocker selector that leaves out transparent objects.
//!
//! A view-blocking selection keeps every object unless all of its styled
//! surfaces are at least half transparent: glazing drops out, an opaque wall,
//! a window with an opaque frame and an unstyled object stay in.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, PRESENTATION_SET, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use serde_json::{Value, json};

/// #1 is glazing (0.7), #2 an opaque wall, #3 glazing in a frame tinted 0.2,
/// #4 a wall without styles.
const STYLED: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#40=IFCCOLOURRGB($,0.5,0.5,0.5);
#41=IFCSURFACESTYLESHADING(#40,0.7);
#42=IFCSURFACESTYLESHADING(#40,$);
#43=IFCSURFACESTYLESHADING(#40,0.2);
#50=IFCSURFACESTYLE('Glass',.BOTH.,(#41));
#51=IFCSURFACESTYLE('Concrete',.BOTH.,(#42));
#52=IFCSURFACESTYLE('Frame',.BOTH.,(#43));
#1=IFCWINDOW('0000000000000000000001',$,'G1',$,$,$,#10,$,$,$,$,$,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#12));
#12=IFCCARTESIANPOINT((1.,0.,0.));
#13=IFCSTYLEDITEM(#12,(#50),$);
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,#20,$,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#22));
#22=IFCCARTESIANPOINT((2.,0.,0.));
#23=IFCSTYLEDITEM(#22,(#51),$);
#3=IFCWINDOW('0000000000000000000003',$,'G3',$,$,$,#30,$,$,$,$,$,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31));
#31=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#32,#33));
#32=IFCCARTESIANPOINT((3.,0.,0.));
#33=IFCCARTESIANPOINT((3.,1.,0.));
#34=IFCSTYLEDITEM(#32,(#50),$);
#35=IFCSTYLEDITEM(#33,(#52),$);
#4=IFCWALL('0000000000000000000004',$,'W4',$,$,$,#60,$,$);
#60=IFCPRODUCTDEFINITIONSHAPE($,$,(#61));
#61=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#62));
#62=IFCCARTESIANPOINT((4.,0.,0.));
";

fn model(data: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn definitions() -> DefinitionPackage {
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": { "id": "axioval:test.definitions", "name": text("test"), "version": "0.1.0", "authors": [] },
        "objectTypes": {},
        "properties": {
            "axioval:test.transparency": {
                "id": "axioval:test.transparency",
                "name": text("transparency"),
                "valueKind": "number",
                "externalNames": [{ "typeSystem": IFC4_TYPE_SYSTEM, "name": "Transparency" }],
            },
        },
        "definitions": {
            "axioval:test.conformance": {
                "id": "axioval:test.conformance",
                "name": text("conformance"),
                "capability": "axioval:capability.selector-conformance",
                "parameters": {
                    "requirement": { "id": "requirement", "name": text("requirement"), "kind": "selector", "required": true },
                    "message": { "id": "message", "name": text("message"), "kind": "string", "required": false },
                },
            },
        },
    }))
    .unwrap()
}

/// Flags every object the blocker selector keeps.
fn ruleset() -> RuleSetPackage {
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": { "id": "axioval:test.ruleset", "name": text("test"), "version": "0.1.0", "authors": [] },
        "definitionPackages": ["axioval:test.definitions"],
        "root": {
            "id": "root",
            "name": text("root"),
            "rules": [{
                "id": "blockers",
                "definitionId": "axioval:test.conformance",
                "name": text("blockers"),
                "severity": "error",
                "parameters": {
                    "requirement": { "type": "selector", "value": {
                        "kind": "not", "operand": { "kind": "all" },
                    }},
                },
                "applicability": { "kind": "not", "operand": {
                    "kind": "property",
                    "propertySet": PRESENTATION_SET,
                    "property": "axioval:test.transparency",
                    "operator": "greaterThanOrEquals",
                    "value": { "type": "number", "value": 0.5 },
                    "quantifier": "all",
                }},
            }],
            "folders": [],
        },
    }))
    .unwrap()
}

fn report(data: &str) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions()], &ruleset()).unwrap();
    let session = import_ifc_session("model.ifc", model(data).as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn only_objects_transparent_on_every_surface_stop_blocking() {
    let report = report(STYLED);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let mut blockers: Vec<_> = report
        .findings()
        .iter()
        .map(|finding| finding.object_id().unwrap().local_id.as_str())
        .collect();
    blockers.sort_unstable();
    blockers.dedup();
    assert_eq!(blockers, ["#2", "#3", "#4"]);
}
