#![allow(clippy::doc_markdown)]

//! Snapshot images for BCF viewpoints, rendered from tessellated meshes.
//!
//! A [`Renderer`] holds each object's triangle mesh in model coordinates
//! (metres) and draws a viewpoint as [`axioval_bcf::export_with_snapshots`]
//! describes it: from its camera, the subject and related objects in their
//! colours, every other object in [`CONTEXT_COLOR`] unless the viewpoint is
//! isolated, cut by its clipping planes. The result is a PNG.
//!
//! **A snapshot is illustrative, never evidence.** It shows tessellated
//! bodies with flat shading at a fixed resolution; nothing is measured from
//! it, and the sink says so in every topic that carries one. It depends on
//! meshes only: no source format, no geometry kernel.
//!
//! # Determinism
//!
//! Rasterisation uses fixed arithmetic order and pixel-centre sampling, and
//! the PNG is written with a fixed filter and compression level, so the same
//! meshes and viewpoint give the same bytes for a given build.

use std::collections::BTreeMap;
use std::io::Write as _;

use axioval_bcf::{SnapshotRenderer, SnapshotView};
use axioval_ir::ObjectId;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use openbim_bcf::write::{Camera, Projection, Vector3};

/// Height of a snapshot in pixels; its width follows the camera's aspect
/// ratio (square without one).
pub const DEFAULT_HEIGHT: u32 = 512;

/// Colour of an object that is neither the subject nor related: light grey.
pub const CONTEXT_COLOR: [u8; 3] = [0xC8, 0xC8, 0xC8];

/// Colour of the empty background: near white.
pub const BACKGROUND: [u8; 3] = [0xF4, 0xF4, 0xF4];

/// Nearest distance a perspective camera draws, in metres.
const NEAR: f64 = 0.01;

/// One object's triangle mesh in model coordinates, in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    positions: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// A mesh from its vertices and triangles; `None` when a coordinate is
    /// not finite or a triangle names a vertex that does not exist.
    #[must_use]
    pub fn new(positions: Vec<[f64; 3]>, triangles: Vec<[u32; 3]>) -> Option<Self> {
        let finite = positions.iter().flatten().all(|c| c.is_finite());
        let count = positions.len();
        let indexed = triangles
            .iter()
            .flatten()
            .all(|&index| (index as usize) < count);
        (finite && indexed).then_some(Self {
            positions,
            triangles,
        })
    }
}

/// A mesh with its bounding sphere, for culling.
struct Prepared {
    mesh: Mesh,
    centre: [f64; 3],
    radius: f64,
}

impl Prepared {
    fn new(mesh: Mesh) -> Self {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for p in &mesh.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        let (centre, radius) = if mesh.positions.is_empty() {
            ([0.0; 3], 0.0)
        } else {
            let centre = [0, 1, 2].map(|axis| f64::midpoint(min[axis], max[axis]));
            (centre, length(sub(max, centre)))
        };
        Self {
            mesh,
            centre,
            radius,
        }
    }
}

/// Renders BCF viewpoints over a fixed set of meshes.
pub struct Renderer {
    meshes: BTreeMap<ObjectId, Prepared>,
    height: u32,
}

impl Renderer {
    /// A renderer over `meshes`, at [`DEFAULT_HEIGHT`].
    #[must_use]
    pub fn new(meshes: BTreeMap<ObjectId, Mesh>) -> Self {
        Self {
            meshes: meshes
                .into_iter()
                .map(|(id, mesh)| (id, Prepared::new(mesh)))
                .collect(),
            height: DEFAULT_HEIGHT,
        }
    }

    /// The same renderer drawing images `height` pixels high (at least 1).
    #[must_use]
    pub fn with_height(mut self, height: u32) -> Self {
        self.height = height.max(1);
        self
    }

    /// The viewpoint as an image; `None` when the subject has no mesh, so
    /// nothing could be highlighted, or the camera is degenerate.
    #[must_use]
    pub fn image(&self, view: &SnapshotView<'_>) -> Option<Image> {
        let subject = view.subject?;
        self.meshes.get(subject)?;
        let eye = Eye::new(view.camera, self.height)?;
        let mut image = Image::new(eye.width, eye.height);
        let color = |id: &ObjectId| {
            if id == subject {
                Some(rgb(view.colors.subject.value()))
            } else if view.related.contains(id) {
                Some(rgb(view.colors.related.value()))
            } else if view.isolate {
                None
            } else {
                Some(CONTEXT_COLOR)
            }
        };
        let planes: Vec<([f64; 3], [f64; 3])> = view
            .clipping_planes
            .iter()
            .map(|plane| (array(plane.location), array(plane.direction)))
            .collect();
        for (id, prepared) in &self.meshes {
            let Some(color) = color(id) else { continue };
            if !eye.may_see(prepared) {
                continue;
            }
            for triangle in &prepared.mesh.triangles {
                let corners = triangle.map(|index| prepared.mesh.positions[index as usize]);
                let mut polygon = corners.to_vec();
                for (location, direction) in &planes {
                    // A plane clips away what lies on the side it points to.
                    polygon = clip(&polygon, |p| -dot(sub(p, *location), *direction));
                }
                if polygon.len() < 3 {
                    continue;
                }
                let shade = eye.shade(corners);
                let color = color.map(|c| {
                    // Channels are 0–255 and the shade at most 1.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let shaded = (f64::from(c) * shade).round().clamp(0.0, 255.0) as u8;
                    shaded
                });
                eye.draw(&mut image, &polygon, color);
            }
        }
        Some(image)
    }
}

