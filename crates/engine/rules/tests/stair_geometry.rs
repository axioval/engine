//! `stair-geometry` and `ramp-geometry` over measured flights and ramps.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ClearanceBelow, ClearanceBelowRequest, ClearanceOutcome,
    ClearanceRequest, ClearanceShape, CompleteClearanceEvidence, ElevationInterval,
    FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle,
    HandrailEvidence, HandrailRequest, Headroom, HeadroomRequest, Landing, LandingEvidence,
    LandingExtent, LandingRequest, MeasuredInterval, MetricDirection, ObstructionEvidence,
    PlacementOutcome, PlacementRequest, PlanSegment, RailMeasurement, RiserClosure, SlopedRun,
    SlopedSurface, Tread, TreadFlight, TreadFlightRequest, WalkingEnd, WalkingLine,
    WalkingLinePlacement, WalkingStretch, WalkingSurfaceError, WalkingSurfaceService,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue};
use axioval_rules::{RampGeometryCheck, StairGeometryCheck};
use common::{
    Model, boolean, findings, id, kind, number, rule, selector, source, string, unevaluated,
};

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

/// The parts of a flight a request is answered with.
#[derive(Clone)]
struct Parts {
    line: WalkingLine,
    base: ElevationInterval,
    top: ElevationInterval,
    treads: Vec<Tread>,
    /// The treads measured along a line from the inner side, if different.
    inner: Option<Vec<Tread>>,
    evidence: Evidence,
}

impl Parts {
    fn answer(&self, request: &TreadFlightRequest) -> Result<TreadFlight, WalkingSurfaceError> {
        let treads = match (request.walking_line(), &self.inner) {
            (WalkingLinePlacement::FromInnerSide(_), Some(inner)) => inner.clone(),
            _ => self.treads.clone(),
        };
        TreadFlight::try_new(
            request.clone(),
            self.line.clone(),
            self.base,
            self.top,
            treads,
            self.evidence.clone(),
        )
    }
}

impl From<TreadFlight> for Parts {
    fn from(flight: TreadFlight) -> Self {
        Self {
            line: flight.walking_line().clone(),
            base: flight.base(),
            top: flight.top(),
            treads: flight.treads().to_vec(),
            inner: None,
            evidence: flight.evidence().clone(),
        }
    }
}

/// The flight of `object`, its walking line on its centre line.
fn straight(object: &str) -> TreadFlightRequest {
    TreadFlightRequest::new(id(object))
}

/// A flight on the floor with the given risers and 0.28 m goings, 1.2 m
/// wide, the last tread its top. With `margin`, every position is widened by
/// it.
fn flight(object: &str, risers: &[f64], margin: f64) -> TreadFlight {
    wide_flight(object, risers, margin, 1.2)
}

fn wide_flight(object: &str, risers: &[f64], margin: f64, width: f64) -> TreadFlight {
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
            Tread::try_new(position(elevation), position(front), position(front + 0.28))
                .unwrap()
                .with_sides(point(0.0), point(width))
                .unwrap(),
        );
    }
    let evidence = Evidence {
        source: source(),
        locator: format!("tread-flight:{object}"),
        exact: margin == 0.0,
    };
    let top = treads.last().unwrap().elevation();
    TreadFlight::try_new(
        straight(object),
        WalkingLine::Straight(x()),
        point(0.0),
        top,
        treads,
        evidence,
    )
    .unwrap()
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
            .unwrap()
            .with_sides(point(0.0), point(1.5))
            .unwrap();
            start += length + 1.5;
            bottom += rise;
            run
        })
        .collect();
    let evidence = Evidence::exact(source(), format!("sloped-runs:{object}"));
    SlopedSurface::try_new(id(object), runs, evidence).unwrap()
}

/// A landing's carrier and, when measured, its depth and width.
type StatedLanding = (ObjectId, Option<(f64, f64)>);

/// Flights, ramps and headroom per object; anything else is unsupported.
#[derive(Default)]
struct Stairs {
    flights: BTreeMap<ObjectId, Parts>,
    ramps: BTreeMap<ObjectId, SlopedSurface>,
    /// Clearance per subject and obstacle.
    above: BTreeMap<(ObjectId, ObjectId), f64>,
    /// The landing per subject and end: its carrier and, when measured, its
    /// depth and width.
    landings: BTreeMap<(ObjectId, WalkingEnd), StatedLanding>,
    /// Clearance below per subject and space.
    below: BTreeMap<(ObjectId, ObjectId), f64>,
    /// Rails per subject and stretch.
    rails: BTreeMap<(ObjectId, WalkingStretch), Vec<(ObjectId, RailMeasurement)>>,
}

impl Stairs {
    fn flight(mut self, flight: TreadFlight) -> Self {
        self.flights
            .insert(flight.object().clone(), Parts::from(flight));
        self
    }

