//! `opening-zone`: each opening lies within its host and inside the zone
//! the host allows openings in.

use std::collections::BTreeMap;
use std::rc::Rc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, QuantityDimension};

use crate::counts::Population;
use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

mod dimensions;
pub(crate) mod face;
mod measured;
mod outline;
mod supports;
mod zones;

use face::{Axis, FaceAxes, Host, ROUNDING, Solid, Span, gap, read_host};
use supports::{Opening, SupportConfig, Supports};

pub(crate) use measured::PlacementMeasures;

/// Requires each selected opening to lie within its host's face and inside
/// the zone the rule allows: clear of the host's ends by `end_distance`,
/// clear of its edges (or of its flanges, with `zone` `web`) by
/// `edge_distance` and at most `edge_distance_maximum` from the edges
/// `maximum_edges` names (`top`, `bottom` or `both`, the high and low ends
/// of `height_axis`; `both` by default), `opening_spacing` clear of every
/// other opening in the
/// same host, `support_distance` along the host from each of its supports
/// (with `support_distance_ratio`, at least that fraction of the host's
/// `span` or `depth`, `support_distance_reference`) and `support_clearance` clear of their footprints in the face, and
/// inside one of the allowed `zones`: rows of insets from the ends, the
/// bottom and the top, each the larger of a fraction of the host's span or
/// depth and a minimum length; and the distances the `dimensions` table
/// bounds, from an opening to the nearest other opening of the host or to
/// one of its edges.
///
/// The host is what `host_path` reaches from the opening among the
/// `host_selector` objects (with IFC, `IfcRelVoidsElement` backward). An
/// opening reaching no such host is not checked; one reaching an object
/// whose selection is undecided is not evaluated. An element reaching
/// several hosts is placed and judged in each: with the derived
/// `axioval:derived.intersects`, a duct or pipe with no void modelled is
/// judged in every beam or wall it passes through.
///
/// Both bodies are read from the reserved body set (`axioval:body`), never
/// from a mesh: the host must be one straight extrusion perpendicular to its
/// profile, of a profile family whose outline the set bounds (rectangles,
/// circles, ellipses and the I, T, U, C, Z and L sections, centred on their
/// position, and free outlines of straight edges, with or without voids);
/// the opening one straight extrusion of a rectangle, rounded rectangle,
/// circle, ellipse or free outline. The host's axes are named by the rule:
/// `length_axis` and `height_axis` are two of `extrusion`, `profile-x` and
/// `profile-y` (a beam: `extrusion` and `profile-y`; a wall extruded up
/// from its plan outline: `profile-x` and `extrusion`), and the third runs
/// through the host. The opening's extent along each face axis is exact:
/// the support of its section swept along its extrusion. Anything else is
/// not evaluated, never approximated.
///
/// A host of a free outline (a wall with a mitred end) has bounds that
/// differ across it. Where the rule's face crosses the outline, the opening
/// must lie inside the outline over the whole depth it passes through the
/// host, and its distance from the host's ends and edges is the least over
/// that depth, measured to the outline itself. Both are exact when the
/// opening is extruded straight through the host; otherwise only the box
/// its extents span is known there, which can pass an opening but never
/// find one. An outline with a curved edge is not evaluated.
///
/// Distances between openings are clear distances in the host's face.
/// Between two rectangles whose sides run along the face axes and which are
/// extruded through the host they are exact; between any other pair only
/// the distance of their extents is known, a lower bound, which can pass a
/// pair but never find one.
///
/// A host's supports are the members it rests on or that connect to it:
/// what `support_path` reaches from the host (with IFC,
/// `IfcRelConnectsElements:either`, which takes in
/// `IfcRelConnectsPathElements`), and the objects that come within
/// `support_gap` of the host in space, both among the `support_selector`
/// objects. Each support must be one straight extrusion the body set bounds;
/// its extent along a face axis is an interval sure to hold the true one,
/// and one sure to lie within it, the same where the outline is exact. A
/// finding needs the inner interval, a pass the outer one; in between, or
/// with a support whose relation or selection is undecided and which may
/// come too close, the opening is not evaluated.
///
/// An opening whose area in the face is below `minimum_opening_area` is
/// ignored: not judged, and no neighbour or target of another. Its area is
/// its section's where it is extruded through the host, and otherwise known
/// small only when the box its extents span is; an opening that may be
/// either is not evaluated, and a possible neighbour.
///
/// Positions are composed from placements in binary arithmetic, so every
/// bound is widened by a nanometre, far below any modelling tolerance.
pub struct OpeningZone;