impl SnapshotRenderer for Renderer {
    fn render(&self, view: &SnapshotView<'_>) -> Option<Vec<u8>> {
        self.image(view).map(|image| image.png())
    }
}

/// An RGB image with a depth per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    /// Nearer is greater; `NEG_INFINITY` where nothing was drawn.
    depth: Vec<f64>,
}

impl Image {
    fn new(width: u32, height: u32) -> Self {
        let pixels = width as usize * height as usize;
        Self {
            width,
            height,
            rgb: BACKGROUND.repeat(pixels),
            depth: vec![f64::NEG_INFINITY; pixels],
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The colour at column `x`, row `y` from the top.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let at = (y as usize * self.width as usize + x as usize) * 3;
        [self.rgb[at], self.rgb[at + 1], self.rgb[at + 2]]
    }

    /// The image as a PNG file: 8-bit RGB, no interlacing, every row
    /// unfiltered, deflated at level 6.
    ///
    /// # Panics
    ///
    /// Never: the file is assembled in memory.
    #[must_use]
    pub fn png(&self) -> Vec<u8> {
        let stride = self.width as usize * 3;
        let mut scanlines = Vec::with_capacity((stride + 1) * self.height as usize);
        for line in self.rgb.chunks_exact(stride) {
            scanlines.push(0);
            scanlines.extend_from_slice(line);
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder
            .write_all(&scanlines)
            .expect("writing to memory cannot fail");
        let data = encoder.finish().expect("writing to memory cannot fail");
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::with_capacity(13);
        header.extend_from_slice(&self.width.to_be_bytes());
        header.extend_from_slice(&self.height.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut png, *b"IHDR", &header);
        chunk(&mut png, *b"IDAT", &data);
        chunk(&mut png, *b"IEND", &[]);
        png
    }

    fn set(&mut self, x: u32, y: u32, depth: f64, color: [u8; 3]) {
        let at = y as usize * self.width as usize + x as usize;
        if depth > self.depth[at] {
            self.depth[at] = depth;
            self.rgb[at * 3..at * 3 + 3].copy_from_slice(&color);
        }
    }
}

fn chunk(png: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).expect("a chunk fits in 4 GiB");
    png.extend_from_slice(&length.to_be_bytes());
    png.extend_from_slice(&kind);
    png.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(&kind);
    crc.update(data);
    png.extend_from_slice(&crc.sum().to_be_bytes());
}

/// A camera fitted to an image.
struct Eye {
    at: [f64; 3],
    forward: [f64; 3],
    up: [f64; 3],
    right: [f64; 3],
    /// Half the visible height at unit distance (perspective) or in metres
    /// (orthogonal).
    half: f64,
    perspective: bool,
    aspect: f64,
    width: u32,
    height: u32,
}

impl Eye {
    fn new(camera: &Camera, height: u32) -> Option<Self> {
        let forward = unit(array(camera.direction))?;
        let up = array(camera.up_vector);
        let up = unit(sub(up, scale(forward, dot(up, forward))))?;
        let right = cross(forward, up);
        let (half, perspective) = match camera.projection {
            Projection::Perspective { field_of_view } => {
                ((field_of_view / 2.0).to_radians().tan(), true)
            }
            Projection::Orthogonal {
                view_to_world_scale,
            } => (view_to_world_scale / 2.0, false),
        };
        let aspect = camera.aspect_ratio.unwrap_or(1.0);
        if !(half.is_finite() && half > 0.0 && aspect.is_finite() && aspect > 0.0) {
            return None;
        }
        // Pixel counts are small positive numbers.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let width = (f64::from(height) * aspect).round().max(1.0) as u32;
        Some(Self {
            at: array(camera.view_point),
            forward,
            up,
            right,
            half,
            perspective,
            aspect,
            width,
            height,
        })
    }

