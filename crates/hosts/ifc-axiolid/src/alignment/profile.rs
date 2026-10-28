//! Profiles of an alignment's parameters along a station range
//! (`AlignmentService::measure_alignment_profile`).
//!
//! Each parameter is read as the laws it is written in, laid end to end
//! along plan distance (**leaves**), and every leaf states its value at a
//! point and a bound on its rate over a stretch, both proven from the law:
//!
//! - the plan curvature of a line, constant or polynomial curvature
//!   (piecewise or chained): the polynomial and its derivative, by interval
//!   Horner evaluation;
//! - the gradient of a polynomial elevation (a constant grade, a parabolic
//!   vertical curve): its derivative and second derivative; of a vertical
//!   circular arc: `grade = tan t` with `sin t = sin t0 + d / R`, so the
//!   rate is `(1 + grade^2)^(3/2) / R`, monotone in `|grade|`, which the
//!   grades at the stretch's ends bound;
//! - the cant `D = left - right` of an `IfcAlignmentCant` segment: the five
//!   transition shapes `D1 + f(xi) (D2 - D1)` whose `f'` rises to its peak
//!   at mid-segment and falls back (linear `1`, Bloss `6 xi (1 - xi)`,
//!   cosine `pi/2 sin(pi xi)`, sine `1 - cos(2 pi xi)`, Helmert `4 xi` then
//!   `4 (1 - xi)`), and the Viennese bend `b sin(psi)`, `psi` blended by
//!   `xi^4 (35 - 84 xi + 70 xi^2 - 20 xi^3)` whose derivative
//!   `140 xi^3 (1 - xi)^3` peaks at mid-segment too. The unsigned cant is
//!   its magnitude, split where it crosses zero, and the cant ratio that
//!   over the rail head distance;
//! - the vertical curvature `k = z'' / (1 + z'^2)^(3/2)` of a polynomial
//!   elevation, its rate `z''' F - 3 z' z''^2 G` with
//!   `F = (1 + z'^2)^(-3/2)` and `G = (1 + z'^2)^(-5/2)` (both monotone in
//!   `|z'|`), by interval arithmetic over the stretch, so a constant
//!   gradient is exactly zero and a parabola's is bounded by its grades; of
//!   a vertical circular arc, the constant `1 / R`.
//!
//! A law whose rate has no such bound (a sinusoidal spiral, a parametric
//! plan piece, a vertical clothoid) is refused by name, never sampled.
//! Leaves are split at station equations, so a piece's stations are its
//! start station plus the distance into it, and every `step` where one is
//! stated. Values the law states at a seam (a segment's authored cant)
//! are kept exact; values computed in floating point are widened outward.

use std::f64::consts::PI;

use axiolid_curve::{ChainPiece2, CurvatureLaw, Curve2, ElevationLaw};
use axiolid_evaluate::elevation_grade;
use axioval_engine::{
    AlignmentError, AlignmentInterval, AlignmentParameter, AlignmentProfile,
    AlignmentProfileRequest, ProfilePiece,
};
use ifc_alignment::{CantLayout, CantSegment, CantSegmentType};

use super::{Centreline, IfcAlignmentService, RESOLUTION, RIGID, evidence, polynomial, widen};

/// Pieces one profile may hold before it is refused.
const PIECES: usize = 100_000;

/// An interval `(lower, upper)`.
type Bounds = (f64, f64);

/// A law over the plan distances `[start, end]`.
struct Leaf<'l> {
    start: f64,
    end: f64,
    law: Law<'l>,
}

