//! Door and window leaves: how each leaf of a door or panel of a window
//! moves, which side it hangs on and the sectors it sweeps.
//!
//! Door clearances, swing directions, escape doors and landings are judged
//! from a door's leaves, and a casement's swing from a window's. This seam
//! supplies them through [`crate::ObjectFrameService::leaves`], which
//! refuses by default: a source that states no operation answers
//! [`DoorLeavesError::Unsupported`], never an empty set of leaves. The
//! types keep their door names; a window's panels are leaves alike.
//!
//! A window panel also states its height along `up` and, when it tilts on
//! a top or bottom hinge, the tilt sector it sweeps in the vertical plane
//! through that hinge ([`DoorLeaf::tilt`]). Its side swing lies in the
//! plane of its bottom edge, at sill height, and it sweeps that sector up
//! its whole height.
//!
//! Everything is in canonical metres and world coordinates. A leaf is
//! described by its closed position (a segment from `origin` along `along`,
//! `width` long), the direction it opens towards (`opening`) and the
//! door's up axis. These three axes are the source's placement axes: they
//! are orthonormal but may be left-handed, since a source may mirror a door.
//! A hinged leaf also carries the side its hinge is on, seen from above
//! looking along `opening`, and the [`SwingSector`] it sweeps.
//!
//! The sector is the symbolic swing the source's convention defines: a
//! quarter disc from the closed leaf to the leaf standing open at right
//! angles, or a half disc for a double-acting leaf. It is not a hardware
//! stop angle, which sources do not record.

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::MetricDirection;
use crate::services::reviewable_exact_evidence;

/// Tolerance on unit length, orthogonality and equal lengths, relative to
/// the magnitudes compared.
const TOLERANCE: f64 = 1.0e-9;

/// A plan polygon's vertices `[x, y]` in metres, in order, not repeating
/// the first.
pub type PlanRing = Vec<[f64; 2]>;

/// Failure to supply a door's leaves.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DoorLeavesError {
    /// The service does not cover the door's source.
    #[error("door-leaf service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The object is neither a door nor a window, so it has no leaves.
    #[error("`{0}` is not a door or window")]
    NotADoor(ObjectId),
    /// The source does not state what the leaves need: an operation type,
    /// an overall width or panel definitions. Nothing is defaulted.
    #[error("the source does not state the door's leaves: {0}")]
    NotStated(String),
    /// The source states the door's operation, but not precisely enough to
    /// place its leaves (a folding or revolving door, an operation whose
    /// direction the source's specification leaves open, contradictory
    /// panels).
    #[error("the door's leaves cannot be derived exactly: {0}")]
    Refused(String),
    /// The door, its placement or a unit cannot be read exactly.
    #[error("the door cannot be read exactly: {0}")]
    Unreadable(String),
    /// The service supplies no door leaves.
    #[error("the object-frame service supplies no door leaves")]
    Unsupported,
    /// The leaves are malformed: not orthonormal, a sector not matching its
    /// leaf, or a hinge side without a swing.
    #[error("door leaves are invalid: {0}")]
    InvalidLeaves(String),
    /// The evidence is not exact, not reviewable, or from another source.
    #[error("door-leaf evidence is not exact and reviewable")]
    InexactEvidence,
    /// The service answered for another door.
    #[error("door-leaf service answered for another door")]
    ResponseRequestMismatch,
}

/// Left or right, seen from above looking along a leaf's opening direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HingeSide {
    /// The left-hand side.
    Left,
    /// The right-hand side.
    Right,
}

impl HingeSide {
    /// The side's spelling in messages and evidence.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Where a leaf sits in its door, as the source states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LeafPosition {
    /// At the low end of the door's width.
    Left,
    /// In the middle.
    Middle,
    /// At the high end of the door's width.
    Right,
    /// At the bottom of a window.
    Bottom,
    /// At the top of a window.
    Top,
    /// Not stated.
    NotDefined,
}

