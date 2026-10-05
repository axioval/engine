//! The authoring catalogue against its golden copy: any change to what a
//! rule may be built from shows in review as a change to
//! `tests/golden/catalogue.json`. Regenerate it with
//! `AXIOVAL_BLESS=1 cargo test -p axioval-rules --test catalogue`.

use axioval_engine::CapabilityRegistry;
use axioval_ir::DefinitionPackage;
use axioval_rules::catalogue::catalogue;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/catalogue.json");

fn built_ins() -> CapabilityRegistry {
    axioval_rules::register_builtins(CapabilityRegistry::new()).expect("built-ins register")
}

fn rendered(packages: &[DefinitionPackage]) -> String {
    let catalogue = catalogue(&built_ins(), packages).expect("every built-in is described");
    let mut json = serde_json::to_string_pretty(&catalogue).expect("the catalogue serializes");
    json.push('\n');
    json
}

#[test]
fn the_catalogue_matches_its_golden_copy() {
    let json = rendered(&[]);
    if std::env::var_os("AXIOVAL_BLESS").is_some() {
        std::fs::write(GOLDEN, &json).expect("the golden copy is writable");
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN).expect("the golden copy exists");
    assert!(
        json == golden,
        "the catalogue changed; review it and regenerate the golden copy with \
         `AXIOVAL_BLESS=1 cargo test -p axioval-rules --test catalogue`"
    );
}

#[test]
fn the_catalogue_is_deterministic_and_versioned() {
    assert_eq!(rendered(&[]), rendered(&[]));
    let value: serde_json::Value = serde_json::from_str(&rendered(&[])).unwrap();
    assert_eq!(
        value["schemaVersion"],
        axioval_ir::catalogue::CATALOGUE_SCHEMA_VERSION
    );
    let ids: Vec<&str> = value["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|capability| capability["id"].as_str().unwrap())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted);
    assert_eq!(ids.len(), built_ins().ids().count());
}

#[test]
fn every_built_in_measured_value_is_available() {
    let value: serde_json::Value = serde_json::from_str(&rendered(&[])).unwrap();
    for list in ["measuredValues", "measuredMembers"] {
        for entry in value[list].as_array().unwrap() {
            assert_eq!(entry["available"], true, "{}", entry["name"]);
        }
    }
}

#[test]
fn the_catalogue_lists_a_package_s_concepts() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let package: DefinitionPackage = serde_json::from_str(&text).unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered(&[package.clone()])).unwrap();
    let concepts = &value["concepts"][0];
    assert_eq!(concepts["package"], package.package.id.as_str());
    assert_eq!(
        concepts["objectTypes"].as_object().unwrap().len(),
        package.object_types.len()
    );
    assert_eq!(
        concepts["definitions"].as_object().unwrap().len(),
        package.definitions.len()
    );
}

#[test]
fn the_german_catalogue_translates_every_built_in_entry() {
    use axioval_engine::catalogue::{CatalogueError, localized};
    let built = catalogue(&built_ins(), &[]).unwrap();
    for locale in ["en", "de"] {
        let (value, fallbacks) = localized(&built, locale).unwrap();
        assert!(fallbacks.is_empty(), "{locale}: {fallbacks:?}");
        assert_eq!(value["languages"], serde_json::json!([locale]));
    }
    let (german, _) = localized(&built, "de").unwrap();
    assert_eq!(german["valueTypes"][0]["label"], "Wahrheitswert");
    let stair = german["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capability| capability["id"] == "axioval:capability.stair-geometry")
        .unwrap();
    assert_eq!(stair["label"], "Treppengeometrie");
    assert!(stair["parameters"][0]["help"].is_string());
    // No list of texts survives localization.
    let text = serde_json::to_string(&german).unwrap();
    assert!(!text.contains("\"language\":\"en\""));
    assert_eq!(
        localized(&built, "fr").unwrap_err(),
        CatalogueError::UnsupportedLocale("fr".into())
    );
}

/// A capability built as a template carries its composition: its forms as
/// expressions with their block trees, which map back to the same
/// expressions. A rule bound to it forks into an `expression` rule whose
/// requirement is the form with the rule's parameters bound in, and whose
/// block tree an editor opens as the starting point of a new rule.
#[test]
fn a_template_s_composition_expands_into_blocks_and_forks() {
    use axioval_engine::CompiledRule;
    use axioval_ir::RuleId;
    use axioval_ir::blocks::{from_blocks, to_blocks};
    use axioval_ir::contract::{Expression, ParameterValue, Selector, Severity};

    let value: serde_json::Value = serde_json::from_str(&rendered(&[])).unwrap();
    let capabilities = value["capabilities"].as_array().unwrap();
    let templated: Vec<&str> = capabilities
        .iter()
        .filter(|capability| capability.get("template").is_some())
        .map(|capability| capability["id"].as_str().unwrap())
        .collect();
    assert_eq!(templated, ["axioval:capability.body-extent"]);
    let template = &capabilities
        .iter()
        .find(|capability| capability["id"] == "axioval:capability.body-extent")
        .unwrap()["template"];
    assert_eq!(template["name"], "body-extent");
    let requirements = template["requirements"].as_array().unwrap();
    assert_eq!(
        requirements.len(),
        template["forms"].as_array().unwrap().len()
    );
    assert_eq!(
        requirements[0]["when"],
        serde_json::json!(["target_property"])
    );
    for requirement in requirements {
        let expression: Expression =
            serde_json::from_value(requirement["expression"].clone()).unwrap();
        let blocks = serde_json::from_value(requirement["blocks"].clone()).unwrap();
        assert_eq!(from_blocks(&blocks).unwrap(), expression);
    }

    let registry = built_ins();
    let capability = registry.get("axioval:capability.body-extent").unwrap();
    let rule = CompiledRule {
        id: RuleId::new("wall-thickness").unwrap(),
        capability: "axioval:capability.body-extent".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: [
            (
                "axis".to_owned(),
                ParameterValue::String {
                    value: "forward".into(),
                },
            ),
            (
                "minimum".to_owned(),
                ParameterValue::Quantity {
                    value: 150.0,
                    unit: "mm".into(),
                },
            ),
        ]
        .into(),
    };
    let fork = axioval_rules::templates::fork(capability.as_ref(), &rule).unwrap();
    let text = serde_json::to_string(&fork.requirement).unwrap();
    // Every slot is filled and every parameter folded in; the bound the
    // rule leaves unstated is left out.
    assert!(!text.contains("{axis}"), "{text}");
    assert!(text.contains("body_extent;axis=forward"), "{text}");
    assert!(
        text.contains("body_position;axis=forward;end=low"),
        "{text}"
    );
    assert!(!text.contains("\"parameter\""), "{text}");
    assert!(!text.contains("at most the upper bound"), "{text}");
    let blocks = to_blocks(&fork.requirement).unwrap();
    assert_eq!(from_blocks(&blocks).unwrap(), fork.requirement);
    // A measured read with its parameters bound is canonical: a
    // `measured.` block with the axis as a field.
    let blocks = serde_json::to_string(&blocks).unwrap();
    assert!(blocks.contains("\"measured.body_extent\""), "{blocks}");
}
