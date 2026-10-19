//! What the sources' effect areas cover of an element, as measured values,
//! measured exactly as `effective-coverage` measures it: the sources and
//! blockers near the element from the plan broad phase, a source counting
//! surely only where it is picked (and, touching, surely touches), the
//! connections through doors and openings, and a source or blocker that
//! cannot be placed widening the cover.
//!
//! - `effective_reaching`: how many sources reach the element; `null`
//!   where the element states no area under `area_property`, which then
//!   is all there is to say;
//! - `effective_area`: the area the cover is a share of: the stated area,
//!   or the footprint;
//! - `effective_covered` and `effective_share`: the part covered and its
//!   share, citing the sources surely contributing and noting why it may
//!   be covered more;
//! - `effective_capacity` and `effective_unread`: the summed capacity of
//!   the sources whose effect meets the footprint, each times its
//!   multiplier, over the contributions read, and how many cannot be read;
//! - `effective_missing`, a member list: each surely contributing source
//!   stating no capacity or multiplier.
//!
//! Every argument is named as the capability names its parameter. The
//! sources' and blockers' extents are read once per selection, the access
//! index once per declaration, and each element's measurement once per run
//! for every value reading it.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, Citation, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement,
    MemberValue, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Around, Capacity, Element, Mode, Multiplier, Services, Setting, Unmeasured};
use crate::measured_kinds::{interval, refused, selection};
use crate::near::{Candidates, bounds};
use crate::selection::object_by_id;
use crate::space_access::{AccessDeclaration, AccessIndex, Pick};
use crate::support::{Parameters, PropertyRef, Unavailable, invalid};

/// Measures `effective_reaching`, `effective_area`, `effective_covered`,
/// `effective_share`, `effective_capacity` and `effective_unread`, and lists
/// `effective_missing`.
pub(crate) struct EffectMeasures;

const REACHING: &str = "effective_reaching";
const AREA: &str = "effective_area";
const COVERED: &str = "effective_covered";
const SHARE: &str = "effective_share";
const CAPACITY: &str = "effective_capacity";
const UNREAD: &str = "effective_unread";
const MISSING: &str = "effective_missing";

/// Checks the access declaration a call names, as the capability read it:
/// what [`AccessDeclaration::parse`] refuses, in its order and words.
///
/// # Errors
///
/// The declaration's refusal.
pub(crate) fn check_arguments(
    arguments: &std::collections::BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    AccessDeclaration::parse(&Parameters(&rule)).map(|_| ())
}

/// The objects an `objects` argument names, as a memo keys them.
#[derive(Clone, Hash, PartialEq, Eq)]
enum Named {
    Kinds(String),
    Selection(SelectionIdentity),
}

/// The objects `key` names, borrowed where the call holds a bound
/// selection: every value of every element reads them, so they are never
/// copied or formatted for a read.
fn named<'c>(
    context: &RuleContext<'_>,
    call: &'c MeasuredCall,
    key: &str,
) -> Result<Option<(Cow<'c, MeasuredSelection>, Named)>, Unavailable> {
    match call.argument(key) {
        None => Ok(None),
        Some(MeasuredArgument::Objects(picked)) => Ok(Some((
            Cow::Borrowed(&**picked),
            Named::Selection(SelectionIdentity(picked.clone())),
        ))),
        Some(argument) => {
            let identity = Named::Kinds(format!("{argument:?}"));
            let picked = selection(context, call, key, None)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
            Ok(picked.map(|picked| (Cow::Owned(picked), identity)))
        }
    }
}

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => Some(*value),
        _ => None,
    }
}

fn property<'c>(call: &'c MeasuredCall, key: &str) -> Option<PropertyRef<'c>> {
    match call.argument(key) {
        Some(MeasuredArgument::Property { set, name }) => Some(PropertyRef {
            set: set.as_deref(),
            name,
        }),
        _ => None,
    }
}

/// The bound arguments an element's measurement depends on besides its
/// sources and blockers, in this order.
const SETTING: [&str; 11] = [
    "mode",
    "range",
    "touch_tolerance",
    "area_property",
    "access_path",
    "capacity_property",
    "capacity_multiplier",
    "capacity_multiplier_property",
    "door_selector",
    "opening_selector",
    "space_selector",
];

