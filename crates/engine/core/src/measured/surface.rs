//! Slopes, falls and tilts: angles measured from the normals of a body's
//! face or from the axes of an object's placement.
//!
//! Every angle is computed with the expression language's sound interval
//! arithmetic over the normal boxes the vertical-extent service certifies,
//! so the interval always holds the exact angle of every piece of the face.
//! A face of several pieces (a warped or tessellated surface) answers the
//! hull over its pieces, never one triangle's value. A piece standing
//! vertical has no gradient, so it leaves the face not evaluated.

use std::f64::consts::{PI, TAU};

use crate::expression::{Interval, IntervalFailure};
use crate::free_space::MetricDirection;
use crate::vertical_extent::FaceNormal;

/// A plan direction in which a gradient is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlanDirection {
    x: Interval,
    y: Interval,
}

impl PlanDirection {
    /// The plan projection of `direction`, normalised; none for a vertical
    /// direction.
    pub(crate) fn of(direction: [f64; 3]) -> Result<Self, String> {
        let [x, y, _] = direction;
        let (x, y) = (Interval::point(x), Interval::point(y));
        let length = square(x)
            .plus(square(y))
            .and_then(Interval::sqrt)
            .map_err(|_| "the direction cannot be measured".to_owned())?;
        if length.lower <= 0.0 {
            return Err("the direction is vertical, so it has no plan direction".into());
        }
        Ok(Self {
            x: x.divided_by(length).map_err(failure)?,
            y: y.divided_by(length).map_err(failure)?,
        })
    }

    /// The plan direction a quarter turn counterclockwise: across this one.
    pub(crate) fn across(self) -> Self {
        Self {
            x: self.y.negate(),
            y: self.x,
        }
    }
}

fn failure(error: IntervalFailure) -> String {
    match error {
        IntervalFailure::ZeroDivisor | IntervalFailure::Domain => {
            "the face stands vertical somewhere, so it has no gradient there".into()
        }
        IntervalFailure::Overflow => "the measurement is not finite".into(),
    }
}

fn square(value: Interval) -> Interval {
    let magnitude = value.abs();
    magnitude.times(magnitude).unwrap_or(Interval {
        lower: 0.0,
        upper: f64::INFINITY,
    })
}

/// A piece's normal, turned to point up: `[x, y, z]` with `z > 0`.
fn upward(normal: &FaceNormal) -> Result<[Interval; 3], String> {
    let (lower, upper) = (normal.lower(), normal.upper());
    let component = |axis: usize| Interval {
        lower: lower[axis],
        upper: upper[axis],
    };
    let [x, y, z] = [component(0), component(1), component(2)];
    if z.lower > 0.0 {
        Ok([x, y, z])
    } else if z.upper < 0.0 {
        Ok([x.negate(), y.negate(), z.negate()])
    } else {
        Err("a piece of the face may stand vertical, so it may have no gradient".into())
    }
}

/// Whether a piece lies exactly level.
#[allow(clippy::float_cmp)]
fn level([x, y, _]: &[Interval; 3]) -> bool {
    x.is_point() && y.is_point() && x.lower == 0.0 && y.lower == 0.0
}

/// The hull of `measure` over every piece.
fn over(
    normals: &[FaceNormal],
    measure: impl Fn(&[Interval; 3]) -> Result<Interval, String>,
) -> Result<Interval, String> {
    let mut hull: Option<Interval> = None;
    for normal in normals {
        let value = measure(&upward(normal)?)?;
        hull = Some(hull.map_or(value, |hull| hull.hull(value)));
    }
    hull.ok_or_else(|| "the face has no pieces".into())
}

/// The steepest gradient of each piece, as an angle from the horizontal in
/// `[0, π/2)`: exactly zero for a level piece.
pub(crate) fn slope(normals: &[FaceNormal]) -> Result<Interval, String> {
    over(normals, |normal| {
        if level(normal) {
            return Ok(Interval::point(0.0));
        }
        let [x, y, z] = *normal;
        let run = square(x).plus(square(y)).and_then(Interval::sqrt);
        let tangent = run.and_then(|run| run.divided_by(z)).map_err(failure)?;
        Ok(clamp(tangent.atan(), 0.0, PI / 2.0))
    })
}

