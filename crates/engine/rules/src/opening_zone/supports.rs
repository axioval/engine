//! A host's supports and connecting members, and where they lie in its
//! face.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use axioval_engine::{
    CapabilityEvaluation, NotEvaluatedReason, ParameterDescriptor, ParameterType, ProximityError,
    ProximityRequest, ProximityServiceHandle, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use super::distance;
use super::face::{Axis, Extent, FaceAxes, Host, ROUNDING, Solid, Span, gap, separation};
use crate::counts::Population;
use crate::level_spacing::{metres, shown};
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// How a rule finds a host's supports and what it requires of them.
pub(super) struct SupportConfig<'a> {
    path: Option<Traversal>,
    pub(super) selector: &'a Selector,
    contact: Option<f64>,
    distance: Option<f64>,
    /// The distance as a fraction of the host's span or depth.
    ratio: Option<(f64, Reference)>,
    clearance: Option<f64>,
}

/// The host extent a support distance ratio multiplies.
#[derive(Clone, Copy)]
enum Reference {
    /// Along the face's length axis: the host's length.
    Span,
    /// Along the face's height axis: the host's depth.
    Depth,
}

impl Reference {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "span" => Ok(Self::Span),
            "depth" => Ok(Self::Depth),
            other => Err(invalid(format!(
                "`support_distance_reference` `{other}` is unsupported; use `span` or `depth`"
            ))),
        }
    }

    fn axis(self, axes: FaceAxes) -> Axis {
        match self {
            Self::Span => axes.length,
            Self::Depth => axes.height,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Span => "span",
            Self::Depth => "depth",
        }
    }
}

/// The distance an opening must keep from each support of one host, as an
/// interval sure to hold the exact requirement, and how a message words it.
struct Required {
    low: f64,
    high: f64,
    text: String,
}

impl<'a> SupportConfig<'a> {
    pub(super) fn parameters() -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("support_path", ParameterType::StringList),
            ParameterDescriptor::optional("support_selector", ParameterType::Selector),
            ParameterDescriptor::optional("support_gap", ParameterType::Quantity),
            ParameterDescriptor::optional("support_distance", ParameterType::Quantity),
            ParameterDescriptor::optional("support_distance_ratio", ParameterType::Number),
            ParameterDescriptor::optional("support_distance_reference", ParameterType::String),
            ParameterDescriptor::optional("support_clearance", ParameterType::Quantity),
        ]
    }

    /// The support declaration, `None` when the rule makes none.
    pub(super) fn parse(parameters: &Parameters<'a>) -> Result<Option<Self>, Unavailable> {
        let path = parameters
            .strings("support_path")?
            .map(Traversal::path)
            .transpose()?;
        let selector = parameters.selector("support_selector")?;
        let contact = distance(parameters, "support_gap")?;
        let required = distance(parameters, "support_distance")?;
        let clearance = distance(parameters, "support_clearance")?;
        let ratio = match (
            parameters.number("support_distance_ratio")?,
            parameters.string("support_distance_reference")?,
        ) {
            (None, None) => None,
            (Some(ratio), Some(reference)) if ratio.is_finite() && ratio > 0.0 => {
                Some((ratio, Reference::parse(reference)?))
            }
            (Some(_), Some(_)) => {
                return Err(invalid("`support_distance_ratio` must be positive"));
            }
            _ => {
                return Err(invalid(
                    "`support_distance_ratio` and `support_distance_reference` go together",
                ));
            }
        };
        let checks = required.is_some() || ratio.is_some() || clearance.is_some();
        let found = path.is_some() || contact.is_some();
        match (checks, found) {
            (false, false) if selector.is_none() => Ok(None),
            (true, true) => Ok(Some(Self {
                path,
                selector: selector.unwrap_or(&Selector::All),
                contact,
                distance: required,
                ratio,
                clearance,
            })),
            (true, false) => Err(invalid(
                "`support_distance` and `support_clearance` need `support_path` or \
                 `support_gap` to find the supports",
            )),
            (false, _) => Err(invalid(
                "`support_path`, `support_gap` and `support_selector` find supports only for \
                 `support_distance` or `support_clearance`",
            )),
        }
    }
}

