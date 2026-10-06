//! `stair-geometry` and `ramp-geometry` as templates: the flight or ramp
//! measured once (an object the walking-surface service cannot measure is
//! open once, for its reason), then every declared check a form check
//! judging the items of a measured list: the steps, runs, landings,
//! clearances, clear widths and handrails the walking-surface service
//! measures, and the searches answering one three-valued result per item
//! (end spaces, doors on and over landings, breaks, rails over surfaces,
//! tactile strips). Each failing item is its own finding (D1), as the
//! capabilities reported them.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, End, Every, Form, FormCheck, Group,
    Groups, Guard, ItemCheck, ItemTest, ItemText, ItemUnit, Items, Judge, Magnitude, On, OnNull,
    OpenItems, Operand, Passing, Range, Requirement, RowCheck, RowColumn, Rows, Service, Services,
    Spread, Template, TemplateValue, Together, TogetherJudge, When,
};
use axioval_ir::contract::Expression;

/// The ramp capability's id.
pub(crate) const RAMP: &str = "axioval:capability.ramp-geometry";

fn measured(name: &'static str, call: &str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: call.to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

fn applies(when: &'static [&'static str], any: &'static [&'static str]) -> Applies {
    Applies {
        when,
        any,
        condition: None,
    }
}

/// A parameter bound named after it.
fn parameter(name: &'static str) -> Requirement {
    Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Parameter(name)),
        }],
        words: "",
    }
}

fn slack(times: f64, value: &'static str) -> Allowance {
    Allowance::Slack {
        times,
        magnitude: Magnitude {
            end: End::Upper,
            operand: Operand::Value(value),
        },
    }
}

fn range(value: &'static str, unit: ItemUnit) -> Range {
    Range {
        value,
        unit,
        at_least: Vec::new(),
        at_most: Vec::new(),
        allowance: Allowance::None,
        grade: true,
        null: OnNull::Skip,
        unmeasured: None,
    }
}

fn test(judge: Judge, fail: &'static str, undecided: &'static str) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: None,
    }
}

fn items(applies: Applies, list: &'static str) -> Items {
    Items {
        applies,
        list,
        refused: Some("{why}"),
        checks: Vec::new(),
        together: None,
        passing: None,
        texts: Vec::new(),
    }
}

fn check(items: Items) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        decision: Decision::Items(Box::new(items)),
        fail: "",
        undecided: "",
        related: None,
    }
}

fn undecided(selector: &'static str) -> When {
    When::Undecided { selector }
}

fn pending(when: Vec<When>, on: On, message: &'static str) -> Effect {
    Effect { when, on, message }
}

fn text(name: &'static str, when: Vec<When>, text: &'static str) -> ItemText {
    ItemText { name, when, text }
}

/// A search's items: each found one a finding worded by the search, each
/// undecided one open with why.
fn searched(applies: Applies, list: &'static str) -> FormCheck {
    let mut found = test(
        Judge::Truth {
            value: "found",
            finding: true,
        },
        "{finding}",
        "{why}",
    );
    found.related = Some("objects");
    check(Items {
        checks: vec![ItemCheck::Test(Box::new(found))],
        ..items(applies, list)
    })
}

/// The clearance above or below: at least the minimum, an undecided
/// object able only to lower it.
fn clearance(
    (minimum, selector): (&'static [&'static str], &'static str),
    list: &'static str,
    (fail, straddles, nothing, passes): (&'static str, &'static str, &'static str, &'static str),
) -> FormCheck {
    let mut judged = test(
        Judge::Range(Box::new(Range {
            at_least: vec![parameter(minimum[0])],
            ..range("clearance", ItemUnit::Length)
        })),
        fail,
        straddles,
    );
    judged.effects = vec![
        pending(vec![undecided(selector)], On::Null, nothing),
        pending(vec![undecided(selector)], On::Pass, passes),
    ];
    judged.related = Some("governing");
    check(Items {
        checks: vec![ItemCheck::Test(Box::new(judged))],
        ..items(applies(minimum, &[]), list)
    })
}

