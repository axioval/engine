//! The elements an alignment is laid out from, in order: each horizontal
//! or vertical element's kind, the stations it runs between, its length
//! and the curvature it states at either end, so expressions can relate
//! consecutive elements (`pairs`).
//!
//! ADR 0004: this seam measures. Whether a clothoid's parameter suits the
//! radius it leads into, or one arc's radius the next one's, is a rule's
//! judgement over these values.
//!
//! An element states its curvature at each end as the design does: a
//! straight end (zero curvature, an unbounded radius), a signed radius
//! (positive turning left in plan, a sag in the vertical), or none the
//! adapter can read, with the reason. Radii, curvatures and a clothoid's
//! parameter `A` (`A^2 = L / |k_end - k_start|`) are derived here from what
//! the element states, never accepted from an adapter, and an end without
//! a read curvature leaves everything derived from it undecided, never a
//! straight.

use axioval_ir::{Evidence, ObjectId};

use crate::alignment::{AlignmentError, AlignmentInterval};

/// Which layout of an alignment the elements belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlignmentLayout {
    /// The plan: lines, arcs and transitions measured by their length.
    Horizontal,
    /// The gradient line over plan distance: constant gradients and
    /// vertical curves.
    Vertical,
}

impl AlignmentLayout {
    /// The name a rule writes for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

/// What an element is, source-neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlignmentElementKind {
    /// A straight line in plan, a constant gradient in the vertical.
    Straight,
    /// A circular arc of constant radius.
    Arc,
    /// A clothoid: curvature linear in its length.
    Clothoid,
    /// A transition of another family (Bloss, cosine, sine, cubic, ...).
    Transition,
    /// A parabolic vertical curve.
    Parabola,
    /// A user-defined or undeclared element.
    Other,
}

impl AlignmentElementKind {
    /// The name a rule reads for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Straight => "straight",
            Self::Arc => "arc",
            Self::Clothoid => "clothoid",
            Self::Transition => "transition",
            Self::Parabola => "parabola",
            Self::Other => "other",
        }
    }
}

/// The curvature an element states at one end.
#[derive(Clone, Debug, PartialEq)]
pub enum ElementEnd {
    /// Straight there: zero curvature, an unbounded radius.
    Straight,
    /// A finite, non-zero signed radius in metres: positive turning left in
    /// plan, a sag (concave upwards) in the vertical.
    Radius(f64),
    /// No curvature the adapter can read there, and why.
    Unread(String),
}

impl ElementEnd {
    /// The signed curvature per metre, an interval holding `1 / radius`
    /// (a point where the division is exact), or why it is undecided.
    pub fn curvature(&self) -> Result<AlignmentInterval, String> {
        match self {
            Self::Straight => AlignmentInterval::exact(0.0).map_err(|error| error.to_string()),
            Self::Radius(radius) => {
                let curvature = 1.0 / radius;
                // `curvature * radius - 1` without rounding: zero exactly
                // when the quotient is.
                let interval = if curvature.mul_add(*radius, -1.0) == 0.0 {
                    AlignmentInterval::exact(curvature)
                } else {
                    AlignmentInterval::try_new(curvature.next_down(), curvature.next_up())
                };
                interval.map_err(|error| error.to_string())
            }
            Self::Unread(why) => Err(why.clone()),
        }
    }

    /// The unsigned radius in metres, or why it is undecided: unbounded
    /// at a straight end, unread where the curvature is.
    pub fn radius(&self) -> Result<f64, String> {
        match self {
            Self::Straight => Err("it is straight there, so its radius is unbounded".into()),
            Self::Radius(radius) => Ok(radius.abs()),
            Self::Unread(why) => Err(why.clone()),
        }
    }
}

/// One element of an alignment's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentElement {
    kind: AlignmentElementKind,
    type_name: String,
    locator: String,
    from: AlignmentInterval,
    to: AlignmentInterval,
    length: f64,
    start: ElementEnd,
    end: ElementEnd,
    gradients: Option<(f64, f64)>,
}

