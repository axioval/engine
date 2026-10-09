//! Whether a product's `Body` is authored as faces that do not close.
//!
//! A face set or a shell lists its faces' boundaries itself, so whether its
//! faces close up is a fact about the file: every edge of a closed shell
//! bounds two faces. An edge bounding one face only is an open edge as
//! written, whatever a mesh compiler makes of it. This reads the authored
//! faces of each `Body` item (polygonal and triangulated face sets, faceted
//! B-reps, shell- and face-based surface models, through mapped items) and
//! counts those edges by their exact coordinates, item by item: points
//! written twice with equal coordinates are one corner. Items of any other
//! kind (swept or boolean solids, curved faces) state no faces here and
//! count nothing.

use std::collections::BTreeMap;

use ifc_model::{EntityId, Model, Value};

/// How deep mapped items may nest before the walk gives up.
const DEPTH: usize = 8;

/// The edges, over every `Body` item of `product` with authored faces, that
/// bound one face only as written: `None` when the product's shape cannot
/// be read, zero when every authored item closes up.
pub(super) fn open_edges(model: &Model, product: EntityId) -> Option<usize> {
    let entity = model.get(product)?;
    let shape = ifc_geometry::Slots::new(product, entity).opt_ref(super::PRODUCT_REPRESENTATION)?;
    let representations = ifc_geometry::ProductShape::new(shape, model.get(shape)?)
        .representations()
        .ok()?;
    let mut open = 0;
    for id in representations {
        let representation = ifc_geometry::Representation::new(id, model.get(id)?);
        if !representation
            .identifier()
            .is_some_and(|identifier| identifier.eq_ignore_ascii_case("Body"))
        {
            continue;
        }
        for item in representation.items().ok()? {
            open += item_open_edges(model, item, DEPTH)?;
        }
    }
    Some(open)
}

/// The open edges of one representation item, each shell of a surface
/// model counted on its own.
fn item_open_edges(model: &Model, item: EntityId, depth: usize) -> Option<usize> {
    let entity = model.get(item)?;
    let reference = |index: usize| entity.reference(index);
    let references = |index: usize| -> Vec<EntityId> {
        entity
            .attribute(index)
            .and_then(Value::as_list)
            .map(|items| items.iter().filter_map(Value::as_ref_id).collect())
            .unwrap_or_default()
    };
    match &*entity.type_name {
        "IFCMAPPEDITEM" if depth > 0 => {
            let map = model.get(reference(0)?)?;
            let shape = map.reference(1)?;
            let items = ifc_geometry::Representation::new(shape, model.get(shape)?)
                .items()
                .ok()?;
            items
                .into_iter()
                .map(|item| item_open_edges(model, item, depth - 1))
                .sum()
        }
        "IFCPOLYGONALFACESET" => {
            let points = point_list(model, reference(0)?)?;
            let remap = index_list(entity.attribute(3));
            let mut rings = Vec::new();
            for face in references(2) {
                let face = model.get(face)?;
                rings.push(index_list(face.attribute(0))?);
                if let Some(Value::List(holes)) = face.attribute(1) {
                    for hole in holes {
                        rings.push(index_list(Some(hole))?);
                    }
                }
            }
            indexed_open_edges(&points, remap.as_deref(), &rings)
        }
        "IFCTRIANGULATEDFACESET" | "IFCTRIANGULATEDIRREGULARNETWORK" => {
            let points = point_list(model, reference(0)?)?;
            let remap = index_list(entity.attribute(4));
            let rings = entity
                .attribute(3)?
                .as_list()?
                .iter()
                .map(|ring| index_list(Some(ring)))
                .collect::<Option<Vec<_>>>()?;
            indexed_open_edges(&points, remap.as_deref(), &rings)
        }
        "IFCFACETEDBREP" => faces_open_edges(model, &model_faces(model, reference(0)?)?),
        "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => references(0)
            .into_iter()
            .map(|shell| faces_open_edges(model, &model_faces(model, shell)?))
            .sum(),
        _ => Some(0),
    }
}

/// The faces of a shell or connected face set.
fn model_faces(model: &Model, shell: EntityId) -> Option<Vec<EntityId>> {
    Some(
        model
            .get(shell)?
            .attribute(0)?
            .as_list()?
            .iter()
            .filter_map(Value::as_ref_id)
            .collect(),
    )
}

/// The open edges of faces bounded by polygon loops; a bound of any other
/// kind (an edge loop) makes the faces unreadable here.
fn faces_open_edges(model: &Model, faces: &[EntityId]) -> Option<usize> {
    let mut rings = Vec::new();
    for face in faces {
        for bound in model.get(*face)?.attribute(0)?.as_list()? {
            let bound = model.get(bound.as_ref_id()?)?;
            let loop_ = model.get(bound.reference(0)?)?;
            if !loop_.is_type("IFCPOLYLOOP") {
                return None;
            }
            let ring = loop_
                .attribute(0)?
                .as_list()?
                .iter()
                .map(|point| coordinates(model.get(point.as_ref_id()?)?.attribute(0)?))
                .collect::<Option<Vec<_>>>()?;
            rings.push(ring);
        }
    }
    Some(count_open(rings.iter().map(Vec::as_slice)))
}

