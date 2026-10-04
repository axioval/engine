//! Spacing of stacked slabs from their measured tops and bottoms.
#![allow(missing_docs)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, ElevationInterval, GeometryFidelity, ObjectBounds,
    PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle, ProximityError,
    ProximityEvidence, ProximityRequest, ProximityService, ProximityServiceHandle, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::SlabStackSpacing;
use common::{Model, findings, id, kind, number, rule, source, strings, unevaluated};

const ID: &str = "axioval:capability.slab-stack-spacing";

/// A slab: plan rectangle `(x0, y0, x1, y1)`, bottom and top elevation, and
/// the chord deviation of a tessellated mesh (`None` when exact).
#[derive(Clone, Copy)]
struct Slab {
    plan: [f64; 4],
    bottom: f64,
    top: f64,
    deviation: Option<f64>,
}

/// Geometry for the slab stubs, with a log of every overlap measured.
#[derive(Default)]
struct Slabs {
    slabs: BTreeMap<ObjectId, Slab>,
    overlaps: Mutex<BTreeSet<(String, String)>>,
}

impl Slabs {
    fn with(mut self, local: &str, plan: [f64; 4], bottom: f64, top: f64) -> Self {
        self.slabs.insert(
            id(local),
            Slab {
                plan,
                bottom,
                top,
                deviation: None,
            },
        );
        self
    }

    fn tessellated(mut self, local: &str, deviation: f64) -> Self {
        self.slabs.get_mut(&id(local)).unwrap().deviation = Some(deviation);
        self
    }

    fn get(&self, object: &ObjectId) -> Option<Slab> {
        self.slabs.get(object).copied()
    }
}

impl VerticalExtentService for Slabs {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let slab = self
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let d = slab.deviation.unwrap_or(0.0);
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact = slab.deviation.is_none();
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(slab.bottom - d, slab.bottom + d)?,
            ElevationInterval::try_new(slab.top - d, slab.top + d)?,
            evidence,
        )
    }
}

fn area(value: f64, slack: f64, locator: String) -> Result<PlanArea, PlanAreaError> {
    let mut evidence = Evidence::exact(source(), locator);
    evidence.exact = slack == 0.0;
    PlanArea::try_new((value - slack).max(0.0), value + slack, evidence)
}

impl PlanAreaService for Slabs {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let slab = self
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let [x0, y0, x1, y1] = slab.plan;
        let slack = slab.deviation.unwrap_or(0.0);
        area((x1 - x0) * (y1 - y0), slack, format!("footprint:{object}"))
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        self.overlaps
            .lock()
            .unwrap()
            .insert((first.local_id.clone(), second.local_id.clone()));
        let a = self
            .get(first)
            .ok_or_else(|| PlanAreaError::UnknownObject(first.clone()))?;
        let b = self
            .get(second)
            .ok_or_else(|| PlanAreaError::UnknownObject(second.clone()))?;
        let width = (a.plan[2].min(b.plan[2]) - a.plan[0].max(b.plan[0])).max(0.0);
        let depth = (a.plan[3].min(b.plan[3]) - a.plan[1].max(b.plan[1])).max(0.0);
        area(
            width * depth,
            a.deviation.unwrap_or(0.0) + b.deviation.unwrap_or(0.0),
            format!("overlap:{first}:{second}"),
        )
    }
}

