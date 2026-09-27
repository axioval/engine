//! `exit-separation`: a space's exits lie far enough apart for its size.
//!
//! The stubs answer only the measurements a test declares and panic on any
//! other, which also proves each separation reaches the service it names.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, GeometryFidelity, NotEvaluatedReason, ObjectBounds, PlanLength,
    PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle, ProjectedDistanceEvidence,
    ProximityError, ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use axioval_rules::ExitSeparation;
use common::{
    Model, findings, id, integer, kind, number, property, rule, selector, source, string, strings,
    unevaluated,
};

const CAPABILITY: &str = "axioval:capability.exit-separation";

/// The longest diagonal of a 20 x 10 m room.
fn diagonal() -> f64 {
    500.0_f64.sqrt()
}

fn interval(lower: f64, upper: f64, locator: String) -> Evidence {
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    Evidence {
        source: source(),
        locator,
        exact,
    }
}

#[derive(Default)]
struct Spans {
    diameters: BTreeMap<String, (f64, f64)>,
    /// `(first, second, span)` -> length interval.
    spans: BTreeMap<(String, String, PlanSpan), (f64, f64)>,
}

impl Spans {
    fn diameter(mut self, local: &str, lower: f64, upper: f64) -> Self {
        self.diameters.insert(local.into(), (lower, upper));
        self
    }
    fn span(mut self, a: &str, b: &str, between: PlanSpan, metres: f64) -> Self {
        self.spans
            .insert((a.into(), b.into(), between), (metres, metres));
        self
    }
}

impl PlanSpanService for Spans {
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        let Some(&(lower, upper)) = self.diameters.get(&object.local_id) else {
            return Err(PlanSpanError::Unavailable(format!(
                "{object} has no footprint (no body)"
            )));
        };
        PlanLength::try_new(
            lower,
            upper,
            interval(lower, upper, format!("plan-diameter:{object}")),
        )
    }

    fn measure_span(
        &self,
        first: &ObjectId,
        second: &ObjectId,
        between: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        let (a, b) = (first.local_id.clone(), second.local_id.clone());
        let (lower, upper) = *self
            .spans
            .get(&(a.clone(), b.clone(), between))
            .unwrap_or_else(|| panic!("unexpected {between:?} span {a}/{b}"));
        PlanLength::try_new(
            lower,
            upper,
            interval(
                lower,
                upper,
                format!("plan-span:{}:{a}:{b}", between.name()),
            ),
        )
    }
}

#[derive(Default)]
struct Plan {
    /// `(subject, counterpart)` -> horizontal distance interval.
    distances: BTreeMap<(String, String), (f64, f64)>,
}

impl Plan {
    fn apart(mut self, a: &str, b: &str, lower: f64, upper: f64) -> Self {
        self.distances.insert((a.into(), b.into()), (lower, upper));
        self
    }
}

impl ProximityService for Plan {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([0.0; 3], [1.0; 3])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("exit separation measures through measure_distance")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        assert_eq!(request.projection(), ProximityProjection::Horizontal);
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let (lower, upper) = *self
            .distances
            .get(&(a.clone(), b.clone()))
            .unwrap_or_else(|| panic!("unexpected plan distance {a}/{b}"));
        #[allow(clippy::float_cmp)]
        let fidelity = if lower == upper {
            GeometryFidelity::Exact
        } else {
            GeometryFidelity::tessellated((upper - lower) / 2.0)?
        };
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            interval(lower, upper, format!("distance:{a}:{b}")),
        )
    }
}

/// Room `hall` (20 x 10 m) on storey `level`, with doors `d1` and `d2` on
/// its boundary (`bounds` runs from a door to its room).
fn model() -> Model {
    Model::default()
        .object("level", "storey")
        .object("hall", "space")
        .object("d1", "door")
        .object("d2", "door")
        .edge("contains", "level", "hall")
        .edge("bounds", "d1", "hall")
        .edge("bounds", "d2", "hall")
}

fn parameters(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut all = vec![
        ("exit_path", strings(&["bounds:backward"])),
        ("exit_selector", selector(kind("door"))),
    ];
    all.extend(extra);
    all
}

fn sprinklered() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("flag", property(Some("Fire"), "Sprinklered")),
        ("flag_path", strings(&["contains:backward"])),
        ("flagged_fraction", number(1.0 / 3.0)),
    ]
}

fn evaluate(
    model: Model,
    spans: Spans,
    plan: Plan,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &ExitSeparation,
        &rule(CAPABILITY, kind("space"), parameters(extra)),
        |services| {
            services
                .register(PlanSpanServiceHandle::new(Arc::new(spans)))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(plan)))
                .unwrap();
        },
    )
}

fn hall() -> Spans {
    Spans::default().diameter("hall", diagonal(), diagonal())
}