/// The landing sizes the rule declares and whether a landing must be
/// there, at each item of `list`.
#[allow(clippy::too_many_lines)]
fn landings(
    list: &'static str,
    sizes: &'static [&'static str],
    any: &'static [&'static str],
) -> FormCheck {
    let mut required = test(
        Judge::Truth {
            value: "present",
            finding: false,
        },
        "no selected slab or landing meets {label}",
        "{why}",
    );
    required.applies = applies(&["landings_required"], &[]);
    required.effects = vec![pending(
        vec![undecided("landing_objects")],
        On::Fail,
        "no selected slab or landing meets {label}; an object the selection could not decide \
         may carry it",
    )];
    let dimension = |value: &'static str,
                     end: &'static str,
                     stated: &'static str,
                     fail: &'static str,
                     straddles: &'static str,
                     carried: &'static str| {
        let mut judged = test(
            Judge::Range(Box::new(Range {
                at_least: vec![
                    Requirement {
                        name: "stated",
                        options: vec![
                            Choice {
                                when: vec![When::Field {
                                    field: "outermost",
                                    value: true,
                                }],
                                bound: Bound::Operand(Operand::Parameter(end)),
                            },
                            Choice {
                                when: Vec::new(),
                                bound: Bound::Operand(Operand::Parameter(stated)),
                            },
                        ],
                        words: "{stated:length}",
                    },
                    Requirement {
                        name: "walking",
                        options: vec![Choice {
                            when: vec![When::Declared {
                                parameter: "landing_at_least_walking_width",
                            }],
                            bound: Bound::Operand(Operand::Value("walking")),
                        }],
                        words: "the {noun}'s width ({walking:length})",
                    },
                ],
                allowance: slack(2.0, "scale"),
                ..range(value, ItemUnit::Length)
            })),
            fail,
            straddles,
        );
        judged.effects = vec![pending(
            vec![undecided("landing_objects")],
            On::Fail,
            carried,
        )];
        judged.related = Some("carriers");
        ItemCheck::Test(Box::new(judged))
    };
    let dimensions = Group {
        applies: applies(&[], sizes),
        when: vec![When::Field {
            field: "present",
            value: true,
        }],
        guards: vec![
            Guard {
                field: "depth",
                when: Vec::new(),
                undecided: None,
                null: OnNull::Judge,
            },
            Guard {
                field: "width",
                when: Vec::new(),
                undecided: None,
                null: OnNull::Judge,
            },
            Guard {
                field: "walking",
                when: vec![When::Declared {
                    parameter: "landing_at_least_walking_width",
                }],
                undecided: None,
                null: OnNull::Open(
                    "the {noun}'s width is not measured, so the landing at {label} is not \
                     compared with it",
                ),
            },
        ],
        checks: vec![
            dimension(
                "depth",
                "end_landing_depth_minimum",
                "landing_depth_minimum",
                "the landing at {label} is {depth:length} deep; at least {requirements} required",
                "the landing at {label} is {depth:length} deep, which straddles at least \
                 {requirements} required",
                "the landing at {label} is {depth:length} deep; at least {requirements} \
                 required; an object the selection could not decide may carry it",
            ),
            dimension(
                "width",
                "end_landing_width_minimum",
                "landing_width_minimum",
                "the landing at {label} is {width:length} wide; at least {requirements} required",
                "the landing at {label} is {width:length} wide, which straddles at least \
                 {requirements} required",
                "the landing at {label} is {width:length} wide; at least {requirements} \
                 required; an object the selection could not decide may carry it",
            ),
        ],
    };
    check(Items {
        checks: vec![ItemCheck::Group(Box::new(Group {
            applies: Applies::default(),
            when: Vec::new(),
            guards: vec![Guard {
                field: "present",
                when: Vec::new(),
                undecided: None,
                null: OnNull::Judge,
            }],
            checks: vec![
                ItemCheck::Test(Box::new(required)),
                ItemCheck::Group(Box::new(dimensions)),
            ],
        }))],
        ..items(applies(&["landing_objects"], any), list)
    })
}

/// The width of every run, or of the flight, within the declared range,
/// in one outcome.
fn every_width(list: &'static str, name: &'static str, unmeasured: &'static str) -> FormCheck {
    check(Items {
        together: Some(Together {
            present: None,
            name,
            judge: TogetherJudge::Every(Box::new(Every {
                range: Range {
                    at_least: vec![parameter("width_minimum")],
                    at_most: vec![parameter("width_maximum")],
                    allowance: slack(1.0, "scale"),
                    null: OnNull::Judge,
                    ..range("width", ItemUnit::Length)
                },
                zero: None,
                item: "{name} is {width:length}",
                fail: "{failing}; {bound} required",
                open: OpenItems::Grouped {
                    straddling: "{items}, which straddles {bound}",
                    unmeasured: "{items} not measured",
                },
                unmeasured_any: Some(unmeasured),
            })),
        }),
        ..items(applies(&[], &["width_minimum", "width_maximum"]), list)
    })
}

