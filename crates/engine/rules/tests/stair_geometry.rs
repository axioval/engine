//! `stair-geometry` and `ramp-geometry` over measured flights and ramps.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ClearanceBelow, ClearanceBelowRequest, ClearanceOutcome,
    ClearanceRequest, ClearanceShape, CompleteClearanceEvidence, ElevationInterval,
    FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle,
    GeometryFidelity, HandrailEvidence, HandrailRequest, Headroom, HeadroomRequest, Landing,
    LandingEvidence, LandingExtent, LandingRequest, MeasuredInterval, MetricDirection,
    ObjectBounds, ObstructionEvidence, PlacementOutcome, PlacementRequest, PlanSegment,
    ProjectedDistanceEvidence, ProximityError, ProximityEvidence, ProximityProjection,
    ProximityRequest, ProximityService, ProximityServiceHandle, RailMeasurement, RiserClosure,
    SlopedRun, SlopedSurface, StretchPart, Tread, TreadFlight, TreadFlightRequest, WalkingEnd,
    WalkingLine, WalkingLinePlacement, WalkingStretch, WalkingSurfaceError, WalkingSurfaceService,
    WalkingSurfaceServiceHandle,
};
use axioval_engine::{
    ClearWidthEvidence, ClearWidthRequest, LandingClearWidth, LandingClearWidthEvidence,
    LandingClearWidthRequest, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle,
    PlanLength, PlanRectangle, PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle,
    RectangleOrientation, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue};
