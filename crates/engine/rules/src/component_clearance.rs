//! `component-clearance`: a free box or cylinder in front of, behind or
//! beside each selected component, placed in the component's own frame on
//! a side the rule states, fixed or sliding sideways.

use std::collections::BTreeSet;

use axioval_engine::{
    BoxClearance, CapabilityEvaluation, ClearanceOutcome, ClearanceRequest, ClearanceShape,
    CompiledRule, ContainmentOutcome, ContainmentRequest, CylinderClearance, ElevationBand,
    FrameOffsetPlacement, FreeSpaceError, FreeSpaceServiceHandle, MetricDirection, MetricFrame,
    MetricPoint, NotEvaluatedReason, ObjectFrame, ObjectFrameServiceHandle, ObjectFront,
    ParameterDescriptor, ParameterType, PlacementDomain, PlacementOrientation, PlacementOutcome,
    PlacementRequest, PlacementShape, RuleCapability, RuleContext, SignedDistanceInterval,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::body_extent::{extent_error, frame_error};
use crate::level_spacing::{extent, metres};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Requires a free volume on a stated side of each selected component.
///
/// The volume is placed in the component's placement frame. The rule states
/// which frame axis is the component's front (`front_axis`: `forward`,
/// `-forward`, `right` or `-right`, or `stated` for the front the source
/// states), because a placement axis says nothing about which way a
/// component faces; the engine never infers one. `side` is `front`, `back`,
/// `left` or `right` of that front, left and right as seen facing along it
/// (the component's own left and right); `both_sides` adds the opposite
/// side as a check of its own.
///
/// The volume is a box (`width` across the side, `depth` away from it) or a
/// cylinder (`radius`), `height` high. Along the side it starts at the
/// component's outermost point plus `offset`; across it, it is centred on
/// the component or flush with its left or right edge (`align`, as seen
/// looking out of that side), moved by `lateral_offset`; its base lies
/// `vertical_offset` above the `height_reference`: the floor (the lowest
/// point of the spaces `space_path` reaches), or the component's bottom or
/// top.
///
/// `size_mode` says what the size is: a `minimum` (the default: the volume,
/// less `size_tolerance` in every dimension, must be free), a `maximum` (no
/// volume `size_tolerance` larger in any one dimension may be free) or
/// `fixed` (both).
///
/// With `slide_from` and `slide_to`, the volume floats: it may slide across
/// the side by any offset between the two, to the right as seen looking out
/// of it, and is free when it is free at one of them. A floating volume is
/// searched in the spaces `space_path` reaches, so it always lies inside
/// them.
///
/// Obstacles are the `obstacles` selection less the component itself and
/// the `allowed_intruders`. With `protrusion`, an obstacle may reach up to
/// that far into the volume through any of its plan sides: the volume
/// checked is the declared one shrunk by `protrusion` on every plan side,
/// so an obstacle it meets reaches further in than allowed, and one it
/// misses reaches no further. With `within_space`, the declared volume's
/// plan must also lie inside the union of the spaces `space_path` reaches,
/// judged on its own.
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

#[derive(Clone, Copy)]
enum FrontAxis {
    Forward,
    Backward,
    Right,
    Left,
    Stated,
}

impl FrontAxis {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "forward" => Ok(Self::Forward),
            "-forward" => Ok(Self::Backward),
            "right" => Ok(Self::Right),
            "-right" => Ok(Self::Left),
            "stated" => Ok(Self::Stated),
            other => Err(invalid(format!(
                "front_axis `{other}` is unsupported; use `forward`, `-forward`, `right`, \
                 `-right` or `stated`"
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
        }
    }
}

#[derive(Clone, Copy)]
enum Align {
    Centre,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Reference {
    Floor,
    Bottom,
    Top,
}

#[derive(Clone, Copy)]
enum Shape {
    Box { width: f64, depth: f64 },
    Cylinder { radius: f64 },
}

/// A volume's plan shape and height.
#[derive(Clone, Copy)]
struct Size {
    shape: Shape,
    height: f64,
}

impl Size {
    fn describe(self) -> String {
        let height = metres(self.height);
        match self.shape {
            Shape::Box { width, depth } => format!(
                "{} wide, {} deep, {height} high",
                metres(width),
                metres(depth)
            ),
            Shape::Cylinder { radius } => {
                format!("{} in radius, {height} high", metres(radius))
            }
        }
    }

    /// Every dimension changed by `by`.
    fn changed(self, by: f64) -> Self {
        Self {
            shape: match self.shape {
                Shape::Box { width, depth } => Shape::Box {
                    width: width + by,
                    depth: depth + by,
                },
                Shape::Cylinder { radius } => Shape::Cylinder {
                    radius: radius + by,
                },
            },
            height: self.height + by,
        }
    }

    /// The sizes `by` larger in one dimension each, with that dimension's
    /// name.
    fn larger(self, by: f64) -> Vec<(&'static str, Self)> {
        let taller = Self {
            height: self.height + by,
            ..self
        };
        let mut sizes = match self.shape {
            Shape::Box { width, depth } => vec![
                (
                    "width",
                    Self {
                        shape: Shape::Box {
                            width: width + by,
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
                            depth: depth + by,
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

    /// The plan shape reaching `by` further out and in: a box `2 by` deeper,
    /// a cylinder `by` wider.
    fn deeper(self, by: f64) -> Self {
        Self {
            shape: match self.shape {
                Shape::Box { width, depth } => Shape::Box {
                    width,
                    depth: depth + 2.0 * by,
                },
                Shape::Cylinder { radius } => Shape::Cylinder {
                    radius: radius + by,
                },
            },
            ..self
        }
    }

    /// Whether `protrusion` leaves a volume of this size.
    fn holds(self, protrusion: f64) -> bool {
        self.height > 0.0
            && match self.shape {
                Shape::Box { width, depth } => 2.0 * protrusion < width.min(depth),
                Shape::Cylinder { radius } => protrusion < radius,
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
    front: FrontAxis,
    size: Size,
    mode: SizeMode,
    tolerance: f64,
    offset: f64,
    lateral_offset: f64,
    align: Align,
    reference: Reference,
    vertical_offset: f64,
    obstacles: &'a Selector,
    allowed: Option<&'a Selector>,
    protrusion: f64,
    within_space: bool,
    /// The offsets across the side a floating volume may slide by.
    slide: Option<(f64, f64)>,
    spaces: Option<Traversal<'a>>,
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

/// The size, its mode and tolerance, checked against `protrusion`.
fn sizes(
    parameters: &Parameters<'_>,
    protrusion: f64,
) -> Result<(Size, SizeMode, f64), Unavailable> {
    let height =
        positive(parameters, "height")?.ok_or_else(|| invalid("parameter `height` is required"))?;
    let shape = match (
        positive(parameters, "width")?,
        positive(parameters, "depth")?,
        positive(parameters, "radius")?,
    ) {
        (Some(width), Some(depth), None) => Shape::Box { width, depth },
        (None, None, Some(radius)) => Shape::Cylinder { radius },
        _ => {
            return Err(invalid(
                "declare `width` and `depth` for a box or `radius` for a cylinder",
            ));
        }
    };
    let size = Size { shape, height };
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
    if !size.holds(protrusion) {
        return Err(invalid(match shape {
            Shape::Box { .. } => {
                "`protrusion` leaves no volume: it must be less than half the width and the \
                 depth"
            }
            Shape::Cylinder { .. } => {
                "`protrusion` leaves no volume: it must be less than the radius"
            }
        }));
    }
    if mode != SizeMode::Maximum && !size.changed(-tolerance).holds(protrusion) {
        return Err(invalid(
            "`size_tolerance` and `protrusion` leave no volume of the minimum size",
        ));
    }
    Ok((size, mode, tolerance))
}

/// The offsets a floating volume may slide by.
fn slide(parameters: &Parameters<'_>) -> Result<Option<(f64, f64)>, Unavailable> {
    match (
        length(parameters, "slide_from")?,
        length(parameters, "slide_to")?,
    ) {
        (None, None) => Ok(None),
        (Some(from), Some(to)) if from <= to => Ok(Some((from, to))),
        (Some(_), Some(_)) => Err(invalid("`slide_from` lies beyond `slide_to`")),
        _ => Err(invalid("`slide_from` and `slide_to` go together")),
    }
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let stated = Side::parse(parameters.required_string("side")?)?;
        let sides = if parameters.boolean("both_sides")?.unwrap_or(false) {
            vec![stated, stated.opposite()]
        } else {
            vec![stated]
        };
        let front = FrontAxis::parse(parameters.required_string("front_axis")?)?;
        let protrusion = non_negative(&parameters, "protrusion")?;
        let (size, mode, tolerance) = sizes(&parameters, protrusion)?;
        let align = match parameters.string("align")? {
            None | Some("centre") => Align::Centre,
            Some("left") => Align::Left,
            Some("right") => Align::Right,
            Some(other) => {
                return Err(invalid(format!(
                    "align `{other}` is unsupported; use `centre`, `left` or `right`"
                )));
            }
        };
        let reference = match parameters.required_string("height_reference")? {
            "floor" => Reference::Floor,
            "bottom" => Reference::Bottom,
            "top" => Reference::Top,
            other => {
                return Err(invalid(format!(
                    "height_reference `{other}` is unsupported; use `floor`, `bottom` or `top`"
                )));
            }
        };
        let slide = slide(&parameters)?;
        let within_space = parameters.boolean("within_space")?.unwrap_or(false);
        let spaces = match parameters.strings("space_path")? {
            Some(path) => Some(Traversal::path(path)?),
            None => None,
        };
        let needs_spaces = within_space || slide.is_some() || matches!(reference, Reference::Floor);
        match (needs_spaces, &spaces) {
            (true, None) => {
                return Err(invalid(
                    "`height_reference` `floor`, `within_space` and a floating volume need \
                     `space_path`",
                ));
            }
            (false, Some(_)) => {
                return Err(invalid(
                    "`space_path` applies only to `height_reference` `floor`, `within_space` \
                     or a floating volume",
                ));
            }
            _ => {}
        }
        Ok(Self {
            sides,
            front,
            size,
            mode,
            tolerance,
            offset: length(&parameters, "offset")?.unwrap_or(0.0),
            lateral_offset: length(&parameters, "lateral_offset")?.unwrap_or(0.0),
            align,
            reference,
            vertical_offset: length(&parameters, "vertical_offset")?.unwrap_or(0.0),
            obstacles: parameters.required_selector("obstacles")?,
            allowed: parameters.selector("allowed_intruders")?,
            protrusion,
            within_space,
            slide,
            spaces,
        })
    }
}

impl RuleCapability for ComponentClearance {
    fn id(&self) -> &'static str {
        ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("side", ParameterType::String),
            ParameterDescriptor::required("front_axis", ParameterType::String),
            ParameterDescriptor::optional("both_sides", ParameterType::Boolean),
            ParameterDescriptor::optional("width", ParameterType::Quantity),
            ParameterDescriptor::optional("depth", ParameterType::Quantity),
            ParameterDescriptor::optional("radius", ParameterType::Quantity),
            ParameterDescriptor::required("height", ParameterType::Quantity),
            ParameterDescriptor::optional("size_mode", ParameterType::String),
            ParameterDescriptor::optional("size_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("offset", ParameterType::Quantity),
            ParameterDescriptor::optional("lateral_offset", ParameterType::Quantity),
            ParameterDescriptor::optional("align", ParameterType::String),
            ParameterDescriptor::optional("slide_from", ParameterType::Quantity),
            ParameterDescriptor::optional("slide_to", ParameterType::Quantity),
            ParameterDescriptor::required("height_reference", ParameterType::String),
            ParameterDescriptor::optional("vertical_offset", ParameterType::Quantity),
            ParameterDescriptor::required("obstacles", ParameterType::Selector),
            ParameterDescriptor::optional("allowed_intruders", ParameterType::Selector),
            ParameterDescriptor::optional("protrusion", ParameterType::Quantity),
            ParameterDescriptor::optional("within_space", ParameterType::Boolean),
            ParameterDescriptor::optional("space_path", ParameterType::StringList),
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
        let services = Services {
            frames,
            extents,
            free_space,
        };
        let obstacles = Obstacles::select(context, &config);
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            for (side, result) in check(context, &config, &services, &obstacles, object) {
                let label = format!("{} clearance", side.name());
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

struct Services<'a> {
    frames: &'a ObjectFrameServiceHandle,
    extents: &'a VerticalExtentServiceHandle,
    free_space: &'a FreeSpaceServiceHandle,
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
        let mut failed = None;
        let mut split = |selector: &Selector, what: &str| {
            let (objects, outcomes) = select_objects(context, selector);
            let mut undecided = BTreeSet::new();
            for outcome in outcomes.not_evaluated_outcomes() {
                match outcome.object_id() {
                    Some(object) => {
                        undecided.insert(object.clone());
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
            let chosen: BTreeSet<ObjectId> =
                objects.iter().map(|object| object.id.clone()).collect();
            (chosen, undecided)
        };
        let (selected, undecided) = split(config.obstacles, "obstacle");
        let (allowed, allowed_undecided) = match config.allowed {
            Some(selector) => split(selector, "allowed-intruder"),
            None => (BTreeSet::new(), BTreeSet::new()),
        };
        let sure = selected
            .iter()
            .filter(|id| !allowed.contains(*id) && !allowed_undecided.contains(*id))
            .cloned()
            .collect();
        let maybe = undecided
            .iter()
            .chain(selected.intersection(&allowed_undecided))
            .filter(|id| !allowed.contains(*id))
            .cloned()
            .collect();
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
        let faces = placed.faces(services, object, *side);
        let free = |size: Size| -> Result<Freedom, Unavailable> {
            let volume = faces.as_ref().map_err(Clone::clone)?.volume(config, size);
            match config.slide {
                None => clearance(config, services, obstacles, object, &volume, size),
                Some(slide) => search(
                    config, services, obstacles, &placed, object, &volume, size, slide,
                ),
            }
        };
        if config.mode != SizeMode::Maximum {
            let size = config.size.changed(-config.tolerance);
            let judged = free(size).map(|freedom| match freedom {
                Freedom::Free(_) => None,
                Freedom::Blocked(message, evidence, related) => Some((
                    format!("({}) {message}", size.describe()),
                    evidence,
                    related,
                )),
            });
            results.push((*side, judged));
        }
        if config.mode != SizeMode::Minimum {
            for (dimension, size) in config.size.larger(config.tolerance) {
                let judged = free(size).map(|freedom| match freedom {
                    Freedom::Blocked(..) => None,
                    Freedom::Free(evidence) => Some((
                        format!(
                            "({}) is free, so the free volume exceeds the maximum {dimension}",
                            size.describe()
                        ),
                        evidence,
                        Vec::new(),
                    )),
                });
                results.push((*side, judged));
            }
        }
        // A floating volume is searched in the spaces, so it lies in them.
        if config.within_space && config.slide.is_none() {
            let containment = match &faces {
                Ok(faces) => containment(
                    config,
                    services,
                    &placed,
                    object,
                    &faces.volume(config, config.size),
                ),
                Err(error) => Err(error.clone()),
            };
            results.push((*side, containment));
        }
    }
    results
}

/// What every side of one component shares: its frame, its front, its
/// base elevation and its spaces.
struct Placement {
    front: [f64; 3],
    up: [f64; 3],
    /// Elevation interval of the height reference.
    base: (f64, f64),
    spaces: Vec<ObjectId>,
    evidence: Vec<Evidence>,
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
        let frame = services
            .frames
            .object_frame(&object.id)
            .map_err(|error| frame_error(&error))?;
        let up = frame.frame().up().components();
        if (up[2] - 1.0).abs() > AXIS_TOLERANCE {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the component's frame is tilted; clearances are measured in upright frames only"
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
        let (spaces, cited) = match &config.spaces {
            Some(path) => {
                let everything: Vec<&Object> = context.project.objects().collect();
                path.related(context, &object.id, &everything)?
            }
            None => (Vec::new(), Vec::new()),
        };
        evidence.extend(cited);
        let base = match config.reference {
            Reference::Bottom | Reference::Top => {
                let measured = extent(services.extents, &object.id)?;
                evidence.push(measured.evidence().clone());
                let elevation = if matches!(config.reference, Reference::Bottom) {
                    measured.bottom()
                } else {
                    measured.top()
                };
                (elevation.lower_metres(), elevation.upper_metres())
            }
            Reference::Floor => {
                if spaces.is_empty() {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        "`space_path` reaches no space, so there is no floor to measure from"
                            .into(),
                    ));
                }
                // The floor is one of the spaces' bottoms; the hull holds it
                // whichever it is.
                let mut hull = (f64::INFINITY, f64::NEG_INFINITY);
                for space in &spaces {
                    let measured = extent(services.extents, space)?;
                    evidence.push(measured.evidence().clone());
                    hull.0 = hull.0.min(measured.bottom().lower_metres());
                    hull.1 = hull.1.max(measured.bottom().upper_metres());
                }
                hull
            }
        };
        Ok(Self {
            front,
            up,
            base,
            spaces,
            evidence,
        })
    }

    /// The component's faces on `side`: the axes and the intervals its
    /// outermost point and its edges across the side are known to.
    fn faces(
        &self,
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
        let (low, high) = (beside.lower(), beside.upper());
        Ok(Faces {
            outward,
            across,
            up,
            face: (along.upper().lower_metres(), along.upper().upper_metres()),
            low: (low.lower_metres(), low.upper_metres()),
            high: (high.lower_metres(), high.upper_metres()),
            base: self.base,
            evidence,
        })
    }
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
    /// The component's outermost point on this side.
    face: (f64, f64),
    /// Its left and right edges across the side.
    low: (f64, f64),
    high: (f64, f64),
    /// Elevation interval of the height reference.
    base: (f64, f64),
    evidence: Vec<Evidence>,
}

impl Faces {
    /// The volume of `size` on this side, with the intervals its position
    /// is known to.
    fn volume(&self, config: &Config<'_>, size: Size) -> Volume {
        let (low, high) = (self.low, self.high);
        let half = match size.shape {
            Shape::Box { width, .. } => width / 2.0,
            Shape::Cylinder { radius } => radius,
        };
        let centre_across = match config.align {
            Align::Centre => (f64::midpoint(low.0, high.0), f64::midpoint(low.1, high.1)),
            Align::Left => (low.0 + half, low.1 + half),
            Align::Right => (high.0 - half, high.1 - half),
        };
        let reach = match size.shape {
            Shape::Box { depth, .. } => depth / 2.0,
            Shape::Cylinder { radius } => radius,
        };
        Volume {
            outward: self.outward,
            across: self.across,
            up: self.up,
            centre_out: (
                self.face.0 + config.offset + reach,
                self.face.1 + config.offset + reach,
            ),
            centre_across: (
                centre_across.0 + config.lateral_offset,
                centre_across.1 + config.lateral_offset,
            ),
            base: (
                self.base.0 + config.vertical_offset,
                self.base.1 + config.vertical_offset,
            ),
            evidence: self.evidence.clone(),
        }
    }
}

/// A volume's axes and the intervals its centre and base are known to.
struct Volume {
    outward: MetricDirection,
    across: MetricDirection,
    up: MetricDirection,
    centre_out: (f64, f64),
    centre_across: (f64, f64),
    base: (f64, f64),
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

impl Bound {
    fn sign(self) -> f64 {
        match self {
            Self::Union => 1.0,
            Self::Common => -1.0,
        }
    }
}

fn spread((low, high): (f64, f64)) -> f64 {
    high - low
}

impl Volume {
    fn exact(&self) -> bool {
        #[allow(clippy::float_cmp)]
        let point = |(low, high): (f64, f64)| low == high;
        point(self.centre_out) && point(self.centre_across) && point(self.base)
    }

    /// The plan shape of one bound, `height` high, shrunk by `inset` on
    /// every plan side; `None` when the common part is empty.
    fn shape(
        &self,
        size: Size,
        inset: f64,
        height: f64,
        bound: Bound,
    ) -> Result<Option<ClearanceShape>, Unavailable> {
        let sign = bound.sign();
        let (out, across) = (spread(self.centre_out), spread(self.centre_across));
        match size.shape {
            Shape::Box { width, depth } => {
                let width = width - 2.0 * inset + sign * across;
                let depth = depth - 2.0 * inset + sign * out;
                if width <= 0.0 || depth <= 0.0 || height <= 0.0 {
                    return Ok(None);
                }
                BoxClearance::try_new(width, depth, height).map(ClearanceShape::Box)
            }
            Shape::Cylinder { radius } => {
                // A disc whose centre lies anywhere in a rectangle stays in
                // the disc grown by the rectangle's half-diagonal and holds
                // the disc shrunk by it.
                let radius = radius - inset + sign * out.hypot(across) / 2.0;
                if radius <= 0.0 || height <= 0.0 {
                    return Ok(None);
                }
                CylinderClearance::try_new(radius, height).map(ClearanceShape::Cylinder)
            }
        }
        .map(Some)
        .map_err(|error| free_space_error(&error))
    }

    /// The middle of the volume's positions, at elevation `elevation`.
    fn centre(&self, object: &ObjectId, elevation: f64) -> Result<MetricPoint, Unavailable> {
        let (o, a, u) = (
            self.outward.components(),
            self.across.components(),
            self.up.components(),
        );
        let (co, ca) = (
            f64::midpoint(self.centre_out.0, self.centre_out.1),
            f64::midpoint(self.centre_across.0, self.centre_across.1),
        );
        let origin = [0, 1, 2].map(|i| co * o[i] + ca * a[i] + elevation * u[i]);
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
        size: Size,
        object: &ObjectId,
        inset: f64,
        bound: Bound,
    ) -> Result<Option<(MetricFrame, ClearanceShape)>, Unavailable> {
        let height = size.height + bound.sign() * spread(self.base);
        let Some(shape) = self.shape(size, inset, height, bound)? else {
            return Ok(None);
        };
        let base = match bound {
            Bound::Union => self.base.0,
            Bound::Common => self.base.1,
        };
        let frame = MetricFrame::try_new(
            self.centre(object, base)?,
            self.across,
            self.outward,
            self.up,
        )
        .map_err(|error| free_space_error(&error))?;
        Ok(Some((frame, shape)))
    }
}

fn free_space_error(error: &FreeSpaceError) -> Unavailable {
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
    size: Size,
) -> Result<Freedom, Unavailable> {
    if let Some(failed) = &obstacles.failed {
        return Err(failed.clone());
    }
    let (sure, maybe) = obstacles.without(&[&object.id]);
    let mut candidates = sure.clone();
    candidates.extend(maybe);
    let ask = |bound: Bound, candidates: &[ObjectId]| {
        volume
            .bound(size, &object.id, config.protrusion, bound)?
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
/// rarely verify, so the free volume is searched `SLACK` deeper on each side
/// within `SLACK` of the line, and holds the volume on the line.
const SLACK: f64 = 1.0e-3;

/// Whether the floating volume fits at some offset across the side,
/// three-valued, by a placement search in the component's spaces.
///
/// The volume's position is an interval. A fit of the volume grown by it
/// (the union) at an offset holds the volume at that offset wherever it
/// is; the volume shrunk by it (the common part) fits at every offset the
/// volume fits at, so its absence proves the volume's.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn search(
    config: &Config<'_>,
    services: &Services<'_>,
    obstacles: &Obstacles,
    placed: &Placement,
    object: &Object,
    volume: &Volume,
    size: Size,
    (from, to): (f64, f64),
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
    // The anchor stands in the middle of the volume's positions, on the
    // floor, with the plan axes of the side; only the offset across it
    // slides.
    let outward = volume.outward.components();
    let outward = direction([outward[0], outward[1], 0.0])?;
    let up = direction([0.0, 0.0, 1.0])?;
    let across = direction(cross(outward.components(), up.components()))?;
    let floor_middle = f64::midpoint(floor.0, floor.1);
    let anchor = MetricFrame::try_new(
        volume.centre(&object.id, floor_middle)?,
        across,
        outward,
        up,
    )
    .map_err(|error| free_space_error(&error))?;
    let reach = spread(floor) / 2.0 + FLOOR_MARGIN;
    let offsets = |slack: f64| {
        Ok::<_, FreeSpaceError>(FrameOffsetPlacement::new(
            anchor.clone(),
            SignedDistanceInterval::try_new(from, to)?,
            SignedDistanceInterval::try_new(-slack, slack)?,
            SignedDistanceInterval::try_new(-reach, reach)?,
        ))
    };
    // The base above the scope's floor, as an interval.
    let lowest = volume.base.0 - floor.1;
    let highest = volume.base.1 - floor.0;
    let request = |bound: Bound,
                   candidates: Vec<ObjectId>|
     -> Result<Option<PlacementRequest>, Unavailable> {
        let (band_from, band_to) = match bound {
            Bound::Union => (lowest, highest + size.height),
            Bound::Common => (highest, lowest + size.height),
        };
        if band_to <= band_from {
            return Ok(None);
        }
        let band = ElevationBand::try_new(band_from, band_to).map_err(|_| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the volume's base lies {} to {} above the floor of {scope}; a floating \
                     volume must stand at or above it",
                    metres(lowest),
                    metres(highest)
                ),
            )
        })?;
        // A witness may stand off the line by the slack, which the deeper
        // volume makes up for; an absence is proven on the line itself.
        let (slack, size) = match bound {
            Bound::Union => (SLACK, size.deeper(SLACK)),
            Bound::Common => (0.0, size),
        };
        let Some(shape) = volume.shape(size, config.protrusion, band_to - band_from, bound)? else {
            return Ok(None);
        };
        let offsets = offsets(slack).map_err(|error| free_space_error(&error))?;
        let shape = match shape {
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
            PlacementDomain::FrameOffsets(offsets),
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
            format!(
                "fits nowhere between {} and {} across the side in {}",
                metres(from),
                metres(to),
                names(&placed.spaces)
            ),
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
    config: &Config<'_>,
    services: &Services<'_>,
    placed: &Placement,
    object: &Object,
    volume: &Volume,
) -> Judged {
    let described = config.size.describe();
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
            .bound(config.size, &object.id, 0.0, bound)?
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