impl SupportConfig<'_> {
    /// The distance an opening must keep from the supports of `host`: the
    /// fixed `support_distance`, or the larger of it and the ratio times
    /// the host's extent along the reference axis, widened by a rounding
    /// step each way wherever the arithmetic rounded. `None` without either.
    fn required(&self, host: &Host, axes: FaceAxes) -> Option<Required> {
        let Some((ratio, reference)) = self.ratio else {
            return self.distance.map(|fixed| Required {
                low: fixed,
                high: fixed,
                text: metres(fixed),
            });
        };
        let (_, bounds) = host.axis(reference.axis(axes));
        let extent = bounds.1 - bounds.0;
        let floor = self.distance.unwrap_or(0.0);
        let low = floor.max((extent.next_down() * ratio).next_down().max(0.0));
        let high = floor.max((extent.next_up() * ratio).next_up());
        let scaled = format!(
            "{ratio} × the host's {} {}",
            reference.noun(),
            metres(extent)
        );
        let why = match self.distance {
            Some(fixed) => format!("the larger of {} and {scaled}", metres(fixed)),
            None => scaled,
        };
        Some(Required {
            low,
            high,
            text: format!("{} ({why})", shown(low, high)),
        })
    }
}

/// A member found beside a host.
struct Member {
    id: ObjectId,
    /// Whether it surely supports the host and is surely selected.
    decided: bool,
}

/// A host's supports, with the evidence that found them.
struct Found {
    members: Vec<Member>,
    evidence: Vec<Evidence>,
}

/// Whether a candidate touches the host.
enum Contact {
    Apart,
    Touching(Evidence),
    Undecided,
}

/// A member's footprint in the host's face: its extents along the length
/// and height, and a rectangle sure to lie within its projection where one
/// is known.
struct Footprint {
    length: Extent,
    height: Extent,
    inner: Option<[Span; 2]>,
}

impl Footprint {
    /// The member's projection onto the face. It is a rectangle holding
    /// the inner one when the member is extruded along a face axis from a
    /// section across it (its section projects onto the other axis as an
    /// interval holding the inner extent), or through the host from a
    /// rectangle whose sides run along the face axes.
    fn of(solid: &Solid, host: &Host, axes: FaceAxes) -> Self {
        let (length_axis, _) = host.axis(axes.length);
        let (height_axis, _) = host.axis(axes.height);
        let (through, _) = host.axis(axes.through());
        let length = solid.extent(host.origin, length_axis);
        let height = solid.extent(host.origin, height_axis);
        let rectangular = solid.extruded_along(length_axis)
            || solid.extruded_along(height_axis)
            || (solid.extruded_along(through) && solid.aligned_rectangle(length_axis, height_axis));
        Self {
            length,
            height,
            inner: rectangular.then_some([length.inner, height.inner]),
        }
    }

    fn outer(&self) -> [Span; 2] {
        [self.length.outer, self.height.outer]
    }
}

/// An opening in a host's face.
pub(super) struct Opening<'p> {
    pub(super) host: &'p ObjectId,
    pub(super) length: Span,
    pub(super) height: Span,
    /// Whether its projection is exactly its extents' rectangle.
    pub(super) exact: bool,
}

/// A finding on an opening: its message, evidence and related objects.
pub(super) type SupportFinding = (String, Vec<Evidence>, Vec<ObjectId>);

/// Finds and reads the supports of each host once.
pub(super) struct Supports<'r, 'c> {
    context: &'r RuleContext<'c>,
    config: &'r SupportConfig<'r>,
    population: &'r Population,
    found: RefCell<BTreeMap<ObjectId, Result<Rc<Found>, Unavailable>>>,
    solids: RefCell<BTreeMap<ObjectId, Result<Rc<Solid>, Unavailable>>>,
}

impl<'r, 'c> Supports<'r, 'c> {
    pub(super) fn new(
        context: &'r RuleContext<'c>,
        config: &'r SupportConfig<'r>,
        population: &'r Population,
    ) -> Self {
        Self {
            context,
            config,
            population,
            found: RefCell::new(BTreeMap::new()),
            solids: RefCell::new(BTreeMap::new()),
        }
    }

    fn found(&self, host: &ObjectId) -> Result<Rc<Found>, Unavailable> {
        if let Some(known) = self.found.borrow().get(host) {
            return known.clone();
        }
        let read = self.find(host).map(Rc::new);
        self.found.borrow_mut().insert(host.clone(), read.clone());
        read
    }

