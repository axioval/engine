//! `level-spacing` as a template: each anchor's levels (`member_selector`)
//! read one by one (`Decision::Each`), ordered by `order`, each level's
//! height the rise to the next one up (the highest's from its contents'
//! tops), judged against `minimum` and `maximum` and against the
//! prevailing height; each level's spaces judged against the prevailing
//! bottom or top elevation among them and against the level's height.

use axioval_engine::template::{
    Applies, Check, Column, Condition, Decision, Difference, Each, Form, Highest, Judgement,
    Members, Nested, Operand, ParameterDefault, Prevailing, Reference, Rise, Service, Services,
    Table, Template, TemplateValue, Term, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{Expression, ScalarValue};
use serde_json::json;

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.level-spacing";

/// A measured value of the object in scope: its `bottom` or `top`.
fn measured(name: &'static str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: name.to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// The vertical-extent service a measured extent needs.
fn extents() -> Services {
    Services {
        needs: vec![Service::VerticalExtent],
        message: "vertical-extent service is not registered",
    }
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::NonNegativeLength {
            parameter: "minimum",
            message: "minimum must be a non-negative length",
        },
        Check::NonNegativeLength {
            parameter: "maximum",
            message: "maximum must be a non-negative length",
        },
        Check::Kind {
            parameter: "consistent",
        },
        Check::Kind {
            parameter: "content_path",
        },
        Check::Kind {
            parameter: "content_selector",
        },
        Check::Requires {
            parameter: "content_selector",
            with: &["content_path"],
            message: "`content_selector` needs a `content_path`",
        },
        Check::Path {
            parameter: "content_path",
        },
        Check::Kind {
            parameter: "space_selector",
        },
        Check::Kind {
            parameter: "space_path",
        },
        Check::NonNegativeLength {
            parameter: "space_tolerance",
            message: "space_tolerance must be a non-negative length",
        },
        Check::Together {
            parameters: &["space_selector", "space_path", "space_tolerance"],
            message: "`space_selector`, `space_path` and `space_tolerance` go together",
        },
        Check::Path {
            parameter: "space_path",
        },
        Check::Kind {
            parameter: "space_height",
        },
        Check::Among {
            parameter: "space_elevation",
            options: &["bottom", "top", "both"],
            message: "`space_elevation` is `{value}`, not `bottom`, `top` or `both`",
        },
        Check::Requires {
            parameter: "space_height",
            with: &["space_selector"],
            message: "`space_height` and `space_elevation` need `space_selector`, `space_path` \
                      and `space_tolerance`",
        },
        Check::Requires {
            parameter: "space_elevation",
            with: &["space_selector"],
            message: "`space_height` and `space_elevation` need `space_selector`, `space_path` \
                      and `space_tolerance`",
        },
        Check::FalseRequires {
            flag: "space_height",
            with: &["space_elevation"],
            message: "with `space_height` false, the spaces need a `space_elevation` to check",
        },
        Check::Declares {
            parameters: &["minimum", "maximum", "consistent", "space_selector"],
            message: "declare a minimum, a maximum, consistent or a space_selector",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "minimum exceeds maximum",
        },
        Check::Required {
            parameter: "member_selector",
        },
        Check::Required { parameter: "order" },
        Check::NonNegativeLength {
            parameter: "tolerance",
            message: "tolerance must be a non-negative length",
        },
        Check::Kind {
            parameter: "ignore_lowest",
        },
        Check::Kind {
            parameter: "ignore_highest",
        },
        Check::Traversal {
            with: &[],
            message: "",
        },
    ]
}

/// The spaces of a level judged against the prevailing exact `side`
/// elevation among them.
fn elevation(side: &'static str, sides: &'static [&'static str]) -> Judgement {
    let (fail, undecided, missing) = match side {
        "bottom" => (
            "space bottom elevation is {bottom:length}, and the prevailing bottom elevation of \
             the spaces of level {member} is {reference:length}; they may differ by at most \
             {space_tolerance:length}",
            "space bottom elevation is {bottom:length}, and the prevailing bottom elevation of \
             the spaces of level {member} is {reference:length}, which straddles the tolerance \
             of {space_tolerance:length}",
            "no space of level {member} has an exact bottom elevation to compare with",
        ),
        _ => (
            "space top elevation is {top:length}, and the prevailing top elevation of the \
             spaces of level {member} is {reference:length}; they may differ by at most \
             {space_tolerance:length}",
            "space top elevation is {top:length}, and the prevailing top elevation of the \
             spaces of level {member} is {reference:length}, which straddles the tolerance of \
             {space_tolerance:length}",
            "no space of level {member} has an exact top elevation to compare with",
        ),
    };
    Judgement {
        applies: Applies {
            condition: Some(Condition::OneOf {
                parameter: "space_elevation",
                values: sides,
            }),
            ..Applies::default()
        },
        decision: Decision::Near {
            value: side,
            reference: Reference::Prevailing(Prevailing {
                value: side,
                missing: Some(missing),
            }),
            tolerance: Operand::Parameter("space_tolerance"),
        },
        fail,
        undecided,
        least: 0,
    }
}

/// A level's spaces: those `space_selector` picks that `space_path`
/// reaches, each with its bottom and top.
fn spaces(name: &'static str, applies: Applies) -> Nested {
    Nested {
        name,
        applies,
        path: "space_path",
        selector: Some("space_selector"),
        undecided: "{undecided} space(s) {relation} cannot be assigned",
        least: 1,
        fewer: None,
        services: Some(extents()),
        services_first: false,
        errors_open_member: false,
        members_with: None,
        values: vec![measured("bottom"), measured("top")],
        differences: Vec::new(),
        checks: Vec::new(),
        table: None,
    }
}

/// The members judged one by one.
#[allow(clippy::too_many_lines)]
fn each() -> Each {
    let order: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": "{order.set}",
        "property": "{order.name}",
    }))
    .expect("a built-in template's expression is well formed");
    let mut elevations = spaces(
        "space elevations",
        Applies {
            when: &["space_selector", "space_elevation"],
            ..Applies::default()
        },
    );
    elevations.least = 2;
    elevations.checks = vec![
        elevation("bottom", &["bottom", "both"]),
        elevation("top", &["top", "both"]),
    ];
    let mut heights = spaces(
        "space heights",
        Applies {
            when: &["space_selector", "space_height"],
            ..Applies::default()
        },
    );
    heights.members_with = Some("height");
    heights.differences = vec![Difference {
        name: "height",
        minuend: "top",
        subtrahend: "bottom",
    }];
    heights.checks = vec![Judgement {
        applies: Applies::default(),
        decision: Decision::Near {
            value: "height",
            reference: Reference::Value("member:height"),
            tolerance: Operand::Parameter("space_tolerance"),
        },
        fail: "space height is {height:length} and its level's height \
               {member:height:length}; they may differ by at most {space_tolerance:length}",
        undecided: "space height is {height:length} and its level's height \
                    {member:height:length}, which straddles the tolerance of \
                    {space_tolerance:length}",
        least: 0,
    }];
    heights.table = Some(axioval_engine::template::NestedTable {
        name: "spaces",
        member: "level",
        columns: vec![
            Column {
                id: "height",
                value: "height",
                dimension: QuantityDimension::Length,
            },
            Column {
                id: "level_height",
                value: "member:height",
                dimension: QuantityDimension::Length,
            },
        ],
    });
    let contents = Nested {
        name: "contents",
        applies: Applies {
            when: &["content_path"],
            ..Applies::default()
        },
        path: "content_path",
        selector: Some("content_selector"),
        undecided: "{undecided} content(s) {relation} cannot be assigned",
        least: 1,
        fewer: Some(
            "the highest level reaches no contents {relation}, so its height cannot be measured",
        ),
        services: Some(extents()),
        services_first: true,
        errors_open_member: true,
        members_with: None,
        values: vec![measured("top")],
        differences: Vec::new(),
        checks: Vec::new(),
        table: None,
    };
    Each {
        values: vec![TemplateValue {
            name: "elevation",
            expression: order,
            expect: None,
            absent: None,
            mismatch: None,
        }],
        order: "elevation",
        unordered: "{member} has no length {order} ({elevation:stated}), so the levels cannot \
                    be measured",
        skip_first: Some("ignore_lowest"),
        skip_last: Some("ignore_highest"),
        rise: Rise {
            name: "height",
            last: Some(Highest {
                nested: "contents",
                value: "top",
            }),
            open: "the highest level has no level above it; its height needs geometry",
        },
        checks: vec![
            Judgement {
                applies: Applies {
                    any: &["minimum", "maximum"],
                    ..Applies::default()
                },
                decision: Decision::Within {
                    value: "height",
                    minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
                    maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
                    rounding: Vec::new(),
                },
                fail: "level height is {height:length}; required {bound}",
                undecided: "level height is {height:length}, which straddles the bound {bound}",
                least: 0,
            },
            Judgement {
                applies: Applies {
                    when: &["consistent"],
                    ..Applies::default()
                },
                decision: Decision::Near {
                    value: "height",
                    reference: Reference::Prevailing(Prevailing {
                        value: "height",
                        missing: None,
                    }),
                    tolerance: Operand::Parameter("tolerance"),
                },
                fail: "level height {height:length} differs from the prevailing \
                       {reference:length}",
                undecided: "level height is {height:length}, which may or may not match the \
                            prevailing {reference:length}",
                least: 2,
            },
        ],
        nested: vec![contents, elevations, heights],
        table: Some(Table {
            name: "levels",
            columns: vec![
                Column {
                    id: "elevation",
                    value: "elevation",
                    dimension: QuantityDimension::Length,
                },
                Column {
                    id: "height",
                    value: "height",
                    dimension: QuantityDimension::Length,
                },
            ],
        }),
    }
}