/// How a leaf moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeafMotion {
    /// Swings about its hinge towards its opening direction.
    Swing,
    /// Swings about its hinge to both sides.
    DoubleSwing,
    /// Turns on its side hinge like [`Self::Swing`], or tilts on its
    /// bottom hinge (a tilt-and-turn window panel).
    TiltAndTurn,
    /// Tilts on a top or bottom hinge only (a top- or bottom-hung window
    /// panel).
    Tilt,
    /// Slides in the given direction when opening: along the door's width,
    /// or along a window panel's width or height.
    Slide(MetricDirection),
    /// Rolls up out of the opening, sweeping no floor area.
    RollUp,
    /// Is taken out rather than opened (a removable window casement).
    Removable,
    /// Does not open.
    Fixed,
}

impl LeafMotion {
    /// Whether the leaf turns on a side hinge and sweeps a [`SwingSector`]
    /// ([`DoorLeaf::swing`]).
    #[must_use]
    pub fn is_hinged(self) -> bool {
        matches!(self, Self::Swing | Self::DoubleSwing | Self::TiltAndTurn)
    }

    /// Whether the leaf tilts on a top or bottom hinge and sweeps a tilt
    /// sector ([`DoorLeaf::tilt`]).
    #[must_use]
    pub fn tilts(self) -> bool {
        matches!(self, Self::TiltAndTurn | Self::Tilt)
    }

    /// The motion's spelling in messages and evidence.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Swing => "swinging",
            Self::DoubleSwing => "double-acting",
            Self::TiltAndTurn => "tilt-and-turn",
            Self::Tilt => "tilting",
            Self::Slide(_) => "sliding",
            Self::RollUp => "rolling up",
            Self::Removable => "removable",
            Self::Fixed => "fixed",
        }
    }
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

fn finite(point: [f64; 3]) -> bool {
    point.iter().all(|c| c.is_finite())
}

fn invalid(message: impl Into<String>) -> DoorLeavesError {
    DoorLeavesError::InvalidLeaves(message.into())
}

/// The floor sector a hinged leaf sweeps, in world metres.
///
/// A single-swing leaf sweeps the quarter disc of `radius` about `hinge`
/// from `closed` (hinge towards the closed leaf's free edge) to `open` (the
/// leaf standing open at right angles). A double-acting leaf sweeps the
/// half disc from the reverse of `open`, through `closed`, to `open`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwingSector {
    hinge: [f64; 3],
    radius: f64,
    closed: MetricDirection,
    open: MetricDirection,
    double_acting: bool,
}

impl SwingSector {
    /// A sector; `closed` and `open` must be perpendicular and the radius
    /// positive.
    pub fn try_new(
        hinge: [f64; 3],
        radius: f64,
        closed: MetricDirection,
        open: MetricDirection,
        double_acting: bool,
    ) -> Result<Self, DoorLeavesError> {
        if !finite(hinge) || !radius.is_finite() || radius <= 0.0 {
            return Err(invalid(
                "a sector needs a finite hinge and a positive radius",
            ));
        }
        if dot(closed.components(), open.components()).abs() > TOLERANCE {
            return Err(invalid(
                "a sector's closed and open directions are not perpendicular",
            ));
        }
        Ok(Self {
            hinge,
            radius,
            closed,
            open,
            double_acting,
        })
    }

    /// The hinge, the sector's centre.
    #[must_use]
    pub fn hinge(&self) -> [f64; 3] {
        self.hinge
    }

    /// The radius, the leaf's width.
    #[must_use]
    pub fn radius_metres(&self) -> f64 {
        self.radius
    }

    /// From the hinge towards the closed leaf's free edge.
    #[must_use]
    pub fn closed(&self) -> MetricDirection {
        self.closed
    }

    /// From the hinge along the leaf standing open at right angles.
    #[must_use]
    pub fn open(&self) -> MetricDirection {
        self.open
    }

    /// Whether the leaf swings to both sides.
    #[must_use]
    pub fn is_double_acting(&self) -> bool {
        self.double_acting
    }

    /// The swept angle in radians: π/2, or π for a double-acting leaf.
    #[must_use]
    pub fn sweep(&self) -> f64 {
        if self.double_acting {
            std::f64::consts::PI
        } else {
            std::f64::consts::FRAC_PI_2
        }
    }