#[test]
fn two_exits_too_close_in_a_large_room_are_found() {
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 2.0, 2.0),
        vec![],
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{evaluation:?}");
    assert_eq!(found[0].0, "hall");
    assert_eq!(
        found[0].1,
        format!(
            "exits {} and {} are 2 m apart between closest points; required at least 11.1803 m \
             (0.5 × the longest plan diagonal of 22.3607 m)",
            id("d1"),
            id("d2")
        )
    );
    let finding = &evaluation.findings()[0];
    let locators: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert!(locators.contains(&"distance:d1:d2"), "{locators:?}");
    assert!(
        locators.contains(&format!("plan-diameter:{}", id("hall")).as_str()),
        "{locators:?}"
    );
    assert_eq!(finding.related, vec![id("d1"), id("d2")]);
    assert!(unevaluated(&evaluation).is_empty());

    // Twelve metres apart is far enough.
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 12.0, 12.0),
        vec![],
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_sprinklered_storey_needs_only_a_third_of_the_diagonal() {
    // Nine metres: short of a half (11.18 m), beyond a third (7.45 m).
    let nine = || Plan::default().apart("d1", "d2", 9.0, 9.0);
    let with_flag = |value: bool| {
        model().value(
            "level",
            "Fire",
            "Sprinklered",
            PropertyValue::Boolean(value),
        )
    };
    let evaluation = evaluate(with_flag(true), hall(), nine(), sprinklered());
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty());

    let evaluation = evaluate(with_flag(false), hall(), nine(), sprinklered());
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{evaluation:?}");
    assert!(
        found[0].1.ends_with(
            "required at least 11.1803 m (0.5 × the longest plan diagonal of 22.3607 m, \
             Fire.Sprinklered (via contains) false)"
        ),
        "{}",
        found[0].1
    );
    // The storey the flag was read from is related.
    assert!(evaluation.findings()[0].related.contains(&id("level")));

    // Sprinklered but still too close.
    let evaluation = evaluate(
        with_flag(true),
        hall(),
        Plan::default().apart("d1", "d2", 5.0, 5.0),
        sprinklered(),
    );
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("required at least 7.4536 m (0.3333 ×"),
        "{evaluation:?}"
    );
}

#[test]
fn an_unknown_flag_decides_only_what_both_fractions_agree_on() {
    // The flag is absent: nine metres passes a third but not a half.
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 9.0, 9.0),
        sprinklered(),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message().to_owned();
    assert!(message.contains("straddles"), "{message}");
    assert!(
        message.contains("Fire.Sprinklered (via contains) unknown"),
        "{message}"
    );

    // Twelve metres satisfies both fractions; two metres fails both.
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 12.0, 12.0),
        sprinklered(),
    );
    assert!(evaluation.findings().is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    let evaluation = evaluate(
        model().text("level", "Fire", "Sprinklered", "yes"),
        hall(),
        Plan::default().apart("d1", "d2", 2.0, 2.0),
        sprinklered(),
    );
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("between 0.3333 and 0.5 ×"),
        "{evaluation:?}"
    );
}

#[test]
fn each_separation_measures_between_its_own_points() {
    // A 10 m diagonal needs 5 m: the doors' closest points are 2 m apart,
    // their centres 4 m and their farthest points 6 m.
    let spans = || {
        Spans::default()
            .diameter("hall", 10.0, 10.0)
            .span("d1", "d2", PlanSpan::Centres, 4.0)
            .span("d1", "d2", PlanSpan::Farthest, 6.0)
    };
    let closest = || Plan::default().apart("d1", "d2", 2.0, 2.0);
    let judge = |separation: Option<&str>, plan: Plan| {
        let extra = separation
            .map(|separation| vec![("separation", string(separation))])
            .unwrap_or_default();
        findings(&evaluate(model(), spans(), plan, extra))
    };
    let found = judge(None, closest());
    assert!(
        found[0].1.contains("2 m apart between closest points"),
        "{found:?}"
    );
    let found = judge(Some("closest"), closest());
    assert_eq!(found.len(), 1);
    // The span services answer the others; the proximity stub knows nothing.
    let found = judge(Some("centres"), Plan::default());
    assert!(
        found[0].1.contains("4 m apart between centres"),
        "{found:?}"
    );
    assert!(judge(Some("farthest"), Plan::default()).is_empty());
}

#[test]
fn fewer_than_two_exits_are_not_checked_unless_a_minimum_is_declared() {
    let single = || {
        Model::default()
            .object("hall", "space")
            .object("d1", "door")
            .object("w1", "window")
            .edge("bounds", "d1", "hall")
            .edge("bounds", "w1", "hall")
    };
    let evaluation = evaluate(single(), hall(), Plan::default(), vec![]);
    assert!(evaluation.findings().is_empty());
    assert!(unevaluated(&evaluation).is_empty());

    let evaluation = evaluate(
        single(),
        hall(),
        Plan::default(),
        vec![("minimum_exits", integer(2))],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".to_owned(),
            "has 1 exit(s) via bounds; at least 2 required".to_owned()
        )]
    );
}

fn three_doors() -> Model {
    model().object("d3", "door").edge("bounds", "d3", "hall")
}

fn three_apart() -> Plan {
    Plan::default()
        .apart("d1", "d2", 2.0, 2.0)
        .apart("d1", "d3", 15.0, 15.0)
        .apart("d2", "d3", 13.0, 13.0)
}