    fn parts(mut self, object: &str, parts: Parts) -> Self {
        self.flights.insert(id(object), parts);
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

    fn landing(
        mut self,
        subject: &str,
        end: WalkingEnd,
        carrier: &str,
        size: Option<(f64, f64)>,
    ) -> Self {
        self.landings
            .insert((id(subject), end), (id(carrier), size));
        self
    }

    fn below(mut self, subject: &str, space: &str, clearance: f64) -> Self {
        self.below.insert((id(subject), id(space)), clearance);
        self
    }

    fn rail(
        mut self,
        subject: &str,
        stretch: WalkingStretch,
        rail: &str,
        measurement: RailMeasurement,
    ) -> Self {
        self.rails
            .entry((id(subject), stretch))
            .or_default()
            .push((id(rail), measurement));
        self
    }
}

impl Stairs {
    /// Refuses `what` of a turning flight.
    fn straight(&self, subject: &ObjectId, what: &str) -> Result<(), WalkingSurfaceError> {
        match self.flights.get(subject) {
            Some(parts) if parts.line.is_turning() => Err(WalkingSurfaceError::Unsupported(
                format!("{subject} is a turning flight; its {what} are not measured"),
            )),
            _ => Ok(()),
        }
    }
}

impl WalkingSurfaceService for Stairs {
    fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        self.flights
            .get(request.object())
            .ok_or_else(|| WalkingSurfaceError::Unsupported("several pieces".into()))?
            .answer(request)
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

    /// Landings stated per end, found only when their carrier is requested;
    /// every one runs along x from an edge at 0. A turning flight's are
    /// refused, as the contract allows.
    fn measure_landing(
        &self,
        request: &LandingRequest,
    ) -> Result<LandingEvidence, WalkingSurfaceError> {
        self.straight(request.subject(), "landings")?;
        let landing = self
            .landings
            .get(&(request.subject().clone(), request.end()))
            .filter(|(carrier, _)| {
                carrier == request.subject() || request.candidates().contains(carrier)
            })
            .map(|(carrier, size)| {
                let extent = size.map(|(depth, width)| {
                    LandingExtent::try_new(point(depth), point(0.0), point(width)).unwrap()
                });
                Landing::new(carrier.clone(), extent)
            });
        let evidence = Evidence::exact(source(), format!("landing:{}", request.subject().local_id));
        LandingEvidence::try_new(request.clone(), x(), point(0.0), landing, evidence)
    }

    fn measure_clearance_below(
        &self,
        request: &ClearanceBelowRequest,
    ) -> Result<ClearanceBelow, WalkingSurfaceError> {
        let mut least: Option<(f64, ObjectId)> = None;
        for space in request.spaces() {
            if let Some(clearance) = self.below.get(&(request.subject().clone(), space.clone()))
                && least.as_ref().is_none_or(|(most, _)| clearance < most)
            {
                least = Some((*clearance, space.clone()));
            }
        }
        let evidence = Evidence {
            source: source(),
            locator: format!("clearance-below:{}", request.subject().local_id),
            exact: false,
        };
        match least {
            None => ClearanceBelow::try_new(request.clone(), None, vec![], evidence),
            Some((clearance, space)) => ClearanceBelow::try_new(
                request.clone(),
                MeasuredInterval::try_new(clearance - 1e-9, clearance + 1e-9).ok(),
                vec![space],
                evidence,
            ),
        }
    }

    /// Rails stated per stretch, found only when requested. A flight's pitch
    /// line runs from 0 to 0.84 m along x, a run's between its ends.
    fn measure_handrails(
        &self,
        request: &HandrailRequest,
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let (pitch, sides) = match request.stretch() {
            WalkingStretch::Flight => {
                self.straight(subject, "handrails")?;
                let flight =
                    self.measure_tread_flight(&TreadFlightRequest::new(subject.clone()))?;
                let sides = flight.treads()[0].sides().unwrap();
                ((point(0.0), point(0.84)), sides)
            }
            WalkingStretch::Run(index) => {
                let ramp = self.measure_sloped_runs(subject)?;
                let run = ramp.runs()[index];
                ((run.start(), run.end()), run.sides().unwrap())
            }
        };
        let rails = self
            .rails
            .get(&(subject.clone(), request.stretch()))
            .into_iter()
            .flatten()
            .filter(|(rail, _)| request.rails().contains(rail))
            .cloned()
            .collect();
        let evidence = Evidence {
            source: source(),
            locator: format!("handrails:{}", subject.local_id),
            exact: false,
        };
        HandrailEvidence::try_new(request.clone(), x(), pitch, sides, rails, evidence)
    }
}

/// A rail across `left` .. `right`, along `start` .. `end`, its top
/// `lowest` .. `highest` above the pitch line; its rises over the extension
/// beyond each end, where stated.
fn rail(
    (left, right): (f64, f64),
    (start, end): (f64, f64),
    (lowest, highest): (f64, f64),
    (bottom, top): (Option<f64>, Option<f64>),
) -> RailMeasurement {
    let height = |value: f64| MeasuredInterval::try_new(value - 1e-9, value + 1e-9).unwrap();
    let rise =
        |value: Option<f64>| value.map(|value| MeasuredInterval::try_new(value, value).unwrap());
    RailMeasurement::try_new(
        (point(start), point(end)),
        (point(left), point(right)),
        height(lowest),
        height(highest),
    )
    .unwrap()
    .with_rises(rise(bottom), rise(top))
    .unwrap()
}

/// Boxes in plan that obstruct any clearance footprint they overlap.
#[derive(Default)]
struct Floor {
    blockers: Vec<(ObjectId, [f64; 2], [f64; 2])>,
}

impl Floor {
    fn blocker(mut self, object: &str, min: [f64; 2], max: [f64; 2]) -> Self {
        self.blockers.push((id(object), min, max));
        self
    }
}

impl FreeSpaceService for Floor {
    fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        let ClearanceShape::Box(shape) = request.shape() else {
            return Err(FreeSpaceError::Unavailable("boxes only".into()));
        };
        let frame = request.frame();
        let [cx, cy, _] = frame.origin().coordinates_metres();
        let ([rx, ry, _], [fx, fy, _]) = (frame.right().components(), frame.forward().components());
        let (w, d) = (shape.width_metres() / 2.0, shape.depth_metres() / 2.0);
        let corners = [(-w, -d), (w, -d), (w, d), (-w, d)]
            .map(|(a, b)| [cx + a * rx + b * fx, cy + a * ry + b * fy]);
        let low = [0, 1].map(|axis| {
            corners
                .iter()
                .map(|c| c[axis])
                .fold(f64::INFINITY, f64::min)
        });
        let high = [0, 1].map(|axis| {
            corners
                .iter()
                .map(|c| c[axis])
                .fold(f64::NEG_INFINITY, f64::max)
        });
        let blockers: Vec<ObjectId> = self
            .blockers
            .iter()
            .filter(|(object, min, max)| {
                request.obstacles().contains(object)
                    && (0..2)
                        .all(|axis| min[axis] < high[axis] - 1e-9 && max[axis] > low[axis] + 1e-9)
            })
            .map(|(object, _, _)| object.clone())
            .collect();
        let evidence = Evidence::exact(source(), "free-space");
        if blockers.is_empty() {
            Ok(ClearanceOutcome::Clear(CompleteClearanceEvidence::try_new(
                request.clone(),
                evidence,
            )?))
        } else {
            Ok(ClearanceOutcome::Obstructed(ObstructionEvidence::try_new(
                request.clone(),
                blockers,
                evidence,
            )?))
        }
    }

    fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("no placements".into()))
    }

    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("no areas".into()))
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
        .object("slab", "slab")
        .object("floor", "slab")
        .object("hall", "space")
        .object("left_rail", "railing")
        .object("low_rail", "railing")
        .object("short_rail", "railing")
        .object("ramp_rail", "railing")
        .object("door", "door")
        .object("bin", "furniture")
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
    // The flight in pieces cannot be measured and says so.
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

fn slabs() -> ParameterValue {
    selector(kind("slab"))
}

#[test]
fn a_narrow_flight_and_a_shallow_landing_are_found() {
    let stairs = stairs()
        .flight(wide_flight("regular", &[0.17; 4], 0.0, 1.0))
        .landing("regular", WalkingEnd::FlightTop, "slab", Some((0.9, 1.0)))
        .landing(
            "regular",
            WalkingEnd::FlightBottom,
            "floor",
            Some((3.0, 4.0)),
        )
        .landing("irregular", WalkingEnd::FlightTop, "slab", Some((1.5, 1.5)));
    let evaluation = check_stairs(
        model(),
        stairs,
        vec![
            ("width_minimum", metres(1.1)),
            ("landing_objects", slabs()),
            ("landing_depth_minimum", metres(1.0)),
            ("landing_at_least_walking_width", boolean(true)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "regular".into(),
                "the flight is 1 m wide; at least 1.1 m required".into()
            ),
            (
                "regular".into(),
                "the landing at the top of the flight is 0.9 m deep; at least 1 m and the \
                 flight's width (1 m) required"
                    .into()
            ),
        ]
    );
    let landing = &evaluation.findings()[1];
    assert_eq!(landing.related, [id("slab")]);
    assert!(
        landing
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "landing:regular")
    );
    // The flight in pieces is not measured; no landing at the bottom of `irregular`
    // is nothing to check.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_missing_landing_is_found_only_when_required_and_decided() {
    let stairs = || {
        stairs()
            .landing("regular", WalkingEnd::FlightTop, "slab", Some((1.5, 1.5)))
            .landing("regular", WalkingEnd::FlightBottom, "floor", None)
            .landing("irregular", WalkingEnd::FlightBottom, "floor", None)
            .landing("irregular", WalkingEnd::FlightTop, "slab", None)
    };
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("landing_objects", selector(kind("slab"))),
            ("landings_required", boolean(true)),
        ],
    );
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    // Only the landing slab carries landings now.
    let model = || {
        model()
            .value("slab", "P", "Landing", PropertyValue::Boolean(true))
            .value("floor", "P", "Landing", PropertyValue::Boolean(false))
    };
    let landing = Selector::Property {
        property_set: Some("P".into()),
        property: "Landing".into(),
        operator: ComparisonOperator::Equals,
        value: Some(boolean(true)),
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    };
    let evaluation = check_stairs(
        model().unreadable("beam"),
        stairs(),
        vec![
            ("landing_objects", selector(landing.clone())),
            ("landings_required", boolean(true)),
        ],
    );
    // The beam's selection is undecided: it might carry the missing
    // landings, so they are not found.
    assert!(findings(&evaluation).is_empty());
    assert!(
        unevaluated(&evaluation)
            .contains(&("regular".into(), NotEvaluatedReason::IncompleteEvidence))
    );
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("landing_objects", selector(landing)),
            ("landings_required", boolean(true)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                "no selected slab or landing meets the bottom of the flight".into()
            ),
            (
                "regular".into(),
                "no selected slab or landing meets the bottom of the flight".into()
            ),
        ]
    );
}

