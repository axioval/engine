//! Property requirement tables: required, optional and forbidden properties
//! with value conditions, and a distinct result for each way a row fails.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BodyVolume, CapabilityEvaluation, FacadeArea, FacadeAreaError, FacadeAreaService,
    FacadeAreaServiceHandle, GeometryFidelity, ObjectBounds, PlanArea, PlanAreaError,
    PlanAreaService, PlanAreaServiceHandle, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, SourceSnapshot, TypeHierarchyError,
    TypeHierarchyService, TypeHierarchyServiceHandle, VolumeInterval,
};
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::PropertyRequirements;
use common::{
    Model, boolean, findings, id, integer, kind, number, property, rule, selector, source, string,
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
            "missing property set: Pset_WallCommon is absent (requirement row 0)".into()
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
fn a_name_pattern_holds_for_every_matching_property() {
    // Both sets match `Pset_*Common`: each `Reference` must hold a value.
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Pset_WallCommon", "Reference", "W-01")
        .text("w1", "Pset_ConcreteCommon", "Reference", "C-01")
        .text("w2", "Pset_WallCommon", "Reference", "W-02")
        .text("w2", "Pset_ConcreteCommon", "Reference", " ")
        .text("w3", "Pset_Other", "Reference", "O-03");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_*Common")),
            ("property", string("Reference")),
            ("requirement", string("required")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "missing value: Pset_ConcreteCommon.Reference is ` ` (requirement row 0)".into()
            ),
            (
                "w3".into(),
                "missing property set: Pset_*Common is absent (requirement row 0)".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn xml_schema_patterns_name_properties_and_values_hold_for_each() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Foo_Bar", "Foobar", "x")
        .text("w1", "Foo_Bar", "Foobaz", "x")
        .text("w2", "Foo_Bar", "Foobar", "x")
        .text("w2", "Foo_Bar", "Foobaz", "y")
        .text("w3", "Foo_Bar", "Other", "x");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set_pattern", string("Foo_.*")),
            ("property_pattern", string("Foo.*")),
            ("requirement", string("required")),
            ("one_of", string("x")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "wrong value: Foo_Bar.Foobaz is `y`; required one of `x` (requirement row 0)"
                    .into()
            ),
            (
                "w3".into(),
                "missing property: /Foo_.*/./Foo.*/ is absent (requirement row 0)".into()
            ),
        ]
    );
}

#[test]
fn a_forbidden_pattern_is_decided_from_the_enumeration() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .text("w1", "Pset_Draft", "NoteA", "temporary")
        .value("w2", "Pset_Draft", "NoteB", PropertyValue::Null)
        .text("w3", "Pset_Final", "Note", "kept");
    let forbidden = |presence: &str| {
        run(
            Model::default()
                .object("w1", "wall")
                .object("w2", "wall")
                .object("w3", "wall")
                .text("w1", "Pset_Draft", "NoteA", "temporary")
                .value("w2", "Pset_Draft", "NoteB", PropertyValue::Null)
                .text("w3", "Pset_Final", "Note", "kept"),
            requirements(vec![row(&[
                ("state", string("exclude")),
                ("property_set", string("Pset_Draft")),
                ("property_pattern", string("Note[A-Z]")),
                ("presence", string(presence)),
            ])]),
        )
    };
    // No matched property may hold a value; a null one does not.
    assert_eq!(
        findings(&forbidden("not-empty")),
        [(
            "w1".into(),
            "forbidden value: Pset_Draft.NoteA is `temporary`, which is not empty (requirement row 0)"
                .into()
        )]
    );
    // None may be there at all.
    assert_eq!(
        findings(&forbidden("defined"))
            .into_iter()
            .map(|(object, _)| object)
            .collect::<Vec<_>>(),
        ["w1", "w2"]
    );
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_Draft")),
            ("requirement", string("forbidden")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "forbidden property set present: Pset_Draft is present with 1 property (requirement row 0)"
                    .into()
            ),
            (
                "w2".into(),
                "forbidden property set present: Pset_Draft is present with 1 property (requirement row 0)"
                    .into()
            ),
        ]
    );
}

