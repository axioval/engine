//! Source-neutral containment: inner elements inside outer elements, their
//! cover to the outer element's faces, and how many each outer element holds.
//!
//! ADR 0004: the proximity service measures the volume two bodies share and
//! the signed distance from a body to a class of another body's faces; this
//! capability decides what those mean against the rule's ratio and bands.
//!
//! - **Contained.** An inner element (the rule's selection) lies in an outer
//!   element (`counterparts`) when the volume they share is at least
//!   `minimum_volume_ratio` of the smaller body's volume. Volumes are
//!   certified intervals, so the ratio is too: contained when its lower
//!   bound reaches the ratio, not contained when its upper bound falls
//!   short, undecided otherwise. With `combine_adjacent`, outer elements
//!   whose surfaces meet are also taken together: the column at a wall
//!   junction lies in the two walls combined.
//! - **Cover.** Each `cover` row bounds the signed distance from an inner
//!   element to one class of its outer element's faces (top, side, bottom,
//!   any), from `inside` (a cover, the distance the whole body keeps from
//!   the faces) or `outside` (a protrusion, how far the body reaches past
//!   them).
//! - **Counts.** `minimum_count` and `maximum_count` bound the inner
//!   elements each outer element holds; `report_orphans` reports an inner
//!   element that lies in none.
//!
//! Every verdict is three-valued: an undecided containment, an unmeasured
//! volume or a straddling interval is reported not evaluated, and a count
//! is judged only when the undecided inner elements cannot change it.
//!
//! It runs as a template ([`axioval_engine::template`]) over the measured
//! `containment_items` of each inner element (the capability's own
//! placement and cover reading, `assess`), the template judging each cover
//! distance against its band and each outer element's count, of the
//! project's `containment_counts`, against the bounds.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, FaceClass, FaceDistanceError,
    FaceDistanceEvidence, FaceDistanceRequest, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, ProximityEvidence, ProximityServiceHandle, RuleCapability, RuleContext,
    TableColumn, VolumeInterval,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, ObjectId};

use crate::clash::measure;
use crate::pairs::{Prepared, Unevaluated, fidelity_note};
use crate::support::{Parameters, Unavailable, invalid};

mod items;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use items::ContainmentItems;

/// Checks that inner elements lie in outer elements, keep their cover to
/// the outer element's faces, and are held in the declared numbers.
///
/// It runs as a template ([`axioval_engine::template`]) over the measured
/// `containment_items` of each inner element and `containment_counts` of
/// the project.
pub struct Containment;

const COVER_COLUMNS: &[TableColumn] = &[
    TableColumn::required("faces", ColumnKind::String),
    TableColumn::optional("side", ColumnKind::String),
    TableColumn::optional("minimum_metres", ColumnKind::Number),
    TableColumn::optional("maximum_metres", ColumnKind::Number),
];

/// Which way a cover band measures.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    /// The distance the body keeps inside the faces.
    Inside,
    /// How far the body reaches past the faces.
    Outside,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Outside => "outside",
        }
    }
}

pub(crate) struct Band {
    faces: FaceClass,
    side: Side,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
}

pub(crate) struct Declaration {
    ratio: f64,
    combine: bool,
    pub(crate) bands: Vec<Band>,
    pub(crate) minimum_count: Option<usize>,
    pub(crate) maximum_count: Option<usize>,
    report_orphans: bool,
}