enum Law<'l> {
    /// `sign * p(d - origin)`, ascending powers.
    Polynomial {
        coefficients: Vec<f64>,
        origin: f64,
        sign: f64,
    },
    /// The grade of a vertical circular arc starting at `origin`.
    Arc { law: &'l ElevationLaw, origin: f64 },
    /// The signed cant `left - right` of a segment.
    Cant { segment: &'l CantSegment, rail: f64 },
    /// The vertical curvature of the elevation `p(d - origin)`, ascending
    /// powers.
    Vertical { elevation: Vec<f64>, origin: f64 },
    /// A value constant along the leaf (a vertical arc's curvature).
    Fixed(Bounds),
}

/// `a - b` as an interval: a point where the subtraction is exact.
fn difference(a: f64, b: f64) -> (f64, f64) {
    // TwoSum: the rounding error of `a + (-b)`.
    let (x, y) = (a, -b);
    let sum = x + y;
    let virtual_y = sum - x;
    let error = (x - (sum - virtual_y)) + (y - virtual_y);
    if error == 0.0 {
        (sum, sum)
    } else {
        widen((sum, sum))
    }
}

fn hull(values: &[f64]) -> (f64, f64) {
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (low, high)
}

/// `[low, high]` times `factor`, outward.
fn scaled((low, high): (f64, f64), factor: f64) -> (f64, f64) {
    let (a, b) = (low * factor, high * factor);
    widen((a.min(b), a.max(b)))
}

/// The range over `[a, b]` of a slope shape `f'` on `[0, 1]` that rises
/// to its peak at `1/2` and falls back symmetrically.
fn unimodal(shape: impl Fn(f64) -> f64, (a, b): (f64, f64)) -> (f64, f64) {
    let (at_a, at_b) = (shape(a), shape(b));
    let high = if a <= 0.5 && b >= 0.5 {
        shape(0.5)
    } else {
        at_a.max(at_b)
    };
    (at_a.min(at_b).max(0.0), high)
}

/// The blend of a Viennese bend's angle and its derivative.
fn viennese(xi: f64) -> f64 {
    xi.powi(4) * (35.0 - 84.0 * xi + 70.0 * xi * xi - 20.0 * xi.powi(3))
}

fn viennese_slope(xi: f64) -> f64 {
    140.0 * xi.powi(3) * (1.0 - xi).powi(3)
}

impl Law<'_> {
    /// Every value at the plan distance `at`.
    fn value(&self, at: f64) -> Result<(f64, f64), String> {
        match self {
            Law::Polynomial {
                coefficients,
                origin,
                sign,
            } => {
                let local = at - origin;
                let value = if local == 0.0 {
                    let first = coefficients.first().copied().unwrap_or(0.0);
                    (first, first)
                } else {
                    polynomial(coefficients, (local, local))
                };
                Ok(signed(*sign, value))
            }
            Law::Arc { law, origin } => {
                let grade = elevation_grade(law, at - origin).map_err(|error| {
                    format!("the vertical arc has no grade at {at} m ({error})")
                })?;
                Ok(widen((grade, grade)))
            }
            Law::Cant { segment, rail } => cant(segment, *rail, at),
            Law::Vertical { elevation, origin } => {
                let local = (at - origin, at - origin);
                let grade = polynomial(&derivative(elevation), local);
                let bend = polynomial(&derivative(&derivative(elevation)), local);
                Ok(times(bend, steepness(grade, 1.5)))
            }
            Law::Fixed(value) => Ok(*value),
        }
    }

    /// Every value over `[a, b]`: bounded from either end by the rate over
    /// the stretch (the mean value theorem), exact where the law is
    /// constant and exact.
    fn over(&self, (a, b): (f64, f64)) -> Result<(f64, f64), String> {
        let (start, end) = (self.value(a)?, self.value(b)?);
        let (slow, fast) = self.rate((a, b))?;
        if zero((slow, fast)) && start == end {
            return Ok(start);
        }
        let width = b - a;
        let rise = (fast.max(0.0) * width).next_up();
        let fall = (slow.min(0.0) * width).next_down();
        let low = (start.0 + fall).next_down().max((end.0 - rise).next_down());
        let high = (start.1 + rise).next_up().min((end.1 - fall).next_up());
        Ok((low.min(start.0).min(end.0), high.max(start.1).max(end.1)))
    }

    /// Every rate along plan distance over `[a, b]`.
    fn rate(&self, (a, b): (f64, f64)) -> Result<(f64, f64), String> {
        match self {
            Law::Polynomial {
                coefficients,
                origin,
                sign,
            } => Ok(signed(
                *sign,
                polynomial(&derivative(coefficients), (a - origin, b - origin)),
            )),
            Law::Arc { law, origin } => {
                let ElevationLaw::CircularArc { radius, .. } = law else {
                    unreachable!("an arc leaf holds an arc");
                };
                let (start, end) = (self.value(a)?, self.value(b)?);
                let (low, high) = hull(&[start.0, start.1, end.0, end.1]);
                // The grade is monotone along the arc: its square is least at
                // zero where it crosses it, else at the nearer end.
                let least = if low <= 0.0 && high >= 0.0 {
                    0.0
                } else {
                    low.abs().min(high.abs())
                };
                let most = low.abs().max(high.abs());
                let rate = |grade: f64| (1.0 + grade * grade).powf(1.5) / radius.abs();
                let _ = origin;
                Ok(signed(radius.signum(), widen((rate(least), rate(most)))))
            }
            Law::Cant { segment, rail } => cant_rate(segment, *rail, (a, b)),
            Law::Vertical { elevation, origin } => {
                let local = (a - origin, b - origin);
                let grade = polynomial(&derivative(elevation), local);
                let second = derivative(&derivative(elevation));
                let bend = polynomial(&second, local);
                let third = polynomial(&derivative(&second), local);
                // k' = z''' F - 3 z' z''^2 G.
                let first = times(third, steepness(grade, 1.5));
                let squared = times(bend, bend);
                let other = times(
                    times(grade, squared),
                    times((3.0, 3.0), steepness(grade, 2.5)),
                );
                Ok(minus(first, other))
            }
            Law::Fixed(_) => Ok((0.0, 0.0)),
        }
    }
}

/// Whether an interval is exactly zero.
fn zero((low, high): Bounds) -> bool {
    low == 0.0 && high == 0.0
}

/// `a * b` for intervals, outward; exactly zero where either factor is.
fn times(a: Bounds, b: Bounds) -> Bounds {
    if zero(a) || zero(b) {
        return (0.0, 0.0);
    }
    let corners = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
    widen(hull(&corners))
}

/// `a - b` for intervals, outward; `a` itself where `b` is exactly zero.
fn minus(a: Bounds, b: Bounds) -> Bounds {
    if zero(b) {
        return a;
    }
    if zero(a) {
        return (-b.1, -b.0);
    }
    widen((a.0 - b.1, a.1 - b.0))
}

/// `(1 + g^2)^(-power)` for every grade `g` in `grade`: falling in `|g|`,
/// so the least and greatest `|g|` bound it.
fn steepness(grade: Bounds, power: f64) -> Bounds {
    let least = if grade.0 <= 0.0 && grade.1 >= 0.0 {
        0.0
    } else {
        grade.0.abs().min(grade.1.abs())
    };
    let most = grade.0.abs().max(grade.1.abs());
    let at = |g: f64| (1.0 + g * g).powf(-power);
    widen((at(most), at(least)))
}

fn signed(sign: f64, (low, high): (f64, f64)) -> (f64, f64) {
    if sign < 0.0 {
        (-high, -low)
    } else {
        (low, high)
    }
}

fn derivative(coefficients: &[f64]) -> Vec<f64> {
    coefficients
        .iter()
        .enumerate()
        .skip(1)
        .map(|(power, coefficient)| {
            #[allow(clippy::cast_precision_loss)]
            let power = power as f64;
            coefficient * power
        })
        .collect()
}

/// Where `at` lies along `segment`, `0` to `1`.
fn xi(segment: &CantSegment, at: f64) -> f64 {
    let start = segment.start_dist_along;
    if at <= start {
        0.0
    } else if at >= start + segment.horizontal_length {
        1.0
    } else {
        ((at - start) / segment.horizontal_length).clamp(0.0, 1.0)
    }
}

/// The authored cants `left - right` at a segment's start and end.
fn ends(segment: &CantSegment) -> Result<(Bounds, Bounds), String> {
    let start = difference(segment.start_cant_left, segment.start_cant_right);
    let end = match (segment.end_cant_left, segment.end_cant_right) {
        (Some(left), Some(right)) => difference(left, right),
        (None, None) => start,
        _ => {
            return Err(format!(
                "cant segment {} states one end cant without the other",
                segment.entity
            ));
        }
    };
    Ok((start, end))
}

