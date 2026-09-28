//! Source-neutral distance capability.
//!
//! Each subject's counterparts must keep a declared distance, in one of three
//! modes:
//!
//! - `nearest` (the default): the nearest counterpart lies within
//!   `minimum_metres` and/or `maximum_metres`, which is no counterpart closer
//!   than the minimum and at least one within the maximum;
//! - `none_closer_than`: no counterpart lies closer than `minimum_metres`;
//! - `at_least`: at least `count` counterparts lie within `maximum_metres`,
//!   and no nearer than `minimum_metres` when one is declared.
//!
//! Distance is measured in the declared `projection`: surface to surface in
//! space, in plan, vertically between bodies above one another (only those
//! above or only those below with `vertical_direction`), or as overlap in
//! plan. Counterparts may be scoped to the subject's containers (its space,
//! its group) through the traversal parameters; with `container_selector`
//! only reached containers it picks count, so two sprinklers sharing a zone
//! of another type than a fire zone are not paired.
//!
//! A `vertical` distance may run between chosen surfaces
//! (`subject_surface` `top` or `bottom`, `counterpart_surface` `top`,
//! `bottom` or `nearest`): `nearest` is the counterpart's surface directly
//! over or under the subject's footprint, such as a sloped slab's underside
//! right above a sprinkler. A `horizontal` distance with `elevation_overlap`
//! `overlapping` relates only counterparts whose heights overlap the
//! subject's, or with `elevation_offset_metres` come closer than that in
//! height: a counterpart on another storey is ignored.
//!
//! With `subject_extent` or `counterpart_extent` `leaf_swing` (or its older
//! name `door_swing`), that side is measured by the plan area its doors'
//! leaves and windows' panels sweep (the swing footprint) instead of its
//! body, in plan (`projection: horizontal`). Each side-hinged sector is
//! bracketed between an inscribed and a circumscribed polygon, so its
//! distance is an interval too; a tilting window panel sweeps an exact
//! rectangle. A door or window whose leaves neither swing nor tilt sweeps
//! nothing and has no distance to anything.
//!
//! Every distance is an interval, a point for exact geometry. A counterpart
//! counts only when its whole interval satisfies the bound, and is certainly
//! excluded only when its whole interval misses it; one whose interval
//! straddles a bound, whose distance or extent could not be read, or whose
//! container could not be decided is undecided. Undecided counterparts are
//! counted as unknown: a verdict is given only when they cannot change it.
//! The broad phase is complete in every projection, so a counterpart it does
//! not propose is proven beyond the search margin.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ConvexPlanRegion, CounterpartSurface, Deviation,
    GeometryFidelity, NotEvaluatedReason, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, ProjectedDistanceEvidence, ProximityProjection, ProximityRequest,
    ProximityServiceHandle, RegionDistanceRequest, RuleCapability, RuleContext, SubjectSurface,
    VerticalDirection, VerticalExtent, VerticalExtentServiceHandle, VerticalSurfaces,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::door_swing::{self, Footprint, box_gap};
use crate::pairs::{
    Prepared, Unevaluated, counterpart_selector, fidelity_note, prepare, reason, refuse_all,
    refuse_declaration,
};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid, traversal_parameters};

/// Requires counterparts to keep a declared distance from each subject.
pub struct Distance;

#[derive(Clone, Copy)]
enum Mode {
    Nearest,
    NoneCloserThan,
    AtLeast(u64),
}

struct Declaration<'a> {
    mode: Mode,
    /// Whether the subjects and the counterparts are measured by their
    /// door swings rather than their bodies.
    swings: (bool, bool),
    minimum: Option<f64>,
    maximum: Option<f64>,
    projection: ProximityProjection,
    scope: Option<Traversal>,
    /// Which reached objects count as containers; every one without it.
    containers: Option<&'a Selector>,
    /// With `elevation_overlap` `overlapping`, the height gap a counterpart
    /// must stay under (zero: the heights overlap).
    elevation: Option<f64>,
}

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.number(name)? {
        Some(value) if value < 0.0 => Err(invalid(format!("`{name}` must not be negative"))),
        other => Ok(other),
    }
}