/// The handrail checks along each stretch the lists name (`{lists}` the
/// arguments every handrail list takes).
#[allow(clippy::too_many_lines)]
fn handrail_checks(
    stretches: &'static str,
    heights: &'static str,
    extensions: &'static str,
    gaps: &'static str,
) -> Vec<FormCheck> {
    let rails = undecided("handrail_objects");
    let measured_stretch = || When::Field {
        field: "measured",
        value: true,
    };
    let passing = || Passing {
        when: vec![rails, measured_stretch()],
        message: "the handrails along {label} pass, but a rail the selection could not decide \
                  may run along it",
        groups: Some(Groups {
            list: stretches,
            key: "stretch",
        }),
    };
    // The sides a rail runs along.
    let one = "no selected handrail runs along a side of {label}; one side required";
    let one_pending = "no selected handrail runs along a side of {label}; one side required; a \
                       rail the selection could not decide may run along it";
    let both = "{found}; both sides required";
    let both_pending =
        "{found}; both sides required; a rail the selection could not decide may run along it";
    let fails = |when: Vec<When>, fail: &'static str, pending_fail: &'static str| {
        let mut failing = test(Judge::Fails, fail, "");
        failing.when = when;
        failing.effects = vec![pending(vec![rails], On::Fail, pending_fail)];
        failing.related = Some("on_sides");
        failing
    };
    let none = || When::Is {
        field: "sides",
        value: 0.0,
    };
    let short = || When::Below {
        field: "sides",
        value: 2.0,
    };
    let mut wider = test(
        Judge::Range(Box::new(Range {
            at_most: vec![parameter("handrail_both_sides_above_width")],
            allowance: slack(1.0, "scale"),
            grade: false,
            null: OnNull::Open(
                "the width of {label} is not measured, so whether it needs handrails on both \
                 sides is not known",
            ),
            ..range("width", ItemUnit::Length)
        })),
        "",
        "{label} is {width:length} wide, which straddles the \
         {handrail_both_sides_above_width:length} above which handrails are required on both \
         sides",
    );
    wider.when = vec![
        When::Equals {
            parameter: "handrail_sides",
            value: "one",
        },
        When::Declared {
            parameter: "handrail_both_sides_above_width",
        },
        short(),
    ];
    wider.then = Some(Box::new(fails(vec![none()], one, one_pending)));
    wider.otherwise = Some(Box::new(fails(Vec::new(), both, both_pending)));
    wider.straddled = Some(Box::new(fails(vec![none()], one, one_pending)));
    wider.related = Some("on_sides");
    let sides = Group {
        applies: applies(&["handrail_sides"], &[]),
        when: Vec::new(),
        guards: Vec::new(),
        checks: vec![
            ItemCheck::Test(Box::new(fails(
                vec![
                    When::Equals {
                        parameter: "handrail_sides",
                        value: "one",
                    },
                    When::Undeclared {
                        parameter: "handrail_both_sides_above_width",
                    },
                    none(),
                ],
                one,
                one_pending,
            ))),
            ItemCheck::Test(Box::new(fails(
                vec![
                    When::Equals {
                        parameter: "handrail_sides",
                        value: "both",
                    },
                    short(),
                ],
                both,
                both_pending,
            ))),
            ItemCheck::Test(Box::new(wider)),
        ],
    };
    let stretch_check = check(Items {
        checks: vec![ItemCheck::Group(Box::new(Group {
            applies: Applies::default(),
            when: Vec::new(),
            guards: vec![Guard {
                field: "measured",
                when: Vec::new(),
                undecided: None,
                null: OnNull::Skip,
            }],
            checks: vec![ItemCheck::Group(Box::new(sides))],
        }))],
        texts: vec![
            text(
                "found",
                vec![none()],
                "no selected handrail runs along a side of {label}",
            ),
            text(
                "found",
                Vec::new(),
                "a handrail runs along the {side} side of {label} only (seen climbing)",
            ),
        ],
        ..items(applies(&["handrail_objects"], &[]), stretches)
    });
    // Each rail's height.
    let height = |value: &'static str,
                  bounds: &'static [&'static str],
                  fail: &'static str,
                  straddles: &'static str,
                  upper: bool| {
        let bound = bounds[0];
        let mut judged = test(
            Judge::Range(Box::new(Range {
                at_least: if upper {
                    Vec::new()
                } else {
                    vec![parameter(bound)]
                },
                at_most: if upper {
                    vec![parameter(bound)]
                } else {
                    Vec::new()
                },
                allowance: slack(1.0, "scale"),
                ..range(value, ItemUnit::Length)
            })),
            fail,
            straddles,
        );
        judged.applies = applies(bounds, &[]);
        judged.related = Some("rails");
        ItemCheck::Test(Box::new(judged))
    };
    let bounds = |minimum: &'static str,
                  maximum: &'static str,
                  both: &'static str,
                  least: &'static str,
                  most: &'static str| {
        vec![
            text(
                "bounds",
                vec![
                    When::Declared { parameter: minimum },
                    When::Declared { parameter: maximum },
                ],
                both,
            ),
            text("bounds", vec![When::Declared { parameter: minimum }], least),
            text("bounds", vec![When::Declared { parameter: maximum }], most),
        ]
    };
    let heights_check = check(Items {
        checks: vec![
            height(
                "lowest",
                &["handrail_height_minimum"],
                "handrail {rail} runs {lowest:length} above the pitch line of {label} at its \
                 lowest; {bounds} required",
                "handrail {rail} runs {lowest:length} above the pitch line of {label} at its \
                 lowest, which straddles {bounds}",
                false,
            ),
            height(
                "highest",
                &["handrail_height_maximum"],
                "handrail {rail} runs {highest:length} above the pitch line of {label} at its \
                 highest; {bounds} required",
                "handrail {rail} runs {highest:length} above the pitch line of {label} at its \
                 highest, which straddles {bounds}",
                true,
            ),
        ],
        passing: Some(passing()),
        texts: bounds(
            "handrail_height_minimum",
            "handrail_height_maximum",
            "{handrail_height_minimum:length} to {handrail_height_maximum:length}",
            "at least {handrail_height_minimum:length}",
            "at most {handrail_height_maximum:length}",
        ),
        refused: None,
        ..items(
            applies(
                &["handrail_objects"],
                &["handrail_height_minimum", "handrail_height_maximum"],
            ),
            heights,
        )
    });
    // Each extension, and the rail's top running level over it.
    let over_middle = |value: bool| When::Field {
        field: "over_middle",
        value,
    };
    let downgrades = |on: On| {
        vec![
            pending(
                vec![over_middle(true)],
                on,
                "{failed}; it reaches over the middle of {label}, so it may be one piece of a \
                 longer rail",
            ),
            pending(
                vec![over_middle(false), rails],
                On::Fail,
                "{failed}; a rail the selection could not decide may continue it",
            ),
        ]
    };
    let mut level = test(
        Judge::Range(Box::new(Range {
            at_most: vec![Requirement {
                name: "level",
                options: vec![Choice {
                    when: Vec::new(),
                    bound: Bound::Literal(1e-6),
                }],
                words: "",
            }],
            grade: false,
            ..range("rise", ItemUnit::Length)
        })),
        "the top of handrail {rail} rises or falls {rise:length} over the \
         {handrail_extension_minimum:length} {place}; it must continue level",
        "the top of handrail {rail} rises or falls {rise:length} over the \
         {handrail_extension_minimum:length} {place}, which may or may not be level",
    );
    level.applies = applies(&["handrail_extension_minimum"], &[]);
    level.effects = downgrades(On::Fail);
    let mut extension = test(
        Judge::Range(Box::new(Range {
            at_least: vec![parameter("handrail_extension_minimum")],
            at_most: vec![parameter("handrail_extension_maximum")],
            allowance: slack(1.0, "scale"),
            null: OnNull::Fail(
                "handrail {rail} runs along {other} straight part of {label} only, so it does not \
                 reach {from}; {bound} required",
            ),
            ..range("reach", ItemUnit::Length)
        })),
        "handrail {rail} reaches {reach:length} {from}; {bound} required",
        "handrail {rail} reaches {reach:length} {from}, which straddles {bound}",
    );
    extension.effects = downgrades(On::FailBelow);
    extension.then = Some(Box::new(level));
    extension.related = Some("rails");
    let extensions_check = check(Items {
        checks: vec![ItemCheck::Test(Box::new(extension))],
        passing: Some(passing()),
        refused: None,
        ..items(
            applies(
                &["handrail_objects"],
                &["handrail_extension_minimum", "handrail_extension_maximum"],
            ),
            extensions,
        )
    });
    // Each gap between consecutive pieces.
    let mut gap = test(
        Judge::Range(Box::new(Range {
            at_most: vec![parameter("handrail_gap_maximum")],
            allowance: slack(2.0, "scale"),
            ..range("gap", ItemUnit::Length)
        })),
        "{pair} leave a gap of {gap:length} in plan; at most {handrail_gap_maximum:length} \
         allowed",
        "{pair} leave a gap of {gap:length} in plan, which straddles at most \
         {handrail_gap_maximum:length} allowed",
    );
    gap.effects = vec![pending(
        vec![rails],
        On::Fail,
        "{failed}; a rail the selection could not decide may bridge it",
    )];
    gap.related = Some("rails");
    let gaps_check = check(Items {
        checks: vec![ItemCheck::Test(Box::new(gap))],
        refused: None,
        ..items(
            applies(&["handrail_objects", "handrail_gap_maximum"], &[]),
            gaps,
        )
    });
    vec![stretch_check, heights_check, extensions_check, gaps_check]
}

