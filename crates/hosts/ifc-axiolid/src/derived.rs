//! Copies a derived source keeps of another source's objects, matched and
//! declared superseded (#390).
//!
//! A model is often derived from another: a discipline model exported from
//! the architectural one keeps copies of its walls and slabs, and one
//! authoring model exported several times with different exchange settings
//! gives several files of the same objects. Federated, every rule would
//! see each such object twice. The host declares which source is derived
//! from which ([`crate::Options::with_derived_from`]); this module matches
//! each derived object to its original and hands the matched pairs to the
//! session as supersessions (`EvidenceSession::with_superseded`), so the
//! engine evaluates each original once.
//!
//! Matching is by external id first: an object of the derived source with
//! the original's `GlobalId` and class is its copy, but only if its body is
//! the original's within the tolerance. With [`Matching::ByIdOrGeometry`],
//! an object with no such counterpart is paired by geometry alone: same
//! class, the same body within the tolerance, and the only candidate on
//! either side. Bodies are compared in the shared world frame through the
//! registered proximity service, never by representation or triangles,
//! since another exchange setting may state one body as a swept solid, a
//! B-rep or triangles ([`Fingerprint`]).
//!
//! The safe side is double reporting, never hiding: any object whose
//! identity with an original cannot be established (a different or moved
//! body, missing or unmeasured geometry on either side, an unaligned
//! frame, an open interval, several candidates) stays evaluated, counted
//! by cause ([`Unmatched`]).

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::Arc;

use axioval_engine::{
    EvidenceSession, ProximityError, ProximityServiceHandle, SurfaceDistanceRequest,
};
use axioval_ir::{ObjectId, SourceId, Superseded, SupersessionKind, SupersessionReason};

/// How a derived source's objects are matched to the original's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Matching {
    /// By external id (`GlobalId`) and class, the body confirmed.
    #[default]
    ById,
    /// By external id as [`Self::ById`], and an object with no such
    /// counterpart by geometry alone: the same class and body, and the
    /// only candidate on either side. For exports that do not keep ids.
    ByIdOrGeometry,
}

/// How a derived object was matched to its original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum MatchedBy {
    /// The same external id and class, the body confirmed.
    Id,
    /// The same class and body, the only candidate on either side.
    Geometry,
}

impl MatchedBy {
    /// The supersession kind the session records.
    #[must_use]
    pub fn kind(self) -> SupersessionKind {
        match self {
            Self::Id => SupersessionKind::ById,
            Self::Geometry => SupersessionKind::ByGeometry,
        }
    }

    /// `by-id` or `by-geometry`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Id => "by-id",
            Self::Geometry => "by-geometry",
        }
    }
}

/// A derived object superseded by its original.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct DerivedMatch {
    /// The copy, of the derived source; no longer evaluated on its own.
    pub derived: ObjectId,
    /// The original, evaluated in its place.
    pub original: ObjectId,
    /// How the two were matched.
    pub matched: MatchedBy,
}

/// Why an object of a derived source stays evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Unmatched {
    /// The original source holds no object with its external id (and, by
    /// geometry, none with its body).
    NoCounterpart,
    /// The object with its external id is of another class.
    DifferentClass,
    /// It or its counterpart occupies no material (a storey, a space
    /// without a body): there is no body to confirm the match with.
    NoBody,
    /// It or its counterpart could not be measured, or lies in a frame not
    /// known to be the other's.
    Unmeasured,
    /// The bodies differ beyond the tolerance: moved or reshaped.
    DifferentBody,
    /// The measurements leave open whether the bodies are one.
    Undecided,
    /// By geometry, several objects of one side match it or its candidate.
    Ambiguous,
}

impl Unmatched {
    /// The cause as the result names it, such as `different-body`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCounterpart => "no-counterpart",
            Self::DifferentClass => "different-class",
            Self::NoBody => "no-body",
            Self::Unmeasured => "unmeasured",
            Self::DifferentBody => "different-body",
            Self::Undecided => "undecided",
            Self::Ambiguous => "ambiguous",
        }
    }
}

/// The tolerance a derived source's bodies are matched within, in metres,
/// unless the host states one: a millimetre, the bridge's chord budget.
pub const DEFAULT_TOLERANCE_METRES: f64 = 0.001;

/// Floating-point slack on every comparison, in metres.
const ROUNDING: f64 = 1e-9;

