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

/// A value that may be stated absent, checks applying only where a string
/// list names them, a check judging a value near another, the refusals a
/// declaration words once while services are missed per object, and the
/// declaration checks over a string list: what `slab-stack-spacing`
/// composes, on a small template of its own.
mod near {
    use super::*;
    use axioval_engine::template::{
        Applies, Check, Condition, Expect, FormCheck, Reference, Refusals, Service, Services,
    };
    use axioval_ir::PropertyValue;

    const ID: &str = "test:near-share";

    fn stated(name: &'static str, property: &str, expect: Option<Expect>) -> TemplateValue {
        TemplateValue {
            name,
            expression: Expression::Property {
                property_set: Some("Pset".into()),
                property: property.into(),
                of: None,
                label: None,
            },
            expect,
            absent: None,
            mismatch: None,
        }
    }

    /// Each panel's height near its reference where `compare` lists
    /// `height`; nothing judged where it states no height.
    fn template(services: bool) -> Template {
        Template {
            id: ID,
            parameters: vec![
                ParameterDescriptor::optional("compare", ParameterType::StringList),
                ParameterDescriptor::optional("tolerance", ParameterType::Number),
            ],
            grades: false,
            name: "near-share",
            refusals: Refusals::ServicesPerObject,
            defaults: Vec::new(),
            declaration: vec![
                Check::AmongEach {
                    parameter: "compare",
                    options: &["height", "width"],
                    message: "compare names `{value}`",
                },
                Check::DeclaresListed {
                    parameters: &["compare"],
                    message: "compare something",
                },
            ],
            services: services.then(|| Services {
                needs: vec![Service::PlanArea],
                message: "no plan areas",
            }),
            texts: Vec::new(),
            forms: vec![Form {
                when: &[],
                values: vec![stated("height", "Height", Some(Expect::Optional))],
                decision: Decision::Within {
                    value: "height",
                    minimum: None,
                    maximum: None,
                    rounding: Vec::new(),
                },
                fail: "",
                undecided: "",
                members: None,
                table: None,
                scope: None,
                unless: Vec::new(),
                grading: None,
                derived: Vec::new(),
                related: None,
                checks: vec![FormCheck {
                    values: vec![stated("reference", "Reference", Some(Expect::Optional))],
                    decision: Decision::Near {
                        value: "height",
                        reference: Reference::Value("reference"),
                        tolerance: Operand::Parameter("tolerance"),
                    },
                    fail: "height {height} differs from {reference}",
                    undecided: "height {height} may differ from {reference}",
                    related: None,
                    grading: None,
                    applies: Some(Applies {
                        when: &[],
                        any: &[],
                        condition: Some(Condition::Lists {
                            parameter: "compare",
                            value: "height",
                        }),
                    }),
                }],
            }],
        }
    }

    fn panels() -> Model {
        let number = PropertyValue::Decimal;
        Model::default()
            .object("near", "panel")
            .value("near", "Pset", "Height", number(3.0))
            .value("near", "Pset", "Reference", number(3.05))
            .object("far", "panel")
            .value("far", "Pset", "Height", number(3.0))
            .value("far", "Pset", "Reference", number(3.5))
            .object("bare", "panel")
            .object("alone", "panel")
            .value("alone", "Pset", "Height", number(3.0))
    }

    #[test]
    fn a_check_judges_near_a_value_where_a_list_names_it() {
        let templated = Templated::new(template(false));
        let judged = |compare: &[&str]| {
            panels().evaluate(
                &templated,
                &rule(
                    ID,
                    kind("panel"),
                    vec![
                        ("compare", common::strings(compare)),
                        ("tolerance", number(0.1)),
                    ],
                ),
            )
        };
        let height = judged(&["height"]);
        assert_eq!(
            findings(&height),
            [("far".to_owned(), "height 3 differs from 3.5".to_owned())]
        );
        assert!(height.not_evaluated_outcomes().is_empty());
        assert!(findings(&judged(&["width"])).is_empty());
    }

