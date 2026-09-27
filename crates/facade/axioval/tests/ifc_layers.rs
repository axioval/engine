//! End to end: layer agreement over every presentation layer of an object.
//!
//! A compiled package checks walls against an agreed layer list, once
//! requiring every layer to be agreed and once requiring at least one to be,
//! and a model without layer assignments reports the rules not applicable to
//! its source instead of passing or flagging each wall.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{
    DefinitionPackage, NotEvaluatedReason, PRESENTATION_SET, Report, RuleSetPackage, Scope,
};
use axioval::rules::register_builtins;
use serde_json::{Value, json};

/// #1 is on A-WALL, #2 on A-WALL and A-AXIS, #3 on A-AXIS alone, #4 has a
/// shape on no layer.
const LAYERED: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,#10,$,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,#20,$,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21,#22));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#22=IFCSHAPEREPRESENTATION(#92,'Axis','Curve2D',(#90));
#3=IFCWALL('0000000000000000000003',$,'W3',$,$,$,#30,$,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31));
#31=IFCSHAPEREPRESENTATION(#92,'Axis','Curve2D',(#90));
#4=IFCWALL('0000000000000000000004',$,'W4',$,$,$,#40,$,$);
#40=IFCPRODUCTDEFINITIONSHAPE($,$,(#41));
#41=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#80=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#11,#21),$);
#81=IFCPRESENTATIONLAYERASSIGNMENT('A-AXIS',$,(#22,#31),$);
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
        "objectTypes": {
            "axioval:test.wall": {
                "id": "axioval:test.wall",
                "name": text("wall"),
                "externalNames": [{ "typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcWall" }],
            },
        },
        "properties": {
            "axioval:test.layer": {
                "id": "axioval:test.layer",
                "name": text("layer"),
                "valueKind": "stringList",
                "externalNames": [{ "typeSystem": IFC4_TYPE_SYSTEM, "name": "Layer" }],
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

fn layer_rule(id: &str, quantifier: &str, agreed: &[&str]) -> Value {
    json!({
        "id": id,
        "definitionId": "axioval:test.conformance",
        "name": text(id),
        "severity": "error",
        "parameters": {
            "requirement": { "type": "selector", "value": {
                "kind": "property",
                "propertySet": PRESENTATION_SET,
                "property": "axioval:test.layer",
                "operator": "oneOf",
                "value": { "type": "stringList", "value": agreed },
                "quantifier": quantifier,
            }},
        },
        "applicability": { "kind": "entityType", "objectType": "axioval:test.wall", "includeSubtypes": false },
    })
}

fn ruleset() -> RuleSetPackage {
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": { "id": "axioval:test.ruleset", "name": text("test"), "version": "0.1.0", "authors": [] },
        "definitionPackages": ["axioval:test.definitions"],
        "root": {
            "id": "root",
            "name": text("root"),
            "rules": [
                layer_rule("every-layer-agreed", "all", &["A-WALL"]),
                layer_rule("some-layer-agreed", "any", &["A-WALL"]),
            ],
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

fn flagged<'a>(report: &'a Report, rule: &str) -> Vec<(&'a str, &'a str)> {
    report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(|finding| {
            (
                finding.object_id().unwrap().local_id.as_str(),
                finding.message.as_str(),
            )
        })
        .collect()
}

#[test]
fn an_object_on_two_layers_is_checked_against_both() {
    let report = report(LAYERED);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // Every layer must be agreed: #2's A-AXIS fails it, #3 fails outright.
    let every = flagged(&report, "every-layer-agreed");
    let objects: Vec<_> = every.iter().map(|(object, _)| *object).collect();
    assert_eq!(objects, ["#2", "#3", "#4"], "{every:?}");
    assert!(
        every
            .iter()
            .any(|(object, message)| *object == "#2" && message.contains("[`A-AXIS`, `A-WALL`]")),
        "{every:?}"
    );
    assert!(
        every
            .iter()
            .any(|(object, message)| *object == "#4" && message.contains("has no value")),
        "{every:?}"
    );
    // One agreed layer suffices: #2 passes on A-WALL.
    let some: Vec<_> = flagged(&report, "some-layer-agreed")
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    assert_eq!(some, ["#3", "#4"]);
}

#[test]
fn a_model_without_layers_is_not_applicable_rather_than_passing() {
    let unlayered: String = LAYERED
        .lines()
        .filter(|line| !line.contains("IFCPRESENTATIONLAYERASSIGNMENT"))
        .flat_map(|line| [line, "\n"])
        .collect();
    let report = report(&unlayered);
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    let outcomes: Vec<_> = report
        .not_evaluated()
        .iter()
        .map(|outcome| (outcome.rule_id.to_string(), &outcome.scope, &outcome.reason))
        .collect();
    assert_eq!(outcomes.len(), 2, "{:?}", report.not_evaluated());
    for (_, scope, reason) in outcomes {
        assert!(
            matches!(scope, Scope::Source(source) if source.document == "model.ifc"),
            "{scope:?}"
        );
        assert_eq!(*reason, NotEvaluatedReason::NotRecorded);
    }
    let message = &report.not_evaluated()[0].message;
    assert!(
        message.contains("assigns no presentation layers") && message.contains("4 object(s)"),
        "{message}"
    );
}