/// One declaration: `derived` was derived from `original`.
pub(crate) type Declaration = (SourceId, SourceId);

/// Refuses declarations naming a source the session does not hold, a
/// source derived from itself or from two sources, and chains (a derived
/// source another one is derived from).
pub(crate) fn validate(
    session: &EvidenceSession,
    declared: &[Declaration],
    tolerance: f64,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(format!(
            "the derived-source tolerance must be finite and not negative, not {tolerance}"
        )
        .into());
    }
    let mut derived: BTreeSet<&SourceId> = BTreeSet::new();
    for (copy, original) in declared {
        for source in [copy, original] {
            if session.snapshot(source).is_none() {
                return Err(format!(
                    "derived-source declaration names `{source}`, which the session does not hold"
                )
                .into());
            }
        }
        if copy == original {
            return Err(format!("`{copy}` is declared derived from itself").into());
        }
        if !derived.insert(copy) {
            return Err(format!("`{copy}` is declared derived from two sources").into());
        }
    }
    if let Some((_, original)) = declared
        .iter()
        .find(|(_, original)| derived.contains(original))
    {
        return Err(format!(
            "`{original}` is declared derived and also the original of another source; declare each derived source from the original directly"
        )
        .into());
    }
    Ok(())
}

/// What the proximity service measures of one body, in the session's world
/// frame: its box, the volume it encloses and its area, with the deviation
/// its mesh may lie from the true surface.
///
/// Nothing here depends on how the body is stated or triangulated: two
/// exports stating one body as a swept solid and as triangles give the same
/// fingerprint within their deviations.
#[derive(Clone, Copy, Debug)]
struct Fingerprint {
    min: [f64; 3],
    max: [f64; 3],
    /// How far the true surface may lie from the mesh.
    deviation: f64,
    /// The certified enclosed volume, `None` where it cannot be bounded (an
    /// open surface).
    volume: Option<(f64, f64)>,
    /// The mesh's area.
    area: f64,
}

/// Whether two fingerprints show one body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Same,
    Different,
    Undecided,
}

impl Fingerprint {
    fn of(proximity: &ProximityServiceHandle, object: &ObjectId) -> Result<Self, Unmatched> {
        let cause = |error: ProximityError| match error {
            ProximityError::NoBody => Unmatched::NoBody,
            _ => Unmatched::Unmeasured,
        };
        let bounds = proximity.bounds(object).map_err(cause)?;
        let surface = proximity.body_surface(object).map_err(cause)?;
        let positions = surface.positions();
        let area = surface
            .triangles()
            .iter()
            .map(|corners| triangle_area(corners.map(|corner| positions[corner as usize])))
            .sum();
        let volume = proximity.measure_body_volume(object).ok().map(|measured| {
            let volume = measured.volume();
            (volume.lower_cubic_metres(), volume.upper_cubic_metres())
        });
        Ok(Self {
            min: bounds.bounds().min(),
            max: bounds.bounds().max(),
            deviation: bounds.fidelity().deviation_metres(),
            volume,
            area,
        })
    }

    /// Whether `self` and `other` are one body within `tolerance` metres.
    ///
    /// Every corner of the boxes within `d`, the tolerance plus both
    /// deviations; the areas within `2π (a + b + c) d`, what offsetting a
    /// convex body by `d` adds to its area at most (`a`, `b`, `c` the
    /// larger box's extents); the certified volumes within the tolerance
    /// times the larger area. Any beyond proves the bodies apart; a volume
    /// that cannot be bounded leaves it open.
    fn compare(&self, other: &Self, tolerance: f64) -> Verdict {
        let d = tolerance + self.deviation + other.deviation + ROUNDING;
        let corners = (0..3).all(|axis| {
            (self.min[axis] - other.min[axis]).abs() <= d
                && (self.max[axis] - other.max[axis]).abs() <= d
        });
        if !corners {
            return Verdict::Different;
        }
        let extents: f64 = (0..3)
            .map(|axis| (self.max[axis] - self.min[axis]).max(other.max[axis] - other.min[axis]))
            .sum();
        let area = self.area.max(other.area);
        if (self.area - other.area).abs()
            > 2.0 * std::f64::consts::PI * extents * d + ROUNDING * area
        {
            return Verdict::Different;
        }
        let (Some(mine), Some(theirs)) = (self.volume, other.volume) else {
            return Verdict::Undecided;
        };
        let gap = (mine.0.max(theirs.0) - mine.1.min(theirs.1)).max(0.0);
        if gap > tolerance * area + ROUNDING * area.max(1.0) {
            return Verdict::Different;
        }
        Verdict::Same
    }

