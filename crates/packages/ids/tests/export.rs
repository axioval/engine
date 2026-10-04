//! Exporting rule packages as IDS: what is exported, what is refused and
//! why, and that an export checks a real model as the package did.
#![allow(missing_docs, clippy::doc_markdown)]

use std::collections::BTreeMap;

use axioval::default_registry;
use axioval::engine::{Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval::ir::contract::{DefinitionPackage, RuleSetPackage, Selector};
use axioval::ir::{MATERIAL_SET, Report};
use axioval_export::{ExportProfile, LossKind};
use axioval_ids::{
    DocumentError, Export, IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM, IFC4X3_TYPE_SYSTEM, IdsProfile,
    Options, Refusal, SPECIFICATION_ANNOTATION, UNWRITABLE_ANNOTATION, export, translate,
};
use openbim_ids::{Facet, Occurrence, Value};
use serde_json::{Value as Json, json};

fn text(value: &str) -> Json {
    json!({ "default": value, "translations": {} })
}

/// A concept named `name` in every release IDS names.
fn concept(id: &str, name: &str) -> Json {
    json!({
        "id": id,
        "name": text(name),
        "externalNames": [
            { "typeSystem": IFC2X3_TYPE_SYSTEM, "name": name },
            { "typeSystem": IFC4_TYPE_SYSTEM, "name": name },
            { "typeSystem": IFC4X3_TYPE_SYSTEM, "name": name },
        ],
    })
}

/// A property concept, which also carries a value kind.
fn property_concept(id: &str, name: &str) -> Json {
    let mut concept = concept(id, name);
    concept["valueKind"] = json!("string");
    concept
}

/// A definition of `capability`, its parameters taken from the registry.
fn definition(capability: &str) -> Json {
    let registry = default_registry().unwrap();
    let parameters: serde_json::Map<String, Json> = registry
        .get(&format!("axioval:capability.{capability}"))
        .unwrap_or_else(|| panic!("{capability} is registered"))
        .parameters()
        .into_iter()
        .map(|descriptor| {
            let mut parameter = json!({
                "id": descriptor.name,
                "name": text(&descriptor.name),
                "kind": descriptor.parameter_type.package_kind(),
                "required": descriptor.required,
            });
            if let axioval::engine::ParameterType::Table(columns) = descriptor.parameter_type {
                parameter["columns"] = columns
                    .iter()
                    .map(|column| {
                        json!({
                            "id": column.id,
                            "name": text(column.id),
                            "kind": column.kind,
                            "required": column.required,
                        })
                    })
                    .collect();
            }
            (descriptor.name.clone(), parameter)
        })
        .collect();
    json!({
        "id": format!("t:{capability}"),
        "name": text(capability),
        "capability": format!("axioval:capability.{capability}"),
        "parameters": parameters,
    })
}

fn definitions() -> DefinitionPackage {
    let capabilities = [
        "property-required",
        "property-data-type",
        "property-value",
        "property-requirements",
        "classification",
        "selector-conformance",
        "object-count",
        "clash",
    ];
    let definitions: serde_json::Map<String, Json> = capabilities
        .iter()
        .map(|capability| (format!("t:{capability}"), definition(capability)))
        .collect();
    let mut ifc4_only = concept("t:slab", "IfcSlab");
    ifc4_only["externalNames"] = json!([{ "typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSlab" }]);
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "t:definitions",
            "name": text("definitions"),
            "version": "1.0.0",
            "authors": [],
        },
        "objectTypes": {
            "t:wall": concept("t:wall", "IfcWall"),
            "t:storey": concept("t:storey", "IfcBuildingStorey"),
            "t:slab": ifc4_only,
        },
        "properties": {
            "t:fire-rating": property_concept("t:fire-rating", "FireRating"),
            "t:combustible": property_concept("t:combustible", "Combustible"),
            "t:width": property_concept("t:width", "Width"),
            "t:name": property_concept("t:name", "Name"),
            "t:kind": property_concept("t:kind", "Kind"),
        },
        "propertySets": {
            "t:wall-common": concept("t:wall-common", "Pset_WallCommon"),
            "t:custom": concept("t:custom", "Custom"),
        },
        "definitions": definitions,
    }))
    .unwrap()
}

fn walls() -> Json {
    json!({ "kind": "entityType", "objectType": "t:wall", "includeSubtypes": false })
}

fn reference(set: &str, property: &str) -> Json {
    json!({ "type": "propertyReference", "propertySet": set, "property": property })
}

fn strings(values: &[&str]) -> Json {
    json!({ "type": "stringList", "value": values })
}

fn string(value: &str) -> Json {
    json!({ "type": "string", "value": value })
}

fn rule(id: &str, capability: &str, applicability: Json, parameters: Json) -> Json {
    let mut rule = json!({
        "id": id,
        "definitionId": format!("t:{capability}"),
        "name": text(id),
        "severity": "error",
    });
    rule["parameters"] = parameters;
    rule["applicability"] = applicability;
    rule
}

/// A property requirement of each kind IDS states, one rule each, over
/// walls.
fn property_rules() -> Vec<Json> {
    let yes = json!({ "type": "boolean", "value": true });
    vec![
        rule(
            "fire-rating-required",
            "property-required",
            walls(),
            json!({ "property": reference("t:wall-common", "t:fire-rating") }),
        ),
        rule(
            "fire-rating-label",
            "property-data-type",
            walls(),
            json!({
                "property": reference("t:wall-common", "t:fire-rating"),
                "data_type": string("IFCLABEL"),
            }),
        ),
        rule(
            "fire-rating-values",
            "property-value",
            walls(),
            json!({
                "property": reference("t:wall-common", "t:fire-rating"),
                "values": strings(&["EI 90", "EI 60"]),
                "quantifier": string("any"),
                "si_units": yes,
            }),
        ),
        rule(
            "width-bounds",
            "property-value",
            walls(),
            json!({
                "property": reference("t:custom", "t:width"),
                "min_inclusive": string("0.1"),
                "max_inclusive": string("0.5"),
                "quantifier": string("all"),
                "si_units": yes,
            }),
        ),
        rule(
            "rating-pattern",
            "property-value",
            walls(),
            json!({
                "property_set_pattern": string("Pset_.*Common"),
                "property_pattern": string("(FireRating)|(AcousticRating)"),
                "patterns": strings(&["EI [0-9]+"]),
                "quantifier": string("any"),
                "si_units": yes,
                "optional": yes,
            }),
        ),
    ]
}

