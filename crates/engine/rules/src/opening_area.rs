//! `opening-area`: a wall's stated gross side area less its net side area
//! equals the area of the openings it hosts.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::counts::Population;
use crate::opening_zone::face::{FaceAxes, Host, ROUNDING, Solid, Span, read_host, separation};
use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, finding, invalid, resolve,
};

/// Requires the openings each selected host holds to account for the
/// difference between its stated gross and net side areas: the sum of their
/// areas in its face must equal `gross_area` less `net_area` within
/// `area_tolerance`.
///
/// The openings are what `opening_path` reaches from the host among the
/// `opening_selector` objects (with IFC, `IfcRelVoidsElement` forward). The
/// host and each opening are read from the reserved body set as
/// `opening-zone` reads them, in the face `length_axis` and `height_axis`
/// span. A side area is measured on the host's middle plane, so an opening
/// counts with the exact area of its section when it crosses that plane
/// and not at all when it stops short of it (a recess). An opening's area is
/// known only when it is one straight extrusion through the host of a
/// rectangle, rounded rectangle, circle, ellipse or free polygon (its voids
/// subtracted) lying in the face, wholly within the host's face and clear
/// of the other openings. A host of a free outline must hold the opening
/// inside that outline, not only inside the box around it.
///
/// A host stating neither area is not checked. One stating only one, an
/// opening whose area cannot be placed, or an opening whose selection is
/// undecided leaves the host not evaluated.
pub struct OpeningArea;

struct Config<'a> {
    openings: Traversal<'a>,
    opening_selector: &'a Selector,
    axes: FaceAxes,
    gross: PropertyRef<'a>,
    net: PropertyRef<'a>,
    tolerance: f64,
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let path = parameters
            .strings("opening_path")?
            .ok_or_else(|| invalid("parameter `opening_path` is required"))?;
        let tolerance = match parameters.quantity("area_tolerance")? {
            None => 0.0,
            Some((value, QuantityDimension::Area)) if value >= 0.0 => value,
            Some(_) => return Err(invalid("`area_tolerance` is not a non-negative area")),
        };
        Ok(Self {
            openings: Traversal::path(path)?,
            opening_selector: parameters
                .selector("opening_selector")?
                .unwrap_or(&Selector::All),
            axes: FaceAxes::parse(
                parameters.required_string("length_axis")?,
                parameters.required_string("height_axis")?,
            )?,
            gross: parameters.required_property("gross_area")?,
            net: parameters.required_property("net_area")?,
            tolerance,
        })
    }
}