/// The gradient of each piece along `direction`, as a signed angle: rising
/// along it is positive, falling negative.
pub(crate) fn slope_along(
    normals: &[FaceNormal],
    direction: PlanDirection,
) -> Result<Interval, String> {
    over(normals, |normal| {
        if level(normal) {
            return Ok(Interval::point(0.0));
        }
        // On the plane `n·p = c`, moving `u` in plan rises `-(n·u)/n_z`.
        let [x, y, z] = *normal;
        let along = x
            .times(direction.x)
            .and_then(|east| y.times(direction.y).and_then(|north| east.plus(north)))
            .and_then(|dot| dot.negate().divided_by(z))
            .map_err(failure)?;
        Ok(clamp(along.atan(), -PI / 2.0, PI / 2.0))
    })
}

/// The fall across `axis`: the magnitude of the gradient a quarter turn
/// from it, as an angle.
pub(crate) fn cross_fall(normals: &[FaceNormal], axis: PlanDirection) -> Result<Interval, String> {
    slope_along(normals, axis.across()).map(Interval::abs)
}

/// The compass bearing of steepest descent, clockwise from the y axis (plan
/// north) in radians, its midpoint in `[0, 2π)`. A level piece has no
/// direction, and a face descending in directions more than a half turn
/// apart has no one direction.
pub(crate) fn gradient_direction(normals: &[FaceNormal]) -> Result<Interval, String> {
    let mut hull: Option<Interval> = None;
    for normal in normals {
        let normal = upward(normal)?;
        if level(&normal) {
            return Err("a piece of the face is level, so it descends nowhere".into());
        }
        // Down the plane is `(n_x, n_y)` in plan; its bearing from north is
        // `atan2(east, north)`.
        let [x, y, _] = normal;
        let bearing = Interval::atan2(x, y)
            .or_else(|_| {
                Interval::atan2(x.negate(), y.negate()).and_then(|turned| {
                    turned.plus(Interval {
                        lower: PI,
                        upper: PI.next_up(),
                    })
                })
            })
            .map_err(|_| "the face is too close to level to descend one way".to_owned())?;
        hull = Some(match hull {
            None => bearing,
            Some(hull) => hull.hull(nearest(bearing, hull)),
        });
    }
    let hull = hull.ok_or_else(|| "the face has no pieces".to_owned())?;
    if hull.upper - hull.lower > PI {
        return Err("the face descends in directions more than a half turn apart".into());
    }
    let middle = f64::midpoint(hull.lower, hull.upper);
    Ok(if middle < 0.0 {
        shift(hull, TAU)
    } else if middle >= TAU {
        shift(hull, -TAU)
    } else {
        hull
    })
}

/// `bearing`, a whole turn either way if that brings it nearer `to`.
fn nearest(bearing: Interval, to: Interval) -> Interval {
    let centre = |interval: Interval| f64::midpoint(interval.lower, interval.upper);
    let offset = centre(bearing) - centre(to);
    if offset > PI {
        shift(bearing, -TAU)
    } else if offset < -PI {
        shift(bearing, TAU)
    } else {
        bearing
    }
}

/// `interval` moved by `by`, rounded outward.
fn shift(interval: Interval, by: f64) -> Interval {
    Interval {
        lower: (interval.lower + by).next_down(),
        upper: (interval.upper + by).next_up(),
    }
}

/// The tilt of an object axis: `from_vertical` measures the own z axis from
/// the vertical, in `[0, π]`; otherwise the axis is measured from the
/// horizontal, unsigned, in `[0, π/2]`.
pub(crate) fn inclination(axis: MetricDirection, from_vertical: bool) -> Result<Interval, String> {
    let [x, y, z] = axis.components().map(Interval::point);
    let run = square(x)
        .plus(square(y))
        .and_then(Interval::sqrt)
        .map_err(failure)?;
    #[allow(clippy::float_cmp)]
    if from_vertical {
        if run.upper == 0.0 && z.lower > 0.0 {
            return Ok(Interval::point(0.0));
        }
        let angle =
            Interval::atan2(run, z).map_err(|_| "the axis points straight down".to_owned())?;
        Ok(clamp(angle, 0.0, PI))
    } else {
        if z.lower == 0.0 && z.upper == 0.0 {
            return Ok(Interval::point(0.0));
        }
        let angle = Interval::atan2(z.abs(), run).map_err(failure)?;
        Ok(clamp(angle, 0.0, PI / 2.0))
    }
}

