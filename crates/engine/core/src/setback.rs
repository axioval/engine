//! Setback areas in front of external walls (#326): the ground a wall's
//! outward faces sweep when moved straight out, in plan, by a depth.
//!
//! ADR 0004: this seam measures. How deep a setback must be (a factor of
//! the wall's height, at least a minimum) is a rule's statement, and
//! whether a setback may leave its parcel or overlap another is a rule's
//! judgement over these areas.
//!
//! A wall's setback is its footprint swept along each outward plan
//! direction by the depth, less the footprint itself: the strip in front
//! of every footprint edge facing that direction, and nothing behind the
//! wall. The depth is an interval (a height above a terrain known to an
//! accuracy), and a direction may be only possibly outward (a face partly
//! covered), so the setback is bracketed: the least sweeps the sure
//! directions by the least depth, the most every direction by the most
//! depth. Every area is an interval holding the area of every setback
//! between the two, under every boundary a zone's stated accuracy allows.
//! A setback straddling a zone's boundary within its accuracy is therefore
//! never surely inside.

use axioval_ir::{Evidence, ObjectId};

use crate::PlanAreaError;
use crate::zone_area::PlanPolygon;

/// One plan direction a wall looks out along: a unit vector in the model's
/// plan, and whether the faces looking along it surely look outside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetbackDirection {
    direction: [f64; 2],
    sure: bool,
}

impl SetbackDirection {
    /// The plan direction `direction` (any length but zero), surely
    /// outward when `sure`, else possibly.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for a direction not finite or of no
    /// length.
    pub fn try_new(direction: [f64; 2], sure: bool) -> Result<Self, PlanAreaError> {
        let length = direction[0].hypot(direction[1]);
        if !length.is_finite() || length == 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "({}, {}) is no plan direction",
                direction[0], direction[1]
            )));
        }
        Ok(Self {
            direction: direction.map(|value| value / length),
            sure,
        })
    }

    /// The unit plan direction.
    #[must_use]
    pub fn direction(&self) -> [f64; 2] {
        self.direction
    }

    /// Whether the faces looking along it surely look outside.
    #[must_use]
    pub fn is_sure(&self) -> bool {
        self.sure
    }
}

/// A wall's setback as asked for: the wall, the plan directions it looks
/// out along (sorted, each once) and the depth in metres, an interval.
#[derive(Clone, Debug, PartialEq)]
pub struct SetbackStrip {
    object: ObjectId,
    directions: Vec<SetbackDirection>,
    depth: (f64, f64),
}

impl SetbackStrip {
    /// The setback of `object` along `directions`, `depth_metres` deep.
    /// A direction stated twice is kept once, surely outward where either
    /// statement is.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for no direction, or a depth not
    /// finite, negative or reversed.
    pub fn try_new(
        object: ObjectId,
        mut directions: Vec<SetbackDirection>,
        depth_metres: (f64, f64),
    ) -> Result<Self, PlanAreaError> {
        if directions.is_empty() {
            return Err(PlanAreaError::Unavailable(format!(
                "{object} looks out along no plan direction, so it has no setback"
            )));
        }
        let (lower, upper) = depth_metres;
        if !(lower.is_finite() && upper.is_finite() && lower >= 0.0 && lower <= upper) {
            return Err(PlanAreaError::Unavailable(format!(
                "a depth of {lower}..{upper} m is not a non-negative length"
            )));
        }
        let key = |direction: &SetbackDirection| {
            (
                direction.direction[0].to_bits(),
                direction.direction[1].to_bits(),
            )
        };
        directions.sort_by(|a, b| key(a).cmp(&key(b)).then(b.sure.cmp(&a.sure)));
        directions.dedup_by(|later, kept| {
            let same = key(later) == key(kept);
            kept.sure |= same && later.sure;
            same
        });
        Ok(Self {
            object,
            directions,
            depth: depth_metres,
        })
    }