    /// The grid cell holding the box's lower corner.
    fn cell(&self, size: f64) -> [i64; 3] {
        // Truncation is the point: models lie well within i64 cells.
        #[allow(clippy::cast_possible_truncation)]
        self.min.map(|value| (value / size).floor() as i64)
    }
}

/// The area of the triangle `corners`.
fn triangle_area([first, second, third]: [[f64; 3]; 3]) -> f64 {
    let edge = |to: [f64; 3]| [to[0] - first[0], to[1] - first[1], to[2] - first[2]];
    let (along, across) = (edge(second), edge(third));
    let normal = [
        along[1] * across[2] - along[2] * across[1],
        along[2] * across[0] - along[0] * across[2],
        along[0] * across[1] - along[1] * across[0],
    ];
    normal.iter().map(|value| value * value).sum::<f64>().sqrt() / 2.0
}

/// Confirms a fingerprint match by the two surfaces' certified Hausdorff
/// distance where the service measures one: within the tolerance confirms
/// it, beyond proves the bodies apart, an open interval or a refusal leaves
/// it open. A pair whose distance is not certified (a tessellated mesh
/// without an exact boundary) rests on the fingerprint.
fn confirm(
    proximity: &ProximityServiceHandle,
    copy: &ObjectId,
    original: &ObjectId,
    tolerance: f64,
) -> Verdict {
    let Ok(surface) = proximity.body_surface(original) else {
        return Verdict::Undecided;
    };
    let Ok(request) =
        SurfaceDistanceRequest::try_new(copy.clone(), Arc::new(surface), tolerance / 4.0)
    else {
        return Verdict::Undecided;
    };
    match proximity.measure_surface_distance(&request) {
        Ok(measured) => {
            let distance = measured.distance();
            if distance.upper_metres() <= tolerance + ROUNDING {
                Verdict::Same
            } else if distance.lower_metres() > tolerance + ROUNDING {
                Verdict::Different
            } else {
                Verdict::Undecided
            }
        }
        // Distances between tessellations are not certified: the
        // fingerprint, deviations included, decides.
        Err(ProximityError::EvidenceFidelityMismatch) => Verdict::Same,
        Err(_) => Verdict::Undecided,
    }
}

/// The matches and the unmatched causes of one session's declarations.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) matches: Vec<DerivedMatch>,
    pub(crate) unmatched: BTreeMap<Unmatched, usize>,
}

/// Matches every declared derived source's objects to the original's and
/// declares the matches superseded on `session`.
pub(crate) fn supersede(
    session: EvidenceSession,
    declared: &[Declaration],
    matching: Matching,
    tolerance: f64,
    id_scheme: &str,
) -> Result<(EvidenceSession, Outcome), Box<dyn Error + Send + Sync>> {
    let mut outcome = Outcome::default();
    if declared.is_empty() {
        return Ok((session, outcome));
    }
    let proximity = session
        .service::<ProximityServiceHandle>()
        .ok_or("matching derived sources needs the proximity service")?
        .clone();
    let mut entries = Vec::new();
    for (copy, original) in declared {
        let matched = match_source(
            &session,
            &proximity,
            (copy, original),
            (matching, tolerance, id_scheme),
            &mut outcome.unmatched,
        );
        for (derived, original, by) in matched {
            entries.push(Superseded {
                object: derived.clone(),
                by: original.clone(),
                reason: SupersessionReason::new(
                    by.kind(),
                    format!(
                        "a copy of {original} kept by the derived source `{}`, its body the original's within {tolerance} m",
                        derived.source,
                    ),
                ),
            });
            outcome.matches.push(DerivedMatch {
                derived,
                original,
                matched: by,
            });
        }
    }
    outcome.matches.sort();
    let session = session.with_superseded(entries)?;
    Ok((session, outcome))
}

