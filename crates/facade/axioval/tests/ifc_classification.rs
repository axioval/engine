//! End to end: classification selectors through the real IFC adapter.
//!
//! Before the classification service existed, a `classification` selector
//! read the project's inline classification list, which the IFC adapter
//! never fills. Every classification rule over an IFC model therefore
//! selected nothing and returned an empty report. These tests pin that the
//! selector now reads what the file states, in both supported releases, and
//! refuses what it cannot decide.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use axioval::engine::{CapabilityRegistry, EvidenceSession, Runtime, compile};
use axioval::ifc::{IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval::ir::contract::{ExternalName, RuleApplicability, Selector};
use axioval::ir::{DefinitionPackage, NotEvaluatedReason, Report, RuleSetPackage};
use axioval::rules::register_builtins;

fn step(schema: &str, data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

/// IFC4: wall #1 is `331`, wall #2 is `332`, wall #3 is unclassified. Both
/// codes sit under group `330` in the `DIN 276` system. Only #2 lacks the
/// required reference, so it is the one finding when it is selected.
const IFC4_WALLS: &str = "\
#1=IFCWALL('a',$,$,$,$,$,$,$,$);
#2=IFCWALL('b',$,$,$,$,$,$,$,$);
#3=IFCWALL('c',$,$,$,$,$,$,$,$);
#10=IFCCLASSIFICATION($,'2018',$,'DIN 276',$,$,$);
#11=IFCCLASSIFICATIONREFERENCE($,'330','Walls',#10,$,$);
#12=IFCCLASSIFICATIONREFERENCE($,'331','Load-bearing',#11,$,$);
#13=IFCCLASSIFICATIONREFERENCE($,'332','Non-load-bearing',#11,$,$);
#20=IFCRELASSOCIATESCLASSIFICATION('r1',$,$,$,(#1),#12);
#21=IFCRELASSOCIATESCLASSIFICATION('r2',$,$,$,(#2),#13);
#30=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W'),$);
#31=IFCPROPERTYSET('p1',$,'Pset_WallCommon',$,(#30));
#32=IFCRELDEFINESBYPROPERTIES('d1',$,$,$,(#1,#3),#31);
";

/// IFC2X3: `ItemReference` instead of `Identification`, and a flat hierarchy.
/// IFC2X3 types `ReferencedSource` as `IfcClassification` only, so a
/// reference cannot name a parent reference as IFC4 allows.
const IFC2X3_WALLS: &str = "\
#1=IFCWALL('a',$,$,$,$,$,$,$);
#2=IFCWALL('b',$,$,$,$,$,$,$);
#10=IFCCLASSIFICATION('DIN','2018',$,'DIN 276');
#13=IFCCLASSIFICATIONREFERENCE($,'332','Non-load-bearing',#10);
#21=IFCRELASSOCIATESCLASSIFICATION('r2',$,$,$,(#2),#13);
#30=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W'),$);
#31=IFCPROPERTYSET('p1',$,'Pset_WallCommon',$,(#30));
#32=IFCRELDEFINESBYPROPERTIES('d1',$,$,$,(#1),#31);
";

fn packages(type_system: &str, selector: Selector) -> (DefinitionPackage, RuleSetPackage) {
    let mut definitions: DefinitionPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let mut rules: RuleSetPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    let name = |value: &str| ExternalName {
        type_system: type_system.into(),
        name: value.into(),
    };
    for (id, value) in [
        ("axioval:example.ifc.reference", "Reference"),
        ("axioval:example.ifc.pset-wall-common", "Pset_WallCommon"),
    ] {
        if let Some(property) = definitions.properties.get_mut(id) {
            property.external_names.push(name(value));
        } else {
            definitions
                .property_sets
                .get_mut(id)
                .unwrap()
                .external_names
                .push(name(value));
        }
    }
    let rule = rules.root.rules.first_mut().unwrap();
    let RuleApplicability::Groups(groups) = &mut rule.applicability else {
        panic!("the minimal example uses named groups");
    };
    groups.groups.get_mut("walls").unwrap().selector = selector;
    (definitions, rules)
}

fn classification(code: &str, include_descendants: bool) -> Selector {
    Selector::Classification {
        system: "DIN 276".into(),
        code: code.into(),
        include_descendants,
    }
}

fn run(session: &EvidenceSession, type_system: &str, selector: Selector) -> Report {
    let (definitions, rules) = packages(type_system, selector);
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    Runtime::new(registry).run_session(session, plan).unwrap()
}

fn flagged(report: &Report) -> Vec<&str> {
    report
        .findings()
        .iter()
        .map(|finding| finding.object_id.local_id.as_str())
        .collect()
}

#[test]
fn an_exact_code_selects_only_its_own_objects() {
    let session = import_ifc_session("model.ifc", &step("IFC4", IFC4_WALLS)).unwrap();
    // 331 is #1, which has its reference: selected, no finding.
    let report = run(&session, IFC4_TYPE_SYSTEM, classification("331", false));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    // 332 is #2, which lacks it.
    let report = run(&session, IFC4_TYPE_SYSTEM, classification("332", false));
    assert_eq!(flagged(&report), ["#2"]);
}

#[test]
fn a_group_code_selects_its_descendants_only_when_asked() {
    let session = import_ifc_session("model.ifc", &step("IFC4", IFC4_WALLS)).unwrap();
    let report = run(&session, IFC4_TYPE_SYSTEM, classification("330", false));
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    let report = run(&session, IFC4_TYPE_SYSTEM, classification("330", true));
    // #1 and #2 are both under 330; only #2 lacks the reference. #3, which
    // is unclassified and also lacks nothing, is never selected.
    assert_eq!(flagged(&report), ["#2"]);
}

#[test]
fn ifc2x3_item_references_are_read_through_their_release() {
    let session = import_ifc_session("model.ifc", &step("IFC2X3", IFC2X3_WALLS)).unwrap();
    let report = run(&session, IFC2X3_TYPE_SYSTEM, classification("332", false));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(flagged(&report), ["#2"]);
    let report = run(&session, IFC2X3_TYPE_SYSTEM, classification("331", false));
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert!(report.not_evaluated().is_empty());
}

#[test]
fn an_ifc4_style_chain_in_an_ifc2x3_file_is_refused_not_flattened() {
    // Legal in IFC4, not in IFC2X3: reading it would invent a hierarchy
    // the file's release cannot state.
    let chained = IFC2X3_WALLS.replace(
        "#13=IFCCLASSIFICATIONREFERENCE($,'332','Non-load-bearing',#10);",
        "#11=IFCCLASSIFICATIONREFERENCE($,'330','Walls',#10);\n\
         #13=IFCCLASSIFICATIONREFERENCE($,'332','Non-load-bearing',#11);",
    );
    let session = import_ifc_session("model.ifc", &step("IFC2X3", &chained)).unwrap();
    let report = run(&session, IFC2X3_TYPE_SYSTEM, classification("330", true));
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert_eq!(
        report.not_evaluated().len(),
        1,
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::InvalidEvidence
    );
}

#[test]
fn a_notation_without_a_system_is_not_evaluated_never_unclassified() {
    let notation = "\
#1=IFCWALL('a',$,$,$,$,$,$,$);
#5=IFCCLASSIFICATIONNOTATIONFACET('332');
#6=IFCCLASSIFICATIONNOTATION((#5));
#7=IFCRELASSOCIATESCLASSIFICATION('r',$,$,$,(#1),#6);
";
    let session = import_ifc_session("model.ifc", &step("IFC2X3", notation)).unwrap();
    let report = run(&session, IFC2X3_TYPE_SYSTEM, classification("332", false));
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert_eq!(
        report.not_evaluated().len(),
        1,
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn a_type_classification_is_inherited_by_its_occurrences() {
    // Only the wall type carries 332; the occurrence #2 inherits it through
    // IfcRelDefinesByType, as `ifc-classification` resolves it.
    let typed = "\
#1=IFCWALL('a',$,$,$,$,$,$,$,$);
#2=IFCWALL('b',$,$,$,$,$,$,$,$);
#5=IFCWALLTYPE('t',$,'T',$,$,$,$,$,$,.STANDARD.);
#6=IFCRELDEFINESBYTYPE('dt',$,$,$,(#2),#5);
#10=IFCCLASSIFICATION($,'2018',$,'DIN 276',$,$,$);
#13=IFCCLASSIFICATIONREFERENCE($,'332','Non-load-bearing',#10,$,$);
#21=IFCRELASSOCIATESCLASSIFICATION('r2',$,$,$,(#5),#13);
";
    let session = import_ifc_session("model.ifc", &step("IFC4", typed)).unwrap();
    let report = run(&session, IFC4_TYPE_SYSTEM, classification("332", false));
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(flagged(&report), ["#2"]);
}

#[test]
fn without_a_classification_service_nothing_is_read_as_unclassified() {
    // A plain project run registers no services at all; before the
    // classification seam this selected nothing and reported an empty pass.
    let session = import_ifc_session("model.ifc", &step("IFC4", IFC4_WALLS)).unwrap();
    let (definitions, rules) = packages(IFC4_TYPE_SYSTEM, classification("332", false));
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry).run(session.project(), plan).unwrap();
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert!(!report.not_evaluated().is_empty());
    assert!(
        report
            .not_evaluated()
            .iter()
            .all(|outcome| outcome.reason == NotEvaluatedReason::MissingService)
    );
}