use axioval_rules::{RampGeometryCheck, StairGeometryCheck};
use common::{
    Model, assert_deviation, boolean, deviation_of, findings, id, kind, number, rule, selector,
    source, string, unevaluated,
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
    /// A turning flight's straight parts, per subject; a turning flight
    /// without them has its landings and handrails refused.
    turning: BTreeMap<ObjectId, Vec<StretchPart>>,
    /// The leaving direction and arrival line per subject and end, where
    /// not along x from 0.
    ends: BTreeMap<(ObjectId, WalkingEnd), (MetricDirection, f64)>,
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

    fn end(
        mut self,
        subject: &str,
        end: WalkingEnd,
        direction: MetricDirection,
        edge: f64,
    ) -> Self {
        self.ends.insert((id(subject), end), (direction, edge));
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
    /// Measures a turning flight's landings and handrails in `parts`.
    fn in_parts(mut self, object: &str, parts: Vec<StretchPart>) -> Self {
        self.turning.insert(id(object), parts);
        self
    }

    /// Refuses `what` of a turning flight whose parts are not stated.
    fn straight(&self, subject: &ObjectId, what: &str) -> Result<(), WalkingSurfaceError> {
        match self.flights.get(subject) {
            Some(parts) if parts.line.is_turning() && !self.turning.contains_key(subject) => {
                Err(WalkingSurfaceError::Unsupported(format!(
                    "{subject} is a turning flight; its {what} are not measured"
                )))
            }
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
        let (direction, edge) = self
            .ends
            .get(&(request.subject().clone(), request.end()))
            .copied()
            .unwrap_or((x(), 0.0));
        LandingEvidence::try_new(request.clone(), direction, point(edge), landing, evidence)
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
        if let (WalkingStretch::Flight, Some(parts)) =
            (request.stretch(), self.turning.get(subject))
        {
            // A turning flight's pitch line ends at 1.12 m along its last
            // part.
            return HandrailEvidence::try_in_parts(
                request.clone(),
                parts.clone(),
                (point(0.0), point(1.12)),
                rails,
                evidence,
            );
        }
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
        .object("lower_piece", "railing")
        .object("upper_piece", "railing")
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
    assert_deviation(
        deviation_of(&evaluation, "the flight has 4 risers"),
        (1.0 / 3.0, 1.0 / 3.0),
    );
    let rise = deviation_of(&evaluation, "the flight rises 0.68 m");
    assert!(rise.0 > 0.13 && rise.1 < 0.14, "{rise:?}");
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
    // The nearest row: 3 m over its 2 m run, 50 % beyond it.
    assert_deviation(deviation_of(&evaluation, "run 1 of 1"), (0.5, 0.5));
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

/// `regular`'s left rail in two pieces 0.1 m apart: the lower reaches 0.3 m
/// beyond the bottom, the upper 0.3 m beyond the top, neither beyond both.
/// Its right rail is one piece.
fn in_pieces() -> Stairs {
    stairs()
        .rail(
            "regular",
            WalkingStretch::Flight,
            "lower_piece",
            rail((1.25, 1.3), (-0.3, 0.4), (0.9, 0.9), (Some(0.0), None)),
        )
        .rail(
            "regular",
            WalkingStretch::Flight,
            "upper_piece",
            rail((1.25, 1.3), (0.5, 1.14), (0.9, 0.9), (None, Some(0.0))),
        )
        .rail(
            "regular",
            WalkingStretch::Flight,
            "left_rail",
            rail((-0.1, -0.05), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        )
}

fn piece_parameters(gap: f64) -> Vec<(&'static str, ParameterValue)> {
    handrail_parameters(vec![
        ("handrail_height_minimum", metres(0.8)),
        ("handrail_extension_minimum", metres(0.3)),
        ("handrail_gap_maximum", metres(gap)),
        ("handrail_sides", string("one")),
    ])
}

#[test]
fn a_handrail_in_pieces_extends_from_its_ends_and_its_gaps_are_found() {
    // `irregular`'s left pieces lie one within the other.
    let stairs = in_pieces()
        .rail(
            "irregular",
            WalkingStretch::Flight,
            "lower_piece",
            rail((1.25, 1.3), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        )
        .rail(
            "irregular",
            WalkingStretch::Flight,
            "upper_piece",
            rail((1.3, 1.35), (0.0, 0.84), (0.9, 0.9), (None, None)),
        );
    let evaluation = check_stairs(model(), stairs, piece_parameters(0.05));
    let (lower, upper) = (id("lower_piece"), id("upper_piece"));
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            format!(
                "handrail pieces {lower} and {upper} along the left side of the flight leave a \
                 gap of 0.1 m in plan; at most 0.05 m allowed"
            )
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related,
        [lower.clone(), upper.clone()]
    );
    let messages: Vec<String> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("irregular")))
        .map(|outcome| outcome.message().to_owned())
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    for what in ["extension", "continuity"] {
        assert!(
            messages.iter().any(|message| message.starts_with(&format!(
                "the handrails along the left side of the flight ({lower}, {upper}) lie beside or \
                 within one another"
            )) && message
                .ends_with(&format!("so its {what} is not measured"))),
            "{messages:?}"
        );
    }
}

#[test]
fn a_handrail_in_pieces_within_the_allowed_gap_passes() {
    let evaluation = check_stairs(model(), in_pieces(), piece_parameters(0.15));
    assert!(
        findings(&evaluation)
            .iter()
            .all(|(object, _)| object != "regular"),
        "{:?}",
        findings(&evaluation)
    );
    assert!(
        !unevaluated(&evaluation)
            .iter()
            .any(|(object, _)| object == "regular"),
        "{:?}",
        unevaluated(&evaluation)
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
/// width (its winders taper); a service refusing its landings and handrails
/// (a winder at an end, a rail it cannot place) leaves those checks not
/// evaluated rather than judged on a frame the flight does not have.
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

/// A service placing a turning flight's landings and handrails in its
/// straight parts: the landing compared with the tread meeting it, and the
/// handrail along a side extending from its first piece's part at the
/// bottom and its last piece's at the top.
#[test]
fn a_turning_flights_landings_and_rails_are_judged_in_its_parts() {
    // `winder` climbs along x over its first two treads (0.9 m wide), then
    // along y; across y (towards -x) its upper part spans -1.2 .. -0.3.
    let y = MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap();
    let parts = vec![
        StretchPart::try_new(x(), (point(0.0), point(0.9))).unwrap(),
        StretchPart::try_new(y, (point(-1.2), point(-0.3))).unwrap(),
    ];
    let stairs = Stairs::default()
        .parts("winder", winder())
        .in_parts("winder", parts)
        .landing("winder", WalkingEnd::FlightTop, "slab", Some((2.0, 0.8)))
        // The left rail: along the lower part from 0.3 m before the foot,
        // on along the upper part to 0.3 m past the top, overlapping at
        // the turn.
        .rail(
            "winder",
            WalkingStretch::Flight,
            "left_rail",
            rail((0.95, 1.0), (-0.3, 0.6), (0.9, 0.9), (Some(0.0), None)),
        )
        .rail(
            "winder",
            WalkingStretch::Flight,
            "upper_piece",
            rail((-0.25, -0.2), (0.5, 1.42), (0.9, 0.9), (None, Some(0.0))).in_part(1),
        )
        // The right rail runs along the upper part only.
        .rail(
            "winder",
            WalkingStretch::Flight,
            "low_rail",
            rail((-1.3, -1.25), (0.5, 1.42), (0.9, 0.9), (None, Some(0.0))).in_part(1),
        );
    let evaluation = check_stairs(
        model(),
        stairs,
        handrail_parameters(vec![
            ("landing_objects", slabs()),
            ("landing_at_least_walking_width", boolean(true)),
            ("handrail_height_minimum", metres(0.8)),
            ("handrail_extension_minimum", metres(0.3)),
            ("handrail_gap_maximum", metres(0.05)),
            ("handrail_sides", string("both")),
        ]),
    );
    let low = id("low_rail");
    let winder: Vec<(String, String)> = findings(&evaluation)
        .into_iter()
        .filter(|(object, _)| object == "winder")
        .collect();
    assert_eq!(
        winder,
        [
            (
                "winder".into(),
                "the landing at the top of the flight is 0.8 m wide; at least the flight's width \
                 (0.9 m) required"
                    .into()
            ),
            (
                "winder".into(),
                format!(
                    "handrail {low} runs along a later straight part of the flight only, so it \
                     does not reach beyond the bottom of the flight; at least 0.3 m required"
                )
            ),
        ]
    );
    assert!(
        !unevaluated(&evaluation)
            .iter()
            .any(|(object, _)| object == "winder"),
        "{:?}",
        evaluation.not_evaluated_outcomes()
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

fn check_stairs_with(
    stairs: Stairs,
    floor: Option<Floor>,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model().evaluate_with(
        &StairGeometryCheck,
        &rule(STAIR, kind("flight"), parameters),
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

fn minus_x() -> MetricDirection {
    MetricDirection::try_new([-1.0, 0.0, 0.0]).unwrap()
}

/// A free space before a flight's first step and after its last: a
/// cupboard 1 m before the bottom riser obstructs a 1.5 m end space, not a
/// 0.9 m one.
#[test]
fn a_cupboard_before_the_bottom_riser_obstructs_a_flights_end_space() {
    // `regular` climbs along x from its first riser at x 0 to its last
    // tread's nosing at 0.84 m, 1.2 m wide from y 0; the bin stands 1 m to
    // 1.2 m before the first riser.
    let stairs = || {
        stairs()
            .end("regular", WalkingEnd::FlightBottom, minus_x(), 0.0)
            .end("regular", WalkingEnd::FlightTop, x(), 0.84)
            .end("irregular", WalkingEnd::FlightBottom, minus_x(), 0.0)
            .end("irregular", WalkingEnd::FlightTop, x(), 0.84)
    };
    let floor = || Floor::default().blocker("bin", [-1.2, 0.4], [-1.0, 0.8]);
    let parameters = |depth: f64| {
        vec![
            ("end_space_depth", metres(depth)),
            ("end_space_width", metres(1.2)),
            ("end_space_height", metres(2.0)),
            ("end_space_obstacles", selector(kind("furniture"))),
        ]
    };
    let evaluation = check_stairs_with(stairs(), Some(floor()), parameters(1.5));
    let bin = id("bin");
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                format!(
                    "{bin} obstructs the free space at the bottom of the flight (1.5 m deep, \
                     1.2 m wide)"
                )
            ),
            (
                "regular".into(),
                format!(
                    "{bin} obstructs the free space at the bottom of the flight (1.5 m deep, \
                     1.2 m wide)"
                )
            ),
        ]
    );
    assert_eq!(evaluation.findings()[0].related, [bin]);
    // The flight in pieces is not measured.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let evaluation = check_stairs_with(stairs(), Some(floor()), parameters(0.9));
    assert!(findings(&evaluation).is_empty());
    // Without the free-space service the end spaces are not checked.
    let evaluation = check_stairs_with(stairs(), None, parameters(1.5));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(unevaluated(&evaluation).len(), 5);
}

/// `regular`'s treads overhanging the one below by 2 cm, every riser
/// closed.
fn overhung() -> TreadFlight {
    let treads: Vec<Tread> = (0..4_u8)
        .map(|step| {
            let (elevation, front) = (0.17 * f64::from(step + 1), 0.28 * f64::from(step));
            Tread::try_new(point(elevation), point(front), point(front + 0.3))
                .unwrap()
                .with_sides(point(0.0), point(1.2))
                .unwrap()
                .with_riser_below(RiserClosure::Closed)
        })
        .collect();
    let top = treads.last().unwrap().elevation();
    TreadFlight::try_new(
        straight("regular"),
        WalkingLine::Straight(x()),
        point(0.0),
        top,
        treads,
        Evidence::exact(source(), "tread-flight:regular"),
    )
    .unwrap()
}

/// A maximum bounds the extension, and from the riser the top extension
/// loses the last tread's overhang.
#[test]
fn a_handrail_extension_is_bounded_and_measured_from_the_riser() {
    // The rail reaches 0.3 m beyond both ends of the pitch line.
    let rails = |stairs: Stairs| {
        stairs.rail(
            "regular",
            WalkingStretch::Flight,
            "left_rail",
            rail((1.25, 1.3), (-0.3, 1.14), (0.9, 0.9), LEVEL),
        )
    };
    let evaluation = check_stairs(
        model(),
        rails(stairs()),
        handrail_parameters(vec![("handrail_extension_maximum", metres(0.25))]),
    );
    let rail = id("left_rail");
    assert_eq!(
        findings(&evaluation),
        [
            (
                "regular".into(),
                format!(
                    "handrail {rail} reaches 0.3 m beyond the bottom of the flight; at most 0.25 \
                     m required"
                )
            ),
            (
                "regular".into(),
                format!(
                    "handrail {rail} reaches 0.3 m beyond the top of the flight; at most 0.25 m \
                     required"
                )
            ),
        ]
    );
    // From the riser: the closed first riser lies under the first nosing,
    // the last 2 cm behind the last nosing.
    let from_riser = |minimum: f64| {
        handrail_parameters(vec![
            ("handrail_extension_minimum", metres(minimum)),
            ("handrail_extension_from", string("riser")),
        ])
    };
    let evaluation = check_stairs(
        model(),
        rails(stairs().flight(overhung())),
        from_riser(0.29),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            format!(
                "handrail {rail} reaches 0.28 m beyond the top riser of the flight; at least \
                 0.29 m required"
            )
        )]
    );
    // Measured from the nosing it passes.
    let evaluation = check_stairs(
        model(),
        rails(stairs().flight(overhung())),
        handrail_parameters(vec![("handrail_extension_minimum", metres(0.29))]),
    );
    assert!(findings(&evaluation).is_empty());
    // A first riser that is not measured may lie anywhere under the first
    // tread, and a last one not measured is unknown.
    let evaluation = check_stairs(model(), rails(stairs()), from_riser(0.35));
    assert!(findings(&evaluation).is_empty());
    let undecided: Vec<_> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| {
            outcome
                .object_id()
                .is_some_and(|id| id.local_id == "regular")
        })
        .map(|outcome| outcome.message().to_owned())
        .collect();
    assert_eq!(undecided.len(), 2, "{undecided:?}");
    assert!(
        undecided[0].contains("beyond the bottom riser of the flight, which straddles"),
        "{undecided:?}"
    );
    assert!(undecided[1].contains("is not known"), "{undecided:?}");
    // The riser applies to stairs, with an extension bound.
    for parameters in [
        handrail_parameters(vec![
            ("handrail_sides", string("one")),
            ("handrail_extension_from", string("riser")),
        ]),
        handrail_parameters(vec![
            ("handrail_extension_minimum", metres(0.3)),
            ("handrail_extension_from", string("tread")),
        ]),
        handrail_parameters(vec![
            ("handrail_extension_minimum", metres(0.3)),
            ("handrail_extension_maximum", metres(0.2)),
        ]),
    ] {
        let evaluation = check_stairs(model(), stairs(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn winders_too_gentle_are_found_and_straight_treads_are_not_winders() {
    let check = |minimum: f64| {
        check_stairs(
            model(),
            stairs().parts("winder", winder()),
            vec![("winder_angle_minimum", degrees(minimum))],
        )
    };
    let evaluation = check(40.0);
    assert_eq!(
        findings(&evaluation),
        [(
            "winder".into(),
            "winder angle 2 of 4 is 35°, winder angle 3 of 4 is 35°; at least 40° required for a \
             winder"
                .into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
    let evaluation = check(30.0);
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    let evaluation = check_stairs(
        model(),
        stairs(),
        vec![
            ("winder_angle_minimum", degrees(40.0)),
            ("winder_angle_maximum", degrees(30.0)),
        ],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// A ramp's two end landings take their own minimums; the landing between
/// its runs the general one.
#[test]
fn a_ramps_end_landings_have_their_own_minimums() {
    let stairs = stairs()
        .landing(
            "gentle",
            WalkingEnd::RunBottom(0),
            "floor",
            Some((1.4, 1.5)),
        )
        .landing("gentle", WalkingEnd::RunTop(0), "gentle", Some((1.4, 1.5)))
        .landing(
            "gentle",
            WalkingEnd::RunBottom(1),
            "gentle",
            Some((1.4, 1.5)),
        )
        .landing("gentle", WalkingEnd::RunTop(1), "slab", Some((1.6, 1.5)));
    let evaluation = check_ramps_with(
        stairs,
        None,
        vec![
            ("landing_objects", slabs()),
            ("landing_depth_minimum", metres(1.2)),
            ("end_landing_depth_minimum", metres(1.5)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "gentle".into(),
            "the landing at the bottom of run 1 of 2 is 1.4 m deep; at least 1.5 m required".into()
        )]
    );
}

/// A flight like `flight`, four 0.17 m risers, standing on `base`.
fn raised_flight(object: &str, base: f64) -> TreadFlight {
    let treads: Vec<Tread> = (0..4_u8)
        .map(|step| {
            let (elevation, front) = (base + 0.17 * f64::from(step + 1), 0.28 * f64::from(step));
            Tread::try_new(point(elevation), point(front), point(front + 0.28))
                .unwrap()
                .with_sides(point(0.0), point(1.2))
                .unwrap()
        })
        .collect();
    let top = treads.last().unwrap().elevation();
    TreadFlight::try_new(
        straight(object),
        WalkingLine::Straight(x()),
        point(base),
        top,
        treads,
        Evidence::exact(source(), format!("tread-flight:{object}")),
    )
    .unwrap()
}

/// Distances in space between pairs of rails; nothing has a box.
#[derive(Default)]
struct Rails(BTreeMap<(String, String), f64>);

impl Rails {
    fn apart(mut self, a: &str, b: &str, distance: f64) -> Self {
        self.0.insert((a.into(), b.into()), distance);
        self.0.insert((b.into(), a.into()), distance);
        self
    }
}

impl ProximityService for Rails {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let pair = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let distance = self.0.get(&pair).copied().unwrap_or(5.0);
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            distance,
            distance,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("distance:{}:{}", pair.0, pair.1)),
        )
    }
}

/// Stair `stair` of flights `lower` (0 to 0.68 m) and `upper` (0.68 to
/// 1.36 m), reached along `parts`, with the rails `l1` and `r1` along the
/// lower flight's left and right sides and `l2` and `r2` along the upper
/// one's, landing rail `r3`, slab `landing` and door `exit`.
fn two_flights() -> Model {
    Model::default()
        .object("stair", "stair")
        .object("lower", "flight")
        .object("upper", "flight")
        .object("landing", "slab")
        .object("exit", "door")
        .object("l1", "railing")
        .object("l2", "railing")
        .object("r1", "railing")
        .object("r2", "railing")
        .object("r3", "railing")
        .edge("parts", "stair", "lower")
        .edge("parts", "stair", "upper")
        .edge("parts", "stair", "landing")
}

fn two_flight_stairs() -> Stairs {
    let rail_at = |left: f64, right: f64| rail((left, right), (-0.3, 1.14), (0.9, 0.9), LEVEL);
    Stairs::default()
        .flight(raised_flight("lower", 0.0))
        .flight(raised_flight("upper", 0.68))
        .rail("lower", WalkingStretch::Flight, "l1", rail_at(1.25, 1.3))
        .rail("lower", WalkingStretch::Flight, "r1", rail_at(-0.1, -0.05))
        .rail("upper", WalkingStretch::Flight, "l2", rail_at(1.25, 1.3))
        .rail("upper", WalkingStretch::Flight, "r2", rail_at(-0.1, -0.05))
        .landing("lower", WalkingEnd::FlightTop, "landing", Some((1.5, 1.2)))
}

fn whole_parameters(
    extra: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("stair_path", common::strings(&["parts"])),
        ("stair_flights", selector(kind("flight"))),
    ];
    parameters.extend(extra);
    parameters
}

fn check_whole(
    stairs: Stairs,
    floor: Floor,
    rails: Rails,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    two_flights().evaluate_with(
        &StairGeometryCheck,
        &rule(STAIR, kind("stair"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
            services
                .register(FreeSpaceServiceHandle::new(Arc::new(floor)))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(rails)))
                .unwrap();
        },
    )
}

/// A two-flight stair whose inner rail stops at the landing fails, unless
/// a selected door stands there; its rise is the whole stair's.
#[test]
fn a_stairs_inner_rail_must_continue_across_its_landing() {
    // The right rails are joined by the landing rail r3; the left rails
    // lie 1 m apart with nothing between them.
    let rails = || {
        Rails::default()
            .apart("r1", "r3", 0.0)
            .apart("r3", "r2", 0.0)
            .apart("l1", "l2", 1.0)
    };
    let parameters = |doors: bool| {
        let mut parameters = handrail_parameters(whole_parameters(vec![
            ("handrail_continuous_across_landings", boolean(true)),
            ("maximum_total_rise", metres(1.2)),
        ]));
        if doors {
            parameters.push(("landing_objects", slabs()));
            parameters.push(("handrail_break_doors", selector(kind("door"))));
            parameters.push(("landing_door_height", metres(2.0)));
        }
        parameters
    };
    let evaluation = check_whole(
        two_flight_stairs(),
        Floor::default(),
        rails(),
        parameters(false),
    );
    let (lower, upper) = (id("lower"), id("upper"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "stair".into(),
                "the stair rises 1.36 m from its lowest flight's base to its highest flight's \
                 top; at most 1.2 m allowed"
                    .into()
            ),
            (
                "stair".into(),
                format!(
                    "the handrail along the left side stops at the landing between {lower} and \
                     {upper}: {} and {} are not joined by selected rails within 0 m of each other",
                    id("l1"),
                    id("l2")
                )
            ),
        ]
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );
    // A door standing in the wall beside the landing breaks the rail there.
    let floor = Floor::default().blocker("exit", [0.5, -0.15], [1.0, -0.05]);
    let evaluation = check_whole(two_flight_stairs(), floor, rails(), parameters(true));
    assert_eq!(common::flagged(&evaluation), ["stair"]);
    assert!(findings(&evaluation)[0].1.starts_with("the stair rises"));
    // A door elsewhere does not.
    let floor = Floor::default().blocker("exit", [5.0, 5.0], [6.0, 6.0]);
    let evaluation = check_whole(two_flight_stairs(), floor, rails(), parameters(true));
    assert_eq!(findings(&evaluation).len(), 2);
}

#[test]
fn whole_stair_declarations_are_checked() {
    for parameters in [
        vec![("maximum_total_rise", metres(3.0))],
        vec![
            ("stair_path", common::strings(&["parts"])),
            ("maximum_total_rise", metres(3.0)),
        ],
        handrail_parameters(whole_parameters(vec![(
            "handrail_break_doors",
            selector(kind("door")),
        )])),
        handrail_parameters(vec![("handrail_continuous_across_landings", boolean(true))]),
    ] {
        let evaluation = check_whole(
            two_flight_stairs(),
            Floor::default(),
            Rails::default(),
            parameters,
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Axis-aligned tactile strips in plan, each `(centre, half extents)`,
/// 5 mm thick on the floor at `z`.
#[derive(Clone, Default)]
struct Strips(BTreeMap<ObjectId, ([f64; 2], [f64; 2], f64)>);

impl Strips {
    fn strip(mut self, object: &str, centre: [f64; 2], half: [f64; 2], z: f64) -> Self {
        self.0.insert(id(object), (centre, half, z));
        self
    }

    fn get(&self, object: &ObjectId) -> Option<([f64; 2], [f64; 2], f64)> {
        self.0.get(object).copied()
    }
}

impl PlanSpanService for Strips {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("not measured here".into()))
    }

    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("not measured here".into()))
    }

    fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        let (centre, half, _) = self
            .get(object)
            .ok_or_else(|| PlanSpanError::UnknownObject(object.clone()))?;
        PlanRectangle::try_new(
            object.clone(),
            centre,
            0.0,
            [[1.0, 0.0], [0.0, 1.0]],
            0.0,
            [(half[0], half[0]), (half[1], half[1])],
            RectangleOrientation::Unique,
            Evidence::exact(source(), format!("rectangle:{}", object.local_id)),
        )
    }
}

impl PlanAreaService for Strips {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (_, half, _) = self
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let area = 4.0 * half[0] * half[1];
        PlanArea::try_new(
            area,
            area,
            Evidence::exact(source(), format!("footprint:{}", object.local_id)),
        )
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }
}

impl VerticalExtentService for Strips {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (_, _, z) = self
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(z)?,
            ElevationInterval::exact(z + 0.005)?,
            Evidence::exact(source(), format!("extent:{}", object.local_id)),
        )
    }
}

fn tactile_parameters(
    extra: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("tactile_objects", selector(kind("tactile"))),
        ("tactile_offset", metres(0.3)),
        ("tactile_depth", metres(0.6)),
    ];
    parameters.extend(extra);
    parameters
}