/// The declared mode, checked against the declared bounds.
fn mode(
    parameters: &Parameters<'_>,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Result<Mode, Unavailable> {
    let count = parameters.integer("count")?;
    Ok(
        match (parameters.string("mode")?.unwrap_or("nearest"), count) {
            ("nearest", None) => {
                if minimum.is_none() && maximum.is_none() {
                    return Err(invalid(
                        "`nearest` needs `minimum_metres`, `maximum_metres` or both",
                    ));
                }
                Mode::Nearest
            }
            ("none_closer_than", None) => {
                if !minimum.is_some_and(|minimum| minimum > 0.0) || maximum.is_some() {
                    return Err(invalid(
                        "`none_closer_than` needs a positive `minimum_metres` and no maximum",
                    ));
                }
                Mode::NoneCloserThan
            }
            ("at_least", Some(count)) => {
                let count = u64::try_from(count)
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| invalid("`count` must be at least one"))?;
                if maximum.is_none() {
                    return Err(invalid("`at_least` needs `maximum_metres`"));
                }
                Mode::AtLeast(count)
            }
            ("at_least", None) => return Err(invalid("`at_least` needs `count`")),
            ("nearest" | "none_closer_than", Some(_)) => {
                return Err(invalid("`count` applies only to `at_least`"));
            }
            (other, _) => return Err(invalid(format!("mode `{other}` is unsupported"))),
        },
    )
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let minimum = length(&parameters, "minimum_metres")?;
    let maximum = length(&parameters, "maximum_metres")?;
    let mode = mode(&parameters, minimum, maximum)?;
    if let (Some(minimum), Some(maximum)) = (minimum, maximum)
        && minimum > maximum
    {
        return Err(invalid("`minimum_metres` exceeds `maximum_metres`"));
    }
    let offset = length(&parameters, "footprint_offset_metres")?;
    let direction = match parameters.string("vertical_direction")? {
        None => None,
        Some("either") => Some(VerticalDirection::Either),
        Some("above") => Some(VerticalDirection::Above),
        Some("below") => Some(VerticalDirection::Below),
        Some(other) => {
            return Err(invalid(format!(
                "vertical direction `{other}` is unsupported; use `above`, `below` or `either`"
            )));
        }
    };
    let surfaces = surfaces(&parameters)?;
    if surfaces.is_some() && parameters.string("projection")? != Some("vertical") {
        return Err(invalid(
            "`subject_surface` and `counterpart_surface` apply only to the `vertical` projection",
        ));
    }
    let projection = match (parameters.string("projection")?, offset, direction) {
        (None | Some("minimum_3d"), None, None) => ProximityProjection::Minimum3d,
        (Some("horizontal"), None, None) => ProximityProjection::Horizontal,
        (Some("plan_overlap"), None, None) => ProximityProjection::PlanOverlap,
        (Some("vertical"), offset, direction) => {
            let surfaces = surfaces.unwrap_or_default();
            if offset.is_some_and(|offset| offset > 0.0)
                && matches!(
                    surfaces,
                    VerticalSurfaces::Between {
                        counterpart: CounterpartSurface::Nearest,
                        ..
                    }
                )
            {
                return Err(invalid(
                    "`counterpart_surface` `nearest` lies over the footprint itself; it takes \
                     no `footprint_offset_metres`",
                ));
            }
            ProximityProjection::Vertical {
                footprint_offset_metres: offset.unwrap_or(0.0),
                direction: direction.unwrap_or(VerticalDirection::Either),
                surfaces,
            }
        }
        (None | Some("minimum_3d" | "horizontal" | "plan_overlap"), Some(_), _) => {
            return Err(invalid(
                "`footprint_offset_metres` applies only to the `vertical` projection",
            ));
        }
        (None | Some("minimum_3d" | "horizontal" | "plan_overlap"), None, Some(_)) => {
            return Err(invalid(
                "`vertical_direction` applies only to the `vertical` projection",
            ));
        }
        (Some(other), _, _) => {
            return Err(invalid(format!("projection `{other}` is unsupported")));
        }
    };
    let extent = |name: &str| match parameters.string(name)? {
        None | Some("body") => Ok(false),
        Some("leaf_swing" | "door_swing") => Ok(true),
        Some(other) => Err(invalid(format!(
            "`{name}` `{other}` is unsupported; use `body` or `leaf_swing`"
        ))),
    };
    let swings = (extent("subject_extent")?, extent("counterpart_extent")?);
    if (swings.0 || swings.1) && projection != ProximityProjection::Horizontal {
        return Err(invalid(
            "a `leaf_swing` extent is a plan footprint: declare `projection` `horizontal`",
        ));
    }
    let scope = parameters.traversal()?;
    let containers = parameters.selector("container_selector")?;
    if containers.is_some() && scope.is_none() {
        return Err(invalid(
            "`container_selector` picks among the containers `relationship` or `path` reaches; \
             declare one",
        ));
    }
    Ok(Declaration {
        mode,
        swings,
        minimum,
        maximum,
        elevation: elevation(&parameters, projection)?,
        projection,
        scope,
        containers,
    })
}

/// The declared surface pair, when both surfaces are declared.
fn surfaces(parameters: &Parameters<'_>) -> Result<Option<VerticalSurfaces>, Unavailable> {
    let subject = match parameters.string("subject_surface")? {
        None => None,
        Some("top") => Some(SubjectSurface::Top),
        Some("bottom") => Some(SubjectSurface::Bottom),
        Some(other) => {
            return Err(invalid(format!(
                "`subject_surface` `{other}` is unsupported; use `top` or `bottom`"
            )));
        }
    };
    let counterpart = match parameters.string("counterpart_surface")? {
        None => None,
        Some("top") => Some(CounterpartSurface::Top),
        Some("bottom") => Some(CounterpartSurface::Bottom),
        Some("nearest") => Some(CounterpartSurface::Nearest),
        Some(other) => {
            return Err(invalid(format!(
                "`counterpart_surface` `{other}` is unsupported; use `top`, `bottom` or \
                 `nearest`"
            )));
        }
    };
    match (subject, counterpart) {
        (None, None) => Ok(None),
        (Some(subject), Some(counterpart)) => Ok(Some(VerticalSurfaces::Between {
            subject,
            counterpart,
        })),
        _ => Err(invalid(
            "`subject_surface` and `counterpart_surface` are declared together",
        )),
    }
}

/// The height gap a counterpart must stay under, with `elevation_overlap`
/// `overlapping`.
fn elevation(
    parameters: &Parameters<'_>,
    projection: ProximityProjection,
) -> Result<Option<f64>, Unavailable> {
    let offset = length(parameters, "elevation_offset_metres")?;
    match (parameters.string("elevation_overlap")?, offset) {
        (None | Some("any"), None) => Ok(None),
        (None | Some("any"), Some(_)) => Err(invalid(
            "`elevation_offset_metres` applies only to `elevation_overlap` `overlapping`",
        )),
        (Some("overlapping"), offset) => {
            if projection == ProximityProjection::Horizontal {
                Ok(Some(offset.unwrap_or(0.0)))
            } else {
                Err(invalid(
                    "`elevation_overlap` applies only to the `horizontal` projection",
                ))
            }
        }
        (Some(other), _) => Err(invalid(format!(
            "`elevation_overlap` `{other}` is unsupported; use `any` or `overlapping`"
        ))),
    }
}

