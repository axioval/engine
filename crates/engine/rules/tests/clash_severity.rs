//! Clash severities by class and size, and what duplicates differ in.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, GeometryFidelity, IntersectionVolume,
    LengthInterval, NotEvaluatedReason, ObjectBounds, OverlapExtents, ProximityError,
    ProximityEvidence, ProximityRequest, ProximityService, ProximityServiceHandle, VolumeInterval,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, ObjectId, PropertyValue, Severity};
use axioval_rules::{Clash, ClashMatrix};

use common::{Model, kind, number, rule, selector, source, string, unevaluated};

/// One measured pair: how deep, how alike, how far it reaches, and the
/// volumes of the two bodies and of what they share.
#[derive(Clone, Copy)]
struct Pair {
    hausdorff: (f64, f64),
    /// `(lower, upper)` along x, y and z; `None` unmeasured.
    extents: Option<[(f64, f64); 3]>,
    /// shared, subject, counterpart.
    volumes: Option<[f64; 3]>,
}

/// Distinct bodies overlapping 0.05 m deep, reaching `smallest` along z and
/// 0.5 m in plan.
fn intersection(smallest: (f64, f64)) -> Pair {
    Pair {
        hausdorff: (1.0, 1.0),
        extents: Some([(0.5, 0.5), (0.5, 0.5), smallest]),
        volumes: None,
    }
}

/// Two copies within a millimetre of each other.
fn copies(subject: f64, counterpart: f64) -> Pair {
    Pair {
        hausdorff: (0.0, 0.001),
        extents: Some([(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]),
        volumes: Some([subject.min(counterpart), subject, counterpart]),
    }
}

struct Stub(BTreeMap<String, Pair>);

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let x = if object.local_id == "a" { 0.0 } else { 0.5 };
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let pair = self.0[&request.counterpart().local_id];
        let measured = ProximityEvidence::try_new(
            request.clone(),
            0.0,
            Some(0.05),
            Some(0.0),
            None,
            GeometryFidelity::Exact,
            Evidence::exact(source(), "proximity"),
        )?
        .with_hausdorff(LengthInterval::try_new(pair.hausdorff.0, pair.hausdorff.1).unwrap())?;
        let measured = match pair.extents {
            Some(axes) => {
                let [along_x, along_y, along_z] =
                    axes.map(|(lower, upper)| LengthInterval::try_new(lower, upper).unwrap());
                measured.with_overlap_extents(OverlapExtents::new(along_x, along_y, along_z))?
            }
            None => measured,
        };
        match pair.volumes {
            Some(volumes) => {
                let [shared, subject, counterpart] =
                    volumes.map(|volume| VolumeInterval::exact(volume).unwrap());
                measured.with_intersection_volume(IntersectionVolume::try_new(
                    shared,
                    subject,
                    counterpart,
                )?)
            }
            None => Ok(measured),
        }
    }
}

/// Subject `a` (a duct) against counterpart `b` of `kind`.
fn model(counterpart: &str) -> Model {
    Model::default()
        .object("a", "duct")
        .object("b", counterpart)
}

fn table(rows: &[&[(&str, ParameterValue)]]) -> ParameterValue {
    ParameterValue::Table {
        value: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(column, value)| ((*column).to_owned(), value.clone()))
                    .collect::<TableRow>()
            })
            .collect(),
    }
}

fn clash(extra: Vec<(&str, ParameterValue)>) -> CompiledRule {
    let mut parameters = vec![
        ("counterparts", selector(kind("wall"))),
        ("penetration_tolerance_metres", number(0.0)),
        ("duplicate_tolerance_metres", number(0.005)),
    ];
    parameters.extend(extra);
    rule("axioval:capability.clash", kind("duct"), parameters)
}

fn run(model: Model, pair: Pair, rule: &CompiledRule) -> CapabilityEvaluation {
    let stub = Arc::new(Stub(BTreeMap::from([("b".to_owned(), pair)])));
    common::clash_held(model, &Clash, rule, |services| {
        services
            .register(ProximityServiceHandle::new(stub.clone()))
            .unwrap();
    })
}

fn only(evaluation: &CapabilityEvaluation) -> (Severity, String) {
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", evaluation.findings());
    };
    (finding.severity.clone(), finding.message.clone())
}

fn by_class() -> (&'static str, ParameterValue) {
    (
        "severity_by_class",
        table(&[
            &[
                ("class", string("duplicate")),
                ("severity", string("error")),
            ],
            &[
                ("class", string("intersection")),
                ("severity", string("info")),
            ],
        ]),
    )
}

