//! Cross-sections normal to an alignment and clearance envelopes swept
//! along it.
//!
//! ADR 0004: this seam measures. Whether a member is thick enough at a
//! station, or whether a structure stays clear of a clearance envelope, is
//! a rule's judgement over these measurements.
//!
//! # The section frame
//!
//! The section at a station is the vertical plane normal to the
//! alignment's plan there. Its coordinates are `lateral`, horizontal and
//! positive to the left of the direction of travel, and `up`, vertical,
//! both in metres from the centreline point on the gradient line: a point
//! of the plane at `(lateral, up)` has that `offset` and that
//! `height_above_gradient` ([`crate::alignment`]). Cant does not turn the
//! frame; an envelope that tilts with the cant is stated tilted.
//!
//! # An interval region
//!
//! A body's section is never known exactly: its mesh deviates from the
//! body by the deviation its compiler certifies, and the plane is placed
//! by a numerical evaluation of the centreline. A [`BodySection`] holds
//! the region as the cut of the body's mesh by the computed plane
//! (oriented segments, the region to their left), the projections onto the
//! plane of every piece of the mesh within `radius` of the plane (the
//! band), and that `radius`, which covers the deviation and the plane's
//! numerical error. Then:
//!
//! - a point **surely** lies in the body's section when it lies in the cut
//!   and farther than `radius` from every band polygon;
//! - a point **possibly** lies in it when it lies in the cut or within
//!   `radius` of a band polygon.
//!
//! The area and the extents are derived here from that region, as
//! intervals sure to hold the exact value, never accepted from an adapter.
//!
//! # A clearance envelope swept along a range
//!
//! A clearance envelope is a simple polygon in section coordinates
//! ([`SectionPolygon`]). Swept from one station to another it is the
//! volume of every point at `(lateral, up)` in the envelope in the section
//! at any distance in between. An [`EnvelopeRequest`] asks, for each
//! selected body, whether it reaches into that volume, and the service
//! answers each with an [`Intrusion`]: `Sure` with the station where a
//! point of the body was proven inside, `Clear` when the body was proven
//! out of the whole volume, `Possible` with the reason otherwise. Sampling
//! the declared step is never enough on its own: between two samples the
//! service bounds how far the envelope can move and must prove a body
//! clear of the envelope grown by that bound, so a body crossing the
//! envelope between two samples, however thin, is found or left possible,
//! never cleared.

use axioval_ir::measured::polygon_problem;
use axioval_ir::{Evidence, ObjectId};

use crate::alignment::{AlignmentError, AlignmentInterval};

/// A simple polygon in section coordinates `(lateral, up)`, in metres,
/// counter-clockwise.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionPolygon {
    vertices: Vec<[f64; 2]>,
}

impl SectionPolygon {
    /// A polygon from its vertices in either orientation: at least three,
    /// finite, enclosing an area and never crossing or touching itself.
    pub fn try_new(mut vertices: Vec<[f64; 2]>) -> Result<Self, AlignmentError> {
        if polygon_problem(&vertices).is_some() {
            return Err(AlignmentError::InvalidMeasurement);
        }
        if twice_signed_area(&vertices) < 0.0 {
            vertices.reverse();
        }
        Ok(Self { vertices })
    }

    /// The vertices, counter-clockwise.
    #[must_use]
    pub fn vertices(&self) -> &[[f64; 2]] {
        &self.vertices
    }

    /// The greatest distance of a vertex from the centreline, laterally.
    #[must_use]
    pub fn lateral_reach(&self) -> f64 {
        self.vertices
            .iter()
            .map(|vertex| vertex[0].abs())
            .fold(0.0, f64::max)
    }

    /// Whether `point` lies inside the polygon, farther than `margin` from
    /// its boundary.
    #[must_use]
    pub fn contains_beyond(&self, point: [f64; 2], margin: f64) -> bool {
        let edges = self.edges();
        winding(edges.iter().copied(), point) != 0
            && edges
                .iter()
                .all(|edge| segment_distance(point, *edge) > margin)
    }

    /// The polygon's edges, counter-clockwise.
    #[must_use]
    pub fn edges(&self) -> Vec<[[f64; 2]; 2]> {
        let count = self.vertices.len();
        (0..count)
            .map(|i| [self.vertices[i], self.vertices[(i + 1) % count]])
            .collect()
    }
}

