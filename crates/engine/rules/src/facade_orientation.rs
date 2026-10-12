//! Which way an object faces, and how high the obstacles in front of it
//! rise: `azimuth`, `tilt` and `obstruction_angle`.
//!
//! The direction an object faces is that of its faces looking outside, as
//! the facade-area service classifies them (`measure_facade_faces`): a
//! window's outer face, a wall's external side, a roof's top. Faces whose
//! outward normals may be parallel form one direction; the object faces
//! the one direction whose area surely exceeds every other's, counting a
//! partly covered face only towards the most it may hold. Where two
//! directions may hold the most, the object faces none: opposite ones
//! leave its outward side undecided (a window with no room declared
//! behind it looks outside both ways), others several directions.
//!
//! The azimuth is the plan bearing of that direction's outward normals,
//! clockwise from true north as the source states it (or from the plan's
//! y axis), the hull over its faces; the tilt is their angle from straight
//! up. The obstruction angle is the greatest elevation above the horizon
//! of any point of the obstacles, seen from the area-weighted centroid of
//! the direction's faces (wholly outside ones, where it has any) along its
//! plan direction, within the sector, as the line-of-sight service
//! measures it. Obstacles the selection leaves undecided may obstruct:
//! they raise only the upper bound.

use std::f64::consts::{PI, TAU};

use axioval_engine::expression::Interval;
use axioval_engine::{
    Citation, CoordinateSystemServiceHandle, FacadeAreaError, FacadeAreaServiceHandle,
    FacadeFaceSide, FaceNormal, MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleContext, SightError, SightServiceHandle, SkyObstruction,
    SkyObstructionRequest,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, SourceId};

use crate::measured_kinds::{interval, refused, selection};
use crate::support::Unavailable;

const AZIMUTH: &str = "azimuth";
const TILT: &str = "tilt";
const OBSTRUCTION_ANGLE: &str = "obstruction_angle";

/// Two outward normals face one direction when the angle between them is
/// within their boxes' spreads and this much more, in radians.
const PARALLEL: f64 = 1e-9;

/// Measures `azimuth`, `tilt` and `obstruction_angle`.
pub(crate) struct FacingMeasures;

/// The memo key of one object's facing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FacingKey;

/// The outward faces of the one direction an object faces.
#[derive(Clone, Debug)]
pub(crate) struct Facing {
    /// The outward normal box of each face.
    normals: Vec<FaceNormal>,
    /// The area-weighted centroid of the faces wholly outside, or of all
    /// where none is.
    eye: [f64; 3],
    /// The area-weighted mean of the faces' unit normals.
    toward: [f64; 3],
    /// Whether the faces are of the object's exact shape.
    exact: bool,
    /// Where the faces came from.
    locator: String,
}

impl Facing {
    /// The outward faces `object` faces with, read once per run.
    pub(crate) fn of(context: &RuleContext<'_>, object: &ObjectId) -> Result<Self, Unavailable> {
        MeasuredMemo::of(context.services, (FacingKey, object.clone()), || {
            let service = context
                .services
                .get::<FacadeAreaServiceHandle>()
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::MissingService,
                        "facade-area service is not registered".to_owned(),
                    )
                })?;
            let faces = service
                .measure_facade_faces(object)
                .map_err(|error| facade_unavailable(&error))?;
            let parts: Vec<Part> = faces
                .faces()
                .iter()
                .filter_map(|face| {
                    Part::of(
                        face.normal(),
                        face.area_square_metres(),
                        face.centroid(),
                        face.side(),
                    )
                })
                .collect();
            dominant(&parts).map(|members| {
                let mut facing = Facing::over(&members);
                facing.exact = faces.evidence().exact;
                facing.locator.clone_from(&faces.evidence().locator);
                facing
            })
        })
    }

    fn over(members: &[&Part]) -> Self {
        let outside: Vec<&&Part> = members
            .iter()
            .filter(|part| part.side == FacadeFaceSide::Outside)
            .collect();
        let weighted: Vec<&Part> = if outside.is_empty() {
            members.to_vec()
        } else {
            outside.into_iter().copied().collect()
        };
        let (mut eye, mut toward, mut weight) = ([0.0; 3], [0.0; 3], 0.0);
        for part in &weighted {
            let share = f64::midpoint(part.area.0, part.area.1);
            for axis in 0..3 {
                eye[axis] += part.centroid[axis] * share;
                toward[axis] += part.unit[axis] * share;
            }
            weight += share;
        }
        if weight > 0.0 {
            eye = eye.map(|value| value / weight);
        } else {
            eye = weighted[0].centroid;
        }
        let length = toward.iter().map(|value| value * value).sum::<f64>().sqrt();
        let toward = if length > 0.0 {
            toward.map(|value| value / length)
        } else {
            weighted[0].unit
        };
        Self {
            normals: members.iter().map(|part| part.normal).collect(),
            eye,
            toward,
            exact: true,
            locator: String::new(),
        }
    }

    /// The compass bearing of the outward normals, clockwise from `north`,
    /// the hull over the faces, in `[0, 2π)` by its midpoint.
    pub(crate) fn azimuth(&self, north: [f64; 2]) -> Result<Interval, String> {
        azimuth(&self.normals, self.toward, north)
    }

    /// The angle of the outward normals from straight up, the hull over
    /// the faces, in `[0, π]`.
    fn tilt(&self) -> Result<Interval, String> {
        self.normals
            .iter()
            .map(tilt)
            .try_fold(None, |hull: Option<Interval>, angle| {
                angle.map(|angle| Some(hull.map_or(angle, |hull| hull.hull(angle))))
            })?
            .ok_or_else(|| "no face looks outside".to_owned())
    }
}

