//! `door-swing` as a template: the spaces a door opens onto, as the
//! measured list `swing_spaces` probes them, judged one by one against
//! `swing_not_into` and together against `swing_into`.

use axioval_engine::template::{
    Any, Applies, Check, Decision, Effect, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, On,
    OpenCase, Refusals, Service, Services, Template, TemplateValue, Together, TogetherJudge,
    Unless, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use serde_json::json;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.door-swing";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("space_path", ParameterType::StringList),
        ParameterDescriptor::optional("swing_into", ParameterType::Selector),
        ParameterDescriptor::optional("swing_not_into", ParameterType::Selector),
    ]
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Required {
            parameter: "space_path",
        },
        Check::Kind {
            parameter: "swing_into",
        },
        Check::Kind {
            parameter: "swing_not_into",
        },
        Check::AnyOf {
            parameters: &["swing_into", "swing_not_into"],
            message: "declare `swing_into`, `swing_not_into` or both",
        },
        Check::Path {
            parameter: "space_path",
        },
    ]
}

/// The spaces either selector may pick among those the door opens onto,
/// each probed once, whether each picks it read as `towards` and
/// `not_towards`.
const LIST: &str = "swing_spaces;path=@space_path;towards=@swing_into;not_towards=@swing_not_into";

/// The spaces of [`LIST`]: a door without hinged leaves, or one whose
/// spaces cannot be reached, leaves the check open once.
fn spaces(declared: &'static str) -> Items {
    Items {
        applies: Applies {
            when: if declared == "swing_into" {
                &["swing_into"]
            } else {
                &["swing_not_into"]
            },
            any: &[],
            condition: None,
        },
        list: LIST,
        refused: Some("door-swing: {why}"),
        checks: Vec::new(),
        together: None,
        passing: None,
        texts: Vec::new(),
        once: true,
        at: None,
        merged: false,
    }
}

fn check(items: Items) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        decision: Decision::Items(Box::new(items)),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        derived: Vec::new(),
        quiet: false,
        unless: None,
        ungraded: false,
    }
}

/// No space `swing_not_into` picks is swung into: each a finding, a space
/// it may pick open, as is one the probes cannot place.
fn not_into() -> FormCheck {
    let test = |picked: When, effects: Vec<Effect>| ItemTest {
        applies: Applies {
            when: &[],
            any: &[],
            condition: None,
        },
        when: vec![picked],
        judge: Judge::Truth {
            value: "into",
            finding: true,
        },
        fail: "swings into {space}, which `swing_not_into` forbids",
        undecided: "door-swing: {why}",
        effects,
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("space"),
    };
    let sure = test(
        When::Field {
            field: "not_towards",
            value: true,
        },
        Vec::new(),
    );
    let possible = test(
        When::Unknown {
            field: "not_towards",
        },
        vec![Effect {
            when: Vec::new(),
            on: On::Fail,
            message: "door-swing: it swings into {space}, which `swing_not_into` may pick",
        }],
    );
    check(Items {
        checks: vec![
            ItemCheck::Test(Box::new(sure)),
            ItemCheck::Test(Box::new(possible)),
        ],
        ..spaces("swing_not_into")
    })
}

/// Some space `swing_into` picks is swung into: where none surely is, the
/// first space left open opens the check, and otherwise the picked spaces
/// it surely swings away from are one finding.
fn into() -> FormCheck {
    let undecided = "door-swing: whether it swings into {space} is undecided";
    let sure = When::Field {
        field: "towards",
        value: true,
    };
    let possible = When::Unknown { field: "towards" };
    check(Items {
        together: Some(Together {
            present: None,
            when: Vec::new(),
            name: "{space}",
            judge: TogetherJudge::Any(Box::new(Any {
                holds: vec![
                    When::Field {
                        field: "into",
                        value: true,
                    },
                    sure,
                ],
                fails: vec![
                    When::Field {
                        field: "away",
                        value: true,
                    },
                    sure,
                ],
                open: [sure, possible]
                    .into_iter()
                    .flat_map(|picked| {
                        [
                            // The probes could not place it.
                            OpenCase {
                                when: vec![When::Unknown { field: "into" }, picked],
                                why: Some("into"),
                                message: "door-swing: {why}",
                            },
                            // Neither probe lies in it.
                            OpenCase {
                                when: vec![When::Unknown { field: "away" }, picked],
                                why: None,
                                message: undecided,
                            },
                        ]
                    })
                    // Swung into, but the selection may not pick it.
                    .chain([OpenCase {
                        when: vec![
                            When::Field {
                                field: "into",
                                value: true,
                            },
                            possible,
                        ],
                        why: None,
                        message: undecided,
                    }])
                    .collect(),
                item: "{space}",
                fail: "swings away from {failing}, which `swing_into` requires it to swing into",
                related: Some("space"),
            })),
        }),
        ..spaces("swing_into")
    })
}

/// The guard read first: a door without hinged leaves (or whose leaves
/// cannot be read) is refused as the capability refused it, before its
/// spaces are listed.
fn leaves() -> Unless {
    Unless {
        applies: Applies {
            when: &[],
            any: &[],
            condition: None,
        },
        value: TemplateValue {
            name: "leaves",
            expression: serde_json::from_value(json!({
                "kind": "property",
                "propertySet": axioval_ir::MEASURED_SET,
                "property": "hinged_leaves",
            }))
            .expect("a measured read"),
            expect: None,
            absent: None,
            mismatch: None,
            refused: Some("door-swing: {why}"),
        },
        guard: true,
    }
}

/// `door-swing`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "door-swing",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::ObjectFrame, Service::FreeSpace],
            message: "door-swing needs the object-frame and free-space services",
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The door is judged by its checks.
            values: vec![TemplateValue {
                name: "door",
                expression: serde_json::from_value(
                    json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
                )
                .expect("a literal"),
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Holds { value: "door" },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![not_into(), into()],
            unless: vec![leaves()],
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