pub(crate) fn declaration(rule: &CompiledRule) -> Result<Declaration, Unavailable> {
    let parameters = Parameters(rule);
    let ratio = parameters
        .number("minimum_volume_ratio")?
        .ok_or_else(|| invalid("`minimum_volume_ratio` is required"))?;
    if !(ratio > 0.0 && ratio <= 1.0) {
        return Err(invalid(
            "`minimum_volume_ratio` must be greater than zero and at most one",
        ));
    }
    let mut bands = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, row) in parameters
        .table("cover")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let faces = row.text("faces")?.unwrap_or_default();
        let faces = FaceClass::parse(faces).ok_or_else(|| {
            invalid(format!(
                "cover row {index}: `faces` must be `top`, `side`, `bottom` or `any`, not `{faces}`"
            ))
        })?;
        let side = match row.text("side")? {
            None | Some("inside") => Side::Inside,
            Some("outside") => Side::Outside,
            Some(other) => {
                return Err(invalid(format!(
                    "cover row {index}: `side` must be `inside` or `outside`, not `{other}`"
                )));
            }
        };
        let (minimum, maximum) = (row.number("minimum_metres")?, row.number("maximum_metres")?);
        if minimum.is_none() && maximum.is_none() {
            return Err(invalid(format!(
                "cover row {index} declares neither `minimum_metres` nor `maximum_metres`"
            )));
        }
        if minimum.into_iter().chain(maximum).any(|value| value < 0.0) {
            return Err(invalid(format!(
                "cover row {index}: a distance must not be negative"
            )));
        }
        if let (Some(minimum), Some(maximum)) = (minimum, maximum)
            && minimum > maximum
        {
            return Err(invalid(format!(
                "cover row {index}: `minimum_metres` exceeds `maximum_metres`"
            )));
        }
        if !seen.insert((faces, side)) {
            return Err(invalid(format!(
                "cover row {index} repeats the {} faces from {}",
                faces.name(),
                side.name()
            )));
        }
        bands.push(Band {
            faces,
            side,
            minimum,
            maximum,
        });
    }
    let count = |name: &str| -> Result<Option<usize>, Unavailable> {
        parameters
            .integer(name)?
            .map(|value| {
                usize::try_from(value)
                    .map_err(|_| invalid(format!("`{name}` must not be negative")))
            })
            .transpose()
    };
    let (minimum_count, maximum_count) = (count("minimum_count")?, count("maximum_count")?);
    if let (Some(minimum), Some(maximum)) = (minimum_count, maximum_count)
        && minimum > maximum
    {
        return Err(invalid("`minimum_count` exceeds `maximum_count`"));
    }
    let report_orphans = parameters.boolean("report_orphans")?.unwrap_or(false);
    if bands.is_empty() && minimum_count.is_none() && maximum_count.is_none() && !report_orphans {
        return Err(invalid(
            "no cover, count or orphan check is declared: nothing is checked",
        ));
    }
    Ok(Declaration {
        ratio,
        combine: parameters.boolean("combine_adjacent")?.unwrap_or(false),
        bands,
        minimum_count,
        maximum_count,
        report_orphans,
    })
}

/// A three-valued answer with the reason it is undecided.
#[derive(Clone)]
enum Verdict {
    Yes,
    No,
    Unknown(NotEvaluatedReason, String),
}

/// One measured pair, oriented inner to outer.
struct Link {
    outer: ObjectId,
    measured: Result<ProximityEvidence, Unavailable>,
    /// Whether the inner element was the measurement's subject.
    inner_is_subject: bool,
}

/// The volumes of a link, oriented: shared, inner, outer.
type Volumes = (VolumeInterval, VolumeInterval, VolumeInterval);

impl Link {
    fn volumes(&self) -> Option<Volumes> {
        let volume = self.measured.as_ref().ok()?.intersection_volume()?;
        Some(if self.inner_is_subject {
            (volume.shared(), volume.subject(), volume.counterpart())
        } else {
            (volume.shared(), volume.counterpart(), volume.subject())
        })
    }

    fn evidence(&self) -> Vec<Evidence> {
        self.measured
            .as_ref()
            .map(|measured| vec![measured.evidence().clone()])
            .unwrap_or_default()
    }

    /// Whether the inner element lies in this one outer element.
    fn contained(&self, ratio: f64) -> Verdict {
        let measured = match &self.measured {
            Ok(measured) => measured,
            Err((reason, message)) => return Verdict::Unknown(reason.clone(), message.clone()),
        };
        let Some(volume) = measured.intersection_volume() else {
            return Verdict::Unknown(
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the volume shared with {} is not measured, so whether it lies inside cannot be decided",
                    self.outer
                ),
            );
        };
        let (lower, upper) = volume.ratio_of_smaller();
        judge_ratio(lower, upper, ratio, &self.outer.to_string(), measured)
    }
}

