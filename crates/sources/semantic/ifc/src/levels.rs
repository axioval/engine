//! A storey's height to the next storey, answered as the measured value
//! `level_height` (`axioval_ir::MEASURED_LEVEL_HEIGHT`).
//!
//! A storey's elevation is the height of its placement's origin in world
//! coordinates, composed by `ifc-geometry`'s `PlacementResolver` as the
//! object frames compose it, in metres through `ifc_properties::exact_unit`.
//! Its siblings are the storeys aggregated by the same spatial parent
//! (`IfcRelAggregates`), or every unaggregated storey. The next storey is
//! the lowest sibling above it; the highest storey has none (an exact
//! absence). A tilted placement, a sibling at the same elevation, a sibling
//! whose elevation cannot be read, or a storey aggregated twice is refused,
//! never guessed.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use ifc_geometry::constraint::local::PlacementResolver;
use ifc_model::{EntityId, Model, Value};
use ifc_properties::exact_unit;

use crate::release::Release;

/// What `level_height` is for one object.
pub(crate) type LevelHeight = Result<Option<(f64, String)>, String>;

/// Storey elevations and parents, read once per model.
pub(crate) struct Levels {
    release: Release,
    index: OnceLock<Result<Index, String>>,
}

struct Index {
    /// Each storey's world elevation in metres, or why it has none.
    elevations: BTreeMap<EntityId, Result<f64, String>>,
    /// Each storey's spatial parent, `None` when unaggregated.
    parents: BTreeMap<EntityId, Option<EntityId>>,
}

impl Levels {
    pub(crate) fn new(release: Release) -> Self {
        Self {
            release,
            index: OnceLock::new(),
        }
    }

    /// The height from `storey` to the next storey above it among its
    /// siblings, with a locator; `Ok(None)` for an object that is not a
    /// storey or a storey with none above.
    pub(crate) fn height(&self, model: &Model, storey: EntityId) -> LevelHeight {
        let is_storey = model
            .get(storey)
            .is_some_and(|entity| entity.type_name.eq_ignore_ascii_case("IfcBuildingStorey"));
        if !is_storey {
            return Ok(None);
        }
        let index = self
            .index
            .get_or_init(|| index(self.release, model))
            .as_ref()
            .map_err(Clone::clone)?;
        let own = index.elevations[&storey].clone()?;
        let parent = index.parents[&storey];
        let mut above: Option<(f64, EntityId)> = None;
        for (sibling, sibling_parent) in &index.parents {
            if *sibling == storey || *sibling_parent != parent {
                continue;
            }
            let elevation = index.elevations[sibling]
                .clone()
                .map_err(|why| format!("sibling storey {sibling} has no elevation: {why}"))?;
            #[allow(clippy::float_cmp)]
            if elevation == own {
                return Err(format!(
                    "storey {sibling} shares the elevation of storey {storey}"
                ));
            }
            if elevation > own && above.is_none_or(|(lowest, _)| elevation < lowest) {
                above = Some((elevation, *sibling));
            }
        }
        Ok(above
            .map(|(elevation, next)| (elevation - own, format!("level-height:{storey}->{next}"))))
    }
}

fn index(release: Release, model: &Model) -> Result<Index, String> {
    let metres = match exact_unit(model, "IFCLENGTHMEASURE", None) {
        Ok(unit) if unit.offset == 0.0 && unit.scale.is_finite() && unit.scale > 0.0 => unit.scale,
        Ok(_) => return Err("the project length unit has no positive finite scale".into()),
        Err(error) => {
            return Err(format!(
                "the project length unit cannot be resolved exactly: {error}"
            ));
        }
    };
    let storeys: Vec<EntityId> = model.ids_of_type("IfcBuildingStorey").to_vec();
    let resolver = Mutex::new(PlacementResolver::new());
    let slot = |entity: &str, name: &str| {
        release
            .schema
            .attribute_names(entity)
            .iter()
            .position(|candidate| *candidate == name)
            .ok_or_else(|| format!("{entity} declares no {name}"))
    };
    let placement = slot("IfcBuildingStorey", "ObjectPlacement")?;
    let mut elevations = BTreeMap::new();
    for storey in &storeys {
        let elevation = (|| {
            let Some(Value::Ref(placement)) = model
                .get(*storey)
                .and_then(|entity| entity.attribute(placement))
            else {
                return Err(format!("storey {storey} has no ObjectPlacement"));
            };
            let transform = resolver
                .lock()
                .map_err(|_| "placement cache lock is poisoned".to_owned())?
                .world_transform(model, *placement)
                .map_err(|error| error.to_string())?;
            let up = transform.basis[2];
            if up[0].abs() > 1e-12 || up[1].abs() > 1e-12 || (up[2] - 1.0).abs() > 1e-12 {
                return Err(format!("storey {storey}'s placement is tilted"));
            }
            Ok(transform.origin[2] * metres)
        })();
        elevations.insert(*storey, elevation);
    }
    let (relating, related) = (
        slot("IfcRelAggregates", "RelatingObject")?,
        slot("IfcRelAggregates", "RelatedObjects")?,
    );
    let mut parents: BTreeMap<EntityId, Option<EntityId>> =
        storeys.iter().map(|storey| (*storey, None)).collect();
    for relationship in model.ids_of_type("IfcRelAggregates") {
        let malformed = |what: &str| format!("{relationship} (IfcRelAggregates) {what}");
        let entity = model
            .get(*relationship)
            .ok_or_else(|| malformed("is indexed but absent"))?;
        let Some(Value::Ref(parent)) = entity.attribute(relating) else {
            return Err(malformed("has no RelatingObject reference"));
        };
        let Some(Value::List(objects)) = entity.attribute(related) else {
            return Err(malformed("has no RelatedObjects list"));
        };
        for object in objects {
            let Value::Ref(object) = object else {
                return Err(malformed("lists a related object that is not a reference"));
            };
            if let Some(held) = parents.get_mut(object) {
                if held.is_some_and(|held| held != *parent) {
                    return Err(format!("storey {object} is aggregated by two parents"));
                }
                *held = Some(*parent);
            }
        }
    }
    Ok(Index {
        elevations,
        parents,
    })
}
