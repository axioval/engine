//! `stair-geometry` and `ramp-geometry` over measured flights and ramps.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ElevationInterval, Headroom, HeadroomRequest, MeasuredInterval,
    MetricDirection, SlopedRun, SlopedSurface, Tread, TreadFlight, WalkingSurfaceError,
    WalkingSurfaceService, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue};
use axioval_rules::{RampGeometryCheck, StairGeometryCheck};
use common::{Model, boolean, findings, id, kind, number, rule, selector, source, unevaluated};

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn point(value: f64) -> ElevationInterval {
    ElevationInterval::exact(value).unwrap()
}

fn around(value: f64, margin: f64) -> ElevationInterval {
    ElevationInterval::try_new(value - margin, value + margin).unwrap()
}

fn x() -> MetricDirection {
    MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap()
}

/// A flight on the floor with the given risers and 0.28 m goings, the last
/// tread its top. With `margin`, every position is widened by it.
fn flight(object: &str, risers: &[f64], margin: f64) -> TreadFlight {
    let mut elevation = 0.0;
    let mut treads = Vec::new();
    for (step, riser) in risers.iter().enumerate() {
        elevation += riser;
        #[allow(clippy::cast_precision_loss)]
        let front = 0.28 * step as f64;
        let position = |value: f64| {
            if margin > 0.0 {
                around(value, margin)
            } else {
                point(value)
            }
        };
        treads.push(
            Tread::try_new(position(elevation), position(front), position(front + 0.28)).unwrap(),
        );
    }
    let evidence = Evidence {
        source: source(),
        locator: format!("tread-flight:{object}"),
        exact: margin == 0.0,
    };
    let top = treads.last().unwrap().elevation();
    TreadFlight::try_new(id(object), x(), point(0.0), top, treads, evidence).unwrap()
}

fn ramp(object: &str, runs: &[(f64, f64)]) -> SlopedSurface {
    let mut start = 0.0;
    let mut bottom = 0.0;
    let runs = runs
        .iter()
        .map(|(rise, length)| {
            let run = SlopedRun::try_new(
                x(),
                point(bottom),
                point(bottom + rise),
                point(start),
                point(start + length),
            )
            .unwrap();
            start += length + 1.5;
            bottom += rise;
            run
        })
        .collect();
    let evidence = Evidence::exact(source(), format!("sloped-runs:{object}"));
    SlopedSurface::try_new(id(object), runs, evidence).unwrap()
}

/// Flights, ramps and headroom per object; anything else is unsupported.
#[derive(Default)]
struct Stairs {
    flights: BTreeMap<ObjectId, TreadFlight>,
    ramps: BTreeMap<ObjectId, SlopedSurface>,
    /// Clearance per subject and obstacle.
    above: BTreeMap<(ObjectId, ObjectId), f64>,
}

impl Stairs {
    fn flight(mut self, flight: TreadFlight) -> Self {
        self.flights.insert(flight.object().clone(), flight);
        self
    }

    fn ramp(mut self, ramp: SlopedSurface) -> Self {
        self.ramps.insert(ramp.object().clone(), ramp);
        self
    }

    fn above(mut self, subject: &str, obstacle: &str, clearance: f64) -> Self {
        self.above.insert((id(subject), id(obstacle)), clearance);
        self
    }
}

impl WalkingSurfaceService for Stairs {
    fn measure_tread_flight(&self, object: &ObjectId) -> Result<TreadFlight, WalkingSurfaceError> {
        self.flights
            .get(object)
            .cloned()
            .ok_or_else(|| WalkingSurfaceError::Unsupported("winders".into()))
    }

    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
        self.ramps
            .get(object)
            .cloned()
            .ok_or_else(|| WalkingSurfaceError::Unavailable("not meshed".into()))
    }

    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
        let mut least: Option<(f64, ObjectId)> = None;
        for obstacle in request.obstacles() {
            if let Some(clearance) = self
                .above
                .get(&(request.subject().clone(), obstacle.clone()))
                && least.as_ref().is_none_or(|(most, _)| clearance < most)
            {
                least = Some((*clearance, obstacle.clone()));
            }
        }
        let evidence = Evidence {
            source: source(),
            locator: format!("headroom:{}", request.subject().local_id),
            exact: false,
        };
        match least {
            None => Headroom::try_new(request.clone(), None, vec![], evidence),
            Some((clearance, obstacle)) => Headroom::try_new(
                request.clone(),
                MeasuredInterval::try_new(clearance - 1e-9, clearance + 1e-9).ok(),
                vec![obstacle],
                evidence,
            ),
        }
    }
}

