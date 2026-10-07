//! `table-allocation` as a template: the allocation of the rule's selection
//! to the rows (`allocations`, a list of the project) judged where each
//! outcome goes. An object no row matches is a finding, each row's objects
//! in a group must be exactly its `count` and their summed area within its
//! `area`, each a graded finding on the source, the project or the anchor;
//! what objects that may still belong to a row leave undecided is open.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, Form, FormCheck, ItemCheck,
    ItemTest, ItemText, ItemUnit, Items, Judge, On, OnNull, Operand, Range, Refusals, Requirement,
    Template, TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.table-allocation";

const ALLOCATIONS: &str = "allocations;rows=@rows;mode=@mode;key_1=@key_1;key_2=@key_2;\
                           key_3=@key_3;key_4=@key_4;case_sensitive=@case_sensitive;\
                           area_property=@area_property;area_mode=@area_mode;\
                           anchor_selector=@anchor_selector;anchor_key=@anchor_key;\
                           across_sources=@across_sources;relationship=@relationship;\
                           direction=@direction;follow_chain=@follow_chain;path=@path;\
                           skip_absent_relationship_ends=@skip_absent_relationship_ends;\
                           selection=@selection";

/// The item's number `field` as a bound named `name`.
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
        grade: true,
        null: OnNull::Skip,
        unmeasured: Some("{why}"),
    }))
}

fn test(
    when: Vec<When>,
    judge: Judge,
    (fail, undecided): (&'static str, &'static str),
    effects: Vec<Effect>,
) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when,
        judge,
        fail,
        undecided,
        effects,
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }))
}

const SUMS: &str = "{name} sums {sum:area} m²{place}; required {required}";
const MORE_SUMS: &str = "table-allocation: {name} sums {sum:area} m²{place} and {open_count:count} \
                         more object(s) may belong to it; required {required}";

fn checks() -> Vec<ItemCheck> {
    vec![
        // An object no row matches.
        test(
            vec![When::Stated { field: "extra" }],
            Judge::Truth {
                value: "extra",
                finding: true,
            },
            ("no row matches ({keys})", ""),
            Vec::new(),
        ),
        // An object or a group left open, for its reason.
        test(
            vec![When::Stated { field: "open" }],
            Judge::Truth {
                value: "open",
                finding: true,
            },
            ("", "{why}"),
            Vec::new(),
        ),
        // A row without a count that matched nothing.
        test(
            vec![
                When::Field {
                    field: "empty",
                    value: true,
                },
                When::Null { field: "count" },
            ],
            Judge::Fails,
            ("{name} matched no object{place}", ""),
            Vec::new(),
        ),
        // Exactly `count` objects.
        test(
            vec![When::Stated { field: "count" }],
            range(
                "found",
                ItemUnit::Count,
                (bound("count", "count"), bound("count", "count")),
            ),
            (
                "{count_words}",
                "table-allocation: {name} has {found:lower0} object(s){place} and \
                 {open_count:count} more that may belong to it; required exactly {count:count}",
            ),
            Vec::new(),
        ),
        // The summed area within the row's, where nothing more may belong.
        test(
            vec![When::Field {
                field: "more",
                value: false,
            }],
            range(
                "sum",
                ItemUnit::Area,
                (bound("low", "low"), bound("high", "high")),
            ),
            (
                SUMS,
                "table-allocation: {name} sums {sum:area} m²{place}, which straddles the \
                 required {required}",
            ),
            Vec::new(),
        ),
        // More may belong: they can only add area.
        test(
            vec![When::Field {
                field: "more",
                value: true,
            }],
            range("sum", ItemUnit::Area, (Vec::new(), bound("high", "high"))),
            (SUMS, MORE_SUMS),
            vec![Effect {
                when: Vec::new(),
                on: On::Pass,
                message: MORE_SUMS,
            }],
        ),
    ]
}

fn allocations() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: ALLOCATIONS,
            refused: Some("table-allocation: {why}"),
            checks: checks(),
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "count_words",
                    when: vec![When::Field {
                        field: "empty",
                        value: true,
                    }],
                    text: "{name} matched no object{place}; required exactly {count:count}",
                },
                ItemText {
                    name: "count_words",
                    when: Vec::new(),
                    text: "{name} has {found:lower0} object(s){place}; required exactly \
                           {count:count}",
                },
            ],
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

/// `table-allocation`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: super::parameters(),
        grades: true,
        name: "table-allocation",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: ALLOCATIONS,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the objects are allocated".into()),
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
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: vec![allocations()],
        }],
    }
}