#[test]
fn a_landing_filling_no_rectangle_or_beside_an_unmeasured_flight_is_not_evaluated() {
    let stairs = stairs()
        .flight({
            // Treads without sides: the flight's width is unknown.
            let treads = vec![
                Tread::try_new(point(0.17), point(0.0), point(0.28)).unwrap(),
                Tread::try_new(point(0.34), point(0.28), point(0.56)).unwrap(),
            ];
            let evidence = Evidence::exact(source(), "tread-flight:irregular");
            TreadFlight::try_new(
                straight("irregular"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.34),
                treads,
                evidence,
            )
            .unwrap()
        })
        .landing("regular", WalkingEnd::FlightTop, "slab", None)
        .landing("irregular", WalkingEnd::FlightTop, "slab", Some((2.0, 2.0)));
    let evaluation = check_stairs(
        model(),
        stairs,
        vec![
            ("landing_objects", slabs()),
            ("landing_at_least_walking_width", boolean(true)),
        ],
    );
    assert!(findings(&evaluation).is_empty());
    let messages: Vec<String> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect();
    assert!(
        messages.iter().any(|message| message
            == "the landing test:model/slab at the top of the flight fills no rectangle along \
                the walking direction, so its size is not measured"),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message
            == "the flight's width is not measured, so the landing at the top of the flight \
                is not compared with it"),
        "{messages:?}"
    );
}

#[test]
fn too_little_headroom_below_a_flight_is_found_over_a_space_floor() {
    let stairs = stairs()
        .below("regular", "hall", 1.5)
        .below("irregular", "hall", 2.4);
    let evaluation = check_stairs(
        model(),
        stairs,
        vec![
            ("minimum_headroom_below", metres(2.0)),
            ("headroom_below_spaces", selector(kind("space"))),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            "headroom below the flight is 1.5 m over the floor of test:model/hall; at least 2 m \
             required"
                .into()
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("hall")]);
}

#[test]
fn landing_and_below_declarations_are_checked() {
    for parameters in [
        vec![("landing_depth_minimum", metres(1.0))],
        vec![("landing_objects", slabs())],
        vec![("minimum_headroom_below", metres(2.0))],
        vec![("headroom_below_spaces", selector(kind("space")))],
        vec![
            ("width_minimum", metres(1.2)),
            ("width_maximum", metres(1.0)),
        ],
    ] {
        let evaluation = check_stairs(model(), stairs(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn ramp_widths_and_run_landings_are_checked_per_run() {
    let evaluation = model().evaluate_with(
        &RampGeometryCheck,
        &rule(
            RAMP,
            kind("ramp"),
            vec![
                ("width_minimum", metres(1.2)),
                ("landing_objects", slabs()),
                ("landing_depth_minimum", metres(1.5)),
            ],
        ),
        |services| {
            let stairs = stairs()
                .landing(
                    "gentle",
                    WalkingEnd::RunBottom(0),
                    "gentle",
                    Some((1.5, 1.5)),
                )
                .landing("gentle", WalkingEnd::RunTop(0), "gentle", Some((1.2, 1.5)))
                .landing(
                    "gentle",
                    WalkingEnd::RunBottom(1),
                    "gentle",
                    Some((1.5, 1.5)),
                )
                .landing("gentle", WalkingEnd::RunTop(1), "slab", Some((2.0, 1.5)));
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "gentle".into(),
            "the landing at the top of run 1 of 2 is 1.2 m deep; at least 1.5 m required".into()
        )]
    );
    // The ramp's own landing relates nothing else.
    assert!(evaluation.findings()[0].related.is_empty());
    let evaluation = model().evaluate_with(
        &RampGeometryCheck,
        &rule(RAMP, kind("ramp"), vec![("width_minimum", metres(1.8))]),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs())))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "gentle".into(),
                "run width 1 of 2 is 1.5 m, run width 2 of 2 is 1.5 m; at least 1.8 m required"
                    .into()
            ),
            (
                "steep".into(),
                "run width 1 of 1 is 1.5 m; at least 1.8 m required".into()
            ),
        ]
    );
}

/// A service measuring flights only refuses landings and the clearance
/// below, and the checks say so rather than pass.
#[test]
fn a_service_without_landings_leaves_them_not_evaluated() {
    struct FlightsOnly;
    impl WalkingSurfaceService for FlightsOnly {
        fn measure_tread_flight(
            &self,
            request: &TreadFlightRequest,
        ) -> Result<TreadFlight, WalkingSurfaceError> {
            stairs().measure_tread_flight(request)
        }
        fn measure_sloped_runs(
            &self,
            object: &ObjectId,
        ) -> Result<SlopedSurface, WalkingSurfaceError> {
            stairs().measure_sloped_runs(object)
        }
        fn measure_headroom(
            &self,
            request: &HeadroomRequest,
        ) -> Result<Headroom, WalkingSurfaceError> {
            stairs().measure_headroom(request)
        }
    }
    let evaluation = model().evaluate_with(
        &StairGeometryCheck,
        &rule(
            STAIR,
            kind("flight"),
            vec![
                ("landing_objects", slabs()),
                ("landings_required", boolean(true)),
                ("minimum_headroom_below", metres(2.0)),
                ("headroom_below_spaces", selector(kind("space"))),
            ],
        ),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(FlightsOnly)))
                .unwrap();
        },
    );
    assert!(evaluation.findings().is_empty());
    let regular = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("regular")))
        .count();
    // Both ends and the clearance below.
    assert_eq!(regular, 3);
}

fn handrail_parameters(
    extra: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("handrail_objects", selector(kind("railing"))),
        ("handrail_reach_across", metres(0.2)),
        ("handrail_reach_above", metres(1.5)),
    ];
    parameters.extend(extra);
    parameters
}

const LEVEL: (Option<f64>, Option<f64>) = (Some(0.0), Some(0.0));