impl AlignmentElement {
    /// An element of `kind`, named `type_name` by its source and found at
    /// `locator`, running from station `from` to station `to` over
    /// `length` metres (plan length, or plan distance for a vertical
    /// element), stating `start` and `end`, and for a vertical element its
    /// gradients at both ends.
    ///
    /// Refuses an empty locator, a negative or non-finite length, a radius
    /// that is zero or not finite, and a gradient that is not finite.
    #[allow(clippy::too_many_arguments)] // Each is one stated property.
    pub fn try_new(
        kind: AlignmentElementKind,
        type_name: impl Into<String>,
        locator: impl Into<String>,
        (from, to): (AlignmentInterval, AlignmentInterval),
        length: f64,
        (start, end): (ElementEnd, ElementEnd),
        gradients: Option<(f64, f64)>,
    ) -> Result<Self, AlignmentError> {
        let locator = locator.into();
        let radius_ok = |end: &ElementEnd| match end {
            ElementEnd::Radius(radius) => radius.is_finite() && *radius != 0.0,
            ElementEnd::Unread(why) => !why.trim().is_empty(),
            ElementEnd::Straight => true,
        };
        let gradients_ok = gradients.is_none_or(|(a, b)| a.is_finite() && b.is_finite());
        let valid = !locator.trim().is_empty()
            && length.is_finite()
            && length >= 0.0
            && radius_ok(&start)
            && radius_ok(&end)
            && gradients_ok;
        if !valid {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            kind,
            type_name: type_name.into(),
            locator,
            from,
            to,
            length,
            start,
            end,
            gradients,
        })
    }

    /// What the element is.
    #[must_use]
    pub fn kind(&self) -> AlignmentElementKind {
        self.kind
    }

    /// The element's type as its source names it (`CLOTHOID`, say).
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Where the element was read, as findings name it.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }

    /// The station it starts at.
    #[must_use]
    pub fn from(&self) -> AlignmentInterval {
        self.from
    }

    /// The station it ends at.
    #[must_use]
    pub fn to(&self) -> AlignmentInterval {
        self.to
    }

    /// Its stated length in metres.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.length
    }

    /// The curvature it states at its start.
    #[must_use]
    pub fn start(&self) -> &ElementEnd {
        &self.start
    }

    /// The curvature it states at its end.
    #[must_use]
    pub fn end(&self) -> &ElementEnd {
        &self.end
    }

    /// The gradients at its start and end; `None` for a horizontal
    /// element.
    #[must_use]
    pub fn gradients(&self) -> Option<(f64, f64)> {
        self.gradients
    }

    /// Its one radius in metres, unsigned, where both ends state the same
    /// (an arc, a parabola's parameter); `None` where they differ (a
    /// transition states no single radius). Undecided, with the reason,
    /// where an end is unread or the element is straight throughout.
    pub fn radius(&self) -> Result<Option<f64>, String> {
        match (&self.start, &self.end) {
            (ElementEnd::Unread(why), _) | (_, ElementEnd::Unread(why)) => Err(why.clone()),
            (ElementEnd::Straight, ElementEnd::Straight) => {
                Err("it is straight, so its radius is unbounded".into())
            }
            #[allow(clippy::float_cmp)] // Stated values: the same or not.
            (ElementEnd::Radius(start), ElementEnd::Radius(end)) if start == end => {
                Ok(Some(start.abs()))
            }
            _ => Ok(None),
        }
    }

    /// A clothoid's parameter `A` in metres, `A^2 = L / |k_end - k_start|`,
    /// an interval holding it; `None` for any other kind. Undecided, with
    /// the reason, where an end is unread or its curvature does not change.
    pub fn clothoid_parameter(&self) -> Result<Option<AlignmentInterval>, String> {
        if self.kind != AlignmentElementKind::Clothoid {
            return Ok(None);
        }
        let (start, end) = (self.start.curvature()?, self.end.curvature()?);
        let low = (end.lower() - start.upper()).next_down();
        let high = (end.upper() - start.lower()).next_up();
        let (least, most) = if low > 0.0 {
            (low, high)
        } else if high < 0.0 {
            (-high, -low)
        } else {
            return Err(
                "its curvature does not change from start to end, so it has no clothoid \
                 parameter"
                    .into(),
            );
        };
        let squared = (
            (self.length / most).next_down().max(0.0),
            (self.length / least).next_up(),
        );
        AlignmentInterval::try_new(
            squared.0.sqrt().next_down().max(0.0),
            squared.1.sqrt().next_up(),
        )
        .map(Some)
        .map_err(|error| error.to_string())
    }
}

/// The elements of one layout of `alignment`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignmentElementsRequest {
    alignment: ObjectId,
    layout: AlignmentLayout,
}

impl AlignmentElementsRequest {
    /// A request for the elements of `layout` of `alignment`.
    #[must_use]
    pub fn new(alignment: ObjectId, layout: AlignmentLayout) -> Self {
        Self { alignment, layout }
    }

    /// The alignment read.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }

    /// The layout read.
    #[must_use]
    pub fn layout(&self) -> AlignmentLayout {
        self.layout
    }
}

/// The elements of one layout of an alignment, in their order along it,
/// with evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentElements {
    request: AlignmentElementsRequest,
    elements: Vec<AlignmentElement>,
    evidence: Evidence,
}