    #[test]
    fn a_declaration_is_refused_once_and_services_per_object() {
        let refused = panels().evaluate(
            &Templated::new(template(false)),
            &rule(
                ID,
                kind("panel"),
                vec![("compare", common::strings(&[" depth"]))],
            ),
        );
        assert_eq!(
            refused.not_evaluated_outcomes()[0].message(),
            "near-share: compare names ` depth`"
        );
        let empty = panels().evaluate(
            &Templated::new(template(false)),
            &rule(ID, kind("panel"), vec![("compare", common::strings(&[]))]),
        );
        assert_eq!(
            unevaluated(&empty),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        let unserved = panels().evaluate(
            &Templated::new(template(true)),
            &rule(
                ID,
                kind("panel"),
                vec![("compare", common::strings(&["height"]))],
            ),
        );
        assert_eq!(unserved.not_evaluated_outcomes().len(), 4);
        assert!(
            unserved
                .not_evaluated_outcomes()
                .iter()
                .all(|outcome| outcome.message() == "no plan areas")
        );
    }
}

/// A value one of two parameters states, thresholds read as parameters of
/// a grading and of the texts wording it, checks applying where a
/// parameter is at least a number, and the declaration checks stated as
/// conditions, over parameters strictly increasing, of a dimension and
/// over an angle: what `counterpart-coverage` composes, on a small template
/// of its own.
mod thresholds {
    use super::*;
    use axioval_engine::template::{
        Applies, Band, Check, Condition, End, FormCheck, Grading, ParameterDefault, Refusals, Text,
    };
    use axioval_ir::contract::ScalarValue;
    use axioval_ir::{PropertyValue, QuantityDimension, Severity};

    const ID: &str = "test:thresholds";

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

    const fn exceeds(parameter: &'static str, end: End) -> Condition {
        Condition::Exceeds {
            value: "share",
            parameter,
            end,
        }
    }

    const HIGH_UPPER: Condition = exceeds("high_above", End::Upper);
    const HIGH_LOWER: Condition = exceeds("high_above", End::Lower);

