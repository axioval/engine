//! `component-clearance`: a free box or cylinder in front of, behind or
//! beside each selected component, placed in the component's own frame on
//! a side the rule states, fixed or sliding sideways and away from it.

use std::collections::BTreeSet;

use axioval_engine::{
    BoxClearance, CapabilityEvaluation, ClearanceOutcome, ClearanceRequest, ClearanceShape,
    CompiledRule, ContainmentOutcome, ContainmentRequest, CylinderClearance, ElevationBand,
    FrameOffsetPlacement, FreeSpaceError, FreeSpaceServiceHandle, MetricDirection, MetricFrame,
    MetricPoint, NotEvaluatedReason, ObjectFrame, ObjectFrameServiceHandle, ObjectFront,
    ParameterDescriptor, ParameterType, PlacementDomain, PlacementOrientation, PlacementOutcome,
    PlacementRequest, PlacementShape, PlanSpanServiceHandle, RuleCapability, RuleContext,
    SignedDistanceInterval, SupportCoverageOutcome, SupportCoverageRequest,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::body_extent::{extent_error, frame_error};
use crate::door_swing;
use crate::level_spacing::{extent, metres, shown};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};
use crate::wall_sides::Walls;

/// Requires a free volume on a stated side of each selected component.
///
/// The volume is placed in the component's placement frame. The rule states
/// which frame axis is the component's front (`front_axis`: `forward`,
/// `-forward`, `right` or `-right`, or `stated` for the front the source
/// states), because a placement axis says nothing about which way a
/// component faces; the engine never infers one from its axes. With
/// `against-wall` the front is derived from the walls (`wall_selector`)
/// beside the sides of the footprint's least-area rectangle, within
/// `wall_reach` of its centre lines, each side's strip narrowed by
/// `wall_inset`: the side surely nearer a wall than every other side is the
/// back, and the front faces away from it. A tie, or no wall within reach,
/// leaves the front not decided, never guessed. For a door, `swing` is the
/// side its hinged leaves open towards and `-swing` the other, as its
/// leaves state them; `align` `handle` or `hinge` puts the volume flush with
/// the edge of its single hinged leaf's handle or hinge. `side` is `front`,
/// `back`, `left` or `right` of that front, left and right as seen facing
/// along it (the component's own left and right); `both_sides` adds the
/// opposite side as a check of its own. `sides` lists several instead,
/// with `quantifier` `all` (each is checked on its own, the default) or
/// `any` (one free side is enough, such as a bed with one free long side).
///
/// The volume is a box (`width` across the side, `depth` away from it) or a
/// cylinder (`radius`), `height` high. Each of width, depth and height is
/// `fixed` (the default), or sized from the component (`*_mode`
/// `component_plus`: its own dimension plus the stated one, or
/// `component_clamped`: that, clamped between `*_minimum` and
/// `*_maximum`). Along the side it starts at the component's outermost
/// point (`depth_from` `face`, the default) or its midline (`midline`)
/// plus `offset`; across it, it is centred on the component or flush with
/// its left or right edge (`align`, as seen looking out of that side),
/// moved by `lateral_offset`; its base lies `vertical_offset` above the
/// `height_reference`: the floor (the lowest point of the spaces
/// `space_path` reaches), or the component's bottom or top. With
/// `top_datum` (the same three) its top lies `top_offset` above that datum
/// instead of `height` above the base.
///
/// `size_mode` says what the size is: a `minimum` (the default: the volume,
/// less `size_tolerance` in every dimension, must be free), a `maximum` (no
/// volume `size_tolerance` larger in any one dimension may be free) or
/// `fixed` (both).
///
/// With `slide_from` and `slide_to`, the volume floats across the side: it
/// may slide by any offset between the two, to the right as seen looking
/// out of it; with `depth_slide_from` and `depth_slide_to` it floats away
/// from the component by any offset between those. It is free when it is
/// free at one of them. A floating volume is searched in the spaces
/// `space_path` reaches, so it always lies inside them.
///
/// Obstacles are the `obstacles` selection less the component itself and
/// the `allowed_intruders`. With `protrusion`, an obstacle may reach up to
/// that far into the volume through any of its plan sides: the volume
/// checked is the declared one shrunk by `protrusion` on every plan side,
/// so an obstacle it meets reaches further in than allowed, and one it
/// misses reaches no further. With `within_space`, the declared volume's
/// plan must also lie inside the union of the spaces `space_path` reaches,
/// judged on its own. With `support_selector`, the declared volume's plan
/// must lie wholly on the tops of the selected bodies (a slab, a landing)
/// within `support_tolerance` of its base.
///
/// Positions are measured intervals, so the volume's position is too. The
/// check measures the union of every position the volume could take (free
/// there means free wherever it is) and their common part (obstructed there
/// means obstructed wherever it is); anything between is not evaluated. An
/// obstacle the selection cannot decide can only obstruct, so a free volume
/// stands and an obstruction by undecided objects alone is not evaluated.
pub struct ComponentClearance;

const ID: &str = "axioval:capability.component-clearance";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Front,
    Back,
    Left,
    Right,
}

impl Side {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "front" => Ok(Self::Front),
            "back" => Ok(Self::Back),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            other => Err(invalid(format!(
                "side `{other}` is unsupported; use `front`, `back`, `left` or `right`"
            ))),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::Front => Self::Back,
            Self::Back => Self::Front,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// The outward direction of this side, given the front and up.
    fn outward(self, front: [f64; 3], up: [f64; 3]) -> [f64; 3] {
        // Facing along the front with up overhead, right is front × up.
        let right = cross(front, up);
        match self {
            Self::Front => front,
            Self::Back => negate(front),
            Self::Right => right,
            Self::Left => negate(right),
        }
    }
}

/// Whether every listed side must be free, or one is enough.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quantifier {
    All,
    Any,
}

#[derive(Clone, Copy)]
enum FrontAxis {
    Forward,
    Backward,
    Right,
    Left,
    Stated,
    /// The side a door's leaves swing into.
    Swing,
    /// The side a door's leaves swing away from.
    Push,
    /// Away from the wall the component stands against.
    AgainstWall,
}

impl FrontAxis {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "forward" => Ok(Self::Forward),
            "-forward" => Ok(Self::Backward),
            "right" => Ok(Self::Right),
            "-right" => Ok(Self::Left),
            "stated" => Ok(Self::Stated),
            "swing" => Ok(Self::Swing),
            "-swing" => Ok(Self::Push),
            "against-wall" => Ok(Self::AgainstWall),
            other => Err(invalid(format!(
                "front_axis `{other}` is unsupported; use `forward`, `-forward`, `right`, \
                 `-right`, `stated`, `swing`, `-swing` or `against-wall`"
            ))),
        }
    }

    fn of(self, frame: &ObjectFrame) -> Result<[f64; 3], Unavailable> {
        let axes = frame.frame();
        match self {
            Self::Forward => Ok(axes.forward().components()),
            Self::Backward => Ok(negate(axes.forward().components())),
            Self::Right => Ok(axes.right().components()),
            Self::Left => Ok(negate(axes.right().components())),
            Self::Stated => match frame.front() {
                ObjectFront::Stated(front) => Ok(front.components()),
                ObjectFront::NotStated => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    "the source states no front for this component; declare `front_axis` as \
                     one of its placement axes"
                        .into(),
                )),
            },
            Self::Swing | Self::Push => unreachable!("a swing side comes from the leaves"),
            Self::AgainstWall => unreachable!("a wall's side comes from the footprint"),
        }
    }

    fn is_swing(self) -> bool {
        matches!(self, Self::Swing | Self::Push)
    }
}

#[derive(Clone, Copy)]
enum Align {
    Centre,
    Left,
    Right,
    /// Flush with the edge a door's single hinged leaf has its handle at.
    Handle,
    /// Flush with the edge that leaf is hinged at.
    Hinge,
}

#[derive(Clone, Copy)]
enum Reference {
    Floor,
    Bottom,
    Top,
}

impl Reference {
    fn name(self) -> &'static str {
        match self {
            Self::Floor => "floor",
            Self::Bottom => "component's bottom",
            Self::Top => "component's top",
        }
    }
}

/// Where along the side the volume starts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthFrom {
    /// The component's outermost point on the side.
    Face,
    /// Its midline between that side and the opposite one.
    Midline,
}

/// How one dimension of the volume is sized.
#[derive(Clone, Copy)]
enum Sizing {
    /// The stated length.
    Fixed(f64),
    /// The component's own dimension plus the stated length.
    Plus(f64),
    /// That, clamped between a minimum and a maximum.
    Clamped {
        add: f64,
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
}

impl Sizing {
    /// The length as an interval, from the component's dimension (asked
    /// only when the sizing needs it).
    fn resolve(
        self,
        component: impl FnOnce() -> Result<(f64, f64), Unavailable>,
    ) -> Result<(f64, f64), Unavailable> {
        match self {
            Self::Fixed(length) => Ok((length, length)),
            Self::Plus(add) => {
                let (low, high) = component()?;
                Ok((low + add, high + add))
            }
            Self::Clamped {
                add,
                minimum,
                maximum,
            } => {
                let (low, high) = component()?;
                // Clamping is monotone, so it maps the interval's ends.
                let clamp = |value: f64| {
                    let value = minimum.map_or(value, |minimum| value.max(minimum));
                    maximum.map_or(value, |maximum| value.min(maximum))
                };
                Ok((clamp(low + add), clamp(high + add)))
            }
        }
    }