impl Declaration<'_> {
    /// No counterpart may come closer than this.
    fn keep_apart(&self) -> Option<f64> {
        match self.mode {
            Mode::Nearest | Mode::NoneCloserThan => self.minimum,
            Mode::AtLeast(_) => None,
        }
    }
    /// At least this many counterparts must lie within `[lower, upper]`.
    fn within(&self) -> Option<(u64, f64, f64)> {
        match self.mode {
            Mode::Nearest => self.maximum.map(|maximum| (1, 0.0, maximum)),
            Mode::NoneCloserThan => None,
            Mode::AtLeast(count) => self
                .maximum
                .map(|maximum| (count, self.minimum.unwrap_or(0.0), maximum)),
        }
    }
    /// The broad-phase margin: beyond it no counterpart matters.
    fn margin(&self) -> f64 {
        self.maximum.or(self.minimum).unwrap_or(0.0)
    }
}

/// What is known about one counterpart of one subject.
struct Candidate {
    counterpart: ObjectId,
    /// Whether it shares a container with the subject and stands at its
    /// heights, as far as declared; why not, when undecided.
    in_scope: Result<bool, String>,
    measured: Result<Measured, Unavailable>,
}

/// A measured distance as an interval, with what it measured and the
/// evidence behind it.
struct Measured {
    lower: f64,
    upper: f64,
    /// `horizontal distance`, `vertical distance`, ...
    what: String,
    /// ` above`, ` below` or nothing.
    side: &'static str,
    /// Why the interval is not a point, for messages.
    note: String,
    evidence: Vec<Evidence>,
}

impl Measured {
    fn projected(measured: &ProjectedDistanceEvidence) -> Self {
        let (what, side) = match measured.request().projection() {
            ProximityProjection::Minimum3d => ("distance".to_owned(), ""),
            ProximityProjection::Horizontal => ("horizontal distance".to_owned(), ""),
            ProximityProjection::Vertical {
                direction,
                surfaces,
                ..
            } => (
                match surfaces {
                    VerticalSurfaces::Extents => "vertical distance".to_owned(),
                    VerticalSurfaces::Between {
                        subject,
                        counterpart,
                    } => format!(
                        "vertical distance from its {} to the {} surface",
                        subject.name(),
                        counterpart.name()
                    ),
                },
                match direction {
                    VerticalDirection::Either => "",
                    VerticalDirection::Above => " above",
                    VerticalDirection::Below => " below",
                },
            ),
            ProximityProjection::PlanOverlap => ("plan-overlap distance".to_owned(), ""),
        };
        let (lower, upper) = measured.interval_metres();
        Self {
            lower,
            upper,
            what,
            side,
            note: fidelity_note(measured.fidelity()),
            evidence: vec![measured.evidence().clone()],
        }
    }

    fn interval_metres(&self) -> (f64, f64) {
        (self.lower, self.upper)
    }
}

impl Candidate {
    fn interval(&self) -> Option<(f64, f64)> {
        self.measured.as_ref().ok().map(Measured::interval_metres)
    }
    fn certainly_in_scope(&self) -> bool {
        self.in_scope == Ok(true)
    }
    fn possibly_in_scope(&self) -> bool {
        self.in_scope != Ok(false)
    }
    /// Why this candidate leaves a check open.
    fn undecided(&self, bound: &str) -> Unavailable {
        if let Err(why) = &self.in_scope {
            return (NotEvaluatedReason::IncompleteEvidence, why.clone());
        }
        match &self.measured {
            Err(unavailable) => unavailable.clone(),
            Ok(measured) => (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} to {} straddles {bound}{}",
                    describe(measured),
                    self.counterpart,
                    measured.note
                ),
            ),
        }
    }
}

/// A distance as a reviewer reads it: a point, a range, or no relation.
fn describe(measured: &Measured) -> String {
    let (projection, side) = (&measured.what, measured.side);
    match measured.interval_metres() {
        (lower, _) if lower.is_infinite() => format!("no {projection}{side}"),
        (lower, upper) if lower >= upper => format!("{projection} {lower:.4} m{side}"),
        (lower, upper) if upper.is_infinite() => {
            format!("{projection} of at least {lower:.4} m{side}, or none")
        }
        (lower, upper) => format!("{projection} between {lower:.4} and {upper:.4} m{side}"),
    }
}

/// The outcome of one check for one subject.
enum Verdict {
    Finding {
        message: String,
        related: Vec<ObjectId>,
        evidence: Vec<Evidence>,
        /// How far the named distance misses its bound; `None` when no
        /// distance was measured against one.
        deviation: Option<Deviation>,
    },
    NotEvaluated(Unavailable),
    Pass,
}

