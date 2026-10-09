//! Bodies made of several closed shells that overlap one another.
//!
//! A product's body is often several closed items in one representation:
//! the legs, seat and back of a chair, the carcass and doors of a cabinet.
//! Each item is a closed solid, the body is their union, and the items may
//! overlap (a leg runs into the seat) or touch. Meshed side by side they make
//! one mesh that is closed, but whose triangles cross where the items meet,
//! so the volume kernel refuses it as self-intersecting and no volume of the
//! body, or shared with it, is measured.
//!
//! Each shell, though, is a closed solid the kernel measures. From them:
//!
//! - the body's volume is at least its largest shell's and at most the sum
//!   of all of theirs;
//! - the volume it shares with another closed body is at least the largest
//!   shell's shared volume and at most the sum of theirs;
//! - the volume it has outside the other body is at most the sum of each
//!   shell's volume outside it (a point of the union outside the other body
//!   lies in some shell, outside the other body), and at least the largest
//!   of them.
//!
//! The last bounds the share of the smaller body
//! ([`IntersectionVolume::with_subject_outside`]): a chair whose every
//! shell lies inside a room lies wholly inside it, however much its shells
//! overlap, and one with a shell surely sticking out of the room by more
//! than the ratio allows does not. Only exact meshes are read this way: a
//! tessellation's chord band would have to widen every shell, and none is
//! needed on the corpus behind #312.

use axiolid_inspect::{enclosed_volume, intersection_volume};
use axiolid_mesh::{TriMesh, audit_mesh};
use axioval_engine::{IntersectionVolume, VolumeInterval};

use super::tolerance;

/// A relative margin on sums of certified bounds, so their rounding never
/// tightens them.
const SUM_MARGIN: f64 = 1e-12;

/// One closed shell of a body: its own mesh, enclosed volume and box.
#[derive(Debug)]
pub(crate) struct Shell {
    mesh: TriMesh,
    /// Certified `(lower, upper)` enclosed volume.
    volume: (f64, f64),
    min: [f64; 3],
    max: [f64; 3],
}

/// The closed shells of `mesh`: its triangles grouped by the corner indices
/// they share. `None` unless there are at least two and each is a closed,
/// consistently wound two-manifold whose volume the kernel certifies; a
/// single shell is the whole mesh the kernel already refused.
pub(crate) fn shells(mesh: &TriMesh) -> Option<Vec<Shell>> {
    let positions = mesh.positions.len();
    let mut parent: Vec<usize> = (0..positions).collect();
    let corners: Vec<[usize; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|corner| [corner[0] as usize, corner[1] as usize, corner[2] as usize])
        .collect();
    if corners.iter().flatten().any(|&index| index >= positions) {
        return None;
    }
    for &[a, b, c] in &corners {
        let first = root(&mut parent, a);
        for other in [b, c] {
            let other = root(&mut parent, other);
            if other != first {
                parent[other] = first;
            }
        }
    }
    // Shells in the order of their first triangle, so the result does not
    // depend on the union's choice of roots.
    let mut groups: Vec<Vec<[usize; 3]>> = Vec::new();
    let mut slot = vec![usize::MAX; positions];
    for triangle in &corners {
        let shell = root(&mut parent, triangle[0]);
        if slot[shell] == usize::MAX {
            slot[shell] = groups.len();
            groups.push(Vec::new());
        }
        groups[slot[shell]].push(*triangle);
    }
    if groups.len() < 2 {
        return None;
    }
    let tolerance = tolerance().ok()?;
    groups
        .into_iter()
        .map(|triangles| {
            let mut renumbered = vec![u32::MAX; positions];
            let mut points = Vec::new();
            let mut indices = Vec::with_capacity(triangles.len() * 3);
            for index in triangles.into_iter().flatten() {
                if renumbered[index] == u32::MAX {
                    renumbered[index] = u32::try_from(points.len()).ok()?;
                    points.push(mesh.positions[index]);
                }
                indices.push(renumbered[index]);
            }
            let shell = TriMesh::new(points, indices);
            if !audit_mesh(&shell, tolerance).is_closed_two_manifold() {
                return None;
            }
            let volume = enclosed_volume(&shell).ok()?;
            let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for point in &shell.positions {
                for (axis, value) in point.to_array().into_iter().enumerate() {
                    min[axis] = min[axis].min(value);
                    max[axis] = max[axis].max(value);
                }
            }
            Some(Shell {
                mesh: shell,
                volume: (volume.lower, volume.upper),
                min,
                max,
            })
        })
        .collect()
}

/// The root of `at`'s set, halving the path on the way.
fn root(parent: &mut [usize], mut at: usize) -> usize {
    while parent[at] != at {
        parent[at] = parent[parent[at]];
        at = parent[at];
    }
    at
}

/// The volume a body of overlapping `shells` shares with the closed body
/// `other` (its mesh, certified volume and box), bounded shell by shell,
/// with what the shells have outside it; `shells_are_subject` orients the
/// result. `disjoint` bodies share nothing. `None` where the kernel refuses
/// a shell against `other`.
pub(crate) fn intersection(
    shells: &[Shell],
    other: &TriMesh,
    other_volume: (f64, f64),
    other_box: ([f64; 3], [f64; 3]),
    disjoint: bool,
    shells_are_subject: bool,
) -> Option<IntersectionVolume> {
    let mut own = (0.0_f64, 0.0_f64);
    let mut shared = (0.0_f64, 0.0_f64);
    let mut outside = (0.0_f64, 0.0_f64);
    for shell in shells {
        own = (own.0.max(shell.volume.0), own.1 + shell.volume.1);
        let apart = (0..3)
            .any(|axis| shell.max[axis] < other_box.0[axis] || other_box.1[axis] < shell.min[axis]);
        let (lower, upper) = if disjoint || apart {
            (0.0, 0.0)
        } else {
            let volume = intersection_volume(&shell.mesh, other).ok()?;
            (volume.lower, volume.upper)
        };
        shared = (shared.0.max(lower), shared.1 + upper);
        outside = (
            outside.0.max(shell.volume.0 - upper),
            outside.1 + (shell.volume.1 - lower).max(0.0),
        );
    }
    own.1 *= 1.0 + SUM_MARGIN;
    outside.1 *= 1.0 + SUM_MARGIN;
    let upper = (shared.1 * (1.0 + SUM_MARGIN))
        .min(own.1)
        .min(other_volume.1);
    let lower = if disjoint {
        0.0
    } else {
        shared.0.max((own.0 - outside.1) * (1.0 - SUM_MARGIN))
    };
    if lower > upper {
        return None;
    }
    let interval = |(lower, upper): (f64, f64)| VolumeInterval::try_new(lower, upper).ok();
    let (own, other) = (interval(own)?, interval(other_volume)?);
    let outside = interval((outside.0.max(0.0).min(outside.1), outside.1))?;
    let shared = interval((lower, upper))?;
    Some(if shells_are_subject {
        IntersectionVolume::try_new(shared, own, other)
            .ok()?
            .with_subject_outside(outside)
    } else {
        IntersectionVolume::try_new(shared, other, own)
            .ok()?
            .with_counterpart_outside(outside)
    })
}
