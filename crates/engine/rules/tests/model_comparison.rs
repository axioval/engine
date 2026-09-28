//! `model-comparison`: two models of one run compared as a rule.
//!
//! The base and revised models are two sources, named by their disciplines.
//! Re-exports renumber every object and may regenerate every identity, so
//! the doors below are matched by their door number.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityEvaluation, SourceDisciplines};
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{Discipline, ExternalId, NotEvaluatedReason, ObjectId, PropertyValue, SourceId};
use axioval_rules::CompareModels;
use common::{Model, boolean, kind, property, rule, string, strings, unevaluated};

const ID: &str = "axioval:capability.model-comparison";

fn document(name: &str) -> SourceId {
    SourceId::new("test", name).unwrap()
}

fn in_base(local: &str) -> ObjectId {
    ObjectId::new(document("base"), local).unwrap()
}

fn in_revised(local: &str) -> ObjectId {
    ObjectId::new(document("revised"), local).unwrap()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::String(value.into())
}

fn disciplines() -> SourceDisciplines {
    SourceDisciplines::new([
        (document("base"), Discipline::new("base").unwrap()),
        (document("revised"), Discipline::new("revised").unwrap()),
    ])
}

/// Doors `D1`, `D2` and `D3` in the base; `D1` (its fire rating changed),
/// `D2` and `D4` in the revision, every one renumbered. Each model has a
/// wall, which the rules below never select.
fn doors() -> Model {
    let door = |model: Model, id: ObjectId, number: &str, rating: &str| {
        model
            .value_of(id.clone(), "Pset_DoorCommon", "Reference", text(number))
            .value_of(id, "Pset_DoorCommon", "FireRating", text(rating))
    };
    let model = Model::default()
        .object_in("base", "#10", "door")
        .object_in("base", "#11", "door")
        .object_in("base", "#12", "door")
        .object_in("base", "#13", "wall")
        .object_in("revised", "#90", "door")
        .object_in("revised", "#91", "door")
        .object_in("revised", "#92", "door")
        .object_in("revised", "#93", "wall");
    let model = door(model, in_base("#10"), "D1", "EI30");
    let model = door(model, in_base("#11"), "D2", "EI30");
    let model = door(model, in_base("#12"), "D3", "EI30");
    let model = door(model, in_revised("#90"), "D1", "EI60");
    let model = door(model, in_revised("#91"), "D2", "EI30");
    door(model, in_revised("#92"), "D4", "EI30")
}

fn compare(
    model: Model,
    selector: Selector,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = parameters;
    parameters.extend([("base", string("base")), ("revised", string("revised"))]);
    model.evaluate_with(
        &CompareModels,
        &rule(ID, selector, parameters),
        |services| {
            services.register(disciplines()).unwrap();
        },
    )
}

/// `(subject, message)` of every finding, sorted.
fn found(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    let mut found = common::findings(evaluation);
    found.sort();
    found
}

fn by_door_number() -> (&'static str, ParameterValue) {
    (
        "identity_property",
        property(Some("Pset_DoorCommon"), "Reference"),
    )
}