/// Every signed cant of `segment` at `at`.
fn cant(segment: &CantSegment, rail: f64, at: f64) -> Result<(f64, f64), String> {
    let (start, end) = ends(segment)?;
    let xi = xi(segment, at);
    // The ends are authored: `xi` is exactly 0 or 1 there (`xi`).
    #[allow(clippy::float_cmp)]
    let (at_start, at_end) = (xi == 0.0, xi == 1.0);
    if at_start || matches!(segment.predefined_type, CantSegmentType::ConstantCant) {
        return Ok(start);
    }
    if at_end {
        return Ok(end);
    }
    if matches!(segment.predefined_type, CantSegmentType::VienneseBend) {
        let (psi_start, psi_end) = ((start.0 / rail).asin(), (end.0 / rail).asin());
        let psi = psi_start + (psi_end - psi_start) * viennese(xi);
        let value = rail * psi.sin();
        return Ok(widen((value, value)));
    }
    let rails = ifc_alignment::cant_at(segment, xi, Some(rail))
        .map_err(|error| format!("cant segment {} cannot be read ({error})", segment.entity))?;
    let value = rails.left - rails.right;
    Ok(widen((value, value)))
}

/// Every rate of the signed cant of `segment` over `[a, b]`.
fn cant_rate(segment: &CantSegment, rail: f64, (a, b): (f64, f64)) -> Result<(f64, f64), String> {
    let (start, end) = ends(segment)?;
    let length = segment.horizontal_length;
    let span = (xi(segment, a), xi(segment, b));
    let change = (end.0 - start.1, end.1 - start.0);
    let per_metre = |(low, high): (f64, f64), (slow, fast): (f64, f64)| {
        // `change / length` times a non-negative shape slope.
        let corners = [
            low * slow / length,
            low * fast / length,
            high * slow / length,
            high * fast / length,
        ];
        widen(hull(&corners))
    };
    Ok(match &segment.predefined_type {
        CantSegmentType::ConstantCant => (0.0, 0.0),
        CantSegmentType::LinearTransition => per_metre(change, (1.0, 1.0)),
        CantSegmentType::BlossCurve => per_metre(change, unimodal(|x| 6.0 * x * (1.0 - x), span)),
        CantSegmentType::CosineCurve => {
            per_metre(change, unimodal(|x| 0.5 * PI * (PI * x).sin(), span))
        }
        CantSegmentType::SineCurve => {
            per_metre(change, unimodal(|x| 1.0 - (2.0 * PI * x).cos(), span))
        }
        CantSegmentType::HelmertCurve => per_metre(
            change,
            unimodal(|x| if x <= 0.5 { 4.0 * x } else { 4.0 * (1.0 - x) }, span),
        ),
        CantSegmentType::VienneseBend => {
            if !(rail.is_finite() && rail > 0.0) || start.0.abs() > rail || end.0.abs() > rail {
                return Err(format!(
                    "Viennese bend {} has no cant within its rail head distance",
                    segment.entity
                ));
            }
            // dD/dd = b cos(psi) (psi2 - psi1) blend'(xi) / L, with psi
            // monotone between its ends, inside (-pi/2, pi/2).
            let (psi_start, psi_end) = ((start.0 / rail).asin(), (end.0 / rail).asin());
            let turn = psi_end - psi_start;
            let psi = |xi: f64| psi_start + turn * viennese(xi);
            let (psi_a, psi_b) = (psi(span.0), psi(span.1));
            let cos_low = psi_a.cos().min(psi_b.cos()).max(0.0);
            let cos_high = if psi_a.min(psi_b) <= 0.0 && psi_a.max(psi_b) >= 0.0 {
                1.0
            } else {
                psi_a.cos().max(psi_b.cos())
            };
            let (slow, fast) = unimodal(viennese_slope, span);
            per_metre(
                scaled((turn, turn), rail),
                (slow * cos_low, fast * cos_high),
            )
        }
        other => {
            return Err(format!(
                "cant segment {} follows a law ({other:?}) whose rate has no certified bound \
                 here",
                segment.entity
            ));
        }
    })
}

/// The leaves of a curvature law over `[start, start + length]`.
fn curvature_leaves<'l>(
    law: &'l CurvatureLaw,
    start: f64,
    length: f64,
    sign: f64,
    out: &mut Vec<Leaf<'l>>,
) -> Result<(), String> {
    let end = start + length;
    match law {
        CurvatureLaw::Constant { curvature } => out.push(Leaf {
            start,
            end,
            law: Law::Polynomial {
                coefficients: vec![*curvature],
                origin: start,
                sign,
            },
        }),
        CurvatureLaw::Polynomial { coefficients } => out.push(Leaf {
            start,
            end,
            law: Law::Polynomial {
                coefficients: coefficients.clone(),
                origin: start,
                sign,
            },
        }),
        CurvatureLaw::Piecewise { breaks, laws } => {
            if laws.len() != breaks.len() + 1 {
                return Err("a piecewise law whose pieces and seams do not match".into());
            }
            for (index, law) in laws.iter().enumerate() {
                let from = if index == 0 { 0.0 } else { breaks[index - 1] };
                let to = breaks.get(index).copied().unwrap_or(length);
                curvature_leaves(law, start + from, to - from, sign, out)?;
            }
        }
        _ => {
            return Err(
                "a transition whose curvature law is no polynomial (a sinusoidal spiral) has \
                 no certified rate bound here"
                    .into(),
            );
        }
    }
    Ok(())
}

/// The orientation of a plan's start frame: `1` turning left is positive.
fn orientation(frame: &axiolid_core::Frame2) -> Result<f64, String> {
    let unit = |v: axiolid_core::Vec2| (v.length() - 1.0).abs() <= RIGID;
    if !(unit(frame.x) && unit(frame.y) && frame.x.dot(frame.y).abs() <= RIGID) {
        return Err("the plan's start frame is not orthonormal".to_owned());
    }
    Ok(frame.x.perp_dot(frame.y).signum())
}