struct Config<'a> {
    hosts: Traversal,
    host_selector: &'a Selector,
    axes: FaceAxes,
    end_distance: Option<f64>,
    edge_distance: Option<f64>,
    /// The largest distance allowed from the low and the high edge.
    edge_maximum: Option<(f64, bool, bool)>,
    web: bool,
    spacing: Option<f64>,
    supports: Option<SupportConfig<'a>>,
    zones: Vec<zones::Zone>,
    dimensions: Vec<dimensions::Dimension<'a>>,
    minimum_area: Option<f64>,
}

pub(crate) fn distance(
    parameters: &Parameters<'_>,
    name: &str,
) -> Result<Option<f64>, Unavailable> {
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
        let axes = FaceAxes::parse(
            parameters.required_string("length_axis")?,
            parameters.required_string("height_axis")?,
        )?;
        let web = match parameters.string("zone")? {
            None | Some("section") => false,
            Some("web") if axes.height == Axis::ProfileY => true,
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
            axes,
            end_distance: distance(&parameters, "end_distance")?,
            edge_distance: distance(&parameters, "edge_distance")?,
            edge_maximum: match (
                distance(&parameters, "edge_distance_maximum")?,
                parameters.string("maximum_edges")?,
            ) {
                (None, None) => None,
                (None, Some(_)) => {
                    return Err(invalid("`maximum_edges` needs `edge_distance_maximum`"));
                }
                (Some(maximum), None | Some("both")) => Some((maximum, true, true)),
                (Some(maximum), Some("bottom")) => Some((maximum, true, false)),
                (Some(maximum), Some("top")) => Some((maximum, false, true)),
                (Some(_), Some(other)) => {
                    return Err(invalid(format!(
                        "`maximum_edges` `{other}` is unsupported; use `top`, `bottom` or `both`"
                    )));
                }
            },
            web,
            spacing: distance(&parameters, "opening_spacing")?,
            supports: SupportConfig::parse(&parameters)?,
            zones: zones::parse(&parameters)?,
            dimensions: dimensions::parse(&parameters)?,
            minimum_area: crate::opening_area::minimum_area(&parameters)?,
        })
    }
}

impl RuleCapability for OpeningZone {
    fn id(&self) -> &'static str {
        "axioval:capability.opening-zone"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("host_path", ParameterType::StringList),
            ParameterDescriptor::optional("host_selector", ParameterType::Selector),
            ParameterDescriptor::required("length_axis", ParameterType::String),
            ParameterDescriptor::required("height_axis", ParameterType::String),
            ParameterDescriptor::optional("end_distance", ParameterType::Quantity),
            ParameterDescriptor::optional("edge_distance", ParameterType::Quantity),
            ParameterDescriptor::optional("edge_distance_maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum_edges", ParameterType::String),
            ParameterDescriptor::optional("zone", ParameterType::String),
            ParameterDescriptor::optional("opening_spacing", ParameterType::Quantity),
            ParameterDescriptor::optional("zones", ParameterType::Table(zones::COLUMNS)),
            ParameterDescriptor::optional("minimum_opening_area", ParameterType::Quantity),
            ParameterDescriptor::optional("dimensions", ParameterType::Table(dimensions::COLUMNS)),
        ];
        parameters.extend(SupportConfig::parameters());
        parameters
    }

    fn grades_deviation(&self) -> bool {
        true
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
        let support_population = config
            .supports
            .as_ref()
            .map(|supports| Population::of(context, supports.selector));
        let dimension_selections = dimensions::Selections::of(context, &config.dimensions);
        let mut judge = Judge {
            dimensions: dimension_selections,
            context,
            rule,
            config: &config,
            hosts: &hosts,
            bodies: BTreeMap::new(),
            placed: BTreeMap::new(),
            supports: support_population
                .as_ref()
                .zip(config.supports.as_ref())
                .map(|(population, config)| Supports::new(context, config, population)),
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

/// An opening placed in one of its hosts' faces.
struct Placed {
    host: ObjectId,
    /// Extents along the length and height axes, from the host's section
    /// origin.
    length: Span,
    height: Span,
    /// Its extent across the host, from the host's section origin.
    through: Span,
    /// Whether the face projection is exactly the extents' rectangle.
    exact: bool,
    /// Whether the rectangle [`Host::section_rect`] builds is exactly
    /// where the opening lies in the host's section plane: it is extruded
    /// through a host whose face holds the extrusion, or it is `exact`.
    section_exact: bool,
    /// Whether its area may be under `minimum_opening_area` or not.
    may_be_small: bool,
    evidence: Vec<Evidence>,
}

impl Placed {
    fn section_rect(&self, host: &Host, axes: FaceAxes) -> [Span; 2] {
        host.section_rect(axes, self.length, self.height, self.through)
    }
}

/// A finding's message and, where it misses a bound, by how far.
type Found = (String, Option<Deviation>);

/// An opening's clear distance from its host's edges or flanges.
struct Clearance {
    /// The distance, negative where the opening reaches into a flange.
    clear: f64,
    /// Whether it is exact rather than a lower bound.
    exact: bool,
    /// Whether it is measured to a free outline.
    outline: bool,
    /// How a finding names what it is measured to.
    what: &'static str,
}

/// Where an opening is: in each checked host it reaches (none when it
/// reaches none), each placement known or not; unknown altogether when its
/// hosts are.
type Placement = Result<Vec<Result<Placed, Unavailable>>, Unavailable>;

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    config: &'r Config<'r>,
    hosts: &'r Population,
    bodies: BTreeMap<ObjectId, Result<Rc<Host>, Unavailable>>,
    placed: BTreeMap<ObjectId, Placement>,
    supports: Option<Supports<'r, 'c>>,
    /// The selections of each row of the dimensioning table.
    dimensions: dimensions::Selections,
}

/// The hosts `traversal` reaches from `object` among `hosts`, none when it
/// reaches none.
pub(crate) fn hosts_of(
    context: &RuleContext<'_>,
    traversal: &Traversal,
    hosts: &Population,
    object: &Object,
) -> Result<(Vec<ObjectId>, Vec<Evidence>), Unavailable> {
    let universe: Vec<&Object> = context
        .project
        .objects()
        .filter(|candidate| hosts.contains(&candidate.id))
        .collect();
    let (reached, evidence) = traversal.related(context, &object.id, &universe)?;
    if let Some(undecided) = reached.iter().find(|id| !hosts.matched.contains(*id)) {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("whether {undecided} is a checked host is undecided"),
        ));
    }
    Ok((reached, evidence))
}

