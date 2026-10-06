//! Effective coverage of an element's footprint by the effect areas of
//! sources: how much of a room its sprinklers, detectors or extinguishers
//! reach.
//!
//! What the sources' effect areas cover, and the capacities they sum, are
//! measured here ([`Element`]) and read through the measured values of
//! [`EffectMeasures`]; whether that suffices is the template's policy
//! (`effective_coverage/template.rs`).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CoverageEvidence, CoverageRequest, EffectMeets,
    EffectReach, NotEvaluatedReason, ParameterDescriptor, Participant, PlanAreaServiceHandle,
    ProximityProjection, ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::plan_area::unavailable;
use crate::space_access::AccessIndex;
use crate::support::{PropertyRef, Resolved, Unavailable, display, resolve, undefined};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::{EffectMeasures, check_arguments};

/// Requires the union of sources' effect areas to cover enough of each
/// selected element's footprint.
///
/// Every object `sources` picks has an effect area in plan, reaching
/// `range` as `mode` says:
///
/// - `grown`: its footprint grown by `range` in every plan direction;
/// - `touching`: the same, counting only sources whose footprint touches
///   the element's, within `touch_tolerance` (0 when not declared);
/// - `travel`: the points of the element's free region within `range` of
///   travel from the centre of the source's footprint, going round
///   obstacles;
/// - `visible`: the points of the free region the source's centre sees,
///   no farther than `range`.
///
/// The free region is the element's footprint less the footprints of the
/// objects `blockers` picks (travel and sight only); a source whose centre
/// lies outside it reaches none of it. With `access_path` (and
/// `door_selector`, `opening_selector`, `space_selector`, read as
/// `space-connection` reads them), travel and sight continue into the
/// spaces the element's doors and openings join it to: the free region
/// also holds their footprints and the doors' and openings', so a
/// sprinkler in the next room covers the element through an open doorway.
/// The union of the effect areas, clipped to the footprint and divided by
/// the footprint's area (or the area `area_property` states), must reach
/// `minimum_ratio`.
///
/// With `capacity_property` and `capacity_multiplier` (a constant) or
/// `capacity_multiplier_property` (read on each source), a second check
/// compares the summed products of the sources whose effect meets the
/// footprint with the element's area: extinguisher rating units times the
/// floor area one unit serves must reach the room's area. A value is read
/// as a number, or a quantity in its SI unit.
///
/// Areas are intervals: the effect areas are bracketed between inner and
/// outer bounds. A source whose selection or touch is undecided counts only
/// towards the upper bound, a blocker whose selection is undecided only
/// narrows the lower bound, and a source whose effect or extent cannot be
/// measured leaves the upper bound at the whole footprint; so does a door
/// or opening whose spaces cannot be read. A ratio straddling the minimum
/// is not evaluated.
///
/// A value that is not stated (absent, null or blank) is a finding of its
/// own, starting `missing value:`: the element's `area_property`, then
/// checked no further, or the capacity or multiplier of a source that
/// surely contributes. A value of another kind is not evaluated.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `effective_share` at least `minimum_ratio`, and the measured
/// `effective_capacity` at least the element's `effective_area`.
pub struct EffectiveCoverage;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for EffectiveCoverage {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// How the sources' effect reaches.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Mode {
    Grown,
    Touching,
    Travel,
    Visible,
}

impl Mode {
    pub(crate) fn of(name: &str) -> Option<Self> {
        Some(match name {
            "grown" => Self::Grown,
            "touching" => Self::Touching,
            "travel" => Self::Travel,
            "visible" => Self::Visible,
            _ => return None,
        })
    }

    fn reach(self) -> EffectReach {
        match self {
            Self::Grown | Self::Touching => EffectReach::Grown,
            Self::Travel => EffectReach::Travel,
            Self::Visible => EffectReach::Visible,
        }
    }
}

/// What each source's capacity is multiplied by.
#[derive(Clone, Copy)]
pub(crate) enum Multiplier<'a> {
    Constant(f64),
    Property(PropertyRef<'a>),
}

#[derive(Clone, Copy)]
pub(crate) struct Capacity<'a> {
    pub(crate) property: PropertyRef<'a>,
    pub(crate) multiplier: Multiplier<'a>,
}

/// How one element's coverage is measured: the reach, its range, the
/// touch tolerance and where the area is stated.
pub(crate) struct Setting<'a> {
    pub(crate) mode: Mode,
    pub(crate) range: f64,
    pub(crate) touch: f64,
    pub(crate) area: Option<PropertyRef<'a>>,
}

impl Setting<'_> {
    /// How far from the element a source may stand in plan and still
    /// count: the touch tolerance, or the range.
    pub(crate) fn margin(&self) -> f64 {
        match self.mode {
            Mode::Touching => self.touch,
            _ => self.range,
        }
    }
}