    fn find(&self, host: &ObjectId) -> Result<Found, Unavailable> {
        let universe: Vec<&Object> = self
            .context
            .project
            .objects()
            .filter(|object| object.id != *host && self.population.contains(&object.id))
            .collect();
        // Whether each member surely supports the host.
        let mut members: BTreeMap<ObjectId, bool> = BTreeMap::new();
        let mut evidence = Vec::new();
        if let Some(path) = &self.config.path {
            let (reached, cited) = path.related(self.context, host, &universe)?;
            evidence.extend(cited);
            members.extend(reached.into_iter().map(|id| (id, true)));
        }
        if let Some(tolerance) = self.config.contact {
            let service = self
                .context
                .services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::MissingService,
                        "proximity service is not registered; `support_gap` measures contact"
                            .to_owned(),
                    )
                })?;
            let host_bounds = service.bounds(host).ok();
            for candidate in &universe {
                if members.get(&candidate.id) == Some(&true) {
                    continue;
                }
                match contact(
                    service,
                    host,
                    host_bounds.as_ref(),
                    &candidate.id,
                    tolerance,
                ) {
                    Contact::Apart => {}
                    Contact::Touching(cited) => {
                        evidence.push(cited);
                        members.insert(candidate.id.clone(), true);
                    }
                    Contact::Undecided => {
                        members.entry(candidate.id.clone()).or_insert(false);
                    }
                }
            }
        }
        Ok(Found {
            members: members
                .into_iter()
                .map(|(id, sure)| Member {
                    decided: sure && self.population.matched.contains(&id),
                    id,
                })
                .collect(),
            evidence,
        })
    }

    fn solid(&self, id: &ObjectId) -> Result<Rc<Solid>, Unavailable> {
        if let Some(known) = self.solids.borrow().get(id) {
            return known.clone();
        }
        let read = self
            .context
            .project
            .object(id)
            .ok_or_else(|| invalid(format!("{id} is not in the project")))
            .and_then(|object| Solid::member(self.context, object, "support"))
            .map(Rc::new);
        self.solids.borrow_mut().insert(id.clone(), read.clone());
        read
    }

    /// Judges an opening against the supports of its host: its distance
    /// from each along the length, and its clearance from each footprint.
    /// Undecided checks are recorded on `evaluation`; findings returned.
    pub(super) fn judge(
        &self,
        subject: &ObjectId,
        opening: &Opening<'_>,
        host: &Host,
        axes: FaceAxes,
        evaluation: &mut CapabilityEvaluation,
    ) -> Vec<SupportFinding> {
        let found = match self.found(opening.host) {
            Ok(found) => found,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(
                    subject.clone(),
                    reason,
                    format!(
                        "the supports of its host {} cannot be found: {message}",
                        opening.host
                    ),
                );
                return Vec::new();
            }
        };
        let measured: Vec<Measured<'_>> = found
            .members
            .iter()
            .map(|member| Measured {
                member,
                footprint: self
                    .solid(&member.id)
                    .map(|solid| (Footprint::of(&solid, host, axes), solid))
                    .map_err(|(_, message)| message),
            })
            .collect();
        let mut findings = Vec::new();
        let mut check = |check: Check<'_>| {
            if let Some(finding) = check.decide(subject, &found, &measured, evaluation) {
                findings.push(finding);
            }
        };
        let host_id = &opening.host.local_id;
        if let Some(required) = self.config.required(host, axes) {
            let (low, high) = (required.low - ROUNDING, required.high - ROUNDING);
            let text = &required.text;
            check(Check {
                required: text.clone(),
                bounds: if self.config.ratio.is_some() {
                    "the shapes, or the required distance from the host's extent, are known \
                     only within bounds"
                } else {
                    "the shapes are known only within bounds"
                },
                near: &|footprint| {
                    let near = gap(opening.length, footprint.length.inner);
                    (near < low).then_some(near)
                },
                clear: &|footprint| gap(opening.length, footprint.length.outer) >= high,
                message: &|member, near, footprint| {
                    let bound = if footprint.length.is_exact() {
                        ""
                    } else {
                        "at most "
                    };
                    format!(
                        "opening is {bound}{} from support {} along its host {host_id}; {text} \
                         required",
                        metres(near),
                        member.local_id,
                    )
                },
                what: "distance along its host from its supports",
            });
        }
        if let Some(required) = self.config.clearance {
            let limit = required - ROUNDING;
            let face = [opening.length, opening.height];
            check(Check {
                required: metres(required),
                bounds: "the shapes are known only within bounds",
                near: &|footprint| {
                    let near = separation(face, footprint.inner?);
                    (opening.exact && near < limit).then_some(near)
                },
                clear: &|footprint| separation(face, footprint.outer()) >= limit,
                message: &|member, near, footprint| {
                    let exact = footprint.length.is_exact() && footprint.height.is_exact();
                    if near < 0.0 {
                        format!(
                            "opening overlaps connecting member {} by {}{} in the face of its \
                             host {host_id}",
                            member.local_id,
                            if exact { "" } else { "at least " },
                            metres(-near)
                        )
                    } else {
                        format!(
                            "opening is {}{} clear of connecting member {} in the face of its \
                             host {host_id}; {} required",
                            if exact { "" } else { "at most " },
                            metres(near),
                            member.local_id,
                            metres(required)
                        )
                    }
                },
                what: "clearance from its host's connecting members",
            });
        }
        findings
    }
}

