//! `relative-count` as a template: the provided and required objects each
//! anchor reaches (two member populations), or each group of the counted
//! objects by a stated value holds, judged by `Decision::Proportion` in
//! exact integer arithmetic.

use axioval_engine::template::{
    Check, Decision, Form, Members, Proportion, ProportionGroups, ProportionParameters, Refusals,
    Template, TemplateValue, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{AggregateFunction, Expression};
use serde_json::json;

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.relative-count";

/// The parameters stating the proportion.
const PROPORTION: ProportionParameters = ProportionParameters {
    provided_unit: "provided_unit",
    required_unit: "required_unit",
    operator: "operator",
    small_below: "small_required_below",
    small_provided: "small_provided",
    table: "table",
    additional_required: "additional_required",
    additional_provided: "additional_provided",
};

/// Each traversal parameter, in the descriptor's order, and its refusal
/// beside `group_property`.
const TRAVERSAL: [(&[&str], &str); 5] = [
    (
        &["relationship"],
        "`relationship` does not apply with group_property",
    ),
    (
        &["direction"],
        "`direction` does not apply with group_property",
    ),
    (
        &["follow_chain"],
        "`follow_chain` does not apply with group_property",
    ),
    (&["path"], "`path` does not apply with group_property"),
    (
        &["skip_absent_relationship_ends"],
        "`skip_absent_relationship_ends` does not apply with group_property",
    ),
];

/// What a rule's parameters must satisfy, in the order the capability
/// checked them: both selectors, the proportion, then the grouping.
fn declaration() -> Vec<Check> {
    let mut checks = vec![
        Check::Required {
            parameter: "provided_selector",
        },
        Check::Required {
            parameter: "required_selector",
        },
        Check::Proportion(PROPORTION),
        Check::Kind {
            parameter: "group_property",
        },
        Check::Requires {
            parameter: "across_sources",
            with: &["group_property"],
            message: "`across_sources` applies only with group_property",
        },
        Check::Requires {
            parameter: "case_sensitive",
            with: &["group_property"],
            message: "`case_sensitive` applies only with group_property",
        },
    ];
    checks.extend(TRAVERSAL.map(|(other, message)| Check::Exclusive {
        one: &["group_property"],
        other,
        message,
    }));
    checks.extend([
        Check::Traversal {
            with: &[],
            message: "",
        },
        Check::Kind {
            parameter: "across_sources",
        },
        Check::Kind {
            parameter: "case_sensitive",
        },
    ]);
    checks
}

/// The count of an anchor's members a selector parameter picks.
fn count(name: &'static str, selector: &str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Aggregate {
            function: AggregateFunction::Count,
            over: Members::source(selector),
            filter: None,
            value: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// A form with `values` deciding by `decision`, its other parts empty.
fn form(
    when: &'static [&'static str],
    values: Vec<TemplateValue>,
    groups: Option<ProportionGroups>,
) -> Form {
    Form {
        grading: None,
        unless: Vec::new(),
        when,
        values,
        decision: Decision::Proportion(Box::new(Proportion {
            provided: "provided",
            required: "required",
            parameters: PROPORTION,
            groups,
        })),
        fail: "",
        undecided: "",
        members: None,
        table: None,
        scope: None,
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
        once: Vec::new(),
        joined: None,
        project: Vec::new(),
    }
}

/// With `group_property`: the rule's selection counted per group of the
/// stated value.
fn groups() -> Form {
    let group: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": "{group_property.set}",
        "property": "{group_property.name}",
    }))
    .expect("a built-in template's expression is well formed");
    let value = TemplateValue {
        name: "group",
        expression: group,
        expect: None,
        absent: None,
        mismatch: None,
    };
    form(
        &["group_property"],
        vec![value],
        Some(ProportionGroups {
            value: "group",
            provided: "provided_selector",
            required: "required_selector",
            across: "across_sources",
            case_sensitive: "case_sensitive",
            label: "group {group_property} {group:stated}",
            ungrouped: "{group_property} is {group:stated}, so the object counts in no group",
            undecided_ungrouped: "{group_property} is {group:stated}, and whether the object is \
                                  counted is undecided",
            undecided: "{label}: {undecided} object(s) cannot be assigned to either population",
            unreadable: "{label}: an object whose {group_property} could not be read may belong \
                         here",
            only_required: "{label}: {required} required object(s) and no provided object; the \
                            group is present only in the required set",
            fail: "{label}: {provided} provided and {required} required object(s); required \
                   {requirement}",
        }),
    )
}

/// Otherwise: each anchor's provided and required members, reached along
/// the rule's traversal or in its whole source.
fn anchors() -> Form {
    Form {
        fail: "{provided:least} provided and {required:least} required object(s) {relation}; \
               required {requirement}",
        members: Some(Members {
            selector: "provided_selector",
            undecided: UndecidedMembers::Open {
                message: "{undecided} related object(s) {relation} cannot be assigned to either \
                          population",
            },
            every_when_unstated: false,
            same_ends: None,
            more: &["required_selector"],
            checks: Vec::new(),
        }),
        ..form(
            &[],
            vec![
                count("provided", "provided_selector"),
                count("required", "required_selector"),
            ],
            None,
        )
    }
}

/// `relative-count`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("provided_selector", ParameterType::Selector),
            ParameterDescriptor::required("required_selector", ParameterType::Selector),
            ParameterDescriptor::optional("provided_unit", ParameterType::Integer),
            ParameterDescriptor::optional("required_unit", ParameterType::Integer),
            ParameterDescriptor::optional("operator", ParameterType::String),
            ParameterDescriptor::optional("small_required_below", ParameterType::Integer),
            ParameterDescriptor::optional("small_provided", ParameterType::Integer),
            ParameterDescriptor::optional("table", ParameterType::StringList),
            ParameterDescriptor::optional("additional_required", ParameterType::Integer),
            ParameterDescriptor::optional("additional_provided", ParameterType::Integer),
            ParameterDescriptor::optional("group_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect(),
        grades: false,
        name: "relative-count",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![groups(), anchors()],
    }
}
