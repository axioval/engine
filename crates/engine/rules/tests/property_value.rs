//! `axioval:capability.property-value`: lexical constraints cast to the value.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, CompletePropertyAbsenceEvidence,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty, RuleCapability, RuleContext,
    ServiceRegistry, SourceSnapshot,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, Property, PropertyValue,
    QuantityDimension, RuleId, SourceId,
};
use axioval_rules::{PropertyValueConstraint, register_builtins};

fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}

fn object() -> Object {
    Object::new(ObjectId::new(source(), "wall-1").unwrap(), "wall")
}

/// One property `P.Code`, or none; resolution is exact either way.
struct OneProperty(Option<Property>, Vec<SourceSnapshot>);
impl PropertyResolutionService for OneProperty {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        match &self.0 {
            Some(property) => Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                property.clone(),
            )?)),
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(source(), "complete native property table"),
                )
                .unwrap(),
            )),
        }
    }
}

enum Outcome {
    Meets,
    Fails(String),
    NotEvaluated(NotEvaluatedReason),
}

fn check(
    value: Option<PropertyValue>,
    data_type: Option<&str>,
    parameters: &[(&str, ParameterValue)],
) -> Outcome {
    let property = value.map(|value| {
        let property = Property::new("P", "Code", value)
            .unwrap()
            .with_evidence(Evidence::exact(source(), "native P.Code"));
        match data_type {
            Some(data_type) => property.with_data_type(data_type).unwrap(),
            None => property,
        }
    });
    let project = Project::new(vec![object()]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(PropertyResolutionServiceHandle::new(Arc::new(OneProperty(
            property,
            vec![],
        ))))
        .unwrap();
    let mut all = BTreeMap::from([(
        "property".to_owned(),
        ParameterValue::PropertyReference {
            property_set: Some("P".into()),
            property: "Code".into(),
        },
    )]);
    for (name, value) in parameters {
        all.insert((*name).to_owned(), value.clone());
    }
    let rule = CompiledRule {
        id: RuleId::new("value").unwrap(),
        capability: "axioval:capability.property-value".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: all,
    };
    outcome(&PropertyValueConstraint.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    ))
}

fn outcome(evaluation: &CapabilityEvaluation) -> Outcome {
    match (evaluation.findings(), evaluation.not_evaluated_outcomes()) {
        ([], []) => Outcome::Meets,
        ([finding], []) => {
            assert!(!finding.evidence.is_empty(), "{}", finding.message);
            Outcome::Fails(finding.message.clone())
        }
        ([], [outcome]) => Outcome::NotEvaluated(outcome.reason().clone()),
        (findings, outcomes) => panic!("{findings:?} {outcomes:?}"),
    }
}

fn values(literals: &[&str]) -> (&'static str, ParameterValue) {
    (
        "values",
        ParameterValue::StringList {
            value: literals
                .iter()
                .map(|literal| (*literal).to_owned())
                .collect(),
        },
    )
}

fn text(name: &'static str, value: &str) -> (&'static str, ParameterValue) {
    (
        name,
        ParameterValue::String {
            value: value.into(),
        },
    )
}

fn optional() -> (&'static str, ParameterValue) {
    ("optional", ParameterValue::Boolean { value: true })
}

#[allow(clippy::unnecessary_wraps)] // feeds the `Option` argument of `check`
fn string(value: &str) -> Option<PropertyValue> {
    Some(PropertyValue::String(value.into()))
}

fn meets(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Meets)
}

fn fails(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Fails(_))
}

fn invalid(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
    )
}

#[test]
fn text_compares_exactly_and_case_sensitively() {
    assert!(meets(&check(string("Bar"), None, &[values(&["Bar"])])));
    assert!(fails(&check(string("bar"), None, &[values(&["Bar"])])));
    assert!(meets(&check(string("1"), None, &[values(&["1"])])));
    assert!(meets(&check(string("B"), None, &[values(&["A", "B"])])));
}

