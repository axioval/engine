//! Storey metrics over measured areas: facade and window-to-wall ratios per
//! storey and per building, facade area per storey, and net-to-gross and
//! empty-area ratios through `area-ratio`.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, FacadeArea, FacadeAreaError, FacadeAreaService,
    FacadeAreaServiceHandle, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle,
    RuleCapability,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{
    Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension, ReportValue,
};
use axioval_rules::{AreaRatio, PlanAreaRange};
use common::{
    Model, findings, id, kind, number, property, rule, selector, source, string, strings,
    unevaluated,
};

/// Facade and plan areas per object, with an uncertainty around each.
#[derive(Default)]
struct Areas(BTreeMap<ObjectId, (f64, f64)>);

impl Areas {
    fn with(mut self, local: &str, area: f64, slack: f64) -> Self {
        self.0.insert(id(local), (area, slack));
        self
    }

    fn get(&self, object: &ObjectId) -> Option<(f64, f64, Evidence)> {
        let (area, slack) = *self.0.get(object)?;
        let mut evidence = Evidence::exact(source(), format!("area:{object}"));
        evidence.exact = slack == 0.0;
        Some(((area - slack).max(0.0), area + slack, evidence))
    }
}

impl FacadeAreaService for Areas {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let (lower, upper, evidence) = self
            .get(object)
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        FacadeArea::try_new(object.clone(), lower, upper, evidence)
    }
}

impl PlanAreaService for Areas {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (lower, upper, evidence) = self
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        PlanArea::try_new(lower, upper, evidence)
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not needed here".into()))
    }
}

fn external() -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("wall"),
            Selector::property(
                Some("Pset_WallCommon".into()),
                "IsExternal",
                ComparisonOperator::Equals,
                Some(ParameterValue::Boolean { value: true }),
            ),
        ],
    }
}

fn gross() -> Selector {
    Selector::AnyOf {
        operands: vec![external(), kind("window")],
    }
}

/// Building `b` with storeys `eg` and `og`. `eg` has external walls of
/// 24 m² and 20 m² net of 4 m² and 2 m² windows, and an internal wall;
/// `og` has an external wall of 30 m² and no window.
fn building() -> (Model, Areas) {
    let mut model = Model::default()
        .object("b", "building")
        .object("eg", "storey")
        .object("og", "storey")
        .edge("aggregates", "b", "eg")
        .edge("aggregates", "b", "og");
    for (wall, storey, is_external) in [
        ("w1", "eg", true),
        ("w2", "eg", true),
        ("w3", "eg", false),
        ("w4", "og", true),
    ] {
        model = model
            .object(wall, "wall")
            .edge("contains", storey, wall)
            .value(
                wall,
                "Pset_WallCommon",
                "IsExternal",
                PropertyValue::Boolean(is_external),
            );
    }
    for window in ["f1", "f2"] {
        model = model
            .object(window, "window")
            .edge("contains", "eg", window);
    }
    let areas = Areas::default()
        .with("w1", 24.0, 0.0)
        .with("w2", 20.0, 0.0)
        .with("w3", 9.0, 0.0)
        .with("w4", 30.0, 0.0)
        .with("f1", 4.0, 0.0)
        .with("f2", 2.0, 0.0);
    (model, areas)
}

fn run(
    model: Model,
    areas: Areas,
    capability: &dyn RuleCapability,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let areas = Arc::new(areas);
    model.evaluate_with(capability, rule, |services| {
        services
            .register(FacadeAreaServiceHandle::new(areas.clone()))
            .unwrap();
        services
            .register(PlanAreaServiceHandle::new(areas))
            .unwrap();
    })
}

fn window_to_wall(maximum: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("measure", string("facade")),
        ("numerator_selector", selector(kind("window"))),
        ("denominator_selector", selector(gross())),
        ("maximum", number(maximum)),
        ("relationship", string("contains")),
    ]
}

const RATIO: &str = "axioval:capability.area-ratio";
const RANGE: &str = "axioval:capability.plan-area";

