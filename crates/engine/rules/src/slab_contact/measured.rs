//! What `slab-contact` measures, as values: how much of a face rests on the
//! objects named (`contact_share`, `contact_area`, `contact_gap`), and
//! where an object's storey stands among its source's storeys
//! (`levels_above`, `levels_below`, `storey_end`), storeys ordered by their
//! `Elevation` attribute and the object on the one storey its traversal
//! reaches.

use axioval_engine::{
    Citation, ContactError, ContactEvidence, ContactRequest, ContactServiceHandle, ContactSide,
    ContactTolerance, MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, SelectionIdentity};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Storeys, ends, ordered, storeys};
use crate::measured_kinds::{every_object_of_kinds, interval, refused, selection, traversal};
use crate::selection::object_by_id;
use crate::support::{Traversal, Unavailable, invalid};

/// Measures `levels_above`, `levels_below` and `storey_end`.
pub(crate) struct StoreyMeasures;

const LEVELS_ABOVE: &str = "levels_above";
const LEVELS_BELOW: &str = "levels_below";
const STOREY_END: &str = "storey_end";

fn count(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
        return Err(invalid("`path` is required"));
    };
    let selector = Selector::Objects {
        objects: every_object_of_kinds(context, call, "levels")
            .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?,
    };
    let traversal = Traversal::path(steps)?;
    let storeys = storeys(context, &selector)?;
    let universe: Vec<&Object> = storeys
        .universe
        .iter()
        .filter_map(|storey| object_by_id(context, storey))
        .collect();
    let (reached, _) = traversal.related(context, object, &universe)?;
    let [storey] = reached.as_slice() else {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{object} reaches {} storeys through {}, so where it lies is unknown",
                reached.len(),
                traversal.relationship
            ),
        ));
    };
    let own = storeys.elevations[storey];
    let above = call.name() == LEVELS_ABOVE;
    let counted = storeys
        .universe
        .iter()
        .filter(|candidate| candidate.source == storey.source)
        .filter(|candidate| {
            let elevation = storeys.elevations[*candidate];
            if above {
                elevation.total_cmp(&own).is_gt()
            } else {
                elevation.total_cmp(&own).is_lt()
            }
        })
        .count();
    #[allow(clippy::cast_precision_loss)]
    let counted = counted as f64;
    // Counted over stated elevations, which the property service answers
    // only with exact evidence: an exact count.
    Ok(Measurement::Rounded {
        lower: counted,
        upper: counted,
        dimension: None,
        locator: format!("{}:{object}:{storey}", call.name()),
    })
}

/// The storeys a rule names, ordered once per run.
#[derive(Hash, PartialEq, Eq)]
enum StoreysKey {
    Kinds(String),
    Selection(SelectionIdentity),
}

/// The traversal arguments of a call, as a memo keys the traversal.
#[derive(Hash, PartialEq, Eq)]
struct TraversalKey([String; 5]);

/// 1 where `object` lies on the end storey `end` names, 0 where not, as
/// `slab-contact` decides which subjects it leaves out.
fn end(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    let picked = match call.argument("storeys") {
        Some(MeasuredArgument::Objects(shared)) => std::borrow::Cow::Borrowed(&**shared),
        _ => std::borrow::Cow::Owned(
            selection(context, call, "storeys", None)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
                .ok_or_else(|| invalid("`storey_end` needs `storeys`"))?,
        ),
    };
    // The traversal, read once per run for every object.
    let walked: Result<Option<Traversal>, Unavailable> = MeasuredMemo::of(
        context.services,
        TraversalKey(
            [
                "relationship",
                "direction",
                "path",
                "follow_chain",
                "skip_absent_relationship_ends",
            ]
            .map(|key| format!("{:?}", call.argument(key))),
        ),
        || traversal(call),
    );
    let traversal = walked?.ok_or_else(|| {
        invalid("`storey_end` needs a `relationship` or `path` to the object's storey")
    })?;
    let key = match call.argument("storeys") {
        Some(MeasuredArgument::Objects(shared)) => {
            StoreysKey::Selection(SelectionIdentity(shared.clone()))
        }
        argument => StoreysKey::Kinds(format!("{argument:?}")),
    };
    let storeys: Result<Storeys, Unavailable> = MeasuredMemo::of(context.services, key, || {
        if let Some((reason, message)) = &picked.first_undecided {
            return Err((
                reason.clone(),
                format!("storey selection is undecided: {message}"),
            ));
        }
        let universe: Vec<&Object> = picked
            .matched
            .iter()
            .filter_map(|storey| object_by_id(context, storey))
            .collect();
        ordered(context, &universe)
    });
    let storeys = storeys
        .map_err(|(reason, message)| (reason, format!("storeys cannot be ordered: {message}")))?;
    let (top, bottom) = ends(context, &traversal, &storeys, object)?;
    let on = if call.choice("end") == Some("bottom") {
        bottom
    } else {
        top
    };
    let flag = if on { 1.0 } else { 0.0 };
    // Decided over stated elevations and relationships, exactly.
    Ok(Measurement::Rounded {
        lower: flag,
        upper: flag,
        dimension: None,
        locator: format!("{STOREY_END}:{object}"),
    })
}