fn model() -> Model {
    Model::default()
        .object("regular", "flight")
        .object("irregular", "flight")
        .object("winder", "flight")
        .object("gentle", "ramp")
        .object("steep", "ramp")
        .object("beam", "beam")
        .object("duct", "beam")
}

fn stairs() -> Stairs {
    Stairs::default()
        .flight(flight("regular", &[0.17, 0.17, 0.17, 0.17], 0.0))
        .flight(flight("irregular", &[0.17, 0.17, 0.21, 0.17], 0.0))
        .ramp(ramp("gentle", &[(0.5, 6.0), (0.5, 6.0)]))
        .ramp(ramp("steep", &[(0.5, 3.0)]))
}

const STAIR: &str = "axioval:capability.stair-geometry";
const RAMP: &str = "axioval:capability.ramp-geometry";

fn check_stairs(
    model: Model,
    stairs: Stairs,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &StairGeometryCheck,
        &rule(STAIR, kind("flight"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
        },
    )
}

#[test]
fn an_irregular_riser_is_found_from_the_flight_geometry() {
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("riser_maximum", metres(0.19)),
            ("riser_tolerance", metres(0.005)),
            ("going_minimum", metres(0.26)),
            ("step_length_minimum", metres(0.59)),
            ("step_length_maximum", metres(0.65)),
            ("maximum_risers", ParameterValue::Integer { value: 18 }),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                "riser 3 of 4 is 0.21 m; at most 0.19 m required".into()
            ),
            (
                "irregular".into(),
                "step length (2r + g) 2 of 3 is 0.7 m; 0.59 m to 0.65 m required".into()
            ),
            (
                "irregular".into(),
                "risers differ by 0.04 m (0.17 m, 0.17 m, 0.21 m, 0.17 m); at most 0.005 m \
                 allowed"
                    .into()
            ),
        ]
    );
    assert_eq!(
        evaluation.findings()[0].evidence[0].locator,
        "tread-flight:irregular"
    );
    // The winder cannot be measured and says so.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_flight_exactly_at_its_bounds_passes() {
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("riser_minimum", metres(0.17)),
            ("riser_maximum", metres(0.21)),
            ("going_minimum", metres(0.28)),
            ("going_maximum", metres(0.28)),
            ("maximum_rise", metres(0.72)),
            ("minimum_risers", ParameterValue::Integer { value: 4 }),
        ],
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
}

#[test]
fn too_many_risers_and_too_high_a_flight_are_found() {
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("maximum_risers", ParameterValue::Integer { value: 3 }),
            ("maximum_rise", metres(0.6)),
        ],
    );
    let found: Vec<String> = findings(&evaluation)
        .into_iter()
        .filter(|(object, _)| object == "regular")
        .map(|(_, message)| message)
        .collect();
    assert_eq!(
        found,
        [
            "the flight has 4 risers; at most 3 allowed",
            "the flight rises 0.68 m; at most 0.6 m allowed",
        ]
    );
}

#[test]
fn a_riser_straddling_its_bound_is_not_evaluated() {
    let stairs = Stairs::default().flight(flight("regular", &[0.19; 4], 0.001));
    let evaluation = check_stairs(model(), stairs, vec![("riser_maximum", metres(0.19))]);
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .contains(&("regular".into(), NotEvaluatedReason::IncompleteEvidence))
    );
}

fn beams() -> Selector {
    kind("beam")
}