pub(crate) struct Services<'a> {
    pub(crate) areas: &'a PlanAreaServiceHandle,
    pub(crate) proximity: &'a ProximityServiceHandle,
}

impl<'a> Services<'a> {
    pub(crate) fn of(context: &RuleContext<'a>) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("{what} service is not registered"),
            )
        };
        Ok(Self {
            areas: context
                .services
                .get::<PlanAreaServiceHandle>()
                .ok_or_else(|| missing("plan-area"))?,
            proximity: context
                .services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| missing("proximity"))?,
        })
    }
}

/// What one element is measured against: the sources and blockers near
/// it, those surely picked, how many cannot be placed, and the access
/// connections.
pub(crate) struct Around<'s> {
    /// Sources within reach of the element, picked or undecided.
    pub(crate) reaching: &'s [ObjectId],
    /// Blockers that may matter to the element, picked or undecided.
    pub(crate) blocking: &'s [ObjectId],
    /// Sources the selector picks.
    pub(crate) sources: &'s BTreeSet<ObjectId>,
    /// Blockers the selector picks.
    pub(crate) blockers: &'s BTreeSet<ObjectId>,
    /// How many sources, and blockers, have an extent that cannot be read:
    /// they may reach, or block in, any element.
    pub(crate) blind: usize,
    pub(crate) blind_blockers: usize,
    pub(crate) index: Option<&'s AccessIndex>,
}

/// A value read from an object.
pub(crate) enum Read {
    Value(f64, Vec<Evidence>),
    /// Not stated: absent, null or blank.
    Missing(String, Vec<Evidence>),
    Unknown(String),
}

/// The area the share and the capacity are measured against.
pub(crate) struct Area {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
    pub(crate) evidence: Vec<Evidence>,
}

/// The coverage request, the connections' evidence, and why the covered
/// area may be larger than measured.
pub(crate) struct Asked {
    pub(crate) request: CoverageRequest,
    pub(crate) notes: Vec<String>,
    /// Whether the upper bound must stay at the whole footprint.
    pub(crate) open: bool,
    pub(crate) evidence: Vec<Evidence>,
}

/// What measuring an element came to: its area, the request and the
/// coverage measured.
pub(crate) struct Measured {
    pub(crate) area: Area,
    pub(crate) asked: Asked,
    pub(crate) coverage: CoverageEvidence,
}

/// Why an element was not measured: its stated area is missing (a finding,
/// with the property's evidence), or something cannot be read.
pub(crate) enum Unmeasured {
    Missing(String, Vec<Evidence>),
    Unavailable(Unavailable),
}

impl From<Unavailable> for Unmeasured {
    fn from(unavailable: Unavailable) -> Self {
        Self::Unavailable(unavailable)
    }
}

/// The covered area as the share check reads it.
pub(crate) struct Coverage {
    pub(crate) covered: (f64, f64),
    pub(crate) share: (f64, f64),
    pub(crate) evidence: Vec<Evidence>,
    /// Why it may be covered more than measured.
    pub(crate) notes: Vec<String>,
}

/// The summed capacity of the sources whose effect meets the footprint,
/// and the missing values of those surely contributing.
pub(crate) struct Summed {
    pub(crate) lower: f64,
    /// The upper end over the contributions read: there is none where one
    /// cannot be read (`unread`).
    pub(crate) upper: f64,
    /// How many contributions cannot be read, the sources that cannot be
    /// placed included.
    pub(crate) unread: usize,
    pub(crate) evidence: Vec<Evidence>,
    /// Why it cannot be decided, in order.
    pub(crate) unknown: Vec<String>,
    /// The sources surely contributing.
    pub(crate) sure: Vec<ObjectId>,
    /// Each surely contributing source's missing value: the source, the
    /// property, the finding's words and the evidence of the absence.
    pub(crate) missing: Vec<(ObjectId, String, String, Vec<Evidence>)>,
}

pub(crate) struct Element<'s, 'a> {
    pub(crate) context: &'s RuleContext<'a>,
    pub(crate) setting: &'s Setting<'s>,
    pub(crate) services: &'s Services<'a>,
    pub(crate) around: Around<'s>,
    pub(crate) object: &'s Object,
}

