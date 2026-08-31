//! Walkability topology contract tests.
use axioval_engine::{
    LengthInterval, ServiceRegistry, VerifiedWalkablePassage, WalkabilityError, WalkabilityRegion,
    WalkabilityRegionId, WalkabilityRequest, WalkabilityRouteOutcome, WalkabilityService,
    WalkabilityServiceHandle, WalkabilitySnapshot,
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