#[test]
fn patterns_and_lengths_apply_to_text() {
    let pattern = (
        "patterns",
        ParameterValue::StringList {
            value: vec!["EI [0-9]+".into()],
        },
    );
    assert!(meets(&check(string("EI 90"), None, &[pattern.clone()])));
    assert!(fails(&check(string("REI 90"), None, &[pattern])));
    let length = ("max_length", ParameterValue::Integer { value: 3 });
    assert!(meets(&check(string("äöü"), None, &[length.clone()])));
    assert!(fails(&check(string("abcd"), None, &[length])));
    let broken = (
        "patterns",
        ParameterValue::StringList {
            value: vec!["[a-z-[aeiou]]".into()],
        },
    );
    assert!(invalid(&check(string("b"), None, &[broken])));
    assert!(invalid(&check(
        string("b"),
        None,
        &[text("min_inclusive", "1")]
    )));
}

#[test]
fn booleans_take_true_false_one_and_zero() {
    let value = Some(PropertyValue::Boolean(false));
    assert!(meets(&check(value.clone(), None, &[values(&["false"])])));
    assert!(meets(&check(value.clone(), None, &[values(&["0"])])));
    assert!(fails(&check(value.clone(), None, &[values(&["true"])])));
    assert!(invalid(&check(value, None, &[values(&["FALSE"])])));
}

#[test]
fn integers_take_integer_literals_and_numeric_bounds() {
    let value = Some(PropertyValue::Integer(42));
    assert!(meets(&check(value.clone(), None, &[values(&["42"])])));
    assert!(meets(&check(value.clone(), None, &[values(&["+42"])])));
    assert!(invalid(&check(value.clone(), None, &[values(&["42.3"])])));
    assert!(meets(&check(
        value.clone(),
        None,
        &[text("min_inclusive", "42")]
    )));
    assert!(fails(&check(
        value.clone(),
        None,
        &[text("max_exclusive", "42")]
    )));
    assert!(meets(&check(
        value.clone(),
        None,
        &[text("max_exclusive", "4.25e1")]
    )));
    assert!(invalid(&check(
        value,
        None,
        &[text("min_inclusive", "4,2")]
    )));
}

#[test]
fn decimals_equal_within_tolerance_and_bound_exactly() {
    let value = Some(PropertyValue::Decimal(42.0));
    assert!(meets(&check(value.clone(), None, &[values(&["42"])])));
    assert!(meets(&check(value.clone(), None, &[values(&["42.00001"])])));
    assert!(fails(&check(value.clone(), None, &[values(&["42.001"])])));
    assert!(meets(&check(
        Some(PropertyValue::Decimal(1234.5)),
        None,
        &[values(&["1.2345E3"])]
    )));
    // Ranges take no tolerance.
    let one = |value: f64, bound: (&'static str, ParameterValue)| {
        check(Some(PropertyValue::Decimal(value)), None, &[bound])
    };
    assert!(fails(&one(0.999_999_99, text("min_inclusive", "1.0"))));
    assert!(meets(&one(1.0, text("min_inclusive", "1.0"))));
    assert!(fails(&one(1.0, text("min_exclusive", "1.0"))));
    assert!(meets(&one(1.000_000_01, text("min_exclusive", "1.0"))));
    assert!(invalid(&check(value, None, &[values(&["42,3"])])));
}

#[test]
fn digits_bound_numbers_and_refuse_text() {
    let total = |digits| ("total_digits", ParameterValue::Integer { value: digits });
    let fraction = |digits| ("fraction_digits", ParameterValue::Integer { value: digits });
    let integer = |value| Some(PropertyValue::Integer(value));
    let decimal = |value| Some(PropertyValue::Decimal(value));
    assert!(meets(&check(integer(-999), None, &[total(3)])));
    assert!(fails(&check(integer(1000), None, &[total(3)])));
    assert!(meets(&check(integer(120), None, &[fraction(0)])));
    assert!(meets(&check(decimal(12.5), None, &[total(3), fraction(1)])));
    assert!(fails(&check(decimal(12.25), None, &[fraction(1)])));
    assert!(fails(&check(decimal(12.25), None, &[total(3)])));
    // 0.1 is not a binary double; its shortest decimal has one digit.
    assert!(meets(&check(decimal(0.1), None, &[total(1), fraction(1)])));
    assert!(invalid(&check(string("12"), None, &[total(3)])));
    assert!(invalid(&check(integer(1), None, &[total(0)])));
    assert!(invalid(&check(integer(1), None, &[fraction(-1)])));
}

