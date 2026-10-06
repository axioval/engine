//! `light-well`: stacked spaces contiguous, with a section large and wide
//! enough for the well's height.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ElevationInterval, PlanLength, PlanSection, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::LightWell;
use common::{Model, findings, id, kind, number, rule, source, strings, unevaluated};

const ID: &str = "axioval:capability.light-well";

/// A closed interval, lower bound first.
type Span = (f64, f64);

/// Elevations per member, and one section answer for any stack.
struct Well {
    extents: BTreeMap<ObjectId, (Span, Span)>,
    /// `(area, width)`; an area of zero is an empty section.
    section: (Span, Span),
}

fn exact((lower, upper): (f64, f64)) -> bool {
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    exact
}

impl VerticalExtentService for Well {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (bottom, top) = self
            .extents
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom.0, bottom.1)?,
            ElevationInterval::try_new(top.0, top.1)?,
            Evidence {
                source: source(),
                locator: format!("extent:{object}"),
                exact: exact(*bottom) && exact(*top),
            },
        )
    }
}

impl PlanSpanService for Well {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_section(&self, objects: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
        let (area, width) = self.section;
        let evidence = |locator: &str, exact| Evidence {
            source: source(),
            locator: locator.into(),
            exact,
        };
        let sides = (area.1 > 0.0).then(|| {
            (
                PlanLength::try_new(width.0, width.1, evidence("width", exact(width))).unwrap(),
                PlanLength::try_new(10.0, 10.0, evidence("length", true)).unwrap(),
            )
        });
        PlanSection::try_new(objects.to_vec(), area, sides, evidence("area", exact(area)))
    }
}

fn row(cells: &[(&str, f64)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), number(*value)))
        .collect()
}

/// Up to 7 m high: 4 m² and 1.5 m wide; any higher: 8 m² and 2.5 m wide.
fn requirements() -> ParameterValue {
    ParameterValue::Table {
        value: vec![
            row(&[
                ("maximum_height_metres", 7.0),
                ("minimum_area_square_metres", 4.0),
                ("minimum_width_metres", 1.5),
            ]),
            row(&[
                ("minimum_area_square_metres", 8.0),
                ("minimum_width_metres", 2.5),
            ]),
        ],
    }
}

/// Zone `well` groups the spaces given as `(local, bottom, top)`.
fn evaluate(
    members: &[(&str, f64, f64)],
    section: (Span, Span),
    tolerance: Option<f64>,
) -> axioval_engine::CapabilityEvaluation {
    let mut model = Model::default().object("well", "zone");
    let mut extents = BTreeMap::new();
    for (local, bottom, top) in members {
        model = model.object(local, "space").edge("groups", "well", local);
        extents.insert(id(local), ((*bottom, *bottom), (*top, *top)));
    }
    let service = Arc::new(Well { extents, section });
    let mut parameters = vec![
        ("member_path", strings(&["groups:forward"])),
        ("requirements", requirements()),
    ];
    if let Some(tolerance) = tolerance {
        parameters.push(("gap_tolerance_metres", number(tolerance)));
    }
    let rule = rule(ID, kind("zone"), parameters);
    model.evaluate_measured(&HELD, &rule, move |services| {
        services
            .register(VerticalExtentServiceHandle::new(service.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(service.clone()))
            .unwrap();
    })
}

/// `light-well` as it runs, held to the implementation it replaced on every
/// evaluation.
static HELD: common::Held = common::Held(&LightWell, &axioval_rules::reference::LightWell);

fn messages(outcome: &axioval_engine::CapabilityEvaluation) -> Vec<String> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect()
}

fn point(value: f64) -> (f64, f64) {
    (value, value)
}

const STACK: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.0, 6.0), ("f2", 6.0, 9.0)];