/// The leaves of the plan curvature of `plan` over `[0, length]`.
fn plan_leaves(plan: &Curve2, length: f64) -> Result<Vec<Leaf<'_>>, String> {
    let mut out = Vec::new();
    match plan {
        Curve2::Line(_) => out.push(Leaf {
            start: 0.0,
            end: length,
            law: Law::Polynomial {
                coefficients: vec![0.0],
                origin: 0.0,
                sign: 1.0,
            },
        }),
        Curve2::Intrinsic(intrinsic) => {
            let sign = orientation(&intrinsic.start)?;
            curvature_leaves(&intrinsic.curvature, 0.0, intrinsic.length, sign, &mut out)?;
        }
        Curve2::Chain(chain) => {
            let sign = orientation(&chain.start)?;
            let mut start = 0.0;
            for piece in &chain.pieces {
                let ChainPiece2::Intrinsic { curvature, length } = piece else {
                    return Err(
                        "a plan piece given as a parametric curve (a cubic parabola) has no \
                         certified curvature rate here"
                            .into(),
                    );
                };
                curvature_leaves(curvature, start, *length, sign, &mut out)?;
                start += length;
            }
        }
        _ => return Err("the plan is no curve whose curvature law is stated".into()),
    }
    Ok(out)
}

/// The leaves of the gradient of `law` over `[start, start + length]`.
fn gradient_leaves<'l>(
    law: &'l ElevationLaw,
    start: f64,
    length: f64,
    out: &mut Vec<Leaf<'l>>,
) -> Result<(), String> {
    match law {
        ElevationLaw::Polynomial { coefficients } => out.push(Leaf {
            start,
            end: start + length,
            law: Law::Polynomial {
                coefficients: derivative(coefficients),
                origin: start,
                sign: 1.0,
            },
        }),
        ElevationLaw::Piecewise { breaks, laws } => {
            if laws.len() != breaks.len() + 1 {
                return Err("a piecewise law whose pieces and seams do not match".into());
            }
            for (index, law) in laws.iter().enumerate() {
                let from = if index == 0 { 0.0 } else { breaks[index - 1] };
                let to = breaks.get(index).copied().unwrap_or(length);
                gradient_leaves(law, start + from, to - from, out)?;
            }
        }
        ElevationLaw::CircularArc { .. } => out.push(Leaf {
            start,
            end: start + length,
            law: Law::Arc { law, origin: start },
        }),
        _ => {
            return Err(
                "a vertical segment whose law is no polynomial or circular arc has no \
                 certified gradient rate here"
                    .into(),
            );
        }
    }
    Ok(())
}