/// One bound argument as a memo keys it: a number by its bits, a selection
/// by its identity. Every value of every element builds one per argument,
/// so nothing is formatted and only what the call states is copied.
#[derive(Hash, PartialEq, Eq)]
enum Keyed {
    Number(bool, u64),
    Choice(&'static str),
    Property(Option<String>, String),
    Path(Vec<String>),
    Selection(SelectionIdentity),
    Other(String),
}

impl Keyed {
    fn of(argument: &MeasuredArgument) -> Self {
        match argument {
            MeasuredArgument::Length(value) => Self::Number(true, value.to_bits()),
            MeasuredArgument::Number(value) => Self::Number(false, value.to_bits()),
            MeasuredArgument::Choice(choice) => Self::Choice(choice),
            MeasuredArgument::Property { set, name } => Self::Property(set.clone(), name.clone()),
            MeasuredArgument::Path(steps) => Self::Path(steps.clone()),
            MeasuredArgument::Objects(picked) => Self::Selection(SelectionIdentity(picked.clone())),
            other => Self::Other(format!("{other:?}")),
        }
    }
}

/// The arguments the access index is built from.
const ACCESS: &[&str] = &[
    "access_path",
    "door_selector",
    "opening_selector",
    "space_selector",
];

/// The sources and blockers a call names, their extents read once per
/// selection.
struct Placed {
    sources: Arc<Candidates>,
    blockers: Arc<Candidates>,
}

fn placed(
    context: &RuleContext<'_>,
    services: &Services<'_>,
    (sources, sources_named): &(Cow<'_, MeasuredSelection>, Named),
    blockers: Option<&(Cow<'_, MeasuredSelection>, Named)>,
) -> Placed {
    #[derive(Hash, PartialEq, Eq)]
    struct Read(Named, Option<Named>);
    let source_candidates =
        MeasuredMemo::of(context.services, Read(sources_named.clone(), None), || {
            Arc::new(
                Candidates::read(
                    services.proximity,
                    sources.matched.clone(),
                    &sources.undecided,
                )
                .0,
            )
        });
    let blocker_candidates = match blockers {
        None => Arc::new(Candidates::read(services.proximity, BTreeSet::new(), &BTreeSet::new()).0),
        Some((blockers, blockers_named)) => MeasuredMemo::of(
            context.services,
            Read(blockers_named.clone(), Some(sources_named.clone())),
            || {
                // A source never blocks.
                let source = |object: &ObjectId| {
                    sources.matched.contains(object) || sources.undecided.contains(object)
                };
                let matched: BTreeSet<ObjectId> = blockers
                    .matched
                    .iter()
                    .filter(|object| !source(object))
                    .cloned()
                    .collect();
                let undecided: BTreeSet<ObjectId> = blockers
                    .undecided
                    .iter()
                    .filter(|object| !source(object))
                    .cloned()
                    .collect();
                Arc::new(Candidates::read(services.proximity, matched, &undecided).0)
            },
        ),
    };
    Placed {
        sources: source_candidates,
        blockers: blocker_candidates,
    }
}

/// The access index a call's path, elements and spaces declare, built once
/// per run for them.
fn index(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
) -> Result<Option<Arc<AccessIndex>>, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Access(ArgumentsKey);
    let Some(MeasuredArgument::Path(steps)) = call.argument("access_path") else {
        return Ok(None);
    };
    let key = Access(ArgumentsKey::of_keys(call, ACCESS));
    MeasuredMemo::of(context.services, key, || {
        let pick = |key: &str| {
            selection(context, call, key, None)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))
        };
        let (doors, openings, spaces) = (
            pick("door_selector")?,
            pick("opening_selector")?,
            pick("space_selector")?,
        );
        let access = AccessDeclaration::of(
            steps,
            doors.as_ref().map(Pick::Selected),
            openings.as_ref().map(Pick::Selected),
            spaces.as_ref().map(Pick::Selected),
        )?;
        Ok(Arc::new(access.index(context)))
    })
    .map(Some)
}

/// Where an element's measurement is cited.
fn locator(object: &Object) -> String {
    format!("effective-coverage:{}", object.id)
}

/// What one element's measurement came to, as the values read it.
struct Summary {
    /// Where it is cited, worded once for every value reading it.
    locator: String,
    area: (f64, f64),
    area_exact: bool,
    reaching: usize,
    covered: (f64, f64),
    share: (f64, f64),
    coverage_exact: bool,
    notes: Vec<String>,
    sure: Vec<ObjectId>,
    capacity: Option<Summed>,
}

/// The summed capacity, as the values read it.
struct Summed {
    lower: f64,
    upper: f64,
    unread: usize,
    exact: bool,
    unknown: Vec<String>,
    sure: Vec<ObjectId>,
    /// Each missing value: the source, the property and whether the
    /// absence was read exactly.
    missing: Vec<(ObjectId, String, bool)>,
}

