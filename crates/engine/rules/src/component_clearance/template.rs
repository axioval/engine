//! `component-clearance` as a template: each side's questions as the
//! search answers them (`clearance_checks`), a volume obstructed, a larger
//! one free, one outside its spaces or one unsupported a finding worded
//! after its side, an answer the positions or selections leave open not
//! evaluated.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, Refusals,
    Service, Services, Template, TemplateValue,
};
use axioval_ir::contract::{Expression, ScalarValue};

const CHECKS: &str = "clearance_checks;side=@side;sides=@sides;quantifier=@quantifier;\
                      front_axis=@front_axis;both_sides=@both_sides;width=@width;\
                      width_mode=@width_mode;width_minimum=@width_minimum;\
                      width_maximum=@width_maximum;depth=@depth;depth_mode=@depth_mode;\
                      depth_minimum=@depth_minimum;depth_maximum=@depth_maximum;\
                      depth_from=@depth_from;radius=@radius;height=@height;\
                      height_mode=@height_mode;height_minimum=@height_minimum;\
                      height_maximum=@height_maximum;size_mode=@size_mode;\
                      size_tolerance=@size_tolerance;offset=@offset;\
                      lateral_offset=@lateral_offset;align=@align;slide_from=@slide_from;\
                      slide_to=@slide_to;depth_slide_from=@depth_slide_from;\
                      depth_slide_to=@depth_slide_to;height_reference=@height_reference;\
                      vertical_offset=@vertical_offset;top_datum=@top_datum;\
                      top_offset=@top_offset;obstacles=@obstacles;\
                      allowed_intruders=@allowed_intruders;protrusion=@protrusion;\
                      within_space=@within_space;space_path=@space_path;\
                      wall_selector=@wall_selector;wall_reach=@wall_reach;\
                      wall_inset=@wall_inset;support_selector=@support_selector;\
                      support_tolerance=@support_tolerance;\
                      clear_width_property=@clear_width_property;\
                      clear_width_from_leaves=@clear_width_from_leaves;\
                      overall_width=@overall_width;width_deduction=@width_deduction";

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
                fail: "{label} {words}",
                undecided: "{label}: {why}",
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

/// `component-clearance`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: super::ID,
        parameters: super::parameters(),
        grades: false,
        name: "component-clearance",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: CHECKS,
        }],
        services: Some(Services {
            needs: vec![
                Service::ObjectFrame,
                Service::VerticalExtent,
                Service::FreeSpace,
            ],
            message: "component-clearance needs the object-frame, vertical-extent and \
                      free-space services",
            only: None,
            whole: true,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the clearances are searched".into()),
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
