//! Template features held to what their documentation says on small
//! templates of their own: two member populations, a ratio whose
//! denominator may be zero, and what `area-ratio` composes besides.
#![allow(missing_docs)]

mod common;

use axioval_engine::template::{
    Decision, Derived, Form, Members, Operand, Template, TemplateValue, Term, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{AggregateFunction, Expression, ParameterValue, Selector};
use axioval_rules::templates::{ForkError, Templated, fork};
use common::{Model, findings, kind, number, rule, selector, string, unevaluated};

const ID: &str = "test:window-share";

fn count(name: &'static str, selector: &str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Aggregate {
            function: AggregateFunction::Count,
            over: Members::source(selector),
            filter: None,
            value: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// Windows per wall of each room: two populations along one relationship,
/// their counts' ratio within `minimum` and `maximum`.
fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("numerator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("denominator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
        ]
        .into_iter()
        .chain([ParameterDescriptor::optional(
            "relationship",
            ParameterType::String,
        )])
        .collect(),
        grades: false,
        name: "window-share",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration: Vec::new(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![
                count("windows", "numerator_selector"),
                count("walls", "denominator_selector"),
            ],
            decision: Decision::Within {
                value: "share",
                minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
                maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
                rounding: Vec::new(),
            },
            fail: "{windows:least} window(s) to {walls:least} wall(s); required {required}",
            undecided: "the share straddles {bound:plain}",
            members: Some(Members {
                selector: "numerator_selector",
                undecided: UndecidedMembers::Widen,
                every_when_unstated: false,
                same_ends: None,
                more: &["denominator_selector"],
                checks: Vec::new(),
            }),
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: vec![Derived::Ratio {
                name: "share",
                numerator: "windows",
                denominator: "walls",
                zero: "the room has no wall {relation}",
            }],
            related: None,
            checks: Vec::new(),
        }],
    }
}

fn rooms() -> Model {
    Model::default()
        .object("r1", "room")
        .object("r2", "room")
        .object("r3", "room")
        .object("w1", "window")
        .object("w2", "window")
        .object("x1", "wall")
        .object("x2", "wall")
        .object("x3", "wall")
        .edge("bounds", "r1", "w1")
        .edge("bounds", "r1", "w2")
        .edge("bounds", "r1", "x1")
        .edge("bounds", "r2", "x2")
        .edge("bounds", "r2", "x3")
}

fn parameters(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("numerator_selector", selector(kind("window"))),
        ("denominator_selector", selector(kind("wall"))),
        ("relationship", string("bounds")),
    ];
    parameters.extend(extra);
    parameters
}

