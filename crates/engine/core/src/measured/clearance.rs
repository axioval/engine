//! Clearances as values: headroom above a walking surface, the clearance
//! below a flight or ramp, the clear width along it, and a space's clear
//! height, each as the walking-surface or space service measures it for
//! `stair-geometry`, `ramp-geometry` and `space-validation`.
//!
//! The obstacles are the project's objects of the named source kinds,
//! subtypes included, never a package's code.

use axioval_ir::measured::{CLEAR_HEIGHT, CLEAR_WIDTH, CLEARANCE_BELOW, HEADROOM, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, MeasuredArgument, Measures};
use crate::properties::PropertyResolutionError;
use crate::walking_surface::{
    ClearWidthRequest, ClearanceBelowRequest, HeadroomRequest, MeasuredInterval, WalkingStretch,
};

/// The headroom, clearance below, clear width or clear height of `object`.
pub(super) const NAMES: &[&str] = &[CLEAR_HEIGHT, CLEAR_WIDTH, CLEARANCE_BELOW, HEADROOM];

impl Measures {
    /// The objects of the kinds `key` names, subtypes included where the
    /// source declares a type hierarchy; a source without one has no
    /// subtypes, so its kinds match exactly.
    pub(super) fn of_kinds(
        &self,
        call: &MeasuredCall,
        key: &str,
        object: &ObjectId,
    ) -> Result<Vec<ObjectId>, PropertyResolutionError> {
        let name = call.name();
        let Some(MeasuredArgument::SourceKind(kinds)) = call.argument(key) else {
            return Err(PropertyResolutionError::InvalidRequest);
        };
        let kinds: Vec<&str> = kinds.split(',').map(str::trim).collect();
        let mut found = Vec::new();
        for candidate in self.kinds.keys() {
            if candidate == object {
                continue;
            }
            let held = self.kinds.get(candidate).map_or("", String::as_str);
            for kind in &kinds {
                let matched = held.eq_ignore_ascii_case(kind)
                    || (self.hierarchy.is_some()
                        && self
                            .is_kind(candidate, kind)
                            .map_err(|error| Self::unavailable(name, object, &error))?);
                if matched {
                    found.push(candidate.clone());
                    break;
                }
            }
        }
        Ok(found)
    }

    /// A clearance of `object`, by `call`'s name.
    pub(super) fn clearance(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let length = QuantityDimension::Length;
        let value = |interval: MeasuredInterval, locator: String| {
            Answer::Value(interval.lower(), interval.upper(), length, locator)
        };
        if name == CLEAR_HEIGHT {
            let height = self
                .spaces
                .as_ref()
                .ok_or_else(|| Self::missing(name, "space"))?
                .get()
                .measure_clear_height(object)
                .map_err(|error| Self::space_refused(name, object, &error))?;
            return Ok(Answer::Value(
                height.metres(),
                height.metres(),
                length,
                height.evidence().locator.clone(),
            ));
        }
        let walking = self
            .walking
            .as_ref()
            .ok_or_else(|| Self::missing(name, "walking-surface"))?;
        match name {
            HEADROOM => {
                let obstacles = self.of_kinds(call, "obstacles", object)?;
                let measured = walking
                    .measure_headroom(&HeadroomRequest::new(object.clone(), obstacles))
                    .map_err(|error| unavailable(error.to_string()))?;
                let locator = measured.evidence().locator.clone();
                Ok(match measured.clearance() {
                    Some(clearance) => value(clearance, locator),
                    None => Answer::Absent(format!("nothing selected stands above: {locator}")),
                })
            }
            CLEARANCE_BELOW => {
                let spaces = self.of_kinds(call, "spaces", object)?;
                let measured = walking
                    .measure_clearance_below(&ClearanceBelowRequest::new(object.clone(), spaces))
                    .map_err(|error| unavailable(error.to_string()))?;
                let locator = measured.evidence().locator.clone();
                Ok(match measured.clearance() {
                    Some(clearance) => value(clearance, locator),
                    None => Answer::Absent(format!("it stands above no selected floor: {locator}")),
                })
            }
            _ => {
                let obstacles = self.of_kinds(call, "obstacles", object)?;
                let band = match (call.argument("band_from"), call.argument("band_to")) {
                    (Some(MeasuredArgument::Length(from)), Some(MeasuredArgument::Length(to)))
                        if from < to =>
                    {
                        (*from, *to)
                    }
                    _ => return Err(unavailable("the band's bottom is not below its top".into())),
                };
                let stretches = if call.choice("along") == Some("runs") {
                    let runs = walking
                        .measure_sloped_runs(object)
                        .map_err(|error| unavailable(error.to_string()))?;
                    (0..runs.runs().len()).map(WalkingStretch::Run).collect()
                } else {
                    vec![WalkingStretch::Flight]
                };
                // The least over the stretches: no narrower than the least
                // lower bound, and no wider than the least upper.
                let mut least: Option<(f64, f64)> = None;
                let mut locators = Vec::new();
                for stretch in stretches {
                    let request = ClearWidthRequest::try_new(
                        object.clone(),
                        stretch,
                        obstacles.clone(),
                        band,
                    )
                    .map_err(|error| unavailable(error.to_string()))?;
                    let measured = walking
                        .measure_clear_width(&request)
                        .map_err(|error| unavailable(error.to_string()))?;
                    let width = measured.width();
                    least = Some(least.map_or((width.lower(), width.upper()), |(low, high)| {
                        (low.min(width.lower()), high.min(width.upper()))
                    }));
                    locators.push(measured.evidence().locator.clone());
                }
                let (lower, upper) =
                    least.ok_or_else(|| unavailable("it has no run to measure".into()))?;
                Ok(Answer::Value(lower, upper, length, locators.join("; ")))
            }
        }
    }
}