/// The plan bearing of each of `normals` clockwise from `north`, hulled
/// on the branch that is continuous around `toward`.
pub(crate) fn azimuth(
    normals: &[FaceNormal],
    toward: [f64; 3],
    north: [f64; 2],
) -> Result<Interval, String> {
    let length = north[0].hypot(north[1]);
    let [north_x, north_y] = north.map(|value| Interval::point(value / length));
    // East is north turned a quarter clockwise.
    let (east_x, east_y) = (north_y, north_x.negate());
    let facing_north = toward[0] * north[0] + toward[1] * north[1] >= 0.0;
    let level = || "the face looking outside is level, so it faces no compass direction";
    let failed = |_| "the azimuth cannot be measured".to_owned();
    let mut hull: Option<Interval> = None;
    for normal in normals {
        let (lower, upper) = (normal.lower(), normal.upper());
        let x = Interval {
            lower: lower[0],
            upper: upper[0],
        };
        let y = Interval {
            lower: lower[1],
            upper: upper[1],
        };
        let sine = x
            .times(east_x)
            .and_then(|left| y.times(east_y).and_then(|right| left.plus(right)))
            .map_err(failed)?;
        let cosine = x
            .times(north_x)
            .and_then(|left| y.times(north_y).and_then(|right| left.plus(right)))
            .map_err(failed)?;
        if sine.contains(0.0) && cosine.contains(0.0) {
            return Err(level().to_owned());
        }
        let angle = if facing_north {
            Interval::atan2(sine, cosine)
        } else {
            // Continuous around south: measured from the opposite way.
            Interval::atan2(sine.negate(), cosine.negate()).map(|turned| Interval {
                lower: (turned.lower + PI).next_down(),
                upper: (turned.upper + PI).next_up(),
            })
        }
        .map_err(|_| "the faces looking outside face several directions".to_owned())?;
        hull = Some(hull.map_or(angle, |hull| hull.hull(angle)));
    }
    let angle = hull.ok_or_else(|| "no face looks outside".to_owned())?;
    let middle = f64::midpoint(angle.lower, angle.upper);
    let turns = (middle / TAU).floor();
    if turns == 0.0 {
        return Ok(angle);
    }
    let shift = turns * TAU;
    Ok(Interval {
        lower: (angle.lower - shift).next_down(),
        upper: (angle.upper - shift).next_up(),
    })
}

/// The angle of `normal` from straight up, in `[0, π]`.
fn tilt(normal: &FaceNormal) -> Result<Interval, String> {
    let (lower, upper) = (normal.lower(), normal.upper());
    let component = |axis: usize| Interval {
        lower: lower[axis],
        upper: upper[axis],
    };
    let failed = |_| "the tilt cannot be measured".to_owned();
    let square = |value: Interval| {
        let magnitude = value.abs();
        magnitude.times(magnitude)
    };
    let across = square(component(0))
        .and_then(|x| square(component(1)).and_then(|y| x.plus(y)))
        .and_then(Interval::sqrt)
        .map_err(failed)?;
    let up = component(2);
    #[allow(clippy::float_cmp)]
    if across.upper == 0.0 {
        return Ok(Interval::point(if up.lower > 0.0 { 0.0 } else { PI }));
    }
    let angle = if up.upper < 0.0 {
        // Below the horizon: measured from straight down.
        Interval::atan2(across, up.negate()).map(|from_down| Interval {
            lower: (PI - from_down.upper).next_down(),
            upper: (PI - from_down.lower).next_up(),
        })
    } else {
        Interval::atan2(across, up)
    }
    .map_err(failed)?;
    Ok(Interval {
        lower: angle.lower.max(0.0),
        upper: angle.upper.min(PI.next_up()),
    })
}