    fn fixed(self) -> Option<f64> {
        match self {
            Self::Fixed(length) => Some(length),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Box {
        width: (f64, f64),
        depth: (f64, f64),
    },
    Cylinder {
        radius: f64,
    },
}

/// How high the volume reaches.
#[derive(Clone, Copy)]
enum Height {
    /// This high above its base.
    Of((f64, f64)),
    /// Up to the top datum, raised by this much.
    ToTop(f64),
}

/// A volume's plan shape and height, each dimension an interval.
#[derive(Clone, Copy)]
struct Size {
    shape: Shape,
    height: Height,
}

fn add((low, high): (f64, f64), by: f64) -> (f64, f64) {
    (low + by, high + by)
}

/// A length interval as a message shows it.
fn length_shown((low, high): (f64, f64)) -> String {
    shown(low, high)
}

impl Size {
    fn describe(self, config: &Config<'_>) -> String {
        let height = match self.height {
            Height::Of(height) => format!("{} high", length_shown(height)),
            Height::ToTop(raised) => {
                let (reference, offset) = config.top.map_or((Reference::Top, 0.0), |top| top);
                format!(
                    "up to {} above the {}",
                    metres(offset + raised),
                    reference.name()
                )
            }
        };
        match self.shape {
            Shape::Box { width, depth } => format!(
                "{} wide, {} deep, {height}",
                length_shown(width),
                length_shown(depth)
            ),
            Shape::Cylinder { radius } => {
                format!("{} in radius, {height}", metres(radius))
            }
        }
    }

    fn raised(height: Height, by: f64) -> Height {
        match height {
            Height::Of(height) => Height::Of(add(height, by)),
            Height::ToTop(raised) => Height::ToTop(raised + by),
        }
    }

    /// Every dimension changed by `by`.
    fn changed(self, by: f64) -> Self {
        Self {
            shape: match self.shape {
                Shape::Box { width, depth } => Shape::Box {
                    width: add(width, by),
                    depth: add(depth, by),
                },
                Shape::Cylinder { radius } => Shape::Cylinder {
                    radius: radius + by,
                },
            },
            height: Self::raised(self.height, by),
        }
    }

    /// The sizes `by` larger in one dimension each, with that dimension's
    /// name.
    fn larger(self, by: f64) -> Vec<(&'static str, Self)> {
        let taller = Self {
            height: Self::raised(self.height, by),
            ..self
        };
        let mut sizes = match self.shape {
            Shape::Box { width, depth } => vec![
                (
                    "width",
                    Self {
                        shape: Shape::Box {
                            width: add(width, by),
                            depth,
                        },
                        ..self
                    },
                ),
                (
                    "depth",
                    Self {
                        shape: Shape::Box {
                            width,
                            depth: add(depth, by),
                        },
                        ..self
                    },
                ),
            ],
            Shape::Cylinder { radius } => vec![(
                "radius",
                Self {
                    shape: Shape::Cylinder {
                        radius: radius + by,
                    },
                    ..self
                },
            )],
        };
        sizes.push(("height", taller));
        sizes
    }
}

/// The declared dimensions, before the component sizes them.
#[derive(Clone, Copy)]
enum Declared {
    Box {
        width: Sizing,
        depth: Sizing,
        height: Option<Sizing>,
    },
    Cylinder {
        radius: f64,
        height: Option<Sizing>,
    },
}

impl Declared {
    /// The plan dimensions and height when every one is fixed.
    fn fixed(self) -> Option<(f64, f64, Option<f64>)> {
        let height = |height: Option<Sizing>| match height {
            None => Some(None),
            Some(height) => height.fixed().map(Some),
        };
        match self {
            Self::Box {
                width,
                depth,
                height: tall,
            } => Some((width.fixed()?, depth.fixed()?, height(tall)?)),
            Self::Cylinder {
                radius,
                height: tall,
            } => Some((radius, radius, height(tall)?)),
        }
    }
}

/// What the declared size bounds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SizeMode {
    Minimum,
    Maximum,
    Fixed,
}

struct Config<'a> {
    sides: Vec<Side>,
    quantifier: Quantifier,
    front: FrontAxis,
    declared: Declared,
    mode: SizeMode,
    tolerance: f64,
    offset: f64,
    depth_from: DepthFrom,
    lateral_offset: f64,
    align: Align,
    reference: Reference,
    vertical_offset: f64,
    /// The top datum and the offset above it, instead of a height.
    top: Option<(Reference, f64)>,
    obstacles: &'a Selector,
    allowed: Option<&'a Selector>,
    protrusion: f64,
    within_space: bool,
    /// The offsets across the side a floating volume may slide by.
    slide: Option<(f64, f64)>,
    /// The offsets away from the component it may slide by.
    depth_slide: Option<(f64, f64)>,
    spaces: Option<Traversal<'a>>,
    /// With `against-wall`: the walls, how far to look and the inset.
    walls: Option<(&'a Selector, f64, f64)>,
    /// What must hold the volume's base, and how far from it vertically.
    support: Option<(&'a Selector, f64)>,
}

impl Config<'_> {
    fn floats(&self) -> bool {
        self.slide.is_some() || self.depth_slide.is_some()
    }
}

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value.is_finite() => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("`{name}` is not finite"))),
        Some(_) => Err(invalid(format!("`{name}` is not a length"))),
    }
}

fn positive(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match length(parameters, name)? {
        Some(value) if value <= 0.0 => Err(invalid(format!("`{name}` must be positive"))),
        other => Ok(other),
    }
}

fn non_negative(parameters: &Parameters<'_>, name: &str) -> Result<f64, Unavailable> {
    match length(parameters, name)? {
        Some(value) if value < 0.0 => Err(invalid(format!("`{name}` is negative"))),
        other => Ok(other.unwrap_or(0.0)),
    }
}

/// How `dimension` is sized: its mode, the stated length and the clamps.
fn sizing(
    parameters: &Parameters<'_>,
    dimension: &str,
    required: bool,
) -> Result<Option<Sizing>, Unavailable> {
    let (minimum, maximum) = (
        positive(parameters, &format!("{dimension}_minimum"))?,
        positive(parameters, &format!("{dimension}_maximum"))?,
    );
    let clamps = minimum.is_some() || maximum.is_some();
    let sizing = match parameters.string(&format!("{dimension}_mode"))? {
        None | Some("fixed") => positive(parameters, dimension)?.map(Sizing::Fixed),
        Some("component_plus") => Some(Sizing::Plus(length(parameters, dimension)?.unwrap_or(0.0))),
        Some("component_clamped") => {
            if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
                if minimum > maximum {
                    return Err(invalid(format!(
                        "`{dimension}_minimum` exceeds `{dimension}_maximum`"
                    )));
                }
            }
            if !clamps {
                return Err(invalid(format!(
                    "`{dimension}_mode` `component_clamped` needs `{dimension}_minimum`, \
                     `{dimension}_maximum` or both"
                )));
            }
            Some(Sizing::Clamped {
                add: length(parameters, dimension)?.unwrap_or(0.0),
                minimum,
                maximum,
            })
        }
        Some(other) => {
            return Err(invalid(format!(
                "{dimension}_mode `{other}` is unsupported; use `fixed`, `component_plus` or \
                 `component_clamped`"
            )));
        }
    };
    if clamps && !matches!(sizing, Some(Sizing::Clamped { .. })) {
        return Err(invalid(format!(
            "`{dimension}_minimum` and `{dimension}_maximum` apply only to `{dimension}_mode` \
             `component_clamped`"
        )));
    }
    if required && sizing.is_none() {
        return Err(invalid(format!("declare `{dimension}`")));
    }
    Ok(sizing)
}

/// The declared size, its mode and tolerance, checked against `protrusion`
/// where the size is fixed.
fn sizes(
    parameters: &Parameters<'_>,
    protrusion: f64,
    to_top: bool,
) -> Result<(Declared, SizeMode, f64), Unavailable> {
    let height = if to_top {
        if parameters.0.parameters.contains_key("height")
            || parameters.0.parameters.contains_key("height_mode")
        {
            return Err(invalid(
                "declare either `height` or `top_datum`: with a top datum the height is the \
                 distance between the two",
            ));
        }
        None
    } else {
        Some(sizing(parameters, "height", false)?.ok_or_else(|| {
            invalid("declare `height`, or `top_datum` for a volume up to a datum")
        })?)
    };
    let radius = positive(parameters, "radius")?;
    let boxed = ["width", "depth", "width_mode", "depth_mode"]
        .iter()
        .any(|name| parameters.0.parameters.contains_key(*name));
    let declared = match (radius, boxed) {
        (None, true) => Declared::Box {
            width: sizing(parameters, "width", true)?.expect("required"),
            depth: sizing(parameters, "depth", true)?.expect("required"),
            height,
        },
        (Some(radius), false) => Declared::Cylinder { radius, height },
        _ => {
            return Err(invalid(
                "declare `width` and `depth` for a box or `radius` for a cylinder",
            ));
        }
    };
    let mode = match parameters.string("size_mode")? {
        None | Some("minimum") => SizeMode::Minimum,
        Some("maximum") => SizeMode::Maximum,
        Some("fixed") => SizeMode::Fixed,
        Some(other) => {
            return Err(invalid(format!(
                "size_mode `{other}` is unsupported; use `minimum`, `maximum` or `fixed`"
            )));
        }
    };
    let tolerance = non_negative(parameters, "size_tolerance")?;
    if mode != SizeMode::Minimum && tolerance <= 0.0 {
        return Err(invalid(
            "a `maximum` or `fixed` size needs a positive `size_tolerance`: the free volume \
             that larger in one dimension is the one that must not fit",
        ));
    }
    // Sizes the component gives are checked once measured.
    if let Some((across, along, height)) = declared.fixed() {
        let cylinder = matches!(declared, Declared::Cylinder { .. });
        let holds = |by: f64| {
            height.is_none_or(|height| height + by > 0.0)
                && if cylinder {
                    protrusion < across + by
                } else {
                    2.0 * protrusion < (across + by).min(along + by)
                }
        };
        if !holds(0.0) {
            return Err(invalid(if cylinder {
                "`protrusion` leaves no volume: it must be less than the radius"
            } else {
                "`protrusion` leaves no volume: it must be less than half the width and the \
                 depth"
            }));
        }
        if mode != SizeMode::Maximum && !holds(-tolerance) {
            return Err(invalid(
                "`size_tolerance` and `protrusion` leave no volume of the minimum size",
            ));
        }
    }
    Ok((declared, mode, tolerance))
}

