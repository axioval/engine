//! Storey heights measured from geometry: the highest storey from its
//! contents, and spaces against their storey's height.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ElevationInterval, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{
    Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension, ReportValue,
};
use axioval_rules::LevelSpacing;
use common::{
    Model, boolean, findings, flagged, id, kind, property, rule, selector, source, string, strings,
    unevaluated,
};

const ID: &str = "axioval:capability.level-spacing";

/// Bottom, top and an uncertainty added to both, per object.
#[derive(Default)]
struct Extents(BTreeMap<ObjectId, (f64, f64, f64)>);

impl Extents {
    fn with(mut self, local: &str, bottom: f64, top: f64, slack: f64) -> Self {
        self.0.insert(id(local), (bottom, top, slack));
        self
    }
}

impl VerticalExtentService for Extents {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (bottom, top, slack) = *self
            .0
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact = slack == 0.0;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom - slack, bottom + slack)?,
            ElevationInterval::try_new(top - slack, top + slack)?,
            evidence,
        )
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

/// Storeys at 0 and 3 m. The upper storey's wall reaches 6.5 m, so it is
/// 3.5 m high; each storey has one 3 m space.
fn model() -> Model {
    let mut model = Model::default().object("b", "building");
    for (local, elevation) in [("eg", 0.0), ("og", 3.0)] {
        model = model
            .object(local, "storey")
            .edge("aggregates", "b", local)
            .value(
                local,
                "Levels",
                "Elevation",
                PropertyValue::Quantity {
                    value: elevation,
                    dimension: QuantityDimension::Length,
                },
            );
    }
    model
        .object("wall-eg", "wall")
        .object("wall-og", "wall")
        .object("space-eg", "space")
        .object("space-og", "space")
        .edge("contains", "eg", "wall-eg")
        .edge("contains", "og", "wall-og")
        .edge("aggregates", "eg", "space-eg")
        .edge("aggregates", "og", "space-og")
}

fn extents() -> Extents {
    Extents::default()
        .with("wall-eg", 0.0, 3.0, 0.0)
        .with("wall-og", 3.0, 6.5, 0.0)
        .with("space-eg", 0.0, 3.0, 0.0)
        .with("space-og", 3.0, 6.0, 0.0)
}

fn parameters(extra: Vec<(&str, ParameterValue)>) -> Vec<(&str, ParameterValue)> {
    let mut parameters = vec![
        ("member_selector", selector(kind("storey"))),
        ("order", property(Some("Levels"), "Elevation")),
        ("relationship", string("aggregates")),
    ];
    parameters.extend(extra);
    parameters
}

fn run(model: Model, extents: Extents, extra: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model.evaluate_with(
        &LevelSpacing,
        &rule(ID, kind("building"), parameters(extra)),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(extents)))
                .unwrap();
        },
    )
}

fn contents() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("content_path", strings(&["contains"])),
        ("content_selector", selector(kind("wall"))),
    ]
}