#[test]
fn a_contiguous_well_large_enough_for_its_height_passes() {
    let outcome = evaluate(STACK, (point(9.0), point(2.8)), None);
    assert!(findings(&outcome).is_empty(), "{:?}", findings(&outcome));
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// Two storeys (6 m) select the first row; three (9 m) the second.
#[test]
fn the_row_follows_the_height_of_the_well() {
    let low = evaluate(&STACK[..2], (point(5.0), point(2.0)), None);
    assert!(findings(&low).is_empty(), "{:?}", findings(&low));
    let high = evaluate(STACK, (point(5.0), point(2.0)), None);
    let found = findings(&high);
    assert_eq!(
        found,
        vec![
            (
                "well".to_owned(),
                "the well's section area is 5 m²; row 1 requires at least 8 m² for a well 9 m \
                 high"
                    .to_owned()
            ),
            (
                "well".to_owned(),
                "the well's width is 2 m; row 1 requires at least 2.5 m for a well 9 m high"
                    .to_owned()
            ),
        ]
    );
    assert_eq!(
        high.findings()[0].related,
        vec![id("f1"), id("f2"), id("g")]
    );
}

#[test]
fn a_vertical_gap_or_no_shared_section_breaks_contiguity() {
    let gap: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f2", 6.0, 9.0)];
    let found = findings(&evaluate(gap, (point(9.0), point(2.8)), None));
    assert_eq!(
        found,
        vec![(
            "well".to_owned(),
            "test:model/f2 starts 3 m above the top of test:model/g, so the well is not \
             contiguous"
                .to_owned()
        )]
    );
    // Within the tolerance a gap is contiguous.
    let slight: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.02, 6.0)];
    assert!(findings(&evaluate(slight, (point(9.0), point(2.8)), Some(0.05))).is_empty());
    assert_eq!(
        findings(&evaluate(slight, (point(9.0), point(2.8)), None)).len(),
        1
    );

    let apart = findings(&evaluate(STACK, (point(0.0), point(0.0)), None));
    assert_eq!(
        apart,
        vec![(
            "well".to_owned(),
            "the 3 stacked spaces share no plan section, so the well is not contiguous".to_owned()
        )]
    );
}

#[test]
fn straddling_values_decide_nothing() {
    let outcome = evaluate(STACK, ((7.0, 9.0), (2.4, 2.6)), None);
    assert!(findings(&outcome).is_empty());
    assert_eq!(
        messages(&outcome),
        vec![
            "the well's section area is between 7 and 9 m²; row 1 requires at least 8 m², \
             undecided",
            "the well's width is between 2.4 m and 2.6 m; row 1 requires at least 2.5 m, \
             undecided",
        ]
    );
    // A well whose height straddles a row's maximum, and a gap straddling
    // the tolerance.
    let straddling: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.0, 7.0)];
    let mut model = Model::default().object("well", "zone");
    let mut extents = BTreeMap::new();
    for (local, bottom, top) in straddling {
        model = model.object(local, "space").edge("groups", "well", local);
        extents.insert(
            id(local),
            ((*bottom, *bottom + 0.04), (*top - 0.1, *top + 0.1)),
        );
    }
    let service = Arc::new(Well {
        extents,
        section: (point(9.0), point(2.8)),
    });
    let rule = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
            ("gap_tolerance_metres", number(0.02)),
        ],
    );
    let outcome = model.evaluate_measured(&HELD, &rule, move |services| {
        services
            .register(VerticalExtentServiceHandle::new(service.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(service.clone()))
            .unwrap();
    });
    assert_eq!(
        messages(&outcome),
        vec![
            "the gap between test:model/g and test:model/f1 is between 0 m and 0.14 m",
            "which row applies to a well between 6.86 m and 7.1 m high is undecided",
        ]
    );
}

