//! Translation rules, and the packages they produce run end to end.
#![allow(missing_docs, clippy::doc_markdown)]

use axioval::default_registry;
use axioval::engine::{Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval::ir::contract::{RuleApplicability, Selector};
use axioval::ir::{NotEvaluatedReason, Report};
use axioval_ids::{
    IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM, Options, OptionsError, Part, Reason, Translation,
    translate,
};
use openbim_ids::IfcVersion;

const HEADER: &str = r#"<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd"><info><title>T</title><author>a@b.org</author></info><specifications>"#;

fn options() -> Options {
    Options {
        package_id: "ids:test".into(),
        version: "1.0.0".into(),
    }
}

/// One specification over `releases`, applying to `applicability` with
/// `occurs` bounds and requiring `requirements`.
fn specification(releases: &str, occurs: &str, applicability: &str, requirements: &str) -> String {
    format!(
        "<specification name=\"S\" ifcVersion=\"{releases}\"><applicability {occurs}>{applicability}</applicability><requirements>{requirements}</requirements></specification>"
    )
}

fn translate_all(specifications: &[String]) -> Translation {
    let text = format!("{HEADER}{}</specifications></ids>", specifications.concat());
    let ids = openbim_ids::from_str(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
    translate(&ids, &options()).unwrap()
}

fn one(releases: &str, occurs: &str, applicability: &str, requirements: &str) -> Translation {
    translate_all(&[specification(releases, occurs, applicability, requirements)])
}

const OPTIONAL: &str = r#"minOccurs="0" maxOccurs="unbounded""#;
const WALL: &str = "<entity><name><simpleValue>IFCWALL</simpleValue></name></entity>";

fn property(set: &str, name: &str, attributes: &str) -> String {
    format!(
        "<property {attributes}><propertySet><simpleValue>{set}</simpleValue></propertySet><baseName><simpleValue>{name}</simpleValue></baseName></property>"
    )
}

fn reasons(translation: &Translation) -> Vec<(Part, Reason)> {
    translation
        .gaps()
        .map(|(_, gap)| (gap.part, gap.reason.clone()))
        .collect()
}

#[test]
fn type_systems_are_the_ifc_adapters() {
    assert_eq!(IFC2X3_TYPE_SYSTEM, axioval::ifc::IFC2X3_TYPE_SYSTEM);
    assert_eq!(IFC4_TYPE_SYSTEM, axioval::ifc::IFC4_TYPE_SYSTEM);
}

#[test]
fn options_are_checked() {
    let ids = openbim_ids::from_str(&format!(
        "{HEADER}{}</specifications></ids>",
        specification("IFC4", OPTIONAL, WALL, "")
    ))
    .unwrap();
    for id in ["test", "Ids:test", "ids:", "ids test:x", ":x"] {
        let options = Options {
            package_id: id.into(),
            ..options()
        };
        assert_eq!(
            translate(&ids, &options).unwrap_err(),
            OptionsError::PackageId(id.into())
        );
    }
    for version in ["1.0", "01.0.0", "1.0.x", ""] {
        let options = Options {
            version: version.into(),
            ..options()
        };
        assert_eq!(
            translate(&ids, &options).unwrap_err(),
            OptionsError::Version(version.into())
        );
    }
}

#[test]
fn a_presence_requirement_becomes_a_property_required_rule() {
    let translation = one(
        "IFC2X3 IFC4",
        OPTIONAL,
        WALL,
        &property("Pset_WallCommon", "FireRating", ""),
    );
    assert!(translation.is_complete(), "{:?}", reasons(&translation));
    let folder = &translation.ruleset.root.folders[0];
    let rule = &folder.rules[0];
    assert_eq!(rule.id, "spec1.facet1");
    assert_eq!(
        translation.definitions.definitions[&rule.definition_id].capability,
        "axioval:capability.property-required"
    );
    // IDS matches the named class only.
    let RuleApplicability::Selector(Selector::EntityType {
        object_type,
        include_subtypes,
    }) = &rule.applicability
    else {
        panic!("{:?}", rule.applicability)
    };
    assert!(!include_subtypes);
    let wall = &translation.definitions.object_types[object_type];
    let systems: Vec<&str> = wall
        .external_names
        .iter()
        .map(|name| name.type_system.as_str())
        .collect();
    assert_eq!(systems, [IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM]);
}

#[test]
fn applicability_bounds_other_than_optional_are_gaps() {
    for (occurs, reason) in [
        (
            "",
            Reason::Count {
                min: 1,
                max: Some(1),
            },
        ),
        (r#"maxOccurs="unbounded""#, Reason::Existence),
        (r#"minOccurs="0" maxOccurs="0""#, Reason::Prohibition),
        (
            r#"minOccurs="2" maxOccurs="5""#,
            Reason::Count {
                min: 2,
                max: Some(5),
            },
        ),
    ] {
        let translation = one("IFC4", occurs, WALL, &property("P", "N", ""));
        assert_eq!(
            reasons(&translation),
            [(Part::Occurrence, reason)],
            "{occurs}"
        );
        // Requirements still hold for every applicable object.
        assert_eq!(translation.specifications[0].rules.len(), 1, "{occurs}");
    }
}

#[test]
fn an_untranslatable_applicability_skips_the_whole_specification() {
    for (applicability, reason) in [
        (
            "<entity><name><simpleValue>IFCWALL</simpleValue></name><predefinedType><simpleValue>SHEAR</simpleValue></predefinedType></entity>".to_owned(),
            Reason::PredefinedType,
        ),
        (
            "<entity><name><simpleValue>IfcWall</simpleValue></name></entity>".to_owned(),
            Reason::EntityCase("IfcWall".into()),
        ),
        (
            "<entity><name><xs:restriction base=\"xs:string\"><xs:pattern value=\"IFC.*\"/></xs:restriction></name></entity>".to_owned(),
            Reason::Restriction,
        ),
        (
            format!("{WALL}<material/>"),
            Reason::FacetKind("material"),
        ),
    ] {
        let translation = one("IFC4", OPTIONAL, &applicability, &property("P", "N", ""));
        let outcome = &translation.specifications[0];
        assert!(outcome.is_skipped(), "{applicability}");
        assert!(outcome.rules.is_empty(), "{applicability}");
        assert!(translation.ruleset.root.folders.is_empty(), "{applicability}");
        assert!(
            outcome.gaps.iter().any(|gap| gap.reason == reason),
            "{applicability}: {:?}",
            outcome.gaps
        );
    }
}

#[test]
fn an_entity_enumeration_selects_any_of_its_classes() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        "<entity><name><xs:restriction base=\"xs:string\"><xs:enumeration value=\"IFCWALL\"/><xs:enumeration value=\"IFCSLAB\"/></xs:restriction></name></entity>",
        &property("P", "N", ""),
    );
    let rule = &translation.ruleset.root.folders[0].rules[0];
    let RuleApplicability::Selector(Selector::AnyOf { operands }) = &rule.applicability else {
        panic!("{:?}", rule.applicability)
    };
    assert_eq!(operands.len(), 2);
}

#[test]
fn requirement_gaps_leave_the_other_requirements_translated() {
    let requirements = [
        property("P", "Typed", "dataType=\"IFCLABEL\""),
        "<property><propertySet><simpleValue>P</simpleValue></propertySet><baseName><simpleValue>Valued</simpleValue></baseName><value><simpleValue>x</simpleValue></value></property>".to_owned(),
        property("P", "Banned", "cardinality=\"prohibited\""),
        property("P", "Maybe", "cardinality=\"optional\""),
        "<attribute><name><simpleValue>Name</simpleValue></name></attribute>".to_owned(),
        "<entity><name><simpleValue>IFCSLAB</simpleValue></name></entity>".to_owned(),
        // Requiring the applicability's own class always holds.
        WALL.to_owned(),
        property("P", "Present", ""),
    ]
    .concat();
    let translation = one("IFC4", OPTIONAL, WALL, &requirements);
    let requirement = |facet| Part::Requirement { facet };
    assert_eq!(
        reasons(&translation),
        [
            (requirement(1), Reason::DataType("IFCLABEL".into())),
            (requirement(2), Reason::PropertyValue),
            (requirement(3), Reason::Prohibited),
            (requirement(5), Reason::FacetKind("attribute")),
            (requirement(6), Reason::EntityRequirement),
        ]
    );
    // The optional value-less property and the redundant entity need no rule.
    assert_eq!(translation.specifications[0].rules, ["spec1.facet8"]);
}

#[test]
fn releases_without_a_type_system_are_gaps() {
    let translation = one("IFC4X3_ADD2", OPTIONAL, WALL, &property("P", "N", ""));
    assert!(translation.specifications[0].is_skipped());
    assert_eq!(
        reasons(&translation),
        [
            (
                Part::Releases,
                Reason::UnsupportedRelease(IfcVersion::Ifc4x3Add2)
            ),
            (Part::Releases, Reason::NoSupportedRelease),
        ]
    );
    let mixed = one("IFC4 IFC4X3_ADD2", OPTIONAL, WALL, &property("P", "N", ""));
    assert_eq!(mixed.specifications[0].rules.len(), 1);
    assert!(!mixed.specifications[0].is_complete());
}

#[test]
fn concepts_are_shared_within_a_release_set_and_split_across_them() {
    let translation = translate_all(&[
        specification("IFC4", OPTIONAL, WALL, &property("P", "N", "")),
        specification("IFC4", OPTIONAL, WALL, &property("P", "N", "")),
        specification("IFC2X3 IFC4", OPTIONAL, WALL, &property("P", "N", "")),
    ]);
    // One wall, property and set per release set.
    assert_eq!(translation.definitions.object_types.len(), 2);
    assert_eq!(translation.definitions.properties.len(), 2);
    assert_eq!(translation.definitions.property_sets.len(), 2);
    assert_eq!(translation.definitions.definitions.len(), 1);
}

#[test]
fn translation_is_deterministic() {
    let make = || {
        translate_all(&[
            specification("IFC4", OPTIONAL, WALL, &property("P", "A", "")),
            specification("IFC2X3", OPTIONAL, WALL, &property("Q", "B", "")),
        ])
    };
    let (first, second) = (make(), make());
    assert_eq!(
        serde_json::to_string(&first.definitions).unwrap(),
        serde_json::to_string(&second.definitions).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&first.ruleset).unwrap(),
        serde_json::to_string(&second.ruleset).unwrap()
    );
}

/// Two walls and a subtype. `#1` carries the property, `#2` has the set
/// without it, `#3` is an `IfcWallStandardCase` without anything.
const IFC4_MODEL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,$,$,$,$,$,$,$);
#3=IFCWALLSTANDARDCASE('0000000000000000000003',$,$,$,$,$,$,$,$);
#4=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 90'),$);
#5=IFCPROPERTYSET('0000000000000000000004',$,'Pset_WallCommon',$,(#4));
#6=IFCRELDEFINESBYPROPERTIES('0000000000000000000005',$,$,$,(#1),#5);
#7=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#8=IFCPROPERTYSET('0000000000000000000006',$,'Pset_WallCommon',$,(#7));
#9=IFCRELDEFINESBYPROPERTIES('0000000000000000000007',$,$,$,(#2),#8);
ENDSEC;
END-ISO-10303-21;
";