#[test]
fn the_highest_level_is_measured_from_its_contents() {
    let mut extra = contents();
    extra.push(("maximum", metres(3.2)));
    let evaluation = run(model(), extents(), extra);
    assert_eq!(
        findings(&evaluation),
        [(
            "og".into(),
            "level height is 3.5 m; required at most 3.2 m".into()
        )]
    );
    assert!(evaluation.findings()[0].related.is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    // Consistency sees the measured height too.
    let mut extra = contents();
    extra.push(("consistent", boolean(true)));
    let evaluation = run(model(), extents(), extra);
    assert_eq!(flagged(&evaluation), ["og"]);
}

#[test]
fn a_measured_height_straddling_a_bound_is_not_evaluated() {
    let mut extra = contents();
    extra.push(("maximum", metres(3.5)));
    let evaluation = run(model(), extents().with("wall-og", 3.0, 6.5, 0.01), extra);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("og".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_highest_level_without_measurable_contents_is_not_evaluated() {
    // No contents of the selected kind.
    let extra = vec![
        ("content_path", strings(&["contains"])),
        ("content_selector", selector(kind("column"))),
        ("maximum", metres(4.0)),
    ];
    let evaluation = run(model(), extents(), extra);
    assert_eq!(
        unevaluated(&evaluation),
        [("og".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // A content the geometry cannot measure.
    let mut extra = contents();
    extra.push(("maximum", metres(4.0)));
    let evaluation = run(model(), Extents::default(), extra);
    assert_eq!(
        unevaluated(&evaluation),
        [("og".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
    // No geometry at all.
    let mut extra = contents();
    extra.push(("maximum", metres(4.0)));
    let evaluation = model().evaluate(
        &LevelSpacing,
        &rule(ID, kind("building"), parameters(extra)),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("og".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

fn spaces(tolerance: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("space_selector", selector(kind("space"))),
        ("space_path", strings(&["aggregates"])),
        ("space_tolerance", metres(tolerance)),
    ]
}

#[test]
fn each_space_must_be_as_high_as_its_level() {
    let evaluation = run(model(), extents(), [contents(), spaces(0.05)].concat());
    assert_eq!(
        findings(&evaluation),
        [(
            "space-og".into(),
            "space height is 3 m and its level's height 3.5 m; they may differ by at most 0.05 m"
                .into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related[0].local_id, "og");

    // Within a wider tolerance both spaces match.
    let evaluation = run(model(), extents(), [contents(), spaces(0.5)].concat());
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_space_whose_height_straddles_the_tolerance_is_not_evaluated() {
    let evaluation = run(
        model(),
        extents().with("space-eg", 0.0, 3.05, 0.01),
        [contents(), spaces(0.05)].concat(),
    );
    assert_eq!(flagged(&evaluation), ["space-og"]);
    assert_eq!(
        unevaluated(&evaluation),
        [(
            "space-eg".to_owned(),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn the_spaces_of_an_unmeasured_highest_level_are_not_judged() {
    // Without contents, only the lower storey has a height.
    let evaluation = run(model(), extents(), spaces(0.05));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("og".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
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
fn every_level_and_space_height_is_reported_in_a_table() {
    let evaluation = run(
        model(),
        extents().with("wall-og", 3.0, 6.5, 0.01),
        [contents(), spaces(0.6)].concat(),
    );
    assert!(evaluation.findings().is_empty());
    let length = |value| ReportValue::exact(value);
    let measured = ReportValue::measured(3.49, 3.51);
    assert_eq!(
        rows(&evaluation, "levels"),
        [
            ("eg".to_owned(), vec![length(0.0), length(3.0)]),
            ("og".to_owned(), vec![length(3.0), measured.clone()]),
        ]
    );
    let level = |local: &str| ReportValue::text(id(local).to_string());
    assert_eq!(
        rows(&evaluation, "spaces"),
        [
            (
                "space-eg".to_owned(),
                vec![level("eg"), length(3.0), length(3.0)]
            ),
            (
                "space-og".to_owned(),
                vec![level("og"), length(3.0), measured]
            ),
        ]
    );
    // Without contents the highest level's height is unknown, and its
    // spaces are not compared, so they have no row.
    let evaluation = run(model(), extents(), spaces(0.05));
    assert_eq!(
        rows(&evaluation, "levels"),
        [
            ("eg".to_owned(), vec![length(0.0), length(3.0)]),
            ("og".to_owned(), vec![length(3.0), ReportValue::Unknown]),
        ]
    );
    assert_eq!(rows(&evaluation, "spaces").len(), 1);
}

#[test]
fn space_and_content_parameters_are_declared_together() {
    let evaluation = run(
        model(),
        extents(),
        vec![("space_selector", selector(kind("space")))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let evaluation = run(
        model(),
        extents(),
        vec![
            ("content_selector", selector(kind("wall"))),
            ("maximum", metres(4.0)),
        ],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
