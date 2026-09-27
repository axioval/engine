//! `opening-zone`: each opening lies within its host and inside the zone
//! the host allows openings in.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, QuantityDimension};

use crate::body_facts::BodyFacts;
use crate::counts::Population;
use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Requires each selected opening to lie within its host's face and inside
/// the zone the rule allows: clear of the host's ends by `end_distance`,
/// clear of its edges (or of its flanges, with `zone` `web`) by
/// `edge_distance`, and `opening_spacing` clear of every other opening in
/// the same host.
///
/// The host is what `host_path` reaches from the opening among the
/// `host_selector` objects (with IFC, `IfcRelVoidsElement` backward). An
/// opening reaching no such host is not checked; one reaching several, or
/// an object whose selection is undecided, is not evaluated.
///
/// Both bodies are read from the reserved body set (`axioval:body`), never
/// from a mesh: the host must be one straight extrusion perpendicular to its
/// profile, of a profile family whose outline the set bounds (rectangles,
/// circles, ellipses and the I, T, U, C, Z and L sections, centred on their
/// position); the opening one straight extrusion of a rectangle, rounded
/// rectangle, circle or ellipse. The host's axes are named by the rule:
/// `length_axis` and `height_axis` are two of `extrusion`, `profile-x` and
/// `profile-y` (a beam: `extrusion` and `profile-y`; a wall extruded up
/// from its plan outline: `profile-x` and `extrusion`), and the third runs
/// through the host. The opening's extent along each face axis is exact:
/// the support of its section swept along its extrusion. Anything else is
/// not evaluated, never approximated.
///
/// Distances between openings are clear distances in the host's face.
/// Between two rectangles whose sides run along the face axes and which are
/// extruded through the host they are exact; between any other pair only
/// the distance of their extents is known, a lower bound, which can pass a
/// pair but never find one.
///
/// Positions are composed from placements in binary arithmetic, so every
/// bound is widened by a nanometre, far below any modelling tolerance.
pub struct OpeningZone;

/// How far composed placements may be off in binary arithmetic.
const ROUNDING: f64 = 1e-9;

/// How nearly two unit vectors must agree to be taken as parallel.
const PARALLEL: f64 = 1e-9;

type Vector = [f64; 3];

fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn along(origin: Vector, a: Vector, s: f64, b: Vector, t: f64) -> Vector {
    [
        origin[0] + a[0] * s + b[0] * t,
        origin[1] + a[1] * s + b[1] * t,
        origin[2] + a[2] * s + b[2] * t,
    ]
}

fn minus(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn parallel(a: Vector, b: Vector) -> bool {
    dot(a, b).abs() >= 1.0 - PARALLEL
}

/// One of the host's three axes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Extrusion,
    ProfileX,
    ProfileY,
}

impl Axis {
    fn parse(name: &str, value: &str) -> Result<Self, Unavailable> {
        match value {
            "extrusion" => Ok(Self::Extrusion),
            "profile-x" => Ok(Self::ProfileX),
            "profile-y" => Ok(Self::ProfileY),
            other => Err(invalid(format!(
                "`{name}` `{other}` is unsupported; use `extrusion`, `profile-x` or `profile-y`"
            ))),
        }
    }
}

struct Config<'a> {
    hosts: Traversal<'a>,
    host_selector: &'a Selector,
    length: Axis,
    height: Axis,
    end_distance: Option<f64>,
    edge_distance: Option<f64>,
    web: bool,
    spacing: Option<f64>,
}

fn distance(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some(_) => Err(invalid(format!("`{name}` is not a non-negative length"))),
    }
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let host_path = parameters
            .strings("host_path")?
            .ok_or_else(|| invalid("parameter `host_path` is required"))?;
        let length = Axis::parse("length_axis", parameters.required_string("length_axis")?)?;
        let height = Axis::parse("height_axis", parameters.required_string("height_axis")?)?;
        if length == height {
            return Err(invalid("`length_axis` and `height_axis` must differ"));
        }
        let web = match parameters.string("zone")? {
            None | Some("section") => false,
            Some("web") if height == Axis::ProfileY => true,
            Some("web") => {
                return Err(invalid(
                    "`zone` `web` lies between the flanges, across `profile-y`: it needs \
                     `height_axis` `profile-y`",
                ));
            }
            Some(other) => {
                return Err(invalid(format!(
                    "`zone` `{other}` is unsupported; use `section` or `web`"
                )));
            }
        };
        Ok(Self {
            hosts: Traversal::path(host_path)?,
            host_selector: parameters
                .selector("host_selector")?
                .unwrap_or(&Selector::All),
            length,
            height,
            end_distance: distance(&parameters, "end_distance")?,
            edge_distance: distance(&parameters, "edge_distance")?,
            web,
            spacing: distance(&parameters, "opening_spacing")?,
        })
    }
}

