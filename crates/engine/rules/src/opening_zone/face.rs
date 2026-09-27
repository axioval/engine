//! A host's face and the extrusions placed in it, read from the reserved
//! body set: the geometry `opening-zone` and `opening-area` share.

use std::rc::Rc;

use axioval_engine::{NotEvaluatedReason, RuleContext};
use axioval_ir::{Evidence, Object, ObjectId};

use super::outline::Polygon;
use crate::body_facts::BodyFacts;
use crate::support::{Unavailable, invalid};

/// How far composed placements may be off in binary arithmetic.
pub(crate) const ROUNDING: f64 = 1e-9;

/// How nearly two unit vectors must agree to be taken as parallel.
const PARALLEL: f64 = 1e-9;

pub(crate) type Vector = [f64; 3];

/// An interval along one axis, lower bound first.
pub(crate) type Span = (f64, f64);

pub(crate) fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn along(origin: Vector, a: Vector, s: f64, b: Vector, t: f64) -> Vector {
    [
        origin[0] + a[0] * s + b[0] * t,
        origin[1] + a[1] * s + b[1] * t,
        origin[2] + a[2] * s + b[2] * t,
    ]
}

pub(crate) fn minus(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn parallel(a: Vector, b: Vector) -> bool {
    dot(a, b).abs() >= 1.0 - PARALLEL
}

/// The clear distance between two spans, zero when they meet.
pub(crate) fn gap(a: Span, b: Span) -> f64 {
    (b.0 - a.1).max(a.0 - b.1).max(0.0)
}

/// How far two spans overlap, negative by their gap when they do not.
fn overlap(a: Span, b: Span) -> f64 {
    a.1.min(b.1) - a.0.max(b.0)
}

/// The separation of two rectangles in the face, each a span along the
/// length and the height: their clear distance, or less than zero by the
/// lesser overlap when their interiors overlap.
pub(crate) fn separation(a: [Span; 2], b: [Span; 2]) -> f64 {
    let (length, height) = (overlap(a[0], b[0]), overlap(a[1], b[1]));
    if length > 0.0 && height > 0.0 {
        -length.min(height)
    } else {
        gap(a[0], b[0]).hypot(gap(a[1], b[1]))
    }
}

/// One of the host's three axes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Extrusion,
    ProfileX,
    ProfileY,
}

impl Axis {
    pub(crate) fn parse(name: &str, value: &str) -> Result<Self, Unavailable> {
        match value {
            "extrusion" => Ok(Self::Extrusion),
            "profile-x" => Ok(Self::ProfileX),
            "profile-y" => Ok(Self::ProfileY),
            other => Err(invalid(format!(
                "`{name}` `{other}` is unsupported; use `extrusion`, `profile-x` or `profile-y`"
            ))),
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Extrusion => 0,
            Self::ProfileX => 1,
            Self::ProfileY => 2,
        }
    }

    /// The coordinate the axis is in the profile's plane: 0 for X, 1 for
    /// Y, `None` for the extrusion.
    fn profile(self) -> Option<usize> {
        match self {
            Self::Extrusion => None,
            Self::ProfileX => Some(0),
            Self::ProfileY => Some(1),
        }
    }
}

/// The face a rule names: the host's length and height axes, and the third
/// axis through the host.
#[derive(Clone, Copy)]
pub(crate) struct FaceAxes {
    pub(crate) length: Axis,
    pub(crate) height: Axis,
}

impl FaceAxes {
    /// Reads `length_axis` and `height_axis`, which must differ.
    pub(crate) fn parse(length: &str, height: &str) -> Result<Self, Unavailable> {
        let length = Axis::parse("length_axis", length)?;
        let height = Axis::parse("height_axis", height)?;
        if length == height {
            return Err(invalid("`length_axis` and `height_axis` must differ"));
        }
        Ok(Self { length, height })
    }

