//! Translation rules, and the packages they produce run end to end.
#![allow(missing_docs, clippy::doc_markdown)]

use axioval::default_registry;
use axioval::engine::{Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval::ir::contract::{ParameterValue, RuleApplicability, Selector};
use axioval::ir::{NotEvaluatedReason, Report};
use axioval_ids::{
    IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM, Options, OptionsError, Part, Reason, Translation,
    translate,
};
use openbim_ids::{IfcVersion, Relation};

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

/// A property facet in set `P` with a `<value>` holding `value` (a
/// `simpleValue` or an `xs:restriction`).
fn valued(name: &str, value: &str) -> String {
    valued_with("P", name, "", value)
}

fn valued_with(set: &str, name: &str, attributes: &str, value: &str) -> String {
    let value = if value.starts_with('<') {
        value.to_owned()
    } else {
        format!("<simpleValue>{value}</simpleValue>")
    };
    format!(
        "<property {attributes}><propertySet><simpleValue>{set}</simpleValue></propertySet><baseName><simpleValue>{name}</simpleValue></baseName><value>{value}</value></property>"
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
fn applicability_bounds_become_an_object_count() {
    let count = |occurs: &str| {
        let translation = one("IFC4", occurs, WALL, &property("P", "N", ""));
        let outcome = &translation.specifications[0];
        let rules = &translation.ruleset.root.folders[0].rules;
        let bounds = rules
            .iter()
            .find(|rule| rule.id == "spec1.occurrence")
            .map(|rule| {
                assert_eq!(
                    translation.definitions.definitions[&rule.definition_id].capability,
                    "axioval:capability.object-count"
                );
                let bound = |name: &str| match rule.parameters.get(name) {
                    Some(ParameterValue::Integer { value }) => Some(*value),
                    None => None,
                    other => panic!("{other:?}"),
                };
                (bound("minimum"), bound("maximum"))
            });
        (bounds, reasons(&translation), outcome.rules.len())
    };
    // The IDS default is exactly one.
    assert_eq!(count(""), (Some((Some(1), Some(1))), vec![], 2));
    assert_eq!(
        count(r#"maxOccurs="unbounded""#),
        (Some((Some(1), None)), vec![], 2)
    );
    assert_eq!(
        count(r#"minOccurs="2" maxOccurs="5""#),
        (Some((Some(2), Some(5))), vec![], 2)
    );
    assert_eq!(count(OPTIONAL), (None, vec![], 1));
    // A prohibited specification takes no requirements.
    assert_eq!(
        count(r#"minOccurs="0" maxOccurs="0""#),
        (
            Some((None, Some(0))),
            vec![(Part::Occurrence, Reason::ProhibitedRequirements)],
            1
        )
    );
}

#[test]
fn an_untranslatable_applicability_skips_the_whole_specification() {
    for (applicability, reason) in [
        (
            "<entity><name><simpleValue>IfcWall</simpleValue></name></entity>".to_owned(),
            Reason::EntityCase("IfcWall".into()),
        ),
        (
            "<entity><name><xs:restriction base=\"xs:string\"><xs:minLength value=\"3\"/></xs:restriction></name></entity>".to_owned(),
            Reason::Restriction,
        ),
        (
            format!("{WALL}{}", property("P", "N", "")),
            Reason::PropertyApplicability,
        ),
        (
            format!("{WALL}<material><value><xs:restriction base=\"xs:string\"><xs:enumeration value=\"Steel\"/><xs:pattern value=\"S.*\"/></xs:restriction></value></material>"),
            Reason::MaterialValue,
        ),
        (
            format!("{WALL}<partOf><entity><name><simpleValue>IFCBUILDING</simpleValue></name></entity></partOf>"),
            Reason::PartOfRelation(None),
        ),
        (
            format!("{WALL}<classification><system><simpleValue>Uniclass</simpleValue></system></classification>"),
            Reason::ClassificationSystem,
        ),
        (
            "<material/>".to_owned(),
            Reason::WithoutEntity,
        ),
    ] {
        let translation = one("IFC4", OPTIONAL, &applicability, &property("P", "N", ""));
        let outcome = &translation.specifications[0];
        assert!(outcome.is_skipped(), "{applicability}");
        assert!(outcome.rules.is_empty(), "{applicability}");
        assert!(translation.ruleset.root.folders.is_empty(), "{applicability}");
        // Nothing is written, so no concept is either.
        assert!(translation.definitions.object_types.is_empty(), "{applicability}");
        assert!(translation.definitions.properties.is_empty(), "{applicability}");
        assert!(
            outcome.gaps.iter().any(|gap| gap.reason == reason),
            "{applicability}: {:?}",
            outcome.gaps
        );
    }
}

#[test]
fn applicability_facets_beyond_the_entity_become_one_selector() {
    let applicability = [
        "<entity><name><simpleValue>IFCWALL</simpleValue></name><predefinedType><simpleValue>SHEAR</simpleValue></predefinedType></entity>",
        "<partOf relation=\"IFCRELCONTAINEDINSPATIALSTRUCTURE\"><entity><name><simpleValue>IFCBUILDINGSTOREY</simpleValue></name></entity></partOf>",
        "<classification><value><simpleValue>EF_25</simpleValue></value><system><simpleValue>Uniclass</simpleValue></system></classification>",
        "<attribute><name><simpleValue>Name</simpleValue></name><value><simpleValue>W1</simpleValue></value></attribute>",
        "<material/>",
    ]
    .concat();
    let translation = one("IFC4", OPTIONAL, &applicability, &property("P", "N", ""));
    assert!(translation.is_complete(), "{:?}", reasons(&translation));
    let rule = &translation.ruleset.root.folders[0].rules[0];
    let RuleApplicability::Selector(Selector::AllOf { operands }) = &rule.applicability else {
        panic!("{:?}", rule.applicability)
    };
    // The entity with its predefined type, then one operand per facet.
    assert_eq!(operands.len(), 5);
    assert!(matches!(
        &operands[1],
        Selector::Related { path, .. } if path == &["IfcRelContainedInSpatialStructure:backward+"]
    ));
}

#[test]
fn only_classes_a_model_session_checks_are_applicable() {
    let entity =
        |name: &str| format!("<entity><name><simpleValue>{name}</simpleValue></name></entity>");
    let gap = |releases: &str, name: &str| {
        let translation = one(releases, OPTIONAL, &entity(name), &property("P", "N", ""));
        let outcome = &translation.specifications[0];
        outcome
            .gaps
            .iter()
            .find(|gap| matches!(gap.part, Part::Applicability { .. }))
            .map(|gap| (outcome.is_skipped(), gap.reason.clone()))
    };
    // A type object is not an occurrence, so rules over it would select nothing.
    assert_eq!(
        gap("IFC4", "IFCWALLTYPE"),
        Some((true, Reason::NotAnObject("IFCWALLTYPE".into())))
    );
    // IfcProject is an IfcObject in IFC2X3 and an IfcContext in IFC4.
    assert_eq!(gap("IFC2X3", "IFCPROJECT"), None);
    assert_eq!(
        gap("IFC2X3 IFC4", "IFCPROJECT"),
        Some((true, Reason::NotAnObject("IFCPROJECT".into())))
    );
    assert_eq!(
        gap("IFC4", "IFCNOSUCHTHING"),
        Some((
            true,
            Reason::UnknownEntity {
                entity: "IFCNOSUCHTHING".into(),
                release: IfcVersion::Ifc4
            }
        ))
    );
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
        valued_with("P", "Banned", "cardinality=\"prohibited\"", "X"),
        valued("Open", "<xs:restriction base=\"xs:string\"/>"),
        property("P", "Gone", "cardinality=\"prohibited\""),
        property("P", "Maybe", "cardinality=\"optional\""),
        "<attribute><name><simpleValue>Name</simpleValue></name></attribute>".to_owned(),
        "<entity><name><simpleValue>IFCSLAB</simpleValue></name></entity>".to_owned(),
        // Requiring the applicability's own class always holds.
        WALL.to_owned(),
        property("P", "Present", ""),
        "<classification><system><simpleValue>Uniclass</simpleValue></system></classification>".to_owned(),
        "<property><propertySet><simpleValue>P</simpleValue></propertySet><baseName><xs:restriction base=\"xs:string\"><xs:pattern value=\"A.*\"/></xs:restriction></baseName></property>".to_owned(),
        "<material><value><simpleValue>Steel</simpleValue></value></material>".to_owned(),
        "<partOf relation=\"IFCRELVOIDSELEMENT IFCRELFILLSELEMENT\"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></partOf>".to_owned(),
        valued(
            "Digits",
            "<xs:restriction base=\"xs:decimal\"><xs:totalDigits value=\"3\"/><xs:fractionDigits value=\"1\"/></xs:restriction>",
        ),
    ]
    .concat();
    let translation = one("IFC4", OPTIONAL, WALL, &requirements);
    let requirement = |facet| Part::Requirement { facet };
    assert_eq!(
        reasons(&translation),
        [
            (requirement(1), Reason::ProhibitedValue),
            (requirement(2), Reason::EmptyRestriction),
            (requirement(10), Reason::NamePattern),
            (
                requirement(12),
                Reason::PartOfRelation(Some(Relation::VoidsElementFillsElement))
            ),
        ]
    );
    // The optional value-less property and the redundant entity need no rule.
    assert_eq!(
        translation.specifications[0].rules,
        [
            "spec1.facet3",
            "spec1.facet5",
            "spec1.facet6",
            "spec1.facet8",
            "spec1.facet9",
            "spec1.facet11",
            "spec1.facet13"
        ]
    );
    let capability = |id: &str| {
        let rule = translation.ruleset.root.folders[0]
            .rules
            .iter()
            .find(|rule| rule.id == id)
            .unwrap();
        translation.definitions.definitions[&rule.definition_id]
            .capability
            .clone()
    };
    assert_eq!(
        capability("spec1.facet3"),
        "axioval:capability.property-requirements"
    );
    assert_eq!(
        capability("spec1.facet5"),
        "axioval:capability.property-required"
    );
    assert_eq!(
        capability("spec1.facet6"),
        "axioval:capability.selector-conformance"
    );
    assert_eq!(
        capability("spec1.facet9"),
        "axioval:capability.classification"
    );
    assert_eq!(
        capability("spec1.facet11"),
        "axioval:capability.selector-conformance"
    );
    assert_eq!(
        capability("spec1.facet13"),
        "axioval:capability.property-value"
    );
    // Every definition compiles against the capability it names.
    compile(
        &default_registry().unwrap(),
        &[translation.definitions.clone()],
        &translation.ruleset,
    )
    .unwrap();
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
        .map(|finding| {
            finding
                .object_id()
                .expect("an object finding")
                .local_id
                .as_str()
        })
        .collect();
    assert_eq!(flagged, ["#2"]);
}

#[test]
fn a_typed_presence_requirement_becomes_a_property_data_type_rule() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        WALL,
        &property("Pset_WallCommon", "FireRating", "dataType=\"IFCLABEL\""),
    );
    assert!(translation.is_complete(), "{:?}", reasons(&translation));
    let rule = &translation.ruleset.root.folders[0].rules[0];
    assert_eq!(
        translation.definitions.definitions[&rule.definition_id].capability,
        "axioval:capability.property-data-type"
    );
    assert_eq!(
        rule.parameters["data_type"],
        ParameterValue::String {
            value: "IFCLABEL".into()
        }
    );
}

#[test]
fn typed_rules_check_the_declared_type_of_a_real_model() {
    let flagged = |data_type: &str| {
        let attributes = format!("dataType=\"{data_type}\"");
        let translation = one(
            "IFC4",
            OPTIONAL,
            WALL,
            &property("Pset_WallCommon", "FireRating", &attributes),
        );
        let report = run(&translation, IFC4_MODEL);
        assert!(
            report.not_evaluated().is_empty(),
            "{:?}",
            report.not_evaluated()
        );
        report
            .findings()
            .iter()
            .map(|finding| {
                (
                    finding
                        .object_id()
                        .expect("an object finding")
                        .local_id
                        .clone(),
                    finding.message.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    // #1 states an IFCLABEL; #2 has no FireRating at all.
    assert_eq!(
        flagged("IFCLABEL"),
        [(
            "#2".into(),
            "missing required property ids:test.property-1".into()
        )]
    );
    assert_eq!(
        flagged("IFCTEXT"),
        [
            (
                "#1".into(),
                "property ids:test.property-1 is IFCLABEL, not IFCTEXT".into()
            ),
            (
                "#2".into(),
                "missing required property ids:test.property-1".into()
            ),
        ]
    );
}

fn only_rule(translation: &Translation) -> &axioval::ir::contract::RuleInstance {
    assert!(translation.is_complete(), "{:?}", reasons(translation));
    let rule = &translation.ruleset.root.folders[0].rules[0];
    assert_eq!(
        translation.definitions.definitions[&rule.definition_id].capability,
        "axioval:capability.property-value"
    );
    rule
}

fn strings(values: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

#[test]
fn a_simple_value_becomes_a_one_element_value_list() {
    let translation = one("IFC4", OPTIONAL, WALL, &valued("Code", "EI 90"));
    let rule = only_rule(&translation);
    assert_eq!(rule.parameters["values"], strings(&["EI 90"]));
    assert!(!rule.parameters.contains_key("optional"));
}

#[test]
fn restriction_facets_become_their_parameters() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        WALL,
        &valued_with(
            "P",
            "Code",
            "dataType=\"IFCLABEL\" cardinality=\"optional\"",
            "<xs:restriction base=\"xs:string\"><xs:enumeration value=\"A\"/><xs:enumeration value=\"B\"/><xs:pattern value=\"[AB]\"/><xs:minLength value=\"1\"/><xs:maxLength value=\"2\"/></xs:restriction>",
        ),
    );
    let rule = only_rule(&translation);
    let text = |value: &str| ParameterValue::String {
        value: value.into(),
    };
    assert_eq!(rule.parameters["values"], strings(&["A", "B"]));
    assert_eq!(rule.parameters["patterns"], strings(&["[AB]"]));
    assert_eq!(
        rule.parameters["min_length"],
        ParameterValue::Integer { value: 1 }
    );
    assert_eq!(
        rule.parameters["max_length"],
        ParameterValue::Integer { value: 2 }
    );
    assert_eq!(rule.parameters["data_type"], text("IFCLABEL"));
    assert_eq!(
        rule.parameters["optional"],
        ParameterValue::Boolean { value: true }
    );

    let bounded = one(
        "IFC4",
        OPTIONAL,
        WALL,
        &valued(
            "Width",
            "<xs:restriction base=\"xs:double\"><xs:minInclusive value=\"0.2\"/><xs:maxExclusive value=\"1e1\"/></xs:restriction>",
        ),
    );
    let rule = only_rule(&bounded);
    assert_eq!(rule.parameters["min_inclusive"], text("0.2"));
    assert_eq!(rule.parameters["max_exclusive"], text("1e1"));
}

#[test]
fn an_optional_typed_property_checks_its_type_only_when_present() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        WALL,
        &property(
            "P",
            "Code",
            "dataType=\"IFCLABEL\" cardinality=\"optional\"",
        ),
    );
    let rule = only_rule(&translation);
    assert!(!rule.parameters.contains_key("values"));
    assert_eq!(
        rule.parameters["optional"],
        ParameterValue::Boolean { value: true }
    );
}

#[test]
fn value_rules_check_a_real_model() {
    let flagged = |attributes: &str, value: &str| {
        let translation = one(
            "IFC4",
            OPTIONAL,
            WALL,
            &valued_with("Pset_WallCommon", "FireRating", attributes, value),
        );
        let report = run(&translation, IFC4_MODEL);
        assert!(
            report.not_evaluated().is_empty(),
            "{:?}",
            report.not_evaluated()
        );
        report
            .findings()
            .iter()
            .map(|finding| {
                finding
                    .object_id()
                    .expect("an object finding")
                    .local_id
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    // #1 states 'EI 90'; #2 has no FireRating.
    assert_eq!(flagged("dataType=\"IFCLABEL\"", "EI 90"), ["#2"]);
    assert_eq!(flagged("", "EI 60"), ["#1", "#2"]);
    assert_eq!(flagged("", "ei 90"), ["#1", "#2"]);
    assert_eq!(
        flagged(
            "",
            "<xs:restriction base=\"xs:string\"><xs:pattern value=\"EI [0-9]+\"/></xs:restriction>"
        ),
        ["#2"]
    );
    // Optional: the absent #2 passes, the present #1 must still match.
    assert_eq!(flagged("cardinality=\"optional\"", "EI 60"), ["#1"]);
    assert!(flagged("cardinality=\"optional\"", "EI 90").is_empty());
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
            .all(|outcome| outcome.reason == NotEvaluatedReason::UnboundConcept)
    );
}

/// A project, site, building and storey aggregated in a chain, and three
/// walls. `#10` is a shear wall named `W1`, contained in the storey,
/// classified `EF_25_10` (under `EF_25`) in `Uniclass`, made of concrete and
/// counting `1234`. `#11` is a user-defined `CUSTOM` wall named `W2`, also
/// contained. `#12` is unnamed, its own type `NOTDEFINED` but typed by a
/// partitioning wall type, and contained nowhere.
const PROJECT_MODEL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('0000000000000000000010',$,'P',$,$,$,$,$,$);
#2=IFCSITE('0000000000000000000011',$,'S',$,$,$,$,$,$,$,$,$,$,$);
#3=IFCBUILDING('0000000000000000000012',$,'B',$,$,$,$,$,$,$,$,$);
#4=IFCBUILDINGSTOREY('0000000000000000000013',$,'L1',$,$,$,$,$,$,$);
#5=IFCRELAGGREGATES('0000000000000000000014',$,$,$,#1,(#2));
#6=IFCRELAGGREGATES('0000000000000000000015',$,$,$,#2,(#3));
#7=IFCRELAGGREGATES('0000000000000000000016',$,$,$,#3,(#4));
#10=IFCWALL('0000000000000000000020',$,'W1',$,$,$,$,$,.SHEAR.);
#11=IFCWALL('0000000000000000000021',$,'W2',$,'CUSTOM',$,$,$,.USERDEFINED.);
#12=IFCWALL('0000000000000000000022',$,$,$,$,$,$,$,.NOTDEFINED.);
#13=IFCWALLTYPE('0000000000000000000023',$,'T',$,$,$,$,$,$,.PARTITIONING.);
#14=IFCRELDEFINESBYTYPE('0000000000000000000024',$,$,$,(#12),#13);
#15=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000025',$,$,$,(#10,#11),#4);
#20=IFCCLASSIFICATION($,$,$,'Uniclass',$,$,$);
#21=IFCCLASSIFICATIONREFERENCE($,'EF_25',$,#20,$,$);
#22=IFCCLASSIFICATIONREFERENCE($,'EF_25_10',$,#21,$,$);
#23=IFCRELASSOCIATESCLASSIFICATION('0000000000000000000026',$,$,$,(#10),#22);
#30=IFCMATERIAL('Concrete',$,$);
#31=IFCRELASSOCIATESMATERIAL('0000000000000000000027',$,$,$,(#10),#30);
#40=IFCPROPERTYSINGLEVALUE('Count',$,IFCINTEGER(1234),$);
#41=IFCPROPERTYSET('0000000000000000000028',$,'P',$,(#40));
#42=IFCRELDEFINESBYPROPERTIES('0000000000000000000029',$,$,$,(#10),#41);
ENDSEC;
END-ISO-10303-21;
";

/// The objects `requirements` flags among those `applicability` selects,
/// with nothing left not evaluated.
fn flagged_in_project(occurs: &str, applicability: &str, requirements: &str) -> Vec<String> {
    let translation = one("IFC4", occurs, applicability, requirements);
    assert!(translation.is_complete(), "{:?}", reasons(&translation));
    let report = run(&translation, PROJECT_MODEL);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let mut flagged: Vec<String> = report
        .findings()
        .iter()
        .flat_map(|finding| {
            let own = finding
                .object_id()
                .map_or_else(|| finding.scope.to_string(), |id| id.local_id.clone());
            std::iter::once(own).chain(finding.related.iter().map(|id| id.local_id.clone()))
        })
        .collect();
    flagged.sort();
    flagged
}

fn attribute(name: &str, attributes: &str, value: Option<&str>) -> String {
    let value = value.map_or_else(String::new, |value| {
        format!("<value><simpleValue>{value}</simpleValue></value>")
    });
    format!(
        "<attribute {attributes}><name><simpleValue>{name}</simpleValue></name>{value}</attribute>"
    )
}

/// Every wall has no `Description`, so this flags each applicable one.
fn every_applicable() -> String {
    attribute("Description", "", None)
}

fn wall_of_type(predefined: &str) -> String {
    format!(
        "<entity><name><simpleValue>IFCWALL</simpleValue></name><predefinedType>{predefined}</predefinedType></entity>"
    )
}

#[test]
fn predefined_types_resolve_as_ids_resolves_them() {
    let applicable = |predefined: &str| {
        flagged_in_project(OPTIONAL, &wall_of_type(predefined), &every_applicable())
    };
    let simple = |value: &str| format!("<simpleValue>{value}</simpleValue>");
    assert_eq!(applicable(&simple("SHEAR")), ["#10"]);
    // A user-defined type is its object type.
    assert_eq!(applicable(&simple("CUSTOM")), ["#11"]);
    assert_eq!(applicable(&simple("USERDEFINED")), ["#11"]);
    // The type object decides over the occurrence's NOTDEFINED.
    assert_eq!(applicable(&simple("PARTITIONING")), ["#12"]);
    assert!(applicable(&simple("NOTDEFINED")).is_empty());
    assert_eq!(
        applicable(
            "<xs:restriction base=\"xs:string\"><xs:pattern value=\"SH.*|PART.*\"/></xs:restriction>"
        ),
        ["#10", "#12"]
    );
    // As a requirement on the applicability's own class.
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &wall_of_type("<simpleValue>SHEAR</simpleValue>")
        ),
        ["#11", "#12"]
    );
}

#[test]
fn attribute_requirements_check_a_real_model() {
    let name = |attributes: &str, value: Option<&str>| {
        flagged_in_project(OPTIONAL, WALL, &attribute("Name", attributes, value))
    };
    assert_eq!(name("", None), ["#12"]);
    assert_eq!(name("", Some("W1")), ["#11", "#12"]);
    assert_eq!(name("cardinality=\"optional\"", Some("W1")), ["#11"]);
    assert_eq!(name("cardinality=\"prohibited\"", None), ["#10", "#11"]);
    assert_eq!(name("cardinality=\"prohibited\"", Some("W2")), ["#11"]);
    // Any of the named attributes holding a value will do.
    let either = "<attribute><name><xs:restriction base=\"xs:string\"><xs:enumeration value=\"Name\"/><xs:enumeration value=\"Description\"/></xs:restriction></name></attribute>";
    assert_eq!(flagged_in_project(OPTIONAL, WALL, either), ["#12"]);
    // As an applicability.
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            &format!("{WALL}{}", attribute("Name", "", Some("W2"))),
            &every_applicable()
        ),
        ["#11"]
    );
}

#[test]
fn classification_material_and_part_of_requirements_check_a_real_model() {
    let classified = |attributes: &str, code: &str| {
        format!(
            "<classification {attributes}><value><simpleValue>{code}</simpleValue></value><system><simpleValue>Uniclass</simpleValue></system></classification>"
        )
    };
    // EF_25_10 is a subclass of EF_25.
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, &classified("", "EF_25")),
        ["#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, &classified("", "EF_25_10_30")),
        ["#10", "#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &classified("cardinality=\"prohibited\"", "EF_25")
        ),
        ["#10"]
    );
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, "<material/>"),
        ["#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, "<material cardinality=\"prohibited\"/>"),
        ["#10"]
    );
    let part_of = |relation: &str, whole: &str| {
        format!(
            "<partOf relation=\"{relation}\"><entity><name><simpleValue>{whole}</simpleValue></name></entity></partOf>"
        )
    };
    let containment = "IFCRELCONTAINEDINSPATIALSTRUCTURE";
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, &part_of(containment, "IFCBUILDINGSTOREY")),
        ["#12"]
    );
    // Containment is followed on its own: the storey is aggregated, not
    // contained, in the building.
    assert_eq!(
        flagged_in_project(OPTIONAL, WALL, &part_of(containment, "IFCBUILDING")),
        ["#10", "#11", "#12"]
    );
    // Aggregation is followed up the whole chain.
    let storey = "<entity><name><simpleValue>IFCBUILDINGSTOREY</simpleValue></name></entity>";
    assert!(
        flagged_in_project(OPTIONAL, storey, &part_of("IFCRELAGGREGATES", "IFCSITE")).is_empty()
    );
    // As an applicability: the walls in the storey.
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            &format!("{WALL}{}", part_of(containment, "IFCBUILDINGSTOREY")),
            &every_applicable()
        ),
        ["#10", "#11"]
    );
}