impl MeasuredProvider for StoreyMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[LEVELS_ABOVE, LEVELS_BELOW, STOREY_END]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        if call.name() == STOREY_END {
            return end(call, object, context).map_err(refused(call.name(), object));
        }
        count(call, object, context).map_err(refused(call.name(), object))
    }
}

/// Measures `contact_area`, `contact_gap` and `contact_share`.
pub(crate) struct ContactMeasures;

const CONTACT_AREA: &str = "contact_area";
const CONTACT_GAP: &str = "contact_gap";
const CONTACT_SHARE: &str = "contact_share";

/// One contact request, measured once per run for every value reading it:
/// the face, the objects named, the side and the tolerances' bits.
#[derive(Hash, PartialEq, Eq)]
struct ContactKey(ObjectId, Named, bool, [u64; 3]);

/// The objects a contact value names, as a memo keys them.
#[derive(Hash, PartialEq, Eq)]
enum Named {
    /// Every other object of the project.
    Every,
    /// The objects of source kinds, as written.
    Kinds(String),
    /// A rule's selection, by identity.
    Selection(SelectionIdentity),
}

/// What a contact request measured, kept for the run: the areas, the gap,
/// what the face rests on and the evidence, without the request.
#[derive(Clone)]
struct Contact {
    area: f64,
    whole: f64,
    gap: Option<f64>,
    touching: Vec<ObjectId>,
    evidence: axioval_ir::Evidence,
}

impl From<ContactEvidence> for Contact {
    fn from(measured: ContactEvidence) -> Self {
        Self {
            area: measured.contact_area_square_metres(),
            whole: measured.whole_area_square_metres(),
            gap: measured.nearest_distance_metres(),
            touching: measured.touching().to_vec(),
            evidence: measured.evidence().clone(),
        }
    }
}

/// Why a contact cannot be measured, for the reason `slab-contact` gives:
/// a body the service could not measure or orient is missing evidence,
/// never a clean face, and an answer it cannot stand behind is invalid
/// evidence.
pub(crate) fn contact_unavailable(error: ContactError) -> Unavailable {
    let reason = match error {
        ContactError::Unavailable | ContactError::UncheckableOrientation => {
            NotEvaluatedReason::IncompleteEvidence
        }
        ContactError::InexactEvidence
        | ContactError::InvalidAreas
        | ContactError::UnrequestedCandidate => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, error.to_string())
}

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// `part / whole` of two exact areas, `whole` positive: a point where the
/// division is exact, otherwise the two neighbours of the rounded quotient,
/// which hold the exact one; kept in `[0, 1]`.
fn share(part: f64, whole: f64) -> (f64, f64) {
    let quotient = part / whole;
    if quotient.mul_add(whole, -part) == 0.0 {
        (quotient, quotient)
    } else {
        (
            quotient.next_down().clamp(0.0, 1.0),
            quotient.next_up().clamp(0.0, 1.0),
        )
    }
}

