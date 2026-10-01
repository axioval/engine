//! `local-circulation`'s space-level entrance checks: `require_entrances`
//! and `check_entrance_width`, both off by default.
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CirculationContact, CirculationMap, CirculationRequest, ClearanceOutcome,
    ClearanceRequest, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService,
    FreeSpaceServiceHandle, PlacementOutcome, PlacementRequest,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::LocalCirculation;
use common::{
    Model, boolean, findings, id, kind, number, property, rule, selector, source, strings,
    unevaluated,
};

const ID: &str = "axioval:capability.local-circulation";

fn length(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

/// Room `s1` with door `d1` (0.80 m clear) and WC `a`; store `s2`, reached
/// by no door and holding nothing; office `s3` with door `d3` (0.95 m
/// clear); office `s4` with door `d4`, which states only its overall
/// width, 1 m.
fn model() -> Model {
    Model::default()
        .object("s1", "space")
        .object("s2", "space")
        .object("s3", "space")
        .object("s4", "space")
        .object("d1", "door")
        .object("d3", "door")
        .object("d4", "door")
        .object("a", "wc")
        .edge("bounds", "d1", "s1")
        .edge("bounds", "d3", "s3")
        .edge("bounds", "d4", "s4")
        .edge("in", "a", "s1")
        .value("d1", "Pset", "ClearWidth", length(0.8))
        .value("d3", "Pset", "ClearWidth", length(0.95))
        .value("d4", "Attributes", "OverallWidth", length(1.0))
}

/// Every subject reaches the one piece of every map.
struct Stub;

impl FreeSpaceService for Stub {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        panic!("no clearance is asked")
    }

    fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        panic!("no placement is asked")
    }

    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        panic!("no free area is asked")
    }

    fn map_circulation(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        let contacts = request
            .subjects()
            .into_iter()
            .map(|subject| CirculationContact::new(subject, vec![(0, None)], vec![0]))
            .collect();
        CirculationMap::try_new(
            request.clone(),
            1,
            1,
            Vec::new(),
            Vec::new(),
            0.05,
            contacts,
            Vec::new(),
            Evidence::exact(source(), "circulation"),
        )
    }
}

fn run(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("component_selector", selector(kind("wc"))),
        ("space_path", strings(&["in"])),
        ("access_path", strings(&["bounds"])),
        ("door_selector", selector(kind("door"))),
        ("space_selector", selector(kind("space"))),
        ("width_metres", number(0.9)),
        ("clear_height_metres", number(2.0)),
    ];
    parameters.extend(extra);
    model.evaluate_with(
        &LocalCirculation,
        &rule(ID, kind("space"), parameters),
        |services| {
            services
                .register(FreeSpaceServiceHandle::new(Arc::new(Stub)))
                .unwrap();
        },
    )
}

fn widths() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("check_entrance_width", boolean(true)),
        ("clear_width_property", property(Some("Pset"), "ClearWidth")),
    ]
}

#[test]
fn both_checks_are_off_by_default() {
    let evaluation = run(model(), Vec::new());
    assert!(findings(&evaluation).is_empty(), "{evaluation:#?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
}

#[test]
fn a_room_reached_by_no_door_is_a_finding_even_when_empty() {
    let evaluation = run(model(), vec![("require_entrances", boolean(true))]);
    assert_eq!(
        findings(&evaluation),
        [(
            "s2".into(),
            format!(
                "{} has no entrance: no door or opening reaches it along bounds",
                id("s2")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
}

#[test]
fn an_entrance_narrower_than_the_path_is_a_finding() {
    // d4 states no clear width and the rule no deduction: not evaluated.
    let evaluation = run(model(), widths());
    assert_eq!(
        findings(&evaluation),
        [(
            "s1".into(),
            format!(
                "entrance {} is narrower than the path: clear width (Pset.ClearWidth) is 0.8 \
                 m; required at least 0.9 m, the path's width",
                id("d1")
            )
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("d1")]);
    assert_eq!(
        unevaluated(&evaluation),
        [("s4".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // With the deduction, d4's 1 m less 0.1 m meets 0.9 m.
    let mut deducted = widths();
    deducted.push((
        "overall_width",
        property(Some("Attributes"), "OverallWidth"),
    ));
    deducted.push((
        "width_deduction",
        ParameterValue::Quantity {
            value: 0.1,
            unit: "m".into(),
        },
    ));
    let evaluation = run(model(), deducted);
    assert_eq!(
        findings(&evaluation)
            .iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["s1"]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
}

#[test]
fn both_checks_stand_together_with_the_path_checks() {
    let mut parameters = widths();
    parameters.push(("require_entrances", boolean(true)));
    let model = model().value("d4", "Pset", "ClearWidth", length(1.0));
    let evaluation = run(model, parameters);
    assert_eq!(
        findings(&evaluation)
            .iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["s1", "s2"]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
}

#[test]
fn the_entrance_width_declaration_is_checked() {
    let without_source = vec![("check_entrance_width", boolean(true))];
    let without_check = vec![("clear_width_property", property(Some("Pset"), "ClearWidth"))];
    let half_deduction = vec![
        ("check_entrance_width", boolean(true)),
        (
            "overall_width",
            property(Some("Attributes"), "OverallWidth"),
        ),
    ];
    for extra in [without_source, without_check, half_deduction] {
        let evaluation = run(model(), extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
