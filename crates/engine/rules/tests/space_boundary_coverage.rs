//! `space-boundary-coverage`: declared boundaries against each space's surface.
//!
//! The capability runs as a template; every fixture runs it and the
//! implementation it replaced (`axioval_rules::reference::SpaceBoundaryCoverage`)
//! and holds the template to its whole outside contract.
#![allow(missing_docs, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryCoverageServiceHandle, BoundaryOverlap, BoundaryPlacement, CapabilityEvaluation,
    CoverageAreas, MeasuredBoundary, SurfaceAreaInterval,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::SpaceBoundaryCoverage;
use axioval_rules::reference::SpaceBoundaryCoverage as Reference;
use common::{Model, findings, id, kind, number, rule, source, unevaluated};

const ID: &str = "axioval:capability.space-boundary-coverage";

/// One space's canned answer.
#[derive(Clone, Debug)]
struct Answer {
    surface: (f64, f64),
    uncovered: (f64, f64),
    overlap: (f64, f64),
    boundaries: Vec<MeasuredBoundary>,
    overlaps: Vec<BoundaryOverlap>,
}

fn interval((lower, upper): (f64, f64)) -> SurfaceAreaInterval {
    SurfaceAreaInterval::try_new(lower, upper).unwrap()
}

fn exact(surface: f64, uncovered: f64, overlap: f64) -> Answer {
    Answer {
        surface: (surface, surface),
        uncovered: (uncovered, uncovered),
        overlap: (overlap, overlap),
        boundaries: Vec::new(),
        overlaps: Vec::new(),
    }
}

fn on(boundary: &str, element: &str, area: f64) -> MeasuredBoundary {
    MeasuredBoundary::new(
        id(boundary),
        Some(id(element)),
        BoundaryPlacement::OnSurface {
            area: interval((area, area)),
        },
    )
}

/// Answers per space; a space without one is refused, and `seen` records
/// each request's tolerance.
#[derive(Default)]
struct Coverages {
    answers: BTreeMap<ObjectId, Answer>,
    tolerance: std::sync::Mutex<Vec<f64>>,
    /// Spaces measured on a tessellation: inexact even where a point.
    inexact: std::collections::BTreeSet<ObjectId>,
}

impl Coverages {
    fn with(mut self, space: &str, answer: Answer) -> Self {
        self.answers.insert(id(space), answer);
        self
    }

    fn inexact(mut self, space: &str) -> Self {
        self.inexact.insert(id(space));
        self
    }
}

impl BoundaryCoverageService for Coverages {
    fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        self.tolerance
            .lock()
            .unwrap()
            .push(request.plane_tolerance_metres());
        let answer = self.answers.get(request.space()).ok_or_else(|| {
            BoundaryCoverageError::Unavailable("a boundary surface is a face surface".into())
        })?;
        let (surface, uncovered) = (answer.surface, answer.uncovered);
        let covered = (
            (surface.0 - uncovered.1).max(0.0),
            (surface.1 - uncovered.0).max(0.0),
        );
        let point = surface.0 == surface.1 && uncovered.0 == uncovered.1;
        let mut evidence =
            Evidence::exact(source(), format!("coverage:{}", request.space().local_id));
        evidence.exact = point
            && answer.overlap.0 == answer.overlap.1
            && !self.inexact.contains(request.space());
        BoundaryCoverage::try_new(
            request.clone(),
            CoverageAreas {
                surface: interval(surface),
                covered: interval(covered),
                uncovered: interval(uncovered),
                overlap: interval(answer.overlap),
            },
            answer.boundaries.clone(),
            answer.overlaps.clone(),
            evidence,
        )
    }
}

fn model() -> Model {
    Model::default()
        .object("whole", "space")
        .object("gappy", "space")
        .object("overlapping", "space")
        .object("misplaced", "space")
        .object("vague", "space")
        .object("face-surface", "space")
}

fn coverages() -> Coverages {
    let mut overlapping = exact(59.0, 0.0, 3.0);
    overlapping.boundaries = vec![on("b1", "slab", 7.5), on("b2", "slab", 7.5)];
    overlapping.overlaps =
        vec![BoundaryOverlap::try_new(id("b2"), id("b1"), interval((3.0, 3.0))).unwrap()];
    let mut misplaced = exact(59.0, 0.0, 0.0);
    misplaced.boundaries = vec![
        on("b3", "wall", 59.0),
        MeasuredBoundary::new(id("b4"), Some(id("column")), BoundaryPlacement::OffSurface),
    ];
    let vague = Answer {
        surface: (58.9, 59.1),
        uncovered: (0.0, 0.2),
        overlap: (0.0, 0.2),
        boundaries: Vec::new(),
        overlaps: Vec::new(),
    };
    Coverages::default()
        .with("whole", exact(59.0, 0.0, 0.0))
        .with("gappy", exact(59.0, 7.5, 0.0))
        .with("overlapping", overlapping)
        .with("misplaced", misplaced)
        .with("vague", vague)
}

