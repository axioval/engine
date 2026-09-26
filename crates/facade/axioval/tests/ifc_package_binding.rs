//! End to end: a real MCS package through the real IFC4 adapter.
//!
//! Before concept binding, this exact pairing -- the MCS minimal example
//! over an IFC4 model with a wall missing its reference -- compiled, ran,
//! and returned an empty report: zero findings, zero not-evaluated. A
//! caller reads that as a pass. These tests pin that it can no longer
//! happen, and that a package bound to the adapter's type system does run.
//!
//! Lives in the facade because it spans the rules and IFC crates: a
//! dev-dependency from `axioval-ifc` on the unpublished sibling
//! `axioval-rules` breaks workspace package verification.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::{IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::contract::ExternalName;
use axioval::ir::{DefinitionPackage, NotEvaluatedReason, RuleSetPackage};
use axioval::rules::register_builtins;

/// A wall, a wall subtype, and a slab. Only the plain wall carries the
/// reference, through `Pset_WallCommon`.
const IFC: &[u8] = b"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCWALL('a',$,$,$,$,$,$,$,$);
#2=IFCWALLSTANDARDCASE('b',$,$,$,$,$,$,$,$);
#3=IFCSLAB('c',$,$,$,$,$,$,$,$);
#4=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);
#5=IFCPROPERTYSET('p',$,'Pset_WallCommon',$,(#4));
#6=IFCRELDEFINESBYPROPERTIES('r',$,$,$,(#1),#5);
ENDSEC;
END-ISO-10303-21;
";

fn packages() -> (DefinitionPackage, RuleSetPackage) {
    (
        serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/definitions.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap(),
    )
}

/// Adds an IFC4 name to every concept the minimal example uses, as a
/// package author targeting IFC4 would.
fn bind_to_ifc4(mut definitions: DefinitionPackage) -> DefinitionPackage {
    let name = |value: &str| ExternalName {
        type_system: IFC4_TYPE_SYSTEM.into(),
        name: value.into(),
    };
    definitions
        .object_types
        .get_mut("axioval:example.ifc.wall")
        .unwrap()
        .external_names
        .push(name("IfcWall"));
    definitions
        .properties
        .get_mut("axioval:example.ifc.reference")
        .unwrap()
        .external_names
        .push(name("Reference"));
    definitions
        .property_sets
        .get_mut("axioval:example.ifc.pset-wall-common")
        .unwrap()
        .external_names
        .push(name("Pset_WallCommon"));
    definitions
}

fn run(definitions: DefinitionPackage) -> axioval::ir::Report {
    let (_, rules) = packages();
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let session = import_ifc_session("model.ifc", IFC).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn unbound_package_over_ifc4_is_not_evaluated_never_an_empty_pass() {
    // The published minimal example names its wall in IFC4.3 only. The
    // adapter declares IFC4, so the concept cannot bind to this model.
    let (definitions, _) = packages();
    let report = run(definitions);
    assert!(report.findings().is_empty());
    assert!(
        !report.not_evaluated().is_empty(),
        "an unbindable package must never produce an empty report"
    );
    // One cause, reported once: the concept fails to bind for the source,
    // not separately for each of the model's three objects.
    assert_eq!(
        report.not_evaluated().len(),
        1,
        "{:?}",
        report.not_evaluated()
    );
    let outcome = &report.not_evaluated()[0];
    assert_eq!(outcome.reason, NotEvaluatedReason::UnboundConcept);
    assert_eq!(outcome.object_id, None);
    assert!(
        outcome
            .message
            .contains("3 object(s) of source `ifc-step:model.ifc` not evaluated (e.g. #1, #2, #3)"),
        "{}",
        outcome.message
    );
}

#[test]
fn package_bound_to_ifc4_selects_subtypes_and_checks_real_properties() {
    let (definitions, _) = packages();
    let report = run(bind_to_ifc4(definitions));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // #1 has the reference. #2 is an IfcWallStandardCase, selected through
    // the IFC4 schema's inheritance, and lacks it. #3 is a slab, not a wall.
    assert_eq!(report.findings().len(), 1, "{:?}", report.findings());
    let finding = &report.findings()[0];
    assert_eq!(finding.object_id.local_id, "#2");
    assert!(
        finding
            .evidence
            .iter()
            .all(|evidence| evidence.exact && evidence.locator.contains("sha256:"))
    );
}