#[test]
fn classification_systems_patterns_and_optional_requirements_check_a_real_model() {
    let classified = |attributes: &str, code: &str| {
        format!(
            "<classification {attributes}><value><simpleValue>{code}</simpleValue></value><system><simpleValue>Uniclass</simpleValue></system></classification>"
        )
    };
    // A system alone, patterns, and an optional classification go through
    // the classification capability.
    let system = |attributes: &str, system: &str| {
        format!("<classification {attributes}><system>{system}</system></classification>")
    };
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &system("", "<simpleValue>Uniclass</simpleValue>")
        ),
        ["#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &system(
                "",
                "<xs:restriction base=\"xs:string\"><xs:pattern value=\"Uni.*\"/></xs:restriction>"
            )
        ),
        ["#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &system(
                "cardinality=\"prohibited\"",
                "<simpleValue>Uniclass</simpleValue>"
            )
        ),
        ["#10"]
    );
    // Optional: unclassified walls pass, the classified one must match.
    assert!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &classified("cardinality=\"optional\"", "EF_25")
        )
        .is_empty()
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &classified("cardinality=\"optional\"", "EF_30")
        ),
        ["#10"]
    );
}

#[test]
fn material_values_check_a_real_model() {
    // A material value is any name the material goes by.
    let named = |attributes: &str, value: &str| {
        format!("<material {attributes}><value>{value}</value></material>")
    };
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &named("", "<simpleValue>Concrete</simpleValue>")
        ),
        ["#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &named("", "<simpleValue>Steel</simpleValue>")
        ),
        ["#10", "#11", "#12"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &named(
                "cardinality=\"optional\"",
                "<xs:restriction base=\"xs:string\"><xs:pattern value=\"St.*\"/></xs:restriction>"
            )
        ),
        ["#10"]
    );
    assert_eq!(
        flagged_in_project(
            OPTIONAL,
            WALL,
            &named(
                "cardinality=\"prohibited\"",
                "<simpleValue>Concrete</simpleValue>"
            )
        ),
        ["#10"]
    );
}