#[test]
fn a_set_alone_asks_for_its_presence() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .text("w1", "Pset_WallCommon", "Reference", "W-01");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_WallCommon")),
            ("requirement", string("required")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w2".into(),
            "missing property set: Pset_WallCommon is absent (requirement row 0)".into()
        )]
    );
    let malformed = run(
        Model::default().object("w1", "wall"),
        requirements(vec![row(&[
            ("state", string("include")),
            ("property_set", string("Pset_WallCommon")),
            ("presence", string("not-empty")),
        ])]),
    );
    assert_eq!(
        unevaluated(&malformed),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_missing_property_in_a_present_set_or_an_unlistable_source_stays_a_missing_property() {
    let rows = || {
        requirements(vec![row(&[
            ("property_set", string("Pset_WallCommon")),
            ("property", string("Status")),
            ("requirement", string("required")),
        ])])
    };
    let present_set =
        Model::default()
            .object("w1", "wall")
            .text("w1", "Pset_WallCommon", "Reference", "W-01");
    assert_eq!(
        findings(&run(present_set, rows())),
        [(
            "w1".into(),
            "missing property: Pset_WallCommon.Status is absent (requirement row 0)".into()
        )]
    );
    // A source that cannot list sets keeps the plain result, and cannot
    // decide a pattern.
    let names_only = Model::default().object("w1", "wall").names_only();
    assert_eq!(
        findings(&run(names_only, rows())),
        [(
            "w1".into(),
            "missing property: Pset_WallCommon.Status is absent (requirement row 0)".into()
        )]
    );
    let pattern = run(
        Model::default().object("w1", "wall").names_only(),
        requirements(vec![row(&[
            ("property_set", string("Pset_*")),
            ("property", string("Status")),
            ("requirement", string("required")),
        ])]),
    );
    assert!(findings(&pattern).is_empty());
    assert_eq!(
        unevaluated(&pattern),
        [("w1".into(), NotEvaluatedReason::BackendUnavailable)]
    );
    let both = run(
        Model::default().object("w1", "wall"),
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property_set_pattern", string("Pset")),
            ("property", string("Status")),
            ("requirement", string("required")),
        ])]),
    );
    assert_eq!(
        unevaluated(&both),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
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

/// A `state` row on `Pset.Status` with further cells.
fn state_row(state: &str, cells: &[(&str, ParameterValue)]) -> TableRow {
    let mut row = row(&[
        ("property_set", string("Pset")),
        ("property", string("Status")),
        ("state", string(state)),
    ]);
    row.extend(
        cells
            .iter()
            .map(|(column, cell)| ((*column).to_owned(), cell.clone())),
    );
    row
}

/// Walls with `Pset.Status` absent (w1), null (w2), blank (w3) and `new`
/// (w4).
fn statuses() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("w4", "wall")
        .value("w2", "Pset", "Status", PropertyValue::Null)
        .text("w3", "Pset", "Status", " ")
        .text("w4", "Pset", "Status", "new")
}

fn presence(state: &str, presence: &str) -> Vec<(String, String)> {
    let evaluation = run(
        statuses(),
        requirements(vec![state_row(state, &[("presence", string(presence))])]),
    );
    assert!(unevaluated(&evaluation).is_empty());
    findings(&evaluation)
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(object, message)| {
            (
                (*object).to_owned(),
                format!("{message} (requirement row 0)"),
            )
        })
        .collect()
}

#[test]
fn an_included_presence_must_hold() {
    assert_eq!(
        presence("include", "defined"),
        pairs(&[("w1", "missing property set: Pset is absent")])
    );
    assert_eq!(
        presence("include", "undefined"),
        pairs(&[
            ("w2", "forbidden property present: Pset.Status is null"),
            ("w3", "forbidden property present: Pset.Status is ` `"),
            ("w4", "forbidden property present: Pset.Status is `new`"),
        ])
    );
    assert_eq!(
        presence("include", "empty"),
        pairs(&[
            ("w1", "missing property set: Pset is absent"),
            ("w4", "wrong value: Pset.Status is `new`; required empty"),
        ])
    );
    assert_eq!(
        presence("include", "not-empty"),
        pairs(&[
            ("w1", "missing property set: Pset is absent"),
            ("w2", "missing value: Pset.Status is null"),
            ("w3", "missing value: Pset.Status is ` `"),
        ])
    );
}