    /// The wall.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The plan directions it looks out along, sorted.
    #[must_use]
    pub fn directions(&self) -> &[SetbackDirection] {
        &self.directions
    }

    /// The depth, in metres: the least and the most.
    #[must_use]
    pub fn depth_metres(&self) -> (f64, f64) {
        self.depth
    }
}

/// Another wall's setback a request's is overlapped with: surely one of
/// the walls compared, or only possibly (a selection left undecided).
#[derive(Clone, Debug, PartialEq)]
pub struct SetbackOther {
    strip: SetbackStrip,
    sure: bool,
}

impl SetbackOther {
    /// `strip`, surely compared when `sure`.
    #[must_use]
    pub fn new(strip: SetbackStrip, sure: bool) -> Self {
        Self { strip, sure }
    }

    /// The other wall's setback.
    #[must_use]
    pub fn strip(&self) -> &SetbackStrip {
        &self.strip
    }

    /// Whether it is surely compared.
    #[must_use]
    pub fn is_sure(&self) -> bool {
        self.sure
    }
}

/// What is measured of one wall's setback: its area, and with a zone the
/// part outside it, with other walls the part their setbacks overlap.
#[derive(Clone, Debug, PartialEq)]
pub struct SetbackRequest {
    strip: SetbackStrip,
    zone: Option<(Vec<PlanPolygon>, f64)>,
    others: Vec<SetbackOther>,
}

impl SetbackRequest {
    /// The setback `strip`, measured alone.
    #[must_use]
    pub fn new(strip: SetbackStrip) -> Self {
        Self {
            strip,
            zone: None,
            others: Vec::new(),
        }
    }

    /// The same request measuring the part outside the zone `polygons`
    /// (in the wall's model coordinates, not overlapping), whose boundary
    /// lies within `accuracy_metres` of the true one.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for no polygon or an accuracy that
    /// is not a finite non-negative length.
    pub fn with_zone(
        mut self,
        polygons: Vec<PlanPolygon>,
        accuracy_metres: f64,
    ) -> Result<Self, PlanAreaError> {
        if polygons.is_empty() {
            return Err(PlanAreaError::Unavailable("the zone has no polygon".into()));
        }
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "an accuracy of {accuracy_metres} m is not a non-negative length"
            )));
        }
        self.zone = Some((polygons, accuracy_metres));
        Ok(self)
    }

    /// The same request measuring the part the setbacks of `others`
    /// overlap, sorted by wall, each wall once (surely compared where any
    /// statement of it is).
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for the wall among its own others,
    /// or one other wall stated with two setbacks.
    pub fn with_others(mut self, mut others: Vec<SetbackOther>) -> Result<Self, PlanAreaError> {
        if others
            .iter()
            .any(|other| other.strip.object == self.strip.object)
        {
            return Err(PlanAreaError::Unavailable(format!(
                "{} cannot overlap its own setback",
                self.strip.object
            )));
        }
        others.sort_by(|a, b| a.strip.object.cmp(&b.strip.object));
        for pair in others.windows(2) {
            if pair[0].strip.object == pair[1].strip.object && pair[0].strip != pair[1].strip {
                return Err(PlanAreaError::Unavailable(format!(
                    "{} is stated with two setbacks",
                    pair[0].strip.object
                )));
            }
        }
        others.dedup_by(|later, kept| {
            let same = later.strip.object == kept.strip.object;
            kept.sure |= same && later.sure;
            same
        });
        self.others = others;
        Ok(self)
    }

    /// The wall's setback.
    #[must_use]
    pub fn strip(&self) -> &SetbackStrip {
        &self.strip
    }

    /// The zone's polygons and the accuracy of its boundary, if asked.
    #[must_use]
    pub fn zone(&self) -> Option<(&[PlanPolygon], f64)> {
        self.zone
            .as_ref()
            .map(|(polygons, accuracy)| (polygons.as_slice(), *accuracy))
    }

    /// The other walls' setbacks, sorted by wall.
    #[must_use]
    pub fn others(&self) -> &[SetbackOther] {
        &self.others
    }
}