#[test]
fn some_pair_or_every_pair_must_be_far_enough_apart() {
    let evaluation = evaluate(three_doors(), hall(), three_apart(), vec![]);
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");

    let evaluation = evaluate(
        three_doors(),
        hall(),
        three_apart(),
        vec![("pairs", string("all"))],
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].1.starts_with(&format!(
            "exits {} and {} are 2 m apart",
            id("d1"),
            id("d2")
        )),
        "{found:?}"
    );

    // No pair far enough: the farthest one is named.
    let evaluation = evaluate(
        three_doors(),
        hall(),
        Plan::default()
            .apart("d1", "d2", 2.0, 2.0)
            .apart("d1", "d3", 5.0, 5.0)
            .apart("d2", "d3", 3.0, 3.0),
        vec![],
    );
    assert_eq!(
        findings(&evaluation)[0].1,
        format!(
            "no two of its 3 exits are far enough apart: {} and {} are 5 m apart between \
             closest points; required at least 11.1803 m (0.5 × the longest plan diagonal of \
             22.3607 m)",
            id("d1"),
            id("d3")
        )
    );
}

fn tagged_door() -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("door"),
            Selector::Property {
                property_set: Some("Exit".into()),
                property: "IsExit".into(),
                operator: ComparisonOperator::Exists,
                value: None,
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            },
        ],
    }
}

#[test]
fn an_undecided_exit_can_only_add_pairs() {
    // d1 and d2 are tagged exits; whether d3 is one cannot be read.
    let run_tagged = |plan: Plan, pairs: &str| {
        three_doors()
            .value("d1", "Exit", "IsExit", PropertyValue::Boolean(true))
            .value("d2", "Exit", "IsExit", PropertyValue::Boolean(true))
            .unreadable("d3")
            .evaluate_with(
                &ExitSeparation,
                &rule(
                    CAPABILITY,
                    kind("space"),
                    vec![
                        ("exit_path", strings(&["bounds:backward"])),
                        ("exit_selector", selector(tagged_door())),
                        ("pairs", string(pairs)),
                    ],
                ),
                |services| {
                    services
                        .register(PlanSpanServiceHandle::new(Arc::new(hall())))
                        .unwrap();
                    services
                        .register(ProximityServiceHandle::new(Arc::new(plan)))
                        .unwrap();
                },
            )
    };
    // A certain pair far enough apart stands whatever d3 is.
    let evaluation = run_tagged(Plan::default().apart("d1", "d2", 12.0, 12.0), "any");
    assert!(evaluation.findings().is_empty());
    assert!(
        !unevaluated(&evaluation)
            .iter()
            .any(|(object, _)| object == "hall"),
        "{evaluation:?}"
    );
    // Too close: d3 might be an exit far from both.
    let evaluation = run_tagged(Plan::default().apart("d1", "d2", 2.0, 2.0), "any");
    assert!(evaluation.findings().is_empty());
    let message = evaluation
        .not_evaluated_outcomes()
        .iter()
        .find(|outcome| outcome.object_id() == Some(&id("hall")))
        .map(|outcome| outcome.message().to_owned())
        .expect("hall is not evaluated");
    assert!(message.contains("whether"), "{message}");
    // With every pair required, a pair too close stands.
    let evaluation = run_tagged(Plan::default().apart("d1", "d2", 2.0, 2.0), "all");
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
}

#[test]
fn a_tessellated_separation_straddling_the_requirement_is_not_evaluated() {
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 11.0, 11.4),
        vec![],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // An interval wholly below is still a finding.
    let evaluation = evaluate(
        model(),
        Spans::default().diameter("hall", diagonal() - 0.1, diagonal() + 0.1),
        Plan::default().apart("d1", "d2", 2.0, 2.2),
        vec![],
    );
    assert_eq!(findings(&evaluation).len(), 1);
}

#[test]
fn missing_services_and_measurements_are_not_evaluated() {
    let evaluation = model().evaluate(
        &ExitSeparation,
        &rule(CAPABILITY, kind("space"), parameters(vec![])),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::MissingService)]
    );
    // No diagonal for the room.
    let evaluation = evaluate(model(), Spans::default(), Plan::default(), vec![]);
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn invalid_declarations_refuse_the_rule() {
    for (extra, needle) in [
        (
            vec![("flag", property(Some("Fire"), "Sprinklered"))],
            "`flag` needs `flagged_fraction`",
        ),
        (
            vec![("flagged_fraction", number(0.3))],
            "`flagged_fraction` needs `flag`",
        ),
        (
            vec![("fraction", number(0.0))],
            "`fraction` must be a positive number",
        ),
        (vec![("separation", string("edges"))], "separation `edges`"),
        (vec![("pairs", string("some"))], "pairs `some`"),
        (
            vec![("minimum_exits", integer(0))],
            "`minimum_exits` must be at least one",
        ),
    ] {
        let evaluation = evaluate(model(), hall(), Plan::default(), extra);
        let outcomes = evaluation.not_evaluated_outcomes();
        assert_eq!(outcomes.len(), 1, "{needle}");
        assert_eq!(
            outcomes[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
        assert!(
            outcomes[0].message().contains(needle),
            "{needle}: {outcomes:?}"
        );
    }
}