    /// Each panel's share at most the lower of two thresholds stated,
    /// graded by the higher one it may exceed, where the growth (`growth`,
    /// or `own_growth`) is not negative.
    #[allow(clippy::too_many_lines)]
    fn template() -> Template {
        Template {
            id: ID,
            parameters: vec![
                ParameterDescriptor::optional("low_above", ParameterType::Number),
                ParameterDescriptor::optional("high_above", ParameterType::Number),
                ParameterDescriptor::optional("growth", ParameterType::Quantity),
                ParameterDescriptor::optional("own_growth", ParameterType::Quantity),
                ParameterDescriptor::optional("turn", ParameterType::Quantity),
            ],
            grades: true,
            name: "thresholds",
            refusals: Refusals::Rule,
            defaults: vec![
                ParameterDefault {
                    parameter: "effective",
                    value: ScalarValue::Quantity {
                        value: 0.0,
                        unit: "m".into(),
                    },
                    from: &["growth", "own_growth"],
                },
                ParameterDefault {
                    parameter: "lowest",
                    value: ScalarValue::Number { value: 0.0 },
                    from: &["low_above", "high_above"],
                },
            ],
            declaration: vec![
                Check::Quantity {
                    parameter: "growth",
                    dimension: QuantityDimension::Length,
                    message: "growth must be a length",
                },
                Check::AnyOf {
                    parameters: &["low_above", "high_above"],
                    message: "declare a threshold",
                },
                Check::Holds {
                    condition: Condition::Not {
                        condition: &Condition::Under {
                            parameter: "high_above",
                            than: 0.0,
                        },
                    },
                    message: "high_above must not be negative",
                },
                Check::Exceeds {
                    parameter: "high_above",
                    earlier: &["low_above"],
                    message: "high_above must exceed low_above",
                },
                Check::AngleBelow {
                    parameter: "turn",
                    below: 45.0,
                    range: "turn must lie in [0, 45) degrees",
                    angle: "turn must be a plane angle",
                },
            ],
            services: None,
            texts: vec![Text {
                name: "graded",
                when: Some(Condition::All {
                    conditions: &[
                        HIGH_UPPER,
                        Condition::Not {
                            condition: &HIGH_LOWER,
                        },
                    ],
                }),
                text: "; graded error by its upper bound",
            }],
            forms: vec![Form {
                when: &[],
                values: vec![stated("share", "Share")],
                decision: Decision::Within {
                    value: "share",
                    minimum: None,
                    maximum: None,
                    rounding: Vec::new(),
                },
                fail: "",
                undecided: "",
                members: None,
                table: None,
                scope: None,
                unless: Vec::new(),
                grading: None,
                derived: Vec::new(),
                related: None,
                checks: vec![FormCheck {
                    values: Vec::new(),
                    decision: Decision::Within {
                        value: "share",
                        minimum: None,
                        maximum: Some(vec![Term::plus(Operand::Parameter("lowest"))]),
                        rounding: Vec::new(),
                    },
                    fail: "share {share:area} above {lowest} grown by {effective:si} m{graded}",
                    undecided: "share {share:area} straddles {lowest}",
                    related: None,
                    grading: Some(Grading {
                        values: Vec::new(),
                        derived: Vec::new(),
                        undecided: Vec::new(),
                        bands: vec![
                            Band {
                                severity: Severity::Error,
                                when: Some(exceeds("high_above", End::Upper)),
                            },
                            Band {
                                severity: Severity::Warning,
                                when: Some(exceeds("low_above", End::Upper)),
                            },
                        ],
                    }),
                    applies: Some(Applies {
                        when: &[],
                        any: &[],
                        condition: Some(Condition::AtLeast {
                            parameter: "effective",
                            than: 0.0,
                        }),
                    }),
                }],
            }],
        }
    }

    fn panels() -> Model {
        let between = |lower: f64, upper: f64| PropertyValue::Measured {
            lower,
            upper,
            dimension: None,
        };
        Model::default()
            .object("low", "panel")
            .value("low", "Pset", "Share", between(0.05, 0.05))
            .object("middle", "panel")
            .value("middle", "Pset", "Share", between(0.3, 0.4))
            .object("across", "panel")
            .value("across", "Pset", "Share", between(0.3, 0.7))
            .object("high", "panel")
            .value("high", "Pset", "Share", between(0.8, 0.9))
            .object("straddling", "panel")
            .value("straddling", "Pset", "Share", between(0.1, 0.3))
    }

    fn metres(value: f64) -> ParameterValue {
        ParameterValue::Quantity {
            value,
            unit: "m".into(),
        }
    }

    fn graded(
        evaluation: &axioval_engine::CapabilityEvaluation,
    ) -> Vec<(String, Severity, String)> {
        let mut found: Vec<_> = evaluation
            .findings()
            .iter()
            .map(|finding| {
                (
                    common::subject(finding),
                    finding.severity.clone(),
                    finding.message.clone(),
                )
            })
            .collect();
        found.sort();
        found
    }

