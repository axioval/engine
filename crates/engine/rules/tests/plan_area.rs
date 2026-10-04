//! Area ratios and plan coverage over measured footprints.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::{AreaRatio, PlanCoverage};
use common::{
    Model, findings, flagged, id, kind, number, rule, selector, source, string, unevaluated,
};

/// Axis-aligned rectangles `(x0, y0, x1, y1)`, with an optional uncertainty
/// in square metres added around every measurement of that object.
#[derive(Default)]
struct Rectangles(BTreeMap<ObjectId, ([f64; 4], f64)>);

impl Rectangles {
    fn with(mut self, local: &str, rectangle: [f64; 4], slack: f64) -> Self {
        self.0.insert(id(local), (rectangle, slack));
        self
    }

    fn get(&self, object: &ObjectId) -> Result<([f64; 4], f64), PlanAreaError> {
        self.0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))
    }
}

fn area(value: f64, slack: f64, locator: String) -> Result<PlanArea, PlanAreaError> {
    let mut evidence = Evidence::exact(source(), locator);
    evidence.exact = slack == 0.0;
    PlanArea::try_new((value - slack).max(0.0), value + slack, evidence)
}

impl PlanAreaService for Rectangles {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let ([x0, y0, x1, y1], slack) = self.get(object)?;
        area((x1 - x0) * (y1 - y0), slack, format!("footprint:{object}"))
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let ([a0, b0, a1, b1], first_slack) = self.get(first)?;
        let ([c0, d0, c1, d1], second_slack) = self.get(second)?;
        let width = (a1.min(c1) - a0.max(c0)).max(0.0);
        let depth = (b1.min(d1) - b0.max(d0)).max(0.0);
        area(
            width * depth,
            first_slack + second_slack,
            format!("overlap:{first}:{second}"),
        )
    }
}

fn run(
    model: Model,
    rectangles: Rectangles,
    capability: &dyn axioval_engine::RuleCapability,
    rule: &axioval_engine::CompiledRule,
) -> axioval_engine::CapabilityEvaluation {
    model.evaluate_with(capability, rule, |services| {
        services
            .register(PlanAreaServiceHandle::new(Arc::new(rectangles)))
            .unwrap();
    })
}

mod area_ratio {
    use super::*;

    const ID: &str = "axioval:capability.area-ratio";

    /// Storey `a` (10 x 10 slab) holds 60 m² of spaces; storey `b` 30 m².
    fn storeys() -> (Model, Rectangles) {
        let model = Model::default()
            .object("a", "storey")
            .object("b", "storey")
            .object("slab-a", "slab")
            .object("slab-b", "slab")
            .object("s1", "space")
            .object("s2", "space")
            .object("s3", "space")
            .edge("contains", "a", "slab-a")
            .edge("contains", "a", "s1")
            .edge("contains", "a", "s2")
            .edge("contains", "b", "slab-b")
            .edge("contains", "b", "s3");
        let rectangles = Rectangles::default()
            .with("slab-a", [0.0, 0.0, 10.0, 10.0], 0.0)
            .with("slab-b", [0.0, 0.0, 10.0, 10.0], 0.0)
            .with("s1", [0.0, 0.0, 6.0, 5.0], 0.0)
            .with("s2", [0.0, 5.0, 6.0, 10.0], 0.0)
            .with("s3", [0.0, 0.0, 3.0, 10.0], 0.0);
        (model, rectangles)
    }

    pub(super) fn storeys_fixture() -> (Model, Rectangles) {
        storeys()
    }

    pub(super) fn parameters_fixture(minimum: f64) -> Vec<(&'static str, ParameterValue)> {
        parameters(minimum)
    }

    fn parameters(minimum: f64) -> Vec<(&'static str, ParameterValue)> {
        vec![
            ("numerator_selector", selector(kind("space"))),
            ("denominator_selector", selector(kind("slab"))),
            ("minimum", number(minimum)),
            ("relationship", string("contains")),
        ]
    }

