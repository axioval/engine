//! `external-wall-validation` as a template: each derivation the rule lists
//! measured once per rule, then each source where the rule selects an
//! object judged by whether the model declares any of them external, and
//! each selected object by its declaration against each envelope and by
//! the two envelopes against each other.

use axioval_engine::template::{
    Applies, Band, Check, Condition, Decision, Form, FormCheck, Grading, Needed, Once, Operand,
    Refusals, ScopeMessages, ScopeSources, Scopes, Service, Services, Template, TemplateValue,
    Term,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::Severity;
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.external-wall-validation";

const DERIVATIONS: &str = "derivations";
const BOUNDING_SELECTOR: &str = "bounding_selector";
const GROUP_SELECTOR: &str = "gross_area_group_selector";
const GROUP_PATH: &str = "gross_area_group_path";

/// The derivations, as the rule lists them.
const OPTIONS: &[&str] = &["all-spaces", "gross-area-groups"];

/// What bounds each derivation, as the rule names it.
macro_rules! bounded {
    ($name:literal, "all-spaces") => {
        concat!($name, ";derivation=all-spaces;bounding=@bounding_selector")
    };
    ($name:literal, "gross-area-groups") => {
        concat!(
            $name,
            ";derivation=gross-area-groups;groups=@gross_area_group_selector;\
             group_path=@gross_area_group_path"
        )
    };
}

/// How many selected objects of a source the model declares external.
const DECLARED: &str = "external_declarations;derivations=@derivations;\
     bounding=@bounding_selector;groups=@gross_area_group_selector;\
     group_path=@gross_area_group_path;objects=@selection";

/// The capability's parameter descriptor.
fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required(DERIVATIONS, ParameterType::StringList),
        ParameterDescriptor::optional(BOUNDING_SELECTOR, ParameterType::Selector),
        ParameterDescriptor::optional(GROUP_SELECTOR, ParameterType::Selector),
        ParameterDescriptor::optional(GROUP_PATH, ParameterType::StringList),
    ]
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
    }
}

/// A severity the capability gave its findings whatever the rule's.
fn severity(severity: Severity) -> Grading {
    Grading {
        values: Vec::new(),
        derived: Vec::new(),
        bands: vec![Band {
            severity,
            when: None,
        }],
        undecided: Vec::new(),
    }
}

/// Where a check of a derivation applies: the rule lists it and it was
/// measured.
fn derived(condition: &'static Condition) -> Applies {
    Applies {
        when: &[],
        any: &[],
        condition: Some(*condition),
    }
}

const ALL_SPACES: Condition = Condition::All {
    conditions: &[
        Condition::Lists {
            parameter: DERIVATIONS,
            value: "all-spaces",
        },
        Condition::Measured {
            value: "all_spaces",
        },
    ],
};

const GROSS_AREA_GROUPS: Condition = Condition::All {
    conditions: &[
        Condition::Lists {
            parameter: DERIVATIONS,
            value: "gross-area-groups",
        },
        Condition::Measured {
            value: "gross_area_groups",
        },
    ],
};

const BOTH: Condition = Condition::All {
    conditions: &[ALL_SPACES, GROSS_AREA_GROUPS],
};

/// Either derivation measured.
const EITHER: Condition = Condition::Not {
    condition: &Condition::All {
        conditions: &[
            Condition::Not {
                condition: &Condition::Measured {
                    value: "all_spaces",
                },
            },
            Condition::Not {
                condition: &Condition::Measured {
                    value: "gross_area_groups",
                },
            },
        ],
    },
};

/// The object bounds either derivation: the inside of that envelope,
/// never compared.
const BOUNDS: Condition = Condition::Not {
    condition: &Condition::All {
        conditions: &[
            Condition::Not {
                condition: &Condition::Above {
                    value: "bounds_all",
                    than: 0.5,
                },
            },
            Condition::Not {
                condition: &Condition::Above {
                    value: "bounds_gross",
                    than: 0.5,
                },
            },
        ],
    },
};

