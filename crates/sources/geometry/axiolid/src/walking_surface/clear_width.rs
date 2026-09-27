//! The clear width along a straight flight or a ramp's run: the narrowest
//! free width across it that the requested obstacles leave within a band
//! of heights above its pitch line.
//!
//! In the stretch's frame (along its walking direction, across it, up),
//! the band is, between consecutive points of the pitch line, a convex
//! region: between the stretch's ends, between its walking surface's
//! sides, and between the heights above the pitch line. Each obstacle
//! triangle is clipped to it (Sutherland–Hodgman against six half-spaces),
//! and each clipped polygon, convex, reaches across at a position along as
//! far as its section there. An obstacle lies on the left or the right of
//! the middle of the walking surface; one reaching over the middle refuses.
//! The free width at a position is the distance between the innermost
//! reach from each side there, the walking surface's side where nothing
//! reaches in; it is piecewise linear with its least value at a vertex of a
//! clipped polygon or an end, so only those positions are evaluated.
//!
//! The width is an interval. Its lower bound grows the band by the pitch
//! line's uncertainty and a tessellated obstacle by its chord deviation;
//! its upper bound shrinks the band and leaves tessellated obstacles out,
//! which can only widen the width. Both carry a numerical margin, so the
//! width is never exact.

use axioval_engine::{
    ClearWidthEvidence, ClearWidthRequest, MeasuredInterval, WalkingStretch, WalkingSurfaceError,
};
use axioval_ir::{Evidence, ObjectId};

use super::handrail::{Pitch, flight_pitch};
use super::{AxiolidWalkingSurfaceService, PlanFrame};
use crate::geometry::{Triangle, triangles};

/// Relative numerical margin on a clear width: the rounding of mapping,
/// clipping and interpolating coordinates, with room.
const MARGIN: f64 = 1e-9;

/// A point in the stretch's frame: along, across and up.
type Local = [f64; 3];

/// A half-space `normal · p + offset >= 0` in the stretch's frame.
#[derive(Clone, Copy)]
struct Plane {
    normal: [f64; 3],
    offset: f64,
}

impl Plane {
    fn at(&self, point: Local) -> f64 {
        self.normal[0].mul_add(
            point[0],
            self.normal[1].mul_add(point[1], self.normal[2].mul_add(point[2], self.offset)),
        )
    }
}

/// The part of convex `polygon` inside `plane`.
fn clip(polygon: &[Local], plane: &Plane) -> Vec<Local> {
    let mut kept = Vec::with_capacity(polygon.len() + 1);
    for (index, current) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        let (here, there) = (plane.at(*current), plane.at(next));
        if here >= 0.0 {
            kept.push(*current);
        }
        if (here >= 0.0) != (there >= 0.0) {
            let share = here / (here - there);
            kept.push([
                share.mul_add(next[0] - current[0], current[0]),
                share.mul_add(next[1] - current[1], current[1]),
                share.mul_add(next[2] - current[2], current[2]),
            ]);
        }
    }
    kept
}

/// The band's convex pieces, one per segment of the pitch line within
/// `along`, between `across` and `from` to `to` above the line.
fn pieces(
    line: &[(f64, f64)],
    along: (f64, f64),
    across: (f64, f64),
    (from, to): (f64, f64),
) -> Vec<[Plane; 6]> {
    let mut pieces = Vec::new();
    for pair in line.windows(2) {
        let ((a, za), (b, zb)) = (pair[0], pair[1]);
        let (low, high) = (a.max(along.0), b.min(along.1));
        if high < low {
            continue;
        }
        let slope = (zb - za) / (b - a);
        // z - (za + slope·(s - a)) - from >= 0, and the same below `to`.
        let base = slope.mul_add(a, -za);
        pieces.push([
            Plane {
                normal: [1.0, 0.0, 0.0],
                offset: -low,
            },
            Plane {
                normal: [-1.0, 0.0, 0.0],
                offset: high,
            },
            Plane {
                normal: [0.0, 1.0, 0.0],
                offset: -across.0,
            },
            Plane {
                normal: [0.0, -1.0, 0.0],
                offset: across.1,
            },
            Plane {
                normal: [-slope, 0.0, 1.0],
                offset: base - from,
            },
            Plane {
                normal: [slope, 0.0, -1.0],
                offset: to - base,
            },
        ]);
    }
    pieces
}

/// The sections of one obstacle within the band, as polygons in plan along
/// and across.
struct Sections {
    obstacle: ObjectId,
    polygons: Vec<Vec<[f64; 2]>>,
}

impl Sections {
    fn cut(obstacle: &ObjectId, local: &[[Local; 3]], band: &[[Plane; 6]]) -> Self {
        let mut polygons = Vec::new();
        for triangle in local {
            for piece in band {
                let mut polygon = triangle.to_vec();
                for plane in piece {
                    if polygon.is_empty() {
                        break;
                    }
                    polygon = clip(&polygon, plane);
                }
                if !polygon.is_empty() {
                    polygons.push(polygon.iter().map(|point| [point[0], point[1]]).collect());
                }
            }
        }
        Self {
            obstacle: obstacle.clone(),
            polygons,
        }
    }

