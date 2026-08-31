//! Free-space evidence and typed host-service contract tests.

use axioval_engine::{
    AreaInterval, BoxClearance, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest,
    ClearanceShape, CompleteClearanceEvidence, CompletePlacementEvidence, CompleteSupportEvidence,
    FrameOffsetPlacement, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService,
    FreeSpaceServiceHandle, MetricDirection, MetricFrame, MetricPoint, MobilityProfile,
    ObstructionEvidence, PlacementDomain, PlacementOutcome, PlacementRequest, ServiceRegistry,
    SignedDistanceInterval, SupportedPlacement, ThresholdVerdict,
};
use axioval_ir::{Evidence, ObjectId, SourceId};
use std::sync::Arc;

fn object(doc: &str, local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("test", doc).unwrap(), local).unwrap()
}
fn point(doc: &str, local: &str) -> MetricPoint {
    MetricPoint::try_new(object(doc, local), [0.0, 0.0, 0.0]).unwrap()
}
fn evidence(locator: &str) -> Evidence {
    Evidence::exact(SourceId::new("test", "geometry").unwrap(), locator)
}
fn profile() -> MobilityProfile {
    MobilityProfile::try_new(0.4, 1.8, 0.05, 0.08).unwrap()
}
fn request(doc: &str, local: &str) -> ClearanceRequest {
    let frame = MetricFrame::try_new(
        point(doc, local),
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap();
    ClearanceRequest::new(
        frame,
        ClearanceShape::Box(BoxClearance::try_new(1.5, 1.2, 2.0).unwrap()),
        vec![object(doc, "wall")],
    )
}

fn placement_request(doc: &str, local: &str) -> PlacementRequest {
    PlacementRequest::new(
        object(doc, local),
        ClearanceShape::Box(BoxClearance::try_new(1.5, 1.2, 2.0).unwrap()),
        vec![object(doc, "wall")],
    )
}

fn placement_shape() -> ClearanceShape {
    ClearanceShape::Box(BoxClearance::try_new(1.5, 1.2, 2.0).unwrap())
}

fn direction(components: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(components).unwrap()
}

fn placement_frame(doc: &str, local: &str) -> MetricFrame {
    MetricFrame::try_new(
        point(doc, local),
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap()
}

#[test]
fn obstacle_candidates_are_deterministic_and_source_qualified() {
    let frame = request("cad", "door").frame().clone();
    let a = object("a", "wall");
    let b = object("b", "wall");
    let req = ClearanceRequest::new(
        frame,
        ClearanceShape::Box(BoxClearance::try_new(1.0, 1.0, 1.0).unwrap()),
        vec![b.clone(), a.clone(), a.clone()],
    );
    assert_eq!(req.obstacles(), &[a, b]);
}

#[test]
fn area_bounds_compare_to_minimum_without_guessing() {
    let bounded = AreaInterval::try_new(8.0, 12.0).unwrap();
    assert_eq!(
        bounded.compare_minimum(7.0).unwrap(),
        ThresholdVerdict::Satisfied
    );
    assert_eq!(
        bounded.compare_minimum(13.0).unwrap(),
        ThresholdVerdict::Violated
    );
    assert_eq!(
        bounded.compare_minimum(10.0).unwrap(),
        ThresholdVerdict::Indeterminate
    );
}

#[test]
fn invalid_area_bounds_fail_closed() {
    assert_eq!(
        AreaInterval::try_new(3.0, 2.0),
        Err(FreeSpaceError::InvalidAreaInterval)
    );
    assert_eq!(
        AreaInterval::try_new(0.0, f64::NAN),
        Err(FreeSpaceError::InvalidAreaInterval)
    );
}

#[test]
fn frame_rejects_parallel_axes() {
    let result = MetricFrame::try_new(
        point("cad", "door"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([2.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    );
    assert_eq!(result, Err(FreeSpaceError::InvalidMetricFrame));
}

#[test]
fn frame_rejects_left_handed_axes() {
    assert_eq!(
        MetricFrame::try_new(
            point("cad", "door"),
            MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 0.0, -1.0]).unwrap(),
        ),
        Err(FreeSpaceError::InvalidMetricFrame)
    );
}

#[test]
fn shapes_reject_non_finite_or_non_positive_dimensions() {
    assert_eq!(
        BoxClearance::try_new(0.0, 1.0, 1.0),
        Err(FreeSpaceError::InvalidClearanceShape)
    );
    assert_eq!(
        BoxClearance::try_new(1.0, f64::INFINITY, 1.0),
        Err(FreeSpaceError::InvalidClearanceShape)
    );
}

#[test]
fn source_qualified_clearance_subjects_do_not_collapse() {
    assert_ne!(
        request("architect", "door-7"),
        request("proprietary-cad", "door-7")
    );
}

#[test]
fn clear_requires_exact_complete_evidence() {
    let mut approximate = evidence("partial-scene");
    approximate.exact = false;
    assert_eq!(
        CompleteClearanceEvidence::try_new(request("cad", "door"), approximate),
        Err(FreeSpaceError::IncompleteClearanceEvidence)
    );
}

#[test]
fn obstruction_requires_an_exact_nonempty_witness() {
    let req = request("cad", "door");
    assert_eq!(
        ObstructionEvidence::try_new(req.clone(), vec![], evidence("collision")),
        Err(FreeSpaceError::EmptyObstructionEvidence)
    );
    let hit = ObstructionEvidence::try_new(req, vec![object("cad", "wall")], evidence("collision"))
        .unwrap();
    assert_eq!(hit.blockers(), &[object("cad", "wall")]);
    assert_eq!(
        ObstructionEvidence::try_new(
            request("cad", "door"),
            vec![object("cad", "not-selected")],
            evidence("wrong-candidate"),
        ),
        Err(FreeSpaceError::UnexpectedObstacleEvidence)
    );
}

#[derive(Clone)]
struct DeterministicFreeSpace;
impl FreeSpaceService for DeterministicFreeSpace {
    fn assess_clearance(&self, req: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        Ok(ClearanceOutcome::Obstructed(ObstructionEvidence::try_new(
            req.clone(),
            vec![object("cad", "wall")],
            evidence("exact-collision"),
        )?))
    }
    fn find_placement(&self, req: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(req.scope().clone(), [2.0, 3.0, 0.0]).unwrap(),
            MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
        )?;
        Ok(PlacementOutcome::Found(
            ClearancePlacementEvidence::try_new(req.clone(), frame, evidence("exact-placement"))?,
        ))
    }
    fn measure_free_area(&self, req: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        FreeAreaEvidence::try_new(
            req.clone(),
            AreaInterval::exact(24.0)?,
            evidence("exact-area"),
        )
    }
}

#[test]
fn typed_free_space_service_is_backend_neutral_and_registerable() {
    let handle = FreeSpaceServiceHandle::new(Arc::new(DeterministicFreeSpace));
    let outcome = handle.assess_clearance(&request("cad", "door")).unwrap();
    let ClearanceOutcome::Obstructed(hit) = outcome else {
        panic!("expected obstruction")
    };
    assert_eq!(hit.blockers(), &[object("cad", "wall")]);
    let area_request = FreeAreaRequest::new(object("cad", "room"), profile(), vec![]);
    let measured = handle.measure_free_area(&area_request).unwrap();
    assert!((measured.available_area().lower_square_metres() - 24.0).abs() < f64::EPSILON);
    let mut services = ServiceRegistry::new();
    services.register(handle).unwrap();
    assert!(services.get::<FreeSpaceServiceHandle>().is_some());
}

#[derive(Clone)]
struct WrongRequestFreeSpace;
impl FreeSpaceService for WrongRequestFreeSpace {
    fn assess_clearance(
        &self,
        _req: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        Ok(ClearanceOutcome::Clear(CompleteClearanceEvidence::try_new(
            request("cad", "other-door"),
            evidence("complete-other-door"),
        )?))
    }
    fn find_placement(&self, _req: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        Ok(PlacementOutcome::NoPlacement(
            CompletePlacementEvidence::try_new(
                placement_request("cad", "other-room"),
                evidence("complete-other-room"),
            )?,
        ))
    }
    fn measure_free_area(
        &self,
        _req: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError> {
        FreeAreaEvidence::try_new(
            FreeAreaRequest::new(object("cad", "other-room"), profile(), vec![]),
            AreaInterval::exact(8.0)?,
            evidence("area-other-room"),
        )
    }
}

#[test]
fn service_rejects_clearance_evidence_for_another_request() {
    let handle = FreeSpaceServiceHandle::new(Arc::new(WrongRequestFreeSpace));
    assert_eq!(
        handle.assess_clearance(&request("cad", "door")),
        Err(FreeSpaceError::ResponseRequestMismatch)
    );
}

#[test]
fn service_rejects_area_evidence_for_another_request() {
    let handle = FreeSpaceServiceHandle::new(Arc::new(WrongRequestFreeSpace));
    let req = FreeAreaRequest::new(object("cad", "room"), profile(), vec![]);
    assert_eq!(
        handle.measure_free_area(&req),
        Err(FreeSpaceError::ResponseRequestMismatch)
    );
}

#[test]
fn no_placement_requires_complete_exact_search_evidence() {
    let mut partial = evidence("partial-search");
    partial.exact = false;
    assert_eq!(
        CompletePlacementEvidence::try_new(placement_request("cad", "room"), partial),
        Err(FreeSpaceError::IncompleteClearanceEvidence)
    );
}

#[test]
fn placement_search_returns_an_exact_frame_witness() {
    let req = placement_request("cad", "room");
    let handle = FreeSpaceServiceHandle::new(Arc::new(DeterministicFreeSpace));
    let PlacementOutcome::Found(found) = handle.find_placement(&req).unwrap() else {
        panic!("expected placement witness")
    };
    assert_eq!(found.request(), &req);
    assert_eq!(found.frame().origin().subject(), &object("cad", "room"));
}

#[test]
fn service_rejects_placement_evidence_for_another_request() {
    let req = placement_request("cad", "room");
    let handle = FreeSpaceServiceHandle::new(Arc::new(WrongRequestFreeSpace));
    assert_eq!(
        handle.find_placement(&req),
        Err(FreeSpaceError::ResponseRequestMismatch)
    );
}

#[test]
fn placement_domains_validate_metric_bounds() {
    assert_eq!(
        SignedDistanceInterval::try_new(1.0, -1.0),
        Err(FreeSpaceError::InvalidOffsetInterval)
    );
    assert_eq!(
        SupportedPlacement::try_new(object("cad", "floor"), -0.1),
        Err(FreeSpaceError::InvalidSupportGap)
    );
}

#[test]
fn supported_placement_is_explicitly_request_bound() {
    let support = SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap();
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::Supported(support.clone()),
    )
    .unwrap();
    assert_eq!(r.domain(), &PlacementDomain::Supported(support));
}

fn relative_domain(src: &str) -> PlacementDomain {
    let offsets = FrameOffsetPlacement::new(
        placement_frame(src, "room"),
        SignedDistanceInterval::try_new(-1.0, 1.0).unwrap(),
        SignedDistanceInterval::try_new(0.0, 2.0).unwrap(),
        SignedDistanceInterval::exact(0.0).unwrap(),
    );
    PlacementDomain::FrameOffsets(offsets)
}

#[test]
fn placement_witness_must_satisfy_frame_offset_domain() {
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        relative_domain("cad"),
    )
    .unwrap();
    let found = MetricFrame::try_new(
        MetricPoint::try_new(object("cad", "room"), [2.0, 0.0, 0.0]).unwrap(),
        direction([1.0, 0.0, 0.0]),
        direction([0.0, 1.0, 0.0]),
        direction([0.0, 0.0, 1.0]),
    )
    .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new(r, found, evidence("placement")),
        Err(FreeSpaceError::PlacementDomainMismatch)
    );
}

