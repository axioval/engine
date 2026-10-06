//! Coverage and conformity of one set of elements by another: how much of
//! each architectural wall no structural wall stands under, in plan and in
//! height, or in the wall's own elevation.
//!
//! What is uncovered is measured here ([`Subject`]'s shares, read through
//! the measured values of [`CoverageMeasures`]); how serious it is, is the
//! template's policy (`counterpart_coverage/template.rs`).

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ElevationCover, ElevationRequest, NotEvaluatedReason,
    ParameterDescriptor, PlanArea, PlanAreaServiceHandle, PlanRectangle, PlanSpanServiceHandle,
    ProximityServiceHandle, RuleCapability, RuleContext, VerticalExtent, VerticalExtentError,
    VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, Object, ObjectId};

use crate::near::Candidates;
use crate::orientation::{Alignment, Tri, aligned, rectangle, rectangle_service};
use crate::plan_area::unavailable;
use crate::support::Unavailable;

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::CoverageMeasures;

/// Requires each selected element to be covered by its counterparts, in plan
/// and in height, graded by the share left uncovered.
///
/// The counterparts are the objects `counterparts` picks, typically another
/// discipline's elements of matching kinds (a `discipline` selector with an
/// entity type): architectural walls against structural walls.
///
/// - **Plan**: the share of the element's footprint outside the union of the
///   counterparts' footprints, each grown by the horizontal tolerance, through
///   the plan-area service's uncovered area.
/// - **Height**: the share of the element's vertical extent outside the union
///   of the vertical extents, each grown by the vertical tolerance, of the
///   counterparts that overlap it in plan (their footprint, grown by the
///   horizontal tolerance, covers some of the element's).
///
/// Two variants differ only in their tolerances: coverage declares one
/// `tolerance` for both checks; conformity declares `horizontal_tolerance`
/// and `vertical_tolerance` separately. A negative tolerance switches its
/// check off; switching every check off is an invalid declaration.
///
/// An uncovered share above `info_above`, `warning_above` or `error_above`
/// (at least one, ascending in that order, each in `[0, 1)`) is a finding of
/// the most severe band it exceeds; the rule's own severity is not used. A
/// share at or below the lowest declared threshold passes.
///
/// Shares are intervals. A share straddling the lowest threshold is not
/// evaluated; one above it that straddles a higher threshold is graded by
/// the most severe band it may reach, and the message says so. Counterparts
/// the selector cannot decide, or whose extent or footprint cannot be read,
/// can only cover more: a pass stands, anything else is not evaluated.
///
/// With `axis_tolerance`, only axis-compatible counterparts count: those
/// whose long axis (from the least-area rectangle of the footprint) lies
/// within that angle of parallel to the element's. A counterpart surely at
/// another angle is left out; one whose angle straddles the tolerance, or
/// whose axes or the element's are not their own (a square, several
/// least-area rectangles, a tessellated footprint), may count: it can only
/// cover more, so it leaves a finding it could remove not evaluated.
/// Without `axis_tolerance`, a perpendicular wall meeting the element
/// within the horizontal tolerance overlaps it in plan and counts towards
/// its height.
///
/// With `measure` `elevation`, plan and height are one check: the share of
/// the element's elevation (its projection onto the vertical plane along the
/// long axis of its footprint's least-area rectangle) that the counterparts'
/// projections leave uncovered. A counterpart counts by its part within the
/// element's depth across the axis, widened by the horizontal tolerance;
/// its projection grows by the horizontal tolerance along the axis and the
/// vertical tolerance in height. A wall under a full-height counterpart on
/// one half and a half-height one on the other passes plan and height, but
/// a quarter of its face is uncovered. An element without a long axis (a
/// square, several least-area rectangles, a tessellated footprint) is not
/// evaluated; both tolerances must be switched on.
///
/// `infill_counterparts` (elevation only) are the members of a frame, such
/// as columns and beams: when more than `infill_above` (a share in `[0, 1)`,
/// half by default) of the elevation is uncovered by the counterparts, the
/// frame's infill (the convex hull of the members' projections, grown the
/// same way) covers too, so a wall filling a column-and-beam bay passes.
/// Whether the infill applies is decided on the share the counterparts
/// leave; a share straddling `infill_above` is judged from both answers.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `counterpart_uncovered_share` of each check at most the lowest declared
/// threshold, graded into the bands it exceeds.
pub struct CounterpartCoverage;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for CounterpartCoverage {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// What one measurement of coverage is taken with: the counterparts'
/// growths and which checks are on.
pub(crate) struct Config {
    /// Growth of each counterpart's footprint, `None` when the plan check is off.
    pub(crate) horizontal: Option<f64>,
    /// Growth of each counterpart's extent, `None` when the height check is off.
    pub(crate) vertical: Option<f64>,
    /// Largest angle, in degrees, between compatible long axes; `None`
    /// counts counterparts at any angle.
    pub(crate) axis: Option<f64>,
    /// Whether plan and height are measured together in the elevation.
    pub(crate) elevation: bool,
    /// The uncovered share above which a frame's infill covers, where a
    /// frame is declared.
    pub(crate) infill: Option<f64>,
}

impl Config {
    /// How much farther than its own footprint a counterpart may stand in
    /// plan and still cover: the horizontal growth, along and across the
    /// axis in the elevation.
    pub(crate) fn margin(&self) -> f64 {
        self.horizontal.unwrap_or(0.0)
            * if self.elevation {
                std::f64::consts::SQRT_2
            } else {
                1.0
            }
    }
}

pub(crate) struct Services<'a> {
    pub(crate) areas: &'a PlanAreaServiceHandle,
    pub(crate) proximity: &'a ProximityServiceHandle,
    pub(crate) rectangles: Option<&'a PlanSpanServiceHandle>,
}