/// The contact of `object`'s face with the objects `with` names, and how
/// many objects it names may be candidates as well.
fn contact(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(std::sync::Arc<Contact>, usize), Unavailable> {
    let service = context
        .services
        .get::<ContactServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "contact service is not registered".to_owned(),
            )
        })?;
    // Without `with`, the face may rest on any other object of the project.
    let (named, identity) = match call.argument("with") {
        None => (None, Named::Every),
        Some(MeasuredArgument::Objects(picked)) => (
            Some(std::borrow::Cow::Borrowed(&**picked)),
            Named::Selection(SelectionIdentity(picked.clone())),
        ),
        Some(argument) => (
            selection(context, call, "with", Some(object))
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
                .map(std::borrow::Cow::Owned),
            Named::Kinds(format!("{argument:?}")),
        ),
    };
    let undecided = named.as_ref().map_or(0, |picked| {
        picked
            .undecided
            .iter()
            .filter(|candidate| *candidate != object)
            .count()
    });
    let side = if call.choice("side") == Some("above") {
        ContactSide::Above
    } else {
        ContactSide::Below
    };
    let tolerances = [
        length(call, "gap"),
        length(call, "intersection"),
        length(call, "polygon"),
    ];
    let tolerance = ContactTolerance::try_new(tolerances[0], tolerances[1], tolerances[2])
        .map_err(|error| invalid(error.to_string()))?;
    let key = ContactKey(
        object.clone(),
        identity,
        side == ContactSide::Above,
        tolerances.map(f64::to_bits),
    );
    let measured: Result<std::sync::Arc<Contact>, ContactError> =
        MeasuredMemo::of(context.services, key, || {
            let candidates: Vec<ObjectId> = match &named {
                None => context
                    .project
                    .objects()
                    .map(|candidate| candidate.id.clone())
                    .filter(|candidate| candidate != object)
                    .collect(),
                Some(picked) => picked
                    .matched
                    .iter()
                    .filter(|candidate| *candidate != object)
                    .cloned()
                    .collect(),
            };
            service
                .measure_contact(&ContactRequest::new(
                    object.clone(),
                    candidates,
                    side,
                    tolerance,
                ))
                .map(|measured| std::sync::Arc::new(Contact::from(measured)))
        });
    Ok((measured.map_err(contact_unavailable)?, undecided))
}

/// A value of the contact: candidates the selection cannot decide may add
/// contact, up to the whole face, and may lie nearer than any measured.
fn contact_value(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let (measured, undecided) = contact(call, object, context)?;
    let locator = measured.evidence.locator.clone();
    let exact = measured.evidence.exact;
    let (area, whole) = (measured.area, measured.whole);
    let value = match call.name() {
        CONTACT_AREA => interval(
            (area, if undecided > 0 { whole } else { area }),
            Some(QuantityDimension::Area),
            exact,
            locator,
        ),
        CONTACT_GAP => match measured.gap {
            _ if undecided > 0 => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("{undecided} object(s) the selection cannot decide may lie nearer"),
                ));
            }
            Some(gap) => interval((gap, gap), Some(QuantityDimension::Length), exact, locator),
            None => Measurement::Absent {
                locator: format!("{locator}: no candidate is reported near"),
            },
        },
        _ => {
            let (lower, upper) = share(area, whole);
            interval(
                (lower, if undecided > 0 { 1.0 } else { upper }),
                None,
                exact,
                locator,
            )
        }
    };
    Ok((
        value,
        Citation {
            // What the face rests on, so a reviewer can open it: the share
            // the rule judges cites it.
            related: if call.name() == CONTACT_SHARE {
                measured.touching.clone()
            } else {
                Vec::new()
            },
            evidence: Vec::new(),
        },
    ))
}

impl MeasuredProvider for ContactMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[CONTACT_AREA, CONTACT_GAP, CONTACT_SHARE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    /// Each request is measured once per run, whichever value reads it.
    fn memoizes(&self) -> bool {
        true
    }

    /// A contact cites the objects the face rests on.
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        contact_value(call, object, context).map_err(refused(call.name(), object))
    }
}
