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
//! never a point. A face warped off its plane by a slab width `w` makes its
//! body a tessellation within `w` (engine#454), so the same band holds it:
//! `2·P·w` is at least `w` times each face's perimeter.
//!
//! Another body near the subject that is a tessellation within `d` may lie
//! anywhere within `d` of its mesh, which can change what a sample sees
//! only where the sample's probe (its start, and its ray up to the first
//! exact body it meets or [`REACH`]) comes within `d` of that mesh, and
//! meeting it there would change the reading (`Scene::settled_look`). Such
//! a sample is undecided: its face counts as possibly facade (in the upper
//! bound only, never the lower), as a partly covered face does, so the
//! interval holds every reading of the body. Every other sample reads the
//! true body as its mesh, and the area stays exact when no face is
//! undecided.
//! The outward faces (`measure_facade_faces`, `measure_enclosed_volume`)
//! close an envelope across their neighbours, which an undecided face
//! leaves unbounded, so they still refuse a tessellation within reach. An
//! unmeasured body anywhere or an unmeasured or bodiless space could
//! change which faces count, so every measurement refuses.
//!
//! Outward faces (of any slope, probed along their normal) meet the object
//! itself at any distance, so a reveal is never one, however wide its
//! opening. The envelope's volume closes across the reveals in front of a
//! window set back from the outer face: the pieces of a wall's faces that
//! look into the wall itself join the outward faces where, and only where,
//! they close open edges of them (`admit`); the closure proof still
//! decides.

use std::collections::BTreeSet;

use axiolid_core::{Point3, Ray3, Tolerance, Vec3};
use axiolid_measure::{WindingMesh, closest_point_on_triangle, closest_points_on_segments};
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
    /// The chord deviation of another body that is a tessellation, within
    /// which its true surface lies of its mesh; `None` for an exact body
    /// and for the subject, whose own deviation widens the area instead.
    deviation: Option<f64>,
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
    ///
    /// Another body that is a tessellation within chord deviation `d` lies
    /// up to `d` beyond its mesh, so it is within reach unless its mesh
    /// lies farther than the reach, the probe's offset and `d`. The facade
    /// area ([`Faces::Steep`]) takes it with `d` and leaves a sample its
    /// true body could change undecided ([`Look::Undecided`]); the outward
    /// faces ([`Faces::All`]) close an envelope across it, which no such
    /// sample bounds, so they refuse it.
    fn near<'s>(
        &'s self,
        subject: &'s ObjectId,
        extent: &Extent,
        faces: Faces,
    ) -> Result<Vec<Near<'s>>, FacadeAreaError> {
        let mut near = Vec::new();
        for (id, mesh) in self.geometry.objects() {
            let Some(body) = mesh_extent(mesh) else {
                continue;
            };
            let deviation = if id == subject {
                None
            } else {
                match self.geometry.fidelity(id) {
                    Ok(GeometryFidelity::Exact) => None,
                    Ok(GeometryFidelity::Tessellated {
                        chord_deviation_metres,
                    }) => Some(chord_deviation_metres),
                    Err(_) => return Err(FacadeAreaError::InvalidMeasurement),
                }
            };
            let reach = match (deviation, faces) {
                (Some(deviation), Faces::Steep) => REACH + PROBE_OFFSET + deviation + ON_SURFACE,
                _ => REACH + ON_SURFACE,
            };
            if extent_gap(extent, &body, false) > reach {
                continue;
            }
            if deviation.is_some() && faces == Faces::All {
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
                deviation,
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
}

/// What one sample of a face sees in front of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// Nothing, or another body first: the outside.
    Outside,
    /// The point in front lies in another body or a space.
    Covered,
    /// A space first: a room.
    Space,
    /// The subject's own body first: a reveal, a niche.
    Itself,
    /// A tessellated body near the probe could change what it sees: its
    /// true surface may contain the probe's start or cross its ray where
    /// its mesh does not, or miss them where its mesh meets them. Either
    /// reading may be the true one, so the sample counts as possibly
    /// outside, never surely ([`Faces::Steep`] only).
    Undecided,
}

/// A subject ready to be probed: its triangles, their winding, the bodies
/// near it and their winding numbers, and its chord deviation.
struct Scene<'s> {
    soup: Vec<Triangle>,
    /// `1` where the triangles are wound outward, `-1` where inward.
    orientation: f64,
    near: Vec<Near<'s>>,
    windings: Vec<WindingMesh<'s, TriMesh>>,
    deviation: Option<f64>,
    faces: Faces,
}