/// Twice the signed area of a polygon, positive counter-clockwise.
fn twice_signed_area(vertices: &[[f64; 2]]) -> f64 {
    let count = vertices.len();
    (0..count)
        .map(|i| {
            let ([ax, ay], [bx, by]) = (vertices[i], vertices[(i + 1) % count]);
            ax * by - bx * ay
        })
        .sum()
}

/// The winding number of oriented segments forming closed cycles around
/// `point`.
fn winding(segments: impl Iterator<Item = [[f64; 2]; 2]>, point: [f64; 2]) -> i64 {
    let mut winding = 0;
    for [a, b] in segments {
        let side = (b[0] - a[0]) * (point[1] - a[1]) - (point[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= point[1] && b[1] > point[1] && side > 0.0 {
            winding += 1;
        } else if b[1] <= point[1] && a[1] > point[1] && side < 0.0 {
            winding -= 1;
        }
    }
    winding
}

/// The distance from `point` to the closed segment `[a, b]`.
fn segment_distance(point: [f64; 2], [a, b]: [[f64; 2]; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let along = if length > 0.0 {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point[0] - a[0] - along * dx).hypot(point[1] - a[1] - along * dy)
}

/// The distance from `point` to a convex polygon (a point, a segment or a
/// polygon in either orientation), zero inside.
fn convex_distance(point: [f64; 2], polygon: &[[f64; 2]]) -> f64 {
    match polygon {
        [] => f64::INFINITY,
        [only] => (point[0] - only[0]).hypot(point[1] - only[1]),
        _ => {
            let count = polygon.len();
            let edges = (0..count).map(|i| [polygon[i], polygon[(i + 1) % count]]);
            let sides: Vec<f64> = edges
                .clone()
                .map(|[a, b]| (b[0] - a[0]) * (point[1] - a[1]) - (point[0] - a[0]) * (b[1] - a[1]))
                .collect();
            let inside = count > 2
                && (sides.iter().all(|side| *side > 0.0) || sides.iter().all(|side| *side < 0.0));
            if inside {
                0.0
            } else {
                edges
                    .map(|edge| segment_distance(point, edge))
                    .fold(f64::INFINITY, f64::min)
            }
        }
    }
}

/// A direction across a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SectionAxis {
    /// Horizontal, positive to the left of the direction of travel.
    Lateral,
    /// Vertical, positive upwards.
    Up,
}

/// The section of `objects`' bodies at `station` along `alignment`.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionRequest {
    alignment: ObjectId,
    station: f64,
    objects: Vec<ObjectId>,
}

impl SectionRequest {
    /// A request for at least one body, sorted without repeats, at a
    /// finite station; the alignment is never one of the bodies.
    pub fn try_new(
        alignment: ObjectId,
        station: f64,
        mut objects: Vec<ObjectId>,
    ) -> Result<Self, AlignmentError> {
        objects.sort();
        objects.dedup();
        if objects.is_empty() || !station.is_finite() || objects.contains(&alignment) {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            alignment,
            station,
            objects,
        })
    }

    /// The alignment cut across.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }

    /// The station, as the alignment labels it, in metres.
    #[must_use]
    pub fn station(&self) -> f64 {
        self.station
    }

    /// The bodies cut, sorted.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }
}

/// One body's section as an interval region (module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct BodySection {
    object: ObjectId,
    cut: Vec<[[f64; 2]; 2]>,
    band: Vec<Vec<[f64; 2]>>,
    radius: f64,
}

