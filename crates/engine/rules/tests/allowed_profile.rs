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

fn angle(degrees: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: degrees.to_radians(),
        dimension: QuantityDimension::PlaneAngle,
    }
}

fn degrees(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "deg".into(),
    }
}

fn run(
    model: Model,
    rows: Vec<TableRow>,
    extra: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![("profiles", ParameterValue::Table { value: rows })];
    parameters.extend(extra);
    model.evaluate(&AllowedProfile, &rule(ID, kind("column"), parameters))
}

/// An asymmetric I-section is judged by both flanges: `width` and
/// `flange_thickness` are the bottom flange's, `top_width` and
/// `top_flange_thickness` the top's.
#[test]
fn an_asymmetric_i_section_with_a_wrong_top_flange_is_found() {
    let section = |model: Model, local: &str, top: f64| {
        member(
            model,
            local,
            "asymmetric-i-shape",
            None,
            &[
                ("BottomFlangeWidth", 0.3),
                ("OverallDepth", 0.5),
                ("WebThickness", 0.01),
                ("BottomFlangeThickness", 0.02),
                ("TopFlangeWidth", top),
                ("TopFlangeThickness", 0.015),
            ],
        )
    };
    let model = section(Model::default(), "good", 0.2);
    let model = section(model, "bad", 0.25);
    let rows = vec![row(
        "asymmetric-i-shape",
        None,
        &[
            ("width", 0.3),
            ("depth", 0.5),
            ("flange_thickness", 0.02),
            ("top_width", 0.2),
            ("top_flange_thickness", 0.015),
        ],
    )];
    assert_eq!(
        findings(&run(model, rows, vec![])),
        [(
            "bad".into(),
            "profile `asymmetric-i-shape` is not an allowed profile; nearest is row 1 \
             (`asymmetric-i-shape`): top_width 0.25 m, allowed 0.2 m"
                .into()
        )]
    );
}

/// Ellipses, trapezia (whose top may be offset either way) and edge radii
/// have their own columns.
#[test]
fn ellipse_trapezium_and_edge_radius_columns_are_read() {
    let model = member(
        Model::default(),
        "ellipse",
        "ellipse",
        None,
        &[("SemiAxis1", 0.2), ("SemiAxis2", 0.1)],
    );
    let model = member(
        model,
        "trapezium",
        "trapezium",
        None,
        &[
            ("BottomXDim", 0.4),
            ("TopXDim", 0.2),
            ("YDim", 0.3),
            ("TopXOffset", -0.05),
        ],
    );
    let model = member(
        model,
        "angle",
        "l-shape",
        None,
        &[("Depth", 0.1), ("Thickness", 0.01), ("EdgeRadius", 0.004)],
    );
    let mut offset = row(
        "trapezium",
        None,
        &[("width", 0.4), ("top_width", 0.2), ("depth", 0.3)],
    );
    offset.insert("top_offset".into(), quantity(-0.05));
    let rows = vec![
        row(
            "ellipse",
            None,
            &[("semi_axis_1", 0.2), ("semi_axis_2", 0.1)],
        ),
        offset,
        row(
            "l-shape",
            None,
            &[("depth", 0.1), ("thickness", 0.01), ("edge_radius", 0.005)],
        ),
    ];
    assert_eq!(
        findings(&run(model, rows, vec![])),
        [(
            "angle".into(),
            "profile `l-shape` is not an allowed profile; nearest is row 3 (`l-shape`): \
             edge_radius 0.004 m, allowed 0.005 m"
                .into()
        )]
    );
}

/// Slopes are plane angles, judged within `angle_tolerance`, never the
/// length tolerance.
#[test]
fn slopes_are_judged_within_the_angle_tolerance() {
    let model = || {
        member(
            Model::default(),
            "u",
            "u-shape",
            None,
            &[("Depth", 0.2), ("FlangeWidth", 0.075)],
        )
        .value("u", BODY_SET, "Profile.FlangeSlope", angle(4.6))
    };
    let mut sloped = row("u-shape", None, &[("depth", 0.2), ("width", 0.075)]);
    sloped.insert("flange_slope".into(), degrees(4.5));
    let rows = vec![sloped];
    let found = findings(&run(model(), rows.clone(), vec![]));
    assert_eq!(found.len(), 1);
    assert!(
        found[0].1.ends_with("flange_slope 4.6°, allowed 4.5°"),
        "{}",
        found[0].1
    );
    let lenient = run(model(), rows, vec![("angle_tolerance", degrees(0.2))]);
    assert!(findings(&lenient).is_empty());
}

/// `match: per_dimension` takes each dimension's allowed values from any
/// row of the type: a width from row 1 with a depth from row 2 fits.
#[test]
fn per_dimension_accepts_any_combination_of_listed_values() {
    let model = || {
        let model = member(
            Model::default(),
            "mixed",
            "rectangle",
            None,
            &[("XDim", 0.2), ("YDim", 0.4)],
        );
        member(
            model,
            "off",
            "rectangle",
            None,
            &[("XDim", 0.25), ("YDim", 0.4)],
        )
    };
    let rows = || {
        vec![
            row("rectangle", None, &[("width", 0.2), ("depth", 0.3)]),
            row("rectangle", None, &[("width", 0.3), ("depth", 0.4)]),
        ]
    };
    let per_dimension = || vec![("match", text("per_dimension"))];
    assert_eq!(
        findings(&run(model(), rows(), per_dimension())),
        [(
            "off".into(),
            "profile `rectangle` is not an allowed profile: width 0.25 m is none of 0.2 m \
             (row 1), 0.3 m (row 2)"
                .into()
        )]
    );
    // Whole rows: the mixed profile fits neither.
    let mut rows_found = findings(&run(model(), rows(), vec![("match", text("rows"))]));
    rows_found.sort();
    assert_eq!(rows_found.len(), 2);
    assert_eq!(rows_found[0].0, "mixed");
}

/// Under `per_dimension` a name that fits no row of the type, or a type no
/// row names, is found; an unknown `match` is an invalid declaration.
#[test]
fn per_dimension_names_types_and_an_unknown_match() {
    let model = || {
        let model = i_shape(Model::default(), "c1", "IPE300", 0.15, 0.3);
        member(model, "c2", "l-shape", None, &[("Depth", 0.1)])
    };
    let mut found = findings(&run(
        model(),
        vec![row("i-shape", Some("HEA*"), &[("width", 0.15)])],
        vec![("match", text("per_dimension"))],
    ));
    found.sort();
    assert_eq!(
        found,
        [
            (
                "c1".into(),
                "profile `i-shape` `IPE300` is not an allowed profile: its name fits none of \
                 rows 1 of its type"
                    .into()
            ),
            (
                "c2".into(),
                "profile `l-shape` is not of an allowed type".into()
            ),
        ]
    );
    let invalid = run(
        model(),
        vec![row("i-shape", None, &[])],
        vec![("match", text("any"))],
    );
    assert!(findings(&invalid).is_empty());
    assert_eq!(
        unevaluated(&invalid),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