/// One face looking outside, as the grouping reads it.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    normal: FaceNormal,
    /// The unit normal at the box's middle.
    pub(crate) unit: [f64; 3],
    /// How far the true normal may lean from `unit`, in radians.
    spread: f64,
    pub(crate) area: (f64, f64),
    centroid: [f64; 3],
    pub(crate) side: FacadeFaceSide,
}

impl Part {
    pub(crate) fn of(
        normal: &FaceNormal,
        area: (f64, f64),
        centroid: [f64; 3],
        side: FacadeFaceSide,
    ) -> Option<Self> {
        let (lower, upper) = (normal.lower(), normal.upper());
        let middle: [f64; 3] = std::array::from_fn(|axis| f64::midpoint(lower[axis], upper[axis]));
        let half: [f64; 3] = std::array::from_fn(|axis| (upper[axis] - lower[axis]) / 2.0);
        let length = middle.iter().map(|value| value * value).sum::<f64>().sqrt();
        let reach = half.iter().map(|value| value * value).sum::<f64>().sqrt();
        if length.is_nan() || length <= 0.0 {
            return None;
        }
        let spread = if reach < length {
            (reach / (length - reach)).atan()
        } else {
            PI
        };
        Some(Self {
            normal: *normal,
            unit: middle.map(|value| value / length),
            spread,
            area,
            centroid,
            side,
        })
    }
}

/// The angle between two unit vectors, in `[0, π]`.
fn between(a: [f64; 3], b: [f64; 3]) -> f64 {
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let sine = cross.iter().map(|value| value * value).sum::<f64>().sqrt();
    let cosine = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    sine.atan2(cosine)
}

/// The faces of the one direction surely holding the most area, or why
/// there is none.
pub(crate) fn dominant(parts: &[Part]) -> Result<Vec<&Part>, Unavailable> {
    let undecided = |why: &str| (NotEvaluatedReason::IncompleteEvidence, why.to_owned());
    // Directions: each the faces whose normals may be parallel to its
    // first face's.
    let mut directions: Vec<Vec<&Part>> = Vec::new();
    for part in parts {
        let joined = directions.iter_mut().find(|members| {
            let first = members[0];
            between(first.unit, part.unit) <= first.spread + part.spread + PARALLEL
        });
        match joined {
            Some(members) => members.push(part),
            None => directions.push(vec![part]),
        }
    }
    if directions.is_empty() {
        return Err(undecided("no face of the body looks outside"));
    }
    let area = |members: &[&Part]| {
        members
            .iter()
            .fold((0.0_f64, 0.0_f64), |(lower, upper), part| {
                let sure = if part.side == FacadeFaceSide::Outside {
                    part.area.0
                } else {
                    0.0
                };
                (lower + sure, upper + part.area.1)
            })
    };
    let areas: Vec<(f64, f64)> = directions.iter().map(|members| area(members)).collect();
    let largest = (0..directions.len())
        .max_by(|&a, &b| areas[a].0.total_cmp(&areas[b].0))
        .unwrap_or(0);
    let rivals: Vec<usize> = (0..directions.len())
        .filter(|&other| other != largest && areas[other].1 >= areas[largest].0)
        .collect();
    if rivals.is_empty() && areas[largest].0 > 0.0 {
        return Ok(directions.swap_remove(largest));
    }
    let opposite = rivals.iter().any(|&other| {
        between(directions[largest][0].unit, directions[other][0].unit)
            >= PI - directions[largest][0].spread - directions[other][0].spread - PARALLEL
    });
    Err(undecided(if opposite {
        "both sides of the body look outside, so which side faces out is undecided"
    } else {
        "the body looks outside in several directions and none surely holds the most area"
    }))
}

