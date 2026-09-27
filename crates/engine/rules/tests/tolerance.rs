//! Numeric tolerance and rounding in uniqueness keys and comparisons.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::ParameterValue;
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::{PropertyComparison, PropertyPredicate, UniqueValue};
use common::{
    Model, findings, flagged, integer, kind, number, property, rule, selector, string, unevaluated,
};

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

/// Storeys with elevations in metres.
fn storeys(elevations: &[(&str, f64)]) -> Model {
    elevations
        .iter()
        .fold(Model::default(), |model, (name, elevation)| {
            model
                .object(name, "storey")
                .value(name, "Pset", "Elevation", metres(*elevation))
        })
}

fn unique(model: Model, extra: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![("property", property(Some("Pset"), "Elevation"))];
    parameters.extend(extra);
    model.evaluate(
        &UniqueValue,
        &rule(
            "axioval:capability.unique-value",
            kind("storey"),
            parameters,
        ),
    )
}

mod unique_value {
    use super::*;

    #[test]
    fn without_a_tolerance_elevations_differing_in_the_tenth_decimal_are_distinct() {
        let evaluation = unique(storeys(&[("a", 3.0), ("b", 3.000_000_000_1)]), vec![]);
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn elevations_exactly_at_the_tolerance_are_duplicates_and_beyond_it_are_not() {
        let model = storeys(&[("a", 1.0), ("b", 1.1), ("c", 1.300_001)]);
        let evaluation = unique(model, vec![("tolerance", number(0.1))]);
        assert_eq!(
            findings(&evaluation),
            [
                (
                    "a".into(),
                    "Pset.Elevation 1 m is also used by 1 other object(s) (within tolerance 0.1)"
                        .into()
                ),
                (
                    "b".into(),
                    "Pset.Elevation 1.1 m is also used by 1 other object(s) (within tolerance 0.1)"
                        .into()
                ),
            ]
        );
    }

    #[test]
    fn a_tolerance_is_judged_pair_by_pair_not_transitively() {
        // b is near a and c; a and c are 0.2 apart and not near each other.
        let model = storeys(&[("a", 1.0), ("b", 1.1), ("c", 1.2)]);
        let evaluation = unique(model, vec![("tolerance", number(0.1))]);
        assert_eq!(flagged(&evaluation), ["a", "b", "c"]);
        let related = |object: &str| {
            let finding = evaluation
                .findings()
                .iter()
                .find(|finding| finding.object_id.local_id == object)
                .unwrap();
            let mut related: Vec<String> = finding
                .related
                .iter()
                .map(|id| id.local_id.clone())
                .collect();
            related.sort();
            (finding.message.contains("by 2 other"), related)
        };
        assert_eq!(related("a"), (false, vec!["b".into()]));
        assert_eq!(related("b"), (true, vec!["a".into(), "c".into()]));
        assert_eq!(related("c"), (false, vec!["b".into()]));
    }

    #[test]
    fn a_relative_tolerance_includes_its_boundary() {
        let model = storeys(&[("a", 3.0), ("b", 4.0), ("c", 5.5)]);
        let evaluation = unique(model, vec![("relative_tolerance", number(0.25))]);
        // |3 - 4| = 0.25 * 4 exactly; |4 - 5.5| = 1.5 > 0.25 * 5.5.
        assert_eq!(flagged(&evaluation), ["a", "b"]);
        assert!(
            evaluation.findings()[0]
                .message
                .ends_with("(within relative tolerance 0.25)")
        );
    }

    #[test]
    fn rounding_to_decimals_forms_classes_rounded_half_away_from_zero() {
        let model = storeys(&[("a", 2.345), ("b", 2.35), ("c", 2.344_9), ("d", 2.34)]);
        let evaluation = unique(model, vec![("decimals", integer(2))]);
        // a and b round to 2.35, c and d to 2.34.
        assert_eq!(flagged(&evaluation), ["a", "b", "c", "d"]);
        let a = &evaluation.findings()[0];
        assert!(
            a.message.ends_with("(rounded to 2 decimal(s))"),
            "{}",
            a.message
        );
        assert_eq!(a.related.len(), 1);
    }

    #[test]
    fn a_quantity_never_matches_a_bare_number_under_a_tolerance() {
        let model = storeys(&[("a", 3.0)]).object("b", "storey").value(
            "b",
            "Pset",
            "Elevation",
            PropertyValue::Decimal(3.0),
        );
        let evaluation = unique(model, vec![("tolerance", number(0.1))]);
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn integers_and_decimals_share_one_domain() {
        let model = Model::default()
            .object("a", "storey")
            .object("b", "storey")
            .value("a", "Pset", "Elevation", PropertyValue::Integer(3))
            .value("b", "Pset", "Elevation", PropertyValue::Decimal(3.05));
        let evaluation = unique(model, vec![("tolerance", number(0.1))]);
        assert_eq!(flagged(&evaluation), ["a", "b"]);
    }

    #[test]
    fn text_is_unaffected_by_a_tolerance() {
        let model = Model::default()
            .object("a", "storey")
            .object("b", "storey")
            .text("a", "Pset", "Elevation", "3.0")
            .text("b", "Pset", "Elevation", "3.05");
        let evaluation = unique(model, vec![("tolerance", number(0.1))]);
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn invalid_tolerances_are_declaration_errors() {
        for extra in [
            vec![("tolerance", number(-0.1))],
            vec![("relative_tolerance", number(1.0))],
            vec![("decimals", integer(-1))],
            vec![("decimals", integer(16))],
            vec![("decimals", integer(2)), ("tolerance", number(0.1))],
            vec![("tolerance", string("0.1"))],
        ] {
            let evaluation = unique(storeys(&[("a", 1.0), ("b", 1.0)]), extra);
            assert!(evaluation.findings().is_empty());
            assert_eq!(
                unevaluated(&evaluation),
                [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
            );
        }
    }
}

mod property_predicate {
    use super::*;

    fn check(operator: &str, extra: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
        let model = Model::default()
            .object("a", "slab")
            .object("b", "slab")
            .object("c", "slab")
            .value("a", "Pset", "Area", PropertyValue::Decimal(10.0))
            .value("b", "Pset", "Area", PropertyValue::Decimal(10.5))
            .value("c", "Pset", "Area", PropertyValue::Decimal(10.500_001));
        let mut parameters = vec![
            ("property_set", string("Pset")),
            ("property", string("Area")),
            ("operator", string(operator)),
        ];
        parameters.extend(extra);
        model.evaluate(
            &PropertyPredicate,
            &rule(
                "axioval:capability.property-predicate",
                kind("slab"),
                parameters,
            ),
        )
    }

    #[test]
    fn equal_holds_exactly_at_the_tolerance() {
        let exact = check("equal", vec![("number", number(10.0))]);
        assert_eq!(flagged(&exact), ["b", "c"]);
        let tolerant = check(
            "equal",
            vec![("number", number(10.0)), ("tolerance", number(0.5))],
        );
        assert_eq!(
            findings(&tolerant),
            [(
                "c".into(),
                "property Pset.Area does not satisfy equal 10 (within tolerance 0.5); \
                 actual value is 10.500001"
                    .into()
            )]
        );
    }

    #[test]
    fn an_order_holds_only_beyond_the_tolerance() {
        let greater = check(
            "greater_than",
            vec![("number", number(10.0)), ("tolerance", number(0.5))],
        );
        // b is within the tolerance, so not greater.
        assert_eq!(flagged(&greater), ["a", "b"]);
        let at_most = check(
            "less_or_equal",
            vec![("number", number(10.0)), ("tolerance", number(0.5))],
        );
        assert_eq!(flagged(&at_most), ["c"]);
    }

    #[test]
    fn rounding_compares_values_as_displayed() {
        let evaluation = check(
            "equal",
            vec![("number", number(11.0)), ("decimals", integer(0))],
        );
        // 10.5 rounds half away from zero to 11.
        assert_eq!(flagged(&evaluation), ["a"]);
    }

    #[test]
    fn a_tolerance_on_a_text_target_is_a_declaration_error() {
        let evaluation = check(
            "equal",
            vec![("text", string("10")), ("tolerance", number(0.5))],
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

mod property_comparison {
    use super::*;

    /// Room areas compared with the sum of their parts.
    fn rooms() -> Model {
        Model::default()
            .object("r1", "room")
            .object("r2", "room")
            .object("p1", "part")
            .object("p2", "part")
            .object("p3", "part")
            .edge("contains", "r1", "p1")
            .edge("contains", "r1", "p2")
            .edge("contains", "r2", "p3")
            .value("r1", "Pset", "Area", PropertyValue::Decimal(20.0))
            .value("r2", "Pset", "Area", PropertyValue::Decimal(10.0))
            .value("p1", "Pset", "Area", PropertyValue::Decimal(10.0))
            .value("p2", "Pset", "Area", PropertyValue::Decimal(10.8))
            .value("p3", "Pset", "Area", PropertyValue::Decimal(9.0))
    }

    fn evaluate(parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
        rooms().evaluate(
            &PropertyComparison,
            &rule(
                "axioval:capability.property-comparison",
                kind("room"),
                parameters,
            ),
        )
    }

    fn compare(
        quantifier: &str,
        target: (&'static str, ParameterValue),
        extra: Vec<(&'static str, ParameterValue)>,
    ) -> CapabilityEvaluation {
        let mut parameters = vec![
            ("compared_selector", selector(kind("part"))),
            ("compared_property", property(Some("Pset"), "Area")),
            target,
            ("operator", string("equals")),
            ("factor", number(1.0)),
            ("component_mode", string("related")),
            ("relationship", string("contains")),
            ("quantifier", string(quantifier)),
        ];
        parameters.extend(extra);
        evaluate(parameters)
    }

    fn own_area() -> (&'static str, ParameterValue) {
        ("target_property", property(Some("Pset"), "Area"))
    }

    #[test]
    fn a_sum_is_equal_within_a_relative_tolerance_including_its_boundary() {
        // r1: 20.8 against 20 is 0.8 apart, beyond 2% of 20.8 (0.416).
        // r2: 9 against 10 is exactly 10% of 10.
        let evaluation = compare("sum", own_area(), vec![("relative_tolerance", number(0.1))]);
        assert!(evaluation.findings().is_empty());
        let strict = compare(
            "sum",
            own_area(),
            vec![("relative_tolerance", number(0.02))],
        );
        assert_eq!(
            findings(&strict),
            [
                (
                    "r1".into(),
                    "sum of compared values is 20.8 and is not equals 20 \
                     (within relative tolerance 0.02)"
                        .into()
                ),
                (
                    "r2".into(),
                    "sum of compared values is 9 and is not equals 10 \
                     (within relative tolerance 0.02)"
                        .into()
                ),
            ]
        );
    }

    #[test]
    fn each_candidate_is_compared_within_an_absolute_tolerance() {
        let target = || ("target_number", number(10.0));
        // 10.8 and 9 are within 1 of 10, 9 exactly at the boundary.
        let within = compare("each", target(), vec![("tolerance", number(1.0))]);
        assert!(within.findings().is_empty());
        let beyond = compare("each", target(), vec![("tolerance", number(0.5))]);
        assert_eq!(
            findings(&beyond),
            [
                (
                    "r1".into(),
                    "candidate test:model/p2 does not satisfy comparison (within tolerance 0.5)"
                        .into()
                ),
                (
                    "r2".into(),
                    "candidate test:model/p3 does not satisfy comparison (within tolerance 0.5)"
                        .into()
                ),
            ]
        );
    }

    #[test]
    fn a_tolerance_on_a_text_target_is_a_declaration_error() {
        let evaluation = compare(
            "each",
            ("target_text", string("10")),
            vec![("decimals", integer(1))],
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