#[test]
fn a_duplicate_gets_its_class_severity() {
    let rule = clash(vec![by_class()]);
    let (severity, message) = only(&run(model("wall"), copies(1.0, 1.0), &rule));
    assert_eq!(severity, Severity::Error);
    assert!(message.starts_with("duplicate of"), "{message}");

    let (severity, _) = only(&run(model("wall"), intersection((0.3, 0.3)), &rule));
    assert_eq!(severity, Severity::Info);

    // A class the table does not name keeps the rule's severity.
    let rule = clash(vec![(
        "severity_by_class",
        table(&[&[
            ("class", string("containment")),
            ("severity", string("info")),
        ]]),
    )]);
    let (severity, _) = only(&run(model("wall"), intersection((0.3, 0.3)), &rule));
    assert_eq!(severity, Severity::Error);
}

fn graded(measure: &str, bounds: [f64; 2]) -> CompiledRule {
    clash(vec![
        by_class(),
        ("grade_by", string(measure)),
        (
            "severity_grades",
            table(&[
                &[
                    ("above", number(bounds[0])),
                    ("severity", string("warning")),
                ],
                &[("above", number(bounds[1])), ("severity", string("error"))],
            ]),
        ),
    ])
}

/// A 10 mm sliver is graded below a 300 mm intersection.
#[test]
fn intersections_are_graded_by_their_smallest_extent() {
    let rule = graded("smallest_extent", [0.025, 0.2]);
    let (severity, message) = only(&run(model("wall"), intersection((0.01, 0.01)), &rule));
    assert_eq!(severity, Severity::Info, "{message}");

    let (severity, _) = only(&run(model("wall"), intersection((0.1, 0.1)), &rule));
    assert_eq!(severity, Severity::Warning);

    let (severity, message) = only(&run(model("wall"), intersection((0.3, 0.3)), &rule));
    assert_eq!(severity, Severity::Error);
    assert!(
        message.ends_with(", graded error by its smallest extent of 0.3000 m"),
        "{message}"
    );

    // Straddling a bound, it takes the most severe grade it may reach.
    let (severity, message) = only(&run(model("wall"), intersection((0.1, 0.25)), &rule));
    assert_eq!(severity, Severity::Error);
    assert!(message.contains("the most severe grade"), "{message}");

    // Unmeasured, it may reach any.
    let unmeasured = Pair {
        extents: None,
        ..intersection((0.0, 0.0))
    };
    let (severity, message) = only(&run(model("wall"), unmeasured, &rule));
    assert_eq!(severity, Severity::Error);
    assert!(message.contains("unmeasured"), "{message}");

    // Grades apply to intersections only.
    let (severity, _) = only(&run(model("wall"), copies(1.0, 1.0), &rule));
    assert_eq!(severity, Severity::Error);
}

#[test]
fn intersections_can_be_graded_by_their_volume() {
    let rule = graded("volume", [0.001, 0.1]);
    let sunk = |shared: f64| Pair {
        volumes: Some([shared, 1.0, 1.0]),
        ..intersection((0.3, 0.3))
    };
    let (severity, _) = only(&run(model("wall"), sunk(0.0005), &rule));
    assert_eq!(severity, Severity::Info);
    let (severity, message) = only(&run(model("wall"), sunk(0.2), &rule));
    assert_eq!(severity, Severity::Error);
    assert!(
        message.contains("shared volume of 0.200000 m³"),
        "{message}"
    );
}

