//! `property-predicate` as a template: the stated property read by the
//! expression evaluator, judged by the generic comparison judge
//! ([`Decision::Compare`]) through the one comparison every rule uses.

use axioval_engine::comparison::Order;
use axioval_engine::template::{
    Comparison, ComparisonTarget, Decision, Form, Operation, Presence, TargetKind, Template,
    TemplateValue, Test,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::Expression;
use serde_json::json;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.property-predicate";

/// The ordered operator words.
const ORDERS: &[Operation] = &[
    Operation {
        word: "equal",
        test: Test::Order(Order::Equal),
    },
    Operation {
        word: "not_equal",
        test: Test::Order(Order::NotEqual),
    },
    Operation {
        word: "greater_than",
        test: Test::Order(Order::Greater),
    },
    Operation {
        word: "greater_or_equal",
        test: Test::Order(Order::GreaterOrEqual),
    },
    Operation {
        word: "less_than",
        test: Test::Order(Order::Less),
    },
    Operation {
        word: "less_or_equal",
        test: Test::Order(Order::LessOrEqual),
    },
];

/// The equality operator words, all a truth takes.
const EQUALITY: &[Operation] = &[
    Operation {
        word: "equal",
        test: Test::Order(Order::Equal),
    },
    Operation {
        word: "not_equal",
        test: Test::Order(Order::NotEqual),
    },
];

/// The operator words a text takes.
const TEXT: &[Operation] = &[
    Operation {
        word: "equal",
        test: Test::Order(Order::Equal),
    },
    Operation {
        word: "not_equal",
        test: Test::Order(Order::NotEqual),
    },
    Operation {
        word: "contains",
        test: Test::Contains,
    },
    Operation {
        word: "matches",
        test: Test::Matches,
    },
];

/// The operator words a text list takes.
const TEXTS: &[Operation] = &[
    Operation {
        word: "one_of",
        test: Test::OneOf,
    },
    Operation {
        word: "none_of",
        test: Test::NoneOf,
    },
];

/// The comparison a rule states, its targets in the order the capability
/// checked them.
fn comparison() -> Comparison {
    let target = |parameter, kind, operators| ComparisonTarget {
        parameter,
        kind,
        operators,
    };
    Comparison {
        operator: "operator",
        presence: &[
            Presence {
                word: "is_defined",
                defined: true,
            },
            Presence {
                word: "is_undefined",
                defined: false,
            },
        ],
        targets: vec![
            target("value", TargetKind::Integer, ORDERS),
            target("number", TargetKind::Number, ORDERS),
            target("quantity", TargetKind::Quantity, ORDERS),
            target("date", TargetKind::Date, ORDERS),
            target("date_time", TargetKind::DateTime, ORDERS),
            target("boolean", TargetKind::Boolean, EQUALITY),
            target("texts", TargetKind::Texts, TEXTS),
            target("text", TargetKind::Text, TEXT),
        ],
        case_sensitive: Some("case_sensitive"),
        precision: Some("precision"),
        tolerance: true,
    }
}

/// `property-predicate`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    let actual: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": "{property_set}",
        "property": "{property}",
    }))
    .expect("a built-in template's expression is well formed");
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("property_set", ParameterType::String),
            ParameterDescriptor::required("property", ParameterType::String),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::optional("value", ParameterType::Integer).per_object(),
            ParameterDescriptor::optional("number", ParameterType::Number).per_object(),
            ParameterDescriptor::optional("quantity", ParameterType::Quantity).per_object(),
            ParameterDescriptor::optional("text", ParameterType::String).per_object(),
            ParameterDescriptor::optional("texts", ParameterType::StringList),
            ParameterDescriptor::optional("boolean", ParameterType::Boolean).per_object(),
            ParameterDescriptor::optional("date", ParameterType::Date).per_object(),
            ParameterDescriptor::optional("date_time", ParameterType::DateTime).per_object(),
            ParameterDescriptor::optional("precision", ParameterType::String),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::support::tolerance_parameters())
        .collect(),
        grades: false,
        name: "property-predicate",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration: Vec::new(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "actual",
                expression: actual,
                expect: None,
                absent: None,
                mismatch: None,
            }],
            decision: Decision::Compare {
                value: "actual",
                comparison: comparison(),
            },
            fail: "property {property_set}.{property} does not satisfy \
                   {operator}{target}{tolerance:suffix}; actual value is {actual:stated}",
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
            project: Vec::new(),
        }],
    }
}