/// A wall's measured setback: its polygons at the least and the most
/// (each the union of the swept strips, in the model's plan coordinates),
/// its area, and as asked the part outside the zone and the part other
/// setbacks overlap, each an interval in square metres, with the other
/// walls whose setbacks may overlap it. The evidence is exact exactly when
/// every interval is a point.
#[derive(Clone, Debug, PartialEq)]
pub struct SetbackArea {
    object: ObjectId,
    least: Vec<PlanPolygon>,
    most: Vec<PlanPolygon>,
    area: (f64, f64),
    outside: Option<(f64, f64)>,
    overlap: Option<(f64, f64)>,
    overlapping: Vec<ObjectId>,
    evidence: Evidence,
}

/// A wall's setback polygons, at the least and at the most.
pub type SetbackPolygons = (Vec<PlanPolygon>, Vec<PlanPolygon>);

impl SetbackArea {
    /// The setback of `object`: `polygons` at the least and the most, its
    /// `area`, the part `outside` a zone and the part `overlap`ped by
    /// others (each where asked), and the walls whose setbacks may overlap
    /// it (`overlapping`, sorted, each once).
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::InvalidMeasurement`] for a bound not finite,
    /// negative or reversed, or a part surely larger than the whole;
    /// [`PlanAreaError::InexactEvidence`] for evidence whose exactness is
    /// not that of its intervals, without a locator, or from another
    /// source than the wall's.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        object: ObjectId,
        (least, most): SetbackPolygons,
        area: (f64, f64),
        outside: Option<(f64, f64)>,
        overlap: Option<(f64, f64)>,
        mut overlapping: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, PlanAreaError> {
        let valid = |(lower, upper): (f64, f64)| {
            lower.is_finite() && upper.is_finite() && lower >= 0.0 && lower <= upper
        };
        let part = |bounds: Option<(f64, f64)>| {
            bounds.is_none_or(|bounds| valid(bounds) && bounds.0 <= area.1)
        };
        if !valid(area) || !part(outside) || !part(overlap) {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = [Some(area), outside, overlap]
            .iter()
            .flatten()
            .all(|(lower, upper)| lower == upper);
        if evidence.exact != exact
            || evidence.locator.trim().is_empty()
            || evidence.source != object.source
        {
            return Err(PlanAreaError::InexactEvidence);
        }
        overlapping.sort();
        overlapping.dedup();
        Ok(Self {
            object,
            least,
            most,
            area,
            outside,
            overlap,
            overlapping,
            evidence,
        })
    }

    /// The wall.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The setback at the least: the sure directions swept by the least
    /// depth. Empty where that depth is zero.
    #[must_use]
    pub fn least_polygons(&self) -> &[PlanPolygon] {
        &self.least
    }

    /// The setback at the most: every direction swept by the most depth.
    #[must_use]
    pub fn most_polygons(&self) -> &[PlanPolygon] {
        &self.most
    }

    /// The setback's area.
    #[must_use]
    pub fn area_square_metres(&self) -> (f64, f64) {
        self.area
    }

    /// The part of it outside the zone, where a zone was asked.
    #[must_use]
    pub fn outside_square_metres(&self) -> Option<(f64, f64)> {
        self.outside
    }

    /// The part of it the other walls' setbacks overlap, where others were
    /// asked.
    #[must_use]
    pub fn overlap_square_metres(&self) -> Option<(f64, f64)> {
        self.overlap
    }

    /// The other walls whose setbacks may overlap it, sorted.
    #[must_use]
    pub fn overlapping(&self) -> &[ObjectId] {
        &self.overlapping
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Whether `answer` answers `request`: the same wall, the zone's part
/// exactly where a zone was asked, the overlap exactly where others were,
/// and only asked walls overlapping it.
pub(crate) fn answers(request: &SetbackRequest, answer: &SetbackArea) -> bool {
    answer.object() == request.strip().object()
        && answer.outside.is_some() == request.zone.is_some()
        && answer.overlap.is_some() != request.others.is_empty()
        && answer.overlapping.iter().all(|wall| {
            request
                .others
                .iter()
                .any(|other| other.strip.object() == wall)
        })
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Directions compared as exactly normalised.
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn wall(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "a").unwrap(), local).unwrap()
    }

    fn south(sure: bool) -> SetbackDirection {
        SetbackDirection::try_new([0.0, -2.0], sure).unwrap()
    }

    #[test]
    fn directions_are_unit_sorted_and_kept_once() {
        assert_eq!(south(true).direction(), [0.0, -1.0]);
        assert!(SetbackDirection::try_new([0.0, 0.0], true).is_err());
        assert!(SetbackDirection::try_new([f64::NAN, 1.0], true).is_err());
        let strip = SetbackStrip::try_new(
            wall("1"),
            vec![
                south(false),
                SetbackDirection::try_new([1.0, 0.0], false).unwrap(),
                south(true),
            ],
            (3.0, 3.2),
        )
        .unwrap();
        assert_eq!(strip.directions().len(), 2);
        assert!(
            strip
                .directions()
                .iter()
                .any(|direction| direction.direction() == [0.0, -1.0] && direction.is_sure()),
            "a direction stated sure once is sure"
        );
        for depth in [(-0.1, 1.0), (2.0, 1.0), (0.0, f64::INFINITY)] {
            assert!(SetbackStrip::try_new(wall("1"), vec![south(true)], depth).is_err());
        }
        assert!(SetbackStrip::try_new(wall("1"), Vec::new(), (1.0, 1.0)).is_err());
    }

    #[test]
    fn a_request_never_overlaps_a_wall_with_itself() {
        let strip = |local: &str| {
            SetbackStrip::try_new(wall(local), vec![south(true)], (1.0, 1.0)).unwrap()
        };
        let request = SetbackRequest::new(strip("1"));
        assert!(
            request
                .clone()
                .with_others(vec![SetbackOther::new(strip("1"), true)])
                .is_err()
        );
        let request = request
            .with_others(vec![
                SetbackOther::new(strip("3"), false),
                SetbackOther::new(strip("2"), false),
                SetbackOther::new(strip("3"), true),
            ])
            .unwrap();
        let others: Vec<(&ObjectId, bool)> = request
            .others()
            .iter()
            .map(|other| (other.strip().object(), other.is_sure()))
            .collect();
        assert_eq!(others, [(&wall("2"), false), (&wall("3"), true)]);
        assert!(request.clone().with_zone(Vec::new(), 0.0).is_err());
    }

    #[test]
    fn an_answer_is_exact_exactly_when_every_interval_is_a_point() {
        let exact = Evidence::exact(SourceId::new("cad", "a").unwrap(), "setback:a");
        let answer = |area, outside, evidence: &Evidence| {
            SetbackArea::try_new(
                wall("1"),
                (Vec::new(), Vec::new()),
                area,
                outside,
                None,
                Vec::new(),
                evidence.clone(),
            )
        };
        assert!(answer((30.0, 30.0), Some((3.0, 3.0)), &exact).is_ok());
        assert_eq!(
            answer((30.0, 32.0), Some((3.0, 3.0)), &exact),
            Err(PlanAreaError::InexactEvidence)
        );
        assert_eq!(
            answer((30.0, 30.0), Some((31.0, 31.0)), &exact),
            Err(PlanAreaError::InvalidMeasurement)
        );
        let elsewhere = Evidence::exact(SourceId::new("cad", "b").unwrap(), "setback:b");
        assert_eq!(
            answer((30.0, 30.0), None, &elsewhere),
            Err(PlanAreaError::InexactEvidence)
        );
    }
}
