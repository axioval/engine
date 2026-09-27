//! Plan orientation shared by the capabilities that judge axes: each
//! footprint's least-area rectangle, the angle between two long axes, and
//! three-valued answers over measured intervals.
//!
//! Only `RectangleOrientation::Unique` rectangles have axes of their own,
//! and only one surely longer side makes a long axis; everything else is
//! undecided, never guessed.

use axioval_engine::{
    DirectionalExtent, MetricDirection, NotEvaluatedReason, PlanRectangle, PlanSpanError,
    PlanSpanServiceHandle, RuleContext, VerticalExtentError, VerticalExtentServiceHandle,
};
use axioval_ir::{ObjectId, QuantityDimension};

use crate::support::{Parameters, Unavailable, invalid};

/// A three-valued answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tri {
    Yes,
    No,
    Maybe,
}

impl Tri {
    /// `Yes` when `sure`, `No` when `excluded`, otherwise `Maybe`.
    pub(crate) fn of(sure: bool, excluded: bool) -> Self {
        if sure {
            Self::Yes
        } else if excluded {
            Self::No
        } else {
            Self::Maybe
        }
    }

    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Maybe,
        }
    }

    pub(crate) fn not(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Maybe => Self::Maybe,
        }
    }

    /// Whether the answer may be yes.
    pub(crate) fn possible(self) -> bool {
        self != Self::No
    }
}

/// How two long axes stand to one another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Alignment {
    Parallel,
    Perpendicular,
    Angled,
}

impl Alignment {
    pub(crate) fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "parallel" => Ok(Self::Parallel),
            "perpendicular" => Ok(Self::Perpendicular),
            "angled" => Ok(Self::Angled),
            other => Err(invalid(format!(
                "orientation `{other}` is unsupported; use `parallel`, `perpendicular` or \
                 `angled`"
            ))),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
            Self::Angled => "angled",
        }
    }

    /// Whether an angle interval in degrees has this alignment, within
    /// `tolerance` degrees of parallel or perpendicular.
    pub(crate) fn holds(self, (lower, upper): (f64, f64), tolerance: f64) -> Tri {
        let square = 90.0 - tolerance;
        match self {
            Self::Parallel => Tri::of(upper <= tolerance, lower > tolerance),
            Self::Perpendicular => Tri::of(lower >= square, upper < square),
            Self::Angled => Tri::of(
                lower > tolerance && upper < square,
                upper <= tolerance || lower >= square,
            ),
        }
    }
}

/// An angle tolerance parameter in degrees, within `[0, 45)` so the three
/// alignments never overlap.
pub(crate) fn angle_tolerance(
    parameters: &Parameters<'_>,
    name: &str,
) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((radians, QuantityDimension::PlaneAngle)) => {
            let degrees = radians.to_degrees();
            if (0.0..45.0).contains(&degrees) {
                Ok(Some(degrees))
            } else {
                Err(invalid(format!("{name} must lie in [0, 45) degrees")))
            }
        }
        Some(_) => Err(invalid(format!("{name} must be a plane angle"))),
    }
}

pub(crate) fn rectangle_service<'a>(
    context: &RuleContext<'a>,
) -> Result<&'a PlanSpanServiceHandle, Unavailable> {
    context.services.get::<PlanSpanServiceHandle>().ok_or((
        NotEvaluatedReason::MissingService,
        "plan-span service is not registered".into(),
    ))
}

/// The least-area rectangle of `object`'s footprint.
pub(crate) fn rectangle(
    service: &PlanSpanServiceHandle,
    object: &ObjectId,
) -> Result<PlanRectangle, Unavailable> {
    service.measure_rectangle(object).map_err(|error| {
        let reason = match error {
            PlanSpanError::UnknownObject(_) | PlanSpanError::Unavailable(_) => {
                NotEvaluatedReason::BackendUnavailable
            }
            PlanSpanError::InvalidMeasurement | PlanSpanError::InexactEvidence => {
                NotEvaluatedReason::InvalidEvidence
            }
        };
        (reason, error.to_string())
    })
}

/// Whether the long axes of `first` and `second` have `alignment` within
/// `tolerance` degrees, and why not when it is undecided.
pub(crate) fn aligned(
    first: &PlanRectangle,
    second: &PlanRectangle,
    alignment: Alignment,
    tolerance: f64,
) -> (Tri, Option<String>) {
    match first.long_axis_angle(second) {
        Ok(angle) => {
            let answer = alignment.holds(angle, tolerance);
            let why = (answer == Tri::Maybe).then(|| {
                format!(
                    "the angle between the long axes of {} and {} ({} degrees) straddles the \
                     {tolerance} degree tolerance",
                    first.object(),
                    second.object(),
                    crate::plan_area::shown(angle.0, angle.1)
                )
            });
            (answer, why)
        }
        Err(reason) => (Tri::Maybe, Some(reason)),
    }
}

/// The lowest and highest positions of `object` along the plan `axis`, each
/// `(lower, upper)` metres.
pub(crate) fn along(
    service: &VerticalExtentServiceHandle,
    object: &ObjectId,
    axis: [f64; 2],
) -> Result<[(f64, f64); 2], Unavailable> {
    let direction = MetricDirection::try_new([axis[0], axis[1], 0.0])
        .map_err(|error| invalid(error.to_string()))?;
    let extent: DirectionalExtent = service
        .measure_directional_extent(object, direction)
        .map_err(|error| extent_unavailable(&error))?;
    let (low, high) = (extent.lower(), extent.upper());
    Ok([
        (low.lower_metres(), low.upper_metres()),
        (high.lower_metres(), high.upper_metres()),
    ])
}

pub(crate) fn extent_unavailable(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Alignment, Tri};

    #[test]
    fn alignments_are_three_valued_and_never_overlap() {
        let tolerance = 5.0;
        for (angle, parallel, perpendicular, angled) in [
            ((0.0, 0.0), Tri::Yes, Tri::No, Tri::No),
            ((4.0, 6.0), Tri::Maybe, Tri::No, Tri::Maybe),
            ((30.0, 30.0), Tri::No, Tri::No, Tri::Yes),
            ((84.0, 86.0), Tri::No, Tri::Maybe, Tri::Maybe),
            ((90.0, 90.0), Tri::No, Tri::Yes, Tri::No),
            ((5.0, 5.0), Tri::Yes, Tri::No, Tri::No),
        ] {
            assert_eq!(Alignment::Parallel.holds(angle, tolerance), parallel);
            assert_eq!(
                Alignment::Perpendicular.holds(angle, tolerance),
                perpendicular
            );
            assert_eq!(Alignment::Angled.holds(angle, tolerance), angled);
        }
    }

    #[test]
    fn no_short_circuits_and_yes_needs_both() {
        assert_eq!(Tri::Yes.and(Tri::Yes), Tri::Yes);
        assert_eq!(Tri::Yes.and(Tri::Maybe), Tri::Maybe);
        assert_eq!(Tri::Maybe.and(Tri::No), Tri::No);
        assert!(Tri::Maybe.possible() && !Tri::No.possible());
    }
}
