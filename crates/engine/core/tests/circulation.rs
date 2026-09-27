//! Circulation map contract: request normalisation and map validation.

use axioval_engine::{
    CirculationContact, CirculationMap, CirculationNode, CirculationNodeKind, CirculationRequest,
    ClearanceOutcome, ClearanceRequest, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError,
    FreeSpaceService, FreeSpaceServiceHandle, LengthInterval, PlacementOutcome, PlacementRequest,
};
use axioval_ir::{Evidence, ObjectId, SourceId};
use std::sync::Arc;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
}

fn evidence() -> Evidence {
    Evidence::exact(SourceId::new("test", "model").unwrap(), "circulation")
}

fn request() -> CirculationRequest {
    CirculationRequest::try_new(
        id("room"),
        vec![id("door"), id("room")],
        vec![id("wc"), id("wc")],
        vec![id("door"), id("wc"), id("room"), id("block")],
        0.9,
        2.0,
        0.05,
    )
    .unwrap()
}

fn node(x: f64, kind: CirculationNodeKind, piece: usize) -> CirculationNode {
    CirculationNode::try_new(
        [x, 1.0, 0.0],
        kind,
        piece,
        LengthInterval::try_new(0.6, 0.6).unwrap(),
    )
    .unwrap()
}

fn map(
    nodes: Vec<CirculationNode>,
    edges: Vec<(usize, usize)>,
    contacts: Vec<CirculationContact>,
) -> Result<CirculationMap, FreeSpaceError> {
    CirculationMap::try_new(
        request(),
        2,
        1,
        nodes,
        edges,
        0.1,
        contacts,
        vec![(1, "no skeleton".into())],
        evidence(),
    )
}

fn contacts() -> Vec<CirculationContact> {
    vec![
        CirculationContact::new(id("door"), vec![(0, Some(0))], vec![0]),
        CirculationContact::new(id("wc"), vec![(0, Some(2)), (1, None)], vec![0]),
    ]
}

fn path() -> Vec<CirculationNode> {
    vec![
        node(0.0, CirculationNodeKind::End, 0),
        node(1.0, CirculationNodeKind::Path, 0),
        node(2.0, CirculationNodeKind::End, 0),
    ]
}

#[test]
fn requests_drop_the_scope_and_never_obstruct_with_an_entrance() {
    let request = request();
    assert_eq!(request.entrances(), [id("door")]);
    assert_eq!(request.components(), [id("wc")]);
    assert_eq!(request.obstacles(), [id("block"), id("wc")]);
    assert_eq!(request.subjects(), [id("door"), id("wc")]);
    assert!((request.band().to_metres() - 2.0).abs() < f64::EPSILON);
    for (width, height, tolerance) in [(0.0, 2.0, 0.0), (0.9, f64::NAN, 0.0), (0.9, 2.0, -1.0)] {
        assert_eq!(
            CirculationRequest::try_new(
                id("room"),
                vec![],
                vec![],
                vec![],
                width,
                height,
                tolerance
            ),
            Err(FreeSpaceError::InvalidClearanceShape)
        );
    }
}

#[test]
fn maps_are_validated() {
    let valid = map(path(), vec![(0, 1), (1, 2)], contacts()).unwrap();
    assert_eq!(valid.neighbours(1), [0, 2]);
    assert!(valid.contact(&id("wc")).unwrap().reaches(1));
    assert_eq!(valid.unmapped_reason(1), Some("no skeleton"));
    // A kind that does not match the neighbours.
    assert!(map(path(), vec![(0, 1)], contacts()).is_err());
    // An edge between pieces, or to no node.
    let mut apart = path();
    apart[2] = node(2.0, CirculationNodeKind::End, 1);
    assert!(map(apart, vec![(0, 1), (1, 2)], contacts()).is_err());
    assert!(map(path(), vec![(0, 1), (1, 3)], contacts()).is_err());
    // Contacts must be the subjects, and name nodes of their pieces.
    assert!(map(path(), vec![(0, 1), (1, 2)], contacts()[..1].to_vec()).is_err());
    let wrong = vec![
        CirculationContact::new(id("door"), vec![(1, Some(0))], vec![0]),
        contacts()[1].clone(),
    ];
    assert!(map(path(), vec![(0, 1), (1, 2)], wrong).is_err());
    let possible = vec![
        CirculationContact::new(id("door"), vec![], vec![1]),
        contacts()[1].clone(),
    ];
    assert!(map(path(), vec![(0, 1), (1, 2)], possible).is_err());
    // Approximate evidence.
    assert_eq!(
        CirculationMap::try_new(
            request(),
            1,
            1,
            Vec::new(),
            Vec::new(),
            0.1,
            vec![
                CirculationContact::new(id("door"), vec![], vec![]),
                CirculationContact::new(id("wc"), vec![], vec![]),
            ],
            Vec::new(),
            Evidence {
                exact: false,
                ..evidence()
            },
        ),
        Err(FreeSpaceError::InexactPlacementEvidence)
    );
}

struct Other;

impl FreeSpaceService for Other {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        unreachable!()
    }
    fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        unreachable!()
    }
    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        unreachable!()
    }
    fn map_circulation(&self, _: &CirculationRequest) -> Result<CirculationMap, FreeSpaceError> {
        CirculationMap::try_new(
            CirculationRequest::try_new(id("hall"), vec![], vec![], vec![], 0.9, 2.0, 0.0)?,
            0,
            0,
            Vec::new(),
            Vec::new(),
            0.1,
            Vec::new(),
            Vec::new(),
            evidence(),
        )
    }
}

struct Silent;

impl FreeSpaceService for Silent {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        unreachable!()
    }
    fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        unreachable!()
    }
    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        unreachable!()
    }
}

#[test]
fn the_handle_rejects_maps_of_other_requests_and_services_refuse_by_default() {
    assert_eq!(
        FreeSpaceServiceHandle::new(Arc::new(Other)).map_circulation(&request()),
        Err(FreeSpaceError::ResponseRequestMismatch)
    );
    assert!(matches!(
        FreeSpaceServiceHandle::new(Arc::new(Silent)).map_circulation(&request()),
        Err(FreeSpaceError::Unavailable(_))
    ));
}