/// The clear width of each item of `list` against the minimum, an
/// undecided obstacle able only to narrow it.
fn clear_widths(minimum: &'static [&'static str], list: &'static str) -> FormCheck {
    let mut judged = test(
        Judge::Range(Box::new(Range {
            at_least: vec![parameter(minimum[0])],
            allowance: slack(1.0, "width"),
            ..range("width", ItemUnit::Length)
        })),
        "{words}; at least {minimum_width} required",
        "{words}, which straddles at least {minimum_width} required",
    );
    judged.effects = vec![pending(
        vec![undecided("clear_width_obstacles")],
        On::Pass,
        "{words}; an obstacle the selection could not decide may narrow it",
    )];
    judged.related = Some("governing");
    let mut texts = clear_width_texts();
    texts.push(text(
        "minimum_width",
        Vec::new(),
        match minimum[0] {
            "clear_width_minimum" => "{clear_width_minimum:length}",
            _ => "{landing_clear_width_minimum:length}",
        },
    ));
    check(Items {
        checks: vec![ItemCheck::Test(Box::new(judged))],
        texts,
        ..items(applies(minimum, &[]), list)
    })
}

fn clear_width_texts() -> Vec<ItemText> {
    vec![
        text(
            "words",
            Vec::new(),
            "the clear width of {label} {clear_width_band_from:length} to \
             {clear_width_band_to:length} above {above} is {width:length} {between}",
        ),
        text(
            "between",
            vec![When::Empty { field: "governing" }],
            "between its own sides",
        ),
        text("between", Vec::new(), "beside {governing:and}"),
    ]
}