#[test]
fn an_excluded_presence_must_not_hold() {
    assert_eq!(
        presence("exclude", "defined"),
        pairs(&[
            ("w2", "forbidden property present: Pset.Status is null"),
            ("w3", "forbidden property present: Pset.Status is ` `"),
            ("w4", "forbidden property present: Pset.Status is `new`"),
        ])
    );
    assert_eq!(
        presence("exclude", "undefined"),
        pairs(&[("w1", "missing property set: Pset is absent")])
    );
    assert_eq!(
        presence("exclude", "empty"),
        pairs(&[
            ("w2", "missing value: Pset.Status is null"),
            ("w3", "missing value: Pset.Status is ` `"),
        ])
    );
    assert_eq!(
        presence("exclude", "not-empty"),
        pairs(&[(
            "w4",
            "forbidden value: Pset.Status is `new`, which is not empty"
        )])
    );
}

#[test]
fn an_ignored_row_is_skipped_even_when_it_could_not_be_checked() {
    let evaluation = run(
        statuses(),
        requirements(vec![
            state_row("ignore", &[("presence", string("defined"))]),
            row(&[
                ("property_set", string("Pset_*")),
                ("property", string("Status")),
                ("state", string("ignore")),
                ("presence", string("defined")),
            ]),
            row(&[
                ("property_set", string("Pset")),
                ("property", string("Status")),
                ("requirement", string("required")),
                ("state", string("ignore")),
            ]),
        ]),
    );
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    // `include` on a requirement row is the row as written.
    let included = run(
        statuses(),
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string("Status")),
            ("requirement", string("required")),
            ("state", string("include")),
        ])]),
    );
    assert_eq!(findings(&included).len(), 3);
}