/// A member with its footprint in the host's face, or why it has none.
struct Measured<'m> {
    member: &'m Member,
    footprint: Result<(Footprint, Rc<Solid>), String>,
}

/// One requirement an opening must meet against every member.
struct Check<'f> {
    /// The requirement as a message words it.
    required: String,
    /// Why a decided member may still be too close.
    bounds: &'static str,
    /// What is measured, in a message.
    what: &'static str,
    /// The measure against the inner footprint, when it surely falls short.
    near: &'f dyn Fn(&Footprint) -> Option<f64>,
    /// Whether the outer footprint surely meets the requirement.
    clear: &'f dyn Fn(&Footprint) -> bool,
    message: &'f dyn Fn(&ObjectId, f64, &Footprint) -> String,
}

impl Check<'_> {
    /// The finding for the members surely too close, or else a not
    /// evaluated outcome when a member may be.
    fn decide(
        &self,
        subject: &ObjectId,
        found: &Found,
        measured: &[Measured<'_>],
        evaluation: &mut CapabilityEvaluation,
    ) -> Option<SupportFinding> {
        let mut sure: Vec<(&Member, f64, &Footprint, &Solid)> = Vec::new();
        let mut unknown = Vec::new();
        for Measured { member, footprint } in measured {
            match footprint {
                Ok((footprint, solid)) => match (self.near)(footprint).filter(|_| member.decided) {
                    Some(near) => sure.push((member, near, footprint, solid)),
                    None if (self.clear)(footprint) => {}
                    None if member.decided => {
                        unknown.push(format!("{} ({})", member.id, self.bounds));
                    }
                    None => unknown.push(format!(
                        "{} (whether it is a selected support of the host is undecided)",
                        member.id
                    )),
                },
                Err(why) => unknown.push(format!("{} ({why})", member.id)),
            }
        }
        let Some((nearest, near, footprint, _)) = sure
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.id.cmp(&b.0.id)))
        else {
            if !unknown.is_empty() {
                evaluation.push_object_not_evaluated(
                    subject.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "its {} may be under {}: {}",
                        self.what,
                        self.required,
                        unknown.join("; ")
                    ),
                );
            }
            return None;
        };
        let mut evidence = found.evidence.clone();
        for (_, _, _, solid) in &sure {
            evidence.extend(solid.evidence.iter().cloned());
        }
        Some((
            (self.message)(&nearest.id, *near, footprint),
            evidence,
            sure.iter().map(|(member, ..)| member.id.clone()).collect(),
        ))
    }
}

/// Whether `candidate` comes within `tolerance` of `host` in space.
fn contact(
    service: &ProximityServiceHandle,
    host: &ObjectId,
    host_bounds: Option<&axioval_engine::ObjectBounds>,
    candidate: &ObjectId,
    tolerance: f64,
) -> Contact {
    let limit = tolerance + ROUNDING;
    match service.bounds(candidate) {
        Err(ProximityError::NoBody) => return Contact::Apart,
        Ok(bounds) => {
            if host_bounds.is_some_and(|host| host.enclosing().gap(&bounds.enclosing()) > limit) {
                return Contact::Apart;
            }
        }
        Err(_) => {}
    }
    let Ok(request) = ProximityRequest::try_new(host.clone(), candidate.clone()) else {
        return Contact::Undecided;
    };
    match service.measure_distance(&request) {
        Ok(measured) => {
            let (lower, upper) = measured.interval_metres();
            if upper <= limit {
                Contact::Touching(measured.evidence().clone())
            } else if lower > limit {
                Contact::Apart
            } else {
                Contact::Undecided
            }
        }
        Err(ProximityError::NoBody) => Contact::Apart,
        Err(_) => Contact::Undecided,
    }
}