/// `level-spacing`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    let length = |value| ScalarValue::Quantity {
        value,
        unit: "m".into(),
    };
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("member_selector", ParameterType::Selector),
            ParameterDescriptor::required("order", ParameterType::PropertyReference),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("consistent", ParameterType::Boolean),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("ignore_lowest", ParameterType::Boolean),
            ParameterDescriptor::optional("ignore_highest", ParameterType::Boolean),
            ParameterDescriptor::optional("content_path", ParameterType::StringList),
            ParameterDescriptor::optional("content_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_path", ParameterType::StringList),
            ParameterDescriptor::optional("space_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("space_height", ParameterType::Boolean),
            ParameterDescriptor::optional("space_elevation", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect(),
        grades: false,
        name: "level-spacing",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: vec![
            ParameterDefault {
                parameter: "tolerance",
                value: length(1e-3),
                from: &[],
            },
            ParameterDefault {
                parameter: "space_height",
                value: ScalarValue::Boolean { value: true },
                from: &[],
            },
        ],
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: Vec::new(),
            decision: Decision::Each(Box::new(each())),
            fail: "",
            undecided: "",
            members: Some(Members {
                selector: "member_selector",
                undecided: UndecidedMembers::Refuse {
                    message: "member selection is undecided: {why}",
                },
                every_when_unstated: false,
                same_ends: None,
                more: &[],
                checks: Vec::new(),
            }),
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
        }],
    }
}