/// The pairs of one derived source matched to its original, counting every
/// object left evaluated by cause.
fn match_source(
    session: &EvidenceSession,
    proximity: &ProximityServiceHandle,
    (copy_source, original_source): (&SourceId, &SourceId),
    (matching, tolerance, id_scheme): (Matching, f64, &str),
    unmatched: &mut BTreeMap<Unmatched, usize>,
) -> Vec<(ObjectId, ObjectId, MatchedBy)> {
    let external = |object: &axioval_ir::Object| {
        object
            .external_ids
            .iter()
            .find(|id| id.scheme == id_scheme)
            .map(|id| id.value.clone())
    };
    let originals: Vec<&axioval_ir::Object> = session
        .project()
        .objects()
        .filter(|object| &object.id.source == original_source)
        .collect();
    let by_id: BTreeMap<String, &axioval_ir::Object> = originals
        .iter()
        .filter_map(|object| Some((external(object)?, *object)))
        .collect();
    let mut pairs = Vec::new();
    // Originals some copy names by id: never a geometric candidate, so a
    // geometric pair can never take an object a copy already claims.
    let mut claimed: BTreeSet<&ObjectId> = BTreeSet::new();
    let mut without_counterpart = Vec::new();
    let mut count = |cause: Unmatched| *unmatched.entry(cause).or_default() += 1;
    for copy in session
        .project()
        .objects()
        .filter(|object| &object.id.source == copy_source)
    {
        let Some(original) = external(copy).and_then(|id| by_id.get(&id).copied()) else {
            without_counterpart.push(copy);
            continue;
        };
        claimed.insert(&original.id);
        if original.kind() != copy.kind() {
            count(Unmatched::DifferentClass);
            continue;
        }
        let verdict = Fingerprint::of(proximity, &copy.id)
            .and_then(|mine| Ok((mine, Fingerprint::of(proximity, &original.id)?)));
        let (mine, theirs) = match verdict {
            Ok(fingerprints) => fingerprints,
            Err(cause) => {
                count(cause);
                continue;
            }
        };
        match same_body(
            proximity,
            (&copy.id, &mine),
            (&original.id, &theirs),
            tolerance,
        ) {
            Verdict::Same => pairs.push((copy.id.clone(), original.id.clone(), MatchedBy::Id)),
            Verdict::Different => count(Unmatched::DifferentBody),
            Verdict::Undecided => count(Unmatched::Undecided),
        }
    }
    if matching == Matching::ById {
        for _ in &without_counterpart {
            count(Unmatched::NoCounterpart);
        }
        return pairs;
    }
    pairs.extend(match_by_geometry(
        proximity,
        (&originals, &claimed),
        &without_counterpart,
        tolerance,
        &mut count,
    ));
    pairs
}

/// The objects of a derived source with no counterpart by id paired by
/// geometry alone: the same class and body, and exactly one candidate on
/// each side; every other one counted by cause.
fn match_by_geometry(
    proximity: &ProximityServiceHandle,
    (originals, claimed): (&[&axioval_ir::Object], &BTreeSet<&ObjectId>),
    without_counterpart: &[&axioval_ir::Object],
    tolerance: f64,
    count: &mut impl FnMut(Unmatched),
) -> Vec<(ObjectId, ObjectId, MatchedBy)> {
    let mut pairs = Vec::new();
    // By geometry: every unclaimed original with a body, indexed by the
    // cell of its box's lower corner. A cell twice the widest corner
    // tolerance holds every candidate in its own or a neighbouring cell.
    let candidates: Vec<(&axioval_ir::Object, Fingerprint)> = originals
        .iter()
        .filter(|original| !claimed.contains(&original.id))
        .filter_map(|original| Some((*original, Fingerprint::of(proximity, &original.id).ok()?)))
        .collect();
    let copies: Vec<(&axioval_ir::Object, Result<Fingerprint, Unmatched>)> = without_counterpart
        .iter()
        .map(|copy| (*copy, Fingerprint::of(proximity, &copy.id)))
        .collect();
    let widest = candidates
        .iter()
        .map(|(_, print)| print.deviation)
        .chain(
            copies
                .iter()
                .filter_map(|(_, print)| print.as_ref().ok().map(|p| p.deviation)),
        )
        .fold(0.0_f64, f64::max);
    let cell = 2.0 * (tolerance + 2.0 * widest + ROUNDING).max(1e-3);
    let mut grid: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    for (index, (_, print)) in candidates.iter().enumerate() {
        grid.entry(print.cell(cell)).or_default().push(index);
    }
    let near = |print: &Fingerprint, kind: &str| -> Vec<usize> {
        let [x, y, z] = print.cell(cell);
        let mut found = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &index in grid.get(&[x + dx, y + dy, z + dz]).into_iter().flatten() {
                        let (candidate, theirs) = &candidates[index];
                        if candidate.kind() == kind
                            && print.compare(theirs, tolerance) == Verdict::Same
                        {
                            found.push(index);
                        }
                    }
                }
            }
        }
        found.sort_unstable();
        found
    };
    // Each copy's candidates, then each candidate's copies.
    let mut chosen: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut found: Vec<Result<Vec<usize>, Unmatched>> = Vec::new();
    for (position, (copy, print)) in copies.iter().enumerate() {
        let near = print.map(|print| near(&print, copy.kind()));
        if let Ok([only]) = near.as_deref() {
            chosen.entry(*only).or_default().push(position);
        }
        found.push(near);
    }
    for ((copy, print), near) in copies.iter().zip(found) {
        let cause = match (print, near.as_deref()) {
            (_, Err(cause)) => *cause,
            (_, Ok([])) => Unmatched::NoCounterpart,
            (Ok(mine), Ok([only])) if chosen.get(only).is_some_and(|copies| copies.len() == 1) => {
                let (original, theirs) = &candidates[*only];
                match same_body(
                    proximity,
                    (&copy.id, mine),
                    (&original.id, theirs),
                    tolerance,
                ) {
                    Verdict::Same => {
                        pairs.push((copy.id.clone(), original.id.clone(), MatchedBy::Geometry));
                        continue;
                    }
                    Verdict::Different => Unmatched::NoCounterpart,
                    Verdict::Undecided => Unmatched::Undecided,
                }
            }
            _ => Unmatched::Ambiguous,
        };
        count(cause);
    }
    pairs
}