/// Each population is counted on its own along the same relationship, and
/// their ratio decides; a room without a wall cannot be divided by.
#[test]
fn two_populations_stand_in_a_ratio() {
    let templated = Templated::new(template());
    let evaluation = rooms().evaluate(
        &templated,
        &rule(ID, kind("room"), parameters(vec![("maximum", number(1.0))])),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "r1".to_owned(),
            "2 window(s) to 1 wall(s); required at most 1".to_owned()
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related.len(),
        3,
        "the windows and the wall"
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("r3".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "the room has no wall via bounds"
    );
}

/// A denominator that may be zero leaves the ratio without an upper bound
/// (where the evaluator's division would refuse): a minimum it surely
/// exceeds still passes, a maximum it may exceed straddles.
#[test]
fn a_denominator_that_may_be_zero_has_no_upper_bound() {
    let templated = Templated::new(template());
    // Whether r1's one wall is external cannot be read: its two windows
    // stand to zero or one external wall.
    let external = Selector::property(
        Some("Pset".into()),
        "External",
        axioval_ir::contract::ComparisonOperator::Equals,
        Some(common::boolean(true)),
    );
    let judge = |extra: Vec<(&'static str, ParameterValue)>| {
        let mut declared = parameters(extra);
        declared[1] = ("denominator_selector", selector(external.clone()));
        rooms()
            .unreadable("x1")
            .evaluate(&templated, &rule(ID, kind("room"), declared))
    };
    let at_least = judge(vec![("minimum", number(1.0))]);
    assert!(
        unevaluated(&at_least).iter().all(|(room, _)| room != "r1"),
        "{:?}",
        unevaluated(&at_least)
    );
    assert!(findings(&at_least).iter().all(|(room, _)| room != "r1"));
    let at_most = judge(vec![("maximum", number(5.0))]);
    assert!(
        at_most
            .not_evaluated_outcomes()
            .iter()
            .any(|outcome| outcome.message() == "the share straddles at most 5"),
        "{:?}",
        at_most.not_evaluated_outcomes()
    );
}

/// A derived value has no expression form the evaluator decides alike.
#[test]
fn a_derived_value_is_not_forked() {
    let templated = Templated::new(template());
    assert!(matches!(
        fork(
            &templated,
            &rule(ID, kind("room"), parameters(vec![("maximum", number(1.0))]))
        ),
        Err(ForkError::Inexpressible(_))
    ));
}

/// Members that leave an anchor open before anything is read, members
/// judged on their own, findings relating one population, a plain-number
/// column, composed text conditions and a mode excluding an option: what
/// `area-ratio` composes, on a small template of their own.
mod checked {
    use super::*;
    use axioval_engine::template::{Check, Column, Condition, MemberCheck, NUMBER, Table, Text};
    use axioval_ir::contract::ComparisonOperator;
    use axioval_ir::{PropertyValue, QuantityDimension, ReportColumnKind};

    const ID: &str = "test:checked-share";

    const MANY: Condition = Condition::All {
        conditions: &[
            Condition::Not {
                condition: &Condition::Zero { value: "windows" },
            },
            Condition::Not {
                condition: &Condition::Zero { value: "walls" },
            },
        ],
    };

    fn stated(name: &'static str, property: &str) -> TemplateValue {
        TemplateValue {
            name,
            expression: Expression::Property {
                property_set: Some("Pset".into()),
                property: property.into(),
                of: None,
                label: None,
            },
            expect: None,
            absent: None,
            mismatch: None,
        }
    }

    /// Windows per wall of each room, each window no wider than `widest`.
    fn template() -> Template {
        let mut template = super::template();
        template.id = ID;
        template.parameters.extend([
            ParameterDescriptor::optional("widest", ParameterType::Number),
            ParameterDescriptor::optional("mode", ParameterType::String),
            ParameterDescriptor::optional("strict", ParameterType::Boolean),
        ]);
        template.declaration = vec![Check::Excludes {
            when: "strict",
            parameters: &["mode"],
            value: "loose",
            message: "a strict rule is never loose",
        }];
        template.texts = vec![
            Text {
                name: "both",
                when: Some(MANY),
                text: " (both counted)",
            },
            Text {
                name: "both",
                when: None,
                text: "",
            },
        ];
        let form = &mut template.forms[0];
        form.fail = "{windows:least} window(s) to {walls:least} wall(s){both}; required \
                     {required}";
        form.related = Some("members:numerator_selector");
        form.table = Some(Table {
            name: "shares",
            columns: vec![
                Column {
                    id: "windows",
                    value: "windows",
                    dimension: NUMBER,
                },
                Column {
                    id: "share",
                    value: "share",
                    dimension: NUMBER,
                },
            ],
        });
        let members = form.members.as_mut().unwrap();
        members.undecided = UndecidedMembers::Open {
            message: "{undecided} member(s) {relation} are undecided",
        };
        members.checks = vec![MemberCheck {
            before: "walls",
            values: vec![stated("width", "Width")],
            decision: Decision::Within {
                value: "width",
                minimum: None,
                maximum: Some(vec![Term::plus(Operand::Parameter("widest"))]),
                rounding: Vec::new(),
            },
            fail: "the window is {width:area} wide; required {bound:plain}",
            undecided: "the window's width straddles {bound:plain}",
            open: "the window's width cannot be read: {why}",
            failed: "{failed} window(s), first {first}, are too wide",
        }];
        template
    }

    fn sized(model: Model, window: &str, width: f64) -> Model {
        model.value(
            window,
            "Pset",
            "Width",
            PropertyValue::Quantity {
                value: width,
                dimension: QuantityDimension::Length,
            },
        )
    }

    /// An undecided member leaves its anchor open before any value is read,
    /// worded with the count and the relation.
    #[test]
    fn an_undecided_member_leaves_the_anchor_open_first() {
        let templated = Templated::new(template());
        let external = Selector::property(
            Some("Pset".into()),
            "External",
            ComparisonOperator::Equals,
            Some(common::boolean(true)),
        );
        let mut declared = parameters(vec![("maximum", number(5.0))]);
        declared[1] = ("denominator_selector", selector(external));
        let evaluation = rooms()
            .unreadable("x1")
            .evaluate(&templated, &rule(ID, kind("room"), declared));
        let open: Vec<(String, String)> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .map(|outcome| {
                (
                    outcome.object_id().unwrap().local_id.clone(),
                    outcome.message().to_owned(),
                )
            })
            .collect();
        assert!(
            open.contains(&("r1".into(), "1 member(s) via bounds are undecided".into())),
            "{open:?}"
        );
    }

    /// A finding relates the one population named, the composed texts
    /// hold, and the table's columns are plain numbers.
    #[test]
    fn a_finding_relates_the_named_population_and_numbers_are_tabled() {
        let templated = Templated::new(template());
        let evaluation = rooms().evaluate(
            &templated,
            &rule(ID, kind("room"), parameters(vec![("maximum", number(1.0))])),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "r1".to_owned(),
                "2 window(s) to 1 wall(s) (both counted); required at most 1".to_owned()
            )]
        );
        let related: Vec<&str> = evaluation.findings()[0]
            .related
            .iter()
            .map(|object| object.local_id.as_str())
            .collect();
        assert_eq!(related, ["w1", "w2"], "the windows only");
        let table = &evaluation.tables()[0];
        assert!(
            table
                .columns()
                .iter()
                .all(|column| matches!(column.kind, ReportColumnKind::Number))
        );
    }

    /// A member failing its check is a finding on the member relating the
    /// anchor, reported once however many anchors reach it; the anchor is
    /// open once the value the check precedes is read.
    #[test]
    fn members_are_judged_on_their_own_and_found_once() {
        let templated = Templated::new(template());
        let model = sized(sized(rooms(), "w1", 2.0), "w2", 0.5).edge("bounds", "r2", "w1");
        let evaluation = model.evaluate(
            &templated,
            &rule(
                ID,
                kind("room"),
                parameters(vec![("maximum", number(5.0)), ("widest", number(1.0))]),
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "w1".to_owned(),
                "the window is 2 wide; required at most 1".to_owned()
            )]
        );
        assert_eq!(
            evaluation.findings()[0].related[0].local_id,
            "r1",
            "the anchor that judged it first"
        );
        let mut open: Vec<(String, String)> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .map(|outcome| {
                (
                    outcome.object_id().unwrap().local_id.clone(),
                    outcome.message().to_owned(),
                )
            })
            .collect();
        open.sort();
        assert_eq!(
            open,
            [
                (
                    "r1".to_owned(),
                    "1 window(s), first test:model/w1, are too wide".to_owned()
                ),
                (
                    "r2".to_owned(),
                    "1 window(s), first test:model/w1, are too wide".to_owned()
                ),
                (
                    "r3".to_owned(),
                    "the room has no wall via bounds".to_owned()
                ),
            ]
        );
    }

    /// A mode excludes an option where it is stated.
    #[test]
    fn a_mode_excludes_an_option() {
        let templated = Templated::new(template());
        let evaluation = rooms().evaluate(
            &templated,
            &rule(
                ID,
                kind("room"),
                parameters(vec![
                    ("maximum", number(5.0)),
                    ("strict", common::boolean(true)),
                    ("mode", string("loose")),
                ]),
            ),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "window-share: a strict rule is never loose"
        );
    }
}