fn judge_ratio(
    lower: f64,
    upper: f64,
    ratio: f64,
    outer: &str,
    measured: &ProximityEvidence,
) -> Verdict {
    if lower >= ratio {
        Verdict::Yes
    } else if upper < ratio {
        Verdict::No
    } else {
        Verdict::Unknown(
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "whether it lies in {outer} cannot be decided: the shared volume is {:.4} to {:.4} of the smaller body, ratio {ratio:.4}{}",
                lower,
                upper,
                fidelity_note(measured.fidelity())
            ),
        )
    }
}

/// Bounds on `part / whole` for a part within the whole, outward rounded
/// and clamped to `[0, 1]`.
fn share(part: (f64, f64), whole: (f64, f64)) -> (f64, f64) {
    let lower = if whole.1 > 0.0 && whole.1.is_finite() {
        (part.0 / whole.1).next_down()
    } else {
        0.0
    };
    let upper = if whole.0 > 0.0 {
        (part.1 / whole.0).next_up()
    } else {
        1.0
    };
    let upper = upper.clamp(0.0, 1.0);
    (lower.clamp(0.0, upper), upper)
}

/// Whether two outer elements meet: `(surely, possibly)`.
fn adjacency(measured: &Result<ProximityEvidence, Unavailable>) -> (bool, bool) {
    match measured {
        Err(_) => (false, true),
        Ok(measured) => {
            let (lower, upper) = measured.separation_interval_metres();
            let crossing = measured
                .penetration_metres()
                .is_some_and(|depth| depth > measured.fidelity().deviation_metres());
            (upper == 0.0 || crossing, lower == 0.0)
        }
    }
}

/// The connected components of `members` under `edges`.
fn components(members: &[usize], edges: &BTreeSet<(usize, usize)>) -> Vec<Vec<usize>> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for &start in members {
        if !seen.insert(start) {
            continue;
        }
        let mut component = vec![start];
        let mut next = 0;
        while next < component.len() {
            let at = component[next];
            next += 1;
            for &other in members {
                let key = (at.min(other), at.max(other));
                if !seen.contains(&other) && edges.contains(&key) {
                    seen.insert(other);
                    component.push(other);
                }
            }
        }
        component.sort_unstable();
        found.push(component);
    }
    found
}

/// Pairwise measurements between outer elements, measured once each.
struct OuterPairs<'s> {
    service: &'s ProximityServiceHandle,
    measured: BTreeMap<(ObjectId, ObjectId), Result<ProximityEvidence, Unavailable>>,
}

impl OuterPairs<'_> {
    fn get(&mut self, a: &ObjectId, b: &ObjectId) -> &Result<ProximityEvidence, Unavailable> {
        let key = if a < b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        let service = self.service;
        self.measured
            .entry(key)
            .or_insert_with_key(|(first, second)| measure(service, first, second))
    }
}

/// How an inner element relates to the outer elements.
struct Placement {
    /// Outer elements it surely lies in, alone.
    inside: Vec<usize>,
    /// Outer elements it may lie in, alone, with why it is undecided.
    undecided: Vec<(usize, NotEvaluatedReason, String)>,
    /// The combination of adjacent outer elements it lies in, when no single
    /// one holds it.
    combined: Option<Vec<usize>>,
    /// Whether a combination might hold it, and why that is undecided.
    combination_undecided: Option<(NotEvaluatedReason, String)>,
}