/// A check of one object of the two values compared, failing where
/// `value` is below `minimum` or above `maximum`.
#[allow(clippy::too_many_arguments)]
fn check(
    values: Vec<TemplateValue>,
    value: &'static str,
    (minimum, maximum): (Option<&'static str>, Option<&'static str>),
    fail: &'static str,
    applies: Option<Applies>,
    unless: Option<Condition>,
    quiet: bool,
) -> FormCheck {
    let bound = |name: &'static str| vec![Term::plus(Operand::Value(name))];
    FormCheck {
        values,
        decision: Decision::Within {
            value,
            minimum: minimum.map(bound),
            maximum: maximum.map(bound),
            rounding: Vec::new(),
        },
        fail,
        undecided: fail,
        related: None,
        grading: Some(severity(Severity::Warning)),
        applies,
        unless,
        quiet,
        derived: Vec::new(),
        ungraded: false,
    }
}

/// Each object's declaration against one derivation: declared external but
/// not on the envelope; on it but not declared, unless its source declares
/// nothing external (whose own finding stands for it). An object whose
/// declaration is unknown is open once.
fn declaration(
    (declared, on): (&'static str, &'static str),
    (declared_read, on_read): (&'static str, &'static str),
    condition: &'static Condition,
    (not_on, not_declared): (&'static str, &'static str),
) -> [FormCheck; 2] {
    [
        check(
            vec![measured(declared, declared_read), measured(on, on_read)],
            on,
            (Some(declared), None),
            not_on,
            Some(derived(condition)),
            None,
            false,
        ),
        check(
            vec![
                measured(declared, declared_read),
                measured(on, on_read),
                measured("source", DECLARED),
            ],
            on,
            (None, Some(declared)),
            not_declared,
            Some(derived(condition)),
            Some(Condition::Below {
                value: "source",
                than: 0.5,
            }),
            true,
        ),
    ]
}

/// The two envelopes against each other, whatever the object declares.
fn envelopes() -> [FormCheck; 2] {
    let values = || {
        vec![
            measured("on_all", bounded!("on_envelope", "all-spaces")),
            measured("on_gross", bounded!("on_envelope", "gross-area-groups")),
            measured("bounds_all", bounded!("bounds_envelope", "all-spaces")),
            measured(
                "bounds_gross",
                bounded!("bounds_envelope", "gross-area-groups"),
            ),
        ]
    };
    [
        check(
            values(),
            "on_all",
            (Some("on_gross"), None),
            "on the gross-area-groups envelope but not on the all-spaces envelope",
            Some(derived(&BOTH)),
            Some(BOUNDS),
            true,
        ),
        check(
            values(),
            "on_all",
            (None, Some("on_gross")),
            "on the all-spaces envelope but not on the gross-area-groups envelope",
            Some(derived(&BOTH)),
            Some(BOUNDS),
            true,
        ),
    ]
}

/// The template.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "external-wall-validation",
        refusals: Refusals::Selected,
        defaults: Vec::new(),
        declaration: vec![
            Check::Required {
                parameter: DERIVATIONS,
            },
            Check::DeclaresListed {
                parameters: &[DERIVATIONS],
                message: "`derivations` names no derivation",
            },
            Check::Listed {
                parameter: DERIVATIONS,
                options: OPTIONS,
                unknown: "envelope derivation `{value}` must be 'all-spaces' or \
                          'gross-area-groups'",
                repeated: "`derivations` lists `{value}` twice",
            },
            Check::Kind {
                parameter: BOUNDING_SELECTOR,
            },
            Check::Kind {
                parameter: GROUP_SELECTOR,
            },
            Check::Kind {
                parameter: GROUP_PATH,
            },
            Check::Together {
                parameters: &[GROUP_SELECTOR, GROUP_PATH],
                message: "declare `gross_area_group_selector` and `gross_area_group_path` \
                          together",
            },
            Check::Path {
                parameter: GROUP_PATH,
            },
            Check::ListedNeeds {
                parameter: DERIVATIONS,
                needs: &[
                    Needed {
                        value: "all-spaces",
                        parameters: &[BOUNDING_SELECTOR],
                        message: "the all-spaces derivation needs `bounding_selector`",
                    },
                    Needed {
                        value: "gross-area-groups",
                        parameters: &[GROUP_SELECTOR, GROUP_PATH],
                        message: "the gross-area-groups derivation needs \
                                  `gross_area_group_selector` and `gross_area_group_path`",
                    },
                ],
            },
        ],
        services: Some(Services {
            needs: vec![Service::EnvelopeMembership],
            message: "envelope-membership service is not registered",
        }),
        texts: Vec::new(),
        forms: vec![form()],
    }
}

