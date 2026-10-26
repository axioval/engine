//! `.mcs` archives packed by the MCS packer read, and each change the MCS
//! reader refuses is refused here too.
#![allow(missing_docs)]

mod support;

use axioval_mcs::{Archive, McsError, PackageKind, read, resolve_definitions};
use serde_json::{Value, json};
use support::{Parts, fixture, unpack, write_zip};

const LIBRARY_RULES_DEFINITIONS: [&str; 2] = [
    "axioval:example.library-concepts",
    "axioval:example.template-library",
];

fn library() -> Archive {
    read(&fixture("template-library.mcs")).unwrap()
}

fn refused(parts: &mut Parts) -> McsError {
    read(&parts.pack()).unwrap_err()
}

#[test]
fn the_template_library_reads_as_a_definitions_package() {
    let archive = library();
    assert_eq!(archive.kind(), PackageKind::Definitions);
    assert_eq!(archive.identity().id, "axioval:example.template-library");
    assert_eq!(archive.identity().version, "0.1.0");
    assert_eq!(archive.pkl_version(), "0.32.1");
    assert!(archive.ruleset().is_none());
    let [definitions] = archive.definitions() else {
        panic!("{:?}", archive.definitions());
    };
    assert_eq!(definitions.package.id, "axioval:example.template-library");
    assert_eq!(
        definitions.definitions.keys().collect::<Vec<_>>(),
        [
            "axioval:example.template.boolean-property-equals",
            "axioval:example.template.property-exists",
        ]
    );
    assert_eq!(
        archive.clone().into_ruleset().unwrap_err(),
        McsError::WrongKind {
            expected: PackageKind::RuleSet,
            found: PackageKind::Definitions,
        }
    );
    assert_eq!(archive.into_definitions().unwrap().len(), 1);
}

#[test]
fn ruleset_archives_read_with_the_definitions_they_carry() {
    let minimal = read(&fixture("minimal.mcs")).unwrap();
    assert_eq!(minimal.kind(), PackageKind::RuleSet);
    assert_eq!(minimal.identity().id, "axioval:example.ruleset");
    assert_eq!(
        minimal.clone().into_definitions().unwrap_err(),
        McsError::WrongKind {
            expected: PackageKind::Definitions,
            found: PackageKind::RuleSet,
        }
    );
    let (ruleset, carried) = minimal.into_ruleset().unwrap();
    assert_eq!(ruleset.definition_packages, ["axioval:example.definitions"]);
    assert_eq!(carried.len(), 1);

    let (ruleset, carried) = read(&fixture("library-rules.mcs"))
        .unwrap()
        .into_ruleset()
        .unwrap();
    assert_eq!(ruleset.definition_packages, LIBRARY_RULES_DEFINITIONS);
    // In the manifest's `definitionEntrypoints` order.
    let ids: Vec<&str> = carried.iter().map(|p| p.package.id.as_str()).collect();
    assert_eq!(ids, LIBRARY_RULES_DEFINITIONS);
    // The library the rule set carries is the published one.
    assert_eq!(carried[1], library().definitions()[0]);
}

#[test]
fn the_template_library_compiles_with_a_ruleset_instantiating_it() {
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let (ruleset, carried) = read(&fixture("library-rules.mcs"))
        .unwrap()
        .into_ruleset()
        .unwrap();
    let definitions = resolve_definitions(library().into_definitions().unwrap(), carried).unwrap();
    // The carried copy equals the supplied one, so it is kept once.
    assert_eq!(definitions.len(), 2);
    let plan = axioval_engine::compile(&registry, &definitions, &ruleset).unwrap();
    assert_eq!(plan.rules().len(), 1);
    assert_eq!(
        plan.rules()[0].capability,
        "axioval:capability.property-exists"
    );
    // The library's own definitions bind as a catalogue too.
    axioval_rules::catalogue::catalogue(&registry, library().definitions()).unwrap();
}

#[test]
fn an_unpacked_archive_packs_again_into_one_that_reads_the_same() {
    for name in ["template-library.mcs", "minimal.mcs", "library-rules.mcs"] {
        let original = read(&fixture(name)).unwrap();
        let again = read(&unpack(&fixture(name)).pack()).unwrap();
        assert_eq!(again.identity(), original.identity(), "{name}");
        assert_eq!(again.definitions(), original.definitions(), "{name}");
        assert_eq!(again.ruleset(), original.ruleset(), "{name}");
    }
}

