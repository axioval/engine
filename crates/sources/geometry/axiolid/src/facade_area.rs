//! Facade area: the part of a mesh's surface that faces the outside.
//!
//! ADR 0004: this module measures surface area; whether a storey's windows
//! glaze too much of its facade is a rule's decision.
//!
//! A face belongs to the facade when it is steep (its normal within 45° of
//! horizontal) and, looking straight out of it horizontally from each of
//! four sample points (the centroids of its midpoint sub-triangles):
//!
//! - the point just outside it lies in no other body and no declared space,
//!   so it is not held against a neighbour (a wall end at a corner, a frame
//!   in its reveal) or flush with a room;
//! - the first body its ray meets within [`REACH`] is neither the object
//!   itself (a reveal, a niche) nor a declared space (a wall's inner face).
//!
//! A ray that meets nothing, or first meets another body, looks outside: a
//! facade across a courtyard still faces the courtyard. Which objects are
//! spaces is semantic, so the host declares them; a room the host did not
//! declare as a space looks like the outside.
//!
//! A face whose samples disagree is partly covered, and the mesh does not
//! say where the cover ends: it widens the interval by its whole area and
//! makes the evidence approximate, rather than being guessed either way.
//! Cover smaller than the spacing of the samples goes unseen.
//!
//! A planar mesh is the object's shape, so its facade measures exactly. A
//! tessellated subject lies within its chord deviation `d` of the true
//! surface; each facade triangle's true patch then differs from it at most
//! inside a band of width `2d` along its edges, so the area widens by
//! `2·P·d + π·d²` for the facade triangles' summed perimeter `P`, and is
//! never a point. A tessellated body within reach, an unmeasured body
//! anywhere or an unmeasured or bodiless space could change which faces
//! count, so the measurement refuses.

use std::collections::BTreeSet;

use axiolid_core::{Point3, Ray3, Tolerance, Vec3};
use axiolid_measure::WindingMesh;
use axiolid_mesh::{TriMesh, audit_mesh};
use axiolid_ray_mesh::intersect_triangle;
use axioval_engine::{FacadeArea, FacadeAreaError, FacadeAreaService, GeometryFidelity};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Extent, Triangle, extent_gap, mesh_extent, triangles};

/// How far a face looks for a space or a body in front of it, in metres.
///
/// A room's space rarely starts farther from the wall that bounds it; a
/// courtyard or a neighbour closer than this still reads as outside, since
/// the first body met is not a space.
pub const REACH: f64 = 1.0;

/// How far outside a face its probe starts, so a neighbour flush with the
/// face contains the probe instead of touching it.
const PROBE_OFFSET: f64 = 1e-6;

/// Distance below which two hits or a point and a surface coincide, and the
/// tolerance handed to the kernel's predicates.
const ON_SURFACE: f64 = 1e-9;

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

/// What a face's probe meets first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Met {
    /// Another body: the face still looks outside.
    Other,
    /// A declared space: the face looks into a room.
    Space,
    /// The object itself: the face looks into its own body.
    Itself,
}

/// A body near the subject, ready to be probed.
struct Near<'a> {
    id: &'a ObjectId,
    met: Met,
    mesh: &'a TriMesh,
    triangles: Vec<Triangle>,
    extent: Extent,
}

/// Measures facade areas of registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidFacadeAreaService {
    geometry: AxiolidGeometry,
    spaces: BTreeSet<ObjectId>,
}

impl AxiolidFacadeAreaService {
    /// Creates a service over the supplied geometry, with no spaces yet.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self {
            geometry,
            spaces: BTreeSet::new(),
        }
    }

    /// Declares a space: a face looking into it faces the interior.
    #[must_use]
    pub fn with_space(mut self, space: ObjectId) -> Self {
        self.spaces.insert(space);
        self
    }

    /// Refuses when an unmeasured body, or a space without a body, could
    /// hide what a face looks at. Neither has a known place, so any face
    /// could be affected.
    fn refuse_unknown(&self) -> Result<(), FacadeAreaError> {
        if let Some((object, reason)) = self.geometry.unmeasured().next() {
            return Err(FacadeAreaError::Unavailable(format!(
                "{object} has a body that was not measured ({reason}), so what the faces look \
                 at is unknown"
            )));
        }
        if let Some(space) = self
            .spaces
            .iter()
            .find(|space| self.geometry.mesh(space).is_none())
        {
            return Err(FacadeAreaError::Unavailable(format!(
                "space {space} has no measured body, so which faces look into it is unknown"
            )));
        }
        Ok(())
    }

    /// The bodies within reach of `extent`, with the subject among them.
    fn near<'s>(
        &'s self,
        subject: &'s ObjectId,
        extent: &Extent,
    ) -> Result<Vec<Near<'s>>, FacadeAreaError> {
        let mut near = Vec::new();
        for (id, mesh) in self.geometry.objects() {
            let Some(body) = mesh_extent(mesh) else {
                continue;
            };
            if extent_gap(extent, &body, false) > REACH + ON_SURFACE {
                continue;
            }
            if id != subject && self.geometry.is_tessellated(id) {
                return Err(FacadeAreaError::Unavailable(format!(
                    "{id} is a tessellation within reach of {subject}, so which faces it \
                     covers is approximate"
                )));
            }
            let met = if id == subject {
                Met::Itself
            } else if self.spaces.contains(id) {
                Met::Space
            } else {
                Met::Other
            };
            near.push(Near {
                id,
                met,
                mesh,
                triangles: triangles(mesh),
                extent: body,
            });
        }
        Ok(near)
    }
}

