//! Heights above the terrain as values: `above_terrain` (of an object's
//! placement origin, lowest or highest point, or level) and
//! `mean_terrain_height` (the mean terrain elevation along an outline), each
//! measured against the terrain the run was given ([`crate::terrain`]).
//!
//! The terrain is the ground the call names, `original` or `finished`;
//! one not supplied leaves the value not evaluated, never measured against
//! the other. The outline is the outer boundary of the object's footprint,
//! or of the union of the footprints of the `outline` kinds' objects
//! `outline_path` reaches (all of them in the object's source without a
//! path). A terrain in map coordinates is reached through the stated map
//! conversion of the object's source ([`MapPlacement`], shared with
//! context zones): `E = E0 + s (a x - b y)`, `N = N0 + s (b x + a y)`,
//! `H = z + H0`. Whatever [`MapPlacement::of`] refuses (no conversion, an
//! inexact map unit, a world frame not stated as the identity, a scale
//! that may convert units, another reference system than the terrain
//! names) leaves the value not evaluated.
//!
//! Every value is an interval: the terrain's, widened by its stated
//! accuracy, against the object's, rounded outward.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::coordinate_system::{MapPlacement, MapPlacementRefusal};
use crate::path::PathSegment;
use crate::properties::PropertyResolutionError;
use crate::terrain::{
    TerrainElevation, TerrainError, TerrainFrame, TerrainKind, TerrainOutlineRequest,
    TerrainPointRequest, TerrainServiceHandle, TerrainSurfaceStatement,
};

/// The height of a point, extent or level above the terrain.
pub(super) const ABOVE: &str = "above_terrain";
/// The mean terrain elevation along an outline.
pub(super) const MEAN: &str = "mean_terrain_height";

/// The names measured here.
pub(super) const NAMES: &[&str] = &[ABOVE, MEAN];

/// How far, in metres, rounding a model point onto a map may move the
/// ground's elevation there: far above the error of the conversion's few
/// operations on coordinates of a few thousand kilometres, times any
/// gradient a terrain has.
const MAP_ROUNDING_METRES: f64 = 1e-6;

/// Refuses parameters that do not go together: `reference=beneath` is of a
/// point, `path` of a level, and an outline path needs the outline's kinds.
pub(super) fn validate(call: &MeasuredCall) -> Result<(), String> {
    let name = call.name();
    let stated = |key: &str| call.argument(key).is_some();
    if stated("outline_path") && !stated("outline") {
        return Err(format!("`{name}` takes `outline_path` only with `outline`"));
    }
    if name != ABOVE {
        return Ok(());
    }
    let of = call.choice("of");
    if call.choice("reference") == Some("beneath") {
        if of != Some("point") {
            return Err(format!(
                "`{name}` measures `reference=beneath` only `of=point`: beneath an extent or a \
                 level the terrain is no one height"
            ));
        }
        if stated("outline") {
            return Err(format!(
                "`{name}` takes no `outline` with `reference=beneath`"
            ));
        }
    }
    if stated("path") && of != Some("level") {
        return Err(format!("`{name}` takes `path` only `of=level`"));
    }
    Ok(())
}

/// Where a model point lies in a terrain's coordinates.
#[derive(Clone, Copy, Debug)]
enum Placement {
    /// The terrain is in the model's coordinates.
    Model,
    /// The terrain is on the map the source's conversion maps onto.
    Map(MapPlacement),
}

impl Placement {
    /// A model plan point in the terrain's coordinates.
    fn plan(self, point: [f64; 2]) -> [f64; 2] {
        match self {
            Self::Model => point,
            Self::Map(map) => map.to_map(point),
        }
    }

    /// A model plan length in the terrain's coordinates.
    fn length(self, metres: f64) -> f64 {
        match self {
            Self::Model => metres,
            Self::Map(map) => metres * map.scale(),
        }
    }

    /// A terrain elevation interval in the model's heights, rounded
    /// outward, widened by the placement's rounding.
    fn height(self, elevation: &TerrainElevation) -> Result<(f64, f64), PropertyResolutionError> {
        let measured = super::span(elevation.lower_metres(), elevation.upper_metres());
        match self {
            Self::Model => Ok((elevation.lower_metres(), elevation.upper_metres())),
            Self::Map(map) => {
                let offset = map.offset_metres()[2];
                let slack = super::span(-MAP_ROUNDING_METRES, MAP_ROUNDING_METRES);
                super::bounds(
                    measured
                        .minus(super::span(offset, offset))
                        .and_then(|height| height.plus(slack)),
                )
            }
        }
    }
}