impl Declaration {
    /// The combined verdict of one component of adjacent outer elements.
    fn combined(&self, links: &[Link], component: &[usize], pairs: &mut OuterPairs<'_>) -> Verdict {
        let names = component
            .iter()
            .map(|&index| links[index].outer.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        // The inner body's volume: every reading holds the true value.
        let mut inner = (0.0_f64, f64::INFINITY);
        let (mut shared_low, mut shared_sum_low, mut shared_high) = (0.0_f64, 0.0_f64, 0.0_f64);
        let (mut outer_low, mut outer_sum_low, mut outer_high) = (0.0_f64, 0.0_f64, 0.0_f64);
        for &index in component {
            let Some((shared, own, outer)) = links[index].volumes() else {
                // An unmeasured member may share anything and be any size.
                shared_high = f64::INFINITY;
                outer_high = f64::INFINITY;
                continue;
            };
            inner = (
                inner.0.max(own.lower_cubic_metres()),
                inner.1.min(own.upper_cubic_metres()),
            );
            shared_low = shared_low.max(shared.lower_cubic_metres());
            shared_sum_low += shared.lower_cubic_metres();
            shared_high += shared.upper_cubic_metres();
            outer_low = outer_low.max(outer.lower_cubic_metres());
            outer_sum_low += outer.lower_cubic_metres();
            outer_high += outer.upper_cubic_metres();
        }
        if !inner.1.is_finite() {
            return Verdict::Unknown(
                NotEvaluatedReason::IncompleteEvidence,
                format!("its volume is not measured against {names}"),
            );
        }
        // What two members share is counted twice in the sums.
        let mut overlap = 0.0;
        for (position, &a) in component.iter().enumerate() {
            for &b in &component[position + 1..] {
                match pairs.get(&links[a].outer, &links[b].outer) {
                    Ok(measured) => match measured.intersection_volume() {
                        Some(volume) => overlap += volume.shared().upper_cubic_metres(),
                        None => overlap = f64::INFINITY,
                    },
                    Err(_) => overlap = f64::INFINITY,
                }
            }
        }
        let shared = (
            shared_low.max(shared_sum_low - overlap),
            shared_high.min(inner.1),
        );
        let outer = (outer_low.max(outer_sum_low - overlap), outer_high);
        let smaller = (inner.0.min(outer.0), inner.1.min(outer.1));
        let (lower, upper) = share(shared, smaller);
        if lower >= self.ratio {
            Verdict::Yes
        } else if upper < self.ratio {
            Verdict::No
        } else {
            Verdict::Unknown(
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether it lies in {names} combined cannot be decided: the shared volume is {lower:.4} to {upper:.4} of the smaller body, ratio {:.4}",
                    self.ratio
                ),
            )
        }
    }

    fn place(&self, links: &[Link], pairs: &mut OuterPairs<'_>) -> Placement {
        let mut placement = Placement {
            inside: Vec::new(),
            undecided: Vec::new(),
            combined: None,
            combination_undecided: None,
        };
        for (index, link) in links.iter().enumerate() {
            match link.contained(self.ratio) {
                Verdict::Yes => placement.inside.push(index),
                Verdict::No => {}
                Verdict::Unknown(reason, message) => {
                    placement.undecided.push((index, reason, message));
                }
            }
        }
        if !self.combine || !placement.inside.is_empty() {
            return placement;
        }
        // Outer elements that may share volume with the inner one.
        let members: Vec<usize> = (0..links.len())
            .filter(|&index| {
                links[index]
                    .volumes()
                    .is_none_or(|(shared, _, _)| shared.upper_cubic_metres() > 0.0)
            })
            .collect();
        let (mut sure, mut possible) = (BTreeSet::new(), BTreeSet::new());
        for (position, &a) in members.iter().enumerate() {
            for &b in &members[position + 1..] {
                let (meets, may_meet) = adjacency(pairs.get(&links[a].outer, &links[b].outer));
                if meets {
                    sure.insert((a, b));
                }
                if may_meet {
                    possible.insert((a, b));
                }
            }
        }
        // An undecided adjacency is read both ways: a combination holds
        // only when it holds of surely adjacent members, and fails only when
        // it fails however the undecided ones are read.
        let mut undecided = None;
        for component in components(&members, &sure) {
            if component.len() < 2 {
                continue;
            }
            match self.combined(links, &component, pairs) {
                Verdict::Yes => {
                    placement.combined = Some(component);
                    return placement;
                }
                Verdict::No => {}
                Verdict::Unknown(reason, message) => {
                    undecided.get_or_insert((reason, message));
                }
            }
        }
        for component in components(&members, &possible) {
            if component.len() < 2 {
                continue;
            }
            match self.combined(links, &component, pairs) {
                Verdict::No => {}
                Verdict::Yes => {
                    undecided.get_or_insert((
                        NotEvaluatedReason::IncompleteEvidence,
                        "it lies in a combination of outer elements whose adjacency cannot be decided"
                            .to_owned(),
                    ));
                }
                Verdict::Unknown(reason, message) => {
                    undecided.get_or_insert((reason, message));
                }
            }
        }
        placement.combination_undecided = undecided;
        placement
    }
}