/// The offsets a floating volume may slide by, between `from` and `to`.
fn slide(
    parameters: &Parameters<'_>,
    from: &str,
    to: &str,
) -> Result<Option<(f64, f64)>, Unavailable> {
    match (length(parameters, from)?, length(parameters, to)?) {
        (None, None) => Ok(None),
        (Some(low), Some(high)) if low <= high => Ok(Some((low, high))),
        (Some(_), Some(_)) => Err(invalid(format!("`{from}` lies beyond `{to}`"))),
        _ => Err(invalid(format!("`{from}` and `{to}` go together"))),
    }
}

fn reference(value: &str, name: &str) -> Result<Reference, Unavailable> {
    match value {
        "floor" => Ok(Reference::Floor),
        "bottom" => Ok(Reference::Bottom),
        "top" => Ok(Reference::Top),
        other => Err(invalid(format!(
            "{name} `{other}` is unsupported; use `floor`, `bottom` or `top`"
        ))),
    }
}

/// The sides to check and how they combine.
fn sides(parameters: &Parameters<'_>) -> Result<(Vec<Side>, Quantifier), Unavailable> {
    let both = parameters.boolean("both_sides")?;
    let quantifier = match parameters.string("quantifier")? {
        None | Some("all") => Quantifier::All,
        Some("any") => Quantifier::Any,
        Some(other) => {
            return Err(invalid(format!(
                "quantifier `{other}` is unsupported; use `all` or `any`"
            )));
        }
    };
    match (parameters.string("side")?, parameters.strings("sides")?) {
        (Some(side), None) => {
            if quantifier == Quantifier::Any {
                return Err(invalid("`quantifier` applies only to `sides`"));
            }
            let side = Side::parse(side)?;
            Ok(if both.unwrap_or(false) {
                (vec![side, side.opposite()], Quantifier::All)
            } else {
                (vec![side], Quantifier::All)
            })
        }
        (None, Some(listed)) => {
            if both.is_some() {
                return Err(invalid("`both_sides` applies only to `side`"));
            }
            let mut sides = Vec::new();
            for side in listed {
                let side = Side::parse(side)?;
                if sides.contains(&side) {
                    return Err(invalid(format!("`sides` lists `{}` twice", side.name())));
                }
                sides.push(side);
            }
            if sides.is_empty() {
                return Err(invalid("`sides` lists no side"));
            }
            Ok((sides, quantifier))
        }
        _ => Err(invalid("declare exactly one of `side` and `sides`")),
    }
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let (sides, quantifier) = sides(&parameters)?;
        let front = FrontAxis::parse(parameters.required_string("front_axis")?)?;
        let walls = walls(&parameters, front)?;
        let top = match parameters.string("top_datum")? {
            Some(datum) => Some((
                reference(datum, "top_datum")?,
                length(&parameters, "top_offset")?.unwrap_or(0.0),
            )),
            None if parameters.0.parameters.contains_key("top_offset") => {
                return Err(invalid("`top_offset` applies only to `top_datum`"));
            }
            None => None,
        };
        let protrusion = non_negative(&parameters, "protrusion")?;
        let (declared, mode, tolerance) = sizes(&parameters, protrusion, top.is_some())?;
        let align = match parameters.string("align")? {
            None | Some("centre") => Align::Centre,
            Some("left") => Align::Left,
            Some("right") => Align::Right,
            Some("handle") => Align::Handle,
            Some("hinge") => Align::Hinge,
            Some(other) => {
                return Err(invalid(format!(
                    "align `{other}` is unsupported; use `centre`, `left`, `right`, `handle` or \
                     `hinge`"
                )));
            }
        };
        let depth_from = match parameters.string("depth_from")? {
            None | Some("face") => DepthFrom::Face,
            Some("midline") => DepthFrom::Midline,
            Some(other) => {
                return Err(invalid(format!(
                    "depth_from `{other}` is unsupported; use `face` or `midline`"
                )));
            }
        };
        let reference = reference(
            parameters.required_string("height_reference")?,
            "height_reference",
        )?;
        let slide = slide(&parameters, "slide_from", "slide_to")?;
        let depth_slide = slide_depth(&parameters)?;
        let floats = slide.is_some() || depth_slide.is_some();
        let within_space = parameters.boolean("within_space")?.unwrap_or(false);
        let support = supports(&parameters, floats)?;
        let spaces = match parameters.strings("space_path")? {
            Some(path) => Some(Traversal::path(path)?),
            None => None,
        };
        let needs_spaces = within_space
            || floats
            || matches!(reference, Reference::Floor)
            || matches!(top, Some((Reference::Floor, _)));
        match (needs_spaces, &spaces) {
            (true, None) => {
                return Err(invalid(
                    "a `floor` datum, `within_space` and a floating volume need `space_path`",
                ));
            }
            (false, Some(_)) => {
                return Err(invalid(
                    "`space_path` applies only to a `floor` datum, `within_space` or a \
                     floating volume",
                ));
            }
            _ => {}
        }
        Ok(Self {
            sides,
            quantifier,
            front,
            declared,
            mode,
            tolerance,
            offset: length(&parameters, "offset")?.unwrap_or(0.0),
            depth_from,
            lateral_offset: length(&parameters, "lateral_offset")?.unwrap_or(0.0),
            align,
            reference,
            vertical_offset: length(&parameters, "vertical_offset")?.unwrap_or(0.0),
            top,
            obstacles: parameters.required_selector("obstacles")?,
            allowed: parameters.selector("allowed_intruders")?,
            protrusion,
            within_space,
            slide,
            depth_slide,
            spaces,
            walls,
            support,
        })
    }
}

/// With `against-wall`: the walls, how far to look and the inset.
fn walls<'a>(
    parameters: &Parameters<'a>,
    front: FrontAxis,
) -> Result<Option<(&'a Selector, f64, f64)>, Unavailable> {
    let inset = length(parameters, "wall_inset")?;
    match (
        matches!(front, FrontAxis::AgainstWall),
        parameters.selector("wall_selector")?,
        positive(parameters, "wall_reach")?,
    ) {
        (true, Some(selector), Some(reach)) => Ok(Some((
            selector,
            reach,
            non_negative(parameters, "wall_inset")?,
        ))),
        (true, _, _) => Err(invalid(
            "`front_axis` `against-wall` needs `wall_selector` and `wall_reach`",
        )),
        (false, None, None) if inset.is_none() => Ok(None),
        (false, _, _) => Err(invalid(
            "`wall_selector`, `wall_reach` and `wall_inset` apply only to `front_axis` \
             `against-wall`",
        )),
    }
}

/// What must hold the volume's base, and how far from it vertically.
fn supports<'a>(
    parameters: &Parameters<'a>,
    floats: bool,
) -> Result<Option<(&'a Selector, f64)>, Unavailable> {
    match (
        parameters.selector("support_selector")?,
        length(parameters, "support_tolerance")?,
    ) {
        (None, None) => Ok(None),
        (Some(selector), Some(tolerance)) if tolerance >= 0.0 && !floats => {
            Ok(Some((selector, tolerance)))
        }
        (Some(_), Some(tolerance)) if tolerance >= 0.0 => Err(invalid(
            "`support_selector` applies to a fixed volume, not a floating one",
        )),
        _ => Err(invalid(
            "`support_selector` and a non-negative `support_tolerance` go together",
        )),
    }
}

/// The offsets away from the component a floating volume may slide by:
/// from its declared place outwards, never back into the component.
fn slide_depth(parameters: &Parameters<'_>) -> Result<Option<(f64, f64)>, Unavailable> {
    let depth = slide(parameters, "depth_slide_from", "depth_slide_to")?;
    if depth.is_some_and(|(from, _)| from < 0.0) {
        return Err(invalid("`depth_slide_from` is negative"));
    }
    Ok(depth)
}