#[test]
fn included_and_excluded_value_conditions() {
    let model = statuses()
        .text("w1", "Pset", "Status", "Existing")
        .text("w2", "Pset", "Status", "demolished")
        .text("w3", "Pset", "Status", "New*");
    // Wildcard alternatives; `\*` is a literal star.
    let evaluation = run(
        model,
        vec![
            (
                "requirements",
                ParameterValue::Table {
                    value: vec![
                        state_row("include", &[("one_of_like", string(r"exist*|new|New\*"))]),
                        state_row("exclude", &[("value_like", string("demol*"))]),
                    ],
                },
            ),
            ("case_sensitive", boolean(false)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "wrong value: Pset.Status is `demolished`; required like one of `exist*`, \
                 `new`, `New\\*` (requirement row 0)"
                    .into()
            ),
            (
                "w2".into(),
                "forbidden value: Pset.Status is `demolished`, which is like `demol*` \
                 (requirement row 1)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
    // Without the escape the star is a wildcard: `New*` matches `Newer`.
    let escaped = run(
        Model::default()
            .object("w1", "wall")
            .text("w1", "Pset", "Status", "Newer"),
        requirements(vec![state_row(
            "include",
            &[("one_of_like", string(r"New\*"))],
        )]),
    );
    assert_eq!(findings(&escaped).len(), 1);
}

#[test]
fn an_included_condition_needs_a_value_and_an_excluded_one_passes_without() {
    let evaluation = run(
        statuses(),
        requirements(vec![
            state_row("include", &[("one_of", string("new"))]),
            state_row("exclude", &[("one_of", string("new"))]),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "missing property set: Pset is absent (requirement row 0)".into()
            ),
            (
                "w2".into(),
                "missing value: Pset.Status is null (requirement row 0)".into()
            ),
            (
                "w3".into(),
                "missing value: Pset.Status is ` ` (requirement row 0)".into()
            ),
            (
                "w4".into(),
                "forbidden value: Pset.Status is `new`, which is one of `new` \
                 (requirement row 1)"
                    .into()
            ),
        ]
    );
}

#[test]
fn contains_reads_text_as_a_substring_and_a_list_by_element() {
    let list = |values: &[&str]| {
        PropertyValue::List(
            values
                .iter()
                .map(|value| PropertyValue::String((*value).into()))
                .collect(),
        )
    };
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("w4", "wall")
        .object("w5", "wall")
        .text("w1", "Pset", "Status", "fire-rated wall")
        .text("w2", "Pset", "Status", "plain")
        .value("w3", "Pset", "Status", list(&["acoustic", "fire"]))
        // An element that merely contains the text is not the element.
        .value("w4", "Pset", "Status", list(&["fireproof"]))
        .value("w5", "Pset", "Status", PropertyValue::Decimal(1.5));
    let evaluation = run(
        model,
        requirements(vec![
            state_row("include", &[("contains", string("fire"))]),
            state_row("exclude", &[("contains", string("acoustic"))]),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "wrong value: Pset.Status is `plain`; required containing `fire` \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w3".into(),
                "forbidden value: Pset.Status is [`acoustic`, `fire`], which is containing \
                 `acoustic` (requirement row 1)"
                    .into()
            ),
            (
                "w4".into(),
                "wrong value: Pset.Status is [`fireproof`]; required containing `fire` \
                 (requirement row 0)"
                    .into()
            ),
        ]
    );
    // A decimal is neither text nor a list: both rows are not evaluated.
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("w5".into(), NotEvaluatedReason::IncompleteEvidence),
            ("w5".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn a_range_may_round_in_its_own_unit() {
    let width = |decimals: Option<i64>| {
        let mut cells = vec![
            ("property_set", string("Qto")),
            ("property", string("Width")),
            ("requirement", string("required")),
            ("minimum", number(300.0)),
            ("unit", string("mm")),
        ];
        if let Some(decimals) = decimals {
            cells.push(("decimals", integer(decimals)));
        }
        row(&cells)
    };
    let model = || {
        Model::default()
            .object("w1", "wall")
            .object("w2", "wall")
            .value("w1", "Qto", "Width", length(0.2996))
            .value("w2", "Qto", "Width", length(0.2994))
    };
    let exact = run(model(), requirements(vec![width(None)]));
    assert_eq!(findings(&exact).len(), 2);
    let rounded = run(model(), requirements(vec![width(Some(0))]));
    assert_eq!(
        findings(&rounded),
        [(
            "w2".into(),
            "wrong value: Qto.Width is 0.2994 m; required at least 300 mm \
             (rounded to 0 decimal(s)) (requirement row 0)"
                .into()
        )]
    );
}

/// Walls with fire ratings and a discipline to categorise by.
fn ratings() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("w4", "wall")
        .object("w5", "wall")
        .object("w6", "wall")
        .text("w1", "Pset", "FireRating", "F30")
        .text("w2", "Pset", "FireRating", "F30")
        .text("w3", "Pset", "FireRating", "F90")
        .text("w4", "Pset", "FireRating", "F30")
        .text("w5", "Pset", "FireRating", "F15")
        .text("w1", "Pset", "Discipline", "Architecture")
        .text("w2", "Pset", "Discipline", "Structure")
        .text("w4", "Pset", "Discipline", "Architecture")
}

fn rating_rule(extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = requirements(vec![row(&[
        ("property_set", string("Pset")),
        ("property", string("FireRating")),
        ("state", string("include")),
        ("one_of", string("F60|F90")),
    ])]);
    parameters.extend(extra);
    run(ratings(), parameters)
}

#[test]
fn findings_group_by_the_value_found() {
    let evaluation = rating_rule(vec![("group_by_value", boolean(true))]);
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w6".into(),
                "missing property set: Pset is absent on 1 object \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w5".into(),
                "wrong value: Pset.FireRating is `F15` on 1 object; required one of `F60`, \
                 `F90` (requirement row 0)"
                    .into()
            ),
            (
                "w1".into(),
                "wrong value: Pset.FireRating is `F30` on 3 objects; required one of `F60`, \
                 `F90` (requirement row 0)"
                    .into()
            ),
        ]
    );
    // The group relates every other object that found the value.
    assert_eq!(evaluation.findings()[2].related, [id("w2"), id("w4")]);
    assert!(evaluation.findings()[0].related.is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn findings_are_categorised_by_a_property() {
    let evaluation = rating_rule(vec![(
        "category_property",
        property(Some("Pset"), "Discipline"),
    )]);
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "[Architecture] wrong value: Pset.FireRating is `F30`; required one of `F60`, \
                 `F90` (requirement row 0)"
                    .into()
            ),
            (
                "w2".into(),
                "[Structure] wrong value: Pset.FireRating is `F30`; required one of `F60`, \
                 `F90` (requirement row 0)"
                    .into()
            ),
            (
                "w4".into(),
                "[Architecture] wrong value: Pset.FireRating is `F30`; required one of \
                 `F60`, `F90` (requirement row 0)"
                    .into()
            ),
            (
                "w5".into(),
                "wrong value: Pset.FireRating is `F15`; required one of `F60`, `F90` \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w6".into(),
                "missing property set: Pset is absent (requirement row 0)".into()
            ),
        ]
    );
}

