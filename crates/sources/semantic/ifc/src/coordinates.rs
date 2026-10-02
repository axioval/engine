//! The source coordinate system from IFC representation contexts.
//!
//! An IFC file states its coordinate system on its model context, the root
//! `IfcGeometricRepresentationContext` whose `ContextType` is `Model`:
//!
//! - `WorldCoordinateSystem` is the frame model coordinates are stated in. It
//!   is resolved by `ifc-geometry`'s `axis_placement_transform`, the same
//!   reader that places bodies, and its origin is converted through the exact
//!   project length unit, never assumed to be in metres.
//! - `TrueNorth` is the plan direction of geographic north.
//! - An IFC4 `IfcMapConversion` whose `SourceCRS` is that context maps the
//!   model onto the named `TargetCRS`. `XAxisAbscissa`/`XAxisOrdinate` and
//!   `Scale` take the schema's stated defaults (no rotation, scale 1) when
//!   unset. The offset is in the map's unit, the target's `MapUnit`: when it
//!   is not stated, or cannot be resolved exactly, the unit is reported
//!   unknown rather than assumed.
//!
//! - The site placement is the `ObjectPlacement` of the file's one `IfcSite`,
//!   composed by `ifc-geometry`'s `PlacementResolver` as object frames are,
//!   origin in metres through the exact project length unit. No site is
//!   absent; several sites, a grid placement or a placement that cannot be
//!   resolved exactly leave it unknown, with the reason.
//!
//! A file without a model context states no coordinate system; several model
//! contexts, or several conversions of the one context, are ambiguous and
//! refused. IFC2X3 has no map conversion, so its files state none.

use std::sync::Arc;

use axioval_engine::{
    CoordinateFrame, CoordinateSystemError, CoordinateSystemService, MapConversion,
    MetricDirection, SitePlacement, SourceCoordinateSystem, SourceSnapshot,
};
use axioval_ir::{Evidence, SourceId};
use ifc_geometry::constraint::local::PlacementResolver;
use ifc_geometry::resource::{Direction, axis_placement_transform};
use ifc_geometry::{RepresentationContext, all_contexts};
use ifc_model::{Entity, EntityId, Model, Value};
use ifc_properties::exact_unit;

use crate::release::Release;

pub(crate) struct IfcCoordinateSystem {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
}