/// No counterpart may come closer than `minimum`.
fn keep_apart(
    minimum: f64,
    candidates: &[Candidate],
    unmeasurable: &[Candidate],
    nearest_mode: bool,
) -> Verdict {
    let mut violating: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| {
            candidate.certainly_in_scope()
                && candidate
                    .interval()
                    .is_some_and(|(_, upper)| upper < minimum)
        })
        .collect();
    if violating.is_empty() {
        let bound = format!("the minimum {minimum:.4} m");
        return match candidates.iter().chain(unmeasurable).find(|candidate| {
            candidate.possibly_in_scope()
                && candidate
                    .interval()
                    .is_none_or(|(lower, _)| lower < minimum)
        }) {
            Some(open) => Verdict::NotEvaluated(open.undecided(&bound)),
            None => Verdict::Pass,
        };
    }
    violating.sort_by(|a, b| {
        let key = |candidate: &Candidate| candidate.interval().map_or(f64::INFINITY, |(l, _)| l);
        key(a)
            .total_cmp(&key(b))
            .then_with(|| a.counterpart.cmp(&b.counterpart))
    });
    let nearest = violating[0];
    let Ok(measured) = &nearest.measured else {
        unreachable!("a violation is measured");
    };
    let message = if nearest_mode {
        format!(
            "nearest counterpart {} is at {}, closer than the required {minimum:.4} m{}",
            nearest.counterpart,
            describe(measured),
            measured.note
        )
    } else {
        format!(
            "{} counterpart(s) closer than the required {minimum:.4} m; nearest {} at {}{}",
            violating.len(),
            nearest.counterpart,
            describe(measured),
            measured.note
        )
    };
    let shown: &[&Candidate] = if nearest_mode {
        &violating[..1]
    } else {
        &violating
    };
    let (lower, upper) = measured.interval_metres();
    Verdict::Finding {
        message,
        deviation: Some(Deviation::below(minimum, lower, upper)),
        related: shown.iter().map(|c| c.counterpart.clone()).collect(),
        evidence: shown
            .iter()
            .filter_map(|c| c.measured.as_ref().ok())
            .flat_map(|measured| measured.evidence.iter().cloned())
            .collect(),
    }
}

/// At least `count` counterparts must lie within `[lower, upper]`.
fn within(
    (count, lower, upper): (u64, f64, f64),
    candidates: &[Candidate],
    unmeasurable: &[Candidate],
    nearest_mode: bool,
) -> Verdict {
    let inside = |(low, high): (f64, f64)| low >= lower && high <= upper;
    let reaches = |(low, high): (f64, f64)| high >= lower && low <= upper;
    let certain = candidates
        .iter()
        .filter(|c| c.certainly_in_scope() && c.interval().is_some_and(inside))
        .count();
    if u64::try_from(certain).unwrap_or(u64::MAX) >= count {
        return Verdict::Pass;
    }
    let possible: Vec<&Candidate> = candidates
        .iter()
        .chain(unmeasurable)
        .filter(|c| c.possibly_in_scope() && c.interval().is_none_or(reaches))
        .collect();
    let range = if lower > 0.0 {
        format!("between {lower:.4} and {upper:.4} m")
    } else {
        format!("within {upper:.4} m")
    };
    if u64::try_from(possible.len()).unwrap_or(u64::MAX) >= count {
        let open = possible
            .iter()
            .find(|c| !(c.certainly_in_scope() && c.interval().is_some_and(inside)))
            .unwrap_or_else(|| unreachable!("fewer certain than possible"));
        return Verdict::NotEvaluated(open.undecided(&format!("the range {range}")));
    }
    // A shortfall. Name the nearest measured counterpart in scope, if any.
    let nearest = candidates
        .iter()
        .filter(|c| c.certainly_in_scope())
        .filter_map(|c| c.measured.as_ref().ok().map(|m| (c, m)))
        .filter(|(_, measured)| measured.interval_metres().0.is_finite())
        .min_by(|(a, am), (b, bm)| {
            am.interval_metres()
                .0
                .total_cmp(&bm.interval_metres().0)
                .then_with(|| a.counterpart.cmp(&b.counterpart))
        });
    let message = match (nearest_mode, nearest) {
        (true, Some((candidate, measured))) => format!(
            "nearest counterpart {} is at {}, farther than the allowed {upper:.4} m{}",
            candidate.counterpart,
            describe(measured),
            measured.note
        ),
        (true, None) => format!("no counterpart lies within {upper:.4} m"),
        (false, _) => format!(
            "{}{} counterpart(s) lie {range}, {count} required",
            if possible.len() > certain {
                "at most "
            } else {
                ""
            },
            possible.len()
        ),
    };
    let deviation = match (nearest_mode, nearest) {
        (true, Some((_, measured))) => {
            let (low, high) = measured.interval_metres();
            Some(Deviation::above(upper, low, high))
        }
        _ => None,
    };
    let named: Vec<&Candidate> = if nearest_mode {
        nearest
            .map(|(candidate, _)| candidate)
            .into_iter()
            .collect()
    } else {
        possible
    };
    Verdict::Finding {
        message,
        deviation,
        related: named.iter().map(|c| c.counterpart.clone()).collect(),
        evidence: named
            .iter()
            .filter_map(|c| c.measured.as_ref().ok())
            .flat_map(|measured| measured.evidence.iter().cloned())
            .collect(),
    }
}

/// The containers an object reaches, with the traversal's evidence.
type Containers = Result<(BTreeSet<ObjectId>, Vec<Evidence>), Unavailable>;

/// Whether a counterpart is in scope; why not decided, when undecided.
type Admission = Result<bool, String>;

/// Both admissions: out when either is, in when both are.
fn both(first: Admission, second: Admission) -> Admission {
    match (first, second) {
        (Ok(false), _) | (_, Ok(false)) => Ok(false),
        (Ok(true), Ok(true)) => Ok(true),
        (Err(why), _) | (_, Err(why)) => Err(why),
    }
}

/// The objects `container_selector` picks, and those it cannot decide.
struct Kinds {
    sure: BTreeSet<ObjectId>,
    undecided: BTreeSet<ObjectId>,
}

