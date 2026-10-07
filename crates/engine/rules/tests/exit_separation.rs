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
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use axioval_rules::ExitSeparation;
use common::{
    Model, boolean, findings, id, integer, kind, number, property, rule, selector, source, string,
    strings, unevaluated,
};

const CAPABILITY: &str = "axioval:capability.exit-separation";

/// The template, held to the implementation it replaced on every
/// evaluation.
static HELD: common::Held =
    common::Held(&ExitSeparation, &axioval_rules::reference::ExitSeparation);

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
    /// Pairs whose distance cannot be measured.
    unmeasured: Vec<(String, String)>,
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
        if self.unmeasured.contains(&(a.clone(), b.clone())) {
            return Err(ProximityError::Unavailable);
        }
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
    let (spans, plan) = (Arc::new(spans), Arc::new(plan));
    model.evaluate_measured(
        &HELD,
        &rule(CAPABILITY, kind("space"), parameters(extra)),
        move |services| {
            services
                .register(PlanSpanServiceHandle::new(spans.clone()))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(plan.clone()))
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
    // The finding cites the measured exits, as exact as the distance and
    // the diagonal they rest on.
    let finding = &evaluation.findings()[0];
    assert!(
        finding.evidence.iter().all(|evidence| evidence.exact),
        "{finding:?}"
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

/// The flag read on the space, then its storey, then its building, with a
/// default when none states it.
fn sprinklered_anywhere(default: Option<bool>) -> Vec<(&'static str, ParameterValue)> {
    let source = |path: Option<&str>| {
        let mut row: TableRow = [
            ("property_set".to_owned(), string("Fire")),
            ("property".to_owned(), string("Sprinklered")),
        ]
        .into_iter()
        .collect();
        if let Some(path) = path {
            row.insert("path".into(), string(path));
        }
        row
    };
    let mut parameters = vec![
        (
            "flag_sources",
            ParameterValue::Table {
                value: vec![
                    source(None),
                    source(Some("contains:backward")),
                    source(Some("contains:backward contains:backward")),
                ],
            },
        ),
        ("flagged_fraction", number(1.0 / 3.0)),
    ];
    if let Some(default) = default {
        parameters.push(("flag_default", boolean(default)));
    }
    parameters
}

#[test]
fn the_flag_is_read_from_the_first_source_stating_it() {
    // Nine metres: short of a half (11.18 m), beyond a third (7.45 m).
    let nine = || Plan::default().apart("d1", "d2", 9.0, 9.0);
    let building = || {
        model()
            .object("site", "building")
            .edge("contains", "site", "level")
    };
    let flag = |model: Model, local: &str, value: bool| {
        model.value(local, "Fire", "Sprinklered", PropertyValue::Boolean(value))
    };

    // The hall states nothing; its storey says sprinklered.
    let evaluation = evaluate(
        flag(building(), "level", true),
        hall(),
        nine(),
        sprinklered_anywhere(Some(false)),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // The hall's own value comes first.
    let evaluation = evaluate(
        flag(flag(building(), "level", true), "hall", false),
        hall(),
        nine(),
        sprinklered_anywhere(None),
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{evaluation:?}");
    assert!(
        found[0].1.ends_with("22.3607 m, Fire.Sprinklered false)"),
        "{}",
        found[0].1
    );

    // Only the building states it.
    let evaluation = evaluate(
        flag(building(), "site", true),
        hall(),
        nine(),
        sprinklered_anywhere(None),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // Nothing stated anywhere: the default applies, and the message says so.
    let evaluation = evaluate(
        building(),
        hall(),
        nine(),
        sprinklered_anywhere(Some(false)),
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{evaluation:?}");
    assert!(
        found[0].1.ends_with(
            "Fire.Sprinklered nor Fire.Sprinklered (via contains) nor Fire.Sprinklered (via \
             contains then contains) not stated, default false)"
        ),
        "{}",
        found[0].1
    );
    let evaluation = evaluate(building(), hall(), nine(), sprinklered_anywhere(Some(true)));
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // Without a default, nothing stated is unknown.
    let evaluation = evaluate(building(), hall(), nine(), sprinklered_anywhere(None));
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );

    // A null on the storey is stated, not absent: the building's value is
    // never reached and the flag is unknown.
    let evaluation = evaluate(
        flag(building(), "site", true).value("level", "Fire", "Sprinklered", PropertyValue::Null),
        hall(),
        nine(),
        sprinklered_anywhere(Some(true)),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message().to_owned();
    assert!(
        message.contains("Fire.Sprinklered (via contains) unknown"),
        "{message}"
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
        let plan = Arc::new(plan);
        three_doors()
            .value("d1", "Exit", "IsExit", PropertyValue::Boolean(true))
            .value("d2", "Exit", "IsExit", PropertyValue::Boolean(true))
            .unreadable("d3")
            .evaluate_measured(
                &HELD,
                &rule(
                    CAPABILITY,
                    kind("space"),
                    vec![
                        ("exit_path", strings(&["bounds:backward"])),
                        ("exit_selector", selector(tagged_door())),
                        ("pairs", string(pairs)),
                    ],
                ),
                move |services| {
                    services
                        .register(PlanSpanServiceHandle::new(Arc::new(hall())))
                        .unwrap();
                    services
                        .register(ProximityServiceHandle::new(plan.clone()))
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
    let evaluation = model().evaluate_measured(
        &HELD,
        &rule(CAPABILITY, kind("space"), parameters(vec![])),
        |_| {},
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
            "`flagged_fraction` needs `flag` or `flag_sources`",
        ),
        (
            vec![("fraction", number(0.0))],
            "`fraction` must be a positive number",
        ),
        (
            vec![("flag_default", boolean(true))],
            "`flag_default` needs `flag` or `flag_sources`",
        ),
        (
            {
                let mut both = sprinklered_anywhere(None);
                both.push(("flag", property(Some("Fire"), "Sprinklered")));
                both
            },
            "declare either `flag` (with `flag_path`) or `flag_sources`, not both",
        ),
        (
            vec![
                ("flag_sources", ParameterValue::Table { value: vec![] }),
                ("flagged_fraction", number(0.3)),
            ],
            "`flag_sources` has no rows",
        ),
        (
            vec![("separation", string("edges"))],
            "separation `edges` is unsupported",
        ),
        (
            vec![("pairs", string("some"))],
            "pairs `some` is unsupported",
        ),
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
        assert_eq!(outcomes[0].message(), format!("exit-separation: {needle}"));
    }
}

/// The separation requirement as an expression over the exit pairs and the
/// longest diagonal: some pair (or, for `all`, no pair falls short) at
/// least the fraction of the diagonal apart, the fraction a third where the
/// storey is sprinklered. It flags and leaves open what `exit-separation`
/// does.
#[test]
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
fn exit_pairs_and_the_diagonal_reach_the_verdicts() {
    use serde_json::{Value, json};
    let number =
        |value: f64| json!({"kind": "literal", "value": {"type": "number", "value": value}});
    let required = |sprinklered: bool| -> Value {
        let fraction = if sprinklered {
            json!({"kind": "if", "branches": [{
                "when": {"kind": "aggregate", "function": "any",
                    "over": {"kind": "path", "path": ["contains:backward"]},
                    "value": {"kind": "compare", "operator": "equals",
                        "left": {"kind": "property", "propertySet": "Fire", "property": "Sprinklered"},
                        "right": {"kind": "literal", "value": {"type": "boolean", "value": true}}}},
                "then": number(1.0 / 3.0)}],
                "else": number(0.5)})
        } else {
            number(0.5)
        };
        json!({"kind": "multiply", "left": fraction,
            "right": {"kind": "property", "propertySet": "axioval:measured", "property": "plan_diameter"}})
    };
    let pairs =
        |between: &str| format!("exit_pairs;exits=bounds:backward;kinds=door;between={between}");
    let separation =
        json!({"kind": "property", "propertySet": "axioval:member", "property": "separation"});
    let some = |between: &str, sprinklered: bool| {
        json!({"kind": "or", "operands": [
            {"kind": "compare", "operator": "lessThan",
             "left": {"kind": "aggregate", "function": "count", "over": {"kind": "measured", "name": pairs(between)}},
             "right": {"kind": "literal", "value": {"type": "integer", "value": 1}}},
            {"kind": "aggregate", "function": "any", "over": {"kind": "measured", "name": pairs(between)},
             "value": {"kind": "compare", "operator": "greaterThanOrEquals",
                 "left": separation.clone(), "right": required(sprinklered)}}]})
    };
    let every = json!({"kind": "aggregate", "function": "none",
        "over": {"kind": "measured", "name": pairs("closest")},
        "value": {"kind": "compare", "operator": "lessThan",
            "left": separation.clone(), "right": required(false)}});
    let with_flag = |value: bool| {
        model().value(
            "level",
            "Fire",
            "Sprinklered",
            PropertyValue::Boolean(value),
        )
    };
    let spans = || {
        Spans::default()
            .diameter("hall", 10.0, 10.0)
            .span("d1", "d2", PlanSpan::Centres, 4.0)
            .span("d1", "d2", PlanSpan::Farthest, 6.0)
    };
    let none_far = || {
        Plan::default()
            .apart("d1", "d2", 2.0, 2.0)
            .apart("d1", "d3", 5.0, 5.0)
            .apart("d2", "d3", 3.0, 3.0)
    };
    type Case = (
        Box<dyn Fn() -> Model>,
        Box<dyn Fn() -> Spans>,
        Box<dyn Fn() -> Plan>,
        Vec<(&'static str, ParameterValue)>,
        Value,
    );
    let cases: Vec<Case> = vec![
        (
            Box::new(model),
            Box::new(hall),
            Box::new(|| Plan::default().apart("d1", "d2", 2.0, 2.0)),
            vec![],
            some("closest", false),
        ),
        (
            Box::new(model),
            Box::new(hall),
            Box::new(|| Plan::default().apart("d1", "d2", 12.0, 12.0)),
            vec![],
            some("closest", false),
        ),
        (
            Box::new(move || with_flag(true)),
            Box::new(hall),
            Box::new(|| Plan::default().apart("d1", "d2", 9.0, 9.0)),
            sprinklered(),
            some("closest", true),
        ),
        (
            Box::new(move || with_flag(false)),
            Box::new(hall),
            Box::new(|| Plan::default().apart("d1", "d2", 9.0, 9.0)),
            sprinklered(),
            some("closest", true),
        ),
        (
            Box::new(move || with_flag(true)),
            Box::new(hall),
            Box::new(|| Plan::default().apart("d1", "d2", 5.0, 5.0)),
            sprinklered(),
            some("closest", true),
        ),
        (
            Box::new(model),
            Box::new(spans),
            Box::new(|| Plan::default().apart("d1", "d2", 2.0, 2.0)),
            vec![],
            some("closest", false),
        ),
        (
            Box::new(model),
            Box::new(spans),
            Box::new(Plan::default),
            vec![("separation", string("centres"))],
            some("centres", false),
        ),
        (
            Box::new(model),
            Box::new(spans),
            Box::new(Plan::default),
            vec![("separation", string("farthest"))],
            some("farthest", false),
        ),
        (
            Box::new(three_doors),
            Box::new(hall),
            Box::new(three_apart),
            vec![],
            some("closest", false),
        ),
        (
            Box::new(three_doors),
            Box::new(hall),
            Box::new(three_apart),
            vec![("pairs", string("all"))],
            every,
        ),
        (
            Box::new(three_doors),
            Box::new(hall),
            Box::new(none_far),
            vec![],
            some("closest", false),
        ),
    ];
    for (index, (model, spans, plan, extra, requirement)) in cases.into_iter().enumerate() {
        let expected = evaluate(model(), spans(), plan(), extra);
        let rule = rule(
            "axioval:capability.expression",
            kind("space"),
            vec![(
                "requirement",
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement).unwrap(),
                },
            )],
        );
        let outcome =
            model().evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                services
                    .register(PlanSpanServiceHandle::new(Arc::new(spans())))
                    .unwrap();
                services
                    .register(ProximityServiceHandle::new(Arc::new(plan())))
                    .unwrap();
            });
        let parity = axioval_rules::parity::compare_evaluations(
            (CAPABILITY, &expected),
            ("expression", &outcome),
        );
        assert!(parity.holds(), "case {index}:\n{}", parity.diff());
    }
}

/// The rule `requirement` states as an expression.
fn as_expression(requirement: serde_json::Value) -> axioval_engine::CompiledRule {
    rule(
        "axioval:capability.expression",
        kind("space"),
        vec![(
            "requirement",
            ParameterValue::Expression {
                value: serde_json::from_value(requirement).unwrap(),
            },
        )],
    )
}

/// Runs the capability and `requirement` on one fixture and returns the
/// harness's comparison.
fn parity(
    model: &dyn Fn() -> Model,
    spans: &dyn Fn() -> Spans,
    plan: &dyn Fn() -> Plan,
    extra: Vec<(&'static str, ParameterValue)>,
    requirement: serde_json::Value,
) -> axioval_rules::parity::ParityEvidence {
    let expected = evaluate(model(), spans(), plan(), extra);
    let outcome = model().evaluate_measured(
        &axioval_rules::ExpressionRequirement,
        &as_expression(requirement),
        |services| {
            services
                .register(PlanSpanServiceHandle::new(Arc::new(spans())))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(plan())))
                .unwrap();
        },
    );
    axioval_rules::parity::compare_evaluations((CAPABILITY, &expected), ("expression", &outcome))
}

/// Straddling separations and diagonals, an unmeasured diagonal, a minimum
/// number of exits and an unknown flag, held to the harness: the
/// expression reaches every verdict but the one an unknown flag leaves open
/// between both fractions.
#[test]
#[allow(clippy::too_many_lines, clippy::type_complexity)]
fn intervals_counts_and_unknown_flags_reach_the_verdicts() {
    use axioval_rules::parity::{Difference, Outcome};
    use serde_json::{Value, json};
    let number =
        |value: f64| json!({"kind": "literal", "value": {"type": "number", "value": value}});
    let pairs = "exit_pairs;exits=bounds:backward;kinds=door";
    // A third where the storey says sprinklered, else a half. An unstated
    // flag is `null`, which decides nothing: the expression states that it
    // reads one as unsprinklered.
    let fraction = json!({"kind": "if", "branches": [{
        "when": {"kind": "aggregate", "function": "any",
            "over": {"kind": "path", "path": ["contains:backward"]},
            "value": {"kind": "compare", "operator": "equals",
                "left": {"kind": "coalesce", "operands": [
                    {"kind": "property", "propertySet": "Fire", "property": "Sprinklered"},
                    {"kind": "literal", "value": {"type": "boolean", "value": false}}]},
                "right": {"kind": "literal", "value": {"type": "boolean", "value": true}}}},
        "then": number(1.0 / 3.0)}],
        "else": number(0.5)});
    let some = |fraction: Value| {
        json!({"kind": "or", "operands": [
            {"kind": "compare", "operator": "lessThan",
             "left": {"kind": "aggregate", "function": "count", "over": {"kind": "measured", "name": pairs}},
             "right": {"kind": "literal", "value": {"type": "integer", "value": 1}}},
            {"kind": "aggregate", "function": "any", "over": {"kind": "measured", "name": pairs},
             "value": {"kind": "compare", "operator": "greaterThanOrEquals",
                 "left": {"kind": "property", "propertySet": "axioval:member", "property": "separation"},
                 "right": {"kind": "multiply", "left": fraction,
                     "right": {"kind": "property", "propertySet": "axioval:measured", "property": "plan_diameter"}}}}]})
    };
    let at_least_two = json!({"kind": "and", "operands": [
        {"kind": "compare", "operator": "greaterThanOrEquals",
         "left": {"kind": "aggregate", "function": "count",
             "over": {"kind": "path", "path": ["bounds:backward"]},
             "where": {"kind": "entityType", "objectType": "door", "includeSubtypes": false}},
         "right": {"kind": "literal", "value": {"type": "integer", "value": 2}}},
        some(number(0.5))]});
    let single = || {
        Model::default()
            .object("hall", "space")
            .object("d1", "door")
            .object("w1", "window")
            .edge("bounds", "d1", "hall")
            .edge("bounds", "w1", "hall")
    };
    let close = || Plan::default().apart("d1", "d2", 2.0, 2.0);
    let far = || Plan::default().apart("d1", "d2", 12.0, 12.0);
    let straddling = || Plan::default().apart("d1", "d2", 11.0, 11.4);
    let wide = || Spans::default().diameter("hall", diagonal() - 0.1, diagonal() + 0.1);
    let below = || Plan::default().apart("d1", "d2", 2.0, 2.2);
    // A separation straddling the requirement, one wholly below it under a
    // straddling diagonal, and a room without a diagonal.
    let fixtures: [(&dyn Fn() -> Spans, &dyn Fn() -> Plan); 3] = [
        (&hall, &straddling),
        (&wide, &below),
        (&Spans::default, &close),
    ];
    for (spans, plan) in fixtures {
        let parity = parity(&model, spans, plan, vec![], some(number(0.5)));
        assert!(parity.holds(), "{}", parity.diff());
        assert_eq!(parity.found + parity.open, 1);
    }
    // One exit: nothing to check, unless two are required.
    let alone = parity(&single, &hall, &Plan::default, vec![], some(number(0.5)));
    assert!(alone.holds(), "{}", alone.diff());
    for (model, found) in [(&single as &dyn Fn() -> Model, 1), (&model, 0)] {
        let parity = parity(
            model,
            &hall,
            &far,
            vec![("minimum_exits", integer(2))],
            at_least_two.clone(),
        );
        assert!(parity.holds(), "{}", parity.diff());
        assert_eq!(parity.found, found);
    }
    // An unknown flag: both fractions agree on twelve and on two metres.
    let agreeing: [(&dyn Fn() -> Plan, usize); 2] = [(&far, 0), (&close, 1)];
    for (plan, found) in agreeing {
        let parity = parity(&model, &hall, plan, sprinklered(), some(fraction.clone()));
        assert!(parity.holds(), "{}", parity.diff());
        assert_eq!(parity.found, found);
    }
    // Nine metres passes a third but not a half: `exit-separation` leaves
    // the hall open, while an expression has no value between the two
    // fractions and reads an unstated flag as unsprinklered.
    let unknown = parity(
        &model,
        &hall,
        &|| Plan::default().apart("d1", "d2", 9.0, 9.0),
        sprinklered(),
        some(fraction),
    );
    assert_eq!(
        unknown.differences,
        vec![Difference {
            scope: id("hall").into(),
            capability: Some(Outcome::NotEvaluated {
                reason: NotEvaluatedReason::IncompleteEvidence
            }),
            expression: Some(Outcome::Finding {
                severity: axioval_ir::Severity::Error,
                exact: true
            }),
            details: vec![],
        }]
    );
}

/// Without the plan-span and proximity services, both leave the hall open
/// for the same reason.
#[test]
fn missing_services_leave_the_expression_open_too() {
    let requirement = serde_json::json!({"kind": "compare", "operator": "greaterThan",
        "left": {"kind": "property", "propertySet": "axioval:measured", "property": "plan_diameter"},
        "right": {"kind": "literal", "value": {"type": "quantity", "value": 0.0, "unit": "m"}}});
    let expected = model().evaluate(
        &ExitSeparation,
        &rule(CAPABILITY, kind("space"), parameters(vec![])),
    );
    let outcome = model().evaluate_measured(
        &axioval_rules::ExpressionRequirement,
        &as_expression(requirement),
        |_| {},
    );
    let parity = axioval_rules::parity::compare_evaluations(
        (CAPABILITY, &expected),
        ("expression", &outcome),
    );
    assert!(parity.holds(), "{}", parity.diff());
    assert_eq!(parity.open, 1);
}

/// Generated rooms: up to four doors, each a tagged exit, an untagged door
/// or one whose tag cannot be read, every pair at a distance (exact,
/// tessellated or not measured), the room's diagonal exact, an interval or
/// unmeasured, under every pair mode, minimum and flag (stated, unstated
/// with or without a default, unreadable); each held to the implementation
/// the template replaced.
#[test]
fn generated_rooms_hold_parity() {
    let apart = [
        Some((2.0, 2.0)),
        Some((9.0, 9.0)),
        Some((10.9, 11.5)),
        Some((13.0, 13.0)),
        None,
    ];
    let mut judged = 0;
    for doors in 0..=4_usize {
        for pattern in 0..24_usize {
            let mut model = Model::default()
                .object("level", "storey")
                .object("hall", "space")
                .edge("contains", "level", "hall");
            let mut plan = Plan::default();
            for door in 0..doors {
                let name = format!("d{door}");
                model = model.object(&name, "door").edge("bounds", &name, "hall");
                model = match (pattern + door) % 5 {
                    0 => model.unreadable(&name),
                    1 => model,
                    _ => model.value(&name, "Exit", "IsExit", PropertyValue::Boolean(true)),
                };
                for other in 0..door {
                    let (a, b) = (format!("d{other}"), name.clone());
                    match apart[(pattern * 7 + door * 3 + other) % apart.len()] {
                        Some((lower, upper)) => plan = plan.apart(&a, &b, lower, upper),
                        None => plan.unmeasured.push((a, b)),
                    }
                }
            }
            model = match pattern % 4 {
                0 => model.value("level", "Fire", "Sprinklered", PropertyValue::Boolean(true)),
                1 => model.value(
                    "level",
                    "Fire",
                    "Sprinklered",
                    PropertyValue::Boolean(false),
                ),
                2 => model.unreadable("level"),
                _ => model,
            };
            let spans = match pattern % 3 {
                0 => hall(),
                1 => Spans::default().diameter("hall", diagonal() - 0.5, diagonal() + 0.5),
                _ => Spans::default(),
            };
            let plan = Arc::new(plan);
            let spans = Arc::new(spans);
            for pairs in ["any", "all"] {
                for minimum in [None, Some(2), Some(3)] {
                    for flag in 0..3 {
                        let mut extra = vec![
                            ("exit_path", strings(&["bounds:backward"])),
                            ("exit_selector", selector(tagged_door())),
                            ("pairs", string(pairs)),
                        ];
                        if let Some(minimum) = minimum {
                            extra.push(("minimum_exits", integer(minimum)));
                        }
                        match flag {
                            0 => extra.extend(sprinklered()),
                            1 => extra.extend(sprinklered_anywhere(Some(false))),
                            _ => {}
                        }
                        let (plan, spans) = (plan.clone(), spans.clone());
                        let evaluation = model.clone().evaluate_measured(
                            &HELD,
                            &rule(CAPABILITY, kind("space"), extra),
                            move |services| {
                                services
                                    .register(PlanSpanServiceHandle::new(spans.clone()))
                                    .unwrap();
                                services
                                    .register(ProximityServiceHandle::new(plan.clone()))
                                    .unwrap();
                            },
                        );
                        judged +=
                            evaluation.findings().len() + evaluation.not_evaluated_outcomes().len();
                    }
                }
            }
        }
    }
    assert!(judged > 0);
}

/// Exits measured only approximately are cited as inexact, as the
/// capability cited them.
#[test]
fn a_separation_measured_approximately_is_inexact() {
    let evaluation = evaluate(
        model(),
        hall(),
        Plan::default().apart("d1", "d2", 2.0, 2.2),
        vec![],
    );
    assert_eq!(common::flagged(&evaluation), ["hall"]);
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| !evidence.exact),
        "{evaluation:#?}"
    );
}