impl ProximityService for Slabs {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let slab = self.get(object).ok_or(ProximityError::Unavailable)?;
        let [x0, y0, x1, y1] = slab.plan;
        let fidelity = match slab.deviation {
            None => GeometryFidelity::Exact,
            Some(deviation) => GeometryFidelity::tessellated(deviation)?,
        };
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x0, y0, slab.bottom], [x1, y1, slab.top])?,
            fidelity,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        unreachable!("slab stacks never measure pairwise proximity")
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn stack_rule(extra: Vec<(&'static str, ParameterValue)>) -> CompiledRule {
    let mut parameters = vec![("minimum_overlap_ratio", number(0.5))];
    parameters.extend(extra);
    rule(ID, kind("slab"), parameters)
}

fn model(slabs: &Slabs) -> Model {
    slabs.slabs.keys().fold(Model::default(), |model, object| {
        model.object(&object.local_id, "slab")
    })
}

fn run(slabs: Slabs, rule: &CompiledRule) -> CapabilityEvaluation {
    let shared = Arc::new(slabs);
    model(&shared).evaluate_with(&SlabStackSpacing, rule, |services| {
        services
            .register(VerticalExtentServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(PlanAreaServiceHandle::new(shared))
            .unwrap();
    })
}

const PLAN: [f64; 4] = [0.0, 0.0, 10.0, 8.0];

/// Three 0.2 m slabs on one plan; the second storey rises 3.5 m instead of 3.
fn three_stacked() -> Slabs {
    Slabs::default()
        .with("s1", PLAN, 0.0, 0.2)
        .with("s2", PLAN, 3.0, 3.2)
        .with("s3", [0.5, 0.0, 10.0, 8.0], 6.5, 6.7)
}

#[test]
fn three_stacked_slabs_with_one_irregular_gap_are_checked() {
    let evaluation = run(
        three_stacked(),
        &stack_rule(vec![
            ("top_to_top_maximum", metres(3.2)),
            ("top_to_bottom_minimum", metres(2.7)),
            ("consistent", strings(&["top_to_top", "bottom_to_bottom"])),
        ]),
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 3, "{found:#?}");
    assert_eq!(
        found[0].1,
        "top-to-top distance to test:model/s3 is 3.5 m; required at most 3.2 m"
    );
    // Two pairs, one gap each: the lower rise prevails.
    assert!(
        found[1]
            .1
            .ends_with("is 3.5 m, which differs from the prevailing 3 m in this stack"),
        "{}",
        found[1].1
    );
    assert!(
        found[2].1.starts_with("bottom-to-bottom distance"),
        "{}",
        found[2].1
    );
    assert!(found.iter().all(|(object, _)| object == "s2"));
    // The finding relates the next slab up and cites both extents and the overlap.
    let first = &evaluation.findings()[0];
    assert_eq!(first.related, vec![id("s3")]);
    let locators: Vec<&str> = first
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert!(
        locators
            .iter()
            .any(|locator| locator.starts_with("overlap:"))
    );
    assert!(locators.contains(&format!("extent:{}", id("s2")).as_str()));
    assert!(locators.contains(&format!("extent:{}", id("s3")).as_str()));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn the_clear_gap_is_measured_from_top_to_underside() {
    let evaluation = run(
        three_stacked(),
        &stack_rule(vec![("top_to_bottom_minimum", metres(3.0))]),
    );
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(object, message)| (object, message.rsplit(" is ").next().unwrap().to_owned()))
            .collect::<Vec<_>>(),
        [("s1".to_owned(), "2.8 m; required at least 3 m".to_owned())]
    );
}

#[test]
fn a_slab_with_no_stack_partner_has_nothing_to_check() {
    // `alone` is elsewhere; `offset` overlaps s1 by a fifth of its footprint,
    // less than the declared half, so it does not stack with it.
    let slabs = Slabs::default()
        .with("s1", PLAN, 0.0, 0.2)
        .with("alone", [30.0, 0.0, 40.0, 8.0], 0.9, 1.1)
        .with("offset", [8.0, 0.0, 18.0, 8.0], 1.0, 1.2);
    let evaluation = run(
        slabs,
        &stack_rule(vec![("top_to_top_minimum", metres(2.5))]),
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:#?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_partial_overlap_above_the_ratio_stacks() {
    // `offset` covers 80 % of s1: they stack, and 1 m is too close.
    let slabs =
        Slabs::default()
            .with("s1", PLAN, 0.0, 0.2)
            .with("offset", [2.0, 0.0, 12.0, 8.0], 1.0, 1.2);
    let evaluation = run(
        slabs,
        &stack_rule(vec![("top_to_top_minimum", metres(2.5))]),
    );
    assert_eq!(evaluation.findings().len(), 1);
    assert_eq!(findings(&evaluation)[0].0, "s1");
}

/// A tessellated slab's elevations are intervals. A bound the interval
/// straddles is not evaluated; one it clears is judged, and the finding
/// carries the approximate evidence rather than presenting it as exact.
#[test]
fn a_tessellated_slab_is_judged_only_on_its_whole_interval() {
    let slabs = three_stacked().tessellated("s3", 0.01);
    let evaluation = run(
        slabs,
        &stack_rule(vec![
            // s2 -> s3 rises 3.5 m +- 0.01: straddles 3.5, clears 3.4.
            ("top_to_top_maximum", metres(3.5)),
            ("bottom_to_bottom_maximum", metres(3.4)),
        ]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("s2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles the bound at most 3.5 m")
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .1
            .ends_with("is between 3.49 m and 3.51 m; required at most 3.4 m"),
        "{}",
        found[0].1
    );
    let evidence = &evaluation.findings()[0].evidence;
    assert!(
        evidence
            .iter()
            .any(|evidence| evidence.locator == format!("extent:{}", id("s3")) && !evidence.exact)
    );
}

#[test]
fn a_tessellated_slab_level_with_its_partner_cannot_be_ordered() {
    let slabs = Slabs::default()
        .with("s1", PLAN, 0.0, 0.2)
        .with("s2", PLAN, 0.0, 0.2)
        .tessellated("s2", 0.01);
    let evaluation = run(
        slabs,
        &stack_rule(vec![("top_to_top_minimum", metres(2.5))]),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("s1".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("s2".to_owned(), NotEvaluatedReason::IncompleteEvidence)
        ]
    );
}

#[test]
fn a_consistent_measure_the_interval_cannot_settle_is_not_evaluated() {
    let slabs = Slabs::default()
        .with("s1", PLAN, 0.0, 0.2)
        .with("s2", PLAN, 3.0, 3.2)
        .with("s3", PLAN, 6.0, 6.2)
        .tessellated("s3", 0.01);
    let evaluation = run(
        slabs,
        &stack_rule(vec![("consistent", strings(&["top_to_top"]))]),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("s2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn an_overlap_ratio_straddling_the_minimum_leaves_the_slab_unevaluated() {
    // 50 % overlap, but the upper slab's footprint is uncertain.
    let slabs = Slabs::default()
        .with("s1", PLAN, 0.0, 0.2)
        .with("s2", [5.0, 0.0, 15.0, 8.0], 3.0, 3.2)
        .tessellated("s2", 0.5);
    let evaluation = run(
        slabs,
        &stack_rule(vec![("top_to_top_maximum", metres(4.0))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("s1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_slab_without_an_extent_leaves_every_stack_unevaluated() {
    let shared = Arc::new(three_stacked());
    let evaluation = model(&shared).object("ghost", "slab").evaluate_with(
        &SlabStackSpacing,
        &stack_rule(vec![("top_to_top_maximum", metres(3.2))]),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(shared.clone()))
                .unwrap();
            services
                .register(PlanAreaServiceHandle::new(shared))
                .unwrap();
        },
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("ghost".to_owned(), NotEvaluatedReason::BackendUnavailable),
            ("s1".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("s2".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("s3".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn without_services_nothing_is_judged() {
    let slabs = three_stacked();
    let evaluation = model(&slabs).evaluate(
        &SlabStackSpacing,
        &stack_rule(vec![("top_to_top_maximum", metres(3.2))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("s1".to_owned(), NotEvaluatedReason::MissingService),
            ("s2".to_owned(), NotEvaluatedReason::MissingService),
            ("s3".to_owned(), NotEvaluatedReason::MissingService),
        ]
    );
}

#[test]
fn a_declaration_without_a_check_or_with_a_bad_ratio_is_invalid() {
    for rule in [
        stack_rule(Vec::new()),
        rule(
            ID,
            kind("slab"),
            vec![
                ("minimum_overlap_ratio", number(0.0)),
                ("top_to_top_maximum", metres(3.2)),
            ],
        ),
        stack_rule(vec![("consistent", strings(&["storey_height"]))]),
        stack_rule(vec![
            ("top_to_top_minimum", metres(4.0)),
            ("top_to_top_maximum", metres(3.0)),
        ]),
    ] {
        let evaluation = run(three_stacked(), &rule);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// With extents available, footprints that cannot meet are never overlaid.
#[test]
fn disjoint_enclosing_boxes_skip_the_overlap_measurement() {
    let shared = Arc::new(three_stacked().with("far", [30.0, 0.0, 40.0, 8.0], 3.0, 3.2));
    let evaluation = model(&shared).evaluate_with(
        &SlabStackSpacing,
        &stack_rule(vec![("top_to_top_maximum", metres(3.2))]),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(shared.clone()))
                .unwrap();
            services
                .register(PlanAreaServiceHandle::new(shared.clone()))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(shared.clone()))
                .unwrap();
        },
    );
    assert_eq!(findings(&evaluation).len(), 1);
    let measured = shared.overlaps.lock().unwrap();
    assert!(!measured.is_empty());
    assert!(
        measured
            .iter()
            .all(|(first, second)| first != "far" && second != "far"),
        "{measured:?}"
    );
}

/// Each threshold of `slab-stack-spacing` is the `stack_distance` of its
/// measure within the bound, none where no slab stacks above: expressions
/// over it reach the capability's verdicts on its fixtures.
mod as_expressions {
    use super::*;
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{
        and, assert_parity, at_least, at_most, m, measured, mm, rule as expression, unless_null,
    };
    use serde_json::Value;

    /// The services to register: proximity boxes too, or not.
    #[derive(Clone, Copy)]
    enum Services {
        None,
        Measuring,
        WithBoxes,
    }

    fn rewrite(
        slabs: fn() -> Slabs,
        ghost: bool,
        services: Services,
        requirement: &Value,
    ) -> CapabilityEvaluation {
        let mut model = model(&slabs());
        if ghost {
            model = model.object("ghost", "slab");
        }
        model.evaluate_measured(
            &ExpressionRequirement,
            &expression(kind("slab"), requirement),
            |registry| {
                let shared = Arc::new(slabs());
                if matches!(services, Services::None) {
                    return;
                }
                registry
                    .register(VerticalExtentServiceHandle::new(shared.clone()))
                    .unwrap();
                registry
                    .register(PlanAreaServiceHandle::new(shared.clone()))
                    .unwrap();
                if matches!(services, Services::WithBoxes) {
                    registry
                        .register(ProximityServiceHandle::new(shared))
                        .unwrap();
                }
            },
        )
    }

    fn capability(
        slabs: fn() -> Slabs,
        ghost: bool,
        services: Services,
        rule: &CompiledRule,
    ) -> CapabilityEvaluation {
        let shared = Arc::new(slabs());
        let mut model = model(&shared);
        if ghost {
            model = model.object("ghost", "slab");
        }
        model.evaluate_with(&SlabStackSpacing, rule, |registry| {
            if matches!(services, Services::None) {
                return;
            }
            registry
                .register(VerticalExtentServiceHandle::new(shared.clone()))
                .unwrap();
            registry
                .register(PlanAreaServiceHandle::new(shared.clone()))
                .unwrap();
            if matches!(services, Services::WithBoxes) {
                registry
                    .register(ProximityServiceHandle::new(shared))
                    .unwrap();
            }
        })
    }

    /// A measure with its minimum and maximum.
    type Band = (&'static str, Option<f64>, Option<f64>);

    /// The bounds of each measure, rewritten.
    fn bounded(bands: &[Band]) -> Value {
        and(bands
            .iter()
            .map(|(measure, minimum, maximum)| {
                let distance = measured(&format!(
                    "stack_distance;measure={measure};slabs=slab;ratio=0.5"
                ));
                let mut tests = Vec::new();
                if let Some(minimum) = minimum {
                    tests.push(at_least(mm(distance.clone()), m(*minimum)));
                }
                if let Some(maximum) = maximum {
                    tests.push(at_most(mm(distance.clone()), m(*maximum)));
                }
                unless_null(&distance, and(tests))
            })
            .collect())
    }

    /// The capability's parameter bounding `measure` from below or above.
    fn bound(measure: &str, minimum: bool) -> &'static str {
        match (measure, minimum) {
            ("top_to_top", true) => "top_to_top_minimum",
            ("top_to_top", false) => "top_to_top_maximum",
            ("bottom_to_bottom", true) => "bottom_to_bottom_minimum",
            ("bottom_to_bottom", false) => "bottom_to_bottom_maximum",
            ("top_to_bottom", true) => "top_to_bottom_minimum",
            _ => "top_to_bottom_maximum",
        }
    }

    fn declared(bands: &[Band]) -> CompiledRule {
        let mut extra = Vec::new();
        for (measure, minimum, maximum) in bands {
            if let Some(minimum) = minimum {
                extra.push((bound(measure, true), metres(*minimum)));
            }
            if let Some(maximum) = maximum {
                extra.push((bound(measure, false), metres(*maximum)));
            }
        }
        stack_rule(extra)
    }

    fn parity(slabs: fn() -> Slabs, ghost: bool, services: Services, bands: &[Band]) {
        let found = capability(slabs, ghost, services, &declared(bands));
        let rewritten = rewrite(slabs, ghost, services, &bounded(bands));
        assert_parity(ID, &found, &rewritten);
    }

    fn alone() -> Slabs {
        Slabs::default()
            .with("s1", PLAN, 0.0, 0.2)
            .with("alone", [30.0, 0.0, 40.0, 8.0], 0.9, 1.1)
            .with("offset", [8.0, 0.0, 18.0, 8.0], 1.0, 1.2)
    }

    fn partial() -> Slabs {
        Slabs::default()
            .with("s1", PLAN, 0.0, 0.2)
            .with("offset", [2.0, 0.0, 12.0, 8.0], 1.0, 1.2)
    }

    fn tessellated() -> Slabs {
        three_stacked().tessellated("s3", 0.01)
    }

    fn level() -> Slabs {
        Slabs::default()
            .with("s1", PLAN, 0.0, 0.2)
            .with("s2", PLAN, 0.0, 0.2)
            .tessellated("s2", 0.01)
    }

    fn straddling_overlap() -> Slabs {
        Slabs::default()
            .with("s1", PLAN, 0.0, 0.2)
            .with("s2", [5.0, 0.0, 15.0, 8.0], 3.0, 3.2)
            .tessellated("s2", 0.5)
    }

    fn with_far() -> Slabs {
        three_stacked().with("far", [30.0, 0.0, 40.0, 8.0], 3.0, 3.2)
    }

    #[test]
    fn every_threshold_reaches_the_verdicts() {
        let checks: [&[Band]; 6] = [
            &[
                ("top_to_top", None, Some(3.2)),
                ("top_to_bottom", Some(2.7), None),
            ],
            &[("top_to_bottom", Some(3.0), None)],
            &[("top_to_top", Some(2.5), None)],
            &[
                ("top_to_top", None, Some(3.5)),
                ("bottom_to_bottom", None, Some(3.4)),
            ],
            &[("bottom_to_bottom", Some(3.0), Some(3.3))],
            &[("top_to_top", None, Some(4.0))],
        ];
        for slabs in [
            three_stacked as fn() -> Slabs,
            alone,
            partial,
            tessellated,
            level,
            straddling_overlap,
        ] {
            for bands in checks {
                parity(slabs, false, Services::Measuring, bands);
            }
        }
    }

    #[test]
    fn an_unmeasurable_slab_or_no_services_leave_the_slabs_open_alike() {
        let bands: &[Band] = &[("top_to_top", None, Some(3.2))];
        parity(three_stacked, true, Services::Measuring, bands);
        parity(three_stacked, false, Services::None, bands);
        parity(with_far, false, Services::WithBoxes, bands);
    }
}