impl Kinds {
    fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
        let (matched, selection) = select_objects(context, selector);
        Self {
            sure: matched.iter().map(|object| object.id.clone()).collect(),
            undecided: selection
                .not_evaluated_outcomes()
                .iter()
                .filter_map(|outcome| outcome.object_id().cloned())
                .collect(),
        }
    }

    /// The reached objects that surely, and that possibly, are containers.
    fn split(&self, reached: &BTreeSet<ObjectId>) -> (BTreeSet<ObjectId>, BTreeSet<ObjectId>) {
        let sure: BTreeSet<ObjectId> = reached.intersection(&self.sure).cloned().collect();
        let mut possible = sure.clone();
        possible.extend(reached.intersection(&self.undecided).cloned());
        (sure, possible)
    }
}

/// Heights for `elevation_overlap`: the subject's extent and each
/// counterpart's, cached.
struct Heights<'r> {
    service: &'r VerticalExtentServiceHandle,
    offset: f64,
    subject: Option<VerticalExtent>,
    extents: BTreeMap<ObjectId, Result<VerticalExtent, String>>,
}

impl Heights<'_> {
    /// Whether `counterpart` comes closer than the offset to the subject in
    /// height (their heights overlap, for an offset of zero).
    fn admit(&mut self, counterpart: &ObjectId) -> Admission {
        let service = self.service;
        let theirs = self
            .extents
            .entry(counterpart.clone())
            .or_insert_with(|| {
                service
                    .measure_vertical_extent(counterpart)
                    .map_err(|error| {
                        format!(
                            "whether {counterpart} stands at the subject's heights could not be \
                             decided: {error}"
                        )
                    })
            })
            .clone()?;
        let own = self
            .subject
            .as_ref()
            .unwrap_or_else(|| unreachable!("a subject's heights are read first"));
        let (bottom, top) = (own.bottom(), own.top());
        let (other_bottom, other_top) = (theirs.bottom(), theirs.top());
        let most = (other_bottom.upper_metres() - top.lower_metres())
            .max(bottom.upper_metres() - other_top.lower_metres());
        let least = (other_bottom.lower_metres() - top.upper_metres())
            .max(bottom.lower_metres() - other_top.upper_metres());
        if most < self.offset {
            Ok(true)
        } else if least >= self.offset {
            Ok(false)
        } else {
            Err(format!(
                "whether {counterpart} stands at the subject's heights straddles the height gap \
                 {:.4} m",
                self.offset
            ))
        }
    }
}

/// Containers reached from objects through the declared traversal, cached,
/// and the heights a counterpart must stand at.
struct Scope<'r> {
    traversal: Option<&'r Traversal>,
    kinds: Option<Kinds>,
    heights: Option<Heights<'r>>,
    context: &'r RuleContext<'r>,
    everything: Vec<&'r Object>,
    reached: BTreeMap<ObjectId, Containers>,
}

impl<'r> Scope<'r> {
    fn new(
        declared: &'r Declaration<'_>,
        heights: Option<&'r VerticalExtentServiceHandle>,
        context: &'r RuleContext<'r>,
    ) -> Self {
        Self {
            traversal: declared.scope.as_ref(),
            kinds: declared
                .containers
                .map(|selector| Kinds::of(context, selector)),
            heights: heights
                .zip(declared.elevation)
                .map(|(service, offset)| Heights {
                    service,
                    offset,
                    subject: None,
                    extents: BTreeMap::new(),
                }),
            context,
            everything: context.project.objects().collect(),
            reached: BTreeMap::new(),
        }
    }

    /// Reads the subject's heights, when they are declared to matter.
    fn enter(&mut self, subject: &ObjectId) -> Result<(), Unavailable> {
        if let Some(heights) = &mut self.heights {
            let extent = heights
                .service
                .measure_vertical_extent(subject)
                .map_err(|error| crate::orientation::extent_unavailable(&error))?;
            heights.subject = Some(extent);
        }
        Ok(())
    }

    fn containers(&mut self, object: &ObjectId) -> &Containers {
        let Self {
            traversal,
            context,
            everything,
            reached,
            ..
        } = self;
        reached.entry(object.clone()).or_insert_with(|| {
            let traversal = traversal.unwrap_or_else(|| unreachable!("scoped only when declared"));
            traversal
                .related(context, object, everything)
                .map(|(found, evidence)| (found.into_iter().collect(), evidence))
        })
    }

