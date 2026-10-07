//! `parking-bay` as a template: the measured `parking_bay` of each bay, its
//! sizes within their bounds, its obstacles counted at most what is
//! allowed, and its searches' answers.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, Form, FormCheck, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, On, OnNull, Operand, Range, Refusals, Requirement, Template,
    TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.parking-bay";

const LIST: &str = "parking_bay;min_width=@min_width;max_width=@max_width;\
                    min_length=@min_length;max_length=@max_length;min_height=@min_height;\
                    max_height=@max_height;aisles=@aisles;aisle_reach=@aisle_reach;\
                    orientation=@orientation;angle_tolerance=@angle_tolerance;\
                    obstacles=@obstacles;obstruction_reach=@obstruction_reach;\
                    end_obstructions=@end_obstructions;side_obstructions=@side_obstructions;\
                    applies_when=@applies_when;orientations=@orientations;\
                    end_states=@end_states;side_states=@side_states;\
                    side_zone_length=@side_zone_length;neighbour_reach=@neighbour_reach;\
                    selection=@selection";

fn bound(name: &'static str, field: &'static str) -> Vec<Requirement> {
    vec![Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Value(field)),
        }],
        words: "",
    }]
}

fn test(
    kind: &'static str,
    judge: Judge,
    (fail, undecided): (&'static str, &'static str),
) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: vec![When::Field {
            field: kind,
            value: true,
        }],
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

fn range(
    value: &'static str,
    unit: ItemUnit,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit,
        at_least,
        at_most,
        allowance: Allowance::None,
        grade: false,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

fn checks() -> Vec<ItemCheck> {
    // A search's own answer: an obstacle-free orientation to an aisle.
    let judged = test(
        "judged",
        Judge::Truth {
            value: "found",
            finding: true,
        },
        ("{message}", "{why}"),
    );
    // Obstacles counted at most what is allowed.
    let counted = test(
        "counting",
        range(
            "count",
            ItemUnit::Count,
            (Vec::new(), bound("allowed", "allowed")),
        ),
        ("{found_words}", "{open_words}"),
    );
    // A size within its bounds; where the filters may leave it out, a
    // shortfall is open.
    let mut sized = test(
        "sized",
        range(
            "size",
            ItemUnit::Length,
            (bound("least", "low"), bound("most", "high")),
        ),
        (
            "{measured}; {bound:plain} m{suffix}",
            "{measured}, which straddles {bound:plain} m{later}",
        ),
    );
    sized.related = None;
    sized.effects = vec![Effect {
        when: vec![When::Field {
            field: "doubtful",
            value: true,
        }],
        on: On::Fail,
        message: "{measured}; {bound:plain} m, if the bound applies to it: {applies_why}",
    }];
    vec![judged, counted, sized]
        .into_iter()
        .map(|test| ItemCheck::Test(Box::new(test)))
        .collect()
}

fn bays() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: LIST,
            refused: Some("{why}"),
            checks: checks(),
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            once: false,
            combined: None,
            reason: None,
            at: None,
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

/// `parking-bay`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: super::NAME,
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        // The capability's declaration, in its order and words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: LIST,
        }],
        // The list reports missing services for each bay, as the capability
        // did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the bay is judged".into()),
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
            checks: vec![bays()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    super::parameters()
}
