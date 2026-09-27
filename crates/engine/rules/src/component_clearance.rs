//! `component-clearance`: a free box or cylinder in front of, behind or
//! beside each selected component, placed in the component's own frame on
//! a side the rule states.

use std::collections::BTreeSet;

use axioval_engine::{
    BoxClearance, CapabilityEvaluation, ClearanceOutcome, ClearanceRequest, ClearanceShape,
    CompiledRule, ContainmentOutcome, ContainmentRequest, CylinderClearance, FreeSpaceError,
    FreeSpaceServiceHandle, MetricDirection, MetricFrame, MetricPoint, NotEvaluatedReason,
    ObjectFrame, ObjectFrameServiceHandle, ObjectFront, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, VerticalExtentServiceHandle,
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
/// top. A minimum size is checked as a box of that size: a larger free
/// volume holds it.
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
/// check measures the union of every position the volume could take (clear
/// means clear wherever it is) and their common part (obstructed there
/// means obstructed wherever it is); anything between is not evaluated. An
/// obstacle the selection cannot decide can only obstruct, so a clear
/// volume stands and an obstruction by undecided objects alone is not
/// evaluated.
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

struct Config<'a> {
    sides: Vec<Side>,
    front: FrontAxis,
    shape: Shape,
    height: f64,
    offset: f64,
    lateral_offset: f64,
    align: Align,
    reference: Reference,
    vertical_offset: f64,
    obstacles: &'a Selector,
    allowed: Option<&'a Selector>,
    protrusion: f64,
    within_space: bool,
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

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let side = Side::parse(parameters.required_string("side")?)?;
        let sides = if parameters.boolean("both_sides")?.unwrap_or(false) {
            vec![side, side.opposite()]
        } else {
            vec![side]
        };
        let front = FrontAxis::parse(parameters.required_string("front_axis")?)?;
        let height = positive(&parameters, "height")?
            .ok_or_else(|| invalid("parameter `height` is required"))?;
        let protrusion = match length(&parameters, "protrusion")? {
            Some(value) if value < 0.0 => return Err(invalid("`protrusion` is negative")),
            other => other.unwrap_or(0.0),
        };
        let shape = match (
            positive(&parameters, "width")?,
            positive(&parameters, "depth")?,
            positive(&parameters, "radius")?,
        ) {
            (Some(width), Some(depth), None) => {
                if 2.0 * protrusion >= width.min(depth) {
                    return Err(invalid(
                        "`protrusion` leaves no volume: it must be less than half the width \
                         and the depth",
                    ));
                }
                Shape::Box { width, depth }
            }
            (None, None, Some(radius)) => {
                if protrusion >= radius {
                    return Err(invalid(
                        "`protrusion` leaves no volume: it must be less than the radius",
                    ));
                }
                Shape::Cylinder { radius }
            }
            _ => {
                return Err(invalid(
                    "declare `width` and `depth` for a box or `radius` for a cylinder",
                ));
            }
        };
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
        let within_space = parameters.boolean("within_space")?.unwrap_or(false);
        let spaces = match parameters.strings("space_path")? {
            Some(path) => Some(Traversal::path(path)?),
            None => None,
        };
        let needs_spaces = within_space || matches!(reference, Reference::Floor);
        match (needs_spaces, &spaces) {
            (true, None) => {
                return Err(invalid(
                    "`height_reference` `floor` and `within_space` need `space_path`",
                ));
            }
            (false, Some(_)) => {
                return Err(invalid(
                    "`space_path` applies only to `height_reference` `floor` or `within_space`",
                ));
            }
            _ => {}
        }
        Ok(Self {
            sides,
            front,
            shape,
            height,
            offset: length(&parameters, "offset")?.unwrap_or(0.0),
            lateral_offset: length(&parameters, "lateral_offset")?.unwrap_or(0.0),
            align,
            reference,
            vertical_offset: length(&parameters, "vertical_offset")?.unwrap_or(0.0),
            obstacles: parameters.required_selector("obstacles")?,
            allowed: parameters.selector("allowed_intruders")?,
            protrusion,
            within_space,
            spaces,
        })
    }

    fn describe(&self) -> String {
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
            ParameterDescriptor::optional("offset", ParameterType::Quantity),
            ParameterDescriptor::optional("lateral_offset", ParameterType::Quantity),
            ParameterDescriptor::optional("align", ParameterType::String),
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
                        format!("{label} ({}) {message}", config.describe()),
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
}

