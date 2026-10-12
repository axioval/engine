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
use axioval_engine::{
    EnclosedVolume, FacadeArea, FacadeAreaError, FacadeAreaService, FacadeFace, FacadeFaceSide,
    FacadeFaces, GeometryFidelity,
};
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

/// Which faces a classification probes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Faces {
    /// Steep faces only, probed horizontally out of them as wound: the
    /// facade area.
    Steep,
    /// Every face of a closed body, probed along its normal turned out of
    /// the body: the outward faces.
    All,
}

/// How a subject's faces look: its triangles, the index and outside count
/// of each face probed, and its chord deviation when it is a tessellation.
struct Classified {
    soup: Vec<Triangle>,
    looked: Vec<(usize, usize)>,
    deviation: Option<f64>,
    /// `1` where the triangles are wound outward, `-1` where inward.
    orientation: f64,
}

impl AxiolidFacadeAreaService {
    /// Probes `faces` of `object` and counts how many samples of each look
    /// outside; a face none of whose samples does is left out.
    fn classify(&self, object: &ObjectId, faces: Faces) -> Result<Classified, FacadeAreaError> {
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
        let health = audit_mesh(mesh, tolerance);
        if soup.is_empty() || !health.is_surface_usable() {
            return Err(FacadeAreaError::Unavailable(format!(
                "the mesh of {object} cannot be measured"
            )));
        }
        // Outward faces need an outside: a closed body, its winding told
        // by the sign of the volume it encloses.
        let orientation = match faces {
            Faces::Steep => 1.0,
            Faces::All => {
                let volume = crate::face_normals::signed_volume(&soup);
                if !health.is_closed_two_manifold() || volume == 0.0 || !volume.is_finite() {
                    return Err(FacadeAreaError::Unavailable(format!(
                        "the mesh of {object} encloses no volume, so its faces have no outside"
                    )));
                }
                volume.signum()
            }
        };
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
        let mut looked = Vec::new();
        for (index, triangle) in soup.iter().enumerate() {
            let [a, b, c] = *triangle;
            let normal = (b - a).cross(c - a) * orientation;
            let doubled = normal.length();
            if doubled <= ON_SURFACE {
                continue;
            }
            let steep = normal.z.abs() / doubled <= std::f64::consts::FRAC_1_SQRT_2;
            let outward = match faces {
                // Steep faces only: a floor, a roof or a sill faces up or down.
                Faces::Steep if !steep => continue,
                Faces::Steep => Vec3::new(normal.x, normal.y, 0.0).normalize(),
                Faces::All => normal / doubled,
            };
            let mut outside = 0;
            for sample in samples(*triangle) {
                let start = sample + outward * PROBE_OFFSET;
                if faces_outside(&near, &windings, start, outward)? {
                    outside += 1;
                }
            }
            if outside > 0 {
                looked.push((index, outside));
            }
        }
        let deviation = match self.geometry.fidelity(object) {
            Ok(GeometryFidelity::Exact) => None,
            Ok(GeometryFidelity::Tessellated {
                chord_deviation_metres,
            }) => Some(chord_deviation_metres),
            Err(_) => return Err(FacadeAreaError::InvalidMeasurement),
        };
        Ok(Classified {
            soup,
            looked,
            deviation,
            orientation,
        })
    }
}

impl FacadeAreaService for AxiolidFacadeAreaService {
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let Classified {
            soup,
            looked,
            deviation,
            ..
        } = self.classify(object, Faces::Steep)?;
        // Area certainly and possibly on the facade, the summed perimeter of
        // their triangles, and how many faces each counted.
        let (mut certain, mut possible, mut perimeter) = (0.0, 0.0, 0.0);
        let (mut faces, mut partial) = (0usize, 0usize);
        for (index, outside) in looked {
            let [a, b, c] = soup[index];
            let doubled = (b - a).cross(c - a).length();
            if outside == SAMPLES {
                certain += doubled / 2.0;
                faces += 1;
            } else {
                possible += doubled / 2.0;
                partial += 1;
            }
            perimeter += (b - a).length() + (c - b).length() + (a - c).length();
        }
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

    fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let Classified {
            soup,
            looked,
            deviation,
            ..
        } = self.classify(object, Faces::All)?;
        let boundary = crate::face_normals::boundary(object, &soup, deviation)
            .map_err(|error| FacadeAreaError::Unavailable(error.to_string()))?;
        let mut faces = Vec::new();
        let mut partial = 0usize;
        for (index, outside) in looked {
            let Ok(at) = boundary.binary_search_by_key(&index, |(index, ..)| *index) else {
                // Degenerate in the certified normals: it covers nothing.
                continue;
            };
            let (_, normal, area) = boundary[at];
            let [a, b, c] = soup[index];
            let centroid = (a + b + c) / 3.0;
            let side = if outside == SAMPLES {
                FacadeFaceSide::Outside
            } else {
                partial += 1;
                FacadeFaceSide::Partly
            };
            faces.push(FacadeFace::try_new(
                normal,
                area,
                centroid.to_array(),
                side,
            )?);
        }
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!(
                "facade-faces:{object}:faces={}:partial={partial}",
                faces.len()
            ),
            exact: deviation.is_none(),
        };
        FacadeFaces::try_new(object.clone(), faces, evidence)
    }

    fn measure_enclosed_volume(
        &self,
        objects: &[ObjectId],
    ) -> Result<EnclosedVolume, FacadeAreaError> {
        let mut asked = objects.to_vec();
        asked.sort();
        asked.dedup();
        if asked.is_empty() {
            return Err(FacadeAreaError::Unavailable(
                "no objects bound the envelope".into(),
            ));
        }
        // Every face looking outside, wound outward, and the chord band of
        // the tessellated ones.
        let (mut faces, mut band, mut exact) = (Vec::new(), 0.0_f64, true);
        for object in &asked {
            let Classified {
                soup,
                looked,
                deviation,
                orientation,
            } = self.classify(object, Faces::All)?;
            for (index, outside) in looked {
                if outside != SAMPLES {
                    return Err(FacadeAreaError::Unavailable(format!(
                        "a face of {object} only partly looks outside, so where the envelope \
                         runs is undecided"
                    )));
                }
                let [a, b, c] = soup[index];
                let face = if orientation < 0.0 {
                    [a, c, b]
                } else {
                    [a, b, c]
                };
                if let Some(deviation) = deviation {
                    exact = false;
                    let area = (b - a).cross(c - a).length() / 2.0;
                    let perimeter = (b - a).length() + (c - b).length() + (a - c).length();
                    band += area * deviation + perimeter * deviation * deviation;
                }
                faces.push(face.map(|point| point.to_array()));
            }
        }
        let volume = enclosed(&faces).map_err(FacadeAreaError::Unavailable)?;
        let band = band * (1.0 + 8.0 * f64::EPSILON);
        let (lower, upper) = (
            (volume.0 - band).next_down().max(0.0),
            (volume.1 + band).next_up(),
        );
        if volume.1 <= 0.0 {
            return Err(FacadeAreaError::Unavailable(
                "the faces looking outside enclose no volume".into(),
            ));
        }
        let evidence = Evidence {
            source: asked[0].source.clone(),
            locator: format!(
                "enclosed-volume:objects={}:faces={}",
                asked.len(),
                faces.len()
            ),
            exact,
        };
        EnclosedVolume::try_new(asked, lower, upper, evidence)
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

/// At most this many face edges are matched to prove an envelope closed.
const MAX_ENVELOPE_EDGES: usize = 30_000;

/// How far apart two points of an envelope's edges may lie and still meet,
/// in metres.
const SEAM: f64 = 1e-9;

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// The volume the outward-wound `faces` enclose, `[lower, upper]`, or why
/// they enclose none.
///
/// The faces close where every stretch of every edge is run as often one
/// way as the other by the faces' edges on its line (a T-junction
/// included): their boundary is then empty, and the divergence theorem
/// gives the volume as the sum of the signed tetrahedra over the faces.
/// Edges meet within [`SEAM`]; the volume a gap that thin may hide, along
/// every edge at the faces' distance, widens the interval with the
/// arithmetic's rounding.
pub(crate) fn enclosed(faces: &[[[f64; 3]; 3]]) -> Result<(f64, f64), String> {
    if faces.len() * 3 > MAX_ENVELOPE_EDGES {
        return Err(format!(
            "the envelope has more than {MAX_ENVELOPE_EDGES} face edges to match, too many to \
             prove it closed"
        ));
    }
    if let Some([x, y, z]) = open_edge(faces) {
        return Err(format!(
            "the faces looking outside do not close: nothing lies beyond the edge at \
             ({x:.3}, {y:.3}, {z:.3})"
        ));
    }
    let origin = faces.first().map_or([0.0; 3], |face| face[0]);
    let (mut volume, mut magnitude, mut reach, mut edges) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for face in faces {
        let [a, b, c] = face.map(|point| minus(point, origin));
        let term = dot(a, cross(b, c)) / 6.0;
        volume += term;
        magnitude += norm(a) * norm(b) * norm(c) / 6.0;
        reach = reach.max(norm(a)).max(norm(b)).max(norm(c));
        edges += norm(minus(b, a)) + norm(minus(c, b)) + norm(minus(a, c));
    }
    // Each tetrahedron rounds within a few ulps of its corners' product,
    // the running sum within a few per term; a seam hides at most a strip `SEAM`
    // wide along every edge, as far out as the farthest corner.
    let count = f64::from(u32::try_from(faces.len()).unwrap_or(u32::MAX));
    let slack = (count + 8.0) * 4.0 * f64::EPSILON * magnitude + SEAM * edges * reach / 3.0;
    Ok(((volume - slack).next_down(), (volume + slack).next_up()))
}

/// A point on an edge of `faces` that nothing lies beyond: run more often
/// one way than the other along its line. `None` where the faces close.
fn open_edge(faces: &[[[f64; 3]; 3]]) -> Option<[f64; 3]> {
    let edges: Vec<([f64; 3], [f64; 3])> = faces
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .filter(|(from, to)| norm(minus(*to, *from)) > SEAM)
        .collect();
    let mut placed = vec![false; edges.len()];
    for first in 0..edges.len() {
        if placed[first] {
            continue;
        }
        let (start, end) = edges[first];
        let length = norm(minus(end, start));
        let direction = minus(end, start).map(|value| value / length);
        let on_line = |point: [f64; 3]| norm(cross(minus(point, start), direction)) <= SEAM;
        // The edges on this line, as stretches along it, each `1` where it
        // runs along the line's direction and `-1` where against it.
        let mut stretches: Vec<(f64, f64, i32)> = Vec::new();
        for (at, (from, to)) in edges.iter().enumerate().skip(first) {
            if placed[at] || !on_line(*from) || !on_line(*to) {
                continue;
            }
            placed[at] = true;
            let (p, q) = (
                dot(minus(*from, start), direction),
                dot(minus(*to, start), direction),
            );
            stretches.push((p.min(q), p.max(q), if q > p { 1 } else { -1 }));
        }
        let mut stops: Vec<f64> = stretches.iter().flat_map(|(p, q, _)| [*p, *q]).collect();
        stops.sort_by(f64::total_cmp);
        stops.dedup_by(|later, earlier| *later - *earlier <= SEAM);
        for pair in stops.windows(2) {
            let middle = f64::midpoint(pair[0], pair[1]);
            let runs: i32 = stretches
                .iter()
                .filter(|(p, q, _)| *p < middle && middle < *q)
                .map(|(_, _, way)| way)
                .sum();
            if runs != 0 {
                return Some(std::array::from_fn(|axis| {
                    start[axis] + direction[axis] * middle
                }));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The twelve outward triangles of the box `min` to `max`.
    fn cube(min: [f64; 3], max: [f64; 3]) -> Vec<[[f64; 3]; 3]> {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        let p = [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1],
        ];
        [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ]
        .iter()
        .map(|corners: &[usize; 3]| corners.map(|corner| p[corner]))
        .collect()
    }

    #[test]
    fn a_closed_box_encloses_its_volume_and_an_open_one_nothing() {
        let faces = cube([0.0, 0.0, 0.0], [10.0, 8.0, 3.6]);
        let (lower, upper) = enclosed(&faces).unwrap();
        assert!(
            lower <= 288.0 && 288.0 <= upper && upper - lower < 1e-5,
            "{lower} {upper}"
        );
        let open = &faces[..11];
        assert!(enclosed(open).unwrap_err().contains("do not close"));
    }

    #[test]
    fn faces_meeting_at_a_t_junction_still_close() {
        // The box's top split in two along x = 4, its two halves meeting
        // the side faces' single edges.
        let mut faces = cube([0.0, 0.0, 0.0], [10.0, 8.0, 3.0]);
        faces.retain(|face| !face.iter().all(|corner| (corner[2] - 3.0).abs() < 1e-12));
        for (x0, x1) in [(0.0, 4.0), (4.0, 10.0)] {
            faces.push([[x0, 0.0, 3.0], [x1, 0.0, 3.0], [x1, 8.0, 3.0]]);
            faces.push([[x0, 0.0, 3.0], [x1, 8.0, 3.0], [x0, 8.0, 3.0]]);
        }
        let (lower, upper) = enclosed(&faces).unwrap();
        assert!(lower <= 240.0 && 240.0 <= upper, "{lower} {upper}");
    }
}
