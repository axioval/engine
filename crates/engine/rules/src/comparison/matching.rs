//! Matching the objects of two revisions.
//!
//! Matchers run in the request's order, each over the objects the ones
//! before it left open. A keyed matcher (a scheme, a property) gives each
//! open object at most one key; a key held by exactly one object on each side
//! pairs them. Whatever cannot be decided is taken out of the comparison and
//! reported, never guessed:
//!
//! - a key held by several objects of one side is ambiguous, and matches
//!   nothing on either side;
//! - an object whose key cannot be read is undecided, and so is every object
//!   of the other side the same matcher keyed but left unpaired, since it may
//!   be that object's partner.
//!
//! An object no matcher could key is unidentified. One some matcher keyed but
//! none paired is added or removed, named by its first key.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    NotEvaluatedReason, ObjectBounds, ObjectFrameError, ObjectFrameServiceHandle, ProximityError,
    ProximityRequest, ProximityServiceHandle,
};
use axioval_ir::{Object, ObjectId, PropertyValue};

use super::{
    AmbiguousIdentity, ComparisonTolerance, Matcher, Revision, Side, UndecidedMatch, facets,
};
use crate::support::{PropertyRef, Traversal, Unavailable, display, resolve, undefined, value_key};

/// Two objects matched, with the identity and the matcher that matched them.
pub(super) struct Pairing<'a> {
    pub(super) identity: String,
    pub(super) matcher: String,
    pub(super) base: &'a Object,
    pub(super) revised: &'a Object,
}

/// An object matched with nothing, named by its first key.
pub(super) struct Single<'a> {
    pub(super) identity: String,
    pub(super) matcher: String,
    pub(super) object: &'a Object,
}

/// Every object of both revisions, in exactly one state.
#[derive(Default)]
pub(super) struct Matching<'a> {
    pub(super) pairs: Vec<Pairing<'a>>,
    pub(super) removed: Vec<Single<'a>>,
    pub(super) added: Vec<Single<'a>>,
    pub(super) unidentified: Vec<(Side, ObjectId)>,
    pub(super) ambiguous: Vec<AmbiguousIdentity>,
    pub(super) undecided: Vec<UndecidedMatch>,
}

/// The objects of one side still open, and the first key each was given.
struct Pool<'a> {
    side: Side,
    open: BTreeMap<&'a ObjectId, &'a Object>,
    first: BTreeMap<&'a ObjectId, (String, String)>,
}

impl<'a> Pool<'a> {
    fn of(side: Side, objects: &[&'a Object]) -> Self {
        Self {
            side,
            open: objects.iter().map(|object| (&object.id, *object)).collect(),
            first: BTreeMap::new(),
        }
    }
}

/// Matches the candidates of both revisions with `matchers`, in order.
pub(super) fn match_objects<'a>(
    base: &Revision<'a>,
    revised: &Revision<'a>,
    matchers: &[Matcher],
) -> Matching<'a> {
    let mut matching = Matching::default();
    let mut pools = [
        Pool::of(Side::Base, &base.candidates),
        Pool::of(Side::Revised, &revised.candidates),
    ];
    for matcher in matchers {
        match matcher {
            Matcher::Scheme(_) | Matcher::Property { .. } => {
                keyed(matcher, [base, revised], &mut pools, &mut matching);
            }
            _ => pairwise(matcher, [base, revised], &mut pools, &mut matching),
        }
    }
    for pool in pools {
        let Pool {
            side,
            open,
            mut first,
        } = pool;
        for (id, object) in open {
            match first.remove(id) {
                Some((matcher, identity)) => {
                    let single = Single {
                        identity,
                        matcher,
                        object,
                    };
                    match side {
                        Side::Base => matching.removed.push(single),
                        Side::Revised => matching.added.push(single),
                    }
                }
                None => matching.unidentified.push((side, id.clone())),
            }
        }
    }
    matching.unidentified.sort();
    matching
        .ambiguous
        .sort_by(|a, b| (&a.identity, &a.matcher, a.side).cmp(&(&b.identity, &b.matcher, b.side)));
    matching
        .undecided
        .sort_by(|a, b| (a.side, &a.object).cmp(&(b.side, &b.object)));
    matching
}

