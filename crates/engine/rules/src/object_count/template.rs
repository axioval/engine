//! `object-count` as a template: the count of the objects the rule
//! selects in each source, or in the project with `across_sources`,
//! widened by those whose selection is undecided, judged by the range
//! judge against `minimum` and `maximum` (at least one without either).

use axioval_engine::template::{
    Check, Condition, Decision, Form, Operand, ScopeMessages, Scopes, Template, TemplateValue,
    Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{AggregateFunction, Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.object-count";

/// How many objects the rule selects in the scope: those surely selected,
/// up to those possibly selected.
fn count() -> TemplateValue {
    TemplateValue {
        name: "count",
        expression: Expression::Aggregate {
            function: AggregateFunction::Count,
            over: Scopes::source(),
            filter: None,
            value: None,
            label: Some("objects selected in the scope".into()),
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// The one object an existence check needs.
fn one() -> TemplateValue {
    TemplateValue {
        name: "one",
        expression: Expression::Literal {
            value: ScalarValue::Integer { value: 1 },
            label: Some("an existence check".into()),
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// The scope: each source, or the project with `across_sources`; only the
/// sources playing a listed discipline with `disciplines`.
fn scopes() -> Scopes {
    Scopes {
        across: "across_sources",
        disciplines: Some("disciplines"),
        messages: ScopeMessages {
            source: "in source `{source}`",
            project: "in the project",
            no_source: "object-count: the project has no source to count in",
            no_discipline: "object-count: no source plays {disciplines}, so there is no source to \
                            count in",
            undeclared: "object-count: source `{source}` declares no discipline, so whether it is \
                         counted is unknown",
            undeclared_member: "source `{source}` declares no discipline, so whether this object \
                                is counted is unknown",
            unlisted: "object-count: its resource objects cannot be listed: {why}",
            no_disciplines: "object-count: source disciplines are not available outside an \
                             evidence session",
        },
    }
}

/// A form counting the scope's objects against `minimum` and `maximum`
/// (unstated ones left out), or, `existence`, against at least one.
fn form(when: &'static [&'static str], existence: bool) -> Form {
    let mut values = vec![count()];
    let minimum = if existence {
        values.push(one());
        Term::plus(Operand::Value("one"))
    } else {
        Term::plus(Operand::Parameter("minimum"))
    };
    Form {
        when,
        values,
        decision: Decision::Within {
            value: "count",
            minimum: Some(vec![minimum]),
            maximum: (!existence).then(|| vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding: Vec::new(),
        },
        fail: "{matched}; required {required:exactly}",
        undecided: "object-count: {count:least} object(s) match the selection {place} and \
                    {undecided} more may; required {required:exactly}",
        members: None,
        table: None,
        scope: Some(scopes()),
        unless: Vec::new(),
        grading: None,
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
    }
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Kind {
            parameter: "minimum",
        },
        Check::Kind {
            parameter: "maximum",
        },
        Check::NonNegative {
            parameters: &["minimum"],
            message: "minimum is negative",
        },
        Check::NonNegative {
            parameters: &["maximum"],
            message: "maximum is negative",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "minimum exceeds maximum",
        },
        Check::Disciplines {
            parameter: "disciplines",
        },
        Check::Kind {
            parameter: "across_sources",
        },
    ]
}

/// `object-count`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
            ParameterDescriptor::optional("maximum", ParameterType::Integer),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
            ParameterDescriptor::optional("disciplines", ParameterType::StringList),
        ],
        grades: false,
        name: "object-count",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: vec![
            Text {
                name: "matched",
                when: Some(Condition::Zero { value: "count" }),
                text: "no object matches the selection {place}",
            },
            Text {
                name: "matched",
                when: None,
                text: "{count:least} object(s) match the selection {place}",
            },
        ],
        forms: vec![
            form(&["minimum"], false),
            form(&["maximum"], false),
            form(&[], true),
        ],
    }
}