/// Values that leave an object unjudged, graded severities chosen by
/// conditions over values read only once a finding stands, refusals per
/// object after a prefix, a check while a flag is on and numbers shown with
/// fixed decimals: what `slab-contact` composes, on a small template of its
/// own.
mod graded {
    use super::*;
    use axioval_engine::template::{
        Applies, Band, Check, Condition, Grading, Refusals, Text, Unless,
    };
    use axioval_ir::{PropertyValue, Severity};

    const ID: &str = "test:graded-share";

    fn stated(name: &'static str, property: &str) -> TemplateValue {
        TemplateValue {
            name,
            expression: Expression::Property {
                property_set: Some("Pset".into()),
                property: property.into(),
                of: None,
                label: None,
            },
            expect: None,
            absent: None,
            mismatch: None,
        }
    }

    /// Each panel's share at least `minimum`, unless it is exempt where
    /// exemptions are honoured; a shortfall graded by its gap.
    fn template() -> Template {
        Template {
            id: ID,
            parameters: vec![
                ParameterDescriptor::required("minimum", ParameterType::Number),
                ParameterDescriptor::optional("honour_exemptions", ParameterType::Boolean),
                ParameterDescriptor::optional("exemption_note", ParameterType::String),
            ],
            grades: false,
            name: "graded-share",
            refusals: Refusals::Prefixed {
                prefix: "graded-share declaration is invalid",
            },
            defaults: Vec::new(),
            declaration: vec![
                Check::Finite {
                    parameters: &["minimum"],
                    above: Some(0.0),
                    at_least: None,
                    message: "minimum must be positive",
                },
                Check::When {
                    flags: &["honour_exemptions"],
                    check: &Check::AnyOf {
                        parameters: &["exemption_note"],
                        message: "honouring exemptions needs `exemption_note`",
                    },
                },
            ],
            services: None,
            texts: vec![
                Text {
                    name: "shortfall",
                    when: Some(Condition::Zero { value: "share" }),
                    text: "nothing shared",
                },
                Text {
                    name: "shortfall",
                    when: None,
                    text: "share {share:lower4} below {minimum:fixed4}",
                },
            ],
            forms: vec![Form {
                when: &[],
                values: vec![stated("share", "Share")],
                decision: Decision::Within {
                    value: "share",
                    minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
                    maximum: None,
                    rounding: Vec::new(),
                },
                fail: "{shortfall}",
                undecided: "the share straddles {bound:plain}",
                members: None,
                table: None,
                scope: None,
                unless: vec![Unless {
                    applies: Applies {
                        when: &["honour_exemptions"],
                        any: &[],
                        condition: None,
                    },
                    value: stated("exempt", "Exempt"),
                }],
                grading: Some(Grading {
                    values: vec![stated("gap", "Gap")],
                    derived: Vec::new(),
                    undecided: Vec::new(),
                    bands: vec![
                        Band {
                            severity: Severity::Error,
                            when: Some(Condition::Absent { value: "gap" }),
                        },
                        Band {
                            severity: Severity::Info,
                            when: Some(Condition::Below {
                                value: "gap",
                                than: 0.1,
                            }),
                        },
                        Band {
                            severity: Severity::Warning,
                            when: Some(Condition::Above {
                                value: "gap",
                                than: 0.5,
                            }),
                        },
                    ],
                }),
                derived: Vec::new(),
                related: None,
                checks: Vec::new(),
            }],
        }
    }