/// Every other alphanumerical requirement IDS states, one rule each.
fn other_rules() -> Vec<Json> {
    vec![
        rule(
            "not-combustible",
            "property-requirements",
            walls(),
            json!({ "requirements": { "type": "table", "value": [{
                "property_set": string("t:wall-common"),
                "property": string("t:combustible"),
                "state": string("exclude"),
                "presence": string("not-empty"),
            }] } }),
        ),
        rule(
            "named",
            "property-value",
            walls(),
            json!({
                "property": reference("axioval:attributes", "t:name"),
                "values": strings(&["W1"]),
            }),
        ),
        rule(
            "classified",
            "classification",
            walls(),
            json!({ "systems": strings(&["Uniclass"]), "codes": strings(&["Ss_25"]) }),
        ),
        rule(
            "has-material",
            "selector-conformance",
            walls(),
            json!({ "requirement": { "type": "selector", "value": {
                "kind": "property", "propertySet": MATERIAL_SET, "property": "t:kind",
                "operator": "exists", "value": null,
            } } }),
        ),
        rule(
            "on-a-storey",
            "selector-conformance",
            walls(),
            json!({ "requirement": { "type": "selector", "value": {
                "kind": "related",
                "path": ["IfcRelContainedInSpatialStructure:backward+"],
                "selector": { "kind": "entityType", "objectType": "t:storey", "includeSubtypes": false },
            } } }),
        ),
        rule(
            "some-wall",
            "object-count",
            walls(),
            json!({ "minimum": { "type": "integer", "value": 1 } }),
        ),
    ]
}

/// Every alphanumerical requirement IDS states, one rule each, over walls.
fn alphanumerical() -> Vec<Json> {
    let mut rules = property_rules();
    rules.extend(other_rules());
    rules
}

fn ruleset(rules: Vec<Json>) -> RuleSetPackage {
    let mut package = json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "t:rules",
            "name": text("Wall information"),
            "version": "1.2.0",
            "description": text("What every wall states"),
            "authors": ["Someone", "someone@example.org"],
        },
        "definitionPackages": ["t:definitions"],
        "root": { "id": "root", "name": text("root") },
    });
    package["root"]["rules"] = Json::Array(rules);
    serde_json::from_value(package).unwrap()
}

fn reasons(export: &Export) -> BTreeMap<&str, &Refusal> {
    export
        .not_exported
        .iter()
        .map(|entry| (entry.rule.as_str(), &entry.reason))
        .collect()
}

#[test]
fn a_mixed_package_exports_its_property_rules_and_lists_the_clash_rule() {
    let mut rules = alphanumerical();
    rules.push(rule(
        "wall-clash",
        "clash",
        walls(),
        json!({ "other": { "type": "selector", "value": walls() } }),
    ));
    let export = export(&[definitions()], &ruleset(rules));
    let exported: Vec<&str> = export
        .specifications
        .iter()
        .flat_map(|specification| specification.rules.iter().map(String::as_str))
        .collect();
    assert_eq!(
        exported,
        [
            "fire-rating-required",
            "fire-rating-label",
            "fire-rating-values",
            "width-bounds",
            "rating-pattern",
            "not-combustible",
            "named",
            "classified",
            "has-material",
            "on-a-storey",
            "some-wall",
        ]
    );
    assert!(!export.is_complete());
    assert_eq!(
        reasons(&export),
        BTreeMap::from([(
            "wall-clash",
            &Refusal::Capability("axioval:capability.clash".to_owned())
        )])
    );
    // Each rule is one specification, named and identified after it.
    let first = &export.specifications[0].specification;
    assert_eq!(first.name, "fire-rating-required");
    assert_eq!(first.identifier.as_deref(), Some("fire-rating-required"));
    let facets: Vec<(&str, Occurrence)> = export
        .specifications
        .iter()
        .filter_map(|exported| exported.specification.requirements.as_ref())
        .map(|requirements| {
            let requirement = &requirements.facets[0];
            (requirement.facet.kind(), requirement.occurrence)
        })
        .collect();
    assert_eq!(
        facets,
        [
            ("property", Occurrence::Required),
            ("property", Occurrence::Required),
            ("property", Occurrence::Required),
            ("property", Occurrence::Required),
            ("property", Occurrence::Optional),
            ("property", Occurrence::Prohibited),
            ("attribute", Occurrence::Required),
            ("classification", Occurrence::Required),
            ("material", Occurrence::Required),
            ("partOf", Occurrence::Required),
        ]
    );
    // The count is the last specification's cardinality.
    let count = &export.specifications[10].specification;
    assert_eq!(count.applicability.min_occurs, 1);
    assert!(count.requirements.is_none());
    // Names given by an escaped alternation are an enumeration again.
    let Some(requirements) = &export.specifications[4].specification.requirements else {
        panic!("a requirement");
    };
    let Facet::Property(pattern) = &requirements.facets[0].facet else {
        panic!("a property facet");
    };
    let Value::Restriction(names) = &pattern.base_name else {
        panic!("an enumeration");
    };
    assert_eq!(names.enumeration, ["FireRating", "AcousticRating"]);
    // The package's metadata is the document's info.
    assert_eq!(export.info.title, "Wall information");
    assert_eq!(export.info.version.as_deref(), Some("1.2.0"));
    assert_eq!(export.info.author.as_deref(), Some("someone@example.org"));
    // The document reads back as written.
    let xml = export.to_xml().unwrap().unwrap();
    let read = openbim_ids::from_str(&xml).unwrap_or_else(|error| panic!("{error}\n{xml}"));
    assert_eq!(read.info, export.info);
    let written: Vec<_> = export
        .specifications
        .iter()
        .map(|exported| exported.specification.clone())
        .collect();
    assert_eq!(read.specifications, written);
}

