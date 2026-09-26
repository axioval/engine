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
            .find(|finding| finding.object_id.local_id == "straddling")
            .unwrap();
        assert_eq!(
            straddling.message,
            "at most 0.5 of the footprint lies within any candidate; required 0.9"
        );
        let outside = evaluation
            .findings()
            .iter()
            .find(|finding| finding.object_id.local_id == "outside")
            .unwrap();
        assert!(
            outside.message.starts_with("at most 0 of"),
            "{}",
            outside.message
        );
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