/// Why an element has no summary: its stated area is missing (where the
/// absence was read, and why), or it cannot be measured.
#[derive(Clone)]
enum Unsummed {
    Missing(String),
    Unavailable(Unavailable),
}

/// The element's measurement, once per run for every value reading it.
fn summary(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &Object,
) -> Result<Arc<Summary>, Unsummed> {
    #[derive(Hash, PartialEq, Eq)]
    struct Cached(ObjectId, Named, Option<Named>, [Option<Keyed>; 11]);
    let unavailable = Unsummed::Unavailable;
    let services = Services::of(context).map_err(unavailable)?;
    let sources = named(context, call, "sources")
        .map_err(unavailable)?
        .ok_or_else(|| Unsummed::Unavailable(invalid("`sources` names no sources")))?;
    let blockers = named(context, call, "blockers").map_err(unavailable)?;
    let mode = call
        .choice("mode")
        .and_then(Mode::of)
        .unwrap_or(Mode::Grown);
    let range =
        length(call, "range").ok_or_else(|| Unsummed::Unavailable(invalid("range is required")))?;
    let key = Cached(
        object.id.clone(),
        sources.1.clone(),
        blockers.as_ref().map(|(_, named)| named.clone()),
        SETTING.map(|key| call.argument(key).map(Keyed::of)),
    );
    MeasuredMemo::of(context.services, key, || {
        let setting = Setting {
            mode,
            range,
            touch: length(call, "touch_tolerance").unwrap_or(0.0),
            area: property(call, "area_property"),
        };
        let placed = placed(context, &services, &sources, blockers.as_ref());
        let own = bounds(services.proximity, &object.id).map_err(|(reason, message)| {
            Unsummed::Unavailable((reason, format!("{message}; its coverage was not checked")))
        })?;
        let index = index(context, call).map_err(Unsummed::Unavailable)?;
        let reaching = placed.sources.near(&own, setting.margin());
        // A walk or a sight line reaching the element within the range
        // stays within the range of it, so only blockers there can cut it.
        let blocking = placed
            .blockers
            .near(&own, if index.is_some() { setting.range } else { 0.0 });
        let element = Element {
            context,
            setting: &setting,
            services: &services,
            around: Around {
                reaching: &reaching,
                blocking: &blocking,
                sources: &placed.sources.matched,
                blockers: &placed.blockers.matched,
                blind: placed.sources.blind.len(),
                blind_blockers: placed.blockers.blind.len(),
                index: index.as_deref(),
            },
            object,
        };
        let measured = match element.measure() {
            Ok(measured) => measured,
            Err(Unmeasured::Missing(message, cited)) => {
                let read = cited
                    .first()
                    .map_or_else(String::new, |cited| format!("{}: ", cited.locator));
                return Err(Unsummed::Missing(format!("{read}{message}")));
            }
            Err(Unmeasured::Unavailable(refused)) => return Err(Unsummed::Unavailable(refused)),
        };
        let covered = element.covered(&measured);
        let capacity = capacity(call).map(|capacity| {
            let summed = element.capacity(&measured, capacity);
            Summed {
                lower: summed.lower,
                upper: summed.upper,
                unread: summed.unread,
                exact: summed.evidence.iter().all(|cited| cited.exact),
                unknown: summed.unknown,
                sure: summed.sure,
                missing: summed
                    .missing
                    .into_iter()
                    .map(|(source, property, _, cited)| {
                        let exact = cited.iter().all(|cited| cited.exact);
                        (source, property, exact)
                    })
                    .collect(),
            }
        });
        Ok(Arc::new(Summary {
            locator: locator(object),
            area: (measured.area.lower, measured.area.upper),
            area_exact: measured.area.evidence.iter().all(|cited| cited.exact),
            reaching: measured.asked.request.sources().len(),
            covered: covered.covered,
            share: covered.share,
            coverage_exact: covered.evidence.iter().all(|cited| cited.exact),
            notes: covered.notes,
            sure: super::contributing(&measured.asked.request, &measured.coverage, true),
            capacity,
        }))
    })
}

/// The capacity a call declares, where it declares one.
fn capacity(call: &MeasuredCall) -> Option<Capacity<'_>> {
    let stated = property(call, "capacity_property")?;
    let multiplier = match (
        length(call, "capacity_multiplier"),
        property(call, "capacity_multiplier_property"),
    ) {
        (Some(constant), None) => Multiplier::Constant(constant),
        (None, Some(per_source)) => Multiplier::Property(per_source),
        _ => return None,
    };
    Some(Capacity {
        property: stated,
        multiplier,
    })
}

