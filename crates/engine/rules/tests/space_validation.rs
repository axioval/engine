//! Space-validation capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin each
//! aspect independently -- in particular that one unavailable aspect does not
//! sink the others, which is the defect the bundled source fact struct had.
#![allow(missing_docs)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axioval_engine::{
    BoundaryGap, BoundaryRequest, Cap, CapCoverage, CapRequest, ClearHeightEvidence, CompiledRule,
    Containment, NotEvaluatedReason, OverlapRequest, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionService, PropertyResolutionServiceHandle,
    RuleCapability, RuleContext, ServiceRegistry, SpaceAspect, SpaceError, SpaceOverlap,
    SpaceService, SpaceServiceHandle, SupportCounts, UnallocatedRegion,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, Severity, SourceId};
use axioval_rules::{SpaceCategory, SpaceValidation};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn rule_with(overrides: &[(&str, ParameterValue)]) -> CompiledRule {
    let mut parameters = BTreeMap::from([
        (
            "required_height_metres".to_string(),
            ParameterValue::Number { value: 2.5 },
        ),
        (
            "uncovered_segment_length_metres".to_string(),
            ParameterValue::Number { value: 1.0 },
        ),
        (
            "check_top_cap".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "check_bottom_cap".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "check_unallocated_area".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "maximum_unallocated_area_square_metres".to_string(),
            ParameterValue::Number { value: 1.0 },
        ),
    ]);
    for (key, value) in overrides {
        parameters.insert((*key).to_string(), value.clone());
    }
    CompiledRule {
        id: RuleId::new("space-validation").unwrap(),
        capability: "axioval:capability.space-validation".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: false,
        },
        parameters,
    }
}
fn rule() -> CompiledRule {
    rule_with(&[])
}

/// Every aspect answers independently, so a test can make exactly one fail.
/// One stubbed answer per aspect.
type Answer<T> = Option<Result<T, SpaceError>>;
type GapRows = Vec<(f64, Vec<ObjectId>)>;
type OverlapRows = Vec<(bool, f64, f64, Containment)>;
type ResidualRows = Vec<(ObjectId, f64, Vec<ObjectId>)>;

#[derive(Default)]
struct Stub {
    duplicates: Answer<Vec<ObjectId>>,
    height: Answer<f64>,
    gaps: Answer<GapRows>,
    overlaps: Answer<OverlapRows>,
    cap: Answer<(f64, f64)>,
    residuals: Answer<ResidualRows>,
    /// The gross floor area every residual states, if any.
    floor: Option<f64>,
    support: Answer<(usize, usize)>,
    /// Every cap request the capability made, in order.
    cap_requests: Mutex<Vec<CapRequest>>,
    /// How often the support counts were asked for.
    support_calls: Mutex<usize>,
    /// Every boundary request the capability made, in order.
    boundary_requests: Mutex<Vec<BoundaryRequest>>,
    /// Every overlap request the capability made, in order.
    overlap_requests: Mutex<Vec<OverlapRequest>>,
}

fn evidence() -> Evidence {
    Evidence::exact(source(), "space:measured")
}

impl SpaceService for Stub {
    fn measure_duplicates(&self, _space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError> {
        self.duplicates.clone().unwrap_or(Ok(Vec::new()))
    }
    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError> {
        let metres = self.height.clone().unwrap_or(Ok(3.0))?;
        ClearHeightEvidence::try_new(space.clone(), metres, evidence())
    }
    fn measure_boundary_gaps(
        &self,
        _space: &ObjectId,
        request: &BoundaryRequest,
    ) -> Result<Vec<BoundaryGap>, SpaceError> {
        self.boundary_requests.lock().unwrap().push(request.clone());
        self.gaps
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(length, elements)| BoundaryGap::try_new(length, elements))
            .collect()
    }
    fn measure_overlaps(
        &self,
        _space: &ObjectId,
        request: &OverlapRequest,
    ) -> Result<Vec<SpaceOverlap>, SpaceError> {
        self.overlap_requests.lock().unwrap().push(request.clone());
        self.overlaps
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(is_space, area, height, containment)| {
                SpaceOverlap::try_new(oid("other"), is_space, area, height, containment)
            })
            .collect()
    }
    fn measure_cap_coverage(
        &self,
        _space: &ObjectId,
        request: &CapRequest,
    ) -> Result<CapCoverage, SpaceError> {
        self.cap_requests.lock().unwrap().push(request.clone());
        let (whole, covered) = self.cap.clone().unwrap_or(Ok((10.0, 10.0)))?;
        CapCoverage::try_new(whole, covered, Vec::new())
    }
    fn measure_unallocated_regions(&self) -> Result<Vec<UnallocatedRegion>, SpaceError> {
        self.residuals
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(storey, area, elements)| {
                let region = UnallocatedRegion::try_new(storey, area, elements)?;
                match self.floor {
                    Some(floor) => region.with_floor_area(floor),
                    None => Ok(region),
                }
            })
            .collect()
    }
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError> {
        *self.support_calls.lock().unwrap() += 1;
        let (slabs, roofs) = self.support.clone().unwrap_or(Ok((2, 1)))?;
        Ok(SupportCounts::new(slabs, roofs, vec![oid("bldg")]))
    }
    fn evidence(&self) -> Evidence {
        evidence()
    }
}

