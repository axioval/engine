//! The local frame georeferenced models are measured in (#372).
//!
//! A model placed in map coordinates puts its site, and every body under
//! it, millions of metres from the origin, where a coordinate rounds to
//! about 1e-9 m: a mesh built there is not the solid, so [`super::placed`]
//! declares it within that rounding and services needing exact evidence
//! refuse it. The bridge therefore measures such a model about a local
//! origin: the plan position its outermost placement states.
//!
//! The translation is taken out of the placements themselves before
//! anything is composed: every placement relative to nothing (the root of
//! every placement chain) has the origin subtracted from its location, in
//! the file's units, so the chains compose at the magnitudes they reach
//! below the site and every mesh is compiled at small coordinates. The
//! chosen root's location becomes the origin exactly; another root's is
//! exact where it lies within a factor of two of it (Sterbenz) and lies far
//! out otherwise, where [`super::placed`] still counts its rounding. Heights
//! are never moved, so levels and elevations read alike in both frames.
//!
//! Only what is placed through such chains moves, so a model is measured in
//! a local frame only where every placement is: no linear or grid placement
//! (each derives its frame from curves stated elsewhere), no alignment
//! (whose curves are stated in model coordinates), every representation
//! context's world coordinate system unrotated (so a root's translation is
//! the model's), every product with a body placed, and every root's
//! location readable. Every source of a session is measured about one
//! origin, or none is: a model that does not qualify leaves every model in
//! its model coordinates, as before.

use std::collections::BTreeMap;

use axioval_engine::{LocalOrigin, LocalOrigins};
use axioval_ir::SourceId;
use ifc_model::{Entity, EntityId, Model, Value};

use super::{ALIGNMENTS, EXACT_REACH, OBJECT_PLACEMENT, PLACEMENT_REL_TO, Parsed};

/// `IfcLocalPlacement.RelativePlacement`, `IfcPlacement.Location` and
/// `IfcCartesianPoint.Coordinates`.
const RELATIVE_PLACEMENT: usize = 1;
const LOCATION: usize = 0;
const COORDINATES: usize = 0;

/// `IfcGeometricRepresentationContext.WorldCoordinateSystem`.
const WORLD_COORDINATE_SYSTEM: usize = 4;

/// `IfcAxis2Placement3D.Axis` and `.RefDirection`, `IfcAxis2Placement2D.RefDirection`.
const AXIS: usize = 1;
const REF_DIRECTION_3D: usize = 2;
const REF_DIRECTION_2D: usize = 1;

/// `IfcDirection.DirectionRatios`.
const DIRECTION_RATIOS: usize = 0;

/// The local origin every source is measured about, in metres, and each
/// source's model with its placements moved there; `None`, with every
/// model untouched, where the first source lies within [`EXACT_REACH`] of
/// its origin or any source does not qualify, or where the first has no resolvable units.
pub(super) fn localize(
    parsed: &mut BTreeMap<SourceId, Parsed>,
    first: &SourceId,
) -> Option<LocalOrigins> {
    let reference = parsed.get(first)?;
    let placed = roots(&reference.model)?;
    // A reference source whose units do not resolve has no scale to
    // localize by, so no origin is derived from any of the sources.
    let scale = reference.units().ok()?.length_to_metres;
    // The root reaching farthest out, the first in id order among equals.
    let reach = |[x, y]: [f64; 2]| x.abs().max(y.abs());
    let (_, [x, y]) = placed
        .iter()
        .copied()
        .max_by(|(a, p), (b, q)| reach(*p).total_cmp(&reach(*q)).then(b.cmp(a)))?;
    if x.abs().max(y.abs()) * scale <= EXACT_REACH {
        return None;
    }
    let origin = LocalOrigin::new([x * scale, y * scale])?;
    let mut shifts = BTreeMap::new();
    for (source, model) in parsed.iter() {
        // A source whose units do not resolve is never meshed, so it is
        // not moved; the others keep their origin.
        if source != first && model.units().is_err() {
            continue;
        }
        let roots = roots(&model.model)?;
        if !qualifies(&model.model) {
            return None;
        }
        // The first source's root becomes the origin exactly; any other
        // source's shift is the origin in its own units.
        let shift = if source == first {
            [x, y]
        } else {
            let scale = model.units().ok()?.length_to_metres;
            let [east, north] = origin.metres();
            [east / scale, north / scale]
        };
        shifts.insert(source.clone(), (roots, shift));
    }
    for (source, (roots, shift)) in shifts {
        let model = &mut parsed.get_mut(&source)?.model;
        for (root, _) in roots {
            move_root(model, root, shift)?;
        }
    }
    Some(LocalOrigins::new(
        parsed.keys().map(|source| (source.clone(), origin)),
    ))
}