/// The template's evaluation of `rule` over `model`, held to the replaced
/// implementation's whole contract.
#[allow(clippy::needless_pass_by_value)]
fn held(
    model: Model,
    coverages: Option<Arc<Coverages>>,
    rule: &axioval_engine::CompiledRule,
) -> CapabilityEvaluation {
    model.holding_contract(
        &SpaceBoundaryCoverage,
        &Reference,
        rule,
        |services| {
            if let Some(coverages) = &coverages {
                services
                    .register(BoundaryCoverageServiceHandle::new(coverages.clone()))
                    .unwrap();
            }
        },
        &[],
        0.0,
    )
}

fn run(coverages: Coverages, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    held(
        model(),
        Some(Arc::new(coverages)),
        &rule(ID, kind("space"), parameters),
    )
}

fn area(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m2".into(),
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

#[test]
fn a_space_short_of_its_share_is_found_and_a_straddling_one_is_not_evaluated() {
    let evaluation = run(coverages(), vec![("minimum_covered_share", number(0.999))]);
    let found = findings(&evaluation);
    assert!(
        found.contains(&(
            "gappy".into(),
            "declared boundaries cover 87.29% of the 59 m² surface, leaving 7.5 m² uncovered; \
         at least 99.9% required"
                .into()
        ))
    );
    assert!(found.contains(&(
        "misplaced".into(),
        format!(
            "space boundary {} lies on no face of the space's body, so it covers nothing",
            id("b4")
        )
    )));
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        unevaluated(&evaluation),
        [
            (
                "face-surface".into(),
                NotEvaluatedReason::BackendUnavailable
            ),
            ("vague".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    let misplaced = evaluation
        .findings()
        .iter()
        .find(|finding| finding.object_id() == Some(&id("misplaced")))
        .unwrap();
    assert_eq!(misplaced.related, vec![id("column")]);
}

#[test]
fn gaps_and_overlaps_are_judged_against_their_maxima() {
    let evaluation = run(
        coverages(),
        vec![
            ("maximum_uncovered_area", area(1.0)),
            ("maximum_overlap_area", area(0.5)),
        ],
    );
    let found = findings(&evaluation);
    assert!(
        found.contains(&(
            "gappy".into(),
            "declared boundaries leave 7.5 m² of the 59 m² surface uncovered; at most 1 m² allowed"
                .into()
        ))
    );
    let overlap = format!(
        "declared boundaries overlap over 3 m² of the surface (boundaries {} and {}); \
         at most 0.5 m² allowed",
        id("b1"),
        id("b2")
    );
    assert!(
        found.contains(&("overlapping".into(), overlap)),
        "{found:?}"
    );
    let overlapping = evaluation
        .findings()
        .iter()
        .find(|finding| finding.object_id() == Some(&id("overlapping")))
        .unwrap();
    assert_eq!(overlapping.related, vec![id("slab")]);
    // The vague space's gap and overlap both lie within their maxima.
    assert!(!found.iter().any(|(space, _)| space == "vague"));
}

#[test]
fn the_plane_tolerance_is_sent_and_defaults_to_zero() {
    let coverages = Arc::new(coverages());
    held(
        model(),
        Some(Arc::clone(&coverages)),
        &rule(
            ID,
            kind("space"),
            vec![
                ("maximum_overlap_area", area(1.0)),
                ("plane_tolerance", metres(0.005)),
            ],
        ),
    );
    assert!(
        coverages
            .tolerance
            .lock()
            .unwrap()
            .iter()
            .all(|t| *t == 0.005)
    );

    let coverages = Arc::new(Coverages::default());
    held(
        model(),
        Some(Arc::clone(&coverages)),
        &rule(ID, kind("space"), vec![("maximum_overlap_area", area(1.0))]),
    );
    let seen = coverages.tolerance.lock().unwrap();
    assert!(!seen.is_empty() && seen.iter().all(|t| *t == 0.0));
}

#[test]
fn bad_declarations_and_a_missing_service_judge_nothing() {
    for (parameters, refused) in [
        (
            vec![],
            "`minimum_covered_share`, `maximum_uncovered_area` or `maximum_overlap_area` is \
             required",
        ),
        (
            vec![("minimum_covered_share", number(1.5))],
            "`minimum_covered_share` lies outside 0 to 1",
        ),
        (
            vec![("minimum_covered_share", number(-0.5))],
            "`minimum_covered_share` lies outside 0 to 1",
        ),
        (
            vec![("maximum_uncovered_area", metres(1.0))],
            "`maximum_uncovered_area` is not an area",
        ),
        (
            vec![("maximum_uncovered_area", metres(-1.0))],
            "`maximum_uncovered_area` is not an area",
        ),
        (
            vec![("maximum_overlap_area", area(-1.0))],
            "`maximum_overlap_area` is negative",
        ),
        (
            vec![
                ("maximum_overlap_area", area(1.0)),
                ("plane_tolerance", area(1.0)),
            ],
            "`plane_tolerance` is not a length",
        ),
        (
            vec![
                ("maximum_overlap_area", area(1.0)),
                ("plane_tolerance", metres(-0.1)),
            ],
            "`plane_tolerance` is negative",
        ),
    ] {
        let evaluation = run(coverages(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            format!("space-boundary-coverage: {refused}")
        );
    }
    let evaluation = held(
        model(),
        None,
        &rule(ID, kind("space"), vec![("maximum_overlap_area", area(1.0))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "space-boundary coverage service is not registered"
    );
}

/// The measured share, uncovered and overlapping areas against the
/// capability's bounds reach its verdicts, a straddling one left open.
#[test]
fn the_measured_coverage_reaches_the_verdicts() {
    let project = model().project();
    let mut services = axioval_engine::ServiceRegistry::new();
    services
        .register(BoundaryCoverageServiceHandle::new(Arc::new(coverages())))
        .unwrap();
    let verdict = |evaluation: &CapabilityEvaluation, space: &str| {
        if findings(evaluation)
            .iter()
            .any(|(object, _)| object == space)
        {
            Some(false)
        } else if unevaluated(evaluation)
            .iter()
            .any(|(object, _)| object == space)
        {
            None
        } else {
            Some(true)
        }
    };
    let read = |space: &str, name: &str| {
        common::measured(&services, &project, &id(space), name)
            .unwrap()
            .unwrap()
    };
    let share = run(coverages(), vec![("minimum_covered_share", number(0.999))]);
    let uncovered = run(coverages(), vec![("maximum_uncovered_area", area(1.0))]);
    let overlap = run(coverages(), vec![("maximum_overlap_area", area(1.0))]);
    let at_most =
        |(lower, upper): (f64, f64), bound: f64| common::at_least((-upper, -lower), -bound);
    // `misplaced` is found for an off-surface boundary whatever its share.
    for space in ["whole", "gappy", "overlapping", "vague"] {
        assert_eq!(
            common::at_least(read(space, "boundary_covered_share"), 0.999),
            verdict(&share, space),
            "{space} share"
        );
        assert_eq!(
            at_most(read(space, "boundary_uncovered_area"), 1.0),
            verdict(&uncovered, space),
            "{space} uncovered"
        );
        assert_eq!(
            at_most(read(space, "boundary_overlap_area"), 1.0),
            verdict(&overlap, space),
            "{space} overlap"
        );
    }
}

/// Each check as an expression rule over the measured share, uncovered
/// and overlapping areas, and no boundary off the surface, held to the
/// parity harness on every space of the fixture.
#[test]
fn the_checks_as_expressions_hold_to_the_parity_harness() {
    use serde_json::{Value, json};
    let measured = |name: &str| json!({"kind": "property", "propertySet": "axioval:measured", "property": name});
    let bound = |operator: &str, name: &str, value: Value| json!({"kind": "compare", "operator": operator, "left": measured(name), "right": value});
    let quantity = |value: f64, unit: &str| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}});
    let checks = [
        (
            vec![("minimum_covered_share", number(0.999))],
            bound(
                "greaterThanOrEquals",
                "boundary_covered_share",
                json!({"kind": "literal", "value": {"type": "number", "value": 0.999}}),
            ),
        ),
        (
            vec![("maximum_uncovered_area", area(1.0))],
            bound(
                "lessThanOrEquals",
                "boundary_uncovered_area",
                quantity(1.0, "m2"),
            ),
        ),
        (
            vec![("maximum_overlap_area", area(0.5))],
            bound(
                "lessThanOrEquals",
                "boundary_overlap_area",
                quantity(0.5, "m2"),
            ),
        ),
    ];
    // An off-surface boundary is a finding whatever the check.
    let on_surface = bound(
        "equals",
        "boundary_off_surface_count",
        json!({"kind": "literal", "value": {"type": "number", "value": 0.0}}),
    );
    for (parameters, check) in checks {
        let requirement = json!({"kind": "and", "operands": [on_surface.clone(), check]});
        let expected = run(coverages(), parameters.clone());
        let expression = rule(
            "axioval:capability.expression",
            kind("space"),
            vec![("requirement", common::expression(requirement.clone()))],
        );
        let outcome = model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &expression,
            |services| {
                services
                    .register(BoundaryCoverageServiceHandle::new(Arc::new(coverages())))
                    .unwrap();
            },
        );
        let parity =
            axioval_rules::parity::compare_evaluations((ID, &expected), ("expression", &outcome));
        assert!(parity.holds(), "{requirement}:\n{}", parity.diff());
    }
}

/// Everything one space leaves open is one outcome, its checks' messages
/// joined in their order.
#[test]
fn what_a_space_leaves_open_is_one_outcome() {
    let evaluation = run(
        coverages(),
        vec![
            ("minimum_covered_share", number(0.999)),
            ("maximum_uncovered_area", area(0.1)),
            ("maximum_overlap_area", area(0.1)),
        ],
    );
    let vague: Vec<&str> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("vague")))
        .map(axioval_engine::CapabilityNotEvaluated::message)
        .collect();
    assert_eq!(
        vague,
        [
            "declared boundaries cover between 99.32% and 100% of the between 58.9 m² and 59.1 \
             m² surface, leaving between 0 m² and 0.2 m² uncovered, which straddles the \
             required 99.9%; declared boundaries leave between 0 m² and 0.2 m² of the between \
             58.9 m² and 59.1 m² surface uncovered, which straddles the allowed 0.1 m²; \
             declared boundaries overlap over between 0 m² and 0.2 m² of the surface, which \
             straddles the allowed 0.1 m²"
        ]
    );
}