impl RuleCapability for ComponentClearance {
    fn id(&self) -> &'static str {
        ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("side", ParameterType::String),
            ParameterDescriptor::optional("sides", ParameterType::StringList),
            ParameterDescriptor::optional("quantifier", ParameterType::String),
            ParameterDescriptor::required("front_axis", ParameterType::String),
            ParameterDescriptor::optional("both_sides", ParameterType::Boolean),
            ParameterDescriptor::optional("width", ParameterType::Quantity),
            ParameterDescriptor::optional("width_mode", ParameterType::String),
            ParameterDescriptor::optional("width_minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("width_maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("depth", ParameterType::Quantity),
            ParameterDescriptor::optional("depth_mode", ParameterType::String),
            ParameterDescriptor::optional("depth_minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("depth_maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("depth_from", ParameterType::String),
            ParameterDescriptor::optional("radius", ParameterType::Quantity),
            ParameterDescriptor::optional("height", ParameterType::Quantity),
            ParameterDescriptor::optional("height_mode", ParameterType::String),
            ParameterDescriptor::optional("height_minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("height_maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("size_mode", ParameterType::String),
            ParameterDescriptor::optional("size_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("offset", ParameterType::Quantity),
            ParameterDescriptor::optional("lateral_offset", ParameterType::Quantity),
            ParameterDescriptor::optional("align", ParameterType::String),
            ParameterDescriptor::optional("slide_from", ParameterType::Quantity),
            ParameterDescriptor::optional("slide_to", ParameterType::Quantity),
            ParameterDescriptor::optional("depth_slide_from", ParameterType::Quantity),
            ParameterDescriptor::optional("depth_slide_to", ParameterType::Quantity),
            ParameterDescriptor::required("height_reference", ParameterType::String),
            ParameterDescriptor::optional("vertical_offset", ParameterType::Quantity),
            ParameterDescriptor::optional("top_datum", ParameterType::String),
            ParameterDescriptor::optional("top_offset", ParameterType::Quantity),
            ParameterDescriptor::required("obstacles", ParameterType::Selector),
            ParameterDescriptor::optional("allowed_intruders", ParameterType::Selector),
            ParameterDescriptor::optional("protrusion", ParameterType::Quantity),
            ParameterDescriptor::optional("within_space", ParameterType::Boolean),
            ParameterDescriptor::optional("space_path", ParameterType::StringList),
            ParameterDescriptor::optional("wall_selector", ParameterType::Selector),
            ParameterDescriptor::optional("wall_reach", ParameterType::Quantity),
            ParameterDescriptor::optional("wall_inset", ParameterType::Quantity),
            ParameterDescriptor::optional("support_selector", ParameterType::Selector),
            ParameterDescriptor::optional("support_tolerance", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("component-clearance: {message}"),
                );
            }
        };
        let (Some(frames), Some(extents), Some(free_space)) = (
            context.services.get::<ObjectFrameServiceHandle>(),
            context.services.get::<VerticalExtentServiceHandle>(),
            context.services.get::<FreeSpaceServiceHandle>(),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "component-clearance needs the object-frame, vertical-extent and free-space \
                 services",
            );
        };
        let spans = context.services.get::<PlanSpanServiceHandle>();
        if config.walls.is_some() && spans.is_none() {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "component-clearance with `front_axis` `against-wall` needs the plan-span service",
            );
        }
        let walls = config
            .walls
            .map(|(selector, _, _)| Walls::select(context, selector));
        let supports = config
            .support
            .map(|(selector, _)| Split::select(context, selector, "support"));
        let services = Services {
            frames,
            extents,
            free_space,
            spans,
            walls: walls.as_ref(),
            supports: supports.as_ref(),
        };
        let obstacles = Obstacles::select(context, &config);
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let results = check(context, &config, &services, &obstacles, object);
            let results = match config.quantifier {
                Quantifier::All => results
                    .into_iter()
                    .map(|(side, result)| (format!("{} clearance", side.name()), result))
                    .collect(),
                Quantifier::Any => vec![any_side(&config, &results)],
            };
            for (label, result) in results {
                match result {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!("{label} {message}"),
                        evidence,
                        related,
                    )),
                    Err((reason, message)) => evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        format!("{label}: {message}"),
                    ),
                }
            }
        }
        evaluation
    }
}

/// One result for several sides of which one free is enough: free when a
/// side passes every question, a finding when every side surely fails one,
/// not evaluated otherwise.
fn any_side(config: &Config<'_>, results: &[(Side, Judged)]) -> (String, Judged) {
    let names: Vec<&str> = config.sides.iter().map(|side| side.name()).collect();
    let label = format!("{} clearance", names.join(" or "));
    let mut failures = Vec::new();
    let mut open = Vec::new();
    for side in &config.sides {
        let mine: Vec<&Judged> = results
            .iter()
            .filter(|(other, _)| other == side)
            .map(|(_, result)| result)
            .collect();
        let failed: Vec<_> = mine
            .iter()
            .filter_map(|result| match result {
                Ok(Some(found)) => Some(found.clone()),
                _ => None,
            })
            .collect();
        if !failed.is_empty() {
            failures.push((*side, failed));
        } else if let Some(Err(error)) = mine.iter().find(|result| result.is_err()) {
            open.push((*side, error.clone()));
        } else {
            return (label, Ok(None));
        }
    }
    if open.is_empty() {
        let mut messages = Vec::new();
        let mut evidence = Vec::new();
        let mut related: Vec<ObjectId> = Vec::new();
        for (side, found) in failures {
            for (message, cited, relates) in found {
                messages.push(format!("{} {message}", side.name()));
                evidence.extend(cited);
                for object in relates {
                    if !related.contains(&object) {
                        related.push(object);
                    }
                }
            }
        }
        return (
            label,
            Ok(Some((
                format!("has no free side: {}", messages.join("; ")),
                evidence,
                related,
            ))),
        );
    }
    let reason = open[0].1.0.clone();
    let reasons: Vec<String> = open
        .into_iter()
        .map(|(side, (_, message))| format!("{}: {message}", side.name()))
        .collect();
    (
        label,
        Err((
            reason,
            format!(
                "no side is surely free and not every side surely fails ({})",
                reasons.join("; ")
            ),
        )),
    )
}

struct Services<'a> {
    frames: &'a ObjectFrameServiceHandle,
    extents: &'a VerticalExtentServiceHandle,
    free_space: &'a FreeSpaceServiceHandle,
    spans: Option<&'a PlanSpanServiceHandle>,
    walls: Option<&'a Walls>,
    supports: Option<&'a Split>,
}

/// A selection split by how sure it is.
struct Split {
    /// Surely selected.
    sure: BTreeSet<ObjectId>,
    /// Undecided.
    maybe: BTreeSet<ObjectId>,
    /// A selection that could not be decided for the rule as a whole.
    failed: Option<Unavailable>,
}

impl Split {
    fn select(context: &RuleContext<'_>, selector: &Selector, what: &str) -> Self {
        let (objects, outcomes) = select_objects(context, selector);
        let mut maybe = BTreeSet::new();
        let mut failed = None;
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => {
                    maybe.insert(object.clone());
                }
                None => {
                    failed.get_or_insert_with(|| {
                        (
                            outcome.reason().clone(),
                            format!("{what} selection is undecided: {}", outcome.message()),
                        )
                    });
                }
            }
        }
        Self {
            sure: objects.iter().map(|object| object.id.clone()).collect(),
            maybe,
            failed,
        }
    }
}

/// The obstacle selection, split by how sure it is.
struct Obstacles {
    /// Selected obstacles surely not allowed to intrude.
    sure: BTreeSet<ObjectId>,
    /// Objects that may be obstacles: undecided by either selection.
    maybe: BTreeSet<ObjectId>,
    /// A selection that could not be decided for the rule as a whole.
    failed: Option<Unavailable>,
}

impl Obstacles {
    fn select(context: &RuleContext<'_>, config: &Config<'_>) -> Self {
        let selected = Split::select(context, config.obstacles, "obstacle");
        let allowed = config
            .allowed
            .map(|selector| Split::select(context, selector, "allowed-intruder"));
        let empty = BTreeSet::new();
        let (allowed_sure, allowed_maybe) = allowed
            .as_ref()
            .map_or((&empty, &empty), |allowed| (&allowed.sure, &allowed.maybe));
        let sure = selected
            .sure
            .iter()
            .filter(|id| !allowed_sure.contains(*id) && !allowed_maybe.contains(*id))
            .cloned()
            .collect();
        let maybe = selected
            .maybe
            .iter()
            .chain(selected.sure.intersection(allowed_maybe))
            .filter(|id| !allowed_sure.contains(*id))
            .cloned()
            .collect();
        let failed = selected
            .failed
            .or_else(|| allowed.and_then(|allowed| allowed.failed));
        Self {
            sure,
            maybe,
            failed,
        }
    }

    /// Sure and possible obstacles, less `excluded`.
    fn without(&self, excluded: &[&ObjectId]) -> (Vec<ObjectId>, Vec<ObjectId>) {
        let keep = |set: &BTreeSet<ObjectId>| -> Vec<ObjectId> {
            set.iter()
                .filter(|id| !excluded.contains(id))
                .cloned()
                .collect()
        };
        (keep(&self.sure), keep(&self.maybe))
    }
}

type Judged = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

/// Whether a volume is free, three-valued; errors are the third value.
enum Freedom {
    /// Free wherever the volume is: the evidence.
    Free(Vec<Evidence>),
    /// Obstructed wherever it is: what it says, the evidence and the
    /// objects it relates.
    Blocked(String, Vec<Evidence>, Vec<ObjectId>),
}

