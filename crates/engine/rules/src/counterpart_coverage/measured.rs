//! How much of an element its counterparts leave uncovered, as measured
//! values, measured exactly as `counterpart-coverage` measures it for one
//! check: in plan, in height or in the element's elevation, the cover from
//! the plan broad phase, axis-compatible counterparts only with an axis
//! tolerance, and a frame's infill in the elevation.
//!
//! - `counterpart_uncovered_share`: the share left uncovered, citing the
//!   counterparts that surely cover part of it and noting why it may be
//!   covered more (a counterpart that cannot be read, axes undecided);
//! - `counterpart_uncovered` and `counterpart_whole`: the part uncovered
//!   and the whole it is a share of (an area, or a height);
//! - `counterpart_covering`: how many counterparts cover part of it, from
//!   those surely covering to every one that may, those that cannot be
//!   read included;
//! - `counterpart_infill`: in the elevation, whether a frame's infill
//!   covers (1), may cover (between 0 and 1) or does not (0), citing the
//!   frame's members.
//!
//! The counterparts are the objects `by` names, the frame members those
//! `frame` names: source kinds, or a rule's selection with the objects it
//! leaves undecided, which may cover. A counterpart whose extent cannot be
//! read may cover anything, so it drops the lower bound to zero. A
//! negative growth switches its check off, as the capability's tolerances
//! do: the cover is then measured with none, and the check refused.
//!
//! The counterparts' extents are read once per selection, and each
//! element's cover with every share its growths switch on once per run,
//! kept as the few numbers and words the values read.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Citation, MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Config, Counterparts, Cover, Services, Share, Subject};
use crate::measured_kinds::{refused, selection};
use crate::near::{Candidates, bounds};
use crate::orientation::Tri;
use crate::plan_area::footprint;
use crate::selection::object_by_id;
use crate::support::{Unavailable, invalid};

/// Measures `counterpart_uncovered_share`, `counterpart_uncovered`,
/// `counterpart_whole`, `counterpart_covering` and `counterpart_infill`.
pub(crate) struct CoverageMeasures;

const SHARE: &str = "counterpart_uncovered_share";
const UNCOVERED: &str = "counterpart_uncovered";
const WHOLE: &str = "counterpart_whole";
const COVERING: &str = "counterpart_covering";
const INFILL: &str = "counterpart_infill";

/// The check a call names.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Check {
    Plan,
    Height,
    /// Plan and height, as a coverage rule declares both: the cover in
    /// plan, the share of the footprint.
    Both,
    Elevation,
}

fn number(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => Some(*value),
        _ => None,
    }
}

/// The objects an `objects` argument names, as a memo keys them.
#[derive(Clone, Hash, PartialEq, Eq)]
enum Named {
    /// The objects of source kinds, as written.
    Kinds(String),
    /// A rule's selection, by identity.
    Selection(SelectionIdentity),
}

/// The counterparts `key` names, with how a memo keys them.
fn named(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<Option<(MeasuredSelection, Named)>, Unavailable> {
    let identity = match call.argument(key) {
        None => return Ok(None),
        Some(MeasuredArgument::Objects(picked)) => {
            Named::Selection(SelectionIdentity(picked.clone()))
        }
        Some(argument) => Named::Kinds(format!("{argument:?}")),
    };
    let picked = selection(context, call, key, None)
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
    Ok(picked.map(|picked| (picked, identity)))
}

/// Everything an element's cover depends on, as a memo keys it.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    by: Named,
    frame: Option<Named>,
    /// The bits of the growths (`None` when switched off), the axis
    /// tolerance and the infill share.
    horizontal: Option<u64>,
    vertical: Option<u64>,
    axis: Option<u64>,
    elevation: bool,
    infill: Option<u64>,
}

/// One call, read.
struct Asked {
    check: Check,
    /// The services this call needs, as the capability asked for them for
    /// its check.
    services: Config,
    /// What the element's cover is measured with: every check the growths
    /// switch on.
    cover: Config,
    by: MeasuredSelection,
    frame: Option<MeasuredSelection>,
    key: Key,
}