/// `interval` within `[lower, upper]`, the range its function can take, so
/// rounding outward never reports an impossible angle.
fn clamp(interval: Interval, lower: f64, upper: f64) -> Interval {
    Interval {
        lower: interval.lower.max(lower).min(upper),
        upper: interval.upper.min(upper).max(lower),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(vector: [f64; 3]) -> FaceNormal {
        FaceNormal::exact(vector).unwrap()
    }

    fn holds(interval: Interval, value: f64) -> bool {
        interval.lower <= value && value <= interval.upper
    }

    #[test]
    fn a_level_face_has_an_exact_zero_slope() {
        let slope = slope(&[exact([0.0, 0.0, 2.0])]).unwrap();
        assert!(slope.is_point());
        assert!(holds(slope, 0.0));
    }

    #[test]
    fn a_plane_rising_one_in_ten_along_x_measures_its_angle() {
        // z = 0.1·x: normal (-0.1, 0, 1).
        let plane = [exact([-0.1, 0.0, 1.0])];
        let angle = 0.1_f64.atan();
        let slope = slope(&plane).unwrap();
        assert!(holds(slope, angle) && slope.upper - slope.lower < 1e-12);
        let east = PlanDirection::of([1.0, 0.0, 0.0]).unwrap();
        assert!(holds(slope_along(&plane, east).unwrap(), angle));
        let west = PlanDirection::of([-1.0, 0.0, 0.0]).unwrap();
        assert!(holds(slope_along(&plane, west).unwrap(), -angle));
        // Across an axis running north the plane falls fully; across one
        // running east it is level.
        let north = PlanDirection::of([0.0, 1.0, 0.0]).unwrap();
        assert!(holds(cross_fall(&plane, north).unwrap(), angle));
        assert!(holds(cross_fall(&plane, east).unwrap(), 0.0));
        // It descends towards the west: a bearing of three quarters of a turn.
        assert!(holds(gradient_direction(&plane).unwrap(), 1.5 * PI));
    }

    #[test]
    fn a_downward_normal_measures_the_same_face() {
        let up = slope(&[exact([-0.1, 0.0, 1.0])]).unwrap();
        let down = slope(&[exact([0.1, 0.0, -1.0])]).unwrap();
        assert_eq!(up, down);
    }

    #[test]
    fn a_warped_face_answers_the_hull_over_its_pieces() {
        let slope = slope(&[exact([0.0, 0.0, 1.0]), exact([-0.2, 0.0, 1.0])]).unwrap();
        assert!(holds(slope, 0.0) && holds(slope, 0.2_f64.atan()));
        assert!(!slope.is_point());
    }

    #[test]
    fn a_vertical_piece_has_no_gradient() {
        let side = FaceNormal::try_new([1.0, 0.0, -0.1], [1.0, 0.0, 0.1]).unwrap();
        assert!(slope(&[side]).is_err());
        assert!(gradient_direction(&[exact([0.0, 0.0, 1.0])]).is_err());
    }

    #[test]
    fn bearings_near_north_and_south_stay_one_interval() {
        // Descending north, slightly either side of it.
        let north =
            gradient_direction(&[exact([0.01, 1.0, 1.0]), exact([-0.01, 1.0, 1.0])]).unwrap();
        assert!(holds(north, 0.01_f64.atan()) && holds(north, -0.01_f64.atan()));
        let south = gradient_direction(&[exact([0.0, -1.0, 1.0])]).unwrap();
        assert!(holds(south, PI));
        let opposed = gradient_direction(&[exact([1.0, 0.0, 1.0]), exact([-1.0, 0.0, 1.0])]);
        assert!(opposed.is_err());
    }

    #[test]
    fn axes_tilt_from_the_vertical_and_the_horizontal() {
        let up = MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap();
        assert_eq!(inclination(up, true).unwrap(), Interval::point(0.0));
        assert!(holds(inclination(up, false).unwrap(), PI / 2.0));
        let east = MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap();
        assert_eq!(inclination(east, false).unwrap(), Interval::point(0.0));
        let tilted = MetricDirection::try_new([0.0, 0.6, 0.8]).unwrap();
        assert!(holds(inclination(tilted, true).unwrap(), 0.75_f64.atan()));
    }
}
