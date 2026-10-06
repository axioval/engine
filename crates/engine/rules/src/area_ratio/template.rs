//! `area-ratio` as a template: two populations of an anchor's members (the
//! numerator's and the denominator's, or the anchor itself), each summed as
//! a measured or stated area, their ratio judged by the range judge,
//! graded, and reported in the table `ratios`. With `numerator_derivation`
//! `light-area`, each numerator member's area is its light area, and a
//! member stating one larger than itself is judged on its own.

use axioval_engine::template::{
    Check, Column, Condition, Decision, Derived, End, Form, Magnitude, MemberCheck, Members,
    NUMBER, Operand, ParameterDefault, Refusals, Table, Template, TemplateValue, Term, Text,
    UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{
    AggregateFunction, Branch, Expression, ExpressionComparison, ScalarValue,
};

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.area-ratio";

/// The selector parameter picking the numerator's members.
const NUMERATOR: &str = "numerator_selector";

/// The selector parameter picking the denominator's members.
const DENOMINATOR: &str = "denominator_selector";

/// The parameter choosing the light-area numerator.
const DERIVATION: &str = "numerator_derivation";

/// The light-area chain, each of its parameters named as the rule names it.
macro_rules! light_chain {
    () => {
        "stated=@numerator_property;overall_width=@overall_width;\
         overall_height=@overall_height;light_area_table=@light_area_table;\
         light_type=@light_type;light_type_path=@light_type_path;\
         light_size_tolerance=@light_size_tolerance;frame_width=@frame_width"
    };
}
const LIGHT: &str = light_chain!();

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required(NUMERATOR, ParameterType::Selector),
        ParameterDescriptor::optional(DENOMINATOR, ParameterType::Selector),
        ParameterDescriptor::optional("minimum", ParameterType::Number),
        ParameterDescriptor::optional("maximum", ParameterType::Number),
        ParameterDescriptor::optional("numerator_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("denominator_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("measure", ParameterType::String),
        ParameterDescriptor::optional("numerator_measure", ParameterType::String),
        ParameterDescriptor::optional("denominator_measure", ParameterType::String),
        ParameterDescriptor::optional(DERIVATION, ParameterType::String),
        ParameterDescriptor::optional("empty_numerator_finding", ParameterType::Boolean),
    ]
    .into_iter()
    .chain(crate::light_area::parameters())
    .chain(traversal_parameters())
    .collect()
}

/// A measured value of the object in scope.
fn measured(name: String) -> Expression {
    Expression::Property {
        property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
        property: name,
        of: None,
        label: None,
    }
}

/// The area of the object in scope on the `side` (`numerator` or
/// `denominator`) of the ratio: stated by `<side>_property`, otherwise
/// measured as `<side>_measure` says, else as `measure` says for both.
fn area(side: &str) -> Expression {
    measured(format!(
        "ratio_area;property=@{side}_property;measure=@{side}_measure;otherwise=@measure"
    ))
}

/// The light-area chain's value `name` with its own parameter.
fn light(name: &str, own: &str) -> Expression {
    measured(format!("{name};{LIGHT};{own}"))
}

/// An aggregate over the members `selector` picks.
fn over(function: AggregateFunction, selector: &str, value: Option<Expression>) -> Expression {
    Expression::Aggregate {
        function,
        over: Members::source(selector),
        filter: None,
        value: value.map(Box::new),
        label: None,
    }
}