pub(crate) fn facade_unavailable(error: &FacadeAreaError) -> Unavailable {
    let reason = match error {
        FacadeAreaError::Unavailable(_) | FacadeAreaError::UnknownObject(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        FacadeAreaError::InvalidMeasurement | FacadeAreaError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

fn sight_unavailable(error: &SightError) -> Unavailable {
    let reason = match error {
        SightError::Unavailable(_) | SightError::UnknownObject(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        SightError::InvalidRequest(_) | SightError::InvalidEvidence(_) => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

/// The north `call` measures from in `source`, with whether it is stated
/// exactly.
pub(crate) fn north_of(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    source: &SourceId,
) -> Result<([f64; 2], bool), Unavailable> {
    if call.choice("reference") == Some("project_north") {
        return Ok(([0.0, 1.0], true));
    }
    let system = context
        .services
        .get::<CoordinateSystemServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "coordinate-system service is not registered".to_owned(),
            )
        })?
        .coordinate_system(source)
        .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
    let north = system.true_north().ok_or_else(|| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            "the source states no true north".to_owned(),
        )
    })?;
    Ok((north, system.evidence().exact))
}

/// The obstacles `call` names, other than `object`: those surely picked
/// and those the selection leaves undecided.
fn obstacles(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<(Vec<ObjectId>, Vec<ObjectId>), Unavailable> {
    let picked = selection(context, call, "obstacles", Some(object))
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?
        .ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                "no obstacles are named".to_owned(),
            )
        })?;
    let sure: Vec<ObjectId> = picked
        .matched
        .iter()
        .filter(|obstacle| *obstacle != object)
        .cloned()
        .collect();
    let possible: Vec<ObjectId> = picked
        .undecided
        .iter()
        .filter(|obstacle| *obstacle != object)
        .cloned()
        .collect();
    Ok((sure, possible))
}

