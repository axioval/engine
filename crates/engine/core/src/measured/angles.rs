//! Angles between objects and to reference directions: the angle between
//! two long axes or two faces, the skew of an axis to a reference, and the
//! plan bearing of an axis from project or true north.
//!
//! The long axes are those of each footprint's least-area rectangle, the
//! same axes `parking-bay` and `wall-spacing` judge, so a measured angle and
//! a capability's alignment agree. Every angle is an interval in radians,
//! rounded outward.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::expression::Interval;
use crate::plan_span::PlanRectangle;
use crate::vertical_extent::FaceNormal;

/// Degrees `(lower, upper)` as radians, rounded outward.
pub(crate) fn radians((lower, upper): (f64, f64)) -> Interval {
    Interval {
        lower: lower.to_radians().next_down().max(0.0),
        upper: upper.to_radians().next_up(),
    }
}

/// The acute angle between two long axes, in `[0, π/2]`.
pub(crate) fn between_axes(own: &PlanRectangle, other: &PlanRectangle) -> Result<Interval, String> {
    let angle = radians(own.long_axis_angle(other)?);
    Ok(Interval {
        lower: angle.lower,
        upper: angle.upper.min(FRAC_PI_2.next_up()),
    })
}

/// How far two long axes are from square to one another, in `[0, π/2]`:
/// the skew of a support to the axis it carries.
pub(crate) fn skew(own: &PlanRectangle, other: &PlanRectangle) -> Result<Interval, String> {
    let angle = between_axes(own, other)?;
    Ok(Interval {
        lower: (FRAC_PI_2 - angle.upper).next_down().max(0.0),
        upper: (FRAC_PI_2 - angle.lower).next_up(),
    })
}

fn components(normal: &FaceNormal) -> [Interval; 3] {
    let (lower, upper) = (normal.lower(), normal.upper());
    [0, 1, 2].map(|axis| Interval {
        lower: lower[axis],
        upper: upper[axis],
    })
}

/// At most this many pairs of pieces are compared between two faces.
const MAX_PIECE_PAIRS: usize = 250_000;

/// The angle between two faces' normals, each turned to point up, in
/// `[0, π]`: the hull over every pair of their pieces.
pub(crate) fn between_faces(own: &[FaceNormal], other: &[FaceNormal]) -> Result<Interval, String> {
    if own.len().saturating_mul(other.len()) > MAX_PIECE_PAIRS {
        return Err(format!(
            "the faces have too many pieces to compare: more than {MAX_PIECE_PAIRS} pairs"
        ));
    }
    let up = |normal: &FaceNormal| {
        let [x, y, z] = components(normal);
        if z.lower > 0.0 {
            Ok([x, y, z])
        } else if z.upper < 0.0 {
            Ok([x.negate(), y.negate(), z.negate()])
        } else {
            Err("a piece of a face may stand vertical, so it has no side looking up".to_owned())
        }
    };
    let own: Vec<[Interval; 3]> = own.iter().map(up).collect::<Result<_, _>>()?;
    let other: Vec<[Interval; 3]> = other.iter().map(up).collect::<Result<_, _>>()?;
    let unmeasurable = |_| "the angle between the faces cannot be measured".to_owned();
    let mut hull: Option<Interval> = None;
    for a in &own {
        for b in &other {
            let angle = vector_angle(a, b).map_err(unmeasurable)?;
            hull = Some(hull.map_or(angle, |hull| hull.hull(angle)));
        }
    }
    hull.ok_or_else(|| "a face has no pieces".into())
}

/// The angle between two vectors, `atan2(|a × b|, a · b)`, in `[0, π]`.
fn vector_angle(
    a: &[Interval; 3],
    b: &[Interval; 3],
) -> Result<Interval, crate::expression::IntervalFailure> {
    let product = |i: usize, j: usize| a[i].times(b[j]);
    let cross = [
        product(1, 2)?.minus(product(2, 1)?)?,
        product(2, 0)?.minus(product(0, 2)?)?,
        product(0, 1)?.minus(product(1, 0)?)?,
    ];
    let square = |value: Interval| {
        let magnitude = value.abs();
        magnitude.times(magnitude)
    };
    let sine = square(cross[0])?
        .plus(square(cross[1])?)?
        .plus(square(cross[2])?)?
        .sqrt()?;
    let cosine = product(0, 0)?.plus(product(1, 1)?)?.plus(product(2, 2)?)?;
    // Parallel normals: the angle is exactly zero only when the sine is.
    #[allow(clippy::float_cmp)]
    if sine.upper == 0.0 && cosine.lower > 0.0 {
        return Ok(Interval::point(0.0));
    }
    let angle = if sine.lower > 0.0 || cosine.lower > 0.0 {
        Interval::atan2(sine, cosine)?
    } else {
        // Near the cut at π: measure from the opposite direction.
        let turned = Interval::atan2(sine, cosine.negate())?;
        Interval {
            lower: (PI - turned.upper).next_down(),
            upper: (PI - turned.lower).next_up(),
        }
    };
    Ok(Interval {
        lower: angle.lower.max(0.0),
        upper: angle.upper.min(PI.next_up()),
    })
}