#[test]
fn specification_cardinality_counts_applicable_objects_per_model() {
    let slab = "<entity><name><simpleValue>IFCSLAB</simpleValue></name></entity>";
    // Required: the model holds no slab, which is a finding about the model.
    let required = flagged_in_project(r#"maxOccurs="unbounded""#, slab, "");
    assert_eq!(required.len(), 1);
    assert!(required[0].contains("model.ifc"), "{required:?}");
    assert!(flagged_in_project(OPTIONAL, slab, "").is_empty());
    // Prohibited: the model holds walls, reported once for the model.
    let prohibited = flagged_in_project(r#"minOccurs="0" maxOccurs="0""#, WALL, "");
    assert!(prohibited.iter().any(|flag| flag.contains("model.ifc")));
    assert!(flagged_in_project(r#"minOccurs="0" maxOccurs="0""#, slab, "").is_empty());
}

#[test]
fn digit_restrictions_check_numbers() {
    let digits = |facets: &str| {
        flagged_in_project(
            OPTIONAL,
            WALL,
            &valued_with(
                "P",
                "Count",
                "cardinality=\"optional\"",
                &format!("<xs:restriction base=\"xs:integer\">{facets}</xs:restriction>"),
            ),
        )
    };
    assert_eq!(digits("<xs:totalDigits value=\"3\"/>"), ["#10"]);
    assert!(digits("<xs:totalDigits value=\"4\"/>").is_empty());
    assert!(digits("<xs:fractionDigits value=\"0\"/>").is_empty());
}
