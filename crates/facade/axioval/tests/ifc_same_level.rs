//! End to end: storeys of several IFC models matched as one level.
//!
//! The architecture model states its storeys in millimetres, the MEP model
//! in metres. A duct on the MEP model's upper storey and the slab on the
//! architecture model's upper storey are candidates of one another only
//! when `axioval:derived.same-level` matches the two storeys.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, EvidenceSession, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, NotEvaluatedReason, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

fn file(unit_prefix: &str, data: &str) -> String {
    format!(
        "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCSIUNIT(*,.LENGTHUNIT.,{unit_prefix},.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#100=IFCBUILDING('0000000000000000000100',$,'B',$,$,$,$,$,.ELEMENT.,$,$,$);
#104=IFCRELAGGREGATES('0000000000000000000104',$,$,$,#3,(#100));
{data}ENDSEC;
END-ISO-10303-21;
"
    )
}

/// Storeys at 0 and 3000 mm; slab #10 on the upper one, #11 on the lower.
fn architecture() -> String {
    file(
        ".MILLI.",
        "#101=IFCBUILDINGSTOREY('0000000000000000000101',$,'EG',$,$,$,$,$,.ELEMENT.,0.);
#102=IFCBUILDINGSTOREY('0000000000000000000102',$,'OG',$,$,$,$,$,.ELEMENT.,3000.);
#103=IFCRELAGGREGATES('0000000000000000000103',$,$,$,#100,(#101,#102));
#10=IFCSLAB('000000000000000000A010',$,'S-OG',$,$,$,$,$,.FLOOR.);
#11=IFCSLAB('000000000000000000A011',$,'S-EG',$,$,$,$,$,.FLOOR.);
#40=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000040',$,$,$,(#10),#102);
#41=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000041',$,$,$,(#11),#101);
",
    )
}

/// Duct #20 on the storey at `elevation` metres (unset when `None`).
fn mep(elevation: Option<f64>) -> String {
    let elevation = elevation.map_or_else(|| "$".to_owned(), |metres| format!("{metres:?}"));
    file(
        "$",
        &format!(
            "#101=IFCBUILDINGSTOREY('0000000000000000000101',$,'Level 1',$,$,$,$,$,.ELEMENT.,{elevation});
#103=IFCRELAGGREGATES('0000000000000000000103',$,$,$,#100,(#101));
#20=IFCDUCTSEGMENT('000000000000000000M020',$,'D1',$,$,$,$,$,.RIGIDSEGMENT.);
#40=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000040',$,$,$,(#20),#101);
"
        ),
    )
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

fn property(id: &str, ifc_name: &str, value_kind: &str) -> Value {
    let mut property = concept(id, ifc_name);
    property["valueKind"] = json!(value_kind);
    property
}

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
    let comparison = "axioval:capability.property-comparison";
    let same = "axioval:capability.same-container";
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": {
            "axioval:test.duct": concept("axioval:test.duct", "IfcDuctSegment"),
            "axioval:test.slab": concept("axioval:test.slab", "IfcSlab"),
            "axioval:test.storey": concept("axioval:test.storey", "IfcBuildingStorey"),
        },
        "properties": {
            "axioval:test.elevation": property("axioval:test.elevation", "Elevation", "quantity"),
            "axioval:test.name": property("axioval:test.name", "Name", "string"),
        },
        "definitions": {
            "axioval:test.comparison": {
                "id": "axioval:test.comparison",
                "name": text("comparison"),
                "capability": comparison,
                "parameters": signature(registry, comparison),
            },
            "axioval:test.same-container": {
                "id": "axioval:test.same-container",
                "name": text("same-container"),
                "capability": same,
                "parameters": signature(registry, same),
            },
        },
    }))
    .unwrap()
}

fn entity(concept: &str) -> Value {
    json!({ "kind": "entityType", "objectType": concept, "includeSubtypes": false })
}