#[test]
fn the_window_to_wall_ratio_of_each_storey_is_bounded() {
    let (model, areas) = building();
    // eg: 6 m² of windows over 44 + 6 m² of external facade.
    let evaluation = run(
        model,
        areas,
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.1)),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "eg".into(),
            "facade area ratio is 0.12 (6 m² of 50 m²); required at most 0.1".into()
        )]
    );
    // The internal wall is not in the denominator, and og, with no
    // windows, passes.
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn the_window_to_wall_ratio_of_the_building_spans_its_storeys() {
    let (model, areas) = building();
    let mut parameters = window_to_wall(0.07);
    parameters.retain(|(name, _)| *name != "relationship");
    parameters.push(("path", strings(&["aggregates:forward", "contains:forward"])));
    // 6 m² over 50 + 30 m²: within eg's 0.1, beyond the building's 0.07.
    let evaluation = run(
        model,
        areas,
        &AreaRatio,
        &rule(RATIO, kind("building"), parameters),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "b".into(),
            "facade area ratio is 0.075 (6 m² of 80 m²); required at most 0.07".into()
        )]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_approximate_facade_straddling_the_bound_is_not_evaluated() {
    let (model, areas) = building();
    let areas = areas.with("w1", 24.0, 1.0);
    let evaluation = run(
        model,
        areas,
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.12)),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("eg".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn without_a_facade_service_the_ratio_is_not_evaluated() {
    let (model, _) = building();
    let evaluation = model.evaluate(
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.1)),
    );
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::MissingService)
    );
}

#[test]
fn an_unknown_measure_is_an_invalid_declaration() {
    let (model, areas) = building();
    let mut parameters = window_to_wall(0.1);
    parameters[0] = ("measure", string("elevation"));
    let evaluation = run(
        model,
        areas,
        &AreaRatio,
        &rule(RATIO, kind("storey"), parameters),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn the_facade_area_of_each_storey_is_bounded() {
    let (model, areas) = building();
    let evaluation = run(
        model,
        areas,
        &PlanAreaRange,
        &rule(
            RANGE,
            kind("storey"),
            vec![
                ("measure", string("facade")),
                ("member_selector", selector(external())),
                ("relationship", string("contains")),
                ("minimum", number(40.0)),
            ],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "og".into(),
            "summed facade area of the members is 30 m²; required at least 40 m²".into()
        )]
    );
}

/// Storey `eg` states a gross floor area of 100 m²; its spaces measure
/// 60 m² and 10 m², the second an empty (void) area.
fn floors() -> (Model, Areas) {
    let model = Model::default()
        .object("eg", "storey")
        .value(
            "eg",
            "Qto_BuildingStoreyBaseQuantities",
            "GrossFloorArea",
            PropertyValue::Quantity {
                value: 100.0,
                dimension: QuantityDimension::Area,
            },
        )
        .object("s1", "space")
        .object("s2", "void")
        .edge("aggregates", "eg", "s1")
        .edge("aggregates", "eg", "s2");
    let areas = Areas::default().with("s1", 60.0, 0.0).with("s2", 10.0, 0.0);
    (model, areas)
}

#[test]
fn net_to_gross_and_empty_area_ratios_are_area_ratios_against_a_stated_gross_area() {
    let gross = property(Some("Qto_BuildingStoreyBaseQuantities"), "GrossFloorArea");
    let (model, areas) = floors();
    let net = run(
        model,
        areas,
        &AreaRatio,
        &rule(
            RATIO,
            kind("storey"),
            vec![
                (
                    "numerator_selector",
                    selector(Selector::AnyOf {
                        operands: vec![kind("space"), kind("void")],
                    }),
                ),
                ("denominator_property", gross.clone()),
                ("minimum", number(0.75)),
                ("relationship", string("aggregates")),
            ],
        ),
    );
    assert_eq!(
        findings(&net),
        [(
            "eg".into(),
            "plan area ratio is 0.7 (70 m² of 100 m²); required at least 0.75".into()
        )]
    );
    let (model, areas) = floors();
    let empty = run(
        model,
        areas,
        &AreaRatio,
        &rule(
            RATIO,
            kind("storey"),
            vec![
                ("numerator_selector", selector(kind("void"))),
                ("denominator_property", gross),
                ("maximum", number(0.05)),
                ("relationship", string("aggregates")),
            ],
        ),
    );
    assert_eq!(
        findings(&empty),
        [(
            "eg".into(),
            "plan area ratio is 0.1 (10 m² of 100 m²); required at most 0.05".into()
        )]
    );
}

/// The rows of the table `name`, by the local id of their object.
fn rows(evaluation: &CapabilityEvaluation, name: &str) -> Vec<(String, Vec<ReportValue>)> {
    let table = evaluation
        .tables()
        .iter()
        .find(|table| table.name() == name)
        .unwrap_or_else(|| panic!("no table {name}"));
    table
        .rows()
        .iter()
        .map(|row| {
            let local = row.scope().object().map_or("-", |id| id.local_id.as_str());
            (local.to_owned(), row.values().to_vec())
        })
        .collect()
}

#[test]
fn every_measured_ratio_is_reported_in_a_table_passing_or_not() {
    let (model, areas) = building();
    let evaluation = run(
        model,
        areas.with("w4", 30.0, 1.0),
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.1)),
    );
    let columns: Vec<_> = evaluation.tables()[0]
        .columns()
        .iter()
        .map(|column| (column.id.as_str(), column.kind.unit_symbol()))
        .collect();
    assert_eq!(
        columns,
        [
            ("numerator_area", Some("m²".to_owned())),
            ("denominator_area", Some("m²".to_owned())),
            ("ratio", None),
        ]
    );
    assert_eq!(
        rows(&evaluation, "ratios"),
        [
            (
                "eg".to_owned(),
                vec![
                    ReportValue::exact(6.0),
                    ReportValue::exact(50.0),
                    ReportValue::exact(0.12),
                ]
            ),
            // og passes with no windows; its facade is approximate.
            (
                "og".to_owned(),
                vec![
                    ReportValue::exact(0.0),
                    ReportValue::measured(29.0, 31.0),
                    ReportValue::exact(0.0),
                ]
            ),
        ]
    );
}