/// One object's key under a keyed matcher and how it is shown, `None` when
/// the object has none.
fn key(
    matcher: &Matcher,
    revision: &Revision<'_>,
    side: Side,
    object: &Object,
) -> Result<Option<(String, String)>, Unavailable> {
    match matcher {
        Matcher::Scheme(scheme) => Ok(object
            .external_id(scheme)
            .map(|identity| (identity.to_owned(), identity.to_owned()))),
        Matcher::Geometry { .. }
        | Matcher::Placement { .. }
        | Matcher::Overlap { .. }
        | Matcher::Related { .. } => unreachable!("pairwise matchers have no key"),
        Matcher::Property { base, revised } => {
            let property = match side {
                Side::Base => base,
                Side::Revised => revised,
            };
            let resolved = resolve(
                &revision.context,
                object,
                PropertyRef {
                    set: property.property_set(),
                    name: property.name(),
                },
            )?;
            let value = resolved.value();
            if undefined(value) {
                return Ok(None);
            }
            let value = value.expect("an undefined value has no key");
            if matches!(value, PropertyValue::Measured { .. }) {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("{property} is a measured interval, which identifies nothing exactly"),
                ));
            }
            let shown = match value {
                PropertyValue::String(text) => text.clone(),
                other => display(Some(other)),
            };
            Ok(Some((value_key(value, false, true), shown)))
        }
    }
}