    /// Unit direction from the hinge at `angle` radians along the sweep:
    /// the first boundary ray (`closed`, or the reverse of `open` for a
    /// double-acting leaf) at zero, `open` at [`Self::sweep`].
    #[must_use]
    pub fn direction_at(&self, angle: f64) -> [f64; 3] {
        let (closed, open) = (self.closed.components(), self.open.components());
        // Measured from `closed`: a double-acting sweep starts a quarter
        // turn before it.
        let from_closed = if self.double_acting {
            angle - std::f64::consts::FRAC_PI_2
        } else {
            angle
        };
        let (sin, cos) = from_closed.sin_cos();
        [
            cos * closed[0] + sin * open[0],
            cos * closed[1] + sin * open[1],
            cos * closed[2] + sin * open[2],
        ]
    }

    /// Whether the sector lies in a horizontal plane (its rotation axis is
    /// vertical), so that it has a plan footprint of its own shape.
    #[must_use]
    pub fn is_horizontal(&self) -> bool {
        let axis = cross(self.closed.components(), self.open.components());
        (axis[2].abs() - 1.0).abs() <= 1.0e-6
    }

    /// Two convex plan polygons bracketing the sector's footprint, each
    /// anticlockwise: the first inscribed (every point lies in the sector),
    /// the second circumscribed (the sector lies in it). `segments` chords
    /// or tangents approximate each quarter turn; their radial gap is at
    /// most `radius · (1 / cos(π / (4 · segments)) − 1)`.
    ///
    /// `None` for a sector that is not horizontal or for no segments.
    #[must_use]
    pub fn plan_bounds(&self, segments: usize) -> Option<(PlanRing, PlanRing)> {
        if segments == 0 || !self.is_horizontal() {
            return None;
        }
        let quarters = if self.double_acting { 2 } else { 1 };
        let count = segments * quarters;
        #[allow(clippy::cast_precision_loss)]
        let step = self.sweep() / count as f64;
        let [hx, hy, _] = self.hinge;
        let at = |angle: f64, radius: f64| {
            let [dx, dy, _] = self.direction_at(angle);
            [hx + radius * dx, hy + radius * dy]
        };
        let mut inner = vec![[hx, hy]];
        let mut outer = vec![[hx, hy]];
        // Tangents at the mid-angles meet at `r / cos(step / 2)`.
        let reach = self.radius / (step / 2.0).cos();
        outer.push(at(0.0, self.radius));
        for index in 0..=count {
            #[allow(clippy::cast_precision_loss)]
            let angle = step * index as f64;
            inner.push(at(angle, self.radius));
            if index < count {
                outer.push(at(angle + step / 2.0, reach));
            }
        }
        outer.push(at(self.sweep(), self.radius));
        for polygon in [&mut inner, &mut outer] {
            if signed_area(polygon) < 0.0 {
                polygon.reverse();
            }
        }
        Some((inner, outer))
    }
}

/// Twice the signed area of a plan polygon: positive when anticlockwise.
fn signed_area(polygon: &[[f64; 2]]) -> f64 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum()
}

/// One door leaf, in world metres.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorLeaf {
    position: LeafPosition,
    motion: LeafMotion,
    origin: [f64; 3],
    along: MetricDirection,
    opening: MetricDirection,
    up: MetricDirection,
    width: f64,
    depth: Option<f64>,
    hinge_side: Option<HingeSide>,
    swing: Option<SwingSector>,
    height: Option<f64>,
    tilt: Option<SwingSector>,
}