fn check_tactile(
    model: Model,
    selected: &str,
    stairs: Stairs,
    strips: Strips,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &StairGeometryCheck,
        &rule(STAIR, kind(selected), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
            services
                .register(PlanSpanServiceHandle::new(Arc::new(strips.clone())))
                .unwrap();
            services
                .register(PlanAreaServiceHandle::new(Arc::new(strips.clone())))
                .unwrap();
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(strips)))
                .unwrap();
        },
    )
}

/// A tactile strip 0.6 m deep must start 0.3 m before the first riser and
/// beyond the last, across the flight: a missing or a narrow one is found.
#[test]
fn a_missing_or_narrow_tactile_strip_is_found() {
    // `regular`'s first riser lies at x 0, `irregular`'s at x -10, both
    // leaving along -x at the bottom; both arrive at x 0.84 at the top.
    // `t1` covers x -0.9 to -0.3 across `regular` (y 0 to 1.2); `t2` only
    // x -10.6 to -10.3 before `irregular`.
    let stairs = stairs()
        .end("regular", WalkingEnd::FlightBottom, minus_x(), 0.0)
        .end("regular", WalkingEnd::FlightTop, x(), 0.84)
        .end("irregular", WalkingEnd::FlightBottom, minus_x(), 10.0)
        .end("irregular", WalkingEnd::FlightTop, x(), 20.0);
    let strips = Strips::default()
        .strip("t1", [-0.6, 0.6], [0.3, 0.6], 0.0)
        .strip("t2", [-10.45, 0.6], [0.15, 0.6], 0.0)
        // On the top landing of `irregular`, x 20.3 to 20.9, but a storey
        // lower.
        .strip("t3", [20.6, 0.6], [0.3, 0.6], -3.0);
    let model = model()
        .object("t1", "tactile")
        .object("t2", "tactile")
        .object("t3", "tactile");
    let evaluation = check_tactile(model, "flight", stairs, strips, tactile_parameters(vec![]));
    let what = |end: &str| {
        let (riser, place) = if end == "top" {
            ("last", "beyond")
        } else {
            ("first", "before")
        };
        format!(
            "the tactile strip at the {end} of the flight (0.6 m deep, 0.3 m {place} the {riser} \
             riser, across the flight)"
        )
    };
    assert_eq!(
        findings(&evaluation),
        [
            (
                "irregular".into(),
                format!(
                    "{} is not covered: {} leave part of it bare",
                    what("bottom"),
                    id("t2")
                )
            ),
            (
                "irregular".into(),
                format!("no selected tactile surface lies in {}", what("top"))
            ),
            (
                "regular".into(),
                format!("no selected tactile surface lies in {}", what("top"))
            ),
        ]
    );
    assert_eq!(evaluation.findings()[0].related, [id("t2")]);
    // The flight in pieces is not measured.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// In a whole stair the landings between flights need no strip unless the
/// rule asks for one there.
#[test]
fn tactile_strips_on_intermediate_landings_are_asked_for() {
    let evaluate = |intermediate: bool| {
        let parameters = tactile_parameters(whole_parameters(vec![(
            "tactile_on_intermediate_landings",
            boolean(intermediate),
        )]));
        check_tactile(
            two_flights(),
            "stair",
            two_flight_stairs(),
            Strips::default(),
            parameters,
        )
    };
    let ends = |evaluation: &CapabilityEvaluation| -> Vec<(String, bool)> {
        findings(evaluation)
            .into_iter()
            .map(|(flight, message)| (flight, message.contains("at the top")))
            .collect()
    };
    assert_eq!(
        ends(&evaluate(false)),
        [("lower".into(), false), ("upper".into(), true)]
    );
    assert_eq!(
        ends(&evaluate(true)),
        [
            ("lower".into(), false),
            ("lower".into(), true),
            ("upper".into(), false),
            ("upper".into(), true),
        ]
    );
    // The strip's parameters are declared together.
    let evaluation = check_tactile(
        model(),
        "flight",
        stairs(),
        Strips::default(),
        vec![("tactile_objects", selector(kind("tactile")))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// A clear width and the obstacles leaving it.
type Narrowing = (f64, Vec<ObjectId>);

/// Clear widths per subject and stretch: the width each set of obstacles
/// leaves; the walking surface's 1.2 m where no requested set applies.
#[derive(Default)]
struct Narrowed {
    stairs: Stairs,
    widths: BTreeMap<(ObjectId, WalkingStretch), Vec<Narrowing>>,
    /// Landing clear widths per subject and end: the carrier, the width and
    /// the obstacles reaching its lower and higher side.
    landings: BTreeMap<(ObjectId, WalkingEnd), StatedClearLanding>,
}

/// A landing's carrier, clear width and the obstacles bounding its sides.
type StatedClearLanding = (ObjectId, f64, Vec<ObjectId>, Vec<ObjectId>);

impl Narrowed {
    fn width(mut self, subject: &str, stretch: WalkingStretch, width: f64, by: &[&str]) -> Self {
        self.widths
            .entry((id(subject), stretch))
            .or_default()
            .push((width, by.iter().map(|local| id(local)).collect()));
        self
    }

    fn landing(
        mut self,
        subject: &str,
        end: WalkingEnd,
        (carrier, width): (&str, f64),
        (low, high): (&[&str], &[&str]),
    ) -> Self {
        let ids = |locals: &[&str]| locals.iter().map(|local| id(local)).collect();
        self.landings.insert(
            (id(subject), end),
            (id(carrier), width, ids(low), ids(high)),
        );
        self
    }
}

impl WalkingSurfaceService for Narrowed {
    fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        self.stairs.measure_tread_flight(request)
    }

    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
        self.stairs.measure_sloped_runs(object)
    }

    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
        self.stairs.measure_headroom(request)
    }

    fn measure_clear_width(
        &self,
        request: &ClearWidthRequest,
    ) -> Result<ClearWidthEvidence, WalkingSurfaceError> {
        let (width, governing) = self
            .widths
            .get(&(request.subject().clone(), request.stretch()))
            .into_iter()
            .flatten()
            .filter(|(_, by)| by.iter().all(|object| request.obstacles().contains(object)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .cloned()
            .unwrap_or((1.2, vec![]));
        ClearWidthEvidence::try_new(
            request.clone(),
            MeasuredInterval::try_new(width - 1e-9, width + 1e-9)?,
            governing,
            Evidence {
                source: source(),
                locator: format!("clear-width:{}", request.subject().local_id),
                exact: false,
            },
        )
    }

    /// Landings stated per end, found only when their carrier is requested,
    /// bounded by the requested obstacles stated for each side.
    fn measure_landing_clear_width(
        &self,
        request: &LandingClearWidthRequest,
    ) -> Result<LandingClearWidthEvidence, WalkingSurfaceError> {
        let landing = request.landing();
        let found = self
            .landings
            .get(&(landing.subject().clone(), landing.end()))
            .filter(|(carrier, ..)| landing.candidates().contains(carrier))
            .map(|(carrier, width, low, high)| {
                let requested = |objects: &Vec<ObjectId>| -> Vec<ObjectId> {
                    objects
                        .iter()
                        .filter(|object| request.obstacles().contains(object))
                        .cloned()
                        .collect()
                };
                let (low, high) = (requested(low), requested(high));
                let governing = low.iter().chain(&high).cloned().collect();
                LandingClearWidth::new(
                    carrier.clone(),
                    MeasuredInterval::try_new(width - 1e-9, width + 1e-9).unwrap(),
                    governing,
                    (low, high),
                )
            });
        LandingClearWidthEvidence::try_new(
            request.clone(),
            found,
            Evidence {
                source: source(),
                locator: format!("landing-clear-width:{}", request.subject().local_id),
                exact: false,
            },
        )
    }
}

fn check_clear(
    stairs: Narrowed,
    capability: &dyn axioval_engine::RuleCapability,
    selected: &str,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let id = if selected == "ramp" { RAMP } else { STAIR };
    model().object("wall", "wall").evaluate_with(
        capability,
        &rule(id, kind(selected), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
        },
    )
}

fn clear_parameters(minimum: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("clear_width_minimum", metres(minimum)),
        ("clear_width_obstacles", selector(kind("railing"))),
        ("clear_width_band_from", metres(0.5)),
        ("clear_width_band_to", metres(1.5)),
    ]
}

/// A 1.2 m flight with 0.1 m rails inside both sides fails a 1.1 m clear
/// width; its walking surface alone would not.
#[test]
fn a_flight_narrowed_by_its_rails_fails_its_clear_width() {
    let narrowed = || {
        Narrowed {
            stairs: stairs(),
            ..Narrowed::default()
        }
        .width(
            "regular",
            WalkingStretch::Flight,
            1.0,
            &["left_rail", "low_rail"],
        )
        .width("gentle", WalkingStretch::Run(1), 1.3, &["ramp_rail"])
    };
    let evaluation = check_clear(
        narrowed(),
        &StairGeometryCheck,
        "flight",
        clear_parameters(1.1),
    );
    let (left, low) = (id("left_rail"), id("low_rail"));
    assert_eq!(
        findings(&evaluation),
        [(
            "regular".into(),
            format!(
                "the clear width of the flight 0.5 m to 1.5 m above its pitch line is 1 m beside \
                 {left} and {low}; at least 1.1 m required"
            )
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [left, low]);
    // A ramp's runs are judged one by one.
    let evaluation = check_clear(
        narrowed(),
        &RampGeometryCheck,
        "ramp",
        clear_parameters(1.4),
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found[0]
            .1
            .starts_with("the clear width of run 1 of 2 0.5 m to 1.5 m")
    );
    assert!(found[1].1.contains("is 1.3 m beside"), "{found:?}");
    // The band's bottom lies below its top, and the four come together.
    for parameters in [
        vec![
            ("clear_width_minimum", metres(1.1)),
            ("clear_width_obstacles", selector(kind("railing"))),
        ],
        {
            let mut parameters = clear_parameters(1.1);
            parameters.push(("clear_width_band_to", metres(0.4)));
            parameters
                .retain(|(name, value)| *name != "clear_width_band_to" || *value == metres(0.4));
            parameters
        },
    ] {
        let evaluation = check_clear(narrowed(), &StairGeometryCheck, "flight", parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

fn landing_clear_parameters(landing: f64, total: f64) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = clear_parameters(1.1);
    parameters.extend([
        ("landing_clear_width_minimum", metres(landing)),
        ("total_clear_width_minimum", metres(total)),
        ("landing_objects", slabs()),
    ]);
    parameters
}

/// A 1.2 m flight arriving at a landing 1 m wide between its rails fails a
/// 1.1 m landing minimum and a 1.1 m total, and passes a 1.1 m flight
/// minimum.
#[test]
fn a_narrow_landing_fails_its_own_and_the_total_clear_width() {
    let narrowed = |high: &'static [&'static str]| {
        Narrowed {
            stairs: stairs(),
            ..Narrowed::default()
        }
        .landing(
            "regular",
            WalkingEnd::FlightTop,
            ("slab", 1.0),
            (&["left_rail"], high),
        )
    };
    let evaluation = check_clear(
        narrowed(&["low_rail"]),
        &StairGeometryCheck,
        "flight",
        landing_clear_parameters(1.1, 1.1),
    );
    let (left, low) = (id("left_rail"), id("low_rail"));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "regular".into(),
                format!(
                    "the clear width of the landing at the top of the flight 0.5 m to 1.5 m \
                     above its level is 1 m beside {left} and {low}; at least 1.1 m required"
                )
            ),
            (
                "regular".into(),
                "the least clear width of the flight and its landings is 1 m, at the landing at \
                 the top of the flight; at least 1.1 m required"
                    .into()
            ),
        ]
    );
    // The fixture's winder is never measured.
    assert_eq!(
        unevaluated(&evaluation),
        [("winder".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // The flight alone is wide enough.
    let evaluation = check_clear(
        narrowed(&["low_rail"]),
        &StairGeometryCheck,
        "flight",
        clear_parameters(1.1),
    );
    assert!(findings(&evaluation).is_empty());
    // A landing a side of which nothing selected bounds is not evaluated;
    // its total is not decided either.
    let evaluation = check_clear(
        narrowed(&[]),
        &StairGeometryCheck,
        "flight",
        landing_clear_parameters(0.9, 0.9),
    );
    assert!(findings(&evaluation).is_empty());
    let open = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("regular")))
        .map(|outcome| outcome.message().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(open.len(), 2, "{open:?}");
    assert!(
        open[0].starts_with(
            "no selected obstacle bounds the left side of the landing at the top of the flight"
        ),
        "{open:?}"
    );
    assert!(open[1].contains("but a width may be narrower"), "{open:?}");
    // The landing minimum needs landing_objects and a clear-width band.
    for parameters in [
        {
            let mut parameters = landing_clear_parameters(1.1, 1.1);
            parameters.retain(|(name, _)| *name != "landing_objects");
            parameters
        },
        vec![
            ("landing_clear_width_minimum", metres(1.1)),
            ("landing_objects", slabs()),
        ],
        {
            let mut parameters = landing_clear_parameters(1.1, 1.1);
            parameters.push(("landing_clear_width_minimum", metres(-1.0)));
            parameters.retain(|(name, value)| {
                *name != "landing_clear_width_minimum" || *value == metres(-1.0)
            });
            parameters
        },
    ] {
        let evaluation = check_clear(narrowed(&[]), &StairGeometryCheck, "flight", parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// In whole-stair mode the total is the least over the stair's flights
/// and the landings between them, reported on the stair.
#[test]
fn a_stairs_least_clear_width_includes_its_intermediate_landing() {
    let narrowed = Narrowed {
        stairs: two_flight_stairs(),
        ..Narrowed::default()
    }
    .landing(
        "lower",
        WalkingEnd::FlightTop,
        ("landing", 1.0),
        (&["r3"], &["l1"]),
    );
    let mut parameters = whole_parameters(vec![
        ("clear_width_obstacles", selector(kind("railing"))),
        ("clear_width_band_from", metres(0.5)),
        ("clear_width_band_to", metres(1.5)),
        ("total_clear_width_minimum", metres(1.1)),
        ("landing_objects", slabs()),
    ]);
    parameters.push(("landing_objects", slabs()));
    parameters.dedup_by(|a, b| a.0 == b.0);
    let evaluation = two_flights().evaluate_with(
        &StairGeometryCheck,
        &rule(STAIR, kind("stair"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(narrowed)))
                .unwrap();
        },
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "stair".into(),
            format!(
                "the least clear width of the stair's flights and the landings between them is \
                 1 m, at the landing at the top of flight {}; at least 1.1 m required",
                id("lower")
            )
        )]
    );
    assert!(evaluation.findings()[0].related.contains(&id("lower")));
}

fn check_ramps_near(
    stairs: Stairs,
    proximity: impl ProximityService,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model().evaluate_with(
        &RampGeometryCheck,
        &rule(RAMP, kind("ramp"), parameters),
        |services| {
            services
                .register(WalkingSurfaceServiceHandle::new(Arc::new(stairs)))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(proximity)))
                .unwrap();
        },
    )
}

/// Ramp `gentle`'s two runs (x 0 to 6 and 7.5 to 13.5, a landing between)
/// with rail `lower_piece` along the left of the first and `upper_piece`
/// along the left of the second.
fn ramp_in_pieces() -> Stairs {
    stairs()
        .rail(
            "gentle",
            WalkingStretch::Run(0),
            "lower_piece",
            rail((1.55, 1.6), (-0.3, 6.3), (0.9, 0.9), LEVEL),
        )
        .rail(
            "gentle",
            WalkingStretch::Run(1),
            "upper_piece",
            rail((1.55, 1.6), (7.2, 13.8), (0.9, 0.9), LEVEL),
        )
}

fn continuity_parameters(tolerance: f64) -> Vec<(&'static str, ParameterValue)> {
    handrail_parameters(vec![
        ("check_continuous_handrails", boolean(true)),
        ("handrail_continuity_tolerance", metres(tolerance)),
    ])
}

/// Distances in space from an interval per pair, measured on a
/// tessellation; nothing has a box.
#[derive(Default)]
struct Measured(BTreeMap<(String, String), (f64, f64)>);

impl ProximityService for Measured {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let (lower, upper) = self
            .0
            .get(&(a.clone(), b.clone()))
            .or_else(|| self.0.get(&(b.clone(), a.clone())))
            .copied()
            .unwrap_or((5.0, 5.0));
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            GeometryFidelity::tessellated(0.01)?,
            Evidence {
                source: source(),
                locator: format!("distance:{a}:{b}"),
                exact: false,
            },
        )
    }
}

