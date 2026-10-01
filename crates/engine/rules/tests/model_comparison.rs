//! `model-comparison`: two models of one run compared as a rule.
//!
//! The base and revised models are two sources, named by their disciplines.
//! Re-exports renumber every object and may regenerate every identity, so
//! the doors below are matched by their door number.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, GeometryFidelity, IntersectionVolume, LengthInterval,
    ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle, RelationshipKind, SourceDisciplines, SourceMetadata,
    SourceMetadataIndex, VolumeInterval,
};
use axioval_ir::contract::SourceField;
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{
    Discipline, Evidence, ExternalId, NotEvaluatedReason, ObjectId, PropertyValue, Scope, Severity,
    SourceId,
};
use axioval_rules::CompareModels;
use common::{Model, boolean, kind, number, property, rule, string, strings, unevaluated};

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

// ---------------------------------------------------------------------------
// Objects without stable identities, timestamps and whole sets.
// ---------------------------------------------------------------------------

/// Exact boxes per object. Between two boxes: their separation, the volume
/// they share and, standing in for the Hausdorff distance, the largest shift
/// of any face.
/// A box: its least and greatest corner.
type Box3 = ([f64; 3], [f64; 3]);

struct Boxes(BTreeMap<ObjectId, Box3>);

impl ProximityService for Boxes {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let (min, max) = self.0.get(object).ok_or(ProximityError::NoBody)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new(*min, *max)?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let (a, b) = (&self.0[request.subject()], &self.0[request.counterpart()]);
        let mut gaps = 0.0;
        let mut shared = 1.0;
        let mut depth = f64::INFINITY;
        let mut shift: f64 = 0.0;
        for axis in 0..3 {
            let gap = (a.0[axis] - b.1[axis]).max(b.0[axis] - a.1[axis]).max(0.0);
            gaps += gap * gap;
            let overlap = (a.1[axis].min(b.1[axis]) - a.0[axis].max(b.0[axis])).max(0.0);
            shared *= overlap;
            depth = depth.min(overlap);
            shift = shift
                .max((a.0[axis] - b.0[axis]).abs())
                .max((a.1[axis] - b.1[axis]).abs());
        }
        let separation = f64::sqrt(gaps);
        let volume = |(min, max): &Box3| (max[0] - min[0]) * (max[1] - min[1]) * (max[2] - min[2]);
        ProximityEvidence::try_new(
            request.clone(),
            separation,
            Some(if separation > 0.0 { 0.0 } else { depth }),
            0.0,
            None,
            GeometryFidelity::Exact,
            Evidence::exact(request.subject().source.clone(), "boxes"),
        )?
        .with_hausdorff(LengthInterval::exact(shift.max(separation)).unwrap())?
        .with_intersection_volume(IntersectionVolume::try_new(
            VolumeInterval::exact(shared)?,
            VolumeInterval::exact(volume(a))?,
            VolumeInterval::exact(volume(b))?,
        )?)
    }
}

/// A door-sized box at `x`.
fn at(x: f64) -> Box3 {
    ([x, 0.0, 0.0], [x + 1.0, 0.1, 2.1])
}

fn compare_with(
    model: Model,
    boxes: &[(ObjectId, Box3)],
    parameters: Vec<(&str, ParameterValue)>,
    stamps: Option<(&str, &str)>,
) -> CapabilityEvaluation {
    let mut parameters = parameters;
    parameters.extend([("base", string("base")), ("revised", string("revised"))]);
    let boxes = Boxes(boxes.iter().cloned().collect());
    model.evaluate_with(
        &CompareModels,
        &rule(ID, kind("door"), parameters),
        |services| {
            services.register(disciplines()).unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(boxes)))
                .unwrap();
            if let Some((base, revised)) = stamps {
                let stamp =
                    |value: &str| SourceMetadata::new().with(SourceField::Timestamp, [value]);
                services
                    .register(SourceMetadataIndex::new([
                        (document("base"), stamp(base)),
                        (document("revised"), stamp(revised)),
                    ]))
                    .unwrap();
            }
        },
    )
}

/// Every door of [`doors`] where its number says: `D1` at 0, `D2` at 2, `D3`
/// at 4 in the base, `D4` at 6 in the revision.
fn door_boxes() -> Vec<(ObjectId, Box3)> {
    vec![
        (in_base("#10"), at(0.0)),
        (in_base("#11"), at(2.0)),
        (in_base("#12"), at(4.0)),
        (in_revised("#90"), at(0.0)),
        (in_revised("#91"), at(2.0)),
        (in_revised("#92"), at(6.0)),
    ]
}

