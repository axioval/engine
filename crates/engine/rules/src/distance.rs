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
//!
//! It runs as a template ([`axioval_engine::template`]) over the measured
//! `distance_items`: the capability's own reading of each subject (the
//! broad phase once per rule, the counterparts in scope, measured, and
//! what keeping apart and lying within come to), the template judging the
//! nearest distances against the bounds and the counterparts counted
//! against `count`; the objects the selections and the broad phase leave
//! open are the measured `distance_open` of the project.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CandidatePair, CapabilityEvaluation, CompiledRule, ConvexPlanRegion, CounterpartSurface,
    GeometryFidelity, NotEvaluatedReason, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, ProjectedDistanceEvidence, ProximityProjection, ProximityRequest,
    ProximityServiceHandle, RegionDistanceRequest, RuleCapability, RuleContext, SubjectSurface,
    VerticalDirection, VerticalExtent, VerticalExtentServiceHandle, VerticalSurfaces,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Object, ObjectId};

use crate::door_swing::{self, Footprint, box_gap};
use crate::pairs::{Prepared, Unevaluated, fidelity_note, reason};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, invalid, traversal_parameters};

mod items;
mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use items::DistanceItems;
pub(crate) use measured::DistanceMeasures;

/// Requires counterparts to keep a declared distance from each subject.
///
/// It runs as a template ([`axioval_engine::template`]) over the measured
/// `distance_items` of each subject and `distance_open` of the project.
pub struct Distance;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for Distance {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn grades_deviation(&self) -> bool {
        TEMPLATE.grades
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The capability's parameters.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::required("counterparts", ParameterType::Selector),
        ParameterDescriptor::optional("minimum_metres", ParameterType::Number).per_object(),
        ParameterDescriptor::optional("maximum_metres", ParameterType::Number).per_object(),
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

/// Checks the rule parameters `distance_items` names, as the rule states
/// them: what the capability refused of its declaration, in its order and
/// words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    declaration(&rule).map(|_| ())
}

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Nearest,
    NoneCloserThan,
    AtLeast(u64),
}