#[test]
fn archives_are_recognised_by_their_mimetype() {
    assert!(axioval_mcs::is_archive(&fixture("minimal.mcs")));
    assert!(!axioval_mcs::is_archive(b"{\"schemaVersion\":\"0.1.0\"}"));
    assert!(!axioval_mcs::is_archive(&write_zip(&[
        ("model.ifc".into(), b"ISO-10303-21;".to_vec()),
        ("b".into(), Vec::new()),
    ])));
}

// The kind comes from the archived manifest.

#[test]
fn a_definitions_archive_carrying_a_rule_set_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let ruleset = unpack(&fixture("library-rules.mcs")).files["normalized/ruleset.json"].clone();
    parts
        .files
        .insert("normalized/ruleset.json".into(), ruleset);
    let root = parts.metadata["packageRoot"].as_str().unwrap().to_owned();
    parts.files.insert(
        format!("{root}/ruleset.pkl"),
        b"amends \"x.pkl\"\n".to_vec(),
    );
    parts.metadata["normalized"]["ruleset.pkl"] = json!("normalized/ruleset.json");
    assert_eq!(refused(&mut parts), McsError::DefinitionsCarryRuleSet);
}

#[test]
fn a_ruleset_archive_without_a_rule_set_is_refused() {
    let mut parts = unpack(&fixture("library-rules.mcs"));
    parts.files.remove("normalized/ruleset.json");
    parts.metadata["normalized"]
        .as_object_mut()
        .unwrap()
        .remove("ruleset.pkl");
    assert_eq!(refused(&mut parts), McsError::RuleSetMissing);
}

#[test]
fn metadata_naming_another_package_than_the_manifest_is_refused() {
    for (field, value) in [
        ("version", "0.2.0"),
        ("id", "axioval:example.other"),
        ("schemaVersion", "0.2.0"),
    ] {
        let mut parts = unpack(&fixture("template-library.mcs"));
        parts.metadata["package"][field] = json!(value);
        assert!(
            matches!(refused(&mut parts), McsError::IdentityMismatch { .. }),
            "{field}"
        );
    }
}

#[test]
fn a_definitions_manifest_naming_none_of_its_documents_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let manifest = parts.manifest();
    let mut value = parts.json(&manifest);
    value["id"] = json!("axioval:example.other-library");
    parts.set_json(&manifest, &value);
    parts.metadata["package"]["id"] = json!("axioval:example.other-library");
    assert_eq!(
        refused(&mut parts),
        McsError::DefinitionsIdentity {
            id: "axioval:example.other-library".into(),
            version: "0.1.0".into(),
            matches: 0,
        }
    );
}

#[test]
fn a_definitions_manifest_naming_another_version_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let member = parts.normalized("definitions.pkl");
    let mut document = parts.json(&member);
    document["package"]["version"] = json!("0.1.1");
    parts.set_json(&member, &document);
    assert!(matches!(
        refused(&mut parts),
        McsError::DefinitionsIdentity { matches: 0, .. }
    ));
}

#[test]
fn a_definitions_package_whose_documents_share_a_package_is_refused() {
    // Two documents with the manifest's identity bind as one package twice.
    let mut parts = unpack(&fixture("template-library.mcs"));
    let root = parts.metadata["packageRoot"].as_str().unwrap().to_owned();
    let member = parts.normalized("definitions.pkl");
    let copy = "normalized/definitions/0000000000000000.json";
    parts
        .files
        .insert(copy.into(), parts.files[&member].clone());
    parts.files.insert(
        format!("{root}/copy.pkl"),
        b"amends \"definitions.pkl\"\n".to_vec(),
    );
    parts.metadata["normalized"]["copy.pkl"] = json!(copy);
    let manifest = parts.manifest();
    let mut value = parts.json(&manifest);
    value["definitionEntrypoints"] = json!(["definitions.pkl", "copy.pkl"]);
    parts.set_json(&manifest, &value);
    assert!(
        matches!(refused(&mut parts), McsError::Binding(detail) if detail.contains("duplicate definition package"))
    );
}

// `definitionPackages` resolve from the modules a rule set carries or from
// the definitions archives supplied, and an unknown ID fails closed.

/// `library-rules.mcs` without the library it carries.
fn rules_without_library() -> Vec<u8> {
    let mut parts = unpack(&fixture("library-rules.mcs"));
    let member = parts.normalized("library.pkl");
    parts.files.remove(&member);
    parts.metadata["normalized"]
        .as_object_mut()
        .unwrap()
        .remove("library.pkl");
    let manifest = parts.manifest();
    let mut value = parts.json(&manifest);
    value["definitionEntrypoints"] = json!(["concepts.pkl"]);
    parts.set_json(&manifest, &value);
    parts.pack()
}