impl RuleCapability for OpeningZone {
    fn id(&self) -> &'static str {
        "axioval:capability.opening-zone"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("host_path", ParameterType::StringList),
            ParameterDescriptor::optional("host_selector", ParameterType::Selector),
            ParameterDescriptor::required("length_axis", ParameterType::String),
            ParameterDescriptor::required("height_axis", ParameterType::String),
            ParameterDescriptor::optional("end_distance", ParameterType::Quantity),
            ParameterDescriptor::optional("edge_distance", ParameterType::Quantity),
            ParameterDescriptor::optional("zone", ParameterType::String),
            ParameterDescriptor::optional("opening_spacing", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("opening-zone: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let openings = Population::of(context, &rule.selector);
        let hosts = Population::of(context, config.host_selector);
        let mut judge = Judge {
            context,
            rule,
            config: &config,
            hosts: &hosts,
            bodies: BTreeMap::new(),
            placed: BTreeMap::new(),
        };
        // Every opening that may be selected is placed, so spacing sees the
        // undecided ones too.
        let candidates: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| openings.contains(&object.id))
            .collect();
        for opening in &candidates {
            let placed = judge.place(opening);
            judge.placed.insert(opening.id.clone(), placed);
        }
        for opening in selected {
            judge.judge(opening, &openings, &mut evaluation);
        }
        evaluation
    }
}

/// A host's face: its section frame, extrusion and bounds.
struct Host {
    origin: Vector,
    axes: [Vector; 3],
    /// Bounds along `Extrusion`, `ProfileX`, `ProfileY`.
    bounds: [(f64, f64); 3],
    /// The part of `ProfileY` between the flanges, where the family has one.
    web: Option<(f64, f64)>,
    family: String,
    evidence: Vec<Evidence>,
}

impl Host {
    fn axis(&self, axis: Axis) -> (Vector, (f64, f64)) {
        let index = match axis {
            Axis::Extrusion => 0,
            Axis::ProfileX => 1,
            Axis::ProfileY => 2,
        };
        (self.axes[index], self.bounds[index])
    }
}

/// An opening's section outline, centred on its position.
#[derive(Clone, Copy)]
enum Shape {
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
}

impl Shape {
    /// The furthest the outline reaches along `(p, q)` in its own frame.
    fn support(self, p: f64, q: f64) -> f64 {
        match self {
            Self::Rectangle { half_x, half_y } => half_x * p.abs() + half_y * q.abs(),
            Self::Rounded {
                half_x,
                half_y,
                radius,
            } => (half_x - radius) * p.abs() + (half_y - radius) * q.abs() + radius * p.hypot(q),
            Self::Circle { radius } => radius * p.hypot(q),
            Self::Ellipse { semi_x, semi_y } => (semi_x * p).hypot(semi_y * q),
        }
    }
}

/// A section frame in world coordinates and the straight extrusion from it.
struct Swept {
    origin: Vector,
    x: Vector,
    y: Vector,
    /// The placement's Z axis, normal to the profile plane.
    normal: Vector,
    direction: Vector,
    depth: f64,
}

/// An opening placed in its host's face.
struct Placed {
    host: ObjectId,
    /// Extents along the length and height axes, from the host's section
    /// origin.
    length: (f64, f64),
    height: (f64, f64),
    /// Whether the face projection is exactly the extents' rectangle.
    exact: bool,
    evidence: Vec<Evidence>,
}

/// Where an opening is: in a checked host, in none, or unknown.
type Placement = Result<Option<Placed>, Unavailable>;

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    config: &'r Config<'r>,
    hosts: &'r Population,
    bodies: BTreeMap<ObjectId, Result<std::rc::Rc<Host>, Unavailable>>,
    placed: BTreeMap<ObjectId, Placement>,
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

fn required(body: &mut BodyFacts<'_>, name: &str) -> Result<f64, Unavailable> {
    body.required_length(&format!("Profile.{name}"))
}

/// Half extents of a host section and the part of its height between the
/// flanges.
type Section = ((f64, f64), Option<(f64, f64)>);

/// Half extents and the web of a host section, centred on its position.
fn host_section(body: &mut BodyFacts<'_>, family: &str) -> Result<Section, Unavailable> {
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
                format!("the host's `{other}` profile states no outline to bound it by"),
            ));
        }
    })
}

fn opening_shape(body: &mut BodyFacts<'_>, family: &str) -> Result<Shape, Unavailable> {
    Ok(match family {
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
        other => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the opening's `{other}` profile has no outline its extent is known for"),
            ));
        }
    })
}