#[test]
fn handrails_too_low_too_short_sloping_or_on_one_side_are_found() {
    let stairs = stairs()
        .rail(
            "regular",
            WalkingStretch::Flight,
            "left_rail",
            rail((1.25, 1.3), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        )
        .rail(
            "regular",
            WalkingStretch::Flight,
            "low_rail",
            rail((-0.1, -0.05), (-0.1, 1.14), (0.75, 0.76), (None, Some(0.0))),
        )
        .rail(
            "irregular",
            WalkingStretch::Flight,
            "short_rail",
            rail(
                (1.25, 1.3),
                (-0.3, 1.14),
                (0.9, 0.9),
                (Some(0.0), Some(0.05)),
            ),
        );
    let evaluation = check_stairs(
        model(),
        stairs,
        handrail_parameters(vec![
            ("handrail_height_minimum", metres(0.8)),
            ("handrail_height_maximum", metres(1.0)),
            ("handrail_extension_minimum", metres(0.3)),
            ("handrail_sides", string("both")),
        ]),
    );
    let (low, short) = (id("low_rail"), id("short_rail"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                format!(
                    "the top of handrail {short} rises or falls 0.05 m over the 0.3 m beyond the \
                     top of the flight; it must continue level"
                )
            ),
            (
                "irregular".into(),
                "a handrail runs along the left side of the flight only (seen climbing); both \
                 sides required"
                    .into()
            ),
            (
                "regular".into(),
                format!(
                    "handrail {low} runs 0.75 m above the pitch line of the flight at its \
                     lowest; 0.8 m to 1 m required"
                )
            ),
            (
                "regular".into(),
                format!(
                    "handrail {low} reaches 0.1 m beyond the bottom of the flight; at least 0.3 \
                     m required"
                )
            ),
        ]
    );
    let too_low = &evaluation.findings()[2];
    assert_eq!(too_low.related, [low]);
    assert!(
        too_low
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "handrails:regular")
    );
    // The flight in pieces is not measured.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn handrails_on_both_sides_are_required_above_a_width() {
    let stairs = || {
        stairs().rail(
            "regular",
            WalkingStretch::Flight,
            "left_rail",
            rail((1.25, 1.3), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        )
    };
    let sides = |width: f64| {
        handrail_parameters(vec![
            ("handrail_sides", string("one")),
            ("handrail_both_sides_above_width", metres(width)),
        ])
    };
    // Both flights are 1.2 m wide.
    let evaluation = check_stairs(model(), stairs(), sides(1.0));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                "no selected handrail runs along a side of the flight; both sides required".into()
            ),
            (
                "regular".into(),
                "a handrail runs along the left side of the flight only (seen climbing); both \
                 sides required"
                    .into()
            ),
        ]
    );
    let evaluation = check_stairs(model(), stairs(), sides(1.5));
    assert_eq!(
        findings(&evaluation),
        [(
            "irregular".into(),
            "no selected handrail runs along a side of the flight; one side required".into()
        )]
    );
}

#[test]
fn an_undecided_rail_leaves_a_missing_side_and_a_pass_not_evaluated() {
    let stairs = stairs().rail(
        "regular",
        WalkingStretch::Flight,
        "left_rail",
        rail((1.25, 1.3), (-0.3, 1.14), (0.9, 0.9), LEVEL),
    );
    let handrail = Selector::Property {
        property_set: Some("P".into()),
        property: "Handrail".into(),
        operator: ComparisonOperator::Equals,
        value: Some(boolean(true)),
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    };
    let evaluation = check_stairs(
        model()
            .value("left_rail", "P", "Handrail", PropertyValue::Boolean(true))
            .unreadable("low_rail"),
        stairs,
        vec![
            ("handrail_objects", selector(handrail)),
            ("handrail_reach_across", metres(0.2)),
            ("handrail_reach_above", metres(1.5)),
            ("handrail_height_minimum", metres(0.8)),
            ("handrail_sides", string("both")),
        ],
    );
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    let undecided = unevaluated(&evaluation);
    for flight in ["regular", "irregular"] {
        assert!(
            undecided.contains(&(flight.into(), NotEvaluatedReason::IncompleteEvidence)),
            "{undecided:?}"
        );
    }
}

