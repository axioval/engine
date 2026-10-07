//! `accessible-route` as a template: the walk's answer for each
//! destination (`route_verdicts`), one cut off from every start or lacking
//! its passing spaces a finding worded as the walk names what blocks it,
//! one the walk cannot tell not evaluated.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, Refusals,
    Template, TemplateValue,
};
use axioval_ir::contract::{Expression, ScalarValue};

const CHECKS: &str = "route_verdicts;route_selector=@route_selector;\
                      start_selector=@start_selector;portal_selector=@portal_selector;\
                      lift_selector=@lift_selector;ramp_selector=@ramp_selector;\
                      stair_selector=@stair_selector;obstacle_selector=@obstacle_selector;\
                      subtract_door_swings=@subtract_door_swings;width_metres=@width_metres;\
                      clear_height_metres=@clear_height_metres;\
                      door_width_metres=@door_width_metres;ramp_width_metres=@ramp_width_metres;\
                      stair_width_metres=@stair_width_metres;forbid_stairs=@forbid_stairs;\
                      clear_width_property=@clear_width_property;\
                      obstruction_depth_metres=@obstruction_depth_metres;\
                      surface_gap_metres=@surface_gap_metres;\
                      passing_width_metres=@passing_width_metres;\
                      passing_length_metres=@passing_length_metres;\
                      passing_spacing_metres=@passing_spacing_metres;\
                      passing_reach_metres=@passing_reach_metres;selection=@selection";

fn checks() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: CHECKS,
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(ItemTest {
                applies: Applies::default(),
                when: Vec::new(),
                judge: Judge::Truth {
                    value: "reached",
                    finding: false,
                },
                fail: "{words}",
                undecided: "{why}",
                effects: Vec::new(),
                then: None,
                otherwise: None,
                straddled: None,
                related: Some("related"),
            }))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
            once: false,
            combined: None,
            reason: Some("reason"),
            joined: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `accessible-route`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: "axioval:capability.accessible-route",
        parameters: super::parameters(),
        grades: false,
        name: "accessible-route",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: CHECKS,
        }],
        // The walk reports a missing walkability service for each
        // destination, as the capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the routes are walked".into()),
                },
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Within {
                value: "judged",
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
            checks: vec![checks()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