#[test]
fn too_little_headroom_is_found_under_the_lowest_obstacle() {
    let stairs = stairs()
        .above("regular", "beam", 1.95)
        .above("regular", "duct", 2.3)
        .above("irregular", "duct", 2.3);
    let evaluation = check_stairs(
        model(),
        stairs,
        vec![
            ("minimum_headroom", metres(2.0)),
            ("headroom_obstacles", selector(beams())),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            "headroom above the walking surface is 1.95 m under test:model/beam; at least 2 m \
             required"
                .into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("beam")]);
}

#[test]
fn an_undecided_obstacle_blocks_a_pass_but_not_a_shortfall() {
    let tall = Selector::AllOf {
        operands: vec![
            kind("beam"),
            Selector::Property {
                property_set: Some("P".into()),
                property: "Exposed".into(),
                operator: ComparisonOperator::Equals,
                value: Some(boolean(true)),
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            },
        ],
    };
    let model = || {
        model()
            .value("beam", "P", "Exposed", PropertyValue::Boolean(true))
            .unreadable("duct")
    };
    let stairs = || {
        stairs()
            .above("regular", "beam", 1.95)
            .above("irregular", "beam", 2.5)
    };
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("minimum_headroom", metres(2.0)),
            ("headroom_obstacles", selector(tall)),
        ],
    );
    let found: Vec<String> = findings(&evaluation).into_iter().map(|(o, _)| o).collect();
    assert_eq!(found, ["regular"]);
    assert!(
        unevaluated(&evaluation)
            .contains(&("irregular".into(), NotEvaluatedReason::IncompleteEvidence))
    );
}

#[test]
fn stair_declarations_are_checked() {
    for parameters in [
        vec![],
        vec![("minimum_headroom", metres(2.0))],
        vec![
            ("riser_minimum", metres(0.2)),
            ("riser_maximum", metres(0.1)),
        ],
        vec![("riser_maximum", number(0.2))],
    ] {
        let evaluation = check_stairs(model(), stairs(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    let evaluation = model().evaluate(
        &StairGeometryCheck,
        &rule(STAIR, kind("flight"), vec![("riser_maximum", metres(0.19))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
    );
}

fn limit(slope: f64, length: Option<f64>, rise: Option<f64>) -> TableRow {
    let mut row = TableRow::new();
    row.insert("maximum_slope".into(), number(slope));
    if let Some(length) = length {
        row.insert("maximum_length".into(), metres(length));
    }
    if let Some(rise) = rise {
        row.insert("maximum_rise".into(), metres(rise));
    }
    row
}

fn check_ramps(parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model().evaluate_with(
        &RampGeometryCheck,
        &rule(RAMP, kind("ramp"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs())))
                .unwrap();
        },
    )
}

#[test]
fn a_ramp_too_steep_for_its_run_is_found() {
    // 1:12 up to 0.5 m of rise, or 1:6 over runs of at most 2 m.
    let evaluation = check_ramps(vec![(
        "slope_limits",
        ParameterValue::Table {
            value: vec![
                limit(1.0 / 12.0, None, Some(0.5)),
                limit(1.0 / 6.0, Some(2.0), None),
            ],
        },
    )]);
    assert_eq!(
        findings(&evaluation),
        [(
            "steep".into(),
            "run 1 of 1 rises 0.5 m over 3 m, a slope of 0.166667; required slope at most \
             0.083333 rising at most 0.5 m; or slope at most 0.166667 over at most 2 m"
                .into()
        )]
    );
    assert_eq!(
        evaluation.findings()[0].evidence[0].locator,
        "sloped-runs:steep"
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn ramp_runs_of_unequal_slope_are_found() {
    let evaluation = check_ramps(vec![("slope_tolerance", number(0.0))]);
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    let evaluation = model().evaluate_with(
        &RampGeometryCheck,
        &rule(RAMP, kind("ramp"), vec![("slope_tolerance", number(0.01))]),
        |services| {
            let stairs = Stairs::default().ramp(ramp("gentle", &[(0.5, 6.0), (0.5, 5.0)]));
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "gentle".into(),
            "run slopes differ by 0.016667 (0.083333, 0.1); at most 0.01 allowed".into()
        )]
    );
    // The steep ramp is not measured here and says so.
    assert_eq!(
        unevaluated(&evaluation),
        [("steep".into(), NotEvaluatedReason::BackendUnavailable)]
    );
}