    /// Whether `counterpart` shares a container with a subject reaching
    /// `subject_containers` and stands at its heights, as declared.
    fn shares(
        &mut self,
        subject_containers: &BTreeSet<ObjectId>,
        counterpart: &ObjectId,
    ) -> Admission {
        let heights = match &mut self.heights {
            Some(heights) => heights.admit(counterpart),
            None => Ok(true),
        };
        if heights == Ok(false) || self.traversal.is_none() {
            return heights;
        }
        let undecided = || {
            format!(
                "whether {counterpart} shares a container with the subject could not be decided"
            )
        };
        let theirs = match self.containers(counterpart) {
            Ok((containers, _)) => containers.clone(),
            Err(_) => return both(heights, Err(undecided())),
        };
        let shared = match &self.kinds {
            None => Ok(!theirs.is_disjoint(subject_containers)),
            Some(kinds) => {
                let (own_sure, own_possible) = kinds.split(subject_containers);
                let (sure, possible) = kinds.split(&theirs);
                if !own_sure.is_disjoint(&sure) {
                    Ok(true)
                } else if own_possible.is_disjoint(&possible) {
                    Ok(false)
                } else {
                    Err(format!(
                        "whether {counterpart} shares a container of the declared kind with the \
                         subject could not be decided"
                    ))
                }
            }
        };
        both(heights, shared)
    }
}
/// Candidates for `subject`: every counterpart the broad phase proposed, in
/// scope or undecided, measured in the declared projection.
fn candidates(
    prepared: &Prepared<'_>,
    declared: &Declaration<'_>,
    scope: &mut Scope<'_>,
    subject: &ObjectId,
    subject_containers: &BTreeSet<ObjectId>,
) -> (Vec<Candidate>, Vec<Candidate>) {
    // The broad phase reports each pair once, in either orientation.
    let proposed = prepared.pairs.iter().filter_map(|pair| {
        if pair.subject() == subject {
            Some(pair.counterpart())
        } else if pair.counterpart() == subject && prepared.counterparts.contains(pair.subject()) {
            Some(pair.subject())
        } else {
            None
        }
    });
    let mut measured = Vec::new();
    for counterpart in proposed {
        let in_scope = scope.shares(subject_containers, counterpart);
        if in_scope == Ok(false) {
            continue;
        }
        let outcome =
            ProximityRequest::projected(subject.clone(), counterpart.clone(), declared.projection)
                .and_then(|request| prepared.service.measure_distance(&request))
                .map(|measured| Measured::projected(&measured))
                .map_err(|error| {
                    (
                        reason(error),
                        format!("distance to {counterpart} could not be measured: {error}"),
                    )
                });
        measured.push(Candidate {
            counterpart: counterpart.clone(),
            in_scope,
            measured: outcome,
        });
    }
    let unmeasurable = prepared
        .unmeasurable_counterparts
        .iter()
        .filter(|counterpart| *counterpart != subject)
        .filter_map(|counterpart| {
            let in_scope = scope.shares(subject_containers, counterpart);
            (in_scope != Ok(false)).then(|| Candidate {
                counterpart: counterpart.clone(),
                in_scope,
                measured: Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the extent of counterpart {counterpart} could not be read, so its distance is unknown"
                    ),
                )),
            })
        })
        .collect();
    (measured, unmeasurable)
}

/// What one side of a pair is measured by.
enum Extent {
    /// The body, with its plan box grown by the chord deviation.
    Body(([f64; 2], [f64; 2])),
    /// The swing footprint of a door's or window's leaves.
    Swing(Footprint),
}

impl Extent {
    /// The plan box around the extent; `None` for a footprint sweeping
    /// nothing.
    fn plan_box(&self) -> Option<([f64; 2], [f64; 2])> {
        match self {
            Self::Body(plan) => Some(*plan),
            Self::Swing(footprint) => footprint.plan_box(),
        }
    }
}

/// Subjects and counterparts measured by door swings on at least one side,
/// with their extents read.
struct Swings<'a> {
    proximity: Option<&'a ProximityServiceHandle>,
    subjects: Vec<ObjectId>,
    counterparts: Vec<ObjectId>,
    extents: BTreeMap<(ObjectId, bool), Extent>,
    unmeasurable_counterparts: BTreeSet<ObjectId>,
    unevaluated: Unevaluated,
    margin: f64,
    sides: (bool, bool),
}

