//! `local-circulation` as a template: the circulation search's answers
//! about the selected spaces and their components
//! (`circulation_verdicts`, a list of each space), each on the object it
//! is about, one not met a finding worded as the search names it, one the
//! search cannot tell not evaluated.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, Refusals,
    Template, TemplateValue,
};
use axioval_ir::contract::{Expression, ScalarValue};

const CHECKS: &str = "circulation_verdicts;component_selector=@component_selector;\
                      space_path=@space_path;access_path=@access_path;\
                      door_selector=@door_selector;opening_selector=@opening_selector;\
                      space_selector=@space_selector;obstacles=@obstacles;\
                      subtract_door_swings=@subtract_door_swings;width_metres=@width_metres;\
                      clear_height_metres=@clear_height_metres;\
                      tolerance_metres=@tolerance_metres;component_mode=@component_mode;\
                      end_width_metres=@end_width_metres;end_length_metres=@end_length_metres;\
                      end_reach_metres=@end_reach_metres;short_end_metres=@short_end_metres;\
                      narrow_end_metres=@narrow_end_metres;merge_path=@merge_path;\
                      band_from_metres=@band_from_metres;\
                      end_exempt_selector=@end_exempt_selector;\
                      end_exempt_reach_metres=@end_exempt_reach_metres;\
                      partner_selector=@partner_selector;require_entrances=@require_entrances;\
                      check_entrance_width=@check_entrance_width;\
                      clear_width_property=@clear_width_property;\
                      clear_width_from_leaves=@clear_width_from_leaves;\
                      overall_width=@overall_width;width_deduction=@width_deduction;\
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

/// `local-circulation`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: "axioval:capability.local-circulation",
        parameters: super::parameters(),
        grades: false,
        name: "local-circulation",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: CHECKS,
        }],
        // The search reports a missing free-space service for each space,
        // as the capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the circulation is searched".into()),
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