impl DoorLeaf {
    /// A leaf whose closed position runs from `origin` along `along` for
    /// `width` metres, opening towards `opening`, with the door's `up`.
    ///
    /// The three axes must be orthonormal (either handedness). A hinged
    /// leaf needs a hinge side and a sector whose radius is the width,
    /// whose open direction is `opening` and whose closed direction runs
    /// along the leaf; every other leaf has neither. A sliding leaf slides
    /// along `along` or against it (a window panel also along `up`).
    /// `depth`, the leaf's thickness where the source states it, must be
    /// positive. A window panel adds its height with [`Self::with_height`]
    /// and, when it tilts, its tilt sector with [`Self::with_tilt`].
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        position: LeafPosition,
        motion: LeafMotion,
        origin: [f64; 3],
        along: MetricDirection,
        opening: MetricDirection,
        up: MetricDirection,
        width: f64,
        depth: Option<f64>,
        hinge_side: Option<HingeSide>,
        swing: Option<SwingSector>,
    ) -> Result<Self, DoorLeavesError> {
        let (x, y, z) = (along.components(), opening.components(), up.components());
        if dot(x, y).abs() > TOLERANCE || dot(y, z).abs() > TOLERANCE || dot(z, x).abs() > TOLERANCE
        {
            return Err(invalid("a leaf's axes are not orthonormal"));
        }
        if !finite(origin) || !width.is_finite() || width <= 0.0 {
            return Err(invalid("a leaf needs a finite origin and a positive width"));
        }
        if depth.is_some_and(|depth| !depth.is_finite() || depth <= 0.0) {
            return Err(invalid("a leaf's stated depth must be positive"));
        }
        match (motion, hinge_side, &swing) {
            (
                LeafMotion::Swing | LeafMotion::DoubleSwing | LeafMotion::TiltAndTurn,
                Some(_),
                Some(sector),
            ) => {
                let double = matches!(motion, LeafMotion::DoubleSwing);
                let same = |a: [f64; 3], b: [f64; 3]| {
                    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE)
                };
                if sector.is_double_acting() != double
                    || (sector.radius_metres() - width).abs() > TOLERANCE * width.max(1.0)
                    || !same(sector.open().components(), y)
                    || dot(sector.closed().components(), x).abs() < 1.0 - TOLERANCE
                {
                    return Err(invalid(
                        "a leaf's sector does not match its width, motion or axes",
                    ));
                }
            }
            (LeafMotion::Swing | LeafMotion::DoubleSwing | LeafMotion::TiltAndTurn, _, _) => {
                return Err(invalid("a hinged leaf needs a hinge side and a sector"));
            }
            (_, None, None) => {
                if let LeafMotion::Slide(direction) = motion
                    && dot(direction.components(), x).abs() < 1.0 - TOLERANCE
                    && dot(direction.components(), z).abs() < 1.0 - TOLERANCE
                {
                    return Err(invalid("a leaf slides along its width or height"));
                }
            }
            _ => return Err(invalid("only a hinged leaf has a hinge side or a sector")),
        }
        Ok(Self {
            position,
            motion,
            origin,
            along,
            opening,
            up,
            width,
            depth,
            hinge_side,
            swing,
            height: None,
            tilt: None,
        })
    }

    /// The leaf with its height along `up`, in metres: a window panel's,
    /// which must be positive.
    pub fn with_height(mut self, height: f64) -> Result<Self, DoorLeavesError> {
        if !height.is_finite() || height <= 0.0 {
            return Err(invalid("a leaf's height must be positive"));
        }
        self.height = Some(height);
        Ok(self)
    }

    /// The leaf with the sector it sweeps tilting on a top or bottom hinge.
    ///
    /// Only a tilting leaf ([`LeafMotion::tilts`]) with a height has one:
    /// its radius is the height, its `open` the leaf's opening direction
    /// and its `closed` runs along `up`, up from a bottom hinge or down
    /// from a top one. The leaf sweeps it along its width.
    pub fn with_tilt(mut self, tilt: SwingSector) -> Result<Self, DoorLeavesError> {
        let Some(height) = self.height.filter(|_| self.motion.tilts()) else {
            return Err(invalid("only a tilting leaf with a height has a tilt"));
        };
        let same =
            |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE);
        if tilt.is_double_acting()
            || (tilt.radius_metres() - height).abs() > TOLERANCE * height.max(1.0)
            || !same(tilt.open().components(), self.opening.components())
            || dot(tilt.closed().components(), self.up.components()).abs() < 1.0 - TOLERANCE
        {
            return Err(invalid("a leaf's tilt does not match its height or axes"));
        }
        self.tilt = Some(tilt);
        Ok(self)
    }

    /// Where the leaf sits in its door.
    #[must_use]
    pub fn position(&self) -> LeafPosition {
        self.position
    }

    /// How the leaf moves.
    #[must_use]
    pub fn motion(&self) -> LeafMotion {
        self.motion
    }

    /// The closed leaf's end at the low end of `along`.
    #[must_use]
    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    /// The door's width direction; the closed leaf runs along it.
    #[must_use]
    pub fn along(&self) -> MetricDirection {
        self.along
    }

    /// The direction the leaf opens towards: the side a single-swing leaf
    /// sweeps.
    #[must_use]
    pub fn opening(&self) -> MetricDirection {
        self.opening
    }

    /// The door's up axis.
    #[must_use]
    pub fn up(&self) -> MetricDirection {
        self.up
    }

    /// Whether the axes are left-handed: the source mirrors the door.
    #[must_use]
    pub fn is_mirrored(&self) -> bool {
        dot(
            cross(self.along.components(), self.opening.components()),
            self.up.components(),
        ) < 0.0
    }

    /// The leaf's width in metres.
    #[must_use]
    pub fn width_metres(&self) -> f64 {
        self.width
    }

    /// The leaf's thickness in metres, where the source states it.
    #[must_use]
    pub fn depth_metres(&self) -> Option<f64> {
        self.depth
    }

    /// The closed leaf's two ends.
    #[must_use]
    pub fn closed_edge(&self) -> ([f64; 3], [f64; 3]) {
        let [dx, dy, dz] = self.along.components();
        let [ox, oy, oz] = self.origin;
        (
            self.origin,
            [
                ox + self.width * dx,
                oy + self.width * dy,
                oz + self.width * dz,
            ],
        )
    }

    /// The side the hinge is on, seen from above looking along
    /// [`Self::opening`]; `None` for a leaf without a hinge.
    #[must_use]
    pub fn hinge_side(&self) -> Option<HingeSide> {
        self.hinge_side
    }

    /// The sector a hinged leaf sweeps; `None` for any other leaf.
    #[must_use]
    pub fn swing(&self) -> Option<&SwingSector> {
        self.swing.as_ref()
    }

    /// The leaf's height along `up` in metres, where the source states it
    /// (a window panel's).
    #[must_use]
    pub fn height_metres(&self) -> Option<f64> {
        self.height
    }

    /// The sector a tilting leaf sweeps in the vertical plane through one
    /// end of its top or bottom hinge; `None` for any other leaf.
    #[must_use]
    pub fn tilt(&self) -> Option<&SwingSector> {
        self.tilt.as_ref()
    }
}