    #[test]
    fn a_storey_below_the_minimum_share_of_space_is_found() {
        let (model, rectangles) = storeys();
        let evaluation = run(
            model,
            rectangles,
            &AreaRatio,
            &rule(ID, kind("storey"), parameters(0.5)),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "b".into(),
                "plan area ratio is 0.3 (30 m² of 100 m²); required at least 0.5".into()
            )]
        );
        assert_eq!(evaluation.findings()[0].related.len(), 1);
    }

    #[test]
    fn an_interval_straddling_the_bound_is_not_evaluated() {
        let (model, _) = storeys();
        let rectangles = storeys().1.with("s3", [0.0, 0.0, 5.0, 10.0], 1.0);
        let evaluation = run(
            model,
            rectangles,
            &AreaRatio,
            &rule(ID, kind("storey"), parameters(0.5)),
        );
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("b".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }

    #[test]
    fn without_geometry_nothing_is_judged() {
        let (model, _) = storeys();
        let evaluation = model.evaluate(&AreaRatio, &rule(ID, kind("storey"), parameters(0.5)));
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("a".to_owned(), NotEvaluatedReason::MissingService),
                ("b".to_owned(), NotEvaluatedReason::MissingService)
            ]
        );
    }
}

mod window_ratio {
    use super::*;
    use axioval_ir::{PropertyValue, QuantityDimension};
    use common::property;