impl<'a> Swings<'a> {
    #[allow(clippy::too_many_lines)]
    /// Selects both groups and reads each object's extent: its door swing
    /// or its body, as its side declares.
    fn prepare(
        context: &RuleContext<'a>,
        rule: &CompiledRule,
        declared: &Declaration<'_>,
    ) -> Result<Self, CapabilityEvaluation> {
        let (subjects, evaluation) = select_objects(context, &rule.selector);
        let Some(selector) = counterpart_selector(rule) else {
            return Err(refuse_all(
                &subjects,
                evaluation,
                &NotEvaluatedReason::InvalidDeclaration,
                "`counterparts` is not a selector",
            ));
        };
        let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
            return Err(refuse_all(
                &subjects,
                evaluation,
                &NotEvaluatedReason::MissingService,
                "door swings need the object-frame service, which is not registered",
            ));
        };
        let proximity = context.services.get::<ProximityServiceHandle>();
        if proximity.is_none() && !(declared.swings.0 && declared.swings.1) {
            return Err(refuse_all(
                &subjects,
                evaluation,
                &NotEvaluatedReason::MissingService,
                "proximity service is not registered",
            ));
        }
        let (counterparts, counterpart_selection) = select_objects(context, selector);
        let mut unevaluated = Unevaluated::default();
        for outcome in evaluation
            .not_evaluated_outcomes()
            .iter()
            .chain(counterpart_selection.not_evaluated_outcomes())
        {
            if let Some(object) = outcome.object_id() {
                unevaluated.push(
                    object.clone(),
                    outcome.reason().clone(),
                    outcome.message().to_owned(),
                );
            }
        }
        let read = |object: &ObjectId, swing: bool| -> Result<Extent, Unavailable> {
            if swing {
                let leaves = door_swing::leaves(frames, object)?;
                return Footprint::of(&leaves).map(Extent::Swing);
            }
            let Some(service) = proximity else {
                return Err((
                    NotEvaluatedReason::MissingService,
                    "proximity service is not registered".into(),
                ));
            };
            let bounds = service
                .bounds(object)
                .map_err(|error| (reason(error), error.to_string()))?;
            if bounds.object() != object {
                return Err((
                    NotEvaluatedReason::InvalidEvidence,
                    "proximity bounds name a different object".into(),
                ));
            }
            let enclosing = bounds.enclosing();
            let (low, high) = (enclosing.min(), enclosing.max());
            Ok(Extent::Body(([low[0], low[1]], [high[0], high[1]])))
        };
        let mut extents = BTreeMap::new();
        let mut kept = (Vec::new(), Vec::new());
        let mut unmeasurable_counterparts = BTreeSet::new();
        for (objects, swing, is_subject) in [
            (&subjects, declared.swings.0, true),
            (&counterparts, declared.swings.1, false),
        ] {
            for object in objects {
                if let Entry::Vacant(slot) = extents.entry((object.id.clone(), swing)) {
                    match read(&object.id, swing) {
                        Ok(extent) => {
                            slot.insert(extent);
                        }
                        Err((why, message)) => {
                            unevaluated.push(
                                object.id.clone(),
                                why,
                                format!("{message}; its distances were not checked"),
                            );
                            if !is_subject {
                                unmeasurable_counterparts.insert(object.id.clone());
                            }
                            continue;
                        }
                    }
                }
                if is_subject {
                    kept.0.push(object.id.clone());
                } else {
                    kept.1.push(object.id.clone());
                }
            }
        }
        // A counterpart whose extent failed as a subject may still have
        // failed only there; one failing as a counterpart is unmeasurable.
        kept.1
            .retain(|object| !unmeasurable_counterparts.contains(object));
        Ok(Self {
            proximity,
            subjects: kept.0,
            counterparts: kept.1,
            extents,
            unmeasurable_counterparts,
            unevaluated,
            margin: declared.margin(),
            sides: declared.swings,
        })
    }

    /// The plan distance from `footprint` to `body`'s footprint: bounded
    /// below through the circumscribed regions, above through the inscribed
    /// ones.
    fn to_body(&self, footprint: &Footprint, body: &ObjectId) -> Result<Measured, Unavailable> {
        let Some(service) = self.proximity else {
            return Err((
                NotEvaluatedReason::MissingService,
                "proximity service is not registered".into(),
            ));
        };
        let measure = |region: &ConvexPlanRegion| {
            service
                .measure_region_distance(&RegionDistanceRequest::new(region.clone(), body.clone()))
                .map_err(|error| {
                    (
                        reason(error),
                        format!(
                            "the distance from a door swing to {body} could not be measured: \
                             {error}"
                        ),
                    )
                })
        };
        let mut measured = Measured {
            lower: f64::INFINITY,
            upper: f64::INFINITY,
            what: "horizontal distance".to_owned(),
            side: "",
            note: String::new(),
            evidence: vec![footprint.evidence.clone()],
        };
        let mut fidelity = GeometryFidelity::Exact;
        for (inner, outer) in &footprint.parts {
            let below = measure(outer)?;
            let above = measure(inner)?;
            measured.lower = measured.lower.min(below.interval_metres().0);
            measured.upper = measured.upper.min(above.interval_metres().1);
            fidelity = fidelity.combined(below.fidelity());
            measured.evidence.push(below.evidence().clone());
            measured.evidence.push(above.evidence().clone());
        }
        measured.note = fidelity_note(fidelity);
        Ok(measured)
    }

    fn measure(&self, subject: &ObjectId, counterpart: &ObjectId) -> Result<Measured, Unavailable> {
        let (Some(from), Some(to)) = (
            self.extents.get(&(subject.clone(), self.sides.0)),
            self.extents.get(&(counterpart.clone(), self.sides.1)),
        ) else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the extent of {subject} or {counterpart} could not be read"),
            ));
        };
        match (from, to) {
            (Extent::Swing(from), Extent::Swing(to)) => {
                let (lower, upper) = from.distance_to(to);
                Ok(Measured {
                    lower,
                    upper,
                    what: "horizontal distance".to_owned(),
                    side: "",
                    note: String::new(),
                    evidence: vec![from.evidence.clone(), to.evidence.clone()],
                })
            }
            (Extent::Swing(from), Extent::Body(_)) => self.to_body(from, counterpart),
            (Extent::Body(_), Extent::Swing(to)) => self.to_body(to, subject),
            (Extent::Body(_), Extent::Body(_)) => Err(invalid(
                "neither side is a door swing, so this pair belongs to the body path",
            )),
        }
    }

    /// Candidates for `subject`: every counterpart whose extent's plan box
    /// comes within the margin of the subject's, in scope or undecided.
    fn candidates(
        &self,
        scope: &mut Scope<'_>,
        subject: &ObjectId,
        subject_containers: &BTreeSet<ObjectId>,
    ) -> (Vec<Candidate>, Vec<Candidate>) {
        let mut measured = Vec::new();
        let from = self
            .extents
            .get(&(subject.clone(), self.sides.0))
            .and_then(Extent::plan_box);
        for counterpart in &self.counterparts {
            if counterpart == subject {
                continue;
            }
            let to = self
                .extents
                .get(&(counterpart.clone(), self.sides.1))
                .and_then(Extent::plan_box);
            // A footprint sweeping nothing has no distance; a box gap
            // beyond the margin bounds the distance from below.
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            if box_gap(from, to) > self.margin {
                continue;
            }
            let in_scope = scope.shares(subject_containers, counterpart);
            if in_scope == Ok(false) {
                continue;
            }
            measured.push(Candidate {
                counterpart: counterpart.clone(),
                in_scope,
                measured: self.measure(subject, counterpart),
            });
        }
        let unmeasurable = self
            .unmeasurable_counterparts
            .iter()
            .filter(|counterpart| *counterpart != subject)
            .filter_map(|counterpart| {
                let in_scope = scope.shares(subject_containers, counterpart);
                (in_scope != Ok(false)).then(|| Candidate {
                    counterpart: counterpart.clone(),
                    in_scope,
                    measured: Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "the extent of counterpart {counterpart} could not be read, so its \
                             distance is unknown"
                        ),
                    )),
                })
            })
            .collect();
        (measured, unmeasurable)
    }
}

