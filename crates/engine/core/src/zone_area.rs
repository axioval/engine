//! Plan containment of footprints in a stated zone (#327): how much of the
//! union of some objects' footprints lies inside a plan polygon supplied
//! beside the model (a parcel, a zoning area), and how much outside it.
//!
//! ADR 0004: this seam measures. Whether a building keeps to its building
//! limit, or how its footprint compares with its parcel's area, is a rule's
//! judgement over these areas.
//!
//! The zone is stated, not measured, so its boundary is known only within
//! the accuracy its statement gives: the true zone holds every point of the
//! stated one farther than that accuracy from its boundary, and lies
//! within the stated one grown by it. The areas inside and outside are
//! intervals sure to hold the exact ones under every boundary that
//! accuracy allows, and the footprints' own tessellation widens them too.
//! A footprint straddling the boundary within its accuracy is therefore
//! never surely inside.

use axioval_ir::{Evidence, ObjectId};

use crate::PlanAreaError;

/// A polygon in plan, in the model's coordinates (metres): its outer ring
/// anticlockwise and each hole clockwise, every ring without its closing
/// point repeated.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanPolygon {
    outer: Vec<[f64; 2]>,
    holes: Vec<Vec<[f64; 2]>>,
}

/// Twice the signed area of a ring, positive anticlockwise.
pub(crate) fn signed_double_area(ring: &[[f64; 2]]) -> f64 {
    let Some(&[x0, y0]) = ring.first() else {
        return 0.0;
    };
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|([ax, ay], [bx, by])| (ax - x0).mul_add(by - y0, -((bx - x0) * (ay - y0))))
        .sum()
}

/// A ring of at least three finite points, no two neighbours equal and
/// enclosing an area, turned the way `anticlockwise` asks.
fn ring(mut points: Vec<[f64; 2]>, anticlockwise: bool) -> Result<Vec<[f64; 2]>, PlanAreaError> {
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    let finite = points.iter().flatten().all(|value| value.is_finite());
    #[allow(clippy::float_cmp)] // Equal as stated: a repeated point.
    let repeated = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .any(|(a, b)| a == b);
    if points.len() < 3 || !finite || repeated {
        return Err(PlanAreaError::InvalidMeasurement);
    }
    let area = signed_double_area(&points);
    if area == 0.0 || !area.is_finite() {
        return Err(PlanAreaError::InvalidMeasurement);
    }
    if (area > 0.0) != anticlockwise {
        points.reverse();
    }
    Ok(points)
}

impl PlanPolygon {
    /// A polygon of `outer` and `holes`, each a ring of at least three
    /// finite points enclosing an area; a ring's closing point, if
    /// repeated, is dropped, and the rings are turned outer anticlockwise,
    /// holes clockwise. Whether the rings cross is the measuring service's
    /// to refuse.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::InvalidMeasurement`] for a ring of fewer than three
    /// points, with a point not finite or repeated beside itself, or
    /// enclosing no area.
    pub fn try_new(outer: Vec<[f64; 2]>, holes: Vec<Vec<[f64; 2]>>) -> Result<Self, PlanAreaError> {
        Ok(Self {
            outer: ring(outer, true)?,
            holes: holes
                .into_iter()
                .map(|hole| ring(hole, false))
                .collect::<Result<_, _>>()?,
        })
    }

    /// The outer ring, anticlockwise.
    #[must_use]
    pub fn outer(&self) -> &[[f64; 2]] {
        &self.outer
    }

    /// The holes, each clockwise.
    #[must_use]
    pub fn holes(&self) -> &[Vec<[f64; 2]>] {
        &self.holes
    }

    /// Every ring, the outer one first.
    pub fn rings(&self) -> impl Iterator<Item = &[[f64; 2]]> {
        std::iter::once(self.outer.as_slice()).chain(self.holes.iter().map(Vec::as_slice))
    }
}

/// How much of the union of some objects' footprints lies inside a stated
/// zone: the objects (sorted, without repeats, of one source), the zone's
/// polygons in the model's coordinates and the accuracy of its boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneAreaRequest {
    objects: Vec<ObjectId>,
    zone: Vec<PlanPolygon>,
    accuracy_metres: f64,
}

impl ZoneAreaRequest {
    /// The union of the footprints of `objects` measured against the zone
    /// `zone` (polygons that do not overlap), whose boundary lies within
    /// `accuracy_metres` of the true one.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for no object, objects of several
    /// sources, no polygon, or an accuracy that is not a finite length.
    pub fn try_new(
        mut objects: Vec<ObjectId>,
        zone: Vec<PlanPolygon>,
        accuracy_metres: f64,
    ) -> Result<Self, PlanAreaError> {
        objects.sort();
        objects.dedup();
        let Some(first) = objects.first() else {
            return Err(PlanAreaError::Unavailable(
                "no object is measured against the zone".into(),
            ));
        };
        if objects.iter().any(|object| object.source != first.source) {
            return Err(PlanAreaError::Unavailable(
                "the objects lie in several sources, whose footprints are not measured together"
                    .into(),
            ));
        }
        if zone.is_empty() {
            return Err(PlanAreaError::Unavailable("the zone has no polygon".into()));
        }
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "an accuracy of {accuracy_metres} m is not a non-negative length"
            )));
        }
        Ok(Self {
            objects,
            zone,
            accuracy_metres,
        })
    }

    /// The objects whose footprints are united, sorted.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    /// The zone's polygons, in the model's coordinates.
    #[must_use]
    pub fn zone(&self) -> &[PlanPolygon] {
        &self.zone
    }

    /// How far the zone's true boundary may lie from the stated one, in
    /// metres.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }
}

