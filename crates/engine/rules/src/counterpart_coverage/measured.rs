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
//! The counterparts' extents are read once per run, each element's cover
//! and shares once for every value and rule reading them.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Citation, MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason, PlanArea,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Candidates, Config, Counterparts, Cover, Infill, Services, Share, Subject, bounds};
use crate::measured_kinds::{refused, selection};
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

/// Everything a coverage measurement depends on, as a memo keys it.
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
    config: Config,
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
    let config = Config {
        horizontal: on("horizontal"),
        vertical: if check == Check::Plan {
            None
        } else {
            on("vertical")
        },
        axis: number(call, "axis_tolerance"),
        elevation,
        infill: frame.as_ref().map(|_| infill_above),
    };
    let key = Key {
        by: by_named,
        frame: frame.as_ref().map(|(_, named)| named.clone()),
        horizontal: config.horizontal.map(f64::to_bits),
        vertical: if elevation {
            config.vertical.map(f64::to_bits)
        } else {
            None
        },
        axis: config.axis.map(f64::to_bits),
        elevation,
        infill: config.infill.map(f64::to_bits),
    };
    Ok(Asked {
        check,
        config,
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

/// What one element's cover came to: in plan, its footprint and cover; in
/// the elevation, the whole share.
#[derive(Clone)]
enum Measured {
    Plan {
        area: PlanArea,
        cover: Cover,
    },
    Elevation {
        share: Share,
        cover: Cover,
        infill: Option<Infill>,
    },
}

/// The element's cover, measured once per run for every value reading it:
/// the services `asked`'s check needs first, then the element's extent.
fn measured(
    context: &RuleContext<'_>,
    asked: &Asked,
    object: &Object,
) -> Result<(Arc<Measured>, Arc<Candidates>), Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Cached(ObjectId, Key);
    let services = Services::of(context, &asked.config)?;
    let counterparts = candidates(context, &services, &asked.by, &asked.key.by);
    let key = Cached(object.id.clone(), asked.key.clone());
    let measured: Result<Arc<Measured>, Unavailable> =
        MeasuredMemo::of(context.services, key, || {
            let own = bounds(services.proximity, &object.id).map_err(|(reason, message)| {
                (reason, format!("{message}; its coverage was not checked"))
            })?;
            let margin = asked.config.margin();
            let found = near(counterparts.clone(), &object.id, &own, margin);
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
                config: &asked.config,
                services: &services,
                counterparts: &found,
                frame: frame.as_ref(),
                object,
            };
            if asked.config.elevation {
                let (share, cover, infill) = subject.elevation_share()?;
                return Ok(Arc::new(Measured::Elevation {
                    share,
                    cover,
                    infill,
                }));
            }
            let area = footprint(context, &object.id)?;
            let cover = subject.cover(&area);
            Ok(Arc::new(Measured::Plan { area, cover }))
        });
    Ok((measured?, counterparts))
}

/// The share `asked`'s check measures of `object`, with the cover it was
/// measured against.
fn share(
    context: &RuleContext<'_>,
    asked: &Asked,
    object: &Object,
) -> Result<(Share, Cover), Unavailable> {
    /// One share of an element's cover, as a memo keys it: the element,
    /// the cover, whether in height and the vertical growth.
    #[derive(Hash, PartialEq, Eq)]
    struct Shared(ObjectId, Key, bool, Option<u64>);
    let (measured, counterparts) = measured(context, asked, object)?;
    match measured.as_ref() {
        Measured::Elevation { share, cover, .. } => Ok((share.clone(), cover.clone())),
        Measured::Plan { area, cover } => {
            let services = Services::of(context, &asked.config)?;
            let found = Counterparts {
                candidates: counterparts,
                near: BTreeMap::new(),
            };
            let subject = Subject {
                config: &asked.config,
                services: &services,
                counterparts: &found,
                frame: None,
                object,
            };
            let height = asked.check == Check::Height;
            let key = Shared(
                object.id.clone(),
                asked.key.clone(),
                height,
                asked.config.vertical.map(f64::to_bits),
            );
            let share: Result<Share, Unavailable> = MeasuredMemo::of(context.services, key, || {
                if height {
                    let (Some(growth), Some(extents)) = (asked.config.vertical, services.extents)
                    else {
                        return Err(invalid("the height check is off: `vertical` is negative"));
                    };
                    subject.height_share(extents, cover, growth)
                } else {
                    let growth = asked.config.horizontal.ok_or_else(|| {
                        invalid("the plan check is off: `horizontal` is negative")
                    })?;
                    subject.plan_share(area, cover, growth)
                }
            });
            Ok((share?, cover.clone()))
        }
    }
}

/// The value `call` names of `object`, with what it cites.
fn value(
    call: &MeasuredCall,
    object: &Object,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let asked = asked(context, call)?;
    let locator = format!("{}:{}", call.name(), object.id);
    match call.name() {
        COVERING => {
            let (measured, _) = measured(context, &asked, object)?;
            let cover = match measured.as_ref() {
                Measured::Plan { cover, .. } | Measured::Elevation { cover, .. } => cover,
            };
            // The counterparts surely covering (a frame's members are no
            // counterparts), to every one that may, unread ones included.
            let sure = cover
                .least
                .iter()
                .filter(|counterpart| cover.most.contains(counterpart))
                .count();
            #[allow(clippy::cast_precision_loss)]
            let (least, most) = (sure as f64, (cover.most.len() + cover.unread) as f64);
            // Counted over what was read, exactly; the interval holds the
            // undecided cover.
            return Ok((
                Measurement::Cited {
                    lower: least,
                    upper: most,
                    dimension: None,
                    locator,
                    exact: true,
                },
                Citation::default(),
            ));
        }
        INFILL => {
            let (measured, _) = measured(context, &asked, object)?;
            let (flag, members) = match measured.as_ref() {
                Measured::Elevation {
                    infill: Some(infill),
                    ..
                } => (
                    match infill.applies {
                        Tri::Yes => (1.0, 1.0),
                        Tri::Maybe => (0.0, 1.0),
                        Tri::No => (0.0, 0.0),
                    },
                    infill.members.clone(),
                ),
                _ => ((0.0, 0.0), Vec::new()),
            };
            return Ok((
                Measurement::Cited {
                    lower: flag.0,
                    upper: flag.1,
                    dimension: None,
                    locator,
                    exact: true,
                },
                Citation {
                    related: members,
                    evidence: Vec::new(),
                    notes: Vec::new(),
                },
            ));
        }
        _ => {}
    }
    let (share, cover) = share(context, &asked, object)?;
    // Measured from exact evidence, the share is cited exact, as the
    // capability cites it; its interval holds the undecided cover, not
    // only rounding, so it is cited rather than rounded.
    let exact = cover.unknown.is_empty() && share.evidence.iter().all(|cited| cited.exact);
    let dimension = if asked.check == Check::Height {
        QuantityDimension::Length
    } else {
        QuantityDimension::Area
    };
    let ((lower, upper), dimension) = match call.name() {
        UNCOVERED => (share.uncovered, Some(dimension)),
        WHOLE => (share.whole, Some(dimension)),
        _ => (share.interval, None),
    };
    let citation = if call.name() == SHARE {
        Citation {
            // What surely covers part of it, which a finding relates.
            related: cover.least.clone(),
            evidence: Vec::new(),
            // Why it may be covered more than measured.
            notes: cover.unknown.iter().chain(&cover.axes).cloned().collect(),
        }
    } else {
        Citation::default()
    };
    Ok((
        Measurement::Cited {
            lower,
            upper,
            dimension,
            locator,
            exact,
        },
        citation,
    ))
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