#[test]
fn handrail_and_ramp_end_declarations_are_checked() {
    for parameters in [
        vec![("handrail_sides", string("both"))],
        handrail_parameters(vec![]),
        handrail_parameters(vec![("handrail_sides", string("three"))]),
        handrail_parameters(vec![
            ("handrail_sides", string("both")),
            ("handrail_both_sides_above_width", metres(1.0)),
        ]),
        vec![
            ("handrail_objects", selector(kind("railing"))),
            ("handrail_extension_minimum", metres(0.3)),
        ],
    ] {
        let evaluation = check_stairs(model(), stairs(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    for parameters in [
        vec![("end_space_depth", metres(1.5))],
        vec![("landing_doors", selector(kind("door")))],
        vec![
            ("landing_doors", selector(kind("door"))),
            ("landing_door_height", metres(2.0)),
        ],
        vec![
            ("end_space_depth", metres(0.0)),
            ("end_space_width", metres(1.5)),
            ("end_space_height", metres(2.0)),
            ("end_space_obstacles", selector(kind("furniture"))),
        ],
    ] {
        let evaluation = check_ramps(parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    // A flight's landing doors are declared as a ramp's.
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![("landing_doors", selector(kind("door")))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// A service measuring flights only refuses handrails, and the check says
/// so rather than find them missing.
#[test]
fn a_service_without_handrails_leaves_them_not_evaluated() {
    struct FlightsOnly;
    impl WalkingSurfaceService for FlightsOnly {
        fn measure_tread_flight(
            &self,
            request: &TreadFlightRequest,
        ) -> Result<TreadFlight, WalkingSurfaceError> {
            stairs().measure_tread_flight(request)
        }
        fn measure_sloped_runs(
            &self,
            object: &ObjectId,
        ) -> Result<SlopedSurface, WalkingSurfaceError> {
            stairs().measure_sloped_runs(object)
        }
        fn measure_headroom(
            &self,
            request: &HeadroomRequest,
        ) -> Result<Headroom, WalkingSurfaceError> {
            stairs().measure_headroom(request)
        }
    }
    let evaluation = model().evaluate_with(
        &StairGeometryCheck,
        &rule(
            STAIR,
            kind("flight"),
            handrail_parameters(vec![("handrail_sides", string("one"))]),
        ),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(FlightsOnly)))
                .unwrap();
        },
    );
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .contains(&("regular".into(), NotEvaluatedReason::IncompleteEvidence))
    );
}

fn check_ramps_with(
    stairs: Stairs,
    floor: Option<Floor>,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model().evaluate_with(
        &RampGeometryCheck,
        &rule(RAMP, kind("ramp"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
            if let Some(floor) = floor {
                services
                    .register(FreeSpaceServiceHandle::new(Arc::new(floor)))
                    .unwrap();
            }
        },
    )
}

#[test]
fn ramp_handrails_end_spaces_and_landing_doors_are_found() {
    // `gentle` runs 0 .. 6 and 7.5 .. 13.5 m along x, `steep` 0 .. 3 m,
    // both 1.5 m wide from y = 0. Every stated landing runs along x from 0.
    let stairs = || {
        stairs()
            .rail(
                "gentle",
                WalkingStretch::Run(0),
                "ramp_rail",
                rail((-0.1, -0.05), (-0.3, 6.3), (0.9, 0.9), LEVEL),
            )
            .rail(
                "gentle",
                WalkingStretch::Run(1),
                "ramp_rail",
                rail((-0.1, -0.05), (7.2, 13.8), (0.9, 0.9), LEVEL),
            )
            .rail(
                "steep",
                WalkingStretch::Run(0),
                "ramp_rail",
                rail((-0.1, -0.05), (0.0, 3.3), (0.9, 0.9), (None, Some(0.0))),
            )
            .landing("gentle", WalkingEnd::RunTop(1), "slab", Some((2.0, 1.5)))
    };
    let floor = || {
        Floor::default()
            .blocker("door", [0.5, 0.2], [1.0, 0.4])
            .blocker("bin", [3.5, 0.5], [4.0, 1.0])
    };
    let parameters = || {
        handrail_parameters(vec![
            ("handrail_height_minimum", metres(0.8)),
            ("handrail_extension_minimum", metres(0.3)),
            ("handrail_sides", string("one")),
            ("landing_objects", slabs()),
            ("landing_doors", selector(kind("door"))),
            ("landing_door_height", metres(2.0)),
            ("end_space_depth", metres(1.5)),
            ("end_space_width", metres(1.5)),
            ("end_space_height", metres(2.0)),
            ("end_space_obstacles", selector(kind("furniture"))),
        ])
    };
    let evaluation = check_ramps_with(stairs(), Some(floor()), parameters());
    let (door, rail, bin) = (id("door"), id("ramp_rail"), id("bin"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "gentle".into(),
                format!("door {door} stands on the landing at the top of run 2 of 2")
            ),
            (
                "steep".into(),
                format!(
                    "handrail {rail} reaches 0 m beyond the bottom of run 1 of 1; at least 0.3 m \
                     required"
                )
            ),
            (
                "steep".into(),
                format!(
                    "{bin} obstructs the free space at the top of the ramp (1.5 m deep, 1.5 m \
                     wide)"
                )
            ),
        ]
    );
    assert_eq!(evaluation.findings()[0].related, [door]);
    assert_eq!(evaluation.findings()[2].related, [bin]);
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );

    // Without a free-space service, end spaces and doors are not checked.
    let evaluation = check_ramps_with(stairs(), None, parameters());
    assert_eq!(findings(&evaluation).len(), 1);
    let undecided = unevaluated(&evaluation);
    // Two end spaces each, and the one landing's doors.
    assert_eq!(undecided.len(), 5, "{undecided:?}");
}

#[test]
fn a_ramp_needs_a_landing_at_every_run_end_when_required() {
    let stairs = stairs()
        .landing("steep", WalkingEnd::RunBottom(0), "floor", None)
        .landing("gentle", WalkingEnd::RunBottom(0), "floor", None)
        .landing("gentle", WalkingEnd::RunTop(0), "gentle", None)
        .landing("gentle", WalkingEnd::RunBottom(1), "gentle", None)
        .landing("gentle", WalkingEnd::RunTop(1), "slab", None);
    let evaluation = check_ramps_with(
        stairs,
        None,
        vec![
            ("landing_objects", slabs()),
            ("landings_required", boolean(true)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "steep".into(),
            "no selected slab or landing meets the top of run 1 of 1".into()
        )]
    );
}

fn degrees(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "deg".into(),
    }
}

/// A turning flight of five treads, 0.18 m risers: two straight treads,
/// two winders turning 35° each, and a straight tread. Along its centre
/// line every going is 0.28 m; along a line from its inner side the
/// winders' goings are 0.22 m. The straight treads are 0.9 m wide.
fn winder() -> Parts {
    let build = |fronts: [f64; 5]| -> Vec<Tread> {
        [0.0_f64, 0.0, 35.0, 70.0, 70.0]
            .iter()
            .zip(fronts)
            .enumerate()
            .map(|(step, (angle, front))| {
                let (sin, cos) = angle.to_radians().sin_cos();
                #[allow(clippy::cast_precision_loss)]
                let elevation = 0.18 * (step + 1) as f64;
                let tread = Tread::try_new(point(elevation), point(front), point(front + 0.3))
                    .unwrap()
                    .with_nosing(PlanSegment::try_new([0.0, 0.0], [cos, sin], 0.0).unwrap())
                    .with_riser_below(RiserClosure::Closed);
                if (2..4).contains(&step) {
                    tread
                } else {
                    tread.with_sides(point(0.0), point(0.9)).unwrap()
                }
            })
            .collect()
    };
    let treads = build([0.0, 0.28, 0.56, 0.84, 1.12]);
    Parts {
        line: WalkingLine::Turning(vec![
            [0.14, 0.45],
            [0.42, 0.45],
            [0.7, 0.55],
            [0.85, 0.8],
            [0.9, 1.1],
        ]),
        base: point(0.0),
        top: treads.last().unwrap().elevation(),
        inner: Some(build([0.0, 0.28, 0.5, 0.72, 1.0])),
        treads,
        evidence: Evidence::exact(source(), "tread-flight:winder"),
    }
}

#[test]
fn sharp_winders_are_found() {
    let evaluation = check_stairs(
        model(),
        stairs().parts("winder", winder()),
        vec![("winder_angle_maximum", degrees(30.0))],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "winder".into(),
            "winder angle 2 of 4 is 35°, winder angle 3 of 4 is 35°; at most 30° required".into()
        )]
    );
    // Flights whose nosings are not measured are not evaluated.
    let unevaluated = unevaluated(&evaluation);
    assert!(unevaluated.contains(&("regular".into(), NotEvaluatedReason::IncompleteEvidence)));
    let message = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .find(|message| message.contains("winder angle"))
        .unwrap();
    assert!(
        message.starts_with(
            "winder angle 1 of 3, winder angle 2 of 3, winder angle 3 of 3 not measured"
        ),
        "{message}"
    );

    // Within the bound, nothing is found.
    let evaluation = check_stairs(
        model(),
        stairs().parts("winder", winder()),
        vec![("winder_angle_maximum", degrees(35.0))],
    );
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
}

