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

use axioval_engine::NotEvaluatedReason;
use axioval_ir::{Object, ObjectId, PropertyValue};

use super::{AmbiguousIdentity, Matcher, Revision, Side, UndecidedMatch};
use crate::support::{PropertyRef, Unavailable, display, resolve, undefined, value_key};

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
        keyed(matcher, [base, revised], &mut pools, &mut matching);
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