    /// The threshold is the first of two parameters stated, the growth the
    /// first of two, and a finding is graded by the most severe threshold
    /// its upper end exceeds, worded so where its lower end does not.
    #[test]
    fn thresholds_and_values_are_read_from_the_first_parameter_stated() {
        let templated = Templated::new(template());
        let evaluation = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![
                    ("low_above", number(0.2)),
                    ("high_above", number(0.5)),
                    ("own_growth", metres(0.02)),
                ],
            ),
        );
        assert_eq!(
            graded(&evaluation),
            [
                (
                    "across".to_owned(),
                    Severity::Error,
                    "share between 0.3 and 0.7 above 0.2 grown by 0.02 m; graded error by its \
                     upper bound"
                        .to_owned()
                ),
                (
                    "high".to_owned(),
                    Severity::Error,
                    "share between 0.8 and 0.9 above 0.2 grown by 0.02 m".to_owned()
                ),
                (
                    "middle".to_owned(),
                    Severity::Warning,
                    "share between 0.3 and 0.4 above 0.2 grown by 0.02 m".to_owned()
                ),
            ]
        );
        assert_eq!(
            unevaluated(&evaluation),
            [(
                "straddling".to_owned(),
                NotEvaluatedReason::IncompleteEvidence
            )]
        );
        // Graded from the lowest threshold stated.
        let deviation = evaluation.deviation(0).expect("graded");
        assert!(deviation.lower() > 0.0, "{deviation:?}");

        // Without the lower threshold, the higher one is the lowest.
        let higher = panels().evaluate(
            &templated,
            &rule(ID, kind("panel"), vec![("high_above", number(0.5))]),
        );
        assert_eq!(
            graded(&higher)
                .into_iter()
                .map(|(panel, severity, _)| (panel, severity))
                .collect::<Vec<_>>(),
            [("high".to_owned(), Severity::Error)]
        );
        assert_eq!(unevaluated(&higher).len(), 1);
    }

    /// A check whose condition does not hold over the parameters is not
    /// judged: a negative growth switches it off.
    #[test]
    fn a_check_applies_where_a_parameter_is_at_least_a_number() {
        let templated = Templated::new(template());
        let off = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![("low_above", number(0.2)), ("growth", metres(-1.0))],
            ),
        );
        assert!(off.findings().is_empty());
        assert!(off.not_evaluated_outcomes().is_empty());
    }

    /// Declaration checks over a dimension, a condition, an increasing pair
    /// and an angle, each worded as stated after the name.
    #[test]
    fn declarations_are_checked_as_conditions_and_orders() {
        let templated = Templated::new(template());
        let refused = |parameters: Vec<(&'static str, ParameterValue)>| {
            let evaluation = panels().evaluate(&templated, &rule(ID, kind("panel"), parameters));
            let outcomes = evaluation.not_evaluated_outcomes();
            assert_eq!(outcomes.len(), 1, "{outcomes:?}");
            assert_eq!(
                outcomes[0].reason(),
                &NotEvaluatedReason::InvalidDeclaration
            );
            outcomes[0].message().to_owned()
        };
        assert_eq!(
            refused(vec![
                ("low_above", number(0.2)),
                (
                    "growth",
                    ParameterValue::Quantity {
                        value: 1.0,
                        unit: "m2".into()
                    }
                )
            ]),
            "thresholds: growth must be a length"
        );
        assert_eq!(refused(vec![]), "thresholds: declare a threshold");
        assert_eq!(
            refused(vec![("high_above", number(-0.1))]),
            "thresholds: high_above must not be negative"
        );
        assert_eq!(
            refused(vec![
                ("low_above", number(0.5)),
                ("high_above", number(0.5))
            ]),
            "thresholds: high_above must exceed low_above"
        );
        let turn = |value: f64, unit: &str| {
            (
                "turn",
                ParameterValue::Quantity {
                    value,
                    unit: unit.into(),
                },
            )
        };
        assert_eq!(
            refused(vec![("low_above", number(0.2)), turn(45.0, "deg")]),
            "thresholds: turn must lie in [0, 45) degrees"
        );
        assert_eq!(
            refused(vec![("low_above", number(0.2)), turn(1.0, "m")]),
            "thresholds: turn must be a plane angle"
        );
        let within = panels().evaluate(
            &templated,
            &rule(
                ID,
                kind("panel"),
                vec![("low_above", number(0.2)), turn(44.0, "deg")],
            ),
        );
        assert!(
            within
                .not_evaluated_outcomes()
                .iter()
                .all(|outcome| outcome.reason() != &NotEvaluatedReason::InvalidDeclaration)
        );
    }
}
