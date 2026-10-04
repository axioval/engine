//! `corridor-end-openings`: no window in the wall a corridor ends at.
//!
//! The stub answers corridor ends only for the spaces a test declares and
//! panics for any other, which also proves a space the selector leaves out
//! is never measured.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CorridorEnd, CorridorEndRequest, CorridorEnds, EndWall,
    NotEvaluatedReason, PlanLength, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle, WallContact,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, ObjectId};
use axioval_rules::CorridorEndOpenings;
use common::{Model, findings, id, kind, number, rule, selector, source, strings, unevaluated};

const CAPABILITY: &str = "axioval:capability.corridor-end-openings";

/// How a window sits against one end wall: gap and facing intervals.
type Contact = ((f64, f64), (f64, f64));

/// One end: its wall (or why it is undecided) and each window against it.
struct End {
    wall: Result<([f64; 2], [f64; 2]), String>,
    contacts: BTreeMap<String, Contact>,
}

impl End {
    fn wall(start: [f64; 2], end: [f64; 2]) -> Self {
        Self {
            wall: Ok((start, end)),
            contacts: BTreeMap::new(),
        }
    }

    fn undecided(why: &str) -> Self {
        Self {
            wall: Err(why.into()),
            contacts: BTreeMap::new(),
        }
    }

    fn window(mut self, local: &str, gap: (f64, f64), facing: (f64, f64)) -> Self {
        self.contacts.insert(local.into(), (gap, facing));
        self
    }
}

#[derive(Default)]
struct Ends(BTreeMap<String, Vec<End>>);

impl Ends {
    fn space(mut self, local: &str, ends: Vec<End>) -> Self {
        self.0.insert(local.into(), ends);
        self
    }
}

fn length(interval: (f64, f64), locator: String) -> PlanLength {
    #[allow(clippy::float_cmp)]
    let exact = interval.0 == interval.1;
    PlanLength::try_new(
        interval.0,
        interval.1,
        Evidence {
            source: source(),
            locator,
            exact,
        },
    )
    .unwrap()
}

impl PlanSpanService for Ends {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        panic!("corridor-end openings measures corridor ends only")
    }

    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        panic!("corridor-end openings measures corridor ends only")
    }

    fn measure_corridor_ends(
        &self,
        request: &CorridorEndRequest,
    ) -> Result<CorridorEnds, PlanSpanError> {
        let space = &request.space().local_id;
        let declared = self
            .0
            .get(space)
            .unwrap_or_else(|| panic!("unexpected corridor ends of {space}"));
        let mut ends = Vec::new();
        for (index, end) in declared.iter().enumerate() {
            let wall = match &end.wall {
                Err(why) => EndWall::Undecided(why.clone()),
                Ok((start, last)) => EndWall::Decided {
                    start: *start,
                    end: *last,
                    contacts: request
                        .subjects()
                        .iter()
                        .map(|subject| {
                            let (gap, facing) = end.contacts[&subject.local_id];
                            let at = format!("{space}:{index}:{}", subject.local_id);
                            WallContact::new(
                                subject.clone(),
                                length(gap, format!("gap:{at}")),
                                length(facing, format!("facing:{at}")),
                            )
                        })
                        .collect(),
                },
            };
            ends.push(CorridorEnd::try_new([1.0, 1.0], (1.0, 1.0), wall)?);
        }
        CorridorEnds::try_new(
            request.space().clone(),
            ends,
            Evidence {
                source: source(),
                locator: format!("corridor-ends:{space}"),
                exact: false,
            },
        )
    }
}

/// Corridor `hall` and room `office`, their windows `bounds` them.
fn model() -> Model {
    Model::default()
        .object("hall", "corridor")
        .object("office", "room")
        .object("end", "window")
        .object("side", "window")
        .object("office-end", "window")
        .object("door", "door")
        .edge("bounds", "end", "hall")
        .edge("bounds", "side", "hall")
        .edge("bounds", "door", "hall")
        .edge("bounds", "office-end", "office")
}

fn evaluate_with(
    model: Model,
    ends: Option<Ends>,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("opening_path", strings(&["bounds:backward"])),
        ("opening_selector", selector(kind("window"))),
    ];
    parameters.extend(extra);
    model.evaluate_with(
        &CorridorEndOpenings,
        &rule(CAPABILITY, kind("corridor"), parameters),
        |services| {
            if let Some(ends) = ends {
                services
                    .register(PlanSpanServiceHandle::new(Arc::new(ends)))
                    .unwrap();
            }
        },
    )
}

