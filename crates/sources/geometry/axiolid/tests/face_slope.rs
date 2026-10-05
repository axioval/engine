//! Face normals over real Axiolid geometry: a planar sloped slab, a warped
//! surface, a tessellated embankment and the ramp fixtures.

use std::f64::consts::PI;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{
    AxiolidGeometry, AxiolidVerticalExtentService, AxiolidWalkingSurfaceService,
};
use axioval_engine::{
    FaceNormal, FacePiece, FacePieceSet, SurfaceFace, VerticalExtentError, VerticalExtentService,
    WalkingSurfaceService,
};
use axioval_ir::{ObjectId, SourceId};

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// The steepest-gradient angle of each normal box's centre, and whether
/// every box is a point.
fn angles(normals: &[FaceNormal]) -> Vec<f64> {
    normals
        .iter()
        .map(|normal| {
            let (lower, upper) = (normal.lower(), normal.upper());
            let [x, y, z] = [0, 1, 2].map(|axis| f64::midpoint(lower[axis], upper[axis]));
            x.hypot(y).atan2(z.abs())
        })
        .collect()
}

/// The slope interval a box allows, from its corners: sound for these
/// fixtures, whose boxes are far from the vertical.
fn slope_range(normal: &FaceNormal) -> (f64, f64) {
    let (lower, upper) = (normal.lower(), normal.upper());
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    for corner in 0..8 {
        let pick = |axis: usize| {
            if corner & (1 << axis) == 0 {
                lower[axis]
            } else {
                upper[axis]
            }
        };
        let slope = pick(0).hypot(pick(1)).atan2(pick(2).abs());
        range = (range.0.min(slope), range.1.max(slope));
    }
    range
}

/// A closed box 4 m along x and 3 m along y whose top rises `rise` along x
/// from 0.3 m, outward-oriented, on a flat bottom.
fn sloped_slab(rise: f64) -> TriMesh {
    let points = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(4.0, 0.0, 0.0),
        Point3::new(4.0, 3.0, 0.0),
        Point3::new(0.0, 3.0, 0.0),
        Point3::new(0.0, 0.0, 0.3),
        Point3::new(4.0, 0.0, 0.3 + rise),
        Point3::new(4.0, 3.0, 0.3 + rise),
        Point3::new(0.0, 3.0, 0.3),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // bottom, facing down
        4, 5, 6, 4, 6, 7, // top, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

/// An open grid of `n` × `n` cells over `[0, size]²` at heights `height`.
fn sheet(n: u32, size: f64, height: impl Fn(f64, f64) -> f64) -> TriMesh {
    let mut points = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (x, y) = (
                size * f64::from(i) / f64::from(n),
                size * f64::from(j) / f64::from(n),
            );
            points.push(Point3::new(x, y, height(x, y)));
        }
    }
    let mut indices = Vec::new();
    let at = |i: u32, j: u32| j * (n + 1) + i;
    for j in 0..n {
        for i in 0..n {
            indices.extend([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            indices.extend([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    TriMesh::new(points, indices)
}

#[test]
fn a_planar_sloped_slab_measures_its_top_and_bottom() {
    // 0.4 over 4 m: exactly one in ten along x, with exact vertices.
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("slab"), sloped_slab(0.4)),
    );
    let top = service
        .measure_face_normals(&id("slab"), SurfaceFace::Top)
        .unwrap();
    assert_eq!(top.normals().len(), 2);
    for normal in top.normals() {
        let (low, high) = slope_range(normal);
        let expected = (0.4_f64 / 4.0).atan();
        assert!(
            low <= expected + 1e-15 && expected - 1e-15 <= high,
            "{low} {high}"
        );
        assert!(high - low < 1e-12);
    }
    let bottom = service
        .measure_face_normals(&id("slab"), SurfaceFace::Bottom)
        .unwrap();
    assert!(bottom.evidence().exact);
    assert!(angles(bottom.normals()).iter().all(|angle| *angle == 0.0));
    // Exact vertices on a level bottom: an exact zero slope.
    assert!(
        bottom
            .normals()
            .iter()
            .all(|normal| normal.lower()[0] == 0.0
                && normal.upper()[0] == 0.0
                && normal.lower()[1] == 0.0
                && normal.upper()[1] == 0.0)
    );
}

#[test]
fn a_mesh_wound_inside_out_reads_the_same_faces() {
    let outward = sloped_slab(0.4);
    let points: Vec<Point3> = (0..8).map(|i| outward.positions[i]).collect();
    let indices: Vec<u32> = outward
        .indices
        .chunks(3)
        .flat_map(|triangle| [triangle[0], triangle[2], triangle[1]])
        .collect();
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new()
            .with_mesh(id("in"), TriMesh::new(points, indices))
            .with_mesh(id("out"), outward),
    );
    let face = |local: &str| {
        service
            .measure_face_normals(&id(local), SurfaceFace::Top)
            .unwrap()
            .normals()
            .len()
    };
    assert_eq!(face("in"), face("out"));
    let top = service
        .measure_face_normals(&id("in"), SurfaceFace::Top)
        .unwrap();
    assert!(top.normals().iter().all(|normal| normal.lower()[2] > 0.0));
}

#[test]
fn a_warped_surface_spans_the_slopes_of_its_pieces() {
    // A hyperbolic paraboloid z = 0.2·x·y over 2 m × 2 m: level at the
    // origin, steepest at the far corner, gradient 0.2·(y, x).
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("warped"), sheet(8, 2.0, |x, y| 0.2 * x * y)),
    );
    let top = service
        .measure_face_normals(&id("warped"), SurfaceFace::Top)
        .unwrap();
    // An open surface: both faces are every piece.
    assert_eq!(top.normals().len(), 128);
    let bottom = service
        .measure_face_normals(&id("warped"), SurfaceFace::Bottom)
        .unwrap();
    assert_eq!(bottom.normals().len(), 128);
    let slopes = angles(top.normals());
    let (least, most) = slopes
        .iter()
        .fold((f64::INFINITY, 0.0_f64), |(low, high), s| {
            (low.min(*s), high.max(*s))
        });
    assert!(least < 0.05 && most > 0.45 && most < (0.2_f64 * 8.0_f64.sqrt()).atan());
}