fn face_reason(error: FaceDistanceError) -> NotEvaluatedReason {
    match error {
        FaceDistanceError::Unsupported => NotEvaluatedReason::BackendUnavailable,
        FaceDistanceError::InvalidMeasurement
        | FaceDistanceError::EvidenceFidelityMismatch
        | FaceDistanceError::SameObject => NotEvaluatedReason::InvalidEvidence,
        FaceDistanceError::Unavailable
        | FaceDistanceError::NoBody
        | FaceDistanceError::NotClosed
        | FaceDistanceError::NoFaces
        | FaceDistanceError::InexactHost
        | FaceDistanceError::AmbiguousFace => NotEvaluatedReason::IncompleteEvidence,
    }
}

fn metres(lower: f64, upper: f64) -> String {
    #[allow(clippy::float_cmp)]
    if lower == upper {
        format!("{lower:.4} m")
    } else {
        format!("{lower:.4} to {upper:.4} m")
    }
}

/// A cover band's reading of one measured distance: the distance the band
/// bounds and the words of each outcome.
pub(crate) struct Banded {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
    /// Below the minimum, where one is declared.
    pub(crate) below: String,
    /// Above the maximum, where one is declared.
    pub(crate) above: String,
    /// Straddling a bound.
    pub(crate) straddles: String,
}

impl Band {
    /// The distance the band bounds, a protrusion the signed distance's
    /// negation, with the words of each outcome.
    pub(crate) fn read(&self, measured: &FaceDistanceEvidence, outer: &ObjectId) -> Banded {
        let signed = measured.signed();
        let (lower, upper) = match self.side {
            Side::Inside => (signed.lower_metres(), signed.upper_metres()),
            Side::Outside => (-signed.upper_metres(), -signed.lower_metres()),
        };
        let what = match self.side {
            Side::Inside => format!("{} cover to {outer}", self.faces.name()),
            Side::Outside => format!("reach past the {} faces of {outer}", self.faces.name()),
        };
        let value = metres(lower, upper);
        let note = fidelity_note(measured.fidelity());
        Banded {
            lower,
            upper,
            below: self.minimum.map_or_else(String::new, |minimum| {
                format!("{what} is {value}, below the minimum {minimum:.4} m{note}")
            }),
            above: self.maximum.map_or_else(String::new, |maximum| {
                format!("{what} is {value}, above the maximum {maximum:.4} m{note}")
            }),
            straddles: format!(
                "whether the {what} keeps its bounds cannot be decided: it is {value}{note}"
            ),
        }
    }
}

/// Counts of inner elements one outer element holds.
#[derive(Default)]
pub(crate) struct Held {
    pub(crate) sure: BTreeSet<ObjectId>,
    pub(crate) possible: BTreeSet<ObjectId>,
}

impl Held {
    /// The inner elements it surely holds, and every one it may hold.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.sure.len(), self.sure.len() + self.possible.len())
    }
}

/// What one inner element comes to, in the capability's order.
pub(crate) enum Item {
    /// It lies in no outer element (a finding, its words and the evidence
    /// of every link), or why that cannot be decided.
    Orphan(Result<(String, Vec<Evidence>), Unavailable>),
    /// One cover band against one outer element it lies in: the face
    /// distance measured, or why it could not be.
    Band {
        band: usize,
        outer: ObjectId,
        measured: Result<Box<FaceDistanceEvidence>, Unavailable>,
        /// The containment's evidence.
        link: Vec<Evidence>,
    },
    /// Why something of it is not checked.
    Open(Unavailable),
}