impl AlignmentElements {
    /// The elements answering `request`, in order. The evidence is exact
    /// exactly when every station is a point: what an element states is
    /// read exactly, and only labelling it with stations may round.
    pub fn try_new(
        request: AlignmentElementsRequest,
        elements: Vec<AlignmentElement>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        let exact = elements
            .iter()
            .all(|element| element.from.is_exact() && element.to.is_exact());
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            elements,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &AlignmentElementsRequest {
        &self.request
    }

    /// The elements, in order along the layout.
    #[must_use]
    pub fn elements(&self) -> &[AlignmentElement] {
        &self.elements
    }

    /// Reviewable provenance of the reading.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use axioval_ir::SourceId;

    use super::*;

    fn point(value: f64) -> AlignmentInterval {
        AlignmentInterval::exact(value).unwrap()
    }

    fn element(kind: AlignmentElementKind, start: ElementEnd, end: ElementEnd) -> AlignmentElement {
        AlignmentElement::try_new(
            kind,
            "CLOTHOID",
            "#101",
            (point(100.0), point(200.0)),
            100.0,
            (start, end),
            None,
        )
        .unwrap()
    }

    #[test]
    fn a_clothoid_into_an_arc_has_its_parameter() {
        let clothoid = element(
            AlignmentElementKind::Clothoid,
            ElementEnd::Straight,
            ElementEnd::Radius(500.0),
        );
        // A^2 = 100 * 500.
        let a = clothoid.clothoid_parameter().unwrap().unwrap();
        let expected = 50_000.0_f64.sqrt();
        assert!(a.lower() <= expected && a.upper() >= expected);
        assert!(a.upper() - a.lower() < 1e-9);
        // Between two radii turning the same way: A^2 = L / |1/R2 - 1/R1|.
        let between = element(
            AlignmentElementKind::Clothoid,
            ElementEnd::Radius(-1000.0),
            ElementEnd::Radius(-500.0),
        );
        let a = between.clothoid_parameter().unwrap().unwrap();
        assert!(a.lower() <= 100_000.0_f64.sqrt() && a.upper() >= 100_000.0_f64.sqrt());
        assert_eq!(between.radius(), Ok(None));
        // Another kind states none; an unread end leaves it undecided.
        let arc = element(
            AlignmentElementKind::Arc,
            ElementEnd::Radius(-500.0),
            ElementEnd::Radius(-500.0),
        );
        assert_eq!(arc.clothoid_parameter(), Ok(None));
        assert_eq!(arc.radius(), Ok(Some(500.0)));
        let unread = element(
            AlignmentElementKind::Clothoid,
            ElementEnd::Unread("states neither end curvature".into()),
            ElementEnd::Straight,
        );
        assert!(unread.clothoid_parameter().unwrap_err().contains("neither"));
        let flat = element(
            AlignmentElementKind::Clothoid,
            ElementEnd::Straight,
            ElementEnd::Straight,
        );
        assert!(
            flat.clothoid_parameter()
                .unwrap_err()
                .contains("does not change")
        );
    }

    #[test]
    fn a_straight_end_has_zero_curvature_and_no_radius() {
        let straight = element(
            AlignmentElementKind::Straight,
            ElementEnd::Straight,
            ElementEnd::Straight,
        );
        let curvature = straight.start().curvature().unwrap();
        assert!(curvature.is_exact() && curvature.lower() == 0.0);
        assert!(straight.start().radius().unwrap_err().contains("unbounded"));
        assert!(straight.radius().unwrap_err().contains("unbounded"));
        // A radius whose inverse rounds is an interval; one that does not
        // stays a point.
        let third = ElementEnd::Radius(3.0).curvature().unwrap();
        assert!(!third.is_exact() && third.lower() < 1.0 / 3.0 + 1e-15);
        assert!(ElementEnd::Radius(-512.0).curvature().unwrap().is_exact());
    }

    #[test]
    fn elements_and_lists_are_validated() {
        let ends = || (ElementEnd::Straight, ElementEnd::Straight);
        let stations = (point(0.0), point(1.0));
        let kind = AlignmentElementKind::Straight;
        assert!(AlignmentElement::try_new(kind, "LINE", " ", stations, 1.0, ends(), None).is_err());
        assert!(
            AlignmentElement::try_new(kind, "LINE", "#1", stations, -1.0, ends(), None).is_err()
        );
        assert!(
            AlignmentElement::try_new(
                kind,
                "LINE",
                "#1",
                stations,
                1.0,
                (ElementEnd::Radius(0.0), ElementEnd::Straight),
                None
            )
            .is_err()
        );
        let source = SourceId::new("test", "road").unwrap();
        let request = AlignmentElementsRequest::new(
            ObjectId::new(source.clone(), "axis").unwrap(),
            AlignmentLayout::Horizontal,
        );
        let line =
            AlignmentElement::try_new(kind, "LINE", "#1", stations, 1.0, ends(), None).unwrap();
        let mut evidence = Evidence::exact(source, "layouts of axis");
        assert!(
            AlignmentElements::try_new(request.clone(), vec![line.clone()], evidence.clone())
                .is_ok()
        );
        evidence.exact = false;
        assert_eq!(
            AlignmentElements::try_new(request, vec![line], evidence),
            Err(AlignmentError::InexactEvidence)
        );
    }
}