impl BodySection {
    /// A section: the oriented segments of the mesh's cut (closed cycles,
    /// the region to their left), the convex polygons of the band (each
    /// at least one vertex) and the radius around them. Every coordinate
    /// and the radius must be finite, the radius not negative.
    pub fn try_new(
        object: ObjectId,
        cut: Vec<[[f64; 2]; 2]>,
        band: Vec<Vec<[f64; 2]>>,
        radius: f64,
    ) -> Result<Self, AlignmentError> {
        let finite = cut
            .iter()
            .flatten()
            .flatten()
            .all(|value| value.is_finite())
            && band
                .iter()
                .flatten()
                .flatten()
                .all(|value| value.is_finite());
        if !finite || !radius.is_finite() || radius < 0.0 || band.iter().any(Vec::is_empty) {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            object,
            cut,
            band,
            radius,
        })
    }

    /// The body cut.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The oriented segments of the mesh's cut, the region to their left.
    #[must_use]
    pub fn cut(&self) -> &[[[f64; 2]; 2]] {
        &self.cut
    }

    /// The convex polygons of the band.
    #[must_use]
    pub fn band(&self) -> &[Vec<[f64; 2]>] {
        &self.band
    }

    /// How far around the band the section may differ from the cut.
    #[must_use]
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Whether the section is known exactly: its radius is zero.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.radius == 0.0
    }

    fn in_cut(&self, point: [f64; 2]) -> bool {
        winding(self.cut.iter().copied(), point) != 0
    }

    fn near_band(&self, point: [f64; 2]) -> bool {
        self.band
            .iter()
            .any(|polygon| convex_distance(point, polygon) <= self.radius)
    }

    /// Whether `point` surely lies in the body's section.
    #[must_use]
    pub fn surely_inside(&self, point: [f64; 2]) -> bool {
        self.in_cut(point) && !self.near_band(point)
    }

    /// Whether `point` possibly lies in the body's section.
    #[must_use]
    pub fn possibly_inside(&self, point: [f64; 2]) -> bool {
        self.in_cut(point) || self.near_band(point)
    }

    /// Whether the plane surely misses the body: no cut and no band.
    #[must_use]
    pub fn surely_empty(&self) -> bool {
        self.cut.is_empty() && self.band.is_empty()
    }

    /// The section's area, in square metres: the cut's area less or plus
    /// the area of every band polygon grown by the radius.
    ///
    /// # Errors
    ///
    /// [`AlignmentError::InvalidMeasurement`] when the sums overflow.
    pub fn area(&self) -> Result<AlignmentInterval, AlignmentError> {
        let (mut twice, mut magnitude) = (0.0_f64, 0.0_f64);
        for [a, b] in &self.cut {
            let term = a[0] * b[1] - b[0] * a[1];
            twice += term;
            magnitude += term.abs();
        }
        let cut = 0.5 * twice;
        let mut band = 0.0;
        for polygon in &self.band {
            let count = polygon.len();
            let area = if count > 2 {
                0.5 * twice_signed_area(polygon).abs()
            } else {
                0.0
            };
            let perimeter: f64 = if count > 1 {
                (0..count)
                    .map(|i| {
                        let (a, b) = (polygon[i], polygon[(i + 1) % count]);
                        (b[0] - a[0]).hypot(b[1] - a[1])
                    })
                    .sum()
            } else {
                0.0
            };
            band += area + perimeter * self.radius + std::f64::consts::PI * self.radius.powi(2);
            magnitude += area + perimeter * self.radius;
        }
        // Each sum rounds once per term.
        #[allow(clippy::cast_precision_loss)]
        let terms = (self.cut.len() + self.band.len() + 1) as f64;
        let rounding = 4.0 * f64::EPSILON * (magnitude + band) * terms;
        let lower = (cut - band - rounding).max(0.0);
        let upper = (cut + band + rounding).max(lower);
        AlignmentInterval::try_new(lower, upper)
    }

    /// How far the section reaches along `axis`, in metres: the spread of
    /// points proven inside up to the spread of everything possibly
    /// inside; `None` when the plane surely misses the body.
    ///
    /// # Errors
    ///
    /// [`AlignmentError::InvalidMeasurement`] when the spread overflows.
    pub fn extent(&self, axis: SectionAxis) -> Result<Option<AlignmentInterval>, AlignmentError> {
        if self.surely_empty() {
            return Ok(None);
        }
        let index = match axis {
            SectionAxis::Lateral => 0,
            SectionAxis::Up => 1,
        };
        let mut outer = (f64::INFINITY, f64::NEG_INFINITY);
        let mut grow = |value: f64, by: f64| {
            outer.0 = outer.0.min(value - by);
            outer.1 = outer.1.max(value + by);
        };
        for segment in &self.cut {
            for point in segment {
                grow(point[index], 0.0);
            }
        }
        for polygon in &self.band {
            for point in polygon {
                grow(point[index], self.radius);
            }
        }
        let mut inner: Option<(f64, f64)> = None;
        for point in self.witnesses() {
            let value = point[index];
            inner = Some(inner.map_or((value, value), |(low, high)| {
                (low.min(value), high.max(value))
            }));
        }
        let scale = outer.0.abs().max(outer.1.abs()).max(1.0);
        let rounding = 8.0 * f64::EPSILON * scale;
        let upper = (outer.1 - outer.0 + rounding).max(0.0);
        let lower = inner.map_or(0.0, |(low, high)| (high - low - rounding).max(0.0));
        AlignmentInterval::try_new(lower.min(upper), upper).map(Some)
    }

    /// Points proven inside: each cut segment's midpoint moved inwards by
    /// a few multiples of the radius, kept where the region surely holds
    /// them.
    #[must_use]
    pub fn witnesses(&self) -> Vec<[f64; 2]> {
        let mut found = Vec::new();
        for [a, b] in &self.cut {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = dx.hypot(dy);
            if length <= 0.0 || !length.is_finite() {
                continue;
            }
            let (nx, ny) = (-dy / length, dx / length);
            let middle = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
            let scale = middle[0].abs().max(middle[1].abs()).max(1.0);
            let base = self.radius * (1.0 + 1e-9) + 1e-12 * scale;
            for factor in [1.0, 2.0, 4.0] {
                let shift = base * factor;
                let point = [middle[0] + nx * shift, middle[1] + ny * shift];
                if self.surely_inside(point) {
                    found.push(point);
                    break;
                }
            }
        }
        found
    }
}