/// Every placement relative to nothing, with its location's plan
/// coordinates in file units; `None` if any cannot be read.
fn roots(model: &Model) -> Option<Vec<(EntityId, [f64; 2])>> {
    model
        .ids_of_type("IFCLOCALPLACEMENT")
        .iter()
        .filter(|id| {
            model
                .get(**id)
                .and_then(|entity| entity.attribute(PLACEMENT_REL_TO))
                .is_none_or(|relative| matches!(relative, Value::Null))
        })
        .map(|&id| {
            let (_, point) = location(model, id)?;
            let coordinates = coordinates(model, point)?;
            Some((id, [*coordinates.first()?, *coordinates.get(1)?]))
        })
        .collect()
}

/// A local placement's relative placement and its location point.
fn location(model: &Model, placement: EntityId) -> Option<(EntityId, EntityId)> {
    let relative = model
        .get(placement)?
        .attribute(RELATIVE_PLACEMENT)?
        .as_ref_id()?;
    let axes = model.get(relative)?;
    if !axes.is_type("IFCAXIS2PLACEMENT3D") && !axes.is_type("IFCAXIS2PLACEMENT2D") {
        return None;
    }
    Some((relative, axes.attribute(LOCATION)?.as_ref_id()?))
}

/// A Cartesian point's coordinates, in file units.
fn coordinates(model: &Model, point: EntityId) -> Option<Vec<f64>> {
    let entity = model.get(point)?;
    if !entity.is_type("IFCCARTESIANPOINT") {
        return None;
    }
    let values = entity.attribute(COORDINATES)?.unwrap_typed().as_list()?;
    let values: Option<Vec<f64>> = values
        .iter()
        .map(|value| value.unwrap_typed().as_f64().filter(|v| v.is_finite()))
        .collect();
    values.filter(|values| (2..=3).contains(&values.len()))
}

/// Whether every placement of `model` is composed through its chain of
/// local placements under an unrotated world coordinate system.
fn qualifies(model: &Model) -> bool {
    let derived = ["IFCLINEARPLACEMENT", "IFCGRIDPLACEMENT"]
        .iter()
        .chain(ALIGNMENTS)
        .any(|type_name| !model.ids_of_type(type_name).is_empty());
    if derived {
        return false;
    }
    let unrotated = model
        .ids_of_type("IFCGEOMETRICREPRESENTATIONCONTEXT")
        .iter()
        .all(|&context| {
            let Some(system) = model
                .get(context)
                .and_then(|entity| entity.attribute(WORLD_COORDINATE_SYSTEM))
                .and_then(Value::as_ref_id)
            else {
                return false;
            };
            unrotated(model, system)
        });
    unrotated
        && ifc_geometry::geometric_products(model)
            .into_iter()
            .all(|product| {
                model
                    .get(product)
                    .and_then(|entity| entity.attribute(OBJECT_PLACEMENT))
                    .is_some_and(|placement| placement.as_ref_id().is_some())
            })
}

/// Whether an axis placement states the identity's axes, or none.
#[allow(clippy::float_cmp)]
fn unrotated(model: &Model, placement: EntityId) -> bool {
    let Some(entity) = model.get(placement) else {
        return false;
    };
    let along = |slot: usize, expected: &[f64]| match entity.attribute(slot) {
        None | Some(Value::Null) => true,
        Some(value) => value
            .as_ref_id()
            .and_then(|direction| model.get(direction))
            .filter(|direction| direction.is_type("IFCDIRECTION"))
            .and_then(|direction| direction.attribute(DIRECTION_RATIOS))
            .and_then(|ratios| ratios.unwrap_typed().as_list())
            .is_some_and(|ratios| {
                ratios.len() == expected.len()
                    && ratios
                        .iter()
                        .zip(expected)
                        .all(|(ratio, expected)| ratio.unwrap_typed().as_f64() == Some(*expected))
            }),
    };
    if entity.is_type("IFCAXIS2PLACEMENT3D") {
        along(AXIS, &[0.0, 0.0, 1.0]) && along(REF_DIRECTION_3D, &[1.0, 0.0, 0.0])
    } else if entity.is_type("IFCAXIS2PLACEMENT2D") {
        along(REF_DIRECTION_2D, &[1.0, 0.0])
    } else {
        false
    }
}

/// Moves a root placement's location by `-shift` (file units): a new point
/// and a new axis placement, so nothing else sharing either moves, and the
/// root placement pointed at them.
fn move_root(model: &mut Model, root: EntityId, [dx, dy]: [f64; 2]) -> Option<()> {
    let (axes, point) = location(model, root)?;
    let mut coordinates = coordinates(model, point)?;
    coordinates[0] -= dx;
    coordinates[1] -= dy;
    let moved = model.push(Entity::new(
        "IFCCARTESIANPOINT",
        vec![Value::List(
            coordinates.into_iter().map(Value::Real).collect(),
        )],
    ));
    let mut placement = model.get(axes)?.clone();
    *placement.attributes.get_mut(LOCATION)? = Value::Ref(moved);
    let placement = model.push(placement);
    let mut local = model.get(root)?.clone();
    *local.attributes.get_mut(RELATIVE_PLACEMENT)? = Value::Ref(placement);
    model.insert(root, local);
    Some(())
}