#[test]
fn every_measured_area_is_reported_in_a_table_in_its_measure() {
    let (model, areas) = building();
    let evaluation = run(
        model,
        areas,
        &PlanAreaRange,
        &rule(
            RANGE,
            kind("storey"),
            vec![
                ("measure", string("facade")),
                ("member_selector", selector(external())),
                ("relationship", string("contains")),
                ("minimum", number(40.0)),
            ],
        ),
    );
    assert_eq!(evaluation.tables()[0].columns()[0].id, "facade_area");
    assert_eq!(
        rows(&evaluation, "areas"),
        [
            ("eg".to_owned(), vec![ReportValue::exact(44.0)]),
            ("og".to_owned(), vec![ReportValue::exact(30.0)]),
        ]
    );
    // Nothing measured, no table.
    let (model, _) = building();
    let evaluation = model.evaluate(
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.1)),
    );
    assert!(evaluation.tables().is_empty());
}

#[test]
fn an_external_wall_ratio_divides_facade_area_by_gross_footprint() {
    let (model, areas) = building();
    let model = model
        .object("s1", "slab")
        .object("s2", "slab")
        .edge("contains", "eg", "s1")
        .edge("contains", "og", "s2");
    let areas = areas.with("s1", 100.0, 0.0).with("s2", 100.0, 0.0);
    let parameters = vec![
        ("numerator_measure", string("facade")),
        ("denominator_measure", string("footprint")),
        ("numerator_selector", selector(external())),
        ("denominator_selector", selector(kind("slab"))),
        ("maximum", number(0.4)),
        ("relationship", string("contains")),
    ];
    let evaluation = run(
        model,
        areas,
        &AreaRatio,
        &rule(RATIO, kind("storey"), parameters.clone()),
    );
    // eg: 24 + 20 m² of external facade over a 100 m² floor; og: 30 m².
    assert_eq!(
        findings(&evaluation),
        [(
            "eg".into(),
            "facade area to plan area ratio is 0.44 (44 m² of 100 m²); required at most 0.4".into()
        )]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    // `measure` states both sides at once, so it never joins them.
    let mut both = parameters;
    both.push(("measure", string("facade")));
    let (model, areas) = building();
    let evaluation = run(model, areas, &AreaRatio, &rule(RATIO, kind("storey"), both));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// The window-to-wall ratio from measured facade areas, summed per storey
/// as an aggregate would, reaches `area-ratio`'s verdict.
#[test]
fn the_measured_facade_areas_reach_the_window_to_wall_verdicts() {
    let (model, areas) = building();
    let (judged, judged_areas) = building();
    let evaluation = run(
        judged,
        judged_areas,
        &AreaRatio,
        &rule(RATIO, kind("storey"), window_to_wall(0.1)),
    );
    let (project, mut services) = model.services();
    services
        .register(FacadeAreaServiceHandle::new(Arc::new(areas)))
        .unwrap();
    let facade = |object: &str| {
        common::measured(&services, &project, &id(object), "facade_area")
            .unwrap()
            .unwrap()
            .0
    };
    for (storey, windows, external) in [
        ("eg", &["f1", "f2"][..], &["w1", "w2", "f1", "f2"][..]),
        ("og", &[][..], &["w4"][..]),
    ] {
        let ratio = windows.iter().map(|w| facade(w)).sum::<f64>()
            / external.iter().map(|w| facade(w)).sum::<f64>();
        let found = findings(&evaluation)
            .iter()
            .any(|(object, _)| object == storey);
        assert_eq!(ratio > 0.1, found, "{storey}: {ratio}");
    }
}

/// The storey metrics rewritten as expressions over measured and stated
/// areas, compared object by object with `area-ratio` and `plan-area`.
mod parity {
    use super::*;
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{
        assert_parity, at_least, at_most, divide, m2, measured, over_path, plain,
        rule as expression, stated,
    };
    use serde_json::{Value, json};

    fn rewrite(
        fixture: fn() -> (Model, Areas),
        slack: Option<(&str, f64)>,
        selector: Selector,
        requirement: &Value,
    ) -> CapabilityEvaluation {
        let areas = || {
            let (_, areas) = fixture();
            Arc::new(match slack {
                Some((object, slack)) => {
                    let area = areas.0[&id(object)].0;
                    areas.with(object, area, slack)
                }
                None => areas,
            })
        };
        fixture().0.evaluate_measured(
            &ExpressionRequirement,
            &expression(selector, requirement),
            |services| {
                services
                    .register(FacadeAreaServiceHandle::new(areas()))
                    .unwrap();
                services
                    .register(PlanAreaServiceHandle::new(areas()))
                    .unwrap();
            },
        )
    }

    fn capability(
        fixture: fn() -> (Model, Areas),
        slack: Option<(&str, f64)>,
        check: &dyn RuleCapability,
        capability: &str,
        selector: Selector,
        parameters: Vec<(&str, ParameterValue)>,
    ) -> CapabilityEvaluation {
        let (model, areas) = fixture();
        let areas = match slack {
            Some((object, slack)) => {
                let area = areas.0[&id(object)].0;
                areas.with(object, area, slack)
            }
            None => areas,
        };
        run(model, areas, check, &rule(capability, selector, parameters))
    }

    /// `value`, an area, summed in square metres over the objects `path`
    /// reaches, those `filter` selects.
    fn summed(path: &[&str], filter: &Selector, value: Value) -> Value {
        over_path("sum", path, filter, Some(divide(value, m2(1.0))))
    }

    fn ratio(numerator: Value, denominator: Value) -> Value {
        json!({"kind": "round", "operand": divide(numerator, denominator),
            "step": plain(1e-9)})
    }

    fn window_to_wall_share(path: &[&str]) -> Value {
        let facade = || measured("facade_area");
        ratio(
            summed(path, &kind("window"), facade()),
            summed(path, &gross(), facade()),
        )
    }

    #[test]
    fn window_to_wall_ratios_reach_the_verdicts() {
        for (slack, maximum) in [
            (None, 0.1),
            (None, 0.12),
            (Some(("w1", 1.0)), 0.12),
            (Some(("w4", 1.0)), 0.1),
        ] {
            let found = capability(
                building,
                slack,
                &AreaRatio,
                RATIO,
                kind("storey"),
                window_to_wall(maximum),
            );
            let rewritten = rewrite(
                building,
                slack,
                kind("storey"),
                &at_most(window_to_wall_share(&["contains"]), plain(maximum)),
            );
            assert_parity(RATIO, &found, &rewritten);
        }
        let path = ["aggregates:forward", "contains:forward"];
        for maximum in [0.07, 0.08] {
            let mut parameters = window_to_wall(maximum);
            parameters.retain(|(name, _)| *name != "relationship");
            parameters.push(("path", strings(&path)));
            let found = capability(
                building,
                None,
                &AreaRatio,
                RATIO,
                kind("building"),
                parameters,
            );
            let rewritten = rewrite(
                building,
                None,
                kind("building"),
                &at_most(window_to_wall_share(&path), plain(maximum)),
            );
            assert_parity(RATIO, &found, &rewritten);
        }
    }

    #[test]
    fn without_a_facade_service_both_leave_every_storey_open() {
        let (model, _) = building();
        let found = model.evaluate(
            &AreaRatio,
            &rule(RATIO, kind("storey"), window_to_wall(0.1)),
        );
        let (model, _) = building();
        let rewritten = model.evaluate_measured(
            &ExpressionRequirement,
            &expression(
                kind("storey"),
                &at_most(window_to_wall_share(&["contains"]), plain(0.1)),
            ),
            |_| {},
        );
        assert_parity(RATIO, &found, &rewritten);
    }

    #[test]
    fn the_summed_facade_area_reaches_the_verdicts() {
        for minimum in [40.0, 30.0, 44.5] {
            let found = capability(
                building,
                None,
                &PlanAreaRange,
                RANGE,
                kind("storey"),
                vec![
                    ("measure", string("facade")),
                    ("member_selector", selector(external())),
                    ("relationship", string("contains")),
                    ("minimum", number(minimum)),
                ],
            );
            let rewritten = rewrite(
                building,
                None,
                kind("storey"),
                &at_least(
                    summed(
                        &["contains"],
                        &external(),
                        measured("plan_area;measure=facade"),
                    ),
                    plain(minimum),
                ),
            );
            assert_parity(RANGE, &found, &rewritten);
        }
    }

    #[test]
    fn ratios_against_a_stated_gross_area_reach_the_verdicts() {
        let gross = || property(Some("Qto_BuildingStoreyBaseQuantities"), "GrossFloorArea");
        let stated_gross = || {
            divide(
                stated("Qto_BuildingStoreyBaseQuantities", "GrossFloorArea"),
                m2(1.0),
            )
        };
        let net = || Selector::AnyOf {
            operands: vec![kind("space"), kind("void")],
        };
        for minimum in [0.75, 0.7, 0.6] {
            let found = capability(
                floors,
                None,
                &AreaRatio,
                RATIO,
                kind("storey"),
                vec![
                    ("numerator_selector", selector(net())),
                    ("denominator_property", gross()),
                    ("minimum", number(minimum)),
                    ("relationship", string("aggregates")),
                ],
            );
            let rewritten = rewrite(
                floors,
                None,
                kind("storey"),
                &at_least(
                    ratio(
                        summed(&["aggregates"], &net(), measured("area")),
                        stated_gross(),
                    ),
                    plain(minimum),
                ),
            );
            assert_parity(RATIO, &found, &rewritten);
        }
        for maximum in [0.05, 0.1] {
            let found = capability(
                floors,
                None,
                &AreaRatio,
                RATIO,
                kind("storey"),
                vec![
                    ("numerator_selector", selector(kind("void"))),
                    ("denominator_property", gross()),
                    ("maximum", number(maximum)),
                    ("relationship", string("aggregates")),
                ],
            );
            let rewritten = rewrite(
                floors,
                None,
                kind("storey"),
                &at_most(
                    ratio(
                        summed(&["aggregates"], &kind("void"), measured("area")),
                        stated_gross(),
                    ),
                    plain(maximum),
                ),
            );
            assert_parity(RATIO, &found, &rewritten);
        }
    }

    fn with_slabs() -> (Model, Areas) {
        let (model, areas) = building();
        let model = model
            .object("s1", "slab")
            .object("s2", "slab")
            .edge("contains", "eg", "s1")
            .edge("contains", "og", "s2");
        (model, areas.with("s1", 100.0, 0.0).with("s2", 100.0, 0.0))
    }

    #[test]
    fn a_facade_over_a_footprint_ratio_reaches_the_verdicts() {
        for maximum in [0.4, 0.3, 0.44] {
            let found = capability(
                with_slabs,
                None,
                &AreaRatio,
                RATIO,
                kind("storey"),
                vec![
                    ("numerator_measure", string("facade")),
                    ("denominator_measure", string("footprint")),
                    ("numerator_selector", selector(external())),
                    ("denominator_selector", selector(kind("slab"))),
                    ("maximum", number(maximum)),
                    ("relationship", string("contains")),
                ],
            );
            let rewritten = rewrite(
                with_slabs,
                None,
                kind("storey"),
                &at_most(
                    ratio(
                        summed(&["contains"], &external(), measured("facade_area")),
                        summed(&["contains"], &kind("slab"), measured("area")),
                    ),
                    plain(maximum),
                ),
            );
            assert_parity(RATIO, &found, &rewritten);
        }
    }
}