    pub(crate) fn through(self) -> Axis {
        [Axis::Extrusion, Axis::ProfileX, Axis::ProfileY]
            .into_iter()
            .find(|axis| *axis != self.length && *axis != self.height)
            .unwrap_or(Axis::Extrusion)
    }
}

/// A host's section frame, extrusion and bounds.
pub(crate) struct Host {
    pub(crate) origin: Vector,
    axes: [Vector; 3],
    /// Bounds along `Extrusion`, `ProfileX`, `ProfileY`: for a free
    /// outline, the box that holds it.
    bounds: [Span; 3],
    /// The section's outline where it is a free polygon; `None` for a
    /// parameterised section, which fills its bounds.
    pub(crate) outline: Option<Rc<Polygon>>,
    /// The part of `ProfileY` between the flanges, where the family has one.
    pub(crate) web: Option<Span>,
    pub(crate) family: String,
    pub(crate) evidence: Vec<Evidence>,
}

impl Host {
    pub(crate) fn axis(&self, axis: Axis) -> (Vector, Span) {
        (self.axes[axis.index()], self.bounds[axis.index()])
    }

    /// The rectangle in the profile's plane an opening occupies, from its
    /// extents along the face's axes and across the host: the part of its
    /// extent `through` that lies within the host's bounds (all of them
    /// when it lies beside the host).
    pub(crate) fn section_rect(
        &self,
        axes: FaceAxes,
        length: Span,
        height: Span,
        through: Span,
    ) -> [Span; 2] {
        let across = |axis: Axis| {
            let bounds = self.bounds[axis.index()];
            let (low, high) = (through.0.max(bounds.0), through.1.min(bounds.1));
            if low < high { (low, high) } else { bounds }
        };
        [Axis::ProfileX, Axis::ProfileY].map(|axis| {
            if axis == axes.length {
                length
            } else if axis == axes.height {
                height
            } else {
                across(axis)
            }
        })
    }

    /// Whether `axis` crosses a free outline, so its bounds differ across
    /// the host.
    pub(crate) fn outlined(&self, axis: Axis) -> bool {
        self.outline.is_some() && axis.profile().is_some()
    }

    /// The clear distance along `axis` from an opening's `extent` to the
    /// host's boundary, towards lower and towards higher coordinates;
    /// across a free outline, from the opening's `rect` in the profile's
    /// plane, over its whole width. `None` when the outline passes through
    /// `rect` or it lies outside the outline.
    pub(crate) fn clearance(&self, axis: Axis, extent: Span, rect: [Span; 2]) -> Option<Span> {
        if let (Some(outline), Some(index)) = (&self.outline, axis.profile()) {
            return outline.clearance(rect, index);
        }
        let bounds = self.bounds[axis.index()];
        Some((extent.0 - bounds.0, bounds.1 - extent.1))
    }
}

/// Reads a host: one straight extrusion perpendicular to a section whose
/// outline the body set bounds.
pub(crate) fn read_host(context: &RuleContext<'_>, id: &ObjectId) -> Result<Host, Unavailable> {
    let object = context
        .project
        .object(id)
        .ok_or_else(|| invalid(format!("host {id} is not in the project")))?;
    let mut body = BodyFacts::of(context, object)?;
    let host = (|| {
        let swept = swept(&mut body, "host")?;
        if !parallel(swept.direction, swept.normal) {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the host is extruded obliquely to its profile".to_owned(),
            ));
        }
        let family = family(&mut body)?;
        let (bounds, web, outline) = if ARBITRARY.contains(&family.as_str()) {
            let outline = polygon(&mut body, &family, "host")?;
            (outline.bounds(), None, Some(Rc::new(outline)))
        } else {
            let ((half_x, half_y), web) = boxed_section(&mut body, &family, "host")?;
            ([(-half_x, half_x), (-half_y, half_y)], web, None)
        };
        Ok(Host {
            origin: swept.origin,
            axes: [swept.direction, swept.x, swept.y],
            bounds: [(0.0, swept.depth), bounds[0], bounds[1]],
            outline,
            web,
            family,
            evidence: Vec::new(),
        })
    })();
    match host {
        Ok(mut host) => {
            host.evidence = body.into_evidence();
            Ok(host)
        }
        Err((reason, message)) => Err((reason, format!("host {id}: {message}"))),
    }
}

