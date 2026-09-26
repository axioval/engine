//! End to end: the semantic capabilities as a compiled package over real IFC.
//!
//! Pins three joins no unit test covers: package concepts binding to IFC
//! attribute names inside the reserved attribute sets, IFC relationship
//! entity names driving scopes and counts, and a type object's name read
//! through `IfcRelDefinesByType`.
#![cfg(feature = "ifc")]
#![allow(missing_docs, clippy::needless_pass_by_value)]

use axioval::engine::{CapabilityRegistry, ParameterType, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use serde_json::{Value, json};

/// A building with two storeys, three spaces and one zone, in millimetres.
///
/// Spaces #10 and #11 are both numbered `101`; #12 is not in the zone; #11
/// is typed `KITCHEN`, which is not agreed. The storeys are named `1` and
/// `3`, so the second does not follow the first.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCBUILDING('0000000000000000000001',$,'B',$,$,$,$,$,.ELEMENT.,$,$,$);
#2=IFCBUILDINGSTOREY('0000000000000000000002',$,'1',$,$,$,$,$,.ELEMENT.,0.);
#3=IFCRELAGGREGATES('0000000000000000000003',$,$,$,#1,(#2,#4));
#4=IFCBUILDINGSTOREY('0000000000000000000004',$,'3',$,$,$,$,$,.ELEMENT.,3000.);
#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#6=IFCUNITASSIGNMENT((#5));
#7=IFCPROJECT('0000000000000000000007',$,'P',$,$,$,$,$,#6);
#10=IFCSPACE('000000000000000000000A',$,'101',$,$,$,$,'Office',.ELEMENT.,.INTERNAL.,$);
#11=IFCSPACE('000000000000000000000B',$,'101',$,$,$,$,'Kitchen',.ELEMENT.,.INTERNAL.,$);
#12=IFCSPACE('000000000000000000000C',$,'102',$,$,$,$,'Office',.ELEMENT.,.INTERNAL.,$);
#13=IFCRELAGGREGATES('000000000000000000000D',$,$,$,#2,(#10,#11,#12));
#20=IFCSPACETYPE('000000000000000000000K',$,'OFFICE',$,$,$,$,$,$,.SPACE.,$);
#21=IFCSPACETYPE('000000000000000000000L',$,'KITCHEN',$,$,$,$,$,$,.SPACE.,$);
#22=IFCRELDEFINESBYTYPE('000000000000000000000M',$,$,$,(#10,#12),#20);
#23=IFCRELDEFINESBYTYPE('000000000000000000000N',$,$,$,(#11),#21);
#30=IFCZONE('000000000000000000000Z',$,'Z1',$,$,$);
#31=IFCRELASSIGNSTOGROUP('000000000000000000000Y',$,$,$,(#10,#11),$,#30);
ENDSEC;
END-ISO-10303-21;
";

const ATTR: &str = axioval::ir::ATTRIBUTE_SET;
const TYPE_ATTR: &str = axioval::ir::TYPE_ATTRIBUTE_SET;

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

fn property_concept(id: &str, ifc_name: &str, value_kind: &str) -> Value {
    let mut concept = concept(id, ifc_name);
    concept["valueKind"] = json!(value_kind);
    concept
}

fn kind(parameter_type: ParameterType) -> &'static str {
    match parameter_type {
        ParameterType::Boolean => "boolean",
        ParameterType::Integer => "integer",
        ParameterType::Number => "number",
        ParameterType::String => "string",
        ParameterType::Quantity => "quantity",
        ParameterType::Enum => "enum",
        ParameterType::Reference => "reference",
        ParameterType::ObjectTypeReference => "objectTypeReference",
        ParameterType::PropertyReference => "propertyReference",
        ParameterType::Selector => "selector",
        ParameterType::StringList => "stringList",
        ParameterType::ReferenceList => "referenceList",
    }
}