/// The declaration checks the walking surfaces of both capabilities share,
/// in the order the capabilities read them.
#[allow(clippy::too_many_lines)]
fn walking_declaration(ramp: bool) -> Vec<Check> {
    let mut checks = vec![
        // The free space at each end.
        Check::Positive {
            parameter: "end_space_depth",
            message: None,
        },
        Check::Positive {
            parameter: "end_space_width",
            message: None,
        },
        Check::Positive {
            parameter: "end_space_height",
            message: None,
        },
        Check::Kind {
            parameter: "end_space_obstacles",
        },
        Check::Together {
            parameters: &[
                "end_space_depth",
                "end_space_width",
                "end_space_height",
                "end_space_obstacles",
            ],
            message: "`end_space_depth`, `end_space_width`, `end_space_height` and \
                      `end_space_obstacles` are declared together",
        },
    ];
    if !ramp {
        checks.push(Check::Kind {
            parameter: "handrail_break_doors",
        });
    }
    checks.extend([
        // The doors on a landing.
        Check::Kind {
            parameter: "landing_doors",
        },
        Check::Positive {
            parameter: "landing_door_height",
            message: None,
        },
        Check::Kind {
            parameter: "landing_door_swing",
        },
        Check::Together {
            parameters: &["landing_doors", "landing_door_height"],
            message: "`landing_doors` and `landing_door_height` are declared together",
        },
        Check::Requires {
            parameter: "landing_door_swing",
            with: &["landing_doors"],
            message: "`landing_door_swing` needs `landing_doors` and `landing_door_height`",
        },
        // The clear width.
        Check::Length {
            parameter: "clear_width_minimum",
        },
    ]);
    let clear: &'static [&'static str] = if ramp {
        &["clear_width_minimum"]
    } else {
        checks.extend([
            Check::Length {
                parameter: "landing_clear_width_minimum",
            },
            Check::Length {
                parameter: "total_clear_width_minimum",
            },
        ]);
        &[
            "clear_width_minimum",
            "landing_clear_width_minimum",
            "total_clear_width_minimum",
        ]
    };
    let clear_message = "a clear-width minimum (`clear_width_minimum`, \
                         `landing_clear_width_minimum` or `total_clear_width_minimum`), \
                         `clear_width_obstacles`, `clear_width_band_from` and \
                         `clear_width_band_to` are declared together, the band's bottom below \
                         its top";
    checks.extend([
        Check::Kind {
            parameter: "clear_width_obstacles",
        },
        Check::Length {
            parameter: "clear_width_band_from",
        },
        Check::Length {
            parameter: "clear_width_band_to",
        },
        Check::Needs {
            all: &[
                "clear_width_obstacles",
                "clear_width_band_from",
                "clear_width_band_to",
            ],
            any: clear,
            missing: clear_message,
            unused: clear_message,
        },
        Check::Below {
            low: "clear_width_band_from",
            high: "clear_width_band_to",
            message: clear_message,
        },
        // The width.
        Check::Length {
            parameter: "width_minimum",
        },
        Check::Length {
            parameter: "width_maximum",
        },
        Check::Ordered {
            low: "width_minimum",
            high: "width_maximum",
            message: "`width_minimum` exceeds `width_maximum`",
        },
        // The landings.
        Check::Kind {
            parameter: "landing_objects",
        },
        Check::Length {
            parameter: "landing_depth_minimum",
        },
        Check::Length {
            parameter: "landing_width_minimum",
        },
        Check::Length {
            parameter: "end_landing_depth_minimum",
        },
        Check::Length {
            parameter: "end_landing_width_minimum",
        },
        Check::Kind {
            parameter: "landing_at_least_walking_width",
        },
        Check::Kind {
            parameter: "landings_required",
        },
        Check::Needs {
            all: &["landing_objects"],
            any: &[
                "landing_depth_minimum",
                "landing_width_minimum",
                "end_landing_depth_minimum",
                "end_landing_width_minimum",
                "landing_at_least_walking_width",
                "landings_required",
                "landing_doors",
                "handrail_break_doors",
                "landing_clear_width_minimum",
                "total_clear_width_minimum",
            ],
            missing: "a landing check needs `landing_objects`",
            unused: "`landing_objects` is declared without a landing check",
        },
        // The clearance below.
        Check::Length {
            parameter: "minimum_headroom_below",
        },
        Check::Kind {
            parameter: "headroom_below_spaces",
        },
        Check::Together {
            parameters: &["minimum_headroom_below", "headroom_below_spaces"],
            message: "`minimum_headroom_below` and `headroom_below_spaces` are declared together",
        },
    ]);
    checks.extend(handrail_declaration(ramp));
    checks
}