#[test]
fn relative_domain_anchor_must_match_request_scope() {
    let r = PlacementRequest::new_in_domain(
        object("cad", "other"),
        placement_shape(),
        vec![],
        relative_domain("cad"),
    );
    assert_eq!(r, Err(FreeSpaceError::PlacementScopeMismatch));
}

fn placement_frame_at(doc: &str, local: &str, xyz: [f64; 3]) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(object(doc, local), xyz).unwrap(),
        direction([1.0, 0.0, 0.0]),
        direction([0.0, 1.0, 0.0]),
        direction([0.0, 0.0, 1.0]),
    )
    .unwrap()
}

#[test]
fn placement_witness_within_frame_offsets_is_accepted() {
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        relative_domain("cad"),
    )
    .unwrap();
    let witness = ClearancePlacementEvidence::try_new(
        r.clone(),
        placement_frame_at("cad", "room", [0.5, 1.0, 0.0]),
        evidence("inside"),
    )
    .unwrap();
    assert_eq!(witness.request(), &r);
}

#[test]
fn support_and_frame_offset_constraints_can_be_conjoined() {
    let support = SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap();
    let PlacementDomain::FrameOffsets(offsets) = relative_domain("cad") else {
        unreachable!()
    };
    let domain = PlacementDomain::SupportedFrameOffsets {
        support: support.clone(),
        offsets,
    };
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        domain.clone(),
    )
    .unwrap();
    assert_eq!(r.domain(), &domain);
    let frame = placement_frame_at("cad", "room", [0.5, 1.0, 0.0]);
    let proof = CompleteSupportEvidence::try_new(
        object("cad", "floor"),
        frame.clone(),
        0.005,
        evidence("combined-support"),
    )
    .unwrap();
    let found = ClearancePlacementEvidence::try_new_supported(
        r.clone(),
        frame,
        proof,
        evidence("combined-placement"),
    )
    .unwrap();
    assert_eq!(found.request(), &r);
}

