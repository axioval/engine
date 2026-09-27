//! End to end: `property-comparison` along a relationship path over real IFC.
//!
//! Fire walls are the walls stating a `Pset_WallCommon.FireRating`. The doors
//! filling their openings, reached through `IfcRelVoidsElement` then
//! `IfcRelFillsElement`, must be of a type whose name matches one of a list
//! of patterns; findings are categorised by the wall's fire rating.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// Fire wall #10 has openings #20 (door #30, type `T30-RS-1`) and #21 (door
/// #31, type `Standard`). Wall #11 states no fire rating; its opening #22
/// holds door #32, also `Standard`.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#10=IFCWALL('000000000000000000000A',$,'W1',$,$,$,$,$,.STANDARD.);
#11=IFCWALL('000000000000000000000B',$,'W2',$,$,$,$,$,.STANDARD.);
#12=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('F90'),$);
#13=IFCPROPERTYSET('000000000000000000000C',$,'Pset_WallCommon',$,(#12));
#14=IFCRELDEFINESBYPROPERTIES('000000000000000000000D',$,$,$,(#10),#13);
#20=IFCOPENINGELEMENT('000000000000000000000E',$,$,$,$,$,$,$,.OPENING.);
#21=IFCOPENINGELEMENT('000000000000000000000F',$,$,$,$,$,$,$,.OPENING.);
#22=IFCOPENINGELEMENT('000000000000000000000G',$,$,$,$,$,$,$,.OPENING.);
#23=IFCRELVOIDSELEMENT('000000000000000000000H',$,$,$,#10,#20);
#24=IFCRELVOIDSELEMENT('000000000000000000000I',$,$,$,#10,#21);
#25=IFCRELVOIDSELEMENT('000000000000000000000J',$,$,$,#11,#22);
#30=IFCDOOR('000000000000000000000K',$,'D1',$,$,$,$,$,$,$,.DOOR.,$,$);
#31=IFCDOOR('000000000000000000000L',$,'D2',$,$,$,$,$,$,$,.DOOR.,$,$);
#32=IFCDOOR('000000000000000000000M',$,'D3',$,$,$,$,$,$,$,.DOOR.,$,$);
#33=IFCRELFILLSELEMENT('000000000000000000000N',$,$,$,#20,#30);
#34=IFCRELFILLSELEMENT('000000000000000000000O',$,$,$,#21,#31);
#35=IFCRELFILLSELEMENT('000000000000000000000P',$,$,$,#22,#32);
#40=IFCDOORTYPE('000000000000000000000Q',$,'T30-RS-1',$,$,$,$,$,$,.DOOR.,.SINGLE_SWING_LEFT.,$,$);
#41=IFCDOORTYPE('000000000000000000000R',$,'Standard',$,$,$,$,$,$,.DOOR.,.SINGLE_SWING_LEFT.,$,$);
#42=IFCRELDEFINESBYTYPE('000000000000000000000S',$,$,$,(#30),#40);
#43=IFCRELDEFINESBYTYPE('000000000000000000000T',$,$,$,(#31,#32),#41);
ENDSEC;
END-ISO-10303-21;
";

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

/// The `property-comparison` definition, its signature taken from the registry.
fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let capability = "axioval:capability.property-comparison";
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
            "axioval:test.wall": concept("axioval:test.wall", "IfcWall"),
            "axioval:test.door": concept("axioval:test.door", "IfcDoor"),
        },
        "properties": {
            "axioval:test.fire-rating": property_concept("axioval:test.fire-rating", "FireRating", "string"),
            "axioval:test.type-name": property_concept("axioval:test.type-name", "Name", "string"),
        },
        "propertySets": {
            "axioval:test.wall-common": concept("axioval:test.wall-common", "Pset_WallCommon"),
        },
        "definitions": {
            "axioval:test.property-comparison": {
                "id": "axioval:test.property-comparison",
                "name": text("property-comparison"),
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

fn ruleset(case_sensitive: bool) -> RuleSetPackage {
    let fire_walls = json!({
        "kind": "allOf",
        "operands": [
            entity("axioval:test.wall"),
            {
                "kind": "property",
                "propertySet": "axioval:test.wall-common",
                "property": "axioval:test.fire-rating",
                "operator": "exists",
            },
        ],
    });
    let fire_rating = json!({ "type": "propertyReference", "property": "axioval:test.fire-rating", "propertySet": "axioval:test.wall-common" });
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
            "id": "fire-wall-door-types",
            "definitionId": "axioval:test.property-comparison",
            "name": text("fire-wall-door-types"),
            "severity": "error",
            "applicability": fire_walls,
            "parameters": {
                "compared_selector": { "type": "selector", "value": entity("axioval:test.door") },
                "compared_property": { "type": "propertyReference", "property": "axioval:test.type-name", "propertySet": TYPE_ATTR },
                "path": { "type": "stringList", "value": ["IfcRelVoidsElement:forward", "IfcRelFillsElement:forward"] },
                "component_mode": { "type": "string", "value": "related" },
                "quantifier": { "type": "string", "value": "each" },
                "operator": { "type": "string", "value": "like" },
                "target_texts": { "type": "stringList", "value": ["t30-*", "EI?30*"] },
                "case_sensitive": { "type": "boolean", "value": case_sensitive },
                "factor": { "type": "number", "value": 1.0 },
                "category_property": fire_rating,
            },
        }]},
    }))
    .unwrap()
}

fn report(case_sensitive: bool) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry);
    let plan = compile(&registry, &[definitions], &ruleset(case_sensitive)).unwrap();
    let session = import_ifc_session("model.ifc", IFC.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

fn findings(report: &Report) -> Vec<(&str, &str)> {
    report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.object_id().map_or("", |id| id.local_id.as_str()),
                finding.message.as_str(),
            )
        })
        .collect()
}

#[test]
fn fire_wall_door_types_are_checked_through_voids_and_fills_against_a_pattern_list() {
    let report = report(false);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // #30's type matches `t30-*` ignoring case; #31's matches nothing; #32
    // is in a wall that is no fire wall.
    assert_eq!(
        findings(&report),
        [(
            "#10",
            "[F90] candidate ifc-step:model.ifc/#31 does not satisfy comparison"
        )]
    );
    let finding = &report.findings()[0];
    for cited in ["IfcRelVoidsElement", "IfcRelFillsElement"] {
        assert!(
            finding
                .evidence
                .iter()
                .any(|evidence| evidence.locator.contains(cited)),
            "{cited} not cited: {:?}",
            finding.evidence
        );
    }
}

#[test]
fn a_case_sensitive_pattern_list_rejects_a_differently_cased_type() {
    let report = report(true);
    let mut flagged: Vec<_> = findings(&report).into_iter().map(|(_, m)| m).collect();
    flagged.sort_unstable();
    assert_eq!(
        flagged,
        [
            "[F90] candidate ifc-step:model.ifc/#30 does not satisfy comparison",
            "[F90] candidate ifc-step:model.ifc/#31 does not satisfy comparison",
        ]
    );
}