impl RuleCapability for OpeningArea {
    fn id(&self) -> &'static str {
        "axioval:capability.opening-area"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("opening_path", ParameterType::StringList),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::required("length_axis", ParameterType::String),
            ParameterDescriptor::required("height_axis", ParameterType::String),
            ParameterDescriptor::required("gross_area", ParameterType::PropertyReference),
            ParameterDescriptor::required("net_area", ParameterType::PropertyReference),
            ParameterDescriptor::optional("area_tolerance", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("opening-area: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let openings = Population::of(context, config.opening_selector);
        for host in selected {
            match check(context, &config, &openings, host) {
                Ok(None) => {}
                Ok(Some((message, evidence, related))) => {
                    evaluation.push_finding(finding(rule, &host.id, message, evidence, related));
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(host.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Mismatch = (String, Vec<Evidence>, Vec<ObjectId>);

/// A stated area, `None` when exactly absent.
fn area(
    context: &RuleContext<'_>,
    host: &Object,
    property: PropertyRef<'_>,
    evidence: &mut Vec<Evidence>,
) -> Result<Option<f64>, Unavailable> {
    let resolved = resolve(context, host, property)
        .map_err(|(reason, message)| (reason, format!("`{property}`: {message}")))?;
    evidence.extend(resolved.evidence());
    match resolved.value() {
        None => Ok(None),
        Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Area,
        }) if value.is_finite() => Ok(Some(*value)),
        Some(other) => Err((
            NotEvaluatedReason::InvalidEvidence,
            format!("`{property}` is {}, not an area", display(Some(other))),
        )),
    }
}

fn square_metres(value: f64) -> String {
    format!("{} m²", (value * 1e6).round() / 1e6)
}

/// Checks one host; `Ok(None)` when it passes or is not checked.
fn check(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    openings: &Population,
    host: &Object,
) -> Result<Option<Mismatch>, Unavailable> {
    let mut evidence = Vec::new();
    let gross = area(context, host, config.gross, &mut evidence)?;
    let net = area(context, host, config.net, &mut evidence)?;
    let (gross, net) = match (gross, net) {
        (None, None) => return Ok(None),
        (Some(gross), Some(net)) => (gross, net),
        (Some(_), None) | (None, Some(_)) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it states only one of `{}` and `{}`",
                    config.gross, config.net
                ),
            ));
        }
    };
    let universe: Vec<&Object> = context
        .project
        .objects()
        .filter(|object| openings.contains(&object.id))
        .collect();
    let (reached, cited) = config.openings.related(context, &host.id, &universe)?;
    evidence.extend(cited);
    if let Some(undecided) = reached.iter().find(|id| !openings.matched.contains(*id)) {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("whether {undecided} is one of its openings is undecided"),
        ));
    }
    let face = read_host(context, &host.id)?;
    evidence.extend(face.evidence.iter().cloned());
    let mut placed: Vec<(ObjectId, [Span; 2])> = Vec::new();
    let mut sum = 0.0;
    for id in &reached {
        let object = context
            .project
            .object(id)
            .ok_or_else(|| invalid(format!("opening {id} is not in the project")))?;
        let (area, rectangle) = opening_area(context, config.axes, &face, object, &mut evidence)
            .map_err(|(reason, message)| (reason, format!("opening {id}: {message}")))?;
        if area > 0.0 {
            if let Some((other, _)) = placed
                .iter()
                .find(|(_, other)| separation(rectangle, *other) < -ROUNDING)
            {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "openings {other} and {id} may overlap in its face, so their areas \
                         cannot be summed"
                    ),
                ));
            }
            placed.push((id.clone(), rectangle));
            sum += area;
        }
    }
    let expected = gross - net;
    let slack = config.tolerance + ROUNDING * (1.0 + gross.abs() + net.abs() + sum);
    if (sum - expected).abs() <= slack {
        return Ok(None);
    }
    let described = if reached.is_empty() {
        "it has no openings".to_owned()
    } else {
        format!(
            "its openings ({}) cover {} of its face",
            reached
                .iter()
                .map(|id| id.local_id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            square_metres(sum)
        )
    };
    Ok(Some((
        format!(
            "{described}, but its gross side area {} less its net side area {} is {}; they \
             must agree within {}",
            square_metres(gross),
            square_metres(net),
            square_metres(expected),
            square_metres(config.tolerance)
        ),
        evidence,
        reached,
    )))
}

/// The area an opening takes from its host's middle plane, and its extents
/// in the face.
fn opening_area(
    context: &RuleContext<'_>,
    axes: FaceAxes,
    host: &Host,
    opening: &Object,
    evidence: &mut Vec<Evidence>,
) -> Result<(f64, [Span; 2]), Unavailable> {
    let incomplete = |message: String| (NotEvaluatedReason::IncompleteEvidence, message);
    let solid = Solid::opening(context, opening)?;
    evidence.extend(solid.evidence.iter().cloned());
    let (length_axis, length_bounds) = host.axis(axes.length);
    let (height_axis, height_bounds) = host.axis(axes.height);
    let (through, through_bounds) = host.axis(axes.through());
    if !solid.extruded_along(through) {
        return Err(incomplete(
            "it is not extruded through its host from a section in the face, so its area in \
             the face is not known"
                .to_owned(),
        ));
    }
    let area = solid.section_area().ok_or_else(|| {
        incomplete(format!(
            "its `{}` section has no area known exactly",
            solid.family()
        ))
    })?;
    let length = solid.extent(host.origin, length_axis).outer;
    let height = solid.extent(host.origin, height_axis).outer;
    let within = |extent: Span, bounds: Span| {
        extent.0 >= bounds.0 - ROUNDING && extent.1 <= bounds.1 + ROUNDING
    };
    if !within(length, length_bounds) || !within(height, height_bounds) {
        return Err(incomplete(
            "it reaches past its host's face, so the part of it the side area loses is not \
             known"
                .to_owned(),
        ));
    }
    let depth = solid.extent(host.origin, through).outer;
    if let Some(outline) = &host.outline
        && outline
            .clearance(host.section_rect(axes, length, height, depth), 0)
            .is_none()
    {
        return Err(incomplete(
            "it may reach past its host's outline, so the part of it the side area loses is \
             not known"
                .to_owned(),
        ));
    }
    let middle = f64::midpoint(through_bounds.0, through_bounds.1);
    if depth.1 < middle - ROUNDING || depth.0 > middle + ROUNDING {
        // A recess stopping short of the middle plane takes no side area.
        return Ok((0.0, [length, height]));
    }
    if depth.0 > middle - ROUNDING || depth.1 < middle + ROUNDING {
        return Err(incomplete(
            "it reaches its host's middle plane only within rounding".to_owned(),
        ));
    }
    Ok((area, [length, height]))
}