/// A wall on a storey with every fact the rules ask for, and a bare wall.
const MODEL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,$,$,$);
#3=IFCBUILDINGSTOREY('0000000000000000000003',$,'Level 1',$,$,$,$,$,.ELEMENT.,0.);
#4=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000004',$,$,$,(#1),#3);
#5=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 90'),$);
#6=IFCPROPERTYSINGLEVALUE('Combustible',$,IFCBOOLEAN(.F.),$);
#7=IFCPROPERTYSET('0000000000000000000007',$,'Pset_WallCommon',$,(#5,#6));
#8=IFCRELDEFINESBYPROPERTIES('0000000000000000000008',$,$,$,(#1),#7);
#9=IFCPROPERTYSINGLEVALUE('Width',$,IFCREAL(0.3),$);
#10=IFCPROPERTYSINGLEVALUE('Width',$,IFCREAL(0.7),$);
#11=IFCPROPERTYSET('0000000000000000000011',$,'Custom',$,(#9));
#12=IFCPROPERTYSET('0000000000000000000012',$,'Custom',$,(#10));
#13=IFCRELDEFINESBYPROPERTIES('0000000000000000000013',$,$,$,(#1),#11);
#14=IFCRELDEFINESBYPROPERTIES('0000000000000000000014',$,$,$,(#2),#12);
#20=IFCCLASSIFICATION($,$,$,'Uniclass',$,$,$);
#21=IFCCLASSIFICATIONREFERENCE($,'Ss_25',$,#20,$,$);
#22=IFCRELASSOCIATESCLASSIFICATION('0000000000000000000022',$,$,$,(#1),#21);
#30=IFCMATERIAL('Concrete',$,$);
#31=IFCRELASSOCIATESMATERIAL('0000000000000000000031',$,$,$,(#1),#30);
ENDSEC;
END-ISO-10303-21;
";