#[test]
fn quantities_need_units_and_are_not_evaluated() {
    let quantity = Some(PropertyValue::Quantity {
        value: 2.5,
        dimension: QuantityDimension::Length,
    });
    assert!(matches!(
        check(quantity, None, &[values(&["2.5"])]),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    ));
}

#[test]
fn required_values_treat_absence_null_and_blank_as_missing() {
    for value in [None, Some(PropertyValue::Null), string(" ")] {
        let Outcome::Fails(message) = check(value, None, &[values(&["x"])]) else {
            panic!("must fail")
        };
        assert_eq!(message, "missing required property Code");
    }
}

#[test]
fn optional_values_pass_when_absent_or_null_and_check_what_is_there() {
    assert!(meets(&check(None, None, &[values(&["x"]), optional()])));
    assert!(meets(&check(
        Some(PropertyValue::Null),
        None,
        &[values(&["x"]), optional()]
    )));
    // An empty string is present, and not "x".
    assert!(fails(&check(
        string(""),
        None,
        &[values(&["x"]), optional()]
    )));
    assert!(meets(&check(
        string("x"),
        None,
        &[values(&["x"]), optional()]
    )));
    // A type alone is a constraint.
    assert!(meets(&check(
        None,
        None,
        &[text("data_type", "IFCLABEL"), optional()]
    )));
    assert!(fails(&check(
        string("x"),
        Some("IFCTEXT"),
        &[text("data_type", "IFCLABEL"), optional()]
    )));
}

#[test]
fn the_declared_type_is_checked_before_the_value() {
    let label = [text("data_type", "IFCLABEL"), values(&["x"])];
    assert!(meets(&check(string("x"), Some("IFCLABEL"), &label)));
    let Outcome::Fails(message) = check(string("x"), Some("IFCTEXT"), &label) else {
        panic!("must fail")
    };
    assert_eq!(message, "property Code is IFCTEXT, not IFCLABEL");
    assert!(matches!(
        check(string("x"), None, &label),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    ));
}

#[test]
fn a_rule_without_constraints_is_an_invalid_declaration() {
    assert!(invalid(&check(string("x"), None, &[])));
    assert!(invalid(&check(string("x"), None, &[optional()])));
    assert!(invalid(&check(
        string("x"),
        None,
        &[("length", ParameterValue::Integer { value: -1 })]
    )));
}

#[test]
fn the_capability_is_registered() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    assert!(registry.get("axioval:capability.property-value").is_some());
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

fn si() -> (&'static str, ParameterValue) {
    ("si_units", ParameterValue::Boolean { value: true })
}

fn quantifier(value: &str) -> (&'static str, ParameterValue) {
    text("quantifier", value)
}

#[allow(clippy::unnecessary_wraps)] // feeds the `Option` argument of `check`
fn bounded(
    lower: Option<f64>,
    upper: Option<f64>,
    set_point: Option<f64>,
) -> Option<PropertyValue> {
    let part = |value: Option<f64>| value.map(|value| Box::new(metres(value)));
    Some(PropertyValue::Bounded {
        lower: part(lower),
        upper: part(upper),
        set_point: part(set_point),
    })
}

#[test]
fn quantities_compare_in_si_when_declared() {
    let length = Some(metres(2.5));
    assert!(meets(&check(
        length.clone(),
        None,
        &[values(&["2.5"]), si()]
    )));
    assert!(fails(&check(
        length.clone(),
        None,
        &[values(&["2500"]), si()]
    )));
    assert!(meets(&check(
        length.clone(),
        None,
        &[text("max_inclusive", "3"), si()]
    )));
    assert!(fails(&check(
        length,
        None,
        &[text("min_exclusive", "2.5"), si()]
    )));
}

