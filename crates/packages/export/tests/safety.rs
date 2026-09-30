//! The safety rule: a difference that changes a verdict is refused, never
//! degraded; presentation alone is no difference at all.

use axioval_export::compare::{
    Catalog, Comparison, difference_loss, first_difference, verdict_difference,
};
use axioval_export::precheck::{PreCheck, pre_check};
use axioval_export::{ExportOutcome, ExportProfile, Loss, LossKind};
use axioval_ir::contract::{DefinitionPackage, RuleInstance, RuleSetPackage, Severity};
use serde_json::json;

/// A definition package whose object type `wall_id` is named `wall_name`,
/// and one definition of a counting capability with a defaulted minimum.
fn definitions(wall_id: &str, wall_name: &str) -> DefinitionPackage {
    serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "package": {
            "id": "t", "name": {"default": "T", "translations": {}},
            "version": "1.0.0", "description": null, "repository": null, "license": null,
        },
        "objectTypes": {
            wall_id: {
                "id": wall_id, "name": {"default": "Wall", "translations": {}},
                "description": null,
                "externalNames": [{"typeSystem": "example", "name": wall_name}],
            },
        },
        "definitions": {
            "t:count": {
                "id": "t:count", "name": {"default": "Count", "translations": {}},
                "description": null, "capability": "example:count",
                "parameters": {
                    "minimum": {
                        "id": "minimum", "name": {"default": "Minimum", "translations": {}},
                        "description": null, "kind": "integer",
                        "referencedValueKind": null, "required": false,
                        "defaultValue": {"type": "integer", "value": 1},
                        "unitDimension": null,
                    },
                    "message": {
                        "id": "message", "name": {"default": "Message", "translations": {}},
                        "description": null, "kind": "string",
                        "referencedValueKind": null, "required": false,
                        "defaultValue": null, "unitDimension": null,
                    },
                },
            },
        },
    }))
    .unwrap()
}

fn rule(value: &serde_json::Value) -> RuleInstance {
    let mut rule = json!({
        "id": "r1",
        "definitionId": "t:count",
        "name": {"default": "At least one wall", "translations": {}},
        "applicability": {"kind": "entityType", "objectType": "t:wall"},
    });
    for (key, field) in value.as_object().unwrap() {
        rule[key] = field.clone();
    }
    serde_json::from_value(rule).unwrap()
}

fn difference(ours: &DefinitionPackage, a: &RuleInstance, b: &RuleInstance) -> Option<String> {
    let ours = std::slice::from_ref(ours);
    let catalog = Catalog::new(ours);
    let comparison = Comparison::default().presentation_parameter("example:count", "message");
    verdict_difference(&catalog, &[a], &catalog, &[b], &comparison).unwrap()
}

#[test]
fn a_difference_that_changes_a_verdict_is_refused_never_degraded() {
    let package = definitions("t:wall", "Wall");
    let original = rule(&json!({}));
    for changed in [
        rule(&json!({"parameters": {"minimum": {"type": "integer", "value": 2}}})),
        rule(&json!({"applicability": {"kind": "all"}})),
        rule(&json!({"severity": "warning"})),
        rule(&json!({"enabled": false})),
    ] {
        let found = difference(&package, &changed, &original)
            .unwrap_or_else(|| panic!("{changed:?} decides like the original"));
        let loss = difference_loss(&changed.id, &found);
        assert_eq!(loss.kind, LossKind::Refused, "{loss}");
        assert_eq!(loss.path, "r1");
    }
}