#[test]
fn a_tessellated_embankment_holds_its_true_slope_and_is_never_exact() {
    // A one-in-two embankment, tessellated within 1 mm.
    let service = AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_tessellated_mesh(
        id("embankment"),
        sheet(4, 4.0, |x, _| 0.5 * x),
        0.001,
    ));
    let top = service
        .measure_face_normals(&id("embankment"), SurfaceFace::Top)
        .unwrap();
    assert!(!top.evidence().exact);
    let expected = 0.5_f64.atan();
    for normal in top.normals() {
        assert!(!normal.is_exact());
        let (low, high) = slope_range(normal);
        assert!(low < expected && expected < high, "{low} {high}");
        assert!(high - low < 0.01);
    }
    // Too coarse: a 5 cm deviation over 1 m triangles cannot bound a slope.
    let coarse = AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_tessellated_mesh(
        id("coarse"),
        sheet(4, 4.0, |x, _| 0.5 * x),
        0.5,
    ));
    assert!(matches!(
        coarse.measure_face_normals(&id("coarse"), SurfaceFace::Top),
        Err(VerticalExtentError::Unavailable(reason)) if reason.contains("too coarsely")
    ));
}

#[test]
fn a_body_without_the_face_is_refused() {
    // A vertical wall sheet has no top.
    let wall = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 1.0),
            Point3::new(0.0, 0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );
    let service =
        AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_mesh(id("wall"), wall));
    assert!(matches!(
        service.measure_face_normals(&id("wall"), SurfaceFace::Top),
        Err(VerticalExtentError::Unavailable(reason)) if reason.contains("no triangle")
    ));
}

/// A ramp side profile: `rise` over `length` between two 1 m landings, on
/// a base 0.1 m thick, as in the walking-surface fixtures.
fn ramp(rise: f64, length: f64) -> TriMesh {
    let profile = [
        [0.0, 0.0],
        [length + 2.0, 0.0],
        [length + 2.0, 0.1 + rise],
        [length + 1.0, 0.1 + rise],
        [1.0, 0.1],
        [0.0, 0.1],
    ];
    let width = 1.5;
    let n = profile.len();
    let mut points: Vec<Point3> = profile
        .iter()
        .map(|p| Point3::new(p[0], 0.0, p[1]))
        .collect();
    points.extend(profile.iter().map(|p| Point3::new(p[0], width, p[1])));
    // The profile, fanned from its corner on the base; the far side wound
    // the other way, then the sides.
    let fan = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]];
    let mut indices = Vec::new();
    for [a, b, c] in fan {
        indices.extend([a, b, c].map(|i: usize| u32::try_from(i).unwrap()));
        indices.extend([a + n, c + n, b + n].map(|i: usize| u32::try_from(i).unwrap()));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n].map(|k| u32::try_from(k).unwrap()));
    }
    TriMesh::new(points, indices)
}