impl Measures {
    /// A height above the terrain, or the mean terrain height, of
    /// `object`, by `call`'s name.
    pub(super) fn terrain_measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let service = self
            .terrain
            .as_ref()
            .ok_or_else(|| Self::missing(name, "terrain"))?;
        let kind = call
            .choice("terrain")
            .and_then(TerrainKind::parse)
            .ok_or(PropertyResolutionError::InvalidRequest)?;
        let surface = service
            .surface(kind)
            .map_err(|error| Self::terrain_refused(name, object, &error))?;
        let placement = self.terrain_placement(name, object, &surface)?;
        let length = Some(QuantityDimension::Length);
        if name == MEAN {
            let (lower, upper, exact, locator) =
                self.mean_terrain(call, object, service, kind, placement)?;
            return Ok(Answer::Value(
                lower,
                upper,
                length,
                format!("{name}:{object};{locator}"),
                exact,
            ));
        }
        // What stands above the terrain: an interval, its exactness, its
        // locator and its plan position where it is a point.
        let ((low, high), exact, cited, point) = match call.choice("of") {
            Some("point") => {
                let (origin, locator) = self.origin(name, object)?;
                (
                    (origin[2], origin[2]),
                    true,
                    locator,
                    Some([origin[0], origin[1]]),
                )
            }
            Some(end @ ("bottom" | "top")) => {
                let extent = self
                    .vertical
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "vertical-extent"))?
                    .measure_vertical_extent(object)
                    .map_err(|error| Self::unavailable(name, object, &error.to_string()))?;
                let bound = if end == "bottom" {
                    extent.bottom()
                } else {
                    extent.top()
                };
                (
                    (bound.lower_metres(), bound.upper_metres()),
                    extent.evidence().exact,
                    extent.evidence().locator.clone(),
                    None,
                )
            }
            Some("level") => {
                let Some((level, elevation)) = self.level(call, object)? else {
                    return Ok(Answer::Absent(format!(
                        "the path from {object} reaches no level"
                    )));
                };
                ((elevation, elevation), true, format!("level:{level}"), None)
            }
            _ => return Err(PropertyResolutionError::InvalidRequest),
        };
        let (terrain, terrain_exact, locator) = if call.choice("reference") == Some("beneath") {
            let point = point.ok_or(PropertyResolutionError::InvalidRequest)?;
            let request = TerrainPointRequest::try_new(kind, placement.plan(point))
                .map_err(|error| Self::terrain_refused(name, object, &error))?;
            let elevation = service
                .measure_elevation(&request)
                .map_err(|error| Self::terrain_refused(name, object, &error))?;
            (
                placement.height(&elevation)?,
                elevation.is_exact(),
                elevation.locator().to_owned(),
            )
        } else {
            let (lower, upper, exact, locator) =
                self.mean_terrain(call, object, service, kind, placement)?;
            ((lower, upper), exact, locator)
        };
        let (lower, upper) =
            super::bounds(super::span(low, high).minus(super::span(terrain.0, terrain.1)))?;
        Ok(Answer::Value(
            lower,
            upper,
            length,
            format!("{name}:{object};{cited};{locator}"),
            exact && terrain_exact,
        ))
    }

    /// The refusal of a terrain measurement.
    fn terrain_refused(
        name: &str,
        object: &ObjectId,
        error: &TerrainError,
    ) -> PropertyResolutionError {
        match error {
            TerrainError::InvalidMeasurement | TerrainError::ResponseRequestMismatch => {
                PropertyResolutionError::InvalidValue
            }
            other => Self::unavailable(name, object, &other.to_string()),
        }
    }

    /// Where `object`'s coordinates lie in `surface`'s.
    fn terrain_placement(
        &self,
        name: &str,
        object: &ObjectId,
        surface: &TerrainSurfaceStatement,
    ) -> Result<Placement, PropertyResolutionError> {
        if surface.frame() == TerrainFrame::Model {
            return Ok(Placement::Model);
        }
        let refused = |why: &str| {
            Self::unavailable(
                name,
                object,
                &format!(
                    "the {} terrain is in map coordinates, and {why}",
                    surface.kind()
                ),
            )
        };
        let system = self
            .coordinates
            .as_ref()
            .ok_or_else(|| Self::missing(name, "coordinate-system"))?
            .coordinate_system(&object.source)
            .map_err(|error| refused(&format!("its source's coordinate system: {error}")))?;
        let placement = MapPlacement::of(&system, surface.crs()).map_err(|refusal| {
            Self::unavailable(
                name,
                object,
                &match (&refusal, surface.crs()) {
                    (MapPlacementRefusal::OtherTarget { .. }, Some(crs)) => {
                        format!(
                            "the {} terrain names `{crs}`, and {refusal}",
                            surface.kind()
                        )
                    }
                    _ => format!(
                        "the {} terrain is in map coordinates, and {refusal}",
                        surface.kind()
                    ),
                },
            )
        })?;
        Ok(Placement::Map(placement))
    }

    /// The objects whose footprints make `object`'s outline.
    fn outline_objects(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Vec<ObjectId>, PropertyResolutionError> {
        let name = call.name();
        let Some(MeasuredArgument::SourceKind(kinds)) = call.argument("outline") else {
            return Ok(vec![object.clone()]);
        };
        let kinds: Vec<&str> = kinds.split(',').map(str::trim).collect();
        let mut found = Vec::new();
        for (candidate, held) in self.kinds.iter() {
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
        if let Some(MeasuredArgument::Path(steps)) = call.argument("outline_path") {
            let steps: Vec<PathSegment> = steps
                .iter()
                .map(|step| PathSegment::parse(step))
                .collect::<Result<_, _>>()
                .map_err(|_| PropertyResolutionError::InvalidRequest)?;
            let (reached, _) = self.reach(name, &steps, object)?;
            found.retain(|candidate| reached.contains(candidate) || candidate == object);
        } else {
            found.retain(|candidate| candidate.source == object.source);
        }
        if found.is_empty() {
            return Err(Self::unavailable(
                name,
                object,
                "no object of the outline's kinds is selected",
            ));
        }
        Ok(found)
    }

    /// The mean terrain elevation along `object`'s outline, in the model's
    /// heights: its bounds, exactness and locator.
    fn mean_terrain(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        service: &TerrainServiceHandle,
        kind: TerrainKind,
        placement: Placement,
    ) -> Result<(f64, f64, bool, String), PropertyResolutionError> {
        let name = call.name();
        let objects = self.outline_objects(call, object)?;
        let outline = self
            .plan
            .as_ref()
            .ok_or_else(|| Self::missing(name, "plan-area"))?
            .measure_footprint_outline(&objects)
            .map_err(|error| {
                Self::unavailable(
                    name,
                    object,
                    &format!("its outline cannot be traced: {error}"),
                )
            })?;
        let rings = outline
            .rings()
            .iter()
            .map(|ring| ring.iter().map(|point| placement.plan(*point)).collect())
            .collect();
        let request = TerrainOutlineRequest::try_new(
            kind,
            rings,
            placement.length(outline.deviation_metres()),
        )
        .map_err(|error| Self::terrain_refused(name, object, &error))?;
        let mean = service
            .measure_mean_elevation(&request)
            .map_err(|error| Self::terrain_refused(name, object, &error))?;
        let (lower, upper) = placement.height(&mean)?;
        Ok((
            lower,
            upper,
            mean.is_exact() && outline.evidence().exact,
            format!("{};{}", mean.locator(), outline.evidence().locator),
        ))
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn parameters_that_do_not_go_together_are_refused() {
        for (name, expected) in [
            (
                "above_terrain;terrain=original;of=level;reference=beneath",
                "only `of=point`",
            ),
            (
                "above_terrain;terrain=original;of=point;reference=beneath;outline=IfcSlab",
                "no `outline`",
            ),
            (
                "above_terrain;terrain=original;of=point;path=contains:backward",
                "only `of=level`",
            ),
            (
                "mean_terrain_height;terrain=original;outline_path=contains:backward",
                "only with `outline`",
            ),
        ] {
            let refused = super::super::parse(name).unwrap_err();
            assert!(refused.contains(expected), "{name}: {refused}");
        }
        for name in [
            "above_terrain;terrain=original;of=point;reference=beneath",
            "above_terrain;terrain=finished;of=level;path=contains:backward;outline=IfcSlab",
            "mean_terrain_height;terrain=original;outline=IfcSlab;outline_path=contains:backward",
        ] {
            assert!(super::super::parse(name).is_ok(), "{name}");
        }
        assert!(
            super::super::parse("above_terrain;of=point").is_err(),
            "a ground is named"
        );
    }
}