/// A coverage measured on a tessellation is never exact, whichever value
/// reads it, a point included; one measured exactly is.
#[test]
fn coverage_measured_approximately_is_never_exact() {
    let project = model().project();
    let mut services = axioval_engine::ServiceRegistry::new();
    services
        .register(BoundaryCoverageServiceHandle::new(Arc::new(coverages())))
        .unwrap();
    for name in [
        "boundary_coverage_off",
        "boundary_coverage_share",
        "boundary_coverage_uncovered",
        "boundary_coverage_overlap",
        "boundary_coverage_surface",
    ] {
        for (space, exact) in [("whole", true), ("vague", false)] {
            let (_, cited) = common::measured_cited(&services, &project, &id(space), name)
                .unwrap()
                .unwrap_or_else(|| panic!("{name} of {space}"));
            assert_eq!(cited, exact, "{name} of {space}");
        }
    }
    // The share of an approximate surface is a point here, and still inexact.
    let mut flat = exact(59.0, 0.0, 0.0);
    flat.boundaries = vec![on("b1", "slab", 59.0)];
    let mut services = axioval_engine::ServiceRegistry::new();
    services
        .register(BoundaryCoverageServiceHandle::new(Arc::new(
            Coverages::default().with("whole", flat).inexact("whole"),
        )))
        .unwrap();
    let (_, cited) =
        common::measured_cited(&services, &project, &id("whole"), "boundary_coverage_share")
            .unwrap()
            .unwrap();
    assert!(!cited);
}

