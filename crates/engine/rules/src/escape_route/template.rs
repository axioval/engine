//! `escape-route` as a template: the escape search's answers about the
//! selected spaces, the passages and doors their occupants rely on, and
//! their sources (`escape_verdicts`, a list of each space), each on what
//! it is about, one not met a finding worded as the search names it, one
//! the search cannot tell not evaluated.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, Refusals,
    Template, TemplateValue,
};
use axioval_ir::contract::{Expression, ScalarValue};

const CHECKS: &str = "escape_verdicts;uses=@uses;widths=@widths;exit_path=@exit_path;\
                      exit_selector=@exit_selector;door_path=@door_path;\
                      door_selector=@door_selector;\
                      clear_width_property=@clear_width_property;\
                      walking_height=@walking_height;walking_step=@walking_step;\
                      sections=@sections;section_path=@section_path;\
                      passage_path=@passage_path;passage_selector=@passage_selector;\
                      passage_width_property=@passage_width_property;\
                      exit_door_direction=@exit_door_direction;\
                      walked_passages=@walked_passages;\
                      no_escape_selector=@no_escape_selector;\
                      compartment_selector=@compartment_selector;\
                      compartment_path=@compartment_path;\
                      compartment_overlap=@compartment_overlap;zones=@zones;\
                      exit_count=@exit_count;route_door_selector=@route_door_selector;\
                      common_path_factor=@common_path_factor;\
                      route_door_direction=@route_door_direction;\
                      minimum_clear_height=@minimum_clear_height;\
                      clear_height_property=@clear_height_property;\
                      overall_height=@overall_height;lining_thickness=@lining_thickness;\
                      threshold_thickness=@threshold_thickness;\
                      stair_selector=@stair_selector;ramp_selector=@ramp_selector;\
                      lift_selector=@lift_selector;stair_length=@stair_length;\
                      vertical_factor=@vertical_factor;selection=@selection";

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
                    value: "met",
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
            at: Some("at"),
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

/// `escape-route`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: "axioval:capability.escape-route",
        parameters: super::parameters(),
        grades: false,
        name: "escape-route",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: CHECKS,
        }],
        // The search reports each missing service for the spaces needing
        // it, as the capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the escape routes are searched".into()),
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