fn value(name: &'static str, expression: Expression) -> TemplateValue {
    TemplateValue {
        name,
        expression,
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// `null` where the rule asks for a finding on an anchor reaching no
/// numerator member, which then stands without a ratio.
fn reached() -> TemplateValue {
    let reached = Expression::Literal {
        value: ScalarValue::Boolean { value: true },
        label: None,
    };
    TemplateValue {
        absent: Some("no numerator object is reached {relation}; the ratio is 0"),
        ..value(
            "reached",
            Expression::If {
                // Members are counted only where the rule asks.
                branches: vec![
                    Branch {
                        when: Expression::Not {
                            operand: Box::new(Expression::Parameter {
                                name: "empty_numerator_finding".into(),
                                label: None,
                            }),
                            label: None,
                        },
                        then: reached.clone(),
                    },
                    Branch {
                        when: Expression::Compare {
                            operator: ExpressionComparison::Equals,
                            left: Box::new(over(AggregateFunction::Count, NUMERATOR, None)),
                            right: Box::new(Expression::Literal {
                                value: ScalarValue::Integer { value: 0 },
                                label: None,
                            }),
                            case_sensitive: true,
                            label: None,
                        },
                        then: Expression::Null { label: None },
                    },
                ],
                otherwise: Box::new(reached),
                label: Some("numerator members".into()),
            },
        )
    }
}

/// The values a form reads: whether the anchor reaches a numerator member,
/// the numerator (light areas, then how many each step gave, where
/// `light`), and the denominator over its members or the anchor's own.
fn values(light_areas: bool, members: bool) -> Vec<TemplateValue> {
    let mut values = vec![reached()];
    if light_areas {
        values.push(value(
            "numerator",
            over(
                AggregateFunction::Sum,
                NUMERATOR,
                Some(light("light_area", "read=area")),
            ),
        ));
        for (name, step) in [
            ("stated_count", "step=stated"),
            ("table_count", "step=table"),
            ("frame_count", "step=frame"),
        ] {
            values.push(value(
                name,
                over(
                    AggregateFunction::Sum,
                    NUMERATOR,
                    Some(light("light_step", step)),
                ),
            ));
        }
    } else {
        values.push(value(
            "numerator",
            over(AggregateFunction::Sum, NUMERATOR, Some(area("numerator"))),
        ));
    }
    values.push(value(
        "denominator",
        if members {
            over(
                AggregateFunction::Sum,
                DENOMINATOR,
                Some(area("denominator")),
            )
        } else {
            area("denominator")
        },
    ));
    values
}

/// A stated light area larger than its member, judged on the member.
fn oversized() -> MemberCheck {
    MemberCheck {
        before: "numerator",
        values: vec![
            value("light", light("light_area", "read=stated")),
            value("overall", light("light_area", "read=overall")),
            value("width", light("light_size", "side=width")),
            value("height", light("light_size", "side=height")),
        ],
        decision: Decision::Within {
            value: "light",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Value("overall"))]),
            rounding: vec![Magnitude {
                end: End::Upper,
                operand: Operand::Value("overall"),
            }],
        },
        fail: "light area {light:area} m² ({numerator_property}) is larger than the overall \
               area {overall:area} m² ({overall_width} {width:area} m × {overall_height} \
               {height:area} m)",
        undecided: "light area {light:area} m² ({numerator_property}) straddles the overall \
                    area {overall:area} m²",
        open: "the light area cannot be compared with the overall area: {why}",
        failed: "{failed} member(s), first {first}, state a light area larger than the element",
    }
}

/// The table `ratios`, one row per anchor whose ratio was measured.
fn ratios() -> Table {
    Table {
        name: "ratios",
        columns: vec![
            Column {
                id: "numerator_area",
                value: "numerator",
                dimension: QuantityDimension::Area,
            },
            Column {
                id: "denominator_area",
                value: "denominator",
                dimension: QuantityDimension::Area,
            },
            Column {
                id: "ratio",
                value: "ratio",
                dimension: NUMBER,
            },
        ],
    }
}

/// The form for a numerator of `light_areas` or not, over denominator
/// `members` or the anchor itself.
fn form(light_areas: bool, members: bool) -> Form {
    let when: &'static [&'static str] = match (light_areas, members) {
        (true, true) => &[DERIVATION, DENOMINATOR],
        (true, false) => &[DERIVATION],
        (false, true) => &[DENOMINATOR],
        (false, false) => &[],
    };
    Form {
        when,
        values: values(light_areas, members),
        decision: Decision::Within {
            value: "ratio",
            minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding: Vec::new(),
        },
        fail: if light_areas {
            "{noun} ratio is {ratio:area} ({numerator:least2} m² of {denominator:least2} m²); \
             required {bound:plain}{provenance}"
        } else {
            "{noun} ratio is {ratio:area} ({numerator:least2} m² of {denominator:least2} m²); \
             required {bound:plain}"
        },
        undecided: "{noun} ratio is {ratio:area}, which straddles the bound {bound:plain}",
        members: Some(Members {
            selector: NUMERATOR,
            undecided: UndecidedMembers::Open {
                message: "{undecided} related object(s) {relation} cannot be assigned",
            },
            every_when_unstated: false,
            same_ends: None,
            more: if members { &[DENOMINATOR] } else { &[] },
            checks: if light_areas {
                vec![oversized()]
            } else {
                Vec::new()
            },
        }),
        table: Some(ratios()),
        scope: None,
        unless: Vec::new(),
        grading: None,
        derived: vec![Derived::Ratio {
            name: "ratio",
            numerator: "numerator",
            denominator: "denominator",
            zero: "the denominator has no {bottom}",
        }],
        related: Some("members:numerator_selector"),
        checks: Vec::new(),
    }
}