impl AxiolidFacadeAreaService {
    /// Probes `faces` of `object` and counts how many samples of each look
    /// outside; a face none of whose samples does is left out.
    fn classify(&self, object: &ObjectId, faces: Faces) -> Result<Classified, FacadeAreaError> {
        let scene = self.scene(object, faces)?;
        let mut looked = Vec::new();
        for (index, triangle) in scene.soup.iter().enumerate() {
            let Some(looks) = scene.looks(*triangle)? else {
                continue;
            };
            // A face with an undecided sample is possibly facade, never
            // surely: it counts with fewer than every sample outside.
            let outside = looks.iter().filter(|look| **look == Look::Outside).count();
            let undecided = looks.contains(&Look::Undecided);
            if outside > 0 || undecided {
                looked.push((index, outside));
            }
        }
        Ok(Classified {
            soup: scene.soup,
            looked,
            deviation: scene.deviation,
        })
    }

    /// The faces of `object` on the envelope, wound outward: those whose
    /// every sample looks outside, and the candidates that may close it
    /// across a reveal.
    ///
    /// A face that does not wholly look outside is cut along the faces of
    /// the other bodies and spaces lying in its plane (a window's side
    /// against its reveal), and each piece is probed again: a piece wholly
    /// outside joins the envelope, a piece held against a body or looking
    /// into a space is left out, and a piece looking only into the subject
    /// itself (a reveal between the outer face and a window set back) is a
    /// candidate. A piece some of whose samples look outside while others
    /// are covered refuses, as an unsplit face did.
    fn envelope_part(&self, object: &ObjectId) -> Result<EnvelopePart, FacadeAreaError> {
        let scene = self.scene(object, Faces::All)?;
        let mut part = EnvelopePart {
            deviation: scene.deviation,
            ..EnvelopePart::default()
        };
        let wound = |[a, b, c]: Triangle| {
            let face = if scene.orientation < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            };
            face.map(|point| point.to_array())
        };
        for triangle in &scene.soup {
            let Some(looks) = scene.looks(*triangle)? else {
                continue;
            };
            if looks.iter().all(|look| *look == Look::Outside) {
                part.sure.push(wound(*triangle));
                continue;
            }
            let covers = scene.covers(*triangle);
            let cells = if covers.is_empty() {
                vec![triangle.map(|point| point.to_array())]
            } else {
                split(triangle.map(|point| point.to_array()), &covers)
                    .map_err(FacadeAreaError::Unavailable)?
            };
            for cell in cells {
                let cell = cell.map(|[x, y, z]| Point3::new(x, y, z));
                let Some(looks) = scene.looks(cell)? else {
                    continue;
                };
                let outside = looks.contains(&Look::Outside);
                let held = looks
                    .iter()
                    .any(|look| matches!(look, Look::Covered | Look::Space));
                match (outside, held) {
                    (true, true) => {
                        return Err(FacadeAreaError::Unavailable(format!(
                            "a face of {object} only partly looks outside, so where the \
                             envelope runs is undecided"
                        )));
                    }
                    (_, true) => {}
                    (true, false) if looks.iter().all(|look| *look == Look::Outside) => {
                        part.sure.push(wound(cell));
                    }
                    _ => part.candidates.push(wound(cell)),
                }
            }
        }
        Ok(part)
    }

    /// `object` ready to be probed for `faces`.
    fn scene<'s>(
        &'s self,
        object: &'s ObjectId,
        faces: Faces,
    ) -> Result<Scene<'s>, FacadeAreaError> {
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
        let near = self.near(object, &extent, faces)?;
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
        let deviation = match self.geometry.fidelity(object) {
            Ok(GeometryFidelity::Exact) => None,
            Ok(GeometryFidelity::Tessellated {
                chord_deviation_metres,
            }) => Some(chord_deviation_metres),
            Err(_) => return Err(FacadeAreaError::InvalidMeasurement),
        };
        Ok(Scene {
            soup,
            orientation,
            near,
            windings,
            deviation,
            faces,
        })
    }
}