/// Rails stopping 0.4 m short of each other on a ramp's landing are a
/// finding with a 0.1 m tolerance; joined rails pass; a gap straddling the
/// tolerance is not evaluated.
#[test]
fn a_ramps_rails_must_continue_across_its_landings() {
    let evaluation = check_ramps_near(
        ramp_in_pieces(),
        Rails::default().apart("lower_piece", "upper_piece", 0.4),
        continuity_parameters(0.1),
    );
    let (lower, upper) = (id("lower_piece"), id("upper_piece"));
    assert_eq!(
        findings(&evaluation),
        [(
            "gentle".into(),
            format!(
                "the handrail along the left side stops at the landing between run 1 of 2 and \
                 run 2 of 2: {lower} and {upper} are not joined by selected rails within 0.1 m \
                 of each other"
            )
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [lower, upper]);
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(object, _)| object != "gentle"),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    // Within the tolerance, or joined by a rail along the landing, they
    // continue.
    for rails in [
        Rails::default().apart("lower_piece", "upper_piece", 0.05),
        Rails::default()
            .apart("lower_piece", "ramp_rail", 0.0)
            .apart("ramp_rail", "upper_piece", 0.0),
    ] {
        let evaluation = check_ramps_near(ramp_in_pieces(), rails, continuity_parameters(0.1));
        assert!(
            findings(&evaluation).is_empty(),
            "{:?}",
            findings(&evaluation)
        );
        assert!(
            unevaluated(&evaluation)
                .iter()
                .all(|(object, _)| object != "gentle"),
            "{:?}",
            evaluation.not_evaluated_outcomes()
        );
    }
    // A tessellated gap between 0.05 and 0.2 m straddles 0.1 m.
    let mut straddling = Measured::default();
    straddling
        .0
        .insert(("lower_piece".into(), "upper_piece".into()), (0.05, 0.2));
    let evaluation = check_ramps_near(ramp_in_pieces(), straddling, continuity_parameters(0.1));
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    let open: Vec<&str> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| outcome.object_id() == Some(&id("gentle")))
        .map(axioval_engine::CapabilityNotEvaluated::message)
        .collect();
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(
        open[0].starts_with("the handrail along the left side may stop at the landing between"),
        "{open:?}"
    );
    // The tolerance needs the check, and the check the handrail parameters.
    for parameters in [
        handrail_parameters(vec![
            ("handrail_height_minimum", metres(0.8)),
            ("handrail_continuity_tolerance", metres(0.1)),
        ]),
        vec![("check_continuous_handrails", boolean(true))],
        continuity_parameters(-0.1),
    ] {
        let evaluation = check_ramps_near(ramp_in_pieces(), Rails::default(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Plan overlaps per pair (`(0, 0)` overlapping, infinite apart, anything
/// between open on a tessellation) and distances in space per pair, 5 m
/// unless stated; nothing has a box.
#[derive(Default)]
struct Footprints {
    plan: BTreeMap<(String, String), (f64, f64)>,
    space: BTreeMap<(String, String), f64>,
}

impl Footprints {
    fn plan(mut self, a: &str, b: &str, overlap: (f64, f64)) -> Self {
        self.plan.insert((a.into(), b.into()), overlap);
        self.plan.insert((b.into(), a.into()), overlap);
        self
    }

    fn touching(mut self, a: &str, b: &str) -> Self {
        self.space.insert((a.into(), b.into()), 0.0);
        self.space.insert((b.into(), a.into()), 0.0);
        self
    }
}

impl ProximityService for Footprints {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let pair = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let (lower, upper) = if request.projection() == ProximityProjection::PlanOverlap {
            self.plan
                .get(&pair)
                .copied()
                .unwrap_or((f64::INFINITY, f64::INFINITY))
        } else {
            let distance = self.space.get(&pair).copied().unwrap_or(5.0);
            (distance, distance)
        };
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        let fidelity = if exact {
            GeometryFidelity::Exact
        } else {
            GeometryFidelity::tessellated(0.01)?
        };
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            Evidence {
                source: source(),
                locator: format!("distance:{}:{}", pair.0, pair.1),
                exact,
            },
        )
    }
}

/// Ramp `gentle` with rail `ramp_rail` along its first run and
/// `upper_piece` along its second; `lower_piece` is a separate piece only
/// touching `ramp_rail`.
fn railed_ramp() -> Stairs {
    stairs()
        .rail(
            "gentle",
            WalkingStretch::Run(0),
            "ramp_rail",
            rail((1.55, 1.6), (-0.3, 6.0), (0.9, 0.9), LEVEL),
        )
        .rail(
            "gentle",
            WalkingStretch::Run(1),
            "upper_piece",
            rail((1.55, 1.6), (7.5, 13.8), (0.9, 0.9), LEVEL),
        )
}

fn obstruction_parameters() -> Vec<(&'static str, ParameterValue)> {
    handrail_parameters(vec![
        ("check_rails_obstruction", boolean(true)),
        ("accessible_surface_selector", selector(kind("space"))),
    ])
}

/// A rail extension reaching into a selected path is a finding; one
/// turning beside it passes; an overlap a tessellation leaves open is not
/// evaluated.
#[test]
fn ramp_rails_reaching_over_an_accessible_surface_are_found() {
    let outcomes = |footprints: Footprints| {
        let evaluation = check_ramps_near(railed_ramp(), footprints, obstruction_parameters());
        let open: Vec<String> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .filter(|outcome| outcome.object_id() == Some(&id("gentle")))
            .map(|outcome| outcome.message().to_owned())
            .collect();
        (findings(&evaluation), open, evaluation)
    };
    let (rail, hall) = (id("ramp_rail"), id("hall"));
    // The rail's extension reaches 0.3 m into the hall's path.
    let (found, open, evaluation) =
        outcomes(Footprints::default().plan("ramp_rail", "hall", (0.0, 0.0)));
    assert_eq!(
        found,
        [(
            "gentle".into(),
            format!(
                "handrail {rail} of the ramp reaches over the accessible surface {hall} in plan"
            )
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related,
        [hall.clone(), rail.clone()]
    );
    assert!(open.is_empty(), "{open:?}");
    // A rail turning along the wall beside the path stays out of it.
    let (found, open, _) = outcomes(Footprints::default());
    assert!(found.is_empty() && open.is_empty(), "{found:?} {open:?}");
    // A separate extension piece joined to the ramp's rail counts too.
    let (found, _, _) = outcomes(
        Footprints::default()
            .touching("ramp_rail", "lower_piece")
            .plan("lower_piece", "hall", (0.0, 0.0)),
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .1
            .starts_with(&format!("handrail {}", id("lower_piece")))
    );
    // A tessellated overlap straddling zero is not evaluated.
    let (found, open, _) =
        outcomes(Footprints::default().plan("ramp_rail", "hall", (0.0, f64::INFINITY)));
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0].starts_with(&format!("whether handrail {rail} of the ramp reaches over")));
    // The selector and the check come together, with the rail parameters.
    for parameters in [
        handrail_parameters(vec![("check_rails_obstruction", boolean(true))]),
        vec![
            ("check_rails_obstruction", boolean(true)),
            ("accessible_surface_selector", selector(kind("space"))),
        ],
        handrail_parameters(vec![
            ("handrail_height_minimum", metres(0.8)),
            ("accessible_surface_selector", selector(kind("space"))),
        ]),
    ] {
        let evaluation = check_ramps_near(railed_ramp(), Footprints::default(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