/// One result per checked side and question, in order.
fn check(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    services: &Services<'_>,
    obstacles: &Obstacles,
    object: &Object,
) -> Vec<(Side, Judged)> {
    let placed = match Placement::of(context, config, services, object) {
        Ok(placed) => placed,
        Err(error) => {
            return config
                .sides
                .iter()
                .map(|side| (*side, Err(error.clone())))
                .collect();
        }
    };
    let mut results = Vec::new();
    for side in &config.sides {
        let faces = placed
            .faces(config, services, object, *side)
            .and_then(|faces| Ok((faces.size(config, &placed)?, faces)));
        let (declared, faces) = match faces {
            Ok(found) => found,
            Err(error) => {
                results.push((*side, Err(error)));
                continue;
            }
        };
        let free = |size: Size| -> Result<Freedom, Unavailable> {
            let volume = faces.volume(config, size);
            if config.floats() {
                search(config, services, obstacles, &placed, object, &volume)
            } else {
                clearance(config, services, obstacles, object, &volume)
            }
        };
        if config.mode != SizeMode::Maximum {
            let size = declared.changed(-config.tolerance);
            let judged = free(size).map(|freedom| match freedom {
                Freedom::Free(_) => None,
                Freedom::Blocked(message, evidence, related) => Some((
                    format!("({}) {message}", size.describe(config)),
                    evidence,
                    related,
                )),
            });
            results.push((*side, judged));
        }
        if config.mode != SizeMode::Minimum {
            for (dimension, size) in declared.larger(config.tolerance) {
                let judged = free(size).map(|freedom| match freedom {
                    Freedom::Blocked(..) => None,
                    Freedom::Free(evidence) => Some((
                        format!(
                            "({}) is free, so the free volume exceeds the maximum {dimension}",
                            size.describe(config)
                        ),
                        evidence,
                        Vec::new(),
                    )),
                });
                results.push((*side, judged));
            }
        }
        // A floating volume is searched in the spaces, so it lies in them.
        if config.within_space && !config.floats() {
            let volume = faces.volume(config, declared);
            let described = declared.describe(config);
            results.push((
                *side,
                containment(services, &placed, object, &volume, &described),
            ));
        }
        if let (Some((_, tolerance)), Some(supports)) = (config.support, services.supports) {
            let volume = faces.volume(config, declared);
            let described = declared.describe(config);
            results.push((
                *side,
                support(services, supports, object, &volume, tolerance, &described),
            ));
        }
    }
    results
}

/// How far a derived front may be turned from the true one: the volume is
/// widened by the arc its points could sweep about the rectangle's centre.
struct Turn {
    /// The rectangle's centre in plan.
    centre: [f64; 2],
    /// The largest turn, in radians.
    angle: f64,
    /// How far the component's footprint reaches from the centre.
    reach: f64,
}

/// What every side of one component shares: its frame, its front, its
/// datums, its height and its spaces.
struct Placement {
    front: [f64; 3],
    up: [f64; 3],
    /// With a front derived from the footprint: how far it may be turned.
    turn: Option<Turn>,
    /// Elevation interval of the height reference.
    base: (f64, f64),
    /// Elevation interval of the top datum, when there is one.
    top: Option<(f64, f64)>,
    /// The component's height, when a height is sized from it.
    height: Option<(f64, f64)>,
    spaces: Vec<ObjectId>,
    evidence: Vec<Evidence>,
    /// For a door with one hinged leaf: from its hinge towards its handle.
    handle: Option<[f64; 3]>,
}

/// Axes of a vector within this of a unit vector's component count as it.
const AXIS_TOLERANCE: f64 = 1.0e-9;

impl Placement {
    fn of(
        context: &RuleContext<'_>,
        config: &Config<'_>,
        services: &Services<'_>,
        object: &Object,
    ) -> Result<Self, Unavailable> {
        let leaves =
            if config.front.is_swing() || matches!(config.align, Align::Handle | Align::Hinge) {
                Some(door_swing::leaves(services.frames, &object.id)?)
            } else {
                None
            };
        let swinging = leaves.as_ref().filter(|_| config.front.is_swing());
        let mut turn = None;
        let (front, up, mut evidence) =
            if let Some((front, derived, mut evidence)) = against_wall(config, services, object)? {
                turn = Some(derived);
                if let Some(leaves) = &leaves {
                    evidence.push(leaves.evidence().clone());
                }
                (front, [0.0, 0.0, 1.0], evidence)
            } else if let Some(leaves) = swinging {
                // The leaves open along a horizontal direction; the volume
                // stands upright whatever way the door's axes point.
                let opening = door_swing::swing_side(leaves)?;
                let front = if matches!(config.front, FrontAxis::Swing) {
                    opening
                } else {
                    negate(opening)
                };
                (front, [0.0, 0.0, 1.0], vec![leaves.evidence().clone()])
            } else {
                let frame = services
                    .frames
                    .object_frame(&object.id)
                    .map_err(|error| frame_error(&error))?;
                let up = frame.frame().up().components();
                if (up[2] - 1.0).abs() > AXIS_TOLERANCE {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        "the component's frame is tilted; clearances are measured in upright \
                         frames only"
                            .into(),
                    ));
                }
                let front = config.front.of(&frame)?;
                if dot(front, up).abs() > AXIS_TOLERANCE {
                    return Err((
                        NotEvaluatedReason::InvalidEvidence,
                        "the stated front is not horizontal".into(),
                    ));
                }
                let mut evidence = vec![frame.evidence().clone()];
                if let Some(leaves) = &leaves {
                    evidence.push(leaves.evidence().clone());
                }
                (front, up, evidence)
            };
        let handle = match (&leaves, config.align) {
            (Some(leaves), Align::Handle | Align::Hinge) => Some(door_swing::handle(leaves)?),
            _ => None,
        };
        let (spaces, cited) = match &config.spaces {
            Some(path) => {
                let everything: Vec<&Object> = context.project.objects().collect();
                path.related(context, &object.id, &everything)?
            }
            None => (Vec::new(), Vec::new()),
        };
        evidence.extend(cited);
        let datums = Datums {
            services,
            object,
            spaces: &spaces,
        };
        let base = datums.elevation(config.reference, &mut evidence)?;
        let top = match config.top {
            Some((reference, _)) => Some(datums.elevation(reference, &mut evidence)?),
            None => None,
        };
        let sized =
            |declared: Option<Sizing>| declared.is_some_and(|sizing| sizing.fixed().is_none());
        let height = match config.declared {
            Declared::Box { height, .. } | Declared::Cylinder { height, .. } if sized(height) => {
                let measured = extent(services.extents, &object.id)?;
                evidence.push(measured.evidence().clone());
                let (bottom, top) = (measured.bottom(), measured.top());
                Some((
                    (top.lower_metres() - bottom.upper_metres()).max(0.0),
                    top.upper_metres() - bottom.lower_metres(),
                ))
            }
            _ => None,
        };
        Ok(Self {
            front,
            up,
            turn,
            base,
            top,
            height,
            spaces,
            evidence,
            handle,
        })
    }

    /// The component's faces on `side`: the axes and the intervals its
    /// outermost points along and across the side are known to.
    fn faces(
        &self,
        config: &Config<'_>,
        services: &Services<'_>,
        object: &Object,
        side: Side,
    ) -> Result<Faces, Unavailable> {
        let outward = side.outward(self.front, self.up);
        // Looking out of the side with up overhead, the box's right.
        let across = cross(outward, self.up);
        let (outward, across, up) = (direction(outward)?, direction(across)?, direction(self.up)?);
        let along = services
            .extents
            .measure_directional_extent(&object.id, outward)
            .map_err(|error| extent_error(&error))?;
        let beside = services
            .extents
            .measure_directional_extent(&object.id, across)
            .map_err(|error| extent_error(&error))?;
        let mut evidence = self.evidence.clone();
        evidence.push(along.evidence().clone());
        evidence.push(beside.evidence().clone());
        let align = match (config.align, self.handle) {
            (Align::Handle | Align::Hinge, Some(handle)) => {
                // The handle lies where the closed leaf runs from its hinge.
                let towards = dot(handle, across.components());
                if towards.abs() < 0.5 {
                    return Err((
                        NotEvaluatedReason::InvalidDeclaration,
                        format!(
                            "the door's handle and hinge lie before and behind its {} side, \
                             not beside it",
                            side.name()
                        ),
                    ));
                }
                match (towards > 0.0, matches!(config.align, Align::Handle)) {
                    (true, true) | (false, false) => Align::Right,
                    _ => Align::Left,
                }
            }
            (align, _) => align,
        };
        let turn = self.turn.as_ref().map(|turn| {
            let (o, a) = (outward.components(), across.components());
            (
                turn.angle,
                turn.reach,
                turn.centre[0] * o[0] + turn.centre[1] * o[1],
                turn.centre[0] * a[0] + turn.centre[1] * a[1],
            )
        });
        let interval = |elevation: axioval_engine::ElevationInterval| {
            (elevation.lower_metres(), elevation.upper_metres())
        };
        Ok(Faces {
            align,
            turn,
            outward,
            across,
            up,
            back: interval(along.lower()),
            face: interval(along.upper()),
            low: interval(beside.lower()),
            high: interval(beside.upper()),
            base: self.base,
            top: self.top,
            evidence,
        })
    }
}

/// Reads datum elevations for one component.
struct Datums<'a> {
    services: &'a Services<'a>,
    object: &'a Object,
    spaces: &'a [ObjectId],
}

