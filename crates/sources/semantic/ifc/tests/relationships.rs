//! Production IFC relationship selection over real STEP input.
//!
//! Every test parses STEP text through `import_ifc_session` and queries the
//! service the session registers; nothing here is a mock.
#![allow(missing_docs)]

use axioval_engine::{
    EvidenceSession, RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionServiceHandle, SemanticRelationship, TraversalDirection,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

fn step(data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

/// Project > building > two storeys; storey 3 holds walls 10/11 through two
/// separate containment entities and a standard-case wall 12; storey 4 holds
/// wall 13. Wall 10 is voided by opening 20, which door 21 fills.
const BUILDING: &str = "\
#1=IFCPROJECT('p',$,'P',$,$,$,$,$,$);
#2=IFCBUILDING('b',$,'B',$,$,$,$,$,$,$,$,$);
#3=IFCBUILDINGSTOREY('s1',$,'EG',$,$,$,$,$,$,$);
#4=IFCBUILDINGSTOREY('s2',$,'OG',$,$,$,$,$,$,$);
#5=IFCRELAGGREGATES('a1',$,$,$,#1,(#2));
#6=IFCRELAGGREGATES('a2',$,$,$,#2,(#3,#4));
#10=IFCWALL('w1',$,$,$,$,$,$,$,$);
#11=IFCWALL('w2',$,$,$,$,$,$,$,$);
#12=IFCWALLSTANDARDCASE('w3',$,$,$,$,$,$,$,$);
#13=IFCWALL('w4',$,$,$,$,$,$,$,$);
#20=IFCOPENINGELEMENT('o1',$,$,$,$,$,$,$,$);
#21=IFCDOOR('d1',$,$,$,$,$,$,$,$,$,$,$,$);
#30=IFCRELCONTAINEDINSPATIALSTRUCTURE('c1',$,$,$,(#10,#12),#3);
#31=IFCRELCONTAINEDINSPATIALSTRUCTURE('c2',$,$,$,(#11),#3);
#32=IFCRELCONTAINEDINSPATIALSTRUCTURE('c3',$,$,$,(#13),#4);
#40=IFCRELVOIDSELEMENT('v1',$,$,$,#10,#20);
#41=IFCRELFILLSELEMENT('f1',$,$,$,#20,#21);
";

fn session(data: &str) -> EvidenceSession {
    import_ifc_session("model.ifc", &step(data)).unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

fn ids(locals: &[&str]) -> Vec<ObjectId> {
    let mut out: Vec<_> = locals.iter().map(|local| id(local)).collect();
    out.sort();
    out
}

fn every_object(session: &EvidenceSession) -> Vec<ObjectId> {
    session.project().objects().map(|o| o.id.clone()).collect()
}

fn related(relationship: &str, direction: TraversalDirection, chain: bool) -> RelationshipQuery {
    RelationshipQuery::Related {
        relationship: SemanticRelationship::try_new(relationship).unwrap(),
        direction,
        follow_chain: chain,
    }
}

fn select(
    session: &EvidenceSession,
    anchor: &str,
    query: RelationshipQuery,
) -> Result<axioval_engine::CompleteRelationshipSelection, RelationshipSelectionError> {
    let request =
        RelationshipSelectionRequest::try_new(id(anchor), every_object(session), query).unwrap();
    session
        .service::<RelationshipSelectionServiceHandle>()
        .expect("the IFC session registers relationship selection")
        .select(&request)
}

#[test]
fn containment_forward_selects_elements_across_split_relationship_entities() {
    let session = session(BUILDING);
    let selection = select(
        &session,
        "#3",
        related(
            "IfcRelContainedInSpatialStructure",
            TraversalDirection::Forward,
            false,
        ),
    )
    .unwrap();
    assert_eq!(
        selection.candidates(),
        ids(&["#10", "#11", "#12"]).as_slice()
    );
    // Completeness proof plus one locator per contributing relationship entity.
    let locators: Vec<_> = selection
        .evidence()
        .iter()
        .map(|e| e.locator.as_str())
        .collect();
    assert!(
        locators
            .iter()
            .any(|l| l.ends_with("relationship-scan:IfcRelContainedInSpatialStructure:3")),
        "{locators:?}"
    );
    assert!(locators.iter().any(|l| l.ends_with("relationship:#30")));
    assert!(locators.iter().any(|l| l.ends_with("relationship:#31")));
    assert!(!locators.iter().any(|l| l.ends_with("relationship:#32")));
    assert!(selection.evidence().iter().all(|e| e.exact));
}

#[test]
fn containment_backward_finds_the_storey() {
    let session = session(BUILDING);
    let selection = select(
        &session,
        "#12",
        related(
            "IfcRelContainedInSpatialStructure",
            TraversalDirection::Backward,
            false,
        ),
    )
    .unwrap();
    assert_eq!(selection.candidates(), ids(&["#3"]).as_slice());
}

#[test]
fn slot_direction_follows_the_schema_not_a_uniform_layout() {
    // Aggregation keeps its relating end in slot 4, containment in slot 5.
    // Reading both the same way would make the storey a child of its walls.
    let session = session(BUILDING);
    let forward = select(
        &session,
        "#10",
        related(
            "IfcRelContainedInSpatialStructure",
            TraversalDirection::Forward,
            false,
        ),
    )
    .unwrap();
    assert!(forward.candidates().is_empty(), "a wall contains nothing");
    let aggregates = select(
        &session,
        "#2",
        related("IfcRelAggregates", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert_eq!(aggregates.candidates(), ids(&["#3", "#4"]).as_slice());
}

#[test]
fn follow_chain_walks_the_whole_decomposition() {
    let session = session(BUILDING);
    let direct = select(
        &session,
        "#1",
        related("IfcRelAggregates", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert_eq!(direct.candidates(), ids(&["#2"]).as_slice());
    let chained = select(
        &session,
        "#1",
        related("IfcRelAggregates", TraversalDirection::Forward, true),
    )
    .unwrap();
    assert_eq!(chained.candidates(), ids(&["#2", "#3", "#4"]).as_slice());
}

#[test]
fn shared_group_is_every_co_member_of_the_same_container() {
    let session = session(BUILDING);
    let selection = select(
        &session,
        "#10",
        RelationshipQuery::SharedGroup {
            relationship: SemanticRelationship::try_new("IfcRelContainedInSpatialStructure")
                .unwrap(),
        },
    )
    .unwrap();
    // Walls on the same storey, through both containment entities; never the
    // anchor itself and never the wall on the other storey.
    assert_eq!(selection.candidates(), ids(&["#11", "#12"]).as_slice());
}

#[test]
fn voids_and_fills_link_host_opening_and_door() {
    let session = session(BUILDING);
    let opening = select(
        &session,
        "#10",
        related("IfcRelVoidsElement", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert_eq!(opening.candidates(), ids(&["#20"]).as_slice());
    let door = select(
        &session,
        "#20",
        related("IfcRelFillsElement", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert_eq!(door.candidates(), ids(&["#21"]).as_slice());
}

#[test]
fn a_supertype_request_includes_every_concrete_subtype() {
    // In IFC4 containment and fills are IfcRelConnects, but voids is an
    // IfcRelDecomposes. From the wall, IfcRelConnects reaches only the storey;
    // the opening is reached through IfcRelDecomposes, alongside aggregation.
    let session = session(BUILDING);
    let connects = select(
        &session,
        "#10",
        related("IfcRelConnects", TraversalDirection::Either, false),
    )
    .unwrap();
    assert_eq!(connects.candidates(), ids(&["#3"]).as_slice());
    let decomposes = select(
        &session,
        "#10",
        related("IfcRelDecomposes", TraversalDirection::Either, false),
    )
    .unwrap();
    assert_eq!(decomposes.candidates(), ids(&["#20"]).as_slice());
    // And from the building, IfcRelDecomposes reaches both aggregated
    // storeys. The project that aggregates it is an IfcContext, not an
    // IfcObject, so it is outside the session's objects and the universe.
    let building = select(
        &session,
        "#2",
        related("IfcRelDecomposes", TraversalDirection::Either, false),
    )
    .unwrap();
    assert_eq!(building.candidates(), ids(&["#3", "#4"]).as_slice());
}

#[test]
fn no_relationship_is_an_exact_empty_answer_with_proof() {
    let session = session(BUILDING);
    let selection = select(
        &session,
        "#13",
        related("IfcRelVoidsElement", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert!(selection.candidates().is_empty());
    assert_eq!(selection.evidence().len(), 1);
    assert!(
        selection.evidence()[0]
            .locator
            .ends_with("relationship-scan:IfcRelVoidsElement:1")
    );
}

#[test]
fn candidates_outside_the_universe_are_not_returned() {
    let session = session(BUILDING);
    let request = RelationshipSelectionRequest::try_new(
        id("#3"),
        ids(&["#11"]),
        related(
            "IfcRelContainedInSpatialStructure",
            TraversalDirection::Forward,
            false,
        ),
    )
    .unwrap();
    let selection = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap()
        .select(&request)
        .unwrap();
    assert_eq!(selection.candidates(), ids(&["#11"]).as_slice());
}

#[test]
fn a_malformed_instance_refuses_the_whole_answer() {
    // #33 names a string where the related list belongs. Skipping it would
    // make the storey look like it holds fewer elements than the file says.
    let data = format!("{BUILDING}#33=IFCRELCONTAINEDINSPATIALSTRUCTURE('c4',$,$,$,'bad',#4);\n");
    let session = session(&data);
    let error = select(
        &session,
        "#3",
        related(
            "IfcRelContainedInSpatialStructure",
            TraversalDirection::Forward,
            false,
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, RelationshipSelectionError::Unavailable(message) if message.contains("#33")),
        "{error:?}"
    );
}

#[test]
fn a_dangling_reference_refuses_the_whole_answer() {
    let data = format!("{BUILDING}#34=IFCRELCONTAINEDINSPATIALSTRUCTURE('c5',$,$,$,(#99),#4);\n");
    let session = session(&data);
    assert!(matches!(
        select(
            &session,
            "#4",
            related(
                "IfcRelContainedInSpatialStructure",
                TraversalDirection::Forward,
                false
            ),
        ),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
}

#[test]
fn unknown_or_non_relationship_types_are_refused_not_empty() {
    let session = session(BUILDING);
    for name in ["IfcRelDoesNotExist", "IfcWall", "contains"] {
        assert!(
            matches!(
                select(
                    &session,
                    "#3",
                    related(name, TraversalDirection::Forward, false)
                ),
                Err(RelationshipSelectionError::Unavailable(_))
            ),
            "{name}"
        );
    }
}

#[test]
fn relationship_types_whose_ends_are_not_plain_references_are_refused() {
    // IfcRelDefinesByProperties can relate an IfcPropertySetDefinitionSet,
    // a defined type holding a set of references, so it is not a plain edge.
    let session = session(BUILDING);
    assert!(matches!(
        select(
            &session,
            "#10",
            related(
                "IfcRelDefinesByProperties",
                TraversalDirection::Backward,
                false
            ),
        ),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
}

#[test]
fn objects_from_another_source_are_refused() {
    let session = session(BUILDING);
    let foreign = ObjectId::new(SourceId::new("ifc-step", "other.ifc").unwrap(), "#3").unwrap();
    let request = RelationshipSelectionRequest::try_new(
        foreign,
        every_object(&session),
        related("IfcRelAggregates", TraversalDirection::Forward, false),
    )
    .unwrap();
    assert!(matches!(
        session
            .service::<RelationshipSelectionServiceHandle>()
            .unwrap()
            .select(&request),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
}