#[test]
fn groups_are_split_by_category() {
    let evaluation = rating_rule(vec![
        ("group_by_value", boolean(true)),
        ("category_property", property(Some("Pset"), "Discipline")),
    ]);
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(object, message)| (object, message.split(';').next().unwrap().to_owned()))
            .collect::<Vec<_>>(),
        [
            (
                "w6".into(),
                "missing property set: Pset is absent on 1 object (requirement row 0)".into()
            ),
            (
                "w5".into(),
                "wrong value: Pset.FireRating is `F15` on 1 object".into()
            ),
            (
                "w1".into(),
                "[Architecture] wrong value: Pset.FireRating is `F30` on 2 objects".into()
            ),
            (
                "w2".into(),
                "[Structure] wrong value: Pset.FireRating is `F30` on 1 object".into()
            ),
        ]
    );
    assert_eq!(evaluation.findings()[2].related, [id("w4")]);
}

#[test]
fn an_unreadable_category_leaves_the_object_not_evaluated() {
    let evaluation = run(
        Model::default()
            .object("w1", "wall")
            .text("w1", "Pset", "FireRating", "F30"),
        vec![
            (
                "requirements",
                ParameterValue::Table {
                    value: vec![row(&[
                        ("property_set", string("Pset")),
                        ("property", string("FireRating")),
                        ("state", string("include")),
                        ("one_of", string("F90")),
                    ])],
                },
            ),
            // A category the source cannot resolve: a blank property name.
            ("category_property", property(Some("Pset"), "")),
        ],
    );
    assert!(findings(&evaluation).is_empty());
    assert_eq!(unevaluated(&evaluation)[0].0, "w1");
}