    fn panels() -> Model {
        let number = PropertyValue::Decimal;
        Model::default()
            .object("full", "panel")
            .value("full", "Pset", "Share", number(0.8))
            .object("none", "panel")
            .value("none", "Pset", "Share", number(0.0))
            .value("none", "Pset", "Gap", PropertyValue::Null)
            .object("near", "panel")
            .value("near", "Pset", "Share", number(0.25))
            .value("near", "Pset", "Gap", number(0.05))
            .object("far", "panel")
            .value("far", "Pset", "Share", number(0.25))
            .value("far", "Pset", "Gap", number(0.7))
            .object("between", "panel")
            .value("between", "Pset", "Share", number(0.25))
            .value("between", "Pset", "Gap", number(0.3))
            .object("exempt", "panel")
            .value("exempt", "Pset", "Share", number(0.1))
            .value("exempt", "Pset", "Exempt", PropertyValue::Boolean(true))
            .value("exempt", "Pset", "Gap", number(0.3))
    }

    fn severities(evaluation: &axioval_engine::CapabilityEvaluation) -> Vec<(String, Severity)> {
        evaluation
            .findings()
            .iter()
            .map(|finding| (common::subject(finding), finding.severity.clone()))
            .collect()
    }

    /// A finding takes the severity of the first band whose condition holds
    /// over the values read to grade it (`between`'s gap holds none, and
    /// takes the rule's own); messages read them too.
    #[test]
    fn a_finding_is_graded_by_the_first_band_that_holds() {
        let templated = Templated::new(template());
        let evaluation = panels().evaluate(
            &templated,
            &rule(ID, kind("panel"), vec![("minimum", number(0.5))]),
        );
        let mut found = severities(&evaluation);
        found.sort();
        assert_eq!(
            found,
            [
                ("between".to_owned(), Severity::Error),
                ("exempt".to_owned(), Severity::Error),
                ("far".to_owned(), Severity::Warning),
                ("near".to_owned(), Severity::Info),
                ("none".to_owned(), Severity::Error),
            ]
        );
        let messages = findings(&evaluation);
        assert!(messages.contains(&("none".to_owned(), "nothing shared".to_owned())));
        assert!(messages.contains(&("near".to_owned(), "share 0.2500 below 0.5000".to_owned())));
    }