/// The handrail declaration checks.
fn handrail_declaration(ramp: bool) -> Vec<Check> {
    let mut checks = vec![
        Check::Kind {
            parameter: "handrail_objects",
        },
        Check::Length {
            parameter: "handrail_reach_across",
        },
        Check::Length {
            parameter: "handrail_reach_above",
        },
        Check::Length {
            parameter: "handrail_height_minimum",
        },
        Check::Length {
            parameter: "handrail_height_maximum",
        },
        Check::Ordered {
            low: "handrail_height_minimum",
            high: "handrail_height_maximum",
            message: "`handrail_height_minimum` exceeds `handrail_height_maximum`",
        },
        Check::Length {
            parameter: "handrail_extension_minimum",
        },
        Check::Length {
            parameter: "handrail_extension_maximum",
        },
        Check::Ordered {
            low: "handrail_extension_minimum",
            high: "handrail_extension_maximum",
            message: "`handrail_extension_minimum` exceeds `handrail_extension_maximum`",
        },
    ];
    checks.extend([
        Check::Length {
            parameter: "handrail_gap_maximum",
        },
        Check::Length {
            parameter: "handrail_both_sides_above_width",
        },
        Check::Among {
            parameter: "handrail_sides",
            options: &["one", "both"],
            message: "`handrail_sides` `{value}` is unsupported; use `one` or `both`",
        },
        Check::RequiresValue {
            parameter: "handrail_both_sides_above_width",
            with: "handrail_sides",
            value: "one",
            message: "`handrail_both_sides_above_width` applies only to `handrail_sides` `one`",
        },
    ]);
    if ramp {
        checks.extend([
            Check::Kind {
                parameter: "check_continuous_handrails",
            },
            Check::Length {
                parameter: "handrail_continuity_tolerance",
            },
            Check::RequiresDeclared {
                parameter: "handrail_continuity_tolerance",
                with: &["check_continuous_handrails"],
                message: "`handrail_continuity_tolerance` needs `check_continuous_handrails`",
            },
            Check::Kind {
                parameter: "check_rails_obstruction",
            },
        ]);
    } else {
        checks.push(Check::Kind {
            parameter: "handrail_continuous_across_landings",
        });
    }
    checks.push(Check::Needs {
        all: &[
            "handrail_objects",
            "handrail_reach_across",
            "handrail_reach_above",
        ],
        any: &[
            "handrail_height_minimum",
            "handrail_height_maximum",
            "handrail_extension_minimum",
            "handrail_extension_maximum",
            "handrail_gap_maximum",
            "handrail_sides",
            "handrail_continuous_across_landings",
            "check_continuous_handrails",
            "check_rails_obstruction",
        ],
        missing: "a handrail check needs `handrail_objects`, `handrail_reach_across` and \
                  `handrail_reach_above`",
        unused: "`handrail_objects` or a handrail reach is declared without a handrail check",
    });
    checks
}