/// The sections answering a [`SectionRequest`]: one per body, in the
/// request's order, with evidence exact exactly when every section is.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    request: SectionRequest,
    bodies: Vec<BodySection>,
    evidence: Evidence,
}

impl Section {
    /// The sections answering `request`.
    pub fn try_new(
        request: SectionRequest,
        bodies: Vec<BodySection>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        let matches = bodies.len() == request.objects.len()
            && bodies
                .iter()
                .zip(&request.objects)
                .all(|(body, object)| &body.object == object);
        if !matches {
            return Err(AlignmentError::InvalidMeasurement);
        }
        let exact = bodies.iter().all(BodySection::is_exact);
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            bodies,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &SectionRequest {
        &self.request
    }

    /// One section per body, in the request's order.
    #[must_use]
    pub fn bodies(&self) -> &[BodySection] {
        &self.bodies
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Which of `bodies` reach into `envelope` swept along `alignment` from
/// station `from` to station `to`, sampled every `step` metres of plan
/// distance.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeRequest {
    alignment: ObjectId,
    envelope: SectionPolygon,
    from: f64,
    to: f64,
    step: f64,
    bodies: Vec<ObjectId>,
}

impl EnvelopeRequest {
    /// A request over at least one body, sorted without repeats; `from`
    /// not after `to`, both finite, and a positive finite step. The
    /// alignment is never one of the bodies.
    pub fn try_new(
        alignment: ObjectId,
        envelope: SectionPolygon,
        (from, to): (f64, f64),
        step: f64,
        mut bodies: Vec<ObjectId>,
    ) -> Result<Self, AlignmentError> {
        bodies.sort();
        bodies.dedup();
        let range = from.is_finite() && to.is_finite() && from <= to;
        if !range
            || !step.is_finite()
            || step <= 0.0
            || bodies.is_empty()
            || bodies.contains(&alignment)
        {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            alignment,
            envelope,
            from,
            to,
            step,
            bodies,
        })
    }

    /// The alignment the envelope is swept along.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }

    /// The clearance envelope, in section coordinates.
    #[must_use]
    pub fn envelope(&self) -> &SectionPolygon {
        &self.envelope
    }

    /// The stations the range starts and ends at, as the alignment labels
    /// them, in metres.
    #[must_use]
    pub fn range(&self) -> (f64, f64) {
        (self.from, self.to)
    }

    /// The declared plan distance between sampled sections, in metres.
    #[must_use]
    pub fn step(&self) -> f64 {
        self.step
    }

    /// The bodies checked, sorted.
    #[must_use]
    pub fn bodies(&self) -> &[ObjectId] {
        &self.bodies
    }
}

/// Whether one body reaches into the swept envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum Intrusion {
    /// A point of the body was proven inside the envelope in the section
    /// at a plan distance in `distance`, within the range.
    Sure {
        /// Plan distances, in metres, the proving section lies at.
        distance: AlignmentInterval,
    },
    /// The body may reach into the envelope; the reason says where and why
    /// it could not be decided.
    Possible(String),
    /// The body was proven out of the whole swept envelope.
    Clear,
}