#[test]
fn malformed_state_rows_are_invalid_declarations() {
    let status = || {
        vec![
            ("property_set", string("Pset")),
            ("property", string("Status")),
        ]
    };
    let with = |cells: &[(&'static str, ParameterValue)]| {
        let mut row = status();
        row.extend(cells.iter().cloned());
        row
    };
    for cells in [
        // Neither a requirement nor a state.
        status(),
        with(&[("state", string("maybe"))]),
        with(&[("state", string("include"))]),
        with(&[
            ("state", string("include")),
            ("presence", string("present")),
        ]),
        with(&[
            ("state", string("include")),
            ("presence", string("defined")),
            ("one_of", string("a")),
        ]),
        with(&[
            ("requirement", string("required")),
            ("presence", string("defined")),
        ]),
        with(&[
            ("requirement", string("required")),
            ("state", string("exclude")),
        ]),
        with(&[("state", string("include")), ("contains", string(""))]),
        with(&[("state", string("include")), ("one_of_like", string("a|"))]),
        with(&[("state", string("include")), ("decimals", integer(1))]),
        with(&[
            ("state", string("include")),
            ("maximum", number(1.0)),
            ("decimals", integer(16)),
        ]),
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

#[test]
fn a_bounded_value_must_lie_within_the_range_as_a_whole() {
    let range = |lower: Option<f64>, upper: Option<f64>| PropertyValue::Bounded {
        lower: lower.map(|value| Box::new(length(value))),
        upper: upper.map(|value| Box::new(length(value))),
        set_point: None,
    };
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .value("w1", "Pset", "Span", range(Some(1.0), Some(5.0)))
        .value("w2", "Pset", "Span", range(Some(1.0), None))
        .value("w3", "Pset", "Span", range(Some(1.0), Some(7.0)));
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string("Span")),
            ("requirement", string("required")),
            ("minimum", number(0.5)),
            ("maximum", number(6.0)),
            ("unit", string("m")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "wrong value: Pset.Span is [range from 1 m, open above]; required between 0.5 and 6 m (requirement row 0)"
                    .into()
            ),
            (
                "w3".into(),
                "wrong value: Pset.Span is [range from 1 m to 7 m]; required between 0.5 and 6 m (requirement row 0)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_required_pattern_needs_a_match_in_every_named_set() {
    let model = Model::default()
        .object("w1", "wall")
        .text("w1", "Pset_WallCommon", "Reference", "W-01")
        .text("w1", "Pset_ConcreteCommon", "Grade", "C30");
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset_*Common")),
            ("property", string("Reference")),
            ("requirement", string("required")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "missing property: Pset_ConcreteCommon.Reference is absent (requirement row 0)".into()
        )]
    );
}

#[test]
fn an_exclusive_bound_fails_its_own_value() {
    let model = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .value("w1", "Pset", "Load", PropertyValue::Decimal(0.0))
        .value("w2", "Pset", "Load", PropertyValue::Decimal(1.0))
        .value("w3", "Pset", "Load", PropertyValue::Decimal(5.0));
    let evaluation = run(
        model,
        requirements(vec![row(&[
            ("property_set", string("Pset")),
            ("property", string("Load")),
            ("requirement", string("required")),
            ("minimum", number(0.0)),
            ("minimum_exclusive", boolean(true)),
            ("maximum", number(5.0)),
            ("maximum_exclusive", boolean(true)),
        ])]),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w1".into(),
                "wrong value: Pset.Load is 0; required more than 0 and less than 5 \
                 (requirement row 0)"
                    .into()
            ),
            (
                "w3".into(),
                "wrong value: Pset.Load is 5; required more than 0 and less than 5 \
                 (requirement row 0)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

fn date(text: &str) -> PropertyValue {
    PropertyValue::Date(text.parse().unwrap())
}

fn date_cell(text: &str) -> ParameterValue {
    ParameterValue::Date {
        value: text.parse().unwrap(),
    }
}

fn inspections() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("w4", "wall")
        .value("w1", "Pset", "Inspected", date("2019-12-31"))
        .value("w2", "Pset", "Inspected", date("2020-01-01"))
        .value(
            "w3",
            "Pset",
            "Inspected",
            PropertyValue::DateTime("2020-06-01T10:00:00Z".parse().unwrap()),
        )
        .value("w4", "Pset", "Inspected", PropertyValue::Decimal(2020.0))
}

fn dated(extra: &[(&str, ParameterValue)]) -> TableRow {
    let mut cells = vec![
        ("property_set", string("Pset")),
        ("property", string("Inspected")),
        ("requirement", string("required")),
        ("minimum_date", date_cell("2020-01-01")),
    ];
    cells.extend_from_slice(extra);
    row(&cells)
}

#[test]
fn a_date_row_bounds_dates_by_day() {
    let evaluation = run(inspections(), requirements(vec![dated(&[])]));
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "wrong value: Pset.Inspected is 2019-12-31; required at least 2020-01-01 \
             (requirement row 0)"
                .into()
        )]
    );
    // A date-time compares with a date bound only by its day, and a number
    // never compares with a date.
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("w3".into(), NotEvaluatedReason::IncompleteEvidence),
            ("w4".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    let evaluation = run(
        inspections(),
        requirements(vec![dated(&[
            ("minimum_exclusive", boolean(true)),
            ("precision", string("day")),
        ])]),
    );
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(object, _)| object)
            .collect::<Vec<_>>(),
        ["w1", "w2"]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("w4".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// Certified body volumes and largest plane faces per object.
#[derive(Default)]
struct Bodies {
    volumes: BTreeMap<ObjectId, (f64, f64)>,
    faces: BTreeMap<ObjectId, f64>,
}

impl ProximityService for Bodies {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }
    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }
    fn measure_body_volume(&self, object: &ObjectId) -> Result<BodyVolume, ProximityError> {
        let (lower, upper) = *self
            .volumes
            .get(object)
            .ok_or(ProximityError::Unavailable)?;
        let fidelity = if lower < upper {
            GeometryFidelity::Tessellated {
                chord_deviation_metres: 0.01,
            }
        } else {
            GeometryFidelity::Exact
        };
        let mut evidence = Evidence::exact(source(), format!("volume:{object}"));
        evidence.exact = fidelity.is_exact();
        BodyVolume::try_new(
            object.clone(),
            VolumeInterval::try_new(lower, upper)?,
            fidelity,
            evidence,
        )
    }
}

