//! `horizontal-guard` as a template: the guard search measured once for
//! the rule's whole selection (`guard_surfaces`, a refusal the rule's),
//! then each selected surface's exposed edges (`guard_edges`) judged one by
//! one, the first defect each edge has, by priority, a finding, and the
//! findings of one defect on a surface one finding relating every element
//! explaining it.
//!
//! An edge whose barriers, tall enough, cover it is guarded unless an
//! object low and broad enough beside a barrier lets one climb it. One
//! reached by barriers along more than half of it has a barrier too low
//! (from its curb, where only measuring from the floor makes it tall
//! enough) or with a hole; otherwise landings wide enough and a short fall
//! below covering it guard it, and the nearest landing names why they do
//! not: too far away, too low, too small or too few. Nothing near is a
//! missing barrier.

use axioval_engine::template::{
    Allowance, Applies, Band, Bound, Check, Choice, Condition, Decision, Form, FormCheck, Grading,
    ItemCheck, ItemTest, ItemUnit, Items, Judge, OnNull, Once, Operand, Range, Refusals,
    Requirement, Service, Services, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::Severity;
use axioval_ir::contract::{Expression, ScalarValue};

use super::EPSILON_M;
use crate::guard_diagnosis::GuardDefect;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.horizontal-guard";

/// Restricts which objects may count as barriers.
const BARRIER_SELECTOR: &str = "barrier_selector";
/// Restricts which objects may count as landings.
const LANDING_SELECTOR: &str = "landing_selector";
/// Restricts which objects may count as climbing aids.
const CLIMBABLE_SELECTOR: &str = "climbable_selector";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("minimum_barrier_height_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_barrier_gap_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_platform_gap_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_landing_gap_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_fall_height_metres", ParameterType::Number),
        ParameterDescriptor::required("minimum_landing_width_metres", ParameterType::Number),
        ParameterDescriptor::required("climbable_barrier_distance_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_climbable_height_metres", ParameterType::Number),
        ParameterDescriptor::required(
            "minimum_climbable_side_length_metres",
            ParameterType::Number,
        ),
        ParameterDescriptor::required("measure_barrier_from_curb", ParameterType::Boolean),
        ParameterDescriptor::optional(BARRIER_SELECTOR, ParameterType::Selector),
        ParameterDescriptor::optional(LANDING_SELECTOR, ParameterType::Selector),
        ParameterDescriptor::optional(CLIMBABLE_SELECTOR, ParameterType::Selector),
    ]
}

/// Every declaration the capability could not realise, in one message.
const UNREALISABLE: &str = "horizontal-guard declaration is missing or not realisable";

/// The search every value names, each argument the rule's parameter:
/// the rule's own selection measured in one request.
macro_rules! search {
    ($name:literal) => {
        concat!(
            $name,
            ";surfaces=@selection;barrier_gap=@maximum_barrier_gap_metres;\
             platform_gap=@maximum_platform_gap_metres;\
             landing_gap=@maximum_landing_gap_metres;\
             landing_width=@minimum_landing_width_metres;\
             climb_distance=@climbable_barrier_distance_metres;\
             climb_side=@minimum_climbable_side_length_metres;\
             from_curb=@measure_barrier_from_curb;\
             climb_height=@maximum_climbable_height_metres;\
             barriers=@barrier_selector;landings=@landing_selector;\
             climbables=@climbable_selector"
        )
    };
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Finite {
            parameters: &[
                "minimum_barrier_height_metres",
                "maximum_barrier_gap_metres",
                "maximum_platform_gap_metres",
                "maximum_landing_gap_metres",
                "maximum_fall_height_metres",
                "minimum_landing_width_metres",
                "climbable_barrier_distance_metres",
                "maximum_climbable_height_metres",
                "minimum_climbable_side_length_metres",
            ],
            above: None,
            at_least: Some(0.0),
            message: UNREALISABLE,
        },
        Check::Holds {
            condition: Condition::Stated {
                parameter: "measure_barrier_from_curb",
            },
            message: UNREALISABLE,
        },
        Check::Kind {
            parameter: BARRIER_SELECTOR,
        },
        Check::Kind {
            parameter: LANDING_SELECTOR,
        },
        Check::Kind {
            parameter: CLIMBABLE_SELECTOR,
        },
    ]
}

/// A requirement bounding a field by the rule's parameter, or a number.
fn bounded(bound: Bound) -> Vec<Requirement> {
    vec![Requirement {
        name: "bound",
        options: vec![Choice {
            when: Vec::new(),
            bound,
        }],
        words: "",
    }]
}

/// A test of an edge's length field: at least or at most its bound.
fn range(
    value: &'static str,
    (at_least, at_most): (Option<Bound>, Option<Bound>),
    allowance: Allowance,
    null: OnNull,
) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit: ItemUnit::Length,
        at_least: at_least.map(bounded).unwrap_or_default(),
        at_most: at_most.map(bounded).unwrap_or_default(),
        allowance,
        grade: false,
        null,
        unmeasured: None,
    }))
}

/// A test, its finding the defect, relating `related`.
fn test(judge: Judge, defect: GuardDefect, related: Option<&'static str>) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge,
        fail: defect.code(),
        undecided: "{why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related,
    }
}

/// The defect itself, wherever the test is reached.
fn found(defect: GuardDefect, related: Option<&'static str>) -> Box<ItemTest> {
    Box::new(test(Judge::Fails, defect, related))
}

/// The rule's parameter as a bound, where a test takes one.
#[allow(clippy::unnecessary_wraps)]
fn parameter(name: &'static str) -> Option<Bound> {
    Some(Bound::Operand(Operand::Parameter(name)))
}