/// A section outline known exactly: a parameterised one centred on its
/// position, or a free polygon in the profile's coordinates.
#[derive(Clone)]
pub(crate) enum Shape {
    Rectangle {
        half_x: f64,
        half_y: f64,
    },
    Rounded {
        half_x: f64,
        half_y: f64,
        radius: f64,
    },
    Circle {
        radius: f64,
    },
    Ellipse {
        semi_x: f64,
        semi_y: f64,
    },
    Polygon(Rc<Polygon>),
}

impl Shape {
    /// How far the outline reaches along `(p, q)` in its own frame, least
    /// and most.
    fn range(&self, p: f64, q: f64) -> Span {
        let symmetric = |reach: f64| (-reach, reach);
        match self {
            Self::Rectangle { half_x, half_y } => symmetric(half_x * p.abs() + half_y * q.abs()),
            Self::Rounded {
                half_x,
                half_y,
                radius,
            } => symmetric(
                (half_x - radius) * p.abs() + (half_y - radius) * q.abs() + radius * p.hypot(q),
            ),
            Self::Circle { radius } => symmetric(radius * p.hypot(q)),
            Self::Ellipse { semi_x, semi_y } => symmetric((semi_x * p).hypot(semi_y * q)),
            Self::Polygon(polygon) => polygon.range(p, q),
        }
    }

    /// The area the outline encloses, less its voids.
    pub(crate) fn area(&self) -> f64 {
        match *self {
            Self::Rectangle { half_x, half_y } => 4.0 * half_x * half_y,
            Self::Rounded {
                half_x,
                half_y,
                radius,
            } => 4.0 * half_x * half_y - (4.0 - std::f64::consts::PI) * radius * radius,
            Self::Circle { radius } => std::f64::consts::PI * radius * radius,
            Self::Ellipse { semi_x, semi_y } => std::f64::consts::PI * semi_x * semi_y,
            Self::Polygon(ref polygon) => polygon.area(),
        }
    }
}

/// A section's outline: known exactly, or only by its bounding box, which
/// the section reaches on all four sides (every family the body set
/// bounds is centred on its position with its full width and depth).
#[derive(Clone)]
enum Outline {
    Exact(Shape),
    Boxed { half_x: f64, half_y: f64 },
}

impl Outline {
    /// How far the outline reaches along `(p, q)`: at least, and at most.
    ///
    /// A boxed section reaches its box's sides, somewhere along them: along
    /// `(p, q)` at least `half_x |p| - half_y |q|` and `half_y |q| -
    /// half_x |p|`, and at most the box's own reach.
    ///
    /// Each is a range from the section's position, least and most.
    fn reach(&self, p: f64, q: f64) -> (Span, Span) {
        match self {
            Self::Exact(shape) => {
                let range = shape.range(p, q);
                (range, range)
            }
            Self::Boxed { half_x, half_y } => {
                let (x, y) = (half_x * p.abs(), half_y * q.abs());
                let inner = (x - y).abs();
                ((-inner, inner), (-(x + y), x + y))
            }
        }
    }
}

/// A section frame in world coordinates and the straight extrusion from it.
pub(crate) struct Swept {
    origin: Vector,
    x: Vector,
    y: Vector,
    /// The placement's Z axis, normal to the profile plane.
    normal: Vector,
    direction: Vector,
    depth: f64,
}

/// An extent along a face axis: an interval sure to lie within the true
/// extent, and one sure to hold it. Both are the same when it is exact.
#[derive(Clone, Copy)]
pub(crate) struct Extent {
    pub(crate) inner: Span,
    pub(crate) outer: Span,
}

