//! `allowed-profile`: a member's section is a row of a table of allowed
//! profiles, by type, name and dimensions within a tolerance.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{BODY_SET, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::AllowedProfile;
use common::{Model, findings, kind, rule, unevaluated};

const ID: &str = "axioval:capability.allowed-profile";

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

/// A one-item extrusion of `family`, named `name`, with `dimensions` in
/// metres.
fn member(
    model: Model,
    local: &str,
    family: &str,
    name: Option<&str>,
    dimensions: &[(&str, f64)],
) -> Model {
    let mut model = model
        .object(local, "column")
        .value(local, BODY_SET, "Count", PropertyValue::Integer(1))
        .text(local, BODY_SET, "Kind", "extrusion")
        .text(local, BODY_SET, "Profile.Type", family);
    if let Some(name) = name {
        model = model.text(local, BODY_SET, "Profile.Name", name);
    }
    for (parameter, value) in dimensions {
        model = model.value(
            local,
            BODY_SET,
            &format!("Profile.{parameter}"),
            length(*value),
        );
    }
    model
}

fn i_shape(model: Model, local: &str, name: &str, width: f64, depth: f64) -> Model {
    member(
        model,
        local,
        "i-shape",
        Some(name),
        &[
            ("OverallWidth", width),
            ("OverallDepth", depth),
            ("WebThickness", 0.0085),
            ("FlangeThickness", 0.014),
        ],
    )
}

fn quantity(metres: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value: metres * 1000.0,
        unit: "mm".into(),
    }
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::String {
        value: value.into(),
    }
}

fn row(family: &str, name: Option<&str>, dimensions: &[(&str, f64)]) -> TableRow {
    let mut cells = TableRow::new();
    cells.insert("type".into(), text(family));
    if let Some(name) = name {
        cells.insert("name".into(), text(name));
    }
    for (column, value) in dimensions {
        cells.insert((*column).into(), quantity(*value));
    }
    cells
}

fn table() -> ParameterValue {
    ParameterValue::Table {
        value: vec![
            row("i-shape", Some("HEA*"), &[("width", 0.3), ("depth", 0.29)]),
            row(
                "i-shape",
                Some("HEB*"),
                &[("width", 0.3), ("depth", 0.3), ("web_thickness", 0.011)],
            ),
            row("rectangle", None, &[("width", 0.3), ("depth", 0.3)]),
        ],
    }
}

fn check(model: Model, tolerance: Option<f64>) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![("profiles", table())];
    if let Some(tolerance) = tolerance {
        parameters.push(("tolerance", quantity(tolerance)));
    }
    model.evaluate(&AllowedProfile, &rule(ID, kind("column"), parameters))
}

#[test]
fn a_profile_fitting_a_row_within_the_tolerance_passes_and_one_off_is_found() {
    let model = i_shape(Model::default(), "c1", "HEA300", 0.3, 0.2905);
    let model = i_shape(model, "c2", "HEA300", 0.3, 0.295);
    let evaluation = check(model, Some(0.001));
    assert_eq!(
        findings(&evaluation),
        [(
            "c2".into(),
            "profile `i-shape` `HEA300` is not an allowed profile; nearest is row 1 (`i-shape` \
             `HEA*`): depth 0.295 m, allowed 0.29 m within 0.001 m"
                .into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
    // Without a tolerance, 0.5 mm off is off.
    let model = i_shape(Model::default(), "c1", "HEA300", 0.3, 0.2905);
    assert_eq!(findings(&check(model, None)).len(), 1);
}

#[test]
fn the_nearest_row_is_the_one_least_off_whatever_its_name() {
    // HEB outer dimensions, named as an HEA, with an HEA web.
    let model = member(
        Model::default(),
        "c1",
        "i-shape",
        Some("HEA300"),
        &[
            ("OverallWidth", 0.3),
            ("OverallDepth", 0.3),
            ("WebThickness", 0.0085),
            ("FlangeThickness", 0.019),
        ],
    );
    let evaluation = check(model, None);
    assert_eq!(
        findings(&evaluation),
        [(
            "c1".into(),
            "profile `i-shape` `HEA300` is not an allowed profile; nearest is row 2 (`i-shape` \
             `HEB*`): name is not `HEB*`; web_thickness 0.0085 m, allowed 0.011 m"
                .into()
        )]
    );
}

#[test]
fn an_arbitrary_profile_wrong_geometry_and_a_disallowed_type_are_told_apart() {
    let model = member(Model::default(), "arbitrary", "arbitrary-closed", None, &[]);
    let model = member(model, "angle", "l-shape", Some("L100"), &[("Depth", 0.1)]);
    let model = model
        .object("brep", "column")
        .value("brep", BODY_SET, "Count", PropertyValue::Integer(1))
        .text("brep", BODY_SET, "Kind", "brep")
        .object("two", "column")
        .value("two", BODY_SET, "Count", PropertyValue::Integer(2))
        .object("none", "column");
    let mut found = findings(&check(model, None));
    found.sort();
    assert_eq!(
        found,
        [
            (
                "angle".into(),
                "profile `l-shape` `L100` is not of an allowed type".into()
            ),
            (
                "arbitrary".into(),
                "arbitrary profile `arbitrary-closed`: no allowed profile is of its type".into()
            ),
            (
                "brep".into(),
                "wrong geometry: the body is a `brep`, not a swept profile; a single swept \
                 profile is required"
                    .into()
            ),
            (
                "none".into(),
                "wrong geometry: the object has no body; a single swept profile is required".into()
            ),
            (
                "two".into(),
                "wrong geometry: the body has 2 items; a single swept profile is required".into()
            ),
        ]
    );
}

#[test]
fn a_mirrored_profile_is_judged_by_its_parent_and_a_derived_one_is_not_evaluated() {
    let model = Model::default()
        .object("mirrored", "column")
        .value("mirrored", BODY_SET, "Count", PropertyValue::Integer(1))
        .text("mirrored", BODY_SET, "Profile.Type", "mirrored")
        .text("mirrored", BODY_SET, "Profile.Parent.Type", "rectangle")
        .value("mirrored", BODY_SET, "Profile.Parent.XDim", length(0.3))
        .value("mirrored", BODY_SET, "Profile.Parent.YDim", length(0.3))
        .object("derived", "column")
        .value("derived", BODY_SET, "Count", PropertyValue::Integer(1))
        .text("derived", BODY_SET, "Profile.Type", "derived");
    let evaluation = check(model, None);
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("derived".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn an_unreadable_body_leaves_the_member_undecided_and_an_unset_dimension_does_not_fit() {
    // Nothing of c1's body can be read.
    let model =
        member(Model::default(), "c1", "rectangle", None, &[("XDim", 0.3)]).unreadable("c1");
    let evaluation = check(model, None);
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".into(), NotEvaluatedReason::BackendUnavailable)]
    );
    // A dimension the source leaves unset does not fit a row stating it.
    let model = member(Model::default(), "c2", "rectangle", None, &[("XDim", 0.3)]);
    assert_eq!(
        findings(&check(model, None)),
        [(
            "c2".into(),
            "profile `rectangle` is not an allowed profile; nearest is row 3 (`rectangle`): \
             states no depth"
                .into()
        )]
    );
}