pub(crate) fn list(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl Judge<'_, '_> {
    fn host_body(&mut self, host: &ObjectId) -> Result<Rc<Host>, Unavailable> {
        if let Some(known) = self.bodies.get(host) {
            return known.clone();
        }
        let read = read_host(self.context, host).map(Rc::new);
        self.bodies.insert(host.clone(), read.clone());
        read
    }

    fn place(&mut self, opening: &Object) -> Placement {
        let (hosts, evidence) = hosts_of(self.context, &self.config.hosts, self.hosts, opening)?;
        if hosts.is_empty() {
            return Ok(Vec::new());
        }
        let solid = Solid::opening(self.context, opening).map(Rc::new);
        Ok(hosts
            .into_iter()
            .map(|host| {
                let solid = solid.clone()?;
                self.place_in(host, &solid, evidence.clone())
            })
            // An opening surely below the minimum area is left out.
            .filter_map(|placed| match placed {
                Ok(None) => None,
                Ok(Some(placed)) => Some(Ok(placed)),
                Err(error) => Some(Err(error)),
            })
            .collect())
    }

    /// Places an opening's solid in one host's face; `None` when its area
    /// there is surely below the minimum.
    fn place_in(
        &mut self,
        host: ObjectId,
        solid: &Solid,
        mut evidence: Vec<Evidence>,
    ) -> Result<Option<Placed>, Unavailable> {
        let face = self.host_body(&host)?;
        let (length_axis, _) = face.axis(self.config.axes.length);
        let (height_axis, _) = face.axis(self.config.axes.height);
        let (through, _) = face.axis(self.config.axes.through());
        let exact =
            solid.aligned_rectangle(length_axis, height_axis) && solid.direction_along(through);
        let length = solid.extent(face.origin, length_axis).outer;
        let height = solid.extent(face.origin, height_axis).outer;
        let across = solid.extent(face.origin, through).outer;
        // Across a host extruded in its face, an opening extruded through
        // it keeps one extent along the face's profile axis at every depth.
        let section_exact = exact
            || (self.config.axes.through() != Axis::Extrusion && solid.direction_along(through));
        let mut may_be_small = false;
        if let Some(minimum) = self.config.minimum_area {
            let below = |area: f64| area < minimum - ROUNDING;
            // Its area in the face: its section's when extruded through the
            // host, otherwise at most the box its extents span.
            let area = solid
                .section_area()
                .filter(|_| solid.extruded_along(through));
            match area {
                Some(area) if below(area) => return Ok(None),
                Some(_) => {}
                None if below((length.1 - length.0) * (height.1 - height.0)) => return Ok(None),
                None => may_be_small = true,
            }
        }
        evidence.extend(solid.evidence.iter().cloned());
        evidence.extend(face.evidence.iter().cloned());
        Ok(Some(Placed {
            host,
            length,
            height,
            through: across,
            exact,
            section_exact,
            may_be_small,
            evidence,
        }))
    }

    fn judge(
        &self,
        opening: &Object,
        openings: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let placements = match self.placed.get(&opening.id) {
            Some(Ok(placements)) => placements,
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
        for placed in placements {
            match placed {
                Ok(placed) if placed.may_be_small => evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "whether its area in its host {} is below `minimum_opening_area` is \
                         undecided: it is not extruded through the host",
                        placed.host.local_id
                    ),
                ),
                Ok(placed) => self.judge_in(opening, placed, openings, evaluation),
                Err((reason, message)) => evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    reason.clone(),
                    message.clone(),
                ),
            }
        }
    }

    /// Judges an opening placed in one of its hosts.
    fn judge_in(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) {
        // Placing the opening read its host.
        let Some(Ok(host)) = self.bodies.get(&placed.host).cloned() else {
            return;
        };
        let mut findings = Vec::new();
        let (_, length_bounds) = host.axis(self.config.axes.length);
        let (_, height_bounds) = host.axis(self.config.axes.height);
        let (outside_length, outside_height) = self.beyond(&host, placed);
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
        let axes = self.config.axes;
        let rect = placed.section_rect(&host, axes);
        let mut crosses_outline = false;
        if outside.is_empty() {
            crosses_outline = Self::crosses_outline(&host, placed, rect, &mut findings)
                .unwrap_or_else(|(reason, message)| {
                    evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                    true
                });
        } else {
            findings.push((
                format!(
                    "opening lies partly outside its host {}: {}",
                    placed.host.local_id,
                    outside.join("; ")
                ),
                None,
            ));
        }
        if !outside_length
            && !crosses_outline
            && let Err((reason, message)) = self.ends(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_height
            && !crosses_outline
            && let Err((reason, message)) = self.edges(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_height
            && !crosses_outline
            && let Err((reason, message)) = self.far_edges(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_length
            && !outside_height
            && !crosses_outline
            && let Err((reason, message)) = self.zones(&host, placed, rect, &mut findings)
        {
            evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
        }
        if !outside_length && !outside_height && !crosses_outline {
            self.dimensions(opening, placed, &host, rect, evaluation);
        }
        if let Some(supports) = &self.supports {
            let face = Opening {
                host: &placed.host,
                length: placed.length,
                height: placed.height,
                exact: placed.exact,
            };
            for (message, evidence, related) in
                supports.judge(&opening.id, &face, &host, self.config.axes, evaluation)
            {
                let mut cited = placed.evidence.clone();
                cited.extend(evidence);
                evaluation.push_finding(self.finding(opening, placed, message, &cited, &related));
            }
        }
        self.spacing(opening, placed, openings, findings, evaluation);
    }

    /// Whether the opening reaches past its host's box along its length and
    /// along its height.
    fn beyond(&self, host: &Host, placed: &Placed) -> (bool, bool) {
        let (_, length_bounds) = host.axis(self.config.axes.length);
        let (_, height_bounds) = host.axis(self.config.axes.height);
        let beyond = |extent: Span, bounds: Span| {
            extent.0 < bounds.0 - ROUNDING || extent.1 > bounds.1 + ROUNDING
        };
        (
            beyond(placed.length, length_bounds),
            beyond(placed.height, height_bounds),
        )
    }

    /// Within the box that holds a free outline, whether the opening
    /// crosses the outline itself: `None` when it may, its place in the
    /// host's section known only within bounds.
    fn crossing(host: &Host, placed: &Placed, rect: [Span; 2]) -> Option<bool> {
        match &host.outline {
            None => Some(false),
            Some(outline) if outline.clearance(rect, 0).is_some() => Some(false),
            Some(_) => placed.section_exact.then_some(true),
        }
    }

    /// Within the box that holds a free outline, whether the opening
    /// crosses the outline itself; an error when it may.
    fn crosses_outline(
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<bool, Unavailable> {
        let Some(crosses) = Self::crossing(host, placed, rect) else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it may cross the edge of its host {}'s outline: it is not extruded \
                     straight through the host, so where it lies in the host's section is \
                     known only within bounds",
                    placed.host.local_id
                ),
            ));
        };
        if !crosses {
            return Ok(false);
        }
        findings.push((
            format!(
                "opening lies partly outside its host {}: it crosses the edge of the host's \
                 outline",
                placed.host.local_id
            ),
            None,
        ));
        Ok(true)
    }

    /// Judges the distance from the host's ends.
    fn ends(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        let Some(required) = self.config.end_distance else {
            return Ok(());
        };
        let Some((clear, exact)) = self.end_clearance(host, placed, rect) else {
            return Ok(());
        };
        if clear >= required - ROUNDING {
            return Ok(());
        }
        if !exact {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "its distance from an end of its host {}'s outline may be under {}: where \
                     it lies in the host's section is known only within bounds",
                    placed.host.local_id,
                    metres(required)
                ),
            ));
        }
        findings.push((
            format!(
                "opening is {} from an end of its host {}; {} required",
                metres(clear.max(0.0)),
                placed.host.local_id,
                metres(required)
            ),
            Some(Deviation::below(required, clear, clear)),
        ));
        Ok(())
    }

    /// The opening's clear distance from the nearer end of its host, and
    /// whether it is exact rather than a lower bound; none where the host
    /// has no ends along its length.
    fn end_clearance(&self, host: &Host, placed: &Placed, rect: [Span; 2]) -> Option<(f64, bool)> {
        let length = self.config.axes.length;
        let (low, high) = host.clearance(length, placed.length, rect)?;
        Some((
            low.min(high),
            placed.section_exact || !host.outlined(length),
        ))
    }

    /// The opening's clear distance from its host's nearer edge (or, with
    /// `zone` `web`, flange), negative where it reaches into a flange;
    /// whether it is exact rather than a lower bound; and whether it is
    /// measured to a free outline. None where the outline bounds no edge
    /// across it.
    fn edge_clearance(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
    ) -> Result<Option<Clearance>, Unavailable> {
        let height = self.config.axes.height;
        if !self.config.web && host.outlined(height) {
            // Across a free outline, the edges are where the outline is.
            return Ok(host
                .clearance(height, placed.height, rect)
                .map(|(low, high)| Clearance {
                    clear: low.min(high),
                    exact: placed.section_exact,
                    outline: true,
                    what: "an edge",
                }));
        }
        let (zone, what) = if self.config.web {
            (Self::web(host, placed)?, "the flanges")
        } else {
            (host.axis(height).1, "an edge")
        };
        Ok(Some(Clearance {
            clear: (placed.height.0 - zone.0).min(zone.1 - placed.height.1),
            exact: true,
            outline: false,
            what,
        }))
    }

    /// The web zone of the host, refused for a profile without one.
    fn web(host: &Host, placed: &Placed) -> Result<Span, Unavailable> {
        host.web.ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "its host {}'s `{}` profile has no web between flanges",
                    placed.host.local_id, host.family
                ),
            )
        })
    }

    /// The opening's distances from its host's low and high edges (or, with
    /// `zone` `web`, flanges), and whether they are exact rather than lower
    /// bounds; none where the outline bounds no edge across it.
    fn far_clearance(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
    ) -> Result<Option<(Span, bool)>, Unavailable> {
        if self.config.web {
            let web = Self::web(host, placed)?;
            return Ok(Some((
                (placed.height.0 - web.0, web.1 - placed.height.1),
                true,
            )));
        }
        let height = self.config.axes.height;
        Ok(host
            .clearance(height, placed.height, rect)
            .map(|clear| (clear, placed.section_exact || !host.outlined(height))))
    }

    /// Judges the clearance from the host's edges, or from its flanges.
    fn edges(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        if self.config.edge_distance.is_none() && !self.config.web {
            return Ok(());
        }
        let required = self.config.edge_distance.unwrap_or(0.0);
        let Some(Clearance {
            clear,
            exact,
            outline,
            what,
        }) = self.edge_clearance(host, placed, rect)?
        else {
            return Ok(());
        };
        if clear >= required - ROUNDING {
            return Ok(());
        }
        if outline {
            if !exact {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "its distance from an edge of its host {}'s outline may be under {}: \
                         where it lies in the host's section is known only within bounds",
                        placed.host.local_id,
                        metres(required)
                    ),
                ));
            }
            findings.push((
                format!(
                    "opening is {} from an edge of its host {}; {} clear required",
                    metres(clear.max(0.0)),
                    placed.host.local_id,
                    metres(required)
                ),
                Some(Deviation::below(required, clear, clear)),
            ));
            return Ok(());
        }
        let distance = if clear < 0.0 {
            format!("reaches {} into", metres(-clear))
        } else {
            format!("is {} from", metres(clear))
        };
        findings.push((
            format!(
                "opening {distance} {what} of its host {}; {} clear required",
                placed.host.local_id,
                metres(required)
            ),
            Some(Deviation::below(required, clear, clear)),
        ));
        Ok(())
    }

    /// Judges the largest distance allowed from the host's low and high
    /// edges (or flanges, with `zone` `web`). A distance measured to a free
    /// outline from the box the opening may lie in is a lower bound, which can
    /// find an opening too far but never pass one.
    fn far_edges(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
        findings: &mut Vec<Found>,
    ) -> Result<(), Unavailable> {
        let Some((maximum, low, high)) = self.config.edge_maximum else {
            return Ok(());
        };
        let Some((clear, exact)) = self.far_clearance(host, placed, rect)? else {
            return Ok(());
        };
        let what = if self.config.web {
            ["the lower flange", "the upper flange"]
        } else {
            ["the bottom edge", "the top edge"]
        };
        let mut open = Vec::new();
        for (checked, distance, edge) in [(low, clear.0, what[0]), (high, clear.1, what[1])] {
            if !checked {
                continue;
            }
            // The distance is exact, or a lower bound of the true one.
            if distance > maximum + ROUNDING {
                // A lower bound over the maximum may be farther still.
                let upper = if exact { distance } else { f64::INFINITY };
                findings.push((
                    format!(
                        "opening is {} from {edge} of its host {}; at most {} allowed",
                        metres(distance),
                        placed.host.local_id,
                        metres(maximum)
                    ),
                    Some(Deviation::above(maximum, distance, upper)),
                ));
            } else if !exact {
                open.push(edge);
            }
        }
        if open.is_empty() {
            return Ok(());
        }
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "its distance from {} of its host {}'s outline may exceed {}: where it lies \
                 in the host's section is known only within bounds",
                open.join(" and "),
                placed.host.local_id,
                metres(maximum)
            ),
        ))
    }

    /// The other openings that are or may be placed in `placed`'s host:
    /// each placement there, or `None` for one whose placement is unknown.
    fn neighbours<'p>(
        &'p self,
        opening: &Object,
        placed: &Placed,
    ) -> Vec<(&'p ObjectId, Option<&'p Placed>)> {
        let mut neighbours = Vec::new();
        for (other, state) in &self.placed {
            if *other == opening.id {
                continue;
            }
            match state {
                Ok(placements) => {
                    for placement in placements {
                        match placement {
                            // It may be too small to count.
                            Ok(neighbour) if neighbour.host == placed.host => neighbours
                                .push((other, (!neighbour.may_be_small).then_some(neighbour))),
                            Ok(_) => {}
                            Err(_) => neighbours.push((other, None)),
                        }
                    }
                }
                Err(_) => neighbours.push((other, None)),
            }
        }
        neighbours
    }

    /// Judges the clear distance to the other openings of the same host and
    /// records every finding of `opening`.
    fn spacing(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        mut findings: Vec<Found>,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let mut evidence = placed.evidence.clone();
        let mut related = Vec::new();
        if let Some(required) = self.config.spacing {
            let mut sure = Vec::new();
            let mut unknown = Vec::new();
            for (other, neighbour) in self.neighbours(opening, placed) {
                // Its host is unknown: it may be this one's.
                let Some(neighbour) = neighbour else {
                    unknown.push(other.clone());
                    continue;
                };
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
                            list(&unknown),
                            metres(required)
                        ),
                    );
                }
            } else {
                let nearest = sure
                    .iter()
                    .map(|(_, clear, _)| *clear)
                    .fold(f64::INFINITY, f64::min);
                findings.push((
                    format!(
                        "opening is {} clear of another opening in its host {}; {} required",
                        metres(nearest),
                        placed.host.local_id,
                        metres(required)
                    ),
                    Some(Deviation::below(required, nearest, nearest)),
                ));
                for (other, _, neighbour) in sure {
                    evidence.extend(neighbour.evidence.iter().cloned());
                    related.push(other);
                }
            }
        }
        for (message, deviation) in findings {
            evaluation.push_finding_deviating(
                self.finding(opening, placed, message, &evidence, &related),
                deviation,
            );
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