impl FacadeAreaService for AxiolidFacadeAreaService {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        if self.geometry.has_no_body(object) {
            return Err(FacadeAreaError::Unavailable(format!(
                "{object} is declared to have no body"
            )));
        }
        self.refuse_unknown()?;
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        let tolerance = tolerance()?;
        let soup = triangles(mesh);
        if soup.is_empty() || !audit_mesh(mesh, tolerance).is_surface_usable() {
            return Err(FacadeAreaError::Unavailable(format!(
                "the mesh of {object} cannot be measured"
            )));
        }
        let extent = mesh_extent(mesh).ok_or_else(|| {
            FacadeAreaError::Unavailable(format!("the mesh of {object} is empty"))
        })?;
        let near = self.near(object, &extent)?;
        let windings = near
            .iter()
            .map(|body| {
                WindingMesh::prepare(body.mesh, tolerance).map_err(|error| {
                    FacadeAreaError::Unavailable(format!(
                        "winding number of {} unavailable: {error:?}",
                        body.id
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Area certainly and possibly on the facade, the summed perimeter of
        // their triangles, and how many faces each counted.
        let (mut certain, mut possible, mut perimeter) = (0.0, 0.0, 0.0);
        let (mut faces, mut partial) = (0usize, 0usize);
        for triangle in &soup {
            let [a, b, c] = *triangle;
            let normal = (b - a).cross(c - a);
            let doubled = normal.length();
            if doubled <= ON_SURFACE {
                continue;
            }
            // Steep faces only: a floor, a roof or a sill faces up or down.
            if normal.z.abs() / doubled > std::f64::consts::FRAC_1_SQRT_2 {
                continue;
            }
            let outward = Vec3::new(normal.x, normal.y, 0.0).normalize();
            let mut outside = 0;
            for sample in samples(*triangle) {
                let start = sample + outward * PROBE_OFFSET;
                if faces_outside(&near, &windings, start, outward)? {
                    outside += 1;
                }
            }
            match outside {
                0 => continue,
                SAMPLES => {
                    certain += doubled / 2.0;
                    faces += 1;
                }
                _ => {
                    possible += doubled / 2.0;
                    partial += 1;
                }
            }
            perimeter += (b - a).length() + (c - b).length() + (a - c).length();
        }

        let deviation = match self.geometry.fidelity(object) {
            Ok(GeometryFidelity::Exact) => None,
            Ok(GeometryFidelity::Tessellated {
                chord_deviation_metres,
            }) => Some(chord_deviation_metres),
            Err(_) => return Err(FacadeAreaError::InvalidMeasurement),
        };
        let band = deviation.map_or(0.0, |d| {
            // Never a point, even for a declared zero deviation.
            (2.0 * perimeter * d + std::f64::consts::PI * d * d)
                .max(f64::EPSILON * (certain + possible).max(1.0))
        });
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!("facade-area:{object}:faces={faces}:partial={partial}"),
            exact: deviation.is_none() && partial == 0,
        };
        let (lower, upper) = ((certain - band).max(0.0), certain + possible + band);
        FacadeArea::try_new(object.clone(), lower, upper, evidence)
    }

    fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        if self.geometry.has_no_body(object) {
            return Err(FacadeAreaError::Unavailable(format!(
                "{object} is declared to have no body"
            )));
        }
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        // A tessellated body's planes are chords of curved faces, so its
        // largest group of coplanar triangles is not a face of the object.
        if !matches!(self.geometry.fidelity(object), Ok(GeometryFidelity::Exact)) {
            return Err(FacadeAreaError::Unavailable(format!(
                "{object} is tessellated, so its plane faces are not certified"
            )));
        }
        let soup = triangles(mesh);
        if soup.is_empty() || !audit_mesh(mesh, tolerance()?).is_surface_usable() {
            return Err(FacadeAreaError::Unavailable(format!(
                "the mesh of {object} cannot be measured"
            )));
        }
        let (area, planes) = largest_plane(&soup);
        if area <= 0.0 {
            return Err(FacadeAreaError::Unavailable(format!(
                "the mesh of {object} has no face with area"
            )));
        }
        FacadeArea::try_new(
            object.clone(),
            area,
            area,
            Evidence {
                source: object.source.clone(),
                locator: format!("face-area:{object}:planes={planes}"),
                exact: true,
            },
        )
    }
}

/// A plane of the mesh: unit normal (sign fixed so its first
/// non-zero component is positive) and the offset along it.
struct Plane {
    normal: Vec3,
    offset: f64,
    area: f64,
}

/// The summed area of the triangles in the plane holding the most, and the
/// number of distinct planes. A triangle joins a plane when its normal is
/// parallel and every corner lies within [`ON_SURFACE`] of it, whichever way
/// it is wound.
fn largest_plane(soup: &[Triangle]) -> (f64, usize) {
    let mut planes: Vec<Plane> = Vec::new();
    for &[a, b, c] in soup {
        let cross = (b - a).cross(c - a);
        let doubled = cross.length();
        if doubled <= ON_SURFACE {
            continue;
        }
        let mut normal = cross / doubled;
        let flip = [normal.x, normal.y, normal.z]
            .into_iter()
            .find(|component| component.abs() > PARALLEL)
            .is_some_and(|component| component < 0.0);
        if flip {
            normal = -normal;
        }
        let on = |plane: &Plane| {
            plane.normal.dot(normal).abs() >= 1.0 - PARALLEL
                && [a, b, c].iter().all(|corner| {
                    (plane.normal.dot(Vec3::new(corner.x, corner.y, corner.z)) - plane.offset).abs()
                        <= ON_SURFACE
                })
        };
        match planes.iter_mut().find(|plane| on(plane)) {
            Some(plane) => plane.area += doubled / 2.0,
            None => planes.push(Plane {
                normal,
                offset: normal.dot(Vec3::new(a.x, a.y, a.z)),
                area: doubled / 2.0,
            }),
        }
    }
    let largest = planes.iter().map(|plane| plane.area).fold(0.0, f64::max);
    (largest, planes.len())
}

/// How far two unit normals may be from parallel and still share a plane.
const PARALLEL: f64 = 1e-12;

fn tolerance() -> Result<Tolerance, FacadeAreaError> {
    Tolerance::new(ON_SURFACE, ON_SURFACE)
        .map_err(|_| FacadeAreaError::Unavailable("invalid tolerance".into()))
}

/// How many points each face is classified at.
const SAMPLES: usize = 4;

/// The centroids of a face's four midpoint sub-triangles: spread over the
/// face, and away from its edges where neighbours meet it.
fn samples([a, b, c]: Triangle) -> [Point3; SAMPLES] {
    let (ab, bc, ca) = ((a + b) / 2.0, (b + c) / 2.0, (c + a) / 2.0);
    [
        (a + ab + ca) / 3.0,
        (ab + b + bc) / 3.0,
        (ca + bc + c) / 3.0,
        (ab + bc + ca) / 3.0,
    ]
}

/// Whether a face looks outside from a probe starting at `start`.
fn faces_outside(
    near: &[Near<'_>],
    windings: &[WindingMesh<'_, TriMesh>],
    start: Point3,
    outward: Vec3,
) -> Result<bool, FacadeAreaError> {
    if covered(near, windings, start)? {
        return Ok(false);
    }
    Ok(first_met(near, start, outward)?.is_none_or(|met| met == Met::Other))
}

/// Whether `point` lies inside a body other than the subject.
fn covered(
    near: &[Near<'_>],
    windings: &[WindingMesh<'_, TriMesh>],
    point: Point3,
) -> Result<bool, FacadeAreaError> {
    let probe: Extent = (point.to_array(), point.to_array());
    for (body, winding) in near.iter().zip(windings) {
        if body.met == Met::Itself || extent_gap(&probe, &body.extent, false) > ON_SURFACE {
            continue;
        }
        let number = winding.winding_number(point).map_err(|error| {
            FacadeAreaError::Unavailable(format!(
                "winding number of {} unavailable: {error:?}",
                body.id
            ))
        })?;
        if number.value.abs() >= INSIDE_WINDING {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What a horizontal ray from `start` meets first within [`REACH`]. Bodies
/// met at the same distance count as the one that excludes the face.
fn first_met(
    near: &[Near<'_>],
    start: Point3,
    direction: Vec3,
) -> Result<Option<Met>, FacadeAreaError> {
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let end = start + direction * REACH;
    let segment: Extent = (start.min(end).to_array(), start.max(end).to_array());
    let tolerance = tolerance()?;
    let mut first: Option<(f64, Met)> = None;
    for body in near {
        if extent_gap(&segment, &body.extent, false) > ON_SURFACE {
            continue;
        }
        for (index, triangle) in body.triangles.iter().enumerate() {
            let hit = intersect_triangle(&ray, *triangle, tolerance, index).map_err(|error| {
                FacadeAreaError::Unavailable(format!("ray intersection unavailable: {error:?}"))
            })?;
            let Some(t) = hit.map(|hit| hit.t).filter(|t| (0.0..=REACH).contains(t)) else {
                continue;
            };
            first = Some(match first {
                None => (t, body.met),
                Some((least, met)) if least < t - ON_SURFACE => (least, met),
                Some((least, _)) if t < least - ON_SURFACE => (t, body.met),
                Some((least, met)) => (least.min(t), met.max(body.met)),
            });
        }
    }
    Ok(first.map(|(_, met)| met))
}