#[test]
fn the_ramp_run_slope_agrees_with_the_steepest_piece_of_the_top() {
    for (rise, length) in [(0.5, 6.0), (0.5, 3.0), (0.35, 4.2)] {
        let geometry = AxiolidGeometry::new().with_mesh(id("ramp"), ramp(rise, length));
        let runs = AxiolidWalkingSurfaceService::new(geometry.clone())
            .measure_sloped_runs(&id("ramp"))
            .unwrap();
        let run = runs.runs()[0].slope();
        let top = AxiolidVerticalExtentService::new(geometry)
            .measure_face_normals(&id("ramp"), SurfaceFace::Top)
            .unwrap();
        let ranges: Vec<(f64, f64)> = top.normals().iter().map(slope_range).collect();
        // The landings are level; the run is the steepest piece, and its
        // angle is the run's slope as an angle.
        assert!(ranges.iter().any(|(low, _)| *low == 0.0));
        let steepest = ranges.iter().fold(0.0_f64, |high, range| high.max(range.1));
        let (low, high) = (run.lower().atan(), run.upper().atan());
        assert!((steepest - high).abs() < 1e-12 && (steepest - low).abs() < 1e-12);
        assert!(steepest < PI / 2.0);
    }
}

/// A prism of the (x, z) `profile` (anticlockwise, convex) extruded
/// `width` along y.
fn prism(profile: &[[f64; 2]], width: f64) -> TriMesh {
    let n = profile.len();
    let mut points: Vec<Point3> = profile
        .iter()
        .map(|p| Point3::new(p[0], 0.0, p[1]))
        .collect();
    points.extend(profile.iter().map(|p| Point3::new(p[0], width, p[1])));
    let index = |i: usize| u32::try_from(i).unwrap();
    let mut indices = Vec::new();
    for i in 1..n - 1 {
        indices.extend([0, i, i + 1].map(index));
        indices.extend([n, n + i + 1, n + i].map(index));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n].map(index));
    }
    TriMesh::new(points, indices)
}

/// A fill 9 m wide at its base and 3 m at its 2 m high crest, 10 m long:
/// a level crest between two batters falling 1:1.5.
fn embankment() -> TriMesh {
    prism(&[[-4.5, 0.0], [4.5, 0.0], [1.5, 2.0], [-1.5, 2.0]], 10.0)
}

/// The slope range over a piece's parts.
fn piece_slopes(piece: &FacePiece) -> (f64, f64) {
    piece
        .normals()
        .iter()
        .map(slope_range)
        .fold((f64::INFINITY, 0.0_f64), |(low, high), (l, h)| {
            (low.min(l), high.max(h))
        })
}

#[test]
fn a_single_body_embankment_has_a_crest_and_two_batters() {
    let batter = (2.0_f64 / 3.0).atan();
    let batter_area = 13.0_f64.sqrt() * 10.0;
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new()
            .with_mesh(id("exact"), embankment())
            .with_tessellated_mesh(id("meshed"), embankment(), 0.001),
    );
    let top = service
        .measure_face_pieces(&id("exact"), FacePieceSet::Top)
        .unwrap();
    assert!(top.evidence().exact);
    // Two triangles each: one level crest and two batters.
    assert_eq!(top.pieces().len(), 3, "{top:#?}");
    let mut level = 0;
    for piece in top.pieces() {
        assert_eq!(piece.normals().len(), 2);
        let (low, high) = piece_slopes(piece);
        let (least, most) = piece.area_square_metres();
        let expected = if high == 0.0 {
            level += 1;
            30.0
        } else {
            assert!(low <= batter + 1e-15 && batter - 1e-15 <= high && high - low < 1e-12);
            batter_area
        };
        assert!(least <= expected && expected <= most && most - least < 1e-9);
    }
    assert_eq!(level, 1);
    // The whole boundary adds the base and the two ends.
    let boundary = service
        .measure_face_pieces(&id("exact"), FacePieceSet::Boundary)
        .unwrap();
    assert_eq!(boundary.pieces().len(), 6);

    // Tessellated: the same pieces, never exact, every interval holding
    // the true slope and area.
    let meshed = service
        .measure_face_pieces(&id("meshed"), FacePieceSet::Top)
        .unwrap();
    assert!(!meshed.evidence().exact);
    assert_eq!(meshed.pieces().len(), 3);
    for piece in meshed.pieces() {
        assert!(piece.normals().iter().all(|normal| !normal.is_exact()));
        let (low, high) = piece_slopes(piece);
        let (least, most) = piece.area_square_metres();
        assert!(least < most);
        if low > 0.1 {
            assert!(low < batter && batter < high && high - low < 0.01);
            assert!(least < batter_area && batter_area < most);
        } else {
            assert!(least < 30.0 && 30.0 < most);
        }
    }

    // An open surface has no outside, so no piece faces a direction.
    let sheet = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("sheet"), sheet(2, 2.0, |x, _| 0.1 * x)),
    );
    assert!(matches!(
        sheet.measure_face_pieces(&id("sheet"), FacePieceSet::Boundary),
        Err(VerticalExtentError::Unavailable(reason)) if reason.contains("open surface")
    ));
    // Its top is one planar piece of eight triangles.
    let top = sheet
        .measure_face_pieces(&id("sheet"), FacePieceSet::Top)
        .unwrap();
    assert_eq!(top.pieces().len(), 1);
    assert_eq!(top.pieces()[0].normals().len(), 8);
}