impl Datums<'_> {
    /// The elevation interval of `reference`, citing what it read.
    fn elevation(
        &self,
        reference: Reference,
        evidence: &mut Vec<Evidence>,
    ) -> Result<(f64, f64), Unavailable> {
        match reference {
            Reference::Bottom | Reference::Top => {
                let measured = extent(self.services.extents, &self.object.id)?;
                evidence.push(measured.evidence().clone());
                let elevation = if matches!(reference, Reference::Bottom) {
                    measured.bottom()
                } else {
                    measured.top()
                };
                Ok((elevation.lower_metres(), elevation.upper_metres()))
            }
            Reference::Floor => {
                if self.spaces.is_empty() {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        "`space_path` reaches no space, so there is no floor to measure from"
                            .into(),
                    ));
                }
                // The floor is one of the spaces' bottoms; the hull holds it
                // whichever it is.
                let mut hull = (f64::INFINITY, f64::NEG_INFINITY);
                for space in self.spaces {
                    let measured = extent(self.services.extents, space)?;
                    evidence.push(measured.evidence().clone());
                    hull.0 = hull.0.min(measured.bottom().lower_metres());
                    hull.1 = hull.1.max(measured.bottom().upper_metres());
                }
                Ok(hull)
            }
        }
    }
}

/// A front derived from the footprint, how far it may be turned and the
/// evidence deriving it.
type Derived = ([f64; 3], Turn, Vec<Evidence>);

/// With `against-wall`: the front facing away from the wall the component
/// stands against, how far it may be turned and the evidence.
fn against_wall(
    config: &Config<'_>,
    services: &Services<'_>,
    object: &Object,
) -> Result<Option<Derived>, Unavailable> {
    let (Some((_, reach, inset)), Some(walls), Some(spans)) =
        (config.walls, services.walls, services.spans)
    else {
        return Ok(None);
    };
    let measured = walls.measure(spans, &object.id, reach, inset)?;
    let back = walls.back(&measured)?;
    let rectangle = measured.rectangle();
    let front = back.side.opposite().outward(rectangle);
    let halves = rectangle.half_extents_metres();
    let turn = Turn {
        centre: rectangle.centre(),
        angle: rectangle.axis_error_radians(),
        reach: halves[0].1.hypot(halves[1].1) + rectangle.centre_radius_metres(),
    };
    Ok(Some(([front[0], front[1], 0.0], turn, back.evidence)))
}

fn direction(vector: [f64; 3]) -> Result<MetricDirection, Unavailable> {
    MetricDirection::try_new(vector).map_err(|error| {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!("clearance axis: {error}"),
        )
    })
}

/// A component's measured faces on one side.
struct Faces {
    outward: MetricDirection,
    across: MetricDirection,
    up: MetricDirection,
    /// The component's outermost point on the opposite side, along the
    /// outward direction.
    back: (f64, f64),
    /// Its outermost point on this side.
    face: (f64, f64),
    /// Its left and right edges across the side.
    low: (f64, f64),
    high: (f64, f64),
    /// Elevation interval of the height reference.
    base: (f64, f64),
    /// Elevation interval of the top datum.
    top: Option<(f64, f64)>,
    evidence: Vec<Evidence>,
    /// The alignment across the side, a door's handle or hinge resolved.
    align: Align,
    /// With a derived front: the largest turn, the footprint's reach from
    /// the rectangle's centre and the centre along and across the side.
    turn: Option<(f64, f64, f64, f64)>,
}

/// The difference `a - b` of two intervals, never below zero.
fn difference(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 - b.1).max(0.0), (a.1 - b.0).max(0.0))
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (f64::midpoint(a.0, b.0), f64::midpoint(a.1, b.1))
}

impl Faces {
    /// The declared size on this side, sized from the component where the
    /// rule says so.
    fn size(&self, config: &Config<'_>, placed: &Placement) -> Result<Size, Unavailable> {
        let height = |declared: Option<Sizing>| -> Result<Height, Unavailable> {
            match declared {
                None => Ok(Height::ToTop(0.0)),
                Some(sizing) => sizing
                    .resolve(|| {
                        placed.height.ok_or_else(|| {
                            (
                                NotEvaluatedReason::InvalidEvidence,
                                "the component's height was not measured".into(),
                            )
                        })
                    })
                    .map(Height::Of),
            }
        };
        match config.declared {
            Declared::Box {
                width,
                depth,
                height: tall,
            } => Ok(Size {
                shape: Shape::Box {
                    width: width.resolve(|| Ok(difference(self.high, self.low)))?,
                    depth: depth.resolve(|| Ok(difference(self.face, self.back)))?,
                },
                height: height(tall)?,
            }),
            Declared::Cylinder {
                radius,
                height: tall,
            } => Ok(Size {
                shape: Shape::Cylinder { radius },
                height: height(tall)?,
            }),
        }
    }

    /// The volume of `size` on this side, with the intervals its edges are
    /// known to.
    fn volume(&self, config: &Config<'_>, size: Size) -> Volume {
        let (low, high) = (self.low, self.high);
        let start = match config.depth_from {
            DepthFrom::Face => self.face,
            DepthFrom::Midline => midpoint(self.back, self.face),
        };
        let start = add(start, config.offset);
        let lateral = config.lateral_offset;
        let plan = match size.shape {
            Shape::Box { width, depth } => {
                let out = [start, (start.0 + depth.0, start.1 + depth.1)];
                let across = match self.align {
                    Align::Centre | Align::Handle | Align::Hinge => {
                        let centre = add(midpoint(low, high), lateral);
                        [
                            (centre.0 - width.1 / 2.0, centre.1 - width.0 / 2.0),
                            (centre.0 + width.0 / 2.0, centre.1 + width.1 / 2.0),
                        ]
                    }
                    Align::Left => {
                        let edge = add(low, lateral);
                        [edge, (edge.0 + width.0, edge.1 + width.1)]
                    }
                    Align::Right => {
                        let edge = add(high, lateral);
                        [(edge.0 - width.1, edge.1 - width.0), edge]
                    }
                };
                Plan::Box { out, across }
            }
            Shape::Cylinder { radius } => {
                let across = match self.align {
                    Align::Centre | Align::Handle | Align::Hinge => midpoint(low, high),
                    Align::Left => add(low, radius),
                    Align::Right => add(high, -radius),
                };
                Plan::Disc {
                    out: add(start, radius),
                    across: add(across, lateral),
                    radius,
                }
            }
        };
        let base = add(self.base, config.vertical_offset);
        let top = match size.height {
            Height::Of(height) => (base.0 + height.0, base.1 + height.1),
            Height::ToTop(raised) => {
                let offset = config.top.map_or(0.0, |(_, offset)| offset) + raised;
                add(self.top.unwrap_or(self.base), offset)
            }
        };
        let mut volume = Volume {
            outward: self.outward,
            across: self.across,
            up: self.up,
            plan,
            base,
            top,
            evidence: self.evidence.clone(),
        };
        // Turned about the rectangle's centre by at most `angle`, every
        // point moves by at most its distance from the centre times the
        // angle; the component's faces by the footprint's reach, the volume
        // by its farthest corner's.
        if let Some((angle, footprint, out, side)) = self.turn {
            let corner = volume.farthest(out, side);
            volume.widen(angle * (footprint + corner));
        }
        volume
    }
}

/// A volume's plan: a box by the intervals its edges are known to, or a
/// disc by its centre's intervals and its radius.
#[derive(Clone, Copy)]
enum Plan {
    Box {
        /// Its near and far edges along the outward direction.
        out: [(f64, f64); 2],
        /// Its left and right edges across the side.
        across: [(f64, f64); 2],
    },
    Disc {
        out: (f64, f64),
        across: (f64, f64),
        radius: f64,
    },
}

/// A volume's axes, plan and the intervals its base and top are known to.
struct Volume {
    outward: MetricDirection,
    across: MetricDirection,
    up: MetricDirection,
    plan: Plan,
    base: (f64, f64),
    top: (f64, f64),
    evidence: Vec<Evidence>,
}

/// Which of the volume's possible positions a request stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    /// Every position together: free here is free anywhere.
    Union,
    /// What every position shares: obstructed here is obstructed anywhere.
    Common,
}

fn spread((low, high): (f64, f64)) -> f64 {
    high - low
}

/// A plan shape at a centre: `(centre out, centre across, shape)`.
#[derive(Clone, Copy, PartialEq)]
enum Outline {
    Box { width: f64, depth: f64 },
    Disc { radius: f64 },
}

/// The extent of one bound between a lower and an upper edge interval,
/// shrunk by `inset` at both ends: its middle and length.
fn edges(lower: (f64, f64), upper: (f64, f64), inset: f64, bound: Bound) -> (f64, f64) {
    let (low, high) = match bound {
        Bound::Union => (lower.0, upper.1),
        Bound::Common => (lower.1, upper.0),
    };
    let (low, high) = (low + inset, high - inset);
    (f64::midpoint(low, high), high - low)
}

impl Volume {
    fn exact(&self) -> bool {
        #[allow(clippy::float_cmp)]
        let point = |(low, high): (f64, f64)| low == high;
        let plan = match self.plan {
            Plan::Box { out, across } => out.iter().chain(&across).all(|edge| point(*edge)),
            Plan::Disc { out, across, .. } => point(out) && point(across),
        };
        plan && point(self.base) && point(self.top)
    }

    /// How far the volume's farthest point can lie from a point at `out`
    /// and `side` along and across the side.
    fn farthest(&self, out: f64, side: f64) -> f64 {
        let far = |edges: &[(f64, f64)], centre: f64| {
            edges
                .iter()
                .flat_map(|(low, high)| [(low - centre).abs(), (high - centre).abs()])
                .fold(0.0, f64::max)
        };
        match self.plan {
            Plan::Box { out: o, across: a } => far(&o, out).hypot(far(&a, side)),
            Plan::Disc {
                out: o,
                across: a,
                radius,
            } => far(&[o], out).hypot(far(&[a], side)) + radius,
        }
    }

