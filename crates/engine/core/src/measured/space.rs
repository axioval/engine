//! Space measurements as values: each aspect `space-validation` judges,
//! measured by the same space-service requests, so a measured value and a
//! comparison reproduce its verdicts.
//!
//! Elements a request names are the project's objects of the source kinds
//! `elements` lists; without it the service's own default applies. A
//! storey's unallocated floor is read from the regions the service reports
//! for it.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, MEASURED_SET, ObjectId, QuantityDimension};

use crate::expression::Interval;

use super::{Answer, Measures};
use crate::properties::PropertyResolutionError;
use crate::space::{
    BoundaryRequest, Cap, CapRequest, OverlapRequest, SpaceError, SpaceService, UnallocatedRegion,
};

/// The names measured here.
pub(super) const NAMES: &[&str] = &[
    "boundary_gap",
    "cap_coverage",
    "duplicate_count",
    "intersection_count",
    "largest_unallocated_region",
    "support_count",
    "unallocated_share",
];

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn count(value: usize, locator: String, exact: bool) -> Answer {
    let value = value as f64;
    Answer::Value(value, value, None, locator, exact)
}

/// The longest of the gaps at least `at_least` long, or their total, summed
/// outward so that it holds the exact sum.
fn gap_length(
    gaps: &[crate::space::BoundaryGap],
    at_least: f64,
    longest: bool,
) -> Result<Interval, PropertyResolutionError> {
    let mut counted = gaps
        .iter()
        .map(crate::space::BoundaryGap::length_metres)
        .filter(|gap| *gap >= at_least);
    if longest {
        let longest = counted.fold(0.0, f64::max);
        return Ok(super::span(longest, longest));
    }
    counted.try_fold(super::span(0.0, 0.0), |total, gap| {
        total
            .plus(super::span(gap, gap))
            .map_err(|_| PropertyResolutionError::InvalidValue)
    })
}

/// The share `part / whole` of two areas the service states, rounded
/// outward and kept in `[0, 1]`.
fn share(part: Interval, whole: f64) -> Result<(f64, f64), PropertyResolutionError> {
    let (lower, upper) = super::bounds(part.divided_by(super::span(whole, whole)))?;
    Ok((lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0)))
}

impl Measures {
    /// Why the space service refused `name` of `object`, as
    /// `space-validation` reads the refusal: an unavailable or unmeasured
    /// aspect is incomplete evidence, an inexact or incoherent answer
    /// conflicting evidence.
    pub(super) fn space_refused(
        name: &str,
        object: &ObjectId,
        error: &SpaceError,
    ) -> PropertyResolutionError {
        let message = format!("`{MEASURED_SET}` value `{name}` of {object}: {error}");
        match error {
            SpaceError::Unavailable | SpaceError::Unmeasured(_) => {
                PropertyResolutionError::Incomplete(message)
            }
            SpaceError::InexactEvidence | SpaceError::InvalidQuantity => {
                PropertyResolutionError::Conflicting(message)
            }
        }
    }

    /// The elements `elements` names, if it is stated.
    fn elements(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Option<Vec<ObjectId>>, PropertyResolutionError> {
        if call.argument("elements").is_none() {
            return Ok(None);
        }
        self.of_kinds(call, "elements", object).map(Some)
    }

    /// A space measurement of `object`, by `call`'s name.
    pub(super) fn space(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: SpaceError| Self::space_refused(name, object, &error);
        let service: &dyn SpaceService = self
            .spaces
            .as_ref()
            .ok_or_else(|| Self::missing(name, "space"))?
            .get();
        // Every aspect is exact as the service's evidence is.
        let Evidence { locator, exact, .. } = service.evidence();
        let elements = self.elements(call, object)?;
        match name {
            "duplicate_count" => {
                let duplicates = service.measure_duplicates(object).map_err(unavailable)?;
                Ok(count(duplicates.len(), locator, exact))
            }
            "boundary_gap" => {
                let mut request = BoundaryRequest::new();
                if let Some(elements) = elements {
                    request = request.with_elements(elements);
                }
                let gaps = service
                    .measure_boundary_gaps(object, &request)
                    .map_err(unavailable)?;
                let value = gap_length(
                    &gaps,
                    length(call, "at_least").unwrap_or(0.0),
                    call.choice("measure") == Some("longest"),
                )?;
                Ok(Answer::Value(
                    value.lower,
                    value.upper,
                    Some(QuantityDimension::Length),
                    locator,
                    exact,
                ))
            }
            "intersection_count" => {
                let mut request = OverlapRequest::new();
                if let Some(elements) = elements {
                    request = request.with_elements(elements);
                }
                let tolerance = length(call, "tolerance").unwrap_or(0.0);
                let overlaps = service
                    .measure_overlaps(object, &request)
                    .map_err(unavailable)?;
                let intersecting = overlaps
                    .iter()
                    .filter(|overlap| overlap.intersects(tolerance))
                    .count();
                Ok(count(intersecting, locator, exact))
            }
            "cap_coverage" => {
                let cap = if call.choice("cap") == Some("bottom") {
                    Cap::Bottom
                } else {
                    Cap::Top
                };
                let mut request = CapRequest::new(cap);
                if let Some(elements) = elements {
                    if elements.is_empty() {
                        return Ok(Answer::Absent(format!(
                            "no element of the kinds named can form the cap: {locator}"
                        )));
                    }
                    request = request.with_elements(elements);
                }
                let coverage = service
                    .measure_cap_coverage(object, &request)
                    .map_err(unavailable)?;
                let covered = coverage.covered_area_square_metres();
                let (lower, upper) = share(
                    super::span(covered, covered),
                    coverage.whole_area_square_metres(),
                )?;
                Ok(Answer::Value(lower, upper, None, locator, exact))
            }
            "support_count" => {
                let counts = service.measure_support_counts().map_err(unavailable)?;
                Ok(count(
                    if call.choice("of") == Some("roofs") {
                        counts.roofs()
                    } else {
                        counts.slabs()
                    },
                    locator,
                    exact,
                ))
            }
            _ => unallocated(service, name, object, (locator, exact)),
        }
    }
}

/// A storey's unallocated floor: its largest region, or its share of the
/// storey's gross floor area.
fn unallocated(
    service: &dyn SpaceService,
    name: &str,
    object: &ObjectId,
    (locator, exact): (String, bool),
) -> Result<Answer, PropertyResolutionError> {
    let regions = service
        .measure_unallocated_regions()
        .map_err(|error| Measures::space_refused(name, object, &error))?;
    let regions: Vec<_> = regions
        .iter()
        .filter(|region| region.storey() == object)
        .collect();
    if name == "largest_unallocated_region" {
        let largest = regions
            .iter()
            .map(|region| region.area_square_metres())
            .fold(0.0, f64::max);
        return Ok(Answer::Value(
            largest,
            largest,
            Some(QuantityDimension::Area),
            locator,
            exact,
        ));
    }
    if regions.is_empty() {
        return Ok(Answer::Value(0.0, 0.0, None, locator, exact));
    }
    let share = UnallocatedRegion::storey_share(&regions).ok_or_else(|| {
        PropertyResolutionError::Incomplete(format!(
            "`{MEASURED_SET}` value `{name}` of {object}: the storey's gross floor area is \
             not measured, so its unallocated share is undefined"
        ))
    })?;
    let (lower, upper) = share.share();
    Ok(Answer::Value(
        lower.clamp(0.0, 1.0),
        upper.clamp(0.0, 1.0),
        None,
        locator,
        exact,
    ))
}