fn evaluate(stub: Stub, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    evaluate_shared(&Arc::new(stub), rule)
}

/// Evaluates over one space, a ceiling covering and a slab, keeping the stub
/// so a test can read back what the capability asked for.
fn evaluate_shared(stub: &Arc<Stub>, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![
        Object::new(oid("space-1"), "space"),
        Object::new(oid("ceiling"), "covering"),
        Object::new(oid("slab"), "slab"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(SpaceServiceHandle::new(stub.clone()))
        .unwrap();
    SpaceValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn a_clean_space_produces_nothing() {
    let outcome = evaluate(Stub::default(), &rule());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn duplicated_space_body_is_an_error() {
    let outcome = evaluate(
        Stub {
            duplicates: Some(Ok(vec![oid("space-2")])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].severity, Severity::Error);
}

/// The coincident spaces travel with the finding, normalised: a reviewer needs
/// to open them, and ordering must not depend on adapter traversal order.
#[test]
fn a_duplicate_finding_names_the_coincident_spaces() {
    let outcome = evaluate(
        Stub {
            duplicates: Some(Ok(vec![oid("space-3"), oid("space-2"), oid("space-3")])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(
        outcome.findings()[0].related,
        vec![oid("space-2"), oid("space-3")]
    );
}

#[test]
fn height_below_requirement_is_reported_but_tolerance_is_respected() {
    let low = evaluate(
        Stub {
            height: Some(Ok(2.0)),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(low.findings().len(), 1);
    assert!(low.findings()[0].message.contains("below required"));

    // 2.4995 is short by less than the 5 mm tolerance: measurement noise,
    // not a low space.
    let within_tolerance = evaluate(
        Stub {
            height: Some(Ok(2.4995)),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(within_tolerance.findings().is_empty());
}

#[test]
fn only_boundary_gaps_at_least_the_declared_length_count() {
    let short = evaluate(
        Stub {
            gaps: Some(Ok(vec![(0.4, vec![oid("w1")])])),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(short.findings().is_empty(), "a short gap is an artefact");

    let long = evaluate(
        Stub {
            gaps: Some(Ok(vec![(2.0, vec![oid("w1")])])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(long.findings().len(), 1);
}

#[test]
fn containment_and_substantial_intersection_are_errors_but_contact_is_not() {
    let contained = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(false, 0.0, 0.0, Containment::SubjectInsideOther)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(contained.findings().len(), 1);

    let intersecting = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(true, 2.0, 1.0, Containment::Partial)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(intersecting.findings().len(), 1);

    // Touching faces: real contact, not an intersection.
    let touching = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(false, 1.0, 0.001, Containment::Partial)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(touching.findings().is_empty());
}

#[test]
fn cap_coverage_is_graded_and_near_complete_coverage_passes() {
    let enabled = [("check_top_cap", ParameterValue::Boolean { value: true })];
    let complete = evaluate(
        Stub {
            cap: Some(Ok((10.0, 9.9))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert!(complete.findings().is_empty());

    let bare = evaluate(
        Stub {
            cap: Some(Ok((10.0, 0.05))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert_eq!(bare.findings()[0].severity, Severity::Error);

    let partial = evaluate(
        Stub {
            cap: Some(Ok((10.0, 5.0))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert_eq!(partial.findings()[0].severity, Severity::Info);
}

/// A cap check without any element that could form the cap says nothing about
/// the space. It must not report every space as uncovered.
#[test]
fn cap_check_is_skipped_when_the_model_has_no_supporting_elements() {
    let outcome = evaluate(
        Stub {
            support: Some(Ok((0, 0))),
            cap: Some(Ok((10.0, 0.0))),
            ..Stub::default()
        },
        &rule_with(&[("check_top_cap", ParameterValue::Boolean { value: true })]),
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn storey_residual_above_the_allowance_is_reported_against_the_storey() {
    let outcome = evaluate(
        Stub {
            residuals: Some(Ok(vec![(oid("storey-1"), 5.0, Vec::new())])),
            ..Stub::default()
        },
        &rule_with(&[(
            "check_unallocated_area",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id(), Some(&oid("storey-1")));

    let within = evaluate(
        Stub {
            residuals: Some(Ok(vec![(oid("storey-1"), 0.5, Vec::new())])),
            ..Stub::default()
        },
        &rule_with(&[(
            "check_unallocated_area",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert!(within.findings().is_empty());
}

#[test]
fn an_unallocated_share_of_the_gross_area_is_bounded() {
    let share = |regions: &[f64], floor: Option<f64>| {
        evaluate(
            Stub {
                residuals: Some(Ok(regions
                    .iter()
                    .map(|area| (oid("storey-1"), *area, Vec::new()))
                    .collect())),
                floor,
                ..Stub::default()
            },
            &rule_with(&[
                (
                    "check_unallocated_area",
                    ParameterValue::Boolean { value: true },
                ),
                (
                    "maximum_unallocated_area_square_metres",
                    ParameterValue::Number { value: 1000.0 },
                ),
                (
                    "maximum_unallocated_share",
                    ParameterValue::Number { value: 0.03 },
                ),
            ]),
        )
    };
    // 5 m² of 100 m² outside every space is 5 %, beyond 3 %.
    let over = share(&[5.0], Some(100.0));
    assert_eq!(over.findings().len(), 1);
    assert_eq!(
        over.findings()[0].message,
        "unallocated_area: 5.000% of the storey's gross floor area (5.000 m2 of 100.000 m2) \
         belongs to no space; required at most 3%"
    );
    assert!(share(&[2.0], Some(100.0)).findings().is_empty());
    // The storey's regions count together: two 2 m² shafts are 4 %.
    let together = share(&[2.0, 2.0], Some(100.0));
    assert_eq!(together.findings().len(), 1);
    assert!(
        together.findings()[0]
            .message
            .contains("4.000% of the storey's gross floor area"),
        "{}",
        together.findings()[0].message
    );
    // Without a gross area the share is undefined.
    let unknown = share(&[5.0], None);
    assert!(unknown.findings().is_empty());
    assert_eq!(
        unknown.not_evaluated_outcomes()[0].object_id(),
        Some(&oid("storey-1"))
    );
    // A share above one is no share.
    let invalid = evaluate(
        Stub::default(),
        &rule_with(&[(
            "maximum_unallocated_share",
            ParameterValue::Number { value: 3.0 },
        )]),
    );
    assert!(invalid.findings().is_empty());
    assert!(
        invalid
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| *outcome.reason() == NotEvaluatedReason::InvalidDeclaration)
    );
}

/// The reason for splitting the bundled fact struct: one unavailable aspect
/// must cost only that aspect. Height is unavailable, yet the duplicate check
/// still reports.
#[test]
fn one_unavailable_aspect_does_not_sink_the_others() {
    let outcome = evaluate(
        Stub {
            height: Some(Err(SpaceError::Unavailable)),
            duplicates: Some(Ok(vec![oid("space-2")])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(
        outcome.findings().len(),
        1,
        "the duplicate finding must survive an unavailable height"
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

/// A measurement refused for unmeasured objects is incomplete evidence, and
/// the outcome names the aspect and the objects that blocked it.
#[test]
fn an_unmeasured_refusal_names_what_blocked_it() {
    let outcome = evaluate(
        Stub {
            cap: Some(Err(SpaceError::unmeasured(
                SpaceAspect::CapCoverage(Cap::Top),
                vec![oid("roof")],
            ))),
            ..Stub::default()
        },
        &rule_with(&[("check_top_cap", ParameterValue::Boolean { value: true })]),
    );
    let refused = outcome
        .not_evaluated_outcomes()
        .iter()
        .find(|outcome| outcome.message().contains("cap coverage"))
        .expect("the cap is not evaluated");
    assert_eq!(refused.reason(), &NotEvaluatedReason::IncompleteEvidence);
    assert!(
        refused.message().contains("top cap coverage")
            && refused.message().contains("cad:model/roof"),
        "{}",
        refused.message()
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![Object::new(oid("space-1"), "space")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = SpaceValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn invalid_declaration_is_refused() {
    let outcome = evaluate(
        Stub::default(),
        &rule_with(&[(
            "required_height_metres",
            ParameterValue::String {
                value: "tall".into(),
            },
        )]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// Every finding names the objects a reviewer must open: the body a space
/// overlaps, the elements along an uncovered boundary, the elements covering a
/// cap. Without them the message says something is wrong but not where.
#[test]
fn findings_name_the_objects_a_reviewer_must_open() {
    let overlap = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(false, 2.0, 1.0, Containment::Partial)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(overlap.findings()[0].related, vec![oid("other")]);

    let boundary = evaluate(
        Stub {
            gaps: Some(Ok(vec![(2.0, vec![oid("wall-1")])])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(boundary.findings()[0].related, vec![oid("wall-1")]);
}

fn of_type(object_type: &str) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(Selector::EntityType {
            object_type: object_type.into(),
            include_subtypes: false,
        }),
    }
}

/// The height tolerance is the rule's, not a constant: 10 cm short passes a
/// rule that allows 20 cm, and fails the default 5 mm.
#[test]
fn the_rule_sets_the_height_tolerance() {
    let low = || Stub {
        height: Some(Ok(2.4)),
        ..Stub::default()
    };
    assert_eq!(evaluate(low(), &rule()).findings().len(), 1);
    let lenient = rule_with(&[("tolerance_metres", ParameterValue::Number { value: 0.2 })]);
    assert!(evaluate(low(), &lenient).findings().is_empty());
}

/// The same tolerance separates contact from intersection: a 1 cm overlap
/// intersects under the default 5 mm and is contact under 2 cm.
#[test]
fn the_rule_sets_the_overlap_thickness_tolerance() {
    let thin = || Stub {
        overlaps: Some(Ok(vec![(false, 2.0, 0.01, Containment::Partial)])),
        ..Stub::default()
    };
    assert_eq!(evaluate(thin(), &rule()).findings().len(), 1);
    let lenient = rule_with(&[("tolerance_metres", ParameterValue::Number { value: 0.02 })]);
    assert!(evaluate(thin(), &lenient).findings().is_empty());
}

#[test]
fn a_negative_or_mistyped_tolerance_is_an_invalid_declaration() {
    for value in [
        ParameterValue::Number { value: -0.1 },
        ParameterValue::String {
            value: "5 mm".into(),
        },
    ] {
        let outcome = evaluate(Stub::default(), &rule_with(&[("tolerance_metres", value)]));
        assert!(outcome.findings().is_empty());
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

/// Without cap selectors the host's declared slabs and roofs bound the caps:
/// the requests carry no elements and the support counts decide availability.
#[test]
fn without_cap_selectors_the_host_declared_elements_are_used() {
    let stub = Arc::new(Stub::default());
    evaluate_shared(
        &stub,
        &rule_with(&[
            ("check_top_cap", ParameterValue::Boolean { value: true }),
            ("check_bottom_cap", ParameterValue::Boolean { value: true }),
        ]),
    );
    let requests = stub.cap_requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.elements().is_none()));
    assert_eq!(*stub.support_calls.lock().unwrap(), 1);
}

/// A cap selector states the bounding elements: the request carries exactly
/// the selection, and a model with no host-declared slab or roof is still
/// checked, since the rule named what caps a space.
#[test]
fn a_cap_selector_chooses_the_bounding_elements() {
    let stub = Arc::new(Stub {
        support: Some(Ok((0, 0))),
        cap: Some(Ok((10.0, 0.0))),
        ..Stub::default()
    });
    let outcome = evaluate_shared(
        &stub,
        &rule_with(&[
            ("check_top_cap", ParameterValue::Boolean { value: true }),
            ("top_cap_elements", of_type("covering")),
        ]),
    );
    let requests = stub.cap_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].elements(), Some(&[oid("ceiling")][..]));
    assert_eq!(outcome.findings().len(), 1);
    assert!(
        outcome.findings()[0]
            .message
            .starts_with(SpaceCategory::UncoveredTopCap.code())
    );
    // Both caps selected: the host's counts are never consulted.
    assert_eq!(*stub.support_calls.lock().unwrap(), 0);
}

/// Each cap has its own selector; the other keeps the host's declaration.
#[test]
fn cap_selectors_are_independent() {
    let stub = Arc::new(Stub::default());
    evaluate_shared(
        &stub,
        &rule_with(&[
            ("check_top_cap", ParameterValue::Boolean { value: true }),
            ("check_bottom_cap", ParameterValue::Boolean { value: true }),
            ("bottom_cap_elements", of_type("slab")),
        ]),
    );
    let requests = stub.cap_requests.lock().unwrap();
    let elements: Vec<_> = requests.iter().map(CapRequest::elements).collect();
    assert_eq!(elements, vec![None, Some(&[oid("slab")][..])]);
}

/// A selector that selects nothing leaves nothing that could form the cap,
/// so the cap is not checked, whatever the host declared.
#[test]
fn a_cap_selector_selecting_nothing_skips_the_cap() {
    let stub = Arc::new(Stub {
        cap: Some(Ok((10.0, 0.0))),
        ..Stub::default()
    });
    let outcome = evaluate_shared(
        &stub,
        &rule_with(&[
            ("check_top_cap", ParameterValue::Boolean { value: true }),
            ("top_cap_elements", of_type("roof")),
        ]),
    );
    assert!(stub.cap_requests.lock().unwrap().is_empty());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// An undecided cap selection may hide the covering element: the cap is not
/// evaluated for each space, and the other aspects still run.
#[test]
fn an_undecided_cap_selection_leaves_only_the_cap_unevaluated() {
    let stub = Arc::new(Stub {
        duplicates: Some(Ok(vec![oid("space-2")])),
        ..Stub::default()
    });
    let classified = ParameterValue::Selector {
        value: Box::new(Selector::Classification {
            system: "uniclass".into(),
            code: Some("Ss_30".into()),
            code_pattern: None,
            include_descendants: false,
        }),
    };
    let outcome = evaluate_shared(
        &stub,
        &rule_with(&[
            ("check_top_cap", ParameterValue::Boolean { value: true }),
            ("top_cap_elements", classified),
        ]),
    );
    assert!(stub.cap_requests.lock().unwrap().is_empty());
    assert_eq!(outcome.findings().len(), 1, "the duplicate still reports");
    let unevaluated = outcome.not_evaluated_outcomes();
    assert_eq!(unevaluated.len(), 1);
    assert_eq!(unevaluated[0].object_id(), Some(&oid("space-1")));
    assert_eq!(unevaluated[0].reason(), &NotEvaluatedReason::MissingService);
}

#[test]
fn a_mistyped_cap_selector_is_an_invalid_declaration() {
    let outcome = evaluate(
        Stub::default(),
        &rule_with(&[(
            "top_cap_elements",
            ParameterValue::String {
                value: "IfcSlab".into(),
            },
        )]),
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// Every sub-check writes its own category code first, so results can be
/// grouped by problem as well as by space.
#[test]
fn each_sub_check_reports_its_category() {
    let on = |key: &'static str| (key, ParameterValue::Boolean { value: true });
    let top = rule_with(&[on("check_top_cap")]);
    let bottom = rule_with(&[on("check_bottom_cap")]);
    let residual = rule_with(&[on("check_unallocated_area")]);
    let overlap = |is_space, containment| Stub {
        overlaps: Some(Ok(vec![(is_space, 2.0, 1.0, containment)])),
        ..Stub::default()
    };
    let half_cap = || Stub {
        cap: Some(Ok((10.0, 5.0))),
        ..Stub::default()
    };
    let cases = [
        (
            Stub {
                duplicates: Some(Ok(vec![oid("space-2")])),
                ..Stub::default()
            },
            rule(),
            SpaceCategory::DuplicateSpace,
        ),
        (
            Stub {
                height: Some(Ok(2.0)),
                ..Stub::default()
            },
            rule(),
            SpaceCategory::InsufficientHeight,
        ),
        (
            Stub {
                gaps: Some(Ok(vec![(2.0, Vec::new())])),
                ..Stub::default()
            },
            rule(),
            SpaceCategory::UncoveredBoundary,
        ),
        (
            overlap(false, Containment::OtherInsideSubject),
            rule(),
            SpaceCategory::ContainedBody,
        ),
        (
            overlap(true, Containment::Partial),
            rule(),
            SpaceCategory::IntersectingSpace,
        ),
        (
            overlap(false, Containment::Partial),
            rule(),
            SpaceCategory::IntersectingComponent,
        ),
        (half_cap(), top, SpaceCategory::UncoveredTopCap),
        (half_cap(), bottom, SpaceCategory::UncoveredBottomCap),
        (
            Stub {
                residuals: Some(Ok(vec![(oid("storey-1"), 5.0, Vec::new())])),
                ..Stub::default()
            },
            residual,
            SpaceCategory::UnallocatedArea,
        ),
    ];
    for (stub, rule, category) in cases {
        let outcome = evaluate(stub, &rule);
        assert_eq!(outcome.findings().len(), 1, "{category:?}");
        let message = &outcome.findings()[0].message;
        assert!(
            message.starts_with(&format!("{}: ", category.code())),
            "{message:?} should start with {}",
            category.code()
        );
    }
}

/// Each unallocated region is judged on its own: two 0.5 m² shafts pass a
/// 1 m² allowance that a 20 m² hole on the same storey fails, and the one
/// finding names the hole's area and the bodies around it.
#[test]
fn each_unallocated_region_is_judged_on_its_own() {
    let outcome = evaluate(
        Stub {
            residuals: Some(Ok(vec![
                (oid("storey-1"), 0.5, vec![oid("shaft-wall-1")]),
                (oid("storey-1"), 20.0, vec![oid("slab"), oid("space-1")]),
                (oid("storey-1"), 0.5, vec![oid("shaft-wall-2")]),
            ])),
            ..Stub::default()
        },
        &rule_with(&[(
            "check_unallocated_area",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert_eq!(outcome.findings().len(), 1);
    let finding = &outcome.findings()[0];
    assert_eq!(finding.object_id(), Some(&oid("storey-1")));
    assert!(finding.message.contains("20.000 m2"), "{}", finding.message);
    assert_eq!(finding.related, vec![oid("slab"), oid("space-1")]);
    // 20 m² against 1 m² is 19 times the allowance over: bands grade it.
    let deviation = outcome.deviation(0).expect("graded");
    assert!(deviation.lower() <= 19.0 && 19.0 <= deviation.upper());
    assert!(SpaceValidation.grades_deviation());
}

/// Without selectors the service's defaults bound and intersect a space: the
/// requests carry no elements.
#[test]
fn without_element_selectors_the_service_defaults_are_used() {
    let stub = Arc::new(Stub::default());
    evaluate_shared(&stub, &rule());
    let boundary = stub.boundary_requests.lock().unwrap();
    let overlap = stub.overlap_requests.lock().unwrap();
    assert_eq!(boundary.len(), 1);
    assert_eq!(boundary[0].elements(), None);
    assert_eq!(overlap.len(), 1);
    assert_eq!(overlap[0].elements(), None);
}

/// `boundary_elements` and `intersection_elements` state the elements that
/// bound and intersect a space; each request carries exactly its selection.
#[test]
fn element_selectors_choose_the_bounding_and_intersecting_elements() {
    let stub = Arc::new(Stub::default());
    evaluate_shared(
        &stub,
        &rule_with(&[
            ("boundary_elements", of_type("covering")),
            ("intersection_elements", of_type("slab")),
        ]),
    );
    assert_eq!(
        stub.boundary_requests.lock().unwrap()[0].elements(),
        Some(&[oid("ceiling")][..])
    );
    assert_eq!(
        stub.overlap_requests.lock().unwrap()[0].elements(),
        Some(&[oid("slab")][..])
    );
}

/// A selector selecting nothing leaves nothing to bound or intersect a
/// space, so that sub-check is skipped rather than reporting every boundary.
#[test]
fn an_element_selector_selecting_nothing_skips_its_sub_check() {
    let stub = Arc::new(Stub {
        gaps: Some(Ok(vec![(2.0, Vec::new())])),
        overlaps: Some(Ok(vec![(false, 2.0, 1.0, Containment::Partial)])),
        ..Stub::default()
    });
    let outcome = evaluate_shared(
        &stub,
        &rule_with(&[
            ("boundary_elements", of_type("wall")),
            ("intersection_elements", of_type("wall")),
        ]),
    );
    assert!(stub.boundary_requests.lock().unwrap().is_empty());
    assert!(stub.overlap_requests.lock().unwrap().is_empty());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// An undecided selection may hide the covering or intersecting element: the
/// sub-check is not evaluated for each space, and the others still run.
#[test]
fn an_undecided_element_selection_leaves_only_its_sub_check_unevaluated() {
    let classified = || ParameterValue::Selector {
        value: Box::new(Selector::Classification {
            system: "uniclass".into(),
            code: Some("Ss_25".into()),
            code_pattern: None,
            include_descendants: false,
        }),
    };
    for key in ["boundary_elements", "intersection_elements"] {
        let stub = Arc::new(Stub {
            height: Some(Ok(2.0)),
            ..Stub::default()
        });
        let outcome = evaluate_shared(&stub, &rule_with(&[(key, classified())]));
        assert_eq!(outcome.findings().len(), 1, "the height still reports");
        let unevaluated = outcome.not_evaluated_outcomes();
        assert_eq!(unevaluated.len(), 1, "{key}");
        assert_eq!(unevaluated[0].object_id(), Some(&oid("space-1")));
        let prefix = key.trim_end_matches("_elements");
        assert!(unevaluated[0].message().starts_with(prefix), "{key}");
    }
}

#[test]
fn a_mistyped_element_selector_is_an_invalid_declaration() {
    for key in ["boundary_elements", "intersection_elements"] {
        let outcome = evaluate(
            Stub::default(),
            &rule_with(&[(key, ParameterValue::Boolean { value: true })]),
        );
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

/// The measured `clear_height` reads the height `space-validation` judges.
#[test]
fn the_measured_clear_height_is_the_one_judged() {
    use axioval_engine::{PropertyResolution, measured_value};
    use axioval_ir::{PropertyValue, QuantityDimension};
    let project = Project::new(vec![Object::new(oid("space-1"), "space")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(SpaceServiceHandle::new(Arc::new(Stub {
            height: Some(Ok(2.0)),
            ..Stub::default()
        })))
        .unwrap();
    let PropertyResolution::Present(height) =
        measured_value(&services, &project, &oid("space-1"), "clear_height").unwrap()
    else {
        panic!("no clear height");
    };
    assert_eq!(
        height.property().value(),
        &PropertyValue::Quantity {
            value: 2.0,
            dimension: QuantityDimension::Length
        }
    );
}

/// The measured value `name` of `object` over `stub`, as a plain number.
fn measured(stub: Stub, object: &str, name: &str) -> f64 {
    use axioval_engine::{PropertyResolution, measured_value};
    use axioval_ir::PropertyValue;
    let project = Project::new(vec![
        Object::new(oid("space-1"), "space"),
        Object::new(oid("storey"), "storey"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(SpaceServiceHandle::new(Arc::new(stub)))
        .unwrap();
    match measured_value(&services, &project, &oid(object), name).unwrap() {
        PropertyResolution::Present(resolved) => match resolved.property().value() {
            PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => *value,
            #[allow(clippy::cast_precision_loss)]
            PropertyValue::Integer(value) => *value as f64,
            other => panic!("{name}: {other:?}"),
        },
        PropertyResolution::Absent(_) => panic!("{name} of {object} is absent"),
    }
}

/// Each aspect of `space-validation` is a measured value and a comparison:
/// the capability finds exactly where the comparison fails.
#[test]
#[allow(clippy::too_many_lines)]
fn every_aspect_is_a_measured_value_and_a_comparison() {
    type Case = (
        &'static str,
        fn() -> Stub,
        Vec<(&'static str, ParameterValue)>,
        &'static str,
        &'static str,
        fn(f64) -> bool,
    );
    let cases: Vec<Case> = vec![
        (
            "no duplicate",
            Stub::default,
            vec![],
            "space-1",
            "duplicate_count",
            |n| n == 0.0,
        ),
        (
            "a duplicate",
            || Stub {
                duplicates: Some(Ok(vec![oid("space-2")])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "duplicate_count",
            |n| n == 0.0,
        ),
        (
            "a short gap",
            || Stub {
                gaps: Some(Ok(vec![(0.5, vec![])])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "boundary_gap;at_least=1",
            |gap| gap == 0.0,
        ),
        (
            "a long gap",
            || Stub {
                gaps: Some(Ok(vec![(0.5, vec![]), (1.5, vec![oid("wall")])])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "boundary_gap;at_least=1",
            |gap| gap == 0.0,
        ),
        (
            "a flat overlap",
            || Stub {
                overlaps: Some(Ok(vec![(true, 2.0, 0.001, Containment::Partial)])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "intersection_count",
            |n| n == 0.0,
        ),
        (
            "an intersecting space",
            || Stub {
                overlaps: Some(Ok(vec![(true, 2.0, 1.0, Containment::Partial)])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "intersection_count",
            |n| n == 0.0,
        ),
        (
            "a contained space",
            || Stub {
                overlaps: Some(Ok(vec![(true, 2.0, 0.0, Containment::SubjectInsideOther)])),
                ..Stub::default()
            },
            vec![],
            "space-1",
            "intersection_count",
            |n| n == 0.0,
        ),
        (
            "a half-covered top",
            || Stub {
                cap: Some(Ok((10.0, 5.0))),
                ..Stub::default()
            },
            vec![("check_top_cap", ParameterValue::Boolean { value: true })],
            "space-1",
            "cap_coverage;cap=top",
            |share| share >= 0.98,
        ),
        (
            "a covered top",
            Stub::default,
            vec![("check_top_cap", ParameterValue::Boolean { value: true })],
            "space-1",
            "cap_coverage;cap=top",
            |share| share >= 0.98,
        ),
        (
            "a large unallocated region",
            || Stub {
                residuals: Some(Ok(vec![(oid("storey"), 3.0, vec![])])),
                ..Stub::default()
            },
            vec![(
                "check_unallocated_area",
                ParameterValue::Boolean { value: true },
            )],
            "storey",
            "largest_unallocated_region",
            |area| area <= 1.0,
        ),
        (
            "a small unallocated region",
            || Stub {
                residuals: Some(Ok(vec![(oid("storey"), 0.5, vec![])])),
                ..Stub::default()
            },
            vec![(
                "check_unallocated_area",
                ParameterValue::Boolean { value: true },
            )],
            "storey",
            "largest_unallocated_region",
            |area| area <= 1.0,
        ),
    ];
    for (case, stub, overrides, object, name, holds) in cases {
        let outcome = evaluate(stub(), &rule_with(&overrides));
        let found = outcome
            .findings()
            .iter()
            .any(|finding| finding.object_id() == Some(&oid(object)));
        let value = measured(stub(), object, name);
        assert_eq!(!holds(value), found, "{case}: `{name}` is {value}");
    }
}

/// Every aspect as expression rules over the measured values, one rule per
/// severity the capability grades with (a cap shortfall is an error below
/// 1 %, a warning up to 15 % and informational below 98 %), the storey's
/// unallocated floor one rule over the storeys. Held to the parity harness
/// over the capability's fixtures, refusals included.
#[test]
#[allow(clippy::too_many_lines, clippy::type_complexity)]
fn the_aspects_as_expression_rules_hold_to_the_parity_harness() {
    use serde_json::{Value, json};
    let measured = |name: &str| json!({"kind": "property", "propertySet": "axioval:measured", "property": name});
    let number =
        |value: f64| json!({"kind": "literal", "value": {"type": "number", "value": value}});
    let quantity = |value: f64, unit: &str| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}});
    let compare = |operator: &str, left: Value, right: Value| json!({"kind": "compare", "operator": operator, "left": left, "right": right});
    let all = |operands: Vec<Value>| json!({"kind": "and", "operands": operands});
    // The top cap where a slab or a roof may form it, at least `share`.
    let top = |share: f64, operator: &str| {
        json!({"kind": "implies",
            "antecedent": {"kind": "or", "operands": [
                compare("greaterThan", measured("support_count"), number(0.0)),
                compare("greaterThan", measured("support_count;of=roofs"), number(0.0))]},
            "consequent": compare(operator, measured("cap_coverage;cap=top"), number(share))})
    };
    let truth = json!({"kind": "literal", "value": {"type": "boolean", "value": true}});
    let space_rules = |cap: bool| {
        let mut error = vec![
            compare("equals", measured("duplicate_count"), number(0.0)),
            compare(
                "equals",
                measured("intersection_count;tolerance=0.005"),
                number(0.0),
            ),
        ];
        let mut warning = vec![
            compare(
                "greaterThanOrEquals",
                measured("clear_height"),
                quantity(2.495, "m"),
            ),
            compare(
                "equals",
                measured("boundary_gap;at_least=1"),
                quantity(0.0, "m"),
            ),
        ];
        let mut info = vec![truth.clone()];
        if cap {
            error.push(top(0.01, "greaterThanOrEquals"));
            warning.push(top(0.15, "greaterThan"));
            info.push(top(0.98, "greaterThanOrEquals"));
        }
        vec![
            (Severity::Error, all(error)),
            (Severity::Warning, all(warning)),
            (Severity::Info, all(info)),
        ]
    };
    let storey_rule = |share: Option<f64>| {
        let mut checks = vec![compare(
            "lessThanOrEquals",
            measured("largest_unallocated_region"),
            quantity(if share.is_some() { 1000.0 } else { 1.0 }, "m2"),
        )];
        if let Some(share) = share {
            checks.push(compare(
                "lessThanOrEquals",
                measured("unallocated_share"),
                number(share),
            ));
        }
        all(checks)
    };
    let enabled = |name: &str| (name.to_owned(), ParameterValue::Boolean { value: true });
    let residuals = |regions: &[f64], floor: Option<f64>| Stub {
        residuals: Some(Ok(regions
            .iter()
            .map(|area| (oid("storey-1"), *area, Vec::new()))
            .collect())),
        floor,
        ..Stub::default()
    };
    let shares = vec![
        enabled("check_unallocated_area"),
        (
            "maximum_unallocated_area_square_metres".to_owned(),
            ParameterValue::Number { value: 1000.0 },
        ),
        (
            "maximum_unallocated_share".to_owned(),
            ParameterValue::Number { value: 0.03 },
        ),
    ];
    let cases: Vec<(&str, Stub, Vec<(String, ParameterValue)>)> = vec![
        ("clean", Stub::default(), vec![]),
        (
            "duplicate",
            Stub {
                duplicates: Some(Ok(vec![oid("space-2")])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "low",
            Stub {
                height: Some(Ok(2.0)),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "low within the tolerance",
            Stub {
                height: Some(Ok(2.4995)),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "unavailable height beside a duplicate",
            Stub {
                height: Some(Err(SpaceError::Unavailable)),
                duplicates: Some(Ok(vec![oid("space-2")])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "unavailable height",
            Stub {
                height: Some(Err(SpaceError::Unavailable)),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "short gap",
            Stub {
                gaps: Some(Ok(vec![(0.4, vec![oid("w1")])])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "long gap",
            Stub {
                gaps: Some(Ok(vec![(0.5, vec![]), (2.0, vec![oid("w1")])])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "contained",
            Stub {
                overlaps: Some(Ok(vec![(false, 0.0, 0.0, Containment::SubjectInsideOther)])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "intersecting",
            Stub {
                overlaps: Some(Ok(vec![(true, 2.0, 1.0, Containment::Partial)])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "touching",
            Stub {
                overlaps: Some(Ok(vec![(false, 1.0, 0.001, Containment::Partial)])),
                ..Stub::default()
            },
            vec![],
        ),
        (
            "nearly covered top",
            Stub {
                cap: Some(Ok((10.0, 9.9))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "bare top",
            Stub {
                cap: Some(Ok((10.0, 0.05))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "barely covered top",
            Stub {
                cap: Some(Ok((10.0, 1.0))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "half-covered top",
            Stub {
                cap: Some(Ok((10.0, 5.0))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "nothing to form the top",
            Stub {
                support: Some(Ok((0, 0))),
                cap: Some(Ok((10.0, 0.0))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "unmeasured top",
            Stub {
                cap: Some(Err(SpaceError::unmeasured(
                    SpaceAspect::CapCoverage(Cap::Top),
                    vec![oid("roof")],
                ))),
                ..Stub::default()
            },
            vec![enabled("check_top_cap")],
        ),
        (
            "large region",
            residuals(&[5.0], None),
            vec![enabled("check_unallocated_area")],
        ),
        (
            "small region",
            residuals(&[0.5], None),
            vec![enabled("check_unallocated_area")],
        ),
        (
            "5 % unallocated",
            residuals(&[5.0], Some(100.0)),
            shares.clone(),
        ),
        (
            "2 % unallocated",
            residuals(&[2.0], Some(100.0)),
            shares.clone(),
        ),
        (
            "4 % in two regions",
            residuals(&[2.0, 2.0], Some(100.0)),
            shares.clone(),
        ),
        ("no gross area", residuals(&[5.0], None), shares.clone()),
    ];
    let project = Project::new(vec![
        Object::new(oid("space-1"), "space"),
        Object::new(oid("ceiling"), "covering"),
        Object::new(oid("slab"), "slab"),
        Object::new(oid("storey-1"), "storey"),
    ])
    .unwrap();
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    for (case, stub, overrides) in cases {
        let overrides: Vec<(&str, ParameterValue)> = overrides
            .iter()
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect();
        let declared = rule_with(&overrides);
        let cap = overrides.iter().any(|(name, _)| *name == "check_top_cap");
        let share = overrides
            .iter()
            .any(|(name, _)| *name == "maximum_unallocated_share");
        let unallocated = overrides
            .iter()
            .any(|(name, _)| *name == "check_unallocated_area");
        let stub = Arc::new(stub);
        let mut services = ServiceRegistry::new();
        services
            .register(SpaceServiceHandle::new(stub.clone()))
            .unwrap();
        let expected = SpaceValidation.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &declared,
        );
        registry.install_measured(&mut services, &project);
        // The expression reads the measured set as a run answers it.
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(
                MeasuredOnly {
                    services: {
                        let mut inner = ServiceRegistry::new();
                        inner.register(SpaceServiceHandle::new(stub)).unwrap();
                        registry.install_measured(&mut inner, &project);
                        inner
                    },
                    project: project.clone(),
                },
            )))
            .unwrap();
        let mut rules: Vec<(Severity, &str, Value)> = space_rules(cap)
            .into_iter()
            .map(|(severity, requirement)| (severity, "space", requirement))
            .collect();
        if unallocated {
            rules.push((
                Severity::Warning,
                "storey",
                storey_rule(share.then_some(0.03)),
            ));
        }
        let mut rewritten = axioval_engine::CapabilityEvaluation::default();
        for (severity, of, requirement) in rules {
            let expression = CompiledRule {
                capability: "axioval:capability.expression".into(),
                severity: match severity {
                    Severity::Error => RuleSeverity::Error,
                    Severity::Warning => RuleSeverity::Warning,
                    Severity::Info => RuleSeverity::Info,
                },
                selector: Selector::EntityType {
                    object_type: of.into(),
                    include_subtypes: false,
                },
                parameters: BTreeMap::from([(
                    "requirement".to_owned(),
                    ParameterValue::Expression {
                        value: serde_json::from_value(requirement).unwrap(),
                    },
                )]),
                ..declared.clone()
            };
            rewritten.absorb(axioval_rules::ExpressionRequirement.evaluate(
                &RuleContext {
                    project: &project,
                    services: &services,
                },
                &expression,
            ));
        }
        let parity = axioval_rules::parity::compare_evaluations(
            ("space-validation", &expected),
            ("expression", &rewritten),
        );
        assert!(
            parity.holds(),
            "{case}:\n{}\n{:?}",
            parity.diff(),
            rewritten.not_evaluated_outcomes()
        );
    }
}

/// The measured set answered as a run answers it, and nothing else.
struct MeasuredOnly {
    services: ServiceRegistry,
    project: Project,
}

impl PropertyResolutionService for MeasuredOnly {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if request.property_set() != Some(axioval_ir::MEASURED_SET) {
            return Err(PropertyResolutionError::Unavailable(
                "only the measured set is answered".into(),
            ));
        }
        axioval_engine::measured_value(
            &self.services,
            &self.project,
            request.object_id(),
            request.property(),
        )
    }
}