/// The footprint of a request's objects and its parts inside and outside
/// the zone, in square metres, each an interval sure to hold the exact
/// area under every boundary the zone's accuracy allows. The evidence is
/// exact exactly when every interval is a point.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneArea {
    objects: Vec<ObjectId>,
    footprint: (f64, f64),
    inside: (f64, f64),
    outside: (f64, f64),
    evidence: Evidence,
}

impl ZoneArea {
    /// The areas of `objects`' footprint, wholly, inside and outside.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::InvalidMeasurement`] for a bound not finite,
    /// negative or reversed, or a part surely larger than the whole;
    /// [`PlanAreaError::InexactEvidence`] for evidence whose exactness is
    /// not that of its intervals, without a locator, or from another
    /// source than the objects'.
    pub fn try_new(
        objects: Vec<ObjectId>,
        footprint: (f64, f64),
        inside: (f64, f64),
        outside: (f64, f64),
        evidence: Evidence,
    ) -> Result<Self, PlanAreaError> {
        let valid = |(lower, upper): (f64, f64)| {
            lower.is_finite() && upper.is_finite() && lower >= 0.0 && lower <= upper
        };
        if !valid(footprint)
            || !valid(inside)
            || !valid(outside)
            || inside.0 > footprint.1
            || outside.0 > footprint.1
        {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = [footprint, inside, outside]
            .iter()
            .all(|(lower, upper)| lower == upper);
        let foreign = objects
            .first()
            .is_none_or(|object| object.source != evidence.source);
        if evidence.exact != exact || evidence.locator.trim().is_empty() || foreign {
            return Err(PlanAreaError::InexactEvidence);
        }
        Ok(Self {
            objects,
            footprint,
            inside,
            outside,
            evidence,
        })
    }

    /// The objects measured, sorted.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    /// The area of the union of their footprints.
    #[must_use]
    pub fn footprint_square_metres(&self) -> (f64, f64) {
        self.footprint
    }

    /// The area of it inside the zone.
    #[must_use]
    pub fn inside_square_metres(&self) -> (f64, f64) {
        self.inside
    }

    /// The area of it outside the zone.
    #[must_use]
    pub fn outside_square_metres(&self) -> (f64, f64) {
        self.outside
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn object(source: &str, local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", source).unwrap(), local).unwrap()
    }

    fn square() -> Vec<[f64; 2]> {
        vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0], [0.0, 0.0]]
    }

    #[test]
    fn rings_are_closed_turned_and_refused_when_degenerate() {
        let polygon = PlanPolygon::try_new(
            square(),
            vec![vec![[0.5, 0.5], [1.5, 0.5], [1.5, 1.5], [0.5, 1.5]]],
        )
        .unwrap();
        assert_eq!(polygon.outer().len(), 4, "the closing point is dropped");
        assert!(signed_double_area(polygon.outer()) > 0.0);
        assert!(signed_double_area(&polygon.holes()[0]) < 0.0);
        for outer in [
            vec![[0.0, 0.0], [1.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            vec![[0.0, 0.0], [f64::NAN, 0.0], [0.0, 1.0]],
        ] {
            assert_eq!(
                PlanPolygon::try_new(outer, Vec::new()),
                Err(PlanAreaError::InvalidMeasurement)
            );
        }
    }

    #[test]
    fn a_request_names_objects_of_one_source() {
        let zone = vec![PlanPolygon::try_new(square(), Vec::new()).unwrap()];
        let request = ZoneAreaRequest::try_new(
            vec![object("a", "2"), object("a", "1"), object("a", "2")],
            zone.clone(),
            0.0,
        )
        .unwrap();
        assert_eq!(request.objects(), [object("a", "1"), object("a", "2")]);
        assert!(
            ZoneAreaRequest::try_new(vec![object("a", "1"), object("b", "1")], zone.clone(), 0.0)
                .is_err()
        );
        assert!(ZoneAreaRequest::try_new(Vec::new(), zone.clone(), 0.0).is_err());
        assert!(ZoneAreaRequest::try_new(vec![object("a", "1")], Vec::new(), 0.0).is_err());
        assert!(ZoneAreaRequest::try_new(vec![object("a", "1")], zone, -0.1).is_err());
    }

    #[test]
    fn an_answer_is_exact_exactly_when_every_interval_is_a_point() {
        let objects = vec![object("a", "1")];
        let exact = Evidence::exact(SourceId::new("cad", "a").unwrap(), "zone-area:a");
        assert!(
            ZoneArea::try_new(
                objects.clone(),
                (4.0, 4.0),
                (3.0, 3.0),
                (1.0, 1.0),
                exact.clone()
            )
            .is_ok()
        );
        assert_eq!(
            ZoneArea::try_new(
                objects.clone(),
                (4.0, 4.0),
                (2.9, 3.0),
                (1.0, 1.1),
                exact.clone()
            ),
            Err(PlanAreaError::InexactEvidence)
        );
        assert_eq!(
            ZoneArea::try_new(objects, (4.0, 4.0), (5.0, 5.0), (0.0, 0.0), exact),
            Err(PlanAreaError::InvalidMeasurement)
        );
    }
}