    /// Widens every plan edge by `margin` both ways.
    fn widen(&mut self, margin: f64) {
        let widen = |(low, high): (f64, f64)| (low - margin, high + margin);
        self.plan = match self.plan {
            Plan::Box { out, across } => Plan::Box {
                out: out.map(widen),
                across: across.map(widen),
            },
            Plan::Disc {
                out,
                across,
                radius,
            } => Plan::Disc {
                out: widen(out),
                across: widen(across),
                radius,
            },
        };
    }

    /// The plan of one bound, shrunk by `inset` on every plan side: its
    /// centre along and across the side and its outline, `None` when the
    /// common part is empty.
    fn outline(&self, inset: f64, bound: Bound) -> Option<(f64, f64, Outline)> {
        match self.plan {
            Plan::Box { out, across } => {
                let (centre_out, depth) = edges(out[0], out[1], inset, bound);
                let (centre_across, width) = edges(across[0], across[1], inset, bound);
                (width > 0.0 && depth > 0.0).then_some((
                    centre_out,
                    centre_across,
                    Outline::Box { width, depth },
                ))
            }
            Plan::Disc {
                out,
                across,
                radius,
            } => {
                // A disc whose centre lies anywhere in a rectangle stays in
                // the disc grown by the rectangle's half-diagonal and holds
                // the disc shrunk by it.
                let sign = match bound {
                    Bound::Union => 1.0,
                    Bound::Common => -1.0,
                };
                let radius = radius - inset + sign * spread(out).hypot(spread(across)) / 2.0;
                (radius > 0.0).then_some((
                    f64::midpoint(out.0, out.1),
                    f64::midpoint(across.0, across.1),
                    Outline::Disc { radius },
                ))
            }
        }
    }

    /// The elevations one bound spans.
    fn vertical(&self, bound: Bound) -> (f64, f64) {
        match bound {
            Bound::Union => (self.base.0, self.top.1),
            Bound::Common => (self.base.1, self.top.0),
        }
    }

    /// The point `out` along and `side` across the side, at `elevation`.
    fn point(
        &self,
        object: &ObjectId,
        out: f64,
        side: f64,
        elevation: f64,
    ) -> Result<MetricPoint, Unavailable> {
        let (o, a, u) = (
            self.outward.components(),
            self.across.components(),
            self.up.components(),
        );
        let origin = [0, 1, 2].map(|i| out * o[i] + side * a[i] + elevation * u[i]);
        MetricPoint::try_new(object.clone(), origin).map_err(|error| {
            (
                NotEvaluatedReason::InvalidEvidence,
                format!("clearance origin: {error}"),
            )
        })
    }

    /// The frame and shape of one bound, shrunk by `inset` on every plan
    /// side; `None` when the common part is empty.
    fn bound(
        &self,
        object: &ObjectId,
        inset: f64,
        bound: Bound,
    ) -> Result<Option<(MetricFrame, ClearanceShape)>, Unavailable> {
        let (bottom, top) = self.vertical(bound);
        let Some((out, side, outline)) = self.outline(inset, bound) else {
            return Ok(None);
        };
        if top <= bottom {
            return Ok(None);
        }
        let shape = clearance_shape(outline, top - bottom)?;
        let frame = MetricFrame::try_new(
            self.point(object, out, side, bottom)?,
            self.across,
            self.outward,
            self.up,
        )
        .map_err(|error| free_space_error(&error))?;
        Ok(Some((frame, shape)))
    }
}

fn clearance_shape(outline: Outline, height: f64) -> Result<ClearanceShape, Unavailable> {
    match outline {
        Outline::Box { width, depth } => {
            BoxClearance::try_new(width, depth, height).map(ClearanceShape::Box)
        }
        Outline::Disc { radius } => {
            CylinderClearance::try_new(radius, height).map(ClearanceShape::Cylinder)
        }
    }
    .map_err(|error| free_space_error(&error))
}

pub(crate) fn free_space_error(error: &FreeSpaceError) -> Unavailable {
    let reason = match error {
        FreeSpaceError::MissingGeometry(_) | FreeSpaceError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        FreeSpaceError::IncompleteClearanceEvidence => NotEvaluatedReason::IncompleteEvidence,
        _ => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("free space: {error}"))
}

fn names(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether the fixed volume is free of obstacles, three-valued.
fn clearance(
    config: &Config<'_>,
    services: &Services<'_>,
    obstacles: &Obstacles,
    object: &Object,
    volume: &Volume,
) -> Result<Freedom, Unavailable> {
    if let Some(failed) = &obstacles.failed {
        return Err(failed.clone());
    }
    let (sure, maybe) = obstacles.without(&[&object.id]);
    let mut candidates = sure.clone();
    candidates.extend(maybe);
    let ask = |bound: Bound, candidates: &[ObjectId]| {
        volume
            .bound(&object.id, config.protrusion, bound)?
            .map(|(frame, shape)| {
                services
                    .free_space
                    .assess_clearance(&ClearanceRequest::new(frame, shape, candidates.to_vec()))
                    .map_err(|error| free_space_error(&error))
            })
            .transpose()
    };
    let cited = |proof: &Evidence| {
        let mut evidence = volume.evidence.clone();
        evidence.push(proof.clone());
        evidence
    };
    let union = ask(Bound::Union, &candidates);
    if let Ok(Some(ClearanceOutcome::Clear(proof))) = &union {
        return Ok(Freedom::Free(cited(proof.evidence())));
    }
    let common = if volume.exact() {
        union.clone()
    } else {
        ask(Bound::Common, &candidates)
    };
    let found = |blockers: &[ObjectId], proof: &Evidence| {
        Ok(Freedom::Blocked(
            format!("is obstructed by {}", names(blockers)),
            cited(proof),
            blockers.to_vec(),
        ))
    };
    if let Ok(Some(ClearanceOutcome::Obstructed(proof))) = &common {
        let decided: Vec<ObjectId> = proof
            .blockers()
            .iter()
            .filter(|id| sure.binary_search(id).is_ok())
            .cloned()
            .collect();
        if !decided.is_empty() {
            return found(&decided, proof.evidence());
        }
        // Only objects the selection could not decide obstruct it: ask
        // again without them.
        return match ask(Bound::Common, &sure) {
            Ok(Some(ClearanceOutcome::Obstructed(proof))) => {
                found(proof.blockers(), proof.evidence())
            }
            Ok(_) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "obstructed only by objects the obstacle selection cannot decide: {}",
                    names(proof.blockers())
                ),
            )),
            Err(error) => Err(error),
        };
    }
    union?;
    common?;
    Err((
        NotEvaluatedReason::IncompleteEvidence,
        "an obstacle meets some positions the measured component leaves open for the volume \
         but not all"
            .into(),
    ))
}

/// Keeps a frame-offset anchor's floor within this of the floor measured.
const FLOOR_MARGIN: f64 = 1.0e-6;

/// How far a witness of a floating volume may stand off its line: a
/// domain without area is searched only at points moved onto it, which
/// rarely verify, so along an axis the volume does not slide on the free
/// volume is searched `SLACK` larger on each side within `SLACK` of the
/// line, and holds the volume on the line.
const SLACK: f64 = 1.0e-3;

/// The offsets a floating volume may take along one of its plan axes, and
/// how much larger a witness must be along it: the slide, or for a bound
/// that is not searched along it, the line itself (with the slack for a
/// witness).
fn offsets(slide: Option<(f64, f64)>, bound: Bound) -> ((f64, f64), f64) {
    match (slide, bound) {
        (Some(slide), _) => (slide, 0.0),
        (None, Bound::Union) => ((-SLACK, SLACK), SLACK),
        (None, Bound::Common) => ((0.0, 0.0), 0.0),
    }
}

/// Where a floating volume may be moved, as a message says it.
fn slid(config: &Config<'_>) -> String {
    let range = |(from, to): (f64, f64)| format!("between {} and {}", metres(from), metres(to));
    match (config.slide, config.depth_slide) {
        (Some(across), None) => format!("{} across the side", range(across)),
        (None, Some(away)) => format!("{} away from the component", range(away)),
        (Some(across), Some(away)) => format!(
            "{} across the side and {} away from the component",
            range(across),
            range(away)
        ),
        (None, None) => "anywhere".into(),
    }
}