impl Extent {
    pub(crate) fn is_exact(self) -> bool {
        (self.outer.0 - self.inner.0).abs() <= ROUNDING
            && (self.outer.1 - self.inner.1).abs() <= ROUNDING
    }
}

/// One straight extrusion read from an object's body facts.
pub(crate) struct Solid {
    swept: Swept,
    outline: Outline,
    family: String,
    pub(crate) evidence: Vec<Evidence>,
}

impl Solid {
    /// Reads an opening: one straight extrusion of a rectangle, rounded
    /// rectangle, circle, ellipse or free polygon.
    pub(crate) fn opening(context: &RuleContext<'_>, object: &Object) -> Result<Self, Unavailable> {
        let mut body = BodyFacts::of(context, object)?;
        let swept = swept(&mut body, "opening")?;
        let family = family(&mut body)?;
        let shape = exact_shape(&mut body, &family, "opening")?.ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!("the opening's `{family}` profile has no outline its extent is known for"),
            )
        })?;
        Ok(Self {
            swept,
            outline: Outline::Exact(shape),
            family,
            evidence: body.into_evidence(),
        })
    }

    /// Reads a support or connecting member: one straight extrusion of any
    /// section the body set bounds, its outline exact where the family
    /// states it and otherwise bounded by its box.
    pub(crate) fn member(
        context: &RuleContext<'_>,
        object: &Object,
        role: &str,
    ) -> Result<Self, Unavailable> {
        let mut body = BodyFacts::of(context, object)?;
        let read = (|| {
            let swept = swept(&mut body, role)?;
            let family = family(&mut body)?;
            let outline = if let Some(shape) = exact_shape(&mut body, &family, role)? {
                Outline::Exact(shape)
            } else {
                let ((half_x, half_y), _) = boxed_section(&mut body, &family, role)?;
                Outline::Boxed { half_x, half_y }
            };
            Ok((swept, outline, family))
        })();
        let (swept, outline, family) = read?;
        Ok(Self {
            swept,
            outline,
            family,
            evidence: body.into_evidence(),
        })
    }

    /// The extent along the unit axis `w`, from `origin`: the reach of the
    /// outline both ways, swept along the extrusion.
    pub(crate) fn extent(&self, origin: Vector, w: Vector) -> Extent {
        let swept = &self.swept;
        let centre = dot(w, minus(swept.origin, origin));
        let (inner, outer) = self.outline.reach(dot(w, swept.x), dot(w, swept.y));
        let sweep = swept.depth * dot(w, swept.direction);
        let span = |reach: Span| {
            (
                centre + reach.0 + sweep.min(0.0),
                centre + reach.1 + sweep.max(0.0),
            )
        };
        Extent {
            inner: span(inner),
            outer: span(outer),
        }
    }

    /// Whether the section is the exact rectangle it states, its sides along
    /// `length` and `height`.
    pub(crate) fn aligned_rectangle(&self, length: Vector, height: Vector) -> bool {
        let aligned = |a: Vector| parallel(a, length) || parallel(a, height);
        matches!(self.outline, Outline::Exact(Shape::Rectangle { .. }))
            && self.family == "rectangle"
            && aligned(self.swept.x)
            && aligned(self.swept.y)
    }

    /// Whether the section lies across `axis` and is extruded along it.
    pub(crate) fn extruded_along(&self, axis: Vector) -> bool {
        parallel(self.swept.direction, axis) && parallel(self.swept.normal, axis)
    }

    /// Whether the extrusion runs along `axis`, the section in any plane.
    pub(crate) fn direction_along(&self, axis: Vector) -> bool {
        parallel(self.swept.direction, axis)
    }

    /// The area the section encloses, where its outline is exact and has
    /// no void.
    pub(crate) fn section_area(&self) -> Option<f64> {
        match &self.outline {
            Outline::Exact(shape) if !self.family.ends_with("-hollow") => Some(shape.area()),
            _ => None,
        }
    }

    pub(crate) fn family(&self) -> &str {
        &self.family
    }
}

