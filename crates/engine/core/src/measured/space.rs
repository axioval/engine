//! Space measurements as values: each aspect `space-validation` judges,
//! measured by the same space-service requests, so a measured value and a
//! comparison reproduce its verdicts.
//!
//! Elements a request names are the project's objects of the source kinds
//! `elements` lists; without it the service's own default applies. A
//! storey's unallocated floor is read from the regions the service reports
//! for it.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::properties::PropertyResolutionError;
use crate::space::{BoundaryRequest, Cap, CapRequest, Containment, OverlapRequest, SpaceService};

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

/// The area below which a partial overlap is no intersection, as
/// `space-validation` counts it.
const OVERLAP_AREA_EPSILON_M2: f64 = 1.0e-8;

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn count(value: usize, locator: String) -> Answer {
    let value = value as f64;
    Answer::Number(value, value, locator)
}

impl Measures {
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
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let service: &dyn SpaceService = self
            .spaces
            .as_ref()
            .ok_or_else(|| Self::missing(name, "space"))?
            .get();
        let locator = service.evidence().locator;
        let elements = self.elements(call, object)?;
        match name {
            "duplicate_count" => {
                let duplicates = service
                    .measure_duplicates(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok(count(duplicates.len(), locator))
            }
            "boundary_gap" => {
                let mut request = BoundaryRequest::new();
                if let Some(elements) = elements {
                    request = request.with_elements(elements);
                }
                let gaps = service
                    .measure_boundary_gaps(object, &request)
                    .map_err(|error| unavailable(error.to_string()))?;
                let longest = call.choice("measure") == Some("longest");
                let at_least = length(call, "at_least").unwrap_or(0.0);
                let counted = gaps
                    .iter()
                    .map(crate::space::BoundaryGap::length_metres)
                    .filter(|gap| *gap >= at_least);
                let value = if longest {
                    counted.fold(0.0, f64::max)
                } else {
                    counted.sum()
                };
                Ok(Answer::Value(
                    value,
                    value,
                    QuantityDimension::Length,
                    locator,
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
                    .map_err(|error| unavailable(error.to_string()))?;
                let intersecting = overlaps
                    .iter()
                    .filter(|overlap| match overlap.containment() {
                        Containment::SubjectInsideOther | Containment::OtherInsideSubject => true,
                        Containment::Partial => {
                            overlap.area_square_metres() >= OVERLAP_AREA_EPSILON_M2
                                && overlap.height_metres() > tolerance
                        }
                    })
                    .count();
                Ok(count(intersecting, locator))
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
                    .map_err(|error| unavailable(error.to_string()))?;
                let share = coverage.covered_ratio();
                Ok(Answer::Number(share, share, locator))
            }
            "support_count" => {
                let counts = service
                    .measure_support_counts()
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok(count(
                    if call.choice("of") == Some("roofs") {
                        counts.roofs()
                    } else {
                        counts.slabs()
                    },
                    locator,
                ))
            }
            _ => unallocated(service, name, object, locator).map_err(unavailable),
        }
    }
}

/// A storey's unallocated floor: its largest region, or its share of the
/// storey's gross floor area.
fn unallocated(
    service: &dyn SpaceService,
    name: &str,
    object: &ObjectId,
    locator: String,
) -> Result<Answer, String> {
    let regions = service
        .measure_unallocated_regions()
        .map_err(|error| error.to_string())?;
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
            QuantityDimension::Area,
            locator,
        ));
    }
    let Some(first) = regions.first() else {
        return Ok(Answer::Number(0.0, 0.0, locator));
    };
    let gross = first
        .floor_area_square_metres()
        .filter(|gross| {
            *gross > 0.0
                && regions
                    .iter()
                    .all(|region| region.floor_area_square_metres() == Some(*gross))
        })
        .ok_or_else(|| {
            "the storey's gross floor area is not measured, so its unallocated share is \
             undefined"
                .to_owned()
        })?;
    let area: f64 = regions
        .iter()
        .map(|region| region.area_square_metres())
        .sum();
    let share = area / gross;
    Ok(Answer::Number(share, share, locator))
}
