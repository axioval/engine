//! `containment` as a template: the measured `containment_items` of each
//! inner element, whether it lies in none and each cover distance within
//! its band; and the measured `containment_counts` of the project, each
//! outer element's count within the bounds and each object the selections
//! and the broad phase leave open.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Template, TemplateValue,
    When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.containment";

const ITEMS: &str = "containment_items;counterparts=@counterparts;\
                     minimum_volume_ratio=@minimum_volume_ratio;\
                     combine_adjacent=@combine_adjacent;cover=@cover;\
                     minimum_count=@minimum_count;maximum_count=@maximum_count;\
                     report_orphans=@report_orphans;selection=@selection";

const COUNTS: &str = "containment_counts;counterparts=@counterparts;\
                      minimum_volume_ratio=@minimum_volume_ratio;\
                      combine_adjacent=@combine_adjacent;cover=@cover;\
                      minimum_count=@minimum_count;maximum_count=@maximum_count;\
                      report_orphans=@report_orphans;selection=@selection";

const FEWER: &str = "holds {held:count} inner elements, fewer than the minimum \
                     {minimum_count:count}";
const MORE: &str = "holds {held:count} inner elements, more than the maximum \
                    {maximum_count:count}";
const BETWEEN: &str = "holds between {held:count} and {may:count} inner elements, so its count \
                       cannot be judged";

fn bound(name: &'static str, operand: Operand) -> Vec<Requirement> {
    vec![Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(operand),
        }],
        words: "",
    }]
}

fn test(
    flag: &'static str,
    judge: Judge,
    (fail, undecided, related): (&'static str, &'static str, Option<&'static str>),
) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when: vec![When::Field {
            field: flag,
            value: true,
        }],
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related,
    }))
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

fn truth(value: &'static str) -> Judge {
    Judge::Truth {
        value,
        finding: true,
    }
}

/// Why something is open, for its reason.
fn open() -> ItemCheck {
    test("open_checked", truth("open"), ("", "{why}", None))
}

fn inner() -> Vec<ItemCheck> {
    vec![
        // It lies in no outer element.
        test(
            "orphan_checked",
            truth("orphan"),
            ("{orphan_words}", "{why}", None),
        ),
        // A cover distance within its band: below the minimum, then above
        // the maximum, a straddle open in the band's words.
        test(
            "band_checked",
            range(
                "distance",
                ItemUnit::Length,
                (bound("minimum", Operand::Value("low")), Vec::new()),
            ),
            ("{below_words}", "{straddle_words}", Some("related")),
        ),
        test(
            "band_checked",
            range(
                "distance",
                ItemUnit::Length,
                (Vec::new(), bound("maximum", Operand::Value("high"))),
            ),
            ("{above_words}", "{straddle_words}", Some("related")),
        ),
        open(),
    ]
}

fn outer() -> Vec<ItemCheck> {
    vec![
        // The inner elements an outer element holds, from sure to possible.
        test(
            "count_checked",
            range(
                "count",
                ItemUnit::Count,
                (
                    bound("minimum", Operand::Parameter("minimum_count")),
                    Vec::new(),
                ),
            ),
            (FEWER, BETWEEN, Some("related")),
        ),
        test(
            "count_checked",
            range(
                "count",
                ItemUnit::Count,
                (
                    Vec::new(),
                    bound("maximum", Operand::Parameter("maximum_count")),
                ),
            ),
            (MORE, BETWEEN, Some("related")),
        ),
        open(),
    ]
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
            joined: Some("; "),
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

/// `containment`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "containment",
        refusals: Refusals::Objects,
        defaults: Vec::new(),
        // The capability's declaration, in its order and words, refused
        // for each inner element.
        declaration: vec![Check::Arguments {
            when: &[],
            value: ITEMS,
        }],
        // The list reports a missing service for each inner element, as
        // the capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the inner element is judged".into()),
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
            checks: vec![items(ITEMS, inner(), None)],
            once: Vec::new(),
            joined: None,
            project: vec![items(COUNTS, outer(), Some("object"))],
        }],
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    super::parameters()
}