impl<'a> Services<'a> {
    /// The services `config`'s checks need, in the order the capability
    /// asked for them: the vertical extents where the height is checked
    /// (read where it is measured).
    pub(crate) fn of(context: &RuleContext<'a>, config: &Config) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("{what} service is not registered"),
            )
        };
        let areas = context
            .services
            .get::<PlanAreaServiceHandle>()
            .ok_or_else(|| missing("plan-area"))?;
        let proximity = context
            .services
            .get::<ProximityServiceHandle>()
            .ok_or_else(|| missing("proximity"))?;
        if config.vertical.is_some()
            && !config.elevation
            && context
                .services
                .get::<VerticalExtentServiceHandle>()
                .is_none()
        {
            return Err(missing("vertical-extent"));
        }
        Ok(Self {
            areas,
            proximity,
            rectangles: if config.axis.is_some() || config.elevation {
                Some(rectangle_service(context)?)
            } else {
                None
            },
        })
    }
}

/// The counterparts near each subject.
pub(crate) struct Counterparts {
    pub(crate) candidates: Arc<Candidates>,
    /// Counterparts near each subject, matched or undecided.
    pub(crate) near: BTreeMap<ObjectId, Vec<ObjectId>>,
}

/// Whether a counterpart's grown footprint covers part of the subject's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlap {
    Sure,
    Maybe,
}

/// An uncovered share as measured for one check, before it is graded.
#[derive(Clone)]
pub(crate) struct Share {
    /// The share, from the most cover's to the least cover's.
    pub(crate) interval: (f64, f64),
    /// The part uncovered (an area, or a height), and the whole it is a
    /// share of.
    pub(crate) uncovered: (f64, f64),
    pub(crate) whole: (f64, f64),
    pub(crate) evidence: Vec<Evidence>,
}

/// Counterparts that surely cover part of the subject, and those that may.
#[derive(Clone)]
pub(crate) struct Cover {
    /// Selected and surely overlapping: the least the cover can be.
    pub(crate) least: Vec<ObjectId>,
    /// Selected or undecided, surely or possibly overlapping: the most.
    pub(crate) most: Vec<ObjectId>,
    /// Why the cover may be larger still than `most`.
    pub(crate) unknown: Vec<String>,
    /// How many counterparts beyond `most` may cover: those whose extent
    /// or cover cannot be read.
    pub(crate) unread: usize,
    /// Why counterparts in `most` only may be axis-compatible.
    pub(crate) axes: Vec<String>,
    pub(crate) evidence: Vec<Evidence>,
}