fn climb() -> [(&'static str, Value); 3] {
    [
        (
            "container_selector",
            json!({ "type": "selector", "value": entity("axioval:test.storey") }),
        ),
        (
            "relationship",
            json!({ "type": "string", "value": "IfcRelContainedInSpatialStructure" }),
        ),
        (
            "direction",
            json!({ "type": "string", "value": "backward" }),
        ),
    ]
}

/// "A duct has no slab on its storey": a finding counts the candidates.
/// `level` is the identity and the property concept it compares.
fn comparison(level: Option<(&str, &str)>) -> Value {
    let mut parameters = json!({
        "compared_selector": { "type": "selector", "value": entity("axioval:test.slab") },
        "target_number": { "type": "number", "value": 0.0 },
        "operator": { "type": "string", "value": "equals" },
        "factor": { "type": "number", "value": 1.0 },
        "component_mode": { "type": "string", "value": "same_space" },
        "quantifier": { "type": "string", "value": "count" },
    });
    for (name, value) in climb() {
        parameters[name] = value;
    }
    if let Some((level, property)) = level {
        parameters["container_relationship"] = json!({ "type": "string", "value": level });
        parameters["level_property"] = json!({
            "type": "propertyReference",
            "property": property,
            "propertySet": "axioval:attributes",
        });
    }
    json!({
        "id": "slabs-on-the-duct-level",
        "definitionId": "axioval:test.comparison",
        "name": text("slabs-on-the-duct-level"),
        "severity": "error",
        "applicability": entity("axioval:test.duct"),
        "parameters": parameters,
    })
}

fn run(session: &EvidenceSession, rule: &Value) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry);
    let ruleset: RuleSetPackage = serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [rule] },
    }))
    .unwrap();
    let plan = compile(&registry, &[definitions], &ruleset).unwrap();
    Runtime::new(registry).run_session(session, plan).unwrap()
}

fn federation(elevation: Option<f64>) -> EvidenceSession {
    EvidenceSession::federate([
        import_ifc_session("arch.ifc", architecture().as_bytes()).unwrap(),
        import_ifc_session("mep.ifc", mep(elevation).as_bytes()).unwrap(),
    ])
    .unwrap()
}

fn messages(report: &Report) -> Vec<String> {
    report
        .findings()
        .iter()
        .map(|finding| format!("{}: {}", finding.object_id().unwrap(), finding.message))
        .collect()
}

#[test]
fn a_duct_and_the_slab_on_the_matching_storey_are_same_level_candidates() {
    let level = Some((
        "axioval:derived.same-level;tolerance=0.01",
        "axioval:test.elevation",
    ));
    let report = run(&federation(Some(3.0)), &comparison(level));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let [finding] = report.findings() else {
        panic!("{:?}", report.findings());
    };
    assert_eq!(finding.object_id().unwrap().local_id, "#20");
    assert!(finding.message.contains('1'), "{}", finding.message);
    // The match cites both storeys' elevations and the level derivation.
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.locator.starts_with("axioval:derived.same-level")),
        "{:?}",
        finding.evidence
    );

    // Without the level match a storey is only its own model's.
    let report = run(&federation(Some(3.0)), &comparison(None));
    assert!(report.findings().is_empty(), "{:?}", messages(&report));
    assert!(report.not_evaluated().is_empty());

    // A storey half a metre off is another level.
    let report = run(&federation(Some(3.5)), &comparison(level));
    assert!(report.findings().is_empty(), "{:?}", messages(&report));
    assert!(report.not_evaluated().is_empty());
}

#[test]
fn a_storey_without_an_elevation_is_undecided_never_another_level() {
    let report = run(
        &federation(None),
        &comparison(Some((
            "axioval:derived.same-level",
            "axioval:test.elevation",
        ))),
    );
    assert!(report.findings().is_empty(), "{:?}", messages(&report));
    let [outcome] = report.not_evaluated() else {
        panic!("{:?}", report.not_evaluated());
    };
    assert_eq!(outcome.reason, NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn levels_match_by_name_and_malformed_identities_are_refused() {
    // The storey names differ, so by name the duct has no slab.
    let report = run(
        &federation(Some(3.0)),
        &comparison(Some((
            "axioval:derived.same-level;by=name",
            "axioval:test.name",
        ))),
    );
    assert!(report.findings().is_empty(), "{:?}", messages(&report));
    assert!(report.not_evaluated().is_empty());

    let report = run(
        &federation(Some(3.0)),
        &comparison(Some((
            "axioval:derived.same-level;by=name;tolerance=1",
            "axioval:test.name",
        ))),
    );
    assert!(report.findings().is_empty());
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::InvalidDeclaration
    );
}