#[test]
fn doors_matched_by_number_report_a_changed_property_of_a_set_compared_whole() {
    let evaluation = compare(
        doors(),
        kind("door"),
        vec![by_door_number(), ("all_property_sets", boolean(true))],
    );
    assert_eq!(
        found(&evaluation),
        vec![
            (
                "#12".into(),
                "removed (property Pset_DoorCommon.Reference:D3)".into()
            ),
            (
                "#90".into(),
                "property changed: property Pset_DoorCommon.FireRating \"EI30\" -> \"EI60\"".into()
            ),
            (
                "#92".into(),
                "added (property Pset_DoorCommon.Reference:D4)".into()
            ),
        ]
    );
    let changed = evaluation
        .findings()
        .iter()
        .find(|finding| finding.message.starts_with("property changed"))
        .unwrap();
    assert_eq!(changed.related, vec![in_base("#10")], "names the base door");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_named_set_is_compared_whole_and_a_named_property_alone() {
    let rows = |column: &str, values: &[&str]| ParameterValue::Table {
        value: values
            .iter()
            .map(|value| TableRow::from([(column.to_owned(), string(value))]))
            .collect(),
    };
    let evaluation = compare(
        doors(),
        kind("door"),
        vec![
            by_door_number(),
            ("property_sets", rows("property_set", &["Pset_DoorCommon"])),
        ],
    );
    assert!(
        found(&evaluation)
            .iter()
            .any(|(door, message)| door == "#90" && message.contains("FireRating")),
        "{evaluation:?}"
    );
    let evaluation = compare(
        doors(),
        kind("door"),
        vec![
            by_door_number(),
            ("properties", rows("property", &["FireRating"])),
        ],
    );
    assert!(
        found(&evaluation)
            .iter()
            .any(|(door, message)| door == "#90" && message.contains("FireRating")),
        "{evaluation:?}"
    );
}

#[test]
fn the_revised_model_may_state_its_identity_in_another_property() {
    let model = Model::default()
        .object_in("base", "#1", "door")
        .object_in("revised", "#2", "door")
        .value_of(in_base("#1"), "Pset_DoorCommon", "Reference", text("D1"))
        .value_of(in_revised("#2"), "Custom", "DoorNumber", text("D1"));
    let evaluation = compare(
        model,
        kind("door"),
        vec![
            by_door_number(),
            (
                "revised_identity_property",
                property(Some("Custom"), "DoorNumber"),
            ),
        ],
    );
    assert!(found(&evaluation).is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_scheme_matches_first_and_the_property_matches_what_it_leaves() {
    // D1 keeps its identity; D2's was regenerated, so only its number
    // matches it.
    let model = doors()
        .object_in("base", "#20", "door")
        .object_in("revised", "#80", "door");
    let evaluation = {
        let guid = |id: ObjectId, value: &str| (id, ExternalId::new("guid", value).unwrap());
        let ids = [
            guid(in_base("#10"), "G1"),
            guid(in_revised("#90"), "G1"),
            guid(in_base("#11"), "G2"),
            guid(in_revised("#91"), "G2-regenerated"),
        ];
        compare(
            model.with_external_ids(&ids),
            kind("door"),
            vec![
                ("identity_scheme", string("guid")),
                by_door_number(),
                ("match_by", strings(&["identity", "property"])),
                ("all_property_sets", boolean(true)),
            ],
        )
    };
    let found = found(&evaluation);
    assert!(
        found
            .iter()
            .any(|(door, message)| door == "#90" && message.starts_with("property changed")),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|(door, _)| door == "#91" || door == "#11"),
        "D2 is matched by its number: {found:?}"
    );
    // #20 and #80 carry neither identity nor number: nothing can match them.
    let mut unmatched: Vec<String> = unevaluated(&evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    unmatched.sort();
    assert_eq!(unmatched, vec!["#20".to_owned(), "#80".to_owned()]);
}

#[test]
fn an_unreadable_number_leaves_its_door_and_every_door_it_may_match_undecided() {
    let evaluation = compare(
        doors().unreadable_object(in_base("#12")),
        kind("door"),
        vec![by_door_number()],
    );
    // D3 cannot be read: the revised D4 may be it, so is not reported
    // added. D1 and D2 are matched either way.
    let mut undecided = unevaluated(&evaluation);
    undecided.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        undecided,
        vec![
            ("#12".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("#92".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    assert!(
        !found(&evaluation)
            .iter()
            .any(|(door, _)| door == "#92" || door == "#12"),
        "{evaluation:?}"
    );
}

#[test]
fn a_number_held_twice_matches_nothing() {
    let model = doors().value_of(
        in_revised("#92"),
        "Pset_DoorCommon",
        "Reference",
        text("D2"),
    );
    let evaluation = compare(model, kind("door"), vec![by_door_number()]);
    let mut ambiguous: Vec<String> = unevaluated(&evaluation)
        .into_iter()
        .filter(|(_, reason)| *reason == NotEvaluatedReason::InvalidEvidence)
        .map(|(object, _)| object)
        .collect();
    ambiguous.sort();
    assert_eq!(ambiguous, vec!["#11", "#91", "#92"]);
    assert!(
        !found(&evaluation)
            .iter()
            .any(|(door, _)| ["#11", "#91", "#92"].contains(&door.as_str())),
        "{evaluation:?}"
    );
}

#[test]
fn a_change_the_selector_cannot_place_is_not_evaluated() {
    // Without a type hierarchy, whether a wall is a door is undecided; the
    // walls, unmatched by number, would be removed and added.
    let evaluation = compare(
        doors(),
        Selector::EntityType {
            object_type: "door".into(),
            include_subtypes: true,
        },
        vec![by_door_number()],
    );
    let found = found(&evaluation);
    assert!(
        !found
            .iter()
            .any(|(object, _)| object == "#13" || object == "#93"),
        "{found:?}"
    );
    // The walls have no number either: unidentified, never reported.
    let walls: Vec<_> = unevaluated(&evaluation)
        .into_iter()
        .filter(|(object, _)| object == "#13" || object == "#93")
        .collect();
    assert_eq!(walls.len(), 2, "{evaluation:?}");
}

#[test]
fn the_two_models_must_be_named_by_one_discipline_each() {
    let run = |disciplines: SourceDisciplines, base: &str| {
        doors().evaluate_with(
            &CompareModels,
            &rule(
                ID,
                kind("door"),
                vec![
                    ("base", string(base)),
                    ("revised", string("revised")),
                    by_door_number(),
                ],
            ),
            |services| services.register(disciplines).unwrap(),
        )
    };
    let only = |evaluation: &CapabilityEvaluation| {
        assert!(evaluation.findings().is_empty());
        unevaluated(evaluation)
    };
    assert_eq!(
        only(&run(disciplines(), "architecture")),
        vec![("-".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let both = SourceDisciplines::new([
        (document("base"), Discipline::new("base").unwrap()),
        (document("revised"), Discipline::new("base").unwrap()),
    ]);
    assert_eq!(
        only(&run(both, "base")),
        vec![("-".to_owned(), NotEvaluatedReason::InvalidEvidence)],
        "two models declare `base`"
    );
    let one = SourceDisciplines::new([(document("revised"), Discipline::new("revised").unwrap())]);
    assert_eq!(
        only(&run(one, "base")),
        vec![("-".to_owned(), NotEvaluatedReason::NotRecorded)],
        "the model declaring nothing may be the base"
    );
    assert_eq!(
        only(&run(disciplines(), "revised")),
        vec![("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
        "a model is not compared with itself"
    );
}

#[test]
fn a_declaration_without_a_matcher_is_invalid() {
    let evaluation = compare(doors(), kind("door"), vec![]);
    assert_eq!(
        unevaluated(&evaluation),
        vec![("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let evaluation = compare(
        doors(),
        kind("door"),
        vec![by_door_number(), ("match_by", strings(&["identity"]))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        vec![("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
