//! Walkability topology contract tests.
use axioval_engine::{
    LengthInterval, PassageAdmission, ServiceRegistry, VerifiedWalkablePassage, VerticalConnector,
    VerticalConnectorKind, WalkabilityError, WalkabilityRegion, WalkabilityRegionId,
    WalkabilityRequest, WalkabilityRouteOutcome, WalkabilityService, WalkabilityServiceHandle,
    WalkabilitySnapshot,
};
use axioval_ir::{Evidence, ObjectId, SourceId};
use std::sync::Arc;
fn oid(src: &str, id: &str) -> ObjectId {
    ObjectId::new(SourceId::new("test", src).unwrap(), id).unwrap()
}
fn ev(loc: &str) -> Evidence {
    Evidence::exact(SourceId::new("test", "geometry").unwrap(), loc)
}
fn rid(id: &str) -> WalkabilityRegionId {
    WalkabilityRegionId::new(id).unwrap()
}
fn req(src: &str) -> WalkabilityRequest {
    WalkabilityRequest::try_new(
        vec![oid(src, "space")],
        vec![oid(src, "door")],
        vec![oid(src, "wall")],
        0.9,
        None,
        true,
        false,
    )
    .unwrap()
}
fn region(id: &str, objs: Vec<ObjectId>) -> WalkabilityRegion {
    WalkabilityRegion::new(rid(id), objs)
}
fn edge(a: &str, b: &str, lo: f64, hi: f64) -> VerifiedWalkablePassage {
    VerifiedWalkablePassage::try_new(
        rid(a),
        rid(b),
        None,
        LengthInterval::try_new(lo, hi).unwrap(),
        ev("passage"),
    )
    .unwrap()
}
#[test]
fn request_is_deterministic_and_source_qualified() {
    let r = req("cad");
    let r2 = WalkabilityRequest::try_new(
        vec![oid("cad", "space"), oid("cad", "space")],
        vec![oid("cad", "door")],
        vec![oid("cad", "wall")],
        0.9,
        None,
        true,
        false,
    )
    .unwrap();
    assert_eq!(r, r2);
    assert_ne!(r, req("other"));
    assert_eq!(r.surfaces().len(), 1);
}
#[test]
fn request_rejects_invalid_width() {
    assert_eq!(
        WalkabilityRequest::try_new(
            vec![oid("cad", "space")],
            vec![],
            vec![],
            f64::NAN,
            None,
            false,
            false
        ),
        Err(WalkabilityError::InvalidMinimumWidth)
    );
}
fn snapshot(edges: Vec<VerifiedWalkablePassage>) -> WalkabilitySnapshot {
    WalkabilitySnapshot::try_new(
        req("cad"),
        vec![
            region("a", vec![oid("cad", "space")]),
            region("b", vec![oid("cad", "door")]),
        ],
        edges,
        ev("complete"),
    )
    .unwrap()
}
#[test]
fn width_bounds_make_routes_three_valued() {
    let from = oid("cad", "space");
    let to = oid("cad", "door");
    assert!(matches!(
        snapshot(vec![edge("a", "b", 1.0, 1.0)])
            .route_between(&from, &to)
            .unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    assert_eq!(
        snapshot(vec![edge("a", "b", 0.5, 0.8)])
            .route_between(&from, &to)
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    assert_eq!(
        snapshot(vec![edge("a", "b", 0.8, 1.0)])
            .route_between(&from, &to)
            .unwrap(),
        WalkabilityRouteOutcome::Indeterminate
    );
}
#[test]
fn snapshot_requires_complete_exact_evidence() {
    let mut e = ev("partial");
    e.exact = false;
    assert_eq!(
        WalkabilitySnapshot::try_new(req("cad"), vec![region("a", vec![])], vec![], e),
        Err(WalkabilityError::IncompleteEvidence)
    );
}
#[test]
fn snapshot_rejects_unknown_passage_endpoint() {
    assert_eq!(
        WalkabilitySnapshot::try_new(
            req("cad"),
            vec![region("a", vec![])],
            vec![edge("a", "missing", 1.0, 1.0)],
            ev("complete")
        ),
        Err(WalkabilityError::UnknownRegion)
    );
}
struct Fixed;
impl WalkabilityService for Fixed {
    fn snapshot(&self, r: &WalkabilityRequest) -> Result<WalkabilitySnapshot, WalkabilityError> {
        WalkabilitySnapshot::try_new(
            r.clone(),
            vec![region("only", vec![oid("cad", "space")])],
            vec![],
            ev("complete"),
        )
    }
}
#[test]
fn service_is_typed_and_request_bound() {
    let h = WalkabilityServiceHandle::new(Arc::new(Fixed));
    let mut services = ServiceRegistry::new();
    services.register(h.clone()).unwrap();
    assert!(services.get::<WalkabilityServiceHandle>().is_some());
    assert_eq!(h.snapshot(&req("cad")).unwrap().request(), &req("cad"));
}
struct Wrong;
impl WalkabilityService for Wrong {
    fn snapshot(&self, _: &WalkabilityRequest) -> Result<WalkabilitySnapshot, WalkabilityError> {
        WalkabilitySnapshot::try_new(
            req("wrong"),
            vec![region("only", vec![])],
            vec![],
            ev("complete"),
        )
    }
}
#[test]
fn service_rejects_another_request() {
    let h = WalkabilityServiceHandle::new(Arc::new(Wrong));
    assert_eq!(
        h.snapshot(&req("cad")),
        Err(WalkabilityError::ResponseRequestMismatch)
    );
}

#[test]
fn snapshot_rejects_unrequested_object_mapping() {
    let r = req("cad");
    let bad = region("a", vec![oid("cad", "other")]);
    assert_eq!(
        WalkabilitySnapshot::try_new(r, vec![bad], vec![], ev("complete")),
        Err(WalkabilityError::UnexpectedMappedObject)
    );
}

#[test]
fn snapshot_rejects_duplicate_passage() {
    let r = req("cad");
    let e = edge("a", "b", 1.2, 1.2);
    assert_eq!(
        WalkabilitySnapshot::try_new(
            r,
            vec![region("a", vec![]), region("b", vec![])],
            vec![e.clone(), e],
            ev("complete")
        ),
        Err(WalkabilityError::DuplicatePassage)
    );
}

#[test]
fn snapshot_enforces_portal_policy() {
    let r = WalkabilityRequest::try_new(
        vec![oid("cad", "space")],
        vec![oid("cad", "door")],
        vec![],
        1.0,
        None,
        false,
        true,
    )
    .unwrap();
    let e = VerifiedWalkablePassage::try_new(
        rid("a"),
        rid("b"),
        Some(oid("cad", "door")),
        LengthInterval::exact(1.2).unwrap(),
        ev("door"),
    )
    .unwrap();
    assert_eq!(
        WalkabilitySnapshot::try_new(
            r,
            vec![region("a", vec![]), region("b", vec![])],
            vec![e],
            ev("complete")
        ),
        Err(WalkabilityError::ForbiddenPortalPassage)
    );
}

fn stair() -> VerticalConnector {
    VerticalConnector::new(oid("cad", "stair"), VerticalConnectorKind::Stair)
}

#[test]
fn a_connector_has_one_kind() {
    assert_eq!(
        req("cad").with_connectors(vec![
            stair(),
            VerticalConnector::new(oid("cad", "stair"), VerticalConnectorKind::Ramp),
        ]),
        Err(WalkabilityError::ConflictingConnector)
    );
    let r = req("cad").with_connectors(vec![stair(), stair()]).unwrap();
    assert_eq!(r.connectors(), &[stair()]);
}

#[test]
fn a_passage_is_a_portal_or_a_connector_never_both() {
    let crossing = VerifiedWalkablePassage::try_new(
        rid("a"),
        rid("b"),
        Some(oid("cad", "door")),
        LengthInterval::try_new(1.0, 1.0).unwrap(),
        ev("passage"),
    )
    .unwrap();
    assert_eq!(
        crossing.with_connector(stair()),
        Err(WalkabilityError::PortalConnectorPassage)
    );
}

fn levels(request: WalkabilityRequest) -> Result<WalkabilitySnapshot, WalkabilityError> {
    WalkabilitySnapshot::try_new(
        request,
        vec![
            region("a", vec![oid("cad", "space")]),
            region("b", vec![oid("cad", "door")]),
        ],
        vec![edge("a", "b", 1.0, 1.0).with_connector(stair()).unwrap()],
        ev("complete"),
    )
}

#[test]
fn a_connector_passage_must_be_requested() {
    assert_eq!(
        levels(req("cad")),
        Err(WalkabilityError::ForbiddenConnectorPassage)
    );
    let ramp = VerticalConnector::new(oid("cad", "stair"), VerticalConnectorKind::Ramp);
    assert_eq!(
        levels(req("cad").with_connectors(vec![ramp]).unwrap()),
        Err(WalkabilityError::ForbiddenConnectorPassage)
    );
}

#[test]
fn forbidding_a_connector_kind_makes_a_route_through_it_unreachable() {
    let snapshot = levels(req("cad").with_connectors(vec![stair()]).unwrap()).unwrap();
    let (from, to) = (oid("cad", "space"), oid("cad", "door"));
    assert!(matches!(
        snapshot.route_between(&from, &to).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    assert!(matches!(
        snapshot
            .route_between_avoiding(&from, &to, &[VerticalConnectorKind::Lift])
            .unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    assert_eq!(
        snapshot
            .route_between_avoiding(&from, &to, &[VerticalConnectorKind::Stair])
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn stated_clear_widths_name_requested_entrances_once() {
    let door = oid("cad", "door");
    let stated = req("cad")
        .with_stated_clear_widths([(door.clone(), 0.85)])
        .unwrap();
    assert_eq!(stated.stated_clear_width(&door), Some(0.85));
    assert_eq!(stated.stated_clear_width(&oid("cad", "space")), None);
    assert_ne!(stated, req("cad"));
    for bad in [
        vec![(oid("cad", "space"), 0.85)],
        vec![(door.clone(), 0.0)],
        vec![(door.clone(), f64::INFINITY)],
        vec![(door.clone(), 0.85), (door.clone(), 0.9)],
    ] {
        assert_eq!(
            req("cad").with_stated_clear_widths(bad),
            Err(WalkabilityError::InvalidStatedClearWidth)
        );
    }
}

/// `a` (the space) to `d` (the door) through `m`: `a`–`m` crosses the
/// gate, `m`–`d` climbs a stair; `x` (far), off to the side of `a`, lies
/// behind a crossing too narrow for the width.
fn chain() -> (WalkabilitySnapshot, VerifiedWalkablePassage) {
    let request = WalkabilityRequest::try_new(
        vec![oid("cad", "space"), oid("cad", "far")],
        vec![oid("cad", "door"), oid("cad", "gate"), oid("cad", "side")],
        vec![],
        0.9,
        None,
        true,
        false,
    )
    .unwrap()
    .with_connectors(vec![VerticalConnector::new(
        oid("cad", "stair"),
        VerticalConnectorKind::Stair,
    )])
    .unwrap();
    let gate = VerifiedWalkablePassage::try_new(
        rid("a"),
        rid("m"),
        Some(oid("cad", "gate")),
        LengthInterval::try_new(1.0, 1.0).unwrap(),
        ev("gate"),
    )
    .unwrap();
    let stair = edge("m", "d", 1.0, 1.0)
        .with_connector(VerticalConnector::new(
            oid("cad", "stair"),
            VerticalConnectorKind::Stair,
        ))
        .unwrap();
    let side = VerifiedWalkablePassage::try_new(
        rid("a"),
        rid("x"),
        Some(oid("cad", "side")),
        LengthInterval::try_new(0.0, 0.5).unwrap(),
        ev("side"),
    )
    .unwrap();
    let snapshot = WalkabilitySnapshot::try_new(
        request,
        vec![
            region("a", vec![oid("cad", "space")]),
            region("m", vec![oid("cad", "gate")]),
            region("d", vec![oid("cad", "door")]),
            region("x", vec![oid("cad", "far")]),
        ],
        vec![gate.clone(), stair, side],
        ev("complete"),
    )
    .unwrap();
    (snapshot, gate)
}

#[test]
fn a_rules_admission_narrows_both_graphs_three_valued() {
    let (snapshot, gate) = chain();
    let (from, to) = (oid("cad", "space"), oid("cad", "door"));
    let admit_all = |_: &VerifiedWalkablePassage| PassageAdmission::Admitted;
    assert!(matches!(
        snapshot
            .route_between_admitting(&from, &to, admit_all)
            .unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    // An undecided gate keeps the route possible, never definite.
    let undecided_gate = |passage: &VerifiedWalkablePassage| {
        if passage == &gate {
            PassageAdmission::Undecided
        } else {
            PassageAdmission::Admitted
        }
    };
    assert_eq!(
        snapshot
            .route_between_admitting(&from, &to, undecided_gate)
            .unwrap(),
        WalkabilityRouteOutcome::Indeterminate
    );
    assert!(
        snapshot
            .blocking_passages(&from, &to, undecided_gate)
            .unwrap()
            .is_empty()
    );
    // Forbidding stairs is a refusal: the stair is the cut, and the
    // narrow side crossing, which guards only `far`, is not part of it.
    assert_eq!(
        snapshot
            .route_between_avoiding(&from, &to, &[VerticalConnectorKind::Stair])
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    let no_stairs = |passage: &VerifiedWalkablePassage| {
        if passage.connector().is_some() {
            PassageAdmission::Refused
        } else {
            PassageAdmission::Admitted
        }
    };
    let blocking = snapshot.blocking_passages(&from, &to, no_stairs).unwrap();
    assert_eq!(blocking.len(), 1, "{blocking:?}");
    assert_eq!(
        blocking[0].connector().map(VerticalConnector::object),
        Some(&oid("cad", "stair"))
    );
    // The side crossing is too narrow for the width: it blocks `far`.
    let far = oid("cad", "far");
    let blocking = snapshot.blocking_passages(&from, &far, admit_all).unwrap();
    assert_eq!(blocking.len(), 1, "{blocking:?}");
    assert_eq!(blocking[0].portal(), Some(&oid("cad", "side")));
    assert_eq!(
        snapshot.blocking_passages(&from, &oid("cad", "nowhere"), admit_all),
        Err(WalkabilityError::ObjectUnavailable)
    );
}

#[test]
fn nothing_blocks_a_region_nothing_joins() {
    let snapshot = snapshot(vec![]);
    let admit_all = |_: &VerifiedWalkablePassage| PassageAdmission::Admitted;
    let (from, to) = (oid("cad", "space"), oid("cad", "door"));
    assert_eq!(
        snapshot
            .route_between_admitting(&from, &to, admit_all)
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    assert!(
        snapshot
            .blocking_passages(&from, &to, admit_all)
            .unwrap()
            .is_empty()
    );
}