#[test]
fn a_definition_package_neither_carried_nor_supplied_is_refused_by_the_compiler() {
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let (ruleset, carried) = read(&rules_without_library())
        .unwrap()
        .into_ruleset()
        .unwrap();
    let definitions = resolve_definitions(Vec::new(), carried.clone()).unwrap();
    let error = axioval_engine::compile(&registry, &definitions, &ruleset).unwrap_err();
    assert_eq!(
        error,
        axioval_engine::EngineError::MissingDefinitionPackage(
            "axioval:example.template-library".into()
        )
    );
    // Supplying the definitions archive resolves it.
    let definitions = resolve_definitions(library().into_definitions().unwrap(), carried).unwrap();
    axioval_engine::compile(&registry, &definitions, &ruleset).unwrap();
}

#[test]
fn a_carried_package_the_rule_set_does_not_declare_is_refused() {
    let mut parts = unpack(&fixture("library-rules.mcs"));
    let mut ruleset = parts.json("normalized/ruleset.json");
    ruleset["definitionPackages"] = json!(["axioval:example.library-concepts"]);
    parts.set_json("normalized/ruleset.json", &ruleset);
    assert!(
        matches!(refused(&mut parts), McsError::Binding(detail) if detail.contains("axioval:example.template-library"))
    );
}

#[test]
fn a_carried_package_differing_from_a_supplied_one_is_refused() {
    let (_, carried) = read(&fixture("library-rules.mcs"))
        .unwrap()
        .into_ruleset()
        .unwrap();
    let mut supplied = library().into_definitions().unwrap();
    supplied[0].package.version = "0.1.1".into();
    assert_eq!(
        resolve_definitions(supplied, carried).unwrap_err(),
        McsError::ConflictingDefinitions("axioval:example.template-library".into())
    );
}

// Archive safety.

fn with_member(name: &str, data: &[u8]) -> Vec<u8> {
    let mut members = support::members(&fixture("template-library.mcs"));
    members.push((name.to_owned(), data.to_vec()));
    write_zip(&members)
}

#[test]
fn member_names_escaping_the_archive_are_refused() {
    for name in [
        "source/../../etc/passwd",
        "/etc/passwd",
        "source\\..\\evil",
        "source/./a",
        "source//a",
        "source/a\u{1}",
        "source/e\u{301}.txt",
    ] {
        assert_eq!(
            read(&with_member(name, b"x")).unwrap_err(),
            McsError::UnsafeName(name.into()),
            "{name:?}"
        );
    }
}

#[test]
fn duplicate_members_are_refused() {
    let members = support::members(&fixture("template-library.mcs"));
    let mut doubled = members.clone();
    doubled.push(members[2].clone());
    assert_eq!(
        read(&write_zip(&doubled)).unwrap_err(),
        McsError::DuplicateMember(members[2].0.clone())
    );
    let folded = members[2].0.to_uppercase().replace("SOURCE", "source");
    assert_eq!(
        read(&with_member(&folded, b"x")).unwrap_err(),
        McsError::DuplicateMember(folded)
    );
}

/// Names collide as Python's `str.casefold()` folds them after NFC, so a
/// non-ASCII pair is refused where upper- then lowercasing would differ
/// (`STRAẞE` lowercases to `straße`, folds to `strasse`).
#[test]
fn names_equal_once_casefolded_are_refused() {
    for (first, second) in [
        ("source/STRASSE.txt", "source/straße.txt"),
        ("source/strasse.txt", "source/STRAẞE.txt"),
        ("source/ΟΔΟΣ.txt", "source/οδος.txt"),
        ("source/Ärger.txt", "source/ärger.txt"),
    ] {
        let mut members = support::members(&fixture("template-library.mcs"));
        members.push((first.to_owned(), b"x".to_vec()));
        members.push((second.to_owned(), b"y".to_vec()));
        assert_eq!(
            read(&write_zip(&members)).unwrap_err(),
            McsError::DuplicateMember(second.into()),
            "{first:?} and {second:?}"
        );
    }
    // Folding equates cases, not letters: `ä` and `a` stay apart.
    let mut members = support::members(&fixture("template-library.mcs"));
    members.push(("source/ärger.txt".to_owned(), b"x".to_vec()));
    members.push(("source/arger.txt".to_owned(), b"y".to_vec()));
    assert!(!matches!(
        read(&write_zip(&members)).unwrap_err(),
        McsError::DuplicateMember(_)
    ));
}