/// The open edges of rings given as one-based indices into `points`,
/// through `remap` (a face set's `PnIndex`) when there is one.
fn indexed_open_edges(
    points: &[[u64; 3]],
    remap: Option<&[usize]>,
    rings: &[Vec<usize>],
) -> Option<usize> {
    let point = |index: usize| -> Option<[u64; 3]> {
        let index = match remap {
            Some(remap) => *remap.get(index.checked_sub(1)?)?,
            None => index,
        };
        points.get(index.checked_sub(1)?).copied()
    };
    let rings = rings
        .iter()
        .map(|ring| ring.iter().map(|index| point(*index)).collect())
        .collect::<Option<Vec<Vec<[u64; 3]>>>>()?;
    Some(count_open(rings.iter().map(Vec::as_slice)))
}

/// The edges, by exact corner coordinates, that one ring uses and no other
/// ring does. Edges of zero length bound nothing and are left out.
fn count_open<'a>(rings: impl Iterator<Item = &'a [[u64; 3]]>) -> usize {
    let mut uses: BTreeMap<([u64; 3], [u64; 3]), usize> = BTreeMap::new();
    for ring in rings {
        for (index, start) in ring.iter().enumerate() {
            let end = ring[(index + 1) % ring.len()];
            if *start != end {
                *uses
                    .entry((*start.min(&end), *start.max(&end)))
                    .or_default() += 1;
            }
        }
    }
    uses.values().filter(|count| **count == 1).count()
}

/// An `IfcCartesianPointList3D`'s points as coordinate keys.
fn point_list(model: &Model, list: EntityId) -> Option<Vec<[u64; 3]>> {
    model
        .get(list)?
        .attribute(0)?
        .as_list()?
        .iter()
        .map(coordinates)
        .collect()
}

/// A coordinate triple's key: its values' bits, `-0.0` read as `0.0`, so
/// points written with equal coordinates share a key. A point with two
/// coordinates lies at zero height.
fn coordinates(value: &Value) -> Option<[u64; 3]> {
    let values = value.as_list()?;
    let mut key = [0.0_f64.to_bits(); 3];
    if !(2..=3).contains(&values.len()) {
        return None;
    }
    for (slot, value) in key.iter_mut().zip(values) {
        let value = value.unwrap_typed().as_f64()?;
        if !value.is_finite() {
            return None;
        }
        *slot = (value + 0.0).to_bits();
    }
    Some(key)
}

/// A list of one-based indices; `None` for anything else, `$` included.
fn index_list(value: Option<&Value>) -> Option<Vec<usize>> {
    value?
        .as_list()?
        .iter()
        .map(|index| usize::try_from(index.unwrap_typed().as_i64()?).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::count_open;

    fn key(x: f64, y: f64) -> [u64; 3] {
        [x.to_bits(), y.to_bits(), 0.0_f64.to_bits()]
    }

    #[test]
    fn rings_sharing_every_edge_leave_none_open() {
        // A square split into two triangles, each edge of the diagonal
        // used once by either, the outline once: four open edges.
        let first = [key(0.0, 0.0), key(1.0, 0.0), key(1.0, 1.0)];
        let second = [key(0.0, 0.0), key(1.0, 1.0), key(0.0, 1.0)];
        assert_eq!(count_open([&first[..], &second[..]].into_iter()), 4);
        // A tetrahedron's four faces close up.
        let point = |x: f64, y: f64, z: f64| [x.to_bits(), y.to_bits(), z.to_bits()];
        let [a, b, c, d] = [
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(0.0, 1.0, 0.0),
            point(0.0, 0.0, 1.0),
        ];
        let faces = [[a, c, b], [a, b, d], [b, c, d], [c, a, d]];
        assert_eq!(count_open(faces.iter().map(|face| &face[..])), 0);
    }

    #[test]
    fn a_corner_inside_a_neighbours_edge_leaves_three_edges_open() {
        // A T-vertex: one ring passes through (0.5, 0), its neighbour runs
        // straight from (0, 0) to (1, 0).
        let split = [key(0.0, 0.0), key(0.5, 0.0), key(1.0, 0.0), key(0.5, -1.0)];
        let whole = [key(1.0, 0.0), key(0.0, 0.0), key(0.5, 1.0)];
        let open = count_open([&split[..], &whole[..]].into_iter());
        // Both of the split ring's halves, the whole edge, and the four
        // outer edges.
        assert_eq!(open, 3 + 4);
    }
}