/// Every leaf of one door, with provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorLeaves {
    door: ObjectId,
    operation: String,
    overall_width: f64,
    lining_thickness: Option<f64>,
    leaves: Vec<DoorLeaf>,
    evidence: Evidence,
}

impl DoorLeaves {
    /// The leaves of `door`, operated as `operation` (the source's own
    /// name for it), `overall_width` metres wide, with the lining thickness
    /// the source states.
    ///
    /// A door has at least one leaf, every tilting leaf its tilt, and the
    /// evidence must be exact, reviewable and from the door's source: the
    /// leaves are derived from what the source states, never estimated.
    /// For a window, `door` is the window, `operation` its partitioning and
    /// the leaves its panels.
    pub fn try_new(
        door: ObjectId,
        operation: impl Into<String>,
        overall_width: f64,
        lining_thickness: Option<f64>,
        leaves: Vec<DoorLeaf>,
        evidence: Evidence,
    ) -> Result<Self, DoorLeavesError> {
        let operation = operation.into();
        if operation.trim().is_empty() || leaves.is_empty() {
            return Err(invalid("a door has an operation and at least one leaf"));
        }
        if leaves
            .iter()
            .any(|leaf| leaf.motion.tilts() && leaf.tilt.is_none())
        {
            return Err(invalid("a tilting leaf needs its tilt sector"));
        }
        if !overall_width.is_finite() || overall_width <= 0.0 {
            return Err(invalid("a door's overall width must be positive"));
        }
        if lining_thickness.is_some_and(|thickness| !thickness.is_finite() || thickness < 0.0) {
            return Err(invalid("a lining thickness must not be negative"));
        }
        if !reviewable_exact_evidence(&evidence) || evidence.source != door.source {
            return Err(DoorLeavesError::InexactEvidence);
        }
        Ok(Self {
            door,
            operation,
            overall_width,
            lining_thickness,
            leaves,
            evidence,
        })
    }

    /// The door.
    #[must_use]
    pub fn door(&self) -> &ObjectId {
        &self.door
    }

    /// The operation type, as the source names it (`SINGLE_SWING_LEFT`).
    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// The door's overall width in metres, lining included.
    #[must_use]
    pub fn overall_width_metres(&self) -> f64 {
        self.overall_width
    }

    /// The lining's thickness across the opening in metres, where the
    /// source states it.
    #[must_use]
    pub fn lining_thickness_metres(&self) -> Option<f64> {
        self.lining_thickness
    }