#[allow(clippy::too_many_lines)]
fn form() -> Form {
    let [not_on_all, not_declared_all] = declaration(
        ("declared_all", "on_all"),
        (
            bounded!("declared_external", "all-spaces"),
            bounded!("on_envelope", "all-spaces"),
        ),
        &ALL_SPACES,
        (
            "declared external but not on the all-spaces envelope",
            "on the all-spaces envelope but not declared external",
        ),
    );
    let [not_on_gross, not_declared_gross] = declaration(
        ("declared_gross", "on_gross"),
        (
            bounded!("declared_external", "gross-area-groups"),
            bounded!("on_envelope", "gross-area-groups"),
        ),
        &GROSS_AREA_GROUPS,
        (
            "declared external but not on the gross-area-groups envelope",
            "on the gross-area-groups envelope but not declared external",
        ),
    );
    let [gross_only, all_only] = envelopes();
    Form {
        when: &[],
        values: vec![
            measured("declared", DECLARED),
            TemplateValue {
                name: "one",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: None,
                },
                expect: None,
                absent: None,
                mismatch: None,
            },
        ],
        decision: Decision::Within {
            value: "declared",
            minimum: Some(vec![Term::plus(Operand::Value("one"))]),
            maximum: None,
            rounding: Vec::new(),
        },
        fail: "no selected object is declared external: the model declares no envelope",
        undecided: "no selected object is declared external, but {declared:upper0} state neither \
                    external nor internal or could not be measured",
        members: None,
        table: None,
        scope: Some(Scopes {
            // No parameter: each source is a scope.
            across: "",
            disciplines: None,
            sources: ScopeSources::Occupied,
            needs: Some(EITHER),
            messages: ScopeMessages {
                source: "in source `{source}`",
                project: "in the project",
                no_source: "external-wall-validation: the run checks no source",
                no_discipline: "external-wall-validation: no source plays {disciplines}",
                undeclared: "external-wall-validation: `{source}` declares no discipline",
                undeclared_member: "external-wall-validation: `{source}` declares no discipline",
                unlisted: "external-wall-validation: the objects of `{source}` cannot be \
                           listed: {why}",
                no_disciplines: "external-wall-validation: source disciplines are not available",
            },
        }),
        derived: Vec::new(),
        // A source finding relates no object.
        related: Some("declared"),
        checks: vec![
            not_on_all,
            not_declared_all,
            not_on_gross,
            not_declared_gross,
            gross_only,
            all_only,
        ],
        unless: Vec::new(),
        grading: Some(severity(Severity::Error)),
        once: vec![
            Once {
                value: measured("all_spaces", bounded!("envelope_size", "all-spaces")),
                applies: Some(Applies {
                    when: &[],
                    any: &[],
                    condition: Some(Condition::Lists {
                        parameter: DERIVATIONS,
                        value: "all-spaces",
                    }),
                }),
                refused: "all-spaces envelope: {why}",
                required: false,
            },
            Once {
                value: measured(
                    "gross_area_groups",
                    bounded!("envelope_size", "gross-area-groups"),
                ),
                applies: Some(Applies {
                    when: &[],
                    any: &[],
                    condition: Some(Condition::Lists {
                        parameter: DERIVATIONS,
                        value: "gross-area-groups",
                    }),
                }),
                refused: "gross-area-groups envelope: {why}",
                required: false,
            },
        ],
        joined: None,
        project: Vec::new(),
    }
}