/// Keys every open object, pairs the keys held once on each side, and takes
/// out whatever the matcher leaves undecided.
fn keyed<'a>(
    matcher: &Matcher,
    revisions: [&Revision<'a>; 2],
    pools: &mut [Pool<'a>; 2],
    matching: &mut Matching<'a>,
) {
    let label = matcher.label();
    // Per side: key -> (shown, claimants).
    let mut claims: [BTreeMap<String, (String, Vec<&'a Object>)>; 2] = Default::default();
    let mut unknown = [0_usize; 2];
    for (index, pool) in pools.iter_mut().enumerate() {
        let mut unreadable = Vec::new();
        for (id, object) in &pool.open {
            match key(matcher, revisions[index], pool.side, object) {
                Ok(Some((key, shown))) => {
                    pool.first
                        .entry(id)
                        .or_insert_with(|| (label.clone(), shown.clone()));
                    claims[index]
                        .entry(key)
                        .or_insert_with(|| (shown, Vec::new()))
                        .1
                        .push(object);
                }
                Ok(None) => {}
                Err((reason, message)) => {
                    unreadable.push(*id);
                    matching.undecided.push(UndecidedMatch {
                        side: pool.side,
                        object: (*id).clone(),
                        reason,
                        message: format!("its {label} cannot be read: {message}"),
                    });
                }
            }
        }
        unknown[index] = unreadable.len();
        for id in unreadable {
            pool.open.remove(id);
        }
    }

    let keys: BTreeSet<String> = claims[0].keys().chain(claims[1].keys()).cloned().collect();
    for key in keys {
        let sides = [claims[0].get(&key), claims[1].get(&key)];
        let count = |index: usize| sides[index].map_or(0, |(_, objects)| objects.len());
        if count(0) > 1 || count(1) > 1 {
            // Nothing claimed twice matches anything: report every claimant
            // and every object the claim leaves without its partner.
            for (index, claim) in sides.iter().enumerate() {
                let Some((shown, objects)) = claim else {
                    continue;
                };
                matching.ambiguous.push(AmbiguousIdentity {
                    side: pools[index].side,
                    identity: shown.clone(),
                    matcher: label.clone(),
                    objects: objects.iter().map(|object| object.id.clone()).collect(),
                });
                for object in objects {
                    pools[index].open.remove(&object.id);
                    pools[index].first.remove(&object.id);
                }
            }
        } else if let [Some((shown, base)), Some((_, revised))] = sides {
            let (base, revised) = (base[0], revised[0]);
            matching.pairs.push(Pairing {
                identity: shown.clone(),
                matcher: label.clone(),
                base,
                revised,
            });
            pools[0].open.remove(&base.id);
            pools[1].open.remove(&revised.id);
        }
    }

    // An unreadable key may be any keyed object's partner on the other side.
    for index in 0..2 {
        let other = 1 - index;
        if unknown[other] == 0 {
            continue;
        }
        let side = pools[other].side;
        for (_, objects) in claims[index].values() {
            for object in objects {
                if pools[index].open.remove(&object.id).is_some() {
                    pools[index].first.remove(&object.id);
                    matching.undecided.push(UndecidedMatch {
                        side: pools[index].side,
                        object: object.id.clone(),
                        reason: NotEvaluatedReason::IncompleteEvidence,
                        message: format!(
                            "it may match one of {} {} object(s) whose {label} cannot be read",
                            unknown[other],
                            side.name()
                        ),
                    });
                }
            }
        }
    }
}

/// Whether a base and a revised object are the same by a pairwise matcher.
enum Verdict {
    Yes,
    No,
    Unknown(String),
}

/// A pair judged by a pairwise matcher: base, revised and the verdict.
type Edge<'a> = (&'a Object, &'a Object, Verdict);

/// Judges an interval against a tolerance: within it, beyond it, or
/// straddling it.
fn within(lower: f64, upper: f64, tolerance: f64, what: &str) -> Verdict {
    if upper <= tolerance {
        Verdict::Yes
    } else if lower > tolerance {
        Verdict::No
    } else {
        Verdict::Unknown(format!(
            "{what} lies between {lower:.4} and {upper:.4}, straddling {tolerance:.4}"
        ))
    }
}

/// Whether two bodies' surfaces coincide within `tolerance` metres: their
/// certified Hausdorff distance, which no less than their separation.
fn coincide(
    proximity: &ProximityServiceHandle,
    base: &ObjectId,
    revised: &ObjectId,
    tolerance: f64,
) -> Verdict {
    let measured = ProximityRequest::try_new(base.clone(), revised.clone())
        .and_then(|request| proximity.measure_proximity(&request));
    let measured = match measured {
        Ok(measured) => measured,
        Err(error) => return Verdict::Unknown(error.to_string()),
    };
    let separation = measured.separation_interval_metres().0;
    match measured.hausdorff_interval_metres() {
        Some(interval) => within(
            interval.lower_metres().max(separation),
            interval.upper_metres(),
            tolerance,
            "the Hausdorff distance",
        ),
        None if separation > tolerance => Verdict::No,
        None => Verdict::Unknown("the service measures no Hausdorff distance".into()),
    }
}

/// Whether two objects are placed alike: frame origins within the length
/// tolerance and axes within the angle tolerance, both unplaced alike.
fn placed_alike(
    frames: Option<&ObjectFrameServiceHandle>,
    base: &ObjectId,
    revised: &ObjectId,
    tolerance: ComparisonTolerance,
) -> Verdict {
    let Some(frames) = frames else {
        return Verdict::Unknown("the object-frame service is not registered".into());
    };
    match (frames.object_frame(base), frames.object_frame(revised)) {
        (Err(ObjectFrameError::NotPlaced(_)), Err(ObjectFrameError::NotPlaced(_))) => Verdict::Yes,
        (Ok(_), Err(ObjectFrameError::NotPlaced(_)))
        | (Err(ObjectFrameError::NotPlaced(_)), Ok(_)) => Verdict::No,
        (Ok(before), Ok(after)) => {
            let (before, after) = (before.frame(), after.frame());
            let axes =
                |frame: &axioval_engine::MetricFrame| [frame.right(), frame.forward(), frame.up()];
            let moved = facets::distance(
                before.origin().coordinates_metres(),
                after.origin().coordinates_metres(),
            );
            let turned = facets::rotation(axes(before), axes(after));
            if moved <= tolerance.length_metres() && turned <= tolerance.angle_radians() {
                Verdict::Yes
            } else {
                Verdict::No
            }
        }
        (Err(error), _) | (_, Err(error)) => Verdict::Unknown(error.to_string()),
    }
}

/// Whether two bodies share at least `minimum` of the larger one's volume.
fn overlap(
    proximity: &ProximityServiceHandle,
    base: &ObjectId,
    revised: &ObjectId,
    minimum: f64,
) -> Verdict {
    let measured = ProximityRequest::try_new(base.clone(), revised.clone())
        .and_then(|request| proximity.measure_proximity(&request));
    let measured = match measured {
        Ok(measured) => measured,
        Err(error) => return Verdict::Unknown(error.to_string()),
    };
    let Some(volume) = measured.intersection_volume() else {
        // Bodies apart at the surface and not nested share no volume.
        return if measured.separation_interval_metres().0 > 0.0 && measured.containment().is_none()
        {
            Verdict::No
        } else {
            Verdict::Unknown("the service certifies no shared volume".into())
        };
    };
    let (subject, counterpart) = (volume.subject(), volume.counterpart());
    let larger_lower = subject
        .lower_cubic_metres()
        .max(counterpart.lower_cubic_metres());
    let larger_upper = subject
        .upper_cubic_metres()
        .max(counterpart.upper_cubic_metres());
    let shared = volume.shared();
    let lower = if larger_upper > 0.0 {
        (shared.lower_cubic_metres() / larger_upper)
            .next_down()
            .max(0.0)
    } else {
        0.0
    };
    let upper = if larger_lower > 0.0 {
        (shared.upper_cubic_metres() / larger_lower)
            .next_up()
            .min(1.0)
    } else {
        1.0
    };
    if lower >= minimum {
        Verdict::Yes
    } else if upper < minimum {
        Verdict::No
    } else {
        Verdict::Unknown(format!(
            "the shared share of the larger body lies between {lower:.3} and {upper:.3}, straddling {minimum}"
        ))
    }
}

/// Takes every open object of both pools out as undecided.
fn refuse_all(
    pools: &mut [Pool<'_>; 2],
    matching: &mut Matching<'_>,
    reason: &NotEvaluatedReason,
    message: &str,
) {
    for pool in pools.iter_mut() {
        for id in std::mem::take(&mut pool.open).into_keys() {
            pool.first.remove(id);
            matching.undecided.push(UndecidedMatch {
                side: pool.side,
                object: id.clone(),
                reason: reason.clone(),
                message: message.to_owned(),
            });
        }
    }
}

/// The objects each open object reaches along `path`: `Ok(None)` for none,
/// the one reached, or why it cannot be told.
fn reached(
    revision: &Revision<'_>,
    traversal: &Traversal<'_>,
    object: &Object,
) -> Result<Option<ObjectId>, String> {
    let (reached, _) = traversal
        .related(&revision.context, &object.id, &revision.objects)
        .map_err(|(_, message)| message)?;
    match <[ObjectId; 1]>::try_from(reached) {
        Ok([one]) => Ok(Some(one)),
        Err(reached) if reached.is_empty() => Ok(None),
        Err(reached) => Err(format!("it reaches {} objects, not one", reached.len())),
    }
}

/// Matches open objects pair by pair: the same kind, and the matcher's
/// geometric or relational test. A pair is matched only when each is the
/// other's one sure candidate and neither has an undecided one; everything
/// with a candidate left over is undecided.
#[allow(clippy::too_many_lines)]
fn pairwise<'a>(
    matcher: &Matcher,
    revisions: [&Revision<'a>; 2],
    pools: &mut [Pool<'a>; 2],
    matching: &mut Matching<'a>,
) {
    let label = matcher.label();
    let one_session = std::ptr::eq(revisions[0].context.services, revisions[1].context.services)
        && std::ptr::eq(revisions[0].context.project, revisions[1].context.project);
    if !one_session {
        refuse_all(
            pools,
            matching,
            &NotEvaluatedReason::InvalidDeclaration,
            &format!("{label} matching needs both models in one session"),
        );
        return;
    }
    let services = revisions[0].context.services;
    let proximity = services.get::<ProximityServiceHandle>();
    let frames = services.get::<ObjectFrameServiceHandle>();
    let needs_bodies = !matches!(matcher, Matcher::Related { .. });
    let Some(proximity) = proximity else {
        refuse_all(
            pools,
            matching,
            &NotEvaluatedReason::MissingService,
            &format!("{label} matching needs the proximity service, which is not registered"),
        );
        return;
    };

    // Each open object's key: its bounds, or the one object it reaches.
    let mut bounds: [BTreeMap<&'a ObjectId, ObjectBounds>; 2] = Default::default();
    let mut hosts: [BTreeMap<&'a ObjectId, ObjectId>; 2] = Default::default();
    let mut unknown: [Vec<&'a Object>; 2] = Default::default();
    let traversal = match matcher {
        Matcher::Related { path, .. } => match Traversal::path(path) {
            Ok(traversal) => Some(traversal),
            Err((reason, message)) => {
                refuse_all(pools, matching, &reason, &message);
                return;
            }
        },
        _ => None,
    };
    for (index, pool) in pools.iter_mut().enumerate() {
        let mut unreadable = Vec::new();
        for (&id, &object) in &pool.open {
            let keyed = if needs_bodies {
                match proximity.bounds(id) {
                    Ok(extent) if extent.object() == id => {
                        bounds[index].insert(id, extent);
                        Ok(true)
                    }
                    Ok(_) => Err("the proximity service answered for another object".to_owned()),
                    Err(ProximityError::NoBody) => Ok(false),
                    Err(error) => Err(error.to_string()),
                }
            } else {
                let traversal = traversal.as_ref().expect("a related matcher has a path");
                match reached(revisions[index], traversal, object) {
                    Ok(Some(host)) => {
                        hosts[index].insert(id, host);
                        Ok(true)
                    }
                    Ok(None) => Ok(false),
                    Err(message) => Err(message),
                }
            };
            match keyed {
                Ok(true) => {
                    pool.first
                        .entry(id)
                        .or_insert_with(|| (label.clone(), id.local_id.clone()));
                }
                Ok(false) => {}
                Err(message) => {
                    unreadable.push(object);
                    matching.undecided.push(UndecidedMatch {
                        side: pool.side,
                        object: id.clone(),
                        reason: NotEvaluatedReason::IncompleteEvidence,
                        message: format!("its {label} key cannot be read: {message}"),
                    });
                }
            }
        }
        for object in &unreadable {
            pool.open.remove(&object.id);
        }
        unknown[index] = unreadable;
    }

    let keyed = |index: usize, id: &ObjectId| {
        bounds[index].contains_key(id) || hosts[index].contains_key(id)
    };
    let pairs: BTreeMap<&ObjectId, &ObjectId> = matching
        .pairs
        .iter()
        .map(|pair| (&pair.base.id, &pair.revised.id))
        .collect();
    let mut edges: Vec<Edge<'a>> = Vec::new();
    for base in pools[0].open.values() {
        if !keyed(0, &base.id) {
            continue;
        }
        for revised in pools[1].open.values() {
            if base.kind != revised.kind || !keyed(1, &revised.id) {
                continue;
            }
            let verdict = match matcher {
                Matcher::Geometry { tolerance_metres }
                | Matcher::Placement {
                    tolerance:
                        ComparisonTolerance {
                            length_metres: tolerance_metres,
                            ..
                        },
                } => {
                    let (a, b) = (&bounds[0][&base.id], &bounds[1][&revised.id]);
                    // Each face of the bounds shifts no further than the
                    // surfaces lie apart.
                    let widening =
                        a.fidelity().deviation_metres() + b.fidelity().deviation_metres();
                    if facets::bounds_shift(&a.bounds(), &b.bounds()) - widening > *tolerance_metres
                    {
                        continue;
                    }
                    let placed = match matcher {
                        Matcher::Placement { tolerance } => {
                            placed_alike(frames, &base.id, &revised.id, *tolerance)
                        }
                        _ => Verdict::Yes,
                    };
                    match placed {
                        Verdict::Yes => {
                            coincide(proximity, &base.id, &revised.id, *tolerance_metres)
                        }
                        other => other,
                    }
                }
                Matcher::Overlap { minimum_ratio } => {
                    let (a, b) = (&bounds[0][&base.id], &bounds[1][&revised.id]);
                    if a.enclosing().gap(&b.enclosing()) > 0.0 {
                        continue;
                    }
                    overlap(proximity, &base.id, &revised.id, *minimum_ratio)
                }
                Matcher::Related {
                    tolerance_metres, ..
                } => {
                    let (from, to) = (&hosts[0][&base.id], &hosts[1][&revised.id]);
                    match pairs.get(from) {
                        Some(partner) if *partner == to => Verdict::Yes,
                        Some(_) => Verdict::No,
                        None if pairs.values().any(|partner| *partner == to) => Verdict::No,
                        None => {
                            let kinds = (
                                revisions[0]
                                    .context
                                    .project
                                    .object(from)
                                    .map(|o| o.kind.as_str()),
                                revisions[1]
                                    .context
                                    .project
                                    .object(to)
                                    .map(|o| o.kind.as_str()),
                            );
                            if kinds.0 == kinds.1 {
                                coincide(proximity, from, to, *tolerance_metres)
                            } else {
                                Verdict::No
                            }
                        }
                    }
                }
                Matcher::Scheme(_) | Matcher::Property { .. } => unreachable!("keyed matchers"),
            };
            if !matches!(verdict, Verdict::No) {
                edges.push((base, revised, verdict));
            }
        }
    }

    // Per side, each object's sure and undecided candidates.
    let mut sure: [BTreeMap<&ObjectId, Vec<&ObjectId>>; 2] = Default::default();
    let mut maybe: [BTreeMap<&ObjectId, Vec<(&ObjectId, &str)>>; 2] = Default::default();
    for (base, revised, verdict) in &edges {
        match verdict {
            Verdict::Yes => {
                sure[0].entry(&base.id).or_default().push(&revised.id);
                sure[1].entry(&revised.id).or_default().push(&base.id);
            }
            Verdict::Unknown(message) => {
                maybe[0]
                    .entry(&base.id)
                    .or_default()
                    .push((&revised.id, message));
                maybe[1]
                    .entry(&revised.id)
                    .or_default()
                    .push((&base.id, message));
            }
            Verdict::No => {}
        }
    }
    let alone = |index: usize, id: &ObjectId| -> Option<&ObjectId> {
        match (sure[index].get(id).map(Vec::as_slice), maybe[index].get(id)) {
            (Some([one]), None) => Some(one),
            _ => None,
        }
    };
    let mut settling: Vec<(&'a Object, &'a Object)> = Vec::new();
    for (base, revised, verdict) in &edges {
        if matches!(verdict, Verdict::Yes)
            && alone(0, &base.id) == Some(&revised.id)
            && alone(1, &revised.id) == Some(&base.id)
        {
            settling.push((base, revised));
        }
    }
    let mut settled: [BTreeSet<ObjectId>; 2] = Default::default();
    for (base, revised) in settling {
        settled[0].insert(base.id.clone());
        settled[1].insert(revised.id.clone());
        pools[0].open.remove(&base.id);
        pools[1].open.remove(&revised.id);
        matching.pairs.push(Pairing {
            identity: base.id.local_id.clone(),
            matcher: label.clone(),
            base,
            revised,
        });
    }
    for index in 0..2 {
        let side = pools[index].side;
        let other = pools[1 - index].side.name();
        let mut undecided: BTreeMap<&ObjectId, (NotEvaluatedReason, String)> = BTreeMap::new();
        for (id, candidates) in &sure[index] {
            if !settled[index].contains(*id) && candidates.len() > 1 {
                undecided.insert(
                    *id,
                    (
                        NotEvaluatedReason::InvalidEvidence,
                        format!(
                            "by {label}, it matches {} {other} objects",
                            candidates.len()
                        ),
                    ),
                );
            }
        }
        for (id, candidates) in &maybe[index] {
            if !settled[index].contains(*id) {
                let (candidate, message) = candidates[0];
                undecided.entry(*id).or_insert_with(|| {
                    (
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("by {label}, it may match {other} object {candidate}: {message}"),
                    )
                });
            }
        }
        for (id, candidates) in &sure[index] {
            if !settled[index].contains(*id) {
                undecided.entry(*id).or_insert_with(|| {
                    (
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "by {label}, its match {} is undecided on the {other} side",
                            candidates[0]
                        ),
                    )
                });
            }
        }
        for (id, (reason, message)) in undecided {
            pools[index].open.remove(id);
            pools[index].first.remove(id);
            matching.undecided.push(UndecidedMatch {
                side,
                object: id.clone(),
                reason,
                message,
            });
        }
    }

    // An object without a readable key may be any keyed object of its kind.
    for index in 0..2 {
        let other = 1 - index;
        if unknown[other].is_empty() {
            continue;
        }
        let kinds: BTreeSet<&str> = unknown[other]
            .iter()
            .map(|object| object.kind.as_str())
            .collect();
        let tainted: Vec<&ObjectId> = pools[index]
            .open
            .iter()
            .filter(|(id, object)| keyed(index, id) && kinds.contains(object.kind.as_str()))
            .map(|(id, _)| *id)
            .collect();
        for id in tainted {
            pools[index].open.remove(id);
            pools[index].first.remove(id);
            matching.undecided.push(UndecidedMatch {
                side: pools[index].side,
                object: id.clone(),
                reason: NotEvaluatedReason::IncompleteEvidence,
                message: format!(
                    "it may match a {} object whose {label} key cannot be read",
                    pools[other].side.name()
                ),
            });
        }
    }
}