/// Copies name what they differ in: their type, their volume, and the
/// quantities the rule compares.
#[test]
fn duplicates_name_the_quantities_that_differ() {
    let (_, message) = only(&run(model("wall"), copies(1.0, 1.0), &clash(vec![])));
    assert!(
        message.ends_with("; the copies differ in type (duct and wall)"),
        "{message}"
    );

    let model = || {
        model("duct")
            .value("a", "Qto", "NetArea", PropertyValue::Decimal(2.0))
            .value("b", "Qto", "NetArea", PropertyValue::Decimal(2.5))
            .value("a", "Qto", "Length", PropertyValue::Decimal(4.0))
            .value("b", "Qto", "Length", PropertyValue::Decimal(4.0))
    };
    let quantities = (
        "duplicate_quantities",
        table(&[
            &[
                ("property_set", string("Qto")),
                ("property", string("NetArea")),
            ],
            &[
                ("property_set", string("Qto")),
                ("property", string("Length")),
            ],
        ]),
    );
    let mut rule = clash(vec![quantities.clone()]);
    rule.parameters
        .insert("counterparts".into(), selector(kind("duct")));
    let evaluation = run(model(), copies(1.0, 1.2), &rule);
    let (_, message) = only(&evaluation);
    assert!(
        message.ends_with(
            "; the copies differ in volume (1.000000 m³ and 1.200000 m³), Qto.NetArea (2 and 2.5)"
        ),
        "{message}"
    );
    // The quantities read are evidence.
    assert_eq!(evaluation.findings()[0].evidence.len(), 5);

    let alike = model().value("b", "Qto", "NetArea", PropertyValue::Decimal(2.0));
    let (_, message) = only(&run(alike, copies(1.0, 1.0), &rule));
    assert!(
        message.ends_with("; the copies agree in type, volume, Qto.NetArea, Qto.Length"),
        "{message}"
    );

    // An unreadable quantity is unknown, never the same.
    let (_, message) = only(&run(model().unreadable("b"), copies(1.0, 1.0), &rule));
    assert!(
        message.ends_with("; whether they differ in Qto.NetArea, Qto.Length is unknown"),
        "{message}"
    );
}

/// A clash matrix cell's own severity wins over the class's; a grade wins
/// over both.
#[test]
fn a_matrix_cell_severity_wins_over_the_class() {
    let matrix = |cell_severity: Option<&str>| {
        let mut cell = vec![("penetration_tolerance_metres", number(0.0))];
        if let Some(severity) = cell_severity {
            cell.push(("severity", string(severity)));
        }
        rule(
            "axioval:capability.clash-matrix",
            kind("duct"),
            vec![
                ("counterparts", selector(kind("wall"))),
                ("cells", table(&[&cell])),
                ("exclude_same_system", common::boolean(false)),
                by_class(),
                ("grade_by", string("smallest_extent")),
                (
                    "severity_grades",
                    table(&[&[("above", number(0.2)), ("severity", string("error"))]]),
                ),
            ],
        )
    };
    let run_matrix = |rule: &CompiledRule, pair: Pair| {
        let stub = Arc::new(Stub(BTreeMap::from([("b".to_owned(), pair)])));
        common::clash_held(model("wall"), &ClashMatrix, rule, |services| {
            services
                .register(ProximityServiceHandle::new(stub.clone()))
                .unwrap();
        })
    };
    let (severity, _) = only(&run_matrix(&matrix(None), intersection((0.1, 0.1))));
    assert_eq!(severity, Severity::Info);
    let (severity, _) = only(&run_matrix(
        &matrix(Some("warning")),
        intersection((0.1, 0.1)),
    ));
    assert_eq!(severity, Severity::Warning);
    let (severity, message) = only(&run_matrix(
        &matrix(Some("warning")),
        intersection((0.3, 0.3)),
    ));
    assert_eq!(severity, Severity::Error);
    assert!(
        message
            .ends_with(", graded error by its smallest extent of 0.3000 m (clash matrix cell 0)"),
        "{message}"
    );
}

#[test]
fn invalid_severities_refuse_the_rule() {
    let grades = |rows: &[&[(&str, ParameterValue)]]| ("severity_grades", table(rows));
    for extra in [
        vec![(
            "severity_by_class",
            table(&[&[("class", string("unmatched")), ("severity", string("info"))]]),
        )],
        vec![(
            "severity_by_class",
            table(&[
                &[("class", string("duplicate")), ("severity", string("info"))],
                &[
                    ("class", string("duplicate")),
                    ("severity", string("error")),
                ],
            ]),
        )],
        vec![grades(&[&[
            ("above", number(0.1)),
            ("severity", string("error")),
        ]])],
        vec![("grade_by", string("smallest_extent"))],
        vec![
            ("grade_by", string("depth")),
            grades(&[&[("above", number(0.1)), ("severity", string("error"))]]),
        ],
        vec![
            ("grade_by", string("volume")),
            grades(&[&[("above", number(-0.1)), ("severity", string("error"))]]),
        ],
        vec![
            ("grade_by", string("volume")),
            grades(&[
                &[("above", number(0.1)), ("severity", string("error"))],
                &[("above", number(0.1)), ("severity", string("info"))],
            ]),
        ],
        vec![
            ("grade_by", string("volume")),
            grades(&[&[("above", number(0.1)), ("severity", string("fatal"))]]),
        ],
    ] {
        let evaluation = run(model("wall"), intersection((0.3, 0.3)), &clash(extra));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            vec![("a".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