/// The handrail lists' arguments, `of` the walking surface.
macro_rules! rails {
    ($list:literal, $of:literal) => {
        concat!(
            $list,
            ";of=",
            $of,
            ";rails=@handrail_objects;reach_across=@handrail_reach_across;\
             reach_above=@handrail_reach_above;level_over=@handrail_extension_minimum"
        )
    };
}

/// `ramp-geometry`, rebuilt as a composition with its outside contract
/// kept.
#[allow(clippy::too_many_lines)]
pub(crate) fn ramp(parameters: Vec<ParameterDescriptor>) -> Template {
    let mut declaration = vec![
        Check::Kind {
            parameter: "slope_limits",
        },
        Check::Rows {
            parameter: "slope_limits",
            columns: &[
                RowCheck::Number {
                    column: "maximum_slope",
                    missing: "a `slope_limits` row needs `maximum_slope`",
                    negative: "`maximum_slope` is negative",
                },
                RowCheck::Length {
                    column: "maximum_length",
                    message: "`maximum_length` is not a non-negative length",
                },
                RowCheck::Length {
                    column: "maximum_rise",
                    message: "`maximum_rise` is not a non-negative length",
                },
            ],
        },
        Check::Kind {
            parameter: "slope_tolerance",
        },
        Check::NonNegative {
            parameters: &["slope_tolerance"],
            message: "`slope_tolerance` is negative",
        },
        Check::Length {
            parameter: "minimum_headroom",
        },
        Check::Kind {
            parameter: "headroom_obstacles",
        },
        Check::Together {
            parameters: &["minimum_headroom", "headroom_obstacles"],
            message: "`minimum_headroom` and `headroom_obstacles` are declared together",
        },
    ];
    declaration.extend(walking_declaration(true));
    declaration.extend([
        Check::Kind {
            parameter: "accessible_surface_selector",
        },
        Check::Needs {
            all: &["accessible_surface_selector"],
            any: &["check_rails_obstruction"],
            missing: "`check_rails_obstruction` and `accessible_surface_selector` are declared \
                      together",
            unused: "`check_rails_obstruction` and `accessible_surface_selector` are declared \
                     together",
        },
        Check::Declares {
            parameters: &[
                "slope_limits",
                "slope_tolerance",
                "minimum_headroom",
                "width_minimum",
                "width_maximum",
                "landing_objects",
                "minimum_headroom_below",
                "handrail_objects",
                "end_space_depth",
                "clear_width_minimum",
            ],
            message: "declare at least one ramp check",
        },
    ]);
    let mut slope = test(
        Judge::Rows(Box::new(Rows {
            table: "slope_limits",
            columns: vec![
                RowColumn {
                    column: "maximum_slope",
                    value: "slope",
                    unit: ItemUnit::Ratio,
                    allowance: Allowance::Stated {
                        times: 1.0,
                        value: "slope_rounding",
                    },
                    words: "slope at most {}",
                },
                RowColumn {
                    column: "maximum_length",
                    value: "length",
                    unit: ItemUnit::Length,
                    allowance: slack(1.0, "scale"),
                    words: " over at most {}",
                },
                RowColumn {
                    column: "maximum_rise",
                    value: "rise",
                    unit: ItemUnit::Length,
                    allowance: slack(1.0, "scale"),
                    words: " rising at most {}",
                },
            ],
            joiner: "; or ",
        })),
        "{label} rises {rise:length} over {length:length}, a slope of {slope:ratio}; required \
         {rows}",
        "{label} rises {rise:length} over {length:length}, a slope of {slope:ratio}, which \
         straddles a slope limit ({rows})",
    );
    slope.applies = applies(&["slope_limits"], &[]);
    let mut checks = vec![
        check(Items {
            checks: vec![ItemCheck::Test(Box::new(slope))],
            refused: None,
            ..items(applies(&["slope_limits"], &[]), "runs")
        }),
        check(Items {
            together: Some(Together {
                present: None,
                name: "",
                judge: TogetherJudge::Spread(Box::new(Spread {
                    value: "slope",
                    unit: ItemUnit::Ratio,
                    tolerance: Operand::Parameter("slope_tolerance"),
                    allowance: Allowance::Stated {
                        times: 2.0,
                        value: "slope_rounding",
                    },
                    fail: "run slopes differ by {spread} ({values}); at most {tolerance} allowed",
                    undecided: "run slopes differ by {spread} ({values}), which straddles the \
                                tolerance {tolerance}",
                })),
            }),
            refused: None,
            ..items(applies(&["slope_tolerance"], &[]), "runs")
        }),
        clearance(
            (&["minimum_headroom"], "headroom_obstacles"),
            "clearances;of=ramp;side=above;obstacles=@headroom_obstacles",
            (
                "headroom above the walking surface is {clearance:length} under {governing}; at \
                 least {minimum_headroom:length} required",
                "headroom above the walking surface is {clearance:length} under {governing}, \
                 which straddles at least {minimum_headroom:length} required",
                "nothing selected stands above the walking surface; an obstacle the selection \
                 could not decide may lower it",
                "headroom above the walking surface is {clearance:length} under {governing}; an \
                 obstacle the selection could not decide may lower it",
            ),
        ),
        every_width(
            "runs",
            "run width {index} of {count}",
            "a run's width is not measured: it fills no rectangle along its slope",
        ),
        landings(
            "landings;of=ramp;landing=@landing_objects",
            &[
                "landing_depth_minimum",
                "landing_width_minimum",
                "end_landing_depth_minimum",
                "end_landing_width_minimum",
                "landing_at_least_walking_width",
            ],
            &[
                "landing_depth_minimum",
                "landing_width_minimum",
                "end_landing_depth_minimum",
                "end_landing_width_minimum",
                "landing_at_least_walking_width",
                "landings_required",
                "landing_doors",
            ],
        ),
        searched(
            applies(&["landing_objects", "landing_doors"], &[]),
            "landing_doors;of=ramp;landing=@landing_objects;doors=@landing_doors;\
             height=@landing_door_height",
        ),
        searched(
            applies(
                &["landing_objects", "landing_doors", "landing_door_swing"],
                &[],
            ),
            "landing_swings;of=ramp;landing=@landing_objects;doors=@landing_doors;\
             height=@landing_door_height",
        ),
        clearance(
            (&["minimum_headroom_below"], "headroom_below_spaces"),
            "clearances;of=ramp;side=below;obstacles=@headroom_below_spaces",
            (
                "headroom below the {noun} is {clearance:length} over the floor of {governing}; \
                 at least {minimum_headroom_below:length} required",
                "headroom below the {noun} is {clearance:length} over the floor of {governing}, \
                 which straddles at least {minimum_headroom_below:length} required",
                "the {noun} stands above no selected space's floor; a space the selection could \
                 not decide may lower it",
                "headroom below the {noun} is {clearance:length} over the floor of {governing}; \
                 a space the selection could not decide may lower it",
            ),
        ),
    ];
    checks.extend(handrail_checks(
        rails!("handrail_stretches", "ramp"),
        rails!("rail_heights", "ramp"),
        rails!("rail_extensions", "ramp"),
        rails!("rail_gaps", "ramp"),
    ));
    checks.extend([
        searched(
            applies(&["handrail_objects", "check_continuous_handrails"], &[]),
            concat!(
                rails!("rail_continuity", "ramp"),
                ";tolerance=@handrail_continuity_tolerance;gap=@handrail_gap_maximum"
            ),
        ),
        clear_widths(
            &["clear_width_minimum"],
            "clear_widths;of=ramp;obstacles=@clear_width_obstacles;\
             band_from=@clear_width_band_from;band_to=@clear_width_band_to",
        ),
        searched(
            applies(&["end_space_depth"], &[]),
            "end_spaces;of=ramp;obstacles=@end_space_obstacles;depth=@end_space_depth;\
             width=@end_space_width;height=@end_space_height",
        ),
        searched(
            applies(&["handrail_objects", "check_rails_obstruction"], &[]),
            concat!(
                rails!("rail_obstructions", "ramp"),
                ";surfaces=@accessible_surface_selector"
            ),
        ),
    ]);
    Template {
        id: RAMP,
        parameters,
        grades: true,
        name: "ramp-geometry",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration,
        services: Some(Services {
            needs: vec![Service::WalkingSurface],
            message: "walking-surface service is not registered",
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![measured("runs", "run_count")],
            // The ramp is measured: its checks judge it.
            decision: Decision::Within {
                value: "runs",
                minimum: None,
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks,
        }],
    }
}