    /// The least and greatest position across of every section.
    fn span(&self) -> Option<(f64, f64)> {
        self.polygons.iter().flatten().map(|point| point[1]).fold(
            None,
            |span: Option<(f64, f64)>, value| {
                Some(span.map_or((value, value), |(low, high)| {
                    (low.min(value), high.max(value))
                }))
            },
        )
    }
}

/// The positions across `polygon`'s section at `along`, if it has one.
fn section(polygon: &[[f64; 2]], along: f64) -> Option<(f64, f64)> {
    let mut span: Option<(f64, f64)> = None;
    let mut add = |value: f64| {
        span = Some(span.map_or((value, value), |(low, high)| {
            (low.min(value), high.max(value))
        }));
    };
    for (index, current) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        #[allow(clippy::float_cmp)]
        if current[0] == along {
            add(current[1]);
        }
        if (current[0] - along) * (next[0] - along) < 0.0 {
            let share = (along - current[0]) / (next[0] - current[0]);
            add(share.mul_add(next[1] - current[1], current[1]));
        }
    }
    span
}

/// Which side of the walking surface's middle an obstacle stands on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Low,
    High,
}

/// The narrowest free width between `sides` over `along` that the sections
/// leave, grown on each side by `grow`, and the obstacles bounding it.
fn narrowest(
    sections: &[(Side, &Sections)],
    sides: (f64, f64),
    along: (f64, f64),
    grow: f64,
) -> (f64, Vec<ObjectId>) {
    let mut positions = vec![along.0, along.1];
    for (_, sections) in sections {
        for point in sections.polygons.iter().flatten() {
            if point[0] >= along.0 && point[0] <= along.1 {
                positions.push(point[0]);
            }
        }
    }
    let mut least: Option<(f64, Vec<ObjectId>)> = None;
    for position in positions {
        let (mut low, mut high) = (sides.0, sides.1);
        let (mut by_low, mut by_high) = (None, None);
        for (side, sections) in sections {
            for polygon in &sections.polygons {
                let Some((near, far)) = section(polygon, position) else {
                    continue;
                };
                // An obstacle bounds the width by name only where it reaches
                // inside the walking surface's side, not by the margin.
                match side {
                    Side::Low if far + grow > low => {
                        low = far + grow;
                        by_low = (far > sides.0).then(|| sections.obstacle.clone());
                    }
                    Side::High if near - grow < high => {
                        high = near - grow;
                        by_high = (near < sides.1).then(|| sections.obstacle.clone());
                    }
                    _ => {}
                }
            }
        }
        let width = (high - low).max(0.0);
        if least.as_ref().is_none_or(|(least, _)| width < *least) {
            least = Some((width, by_low.into_iter().chain(by_high).collect()));
        }
    }
    least.unwrap_or((0.0, Vec::new()))
}