impl FacadeAreaService for Bodies {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        Err(FacadeAreaError::UnknownObject(object.clone()))
    }
    fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let area = *self
            .faces
            .get(object)
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        FacadeArea::try_new(
            object.clone(),
            area,
            area,
            Evidence::exact(source(), format!("face:{object}")),
        )
    }
}

fn masses() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .value("w1", "Pset", "Mass", PropertyValue::Decimal(1000.0))
        .value("w2", "Pset", "Mass", PropertyValue::Decimal(1000.0))
        .value("w3", "Pset", "Mass", PropertyValue::Decimal(1000.0))
}

fn divided(per: &str) -> CapabilityEvaluation {
    let mut bodies = Bodies::default();
    bodies.volumes.insert(id("w1"), (1.0, 1.0));
    bodies.volumes.insert(id("w2"), (4.0, 4.0));
    // 1000 / [1.9, 2.1] straddles 500.
    bodies.volumes.insert(id("w3"), (1.9, 2.1));
    bodies.faces.insert(id("w1"), 10.0);
    bodies.faces.insert(id("w2"), 1.0);
    let bodies = Arc::new(bodies);
    masses().evaluate_with(
        &PropertyRequirements,
        &rule(
            ID,
            kind("wall"),
            requirements(vec![row(&[
                ("property_set", string("Pset")),
                ("property", string("Mass")),
                ("requirement", string("required")),
                ("maximum", number(500.0)),
                ("per", string(per)),
            ])]),
        ),
        move |services| {
            services
                .register(ProximityServiceHandle::new(bodies.clone()))
                .unwrap();
            services
                .register(FacadeAreaServiceHandle::new(bodies))
                .unwrap();
        },
    )
}

#[test]
fn a_range_divided_by_the_measured_volume_or_face_area() {
    let evaluation = divided("measured-volume");
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            "wrong value: Pset.Mass is 1000 (1000 per m³ of measured volume); required at most \
             500 per m³ of measured volume (requirement row 0)"
                .into()
        )]
    );
    // A straddling quotient is not evaluated.
    assert_eq!(
        unevaluated(&evaluation),
        [("w3".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let evaluation = divided("measured-face-area");
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(object, _)| object)
            .collect::<Vec<_>>(),
        ["w2"]
    );
    // w3 has no measured face: the service refuses, so it is not evaluated.
    assert_eq!(
        unevaluated(&evaluation),
        [("w3".into(), NotEvaluatedReason::BackendUnavailable)]
    );
}

#[test]
fn malformed_bounds_are_invalid_declarations() {
    for extra in [
        vec![("minimum_exclusive", boolean(true))],
        vec![
            ("maximum", number(1.0)),
            ("minimum_exclusive", boolean(true)),
        ],
        vec![
            ("minimum", number(1.0)),
            ("maximum", number(1.0)),
            ("maximum_exclusive", boolean(true)),
        ],
        vec![
            ("minimum", number(1.0)),
            ("minimum_date", date_cell("2020-01-01")),
        ],
        vec![
            ("minimum_date", date_cell("2021-01-01")),
            ("maximum_date", date_cell("2020-01-01")),
        ],
        vec![("precision", string("day"))],
        vec![
            ("minimum_date", date_cell("2020-01-01")),
            ("precision", string("hour")),
        ],
        vec![("maximum", number(1.0)), ("per", string("measured-mass"))],
    ] {
        let mut cells = vec![
            ("property", string("A")),
            ("requirement", string("required")),
        ];
        cells.extend(extra);
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