#[test]
fn presentation_defaults_and_concept_ids_are_no_difference() {
    let package = definitions("t:wall", "Wall");
    let original = rule(&json!({}));
    // A name, a description, a presentation parameter, and a default stated.
    let presented = rule(&json!({
        "id": "renamed",
        "name": {"default": "Walls", "translations": {"de": "Wände"}},
        "description": {"default": "Every model has one", "translations": {}},
        "parameters": {
            "minimum": {"type": "integer", "value": 1},
            "message": {"type": "string", "value": "No wall"},
        },
    }));
    assert_eq!(difference(&package, &presented, &original), None);

    // A concept compared by the names it binds to, not by its id.
    let theirs = [definitions("x:w", "Wall")];
    let ours = [package];
    let other = rule(&json!({"applicability": {"kind": "entityType", "objectType": "x:w"}}));
    let comparison = Comparison::default();
    let found = verdict_difference(
        &Catalog::new(&ours),
        &[&original],
        &Catalog::new(&theirs),
        &[&other],
        &comparison,
    )
    .unwrap();
    assert_eq!(found, None);

    // Named in another case, it differs unless the comparison ignores case.
    let theirs = [definitions("x:w", "WALL")];
    let differs = |comparison: &Comparison| {
        verdict_difference(
            &Catalog::new(&ours),
            &[&original],
            &Catalog::new(&theirs),
            &[&other],
            comparison,
        )
        .unwrap()
    };
    assert_eq!(
        differs(&comparison).as_deref(),
        Some("rules[0].applicability.objectType")
    );
    assert_eq!(
        differs(&Comparison::default().case_insensitive_object_types()),
        None
    );
}

#[test]
fn the_first_difference_is_a_path() {
    let a = json!({"x": [1, {"y": 2}], "z": null});
    assert_eq!(
        first_difference(&a, &json!({"x": [1, {"y": 2}]}), "r"),
        None
    );
    assert_eq!(
        first_difference(&a, &json!({"x": [1, {"y": 3}]}), "r").as_deref(),
        Some("r.x[1].y")
    );
    assert_eq!(
        first_difference(&a, &json!({"x": [1]}), "r").as_deref(),
        Some("r.x (2 against 1 as translated)")
    );
}

#[test]
fn a_rule_is_plain_or_names_the_first_reason_it_is_not() {
    let plain = rule(&json!({}));
    assert!(pre_check(&plain, false, &Severity::Error).is_ok());
    assert_eq!(
        pre_check(&plain, true, &Severity::Error),
        Err(PreCheck::Gated)
    );
    let warning = rule(&json!({"severity": "warning", "enabled": false}));
    assert_eq!(
        pre_check(&warning, false, &Severity::Error),
        Err(PreCheck::Disabled)
    );
    let warning = rule(&json!({"severity": "warning"}));
    assert_eq!(
        pre_check(&warning, false, &Severity::Error),
        Err(PreCheck::Severity("warning".to_owned()))
    );
}

/// A profile outside this crate implements only `export`.
struct Plain;

impl ExportProfile for Plain {
    fn id(&self) -> &'static str {
        "example"
    }

    fn export(&self, _: &[DefinitionPackage], ruleset: &RuleSetPackage) -> ExportOutcome {
        ExportOutcome {
            artifact: Some(Vec::new()),
            losses: vec![Loss::degraded(
                format!("folders/{}", ruleset.root.id),
                "folders flattened",
            )],
            ..ExportOutcome::default()
        }
    }
}

#[test]
fn optional_exports_are_refused_until_a_profile_supports_them() {
    let profile: &dyn ExportProfile = &Plain;
    assert_eq!(profile.format(), "example");
    let outcome = profile.export_takeoff(&[]);
    assert!(outcome.artifact.is_none());
    assert_eq!(
        outcome.losses,
        vec![Loss::refused("takeoff", "example export writes no takeoff")]
    );
    let outcome = profile.export_classification(&[], &ruleset());
    assert_eq!(outcome.refused().count(), 1);

    let outcome = profile.export(&[], &ruleset());
    assert!(!outcome.is_complete());
    assert_eq!(outcome.refused().count(), 0);
    assert_eq!(outcome.degraded().count(), 1);
}

fn ruleset() -> RuleSetPackage {
    serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "package": {
            "id": "t", "name": {"default": "T", "translations": {}},
            "version": "1.0.0", "description": null, "repository": null, "license": null,
        },
        "definitionPackages": ["t"],
        "root": {"id": "root", "name": {"default": "Root", "translations": {}}, "description": null},
    }))
    .unwrap()
}