#[test]
fn two_exports_with_fresh_identities_match_by_geometry() {
    let evaluation = compare_with(
        doors(),
        &door_boxes(),
        vec![
            ("match_by", strings(&["geometry"])),
            ("all_property_sets", boolean(true)),
        ],
        None,
    );
    let found = found(&evaluation);
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(
        found[0],
        ("#12".to_owned(), "removed (geometry:#12)".to_owned())
    );
    assert!(
        found[1].0 == "#90" && found[1].1.contains("FireRating"),
        "{found:?}"
    );
    assert_eq!(
        found[2],
        ("#92".to_owned(), "added (geometry:#92)".to_owned())
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_body_coinciding_with_two_matches_neither() {
    let mut boxes = door_boxes();
    boxes.push((in_revised("#94"), at(0.0)));
    let model = doors().object_in("revised", "#94", "door");
    let evaluation = compare_with(
        model,
        &boxes,
        vec![("match_by", strings(&["geometry"]))],
        None,
    );
    let mut undecided = unevaluated(&evaluation);
    undecided.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        undecided,
        vec![
            ("#10".to_owned(), NotEvaluatedReason::InvalidEvidence),
            ("#90".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("#94".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    assert!(
        !found(&evaluation)
            .iter()
            .any(|(door, _)| ["#10", "#90", "#94"].contains(&door.as_str())),
        "{evaluation:?}"
    );
}

#[test]
fn a_moved_body_matches_by_its_overlap_with_the_larger_one() {
    // D2 moved 0.2 m: not the same geometry, but 80 % of it overlaps.
    let mut boxes = door_boxes();
    boxes[4] = (in_revised("#91"), at(2.2));
    let run = |ratio: f64| {
        compare_with(
            doors(),
            &boxes,
            vec![
                ("match_by", strings(&["geometry", "overlap"])),
                ("minimum_overlap_ratio", number(ratio)),
            ],
            None,
        )
    };
    let matched = found(&run(0.75));
    assert!(
        !matched
            .iter()
            .any(|(door, _)| door == "#91" || door == "#11"),
        "{matched:?}"
    );
    let unmatched = found(&run(0.9));
    assert!(
        unmatched
            .iter()
            .any(|(door, message)| door == "#91" && message.starts_with("added"))
    );
}

#[test]
fn a_door_matches_through_its_matched_opening() {
    // The doors changed shape; their openings did not.
    let model = doors()
        .object_in("base", "#40", "opening")
        .object_in("revised", "#80", "opening")
        .edge_of("fills", in_base("#10"), in_base("#40"))
        .edge_of("fills", in_revised("#90"), in_revised("#80"));
    let boxes = vec![
        (in_base("#10"), at(0.0)),
        (in_revised("#90"), ([0.0, 0.0, 0.0], [0.9, 0.1, 2.0])),
        (in_base("#40"), at(0.0)),
        (in_revised("#80"), at(0.0)),
    ];
    let evaluation = compare_with(
        model,
        &boxes,
        vec![
            ("match_by", strings(&["related"])),
            ("match_path", strings(&["fills:forward"])),
            ("all_property_sets", boolean(true)),
        ],
        None,
    );
    let found = found(&evaluation);
    assert!(
        found
            .iter()
            .any(|(door, message)| door == "#90" && message.contains("FireRating")),
        "{found:?}"
    );
    assert!(!found.iter().any(|(door, _)| door == "#10"), "{found:?}");
}

#[test]
fn a_revision_older_than_its_base_is_an_error_finding() {
    let run = |stamps: (&str, &str)| {
        compare_with(
            doors(),
            &door_boxes(),
            vec![by_door_number(), ("compare_timestamps", boolean(true))],
            Some(stamps),
        )
    };
    let swapped = run(("2024-05-01T10:00:00", "2024-01-01T10:00:00"));
    let older: Vec<_> = swapped
        .findings()
        .iter()
        .filter(|finding| finding.message.starts_with("timestamp changed"))
        .collect();
    assert_eq!(older.len(), 1, "{swapped:?}");
    assert_eq!(older[0].scope, Scope::Source(document("revised")));
    assert_eq!(older[0].severity, Severity::Error);

    let in_order = run(("2024-01-01T10:00:00Z", "2024-05-01T10:00:00+02:00"));
    assert!(
        !in_order
            .findings()
            .iter()
            .any(|finding| finding.message.starts_with("timestamp")),
        "{in_order:?}"
    );
    // Hours apart without offsets: either may be the earlier.
    let close = run(("2024-05-01T12:00:00", "2024-05-01T10:00:00"));
    assert!(
        close
            .not_evaluated_outcomes()
            .iter()
            .any(|outcome| outcome.message().contains("either order")),
        "{close:?}"
    );
}

#[test]
fn a_set_on_one_side_only_is_added_or_removed_whole() {
    let model = Model::default()
        .object_in("base", "#1", "door")
        .object_in("revised", "#2", "door")
        .value_of(in_base("#1"), "Pset_DoorCommon", "Reference", text("D1"))
        .value_of(in_base("#1"), "Pset_Old", "A", text("x"))
        .value_of(in_base("#1"), "Pset_Old", "B", text("y"))
        .value_of(in_revised("#2"), "Pset_DoorCommon", "Reference", text("D1"))
        .value_of(in_revised("#2"), "Pset_New", "C", text("z"));
    let evaluation = compare(
        model,
        kind("door"),
        vec![by_door_number(), ("all_property_sets", boolean(true))],
    );
    assert_eq!(
        found(&evaluation),
        vec![(
            "#2".to_owned(),
            "property changed: property set Pset_New added; property set Pset_Old removed"
                .to_owned()
        )]
    );
}

/// The base of a small building, `(local, kind, guid)`: storeys `S1` and
/// `S2`, door `D` in `S1`, walls `A` and `B` in `S1` voided by openings
/// `O1` and `O2`, window `W` filling `O1`.
const BUILDING: [(&str, &str, &str); 8] = [
    ("#1", "storey", "S1"),
    ("#2", "storey", "S2"),
    ("#10", "door", "D"),
    ("#20", "wall", "A"),
    ("#21", "wall", "B"),
    ("#30", "opening", "O1"),
    ("#31", "opening", "O2"),
    ("#40", "window", "W"),
];

use RelationshipKind::{Containment, Fills, Voids};

/// An edge of a kind, from relating to related local id.
type Edge = (RelationshipKind, String, String);

/// A revision's objects (`(local, kind, guid)`) and edges.
type Revised = (Vec<(String, &'static str, Option<&'static str>)>, Vec<Edge>);

/// The building's edges, with the door in `storey` and the window in
/// `opening`, by local id.
fn building_edges(storey: &str, opening: &str) -> Vec<Edge> {
    [
        (Containment, "#1", "#20"),
        (Containment, "#1", "#21"),
        (Containment, storey, "#10"),
        (Voids, "#20", "#30"),
        (Voids, "#21", "#31"),
        (Fills, opening, "#40"),
    ]
    .into_iter()
    .map(|(kind, relating, related)| (kind, relating.to_owned(), related.to_owned()))
    .collect()
}

/// The building's revision renumbered (`#10` becomes `#910`), its objects
/// and edges, each object with its `guid`.
fn renumbered(edges: &[Edge]) -> Revised {
    let local = |local: &str| format!("#9{}", &local[1..]);
    (
        BUILDING
            .iter()
            .map(|(id, kind, guid)| (local(id), *kind, Some(*guid)))
            .collect(),
        edges
            .iter()
            .map(|(kind, a, b)| (*kind, local(a), local(b)))
            .collect(),
    )
}

/// Both revisions in one model: the base building and the given revision,
/// answering the relationship kinds in `known`.
fn building(known: &[RelationshipKind], (objects, edges): Revised) -> Model {
    let mut model = Model::default();
    for kind in known {
        model = model.known(kind.relationship().as_str());
    }
    let mut ids = Vec::new();
    for (local, kind, guid) in BUILDING {
        model = model.object_in("base", local, kind);
        ids.push((in_base(local), ExternalId::new("guid", guid).unwrap()));
    }
    for (kind, relating, related) in building_edges("#1", "#30") {
        model = model.edge_of(
            kind.relationship().as_str(),
            in_base(&relating),
            in_base(&related),
        );
    }
    for (local, kind, guid) in &objects {
        model = model.object_in("revised", local, kind);
        if let Some(guid) = guid {
            ids.push((in_revised(local), ExternalId::new("guid", *guid).unwrap()));
        }
    }
    for (kind, relating, related) in edges {
        model = model.edge_of(
            kind.relationship().as_str(),
            in_revised(&relating),
            in_revised(&related),
        );
    }
    model.with_external_ids(&ids)
}

fn compare_relationships(model: Model) -> CapabilityEvaluation {
    compare(
        model,
        Selector::All,
        vec![
            ("identity_scheme", string("guid")),
            ("compare_relationships", boolean(true)),
        ],
    )
}

/// `(object, message)` of every not-evaluated outcome, sorted.
fn gaps(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    let mut gaps: Vec<(String, String)> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map_or_else(|| "-".to_owned(), |object| object.local_id.clone()),
                outcome.message().to_owned(),
            )
        })
        .collect();
    gaps.sort();
    gaps
}

fn pair(object: &str, message: &str) -> (String, String) {
    (object.to_owned(), message.to_owned())
}

#[test]
fn a_door_moved_to_another_storey_and_a_window_to_another_wall_change_relationships() {
    let evaluation = compare_relationships(building(
        &RelationshipKind::ALL,
        renumbered(&building_edges("#2", "#31")),
    ));
    assert_eq!(gaps(&evaluation), Vec::new());
    assert_eq!(
        found(&evaluation),
        vec![
            pair("#91", "relationship changed: containment -[D] +[]"),
            pair("#910", "relationship changed: containment -[S1] +[S2]"),
            pair("#92", "relationship changed: containment -[] +[D]"),
            pair("#930", "relationship changed: fills -[W] +[]"),
            pair("#931", "relationship changed: fills -[] +[W]"),
            pair("#940", "relationship changed: fills -[O1] +[O2]"),
        ]
    );
}

#[test]
fn an_unchanged_model_reports_no_relationship_difference() {
    let evaluation = compare_relationships(building(
        &RelationshipKind::ALL,
        renumbered(&building_edges("#1", "#30")),
    ));
    assert_eq!(found(&evaluation), Vec::new());
    assert_eq!(gaps(&evaluation), Vec::new());
}

#[test]
fn an_unmatched_related_object_is_reported_as_such_never_as_a_change() {
    // The window now fills a new opening `O3`, and `O1` is gone: the
    // revision may have regenerated the same opening, so the window's fills
    // are no change, and both openings are named unmatched.
    let (mut objects, mut edges) = renumbered(&building_edges("#1", "#30"));
    objects.retain(|(local, _, _)| local != "#930");
    objects.push(("#932".to_owned(), "opening", Some("O3")));
    edges.retain(|(_, a, b)| a != "#930" && b != "#930");
    edges.push((Fills, "#932".to_owned(), "#940".to_owned()));
    let evaluation = compare_relationships(building(&RelationshipKind::ALL, (objects, edges)));
    let found = found(&evaluation);
    assert_eq!(
        found,
        vec![
            pair("#30", "removed (guid:O1)"),
            pair("#932", "added (guid:O3)"),
        ]
    );
    // Wall `A` lost a removed opening: unmatched too, not a change.
    assert_eq!(
        gaps(&evaluation),
        vec![
            pair(
                "#920",
                "relationship voids not compared: unmatched related objects (base removed \
                 guid:O1); compared through matched objects only"
            ),
            pair(
                "#940",
                "relationship fills not compared: unmatched related objects (base removed \
                 guid:O1; revised added guid:O3); compared through matched objects only"
            ),
        ]
    );
}

#[test]
fn a_related_object_without_a_decided_match_leaves_the_kind_not_evaluated() {
    // The revised door is also contained in a storey nothing identifies.
    let (mut objects, mut edges) = renumbered(&building_edges("#1", "#30"));
    objects.push(("#99".to_owned(), "storey", None));
    edges.push((Containment, "#99".to_owned(), "#910".to_owned()));
    let evaluation = compare_relationships(building(&RelationshipKind::ALL, (objects, edges)));
    assert_eq!(found(&evaluation), Vec::new());
    let gaps = gaps(&evaluation);
    assert!(
        gaps.contains(&pair(
            "#910",
            "relationship containment not compared: undecided related objects (revised \
             test:revised/#99); compared through matched objects only"
        )),
        "{gaps:?}"
    );
}

#[test]
fn a_kind_the_source_cannot_list_is_not_evaluated_for_every_matched_object() {
    let known: Vec<RelationshipKind> = RelationshipKind::ALL
        .into_iter()
        .filter(|kind| *kind != RelationshipKind::Connection)
        .collect();
    let evaluation =
        compare_relationships(building(&known, renumbered(&building_edges("#1", "#30"))));
    assert_eq!(found(&evaluation), Vec::new());
    let gaps = gaps(&evaluation);
    assert_eq!(gaps.len(), BUILDING.len(), "{gaps:?}");
    assert!(
        gaps.iter()
            .all(|(_, message)| message.starts_with("relationship connection not compared")),
        "{gaps:?}"
    );
}