/// Reads a straight extrusion of one item and its section frame.
fn swept(body: &mut BodyFacts<'_>, role: &str) -> Result<Swept, Unavailable> {
    let incomplete = |message: String| (NotEvaluatedReason::IncompleteEvidence, message);
    match body.integer("Count")? {
        None => return Err(incomplete(format!("the {role} has no body"))),
        Some(1) => {}
        Some(count) => {
            return Err(incomplete(format!(
                "the {role}'s body has {count} items; one straight extrusion is needed"
            )));
        }
    }
    let kind = body.text("Kind")?.unwrap_or_else(|| "unstated".to_owned());
    if kind != "extrusion" {
        return Err(incomplete(format!(
            "the {role}'s body is a `{kind}`, not a straight extrusion"
        )));
    }
    let origin = body.point("Placement.Origin")?;
    let x = body.vector("Placement.XAxis")?;
    let y = body.vector("Placement.YAxis")?;
    let normal = body.vector("Placement.ZAxis")?;
    let offset_x = body.length("Profile.PositionX")?.unwrap_or(0.0);
    let offset_y = body.length("Profile.PositionY")?.unwrap_or(0.0);
    let angle = body.angle("Profile.PositionAngle")?.unwrap_or(0.0);
    let (sin, cos) = angle.sin_cos();
    Ok(Swept {
        origin: along(origin, x, offset_x, y, offset_y),
        x: along([0.0; 3], x, cos, y, sin),
        y: along([0.0; 3], x, -sin, y, cos),
        normal,
        direction: body.vector("Extrusion.Direction")?,
        depth: body.required_length("Extrusion.Depth")?,
    })
}

fn family(body: &mut BodyFacts<'_>) -> Result<String, Unavailable> {
    Ok(body
        .text("Profile.Type")?
        .unwrap_or_else(|| "unstated".to_owned()))
}

fn required(body: &mut BodyFacts<'_>, name: &str) -> Result<f64, Unavailable> {
    body.required_length(&format!("Profile.{name}"))
}

/// Half extents of a section and the part of its height between the
/// flanges.
type Section = ((f64, f64), Option<Span>);

/// Half extents and the web of a section, centred on its position.
fn boxed_section(
    body: &mut BodyFacts<'_>,
    family: &str,
    role: &str,
) -> Result<Section, Unavailable> {
    let flanged = |body: &mut BodyFacts<'_>, width: &str, thickness: &str| {
        let half_depth = required(body, "OverallDepth").or_else(|_| required(body, "Depth"))? / 2.0;
        let flange = required(body, thickness)?;
        Ok::<_, Unavailable>((
            (required(body, width)? / 2.0, half_depth),
            Some((-half_depth + flange, half_depth - flange)),
        ))
    };
    Ok(match family {
        "rectangle" | "rounded-rectangle" | "rectangle-hollow" => (
            (required(body, "XDim")? / 2.0, required(body, "YDim")? / 2.0),
            None,
        ),
        "circle" | "circle-hollow" => {
            let radius = required(body, "Radius")?;
            ((radius, radius), None)
        }
        "ellipse" => (
            (required(body, "SemiAxis1")?, required(body, "SemiAxis2")?),
            None,
        ),
        "i-shape" => flanged(body, "OverallWidth", "FlangeThickness")?,
        "u-shape" => flanged(body, "FlangeWidth", "FlangeThickness")?,
        "c-shape" => flanged(body, "Width", "WallThickness")?,
        "z-shape" => {
            let half_depth = required(body, "Depth")? / 2.0;
            let flange = required(body, "FlangeThickness")?;
            let width = 2.0 * required(body, "FlangeWidth")? - required(body, "WebThickness")?;
            (
                (width / 2.0, half_depth),
                Some((-half_depth + flange, half_depth - flange)),
            )
        }
        "t-shape" => {
            let half_depth = required(body, "Depth")? / 2.0;
            let flange = required(body, "FlangeThickness")?;
            (
                (required(body, "FlangeWidth")? / 2.0, half_depth),
                Some((-half_depth, half_depth - flange)),
            )
        }
        "l-shape" => (
            (
                required(body, "Width")? / 2.0,
                required(body, "Depth")? / 2.0,
            ),
            None,
        ),
        "asymmetric-i-shape" => {
            let half_depth = required(body, "OverallDepth")? / 2.0;
            let width = required(body, "BottomFlangeWidth")?.max(required(body, "TopFlangeWidth")?);
            let bottom = required(body, "BottomFlangeThickness")?;
            let top = required(body, "TopFlangeThickness")?;
            (
                (width / 2.0, half_depth),
                Some((-half_depth + bottom, half_depth - top)),
            )
        }
        other => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the {role}'s `{other}` profile states no outline to bound it by"),
            ));
        }
    })
}