    /// The leaves, in the source's order.
    #[must_use]
    pub fn leaves(&self) -> &[DoorLeaf] {
        &self.leaves
    }

    /// The leaves that swing on a side hinge: single-swing, double-acting
    /// and tilt-and-turn.
    pub fn hinged(&self) -> impl Iterator<Item = &DoorLeaf> {
        self.leaves.iter().filter(|leaf| leaf.motion.is_hinged())
    }

    /// Reviewable provenance.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// How far below a floor a swept sector's hinge may lie and still stand
/// on that floor: a door's leaf starts at its sill, which may sit below
/// the finished floor of the space it opens into, but never a storey
/// below it.
pub const SWEPT_FLOOR_REACH_METRES: f64 = 0.5;

/// The floor sectors a door's hinged leaves sweep, counted as obstacles by
/// a free-space or walkability request whose rule chose the door.
///
/// A swept sector is an obstacle in plan, over the whole elevation band of
/// a floor it stands on ([`Self::stands_on`]). Its footprint is never
/// measured from a door's body: a backend brackets each sector with
/// [`SwingSector::plan_bounds`] and proves a fit, a path or a connection
/// against the circumscribed polygon, and an absence, a narrowing or a
/// separation against the inscribed one.
#[derive(Clone, Debug, PartialEq)]
pub struct SweptDoor {
    door: ObjectId,
    sectors: Vec<SwingSector>,
}

impl SweptDoor {
    /// The sectors `door` sweeps.
    ///
    /// # Errors
    ///
    /// [`DoorLeavesError::InvalidLeaves`] for no sector or a sector that
    /// is not horizontal, which sweeps no plan area of its own shape.
    pub fn try_new(door: ObjectId, sectors: Vec<SwingSector>) -> Result<Self, DoorLeavesError> {
        if sectors.is_empty() {
            return Err(invalid("a swept door sweeps at least one sector"));
        }
        if sectors.iter().any(|sector| !sector.is_horizontal()) {
            return Err(invalid("a swept sector must lie in a horizontal plane"));
        }
        Ok(Self { door, sectors })
    }

    /// Every sector the hinged leaves of `leaves` sweep, or `None` when no
    /// leaf swings on a side hinge (a sliding or fixed door sweeps no
    /// floor).
    ///
    /// # Errors
    ///
    /// [`DoorLeavesError::InvalidLeaves`] for a leaf swinging outside a
    /// horizontal plane.
    pub fn of(leaves: &DoorLeaves) -> Result<Option<Self>, DoorLeavesError> {
        let sectors: Vec<SwingSector> = leaves
            .hinged()
            .filter_map(DoorLeaf::swing)
            .copied()
            .collect();
        if sectors.is_empty() {
            return Ok(None);
        }
        Self::try_new(leaves.door().clone(), sectors).map(Some)
    }

    /// The door.
    #[must_use]
    pub fn door(&self) -> &ObjectId {
        &self.door
    }

    /// The sectors, in the source's leaf order.
    #[must_use]
    pub fn sectors(&self) -> &[SwingSector] {
        &self.sectors
    }