impl IfcCoordinateSystem {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
        }
    }

    /// The value of `entity`'s attribute `name`, by its slot in this
    /// release's schema.
    fn attribute<'m>(&self, entity: &'m Entity, name: &str) -> Option<&'m Value> {
        let slot = self
            .release
            .schema
            .attribute_names(&entity.type_name)
            .iter()
            .position(|attribute| attribute.eq_ignore_ascii_case(name))?;
        entity.attribute(slot)
    }

    fn number(
        &self,
        entity: &Entity,
        id: EntityId,
        name: &str,
    ) -> Result<Option<f64>, CoordinateSystemError> {
        match self.attribute(entity, name).map(Value::unwrap_typed) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Real(value)) => Ok(Some(*value)),
            #[allow(clippy::cast_precision_loss)] // STEP integers in a real slot
            Some(Value::Integer(value)) => Ok(Some(*value as f64)),
            Some(_) => Err(CoordinateSystemError::Unreadable(format!(
                "{id}.{name} is not a number"
            ))),
        }
    }

    fn required(
        &self,
        entity: &Entity,
        id: EntityId,
        name: &str,
    ) -> Result<f64, CoordinateSystemError> {
        self.number(entity, id, name)?
            .ok_or_else(|| CoordinateSystemError::Unreadable(format!("{id}.{name} is not set")))
    }

    fn reference(&self, entity: &Entity, name: &str) -> Option<EntityId> {
        match self.attribute(entity, name) {
            Some(Value::Ref(id)) => Some(*id),
            _ => None,
        }
    }

    /// The single root model context, if the file has one.
    fn model_context(&self) -> Result<Option<RepresentationContext<'_>>, CoordinateSystemError> {
        let contexts: Vec<RepresentationContext<'_>> = all_contexts(&self.model)
            .into_iter()
            .filter(|context| {
                !context.is_sub_context()
                    && context
                        .context_type()
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("Model"))
            })
            .collect();
        match contexts.as_slice() {
            [] => Ok(None),
            [context] => Ok(Some(*context)),
            several => Err(CoordinateSystemError::Ambiguous(format!(
                "{} model contexts ({})",
                several.len(),
                several
                    .iter()
                    .map(|context| context.id().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    fn world(
        &self,
        context: &RepresentationContext<'_>,
    ) -> Result<CoordinateFrame, CoordinateSystemError> {
        let id = context
            .world_coordinate_system(&self.model)
            .ok_or_else(|| {
                CoordinateSystemError::Unreadable(format!(
                    "{}.WorldCoordinateSystem is not set",
                    context.id()
                ))
            })?;
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| CoordinateSystemError::Unreadable(format!("{id} is missing")))?;
        let transform = axis_placement_transform(&self.model, id, entity)
            .map_err(|error| CoordinateSystemError::Unsupported(error.to_string()))?;
        frame(transform.origin, transform.basis, self.metres()?)
    }

    /// Metres per project length unit, resolved exactly.
    fn metres(&self) -> Result<f64, CoordinateSystemError> {
        match exact_unit(&self.model, "IFCLENGTHMEASURE", None) {
            Ok(unit) if unit.offset == 0.0 && unit.scale.is_finite() && unit.scale > 0.0 => {
                Ok(unit.scale)
            }
            Ok(_) => Err(CoordinateSystemError::Unreadable(
                "the project length unit has no positive finite scale".into(),
            )),
            Err(error) => Err(CoordinateSystemError::Unreadable(format!(
                "the project length unit cannot be resolved exactly: {error}"
            ))),
        }
    }

    /// The placement of the file's one site, and the site's id.
    fn site(&self) -> (SitePlacement, Option<EntityId>) {
        let sites = self.model.ids_of_type("IFCSITE");
        let id = match sites {
            [] => return (SitePlacement::Absent, None),
            [id] => *id,
            several => {
                return (
                    SitePlacement::Unknown(format!(
                        "{} sites ({}); none of them is the source's site",
                        several.len(),
                        several
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    None,
                );
            }
        };
        match self.site_frame(id) {
            Ok(frame) => (SitePlacement::Stated(frame), Some(id)),
            Err(reason) => (SitePlacement::Unknown(reason), Some(id)),
        }
    }

    fn site_frame(&self, id: EntityId) -> Result<CoordinateFrame, String> {
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| format!("{id} is missing"))?;
        let placement = match self.attribute(entity, "ObjectPlacement") {
            Some(Value::Ref(placement)) => *placement,
            Some(Value::Null) | None => return Err(format!("{id}.ObjectPlacement is not set")),
            Some(_) => return Err(format!("{id}.ObjectPlacement is not a reference")),
        };
        let kind = self
            .model
            .get(placement)
            .map(|entity| entity.type_name.to_ascii_uppercase())
            .ok_or_else(|| format!("{id}.ObjectPlacement {placement} is missing"))?;
        if kind != "IFCLOCALPLACEMENT" {
            return Err(format!(
                "{id} is placed by {kind} {placement}; only IfcLocalPlacement chains are resolved"
            ));
        }
        let transform = PlacementResolver::new()
            .world_transform(&self.model, placement)
            .map_err(|error| error.to_string())?;
        let metres = self.metres().map_err(|error| error.to_string())?;
        frame(transform.origin, transform.basis, metres)
            .map_err(|error| format!("{id}'s placement: {error}"))
    }

    fn true_north(
        &self,
        context: &RepresentationContext<'_>,
    ) -> Result<Option<[f64; 2]>, CoordinateSystemError> {
        let Some(id) = context.true_north(&self.model) else {
            return Ok(None);
        };
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| CoordinateSystemError::Unreadable(format!("{id} is missing")))?;
        let ratios = Direction::new(id, entity)
            .ratios()
            .map_err(|error| CoordinateSystemError::Unreadable(error.to_string()))?;
        match ratios.as_slice() {
            [x, y] | [x, y, _] => Ok(Some([*x, *y])),
            _ => Err(CoordinateSystemError::Unreadable(format!(
                "{id} is not a plan direction"
            ))),
        }
    }

    /// The map conversion of `context`, if the file states one.
    fn map(
        &self,
        context: EntityId,
    ) -> Result<Option<(EntityId, MapConversion)>, CoordinateSystemError> {
        let conversions = self.model.ids_of_type("IFCMAPCONVERSION");
        let own: Vec<EntityId> = conversions
            .iter()
            .copied()
            .filter(|&id| {
                self.model
                    .get(id)
                    .and_then(|entity| self.reference(entity, "SourceCRS"))
                    == Some(context)
            })
            .collect();
        let id = match own.as_slice() {
            [] if conversions.is_empty() => return Ok(None),
            [] => {
                return Err(CoordinateSystemError::Unsupported(format!(
                    "map conversions {} convert from something other than the model context {context}",
                    conversions
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            [id] => *id,
            several => {
                return Err(CoordinateSystemError::Ambiguous(format!(
                    "{} map conversions of the model context {context}",
                    several.len()
                )));
            }
        };
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| CoordinateSystemError::Unreadable(format!("{id} is missing")))?;
        let offset = [
            self.required(entity, id, "Eastings")?,
            self.required(entity, id, "Northings")?,
            self.required(entity, id, "OrthogonalHeight")?,
        ];
        let x_axis = [
            self.number(entity, id, "XAxisAbscissa")?.unwrap_or(1.0),
            self.number(entity, id, "XAxisOrdinate")?.unwrap_or(0.0),
        ];
        let scale = self.number(entity, id, "Scale")?.unwrap_or(1.0);
        let (target, unit) = match self.reference(entity, "TargetCRS") {
            Some(target) => self.target(target)?,
            None => {
                return Err(CoordinateSystemError::Unreadable(format!(
                    "{id}.TargetCRS is not a reference"
                )));
            }
        };
        Ok(Some((
            id,
            MapConversion::try_new(target, offset, x_axis, scale, unit)?,
        )))
    }

    /// The target system's name and metres per map unit, when known.
    fn target(&self, id: EntityId) -> Result<(Option<String>, Option<f64>), CoordinateSystemError> {
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| CoordinateSystemError::Unreadable(format!("{id} is missing")))?;
        let name = match self.attribute(entity, "Name").map(Value::unwrap_typed) {
            Some(Value::Text(name)) if !name.trim().is_empty() => Some(name.to_string()),
            _ => None,
        };
        let unit = self.reference(entity, "MapUnit").and_then(|unit| {
            exact_unit(&self.model, "IFCLENGTHMEASURE", Some(unit))
                .ok()
                .filter(|unit| unit.offset == 0.0 && unit.scale.is_finite() && unit.scale > 0.0)
                .map(|unit| unit.scale)
        });
        Ok((name, unit))
    }
}

/// A frame from a composed placement, its origin scaled to metres.
fn frame(
    origin: [f64; 3],
    basis: [[f64; 3]; 3],
    metres: f64,
) -> Result<CoordinateFrame, CoordinateSystemError> {
    let axis = |vector: [f64; 3]| {
        MetricDirection::try_new(vector).map_err(|_| CoordinateSystemError::InvalidMeasurement)
    };
    let [x, y, z] = basis;
    CoordinateFrame::try_new(
        origin.map(|value| value * metres),
        axis(x)?,
        axis(y)?,
        axis(z)?,
    )
}

impl CoordinateSystemService for IfcCoordinateSystem {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn coordinate_system(
        &self,
        source: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        let fingerprint = self.snapshots[0].fingerprint();
        let (site, site_id) = self.site();
        let site_locator = site_id.map_or_else(String::new, |id| format!(":site:{id}"));
        let Some(context) = self.model_context()? else {
            return Ok(SourceCoordinateSystem::try_new(
                source.clone(),
                None,
                None,
                None,
                Evidence::exact(
                    source.clone(),
                    format!("ifc:{fingerprint}:coordinate-system:no-model-context{site_locator}"),
                ),
            )?
            .with_site(site));
        };
        let world = self.world(&context)?;
        let true_north = self.true_north(&context)?;
        let map = self.map(context.id())?;
        let locator = format!(
            "ifc:{fingerprint}:coordinate-system:{}:{}{site_locator}",
            context.id(),
            map.as_ref()
                .map_or_else(|| "no-map-conversion".to_owned(), |(id, _)| id.to_string())
        );
        Ok(SourceCoordinateSystem::try_new(
            source.clone(),
            Some(world),
            true_north,
            map.map(|(_, map)| map),
            Evidence::exact(source.clone(), locator),
        )?
        .with_site(site))
    }
}
