//! `space-boundary-coverage`: declared boundaries against each space's surface.
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
use common::{Model, findings, id, kind, number, rule, source, unevaluated};

const ID: &str = "axioval:capability.space-boundary-coverage";

/// One space's canned answer.
#[derive(Clone)]
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
}

impl Coverages {
    fn with(mut self, space: &str, answer: Answer) -> Self {
        self.answers.insert(id(space), answer);
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
        evidence.exact = point && answer.overlap.0 == answer.overlap.1;
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

fn run(coverages: Coverages, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model().evaluate_with(
        &SpaceBoundaryCoverage,
        &rule(ID, kind("space"), parameters),
        |services| {
            services
                .register(BoundaryCoverageServiceHandle::new(Arc::new(coverages)))
                .unwrap();
        },
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
    let shared = Arc::clone(&coverages);
    model().evaluate_with(
        &SpaceBoundaryCoverage,
        &rule(
            ID,
            kind("space"),
            vec![
                ("maximum_overlap_area", area(1.0)),
                ("plane_tolerance", metres(0.005)),
            ],
        ),
        |services| {
            services
                .register(BoundaryCoverageServiceHandle::new(shared))
                .unwrap();
        },
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
    let shared = Arc::clone(&coverages);
    model().evaluate_with(
        &SpaceBoundaryCoverage,
        &rule(ID, kind("space"), vec![("maximum_overlap_area", area(1.0))]),
        |services| {
            services
                .register(BoundaryCoverageServiceHandle::new(shared))
                .unwrap();
        },
    );
    let seen = coverages.tolerance.lock().unwrap();
    assert!(!seen.is_empty() && seen.iter().all(|t| *t == 0.0));
}

#[test]
fn bad_declarations_and_a_missing_service_judge_nothing() {
    for parameters in [
        vec![],
        vec![("minimum_covered_share", number(1.5))],
        vec![("maximum_uncovered_area", metres(1.0))],
        vec![("maximum_overlap_area", area(-1.0))],
        vec![
            ("maximum_overlap_area", area(1.0)),
            ("plane_tolerance", area(1.0)),
        ],
    ] {
        assert_eq!(
            unevaluated(&run(coverages(), parameters)),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    let evaluation = model().evaluate(
        &SpaceBoundaryCoverage,
        &rule(ID, kind("space"), vec![("maximum_overlap_area", area(1.0))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
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