impl Scene<'_> {
    /// What each sample of `triangle`, wound as the subject's mesh is, sees
    /// in front of it; `None` for a face without area, or a face the
    /// classification does not probe.
    fn looks(&self, triangle: Triangle) -> Result<Option<[Look; SAMPLES]>, FacadeAreaError> {
        let [a, b, c] = triangle;
        let normal = (b - a).cross(c - a) * self.orientation;
        let doubled = normal.length();
        if doubled <= ON_SURFACE {
            return Ok(None);
        }
        let steep = normal.z.abs() / doubled <= std::f64::consts::FRAC_1_SQRT_2;
        let outward = match self.faces {
            // Steep faces only: a floor, a roof or a sill faces up or down.
            Faces::Steep if !steep => return Ok(None),
            Faces::Steep => Vec3::new(normal.x, normal.y, 0.0).normalize(),
            Faces::All => normal / doubled,
        };
        let mut looks = [Look::Outside; SAMPLES];
        for (look, sample) in looks.iter_mut().zip(samples(triangle)) {
            let start = sample + outward * PROBE_OFFSET;
            *look = if self.faces == Faces::Steep {
                self.settled_look(start, outward)?
            } else {
                look_from(&self.near, &self.windings, start, outward, self.faces)?
            };
        }
        Ok(Some(looks))
    }

    /// What the probe from `start` along `outward` sees, or
    /// [`Look::Undecided`] where a tessellated body near the subject could
    /// change it.
    ///
    /// A body declared within chord deviation `d` has its true surface
    /// within `d` of its mesh and its mesh within `d` of its true surface,
    /// so the true body and the mesh hold the same points farther than `d`
    /// from the mesh, and a set farther than `d` from the mesh meets
    /// neither surface. The probe reads such a body twice:
    ///
    /// - at its start, inside the body or not: decided unless the start
    ///   lies within `d` of the mesh, or inside an exact body, which covers
    ///   it whatever the tessellation does;
    /// - along its ray, by what it meets first. The exact bodies' first
    ///   meeting is known; the tessellated body changes the look only by
    ///   being met before it (or at it, within [`ON_SURFACE`]), and only
    ///   when that changes the reading: another body met first where the
    ///   exact bodies leave nothing or another body reads the same,
    ///   outside. Otherwise the ray up to that meeting (to [`REACH`] where
    ///   there is none) must lie farther than `d` from the mesh.
    ///
    /// Distances are taken within `d` plus [`ON_SURFACE`] for their
    /// rounding, and a distance the kernel cannot measure leaves the sample
    /// undecided. Boxes only rule triangles out: the gap between two boxes
    /// never exceeds the distance between what they hold. A decided sample
    /// reads every true body as its mesh, so it sees what the meshes show.
    fn settled_look(&self, start: Point3, outward: Vec3) -> Result<Look, FacadeAreaError> {
        let tessellated: Vec<&Near<'_>> = self
            .near
            .iter()
            .filter(|body| body.deviation.is_some())
            .collect();
        if tessellated.is_empty() {
            return look_from(&self.near, &self.windings, start, outward, self.faces);
        }
        let exact = |body: &Near<'_>| body.deviation.is_none();
        if covered(&self.near, &self.windings, start, exact)? {
            return Ok(Look::Covered);
        }
        let near = |body: &Near<'_>, to: Point3| {
            let margin = body.deviation.unwrap_or(0.0) + ON_SURFACE;
            let segment: Extent = (start.min(to).to_array(), start.max(to).to_array());
            extent_gap(&segment, &body.extent, false) <= margin
                && body.triangles.iter().any(|triangle| {
                    let boxed: Extent = (
                        triangle[0].min(triangle[1]).min(triangle[2]).to_array(),
                        triangle[0].max(triangle[1]).max(triangle[2]).to_array(),
                    );
                    extent_gap(&segment, &boxed, false) <= margin
                        && segment_gap(start, to, *triangle).is_none_or(|gap| gap <= margin)
                })
        };
        if tessellated.iter().any(|body| near(body, start)) {
            return Ok(Look::Undecided);
        }
        let first = first_met_at(&self.near, start, outward, self.faces, exact)?;
        let reach = first.map_or(REACH, |(t, _)| (t + ON_SURFACE).min(REACH));
        let to = start + outward * reach;
        let changes = |body: &Near<'_>| {
            body.met != Met::Other || first.is_some_and(|(_, met)| met != Met::Other)
        };
        if tessellated
            .iter()
            .any(|body| changes(body) && near(body, to))
        {
            return Ok(Look::Undecided);
        }
        look_from(&self.near, &self.windings, start, outward, self.faces)
    }

    /// The triangles of the other bodies and spaces near the subject that
    /// lie in the plane of `triangle` (every corner within [`ON_SURFACE`]
    /// of it) and whose box meets its box.
    fn covers(&self, triangle: Triangle) -> Vec<[[f64; 3]; 3]> {
        let face = triangle.map(|point| point.to_array());
        let normal = cross(minus(face[1], face[0]), minus(face[2], face[0]));
        let length = norm(normal);
        if length <= ON_SURFACE {
            return Vec::new();
        }
        let normal = normal.map(|value| value / length);
        let boxed = |points: &[[f64; 3]; 3]| -> Extent {
            (
                std::array::from_fn(|axis| points.iter().map(|p| p[axis]).fold(f64::MAX, f64::min)),
                std::array::from_fn(|axis| points.iter().map(|p| p[axis]).fold(f64::MIN, f64::max)),
            )
        };
        let extent = boxed(&face);
        let mut covers = Vec::new();
        for body in &self.near {
            if body.met == Met::Itself || extent_gap(&extent, &body.extent, false) > ON_SURFACE {
                continue;
            }
            for other in &body.triangles {
                let other = other.map(|point| point.to_array());
                if extent_gap(&extent, &boxed(&other), false) > ON_SURFACE {
                    continue;
                }
                let flat = other
                    .iter()
                    .all(|corner| dot(normal, minus(*corner, face[0])).abs() <= ON_SURFACE);
                if flat {
                    covers.push(other);
                }
            }
        }
        covers
    }
}