/// One definition per capability, its signature taken from the registry.
fn definitions(registry: &CapabilityRegistry, capabilities: &[&str]) -> DefinitionPackage {
    let mut definitions = serde_json::Map::new();
    for capability in capabilities {
        let id = format!("axioval:test.{capability}");
        let parameters: serde_json::Map<String, Value> = registry
            .get(&format!("axioval:capability.{capability}"))
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
        definitions.insert(
            id.clone(),
            json!({
                "id": id,
                "name": text(capability),
                "capability": format!("axioval:capability.{capability}"),
                "parameters": parameters,
            }),
        );
    }
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": {
            "axioval:test.space": concept("axioval:test.space", "IfcSpace"),
            "axioval:test.zone": concept("axioval:test.zone", "IfcZone"),
            "axioval:test.building": concept("axioval:test.building", "IfcBuilding"),
            "axioval:test.storey": concept("axioval:test.storey", "IfcBuildingStorey"),
        },
        "properties": {
            "axioval:test.number": property_concept("axioval:test.number", "Name", "string"),
            "axioval:test.type-name": property_concept("axioval:test.type-name", "Name", "string"),
            "axioval:test.elevation": property_concept("axioval:test.elevation", "Elevation", "quantity"),
        },
        "definitions": definitions,
    }))
    .unwrap()
}

fn entity(concept: &str) -> Value {
    json!({ "kind": "entityType", "objectType": concept, "includeSubtypes": false })
}

fn rule(id: &str, capability: &str, applies_to: &str, parameters: Value) -> Value {
    json!({
        "id": id,
        "definitionId": format!("axioval:test.{capability}"),
        "name": text(id),
        "severity": "error",
        "parameters": parameters,
        "applicability": entity(applies_to),
    })
}

fn ruleset() -> RuleSetPackage {
    let rules = vec![
        rule(
            "unique-space-numbers",
            "unique-value",
            "axioval:test.space",
            json!({
                "property": { "type": "propertyReference", "property": "axioval:test.number", "propertySet": ATTR },
                "relationship": { "type": "string", "value": "IfcRelAggregates" },
                "direction": { "type": "string", "value": "backward" },
            }),
        ),
        rule(
            "spaces-in-zones",
            "related-count",
            "axioval:test.space",
            json!({
                "related_selector": { "type": "selector", "value": entity("axioval:test.zone") },
                "relationship": { "type": "string", "value": "IfcRelAssignsToGroup" },
                "direction": { "type": "string", "value": "backward" },
                "minimum": { "type": "integer", "value": 1 },
            }),
        ),
        rule(
            "agreed-space-types",
            "selector-conformance",
            "axioval:test.space",
            json!({
                "requirement": { "type": "selector", "value": {
                    "kind": "property",
                    "propertySet": TYPE_ATTR,
                    "property": "axioval:test.type-name",
                    "operator": "matches",
                    "value": { "type": "string", "value": "^(OFFICE|CORRIDOR)$" },
                }},
            }),
        ),
        rule(
            "storey-names",
            "name-sequence",
            "axioval:test.building",
            json!({
                "member_selector": { "type": "selector", "value": entity("axioval:test.storey") },
                "name": { "type": "propertyReference", "property": "axioval:test.number", "propertySet": ATTR },
                "order": { "type": "propertyReference", "property": "axioval:test.elevation", "propertySet": ATTR },
                "relationship": { "type": "string", "value": "IfcRelAggregates" },
            }),
        ),
    ];
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "rules": rules, "folders": [] },
    }))
    .unwrap()
}

fn report() -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(
        &registry,
        &[
            "unique-value",
            "related-count",
            "selector-conformance",
            "name-sequence",
        ],
    );
    let plan = compile(&registry, &[definitions], &ruleset()).unwrap();
    let session = import_ifc_session("model.ifc", IFC.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

fn flagged<'a>(report: &'a Report, rule: &str) -> Vec<&'a str> {
    report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(|finding| finding.object_id.local_id.as_str())
        .collect()
}

#[test]
fn space_numbers_repeat_within_their_storey() {
    assert_eq!(flagged(&report(), "unique-space-numbers"), ["#10", "#11"]);
}

#[test]
fn a_space_outside_every_zone_is_found_through_group_assignment() {
    assert_eq!(flagged(&report(), "spaces-in-zones"), ["#12"]);
}

#[test]
fn the_space_type_name_is_read_from_the_type_object() {
    assert_eq!(flagged(&report(), "agreed-space-types"), ["#11"]);
}

#[test]
fn storeys_are_ordered_by_their_elevation_in_si() {
    let report = report();
    let findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == "storey-names")
        .map(|finding| {
            (
                finding.object_id.local_id.as_str(),
                finding.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        findings,
        [(
            "#4",
            "axioval:attributes.axioval:test.number 3 does not follow 1; expected 2"
        )]
    );
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
}