#[test]
fn a_list_needs_a_quantifier_and_is_judged_value_by_value() {
    let list = Some(PropertyValue::List(vec![
        PropertyValue::String("X".into()),
        PropertyValue::String("Y".into()),
    ]));
    assert!(matches!(
        check(list.clone(), None, &[values(&["Y"])]),
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidEvidence)
    ));
    assert!(meets(&check(
        list.clone(),
        None,
        &[values(&["Y"]), quantifier("any")]
    )));
    assert!(fails(&check(
        list.clone(),
        None,
        &[values(&["Z"]), quantifier("any")]
    )));
    assert!(fails(&check(
        list.clone(),
        None,
        &[values(&["Y"]), quantifier("all")]
    )));
    assert!(meets(&check(
        list.clone(),
        None,
        &[values(&["X", "Y"]), quantifier("all")]
    )));
    // A scalar under a quantifier is judged as itself.
    assert!(meets(&check(
        string("Y"),
        None,
        &[values(&["Y"]), quantifier("all")]
    )));
    // An empty list states nothing.
    assert!(fails(&check(
        Some(PropertyValue::List(Vec::new())),
        None,
        &[values(&["Y"]), quantifier("any")]
    )));
    assert!(invalid(&check(
        list,
        None,
        &[values(&["Y"]), quantifier("some")]
    )));
}

#[test]
fn a_bounded_value_is_judged_by_its_stated_values_and_its_open_ends() {
    let range = bounded(Some(1.0), Some(5.0), Some(3.0));
    for value in ["1", "3", "5"] {
        assert!(meets(&check(
            range.clone(),
            None,
            &[values(&[value]), quantifier("any"), si()]
        )));
    }
    assert!(fails(&check(
        range.clone(),
        None,
        &[values(&["2"]), quantifier("any"), si()]
    )));
    let within = [
        text("min_exclusive", "0.5"),
        text("max_inclusive", "5"),
        quantifier("all"),
        si(),
    ];
    assert!(meets(&check(range, None, &within)));
    assert!(fails(&check(
        bounded(Some(1.0), Some(6.0), None),
        None,
        &within
    )));
    // An open end has values beyond every bound on its side.
    assert!(fails(&check(bounded(None, Some(4.0), None), None, &within)));
    assert!(fails(&check(bounded(Some(3.0), None, None), None, &within)));
    // Without a bound on that side, the open end does not matter.
    assert!(meets(&check(
        bounded(Some(3.0), None, None),
        None,
        &[text("min_inclusive", "2"), quantifier("all"), si()]
    )));
}

#[test]
fn a_table_is_judged_by_its_defining_and_defined_values() {
    let table = Some(PropertyValue::Table(vec![axioval_ir::PropertyTableRow {
        defining: PropertyValue::String("X".into()),
        defined: metres(1.0),
    }]));
    assert!(meets(&check(
        table.clone(),
        None,
        &[values(&["X"]), quantifier("any")]
    )));
    // A value that cannot be judged leaves `any` open when nothing meets.
    assert!(matches!(
        check(table.clone(), None, &[values(&["Y"]), quantifier("any")]),
        Outcome::NotEvaluated(_)
    ));
    let texts = Some(PropertyValue::Table(vec![axioval_ir::PropertyTableRow {
        defining: PropertyValue::String("X".into()),
        defined: PropertyValue::String("Z".into()),
    }]));
    assert!(fails(&check(
        texts,
        None,
        &[values(&["Y"]), quantifier("any")]
    )));
    assert!(matches!(
        check(
            table.clone(),
            None,
            &[text("min_inclusive", "0"), quantifier("any")]
        ),
        Outcome::NotEvaluated(_)
    ));
    // A table whose columns declare different types reports none.
    assert!(matches!(
        check(
            table,
            None,
            &[
                text("data_type", "IFCLABEL"),
                values(&["X"]),
                quantifier("any")
            ]
        ),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    ));
}