/// A length raised by a micrometre against its bound, as the capability
/// compared measured lengths.
const RAISED: Allowance = Allowance::Raised { value: EPSILON_M };

/// The barrier height a field must reach.
fn tall_enough(value: &'static str, defect: GuardDefect, null: OnNull) -> ItemTest {
    test(
        range(
            value,
            (parameter("minimum_barrier_height_metres"), None),
            RAISED,
            null,
        ),
        defect,
        Some("tallest"),
    )
}

/// Landings below: covering the edge within the fall allowed, or the
/// nearest naming why they do not; nothing near is a missing barrier.
fn landings() -> Box<ItemTest> {
    let nearest = |value, bound, allowance, defect| {
        test(
            range(value, bound, allowance, OnNull::Skip),
            defect,
            Some("nearest"),
        )
    };
    let mut small = nearest(
        "nearest_width",
        (parameter("minimum_landing_width_metres"), None),
        RAISED,
        GuardDefect::LandingsTooSmall,
    );
    small.then = Some(found(GuardDefect::InsufficientLandings, Some("nearest")));
    let mut low = nearest(
        "nearest_fall",
        (None, parameter("maximum_fall_height_metres")),
        Allowance::Fixed { value: EPSILON_M },
        GuardDefect::LandingTooLow,
    );
    low.then = Some(Box::new(small));
    let mut far = nearest(
        "nearest_gap",
        (None, parameter("maximum_landing_gap_metres")),
        Allowance::None,
        GuardDefect::LandingTooFarAway,
    );
    far.then = Some(Box::new(low));
    // Only where a landing was measured at all.
    far.when = vec![When::Stated {
        field: "nearest_gap",
    }];
    let mut covered = test(
        range(
            "landing_fall",
            (None, parameter("maximum_fall_height_metres")),
            RAISED,
            OnNull::Unmet,
        ),
        GuardDefect::MissingBarrier,
        None,
    );
    covered.otherwise = Some(Box::new(far));
    Box::new(covered)
}

/// The first defect an edge has, by priority.
fn edge() -> ItemTest {
    // Covered by barriers tall enough: only an object to climb defeats them.
    let mut climbed = test(
        range(
            "climbable_height",
            (None, parameter("maximum_climbable_height_metres")),
            Allowance::Fixed { value: EPSILON_M },
            OnNull::Skip,
        ),
        GuardDefect::BarrierTooLowDueToClimbableObject,
        Some("climbable"),
    );
    climbed.then = Some(found(
        GuardDefect::BarrierTooLowDueToClimbableObject,
        Some("climbable"),
    ));
    // Too tall to climb: guarded.
    climbed.otherwise = Some(Box::new(test(
        range(
            "climbable_height",
            (None, None),
            Allowance::None,
            OnNull::Skip,
        ),
        GuardDefect::BarrierTooLowDueToClimbableObject,
        None,
    )));
    // Reached along more than half the edge, a barrier is present: too low
    // (tall enough from the floor, from its curb), or holed.
    let mut curb = tall_enough("tallest_top", GuardDefect::BarrierTooLow, OnNull::Skip);
    curb.when = vec![When::Stated {
        field: "tallest_top",
    }];
    curb.then = Some(found(GuardDefect::BarrierTooLowDueToCurb, Some("tallest")));
    let mut holed = tall_enough("partial_height", GuardDefect::HoleInBarrier, OnNull::Unmet);
    holed.then = Some(found(GuardDefect::HoleInBarrier, Some("tallest")));
    holed.otherwise = Some(landings());
    let mut present = tall_enough("tallest_barrier", GuardDefect::BarrierTooLow, OnNull::Skip);
    present.otherwise = Some(Box::new(curb));
    present.then = Some(Box::new(holed));
    let mut reached = test(
        range(
            "barrier_share",
            (None, Some(Bound::Literal(0.5))),
            Allowance::None,
            OnNull::Skip,
        ),
        GuardDefect::BarrierTooLow,
        None,
    );
    reached.then = Some(landings());
    reached.otherwise = Some(Box::new(present));
    let mut guarded = test(
        range(
            "guarded_height",
            (parameter("minimum_barrier_height_metres"), None),
            RAISED,
            OnNull::Unmet,
        ),
        GuardDefect::MissingBarrier,
        None,
    );
    guarded.then = Some(Box::new(climbed));
    guarded.otherwise = Some(Box::new(reached));
    guarded
}

/// Each surface's edges, every distinct defect one finding, at error.
fn edges() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: search!("guard_edges"),
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(edge()))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: true,
            at: None,
            once: false,
            combined: None,
            reason: None,
            joined: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: Some(Grading {
            values: Vec::new(),
            derived: Vec::new(),
            bands: vec![Band {
                severity: Severity::Error,
                when: None,
            }],
            undecided: Vec::new(),
        }),
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `horizontal-guard`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    let zero = TemplateValue {
        name: "zero",
        expression: Expression::Literal {
            value: ScalarValue::Number { value: 0.0 },
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    };
    let surfaces = TemplateValue {
        name: "surfaces",
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: search!("guard_surfaces").to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    };
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "horizontal-guard",
        refusals: Refusals::Selected,
        defaults: Vec::new(),
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::Guard],
            message: "guard service is not registered",
            only: None,
            whole: false,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![zero],
            decision: Decision::Within {
                value: "zero",
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
            checks: vec![edges()],
            unless: Vec::new(),
            grading: None,
            joined: None,
            once: vec![Once {
                value: surfaces,
                applies: None,
                refused: "{why}",
                required: true,
            }],
            project: Vec::new(),
        }],
    }
}