type Judged = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

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
        let volume = placed.volume(config, services, object, *side);
        let clearance = match &volume {
            Ok(volume) => clearance(config, services, obstacles, object, volume),
            Err(error) => Err(error.clone()),
        };
        results.push((*side, clearance));
        if config.within_space {
            let containment = match &volume {
                Ok(volume) => containment(config, services, &placed, object, volume),
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

    /// The volume on `side`, with the intervals its position is known to.
    fn volume(
        &self,
        config: &Config<'_>,
        services: &Services<'_>,
        object: &Object,
        side: Side,
    ) -> Result<Volume, Unavailable> {
        let outward = side.outward(self.front, self.up);
        // Looking out of the side with up overhead, the box's right.
        let across = cross(outward, self.up);
        let direction = |vector: [f64; 3]| {
            MetricDirection::try_new(vector).map_err(|error| {
                (
                    NotEvaluatedReason::InvalidEvidence,
                    format!("clearance axis: {error}"),
                )
            })
        };
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
        // The component's outermost point on this side.
        let face = (along.upper().lower_metres(), along.upper().upper_metres());
        let (low, high) = (beside.lower(), beside.upper());
        let half = match config.shape {
            Shape::Box { width, .. } => width / 2.0,
            Shape::Cylinder { radius } => radius,
        };
        let centre_across = match config.align {
            Align::Centre => (
                f64::midpoint(low.lower_metres(), high.lower_metres()),
                f64::midpoint(low.upper_metres(), high.upper_metres()),
            ),
            Align::Left => (low.lower_metres() + half, low.upper_metres() + half),
            Align::Right => (high.lower_metres() - half, high.upper_metres() - half),
        };
        let reach = match config.shape {
            Shape::Box { depth, .. } => depth / 2.0,
            Shape::Cylinder { radius } => radius,
        };
        Ok(Volume {
            outward,
            across,
            up,
            centre_out: (
                face.0 + config.offset + reach,
                face.1 + config.offset + reach,
            ),
            centre_across: (
                centre_across.0 + config.lateral_offset,
                centre_across.1 + config.lateral_offset,
            ),
            base: (
                self.base.0 + config.vertical_offset,
                self.base.1 + config.vertical_offset,
            ),
            evidence,
        })
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
    /// Every position together: clear here is clear anywhere.
    Union,
    /// What every position shares: obstructed here is obstructed anywhere.
    Common,
}

impl Volume {
    fn exact(&self) -> bool {
        #[allow(clippy::float_cmp)]
        let point = |(low, high): (f64, f64)| low == high;
        point(self.centre_out) && point(self.centre_across) && point(self.base)
    }

    /// The frame and shape of one bound, shrunk by `inset` on every plan
    /// side; `None` when the common part is empty.
    fn bound(
        &self,
        config: &Config<'_>,
        object: &ObjectId,
        inset: f64,
        bound: Bound,
    ) -> Result<Option<(MetricFrame, ClearanceShape)>, Unavailable> {
        let spread = |(low, high): (f64, f64)| high - low;
        let sign = match bound {
            Bound::Union => 1.0,
            Bound::Common => -1.0,
        };
        let (out, across, rise) = (
            spread(self.centre_out),
            spread(self.centre_across),
            spread(self.base),
        );
        let height = config.height + sign * rise;
        let base = match bound {
            Bound::Union => self.base.0,
            Bound::Common => self.base.1,
        };
        let shape = match config.shape {
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
        .map_err(|error| free_space_error(&error))?;
        let (o, a, u) = (
            self.outward.components(),
            self.across.components(),
            self.up.components(),
        );
        let (co, ca) = (
            f64::midpoint(self.centre_out.0, self.centre_out.1),
            f64::midpoint(self.centre_across.0, self.centre_across.1),
        );
        let origin = [0, 1, 2].map(|i| co * o[i] + ca * a[i] + base * u[i]);
        let point = MetricPoint::try_new(object.clone(), origin).map_err(|error| {
            (
                NotEvaluatedReason::InvalidEvidence,
                format!("clearance origin: {error}"),
            )
        })?;
        let frame = MetricFrame::try_new(point, self.across, self.outward, self.up)
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

/// Whether the volume is free of obstacles, three-valued.
fn clearance(
    config: &Config<'_>,
    services: &Services<'_>,
    obstacles: &Obstacles,
    object: &Object,
    volume: &Volume,
) -> Judged {
    if let Some(failed) = &obstacles.failed {
        return Err(failed.clone());
    }
    let without = |set: &BTreeSet<ObjectId>| -> Vec<ObjectId> {
        set.iter().filter(|id| **id != object.id).cloned().collect()
    };
    let sure = without(&obstacles.sure);
    let mut candidates = sure.clone();
    candidates.extend(without(&obstacles.maybe));
    let ask = |bound: Bound, candidates: &[ObjectId]| {
        volume
            .bound(config, &object.id, config.protrusion, bound)?
            .map(|(frame, shape)| {
                services
                    .free_space
                    .assess_clearance(&ClearanceRequest::new(frame, shape, candidates.to_vec()))
                    .map_err(|error| free_space_error(&error))
            })
            .transpose()
    };
    let union = ask(Bound::Union, &candidates);
    if let Ok(Some(ClearanceOutcome::Clear(_))) = &union {
        return Ok(None);
    }
    let common = if volume.exact() {
        union.clone()
    } else {
        ask(Bound::Common, &candidates)
    };
    let found = |blockers: &[ObjectId], proof: &Evidence| {
        let mut evidence = volume.evidence.clone();
        evidence.push(proof.clone());
        let names: Vec<String> = blockers.iter().map(ToString::to_string).collect();
        Ok(Some((
            format!("is obstructed by {}", names.join(", ")),
            evidence,
            blockers.to_vec(),
        )))
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
                    proof
                        .blockers()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
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

/// Whether the declared volume's plan lies inside the reached spaces.
fn containment(
    config: &Config<'_>,
    services: &Services<'_>,
    placed: &Placement,
    object: &Object,
    volume: &Volume,
) -> Judged {
    let spaces: Vec<String> = placed.spaces.iter().map(ToString::to_string).collect();
    if placed.spaces.is_empty() {
        return Ok(Some((
            "has no space to lie in: `space_path` reaches none".into(),
            volume.evidence.clone(),
            Vec::new(),
        )));
    }
    let ask = |bound: Bound| {
        volume
            .bound(config, &object.id, 0.0, bound)?
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
            format!("extends outside {}", spaces.join(", ")),
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
