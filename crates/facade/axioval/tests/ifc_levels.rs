//! End to end: "on storeys more than 7 m above ground" as an expression
//! over level values, across a federation of two IFC models, each measured
//! from its own ground storey and told apart by its discipline.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use std::fmt::Write as _;

use axioval::engine::{CapabilityRegistry, EvidenceSession, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, Discipline, Report, RuleSetPackage, SourceId};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// A model in metres with storeys `(id, elevation)` and spaces `(id,
/// storey id)` aggregated by them.
fn model(storeys: &[(u32, f64)], spaces: &[(u32, u32)]) -> String {
    let mut data = String::from(
        "#90=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,$,#91);
#1=IFCBUILDING('0000000000000000000001',$,'B',$,$,$,$,$,.ELEMENT.,$,$,$);
",
    );
    let guid = |id: u32| format!("{id:0>22}");
    let storeys_of_building: Vec<String> = storeys.iter().map(|(id, _)| format!("#{id}")).collect();
    for (id, elevation) in storeys {
        let (placement, axis, point) = (id + 1000, id + 2000, id + 3000);
        writeln!(
            data,
            "#{point}=IFCCARTESIANPOINT((0.,0.,{elevation:?}));
#{axis}=IFCAXIS2PLACEMENT3D(#{point},$,$);
#{placement}=IFCLOCALPLACEMENT($,#{axis});
#{id}=IFCBUILDINGSTOREY('{}',$,'S{id}',$,$,#{placement},$,$,.ELEMENT.,{elevation:?});",
            guid(*id)
        )
        .unwrap();
    }
    writeln!(
        data,
        "#2=IFCRELAGGREGATES('0000000000000000000002',$,$,$,#1,({}));",
        storeys_of_building.join(",")
    )
    .unwrap();
    for (space, storey) in spaces {
        writeln!(
            data,
            "#{space}=IFCSPACE('{}',$,'R{space}',$,$,$,$,$,.ELEMENT.,.INTERNAL.,$);
#{}=IFCRELAGGREGATES('{}',$,$,$,#{storey},(#{space}));",
            guid(*space),
            space + 5000,
            guid(space + 5000)
        )
        .unwrap();
    }
    format!(
        "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
{data}ENDSEC;
END-ISO-10303-21;
"
    )
}

fn source(document: &str) -> SourceId {
    SourceId::new("ifc-step", document).unwrap()
}

/// An architectural model with storeys at 0, 3.5, 7 and 10.5 m, and a
/// structural one with storeys at -3, 0, 4 and 8.5 m, a space on each.
fn federation() -> EvidenceSession {
    let member = |document: &str, ifc: String, discipline: &str| {
        import_ifc_session(document, ifc.as_bytes())
            .unwrap()
            .with_discipline(&source(document), Discipline::new(discipline).unwrap())
            .unwrap()
    };
    EvidenceSession::federate([
        member(
            "arch.ifc",
            model(
                &[(10, 0.0), (11, 3.5), (12, 7.0), (13, 10.5)],
                &[(20, 10), (22, 12), (23, 13)],
            ),
            "architecture",
        ),
        member(
            "struct.ifc",
            model(
                &[(10, -3.0), (11, 0.0), (12, 4.0), (13, 8.5)],
                &[(20, 10), (23, 13)],
            ),
            "structure",
        ),
    ])
    .unwrap()
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

const EXPRESSION: &str = "axioval:capability.expression";

fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let parameters: serde_json::Map<String, Value> = registry
        .get(EXPRESSION)
        .unwrap()
        .parameters()
        .into_iter()
        .map(|descriptor| {
            (
                descriptor.name.clone(),
                json!({"id": descriptor.name, "name": text(&descriptor.name),
                    "kind": kind(descriptor.parameter_type), "required": descriptor.required}),
            )
        })
        .collect();
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {"id": "axioval:test.definitions", "name": text("test"),
            "version": "0.1.0", "authors": []},
        "objectTypes": {"axioval:test.space": {"id": "axioval:test.space", "name": text("space"),
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSpace"}]}},
        "definitions": {"axioval:test.expression": {"id": "axioval:test.expression",
            "name": text("expression"), "capability": EXPRESSION, "parameters": parameters}},
    }))
    .unwrap()
}

fn measured(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:measured",
        "property": format!("{name};path=IfcRelAggregates:backward")})
}

#[allow(clippy::needless_pass_by_value)]
fn at_most(left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals", "left": left, "right": right})
}

fn metres(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}})
}

fn rule(id: &str, requirement: &Value) -> Value {
    json!({"id": id, "definitionId": "axioval:test.expression", "name": text(id),
        "severity": "error",
        "applicability": {"kind": "entityType", "objectType": "axioval:test.space",
            "includeSubtypes": false},
        "parameters": {"requirement": {"type": "expression", "value": requirement}}})
}

fn ruleset() -> RuleSetPackage {
    let low = at_most(measured("height_above_ground"), metres(7.0));
    let architecture = json!({"kind": "compare", "operator": "equals",
        "left": {"kind": "property", "propertySet": "axioval:source", "property": "discipline"},
        "right": {"kind": "literal", "value": {"type": "string", "value": "architecture"}}});
    let low_architecture =
        json!({"kind": "implies", "antecedent": architecture, "consequent": low});
    let third = at_most(
        measured("level_index"),
        json!({"kind": "literal", "value": {"type": "integer", "value": 2}}),
    );
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {"id": "axioval:test.ruleset", "name": text("test"),
            "version": "0.1.0", "authors": []},
        "definitionPackages": ["axioval:test.definitions"],
        "root": {"id": "root", "name": text("root"), "folders": [], "rules": [
            rule("at-most-7-m-above-ground", &low),
            rule("architecture-at-most-7-m-above-ground", &low_architecture),
            rule("at-most-the-third-storey", &third),
        ]},
    }))
    .unwrap()
}

fn run() -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry);
    let plan = compile(&registry, &[definitions], &ruleset()).unwrap();
    Runtime::new(registry)
        .run_session(&federation(), plan)
        .unwrap()
}

fn found(report: &Report, rule: &str) -> Vec<String> {
    let mut found: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(|finding| finding.object_id().unwrap().to_string())
        .collect();
    found.sort();
    found
}

#[test]
fn storeys_above_ground_are_measured_from_each_models_own_ground() {
    let report = run();
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    // Architecture: 10.5 m above its ground; structure: 8.5 m above its
    // ground at 0 m, its basement at -3 m below it.
    assert_eq!(
        found(&report, "at-most-7-m-above-ground"),
        ["ifc-step:arch.ifc/#23", "ifc-step:struct.ifc/#23"]
    );
    // Only the architectural model's objects are held to it.
    assert_eq!(
        found(&report, "architecture-at-most-7-m-above-ground"),
        ["ifc-step:arch.ifc/#23"]
    );
    // Indices count from each model's ground: the architectural top storey
    // is the fourth (3), the structural top the third above its basement (2).
    assert_eq!(
        found(&report, "at-most-the-third-storey"),
        ["ifc-step:arch.ifc/#23"]
    );
}