/// A 20 x 2 m corridor: a window in its east end wall, one in a side wall.
fn straight() -> Ends {
    Ends::default().space(
        "hall",
        vec![
            End::wall([0.0, 0.0], [0.0, 2.0])
                .window("end", (20.0, 20.0), (1.0, 1.0))
                .window("side", (9.0, 9.0), (0.0, 0.0)),
            End::wall([20.0, 0.0], [20.0, 2.0])
                .window("end", (0.0, 0.0), (1.0, 1.0))
                .window("side", (9.0, 9.0), (0.0, 0.0)),
        ],
    )
}

#[test]
fn a_window_in_a_corridors_end_wall_is_found_and_one_in_its_side_wall_is_not() {
    let evaluation = evaluate_with(model(), Some(straight()), vec![]);
    assert_eq!(
        findings(&evaluation),
        [(
            "end".to_owned(),
            format!(
                "sits in the end wall of corridor {}: 0 m from the wall (20, 0)–(20, 2) and \
                 facing 1 m of it",
                id("hall")
            )
        )]
    );
    let finding = &evaluation.findings()[0];
    assert_eq!(finding.related, vec![id("hall")]);
    let locators: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    for cited in ["corridor-ends:hall", "gap:hall:1:end", "facing:hall:1:end"] {
        assert!(locators.contains(&cited), "{locators:?}");
    }
    // The skeleton is approximate, and the finding says so.
    assert!(finding.evidence.iter().any(|evidence| !evidence.exact));
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn an_l_shaped_corridor_is_judged_at_each_of_its_ends() {
    // The west end and the north end; the east window sits at the bend.
    let ends = Ends::default().space(
        "hall",
        vec![
            End::wall([0.0, 0.0], [0.0, 1.5])
                .window("end", (7.0, 7.0), (0.0, 0.0))
                .window("side", (8.0, 8.0), (0.0, 0.0)),
            End::wall([6.5, 8.0], [8.0, 8.0])
                .window("end", (0.0, 0.0), (1.0, 1.0))
                .window("side", (6.8, 6.8), (0.0, 0.0)),
        ],
    );
    let evaluation = evaluate_with(model(), Some(ends), vec![]);
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{evaluation:?}");
    assert_eq!(found[0].0, "end");
    assert!(found[0].1.contains("(6.5, 8)–(8, 8)"), "{}", found[0].1);
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_room_that_is_not_a_corridor_is_not_checked() {
    // The stub knows only the corridor: asking for the office would panic.
    let evaluation = evaluate_with(model(), Some(straight()), vec![]);
    assert!(
        evaluation
            .findings()
            .iter()
            .all(|finding| finding.related != vec![id("office")])
    );
    assert!(!findings(&evaluation).iter().any(|(o, _)| o == "office-end"));
}

#[test]
fn an_undecided_end_wall_or_a_straddling_contact_is_not_evaluated() {
    let ends = Ends::default().space(
        "hall",
        vec![
            End::undecided("the path is too short to give its direction"),
            End::wall([20.0, 0.0], [20.0, 2.0])
                .window("end", (0.0, 0.0), (0.05, 0.2))
                .window("side", (9.0, 9.0), (0.0, 0.0)),
        ],
    );
    let evaluation = evaluate_with(model(), Some(ends), vec![]);
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("end".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("side".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(message.contains("too short"), "{message}");
    assert!(message.contains("straddles"), "{message}");

    // A wall found elsewhere still stands beside an undecided end.
    let ends = Ends::default().space(
        "hall",
        vec![
            End::undecided("the path is too short to give its direction"),
            End::wall([20.0, 0.0], [20.0, 2.0])
                .window("end", (0.0, 0.0), (1.0, 1.0))
                .window("side", (9.0, 9.0), (0.0, 0.0)),
        ],
    );
    let evaluation = evaluate_with(model(), Some(ends), vec![]);
    assert_eq!(findings(&evaluation).len(), 1);
    assert_eq!(
        unevaluated(&evaluation),
        [("side".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn the_tolerances_are_the_rules() {
    // A window 0.6 m behind the end wall's face: outside the default depth,
    // inside a deeper one; facing only 0.1 m is not enough by default.
    let ends = || {
        Ends::default().space(
            "hall",
            vec![
                End::wall([20.0, 0.0], [20.0, 2.0])
                    .window("end", (0.6, 0.6), (1.0, 1.0))
                    .window("side", (0.0, 0.0), (0.1, 0.1)),
            ],
        )
    };
    let evaluation = evaluate_with(model(), Some(ends()), vec![]);
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let evaluation = evaluate_with(
        model(),
        Some(ends()),
        vec![("wall_depth", number(0.7)), ("facing", number(0.05))],
    );
    let mut found: Vec<String> = findings(&evaluation).into_iter().map(|(o, _)| o).collect();
    found.sort();
    assert_eq!(found, ["end", "side"]);
    let evaluation = evaluate_with(model(), Some(ends()), vec![("facing", number(-1.0))]);
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn without_the_plan_span_service_nothing_is_evaluated() {
    let evaluation = evaluate_with(model(), None, vec![]);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

/// Each window judged by an expression over the end walls of the corridors
/// it bounds: none within the wall depth and facing more than the minimum.
/// It flags and leaves open what `corridor-end-openings` does.
#[test]
#[allow(clippy::too_many_lines, clippy::type_complexity)]
fn end_walls_as_members_reach_the_verdicts() {
    use serde_json::json;
    let field =
        |name: &str| json!({"kind": "property", "propertySet": "axioval:member", "property": name});
    let m = |value: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}});
    let requirement = |depth: f64, facing: f64| {
        json!({"kind": "aggregate", "function": "none",
            "over": {"kind": "measured", "name": "end_walls;corridor=bounds:forward;kinds=corridor"},
            "value": {"kind": "and", "operands": [
                {"kind": "compare", "operator": "lessThanOrEquals", "left": field("gap"), "right": m(depth)},
                {"kind": "compare", "operator": "greaterThan", "left": field("facing"), "right": m(facing)}]}})
    };
    let tolerances = || {
        Ends::default().space(
            "hall",
            vec![
                End::wall([20.0, 0.0], [20.0, 2.0])
                    .window("end", (0.6, 0.6), (1.0, 1.0))
                    .window("side", (0.0, 0.0), (0.1, 0.1)),
            ],
        )
    };
    let l_shaped = || {
        Ends::default().space(
            "hall",
            vec![
                End::wall([0.0, 0.0], [0.0, 1.5])
                    .window("end", (7.0, 7.0), (0.0, 0.0))
                    .window("side", (8.0, 8.0), (0.0, 0.0)),
                End::wall([6.5, 8.0], [8.0, 8.0])
                    .window("end", (0.0, 0.0), (1.0, 1.0))
                    .window("side", (6.8, 6.8), (0.0, 0.0)),
            ],
        )
    };
    let straddling = || {
        Ends::default().space(
            "hall",
            vec![
                End::undecided("the path is too short to give its direction"),
                End::wall([20.0, 0.0], [20.0, 2.0])
                    .window("end", (0.0, 0.0), (0.05, 0.2))
                    .window("side", (9.0, 9.0), (0.0, 0.0)),
            ],
        )
    };
    let beside_undecided = || {
        Ends::default().space(
            "hall",
            vec![
                End::undecided("the path is too short to give its direction"),
                End::wall([20.0, 0.0], [20.0, 2.0])
                    .window("end", (0.0, 0.0), (1.0, 1.0))
                    .window("side", (9.0, 9.0), (0.0, 0.0)),
            ],
        )
    };
    let cases: Vec<(
        fn() -> Ends,
        Vec<(&'static str, ParameterValue)>,
        (f64, f64),
    )> = vec![
        (straight, vec![], (0.5, 0.1)),
        (l_shaped, vec![], (0.5, 0.1)),
        (straddling, vec![], (0.5, 0.1)),
        (beside_undecided, vec![], (0.5, 0.1)),
        (tolerances, vec![], (0.5, 0.1)),
        (
            tolerances,
            vec![("wall_depth", number(0.7)), ("facing", number(0.05))],
            (0.7, 0.05),
        ),
    ];
    for (index, (ends, extra, (depth, facing))) in cases.into_iter().enumerate() {
        let expected = evaluate_with(model(), Some(ends()), extra);
        let rule = rule(
            "axioval:capability.expression",
            kind("window"),
            vec![(
                "requirement",
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement(depth, facing)).unwrap(),
                },
            )],
        );
        let outcome =
            model().evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                services
                    .register(PlanSpanServiceHandle::new(Arc::new(ends())))
                    .unwrap();
            });
        let parity = axioval_rules::parity::compare_evaluations(
            (CAPABILITY, &expected),
            ("expression", &outcome),
        );
        assert!(parity.holds(), "case {index}:\n{}", parity.diff());
    }
}