impl AxiolidWalkingSurfaceService {
    /// The clear width along the requested stretch.
    #[allow(clippy::too_many_lines)]
    pub(super) fn clear_width(
        &self,
        request: &ClearWidthRequest,
    ) -> Result<ClearWidthEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let pitch: Pitch = match request.stretch() {
            WalkingStretch::Flight => {
                let (flight, _) = self.framed_flight(subject)?;
                if flight.walking_line().is_turning() {
                    return Err(WalkingSurfaceError::Unsupported(format!(
                        "{subject} is a turning flight, whose clear width across its winders is \
                         not measured"
                    )));
                }
                flight_pitch(subject, &flight)?
            }
            WalkingStretch::Run(index) => self.run_pitch(subject, index)?,
        };
        let line = pitch.nominal();
        let (Some(first), Some(last)) = (line.first(), line.last()) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        if line.len() < 2 {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "the pitch line of {subject} has a single point, so no band lies along it"
            )));
        }
        let slope = Pitch::slope(&line);
        let height_error = pitch.error(slope);
        let along_error = pitch.along_error();
        let (left, right) = pitch.sides;
        let frame = PlanFrame::new(pitch.direction);
        let band = request.band();
        let mut exact = Vec::new();
        let mut scale = first
            .0
            .abs()
            .max(last.0.abs())
            .max(first.1.abs())
            .max(last.1.abs())
            .max(left.lower_metres().abs())
            .max(right.upper_metres().abs())
            .max(1.0);
        let mut obstacles = Vec::new();
        for obstacle in request.obstacles() {
            let Some((soup, deviation)) = self.obstacle(obstacle)? else {
                continue;
            };
            let local: Vec<[Local; 3]> = soup
                .iter()
                .map(|triangle| {
                    triangle.map(|point| {
                        let [along, across] = frame.map(point.x, point.y);
                        [along, across, point.z]
                    })
                })
                .collect();
            for point in local.iter().flatten() {
                scale = point
                    .iter()
                    .fold(scale, |scale, value| scale.max(value.abs()));
            }
            obstacles.push((obstacle.clone(), local, deviation));
            if deviation == 0.0 {
                exact.push(obstacles.len() - 1);
            }
        }
        let margin = MARGIN * scale * (1.0 + slope);
        let deviation = obstacles
            .iter()
            .map(|(_, _, deviation)| *deviation)
            .fold(0.0_f64, f64::max);
        // Grown: sure to hold every obstacle point that may lie in the band.
        let grow = height_error + deviation + margin;
        let grown = pieces(
            &line,
            (first.0 - along_error - grow, last.0 + along_error + grow),
            (left.lower_metres() - grow, right.upper_metres() + grow),
            (band.0 - grow, band.1 + grow),
        );
        // Shrunk: every point surely in the band.
        let inset = height_error + margin;
        let shrunk_along = (first.0 + along_error + inset, last.0 - along_error - inset);
        let shrunk = pieces(
            &line,
            shrunk_along,
            (left.upper_metres() + inset, right.lower_metres() - inset),
            (band.0 + inset, band.1 - inset),
        );
        let middle = f64::midpoint(
            f64::midpoint(left.lower_metres(), left.upper_metres()),
            f64::midpoint(right.lower_metres(), right.upper_metres()),
        );
        let mut near = Vec::new();
        let mut sure = Vec::new();
        for (index, (obstacle, local, deviation)) in obstacles.iter().enumerate() {
            let sections = Sections::cut(obstacle, local, &grown);
            let Some((low, high)) = sections.span() else {
                if encloses(local, &line, band, middle) {
                    return Err(WalkingSurfaceError::Unsupported(format!(
                        "{obstacle} may enclose the band over {subject} without a face in it"
                    )));
                }
                continue;
            };
            let side = if high + deviation < middle {
                Side::Low
            } else if low - deviation > middle {
                Side::High
            } else {
                return Err(WalkingSurfaceError::Unsupported(format!(
                    "{obstacle} reaches over the middle of {subject}, so the free width beside \
                     it is not measured"
                )));
            };
            near.push((side, sections, *deviation));
            if exact.contains(&index) && shrunk_along.0 <= shrunk_along.1 {
                sure.push((side, Sections::cut(obstacle, local, &shrunk)));
            }
        }
        let grown_sections: Vec<(Side, &Sections)> = near
            .iter()
            .map(|(side, sections, _)| (*side, sections))
            .collect();
        let (lower, governing) = narrowest(
            &grown_sections,
            (left.upper_metres(), right.lower_metres()),
            (first.0 - along_error - grow, last.0 + along_error + grow),
            deviation + margin,
        );
        let upper = if shrunk_along.0 <= shrunk_along.1 {
            let sure_sections: Vec<(Side, &Sections)> = sure
                .iter()
                .map(|(side, sections)| (*side, sections))
                .collect();
            narrowest(
                &sure_sections,
                (left.lower_metres(), right.upper_metres()),
                shrunk_along,
                -margin,
            )
            .0
        } else {
            right.upper_metres() - left.lower_metres() + margin
        };
        let width =
            MeasuredInterval::try_new((lower - margin).max(0.0), upper.max(lower) + margin)?;
        ClearWidthEvidence::try_new(
            request.clone(),
            width,
            governing,
            Evidence {
                source: subject.source.clone(),
                locator: format!("clear-width:{subject}"),
                exact: false,
            },
        )
    }

    /// An obstacle's triangles and chord deviation, `None` without a body.
    fn obstacle(
        &self,
        obstacle: &ObjectId,
    ) -> Result<Option<(Vec<Triangle>, f64)>, WalkingSurfaceError> {
        if self.geometry.has_no_body(obstacle) {
            return Ok(None);
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == obstacle)
        {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "obstacle {obstacle} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(obstacle)
            .ok_or_else(|| WalkingSurfaceError::UnknownObject(obstacle.clone()))?;
        let deviation = self
            .geometry
            .fidelity(obstacle)
            .map_err(|_| {
                WalkingSurfaceError::Unavailable(format!(
                    "obstacle {obstacle} has an invalid chord deviation"
                ))
            })?
            .deviation_metres();
        Ok(Some((triangles(mesh), deviation)))
    }
}

/// Whether an obstacle with no face in the band may still enclose it: its
/// box in the stretch's frame holds the band's middle, `across` the middle
/// of the walking surface.
fn encloses(
    local: &[[Local; 3]],
    line: &[(f64, f64)],
    (from, to): (f64, f64),
    across: f64,
) -> bool {
    let (Some(first), Some(last)) = (line.first(), line.last()) else {
        return false;
    };
    let along = f64::midpoint(first.0, last.0);
    let height = super::handrail::pitch_at(line, along) + f64::midpoint(from, to);
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for point in local.iter().flatten() {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let centre = [along, across, height];
    (0..3).all(|axis| low[axis] <= centre[axis] && centre[axis] <= high[axis])
}