#[test]
fn placement_witness_must_preserve_anchor_orientation() {
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        relative_domain("cad"),
    )
    .unwrap();
    let found = MetricFrame::try_new(
        point("cad", "room"),
        direction([0.0, 1.0, 0.0]),
        direction([-1.0, 0.0, 0.0]),
        direction([0.0, 0.0, 1.0]),
    )
    .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new(r, found, evidence("rotated")),
        Err(FreeSpaceError::PlacementDomainMismatch)
    );
}

#[test]
fn exact_offset_domain_rejects_tolerance_expansion() {
    let offsets = FrameOffsetPlacement::new(
        placement_frame("cad", "room"),
        SignedDistanceInterval::exact(0.0).unwrap(),
        SignedDistanceInterval::exact(0.0).unwrap(),
        SignedDistanceInterval::exact(0.0).unwrap(),
    );
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::FrameOffsets(offsets),
    )
    .unwrap();
    let found = placement_frame_at("cad", "room", [5.0e-10, 0.0, 0.0]);
    assert_eq!(
        ClearancePlacementEvidence::try_new(r, found, evidence("outside-exact")),
        Err(FreeSpaceError::PlacementDomainMismatch)
    );
}

#[test]
fn supported_domain_rejects_found_witness_without_support_proof() {
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::Supported(
            SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new(
            r,
            placement_frame("cad", "room"),
            evidence("placement")
        ),
        Err(FreeSpaceError::MissingSupportEvidence)
    );
}

