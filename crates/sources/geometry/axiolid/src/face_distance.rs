//! Signed distance from a body to one class of another body's faces.
//!
//! Answers [`axioval_engine::ProximityService::measure_face_distance`]: with
//! `F` the host's faces of the requested class, each point of the body has
//! the signed distance `+d(p, F)` inside the host and `-d(p, F)` outside it,
//! and the body's distance is the least of them (see
//! [`FaceDistanceEvidence`]). A cover is a positive distance, a protrusion a
//! negative one.
//!
//! - **Face classes** come from each host triangle's outward normal: top
//!   within 45° of up, bottom within 45° of down, side otherwise. The
//!   outward side is the one the host's signed volume says, so either
//!   winding is read correctly. A triangle within rounding of 45° refuses the
//!   measurement for a class it might or might not belong to.
//! - **A body wholly inside the host** has its distance exactly: the least
//!   triangle-to-triangle distance from the body to `F`. Inside is proven,
//!   not assumed: every body vertex lies inside the host by winding number
//!   and the body's surface stays clear of the host's. For planar meshes
//!   that is the exact answer.
//! - **Otherwise** the distance is bounded. Above by any point whose side is
//!   decided (a vertex clear of the host surface, signed by its winding
//!   number), by zero where the body reaches `F`, and by the body's distance
//!   to `F`. Below by minus the farthest the part outside the host can lie
//!   from `F`, since every point inside counts positive: that part lies in
//!   the hull of the body's vertices not proven inside, the points where the
//!   two surfaces cross and the host's vertices near the body; the distance
//!   to one face is convex, so no point of the hull lies farther from that
//!   face than its farthest spanning point, and the least of those over the
//!   faces bounds every point. That bound is exact when the outside part
//!   lies over one face triangle and may be wider over several.
//!
//! The host must be an exact closed solid: a tessellation's chords do not
//! state its true faces' normals. A tessellated body widens both bounds by
//! its chord deviation, and its vertices decide a side only when farther
//! than the deviation from the host surface.

use axiolid_measure::WindingMesh;
use axioval_engine::{
    Bounds3, FaceClass, FaceDistanceError, FaceDistanceEvidence, FaceDistanceRequest,
    ProximityError, SignedDistanceInterval,
};
use axioval_ir::Evidence;

use crate::geometry::{AxiolidGeometry, Triangle};
use crate::proximity::{
    AxiolidProximityService, Body, Indexed, LINEAR_TOLERANCE, crossings, inside, separation,
    soup_distance_above, surface_distance, tolerance, triangle_box, vertices,
};

/// How close to 45° a face's normal may come before its class is undecided.
const CLASS_TOLERANCE: f64 = 1e-9;

fn refusal(error: ProximityError) -> FaceDistanceError {
    match error {
        ProximityError::NoBody => FaceDistanceError::NoBody,
        ProximityError::InvalidMeasurement => FaceDistanceError::InvalidMeasurement,
        _ => FaceDistanceError::Unavailable,
    }
}

/// Measures the request.
pub(crate) fn measure(
    service: &AxiolidProximityService,
    geometry: &AxiolidGeometry,
    request: &FaceDistanceRequest,
) -> Result<FaceDistanceEvidence, FaceDistanceError> {
    let host = service.body(request.host()).map_err(refusal)?;
    if !geometry
        .fidelity(request.host())
        .map_err(refusal)?
        .is_exact()
    {
        return Err(FaceDistanceError::InexactHost);
    }
    if !host.solid {
        return Err(FaceDistanceError::NotClosed);
    }
    let body = service.body(request.body()).map_err(refusal)?;
    let fidelity = geometry.fidelity(request.body()).map_err(refusal)?;
    let faces = faces(&host, request.faces())?;
    let (lower, upper) = signed(&body, &host, &faces, fidelity.deviation_metres())?;
    let signed = SignedDistanceInterval::try_new(lower, upper)
        .map_err(|_| FaceDistanceError::InvalidMeasurement)?;
    FaceDistanceEvidence::try_new(
        request.clone(),
        signed,
        fidelity,
        Evidence {
            source: request.body().source.clone(),
            locator: format!(
                "axiolid:face-distance:{}:{}:{}",
                request.faces().name(),
                request.body(),
                request.host()
            ),
            exact: fidelity.is_exact(),
        },
    )
}