fn asked(context: &RuleContext<'_>, call: &MeasuredCall) -> Result<Asked, Unavailable> {
    let check = match call.choice("measure") {
        Some("height") => Check::Height,
        Some("elevation") => Check::Elevation,
        Some("plan_and_height") => Check::Both,
        _ => Check::Plan,
    };
    let on = |key: &str| {
        let growth = number(call, key).unwrap_or(0.0);
        (growth >= 0.0).then_some(growth)
    };
    let elevation = check == Check::Elevation;
    let (by, by_named) =
        named(context, call, "by")?.ok_or_else(|| invalid("`by` names no counterparts"))?;
    let frame = if elevation {
        named(context, call, "frame")?
    } else {
        None
    };
    let infill_above = number(call, "infill_above").unwrap_or(0.5);
    if !(0.0..1.0).contains(&infill_above) {
        return Err(invalid("infill_above must lie in [0, 1)"));
    }
    let cover = Config {
        horizontal: on("horizontal"),
        vertical: on("vertical"),
        axis: number(call, "axis_tolerance"),
        elevation,
        infill: frame.as_ref().map(|_| infill_above),
    };
    let services = Config {
        // A plan share needs no extents.
        vertical: if check == Check::Plan {
            None
        } else {
            cover.vertical
        },
        ..cover
    };
    let key = Key {
        by: by_named,
        frame: frame.as_ref().map(|(_, named)| named.clone()),
        horizontal: cover.horizontal.map(f64::to_bits),
        vertical: cover.vertical.map(f64::to_bits),
        axis: cover.axis.map(f64::to_bits),
        elevation,
        infill: cover.infill.map(f64::to_bits),
    };
    Ok(Asked {
        check,
        services,
        cover,
        by,
        frame: frame.map(|(selection, _)| selection),
        key,
    })
}

/// The candidates `picked` names, their extents read once per run.
fn candidates(
    context: &RuleContext<'_>,
    services: &Services<'_>,
    picked: &MeasuredSelection,
    named: &Named,
) -> Arc<Candidates> {
    #[derive(Hash, PartialEq, Eq)]
    struct Read(Named);
    MeasuredMemo::of(context.services, Read(named.clone()), || {
        Arc::new(
            Candidates::read(
                services.proximity,
                picked.matched.clone(),
                &picked.undecided,
            )
            .0,
        )
    })
}

/// The counterparts near `object` among `candidates`.
fn near(
    candidates: Arc<Candidates>,
    object: &ObjectId,
    own: &axioval_engine::ObjectBounds,
    margin: f64,
) -> Counterparts {
    let near = candidates.near(own, margin);
    Counterparts {
        candidates,
        near: BTreeMap::from([(object.clone(), near)]),
    }
}

/// One check's share, as the values read it.
#[derive(Clone)]
struct Part {
    share: (f64, f64),
    uncovered: (f64, f64),
    whole: (f64, f64),
    /// Measured from exact evidence, every counterpart that may cover read.
    exact: bool,
}

/// What an element's cover came to: the counterparts surely covering,
/// how many may, why it may be covered more, each check's share and the
/// frame's infill.
struct Summary {
    least: Vec<ObjectId>,
    counted: (usize, usize),
    notes: Vec<String>,
    plan: Option<Result<Part, Unavailable>>,
    height: Option<Result<Part, Unavailable>>,
    elevation: Option<Part>,
    infill: Option<(Tri, Vec<ObjectId>)>,
}

fn part(share: &Share, cover: &Cover) -> Part {
    Part {
        share: share.interval,
        uncovered: share.uncovered,
        whole: share.whole,
        exact: cover.unknown.is_empty() && share.evidence.iter().all(|cited| cited.exact),
    }
}

impl Summary {
    fn of(cover: &Cover) -> Self {
        // The counterparts surely covering (a frame's members are no
        // counterparts), to every one that may, unread ones included.
        let sure = cover
            .least
            .iter()
            .filter(|counterpart| cover.most.contains(counterpart))
            .count();
        Self {
            least: cover.least.clone(),
            counted: (sure, cover.most.len() + cover.unread),
            notes: cover.unknown.iter().chain(&cover.axes).cloned().collect(),
            plan: None,
            height: None,
            elevation: None,
            infill: None,
        }
    }
}