fn run(translation: &Translation, model: &str) -> Report {
    let registry = default_registry().unwrap();
    let plan = compile(
        &registry,
        &[translation.definitions.clone()],
        &translation.ruleset,
    )
    .unwrap();
    let session = import_ifc_session("model.ifc", model.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

#[test]
fn translated_rules_check_a_real_model() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        WALL,
        &property("Pset_WallCommon", "FireRating", ""),
    );
    let report = run(&translation, IFC4_MODEL);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    // #2 lacks the property; #3 is a subclass, which IDS does not apply to.
    let flagged: Vec<&str> = report
        .findings()
        .iter()
        .map(|finding| finding.object_id.local_id.as_str())
        .collect();
    assert_eq!(flagged, ["#2"]);
}

#[test]
fn a_rule_for_another_release_is_not_evaluated_never_passed() {
    let translation = one(
        "IFC2X3",
        OPTIONAL,
        WALL,
        &property("Pset_WallCommon", "FireRating", ""),
    );
    let report = run(&translation, IFC4_MODEL);
    assert!(report.findings().is_empty());
    assert!(!report.not_evaluated().is_empty());
    assert!(
        report
            .not_evaluated()
            .iter()
            .all(|outcome| outcome.reason == NotEvaluatedReason::InvalidDeclaration)
    );
}