/// The host's triangles of `class`, indexed.
fn faces(host: &Body<'_>, class: FaceClass) -> Result<Indexed<Triangle>, FaceDistanceError> {
    // Twice the signed volume's sign says which way the winding faces.
    let orientation: f64 = host
        .soup
        .items
        .iter()
        .map(|[a, b, c]| a.dot(b.cross(*c)))
        .sum();
    if !orientation.is_finite() || orientation == 0.0 {
        return Err(FaceDistanceError::NotClosed);
    }
    let outward = orientation.signum();
    let limit = std::f64::consts::FRAC_1_SQRT_2;
    let mut selected = Vec::new();
    for triangle in &host.soup.items {
        let [a, b, c] = *triangle;
        let normal = (b - a).cross(c - a) * outward;
        let length = normal.length();
        if length == 0.0 || !length.is_finite() {
            continue;
        }
        let up = normal.z / length;
        let admitted = match class {
            FaceClass::Any => true,
            FaceClass::Top => up >= limit,
            FaceClass::Bottom => up <= -limit,
            FaceClass::Side => up.abs() < limit,
        };
        if class != FaceClass::Any && (up.abs() - limit).abs() <= CLASS_TOLERANCE {
            return Err(FaceDistanceError::AmbiguousFace);
        }
        if admitted {
            selected.push(*triangle);
        }
    }
    if selected.is_empty() {
        return Err(FaceDistanceError::NoFaces);
    }
    let boxes = selected.iter().map(triangle_box).collect();
    Indexed::build(selected, boxes).map_err(refusal)
}

/// `(lower, upper)` bounds on the signed distance, widened by the body's
/// chord deviation.
fn signed(
    body: &Body<'_>,
    host: &Body<'_>,
    faces: &Indexed<Triangle>,
    deviation: f64,
) -> Result<(f64, f64), FaceDistanceError> {
    let to_faces = separation(&body.soup, faces).map_err(refusal)?;
    let to_host = separation(&body.soup, &host.soup).map_err(refusal)?;
    // A vertex nearer the host surface than this may lie on either side of
    // it once the body's true surface is taken into account.
    let margin = LINEAR_TOLERANCE + deviation;
    let winding = WindingMesh::prepare(host.mesh, tolerance().map_err(refusal)?)
        .map_err(|_| FaceDistanceError::Unavailable)?;
    let mut all_inside = to_host > margin;
    let mut upper = f64::INFINITY;
    // Points spanning the part of the body outside the host: every vertex
    // not proven inside, and the points where the two surfaces cross.
    let mut outside = Vec::new();
    for point in vertices(body) {
        let clear = surface_distance(point, host).map_err(refusal)? > margin;
        if !clear {
            all_inside = false;
            outside.push(point);
            continue;
        }
        let from_faces = soup_distance_above(point, faces, 0.0).map_err(refusal)?;
        if inside(&winding, point).map_err(refusal)? {
            upper = upper.min(from_faces);
        } else {
            all_inside = false;
            upper = upper.min(-from_faces);
            outside.push(point);
        }
    }
    if all_inside {
        // Every point of the body lies inside the host and clear of its
        // surface, so the nearest point to the faces is on the body's surface.
        return Ok((to_faces - deviation, to_faces + deviation));
    }
    if to_faces <= LINEAR_TOLERANCE {
        upper = upper.min(0.0);
    }
    outside.extend(crossings(body, host).map_err(refusal)?);
    outside.extend(crossings(host, body).map_err(refusal)?);
    let reach = body.soup.bounds.expanded(margin);
    outside.extend(vertices(host).into_iter().filter(|point| {
        (0..3).all(|axis| (reach.min()[axis]..=reach.max()[axis]).contains(&point[axis]))
    }));
    // Inside the host every point counts positive, so only the outside part
    // can pull the distance below zero, and no further than it reaches.
    let farthest = farthest_bound(&outside, faces)?;
    // The body's point nearest the faces counts at most its distance.
    let upper = upper.min(to_faces);
    let lower = (-farthest).min(upper);
    Ok((lower - deviation, upper + deviation))
}

/// An upper bound on how far any point of the hull of `points` lies from
/// the faces: the least, over the faces, of the farthest point from it.
/// Zero for no points.
fn farthest_bound(
    points: &[axiolid_core::Point3],
    faces: &Indexed<Triangle>,
) -> Result<f64, FaceDistanceError> {
    let mut best = f64::INFINITY;
    for (face, bounds) in faces.items.iter().zip(&faces.boxes) {
        // A face whose box is already farther than the best from some
        // vertex cannot improve it.
        let mut farthest: f64 = 0.0;
        for point in points {
            let gap = Bounds3::try_new(point.to_array(), point.to_array())
                .map_err(refusal)?
                .gap(bounds);
            if gap >= best {
                farthest = f64::INFINITY;
                break;
            }
            let distance = axiolid_measure::closest_point_on_triangle(*point, *face)
                .map(|closest| closest.distance(*point))
                .map_err(|_| FaceDistanceError::Unavailable)?;
            farthest = farthest.max(distance);
            if farthest >= best {
                break;
            }
        }
        best = best.min(farthest);
    }
    if best.is_finite() {
        Ok(best)
    } else {
        Err(FaceDistanceError::InvalidMeasurement)
    }
}