pub(crate) struct Declaration {
    pub(crate) mode: Mode,
    /// Whether the subjects and the counterparts are measured by their
    /// door swings rather than their bodies.
    pub(crate) swings: (bool, bool),
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
    pub(crate) projection: ProximityProjection,
    pub(crate) scope: Option<Traversal>,
    /// Which reached objects count as containers; every one without it.
    pub(crate) containers: Option<Selector>,
    /// With `elevation_overlap` `overlapping`, the height gap a counterpart
    /// must stay under (zero: the heights overlap).
    pub(crate) elevation: Option<f64>,
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

pub(crate) fn declaration(rule: &CompiledRule) -> Result<Declaration, Unavailable> {
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
    let containers = parameters.selector("container_selector")?.cloned();
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

impl Declaration {
    /// No counterpart may come closer than this.
    pub(crate) fn keep_apart(&self) -> Option<f64> {
        match self.mode {
            Mode::Nearest | Mode::NoneCloserThan => self.minimum,
            Mode::AtLeast(_) => None,
        }
    }
    /// At least this many counterparts must lie within `[lower, upper]`.
    pub(crate) fn within(&self) -> Option<(u64, f64, f64)> {
        match self.mode {
            Mode::Nearest => self.maximum.map(|maximum| (1, 0.0, maximum)),
            Mode::NoneCloserThan => None,
            Mode::AtLeast(count) => self
                .maximum
                .map(|maximum| (count, self.minimum.unwrap_or(0.0), maximum)),
        }
    }
    /// The broad-phase margin: beyond it no counterpart matters.
    pub(crate) fn margin(&self) -> f64 {
        self.maximum.or(self.minimum).unwrap_or(0.0)
    }
}

/// What is known about one counterpart of one subject.
pub(crate) struct Candidate {
    pub(crate) counterpart: ObjectId,
    /// Whether it shares a container with the subject and stands at its
    /// heights, as far as declared; why not, when undecided.
    pub(crate) in_scope: Result<bool, String>,
    pub(crate) measured: Result<Measured, Unavailable>,
}

/// A measured distance as an interval, with what it measured and the
/// evidence behind it.
pub(crate) struct Measured {
    lower: f64,
    upper: f64,
    /// `horizontal distance`, `vertical distance`, ...
    what: String,
    /// ` above`, ` below` or nothing.
    side: &'static str,
    /// Why the interval is not a point, for messages.
    note: String,
    pub(crate) evidence: Vec<Evidence>,
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

    pub(crate) fn interval_metres(&self) -> (f64, f64) {
        (self.lower, self.upper)
    }
}

impl Candidate {
    pub(crate) fn interval(&self) -> Option<(f64, f64)> {
        self.measured.as_ref().ok().map(Measured::interval_metres)
    }
    pub(crate) fn certainly_in_scope(&self) -> bool {
        self.in_scope == Ok(true)
    }
    pub(crate) fn possibly_in_scope(&self) -> bool {
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

/// What a check judged: the distance named against its bound, the
/// counterparts surely and possibly counted, or nothing measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Judged {
    /// The nearest counterpart's distance, which misses (or meets) the
    /// bound.
    Distance(f64, f64),
    /// The counterparts surely and possibly within the range.
    Count(u64, u64),
    /// No distance and no count: nothing the bound is judged on.
    Nothing,
}

/// The outcome of one check for one subject.
pub(crate) enum Verdict {
    Finding {
        message: String,
        related: Vec<ObjectId>,
        evidence: Vec<Evidence>,
        /// What the finding judged.
        judged: Judged,
    },
    NotEvaluated(Unavailable),
    Pass(Judged),
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
            None => Verdict::Pass(Judged::Nothing),
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
        judged: Judged::Distance(lower, upper),
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
    let possible: Vec<&Candidate> = candidates
        .iter()
        .chain(unmeasurable)
        .filter(|c| c.possibly_in_scope() && c.interval().is_none_or(reaches))
        .collect();
    let counted = Judged::Count(
        u64::try_from(certain).unwrap_or(u64::MAX),
        u64::try_from(possible.len()).unwrap_or(u64::MAX),
    );
    if u64::try_from(certain).unwrap_or(u64::MAX) >= count {
        return Verdict::Pass(counted);
    }
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
    let judged = match (nearest_mode, nearest) {
        (true, Some((_, measured))) => {
            let (low, high) = measured.interval_metres();
            Judged::Distance(low, high)
        }
        (true, None) => Judged::Nothing,
        (false, _) => counted,
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
        judged,
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
pub(crate) struct Kinds {
    pub(crate) sure: BTreeSet<ObjectId>,
    pub(crate) undecided: BTreeSet<ObjectId>,
}

impl Kinds {
    pub(crate) fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
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

/// What a rule's subjects share: the containers each object reaches and
/// each counterpart's heights, read once.
#[derive(Default)]
pub(crate) struct Caches {
    reached: BTreeMap<ObjectId, Containers>,
    extents: BTreeMap<ObjectId, Result<VerticalExtent, String>>,
}

/// Containers reached from objects through the declared traversal, and the
/// heights a counterpart must stand at, around one subject.
pub(crate) struct Scope<'r, 'c> {
    traversal: Option<&'r Traversal>,
    kinds: Option<&'r Kinds>,
    /// The vertical-extent service and the height gap, with
    /// `elevation_overlap` `overlapping`.
    heights: Option<(&'r VerticalExtentServiceHandle, f64)>,
    /// The subject's heights, read by [`Self::enter`].
    subject: Option<VerticalExtent>,
    context: &'r RuleContext<'c>,
    everything: &'r [ObjectId],
    caches: &'r mut Caches,
}

impl<'r, 'c> Scope<'r, 'c> {
    pub(crate) fn new(
        declared: &'r Declaration,
        kinds: Option<&'r Kinds>,
        (context, everything): (&'r RuleContext<'c>, &'r [ObjectId]),
        caches: &'r mut Caches,
    ) -> Self {
        Self {
            traversal: declared.scope.as_ref(),
            kinds,
            heights: context
                .services
                .get::<VerticalExtentServiceHandle>()
                .zip(declared.elevation),
            subject: None,
            context,
            everything,
            caches,
        }
    }

    /// Reads the subject's heights, when they are declared to matter.
    fn enter(&mut self, subject: &ObjectId) -> Result<(), Unavailable> {
        if let Some((service, _)) = self.heights {
            let extent = service
                .measure_vertical_extent(subject)
                .map_err(|error| crate::orientation::extent_unavailable(&error))?;
            self.subject = Some(extent);
        }
        Ok(())
    }

    fn containers(&mut self, object: &ObjectId) -> &Containers {
        let Self {
            traversal,
            context,
            everything,
            caches,
            ..
        } = self;
        caches.reached.entry(object.clone()).or_insert_with(|| {
            let traversal = traversal.unwrap_or_else(|| unreachable!("scoped only when declared"));
            traversal
                .related_among(context, object, everything, everything)
                .map(|(found, evidence)| (found.into_iter().collect(), evidence))
        })
    }

    /// Whether `counterpart` comes closer than the offset to the subject in
    /// height (their heights overlap, for an offset of zero).
    fn admit(&mut self, counterpart: &ObjectId) -> Admission {
        let Some((service, offset)) = self.heights else {
            return Ok(true);
        };
        let theirs = self
            .caches
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
        if most < offset {
            Ok(true)
        } else if least >= offset {
            Ok(false)
        } else {
            Err(format!(
                "whether {counterpart} stands at the subject's heights straddles the height gap \
                 {offset:.4} m"
            ))
        }
    }

    /// Whether `counterpart` shares a container with a subject reaching
    /// `subject_containers` and stands at its heights, as declared.
    fn shares(
        &mut self,
        subject_containers: &BTreeSet<ObjectId>,
        counterpart: &ObjectId,
    ) -> Admission {
        let heights = self.admit(counterpart);
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
        let shared = match self.kinds {
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

/// Subjects and counterparts measured by their bodies, after the broad
/// phase.
pub(crate) struct Bodies {
    subjects: Vec<ObjectId>,
    unmeasurable_subjects: BTreeSet<ObjectId>,
    counterparts: BTreeSet<ObjectId>,
    unmeasurable_counterparts: BTreeSet<ObjectId>,
    pairs: Vec<CandidatePair>,
    unevaluated: Unevaluated,
}

impl From<Prepared<'_>> for Bodies {
    fn from(prepared: Prepared<'_>) -> Self {
        Self {
            subjects: prepared.subjects,
            unmeasurable_subjects: prepared.unmeasurable_subjects,
            counterparts: prepared.counterparts,
            unmeasurable_counterparts: prepared.unmeasurable_counterparts,
            pairs: prepared.pairs,
            unevaluated: prepared.unevaluated,
        }
    }
}

/// The counterparts whose extent could not be read, as candidates of
/// `subject`, in scope or undecided.
fn unmeasurable(
    counterparts: &BTreeSet<ObjectId>,
    scope: &mut Scope<'_, '_>,
    subject: &ObjectId,
    subject_containers: &BTreeSet<ObjectId>,
) -> Vec<Candidate> {
    counterparts
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
        .collect()
}

impl Bodies {
    /// Candidates for `subject`: every counterpart the broad phase
    /// proposed, in scope or undecided, measured in the declared
    /// projection.
    pub(crate) fn candidates(
        &self,
        service: &ProximityServiceHandle,
        projection: ProximityProjection,
        scope: &mut Scope<'_, '_>,
        (subject, subject_containers): (&ObjectId, &BTreeSet<ObjectId>),
    ) -> (Vec<Candidate>, Vec<Candidate>) {
        // The broad phase reports each pair once, in either orientation.
        let proposed = self.pairs.iter().filter_map(|pair| {
            if pair.subject() == subject {
                Some(pair.counterpart())
            } else if pair.counterpart() == subject && self.counterparts.contains(pair.subject()) {
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
                ProximityRequest::projected(subject.clone(), counterpart.clone(), projection)
                    .and_then(|request| service.measure_distance(&request))
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
        let unmeasurable = unmeasurable(
            &self.unmeasurable_counterparts,
            scope,
            subject,
            subject_containers,
        );
        (measured, unmeasurable)
    }
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
pub(crate) struct Swings {
    subjects: Vec<ObjectId>,
    unmeasurable_subjects: BTreeSet<ObjectId>,
    counterparts: Vec<ObjectId>,
    extents: BTreeMap<(ObjectId, bool), Extent>,
    unmeasurable_counterparts: BTreeSet<ObjectId>,
    unevaluated: Unevaluated,
    margin: f64,
    sides: (bool, bool),
}

impl Swings {
    /// Reads each selected object's extent, its door swing or its body, as
    /// its side declares; `unevaluated` the objects the selections left
    /// undecided. Why every subject is refused, when a service is missing.
    fn among(
        context: &RuleContext<'_>,
        declared: &Declaration,
        (subjects, counterparts): (&[&Object], &[&Object]),
        mut unevaluated: Unevaluated,
    ) -> Result<Self, Unavailable> {
        let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
            return Err((
                NotEvaluatedReason::MissingService,
                "door swings need the object-frame service, which is not registered".to_owned(),
            ));
        };
        let proximity = context.services.get::<ProximityServiceHandle>();
        if proximity.is_none() && !(declared.swings.0 && declared.swings.1) {
            return Err((
                NotEvaluatedReason::MissingService,
                "proximity service is not registered".to_owned(),
            ));
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
        let mut unmeasurable_subjects = BTreeSet::new();
        let mut unmeasurable_counterparts = BTreeSet::new();
        for (objects, swing, is_subject) in [
            (subjects, declared.swings.0, true),
            (counterparts, declared.swings.1, false),
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
                            if is_subject {
                                unmeasurable_subjects.insert(object.id.clone());
                            } else {
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
        // A subject whose extent was read once, kept, is measurable.
        kept.0
            .retain(|object| !unmeasurable_subjects.contains(object));
        Ok(Self {
            subjects: kept.0,
            unmeasurable_subjects,
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
    fn to_body(
        proximity: Option<&ProximityServiceHandle>,
        footprint: &Footprint,
        body: &ObjectId,
    ) -> Result<Measured, Unavailable> {
        let Some(service) = proximity else {
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

    fn measure(
        &self,
        proximity: Option<&ProximityServiceHandle>,
        subject: &ObjectId,
        counterpart: &ObjectId,
    ) -> Result<Measured, Unavailable> {
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
            (Extent::Swing(from), Extent::Body(_)) => Self::to_body(proximity, from, counterpart),
            (Extent::Body(_), Extent::Swing(to)) => Self::to_body(proximity, to, subject),
            (Extent::Body(_), Extent::Body(_)) => Err(invalid(
                "neither side is a door swing, so this pair belongs to the body path",
            )),
        }
    }

    /// Candidates for `subject`: every counterpart whose extent's plan box
    /// comes within the margin of the subject's, in scope or undecided.
    fn candidates(
        &self,
        proximity: Option<&ProximityServiceHandle>,
        scope: &mut Scope<'_, '_>,
        (subject, subject_containers): (&ObjectId, &BTreeSet<ObjectId>),
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
                measured: self.measure(proximity, subject, counterpart),
            });
        }
        let unmeasurable = unmeasurable(
            &self.unmeasurable_counterparts,
            scope,
            subject,
            subject_containers,
        );
        (measured, unmeasurable)
    }
}

/// Where the pairs come from: bodies through the broad phase, or door
/// swings on at least one side.
pub(crate) enum Pairs {
    Bodies(Bodies),
    Swings(Swings),
}

impl Pairs {
    /// Both groups, already selected (the objects in the project's order,
    /// `unevaluated` those their selections left undecided), prepared as
    /// the declaration measures them; why every subject is refused, where
    /// a service is missing or the broad phase cannot run.
    pub(crate) fn among(
        context: &RuleContext<'_>,
        declared: &Declaration,
        groups: (&[&Object], &[&Object]),
        unevaluated: Unevaluated,
    ) -> Result<Self, Unavailable> {
        if declared.swings.0 || declared.swings.1 {
            Swings::among(context, declared, groups, unevaluated).map(Pairs::Swings)
        } else {
            crate::pairs::prepare_among(
                context,
                groups,
                unevaluated,
                (declared.margin(), declared.projection),
            )
            .map(|prepared| Pairs::Bodies(prepared.into()))
        }
    }

    /// The subjects judged, in the selection's order.
    pub(crate) fn subjects(&self) -> &[ObjectId] {
        match self {
            Self::Bodies(bodies) => &bodies.subjects,
            Self::Swings(swings) => &swings.subjects,
        }
    }

    /// The selected subjects whose extent could not be read.
    pub(crate) fn unmeasurable_subjects(&self) -> &BTreeSet<ObjectId> {
        match self {
            Self::Bodies(bodies) => &bodies.unmeasurable_subjects,
            Self::Swings(swings) => &swings.unmeasurable_subjects,
        }
    }

    /// What the selections and the extents left open, by object.
    pub(crate) fn unevaluated(&self) -> &Unevaluated {
        match self {
            Self::Bodies(bodies) => &bodies.unevaluated,
            Self::Swings(swings) => &swings.unevaluated,
        }
    }

    /// What the selections and the extents left open, by object, taken.
    #[cfg(feature = "parity-reference")]
    pub(crate) fn into_unevaluated(self) -> Unevaluated {
        match self {
            Self::Bodies(bodies) => bodies.unevaluated,
            Self::Swings(swings) => swings.unevaluated,
        }
    }

    fn candidates(
        &self,
        context: &RuleContext<'_>,
        declared: &Declaration,
        scope: &mut Scope<'_, '_>,
        subject: (&ObjectId, &BTreeSet<ObjectId>),
    ) -> (Vec<Candidate>, Vec<Candidate>) {
        let proximity = context.services.get::<ProximityServiceHandle>();
        match self {
            Self::Bodies(bodies) => bodies.candidates(
                proximity.unwrap_or_else(|| unreachable!("bodies are measured by the service")),
                declared.projection,
                scope,
                subject,
            ),
            Self::Swings(swings) => swings.candidates(proximity, scope, subject),
        }
    }
}

/// What one subject's checks come to: keeping its counterparts apart and
/// having them within reach, as declared, and the evidence of its
/// containers.
pub(crate) struct Verdicts {
    pub(crate) apart: Option<Verdict>,
    pub(crate) within: Option<Verdict>,
    pub(crate) scope_evidence: Vec<Evidence>,
}

/// Judges one subject's counterparts, or why its heights or containers
/// cannot be read.
pub(crate) fn verdicts(
    (context, declared): (&RuleContext<'_>, &Declaration),
    pairs: &Pairs,
    scope: &mut Scope<'_, '_>,
    subject: &ObjectId,
) -> Result<Verdicts, Unavailable> {
    scope.enter(subject).map_err(|(reason, message)| {
        (
            reason,
            format!("the subject's heights could not be read: {message}"),
        )
    })?;
    let (subject_containers, scope_evidence) = if declared.scope.is_some() {
        match scope.containers(subject) {
            Ok((containers, evidence)) => (containers.clone(), evidence.clone()),
            Err((reason, message)) => {
                return Err((
                    reason.clone(),
                    format!("the subject's containers could not be decided: {message}"),
                ));
            }
        }
    } else {
        (BTreeSet::new(), Vec::new())
    };
    let (measured, unmeasurable) =
        pairs.candidates(context, declared, scope, (subject, &subject_containers));
    let nearest_mode = matches!(declared.mode, Mode::Nearest);
    Ok(Verdicts {
        apart: declared
            .keep_apart()
            .map(|minimum| keep_apart(minimum, &measured, &unmeasurable, nearest_mode)),
        within: declared
            .within()
            .map(|bounds| within(bounds, &measured, &unmeasurable, nearest_mode)),
        scope_evidence,
    })
}