/// The profile families the body set states a free outline for.
const ARBITRARY: &[&str] = &["arbitrary-closed", "arbitrary-with-voids"];

/// A free outline and its voids, as the body set states them.
fn polygon(body: &mut BodyFacts<'_>, family: &str, role: &str) -> Result<Polygon, Unavailable> {
    let ring = |body: &mut BodyFacts<'_>, prefix: &str| {
        let x = body.required_lengths(&format!("Profile.{prefix}OutlineX"))?;
        let y = body.required_lengths(&format!("Profile.{prefix}OutlineY"))?;
        if x.len() != y.len() {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "the {role}'s `Profile.{prefix}OutlineX` has {} coordinates, \
                     `Profile.{prefix}OutlineY` {}",
                    x.len(),
                    y.len()
                ),
            ));
        }
        Ok(x.into_iter()
            .zip(y)
            .map(|(x, y)| [x, y])
            .collect::<Vec<_>>())
    };
    let outer = ring(body, "")?;
    let mut voids = Vec::new();
    if family == "arbitrary-with-voids" {
        let count = body
            .integer("Profile.VoidCount")?
            .ok_or_else(|| invalid(format!("the {role}'s profile states no `VoidCount`")))?;
        for void in 1..=count {
            voids.push(ring(body, &format!("Void{void}."))?);
        }
    }
    Polygon::new(outer, voids).map_err(|message| {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!("the {role}'s profile outline is not one region: {message}"),
        )
    })
}

/// The exact outline of a rectangle, rounded rectangle, circle, ellipse
/// (a hollow one reaches as far as its outer outline) or free polygon;
/// `None` for another family.
fn exact_shape(
    body: &mut BodyFacts<'_>,
    family: &str,
    role: &str,
) -> Result<Option<Shape>, Unavailable> {
    if ARBITRARY.contains(&family) {
        return Ok(Some(Shape::Polygon(Rc::new(polygon(body, family, role)?))));
    }
    Ok(Some(match family {
        "rectangle" | "rectangle-hollow" => Shape::Rectangle {
            half_x: required(body, "XDim")? / 2.0,
            half_y: required(body, "YDim")? / 2.0,
        },
        "rounded-rectangle" => Shape::Rounded {
            half_x: required(body, "XDim")? / 2.0,
            half_y: required(body, "YDim")? / 2.0,
            radius: required(body, "RoundingRadius")?,
        },
        "circle" | "circle-hollow" => Shape::Circle {
            radius: required(body, "Radius")?,
        },
        "ellipse" => Shape::Ellipse {
            semi_x: required(body, "SemiAxis1")?,
            semi_y: required(body, "SemiAxis2")?,
        },
        _ => return Ok(None),
    }))
}