#[test]
fn a_well_without_members_or_with_an_unmeasured_one_is_not_evaluated() {
    // The zone groups nothing; another zone groups the only space.
    let empty = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
        ],
    );
    let outcome = Model::default()
        .object("well", "zone")
        .object("other", "court")
        .object("g", "space")
        .edge("groups", "other", "g")
        .evaluate_measured(&HELD, &empty, |_| {});
    assert_eq!(
        unevaluated(&outcome),
        vec![("well".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        messages(&outcome),
        vec!["test:model/well reaches no space through groups"]
    );
    let rule = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
        ],
    );
    let outcome = Model::default()
        .object("well", "zone")
        .object("g", "space")
        .edge("groups", "well", "g")
        .evaluate_measured(&HELD, &rule, |_| {});
    assert_eq!(
        unevaluated(&outcome),
        vec![("well".to_owned(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        messages(&outcome),
        vec!["vertical-extent service is not registered"]
    );
}

/// The well's judgement under [`requirements`] as an expression rule over
/// its measured gap, section and height, with gaps up to `tolerance`.
fn as_expression(tolerance: f64) -> axioval_engine::CompiledRule {
    use serde_json::{Value, json};
    let measured = |name: &str| {
        json!({"kind": "property", "propertySet": "axioval:measured",
            "property": format!("{name};members=groups:forward")})
    };
    let quantity = |value: f64, unit: &str| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}});
    let compare = |operator: &str, left: Value, right: Value| json!({"kind": "compare", "operator": operator, "left": left, "right": right});
    let row = |area: f64, width: f64| {
        json!({"kind": "and", "operands": [
            compare("greaterThanOrEquals", measured("well_section_area"), quantity(area, "m2")),
            {"kind": "implies",
             "antecedent": {"kind": "isDefined", "operand": measured("well_section_width")},
             "consequent": compare("greaterThanOrEquals", measured("well_section_width"), quantity(width, "m"))}]})
    };
    let requirement = json!({"kind": "and", "operands": [
        compare("lessThanOrEquals", measured("well_gap"), quantity(tolerance, "m")),
        compare("greaterThan", measured("well_section_area"), quantity(0.0, "m2")),
        {"kind": "if", "branches": [{
            "when": compare("lessThanOrEquals", measured("well_height"), quantity(7.0, "m")),
            "then": row(4.0, 1.5)}],
         "else": row(8.0, 2.5)}]});
    rule(
        "axioval:capability.expression",
        kind("zone"),
        vec![(
            "requirement",
            ParameterValue::Expression {
                value: serde_json::from_value(requirement).unwrap(),
            },
        )],
    )
}

/// A well grouping nothing, or measured without the services, is left open
/// by the expression for the same reason as by `light-well`.
#[test]
fn an_empty_or_unmeasured_well_is_open_to_the_expression_too() {
    let empty = || {
        Model::default()
            .object("well", "zone")
            .object("other", "court")
            .object("g", "space")
            .edge("groups", "other", "g")
    };
    let unmeasured = || {
        Model::default()
            .object("well", "zone")
            .object("g", "space")
            .edge("groups", "well", "g")
    };
    let capability = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
        ],
    );
    for model in [empty, unmeasured] {
        let expected = model().evaluate_measured(&HELD, &capability, |_| {});
        let outcome = model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &as_expression(0.0),
            |_| {},
        );
        let parity =
            axioval_rules::parity::compare_evaluations((ID, &expected), ("expression", &outcome));
        assert!(parity.holds(), "{}", parity.diff());
        assert_eq!(parity.open, 1);
    }
}

/// The well's judgement as an expression over its measured section, height
/// and gaps flags and leaves open the same wells as `light-well`.
#[test]
#[allow(clippy::type_complexity)]
fn the_well_as_an_expression_over_its_values_reaches_the_verdicts() {
    let slight: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.02, 6.0)];
    let gap: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f2", 6.0, 9.0)];
    let cases: Vec<(&[(&str, f64, f64)], (Span, Span), Option<f64>)> = vec![
        (STACK, (point(9.0), point(2.8)), None),
        (&STACK[..2], (point(5.0), point(2.0)), None),
        (STACK, (point(5.0), point(2.0)), None),
        (gap, (point(9.0), point(2.8)), None),
        (slight, (point(9.0), point(2.8)), Some(0.05)),
        (slight, (point(9.0), point(2.8)), None),
        (STACK, (point(0.0), point(0.0)), None),
        (STACK, ((7.0, 9.0), (2.4, 2.6)), None),
    ];
    for (index, (members, section, tolerance)) in cases.into_iter().enumerate() {
        let expected = evaluate(members, section, tolerance);
        let mut model = Model::default().object("well", "zone");
        let mut extents = BTreeMap::new();
        for (local, bottom, top) in members {
            model = model.object(local, "space").edge("groups", "well", local);
            extents.insert(id(local), ((*bottom, *bottom), (*top, *top)));
        }
        let service = Arc::new(Well { extents, section });
        let rule = as_expression(tolerance.unwrap_or(0.0));
        let outcome =
            model.evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                services
                    .register(VerticalExtentServiceHandle::new(service.clone()))
                    .unwrap();
                services
                    .register(PlanSpanServiceHandle::new(service.clone()))
                    .unwrap();
            });
        // The capability reports each failed bound, the conjunction one
        // finding (divergence D1).
        let parity = axioval_rules::parity::Parity::outcomes()
            .uncounted()
            .compare_evaluations((ID, &expected), ("expression", &outcome));
        assert!(parity.holds(), "case {index}:\n{}", parity.diff());
    }
}