/// The faces one object adds to the envelope, wound outward: `sure` ones,
/// and `candidates` that join it only where they close it.
#[derive(Default)]
struct EnvelopePart {
    sure: Vec<[[f64; 3]; 3]>,
    candidates: Vec<[[f64; 3]; 3]>,
    deviation: Option<f64>,
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
            witness: None,
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
            witness: None,
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
        // Every face looking outside, wound outward, then the reveals that
        // close it, each with the chord deviation of its object.
        let (mut faces, mut candidates) = (Vec::new(), Vec::new());
        for object in &asked {
            let part = self.envelope_part(object)?;
            faces.extend(part.sure.into_iter().map(|face| (face, part.deviation)));
            candidates.extend(
                part.candidates
                    .into_iter()
                    .map(|face| (face, part.deviation)),
            );
        }
        let (mut faces, deviations): (Vec<_>, Vec<_>) = faces.into_iter().unzip();
        let (candidates, candidate_deviations): (Vec<_>, Vec<_>) = candidates.into_iter().unzip();
        let admitted = admit(&mut faces, &candidates).map_err(FacadeAreaError::Unavailable)?;
        let (mut band, mut exact) = (0.0_f64, true);
        let deviations = deviations
            .into_iter()
            .chain(admitted.iter().map(|index| candidate_deviations[*index]));
        for (face, deviation) in faces.iter().zip(deviations) {
            if let Some(deviation) = deviation {
                exact = false;
                let [a, b, c] = *face;
                let area = norm(cross(minus(b, a), minus(c, a))) / 2.0;
                let perimeter = norm(minus(b, a)) + norm(minus(c, b)) + norm(minus(a, c));
                band += area * deviation + perimeter * deviation * deviation;
            }
        }
        let origin = objects
            .first()
            .map(|object| self.geometry.local_origin(&object.source))
            .unwrap_or_default();
        let volume = enclosed(&faces, origin).map_err(FacadeAreaError::Unavailable)?;
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
            locator: if admitted.is_empty() {
                format!(
                    "enclosed-volume:objects={}:faces={}",
                    asked.len(),
                    faces.len()
                )
            } else {
                format!(
                    "enclosed-volume:objects={}:faces={}:reveal-faces={}",
                    asked.len(),
                    faces.len(),
                    admitted.len()
                )
            },
            exact,
            witness: None,
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
                witness: None,
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

/// What a face sees from a probe starting at `start`.
fn look_from(
    near: &[Near<'_>],
    windings: &[WindingMesh<'_, TriMesh>],
    start: Point3,
    outward: Vec3,
    faces: Faces,
) -> Result<Look, FacadeAreaError> {
    if covered(near, windings, start, |_| true)? {
        return Ok(Look::Covered);
    }
    Ok(match first_met(near, start, outward, faces)? {
        None | Some(Met::Other) => Look::Outside,
        Some(Met::Space) => Look::Space,
        Some(Met::Itself) => Look::Itself,
    })
}

/// Whether `point` lies inside a body other than the subject.
fn covered(
    near: &[Near<'_>],
    windings: &[WindingMesh<'_, TriMesh>],
    point: Point3,
    counted: impl Fn(&Near<'_>) -> bool,
) -> Result<bool, FacadeAreaError> {
    let probe: Extent = (point.to_array(), point.to_array());
    for (body, winding) in near.iter().zip(windings) {
        if body.met == Met::Itself
            || !counted(body)
            || extent_gap(&probe, &body.extent, false) > ON_SURFACE
        {
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

/// What a ray from `start` meets first within [`REACH`]. Bodies met at the
/// same distance count as the one that excludes the face.
///
/// An outward face ([`Faces::All`]) meets the subject itself at any
/// distance: a reveal looks across its opening into its own body however
/// wide the opening is.
fn first_met(
    near: &[Near<'_>],
    start: Point3,
    direction: Vec3,
    faces: Faces,
) -> Result<Option<Met>, FacadeAreaError> {
    Ok(first_met_at(near, start, direction, faces, |_| true)?.map(|(_, met)| met))
}

/// [`first_met`] among the bodies `counted` keeps, with the distance along
/// the ray at which it is met.
fn first_met_at(
    near: &[Near<'_>],
    start: Point3,
    direction: Vec3,
    faces: Faces,
    counted: impl Fn(&Near<'_>) -> bool,
) -> Result<Option<(f64, Met)>, FacadeAreaError> {
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let end = start + direction * REACH;
    let segment: Extent = (start.min(end).to_array(), start.max(end).to_array());
    let tolerance = tolerance()?;
    let mut first: Option<(f64, Met)> = None;
    for body in near.iter().filter(|body| counted(body)) {
        let unbounded = faces == Faces::All && body.met == Met::Itself;
        if !unbounded && extent_gap(&segment, &body.extent, false) > ON_SURFACE {
            continue;
        }
        let reach = if unbounded { f64::INFINITY } else { REACH };
        for (index, triangle) in body.triangles.iter().enumerate() {
            let hit = intersect_triangle(&ray, *triangle, tolerance, index).map_err(|error| {
                FacadeAreaError::Unavailable(format!("ray intersection unavailable: {error:?}"))
            })?;
            let Some(t) = hit.map(|hit| hit.t).filter(|t| (0.0..=reach).contains(t)) else {
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
    Ok(first)
}

/// The distance from the segment `[from, to]` to `triangle`; `None` where
/// the kernel cannot measure it.
///
/// A segment that crosses the triangle is at distance zero; otherwise the
/// nearest points pair an end of the segment with the triangle or an edge
/// of the triangle with the segment. A triangle without area is its edges,
/// which the edge distances already cover.
fn segment_gap(from: Point3, to: Point3, triangle: Triangle) -> Option<f64> {
    let tolerance = tolerance().ok()?;
    let ray = Ray3 {
        origin: from,
        direction: to - from,
    };
    let flat = (triangle[1] - triangle[0])
        .cross(triangle[2] - triangle[0])
        .length_squared()
        == 0.0;
    let mut gap = f64::INFINITY;
    if !flat {
        if from != to
            && intersect_triangle(&ray, triangle, tolerance, 0)
                .ok()?
                .is_some_and(|hit| (0.0..=1.0).contains(&hit.t))
        {
            return Some(0.0);
        }
        for point in [from, to] {
            let closest = closest_point_on_triangle(point, triangle).ok()?;
            gap = gap.min(closest.distance(point));
        }
    }
    for edge in 0..3 {
        let pair =
            closest_points_on_segments([triangle[edge], triangle[(edge + 1) % 3]], [from, to])
                .ok()?;
        gap = gap.min(pair.distance_squared.sqrt());
    }
    Some(gap)
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
pub(crate) fn enclosed(
    faces: &[[[f64; 3]; 3]],
    origin: axioval_engine::LocalOrigin,
) -> Result<(f64, f64), String> {
    if faces.len() * 3 > MAX_ENVELOPE_EDGES {
        return Err(format!(
            "the envelope has more than {MAX_ENVELOPE_EDGES} face edges to match, too many to \
             prove it closed"
        ));
    }
    // Reported in model coordinates (#372).
    if let Some([x, y, z]) = open_edge(faces).map(|edge| origin.point_to_model(edge)) {
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

/// At most this many pieces one face is cut into along the faces in its
/// plane.
const MAX_PIECES: usize = 1024;

/// `polygon`, a convex polygon, on each side of the plane through `at`
/// normal to `normal`: `(along, against)`. A corner within [`ON_SURFACE`]
/// of the plane lies on both sides.
fn cut(polygon: &[[f64; 3]], at: [f64; 3], normal: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let sides: Vec<f64> = polygon
        .iter()
        .map(|point| dot(normal, minus(*point, at)))
        .collect();
    if sides.iter().all(|side| *side >= -ON_SURFACE) {
        return (polygon.to_vec(), Vec::new());
    }
    if sides.iter().all(|side| *side <= ON_SURFACE) {
        return (Vec::new(), polygon.to_vec());
    }
    let (mut along, mut against) = (Vec::new(), Vec::new());
    for (index, (point, side)) in polygon.iter().zip(&sides).enumerate() {
        let next = (index + 1) % polygon.len();
        let (other, beyond) = (polygon[next], sides[next]);
        if *side >= -ON_SURFACE {
            along.push(*point);
        }
        if *side <= ON_SURFACE {
            against.push(*point);
        }
        let crosses = (*side > ON_SURFACE && beyond < -ON_SURFACE)
            || (*side < -ON_SURFACE && beyond > ON_SURFACE);
        if crosses {
            let share = side / (side - beyond);
            let crossing: [f64; 3] =
                std::array::from_fn(|axis| point[axis] + (other[axis] - point[axis]) * share);
            along.push(crossing);
            against.push(crossing);
        }
    }
    (along, against)
}

/// Twice the area of a planar polygon.
fn doubled_area(polygon: &[[f64; 3]]) -> f64 {
    let Some(first) = polygon.first() else {
        return 0.0;
    };
    let mut sum = [0.0; 3];
    for pair in polygon[1..].windows(2) {
        let term = cross(minus(pair[0], *first), minus(pair[1], *first));
        sum = std::array::from_fn(|axis| sum[axis] + term[axis]);
    }
    norm(sum)
}

/// `triangle` cut along the triangles `covers` lying in its plane, as
/// triangles wound as it is: each piece lies wholly inside or wholly
/// outside every cover, up to [`ON_SURFACE`].
fn split(triangle: [[f64; 3]; 3], covers: &[[[f64; 3]; 3]]) -> Result<Vec<[[f64; 3]; 3]>, String> {
    let [a, b, c] = triangle;
    let normal = cross(minus(b, a), minus(c, a));
    let length = norm(normal);
    if length <= ON_SURFACE {
        return Ok(Vec::new());
    }
    let normal = normal.map(|value| value / length);
    let kept = |polygon: &[[f64; 3]]| {
        polygon.len() >= 3 && doubled_area(polygon) > ON_SURFACE * ON_SURFACE
    };
    let mut pieces = vec![triangle.to_vec()];
    for cover in covers {
        // The cover's edges as planes across the face, each normal turned
        // towards the cover's inside.
        let mut planes = Vec::with_capacity(3);
        for edge in 0..3 {
            let (from, to, opposite) = (cover[edge], cover[(edge + 1) % 3], cover[(edge + 2) % 3]);
            let across = cross(normal, minus(to, from));
            let size = norm(across);
            if size <= ON_SURFACE {
                break;
            }
            let mut across = across.map(|value| value / size);
            if dot(across, minus(opposite, from)) < 0.0 {
                across = across.map(|value| -value);
            }
            planes.push((from, across));
        }
        if planes.len() < 3 {
            continue;
        }
        let mut next = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let (mut inside, mut outside) = (piece.clone(), Vec::new());
            for (at, across) in &planes {
                let (along, against) = cut(&inside, *at, *across);
                if kept(&against) {
                    outside.push(against);
                }
                inside = along;
                if !kept(&inside) {
                    break;
                }
            }
            if kept(&inside) {
                next.extend(outside);
                next.push(inside);
            } else {
                next.push(piece);
            }
        }
        pieces = next;
        if pieces.len() > MAX_PIECES {
            return Err(format!(
                "a face is cut into more than {MAX_PIECES} pieces by the faces in its plane, too \
                 many to place the envelope"
            ));
        }
    }
    Ok(pieces
        .iter()
        .flat_map(|piece| {
            (1..piece.len() - 1).map(move |index| [piece[0], piece[index], piece[index + 1]])
        })
        .collect())
}

/// An edge of a face, from its first point to its second.
type Edge = ([f64; 3], [f64; 3]);

/// The edges of `faces` longer than [`SEAM`].
fn edges(faces: &[[[f64; 3]; 3]]) -> Vec<Edge> {
    faces
        .iter()
        .flat_map(|[a, b, c]| [(*a, *b), (*b, *c), (*c, *a)])
        .filter(|(from, to)| norm(minus(*to, *from)) > SEAM)
        .collect()
}

/// A straight line through `start` along the unit `direction`.
#[derive(Clone, Copy)]
struct Line {
    start: [f64; 3],
    direction: [f64; 3],
}

impl Line {
    fn of((from, to): Edge) -> Self {
        let length = norm(minus(to, from));
        Self {
            start: from,
            direction: minus(to, from).map(|value| value / length),
        }
    }

    fn holds(&self, point: [f64; 3]) -> bool {
        norm(cross(minus(point, self.start), self.direction)) <= SEAM
    }

    fn holds_edge(&self, (from, to): &Edge) -> bool {
        self.holds(*from) && self.holds(*to)
    }

    /// `edge` as a stretch along the line, `1` where it runs along it and
    /// `-1` where against it.
    fn stretch(&self, (from, to): &Edge) -> (f64, f64, i32) {
        let (p, q) = (
            dot(minus(*from, self.start), self.direction),
            dot(minus(*to, self.start), self.direction),
        );
        (p.min(q), p.max(q), if q > p { 1 } else { -1 })
    }

    /// The length along the line that `edges` run more often one way than
    /// the other, counted as often as the excess.
    fn open_length<'e>(&self, edges: impl Iterator<Item = &'e Edge>) -> f64 {
        let stretches: Vec<(f64, f64, i32)> = edges
            .filter(|edge| self.holds_edge(edge))
            .map(|edge| self.stretch(edge))
            .collect();
        let mut stops: Vec<f64> = stretches.iter().flat_map(|(p, q, _)| [*p, *q]).collect();
        stops.sort_by(f64::total_cmp);
        stops.dedup_by(|later, earlier| *later - *earlier <= SEAM);
        stops
            .windows(2)
            .map(|pair| {
                let middle = f64::midpoint(pair[0], pair[1]);
                let runs: i32 = stretches
                    .iter()
                    .filter(|(p, q, _)| *p < middle && middle < *q)
                    .map(|(_, _, way)| way)
                    .sum();
                f64::from(runs.abs()) * (pair[1] - pair[0])
            })
            .sum()
    }
}

/// The representative of `index`'s group in the union-find `parent`.
fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

/// At most this many candidate face edges are matched against the
/// envelope.
const MAX_CANDIDATE_EDGES: usize = 12_000;

/// Adds to `faces` the groups of `candidates` that close it, and returns
/// the indices of the candidates added, in order.
///
/// Candidates are grouped by the edges they share (stretches along one line
/// overlapping by more than [`SEAM`]). A group joins only where it has
/// open edges and every one of them closes an open edge of the faces taken
/// so far: on its lines, the open length of both together is that of the
/// faces less that of the group. A reveal between a wall's outer face and a
/// window set back closes the hole in the one and the rim of the other; a
/// reveal on the room's side of the window, or a closed surface such as a
/// cavity, does not, and is left out.
fn admit(
    faces: &mut Vec<[[f64; 3]; 3]>,
    candidates: &[[[f64; 3]; 3]],
) -> Result<Vec<usize>, String> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let owned: Vec<(usize, Edge)> = candidates
        .iter()
        .enumerate()
        .flat_map(|(index, face)| {
            edges(std::slice::from_ref(face))
                .into_iter()
                .map(move |edge| (index, edge))
        })
        .collect();
    if owned.len() > MAX_CANDIDATE_EDGES {
        return Err(format!(
            "the envelope has more than {MAX_CANDIDATE_EDGES} edges of faces that may close it, \
             too many to match"
        ));
    }
    // Union-find over the candidates.
    let mut parent: Vec<usize> = (0..candidates.len()).collect();
    for (at, (first, edge)) in owned.iter().enumerate() {
        let line = Line::of(*edge);
        let (start, end, _) = line.stretch(edge);
        for (second, other) in &owned[at + 1..] {
            if first == second || !line.holds_edge(other) {
                continue;
            }
            let (from, to, _) = line.stretch(other);
            if end.min(to) - start.max(from) > SEAM {
                let (one, two) = (root(&mut parent, *first), root(&mut parent, *second));
                parent[one.max(two)] = one.min(two);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot: Vec<Option<usize>> = vec![None; candidates.len()];
    for index in 0..candidates.len() {
        let top = root(&mut parent, index);
        if let Some(group) = slot[top] {
            groups[group].push(index);
        } else {
            slot[top] = Some(groups.len());
            groups.push(vec![index]);
        }
    }
    let mut taken = edges(faces);
    let mut admitted = Vec::new();
    for group in groups {
        let own: Vec<Edge> = owned
            .iter()
            .filter(|(index, _)| group.binary_search(index).is_ok())
            .map(|(_, edge)| *edge)
            .collect();
        let mut lines: Vec<Line> = Vec::new();
        for edge in &own {
            if !lines.iter().any(|line| line.holds_edge(edge)) {
                lines.push(Line::of(*edge));
            }
        }
        let (mut theirs, mut ours, mut both) = (0.0, 0.0, 0.0);
        for line in &lines {
            theirs += line.open_length(taken.iter());
            ours += line.open_length(own.iter());
            both += line.open_length(taken.iter().chain(own.iter()));
        }
        let slack = SEAM * f64::from(u32::try_from(own.len()).unwrap_or(u32::MAX));
        if ours > slack && both <= theirs - ours + slack {
            taken.extend(own);
            faces.extend(group.iter().map(|index| candidates[*index]));
            admitted.extend(group);
        }
    }
    Ok(admitted)
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
        let (lower, upper) = enclosed(&faces, axioval_engine::LocalOrigin::default()).unwrap();
        assert!(
            lower <= 288.0 && 288.0 <= upper && upper - lower < 1e-5,
            "{lower} {upper}"
        );
        let open = &faces[..11];
        assert!(
            enclosed(open, axioval_engine::LocalOrigin::default())
                .unwrap_err()
                .contains("do not close")
        );
    }

    #[test]
    fn a_face_is_cut_into_pieces_wholly_inside_or_outside_a_cover() {
        let face = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
        let cover = [[1.0, 1.0, 0.0], [2.0, 1.0, 0.0], [1.0, 2.0, 0.0]];
        let pieces = split(face, &[cover]).unwrap();
        let area = |piece: &[[f64; 3]; 3]| doubled_area(piece) / 2.0;
        let total: f64 = pieces.iter().map(area).sum();
        assert!((total - 8.0).abs() < 1e-12, "{total}");
        // A piece lies inside every edge of the cover or wholly outside one,
        // and the pieces inside make up the cover.
        let sides = |point: [f64; 3]| [point[1] - 1.0, point[0] - 1.0, 3.0 - point[0] - point[1]];
        let mut covered = 0.0;
        for piece in &pieces {
            let within = |edge: usize, inward: bool| {
                piece.iter().all(|corner| {
                    let side = sides(*corner)[edge];
                    if inward {
                        side >= -1e-12
                    } else {
                        side <= 1e-12
                    }
                })
            };
            assert!(
                (0..3).all(|edge| within(edge, true)) || (0..3).any(|edge| within(edge, false)),
                "{piece:?}"
            );
            let centre: [f64; 3] =
                std::array::from_fn(|axis| piece.iter().map(|p| p[axis]).sum::<f64>() / 3.0);
            if sides(centre).iter().all(|side| *side > 0.0) {
                covered += area(piece);
            }
        }
        assert!((covered - 0.5).abs() < 1e-12, "{covered}");
        // A cover elsewhere leaves the face whole.
        let apart = [[5.0, 5.0, 0.0], [6.0, 5.0, 0.0], [5.0, 6.0, 0.0]];
        assert_eq!(split(face, &[apart]).unwrap(), vec![face]);
    }

    #[test]
    fn only_faces_closing_open_edges_are_admitted() {
        let cube = cube([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        let top: Vec<[[f64; 3]; 3]> = cube
            .iter()
            .filter(|face| face.iter().all(|corner| (corner[2] - 2.0).abs() < 1e-12))
            .copied()
            .collect();
        let open: Vec<[[f64; 3]; 3]> = cube
            .iter()
            .filter(|face| !top.contains(face))
            .copied()
            .collect();
        // Half the lid closes only part of the rim.
        let mut faces = open.clone();
        assert!(admit(&mut faces, &top[..1]).unwrap().is_empty());
        // A closed surface beside it closes nothing.
        let apart = cube_at([5.0, 0.0, 0.0]);
        assert!(admit(&mut faces, &apart).unwrap().is_empty());
        assert_eq!(faces.len(), open.len());
        // The whole lid closes it.
        let mut candidates = apart;
        candidates.extend(top.iter().copied());
        let admitted = admit(&mut faces, &candidates).unwrap();
        assert_eq!(admitted, vec![12, 13]);
        let (lower, upper) = enclosed(&faces, axioval_engine::LocalOrigin::default()).unwrap();
        assert!(lower <= 8.0 && 8.0 <= upper, "{lower} {upper}");
    }

    fn cube_at(min: [f64; 3]) -> Vec<[[f64; 3]; 3]> {
        cube(min, min.map(|value| value + 1.0))
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
        let (lower, upper) = enclosed(&faces, axioval_engine::LocalOrigin::default()).unwrap();
        assert!(lower <= 240.0 && 240.0 <= upper, "{lower} {upper}");
    }
}