/// The compass bearing of the plan direction `direction` clockwise from
/// `north`, both plan vectors; its midpoint in `[0, 2π)`, or in `[0, π)`
/// for an undirected axis.
pub(crate) fn bearing(
    direction: [f64; 2],
    north: [f64; 2],
    undirected: bool,
) -> Result<Interval, String> {
    let [x, y] = direction.map(Interval::point);
    let [north_x, north_y] = north.map(Interval::point);
    let failed = |_| "the bearing cannot be measured".to_owned();
    // Clockwise from north: the sine is `direction × north`, the cosine
    // their dot product.
    let sine = x
        .times(north_y)
        .and_then(|left| y.times(north_x).and_then(|right| left.minus(right)))
        .map_err(failed)?;
    let cosine = x
        .times(north_x)
        .and_then(|left| y.times(north_y).and_then(|right| left.plus(right)))
        .map_err(failed)?;
    #[allow(clippy::float_cmp)]
    if sine.lower == 0.0 && sine.upper == 0.0 && cosine.lower > 0.0 {
        return Ok(Interval::point(0.0));
    }
    let angle = Interval::atan2(sine, cosine)
        .or_else(|_| {
            Interval::atan2(sine.negate(), cosine.negate()).map(|turned| Interval {
                lower: (turned.lower + PI).next_down(),
                upper: (turned.upper + PI).next_up(),
            })
        })
        .map_err(|_| "the direction has no plan bearing".to_owned())?;
    let period = if undirected { PI } else { TAU };
    let middle = f64::midpoint(angle.lower, angle.upper);
    let turns = (middle / period).floor();
    if turns == 0.0 {
        return Ok(angle);
    }
    let shift = turns * period;
    Ok(Interval {
        lower: (angle.lower - shift).next_down(),
        upper: (angle.upper - shift).next_up(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holds(interval: Interval, value: f64) -> bool {
        interval.lower <= value && value <= interval.upper
    }

    #[test]
    fn bearings_turn_clockwise_from_north() {
        let north = [0.0, 1.0];
        assert_eq!(
            bearing([0.0, 2.0], north, false).unwrap(),
            Interval::point(0.0)
        );
        assert!(holds(bearing([1.0, 0.0], north, false).unwrap(), FRAC_PI_2));
        assert!(holds(bearing([0.0, -1.0], north, false).unwrap(), PI));
        assert!(holds(bearing([-1.0, 0.0], north, false).unwrap(), 1.5 * PI));
        // An undirected axis pointing west is the same axis as east.
        assert!(holds(bearing([-1.0, 0.0], north, true).unwrap(), FRAC_PI_2));
        // True north turned a quarter clockwise: east is its north.
        assert!(holds(bearing([1.0, 0.0], [1.0, 0.0], false).unwrap(), 0.0));
    }

    #[test]
    fn faces_meet_at_the_angle_of_their_normals() {
        let level = [FaceNormal::exact([0.0, 0.0, 1.0]).unwrap()];
        let pitched = [FaceNormal::exact([-1.0, 0.0, 1.0]).unwrap()];
        assert_eq!(between_faces(&level, &level).unwrap(), Interval::point(0.0));
        assert!(holds(
            between_faces(&level, &pitched).unwrap(),
            std::f64::consts::FRAC_PI_4
        ));
        // A downward normal is turned up first.
        let under = [FaceNormal::exact([1.0, 0.0, -1.0]).unwrap()];
        assert!(holds(
            between_faces(&level, &under).unwrap(),
            std::f64::consts::FRAC_PI_4
        ));
    }
}