/// Generated wells of random stacks (exact and inexact extents, gaps and
/// overlaps, some spaces unmeasured), sections (empty, inexact, without a
/// width) and requirement rows, each held to the implementation the
/// template replaced.
mod generated {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axioval_engine::{PlanSpanServiceHandle, VerticalExtentServiceHandle};
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::{
        HELD, ID, Model, ParameterValue, Span, Well, id, kind, number, row, rule, strings,
    };

    /// A value in centimetres, exact or widened by a few.
    fn span(range: std::ops::Range<u32>) -> impl Strategy<Value = Span> {
        (range, 0u32..4).prop_map(|(low, slack)| {
            let low = f64::from(low) / 100.0;
            (low, low + f64::from(slack) / 100.0)
        })
    }

    /// A space: its bottom and its height above it, or unmeasured.
    fn space() -> impl Strategy<Value = Option<(Span, Span)>> {
        prop_oneof![
            8 => (span(0..900), span(50..400)).prop_map(|(bottom, height)| {
                Some((bottom, (bottom.0 + height.0, bottom.1 + height.1)))
            }),
            1 => Just(None),
        ]
    }

    fn requirement() -> impl Strategy<Value = Vec<(&'static str, f64)>> {
        (
            proptest::option::of(100u32..1200),
            proptest::option::of(0u32..2000),
            proptest::option::of(0u32..400),
        )
            .prop_filter_map(
                "a row requires an area or a width",
                |(up_to, area, width)| {
                    if area.is_none() && width.is_none() {
                        return None;
                    }
                    let mut cells = Vec::new();
                    if let Some(up_to) = up_to {
                        cells.push(("maximum_height_metres", f64::from(up_to) / 100.0));
                    }
                    if let Some(area) = area {
                        cells.push(("minimum_area_square_metres", f64::from(area) / 100.0));
                    }
                    if let Some(width) = width {
                        cells.push(("minimum_width_metres", f64::from(width) / 100.0));
                    }
                    Some(cells)
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_wells_hold_parity(
            spaces in vec(space(), 0..4),
            area in prop_oneof![1 => Just((0.0, 0.0)), 4 => span(0..2000)],
            width in span(0..400),
            rows in vec(requirement(), 1..4),
            tolerance in proptest::option::of(0u32..20),
        ) {
            let mut model = Model::default().object("well", "zone");
            let mut extents = BTreeMap::new();
            for (index, space) in spaces.iter().enumerate() {
                let local = format!("s{index}");
                model = model.object(&local, "space").edge("groups", "well", &local);
                if let Some(extent) = space {
                    extents.insert(id(&local), *extent);
                }
            }
            let service = Arc::new(Well { extents, section: (area, width) });
            let mut parameters = vec![
                ("member_path", strings(&["groups:forward"])),
                (
                    "requirements",
                    ParameterValue::Table {
                        value: rows.iter().map(|cells| row(cells)).collect(),
                    },
                ),
            ];
            if let Some(tolerance) = tolerance {
                parameters.push(("gap_tolerance_metres", number(f64::from(tolerance) / 100.0)));
            }
            let rule = rule(ID, kind("zone"), parameters);
            // `HELD` holds the template to the reference.
            model.evaluate_measured(&HELD, &rule, move |services| {
                services
                    .register(VerticalExtentServiceHandle::new(service.clone()))
                    .unwrap();
                services
                    .register(PlanSpanServiceHandle::new(service.clone()))
                    .unwrap();
            });
        }
    }
}