impl Element<'_, '_> {
    /// The element's area, request and coverage, measured in the order the
    /// capability measured them: the stated area first.
    pub(crate) fn measure(&self) -> Result<Measured, Unmeasured> {
        let stated = match self.setting.area {
            None => None,
            Some(property) => match self.read(&self.object.id, property, true) {
                Read::Value(value, evidence) => Some((value, evidence)),
                Read::Missing(message, evidence) => {
                    return Err(Unmeasured::Missing(message, evidence));
                }
                Read::Unknown(why) => {
                    return Err(Unmeasured::Unavailable((
                        NotEvaluatedReason::IncompleteEvidence,
                        why,
                    )));
                }
            },
        };
        let mut asked = self.request()?;
        let coverage = self
            .services
            .areas
            .measure_coverage(&asked.request)
            .map_err(unavailable)?;
        for (source, meets) in coverage.effects() {
            if let EffectMeets::Unmeasured(reason) = meets {
                asked
                    .notes
                    .push(format!("the effect of {source} is unmeasured: {reason}"));
            }
        }
        let area = if let Some((value, evidence)) = stated {
            Area {
                lower: value,
                upper: value,
                evidence,
            }
        } else {
            let footprint = coverage.footprint();
            Area {
                lower: footprint.lower_square_metres(),
                upper: footprint.upper_square_metres(),
                evidence: vec![footprint.evidence().clone()],
            }
        };
        Ok(Measured {
            area,
            asked,
            coverage,
        })
    }

    /// The coverage request, with the element's connections.
    fn request(&self) -> Result<Asked, Unavailable> {
        let own = &self.object.id;
        let mut notes = Vec::new();
        if self.around.blind > 0 {
            notes.push(format!(
                "{} source(s) have no readable extent, so they may cover it",
                self.around.blind
            ));
        }
        let mut sources = Vec::new();
        for source in self.around.reaching {
            let selected = self.around.sources.contains(source);
            let touching = if self.setting.mode == Mode::Touching {
                match self.touches(source) {
                    Some(true) => true,
                    Some(false) => continue,
                    None => false,
                }
            } else {
                true
            };
            sources.push(Participant::new(source.clone(), selected && touching));
        }
        let blockers = self
            .around
            .blocking
            .iter()
            .map(|blocker| {
                let certain = self.around.blockers.contains(blocker);
                Participant::new(blocker.clone(), certain)
            })
            .collect();
        let mut request = CoverageRequest::try_new(
            own.clone(),
            self.setting.mode.reach(),
            self.setting.range,
            sources,
            blockers,
        )
        .map_err(unavailable)?;
        let mut open = self.around.blind > 0;
        let mut evidence = Vec::new();
        if let Some(index) = self.around.index {
            let (joined, unknown) = index.connections(own);
            if !unknown.is_empty() {
                open = true;
                notes.extend(
                    unknown
                        .into_iter()
                        .map(|why| format!("a door or opening may join more spaces: {why}")),
                );
            }
            let (mut spaces, mut passages) = (Vec::new(), Vec::new());
            for connection in joined {
                if connection.certain {
                    evidence.extend(connection.evidence);
                }
                spaces.push(Participant::new(connection.space, connection.certain));
                passages.push(Participant::new(connection.via, connection.certain));
            }
            request = request
                .with_connections(spaces, passages)
                .map_err(unavailable)?;
        }
        Ok(Asked {
            request,
            notes,
            open,
            evidence,
        })
    }

    /// Whether a source's footprint touches the element's: `None` when the
    /// measured distance straddles the tolerance or cannot be measured.
    fn touches(&self, source: &ObjectId) -> Option<bool> {
        let request = ProximityRequest::projected(
            self.object.id.clone(),
            source.clone(),
            ProximityProjection::Horizontal,
        )
        .ok()?;
        let (lower, upper) = self
            .services
            .proximity
            .measure_distance(&request)
            .ok()?
            .interval_metres();
        if upper <= self.setting.touch {
            Some(true)
        } else if lower > self.setting.touch {
            Some(false)
        } else {
            None
        }
    }

    /// The covered area and its share of the element's area, as the share
    /// check reads them.
    pub(crate) fn covered(&self, measured: &Measured) -> Coverage {
        let (area, asked) = (&measured.area, &measured.asked);
        let footprint = measured.coverage.footprint();
        let covered = measured.coverage.covered();
        let (lower, mut upper) = (covered.lower_square_metres(), covered.upper_square_metres());
        if asked.open {
            upper = footprint.upper_square_metres();
        }
        let lower = if self.around.blind_blockers == 0 {
            lower
        } else {
            0.0
        };
        let mut evidence: Vec<Evidence> =
            measured.coverage.evidence().into_iter().cloned().collect();
        if self.setting.area.is_some() {
            evidence.extend(area.evidence.iter().cloned());
        }
        evidence.extend(asked.evidence.iter().cloned());
        let mut notes = asked.notes.clone();
        if self.around.blind_blockers > 0 {
            notes.push(format!(
                "{} blocker(s) have no readable extent, so they may block it",
                self.around.blind_blockers
            ));
        }
        Coverage {
            covered: (lower, upper),
            share: ratio((lower, upper), (area.lower, area.upper)),
            evidence,
            notes,
        }
    }