/// The value `call` names of `object`, with what it cites.
fn value(
    call: &MeasuredCall,
    object: &Object,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    // One measurement of the element, cited once however many values read
    // it, under the locator worded with it.
    let summary = match summary(context, call, object) {
        Ok(summary) => summary,
        // Nothing is measured of an element stating no area: its reach is
        // stated absent, the element's finding; every other value refused.
        Err(Unsummed::Missing(why)) if call.name() == REACHING => {
            return Ok((
                Measurement::Absent {
                    locator: format!("{}: {why}", self::locator(object)),
                },
                Citation::default(),
            ));
        }
        Err(Unsummed::Missing(_)) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the element states no area".to_owned(),
            ));
        }
        Err(Unsummed::Unavailable(refused)) => return Err(refused),
    };
    let locator = &summary.locator;
    let area = Some(QuantityDimension::Area);
    let cited = |(lower, upper): (f64, f64), dimension, exact| Measurement::Cited {
        lower,
        upper,
        dimension,
        locator: locator.clone(),
        exact,
    };
    Ok(match call.name() {
        REACHING => {
            #[allow(clippy::cast_precision_loss)]
            let count = summary.reaching as f64;
            (
                interval((count, count), None, true, locator.clone()),
                Citation::default(),
            )
        }
        AREA => (
            cited(summary.area, area, summary.area_exact),
            Citation::default(),
        ),
        COVERED => (
            cited(summary.covered, area, summary.coverage_exact),
            Citation::default(),
        ),
        SHARE => (
            cited(summary.share, None, summary.coverage_exact),
            Citation {
                // The sources surely contributing, which a finding relates.
                related: summary.sure.clone(),
                evidence: Vec::new(),
                // Why it may be covered more than measured.
                notes: summary.notes.clone(),
                sources: Vec::new(),
            },
        ),
        name => {
            let summed = summary
                .capacity
                .as_ref()
                .ok_or_else(|| invalid("no capacity is declared"))?;
            if name == UNREAD {
                #[allow(clippy::cast_precision_loss)]
                let count = summed.unread as f64;
                (
                    interval((count, count), None, true, locator.clone()),
                    Citation::default(),
                )
            } else {
                (
                    cited((summed.lower, summed.upper), area, summed.exact),
                    Citation {
                        related: summed.sure.clone(),
                        evidence: Vec::new(),
                        // Why the sum cannot be bounded above.
                        notes: summed.unknown.clone(),
                        sources: Vec::new(),
                    },
                )
            }
        }
    })
}

/// The missing values of `object`'s surely contributing sources.
fn missing(
    call: &MeasuredCall,
    object: &Object,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let summary = match summary(context, call, object) {
        Ok(summary) => summary,
        Err(Unsummed::Missing(_)) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the element states no area".to_owned(),
            ));
        }
        Err(Unsummed::Unavailable(refused)) => return Err(refused),
    };
    let summed = summary
        .capacity
        .as_ref()
        .ok_or_else(|| invalid("no capacity is declared"))?;
    Ok(summed
        .missing
        .iter()
        .map(|(source, property, exact)| MeasuredMember {
            certain: true,
            exact: *exact,
            fields: [
                (
                    "source",
                    MemberValue::Objects {
                        objects: vec![source.clone()],
                    },
                ),
                (
                    "property",
                    MemberValue::Text {
                        text: property.clone(),
                    },
                ),
                (
                    "missing",
                    MemberValue::Truth {
                        value: true,
                        locator: format!("{MISSING}:{}:{source}", object.id),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            evidence: Vec::new(),
        })
        .collect())
}

impl MeasuredProvider for EffectMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[REACHING, AREA, COVERED, SHARE, CAPACITY, UNREAD]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[MISSING]
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let refuse = refused(call.name(), object);
        let Some(found) = object_by_id(context, object) else {
            return Err(refuse((
                NotEvaluatedReason::IncompleteEvidence,
                "the object is not in the project".into(),
            )));
        };
        missing(call, found, context).map_err(refuse)
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

    /// Each element is measured once per run, whichever value reads it.
    fn memoizes(&self) -> bool {
        true
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let refuse = refused(call.name(), object);
        let Some(found) = object_by_id(context, object) else {
            return Err(refuse((
                NotEvaluatedReason::IncompleteEvidence,
                "the object is not in the project".into(),
            )));
        };
        value(call, found, context).map_err(refuse)
    }
}