/// The intrusions answering an [`EnvelopeRequest`], one per body in the
/// request's order, with evidence exact exactly when every body was
/// decided.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeSweep {
    request: EnvelopeRequest,
    intrusions: Vec<(ObjectId, Intrusion)>,
    evidence: Evidence,
}

impl EnvelopeSweep {
    /// The intrusions answering `request`.
    pub fn try_new(
        request: EnvelopeRequest,
        intrusions: Vec<(ObjectId, Intrusion)>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        let matches = intrusions.len() == request.bodies.len()
            && intrusions
                .iter()
                .zip(&request.bodies)
                .all(|((body, _), requested)| body == requested);
        if !matches {
            return Err(AlignmentError::InvalidMeasurement);
        }
        let decided = intrusions
            .iter()
            .all(|(_, intrusion)| !matches!(intrusion, Intrusion::Possible(_)));
        if evidence.exact != decided || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            intrusions,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &EnvelopeRequest {
        &self.request
    }

    /// One intrusion per body, in the request's order.
    #[must_use]
    pub fn intrusions(&self) -> &[(ObjectId, Intrusion)] {
        &self.intrusions
    }

    /// How many bodies surely intrude, and how many may: an interval
    /// holding the exact count.
    #[must_use]
    pub fn count(&self) -> (usize, usize) {
        let sure = self
            .intrusions
            .iter()
            .filter(|(_, intrusion)| matches!(intrusion, Intrusion::Sure { .. }))
            .count();
        let possible = self
            .intrusions
            .iter()
            .filter(|(_, intrusion)| !matches!(intrusion, Intrusion::Clear))
            .count();
        (sure, possible)
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    fn evidence(exact: bool) -> Evidence {
        let mut evidence = Evidence::exact(SourceId::new("test", "model").unwrap(), "cut");
        evidence.exact = exact;
        evidence
    }

    /// A box section 2 m wide and 1 m high, from `(-1, 0)` to `(1, 1)`,
    /// counter-clockwise, its sides the band, within `radius`.
    fn rectangle(radius: f64) -> BodySection {
        let corners = [[-1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [-1.0, 1.0]];
        let cut: Vec<[[f64; 2]; 2]> = (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect();
        let band = cut.iter().map(|[a, b]| vec![*a, *b]).collect();
        BodySection::try_new(id("wall"), cut, band, radius).unwrap()
    }

    #[test]
    fn a_rectangle_has_its_area_and_extents_within_the_radius() {
        let exact = rectangle(0.0);
        let area = exact.area().unwrap();
        assert!(area.lower() <= 2.0 && area.upper() >= 2.0 && area.upper() - area.lower() < 1e-12);
        let wide = rectangle(0.001);
        let area = wide.area().unwrap();
        assert!(
            area.lower() < 2.0 - 0.005 && area.lower() > 1.98,
            "{area:?}"
        );
        assert!(
            area.upper() > 2.0 + 0.005 && area.upper() < 2.02,
            "{area:?}"
        );
        let up = wide.extent(SectionAxis::Up).unwrap().unwrap();
        assert!(up.lower() <= 1.0 - 0.002 && up.lower() > 0.99, "{up:?}");
        assert!(up.upper() >= 1.002 && up.upper() < 1.003, "{up:?}");
        let lateral = wide.extent(SectionAxis::Lateral).unwrap().unwrap();
        assert!(
            lateral.lower() <= 2.0 && lateral.upper() >= 2.0,
            "{lateral:?}"
        );
        assert!(wide.surely_inside([0.0, 0.5]));
        assert!(!wide.surely_inside([0.0, 0.0005]));
        assert!(wide.possibly_inside([0.0, -0.0005]));
        assert!(!wide.possibly_inside([0.0, -0.01]));
    }

    #[test]
    fn a_plane_near_a_body_but_not_cutting_it_is_possibly_a_section() {
        // A face lying just beyond the plane: no cut, but a band.
        let near = BodySection::try_new(
            id("slab"),
            Vec::new(),
            vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
            0.001,
        )
        .unwrap();
        let area = near.area().unwrap();
        assert_eq!(area.lower(), 0.0);
        assert!(area.upper() >= 0.5);
        let up = near.extent(SectionAxis::Up).unwrap().unwrap();
        assert_eq!(up.lower(), 0.0);
        assert!(up.upper() >= 1.0);
        let missed = BodySection::try_new(id("slab"), Vec::new(), Vec::new(), 0.001).unwrap();
        assert!(missed.extent(SectionAxis::Up).unwrap().is_none());
        assert_eq!(missed.area().unwrap().upper(), 0.0);
    }

    #[test]
    fn polygons_are_simple_and_turned_counter_clockwise() {
        let clockwise =
            SectionPolygon::try_new(vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]]).unwrap();
        assert_eq!(clockwise.vertices()[1], [2.0, 2.0]);
        assert!(clockwise.contains_beyond([1.0, 1.0], 0.5));
        assert!(!clockwise.contains_beyond([1.0, 1.9], 0.5));
        assert!(!clockwise.contains_beyond([3.0, 1.0], 0.0));
        assert_eq!(clockwise.lateral_reach(), 2.0);
        assert!(SectionPolygon::try_new(vec![[0.0, 0.0], [1.0, 1.0]]).is_err());
    }

    #[test]
    fn requests_and_answers_must_match() {
        assert!(SectionRequest::try_new(id("axis"), 1.0, vec![id("axis")]).is_err());
        assert!(SectionRequest::try_new(id("axis"), f64::NAN, vec![id("wall")]).is_err());
        let request = SectionRequest::try_new(id("axis"), 1.0, vec![id("wall")]).unwrap();
        assert!(Section::try_new(request.clone(), vec![rectangle(0.0)], evidence(true)).is_ok());
        assert_eq!(
            Section::try_new(request.clone(), vec![rectangle(0.1)], evidence(true)),
            Err(AlignmentError::InexactEvidence)
        );
        assert_eq!(
            Section::try_new(request, Vec::new(), evidence(true)),
            Err(AlignmentError::InvalidMeasurement)
        );
        let envelope =
            SectionPolygon::try_new(vec![[-2.0, 0.0], [2.0, 0.0], [2.0, 5.0], [-2.0, 5.0]])
                .unwrap();
        assert!(
            EnvelopeRequest::try_new(id("axis"), envelope.clone(), (5.0, 1.0), 1.0, vec![id("a")])
                .is_err()
        );
        assert!(
            EnvelopeRequest::try_new(id("axis"), envelope.clone(), (1.0, 5.0), 0.0, vec![id("a")])
                .is_err()
        );
        let request = EnvelopeRequest::try_new(
            id("axis"),
            envelope,
            (0.0, 10.0),
            1.0,
            vec![id("b"), id("a")],
        )
        .unwrap();
        assert_eq!(request.bodies(), [id("a"), id("b")]);
        let sweep = EnvelopeSweep::try_new(
            request.clone(),
            vec![
                (id("a"), Intrusion::Possible("between 1 and 2 m".into())),
                (id("b"), Intrusion::Clear),
            ],
            evidence(false),
        )
        .unwrap();
        assert_eq!(sweep.count(), (0, 1));
        assert_eq!(
            EnvelopeSweep::try_new(
                request.clone(),
                vec![(id("a"), Intrusion::Clear), (id("b"), Intrusion::Clear)],
                evidence(false),
            ),
            Err(AlignmentError::InexactEvidence)
        );
        assert_eq!(
            EnvelopeSweep::try_new(request, vec![(id("a"), Intrusion::Clear)], evidence(true)),
            Err(AlignmentError::InvalidMeasurement)
        );
    }
}