/// Whether the copy's body is the original's: the fingerprints agree, and
/// the surfaces' distance, where measured, confirms it.
fn same_body(
    proximity: &ProximityServiceHandle,
    (copy, mine): (&ObjectId, &Fingerprint),
    (original, theirs): (&ObjectId, &Fingerprint),
    tolerance: f64,
) -> Verdict {
    match mine.compare(theirs, tolerance) {
        Verdict::Same => confirm(proximity, copy, original, tolerance),
        verdict => verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::{Fingerprint, Verdict};

    /// A box `min..max` with its exact volume and area.
    fn cuboid(min: [f64; 3], max: [f64; 3], deviation: f64) -> Fingerprint {
        let [a, b, c] = [0, 1, 2].map(|axis| max[axis] - min[axis]);
        Fingerprint {
            min,
            max,
            deviation,
            volume: Some((a * b * c, a * b * c)),
            area: 2.0 * (a * b + b * c + c * a),
        }
    }

    #[test]
    fn a_body_within_the_tolerance_is_the_same_and_one_beyond_it_is_not() {
        let wall = cuboid([0.0, -0.1, 0.0], [4.0, 0.1, 3.0], 0.0);
        assert_eq!(wall.compare(&wall, 0.001), Verdict::Same);
        let nudged = cuboid([0.0005, -0.1, 0.0], [4.0005, 0.1, 3.0], 0.0);
        assert_eq!(wall.compare(&nudged, 0.001), Verdict::Same);
        let moved = cuboid([0.002, -0.1, 0.0], [4.002, 0.1, 3.0], 0.0);
        assert_eq!(wall.compare(&moved, 0.001), Verdict::Different);
        // A deviation widens what a tessellated body can be told apart by.
        let tessellated = cuboid([0.002, -0.1, 0.0], [4.002, 0.1, 3.0], 0.001);
        assert_eq!(wall.compare(&tessellated, 0.001), Verdict::Same);
        let thicker = cuboid([0.0, -0.15, 0.0], [4.0, 0.15, 3.0], 0.0);
        assert_eq!(wall.compare(&thicker, 0.001), Verdict::Different);
    }

    #[test]
    fn a_body_whose_volume_cannot_be_bounded_is_undecided_never_the_same() {
        let wall = cuboid([0.0, -0.1, 0.0], [4.0, 0.1, 3.0], 0.0);
        let open = Fingerprint {
            volume: None,
            ..wall
        };
        assert_eq!(wall.compare(&open, 0.001), Verdict::Undecided);
        // The same box and area, a different volume: a hollow inside.
        let hollow = Fingerprint {
            volume: Some((1.0, 1.0)),
            ..wall
        };
        assert_eq!(wall.compare(&hollow, 0.001), Verdict::Different);
    }
}