/// Whether, and with which members, a frame's infill covers an elevation.
#[derive(Clone)]
pub(crate) struct Infill {
    /// Whether the uncovered share without it surely, possibly or surely
    /// not exceeds the share above which it covers.
    pub(crate) applies: Tri,
    /// The frame members that may span the infill.
    pub(crate) members: Vec<ObjectId>,
}

pub(crate) struct Subject<'s, 'a> {
    pub(crate) config: &'s Config,
    pub(crate) services: &'s Services<'a>,
    pub(crate) counterparts: &'s Counterparts,
    pub(crate) frame: Option<&'s Counterparts>,
    pub(crate) object: &'s Object,
}

impl Subject<'_, '_> {
    /// Sorts the near counterparts by whether their grown footprint covers
    /// part of the subject's: surely when the subject's uncovered area is
    /// surely smaller than its footprint, not at all when it surely is not.
    pub(crate) fn cover(&self, area: &PlanArea) -> Cover {
        let growth = self.config.horizontal.unwrap_or(0.0);
        let mut cover = Cover {
            least: Vec::new(),
            most: Vec::new(),
            unknown: Vec::new(),
            unread: 0,
            axes: Vec::new(),
            evidence: vec![area.evidence().clone()],
        };
        let own = self.services.rectangles.map(|service| {
            rectangle(service, &self.object.id).map_err(|(_, message)| {
                format!("the axes of {} are unknown: {message}", self.object.id)
            })
        });
        let blind = self.counterparts.candidates.blind.len();
        if blind > 0 {
            cover.unread += blind;
            cover.unknown.push(format!(
                "{blind} counterpart(s) have no readable extent, so they may cover it"
            ));
        }
        let near = self
            .counterparts
            .near
            .get(&self.object.id)
            .map_or(&[][..], Vec::as_slice);
        for counterpart in near {
            let uncovered = match self.services.areas.measure_uncovered_area(
                &self.object.id,
                std::slice::from_ref(counterpart),
                growth,
            ) {
                Ok(uncovered) => uncovered,
                Err(error) => {
                    cover.unread += 1;
                    cover.unknown.push(format!(
                        "whether {counterpart} covers it is unknown: {}",
                        unavailable(error).1
                    ));
                    continue;
                }
            };
            let overlap = if uncovered.upper_square_metres() < area.lower_square_metres() {
                Overlap::Sure
            } else if uncovered.lower_square_metres() >= area.upper_square_metres() {
                continue;
            } else {
                Overlap::Maybe
            };
            let overlap = match (&own, self.config.axis) {
                (Some(own), Some(tolerance)) => {
                    match self.compatible(own, counterpart, tolerance, &mut cover) {
                        Tri::No => continue,
                        Tri::Yes => overlap,
                        Tri::Maybe => Overlap::Maybe,
                    }
                }
                _ => overlap,
            };
            cover.evidence.push(uncovered.evidence().clone());
            let selected = self.counterparts.candidates.matched.contains(counterpart);
            if selected && overlap == Overlap::Sure {
                cover.least.push(counterpart.clone());
            }
            cover.most.push(counterpart.clone());
        }
        cover
    }

    /// Whether `counterpart`'s long axis lies within `tolerance` degrees of
    /// parallel to the subject's.
    fn compatible(
        &self,
        own: &Result<PlanRectangle, String>,
        counterpart: &ObjectId,
        tolerance: f64,
        cover: &mut Cover,
    ) -> Tri {
        let Some(service) = self.services.rectangles else {
            return Tri::Maybe;
        };
        let theirs = rectangle(service, counterpart)
            .map_err(|(_, message)| format!("the axes of {counterpart} are unknown: {message}"));
        let (answer, why) = match (own, &theirs) {
            (Ok(own), Ok(theirs)) => {
                cover
                    .evidence
                    .extend([own.evidence().clone(), theirs.evidence().clone()]);
                aligned(own, theirs, Alignment::Parallel, tolerance)
            }
            (Err(why), _) | (_, Err(why)) => (Tri::Maybe, Some(why.clone())),
        };
        if let Some(why) = why {
            cover.axes.push(why);
        }
        answer
    }