/// The leaves of the vertical curvature of `law` over
/// `[start, start + length]`.
fn vertical_leaves<'l>(
    law: &'l ElevationLaw,
    start: f64,
    length: f64,
    out: &mut Vec<Leaf<'l>>,
) -> Result<(), String> {
    match law {
        ElevationLaw::Polynomial { coefficients } => out.push(Leaf {
            start,
            end: start + length,
            law: Law::Vertical {
                elevation: coefficients.clone(),
                origin: start,
            },
        }),
        ElevationLaw::Piecewise { breaks, laws } => {
            if laws.len() != breaks.len() + 1 {
                return Err("a piecewise law whose pieces and seams do not match".into());
            }
            for (index, law) in laws.iter().enumerate() {
                let from = if index == 0 { 0.0 } else { breaks[index - 1] };
                let to = breaks.get(index).copied().unwrap_or(length);
                vertical_leaves(law, start + from, to - from, out)?;
            }
        }
        // A circle in the (distance, height) plane: `1 / R` throughout,
        // positive in a sag.
        ElevationLaw::CircularArc { radius, .. } => {
            let curvature = 1.0 / radius;
            out.push(Leaf {
                start,
                end: start + length,
                law: Law::Fixed(widen((curvature, curvature))),
            });
        }
        _ => {
            return Err(
                "a vertical segment whose law is no polynomial or circular arc (a vertical \
                 clothoid) has no certified curvature bound here"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Every vertical curvature of `law`, laid along `[0, length]`, over the
/// plan distances `stretch`.
pub(super) fn vertical_curvature(
    law: &ElevationLaw,
    length: f64,
    (a, b): (f64, f64),
) -> Result<(f64, f64), String> {
    let mut leaves = Vec::new();
    vertical_leaves(law, 0.0, length, &mut leaves)?;
    let mut out: Option<Bounds> = None;
    for leaf in &leaves {
        let (first, last) = (a.max(leaf.start), b.min(leaf.end));
        if first <= last {
            let value = leaf.law.over((first, last))?;
            out = Some(out.map_or(value, |seen| (seen.0.min(value.0), seen.1.max(value.1))));
        }
    }
    out.ok_or_else(|| format!("no vertical segment states [{a}, {b}] m along it"))
}

/// The leaves of the signed cant of `layout`.
fn cant_leaves(layout: &CantLayout) -> Vec<Leaf<'_>> {
    layout
        .segments()
        .iter()
        .filter(|segment| segment.horizontal_length > 0.0)
        .map(|segment| Leaf {
            start: segment.start_dist_along,
            end: segment.start_dist_along + segment.horizontal_length,
            law: Law::Cant {
                segment,
                rail: layout.rail_head_distance,
            },
        })
        .collect()
}

fn interval((low, high): (f64, f64)) -> Result<AlignmentInterval, AlignmentError> {
    AlignmentInterval::try_new(low, high)
}

impl Centreline {
    /// The station labelling the plan distance `at`.
    fn station_at(&self, at: f64) -> Result<f64, AlignmentError> {
        match self
            .stationing
            .as_ref()
            .map_err(|reason| AlignmentError::Unavailable(reason.clone()))?
        {
            None => Ok(at),
            Some(stationing) => stationing.station_at(at).map_err(|error| {
                AlignmentError::Unavailable(format!("no station labels {at} m along it ({error})"))
            }),
        }
    }

    /// Where a piece must end: the station equations' distances.
    fn equations(&self) -> Vec<f64> {
        match &self.stationing {
            Ok(Some(stationing)) => stationing
                .equations()
                .iter()
                .map(|equation| equation.distance_along)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The pieces of `leaves` over `[from, to]`, split at the station
    /// equations and every `step`.
    fn pieces(
        &self,
        leaves: &[Leaf<'_>],
        (from, to): (f64, f64),
        step: Option<f64>,
    ) -> Result<Vec<ProfilePiece>, AlignmentError> {
        let unavailable = AlignmentError::Unavailable;
        #[allow(clippy::float_cmp)]
        let point = from == to;
        let mut cuts: Vec<(usize, f64, f64)> = Vec::new();
        let mut covered = from;
        for (index, leaf) in leaves.iter().enumerate() {
            let (low, high) = (leaf.start.max(from), leaf.end.min(to));
            let overlaps = if point {
                leaf.start <= from && from <= leaf.end
            } else {
                low < high
            };
            if !overlaps {
                continue;
            }
            if low > covered + RESOLUTION {
                return Err(unavailable(format!(
                    "nothing of {} states it at {covered} m along it",
                    self.locator
                )));
            }
            // Seams written as sums may differ by rounding: snap to the
            // previous end so the pieces meet.
            let low = if point { from } else { covered.max(from) };
            let mut points = vec![low];
            let equations = self.equations();
            points.extend(equations.into_iter().filter(|at| *at > low && *at < high));
            points.push(if point { from } else { high });
            if let Some(step) = step {
                let mut stepped = vec![points[0]];
                for pair in points.windows(2) {
                    let width = pair[1] - pair[0];
                    let count = (width / step).ceil().max(1.0);
                    if count > 1.0e5 {
                        return Err(unavailable(format!(
                            "a step of {step} m samples more than {PIECES} pieces"
                        )));
                    }
                    let mut k = 1.0;
                    while k < count {
                        stepped.push(pair[0] + width * k / count);
                        k += 1.0;
                    }
                    stepped.push(pair[1]);
                }
                points = stepped;
            }
            for pair in points.windows(2) {
                cuts.push((index, pair[0], pair[1]));
            }
            if cuts.len() > PIECES {
                return Err(unavailable(format!(
                    "the range holds more than {PIECES} pieces"
                )));
            }
            covered = high;
        }
        if cuts.is_empty() || (!point && covered + RESOLUTION < to) {
            return Err(unavailable(format!(
                "nothing of {} states it at {covered} m along it",
                self.locator
            )));
        }
        let mut pieces = Vec::with_capacity(cuts.len());
        for (index, a, b) in cuts {
            let law = &leaves[index].law;
            pieces.push(ProfilePiece::try_new(
                interval((a, b))?,
                self.station_at(a)?,
                interval(law.value(a).map_err(unavailable)?)?,
                interval(law.value(b).map_err(unavailable)?)?,
                interval(law.rate((a, b)).map_err(unavailable)?)?,
            )?);
        }
        Ok(pieces)
    }
}

impl IfcAlignmentService {
    /// A parameter's profile over a station range (module documentation).
    pub(super) fn profile(
        &self,
        request: &AlignmentProfileRequest,
    ) -> Result<AlignmentProfile, AlignmentError> {
        let alignment = request.alignment();
        let centreline = self.centreline(alignment)?;
        let from = request
            .from()
            .map(|station| centreline.distance_of(station))
            .transpose()?
            .unwrap_or(0.0);
        let to = request
            .to()
            .map(|station| centreline.distance_of(station))
            .transpose()?
            .unwrap_or(centreline.length);
        if from > to {
            return Err(AlignmentError::Unavailable(format!(
                "station {} lies beyond station {} along {alignment}",
                request.from().unwrap_or(0.0),
                request.to().unwrap_or(0.0)
            )));
        }
        let elevated = centreline.elevated();
        let unavailable = AlignmentError::Unavailable;
        let (leaves, what, rail) = match request.parameter() {
            AlignmentParameter::Curvature => (
                plan_leaves(&elevated.plan, centreline.length).map_err(unavailable)?,
                "plan curvature",
                None,
            ),
            AlignmentParameter::Gradient => {
                let mut leaves = Vec::new();
                gradient_leaves(&elevated.elevation, 0.0, centreline.length, &mut leaves)
                    .map_err(unavailable)?;
                (leaves, "gradient", None)
            }
            AlignmentParameter::VerticalCurvature => {
                let mut leaves = Vec::new();
                vertical_leaves(&elevated.elevation, 0.0, centreline.length, &mut leaves)
                    .map_err(unavailable)?;
                (leaves, "vertical curvature", None)
            }
            parameter @ (AlignmentParameter::Cant | AlignmentParameter::CantRatio) => {
                let layout = centreline
                    .cant
                    .as_ref()
                    .map_err(|reason| unavailable(reason.clone()))?;
                let Some(layout) = layout else {
                    let locator = format!("{} states no cant layout", centreline.locator);
                    return AlignmentProfile::try_new(
                        request.clone(),
                        None,
                        evidence(&alignment.source, locator, true),
                    );
                };
                let ratio = parameter == AlignmentParameter::CantRatio;
                (
                    cant_leaves(layout),
                    if ratio { "cant ratio" } else { "cant" },
                    Some((layout.rail_head_distance, ratio)),
                )
            }
        };
        let mut pieces = centreline.pieces(&leaves, (from, to), request.step())?;
        if let Some((rail, ratio)) = rail {
            if ratio && !(rail.is_finite() && rail > 0.0) {
                return Err(unavailable(format!(
                    "the cant layout of {alignment} states no positive rail head distance"
                )));
            }
            let mut unsigned = Vec::with_capacity(pieces.len());
            for piece in pieces {
                let piece = if ratio {
                    piece.scaled(1.0 / rail)?
                } else {
                    piece
                };
                unsigned.extend(piece.magnitude()?);
            }
            pieces = unsigned;
        }
        let exact = pieces.iter().all(|piece| {
            piece.start().is_exact() && piece.end().is_exact() && piece.rate().is_exact()
        });
        let locator = format!(
            "{what} of {} from {} m to {} m along it, {} pieces",
            centreline.locator,
            from,
            to,
            pieces.len()
        );
        AlignmentProfile::try_new(
            request.clone(),
            Some(pieces),
            evidence(&alignment.source, locator, exact),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_engine::{AlignmentServiceHandle, ProfileExtreme};
    use axioval_ir::ObjectId;

    /// An IFC4X3 alignment by its layouts, stationed from 1000: a 300 m
    /// straight plan; a gradient of 2 % to 100 m, a parabolic sag to a
    /// peak of 6.2 % at 160 m, a parabolic crest down to 3 % at 240 m and
    /// 3 % to the end; and a cant over a 1.5 m rail head distance of
    /// 0.0375 m (2.5 %) to 100 m, a `transition` over 50 m to 0.09 m (6 %)
    /// and 0.09 m to the end.
    fn layouts(transition: &str) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
             FILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4X3_ADD2'));\nENDSEC;\nDATA;\n\
             #4=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #5=IFCAXIS2PLACEMENT3D(#4,$,$);\n\
             #7=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-6,#5,$);\n\
             #9001=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
             #9002=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
             #9003=IFCUNITASSIGNMENT((#9001,#9002));\n\
             #9004=IFCPROJECT('0000000000000000009004',$,'P',$,$,$,$,(#7),#9003);\n\
             #100=IFCCARTESIANPOINT((0.,0.));\n\
             #101=IFCALIGNMENTHORIZONTALSEGMENT($,$,#100,0.,0.,0.,300.,$,.LINE.);\n\
             #104=IFCCARTESIANPOINT((300.,0.));\n\
             #105=IFCALIGNMENTHORIZONTALSEGMENT($,$,#104,0.,0.,0.,0.,$,.LINE.);\n\
             #111=IFCALIGNMENTSEGMENT('seg-h-0000000000000001',$,$,$,$,$,$,#101);\n\
             #115=IFCALIGNMENTSEGMENT('seg-h-0000000000000003',$,$,$,$,$,$,#105);\n\
             #121=IFCALIGNMENTHORIZONTAL('align-horiz-0000000001',$,$,$,$,$,$);\n\
             #122=IFCRELNESTS('nest-h-000000000000001',$,$,$,#121,(#111,#115));\n\
             #200=IFCALIGNMENTVERTICALSEGMENT($,$,0.,100.,50.,0.02,0.02,$,.CONSTANTGRADIENT.);\n\
             #201=IFCALIGNMENTVERTICALSEGMENT($,$,100.,60.,52.,0.02,0.062,1428.5714285714287,.PARABOLICARC.);\n\
             #202=IFCALIGNMENTVERTICALSEGMENT($,$,160.,80.,54.46,0.062,0.03,-2500.,.PARABOLICARC.);\n\
             #203=IFCALIGNMENTVERTICALSEGMENT($,$,240.,60.,58.14,0.03,0.03,$,.CONSTANTGRADIENT.);\n\
             #204=IFCALIGNMENTVERTICALSEGMENT($,$,300.,0.,59.94,0.03,0.03,$,.CONSTANTGRADIENT.);\n\
             #211=IFCALIGNMENTSEGMENT('seg-v-0000000000000001',$,$,$,$,$,$,#200);\n\
             #212=IFCALIGNMENTSEGMENT('seg-v-0000000000000002',$,$,$,$,$,$,#201);\n\
             #213=IFCALIGNMENTSEGMENT('seg-v-0000000000000003',$,$,$,$,$,$,#202);\n\
             #214=IFCALIGNMENTSEGMENT('seg-v-0000000000000004',$,$,$,$,$,$,#203);\n\
             #215=IFCALIGNMENTSEGMENT('seg-v-0000000000000005',$,$,$,$,$,$,#204);\n\
             #221=IFCALIGNMENTVERTICAL('align-vert-00000000001',$,$,$,$,$,$);\n\
             #222=IFCRELNESTS('nest-v-000000000000001',$,$,$,#221,(#211,#212,#213,#214,#215));\n\
             #300=IFCALIGNMENTCANTSEGMENT($,$,0.,100.,0.0375,0.0375,0.,0.,.CONSTANTCANT.);\n\
             #301=IFCALIGNMENTCANTSEGMENT($,$,100.,50.,0.0375,0.09,0.,0.,.{transition}.);\n\
             #302=IFCALIGNMENTCANTSEGMENT($,$,150.,150.,0.09,0.09,0.,0.,.CONSTANTCANT.);\n\
             #303=IFCALIGNMENTCANTSEGMENT($,$,300.,0.,0.09,$,0.,$,.CONSTANTCANT.);\n\
             #311=IFCALIGNMENTSEGMENT('seg-c-0000000000000001',$,$,$,$,$,$,#300);\n\
             #312=IFCALIGNMENTSEGMENT('seg-c-0000000000000002',$,$,$,$,$,$,#301);\n\
             #313=IFCALIGNMENTSEGMENT('seg-c-0000000000000003',$,$,$,$,$,$,#302);\n\
             #314=IFCALIGNMENTSEGMENT('seg-c-0000000000000004',$,$,$,$,$,$,#303);\n\
             #321=IFCALIGNMENTCANT('align-cant-00000000001',$,$,$,$,$,$,1.5);\n\
             #322=IFCRELNESTS('nest-c-000000000000001',$,$,$,#321,(#311,#312,#313,#314));\n\
             #401=IFCALIGNMENT('0000000000000000000401',$,$,$,$,$,$,$);\n\
             #402=IFCRELNESTS('nest-a-000000000000001',$,$,$,#401,(#121,#221,#321));\n\
             #501=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #502=IFCDIRECTION((1.,0.,0.));\n\
             #503=IFCVECTOR(#502,1.);\n\
             #504=IFCLINE(#501,#503);\n\
             #510=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(0.),$,$,$,#504);\n\
             #513=IFCAXIS2PLACEMENTLINEAR(#510,$,$);\n\
             #514=IFCLINEARPLACEMENT($,#513,$);\n\
             #520=IFCREFERENT('referent-0000000000001',$,'STA 1+000',$,$,#514,$,.STATION.);\n\
             #521=IFCRELNESTS('nest-r-000000000000001',$,$,$,#401,(#520));\n\
             #531=IFCPROPERTYSINGLEVALUE('Station',$,IFCLENGTHMEASURE(1000.),$);\n\
             #533=IFCPROPERTYSET('pset-stationing-000001',$,'Pset_Stationing',$,(#531));\n\
             #534=IFCRELDEFINESBYPROPERTIES('rel-defprop-0000000001',$,$,$,(#520),#533);\n\
             ENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    /// The service over `layouts(transition)`, and the alignment.
    fn service(transition: &str) -> (AlignmentServiceHandle, ObjectId) {
        use super::super::super::{ModelBytes, Options, attach};
        let bytes = layouts(transition);
        let session = axioval_ifc::import_ifc_session("layouts.ifc", bytes.as_bytes()).unwrap();
        let models: ModelBytes = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), bytes.as_bytes().to_vec()))
            .collect();
        let (session, _) = attach(session, &models, Options::default()).unwrap();
        let source = session.snapshots().next().unwrap().source().clone();
        let service = session.service::<AlignmentServiceHandle>().unwrap();
        (service.clone(), ObjectId::new(source, "#401").unwrap())
    }

    fn profile(
        service: &AlignmentServiceHandle,
        alignment: &ObjectId,
        parameter: AlignmentParameter,
        (from, to): (Option<f64>, Option<f64>),
    ) -> Result<AlignmentProfile, AlignmentError> {
        service.measure_alignment_profile(
            &AlignmentProfileRequest::try_new(alignment.clone(), parameter, from, to, None)
                .unwrap(),
        )
    }

    /// #339: the gradient's 6.2 % peak at station 1160, read from the
    /// layouts' parabolic curves within the rounding of their laws.
    #[test]
    fn the_peak_gradient_of_the_layouts_is_found_at_its_station() {
        let (service, alignment) = service("LINEARTRANSITION");
        let gradient = profile(
            &service,
            &alignment,
            AlignmentParameter::Gradient,
            (None, None),
        )
        .unwrap();
        let greatest = gradient.extreme(ProfileExtreme::Greatest).unwrap();
        let value = greatest.value();
        assert!(
            value.lower() <= 0.062
                && value.upper() >= 0.062
                && value.upper() - value.lower() < 1e-12,
            "{value:?}"
        );
        let [(from, to)] = greatest.stations() else {
            panic!("one stretch: {:?}", greatest.stations());
        };
        assert!(
            *from <= 1160.0 && *to >= 1160.0 && to - from < 1e-6,
            "{from}..{to}"
        );
        // The constant 2 % is exact: the least, over the first 100 m.
        let least = gradient.extreme(ProfileExtreme::Least).unwrap();
        assert_eq!((least.value().lower(), least.value().upper()), (0.02, 0.02));
        // Over a range that stops on the sag, the end is the greatest.
        let rising = profile(
            &service,
            &alignment,
            AlignmentParameter::Gradient,
            (Some(1000.0), Some(1130.0)),
        )
        .unwrap();
        let greatest = rising.extreme(ProfileExtreme::Greatest).unwrap();
        assert!((greatest.value().upper() - 0.041).abs() < 1e-12);
        // A straight plan: no curvature anywhere, exactly.
        let curvature = profile(
            &service,
            &alignment,
            AlignmentParameter::Curvature,
            (None, None),
        )
        .unwrap();
        let greatest = curvature.extreme(ProfileExtreme::Greatest).unwrap();
        assert_eq!(
            (greatest.value().lower(), greatest.value().upper()),
            (0.0, 0.0)
        );
        assert!(greatest.is_exact());
        // A range beyond the end is refused.
        assert!(matches!(
            profile(
                &service,
                &alignment,
                AlignmentParameter::Gradient,
                (Some(1100.0), Some(1400.0)),
            ),
            Err(AlignmentError::OffRange(_))
        ));
    }

    /// #337: the parabolic sag's vertical curvature is greatest where its
    /// grade is least, 0.042 / 60 / (1 + 0.02^2)^(3/2) at station 1100;
    /// the crest's least where its grade is least, at 1240; both constant
    /// gradients are exactly zero.
    #[test]
    fn the_vertical_curvature_of_the_parabolas_is_bounded_by_their_grades() {
        let (service, alignment) = service("LINEARTRANSITION");
        let curvature = |range| {
            profile(
                &service,
                &alignment,
                AlignmentParameter::VerticalCurvature,
                range,
            )
            .unwrap()
        };
        let whole = curvature((None, None));
        let greatest = whole.extreme(ProfileExtreme::Greatest).unwrap();
        let sag = 0.042 / 60.0 / 1.0004_f64.powf(1.5);
        let value = greatest.value();
        assert!(
            value.lower() <= sag && value.upper() >= sag && value.upper() - value.lower() < 1e-12,
            "{value:?}"
        );
        assert!(
            greatest
                .stations()
                .iter()
                .any(|(from, to)| *from <= 1100.0 && *to >= 1100.0)
        );
        let least = whole.extreme(ProfileExtreme::Least).unwrap();
        let crest = -0.032 / 80.0 / 1.0009_f64.powf(1.5);
        let value = least.value();
        assert!(
            value.lower() <= crest
                && value.upper() >= crest
                && value.upper() - value.lower() < 1e-12,
            "{value:?}"
        );
        // The first constant gradient: exactly straight.
        let straight = curvature((Some(1000.0), Some(1090.0)));
        let value = straight.extreme(ProfileExtreme::Greatest).unwrap();
        assert!(value.is_exact());
        assert_eq!((value.value().lower(), value.value().upper()), (0.0, 0.0));
    }

    /// The cant is greatest, 0.09 m or 6 %, from the transition's end on.
    #[test]
    fn the_greatest_cant_holds_from_the_transition_end() {
        let (service, alignment) = service("LINEARTRANSITION");
        for (parameter, expected) in [
            (AlignmentParameter::Cant, 0.09),
            (AlignmentParameter::CantRatio, 0.06),
        ] {
            let cant = profile(&service, &alignment, parameter, (None, None)).unwrap();
            let greatest = cant.extreme(ProfileExtreme::Greatest).unwrap();
            let value = greatest.value();
            assert!(
                value.lower() <= expected
                    && value.upper() >= expected
                    && value.upper() - value.lower() < 1e-12,
                "{parameter:?}: {value:?}"
            );
            let [(from, to)] = greatest.stations() else {
                panic!("one stretch: {:?}", greatest.stations());
            };
            assert!(
                (from - 1150.0).abs() < 1e-6 && (to - 1300.0).abs() < 1e-6,
                "{from}..{to}"
            );
        }
    }

    /// #338: the linear cant transition from 2.5 % to 6 % over 50 m
    /// changes by 0.07 % per metre, at a station inside it and over it.
    #[test]
    fn the_cant_transition_changes_by_its_rate() {
        let (service, alignment) = service("LINEARTRANSITION");
        let holds = |value: AlignmentInterval, expected: f64, width: f64| {
            assert!(
                value.lower() <= expected
                    && expected <= value.upper()
                    && value.upper() - value.lower() <= width,
                "{value:?} must hold {expected} within {width}"
            );
        };
        let at = |station: f64| {
            profile(
                &service,
                &alignment,
                AlignmentParameter::CantRatio,
                (Some(station), Some(station)),
            )
            .unwrap()
            .rates()
            .unwrap()
            .value()
        };
        holds(at(1125.0), 0.0007, 1e-14);
        // In metres of cant over the 1.5 m rail head distance.
        let cant = profile(
            &service,
            &alignment,
            AlignmentParameter::Cant,
            (Some(1125.0), Some(1125.0)),
        )
        .unwrap();
        holds(cant.rates().unwrap().value(), 0.00105, 1e-14);
        // Where the transition starts, both one-sided rates.
        let seam = at(1100.0);
        assert!(seam.lower() <= 0.0 && seam.upper() >= 0.0007, "{seam:?}");
        // Over the transition's 50 m.
        let over = profile(
            &service,
            &alignment,
            AlignmentParameter::CantRatio,
            (Some(1100.0), Some(1150.0)),
        )
        .unwrap();
        holds(
            over.difference_rate().unwrap().unwrap().value(),
            0.0007,
            1e-14,
        );
        // The steepest ramp anywhere along it is the transition's.
        let whole = profile(
            &service,
            &alignment,
            AlignmentParameter::CantRatio,
            (None, None),
        )
        .unwrap();
        let steepest = whole.rate_extreme(ProfileExtreme::Greatest).unwrap();
        holds(steepest.value(), 0.0007, 1e-14);
        let [(from, to)] = steepest.stations() else {
            panic!("one stretch: {:?}", steepest.stations());
        };
        assert!((from - 1100.0).abs() < 1e-9 && (to - 1150.0).abs() < 1e-9);
    }

    /// Every transition shape's rate bound holds the slope between the ends
    /// of every sampled stretch, so no sample can escape it.
    #[test]
    fn every_cant_shape_is_bounded_by_its_rate() {
        for shape in [
            "LINEARTRANSITION",
            "BLOSSCURVE",
            "COSINECURVE",
            "SINECURVE",
            "HELMERTCURVE",
            "VIENNESEBEND",
        ] {
            let (service, alignment) = service(shape);
            let request = AlignmentProfileRequest::try_new(
                alignment.clone(),
                AlignmentParameter::Cant,
                Some(1100.0),
                Some(1150.0),
                Some(2.5),
            )
            .unwrap();
            let profile = service.measure_alignment_profile(&request).unwrap();
            let pieces = profile.pieces().unwrap();
            assert_eq!(pieces.len(), 20, "{shape}");
            for piece in pieces {
                let (a, b) = (piece.distance().lower(), piece.distance().upper());
                let (start, end) = (piece.start(), piece.end());
                let low = (end.lower() - start.upper()) / (b - a);
                let high = (end.upper() - start.lower()) / (b - a);
                let rate = piece.rate();
                assert!(
                    high >= rate.lower() - 1e-12 && low <= rate.upper() + 1e-12,
                    "{shape} [{a}, {b}]: slope [{low}, {high}] outside {rate:?}"
                );
                assert!(rate.lower() >= -1e-15, "{shape}: a rising cant falls");
            }
            // The greatest is the transition's end, 0.09 m.
            let greatest = profile.extreme(ProfileExtreme::Greatest).unwrap();
            assert!((greatest.value().upper() - 0.09).abs() < 1e-12, "{shape}");
        }
    }

    #[test]
    fn a_law_without_a_rate_bound_is_refused_by_name() {
        let sinusoid = CurvatureLaw::Sinusoid {
            mean: 0.0,
            amplitude: 0.01,
            angular_frequency: 0.1,
            phase: 0.0,
        };
        let mut leaves = Vec::new();
        let why = curvature_leaves(&sinusoid, 0.0, 10.0, 1.0, &mut leaves).unwrap_err();
        assert!(why.contains("sinusoidal spiral"), "{why}");
        // A vertical circular arc's grade rate grows with the grade.
        let arc = ElevationLaw::CircularArc {
            height: 10.0,
            grade: -0.04,
            radius: 1000.0,
        };
        let leaf = Law::Arc {
            law: &arc,
            origin: 0.0,
        };
        let (low, high) = leaf.rate((0.0, 80.0)).unwrap();
        assert!(
            low > 0.0 && low <= 1.0 / 1000.0 && high >= 1.0016 / 1000.0,
            "[{low}, {high}]"
        );
        let (start, end) = (leaf.value(0.0).unwrap(), leaf.value(80.0).unwrap());
        assert!(end.1 - start.0 <= high * 80.0 && end.0 - start.1 >= low * 80.0);
    }
}