    /// The summed capacity of the sources whose effect meets the footprint,
    /// each times its multiplier, and the missing values of those surely
    /// contributing.
    pub(crate) fn capacity(&self, measured: &Measured, capacity: Capacity<'_>) -> Summed {
        let (asked, area) = (&measured.asked, &measured.area);
        let mut summed = Summed {
            lower: 0.0,
            upper: 0.0,
            unread: usize::from(asked.open),
            evidence: area.evidence.clone(),
            unknown: Vec::new(),
            sure: contributing(&asked.request, &measured.coverage, true),
            missing: Vec::new(),
        };
        for source in contributing(&asked.request, &measured.coverage, false) {
            let certain = summed.sure.contains(&source);
            let factor = match capacity.multiplier {
                Multiplier::Constant(multiplier) => Read::Value(multiplier, Vec::new()),
                Multiplier::Property(property) => self.read(&source, property, false),
            };
            match (self.read(&source, capacity.property, false), factor) {
                (Read::Value(value, found), Read::Value(factor, cited)) => {
                    summed.evidence.extend(found);
                    summed.evidence.extend(cited);
                    summed.upper += value * factor;
                    if certain {
                        summed.lower += value * factor;
                    }
                }
                (value, factor) => {
                    summed.unread += 1;
                    for (read, property) in [
                        (value, capacity.property),
                        (
                            factor,
                            match capacity.multiplier {
                                Multiplier::Property(property) => property,
                                Multiplier::Constant(_) => capacity.property,
                            },
                        ),
                    ] {
                        match read {
                            Read::Value(..) => {}
                            Read::Missing(message, cited) => {
                                if certain {
                                    summed.missing.push((
                                        source.clone(),
                                        property.to_string(),
                                        message.clone(),
                                        cited,
                                    ));
                                }
                                summed.unknown.push(message);
                            }
                            Read::Unknown(why) => summed.unknown.push(why),
                        }
                    }
                }
            }
        }
        summed
    }

    /// A non-negative number an object states: a number, or a quantity in
    /// its SI unit, which must be an `area`.
    pub(crate) fn read(&self, holder: &ObjectId, property: PropertyRef<'_>, area: bool) -> Read {
        let Some(object) = self.context.project.object(holder) else {
            return Read::Unknown(format!("{holder} is not in the project"));
        };
        let resolved = match resolve(self.context, object, property) {
            Ok(resolved) => resolved,
            Err((_, message)) => {
                return Read::Unknown(format!("{property} of {holder}: {message}"));
            }
        };
        let cited = resolved.evidence();
        let value = match &resolved {
            Resolved::Absent(_) => None,
            Resolved::Present(stated) => Some(&stated.value),
        };
        if undefined(value) {
            let what = if *holder == self.object.id {
                "its".to_owned()
            } else {
                format!("{holder}'s")
            };
            return Read::Missing(
                format!("missing value: {what} {property} is not stated"),
                cited,
            );
        }
        let number = match value {
            Some(PropertyValue::Integer(value)) => crate::support::exact_f64(*value),
            Some(PropertyValue::Decimal(value)) => Some(*value),
            Some(PropertyValue::Quantity { value, dimension })
                if !area || *dimension == QuantityDimension::Area =>
            {
                Some(*value)
            }
            _ => None,
        }
        .filter(|value| value.is_finite() && *value >= 0.0);
        match number {
            Some(number) => Read::Value(number, cited),
            None => Read::Unknown(format!(
                "{holder} states no non-negative {}{property} ({})",
                if area { "area " } else { "" },
                display(value)
            )),
        }
    }
}

/// The sources whose effect meets the footprint: surely and certainly, or
/// possibly (surely, possibly or unmeasured, certain or not).
pub(crate) fn contributing(
    request: &CoverageRequest,
    measured: &CoverageEvidence,
    sure: bool,
) -> Vec<ObjectId> {
    request
        .sources()
        .iter()
        .zip(measured.effects())
        .filter(|(participant, (_, meets))| {
            if sure {
                participant.is_certain() && *meets == EffectMeets::Surely
            } else {
                *meets != EffectMeets::No
            }
        })
        .map(|(participant, _)| participant.object().clone())
        .collect()
}

/// `part / whole` over intervals, within `[0, 1]`.
fn ratio(part: (f64, f64), whole: (f64, f64)) -> (f64, f64) {
    let lower = if whole.1 > 0.0 { part.0 / whole.1 } else { 0.0 };
    let upper = if whole.0 > 0.0 { part.1 / whole.0 } else { 1.0 };
    (lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0))
}
