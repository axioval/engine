//! The largest magnitude met composing a product's placement (engine#369).
//!
//! The world transform of a product is its chain of local placements
//! composed, and a chain that cancels large translations (a site at
//! +5.6e6 m, a building at -5.6e6 m beneath it) rounds at the scale of the
//! intermediate ones while the coordinates it yields look small. The
//! rounding of a mesh placed by it is bounded from the largest magnitude
//! along the chain, not from the final coordinates alone ([`super::placed`]).

use ifc_model::{EntityId, Model};

use super::local_frame::{coordinates, location};
use super::{OBJECT_PLACEMENT, PLACEMENT_REL_TO};

/// A bound, in metres, on the translation any partial composition of
/// `product`'s placement chain reaches: the sum of the lengths of the
/// chain's locations. A rotation keeps a translation's length, so no
/// partial composition lies farther from the origin than that sum.
///
/// A level that is not a local placement with a readable location (a
/// linear or grid placement) ends the walk: its own and its ancestors'
/// magnitudes are not counted, and the coordinates it yields still are.
pub(super) fn placement_reach(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    product: EntityId,
) -> f64 {
    let mut next = model
        .get(product)
        .and_then(|entity| entity.attribute(OBJECT_PLACEMENT))
        .and_then(ifc_model::Value::as_ref_id);
    let mut seen = Vec::new();
    let mut reach = 0.0_f64;
    while let Some(placement) = next {
        // A chain that loops has no world transform to bound.
        if seen.contains(&placement) {
            break;
        }
        seen.push(placement);
        let Some(point) =
            location(model, placement).and_then(|(_, point)| coordinates(model, point))
        else {
            break;
        };
        reach +=
            point.iter().map(|value| value * value).sum::<f64>().sqrt() * units.length_to_metres;
        next = model
            .get(placement)
            .and_then(|entity| entity.attribute(PLACEMENT_REL_TO))
            .and_then(ifc_model::Value::as_ref_id);
    }
    reach
}
