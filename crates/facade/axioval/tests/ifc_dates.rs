//! End to end: a compiled package compares IFC dates with a date literal.
//!
//! Pins the join no adapter test covers: `IfcDate`, `IfcDateTime` and
//! `IfcTimeStamp` values read by the IFC adapter, compared by
//! `property-predicate` at day precision with a `date` literal the package
//! states, and an `IfcDateTime` without a UTC offset left not evaluated.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

mod common;
use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::{DefinitionPackage, NotEvaluatedReason, Report, RuleSetPackage};
use axioval::rules::register_builtins;
use common::kind;
use serde_json::{Value, json};

/// #1 was inspected on 1 September, #2 late on 30 September where it was
/// recorded (1 October in UTC), #3 at a local time with no offset, #4 by
/// time stamp in November 2023, #5 never.
const IFC: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCDOOR('0000000000000000000001',$,'D1',$,$,$,$,$,$,$,$,$,$);
#2=IFCDOOR('0000000000000000000002',$,'D2',$,$,$,$,$,$,$,$,$,$);
#3=IFCDOOR('0000000000000000000003',$,'D3',$,$,$,$,$,$,$,$,$,$);
#4=IFCDOOR('0000000000000000000004',$,'D4',$,$,$,$,$,$,$,$,$,$);
#5=IFCDOOR('0000000000000000000005',$,'D5',$,$,$,$,$,$,$,$,$,$);
#11=IFCPROPERTYSINGLEVALUE('LastInspection',$,IFCDATE('2026-09-01'),$);
#12=IFCPROPERTYSINGLEVALUE('LastInspection',$,IFCDATETIME('2026-09-30T23:30:00-02:00'),$);
#13=IFCPROPERTYSINGLEVALUE('LastInspection',$,IFCDATETIME('2026-09-30T12:00:00'),$);
#14=IFCPROPERTYSINGLEVALUE('LastInspection',$,IFCTIMESTAMP(1700000000),$);
#21=IFCPROPERTYSET('0000000000000000000021',$,'Pset_Inspection',$,(#11));
#22=IFCPROPERTYSET('0000000000000000000022',$,'Pset_Inspection',$,(#12));
#23=IFCPROPERTYSET('0000000000000000000023',$,'Pset_Inspection',$,(#13));
#24=IFCPROPERTYSET('0000000000000000000024',$,'Pset_Inspection',$,(#14));
#31=IFCRELDEFINESBYPROPERTIES('0000000000000000000031',$,$,$,(#1),#21);
#32=IFCRELDEFINESBYPROPERTIES('0000000000000000000032',$,$,$,(#2),#22);
#33=IFCRELDEFINESBYPROPERTIES('0000000000000000000033',$,$,$,(#3),#23);
#34=IFCRELDEFINESBYPROPERTIES('0000000000000000000034',$,$,$,(#4),#24);
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
    let mut inspection = concept("axioval:test.last-inspection", "LastInspection");
    inspection["valueKind"] = json!("date");
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": { "axioval:test.door": concept("axioval:test.door", "IfcDoor") },
        "propertySets": {
            "axioval:test.inspection": concept("axioval:test.inspection", "Pset_Inspection"),
        },
        "properties": { "axioval:test.last-inspection": inspection },
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

fn ruleset(since: &str) -> Value {
    json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "axioval:test.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["axioval:test.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": [{
            "id": "inspected-this-month",
            "definitionId": "axioval:test.property-predicate",
            "name": text("inspected-this-month"),
            "severity": "error",
            "parameters": {
                "property_set": { "type": "string", "value": "axioval:test.inspection" },
                "property": { "type": "string", "value": "axioval:test.last-inspection" },
                "operator": { "type": "string", "value": "greater_or_equal" },
                "date": { "type": "date", "value": since },
                "precision": { "type": "string", "value": "day" },
            },
            "applicability": {
                "kind": "entityType",
                "objectType": "axioval:test.door",
                "includeSubtypes": false,
            },
        }] },
    })
}

fn report() -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let ruleset: RuleSetPackage = serde_json::from_value(ruleset("2026-09-15")).unwrap();
    let plan = compile(&registry, &[definitions(&registry)], &ruleset).unwrap();
    let session = import_ifc_session("model.ifc", IFC.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn inspection_dates_are_compared_by_the_day_they_state() {
    let report = report();
    let flagged: Vec<_> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.object_id().map_or("", |id| id.local_id.as_str()),
                finding.message.as_str(),
            )
        })
        .collect();
    // #2 states 30 September and passes although it is 1 October in UTC.
    assert_eq!(
        flagged,
        [
            (
                "#1",
                "property axioval:test.inspection.axioval:test.last-inspection \
                 does not satisfy greater_or_equal 2026-09-15; actual value is 2026-09-01"
            ),
            (
                "#4",
                "property axioval:test.inspection.axioval:test.last-inspection \
                 does not satisfy greater_or_equal 2026-09-15; actual value is \
                 2023-11-14T22:13:20Z"
            ),
            (
                "#5",
                "property axioval:test.inspection.axioval:test.last-inspection \
                 does not satisfy greater_or_equal 2026-09-15; actual value is absent"
            ),
        ]
    );
    // A local time without an offset is incomplete, never guessed.
    let open: Vec<_> = report
        .not_evaluated()
        .iter()
        .map(|outcome| {
            (
                outcome.object_id().map_or("", |id| id.local_id.as_str()),
                outcome.reason.clone(),
            )
        })
        .collect();
    assert_eq!(open, [("#3", NotEvaluatedReason::IncompleteEvidence)]);
}

#[test]
fn a_date_literal_that_is_no_calendar_day_is_refused_when_the_package_is_read() {
    let error = serde_json::from_value::<RuleSetPackage>(ruleset("2026-02-30")).unwrap_err();
    assert!(error.to_string().contains("no such day"), "{error}");
}
