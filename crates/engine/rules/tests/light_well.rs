//! `light-well`: stacked spaces contiguous, with a section large and wide
//! enough for the well's height.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ElevationInterval, PlanLength, PlanSection, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::LightWell;
use common::{Model, findings, id, kind, number, rule, source, strings, unevaluated};

const ID: &str = "axioval:capability.light-well";

/// A closed interval, lower bound first.
type Span = (f64, f64);

/// Elevations per member, and one section answer for any stack.
struct Well {
    extents: BTreeMap<ObjectId, (Span, Span)>,
    /// `(area, width)`; an area of zero is an empty section.
    section: (Span, Span),
}

fn exact((lower, upper): (f64, f64)) -> bool {
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    exact
}

impl VerticalExtentService for Well {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (bottom, top) = self
            .extents
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom.0, bottom.1)?,
            ElevationInterval::try_new(top.0, top.1)?,
            Evidence {
                source: source(),
                locator: format!("extent:{object}"),
                exact: exact(*bottom) && exact(*top),
            },
        )
    }
}

impl PlanSpanService for Well {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_section(&self, objects: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
        let (area, width) = self.section;
        let evidence = |locator: &str, exact| Evidence {
            source: source(),
            locator: locator.into(),
            exact,
        };
        let sides = (area.1 > 0.0).then(|| {
            (
                PlanLength::try_new(width.0, width.1, evidence("width", exact(width))).unwrap(),
                PlanLength::try_new(10.0, 10.0, evidence("length", true)).unwrap(),
            )
        });
        PlanSection::try_new(objects.to_vec(), area, sides, evidence("area", exact(area)))
    }
}

fn row(cells: &[(&str, f64)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), number(*value)))
        .collect()
}

/// Up to 7 m high: 4 m² and 1.5 m wide; any higher: 8 m² and 2.5 m wide.
fn requirements() -> ParameterValue {
    ParameterValue::Table {
        value: vec![
            row(&[
                ("maximum_height_metres", 7.0),
                ("minimum_area_square_metres", 4.0),
                ("minimum_width_metres", 1.5),
            ]),
            row(&[
                ("minimum_area_square_metres", 8.0),
                ("minimum_width_metres", 2.5),
            ]),
        ],
    }
}

/// Zone `well` groups the spaces given as `(local, bottom, top)`.
fn evaluate(
    members: &[(&str, f64, f64)],
    section: (Span, Span),
    tolerance: Option<f64>,
) -> axioval_engine::CapabilityEvaluation {
    let mut model = Model::default().object("well", "zone");
    let mut extents = BTreeMap::new();
    for (local, bottom, top) in members {
        model = model.object(local, "space").edge("groups", "well", local);
        extents.insert(id(local), ((*bottom, *bottom), (*top, *top)));
    }
    let service = Arc::new(Well { extents, section });
    let mut parameters = vec![
        ("member_path", strings(&["groups:forward"])),
        ("requirements", requirements()),
    ];
    if let Some(tolerance) = tolerance {
        parameters.push(("gap_tolerance_metres", number(tolerance)));
    }
    let rule = rule(ID, kind("zone"), parameters);
    model.evaluate_with(&LightWell, &rule, move |services| {
        services
            .register(VerticalExtentServiceHandle::new(service.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(service))
            .unwrap();
    })
}

fn point(value: f64) -> (f64, f64) {
    (value, value)
}

const STACK: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.0, 6.0), ("f2", 6.0, 9.0)];

#[test]
fn a_contiguous_well_large_enough_for_its_height_passes() {
    let outcome = evaluate(STACK, (point(9.0), point(2.8)), None);
    assert!(findings(&outcome).is_empty(), "{:?}", findings(&outcome));
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// Two storeys (6 m) select the first row; three (9 m) the second.
#[test]
fn the_row_follows_the_height_of_the_well() {
    let low = evaluate(&STACK[..2], (point(5.0), point(2.0)), None);
    assert!(findings(&low).is_empty(), "{:?}", findings(&low));
    let high = evaluate(STACK, (point(5.0), point(2.0)), None);
    let found = findings(&high);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0]
            .1
            .starts_with("the well's section area is 5 m²; row 1 requires at least 8 m²"),
        "{found:?}"
    );
    assert!(
        found[1]
            .1
            .starts_with("the well's width is 2 m; row 1 requires at least 2.5 m"),
        "{found:?}"
    );
    assert_eq!(
        high.findings()[0].related,
        vec![id("f1"), id("f2"), id("g")]
    );
}

#[test]
fn a_vertical_gap_or_no_shared_section_breaks_contiguity() {
    let gap: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f2", 6.0, 9.0)];
    let found = findings(&evaluate(gap, (point(9.0), point(2.8)), None));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].1.contains("starts 3 m above the top of"),
        "{found:?}"
    );
    // Within the tolerance a gap is contiguous.
    let slight: &[(&str, f64, f64)] = &[("g", 0.0, 3.0), ("f1", 3.02, 6.0)];
    assert!(findings(&evaluate(slight, (point(9.0), point(2.8)), Some(0.05))).is_empty());
    assert_eq!(
        findings(&evaluate(slight, (point(9.0), point(2.8)), None)).len(),
        1
    );

    let apart = findings(&evaluate(STACK, (point(0.0), point(0.0)), None));
    assert_eq!(apart.len(), 1, "{apart:?}");
    assert!(apart[0].1.contains("share no plan section"), "{apart:?}");
}

#[test]
fn straddling_values_decide_nothing() {
    let outcome = evaluate(STACK, ((7.0, 9.0), (2.4, 2.6)), None);
    assert!(findings(&outcome).is_empty());
    assert_eq!(unevaluated(&outcome).len(), 2);
}

#[test]
fn a_well_without_members_or_with_an_unmeasured_one_is_not_evaluated() {
    // The zone groups nothing; another zone groups the only space.
    let empty = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
        ],
    );
    let outcome = Model::default()
        .object("well", "zone")
        .object("other", "court")
        .object("g", "space")
        .edge("groups", "other", "g")
        .evaluate(&LightWell, &empty);
    assert_eq!(
        unevaluated(&outcome),
        vec![("well".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let rule = rule(
        ID,
        kind("zone"),
        vec![
            ("member_path", strings(&["groups:forward"])),
            ("requirements", requirements()),
        ],
    );
    let outcome = Model::default()
        .object("well", "zone")
        .object("g", "space")
        .edge("groups", "well", "g")
        .evaluate(&LightWell, &rule);
    assert_eq!(
        unevaluated(&outcome),
        vec![("well".to_owned(), NotEvaluatedReason::MissingService)]
    );
}
