//! Where an object stands among its source's levels: the elevation of its
//! level, the level's index counted from the ground level, and its height
//! above it.
//!
//! The level is the object itself, or the one level its `path` reaches.
//! A source's levels are its objects of that level's kind, at the
//! elevations of their placement origins. The ground level is the lowest
//! at or above `datum` (default 0, the source's own zero): it is index 0,
//! those above count up and those below count down, levels at one
//! elevation sharing an index.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::path::PathSegment;
use crate::properties::PropertyResolutionError;

/// The names measured here.
pub(super) const NAMES: &[&str] = &["height_above_ground", "level_elevation", "level_index"];

impl Measures {
    /// The level a call reads for `object`, and its elevation; `None` when
    /// the path reaches no level.
    fn level(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Option<(ObjectId, f64)>, PropertyResolutionError> {
        let name = call.name();
        let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
            let (origin, _) = self.origin(name, object)?;
            return Ok(Some((object.clone(), origin[2])));
        };
        let steps: Vec<PathSegment> = steps
            .iter()
            .map(|step| PathSegment::parse(step))
            .collect::<Result<_, _>>()
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let (reached, _) = self.reach(name, &steps, object)?;
        let mut levels = Vec::new();
        for level in reached {
            let (origin, _) = self.origin(name, &level)?;
            levels.push((level, origin[2]));
        }
        let Some((first, elevation)) = levels.first().cloned() else {
            return Ok(None);
        };
        #[allow(clippy::float_cmp)]
        if let Some((other, _)) = levels.iter().find(|(_, other)| *other != elevation) {
            return Err(PropertyResolutionError::Conflicting(format!(
                "the path from {object} reaches levels {first} and {other} at different elevations"
            )));
        }
        Ok(Some((first, elevation)))
    }

    /// A level value of `object`, by `call`'s name.
    pub(super) fn level_measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let Some((level, elevation)) = self.level(call, object)? else {
            return Ok(Answer::Absent(format!(
                "the path from {object} reaches no level"
            )));
        };
        let locator = format!("{name}:{object}:{level}");
        if name == "level_elevation" {
            return Ok(Answer::Value(
                elevation,
                elevation,
                QuantityDimension::Length,
                locator,
            ));
        }
        let datum = match call.argument("datum") {
            Some(MeasuredArgument::Length(datum)) => *datum,
            _ => 0.0,
        };
        let kind = self.kinds.get(&level).cloned().unwrap_or_default();
        let mut elevations = Vec::new();
        for (other, other_kind) in self.kinds.iter() {
            if *other_kind == kind && other.source == level.source {
                elevations.push(self.origin(name, other)?.0[2]);
            }
        }
        elevations.sort_by(f64::total_cmp);
        elevations.dedup_by(|a, b| a.to_bits() == b.to_bits());
        let ground = elevations
            .iter()
            .position(|other| *other >= datum)
            .ok_or_else(|| {
                Self::unavailable(
                    name,
                    object,
                    &format!("no {kind} lies at or above {datum} m"),
                )
            })?;
        if name == "height_above_ground" {
            let ground = elevations[ground];
            let (lower, upper) = crate::measured::rounded_difference(elevation, ground);
            return Ok(Answer::Value(
                lower,
                upper,
                QuantityDimension::Length,
                locator,
            ));
        }
        let own = elevations
            .iter()
            .position(|other| other.to_bits() == elevation.to_bits())
            .unwrap_or(ground);
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
        let index = (own as i64 - ground as i64) as f64;
        Ok(Answer::Number(index, index, locator))
    }
}
