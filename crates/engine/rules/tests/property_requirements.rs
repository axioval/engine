//! Property requirement tables: required, optional and forbidden properties
//! with value conditions, and a distinct result for each way a row fails.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle,
    SourceSnapshot, TypeHierarchyError, TypeHierarchyService, TypeHierarchyServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::PropertyRequirements;
use common::{
    Model, boolean, findings, id, kind, number, property, rule, selector, source, string,
    unevaluated,
};

const ID: &str = "axioval:capability.property-requirements";

/// A row from `(column, cell)` pairs; text cells are strings.
fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, cell)| ((*column).to_owned(), cell.clone()))
        .collect()
}

fn requirements(rows: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![("requirements", ParameterValue::Table { value: rows })]
}

fn run(model: Model, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model.evaluate(&PropertyRequirements, &rule(ID, kind("wall"), parameters))
}

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

#[test]
fn a_required_property_that_is_absent_is_a_missing_property() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .text("w2", "Pset_WallCommon", "Reference", "W-01");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_WallCommon")),
            ("property", string("Reference")),
            ("requirement", string("required")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "missing property: Pset_WallCommon.Reference is absent (requirement row 0)".into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
    // The finding cites the exact absence proof.
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator.starts_with("absent:"))
    );
}

#[test]
fn a_required_property_without_a_value_is_a_missing_value() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .value("w1", "Pset", "Reference", PropertyValue::Null)
        .text("w2", "Pset", "Reference", "  ");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string("Reference")),
            ("requirement", string("required")),
            ("value_like", string("W-*")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "missing value: Pset.Reference is null (requirement row 0)".into()
            ),
            (
                "w2".into(),
                "missing value: Pset.Reference is `  ` (requirement row 0)".into()
            ),
        ]
    );
}

#[test]
fn a_forbidden_property_that_is_present_is_reported_whatever_its_value() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Pset_Draft", "Comment", "temporary")
        .value("w2", "Pset_Draft", "Comment", PropertyValue::Null);
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_Draft")),
            ("property", string("Comment")),
            ("requirement", string("forbidden")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "forbidden property present: Pset_Draft.Comment is `temporary` \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w2".into(),
                "forbidden property present: Pset_Draft.Comment is null (requirement row 0)".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_forbidden_value_is_reported_and_other_values_pass() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Pset", "Status", "demolish")
        .text("w2", "Pset", "Status", "new")
        .value(
            "w3",
            "Pset",
            "Status",
            PropertyValue::List(vec![
                PropertyValue::String("new".into()),
                PropertyValue::String("Temporary".into()),
            ]),
        );
    let evaluation = run(
        model,
        vec![
            (
                "requirements",
                ParameterValue::Table {
                    value: vec![row(&[
                        ("property_set", string("Pset")),
                        ("property", string("Status")),
                        ("requirement", string("forbidden")),
                        ("one_of", string("demolish|temporary")),
                    ])],
                },
            ),
            ("case_sensitive", boolean(false)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "forbidden value: Pset.Status is `demolish`, which is one of `demolish`, \
                 `temporary` (requirement row 0)"
                    .into()
            ),
            (
                "w3".into(),
                "forbidden value: Pset.Status is [`new`, `Temporary`], which is one of \
                 `demolish`, `temporary` (requirement row 0)"
                    .into()
            ),
        ]
    );
}