/// The forked rule, an expression requiring no boundary off the surface
/// and every declared check, reaches the template's verdicts on every
/// space: found, open or passed.
#[test]
fn the_forked_rule_reaches_the_templates_verdicts() {
    use axioval_rules::ExpressionRequirement;
    use axioval_rules::templates::{Fork, fork};
    let verdicts = |evaluation: &CapabilityEvaluation| {
        let mut found: Vec<ObjectId> = evaluation
            .findings()
            .iter()
            .filter_map(|finding| finding.object_id().cloned())
            .collect();
        found.sort();
        found.dedup();
        let mut open: Vec<ObjectId> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .filter(|object| !found.contains(object))
            .collect();
        open.sort();
        open.dedup();
        (found, open)
    };
    for parameters in [
        vec![("minimum_covered_share", number(0.999))],
        vec![("maximum_uncovered_area", area(1.0))],
        vec![
            ("maximum_overlap_area", area(0.5)),
            ("plane_tolerance", metres(0.01)),
        ],
        vec![
            ("minimum_covered_share", number(0.5)),
            ("maximum_uncovered_area", area(0.1)),
            ("maximum_overlap_area", area(0.1)),
        ],
    ] {
        let bound = rule(ID, kind("space"), parameters);
        let forked = fork(&SpaceBoundaryCoverage, &bound).unwrap();
        let mut expression_rule = bound.clone();
        expression_rule.capability = Fork::CAPABILITY.into();
        expression_rule.parameters = forked.parameters();
        let shared = Arc::new(coverages());
        let register = |services: &mut axioval_engine::ServiceRegistry| {
            services
                .register(BoundaryCoverageServiceHandle::new(shared.clone()))
                .unwrap();
        };
        let template = model().evaluate_measured(&SpaceBoundaryCoverage, &bound, register);
        let forked = model().evaluate_measured(&ExpressionRequirement, &expression_rule, register);
        assert_eq!(
            verdicts(&template),
            verdicts(&forked),
            "{:?}",
            bound.parameters
        );
    }
}