/// Where the pairs come from: bodies through the broad phase, or door
/// swings on at least one side.
enum Pairs<'a> {
    Bodies(Prepared<'a>),
    Swings(Swings<'a>),
}

impl RuleCapability for Distance {
    fn id(&self) -> &'static str {
        "axioval:capability.distance"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("counterparts", ParameterType::Selector),
            ParameterDescriptor::optional("minimum_metres", ParameterType::Number),
            ParameterDescriptor::optional("maximum_metres", ParameterType::Number),
            ParameterDescriptor::optional("mode", ParameterType::String),
            ParameterDescriptor::optional("count", ParameterType::Integer),
            ParameterDescriptor::optional("projection", ParameterType::String),
            ParameterDescriptor::optional("footprint_offset_metres", ParameterType::Number),
            ParameterDescriptor::optional("vertical_direction", ParameterType::String),
            ParameterDescriptor::optional("subject_extent", ParameterType::String),
            ParameterDescriptor::optional("counterpart_extent", ParameterType::String),
            ParameterDescriptor::optional("subject_surface", ParameterType::String),
            ParameterDescriptor::optional("counterpart_surface", ParameterType::String),
            ParameterDescriptor::optional("elevation_overlap", ParameterType::String),
            ParameterDescriptor::optional("elevation_offset_metres", ParameterType::Number),
            ParameterDescriptor::optional("container_selector", ParameterType::Selector),
        ];
        parameters.extend(traversal_parameters());
        parameters
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        let heights = context.services.get::<VerticalExtentServiceHandle>();
        if declared.elevation.is_some() && heights.is_none() {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "distance: `elevation_overlap` needs the vertical-extent service, which is not \
                 registered",
            );
        }
        let pairs = if declared.swings.0 || declared.swings.1 {
            match Swings::prepare(context, rule, &declared) {
                Ok(swings) => Pairs::Swings(swings),
                Err(refused) => return refused,
            }
        } else {
            match prepare(context, rule, Some(declared.margin()), declared.projection) {
                Ok(prepared) => Pairs::Bodies(prepared),
                Err(refused) => return refused,
            }
        };
        let subjects = match &pairs {
            Pairs::Bodies(prepared) => prepared.subjects.clone(),
            Pairs::Swings(swings) => swings.subjects.clone(),
        };
        let mut scope = Scope::new(&declared, heights, context);
        let mut evaluation = CapabilityEvaluation::default();

        for subject in &subjects {
            if let Err((reason, message)) = scope.enter(subject) {
                evaluation.push_object_not_evaluated(
                    subject.clone(),
                    reason,
                    format!("the subject's heights could not be read: {message}"),
                );
                continue;
            }
            let (subject_containers, scope_evidence) = if declared.scope.is_some() {
                match scope.containers(subject) {
                    Ok((containers, evidence)) => (containers.clone(), evidence.clone()),
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(
                            subject.clone(),
                            reason.clone(),
                            format!("the subject's containers could not be decided: {message}"),
                        );
                        continue;
                    }
                }
            } else {
                (BTreeSet::new(), Vec::new())
            };
            let (measured, unmeasurable) = match &pairs {
                Pairs::Bodies(prepared) => candidates(
                    prepared,
                    &declared,
                    &mut scope,
                    subject,
                    &subject_containers,
                ),
                Pairs::Swings(swings) => {
                    swings.candidates(&mut scope, subject, &subject_containers)
                }
            };
            judge(
                &mut evaluation,
                rule,
                &declared,
                subject,
                (&measured, &unmeasurable),
                scope_evidence,
            );
        }
        match pairs {
            Pairs::Bodies(prepared) => prepared.unevaluated.drain_into(&mut evaluation),
            Pairs::Swings(swings) => swings.unevaluated.drain_into(&mut evaluation),
        }
        evaluation
    }
}

/// Judges one subject's candidates and records the outcome.
fn judge(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    subject: &ObjectId,
    (measured, unmeasurable): (&[Candidate], &[Candidate]),
    scope_evidence: Vec<Evidence>,
) {
    let nearest_mode = matches!(declared.mode, Mode::Nearest);
    let apart = declared
        .keep_apart()
        .map(|minimum| keep_apart(minimum, measured, unmeasurable, nearest_mode));
    let reach = declared
        .within()
        .map(|bounds| within(bounds, measured, unmeasurable, nearest_mode));

    let mut messages = Vec::new();
    let mut related = Vec::new();
    let mut evidence = Vec::new();
    let mut open = None;
    // A finding missing both bounds grades by the worse; one naming
    // no distance against its bound is not graded.
    let mut deviations = Vec::new();
    for verdict in [apart, reach].into_iter().flatten() {
        match verdict {
            Verdict::Finding {
                message,
                related: named,
                evidence: cited,
                deviation,
            } => {
                messages.push(message);
                related.extend(named);
                evidence.extend(cited);
                deviations.push(deviation);
            }
            Verdict::NotEvaluated(unavailable) => {
                open.get_or_insert(unavailable);
            }
            Verdict::Pass => {}
        }
    }
    // A certain violation stands whatever is undecided.
    if !messages.is_empty() {
        evidence.extend(scope_evidence);
        let deviation = deviations
            .into_iter()
            .reduce(|a, b| a.zip(b).map(|(a, b)| a.worst(b)))
            .flatten();
        evaluation.push_finding_deviating(
            finding(rule, subject, messages.join("; "), evidence, related),
            deviation,
        );
    } else if let Some((reason, message)) = open {
        evaluation.push_object_not_evaluated(subject.clone(), reason, message);
    }
}
