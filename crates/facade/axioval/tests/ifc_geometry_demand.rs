//! What a clash rule written for IFC4 asks of an IFC2X3 model's geometry
//! (#449): a class only IFC4 declares (`IfcDuctSegment`) selects no IFC2X3
//! object, so the demand stays bounded by the rule's other classes rather
//! than reaching every object.
//!
//! Lives in the facade because it spans the rules and IFC crates.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use std::collections::BTreeSet;

use axioval::engine::{CapabilityRegistry, GeometryDemand, compile, geometry_demand};
use axioval::ifc::import_ifc_session;
use axioval::ir::{DefinitionPackage, RuleSetPackage};
use axioval::rules::register_builtins;

/// A wall, a slab, a piece of furniture, a space and a storey, in IFC2X3.
const IFC2X3: &[u8] = b"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC2X3'));
ENDSEC;
DATA;
#1=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$);
#2=IFCSLAB('000000000000000000000l',$,$,$,$,$,$,$,$);
#3=IFCFURNISHINGELEMENT('000000000000000000000f',$,$,$,$,$,$,$);
#4=IFCSPACE('000000000000000000000s',$,'R1',$,$,$,$,$,.ELEMENT.,.INTERNAL.,$);
#5=IFCBUILDINGSTOREY('000000000000000000000b',$,'L1',$,$,$,$,$,.ELEMENT.,0.);
ENDSEC;
END-ISO-10303-21;
";

/// The public parity clash vocabulary, which binds `duct` to
/// `IfcDuctSegment` in every release it names, IFC2X3 included, and its
/// `clash-clearance` rule: furniture, ducts, windows and openings against
/// walls, slabs, columns and beams.
fn packages() -> (DefinitionPackage, RuleSetPackage) {
    let definitions = serde_json::from_str(include_str!(
        "../../../../fixtures/parity/cases/clashes/definitions.json"
    ))
    .unwrap();
    let mut rules: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/parity/cases/clashes/ruleset.json"
    ))
    .unwrap();
    rules["root"]["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|rule| rule["id"] == "clash-clearance");
    (definitions, serde_json::from_value(rules).unwrap())
}

#[test]
fn a_class_ifc2x3_lacks_bounds_the_demand_by_the_others() {
    let (definitions, rules) = packages();
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let session = import_ifc_session("model.ifc", IFC2X3).unwrap();
    let GeometryDemand::Objects { objects, .. } = geometry_demand(&plan, &registry, &session)
    else {
        panic!("a clash rule is bounded by its selectors");
    };
    let meshed: BTreeSet<&str> = objects.iter().map(|id| id.local_id.as_str()).collect();
    // The space and the storey are none of the rule's classes; before #449
    // the duct's `IfcDuctSegment`, unknown to IFC2X3, kept them too.
    assert_eq!(meshed, BTreeSet::from(["#1", "#2", "#3"]));
}