fn run(definitions: &DefinitionPackage, ruleset: &RuleSetPackage) -> Report {
    let registry = default_registry().unwrap();
    let plan = compile(&registry, std::slice::from_ref(definitions), ruleset).unwrap();
    let session = import_ifc_session("model.ifc", MODEL.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

/// What each rule reported, the rule named by `name`: its findings'
/// subjects and severities, and what it left not evaluated.
fn verdicts(report: &Report, name: impl Fn(&str) -> String) -> Vec<String> {
    let mut verdicts: Vec<String> = report
        .findings()
        .iter()
        .map(|finding| {
            format!(
                "{} finding {:?} {:?}",
                name(&finding.rule_id.to_string()),
                finding.scope,
                finding.severity
            )
        })
        .chain(report.not_evaluated().iter().map(|outcome| {
            format!(
                "{} not evaluated {:?} {:?}",
                name(&outcome.rule_id.to_string()),
                outcome.scope,
                outcome.reason
            )
        }))
        .collect();
    verdicts.sort();
    verdicts
}

#[test]
fn an_export_translated_back_reports_what_the_package_did() {
    let definitions = definitions();
    let package = ruleset(alphanumerical());
    let export = export(std::slice::from_ref(&definitions), &package);
    assert!(export.is_complete(), "{:?}", export.not_exported);
    let xml = export.to_xml().unwrap().unwrap();
    let ids = openbim_ids::from_str(&xml).unwrap();
    let translation = translate(&ids, &Options::new("ids:back", "1.0.0")).unwrap();
    assert!(translation.is_complete());
    // Specification n is the n-th exported rule.
    let exported: BTreeMap<String, String> = translation
        .specifications
        .iter()
        .flat_map(|outcome| {
            let rule = export.specifications[outcome.number - 1].rules[0].clone();
            outcome
                .rules
                .iter()
                .map(move |id| (id.clone(), rule.clone()))
        })
        .collect();
    let original = run(&definitions, &package);
    let back = run(&translation.definitions, &translation.ruleset);
    assert!(!original.findings().is_empty());
    assert!(
        original.not_evaluated().is_empty(),
        "{:?}",
        original.not_evaluated()
    );
    assert_eq!(
        verdicts(&original, ToOwned::to_owned),
        verdicts(&back, |id| exported[id].clone())
    );
}

#[test]
fn a_translated_document_is_exported_as_it_was_written() {
    let text = r#"<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema"><info><title>Walls &amp; slabs</title><copyright>c</copyright><version>2</version><description>d</description><author>a@b.org</author><date>2024-06-01</date><purpose>p</purpose><milestone>m</milestone></info><specifications>
      <specification name="Walls" ifcVersion="IFC2X3 IFC4" identifier="W-1" description="Every wall" instructions="Rate &quot;every&quot;&#10;wall">
        <applicability minOccurs="1" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name><predefinedType><simpleValue>SOLIDWALL</simpleValue></predefinedType></entity><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>IsExternal</simpleValue></baseName></property></applicability>
        <requirements description="r"><property cardinality="optional" uri="https://example.org/p" instructions="i" dataType="IFCLABEL"><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue> FireRating </simpleValue></baseName><value><xs:restriction base="xs:string"><xs:enumeration value="EI 90"/><xs:pattern value="EI [0-9]+"/></xs:restriction></value></property><material cardinality="prohibited"/></requirements>
      </specification>
      <specification name="Nothing to check" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCSLAB</simpleValue></name></entity></applicability><requirements><property cardinality="optional"><propertySet><simpleValue>P</simpleValue></propertySet><baseName><simpleValue>Q</simpleValue></baseName></property></requirements></specification>
    </specifications></ids>"#;
    let ids = openbim_ids::from_str(text).unwrap();
    let translation = translate(&ids, &Options::new("ids:walls", "1.0.0")).unwrap();
    assert!(translation.is_complete());
    // Every specification keeps its origin, the one without rules too.
    let folders = &translation.ruleset.root.folders;
    assert_eq!(folders.len(), 2);
    assert!(folders[1].rules.is_empty());
    assert!(
        folders
            .iter()
            .all(|folder| folder.annotations.contains_key(SPECIFICATION_ANNOTATION))
    );
    // Through JSON, as the packages are written.
    let definitions: DefinitionPackage =
        serde_json::from_str(&serde_json::to_string(&translation.definitions).unwrap()).unwrap();
    let ruleset: RuleSetPackage =
        serde_json::from_str(&serde_json::to_string(&translation.ruleset).unwrap()).unwrap();
    let export = export(&[definitions], &ruleset);
    assert!(export.is_complete(), "{:?}", export.not_exported);
    let again = openbim_ids::from_str(&export.to_xml().unwrap().unwrap()).unwrap();
    assert_eq!(again.info, ids.info);
    assert_eq!(again.specifications, ids.specifications);
}

#[test]
fn an_edited_or_prefiltered_translation_is_not_written_back() {
    let text = r#"<ids xmlns="http://standards.buildingsmart.org/IDS"><info><title>T</title></info><specifications><specification name="Rated" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName><value><simpleValue>EI 90</simpleValue></value></property></requirements></specification></specifications></ids>"#;
    let ids = openbim_ids::from_str(text).unwrap();
    let options = Options::new("ids:t", "1.0.0");
    let translation = translate(&ids, &options).unwrap();
    let mut edited = translation.ruleset.clone();
    edited.root.folders[0].rules[0].parameters.insert(
        "values".to_owned(),
        serde_json::from_value(strings(&["EI 60"])).unwrap(),
    );
    let export_of = |ruleset: &RuleSetPackage, definitions: &DefinitionPackage| {
        export(std::slice::from_ref(definitions), ruleset)
    };
    let refused = export_of(&edited, &translation.definitions);
    assert!(refused.specifications.is_empty());
    assert!(refused.to_xml().unwrap().is_none());
    let Refusal::Origin { specification, why } = &refused.not_exported[0].reason else {
        panic!("{:?}", refused.not_exported);
    };
    assert_eq!(specification, "Rated");
    assert!(why.contains("parameters.values"), "{why}");
    let filter: Selector = serde_json::from_value(
        json!({"kind": "entityType", "objectType": "IfcWall", "includeSubtypes": false}),
    )
    .unwrap();
    let filtered = translate(&ids, &options.clone().with_filter(filter)).unwrap();
    let refused = export_of(&filtered.ruleset, &filtered.definitions);
    assert_eq!(refused.not_exported.len(), 1);
    assert!(matches!(
        refused.not_exported[0].reason,
        Refusal::Origin { .. }
    ));
    // Without its origin, the edited rule is read as a specification of its
    // own.
    edited.root.folders[0].annotations.clear();
    let own = export_of(&edited, &translation.definitions);
    assert!(own.is_complete(), "{:?}", own.not_exported);
    let Some(requirements) = &own.specifications[0].specification.requirements else {
        panic!("a requirement");
    };
    let Facet::Property(property) = &requirements.facets[0].facet else {
        panic!("a property facet");
    };
    assert_eq!(property.value, Some(Value::Simple("EI 60".to_owned())));
}

#[test]
fn rules_ids_cannot_state_are_listed_with_why() {
    let mut subtypes = walls();
    subtypes["includeSubtypes"] = json!(true);
    let slabs = json!({ "kind": "entityType", "objectType": "t:slab", "includeSubtypes": false });
    let required = || json!({ "property": reference("t:wall-common", "t:fire-rating") });
    let mut warning = rule("warning", "property-required", walls(), required());
    warning["severity"] = json!("warning");
    let mut disabled = rule("disabled", "property-required", walls(), required());
    disabled["enabled"] = json!(false);
    let mut auxiliary = rule("auxiliary", "property-required", walls(), required());
    auxiliary["auxiliary"] = json!(true);
    let mut gated = rule("gated", "property-required", walls(), required());
    gated["gate"] = json!({"rule": "warning", "condition": "allIfPassed"});
    let rules = vec![
        rule("subtypes", "property-required", subtypes, required()),
        rule("ifc4-only", "property-required", slabs, required()),
        warning,
        disabled,
        auxiliary,
        gated,
        // IDS states measures in SI units; this rule reads the model's.
        rule(
            "model-units",
            "property-value",
            walls(),
            json!({
                "property": reference("t:wall-common", "t:fire-rating"),
                "values": strings(&["EI 90"]),
                "quantifier": string("any"),
            }),
        ),
        rule(
            "any-object",
            "property-required",
            json!({ "kind": "all" }),
            required(),
        ),
    ];
    let export = export(&[definitions()], &ruleset(rules));
    assert!(export.specifications.is_empty());
    let reasons = reasons(&export);
    let shown: BTreeMap<&str, String> = reasons
        .iter()
        .map(|(rule, reason)| (*rule, reason.to_string()))
        .collect();
    assert!(shown["subtypes"].contains("with its subtypes"), "{shown:?}");
    assert!(
        matches!(reasons["ifc4-only"], Refusal::Concept(why) if why.contains(IFC2X3_TYPE_SYSTEM)),
        "{shown:?}"
    );
    assert_eq!(reasons["warning"], &Refusal::Severity("warning".to_owned()));
    assert_eq!(reasons["disabled"], &Refusal::Disabled);
    assert_eq!(reasons["auxiliary"], &Refusal::Auxiliary);
    assert_eq!(reasons["gated"], &Refusal::Gated);
    assert!(
        matches!(reasons["model-units"], Refusal::Differs(path) if path.contains("si_units")),
        "{shown:?}"
    );
    assert!(
        matches!(reasons["any-object"], Refusal::Selector(_)),
        "{shown:?}"
    );
}

#[test]
fn the_writer_keeps_every_character() {
    let text = "<ids xmlns=\"http://standards.buildingsmart.org/IDS\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><info><title> a &lt;b&gt; &amp; c </title></info><specifications><specification name=\"x&quot;y&#10;z&#9;w&#13;\" ifcVersion=\"IFC4\"><applicability minOccurs=\"0\" maxOccurs=\"unbounded\"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><attribute><name><simpleValue>Name</simpleValue></name><value><simpleValue>  two\n lines &amp; \"quotes\" </simpleValue></value></attribute></requirements></specification></specifications></ids>";
    let ids = openbim_ids::from_str(text).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    let export = export(
        std::slice::from_ref(&translation.definitions),
        &translation.ruleset,
    );
    let again = openbim_ids::from_str(&export.to_xml().unwrap().unwrap()).unwrap();
    assert_eq!(again.info, ids.info);
    assert_eq!(again.specifications, ids.specifications);
    assert_eq!(again.specifications[0].name, "x\"y\nz\tw\r");
}

#[test]
fn the_ids_profile_refuses_every_verdict_changing_edit() {
    let text = r#"<ids xmlns="http://standards.buildingsmart.org/IDS"><info><title>T</title></info><specifications><specification name="Rated" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName><value><simpleValue>EI 90</simpleValue></value></property></requirements></specification></specifications></ids>"#;
    let ids = openbim_ids::from_str(text).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    let definitions = std::slice::from_ref(&translation.definitions);
    let profile: &dyn ExportProfile = &IdsProfile;
    assert_eq!(profile.id(), "ids");

    // Unedited, the profile holds every rule and loses nothing.
    let outcome = profile.export(definitions, &translation.ruleset);
    assert!(outcome.is_complete(), "{:?}", outcome.losses);
    let rule = translation.ruleset.root.folders[0].rules[0].id.clone();
    assert_eq!(outcome.exported, vec![rule.clone()]);
    assert_eq!(outcome.contents.as_deref(), Some("1 specification(s)"));
    assert_eq!(
        outcome.artifact,
        export(definitions, &translation.ruleset)
            .to_xml()
            .unwrap()
            .map(String::into_bytes)
    );

    // A changed value decides differently: refused, never degraded.
    let mut edited = translation.ruleset.clone();
    edited.root.folders[0].rules[0].parameters.insert(
        "values".to_owned(),
        serde_json::from_value(strings(&["EI 60"])).unwrap(),
    );
    let outcome = profile.export(definitions, &edited);
    assert!(outcome.artifact.is_none());
    assert!(outcome.exported.is_empty());
    assert_eq!(outcome.degraded().count(), 0);
    let [loss] = outcome.losses.as_slice() else {
        panic!("{:?}", outcome.losses);
    };
    assert_eq!(loss.kind, LossKind::Refused);
    assert_eq!(loss.path, rule);
    assert!(loss.reason.contains("parameters.values"), "{}", loss.reason);
}

/// A document of one specification translated to one rule.
const RATED: &str = r#"<ids xmlns="http://standards.buildingsmart.org/IDS"><info><title>T</title></info><specifications><specification name="Rated" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName><value><xs:restriction xmlns:xs="http://www.w3.org/2001/XMLSchema" base="xs:string"><xs:enumeration value="EI 90"/><xs:enumeration value="EI 120"/></xs:restriction></value></property></requirements></specification></specifications></ids>"#;

#[test]
fn annotations_written_by_earlier_releases_still_export() {
    let ids = openbim_ids::from_str(RATED).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    assert!(translation.is_complete());
    let annotation = &translation.ruleset.root.folders[0].annotations[SPECIFICATION_ANNOTATION];
    // Today's form: the element as the IDS 1.0 writer writes it.
    assert!(
        annotation.starts_with("<specification name=\"Rated\""),
        "{annotation}"
    );
    assert!(annotation.ends_with("</specification>"), "{annotation}");
    // The single-line form earlier releases wrote, `xs` undeclared, reads
    // as well.
    let mut earlier = translation.ruleset.clone();
    earlier.root.folders[0].annotations.insert(
        SPECIFICATION_ANNOTATION.to_owned(),
        r#"<specification name="Rated" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property cardinality="required"><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName><value><xs:restriction base="xs:string"><xs:enumeration value="EI 90"/><xs:enumeration value="EI 120"/></xs:restriction></value></property></requirements></specification>"#.to_owned(),
    );
    for ruleset in [&translation.ruleset, &earlier] {
        let export = export(std::slice::from_ref(&translation.definitions), ruleset);
        assert!(export.is_complete(), "{:?}", export.not_exported);
        let again = openbim_ids::from_str(&export.to_xml().unwrap().unwrap()).unwrap();
        assert_eq!(again.specifications, ids.specifications);
    }
}

#[test]
fn an_info_ids_cannot_write_refuses_the_document_where_it_fails() {
    let ids = openbim_ids::from_str(RATED).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    let mut ruleset = translation.ruleset.clone();
    ruleset.root.annotations.insert(
        format!("{}date", axioval_ids::INFO_ANNOTATION),
        "yesterday".to_owned(),
    );
    let definitions = std::slice::from_ref(&translation.definitions);
    let Err(DocumentError::Write(error)) = export(definitions, &ruleset).to_xml() else {
        panic!("the writer refuses the date");
    };
    assert_eq!(error.location(), "info/date");
    // The profile writes no document and refuses every rule, saying where.
    let outcome = IdsProfile.export(definitions, &ruleset);
    assert!(outcome.artifact.is_none());
    assert!(outcome.exported.is_empty());
    let rule = &ruleset.root.folders[0].rules[0].id;
    assert_eq!(outcome.losses.len(), 1);
    assert_eq!(&outcome.losses[0].path, rule);
    assert_eq!(outcome.losses[0].kind, LossKind::Refused);
    assert!(
        outcome.losses[0].reason.contains("info/date"),
        "{}",
        outcome.losses[0].reason
    );
}

#[test]
fn a_specification_ids_cannot_write_back_is_refused_where_it_fails() {
    let text = r#"<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema"><info><title>T</title></info><specifications><specification name="Loose" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName></property><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>AcousticRating</simpleValue></baseName><value><xs:restriction base="xs:string"/></value></property></requirements></specification></specifications></ids>"#;
    let ids = openbim_ids::from_str(text).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    // The first requirement translates; the empty restriction is a gap and
    // one the IDS 1.0 writer refuses, so the folder keeps why.
    let folder = &translation.ruleset.root.folders[0];
    assert!(!folder.rules.is_empty());
    assert!(!folder.annotations.contains_key(SPECIFICATION_ANNOTATION));
    assert_eq!(
        folder.annotations[UNWRITABLE_ANNOTATION]
            .split_once(": ")
            .unwrap()
            .0,
        "requirements/facets[1]/value/xs:restriction"
    );
    // Its rules are refused, never exported one by one.
    let export = export(
        std::slice::from_ref(&translation.definitions),
        &translation.ruleset,
    );
    assert!(export.specifications.is_empty());
    assert_eq!(export.not_exported.len(), folder.rules.len());
    for entry in &export.not_exported {
        let Refusal::Unwritable {
            specification,
            location,
            why,
        } = &entry.reason
        else {
            panic!("{entry}");
        };
        assert_eq!(specification, "Loose");
        assert_eq!(location, "requirements/facets[1]/value/xs:restriction");
        assert!(why.contains("restriction"), "{why}");
    }
}

#[test]
fn an_export_the_audit_refuses_is_never_written() {
    // A specification no exporter reading produces: a class IFC4 does not
    // define. Were an exporter bug to produce it, nothing is written.
    let mut specification =
        openbim_ids::Specification::new("Rabbits", [openbim_ids::IfcVersion::Ifc4]);
    specification
        .applicability
        .facets
        .push(openbim_ids::Entity::new("IFCRABBIT").into());
    let bad = Export {
        info: openbim_ids::Info::new("Zoo"),
        specifications: vec![axioval_ids::ExportedSpecification {
            specification,
            rules: vec!["r1".to_owned()],
        }],
        not_exported: Vec::new(),
    };
    let Err(DocumentError::Invalid(invalid)) = bad.to_xml() else {
        panic!("written");
    };
    let codes: Vec<&str> = invalid
        .errors()
        .map(|finding| finding.code.as_str())
        .collect();
    assert_eq!(codes, ["entity-unknown"]);
    // The profile writes no document either, and refuses the rule with the
    // findings.
    let outcome = axioval_export::ExportOutcome::from(bad);
    assert!(outcome.artifact.is_none());
    assert!(outcome.exported.is_empty());
    assert_eq!(outcome.losses.len(), 1);
    assert_eq!(outcome.losses[0].kind, LossKind::Refused);
    assert!(
        outcome.losses[0].reason.contains("entity-unknown"),
        "{}",
        outcome.losses[0].reason
    );
}

#[test]
fn a_rule_reading_as_an_invalid_specification_is_refused() {
    // IFCCHIMNEY is an IFC4 class. Translated for IFC4, its folder is
    // written back as it was read; read on its own, the rule would be a
    // specification for every release, IFC2X3 included, which does not
    // define the class: refused, never written.
    let text = r#"<ids xmlns="http://standards.buildingsmart.org/IDS"><info><title>T</title></info><specifications><specification name="Chimneys" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCCHIMNEY</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>P</simpleValue></propertySet><baseName><simpleValue>N</simpleValue></baseName></property></requirements></specification></specifications></ids>"#;
    let ids = openbim_ids::from_str(text).unwrap();
    let translation = translate(&ids, &Options::new("ids:t", "1.0.0")).unwrap();
    let definitions = std::slice::from_ref(&translation.definitions);
    let kept = export(definitions, &translation.ruleset);
    assert!(kept.is_complete(), "{:?}", kept.not_exported);
    let again = openbim_ids::from_str(&kept.to_xml().unwrap().unwrap()).unwrap();
    assert_eq!(again.specifications, ids.specifications);

    let mut detached = translation.ruleset.clone();
    detached.root.folders[0].annotations.clear();
    let refused = export(definitions, &detached);
    assert!(refused.specifications.is_empty());
    let Refusal::Invalid { findings, .. } = &refused.not_exported[0].reason else {
        panic!("{:?}", refused.not_exported);
    };
    assert!(
        findings.iter().any(|finding| {
            finding.code == axioval_ids::AuditCode::EntityUnknown
                && finding.ifc_version == Some(openbim_ids::IfcVersion::Ifc2x3)
        }),
        "{findings:?}"
    );
    assert!(
        refused.not_exported[0]
            .to_string()
            .contains("entity-unknown at applicability/facets[0]/name (IFC2X3)"),
        "{}",
        refused.not_exported[0]
    );
}

/// Tasks of priority 12, 123, none and -7.
const TASKS: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCTASK('0000000000000000000041',$,'T1',$,$,$,$,$,$,.F.,12,$,$);
#2=IFCTASK('0000000000000000000042',$,'T2',$,$,$,$,$,$,.F.,123,$,$);
#3=IFCTASK('0000000000000000000043',$,'T3',$,$,$,$,$,$,.F.,$,$,$);
#4=IFCTASK('0000000000000000000044',$,'T4','D',$,$,$,$,$,.F.,-7,$,$);
ENDSEC;
END-ISO-10303-21;
";

fn run_tasks(definitions: &DefinitionPackage, ruleset: &RuleSetPackage) -> Report {
    let registry = default_registry().unwrap();
    let plan = compile(&registry, std::slice::from_ref(definitions), ruleset).unwrap();
    let session = import_ifc_session("model.ifc", TASKS.as_bytes()).unwrap();
    Runtime::new(registry).run_session(&session, plan).unwrap()
}

/// The objects a report flags, sorted, with nothing left not evaluated.
fn flagged(report: &Report) -> Vec<String> {
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    let mut flagged: Vec<String> = report
        .findings()
        .iter()
        .filter_map(|finding| finding.object_id().map(|id| id.local_id.clone()))
        .collect();
    flagged.sort();
    flagged
}

/// A prohibited digit restriction (an expression rule) and one in the
/// applicability (an auxiliary expression rule).
const DIGITS: &str = r#"<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema"><info><title>Tasks</title></info><specifications>
  <specification name="Priorities" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCTASK</simpleValue></name></entity></applicability><requirements><attribute cardinality="prohibited"><name><simpleValue>Priority</simpleValue></name><value><xs:restriction base="xs:integer"><xs:minInclusive value="0"/><xs:totalDigits value="2"/></xs:restriction></value></attribute></requirements></specification>
  <specification name="Low priorities" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCTASK</simpleValue></name></entity><attribute><name><simpleValue>Priority</simpleValue></name><value><xs:restriction base="xs:integer"><xs:totalDigits value="2"/></xs:restriction></value></attribute></applicability><requirements><attribute><name><simpleValue>Description</simpleValue></name></attribute></requirements></specification>
</specifications></ids>"#;

#[test]
fn expression_rules_round_trip_through_ids() {
    let ids = openbim_ids::from_str(DIGITS).unwrap();
    let translation = translate(&ids, &Options::new("ids:tasks", "1.0.0")).unwrap();
    assert!(translation.is_complete());
    let definitions = std::slice::from_ref(&translation.definitions);
    let expression = "axioval:capability.expression";
    let capability = |rule: &axioval::ir::contract::RuleInstance| {
        translation.definitions.definitions[&rule.definition_id]
            .capability
            .clone()
    };
    let folders = &translation.ruleset.root.folders;
    assert_eq!(capability(&folders[0].rules[0]), expression);
    assert_eq!(capability(&folders[1].rules[0]), expression);
    // 12 breaks the prohibition; -7 is below 0 and 123 has three digits.
    // #1 and #4 are low priorities, and only #4 has a description.
    let original = run_tasks(&translation.definitions, &translation.ruleset);
    assert_eq!(flagged(&original), ["#1", "#1"]);

    // Exported as written, specification by specification.
    let kept = export(definitions, &translation.ruleset);
    assert!(kept.is_complete(), "{:?}", kept.not_exported);
    let xml = kept.to_xml().unwrap().unwrap();
    let again = openbim_ids::from_str(&xml).unwrap();
    assert_eq!(again.specifications, ids.specifications);
    let back = translate(&again, &Options::new("ids:tasks", "1.0.0")).unwrap();
    assert_eq!(back.definitions, translation.definitions);
    assert_eq!(back.ruleset, translation.ruleset);

    // Without its origin, the plain expression rule is read on its own;
    // the applicability's auxiliary rule and the rule reading it are not.
    let mut detached = translation.ruleset.clone();
    for folder in &mut detached.root.folders {
        folder.annotations.clear();
    }
    let alone = export(definitions, &detached);
    assert_eq!(alone.specifications.len(), 1);
    assert_eq!(alone.specifications[0].rules, ["spec1.facet1"]);
    let reasons = reasons(&alone);
    assert_eq!(reasons["spec2.applicability2"], &Refusal::Auxiliary);
    assert!(matches!(reasons["spec2.facet1"], Refusal::Selector(_)));
    let requirement = &alone.specifications[0]
        .specification
        .requirements
        .as_ref()
        .unwrap()
        .facets[0];
    assert_eq!(requirement.occurrence, Occurrence::Prohibited);
    assert_eq!(
        requirement.facet,
        ids.specifications[0].requirements.as_ref().unwrap().facets[0].facet
    );
    // Translated back, it flags what the rule did.
    let read = openbim_ids::from_str(&alone.to_xml().unwrap().unwrap()).unwrap();
    let read = translate(&read, &Options::new("ids:back", "1.0.0")).unwrap();
    assert!(read.is_complete());
    let mut only_first = translation.ruleset.clone();
    only_first.root.folders.truncate(1);
    assert_eq!(
        flagged(&run_tasks(&read.definitions, &read.ruleset)),
        flagged(&run_tasks(&translation.definitions, &only_first))
    );
}

/// The definitions with an `expression` definition, tasks, and
/// `Priority` declared an integer (`t:priority`) and as a string
/// (`t:priority-text`).
fn expression_definitions() -> DefinitionPackage {
    let mut package = serde_json::to_value(definitions()).unwrap();
    package["definitions"]["t:expression"] = definition("expression");
    package["objectTypes"]["t:task"] = concept("t:task", "IfcTask");
    let mut priority = property_concept("t:priority", "Priority");
    priority["valueKind"] = json!("integer");
    package["properties"]["t:priority"] = priority;
    package["properties"]["t:priority-text"] = property_concept("t:priority-text", "Priority");
    serde_json::from_value(package).unwrap()
}

fn tasks() -> Json {
    json!({ "kind": "entityType", "objectType": "t:task", "includeSubtypes": false })
}

fn attribute(concept: &str) -> Json {
    json!({ "kind": "property", "propertySet": "axioval:attributes", "property": concept })
}

fn integer(value: i64) -> Json {
    json!({ "kind": "literal", "value": { "type": "integer", "value": value } })
}

/// `isDefined(concept)` and `tests`, negated when `prohibited`.
fn facet(concept: &str, tests: Vec<Json>, prohibited: bool) -> Json {
    let mut operands = vec![json!({ "kind": "isDefined", "operand": attribute(concept) })];
    operands.extend(tests);
    let holds = json!({ "kind": "and", "operands": operands });
    if prohibited {
        json!({ "kind": "not", "operand": holds })
    } else {
        holds
    }
}

fn digits(concept: &str, largest: i64) -> Json {
    json!({ "kind": "between", "operand": attribute(concept), "low": integer(-largest), "high": integer(largest) })
}

fn expression_rule(id: &str, requirement: Json) -> Json {
    let mut parameters = json!({ "requirement": { "type": "expression" } });
    parameters["requirement"]["value"] = requirement;
    rule(id, "expression", tasks(), parameters)
}

#[test]
fn an_authored_expression_rule_ids_states_is_exported() {
    let definitions = expression_definitions();
    let mut labelled = expression_rule(
        "with-message",
        facet("t:priority", vec![digits("t:priority", 99)], true),
    );
    labelled["parameters"]["message"] = string("too low a priority");
    let package = ruleset(vec![
        expression_rule(
            "prohibited",
            facet(
                "t:priority",
                vec![
                    json!({ "kind": "compare", "operator": "greaterThanOrEquals", "left": attribute("t:priority"), "right": integer(0) }),
                    digits("t:priority", 99),
                ],
                true,
            ),
        ),
        // A message is presentation.
        labelled,
    ]);
    let export = export(std::slice::from_ref(&definitions), &package);
    assert!(export.is_complete(), "{:?}", export.not_exported);
    let xml = export.to_xml().unwrap().unwrap();
    assert!(xml.contains("<xs:totalDigits value=\"2\"/>"), "{xml}");
    let read = translate(
        &openbim_ids::from_str(&xml).unwrap(),
        &Options::new("ids:back", "1.0.0"),
    )
    .unwrap();
    assert!(read.is_complete());
    let original = run_tasks(&definitions, &package);
    assert_eq!(flagged(&original), ["#1", "#1", "#4"]);
    assert_eq!(
        flagged(&run_tasks(&read.definitions, &read.ruleset)),
        flagged(&original)
    );
}

/// Expression rules no IDS facet states, each by its id.
fn unstated_expression_rules() -> Vec<Json> {
    let mut graded = expression_rule(
        "graded",
        facet("t:priority", vec![digits("t:priority", 99)], true),
    );
    graded["parameters"]["deviation"] = json!({ "type": "expression", "value": integer(1) });
    vec![
        expression_rule(
            "aggregate",
            facet(
                "t:priority",
                vec![json!({ "kind": "aggregate", "function": "any",
                    "over": { "kind": "path", "path": ["IfcRelAggregates:forward"] },
                    "value": { "kind": "isDefined", "operand": attribute("t:priority") } })],
                false,
            ),
        ),
        expression_rule(
            "arithmetic",
            facet(
                "t:priority",
                vec![json!({ "kind": "compare", "operator": "lessThanOrEquals",
                    "left": { "kind": "multiply", "left": attribute("t:priority"), "right": integer(2) },
                    "right": integer(30) })],
                true,
            ),
        ),
        expression_rule(
            "measured",
            json!({ "kind": "and", "operands": [{ "kind": "isDefined", "operand":
                { "kind": "property", "propertySet": "axioval:measured", "property": "extent_z" } }] }),
        ),
        expression_rule(
            "text",
            facet("t:priority-text", vec![digits("t:priority-text", 99)], true),
        ),
        expression_rule(
            "pattern",
            facet(
                "t:priority",
                vec![
                    json!({ "kind": "compare", "operator": "like", "left": attribute("t:priority"), "right": integer(1) }),
                ],
                true,
            ),
        ),
        expression_rule(
            "not-digits",
            facet("t:priority", vec![digits("t:priority", 50)], true),
        ),
        expression_rule(
            "comparison",
            json!({ "kind": "compare", "operator": "greaterThan", "left": attribute("t:priority"), "right": integer(1) }),
        ),
        graded,
        // Equivalent, but IDS translates a required facet to
        // `property-value`, so the export cannot show it.
        expression_rule(
            "required",
            facet("t:priority", vec![digits("t:priority", 99)], false),
        ),
    ]
}

#[test]
fn expression_rules_ids_cannot_state_are_refused_naming_the_node() {
    let definitions = expression_definitions();
    let export = export(
        std::slice::from_ref(&definitions),
        &ruleset(unstated_expression_rules()),
    );
    assert!(export.specifications.is_empty());
    let reasons = reasons(&export);
    let node = |rule: &str| match reasons[rule] {
        Refusal::Expression { path, kind, .. } => (path.as_str(), kind.as_str()),
        other => panic!("{rule}: {other}"),
    };
    assert_eq!(node("aggregate"), ("requirement.and[1]", "aggregate"));
    assert_eq!(
        reasons["aggregate"].to_string(),
        "expression node `requirement.and[1]` (`aggregate`) has no IDS facet: IDS states no such computation"
    );
    assert_eq!(
        node("arithmetic"),
        ("requirement.not.operand.and[1].compare.left", "multiply")
    );
    assert_eq!(
        node("measured"),
        ("requirement.and[0].isDefined.operand", "property")
    );
    assert!(reasons["measured"].to_string().contains("axioval:measured"));
    assert_eq!(
        node("text"),
        (
            "requirement.not.operand.and[0].isDefined.operand",
            "property"
        )
    );
    assert!(
        reasons["text"]
            .to_string()
            .contains("not declared an integer")
    );
    assert_eq!(
        node("pattern"),
        ("requirement.not.operand.and[1]", "compare")
    );
    assert_eq!(
        node("not-digits"),
        ("requirement.not.operand.and[1]", "between")
    );
    assert_eq!(node("comparison"), ("requirement", "compare"));
    assert!(matches!(reasons["graded"], Refusal::Parameter(why) if why.contains("deviation")));
    assert!(
        matches!(reasons["required"], Refusal::Differs(_)),
        "{}",
        reasons["required"]
    );
}

#[test]
fn the_ids_profile_declares_its_expression_nodes_and_refuses_naming_the_node() {
    let profile = IdsProfile;
    assert!(profile.expression_kinds().contains(&"between"));
    assert!(!profile.expression_kinds().contains(&"aggregate"));
    let outcome = profile.export(
        std::slice::from_ref(&expression_definitions()),
        &ruleset(vec![expression_rule(
            "aggregate",
            facet(
                "t:priority",
                vec![json!({ "kind": "aggregate", "function": "count",
                    "over": { "kind": "path", "path": ["IfcRelAggregates:forward"] } })],
                false,
            ),
        )]),
    );
    let loss = &outcome.losses[0];
    assert_eq!(loss.kind, LossKind::Refused);
    assert_eq!(loss.path, "aggregate");
    assert!(
        loss.reason
            .starts_with("expression node `requirement.and[1]` (`aggregate`)"),
        "{loss}"
    );
}

#[test]
fn an_expression_selecting_the_objects_is_refused_naming_the_node() {
    let definitions = expression_definitions();
    let selected = json!({ "kind": "allOf", "operands": [tasks(), { "kind": "expression",
        "expression": { "kind": "compare", "operator": "greaterThan",
            "left": attribute("t:priority"), "right": integer(1) } }] });
    let mut parameters = json!({ "requirement": { "type": "expression" } });
    parameters["requirement"]["value"] = facet("t:priority", vec![digits("t:priority", 99)], true);
    let export = export(
        std::slice::from_ref(&definitions),
        &ruleset(vec![rule("selected", "expression", selected, parameters)]),
    );
    let reasons = reasons(&export);
    match reasons["selected"] {
        Refusal::Expression { path, kind, .. } => {
            assert_eq!(
                (path.as_str(), kind.as_str()),
                ("selector.expression", "compare")
            );
        }
        other => panic!("{other}"),
    }
}