    /// The share of the footprint outside every counterpart grown by
    /// `growth`.
    pub(crate) fn plan_share(
        &self,
        area: &PlanArea,
        cover: &Cover,
        growth: f64,
    ) -> Result<Share, Unavailable> {
        let measure = |objects: &[ObjectId]| {
            self.services
                .areas
                .measure_uncovered_area(&self.object.id, objects, growth)
                .map_err(unavailable)
        };
        let least = measure(&cover.least)?;
        let mut evidence = cover.evidence.clone();
        evidence.push(least.evidence().clone());
        let upper = least.upper_square_metres();
        let mut lower = least.lower_square_metres();
        if cover.most != cover.least {
            let most = measure(&cover.most)?;
            evidence.push(most.evidence().clone());
            lower = most.lower_square_metres();
        }
        if !cover.unknown.is_empty() {
            lower = 0.0;
        }
        let whole = (area.lower_square_metres(), area.upper_square_metres());
        Ok(Share {
            interval: ratio((lower, upper), whole),
            uncovered: (lower, upper),
            whole,
            evidence,
        })
    }

    /// The share of the height outside every counterpart overlapping the
    /// element in plan, grown by `growth`.
    pub(crate) fn height_share(
        &self,
        extents: &VerticalExtentServiceHandle,
        cover: &Cover,
        growth: f64,
    ) -> Result<Share, Unavailable> {
        let measure = |object: &ObjectId| {
            extents
                .measure_vertical_extent(object)
                .map_err(|error| extent_unavailable(&error))
        };
        let own = measure(&self.object.id)?;
        let mut evidence = cover.evidence.clone();
        evidence.push(own.evidence().clone());
        let mut measured: BTreeMap<&ObjectId, VerticalExtent> = BTreeMap::new();
        for counterpart in &cover.most {
            let extent = measure(counterpart)?;
            evidence.push(extent.evidence().clone());
            measured.insert(counterpart, extent);
        }
        let of = |objects: &[ObjectId]| -> Vec<&VerticalExtent> {
            objects.iter().filter_map(|id| measured.get(id)).collect()
        };
        let invalid_growth = |error: VerticalExtentError| extent_unavailable(&error);
        let (_, upper) = own
            .uncovered_height(&of(&cover.least), growth)
            .map_err(invalid_growth)?;
        let (mut lower, _) = own
            .uncovered_height(&of(&cover.most), growth)
            .map_err(invalid_growth)?;
        if !cover.unknown.is_empty() {
            lower = 0.0;
        }
        let height = own.height_metres();
        if height.1 <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} has no height", self.object.id),
            ));
        }
        Ok(Share {
            interval: ratio((lower, upper), height),
            uncovered: (lower, upper),
            whole: height,
            evidence,
        })
    }

    /// The near counterparts of `found` as a cover of the elevation: every
    /// selected one surely, undecided ones and those only possibly
    /// axis-compatible possibly. Axes are compared only when `own` holds
    /// the element's rectangle.
    fn elevation_cover(
        &self,
        found: &Counterparts,
        own: Option<&Result<PlanRectangle, String>>,
    ) -> Cover {
        let mut cover = Cover {
            least: Vec::new(),
            most: Vec::new(),
            unknown: Vec::new(),
            unread: 0,
            axes: Vec::new(),
            evidence: Vec::new(),
        };
        let blind = found.candidates.blind.len();
        if blind > 0 {
            cover.unread += blind;
            cover.unknown.push(format!(
                "{blind} counterpart(s) have no readable extent, so they may cover it"
            ));
        }
        let near = found
            .near
            .get(&self.object.id)
            .map_or(&[][..], Vec::as_slice);
        for counterpart in near {
            let sure = match (own, self.config.axis) {
                (Some(own), Some(tolerance)) => {
                    match self.compatible(own, counterpart, tolerance, &mut cover) {
                        Tri::No => continue,
                        Tri::Yes => true,
                        Tri::Maybe => false,
                    }
                }
                _ => true,
            };
            if sure && found.candidates.matched.contains(counterpart) {
                cover.least.push(counterpart.clone());
            }
            cover.most.push(counterpart.clone());
        }
        cover
    }

    /// The uncovered share of the element's elevation, the cover it was
    /// measured against, and the frame's infill where one is declared and
    /// may apply.
    pub(crate) fn elevation_share(&self) -> Result<(Share, Cover, Option<Infill>), Unavailable> {
        let (Some(rectangles), Some(along), Some(vertical)) = (
            self.services.rectangles,
            self.config.horizontal,
            self.config.vertical,
        ) else {
            return Err(crate::support::invalid(
                "the elevation is measured with both growths; neither may be negative",
            ));
        };
        let own = rectangle(rectangles, &self.object.id)?;
        let axis = own.long_axis().map_err(|why| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "elevation: {} has no long axis to measure its elevation along: {why}",
                    self.object.id
                ),
            )
        })?;
        let axis = own.axes()[axis];
        let mut cover = self.elevation_cover(self.counterparts, Some(&Ok(own.clone())));
        cover.evidence.push(own.evidence().clone());
        let measure = |objects: &[ObjectId], frame: &[ObjectId]| {
            ElevationRequest::try_new(
                self.object.id.clone(),
                axis,
                objects,
                frame,
                along,
                vertical,
            )
            .and_then(|request| self.services.areas.measure_elevation_cover(&request))
            .map_err(unavailable)
        };
        // The area left by the least cover bounds it from above, the area
        // left by the most from below, and something unread may cover all.
        let share = |least: &ElevationCover, most: &ElevationCover, unknown: bool| {
            let upper = least.uncovered_square_metres().1;
            let lower = if unknown {
                0.0
            } else {
                most.uncovered_square_metres().0
            };
            (
                (lower, upper),
                ratio((lower, upper), least.area_square_metres()),
            )
        };
        let least = measure(&cover.least, &[])?;
        let most = if cover.most == cover.least {
            least.clone()
        } else {
            measure(&cover.most, &[])?
        };
        cover
            .evidence
            .extend([least.evidence().clone(), most.evidence().clone()]);
        let area = least.area_square_metres();
        let (mut uncovered, mut shares) = share(&least, &most, !cover.unknown.is_empty());
        let mut framed = None;
        if let (Some(frame), Some(above)) = (self.frame, self.config.infill) {
            let members = self.elevation_cover(frame, None);
            let applies = Tri::of(shares.0 > above, shares.1 <= above);
            if applies != Tri::No && !(members.most.is_empty() && members.unknown.is_empty()) {
                let least_framed = measure(&cover.least, &members.least)?;
                let most_framed = measure(&cover.most, &members.most)?;
                cover.evidence.extend([
                    least_framed.evidence().clone(),
                    most_framed.evidence().clone(),
                ]);
                let unknown = !cover.unknown.is_empty() || !members.unknown.is_empty();
                let (frame_uncovered, frame_shares) = share(&least_framed, &most_framed, unknown);
                cover.unknown.extend(members.unknown.iter().cloned());
                cover.unread += members.unread;
                if applies == Tri::Yes {
                    (uncovered, shares) = (frame_uncovered, frame_shares);
                    cover.least.extend(members.least.iter().cloned());
                } else {
                    uncovered.0 = frame_uncovered.0;
                    shares.0 = frame_shares.0;
                }
                framed = Some(Infill {
                    applies,
                    members: members.most,
                });
            }
        }
        let evidence = cover.evidence.clone();
        Ok((
            Share {
                interval: shares,
                uncovered,
                whole: area,
                evidence,
            },
            cover,
            framed,
        ))
    }
}

/// `part / whole` over intervals, within `[0, 1]`.
fn ratio(part: (f64, f64), whole: (f64, f64)) -> (f64, f64) {
    let lower = if whole.1 > 0.0 { part.0 / whole.1 } else { 0.0 };
    let upper = if whole.0 > 0.0 { part.1 / whole.0 } else { 1.0 };
    (lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0))
}

fn extent_unavailable(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}