/// What the capability reads of a rule's elements: each inner element's
/// items, what each outer element holds, and what the selections and the
/// extents left open.
pub(crate) struct Assessed {
    pub(crate) inner: Vec<(ObjectId, Vec<Item>)>,
    pub(crate) held: BTreeMap<ObjectId, Held>,
    pub(crate) unmeasurable_subjects: BTreeSet<ObjectId>,
    pub(crate) unevaluated: Unevaluated,
}

impl Declaration {
    /// Each cover band's distance to `link`'s outer element, measured once
    /// per class of faces.
    fn cover(&self, service: &ProximityServiceHandle, inner: &ObjectId, link: &Link) -> Vec<Item> {
        let mut measured: BTreeMap<FaceClass, Result<FaceDistanceEvidence, FaceDistanceError>> =
            BTreeMap::new();
        let mut items = Vec::new();
        for (index, band) in self.bands.iter().enumerate() {
            let answer = measured.entry(band.faces).or_insert_with(|| {
                FaceDistanceRequest::try_new(inner.clone(), link.outer.clone(), band.faces)
                    .and_then(|request| service.measure_face_distance(&request))
            });
            items.push(Item::Band {
                band: index,
                outer: link.outer.clone(),
                measured: answer.clone().map(Box::new).map_err(|error| {
                    (
                        face_reason(error),
                        format!(
                            "its {} distance to {} could not be measured: {error}",
                            band.faces.name(),
                            link.outer
                        ),
                    )
                }),
                link: link.evidence(),
            });
        }
        items
    }
}