#[test]
fn a_turning_flight_is_walked_where_the_rule_places_its_line() {
    let centre = check_stairs(
        model(),
        stairs().parts("winder", winder()),
        vec![("going_minimum", metres(0.25))],
    );
    assert!(findings(&centre).is_empty(), "{:?}", findings(&centre));
    let inner = check_stairs(
        model(),
        stairs().parts("winder", winder()),
        vec![
            ("going_minimum", metres(0.25)),
            ("walking_line_offset", metres(0.3)),
        ],
    );
    assert_eq!(
        findings(&inner),
        [(
            "winder".into(),
            "going 2 of 4 is 0.22 m, going 3 of 4 is 0.22 m; at least 0.25 m required".into()
        )]
    );
}

#[test]
fn open_risers_are_found_when_forbidden() {
    use RiserClosure::{Closed, NotMeasured, Open};
    let closing = |risers: [RiserClosure; 4], object: &str| {
        let mut parts = Parts::from(flight(object, &[0.17; 4], 0.0));
        parts.treads = parts
            .treads
            .iter()
            .zip(risers)
            .map(|(tread, riser)| tread.with_riser_below(riser))
            .collect();
        parts
    };
    let stairs = Stairs::default()
        .parts("regular", closing([Closed, Open, Open, Closed], "regular"))
        .parts(
            "irregular",
            closing([NotMeasured, Closed, Closed, Closed], "irregular"),
        )
        .parts("winder", closing([Closed; 4], "winder"));
    let evaluation = check_stairs(model(), stairs, vec![("forbid_open_risers", boolean(true))]);
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            "riser 2 of 4 is open, riser 3 of 4 is open; closed risers required".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("irregular".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("whether riser 1 of 4 is closed is not measured")
    );
}

#[test]
fn walking_line_and_winder_declarations_are_checked() {
    for parameters in [
        vec![("walking_line_offset", metres(0.0))],
        vec![("winder_angle_maximum", metres(0.3))],
        vec![("width_minimum", degrees(1.0))],
        vec![("forbid_open_risers", number(1.0))],
    ] {
        let evaluation = check_stairs(model(), stairs(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// A turning flight has headroom below it measured as any flight's, but no
/// width (its winders taper), and the service refuses its landings and
/// handrails: those checks are not evaluated rather than judged on a frame
/// the flight does not have.
#[test]
fn a_turning_flight_keeps_its_headroom_below_and_leaves_width_landings_and_rails_open() {
    let stairs = Stairs::default()
        .parts("winder", winder())
        .below("winder", "hall", 1.8)
        .landing("winder", WalkingEnd::FlightTop, "slab", Some((2.0, 2.0)))
        .rail(
            "winder",
            WalkingStretch::Flight,
            "left_rail",
            rail((0.95, 1.0), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        );
    let evaluation = check_stairs(
        model(),
        stairs,
        handrail_parameters(vec![
            ("width_minimum", metres(0.8)),
            ("landing_objects", slabs()),
            ("landings_required", boolean(true)),
            ("minimum_headroom_below", metres(2.0)),
            ("headroom_below_spaces", selector(kind("space"))),
            ("handrail_height_minimum", metres(0.8)),
        ]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "winder".into(),
            "headroom below the flight is 1.8 m over the floor of test:model/hall; at least 2 m \
             required"
                .into()
        )]
    );
    let messages: Vec<String> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("winder")))
        .map(|outcome| outcome.message().to_owned())
        .collect();
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("the flight's width is not measured")),
        "{messages:?}"
    );
    for end in ["bottom", "top"] {
        assert!(
            messages.iter().any(|message| message
                .starts_with(&format!("landing at the {end} of the flight: "))
                && message.contains("turning flight")),
            "{messages:?}"
        );
    }
    assert!(
        messages
            .iter()
            .any(|message| message.contains("handrails") && message.contains("turning flight")),
        "{messages:?}"
    );
}