/// Whether the floating volume fits at some offset, three-valued, by a
/// placement search in the component's spaces.
///
/// The volume's position is an interval. A fit of the volume grown by it
/// (the union) at an offset holds the volume at that offset wherever it
/// is; the volume shrunk by it (the common part) fits at every offset the
/// volume fits at, so its absence proves the volume's.
#[allow(clippy::too_many_lines)]
fn search(
    config: &Config<'_>,
    services: &Services<'_>,
    obstacles: &Obstacles,
    placed: &Placement,
    object: &Object,
    volume: &Volume,
) -> Result<Freedom, Unavailable> {
    if let Some(failed) = &obstacles.failed {
        return Err(failed.clone());
    }
    let Some((scope, merged)) = placed.spaces.split_first() else {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            "`space_path` reaches no space to search for the floating volume".into(),
        ));
    };
    let floor = extent(services.extents, scope)?;
    let mut evidence = volume.evidence.clone();
    evidence.push(floor.evidence().clone());
    let floor = (floor.bottom().lower_metres(), floor.bottom().upper_metres());
    let mut excluded: Vec<&ObjectId> = placed.spaces.iter().collect();
    excluded.push(&object.id);
    let (sure, maybe) = obstacles.without(&excluded);
    // The anchor stands in the middle of the bound's plan, on the floor,
    // with the plan axes of the side; only the offsets slide.
    let outward = volume.outward.components();
    let outward = direction([outward[0], outward[1], 0.0])?;
    let up = direction([0.0, 0.0, 1.0])?;
    let across = direction(cross(outward.components(), up.components()))?;
    let floor_middle = f64::midpoint(floor.0, floor.1);
    let reach = spread(floor) / 2.0 + FLOOR_MARGIN;
    let request = |bound: Bound,
                   candidates: Vec<ObjectId>|
     -> Result<Option<PlacementRequest>, Unavailable> {
        // The base and top above the scope's floor, as intervals.
        let (bottom, top) = volume.vertical(bound);
        let (band_from, band_to) = match bound {
            Bound::Union => (bottom - floor.1, top - floor.0),
            Bound::Common => (bottom - floor.0, top - floor.1),
        };
        if band_to <= band_from {
            return Ok(None);
        }
        let band = ElevationBand::try_new(band_from, band_to).map_err(|_| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the volume's base lies {} above the floor of {scope}; a floating volume \
                     must stand at or above it",
                    shown(volume.base.0 - floor.1, volume.base.1 - floor.0)
                ),
            )
        })?;
        let Some((out, side, outline)) = volume.outline(config.protrusion, bound) else {
            return Ok(None);
        };
        // A witness may stand off the line by the slack, which the larger
        // volume makes up for; an absence is proven on the line itself.
        let (across_range, across_slack) = offsets(config.slide, bound);
        let (out_range, out_slack) = offsets(config.depth_slide, bound);
        let outline = match outline {
            Outline::Box { width, depth } => Outline::Box {
                width: width + 2.0 * across_slack,
                depth: depth + 2.0 * out_slack,
            },
            Outline::Disc { radius } => Outline::Disc {
                radius: radius + across_slack.hypot(out_slack),
            },
        };
        let anchor = MetricFrame::try_new(
            volume.point(&object.id, out, side, floor_middle)?,
            across,
            outward,
            up,
        )
        .map_err(|error| free_space_error(&error))?;
        let domain = (|| {
            Ok::<_, FreeSpaceError>(FrameOffsetPlacement::new(
                anchor.clone(),
                SignedDistanceInterval::try_new(across_range.0, across_range.1)?,
                SignedDistanceInterval::try_new(out_range.0, out_range.1)?,
                SignedDistanceInterval::try_new(-reach, reach)?,
            ))
        })()
        .map_err(|error| free_space_error(&error))?;
        let shape = match clearance_shape(outline, band_to - band_from)? {
            ClearanceShape::Box(shape) => PlacementShape::Box {
                shape,
                orientation: PlacementOrientation::Fixed(anchor.clone()),
            },
            ClearanceShape::Cylinder(shape) => PlacementShape::Cylinder(shape),
        };
        PlacementRequest::new_in_domain(
            scope.clone(),
            shape,
            candidates,
            PlacementDomain::FrameOffsets(domain),
        )
        .and_then(|request| request.with_merged_scopes(merged.to_vec()))
        .map(|request| Some(request.with_band(band)))
        .map_err(|error| free_space_error(&error))
    };
    let find = |request: &Option<PlacementRequest>| {
        request
            .as_ref()
            .map(|request| {
                services
                    .free_space
                    .find_placement(request)
                    .map_err(|error| free_space_error(&error))
            })
            .transpose()
    };
    let ask = |bound: Bound, candidates: Vec<ObjectId>| find(&request(bound, candidates)?);
    let cited = |proof: &Evidence| {
        let mut evidence = evidence.clone();
        evidence.push(proof.clone());
        evidence
    };
    let mut candidates = sure.clone();
    candidates.extend(maybe.iter().cloned());
    let asked = request(Bound::Union, candidates.clone())?;
    let union = find(&asked);
    if let Ok(Some(PlacementOutcome::Found(found))) = &union {
        return Ok(Freedom::Free(cited(found.evidence())));
    }
    // A volume known exactly on an exact floor asks one question.
    let common_request = request(Bound::Common, candidates)?;
    let common = if common_request == asked {
        union.clone()
    } else {
        find(&common_request)
    };
    let blocked = |proof: &Evidence| {
        Ok(Freedom::Blocked(
            format!("fits nowhere {} in {}", slid(config), names(&placed.spaces)),
            cited(proof),
            placed.spaces.clone(),
        ))
    };
    if let Ok(Some(PlacementOutcome::NoPlacement(proof))) = &common {
        if maybe.is_empty() {
            return blocked(proof.evidence());
        }
        // Undecided obstacles may be what leaves no room: ask again
        // without them.
        return match ask(Bound::Common, sure) {
            Ok(Some(PlacementOutcome::NoPlacement(proof))) => blocked(proof.evidence()),
            Ok(_) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the volume fits only if objects the obstacle selection cannot decide are \
                     not obstacles: {}",
                    names(&maybe)
                ),
            )),
            Err(error) => Err(error),
        };
    }
    union?;
    common?;
    Err((
        NotEvaluatedReason::IncompleteEvidence,
        "the volume fits at some positions the measured component leaves open but not at all"
            .into(),
    ))
}

/// Whether the declared volume's plan lies inside the reached spaces.
fn containment(
    services: &Services<'_>,
    placed: &Placement,
    object: &Object,
    volume: &Volume,
    described: &str,
) -> Judged {
    let spaces: Vec<String> = placed.spaces.iter().map(ToString::to_string).collect();
    if placed.spaces.is_empty() {
        return Ok(Some((
            format!("({described}) has no space to lie in: `space_path` reaches none"),
            volume.evidence.clone(),
            Vec::new(),
        )));
    }
    let ask = |bound: Bound| {
        volume
            .bound(&object.id, 0.0, bound)?
            .map(|(frame, shape)| {
                services
                    .free_space
                    .assess_containment(&ContainmentRequest::new(
                        frame,
                        shape,
                        placed.spaces.clone(),
                    ))
                    .map_err(|error| free_space_error(&error))
            })
            .transpose()
    };
    let union = ask(Bound::Union);
    if let Ok(Some(ContainmentOutcome::Inside(_))) = &union {
        return Ok(None);
    }
    let common = if volume.exact() {
        union.clone()
    } else {
        ask(Bound::Common)
    };
    if let Ok(Some(ContainmentOutcome::Outside(proof))) = &common {
        let mut evidence = volume.evidence.clone();
        evidence.push(proof.evidence().clone());
        return Ok(Some((
            format!("({described}) extends outside {}", spaces.join(", ")),
            evidence,
            placed.spaces.clone(),
        )));
    }
    union?;
    common?;
    Err((
        NotEvaluatedReason::IncompleteEvidence,
        format!(
            "whether the volume lies inside {} depends on where in its measured interval the \
             component is",
            spaces.join(", ")
        ),
    ))
}

/// Whether the tops of the supports hold the declared volume's plan within
/// `tolerance` of its base.
///
/// Supported needs the union of the volume's positions covered by the tops
/// of sure supports within `tolerance` of every elevation the base may
/// have; unsupported needs part of their common part outside the tops of
/// every possible support within `tolerance` of any of them.
fn support(
    services: &Services<'_>,
    supports: &Split,
    object: &Object,
    volume: &Volume,
    tolerance: f64,
    described: &str,
) -> Judged {
    if let Some(failed) = &supports.failed {
        return Err(failed.clone());
    }
    let without = |set: &BTreeSet<ObjectId>| -> Vec<ObjectId> {
        set.iter().filter(|id| **id != object.id).cloned().collect()
    };
    let sure = without(&supports.sure);
    let mut possible = sure.clone();
    possible.extend(without(&supports.maybe));
    let base = volume.base;
    let ask = |bound: Bound, candidates: &[ObjectId], (from, to): (f64, f64)| {
        if to < from {
            return Ok(None);
        }
        volume
            .bound(&object.id, 0.0, bound)?
            .map(|(frame, shape)| {
                SupportCoverageRequest::try_new(frame, shape, candidates.to_vec(), from, to)
                    .and_then(|request| services.free_space.assess_support_coverage(&request))
                    .map_err(|error| free_space_error(&error))
            })
            .transpose()
    };
    let supported = ask(
        Bound::Union,
        &sure,
        (base.1 - tolerance, base.0 + tolerance),
    );
    if let Ok(Some(SupportCoverageOutcome::Supported(_))) = &supported {
        return Ok(None);
    }
    let unsupported = ask(
        Bound::Common,
        &possible,
        (base.0 - tolerance, base.1 + tolerance),
    );
    if let Ok(Some(SupportCoverageOutcome::Unsupported(proof))) = &unsupported {
        let mut evidence = volume.evidence.clone();
        evidence.push(proof.evidence().clone());
        return Ok(Some((
            format!(
                "({described}) is not wholly supported: part of it lies over no top of the \
                 supports within {} of its base",
                metres(tolerance)
            ),
            evidence,
            Vec::new(),
        )));
    }
    supported?;
    unsupported?;
    Err((
        NotEvaluatedReason::IncompleteEvidence,
        if supports.maybe.is_empty() {
            "whether the supports hold the volume depends on where in its measured interval \
             the component is"
                .into()
        } else {
            format!(
                "the volume may be supported only by objects the support selection cannot \
                 decide: {}",
                names(&supports.maybe.iter().cloned().collect::<Vec<_>>())
            )
        },
    ))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn negate(a: [f64; 3]) -> [f64; 3] {
    a.map(|value| -value)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
