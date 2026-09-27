//! End to end: a `related` selector picks fire-wall doors over real IFC.
//!
//! A door is a fire-wall door when the wall whose opening it fills, reached
//! through `IfcRelFillsElement` then `IfcRelVoidsElement` backwards, states
//! `Pset_WallCommon.Compartmentation` true. Those doors must state a
//! `Pset_DoorCommon.FireRating`.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use axioval::engine::{CapabilityRegistry, ParameterType, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use serde_json::{Value, json};

/// Fire wall #10 has openings #20 (door #30, rated) and #21 (door #31,
/// unrated). Wall #11 is no compartment wall; its door #32 is unrated. Wall
/// #12 states nothing; its door #33 is unrated. Door #34 fills no opening.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#10=IFCWALL('000000000000000000000A',$,'W1',$,$,$,$,$,.STANDARD.);
#11=IFCWALL('000000000000000000000B',$,'W2',$,$,$,$,$,.STANDARD.);
#12=IFCWALL('000000000000000000000C',$,'W3',$,$,$,$,$,.STANDARD.);
#13=IFCPROPERTYSINGLEVALUE('Compartmentation',$,IFCBOOLEAN(.T.),$);
#14=IFCPROPERTYSET('000000000000000000000D',$,'Pset_WallCommon',$,(#13));
#15=IFCRELDEFINESBYPROPERTIES('000000000000000000000E',$,$,$,(#10),#14);
#16=IFCPROPERTYSINGLEVALUE('Compartmentation',$,IFCBOOLEAN(.F.),$);
#17=IFCPROPERTYSET('000000000000000000000F',$,'Pset_WallCommon',$,(#16));
#18=IFCRELDEFINESBYPROPERTIES('000000000000000000000G',$,$,$,(#11),#17);
#20=IFCOPENINGELEMENT('000000000000000000000H',$,$,$,$,$,$,$,.OPENING.);
#21=IFCOPENINGELEMENT('000000000000000000000I',$,$,$,$,$,$,$,.OPENING.);
#22=IFCOPENINGELEMENT('000000000000000000000J',$,$,$,$,$,$,$,.OPENING.);
#23=IFCOPENINGELEMENT('000000000000000000000K',$,$,$,$,$,$,$,.OPENING.);
#24=IFCRELVOIDSELEMENT('000000000000000000000L',$,$,$,#10,#20);
#25=IFCRELVOIDSELEMENT('000000000000000000000M',$,$,$,#10,#21);
#26=IFCRELVOIDSELEMENT('000000000000000000000N',$,$,$,#11,#22);
#27=IFCRELVOIDSELEMENT('000000000000000000000O',$,$,$,#12,#23);
#30=IFCDOOR('000000000000000000000P',$,'D1',$,$,$,$,$,$,$,.DOOR.,$,$);
#31=IFCDOOR('000000000000000000000Q',$,'D2',$,$,$,$,$,$,$,.DOOR.,$,$);
#32=IFCDOOR('000000000000000000000R',$,'D3',$,$,$,$,$,$,$,.DOOR.,$,$);
#33=IFCDOOR('000000000000000000000S',$,'D4',$,$,$,$,$,$,$,.DOOR.,$,$);
#34=IFCDOOR('000000000000000000000T',$,'D5',$,$,$,$,$,$,$,.DOOR.,$,$);
#35=IFCRELFILLSELEMENT('000000000000000000000U',$,$,$,#20,#30);
#36=IFCRELFILLSELEMENT('000000000000000000000V',$,$,$,#21,#31);
#37=IFCRELFILLSELEMENT('000000000000000000000W',$,$,$,#22,#32);
#38=IFCRELFILLSELEMENT('000000000000000000000X',$,$,$,#23,#33);
#40=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI30'),$);
#41=IFCPROPERTYSET('000000000000000000000Y',$,'Pset_DoorCommon',$,(#40));
#42=IFCRELDEFINESBYPROPERTIES('000000000000000000000Z',$,$,$,(#30),#41);
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
        ParameterType::Table(_) => "table",
    }
}

/// The `property-required` definition, its signature taken from the registry.
fn definitions(registry: &CapabilityRegistry) -> DefinitionPackage {
    let capability = "axioval:capability.property-required";
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
            "axioval:test.compartmentation": property_concept("axioval:test.compartmentation", "Compartmentation", "boolean"),
            "axioval:test.fire-rating": property_concept("axioval:test.fire-rating", "FireRating", "string"),
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

fn ruleset() -> RuleSetPackage {
    let fire_wall_doors = json!({
        "kind": "allOf",
        "operands": [
            entity("axioval:test.door"),
            {
                "kind": "related",
                "path": ["IfcRelFillsElement:backward", "IfcRelVoidsElement:backward"],
                "selector": {
                    "kind": "allOf",
                    "operands": [
                        entity("axioval:test.wall"),
                        {
                            "kind": "property",
                            "propertySet": "axioval:test.wall-common",
                            "property": "axioval:test.compartmentation",
                            "operator": "equals",
                            "value": { "type": "boolean", "value": true },
                        },
                    ],
                },
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
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [{
            "id": "fire-wall-doors-are-rated",
            "definitionId": "axioval:test.property-required",
            "name": text("fire-wall-doors-are-rated"),
            "severity": "error",
            "applicability": fire_wall_doors,
            "parameters": {
                "property": {
                    "type": "propertyReference",
                    "property": "axioval:test.fire-rating",
                    "propertySet": "axioval:test.door-common",
                },
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
fn fire_wall_doors_are_selected_through_fills_and_voids() {
    let report = report();
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // #30 is rated; #32's wall is no compartment wall, #33's states none
    // and #34 is in no wall, so only #31 is a finding.
    let flagged: Vec<&str> = report
        .findings()
        .iter()
        .filter_map(|finding| finding.object_id().map(|id| id.local_id.as_str()))
        .collect();
    assert_eq!(flagged, ["#31"]);
}
