//! `distance` as a template: the measured `distance_items` of each
//! subject, its nearest violating counterpart's distance at least the
//! minimum, its nearest counterpart's distance at most the maximum (or
//! the counterparts within the range at least `count`), the findings
//! joined into one; and the measured `distance_open` of the project, each
//! object the selections and the broad phase leave open.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Service, Services,
    Template, TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.distance";

const ITEMS: &str = "distance_items;counterparts=@counterparts;\
                     minimum_metres=@minimum_metres;maximum_metres=@maximum_metres;\
                     mode=@mode;count=@count;projection=@projection;\
                     footprint_offset_metres=@footprint_offset_metres;\
                     vertical_direction=@vertical_direction;subject_extent=@subject_extent;\
                     counterpart_extent=@counterpart_extent;subject_surface=@subject_surface;\
                     counterpart_surface=@counterpart_surface;\
                     elevation_overlap=@elevation_overlap;\
                     elevation_offset_metres=@elevation_offset_metres;\
                     container_selector=@container_selector;relationship=@relationship;\
                     direction=@direction;follow_chain=@follow_chain;path=@path;\
                     skip_absent_relationship_ends=@skip_absent_relationship_ends;\
                     selection=@selection";

const OPEN: &str = "distance_open;counterparts=@counterparts;\
                    minimum_metres=@minimum_metres;maximum_metres=@maximum_metres;\
                    mode=@mode;count=@count;projection=@projection;\
                    footprint_offset_metres=@footprint_offset_metres;\
                    vertical_direction=@vertical_direction;subject_extent=@subject_extent;\
                    counterpart_extent=@counterpart_extent;subject_surface=@subject_surface;\
                    counterpart_surface=@counterpart_surface;\
                    elevation_overlap=@elevation_overlap;\
                    elevation_offset_metres=@elevation_offset_metres;\
                    container_selector=@container_selector;relationship=@relationship;\
                    direction=@direction;follow_chain=@follow_chain;path=@path;\
                    skip_absent_relationship_ends=@skip_absent_relationship_ends;\
                    selection=@selection";

fn bound(name: &'static str, parameter: &'static str) -> Vec<Requirement> {
    vec![Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Parameter(parameter)),
        }],
        words: "",
    }]
}

fn test(when: Vec<When>, judge: Judge, (fail, related): (&'static str, &'static str)) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when,
        judge,
        fail,
        undecided: "{why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some(related),
    }
}

fn checked(field: &'static str) -> When {
    When::Field { field, value: true }
}

fn range(
    value: &'static str,
    unit: ItemUnit,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
    grade: bool,
) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit,
        at_least,
        at_most,
        allowance: Allowance::None,
        grade,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

fn checks() -> Vec<ItemCheck> {
    // No counterpart closer than the minimum: the nearest violating one.
    let apart = test(
        vec![checked("apart_checked")],
        range(
            "apart",
            ItemUnit::Length,
            (bound("minimum", "minimum_metres"), Vec::new()),
            true,
        ),
        ("{apart_words}", "apart_related"),
    );
    // The nearest counterpart within the maximum.
    let reach = test(
        vec![checked("reach_checked")],
        range(
            "reach",
            ItemUnit::Length,
            (Vec::new(), bound("maximum", "maximum_metres")),
            true,
        ),
        ("{reach_words}", "reach_related"),
    );
    // No counterpart within the maximum at all.
    let none = test(
        vec![checked("reach_checked")],
        Judge::Truth {
            value: "none_within",
            finding: true,
        },
        ("{reach_words}", "reach_related"),
    );
    // At least `count` counterparts within the range.
    let count = test(
        vec![checked("count_checked")],
        range(
            "count",
            ItemUnit::Count,
            (bound("count", "count"), Vec::new()),
            false,
        ),
        ("{count_words}", "count_related"),
    );
    vec![apart, reach, none, count]
        .into_iter()
        .map(|test| ItemCheck::Test(Box::new(test)))
        .collect()
}

fn items(list: &'static str, checks: Vec<ItemCheck>, at: Option<&'static str>) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list,
            refused: at.is_none().then_some("{why}"),
            checks,
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            once: false,
            combined: None,
            reason: Some("reason"),
            at,
            joined: at.is_none().then_some("; "),
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

/// Each object the selections and the broad phase leave open, open for
/// its reason.
fn open() -> FormCheck {
    let open = ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Truth {
            value: "open",
            finding: true,
        },
        fail: "",
        undecided: "{why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: None,
    };
    items(OPEN, vec![ItemCheck::Test(Box::new(open))], Some("object"))
}

/// `distance`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: true,
        name: "distance",
        refusals: Refusals::Objects,
        defaults: Vec::new(),
        // The capability's declaration, in its order and words, refused
        // for each subject.
        declaration: vec![Check::Arguments {
            when: &[],
            value: ITEMS,
        }],
        // A service only `elevation_overlap` `overlapping` reads, asked
        // for once for the rule; the list reports the others for each
        // subject, as the capability did.
        services: Some(Services {
            needs: vec![Service::VerticalExtent],
            message: "`elevation_overlap` needs the vertical-extent service, which is not \
                      registered",
            only: Some(("elevation_overlap", "overlapping")),
            whole: true,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the subject is judged".into()),
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
            checks: vec![items(ITEMS, checks(), None)],
            once: Vec::new(),
            joined: None,
            project: vec![open()],
        }],
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    super::parameters()
}