/// The greatest elevation above the horizon of the call's obstacles in
/// front of `facing`, with the obstacles that may attain it.
fn obstruction(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    object: &ObjectId,
    facing: &Facing,
) -> Result<(Measurement, Citation), Unavailable> {
    let undecided = |why: &str| (NotEvaluatedReason::IncompleteEvidence, why.to_owned());
    if !facing.exact {
        return Err(undecided(
            "the object is a tessellation, so the direction it faces is approximate",
        ));
    }
    let [east, north, _] = facing.toward;
    if east.hypot(north) < 1e-9 {
        return Err(undecided(
            "the face looking outside is level, so it looks along no plan direction",
        ));
    }
    let sector = match call.argument("sector") {
        Some(MeasuredArgument::Number(degrees)) => *degrees,
        _ => 0.0,
    };
    let half = (sector.to_radians() / 2.0).clamp(0.0, std::f64::consts::FRAC_PI_2);
    let (sure, possible) = obstacles(call, context, object)?;
    let service = context
        .services
        .get::<SightServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "line-of-sight service is not registered".to_owned(),
            )
        })?;
    let ask = |obstacles: Vec<ObjectId>| -> Result<SkyObstruction, Unavailable> {
        let request = SkyObstructionRequest::try_new(
            object.clone(),
            facing.eye,
            [east, north],
            half,
            obstacles,
        )
        .map_err(|error| sight_unavailable(&error))?;
        service
            .measure_sky_obstruction(&request)
            .map_err(|error| sight_unavailable(&error))
    };
    let least = ask(sure.clone())?;
    let most = if possible.is_empty() {
        least.clone()
    } else {
        ask(sure.into_iter().chain(possible.iter().cloned()).collect())?
    };
    let (lower, _) = least.angle_radians();
    let (_, upper) = most.angle_radians();
    let exact = possible.is_empty() && least.evidence().exact && most.evidence().exact;
    let locator = format!("{OBSTRUCTION_ANGLE}:{object}:{}", least.evidence().locator);
    let mut related: Vec<ObjectId> = least.highest().to_vec();
    related.extend(most.highest().iter().cloned());
    related.sort();
    related.dedup();
    let mut evidence: Vec<Evidence> = vec![least.evidence().clone()];
    if !possible.is_empty() {
        evidence.push(most.evidence().clone());
    }
    let mut notes = Vec::new();
    if most.angle_radians().1 > least.angle_radians().1 {
        notes.push(format!(
            "objects the selection leaves undecided may rise higher: {}",
            most.highest()
                .iter()
                .filter(|obstacle| possible.contains(obstacle))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok((
        interval(
            (lower, upper.max(lower)),
            Some(QuantityDimension::PlaneAngle),
            exact,
            locator,
        ),
        Citation {
            related,
            evidence,
            notes,
            sources: Vec::new(),
        },
    ))
}

impl MeasuredProvider for FacingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[AZIMUTH, TILT, OBSTRUCTION_ANGLE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let refuse = refused(call.name(), object);
        let incomplete = |why: String| refuse((NotEvaluatedReason::IncompleteEvidence, why));
        let facing = Facing::of(context, object).map_err(&refuse)?;
        let angle = |value: Interval, exact: bool, what: &str| {
            interval(
                (value.lower, value.upper),
                Some(QuantityDimension::PlaneAngle),
                exact,
                format!("{what}:{object}:{}", facing.locator),
            )
        };
        match call.name() {
            AZIMUTH => {
                let (north, stated) = north_of(call, context, &object.source).map_err(&refuse)?;
                let value = facing.azimuth(north).map_err(incomplete)?;
                Ok((
                    angle(value, facing.exact && stated, AZIMUTH),
                    Citation::default(),
                ))
            }
            TILT => {
                let value = facing.tilt().map_err(incomplete)?;
                Ok((angle(value, facing.exact, TILT), Citation::default()))
            }
            _ => obstruction(call, context, object, &facing).map_err(&refuse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(vector: [f64; 3]) -> FaceNormal {
        FaceNormal::exact(vector).unwrap()
    }

    fn part(vector: [f64; 3], area: f64, side: FacadeFaceSide) -> Part {
        Part::of(&normal(vector), (area, area), [0.0; 3], side).unwrap()
    }

    fn holds(interval: Interval, value: f64) -> bool {
        interval.lower <= value && value <= interval.upper
    }

    #[test]
    fn azimuths_turn_clockwise_from_north() {
        let at =
            |vector: [f64; 3], north: [f64; 2]| azimuth(&[normal(vector)], vector, north).unwrap();
        assert!(holds(at([0.0, -1.0, 0.0], [0.0, 1.0]), PI));
        assert!(holds(at([1.0, 0.0, 0.0], [0.0, 1.0]), PI / 2.0));
        assert!(holds(at([-1.0, 0.0, 0.0], [0.0, 1.0]), 1.5 * PI));
        assert!(holds(at([0.0, 1.0, 0.0], [0.0, 1.0]), 0.0));
        // True north along +x: the plan's south is west of it.
        assert!(holds(at([0.0, -1.0, 0.0], [1.0, 0.0]), PI / 2.0));
        // A pitched roof face looking south still faces south.
        assert!(holds(at([0.0, -1.0, 1.0], [0.0, 1.0]), PI));
        let why = azimuth(&[normal([0.0, 0.0, 1.0])], [0.0, 0.0, 1.0], [0.0, 1.0]).unwrap_err();
        assert!(why.contains("level"), "{why}");
    }

    #[test]
    fn tilts_run_from_up_to_down() {
        assert_eq!(
            tilt(&normal([0.0, 0.0, 1.0])).unwrap(),
            Interval::point(0.0)
        );
        assert_eq!(
            tilt(&normal([0.0, 0.0, -2.0])).unwrap(),
            Interval::point(PI)
        );
        assert!(holds(tilt(&normal([0.0, -1.0, 0.0])).unwrap(), PI / 2.0));
        assert!(holds(tilt(&normal([0.0, -1.0, 1.0])).unwrap(), PI / 4.0));
        assert!(holds(tilt(&normal([1.0, 0.0, -1.0])).unwrap(), 0.75 * PI));
    }

    #[test]
    fn the_direction_surely_holding_the_most_area_is_faced() {
        use FacadeFaceSide::{Outside, Partly};
        let south = part([0.0, -1.0, 0.0], 3.0, Outside);
        let top = part([0.0, 0.0, 1.0], 0.6, Outside);
        let parts = [south.clone(), top.clone(), south.clone()];
        let chosen = dominant(&parts).unwrap();
        assert_eq!(chosen.len(), 2);
        // A partly covered face may hold up to its whole area.
        let rival = part([1.0, 0.0, 0.0], 4.0, Partly);
        let (_, why) = dominant(&[south.clone(), rival]).unwrap_err();
        assert!(why.contains("several directions"), "{why}");
        // Both sides looking outside leave the outward side undecided.
        let north = part([0.0, 1.0, 0.0], 3.0, Outside);
        let (_, why) = dominant(&[south.clone(), north, top]).unwrap_err();
        assert!(why.contains("which side faces out"), "{why}");
        let (_, why) = dominant(&[]).unwrap_err();
        assert!(why.contains("no face"), "{why}");
    }
}
