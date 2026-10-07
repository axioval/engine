//! `opening-spaces` as a template: each source the rule reaches judged by
//! whether any of its walls declares itself external, and each selected
//! element by the measured list `connected_spaces`: as many spaces as its
//! host's exposure needs, on the faces it needs them.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Condition, Decision, Form, FormCheck, Grading,
    ItemCheck, ItemTest, ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement,
    ScopeMessages, ScopeSources, Scopes, Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.opening-spaces";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("host_path", ParameterType::StringList),
        ParameterDescriptor::required("host_selector", ParameterType::Selector),
        ParameterDescriptor::required("external_property", ParameterType::PropertyReference),
        ParameterDescriptor::required("space_path", ParameterType::StringList),
        ParameterDescriptor::optional("space_selector", ParameterType::Selector),
    ]
}

/// The element's hosts and spaces.
const CONNECTED: &str = "connected_spaces;host_path=@host_path;host_selector=@host_selector;\
                         external_property=@external_property;space_path=@space_path;\
                         space_selector=@space_selector";

/// A source's walls, as measured values of the source.
macro_rules! walls {
    ($name:literal) => {
        concat!(
            $name,
            ";host_selector=@host_selector;external_property=@external_property"
        )
    };
}

fn measured(name: &'static str, property: &'static str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: property.to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

/// How the element relates, worded for a finding.
const VIOLATION: &str = "{relates}; in {wall} ({hosts}) it needs {requirement}";

/// A test of an item number against the spaces the exposure needs.
fn counted(value: &'static str, least: bool, then: ItemTest) -> ItemTest {
    let needed = vec![Requirement {
        name: "needed",
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Value("expected")),
        }],
        words: "",
    }];
    let (at_least, at_most) = if least {
        (needed, Vec::new())
    } else {
        (Vec::new(), needed)
    };
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Range(Box::new(Range {
            value,
            unit: ItemUnit::Count,
            at_least,
            at_most,
            allowance: Allowance::None,
            grade: false,
            null: OnNull::Judge,
            unmeasured: None,
        })),
        fail: VIOLATION,
        undecided: VIOLATION,
        effects: Vec::new(),
        then: Some(Box::new(then)),
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

/// A truth of the item that must hold.
fn holds(value: &'static str, fail: &'static str, then: Option<ItemTest>) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Truth {
            value,
            finding: false,
        },
        fail,
        undecided: "{why}",
        effects: Vec::new(),
        then: then.map(Box::new),
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

/// Each element: not more spaces than its exposure needs, not fewer than
/// it may, every space decided, and, for the derived adjacency, on the
/// faces it needs.
fn elements() -> FormCheck {
    let sides = holds(
        "sides",
        "{placed}; in {wall} ({hosts}) it needs {requirement}",
        None,
    );
    let known = holds("known", "", Some(sides));
    let test = counted("count", false, counted("possible", true, known));
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: CONNECTED,
            // An element whose hosts or spaces cannot be read is open, as
            // the capability worded it.
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(test))],
            together: None,
            passing: None,
            texts: Vec::new(),
            once: false,
            at: None,
            merged: false,
            combined: None,
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

/// `opening-spaces`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "opening-spaces",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        // The paths, selectors and property, as the measurement reads
        // them: in the capability's order and words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: CONNECTED,
        }],
        services: None,
        texts: vec![
            Text {
                name: "found",
                text: "source `{source}` has no host wall, so none is external",
                when: Some(Condition::Zero { value: "walls" }),
            },
            Text {
                name: "found",
                text: "none of the {walls:upper0} host wall(s) in source `{source}` is \
                       declared external",
                when: None,
            },
        ],
        forms: vec![form()],
    }
}

fn form() -> Form {
    Form {
        when: &[],
        values: vec![
            measured("declared", walls!("any_external")),
            TemplateValue {
                name: "one",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: None,
                },
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            },
        ],
        decision: Decision::Within {
            value: "declared",
            minimum: Some(vec![Term::plus(Operand::Value("one"))]),
            maximum: None,
            rounding: Vec::new(),
        },
        fail: "{found}",
        undecided: "opening-spaces: no wall in source `{source}` is declared external, but \
                    {unknown:upper0} wall(s) do not declare `{external_property}` and \
                    {maybe:upper0} more may be walls",
        members: None,
        table: None,
        scope: Some(Scopes {
            // No parameter: each source is a scope.
            across: "",
            disciplines: None,
            sources: ScopeSources::Reached {
                selector: "host_selector",
            },
            needs: None,
            messages: ScopeMessages {
                source: "in source `{source}`",
                project: "in the project",
                no_source: "opening-spaces: the run checks no source",
                no_discipline: "opening-spaces: no source plays {disciplines}",
                undeclared: "opening-spaces: `{source}` declares no discipline",
                undeclared_member: "opening-spaces: `{source}` declares no discipline",
                unlisted: "opening-spaces: the objects of `{source}` cannot be listed: {why}",
                no_disciplines: "opening-spaces: source disciplines are not available",
            },
        }),
        derived: Vec::new(),
        // A source's finding relates its walls.
        related: Some("declared"),
        checks: vec![elements()],
        unless: Vec::new(),
        // What only the messages word: the walls where the source fails,
        // the walls declaring nothing and the possible ones where it is
        // undecided.
        grading: Some(Grading {
            values: vec![measured("walls", walls!("host_walls"))],
            derived: Vec::new(),
            bands: Vec::new(),
            undecided: vec![
                measured("unknown", walls!("undeclared_hosts")),
                measured("maybe", walls!("possible_hosts")),
            ],
        }),
        once: Vec::new(),
        project: Vec::new(),
        joined: None,
    }
}