/// Doors standing 0 .. 2.1 m high, whatever their level.
struct DoorHeights;

impl axioval_engine::VerticalExtentService for DoorHeights {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<axioval_engine::VerticalExtent, axioval_engine::VerticalExtentError> {
        axioval_engine::VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(0.0).unwrap(),
            ElevationInterval::exact(2.1).unwrap(),
            Evidence::exact(source(), format!("extent:{}", object.local_id)),
        )
    }
}

#[test]
fn a_door_swinging_over_a_ramp_landing_is_found() {
    use common::doors::{Doors, hinged};
    // Every stated landing runs from x 0 to 2 and y 0 to 1.5. `door`,
    // hinged at (1, 2), swings south over it; turned to swing north, it
    // sweeps y 2 .. 2.9 and misses it.
    let landing = || stairs().landing("gentle", WalkingEnd::RunTop(1), "slab", Some((2.0, 1.5)));
    let run = |open: [f64; 3], heights: bool| {
        let doors = Doors::default().door(
            "door",
            vec![hinged([1.0, 2.0, 0.0], [-1.0, 0.0, 0.0], open, 0.9, false)],
            1.0,
            None,
        );
        model().evaluate_with(
            &RampGeometryCheck,
            &rule(
                RAMP,
                kind("ramp"),
                vec![
                    ("landing_objects", slabs()),
                    ("landing_doors", selector(kind("door"))),
                    ("landing_door_height", metres(2.0)),
                    ("landing_door_swing", boolean(true)),
                ],
            ),
            |services| {
                services
                    .register(WalkingSurfaceServiceHandle::new(Arc::new(landing())))
                    .unwrap();
                services
                    .register(FreeSpaceServiceHandle::new(Arc::new(Floor::default())))
                    .unwrap();
                services.register(doors.handle()).unwrap();
                if heights {
                    services
                        .register(axioval_engine::VerticalExtentServiceHandle::new(Arc::new(
                            DoorHeights,
                        )))
                        .unwrap();
                }
            },
        )
    };
    let evaluation = run([0.0, -1.0, 0.0], true);
    assert_eq!(
        findings(&evaluation),
        [(
            "gentle".into(),
            format!(
                "door {} swings over the landing at the top of run 2 of 2",
                id("door")
            )
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("door")]);
    // Without its height the door may stand on another level.
    let evaluation = run([0.0, -1.0, 0.0], false);
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("gentle".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // Swinging away, it passes, height or not.
    let evaluation = run([0.0, 1.0, 0.0], false);
    assert!(findings(&evaluation).is_empty() && unevaluated(&evaluation).is_empty());
}

#[test]
fn a_door_on_or_swinging_over_a_stair_landing_is_found() {
    use common::doors::{Doors, hinged};
    // The landing at the top of `regular` runs from x 0 to 2 and y 0 to
    // 1.2, 0.68 m up. `door`, hinged at (1, 1.7), swings south over it;
    // turned to swing north, it sweeps y 1.7 .. 2.6 and misses it.
    let landing = || stairs().landing("regular", WalkingEnd::FlightTop, "slab", Some((2.0, 1.2)));
    let run = |open: [f64; 3], swing: bool, floor: Floor| {
        let doors = Doors::default().door(
            "door",
            vec![hinged([1.0, 1.7, 0.0], [-1.0, 0.0, 0.0], open, 0.9, false)],
            1.0,
            None,
        );
        let mut parameters = vec![
            ("landing_objects", slabs()),
            ("landing_doors", selector(kind("door"))),
            ("landing_door_height", metres(2.0)),
        ];
        if swing {
            parameters.push(("landing_door_swing", boolean(true)));
        }
        model().evaluate_with(
            &StairGeometryCheck,
            &rule(STAIR, kind("flight"), parameters),
            |services| {
                services
                    .register(WalkingSurfaceServiceHandle::new(Arc::new(landing())))
                    .unwrap();
                services
                    .register(FreeSpaceServiceHandle::new(Arc::new(floor)))
                    .unwrap();
                services.register(doors.handle()).unwrap();
                services
                    .register(axioval_engine::VerticalExtentServiceHandle::new(Arc::new(
                        DoorHeights,
                    )))
                    .unwrap();
            },
        )
    };
    let door = id("door");
    // Standing on the landing.
    let evaluation = run(
        [0.0, 1.0, 0.0],
        false,
        Floor::default().blocker("door", [0.5, 0.2], [1.0, 0.4]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            format!("door {door} stands on the landing at the top of the flight")
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [door.clone()]);
    // Swinging over it from beside it.
    let evaluation = run([0.0, -1.0, 0.0], true, Floor::default());
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            format!("door {door} swings over the landing at the top of the flight")
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [door]);
    // Swinging away, it passes; the flight in pieces is not measured.
    let evaluation = run([0.0, 1.0, 0.0], true, Floor::default());
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}