/// Places every inner element of `prepared` in the outer elements and
/// reads its cover, counting what each outer element holds.
#[allow(clippy::too_many_lines)]
pub(crate) fn assess(declared: &Declaration, prepared: Prepared<'_>) -> Assessed {
    let service = prepared.service;
    let subjects: BTreeSet<&ObjectId> = prepared.subjects.iter().collect();
    // Each pair is measured once and read from whichever end is inner.
    let mut links: BTreeMap<ObjectId, Vec<Link>> = BTreeMap::new();
    for pair in &prepared.pairs {
        let (subject, counterpart) = (pair.subject(), pair.counterpart());
        let measured = measure(service, subject, counterpart);
        let reverse = subjects.contains(counterpart) && prepared.counterparts.contains(subject);
        if reverse {
            links.entry(counterpart.clone()).or_default().push(Link {
                outer: subject.clone(),
                measured: measured.clone(),
                inner_is_subject: false,
            });
        }
        links.entry(subject.clone()).or_default().push(Link {
            outer: counterpart.clone(),
            measured,
            inner_is_subject: true,
        });
    }
    let mut pairs = OuterPairs {
        service,
        measured: BTreeMap::new(),
    };
    let mut held: BTreeMap<ObjectId, Held> = prepared
        .counterparts
        .iter()
        .map(|outer| (outer.clone(), Held::default()))
        .collect();
    // An inner element whose extent is unknown may lie in any outer one.
    for inner in &prepared.unmeasurable_subjects {
        for (outer, count) in &mut held {
            if outer != inner {
                count.possible.insert(inner.clone());
            }
        }
    }
    let unmeasurable_outer = !prepared.unmeasurable_counterparts.is_empty();
    let no_links = Vec::new();
    let mut assessed = Vec::new();
    for inner in &prepared.subjects {
        let mut items = Vec::new();
        let links = links.get(inner).unwrap_or(&no_links);
        let placement = declared.place(links, &mut pairs);
        for &index in &placement.inside {
            if let Some(count) = held.get_mut(&links[index].outer) {
                count.sure.insert(inner.clone());
            }
        }
        for (index, _, _) in &placement.undecided {
            if let Some(count) = held.get_mut(&links[*index].outer) {
                count.possible.insert(inner.clone());
            }
        }
        if let Some(component) = &placement.combined {
            for &index in component {
                let Some(count) = held.get_mut(&links[index].outer) else {
                    continue;
                };
                match links[index].volumes() {
                    Some((shared, _, _)) if shared.lower_cubic_metres() > 0.0 => {
                        count.sure.insert(inner.clone());
                    }
                    _ => {
                        count.possible.insert(inner.clone());
                    }
                }
            }
        }
        if placement.combination_undecided.is_some() {
            for link in links {
                if link
                    .volumes()
                    .is_none_or(|(shared, _, _)| shared.upper_cubic_metres() > 0.0)
                    && let Some(count) = held.get_mut(&link.outer)
                {
                    count.possible.insert(inner.clone());
                }
            }
        }

        let held_somewhere = !placement.inside.is_empty() || placement.combined.is_some();
        let undecided = placement
            .undecided
            .first()
            .map(|(_, reason, message)| (reason.clone(), message.clone()))
            .or_else(|| placement.combination_undecided.clone());
        if declared.report_orphans && !held_somewhere {
            items.push(Item::Orphan(match (&undecided, unmeasurable_outer) {
                (Some(open), _) => Err(open.clone()),
                (None, true) => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    "an outer element could not be measured, so whether this lies in none cannot be decided"
                        .to_owned(),
                )),
                (None, false) => Ok((
                    format!(
                        "lies in no outer element: none shares {:.4} of the smaller body's volume",
                        declared.ratio
                    ),
                    links.iter().flat_map(Link::evidence).collect(),
                )),
            }));
        }
        if !declared.bands.is_empty() {
            for &index in &placement.inside {
                items.extend(declared.cover(service, inner, &links[index]));
            }
            for (index, reason, message) in &placement.undecided {
                items.push(Item::Open((
                    reason.clone(),
                    format!(
                        "its cover to {} is not checked: {message}",
                        links[*index].outer
                    ),
                )));
            }
            if placement.combined.is_some() {
                items.push(Item::Open((
                    NotEvaluatedReason::BackendUnavailable,
                    "it lies only in a combination of outer elements, whose faces are not measured as one body, so its cover is not checked"
                        .to_owned(),
                )));
            } else if placement.inside.is_empty()
                && let Some((reason, message)) = &placement.combination_undecided
            {
                items.push(Item::Open((
                    reason.clone(),
                    format!("its cover is not checked: {message}"),
                )));
            }
        }
        assessed.push((inner.clone(), items));
    }
    Assessed {
        inner: assessed,
        held,
        unmeasurable_subjects: prepared.unmeasurable_subjects,
        unevaluated: prepared.unevaluated,
    }
}

/// The capability's parameters.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("counterparts", ParameterType::Selector),
        ParameterDescriptor::required("minimum_volume_ratio", ParameterType::Number),
        ParameterDescriptor::optional("combine_adjacent", ParameterType::Boolean),
        ParameterDescriptor::optional("cover", ParameterType::Table(COVER_COLUMNS)),
        ParameterDescriptor::optional("minimum_count", ParameterType::Integer),
        ParameterDescriptor::optional("maximum_count", ParameterType::Integer),
        ParameterDescriptor::optional("report_orphans", ParameterType::Boolean),
    ]
}

/// Checks the rule parameters `containment_items` names, as the rule
/// states them: what the capability refused of its declaration, in its
/// order and words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    declaration(&rule).map(|_| ())
}

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for Containment {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_is_rounded_outward_and_open_without_a_whole() {
        let (lower, upper) = share((1.0, 1.0), (3.0, 3.0));
        assert!(lower < 1.0 / 3.0 && upper > 1.0 / 3.0);
        assert_eq!(share((0.5, 0.5), (0.0, f64::INFINITY)), (0.0, 1.0));
    }

    #[test]
    fn components_follow_the_edges() {
        let edges = BTreeSet::from([(0, 1), (2, 3)]);
        assert_eq!(
            components(&[0, 1, 2, 3, 4], &edges),
            vec![vec![0, 1], vec![2, 3], vec![4]]
        );
    }
}