/// Parameters only the light-area numerator reads, in the order it named
/// them refusing them without it.
const LIGHT_OWN: [&str; 7] = [
    "overall_width",
    "overall_height",
    "light_area_table",
    "light_type",
    "light_type_path",
    "light_size_tolerance",
    "frame_width",
];

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    let mut checks = vec![
        Check::Kind {
            parameter: "minimum",
        },
        Check::Kind {
            parameter: "maximum",
        },
        Check::AnyOf {
            parameters: &["minimum", "maximum"],
            message: "minimum or maximum is required",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "minimum exceeds maximum",
        },
        Check::Kind {
            parameter: "numerator_property",
        },
        Check::Required {
            parameter: NUMERATOR,
        },
        Check::Kind {
            parameter: DENOMINATOR,
        },
        Check::Kind {
            parameter: DERIVATION,
        },
    ];
    checks.extend(LIGHT_OWN.map(|parameter| Check::Requires {
        parameter,
        with: &[DERIVATION],
        message: match parameter {
            "overall_width" => {
                "`overall_width` applies only to `numerator_derivation` `light-area`"
            }
            "overall_height" => {
                "`overall_height` applies only to `numerator_derivation` `light-area`"
            }
            "light_area_table" => {
                "`light_area_table` applies only to `numerator_derivation` `light-area`"
            }
            "light_type" => "`light_type` applies only to `numerator_derivation` `light-area`",
            "light_type_path" => {
                "`light_type_path` applies only to `numerator_derivation` `light-area`"
            }
            "light_size_tolerance" => {
                "`light_size_tolerance` applies only to `numerator_derivation` `light-area`"
            }
            _ => "`frame_width` applies only to `numerator_derivation` `light-area`",
        },
    }));
    checks.extend([
        Check::Among {
            parameter: DERIVATION,
            options: &["light-area"],
            message: "numerator_derivation `{value}` is unsupported; the only one is \
                      `light-area`",
        },
        Check::Arguments {
            when: &[DERIVATION],
            value: concat!("light_area;", light_chain!()),
        },
        Check::Kind {
            parameter: "empty_numerator_finding",
        },
        Check::Kind {
            parameter: "denominator_property",
        },
        Check::Choice {
            parameter: "measure",
            options: &["footprint", "facade"],
        },
        Check::Choice {
            parameter: "numerator_measure",
            options: &["footprint", "facade"],
        },
        Check::Choice {
            parameter: "denominator_measure",
            options: &["footprint", "facade"],
        },
        Check::Exclusive {
            one: &["measure"],
            other: &["numerator_measure", "denominator_measure"],
            message: "`measure` applies to both sides; declare it or `numerator_measure` and \
                      `denominator_measure`, not both",
        },
        Check::Traversal {
            with: &[],
            message: "",
        },
        Check::Excludes {
            when: DERIVATION,
            parameters: &["measure", "numerator_measure", "denominator_measure"],
            value: "facade",
            message: "a `facade` measure does not combine with `numerator_derivation` \
                      `light-area`",
        },
    ]);
    checks
}

const FACADE: Condition = Condition::Equals {
    parameter: "measure",
    value: "facade",
};
const NUMERATOR_FACADE: Condition = Condition::Equals {
    parameter: "numerator_measure",
    value: "facade",
};
const DENOMINATOR_FACADE: Condition = Condition::Equals {
    parameter: "denominator_measure",
    value: "facade",
};
const STATED: Condition = Condition::Not {
    condition: &Condition::Zero {
        value: "stated_count",
    },
};
const TABLED: Condition = Condition::Not {
    condition: &Condition::Zero {
        value: "table_count",
    },
};
const FRAMED: Condition = Condition::Not {
    condition: &Condition::Zero {
        value: "frame_count",
    },
};

fn text(name: &'static str, when: Option<Condition>, text: &'static str) -> Text {
    Text { name, when, text }
}

/// The message parts: what each side measures, and the light-area steps
/// that gave the numerator.
fn texts() -> Vec<Text> {
    vec![
        text("noun", Some(FACADE), "facade area"),
        text(
            "noun",
            Some(Condition::All {
                conditions: &[NUMERATOR_FACADE, DENOMINATOR_FACADE],
            }),
            "facade area",
        ),
        text("noun", Some(NUMERATOR_FACADE), "facade area to plan area"),
        text("noun", Some(DENOMINATOR_FACADE), "plan area to facade area"),
        text("noun", None, "plan area"),
        text("bottom", Some(FACADE), "facade area"),
        text("bottom", Some(DENOMINATOR_FACADE), "facade area"),
        text("bottom", None, "plan area"),
        text(
            "provenance",
            Some(Condition::All {
                conditions: &[STATED, TABLED, FRAMED],
            }),
            "; light areas: {stated_count:least} stated, {table_count:least} from the \
             light-area table, {frame_count:least} by frame allowance",
        ),
        text(
            "provenance",
            Some(Condition::All {
                conditions: &[STATED, TABLED],
            }),
            "; light areas: {stated_count:least} stated, {table_count:least} from the \
             light-area table",
        ),
        text(
            "provenance",
            Some(Condition::All {
                conditions: &[STATED, FRAMED],
            }),
            "; light areas: {stated_count:least} stated, {frame_count:least} by frame \
             allowance",
        ),
        text(
            "provenance",
            Some(Condition::All {
                conditions: &[TABLED, FRAMED],
            }),
            "; light areas: {table_count:least} from the light-area table, \
             {frame_count:least} by frame allowance",
        ),
        text(
            "provenance",
            Some(STATED),
            "; light areas: {stated_count:least} stated",
        ),
        text(
            "provenance",
            Some(TABLED),
            "; light areas: {table_count:least} from the light-area table",
        ),
        text(
            "provenance",
            Some(FRAMED),
            "; light areas: {frame_count:least} by frame allowance",
        ),
    ]
}

/// `area-ratio`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: true,
        name: "area-ratio",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "empty_numerator_finding",
            value: ScalarValue::Boolean { value: false },
        }],
        declaration: declaration(),
        services: None,
        texts: texts(),
        forms: vec![
            form(true, true),
            form(true, false),
            form(false, true),
            form(false, false),
        ],
    }
}