mod generated {
    use super::*;
    use proptest::collection::vec;
    use proptest::prelude::*;

    /// One space's answer: its surface, the uncovered and overlapping parts
    /// (some intervals), its boundaries (some off the surface or without an
    /// element) and the pairs overlapping, surely or possibly; or none, a
    /// refusal.
    fn answer() -> impl Strategy<Value = Option<(Answer, bool)>> {
        let part = (0u32..40, 0u32..3)
            .prop_map(|(low, width)| (f64::from(low) / 4.0, f64::from(low + width) / 4.0));
        let boundary = (0u32..4, any::<bool>(), proptest::option::of(0u32..3)).prop_map(
            |(index, on_surface, element)| {
                let name = format!("b{index}");
                let element = element.map(|element| id(&format!("e{element}")));
                if on_surface {
                    MeasuredBoundary::new(
                        id(&name),
                        element,
                        BoundaryPlacement::OnSurface {
                            area: interval((2.0, 2.0)),
                        },
                    )
                } else {
                    MeasuredBoundary::new(id(&name), element, BoundaryPlacement::OffSurface)
                }
            },
        );
        proptest::option::weighted(
            0.85,
            (
                30u32..60,
                part.clone(),
                part,
                vec(boundary, 0..4),
                vec((0u32..4, 0u32..4, any::<bool>()), 0..3),
                any::<bool>(),
            )
                .prop_map(|(surface, uncovered, overlap, boundaries, pairs, exact)| {
                    let mut boundaries = boundaries;
                    boundaries.sort_by(|a, b| a.boundary().cmp(b.boundary()));
                    boundaries.dedup_by(|a, b| a.boundary() == b.boundary());
                    let overlaps = pairs
                        .into_iter()
                        .filter(|(first, second, _)| first < second)
                        .filter_map(|(first, second, sure)| {
                            BoundaryOverlap::try_new(
                                id(&format!("b{first}")),
                                id(&format!("b{second}")),
                                interval((if sure { 0.5 } else { 0.0 }, 1.0)),
                            )
                            .ok()
                        })
                        .collect();
                    let surface = f64::from(surface);
                    (
                        Answer {
                            surface: (surface, surface),
                            uncovered: (uncovered.0.min(surface), uncovered.1.min(surface)),
                            overlap,
                            boundaries,
                            overlaps,
                        },
                        exact,
                    )
                }),
        )
    }

    fn parameters() -> impl Strategy<Value = Vec<(&'static str, ParameterValue)>> {
        (
            proptest::option::of(0u32..=100),
            proptest::option::of(0u32..40),
            proptest::option::of(0u32..6),
            proptest::option::of(0u32..3),
        )
            .prop_map(|(share, uncovered, overlap, plane)| {
                let mut parameters = Vec::new();
                if let Some(share) = share {
                    parameters.push(("minimum_covered_share", number(f64::from(share) / 100.0)));
                }
                if let Some(uncovered) = uncovered {
                    parameters.push(("maximum_uncovered_area", area(f64::from(uncovered) / 4.0)));
                }
                if let Some(overlap) = overlap {
                    parameters.push(("maximum_overlap_area", area(f64::from(overlap) / 4.0)));
                }
                if let Some(plane) = plane {
                    parameters.push(("plane_tolerance", metres(f64::from(plane) / 100.0)));
                }
                parameters
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_spaces_hold_parity(
            answers in vec(answer(), 1..5),
            parameters in parameters(),
        ) {
            let mut model = Model::default();
            let mut coverages = Coverages::default();
            for (index, answer) in answers.into_iter().enumerate() {
                let space = format!("s{index}");
                model = model.object(&space, "space");
                if let Some((answer, exact)) = answer {
                    coverages = coverages.with(&space, answer);
                    if !exact {
                        coverages = coverages.inexact(&space);
                    }
                }
            }
            held(model, Some(Arc::new(coverages)), &rule(ID, kind("space"), parameters));
        }
    }
}