#[test]
fn supported_witness_requires_exact_frame_bound_support_proof() {
    let frame = placement_frame("cad", "room");
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::Supported(
            SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap(),
        ),
    )
    .unwrap();
    let support = CompleteSupportEvidence::try_new(
        object("cad", "floor"),
        frame.clone(),
        0.005,
        evidence("full-base-support"),
    )
    .unwrap();
    let found = ClearancePlacementEvidence::try_new_supported(
        r.clone(),
        frame,
        support,
        evidence("placement"),
    )
    .unwrap();
    assert_eq!(found.request(), &r);
}

#[test]
fn support_proof_must_name_requested_source_qualified_support() {
    let frame = placement_frame("cad", "room");
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::Supported(
            SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap(),
        ),
    )
    .unwrap();
    let proof = CompleteSupportEvidence::try_new(
        object("other", "floor"),
        frame.clone(),
        0.0,
        evidence("wrong-support"),
    )
    .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new_supported(r, frame, proof, evidence("placement")),
        Err(FreeSpaceError::SupportEvidenceMismatch)
    );
}

#[test]
fn support_proof_must_match_frame_and_gap_limit() {
    let frame = placement_frame("cad", "room");
    let r = PlacementRequest::new_in_domain(
        object("cad", "room"),
        placement_shape(),
        vec![],
        PlacementDomain::Supported(
            SupportedPlacement::try_new(object("cad", "floor"), 0.01).unwrap(),
        ),
    )
    .unwrap();
    let moved = placement_frame_at("cad", "room", [0.1, 0.0, 0.0]);
    let wrong_frame =
        CompleteSupportEvidence::try_new(object("cad", "floor"), moved, 0.0, evidence("support"))
            .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new_supported(
            r.clone(),
            frame.clone(),
            wrong_frame,
            evidence("placement")
        ),
        Err(FreeSpaceError::SupportEvidenceMismatch)
    );
    let excessive = CompleteSupportEvidence::try_new(
        object("cad", "floor"),
        frame.clone(),
        0.02,
        evidence("support"),
    )
    .unwrap();
    assert_eq!(
        ClearancePlacementEvidence::try_new_supported(r, frame, excessive, evidence("placement")),
        Err(FreeSpaceError::SupportEvidenceMismatch)
    );
}

#[test]
fn support_proof_must_be_exact_and_reviewable() {
    let approximate = Evidence {
        source: SourceId::new("test", "geometry").unwrap(),
        locator: "support".into(),
        exact: false,
    };
    assert_eq!(
        CompleteSupportEvidence::try_new(
            object("cad", "floor"),
            placement_frame("cad", "room"),
            0.0,
            approximate
        ),
        Err(FreeSpaceError::InexactSupportEvidence)
    );
}