    /// Camera coordinates: right, up, depth along the view.
    fn local(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(p, self.at);
        [dot(d, self.right), dot(d, self.up), dot(d, self.forward)]
    }

    /// Whether any of the sphere around `mesh` may be in view.
    fn may_see(&self, mesh: &Prepared) -> bool {
        let [x, y, z] = self.local(mesh.centre);
        let r = mesh.radius;
        if self.perspective {
            if z + r < NEAR {
                return false;
            }
            let reach = z.max(NEAR) * self.half;
            x.abs() - r <= reach * self.aspect * 2.0 && y.abs() - r <= reach * 2.0
        } else {
            x.abs() - r <= self.half * self.aspect && y.abs() - r <= self.half
        }
    }

    /// Flat shading of a triangle: lit from over the viewer's shoulder, both
    /// sides alike.
    fn shade(&self, [a, b, c]: [[f64; 3]; 3]) -> f64 {
        let Some(normal) = unit(cross(sub(b, a), sub(c, a))) else {
            return 1.0;
        };
        let light = unit(sub(scale(self.up, 0.6), self.forward)).unwrap_or(self.forward);
        0.45 + 0.55 * dot(normal, light).abs()
    }

    /// Draws a convex polygon in world coordinates.
    fn draw(&self, image: &mut Image, polygon: &[[f64; 3]], color: [u8; 3]) {
        let mut local: Vec<[f64; 3]> = polygon.iter().map(|&p| self.local(p)).collect();
        if self.perspective {
            local = clip(&local, |p| p[2] - NEAR);
            if local.len() < 3 {
                return;
            }
        }
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        // Screen x and y in pixels, and a depth that is affine in screen
        // space and greater when nearer.
        let screen: Vec<[f64; 3]> = local
            .iter()
            .map(|&[x, y, z]| {
                let (nx, ny, depth) = if self.perspective {
                    (
                        x / (z * self.half * self.aspect),
                        y / (z * self.half),
                        1.0 / z,
                    )
                } else {
                    (x / (self.half * self.aspect), y / self.half, -z)
                };
                [
                    f64::midpoint(nx, 1.0) * w,
                    f64::midpoint(-ny, 1.0) * h,
                    depth,
                ]
            })
            .collect();
        for index in 1..screen.len() - 1 {
            fill(image, [screen[0], screen[index], screen[index + 1]], color);
        }
    }
}

/// Fills one screen triangle, sampling pixel centres.
#[allow(clippy::many_single_char_names)]
fn fill(image: &mut Image, [a, b, c]: [[f64; 3]; 3], color: [u8; 3]) {
    let area = edge(a, b, c);
    if area.abs() < 1e-12 {
        return;
    }
    let (w, h) = (f64::from(image.width), f64::from(image.height));
    let lo = |i: usize, limit: f64| a[i].min(b[i]).min(c[i]).floor().clamp(0.0, limit);
    let hi = |i: usize, limit: f64| a[i].max(b[i]).max(c[i]).ceil().clamp(0.0, limit);
    // Bounds are clamped to the image, so they are small non-negative
    // integers.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (x0, x1, y0, y1) = (
        lo(0, w) as u32,
        hi(0, w) as u32,
        lo(1, h) as u32,
        hi(1, h) as u32,
    );
    for y in y0..y1 {
        for x in x0..x1 {
            let p = [f64::from(x) + 0.5, f64::from(y) + 0.5, 0.0];
            let (wa, wb, wc) = (edge(b, c, p), edge(c, a, p), edge(a, b, p));
            let inside = if area > 0.0 {
                wa >= 0.0 && wb >= 0.0 && wc >= 0.0
            } else {
                wa <= 0.0 && wb <= 0.0 && wc <= 0.0
            };
            if inside {
                let depth = (wa * a[2] + wb * b[2] + wc * c[2]) / area;
                image.set(x, y, depth, color);
            }
        }
    }
}

fn edge(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// The part of a convex polygon where `inside` is not negative
/// (Sutherland–Hodgman against one plane).
fn clip(polygon: &[[f64; 3]], inside: impl Fn([f64; 3]) -> f64) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (index, &current) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        let (a, b) = (inside(current), inside(next));
        if a >= 0.0 {
            out.push(current);
        }
        if (a >= 0.0) != (b >= 0.0) {
            let t = a / (a - b);
            out.push([0, 1, 2].map(|axis| current[axis] + (next[axis] - current[axis]) * t));
        }
    }
    out
}

fn rgb(argb: u32) -> [u8; 3] {
    let [_, r, g, b] = argb.to_be_bytes();
    [r, g, b]
}

fn array(v: Vector3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|c| c * s)
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

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = length(a);
    (l.is_finite() && l > 0.0).then(|| scale(a, 1.0 / l))
}