#[test]
fn oversized_members_and_archives_are_refused() {
    let big = vec![b'a'; axioval_mcs::MAX_FILE + 1];
    assert!(matches!(
        read(&with_member("source/big.txt", &big)).unwrap_err(),
        McsError::Limit(detail) if detail.contains("exceeds")
    ));
    let bomb = vec![0_u8; 1_000_000];
    assert!(matches!(
        read(&with_member("source/bomb.txt", &bomb)).unwrap_err(),
        McsError::Limit(detail) if detail.contains("100 to 1")
    ));
    let huge = vec![0_u8; axioval_mcs::MAX_COMPRESSED + 1];
    assert!(matches!(read(&huge).unwrap_err(), McsError::Limit(_)));
    let mut many = support::members(&fixture("template-library.mcs"));
    for index in 0..axioval_mcs::MAX_ENTRIES {
        many.push((format!("source/many/{index}"), Vec::new()));
    }
    assert_eq!(
        read(&write_zip(&many)).unwrap_err(),
        McsError::Container("invalid archive entry count".into())
    );
}

#[test]
fn non_canonical_containers_are_refused() {
    let raw = fixture("template-library.mcs");
    let mut trailing = raw.clone();
    trailing.push(0);
    assert!(matches!(
        read(&trailing).unwrap_err(),
        McsError::Container(_)
    ));
    let mut preamble = vec![0_u8; 4];
    preamble.extend(&raw);
    assert!(matches!(
        read(&preamble).unwrap_err(),
        McsError::Container(_)
    ));
    // A timestamp other than 1980-01-01.
    let mut dated = raw.clone();
    dated[10] = 1;
    assert!(matches!(read(&dated).unwrap_err(), McsError::Container(_)));
    // A flipped payload byte fails its CRC-32.
    let mut flipped = raw.clone();
    let last = raw.len() - 23 - 2000;
    flipped[last] ^= 0xff;
    assert!(read(&flipped).is_err());
    // mimetype first.
    let mut members = support::members(&raw);
    members.swap(0, 1);
    assert!(matches!(
        read(&write_zip(&members)).unwrap_err(),
        McsError::Container(_)
    ));
}

#[test]
fn an_inventory_disagreeing_with_the_members_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let packed = parts.pack();
    // A member left out of the inventory.
    let mut members = support::members(&packed);
    members.push(("source/extra.txt".into(), b"x".to_vec()));
    assert_eq!(
        read(&write_zip(&members)).unwrap_err(),
        McsError::Metadata("archive contains undeclared or missing payload".into())
    );
    // A member changed after it was inventoried.
    let mut members = support::members(&packed);
    let index = members
        .iter()
        .position(|(name, _)| name == "source/LICENSE")
        .unwrap();
    members[index].1.push(b'\n');
    assert_eq!(
        read(&write_zip(&members)).unwrap_err(),
        McsError::Metadata("inventory size/hash mismatch".into())
    );
}

#[test]
fn metadata_with_a_duplicate_key_is_refused() {
    let mut members = support::members(&fixture("template-library.mcs"));
    let text = String::from_utf8(members[1].1.clone()).unwrap();
    members[1].1 = text
        .replacen('{', "{\"format\":\"axioval.mcs\",", 1)
        .into_bytes();
    assert!(matches!(
        read(&write_zip(&members)).unwrap_err(),
        McsError::Metadata(detail) if detail.contains("duplicate JSON key")
    ));
}

#[test]
fn an_invalid_archived_manifest_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let manifest = parts.manifest();
    let mut value: Value = parts.json(&manifest);
    value["entrypoint"] = json!("ruleset.pkl");
    parts.set_json(&manifest, &value);
    assert!(
        matches!(refused(&mut parts), McsError::Manifest(detail) if detail.contains("entrypoint"))
    );
}

#[test]
fn a_normalized_mapping_disagreeing_with_the_manifest_is_refused() {
    let mut parts = unpack(&fixture("template-library.mcs"));
    let manifest = parts.manifest();
    let mut value: Value = parts.json(&manifest);
    value["definitionEntrypoints"] = json!(["definitions.pkl", "other.pkl"]);
    parts.set_json(&manifest, &value);
    assert_eq!(
        refused(&mut parts),
        McsError::Metadata("normalized mapping disagrees with manifest entrypoints".into())
    );
}