/// The element's cover and every share its growths switch on, measured
/// once per run for every value reading it.
fn summary(
    context: &RuleContext<'_>,
    asked: &Asked,
    object: &Object,
) -> Result<Arc<Summary>, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Cached(ObjectId, Key);
    // Every check's services but the extents, which only the height asks.
    let services = Services::of(
        context,
        &Config {
            vertical: None,
            ..asked.cover
        },
    )?;
    let key = Cached(object.id.clone(), asked.key.clone());
    MeasuredMemo::of(context.services, key, || {
        let counterparts = candidates(context, &services, &asked.by, &asked.key.by);
        let own = bounds(services.proximity, &object.id).map_err(|(reason, message)| {
            (reason, format!("{message}; its coverage was not checked"))
        })?;
        let margin = asked.cover.margin();
        let found = near(counterparts, &object.id, &own, margin);
        let frame = match (&asked.frame, &asked.key.frame) {
            (Some(picked), Some(named)) => Some(near(
                candidates(context, &services, picked, named),
                &object.id,
                &own,
                margin,
            )),
            _ => None,
        };
        let subject = Subject {
            config: &asked.cover,
            services: &services,
            counterparts: &found,
            frame: frame.as_ref(),
            object,
        };
        if asked.cover.elevation {
            let (share, cover, infill) = subject.elevation_share()?;
            let mut summary = Summary::of(&cover);
            summary.elevation = Some(part(&share, &cover));
            summary.infill = infill.map(|infill| (infill.applies, infill.members));
            return Ok(Arc::new(summary));
        }
        let area = footprint(context, &object.id)?;
        let cover = subject.cover(&area);
        let mut summary = Summary::of(&cover);
        summary.plan = asked.cover.horizontal.map(|growth| {
            subject
                .plan_share(&area, &cover, growth)
                .map(|share| part(&share, &cover))
        });
        summary.height = asked.cover.vertical.map(|growth| {
            let extents = context
                .services
                .get::<VerticalExtentServiceHandle>()
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::MissingService,
                        "vertical-extent service is not registered".to_owned(),
                    )
                })?;
            subject
                .height_share(extents, &cover, growth)
                .map(|share| part(&share, &cover))
        });
        Ok(Arc::new(summary))
    })
}

/// The value `call` names of `object`, with what it cites.
fn value(
    call: &MeasuredCall,
    object: &Object,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let asked = asked(context, call)?;
    // The services the call's check needs, before anything is measured.
    Services::of(context, &asked.services)?;
    let summary = summary(context, &asked, object)?;
    let locator = format!("{}:{}", call.name(), object.id);
    let cited = |lower: f64, upper: f64, dimension, exact| Measurement::Cited {
        lower,
        upper,
        dimension,
        locator: locator.clone(),
        exact,
    };
    match call.name() {
        COVERING => {
            #[allow(clippy::cast_precision_loss)]
            let (least, most) = (summary.counted.0 as f64, summary.counted.1 as f64);
            // Counted over what was read, exactly; the interval holds the
            // undecided cover.
            return Ok((cited(least, most, None, true), Citation::default()));
        }
        INFILL => {
            let (flag, members) = match &summary.infill {
                Some((applies, members)) => (
                    match applies {
                        Tri::Yes => (1.0, 1.0),
                        Tri::Maybe => (0.0, 1.0),
                        Tri::No => (0.0, 0.0),
                    },
                    members.clone(),
                ),
                None => ((0.0, 0.0), Vec::new()),
            };
            return Ok((
                cited(flag.0, flag.1, None, true),
                Citation {
                    related: members,
                    evidence: Vec::new(),
                    notes: Vec::new(),
                },
            ));
        }
        _ => {}
    }
    let part = match asked.check {
        Check::Plan | Check::Both => summary
            .plan
            .clone()
            .ok_or_else(|| invalid("the plan check is off: `horizontal` is negative"))??,
        Check::Height => summary
            .height
            .clone()
            .ok_or_else(|| invalid("the height check is off: `vertical` is negative"))??,
        Check::Elevation => summary
            .elevation
            .clone()
            .ok_or_else(|| invalid("the elevation is measured with both growths"))?,
    };
    let dimension = if asked.check == Check::Height {
        QuantityDimension::Length
    } else {
        QuantityDimension::Area
    };
    // Measured from exact evidence, the share is cited exact, as the
    // capability cites it; its interval holds the undecided cover, not
    // only rounding, so it is cited rather than rounded.
    Ok(match call.name() {
        UNCOVERED => (
            cited(
                part.uncovered.0,
                part.uncovered.1,
                Some(dimension),
                part.exact,
            ),
            Citation::default(),
        ),
        WHOLE => (
            cited(part.whole.0, part.whole.1, Some(dimension), part.exact),
            Citation::default(),
        ),
        _ => (
            cited(part.share.0, part.share.1, None, part.exact),
            Citation {
                // What surely covers part of it, which a finding relates.
                related: summary.least.clone(),
                evidence: Vec::new(),
                // Why it may be covered more than measured.
                notes: summary.notes.clone(),
            },
        ),
    })
}

impl MeasuredProvider for CoverageMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[SHARE, UNCOVERED, WHOLE, COVERING, INFILL]
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

    /// Each element's cover is measured once per run, whichever value
    /// reads it.
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