    fn glazing(area: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value: area,
            dimension: QuantityDimension::Area,
        }
    }

    #[test]
    fn stated_window_areas_are_related_to_measured_floor_area() {
        // One eighth of the floor area must be glazed: 5 m² of 50 m² is not.
        let model = Model::default()
            .object("st", "storey")
            .object("room", "space")
            .object("w1", "window")
            .object("w2", "window")
            .edge("contains", "st", "room")
            .edge("contains", "st", "w1")
            .edge("contains", "st", "w2")
            .value("w1", "Qto", "Area", glazing(3.0))
            .value("w2", "Qto", "Area", glazing(2.0));
        let rectangles = Rectangles::default().with("room", [0.0, 0.0, 10.0, 5.0], 0.0);
        let parameters = vec![
            ("numerator_selector", selector(kind("window"))),
            ("numerator_property", property(Some("Qto"), "Area")),
            ("denominator_selector", selector(kind("space"))),
            ("minimum", number(0.125)),
            ("relationship", string("contains")),
        ];
        let evaluation = run(
            model,
            rectangles,
            &AreaRatio,
            &rule("axioval:capability.area-ratio", kind("storey"), parameters),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "st".into(),
                "plan area ratio is 0.1 (5 m² of 50 m²); required at least 0.125".into()
            )]
        );
    }

    #[test]
    fn an_atrium_spanning_two_storeys_counts_in_both_numerators() {
        // The derived relationship runs from each space to every storey it
        // spans; walked backward, each storey reaches the atrium.
        const SPANS: &str = "axioval:derived.spans-level;overlap=1";
        let model = Model::default()
            .object("eg", "storey")
            .object("og", "storey")
            .object("atrium", "space")
            .object("office", "space")
            .object("upper", "space")
            .object("eg-slab", "slab")
            .object("og-slab", "slab")
            .edge(SPANS, "atrium", "eg")
            .edge(SPANS, "atrium", "og")
            .edge(SPANS, "office", "eg")
            .edge(SPANS, "upper", "og")
            .edge(SPANS, "eg-slab", "eg")
            .edge(SPANS, "og-slab", "og");
        let rectangles = Rectangles::default()
            .with("atrium", [0.0, 0.0, 4.0, 5.0], 0.0)
            .with("office", [4.0, 0.0, 10.0, 5.0], 0.0)
            .with("upper", [4.0, 0.0, 10.0, 5.0], 0.0)
            .with("eg-slab", [0.0, 0.0, 10.0, 5.0], 0.0)
            .with("og-slab", [0.0, 0.0, 10.0, 5.0], 0.0);
        let parameters = vec![
            ("numerator_selector", selector(kind("space"))),
            ("denominator_selector", selector(kind("slab"))),
            ("minimum", number(0.9)),
            ("relationship", string(SPANS)),
            ("direction", string("backward")),
        ];
        let evaluation = run(
            model,
            rectangles,
            &AreaRatio,
            &rule("axioval:capability.area-ratio", kind("storey"), parameters),
        );
        // Without the atrium the upper storey's spaces cover 30 m² of 50 m².
        assert!(
            evaluation.findings().is_empty(),
            "{:?}",
            findings(&evaluation)
        );
        assert!(evaluation.not_evaluated_outcomes().is_empty());
    }

    #[test]
    fn a_window_without_a_stated_area_leaves_the_storey_unjudged() {
        let model = Model::default()
            .object("st", "storey")
            .object("room", "space")
            .object("w1", "window")
            .edge("contains", "st", "room")
            .edge("contains", "st", "w1");
        let rectangles = Rectangles::default().with("room", [0.0, 0.0, 10.0, 5.0], 0.0);
        let evaluation = run(
            model,
            rectangles,
            &AreaRatio,
            &rule(
                "axioval:capability.area-ratio",
                kind("storey"),
                vec![
                    ("numerator_selector", selector(kind("window"))),
                    ("numerator_property", property(Some("Qto"), "Area")),
                    ("denominator_selector", selector(kind("space"))),
                    ("minimum", number(0.125)),
                    ("relationship", string("contains")),
                ],
            ),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("st".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }
}

mod light_area {
    use super::*;
    use axioval_ir::contract::TableRow;
    use axioval_ir::{PropertyValue, QuantityDimension};
    use common::{boolean, property, strings};

    const ID: &str = "axioval:capability.area-ratio";
    const ATTRIBUTES: &str = "axioval:attributes";

    fn metres(value: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }
    }

    fn square_metres(value: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Area,
        }
    }

    fn quantity(value: f64, unit: &str) -> ParameterValue {
        ParameterValue::Quantity {
            value,
            unit: unit.into(),
        }
    }

    /// A light-area row: type pattern, width and height in mm, area in m².
    fn row(kind: Option<&str>, width: f64, height: f64, light: f64) -> TableRow {
        let mut cells = TableRow::new();
        if let Some(kind) = kind {
            cells.insert("type".into(), string(kind));
        }
        cells.insert("width".into(), quantity(width, "mm"));
        cells.insert("height".into(), quantity(height, "mm"));
        cells.insert("light_area".into(), quantity(light, "m2"));
        cells
    }

    /// Room `room` (20 m²) with the windows `windows` in it. Each window's
    /// type name is on its type object `<id>-type`.
    fn rooms(windows: &[&str]) -> (Model, Rectangles) {
        let mut model = Model::default().object("room", "space");
        for window in windows {
            let kind = format!("{window}-type");
            model = model
                .object(window, "window")
                .object(&kind, "windowType")
                .edge("opens", "room", window)
                .edge("typed", window, &kind);
        }
        let rectangles = Rectangles::default().with("room", [0.0, 0.0, 4.0, 5.0], 0.0);
        (model, rectangles)
    }

    /// `rooms(&["w1"])` with a second room, `bare` (20 m²), without a window.
    fn with_bare_room() -> (Model, Rectangles) {
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model.object("bare", "space"), "w1", 2.0, 2.0).value(
            "w1",
            "Pset",
            "LightArea",
            square_metres(3.0),
        );
        (model, rectangles.with("bare", [0.0, 0.0, 4.0, 5.0], 0.0))
    }

    fn sized(model: Model, window: &str, width: f64, height: f64) -> Model {
        model
            .value(window, ATTRIBUTES, "OverallWidth", metres(width))
            .value(window, ATTRIBUTES, "OverallHeight", metres(height))
    }

    /// Light-area mode over `opens`, one eighth of the floor required.
    fn parameters(
        extra: Vec<(&'static str, ParameterValue)>,
    ) -> Vec<(&'static str, ParameterValue)> {
        let mut parameters = vec![
            ("numerator_selector", selector(kind("window"))),
            ("numerator_derivation", string("light-area")),
            ("numerator_property", property(Some("Pset"), "LightArea")),
            ("overall_width", property(Some(ATTRIBUTES), "OverallWidth")),
            (
                "overall_height",
                property(Some(ATTRIBUTES), "OverallHeight"),
            ),
            ("light_type", property(Some(ATTRIBUTES), "Name")),
            ("light_type_path", strings(&["typed"])),
            (
                "light_area_table",
                ParameterValue::Table {
                    value: vec![
                        row(Some("Case*"), 1500.0, 1500.0, 1.8),
                        row(None, 1500.0, 1500.0, 1.6),
                    ],
                },
            ),
            ("frame_width", quantity(50.0, "mm")),
            ("minimum", number(0.125)),
            ("relationship", string("opens")),
        ];
        for (name, value) in extra {
            parameters.retain(|(held, _)| *held != name);
            parameters.push((name, value));
        }
        parameters
    }

    fn evaluate(
        model: Model,
        rectangles: Rectangles,
        parameters: Vec<(&'static str, ParameterValue)>,
    ) -> axioval_engine::CapabilityEvaluation {
        run(
            model,
            rectangles,
            &AreaRatio,
            &rule(ID, kind("space"), parameters),
        )
    }

    fn room_finding(evaluation: &axioval_engine::CapabilityEvaluation) -> &axioval_ir::Finding {
        evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id().unwrap().local_id == "room")
            .expect("a finding against the room")
    }

    fn records(finding: &axioval_ir::Finding) -> Vec<String> {
        finding
            .evidence
            .iter()
            .filter(|evidence| evidence.locator.starts_with("axioval:derived.light-area"))
            .map(|evidence| evidence.locator.clone())
            .collect()
    }

    fn step(window: &str, step: &str) -> String {
        format!("axioval:derived.light-area:{}:step={step}", id(window))
    }

    #[test]
    fn a_stated_light_area_comes_first() {
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.5, 1.5)
            .value("w1", "Pset", "LightArea", square_metres(2.0))
            .text("w1-type", ATTRIBUTES, "Name", "Casement");
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        let finding = room_finding(&evaluation);
        assert_eq!(
            finding.message,
            "plan area ratio is 0.1 (2 m² of 20 m²); required at least 0.125; light areas: 1 stated"
        );
        assert_eq!(records(finding), [step("w1", "stated")]);
    }

    #[test]
    fn without_a_stated_area_the_most_specific_table_row_applies() {
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.5, 1.5).text("w1-type", ATTRIBUTES, "Name", "Casement");
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        let finding = room_finding(&evaluation);
        assert_eq!(
            finding.message,
            "plan area ratio is 0.09 (1.8 m² of 20 m²); required at least 0.125; \
             light areas: 1 from the light-area table"
        );
        assert_eq!(records(finding), [step("w1", "table;row=0")]);
    }

    #[test]
    fn without_a_matching_row_the_frame_allowance_applies() {
        // 1.2 m × 1.0 m less 2 · 2.2 m · 0.05 m = 0.98 m².
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.2, 1.0).text("w1-type", ATTRIBUTES, "Name", "Casement");
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        let finding = room_finding(&evaluation);
        assert_eq!(
            finding.message,
            "plan area ratio is 0.049 (0.98 m² of 20 m²); required at least 0.125; \
             light areas: 1 by frame allowance"
        );
        assert_eq!(
            records(finding),
            [step("w1", "frame-allowance;frame_width=0.05")]
        );
    }

    #[test]
    fn each_member_records_the_step_that_produced_its_area() {
        // 2.0 stated + 1.6 from the untyped row + 0.98 by frame allowance.
        let (model, rectangles) = rooms(&["w1", "w2", "w3"]);
        let model =
            sized(model, "w1", 1.5, 1.5).value("w1", "Pset", "LightArea", square_metres(2.0));
        let model = sized(model, "w2", 1.5, 1.5).text("w2-type", ATTRIBUTES, "Name", "Fixed");
        let model = sized(model, "w3", 1.2, 1.0).text("w3-type", ATTRIBUTES, "Name", "Fixed");
        let evaluation = evaluate(
            model,
            rectangles,
            parameters(vec![("minimum", number(0.25))]),
        );
        let finding = room_finding(&evaluation);
        assert_eq!(
            finding.message,
            "plan area ratio is 0.229 (4.58 m² of 20 m²); required at least 0.25; \
             light areas: 1 stated, 1 from the light-area table, 1 by frame allowance"
        );
        assert_eq!(
            records(finding),
            [
                step("w1", "stated"),
                step("w2", "table;row=1"),
                step("w3", "frame-allowance;frame_width=0.05"),
            ]
        );
    }

    #[test]
    fn a_stated_area_that_is_not_an_area_does_not_fall_back() {
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.2, 1.0)
            .value("w1", "Pset", "LightArea", PropertyValue::Null)
            .text("w1-type", ATTRIBUTES, "Name", "Casement");
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("room".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }

    #[test]
    fn an_unknown_type_name_does_not_fall_back_to_the_frame_allowance() {
        // A typed row fits the size, so the missing name decides nothing.
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.5, 1.5);
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("room".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
        assert!(
            evaluation.not_evaluated_outcomes()[0]
                .message()
                .contains("cannot be decided"),
            "{:?}",
            evaluation.not_evaluated_outcomes()
        );
    }

    #[test]
    fn a_value_the_chain_cannot_produce_is_not_evaluated() {
        // No stated area and no overall width: neither fallback applies.
        let (model, rectangles) = rooms(&["w1"]);
        let model = model
            .value("w1", ATTRIBUTES, "OverallHeight", metres(1.0))
            .text("w1-type", ATTRIBUTES, "Name", "Casement");
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("room".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
        // Without a frame allowance, a size no row lists has no light area.
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.2, 1.0).text("w1-type", ATTRIBUTES, "Name", "Casement");
        let mut without_frame = parameters(vec![]);
        without_frame.retain(|(name, _)| *name != "frame_width");
        let evaluation = evaluate(model, rectangles, without_frame);
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("room".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }

    #[test]
    fn a_light_area_larger_than_the_element_is_found() {
        let (model, rectangles) = rooms(&["w1"]);
        let model =
            sized(model, "w1", 1.0, 1.0).value("w1", "Pset", "LightArea", square_metres(3.0));
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert_eq!(
            findings(&evaluation),
            [(
                "w1".into(),
                "light area 3 m² (Pset.LightArea) is larger than the overall area 1 m² \
                 (axioval:attributes.OverallWidth 1 m × axioval:attributes.OverallHeight 1 m)"
                    .into()
            )]
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("room".to_owned(), NotEvaluatedReason::InvalidEvidence)]
        );
    }

    #[test]
    fn a_stated_area_without_an_overall_size_is_used_but_not_compared() {
        let (model, rectangles) = rooms(&["w1"]);
        let model = model.value("w1", "Pset", "LightArea", square_metres(2.0));
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert_eq!(flagged(&evaluation), ["room"]);
        assert_eq!(
            unevaluated(&evaluation),
            [("w1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }

    #[test]
    fn a_space_with_no_window_is_found_when_asked() {
        let (model, rectangles) = with_bare_room();
        let evaluation = evaluate(
            model,
            rectangles,
            parameters(vec![("empty_numerator_finding", boolean(true))]),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "bare".into(),
                "no numerator object is reached via opens; the ratio is 0".into()
            )]
        );
        // Without it, the empty space fails as a ratio of 0.
        let (model, rectangles) = with_bare_room();
        let evaluation = evaluate(model, rectangles, parameters(vec![]));
        assert_eq!(
            findings(&evaluation),
            [(
                "bare".into(),
                "plan area ratio is 0 (0 m² of 20 m²); required at least 0.125".into()
            )]
        );
    }

    #[test]
    fn facade_measurement_does_not_combine_with_light_areas() {
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.2, 1.0);
        let evaluation = evaluate(
            model,
            rectangles,
            parameters(vec![("measure", string("facade"))]),
        );
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        // `footprint`, the default, stated explicitly, combines.
        let (model, rectangles) = rooms(&["w1"]);
        let model = sized(model, "w1", 1.2, 1.0);
        let evaluation = evaluate(
            model,
            rectangles,
            parameters(vec![("measure", string("footprint"))]),
        );
        assert_eq!(flagged(&evaluation), ["room"]);
    }

    #[test]
    fn light_area_parameters_need_the_mode_and_a_fallback() {
        let (model, rectangles) = rooms(&[]);
        let evaluation = evaluate(
            model,
            rectangles,
            vec![
                ("numerator_selector", selector(kind("window"))),
                ("frame_width", quantity(50.0, "mm")),
                ("minimum", number(0.125)),
            ],
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        let (model, rectangles) = rooms(&[]);
        let mut no_fallback = parameters(vec![]);
        no_fallback.retain(|(name, _)| !matches!(*name, "frame_width" | "light_area_table"));
        let evaluation = evaluate(model, rectangles, no_fallback);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

mod plan_coverage {
    use super::*;

    const ID: &str = "axioval:capability.plan-coverage";

    /// Compartment `c1` spans x 0..10, `c2` x 10..20. `inside` sits in c1,
    /// `straddling` half in each, `outside` beyond both.
    fn plan() -> (Model, Rectangles) {
        let model = Model::default()
            .object("c1", "compartment")
            .object("c2", "compartment")
            .object("inside", "space")
            .object("straddling", "space")
            .object("outside", "space");
        let rectangles = Rectangles::default()
            .with("c1", [0.0, 0.0, 10.0, 10.0], 0.0)
            .with("c2", [10.0, 0.0, 20.0, 10.0], 0.0)
            .with("inside", [1.0, 1.0, 4.0, 4.0], 0.0)
            .with("straddling", [8.0, 0.0, 12.0, 2.0], 0.0)
            .with("outside", [30.0, 0.0, 32.0, 2.0], 0.0);
        (model, rectangles)
    }

    #[test]
    fn a_space_must_lie_mostly_within_one_compartment() {
        let (model, rectangles) = plan();
        let evaluation = run(
            model,
            rectangles,
            &PlanCoverage,
            &rule(
                ID,
                kind("space"),
                vec![
                    ("candidate_selector", selector(kind("compartment"))),
                    ("minimum_ratio", number(0.9)),
                ],
            ),
        );
        assert_eq!(flagged(&evaluation), ["outside", "straddling"]);
        let straddling = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id().unwrap().local_id == "straddling")
            .unwrap();
        assert_eq!(
            straddling.message,
            "at most 0.5 of the footprint lies within any candidate; required 0.9"
        );
        let outside = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id().unwrap().local_id == "outside")
            .unwrap();
        assert!(
            outside.message.starts_with("at most 0 of"),
            "{}",
            outside.message
        );
    }

    /// The largest measured plan overlap over the measured footprint area
    /// reaches the capability's verdict for every space.
    #[test]
    fn the_measured_overlap_share_reaches_the_verdicts() {
        let (model, rectangles) = plan();
        let project = model.project();
        let mut services = axioval_engine::ServiceRegistry::new();
        services
            .register(PlanAreaServiceHandle::new(Arc::new(rectangles)))
            .unwrap();
        let (judged, judged_rectangles) = plan();
        let evaluation = run(
            judged,
            judged_rectangles,
            &PlanCoverage,
            &rule(
                ID,
                kind("space"),
                vec![
                    ("candidate_selector", selector(kind("compartment"))),
                    ("minimum_ratio", number(0.9)),
                ],
            ),
        );
        for space in ["inside", "straddling", "outside"] {
            let read = |name: &str| {
                common::measured(&services, &project, &id(space), name)
                    .unwrap()
                    .unwrap()
            };
            let (overlap, area) = (read("plan_overlap;with=compartment"), read("area"));
            let share = (overlap.0 / area.1, overlap.1 / area.0);
            assert_eq!(
                common::at_least(share, 0.9),
                Some(!flagged(&evaluation).contains(&space.to_owned())),
                "{space}"
            );
        }
    }

    #[test]
    fn the_ratio_must_be_a_share() {
        let (model, rectangles) = plan();
        let evaluation = run(
            model,
            rectangles,
            &PlanCoverage,
            &rule(
                ID,
                kind("space"),
                vec![
                    ("candidate_selector", selector(kind("compartment"))),
                    ("minimum_ratio", number(1.5)),
                ],
            ),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

mod plan_area_range {
    use super::*;
    use axioval_ir::contract::{ComparisonOperator, Selector};
    use axioval_rules::PlanAreaRange;

    const ID: &str = "axioval:capability.plan-area";

    #[test]
    fn each_space_area_lies_within_the_range_boundaries_included() {
        let model = Model::default()
            .object("large", "space")
            .object("small", "space")
            .object("tiny", "space")
            .object("bodiless", "space");
        let rectangles = Rectangles::default()
            .with("large", [0.0, 0.0, 4.0, 5.0], 0.0)
            .with("small", [0.0, 0.0, 3.0, 2.0], 0.0)
            .with("tiny", [0.0, 0.0, 2.0, 2.0], 0.0)
            // A declared bodiless object measures an exactly empty footprint.
            .with("bodiless", [0.0, 0.0, 0.0, 0.0], 0.0);
        let evaluation = run(
            model,
            rectangles,
            &PlanAreaRange,
            &rule(
                ID,
                kind("space"),
                vec![("minimum", number(6.0)), ("maximum", number(20.0))],
            ),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "tiny".into(),
                "plan area is 4 m²; required at least 6 m²".into()
            )]
        );
        assert_eq!(
            unevaluated(&evaluation),
            [(
                "bodiless".to_owned(),
                NotEvaluatedReason::IncompleteEvidence
            )]
        );
    }

    #[test]
    fn a_tessellated_area_straddling_a_bound_is_not_evaluated() {
        let model = Model::default()
            .object("straddling", "space")
            .object("beyond", "space");
        let rectangles = Rectangles::default()
            .with("straddling", [0.0, 0.0, 5.0, 4.0], 1.0)
            .with("beyond", [0.0, 0.0, 5.0, 5.0], 1.0);
        let evaluation = run(
            model,
            rectangles,
            &PlanAreaRange,
            &rule(ID, kind("space"), vec![("maximum", number(20.0))]),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "beyond".into(),
                "plan area is between 24 and 26 m²; required at most 20 m²".into()
            )]
        );
        assert!(!evaluation.findings()[0].evidence[0].exact);
        assert_eq!(
            unevaluated(&evaluation),
            [(
                "straddling".to_owned(),
                NotEvaluatedReason::IncompleteEvidence
            )]
        );
    }

    /// Storey `a` holds 26 m² of spaces, `b` 30 m², `c` 23 to 27 m², `d` a
    /// bodiless space. Storey `a`'s slab is not a member.
    fn storeys() -> (Model, Rectangles) {
        let model = Model::default()
            .object("a", "storey")
            .object("b", "storey")
            .object("c", "storey")
            .object("d", "storey")
            .object("slab", "slab")
            .object("a1", "space")
            .object("a2", "space")
            .object("b1", "space")
            .object("c1", "space")
            .object("d1", "space")
            .edge("contains", "a", "slab")
            .edge("contains", "a", "a1")
            .edge("contains", "a", "a2")
            .edge("contains", "b", "b1")
            .edge("contains", "c", "c1")
            .edge("contains", "d", "d1");
        let rectangles = Rectangles::default()
            .with("slab", [0.0, 0.0, 10.0, 10.0], 0.0)
            .with("a1", [0.0, 0.0, 4.0, 5.0], 0.0)
            .with("a2", [4.0, 0.0, 7.0, 2.0], 0.0)
            .with("b1", [0.0, 0.0, 6.0, 5.0], 0.0)
            .with("c1", [0.0, 0.0, 5.0, 5.0], 2.0)
            .with("d1", [0.0, 0.0, 0.0, 0.0], 0.0);
        (model, rectangles)
    }

    fn members(member: Selector, maximum: f64) -> Vec<(&'static str, ParameterValue)> {
        vec![
            ("member_selector", selector(member)),
            ("maximum", number(maximum)),
            ("relationship", string("contains")),
        ]
    }

    #[test]
    fn the_space_area_of_each_storey_is_summed_and_bounded() {
        let (model, rectangles) = storeys();
        let evaluation = run(
            model,
            rectangles,
            &PlanAreaRange,
            &rule(ID, kind("storey"), members(kind("space"), 26.0)),
        );
        assert_eq!(
            findings(&evaluation),
            [(
                "b".into(),
                "summed plan area of the members is 30 m²; required at most 26 m²".into()
            )]
        );
        assert_eq!(evaluation.findings()[0].related.len(), 1);
        assert_eq!(
            unevaluated(&evaluation),
            [
                ("c".to_owned(), NotEvaluatedReason::IncompleteEvidence),
                ("d".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ]
        );
    }

    #[test]
    fn undecided_members_leave_only_an_excess_standing() {
        let room = Selector::property(
            Some("Pset".into()),
            "IsRoom",
            ComparisonOperator::Exists,
            None,
        );
        let (model, rectangles) = storeys();
        let model = model
            .text("a1", "Pset", "IsRoom", "yes")
            .text("b1", "Pset", "IsRoom", "yes")
            .unreadable("a2")
            .object("b2", "space")
            .edge("contains", "b", "b2")
            .unreadable("b2");
        let rectangles = rectangles.with("b2", [0.0, 0.0, 1.0, 1.0], 0.0);
        let evaluation = run(
            model,
            rectangles,
            &PlanAreaRange,
            &rule(ID, kind("storey"), members(room, 26.0)),
        );
        // `b` already exceeds the maximum; `a` might, with `a2`.
        assert_eq!(flagged(&evaluation), ["b"]);
        assert!(
            unevaluated(&evaluation)
                .contains(&("a".to_owned(), NotEvaluatedReason::IncompleteEvidence))
        );
    }

    #[test]
    fn a_relationship_needs_a_member_selector() {
        let (model, rectangles) = storeys();
        let evaluation = run(
            model,
            rectangles,
            &PlanAreaRange,
            &rule(
                ID,
                kind("storey"),
                vec![
                    ("maximum", number(26.0)),
                    ("relationship", string("contains")),
                ],
            ),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// `area-ratio`'s numerator and denominator as measured footprint areas,
/// summed by aggregates over the same relationship, reach its verdicts as
/// an expression ratio.
mod as_expressions {
    use super::*;
    use axioval_engine::CapabilityRegistry;
    use common::runtime::{
        definitions, entity, plan, rule as package_rule, run, session, snapshot,
    };
    use serde_json::{Value, json};

    const EXPRESSION: &str = "axioval:capability.expression";

    fn summed(kind: &str) -> Value {
        json!({"kind": "aggregate", "function": "sum",
            "over": {"kind": "path", "path": ["contains"]},
            "where": entity(kind),
            "value": {"kind": "property", "propertySet": "axioval:measured", "property": "area"}})
    }

    #[test]
    fn a_storey_short_of_its_space_share_is_found_as_by_area_ratio() {
        let (model, rectangles) = super::area_ratio::storeys_fixture();
        let (judged, judged_rectangles) = super::area_ratio::storeys_fixture();
        let capability = super::run(
            judged,
            judged_rectangles,
            &AreaRatio,
            &common::rule(
                "axioval:capability.area-ratio",
                kind("storey"),
                super::area_ratio::parameters_fixture(0.5),
            ),
        );
        let registry = axioval_rules::register_builtins(CapabilityRegistry::new()).unwrap();
        let package = definitions(
            &registry,
            &[EXPRESSION],
            &["storey", "slab", "space"],
            &[],
            &[],
        );
        let ratio = package_rule(
            "ratio",
            EXPRESSION,
            "error",
            entity("storey"),
            json!({"requirement": {"type": "expression", "value": {
                "kind": "compare", "operator": "greaterThanOrEquals",
                "left": {"kind": "divide", "left": summed("space"), "right": summed("slab")},
                "right": {"kind": "literal", "value": {"type": "number", "value": 0.5}}}}}),
            json!({}),
        );
        let plan = plan(&registry, &package, vec![ratio]).unwrap();
        let session = session(model)
            .with_host_service(
                PlanAreaServiceHandle::new(Arc::new(rectangles)),
                &[snapshot()],
            )
            .unwrap();
        let report = run(registry, plan, &session, |runtime| runtime).unwrap();
        let found: Vec<String> = report.findings().iter().map(common::subject).collect();
        assert_eq!(found, flagged(&capability));
        assert!(
            report.not_evaluated.is_empty(),
            "{:?}",
            report.not_evaluated
        );
    }
}