#[test]
fn a_value_outside_its_conditions_is_a_wrong_value() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Pset", "Reference", "X-01")
        .text("w2", "Pset", "Reference", "W-01")
        .text("w3", "Pset", "Reference", "W-02")
        .text("w1", "Pset", "FireRating", "F90")
        .text("w2", "Pset", "FireRating", "F30")
        .text("w3", "Pset", "FireRating", "F90")
        .value("w1", "Qto", "Width", length(0.24))
        .value("w2", "Qto", "Width", length(0.24))
        .value("w3", "Qto", "Width", length(0.05));
    let evaluation = run(
        model,
        requirements(vec![
            row(&[
                ("property_set", string("Pset")),
                ("property", string("Reference")),
                ("requirement", string("required")),
                ("value_like", string("W-*")),
            ]),
            row(&[
                ("property_set", string("Pset")),
                ("property", string("FireRating")),
                ("requirement", string("required")),
                ("one_of", string("F60|F90")),
            ]),
            row(&[
                ("property_set", string("Qto")),
                ("property", string("Width")),
                ("requirement", string("required")),
                ("minimum", number(100.0)),
                ("maximum", number(300.0)),
                ("unit", string("mm")),
            ]),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "wrong value: Pset.Reference is `X-01`; required like `W-*` \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w2".into(),
                "wrong value: Pset.FireRating is `F30`; \
                 required one of `F60`, `F90` (requirement row 1)"
                    .into()
            ),
            (
                "w3".into(),
                "wrong value: Qto.Width is 0.05 m; required between 100 and 300 mm \
                 (requirement row 2)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn an_optional_property_may_be_absent_but_not_wrong() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w2", "Pset", "Finish", "paint")
        .text("w3", "Pset", "Finish", "gold");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string("Finish")),
            ("requirement", string("optional")),
            ("one_of", string("paint|plaster")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w3".into(),
            "wrong value: Pset.Finish is `gold`; required one \
             of `paint`, `plaster` (requirement row 0)"
                .into()
        )]
    );
}

/// Plan areas per object.
#[derive(Default)]
struct Areas(BTreeMap<ObjectId, (f64, f64)>);

impl PlanAreaService for Areas {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (area, slack) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("footprint:{object}"));
        evidence.exact = slack == 0.0;
        PlanArea::try_new(area - slack, area + slack, evidence)
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }
}

#[test]
fn a_range_divided_by_the_measured_area() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .value("w1", "Pset", "Load", PropertyValue::Decimal(50.0))
        .value("w2", "Pset", "Load", PropertyValue::Decimal(50.0))
        .value("w3", "Pset", "Load", PropertyValue::Decimal(50.0));
    let mut areas = Areas::default();
    areas.0.insert(id("w1"), (10.0, 0.0));
    areas.0.insert(id("w2"), (2.0, 0.0));
    // 50 / [4, 6] straddles 10.
    areas.0.insert(id("w3"), (5.0, 1.0));
    let measured = row(&[
        ("property_set", string("Pset")),
        ("property", string("Load")),
        ("requirement", string("required")),
        ("maximum", number(10.0)),
        ("per", string("measured-area")),
    ]);
    let evaluation = model.evaluate_with(
        &PropertyRequirements,
        &rule(
            ID,
            kind("wall"),
            vec![(
                "requirements",
                ParameterValue::Table {
                    value: vec![measured],
                },
            )],
        ),
        |services| {
            services
                .register(PlanAreaServiceHandle::new(Arc::new(areas)))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w2".into(),
            "wrong value: Pset.Load is 50 (25 per m² of measured plan area); required at most \
             10 per m² of measured plan area (requirement row 0)"
                .into()
        )]
    );
    // w3's quotient straddles the bound.
    assert_eq!(
        unevaluated(&evaluation),
        [("w3".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_stated_area_divides_the_value() {
    let model = Model::default()
        .object("w1", "wall")
        .value("w1", "Pset", "Load", PropertyValue::Decimal(50.0))
        .value(
            "w1",
            "Qto",
            "NetArea",
            PropertyValue::Quantity {
                value: 2.0,
                dimension: QuantityDimension::Area,
            },
        );
    let evaluation = run(
        model,
        vec![
            (
                "requirements",
                ParameterValue::Table {
                    value: vec![row(&[
                        ("property_set", string("Pset")),
                        ("property", string("Load")),
                        ("requirement", string("required")),
                        ("minimum", number(1.0)),
                        ("maximum", number(10.0)),
                        ("per", string("stated-area")),
                    ])],
                },
            ),
            ("area_property", property(Some("Qto"), "NetArea")),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "wrong value: Pset.Load is 50 (25 per m² of stated area); required between 1 and \
             10 per m² of stated area (requirement row 0)"
                .into()
        )]
    );
}

struct Hierarchy(Vec<SourceSnapshot>);

impl TypeHierarchyService for Hierarchy {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, TypeHierarchyError> {
        Ok(kind == ancestor || (kind == "wall-standard-case" && ancestor == "wall"))
    }
}

#[test]
fn a_row_applies_to_an_exact_class_or_to_its_subtypes() {
    let model = Model::default()
        .object("w", "wall")
        .object("s", "wall-standard-case");
    let wall = |include_subtypes| {
        selector(Selector::EntityType {
            object_type: "wall".into(),
            include_subtypes,
        })
    };
    let evaluation = model.evaluate_with(
        &PropertyRequirements,
        &rule(
            ID,
            Selector::All,
            requirements(vec![
                row(&[
                    ("applies_to", wall(false)),
                    ("property", string("Exact")),
                    ("requirement", string("required")),
                ]),
                row(&[
                    ("applies_to", wall(true)),
                    ("property", string("Inherited")),
                    ("requirement", string("required")),
                ]),
            ]),
        ),
        |services| {
            let snapshot = SourceSnapshot::try_new(source(), "r", "sha256:types").unwrap();
            services
                .register(TypeHierarchyServiceHandle::new(Arc::new(Hierarchy(vec![
                    snapshot,
                ]))))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "s".into(),
                "missing property: Inherited is absent (requirement row 1)".into()
            ),
            (
                "w".into(),
                "missing property: Exact is absent (requirement row 0)".into()
            ),
            (
                "w".into(),
                "missing property: Inherited is absent (requirement row 1)".into()
            ),
        ]
    );
}

#[test]
fn name_patterns_and_set_presence_are_refused_while_other_rows_are_checked() {
    // Both sets match `Pset_*Common`, but the property service can only
    // answer exactly named properties, so the row is refused, never passed.
    let model = Model::default()
        .object("w1", "wall")
        .text("w1", "Pset_WallCommon", "Reference", "W-01")
        .text("w1", "Pset_ConcreteCommon", "Reference", "C-01");
    let evaluation = run(
        model,
        requirements(vec![
            row(&[
                ("property_set", string("Pset_*Common")),
                ("property", string("Reference")),
                ("requirement", string("required")),
            ]),
            row(&[
                ("property_set", string("Pset_WallCommon")),
                ("requirement", string("required")),
            ]),
            row(&[
                ("property_set", string("Pset_WallCommon")),
                ("property", string("Status")),
                ("requirement", string("required")),
            ]),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "missing property: Pset_WallCommon.Status is absent (requirement row 2)".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("-".into(), NotEvaluatedReason::MissingService),
            ("-".into(), NotEvaluatedReason::MissingService),
        ]
    );
    let messages: Vec<_> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect();
    assert!(
        messages[0]
            .starts_with("property-requirements row 0: `Pset_*Common.Reference` is a name pattern"),
        "{messages:?}"
    );
    assert!(messages[1].contains("presence"), "{messages:?}");
}

#[test]
fn an_escaped_wildcard_is_an_exact_name() {
    let model = Model::default()
        .object("w1", "wall")
        .text("w1", "Pset", "Size*", "big");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string(r"Size\*")),
            ("requirement", string("forbidden")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "forbidden property present: Pset.Size* is `big` (requirement row 0)".into()
        )]
    );
}