    /// The sectors that stand on a floor at `floor` metres whose band
    /// reaches up to `top`: those whose hinge lies at most
    /// [`SWEPT_FLOOR_REACH_METRES`] below the floor and below the top.
    pub fn stands_on(&self, floor: f64, top: f64) -> impl Iterator<Item = &SwingSector> {
        self.sectors.iter().filter(move |sector| {
            let z = sector.hinge()[2];
            z >= floor - SWEPT_FLOOR_REACH_METRES && z < top
        })
    }
}

/// `swept` sorted by door, each door once; `None` when a door is given
/// twice with different sectors.
pub(crate) fn tidy_swept(mut swept: Vec<SweptDoor>) -> Option<Vec<SweptDoor>> {
    swept.sort_by(|a, b| a.door.cmp(&b.door));
    swept.dedup();
    if swept.windows(2).any(|pair| pair[0].door == pair[1].door) {
        return None;
    }
    Some(swept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn door() -> ObjectId {
        ObjectId::new(source(), "d").unwrap()
    }

    fn direction(vector: [f64; 3]) -> MetricDirection {
        MetricDirection::try_new(vector).unwrap()
    }

    /// A 0.9 m leaf closed along +x from the origin, opening towards +y,
    /// hinged at the origin (left, seen looking along +y).
    fn left_hinged(double: bool) -> DoorLeaf {
        let sector = SwingSector::try_new(
            [0.0; 3],
            0.9,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            double,
        )
        .unwrap();
        DoorLeaf::try_new(
            LeafPosition::NotDefined,
            if double {
                LeafMotion::DoubleSwing
            } else {
                LeafMotion::Swing
            },
            [0.0; 3],
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
            0.9,
            Some(0.04),
            Some(HingeSide::Left),
            Some(sector),
        )
        .unwrap()
    }

    #[test]
    fn a_sector_brackets_the_quarter_disc() {
        let leaf = left_hinged(false);
        let sector = leaf.swing().unwrap();
        assert!(sector.is_horizontal());
        let (inner, outer) = sector.plan_bounds(16).unwrap();
        assert!(signed_area(&inner) > 0.0 && signed_area(&outer) > 0.0);
        let quarter = std::f64::consts::FRAC_PI_4 * 0.81;
        let area = |polygon: &[[f64; 2]]| signed_area(polygon) / 2.0;
        assert!(area(&inner) < quarter && quarter < area(&outer));
        assert!(area(&outer) - area(&inner) < 1e-2);
        // Every inscribed vertex lies within the radius, in the quadrant.
        for [x, y] in inner {
            assert!(x >= -1e-12 && y >= -1e-12 && x.hypot(y) <= 0.9 + 1e-12);
        }
        let open = sector.direction_at(sector.sweep());
        assert!((open[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_double_acting_sector_is_a_half_disc() {
        let sector = *left_hinged(true).swing().unwrap();
        assert!((sector.sweep() - std::f64::consts::PI).abs() < 1e-12);
        let start = sector.direction_at(0.0);
        assert!((start[1] + 1.0).abs() < 1e-12, "{start:?}");
        let (inner, _) = sector.plan_bounds(8).unwrap();
        assert!(inner.iter().any(|[_, y]| *y < -0.5));
        assert!(inner.iter().any(|[_, y]| *y > 0.5));
    }

    #[test]
    fn a_swept_door_takes_its_hinged_leaves_sectors() {
        let exact = Evidence::exact(source(), "leaves");
        let leaves = DoorLeaves::try_new(
            door(),
            "SINGLE_SWING_LEFT",
            0.9,
            None,
            vec![left_hinged(false)],
            exact,
        )
        .unwrap();
        let swept = SweptDoor::of(&leaves).unwrap().unwrap();
        assert_eq!(swept.door(), &door());
        assert_eq!(swept.sectors().len(), 1);
        // It stands on a floor up to half a metre above its hinge, below
        // the band's top; not on the storey above or below.
        assert_eq!(swept.stands_on(0.4, 2.0).count(), 1);
        assert_eq!(swept.stands_on(-0.2, 2.0).count(), 1);
        assert_eq!(swept.stands_on(0.6, 2.6).count(), 0);
        assert_eq!(swept.stands_on(-3.0, -0.9).count(), 0);
        assert!(SweptDoor::try_new(door(), Vec::new()).is_err());
        let vertical = SwingSector::try_new(
            [0.0; 3],
            1.0,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
            false,
        )
        .unwrap();
        assert!(SweptDoor::try_new(door(), vec![vertical]).is_err());
        // One door twice is kept once; with other sectors it conflicts.
        let other = SweptDoor::try_new(door(), vec![*left_hinged(true).swing().unwrap()]).unwrap();
        assert_eq!(
            tidy_swept(vec![swept.clone(), swept.clone()]),
            Some(vec![swept.clone()])
        );
        assert_eq!(tidy_swept(vec![swept, other]), None);
    }

    #[test]
    fn a_vertical_sector_has_no_plan_footprint() {
        let sector = SwingSector::try_new(
            [0.0; 3],
            1.0,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
            false,
        )
        .unwrap();
        assert!(!sector.is_horizontal());
        assert!(sector.plan_bounds(4).is_none());
    }

    #[test]
    fn leaves_must_be_consistent() {
        let leaf = left_hinged(false);
        assert!(!leaf.is_mirrored());
        let (_, end) = leaf.closed_edge();
        assert!((end[0] - 0.9).abs() < 1e-12 && end[1].abs() < 1e-12);
        // A sector of another radius, a hinge side without a sector, and a
        // sliding leaf moving across its width are refused.
        let wrong = SwingSector::try_new(
            [0.0; 3],
            0.8,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            false,
        )
        .unwrap();
        let axes = (
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
        );
        let build = |motion, side, sector| {
            DoorLeaf::try_new(
                LeafPosition::Left,
                motion,
                [0.0; 3],
                axes.0,
                axes.1,
                axes.2,
                0.9,
                None,
                side,
                sector,
            )
        };
        assert!(build(LeafMotion::Swing, Some(HingeSide::Left), Some(wrong)).is_err());
        assert!(build(LeafMotion::Swing, Some(HingeSide::Left), None).is_err());
        assert!(build(LeafMotion::Fixed, Some(HingeSide::Left), None).is_err());
        assert!(build(LeafMotion::Slide(axes.1), None, None).is_err());
        assert!(build(LeafMotion::Slide(axes.0), None, None).is_ok());
        assert!(build(LeafMotion::Slide(axes.2), None, None).is_ok());
        assert!(build(LeafMotion::TiltAndTurn, Some(HingeSide::Left), None).is_err());
        // Mirrored axes are accepted and reported.
        let mirrored = DoorLeaf::try_new(
            LeafPosition::Left,
            LeafMotion::Fixed,
            [0.0; 3],
            axes.0,
            direction([0.0, -1.0, 0.0]),
            axes.2,
            0.9,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(mirrored.is_mirrored());
    }

    #[test]
    fn door_leaves_need_exact_evidence_from_the_door_source() {
        let exact = Evidence::exact(source(), "door-operation:d");
        assert!(
            DoorLeaves::try_new(
                door(),
                "SINGLE_SWING_LEFT",
                1.0,
                Some(0.05),
                vec![left_hinged(false)],
                exact.clone()
            )
            .is_ok()
        );
        let mut approximate = exact.clone();
        approximate.exact = false;
        let foreign = Evidence::exact(SourceId::new("cad", "other").unwrap(), "x");
        for evidence in [approximate, foreign] {
            assert_eq!(
                DoorLeaves::try_new(door(), "X", 1.0, None, vec![left_hinged(false)], evidence),
                Err(DoorLeavesError::InexactEvidence)
            );
        }
        assert!(DoorLeaves::try_new(door(), "X", 1.0, None, vec![], exact).is_err());
    }
    #[test]
    fn a_tilting_panel_states_its_height_and_tilt() {
        let axes = (
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
        );
        let tilt = |radius: f64, closed: [f64; 3]| {
            SwingSector::try_new([0.0, 0.0, 0.9], radius, direction(closed), axes.1, false).unwrap()
        };
        let panel = || {
            DoorLeaf::try_new(
                LeafPosition::NotDefined,
                LeafMotion::Tilt,
                [0.0, 0.0, 0.9],
                axes.0,
                axes.1,
                axes.2,
                0.8,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let tilted = panel()
            .with_height(1.2)
            .unwrap()
            .with_tilt(tilt(1.2, [0.0, 0.0, 1.0]))
            .unwrap();
        assert_eq!(tilted.height_metres(), Some(1.2));
        assert!(!tilted.tilt().unwrap().is_horizontal());
        // A tilt of another radius, across the width, or without a height
        // is refused, and so is a tilting leaf without its tilt.
        let tall = || panel().with_height(1.2).unwrap();
        assert!(tall().with_tilt(tilt(1.0, [0.0, 0.0, 1.0])).is_err());
        assert!(tall().with_tilt(tilt(1.2, [1.0, 0.0, 0.0])).is_err());
        assert!(panel().with_tilt(tilt(1.2, [0.0, 0.0, 1.0])).is_err());
        assert!(panel().with_height(0.0).is_err());
        let exact = Evidence::exact(source(), "window-operation:d");
        assert!(
            DoorLeaves::try_new(
                door(),
                "SINGLE_PANEL",
                0.8,
                None,
                vec![tall()],
                exact.clone()
            )
            .is_err()
        );
        assert!(
            DoorLeaves::try_new(door(), "SINGLE_PANEL", 0.8, None, vec![tilted], exact).is_ok()
        );
    }
}