impl Judge<'_, '_> {
    fn host_body(&mut self, host: &ObjectId) -> Result<std::rc::Rc<Host>, Unavailable> {
        if let Some(known) = self.bodies.get(host) {
            return known.clone();
        }
        let read = self.read_host(host).map(std::rc::Rc::new);
        self.bodies.insert(host.clone(), read.clone());
        read
    }

    fn read_host(&self, id: &ObjectId) -> Result<Host, Unavailable> {
        let object = self
            .context
            .project
            .object(id)
            .ok_or_else(|| invalid(format!("host {id} is not in the project")))?;
        let mut body = BodyFacts::of(self.context, object)?;
        let host = (|| {
            let swept = swept(&mut body, "host")?;
            if !parallel(swept.direction, swept.normal) {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    "the host is extruded obliquely to its profile".to_owned(),
                ));
            }
            let family = body
                .text("Profile.Type")?
                .unwrap_or_else(|| "unstated".to_owned());
            let ((half_x, half_y), web) = host_section(&mut body, &family)?;
            Ok(Host {
                origin: swept.origin,
                axes: [swept.direction, swept.x, swept.y],
                bounds: [(0.0, swept.depth), (-half_x, half_x), (-half_y, half_y)],
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

    /// The opening's checked host, `None` when it has none.
    fn host_of(&self, opening: &Object) -> Result<(Option<ObjectId>, Vec<Evidence>), Unavailable> {
        let universe: Vec<&Object> = self
            .context
            .project
            .objects()
            .filter(|object| self.hosts.contains(&object.id))
            .collect();
        let (reached, evidence) =
            self.config
                .hosts
                .related(self.context, &opening.id, &universe)?;
        if let Some(undecided) = reached.iter().find(|id| !self.hosts.matched.contains(*id)) {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("whether {undecided} is a checked host is undecided"),
            ));
        }
        match reached.as_slice() {
            [] => Ok((None, evidence)),
            [host] => Ok((Some(host.clone()), evidence)),
            several => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it has {} hosts ({}); an opening belongs to one",
                    several.len(),
                    several
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        }
    }

    fn place(&mut self, opening: &Object) -> Placement {
        let (host, mut evidence) = self.host_of(opening)?;
        let Some(host) = host else {
            return Ok(None);
        };
        let face = self.host_body(&host)?;
        let mut body = BodyFacts::of(self.context, opening)?;
        let swept = swept(&mut body, "opening")?;
        let family = body
            .text("Profile.Type")?
            .unwrap_or_else(|| "unstated".to_owned());
        let shape = opening_shape(&mut body, &family)?;
        let extent = |axis: Axis| {
            let (w, _) = face.axis(axis);
            let centre = dot(w, minus(swept.origin, face.origin));
            let reach = shape.support(dot(w, swept.x), dot(w, swept.y));
            let sweep = swept.depth * dot(w, swept.direction);
            (
                centre - reach + sweep.min(0.0),
                centre + reach + sweep.max(0.0),
            )
        };
        let (length_axis, _) = face.axis(self.config.length);
        let (height_axis, _) = face.axis(self.config.height);
        let through = [Axis::Extrusion, Axis::ProfileX, Axis::ProfileY]
            .into_iter()
            .find(|axis| *axis != self.config.length && *axis != self.config.height)
            .map(|axis| face.axis(axis).0)
            .unwrap_or_default();
        let aligned = |a: Vector| parallel(a, length_axis) || parallel(a, height_axis);
        let exact = matches!(shape, Shape::Rectangle { .. })
            && aligned(swept.x)
            && aligned(swept.y)
            && parallel(swept.direction, through);
        evidence.extend(body.into_evidence());
        evidence.extend(face.evidence.iter().cloned());
        Ok(Some(Placed {
            host,
            length: extent(self.config.length),
            height: extent(self.config.height),
            exact,
            evidence,
        }))
    }

    fn judge(
        &self,
        opening: &Object,
        openings: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let placed = match self.placed.get(&opening.id) {
            Some(Ok(Some(placed))) => placed,
            Some(Ok(None)) => return,
            Some(Err((reason, message))) => {
                evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                return;
            }
            None => {
                evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "the opening was not placed".to_owned(),
                );
                return;
            }
        };
        // Placing the opening read its host.
        let Some(Ok(host)) = self.bodies.get(&placed.host) else {
            return;
        };
        let mut findings = Vec::new();
        let (_, length_bounds) = host.axis(self.config.length);
        let (_, height_bounds) = host.axis(self.config.height);
        let beyond = |extent: (f64, f64), bounds: (f64, f64)| {
            extent.0 < bounds.0 - ROUNDING || extent.1 > bounds.1 + ROUNDING
        };
        let outside_length = beyond(placed.length, length_bounds);
        let outside_height = beyond(placed.height, height_bounds);
        let outside: Vec<String> = [
            ("length", outside_length, placed.length, length_bounds),
            ("height", outside_height, placed.height, height_bounds),
        ]
        .into_iter()
        .filter(|(_, out, _, _)| *out)
        .map(|(name, _, extent, bounds)| {
            format!(
                "along its {name} it spans {} to {}, the host {} to {}",
                metres(extent.0),
                metres(extent.1),
                metres(bounds.0),
                metres(bounds.1)
            )
        })
        .collect();
        if !outside.is_empty() {
            findings.push(format!(
                "opening lies partly outside its host {}: {}",
                placed.host.local_id,
                outside.join("; ")
            ));
        }
        if let Some(required) = self.config.end_distance
            && !outside_length
        {
            let clear = (placed.length.0 - length_bounds.0).min(length_bounds.1 - placed.length.1);
            if clear < required - ROUNDING {
                findings.push(format!(
                    "opening is {} from an end of its host {}; {} required",
                    metres(clear.max(0.0)),
                    placed.host.local_id,
                    metres(required)
                ));
            }
        }
        if !outside_height && let Err((reason, message)) = self.edges(host, placed, &mut findings) {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        self.spacing(opening, placed, openings, findings, evaluation);
    }

    /// Judges the clearance from the host's edges, or from its flanges.
    fn edges(
        &self,
        host: &Host,
        placed: &Placed,
        findings: &mut Vec<String>,
    ) -> Result<(), Unavailable> {
        if self.config.edge_distance.is_none() && !self.config.web {
            return Ok(());
        }
        let required = self.config.edge_distance.unwrap_or(0.0);
        let (zone, what) = if self.config.web {
            let web = host.web.ok_or_else(|| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "its host {}'s `{}` profile has no web between flanges",
                        placed.host.local_id, host.family
                    ),
                )
            })?;
            (web, "the flanges")
        } else {
            (host.axis(self.config.height).1, "an edge")
        };
        let clear = (placed.height.0 - zone.0).min(zone.1 - placed.height.1);
        if clear < required - ROUNDING {
            let distance = if clear < 0.0 {
                format!("reaches {} into", metres(-clear))
            } else {
                format!("is {} from", metres(clear))
            };
            findings.push(format!(
                "opening {distance} {what} of its host {}; {} clear required",
                placed.host.local_id,
                metres(required)
            ));
        }
        Ok(())
    }

    /// Judges the clear distance to the other openings of the same host and
    /// records every finding of `opening`.
    fn spacing(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        mut findings: Vec<String>,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let mut evidence = placed.evidence.clone();
        let mut related = Vec::new();
        if let Some(required) = self.config.spacing {
            let mut sure = Vec::new();
            let mut unknown = Vec::new();
            for (other, state) in &self.placed {
                if *other == opening.id {
                    continue;
                }
                let neighbour = match state {
                    Ok(Some(neighbour)) if neighbour.host == placed.host => neighbour,
                    Ok(_) => continue,
                    // Its host is unknown: it may be this one's.
                    Err(_) => {
                        unknown.push(other.clone());
                        continue;
                    }
                };
                let gap = |a: (f64, f64), b: (f64, f64)| (b.0 - a.1).max(a.0 - b.1).max(0.0);
                let clear = gap(placed.length, neighbour.length)
                    .hypot(gap(placed.height, neighbour.height));
                if clear >= required - ROUNDING {
                    continue;
                }
                if placed.exact && neighbour.exact && openings.matched.contains(other) {
                    sure.push((other.clone(), clear, neighbour));
                } else {
                    unknown.push(other.clone());
                }
            }
            if sure.is_empty() {
                if !unknown.is_empty() {
                    evaluation.push_object_not_evaluated(
                        opening.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "its clear distance to {} may be under {}: their outlines or \
                             hosts are not known exactly",
                            unknown
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
                            metres(required)
                        ),
                    );
                }
            } else {
                let nearest = sure
                    .iter()
                    .map(|(_, clear, _)| *clear)
                    .fold(f64::INFINITY, f64::min);
                findings.push(format!(
                    "opening is {} clear of another opening in its host {}; {} required",
                    metres(nearest),
                    placed.host.local_id,
                    metres(required)
                ));
                for (other, _, neighbour) in sure {
                    evidence.extend(neighbour.evidence.iter().cloned());
                    related.push(other);
                }
            }
        }
        for message in findings {
            evaluation.push_finding(self.finding(opening, placed, message, &evidence, &related));
        }
    }

    fn finding(
        &self,
        opening: &Object,
        placed: &Placed,
        message: String,
        evidence: &[Evidence],
        related: &[ObjectId],
    ) -> Finding {
        let mut related = related.to_vec();
        related.push(placed.host.clone());
        finding(self.rule, &opening.id, message, evidence.to_vec(), related)
    }
}