#[test]
fn a_value_the_condition_cannot_judge_is_not_evaluated() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .value("w1", "Qto", "Width", length(0.2))
        .text("w2", "Qto", "Width", "wide")
        .unreadable("w2");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Qto")),
            ("property", string("Width")),
            ("requirement", string("required")),
            ("maximum", number(1.0)),
        ])]),
    );
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("w1".into(), NotEvaluatedReason::IncompleteEvidence),
            ("w2".into(), NotEvaluatedReason::BackendUnavailable),
        ]
    );
}

#[test]
fn malformed_rows_are_invalid_declarations() {
    for cells in [
        vec![("property", string("A")), ("requirement", string("must"))],
        vec![("requirement", string("required"))],
        vec![
            ("property", string("A")),
            ("requirement", string("required")),
            ("minimum", number(2.0)),
            ("maximum", number(1.0)),
        ],
        vec![
            ("property", string("A")),
            ("requirement", string("required")),
            ("unit", string("mm")),
        ],
        vec![
            ("property", string("A")),
            ("requirement", string("required")),
            ("maximum", number(1.0)),
            ("per", string("stated-volume")),
        ],
        vec![
            ("property", string("A")),
            ("requirement", string("required")),
            ("one_of", string("a||b")),
        ],
        vec![
            ("property_set", string("P")),
            ("requirement", string("required")),
            ("value_like", string("x")),
        ],
    ] {
        let evaluation = run(
            Model::default().object("w1", "wall"),
            requirements(vec![row(&cells)]),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
            "{cells:?}"
        );
    }
}
