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
fn applicability_bounds_become_a_population_rule() {
    let integer = |value: i64| ParameterValue::Integer { value };
    for (occurs, min, max, rules) in [
        ("", Some(1), Some(1), 2),
        (r#"maxOccurs="unbounded""#, Some(1), None, 2),
        (r#"minOccurs="2" maxOccurs="5""#, Some(2), Some(5), 2),
        // A prohibited specification's requirements are not checked.
        (r#"minOccurs="0" maxOccurs="0""#, None, Some(0), 1),
    ] {
        let translation = one("IFC4", occurs, WALL, &property("P", "N", ""));
        assert!(
            translation.is_complete(),
            "{occurs}: {:?}",
            reasons(&translation)
        );
        let specification = &translation.specifications[0];
        assert_eq!(specification.rules.len(), rules, "{occurs}");
        assert_eq!(specification.rules[0], "spec1.occurrence", "{occurs}");
        let rule = &translation.ruleset.root.folders[0].rules[0];
        assert_eq!(
            translation.definitions.definitions[&rule.definition_id].capability,
            "axioval:capability.population"
        );
        assert_eq!(
            rule.parameters.get("min"),
            min.map(integer).as_ref(),
            "{occurs}"
        );
        assert_eq!(
            rule.parameters.get("max"),
            max.map(integer).as_ref(),
            "{occurs}"
        );
    }
    // Optional bounds ask for nothing about the population.
    let optional = one("IFC4", OPTIONAL, WALL, &property("P", "N", ""));
    assert_eq!(optional.specifications[0].rules, ["spec1.facet1"]);
}

#[test]
fn a_required_specification_fails_a_model_without_applicable_objects() {
    let translation = one(
        "IFC4",
        r#"maxOccurs="unbounded""#,
        "<entity><name><simpleValue>IFCBEAM</simpleValue></name></entity>",
        "",
    );
    let report = run(&translation, IFC4_MODEL);
    assert!(report.findings().is_empty());
    assert_eq!(report.rule_findings().len(), 1);
    assert_eq!(
        report.rule_findings()[0].rule_id.to_string(),
        "spec1.occurrence"
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
            "<entity><name><xs:restriction base=\"xs:string\"><xs:pattern value=\"IFC.*\"/></xs:restriction></name></entity>".to_owned(),
            Reason::Restriction,
        ),
        // Without an entity, IDS also applies to type objects.
        ("<material/>".to_owned(), Reason::WithoutEntity),
        (
            format!("{WALL}<property><propertySet><xs:restriction base=\"xs:string\"><xs:pattern value=\"Pset_.*\"/></xs:restriction></propertySet><baseName><simpleValue>N</simpleValue></baseName></property>"),
            Reason::Restriction,
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
fn applicability_facets_become_meets_conditions() {
    let translation = one(
        "IFC4",
        OPTIONAL,
        &format!(
            "<entity><name><simpleValue>IFCWALL</simpleValue></name><predefinedType><simpleValue>SHEAR</simpleValue></predefinedType></entity>{}<material/>",
            property("Pset_WallCommon", "FireRating", "")
        ),
        &property("P", "N", ""),
    );
    assert!(translation.is_complete(), "{:?}", reasons(&translation));
    let rule = &translation.ruleset.root.folders[0].rules[0];
    let RuleApplicability::Selector(Selector::AllOf { operands }) = &rule.applicability else {
        panic!("{:?}", rule.applicability)
    };
    let capabilities: Vec<&str> = operands
        .iter()
        .filter_map(|operand| match operand {
            Selector::Meets { capability, .. } => Some(capability.as_str()),
            _ => None,
        })
        .collect();
    assert!(matches!(operands[0], Selector::EntityType { .. }));
    assert_eq!(
        capabilities,
        [
            "axioval:capability.predefined-type",
            "axioval:capability.property-required",
            "axioval:capability.material"
        ]
    );
}

#[test]
fn applicability_conditions_select_on_a_real_model() {
    // Only walls stating FireRating are applicable: #1. It has no IsExternal.
    let translation = one(
        "IFC4",
        OPTIONAL,
        &format!("{WALL}{}", property("Pset_WallCommon", "FireRating", "")),
        &property("Pset_WallCommon", "IsExternal", ""),
    );
    let report = run(&translation, IFC4_MODEL);
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let flagged: Vec<&str> = report
        .findings()
        .iter()
        .map(|finding| finding.object_id.local_id.as_str())
        .collect();
    assert_eq!(flagged, ["#1"]);
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
        valued(
            "Digits",
            "<xs:restriction base=\"xs:decimal\"><xs:totalDigits value=\"3\"/></xs:restriction>",
        ),
        valued("Open", "<xs:restriction base=\"xs:string\"/>"),
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
            (requirement(1), Reason::RestrictionFacet("totalDigits")),
            (requirement(2), Reason::EmptyRestriction),
        ]
    );
    // The optional value-less property and the redundant entity need no rule;
    // the prohibited property and the other class translate.
    assert_eq!(
        translation.specifications[0].rules,
        [
            "spec1.facet3",
            "spec1.facet5",
            "spec1.facet6",
            "spec1.facet8"
        ]
    );
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
            .map(|finding| (finding.object_id.local_id.clone(), finding.message.clone()))
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
            .map(|finding| finding.object_id.local_id.clone())
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
fn attribute_rules_check_a_real_model() {
    let flagged = |requirement: &str| {
        let translation = one("IFC4", OPTIONAL, WALL, requirement);
        let rule = &translation.ruleset.root.folders[0].rules[0];
        assert_eq!(
            translation.definitions.definitions[&rule.definition_id].capability,
            "axioval:capability.attribute-value"
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
            .map(|finding| finding.object_id.local_id.clone())
            .collect::<Vec<_>>()
    };
    let attribute = |cardinality: &str, value: &str| {
        let value = if value.is_empty() {
            String::new()
        } else {
            format!("<value><simpleValue>{value}</simpleValue></value>")
        };
        format!(
            "<attribute {cardinality}><name><simpleValue>GlobalId</simpleValue></name>{value}</attribute>"
        )
    };
    // Every wall in the model has a GlobalId; only #1's is ...01.
    assert!(flagged(&attribute("", "")).is_empty());
    assert_eq!(flagged(&attribute("", "0000000000000000000001")), ["#2"]);
    let name = "<attribute><name><simpleValue>Name</simpleValue></name></attribute>";
    // No wall in the model is named.
    assert_eq!(flagged(name), ["#1", "#2"]);
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