    /// A value surely true leaves its object unjudged where it applies, and
    /// is not read where it does not.
    #[test]
    fn a_value_leaves_its_object_unjudged_where_it_applies() {
        let templated = Templated::new(template());
        let honoured = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![
                    ("minimum", number(0.5)),
                    ("honour_exemptions", common::boolean(true)),
                    ("exemption_note", string("signed off")),
                ],
            ),
        );
        assert!(
            severities(&honoured)
                .iter()
                .all(|(panel, _)| panel != "exempt")
        );
        // Where it applies, a value stated absent is a missing-information
        // finding, as any value's.
        assert!(
            findings(&honoured)
                .contains(&("near".to_owned(), "`exempt` is stated absent".to_owned())),
            "{:?}",
            findings(&honoured)
        );
    }

    /// A refused declaration is reported for each object after the prefix,
    /// and a check applies only while its flag is on.
    #[test]
    fn a_refusal_is_reported_per_object_after_its_prefix() {
        let templated = Templated::new(template());
        let refused = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![
                    ("minimum", number(0.5)),
                    ("honour_exemptions", common::boolean(true)),
                ],
            ),
        );
        assert_eq!(refused.not_evaluated_outcomes().len(), 6);
        assert!(refused.not_evaluated_outcomes().iter().all(|outcome| {
            outcome.reason() == &NotEvaluatedReason::InvalidDeclaration
                && outcome.message()
                    == "graded-share declaration is invalid: honouring exemptions needs \
                        `exemption_note`"
        }));
        let off = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![
                    ("minimum", number(0.5)),
                    ("honour_exemptions", common::boolean(false)),
                ],
            ),
        );
        assert!(off.not_evaluated_outcomes().is_empty());
    }

    /// A rule forked from such a form passes an object a value leaves
    /// unjudged.
    #[test]
    fn a_fork_passes_what_a_value_leaves_unjudged() {
        let templated = Templated::new(template());
        let forked = fork(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![
                    ("minimum", number(0.5)),
                    ("honour_exemptions", common::boolean(true)),
                    ("exemption_note", string("signed off")),
                ],
            ),
        )
        .unwrap();
        let text = serde_json::to_string(&forked.requirement).unwrap();
        assert!(text.contains("unless exempt"), "{text}");
        assert!(text.contains("\"kind\":\"or\""), "{text}");
    }
}
