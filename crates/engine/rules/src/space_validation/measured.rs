//! What `space-validation` measures of each space and of the project, as
//! measured values and member lists, each aspect asked of the space
//! service with one request, as the capability asked it.
//!
//! - `space_duplicates`: how many other spaces have the same body, citing
//!   them;
//! - `space_height`: the clear height;
//! - `space_uncovered_boundary`: the summed length of the boundary runs at
//!   least `segment` long no element covers, citing the elements along them;
//! - `space_intersections` and the list `space_overlaps`: the bodies the
//!   space contains, is contained by or intersects;
//! - `space_cap`: the share of the top or bottom cap the elements cover,
//!   citing them;
//! - `space_supports`, of the project: how many slabs could form a cap,
//!   asked only where a checked cap names no elements;
//! - `unallocated_regions` and `unallocated_storeys`, of the project: each
//!   region of storey floor belonging to no space, and each storey's
//!   unallocated share of its gross floor area.
//!
//! Elements a value names (`elements`) are the rule's selection: one that
//! leaves objects undecided refuses the value, worded as the capability
//! worded it (`boundary elements could not be selected: …`), and one of
//! nothing states the value absent: nothing to judge the space by.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BoundaryRequest, Cap, CapRequest, Citation, MeasuredMember, MeasuredMemo, MeasuredProvider,
    Measurement, MemberValue, OverlapRequest, PropertyResolutionError, RuleContext, SpaceError,
    SpaceService, SpaceServiceHandle, SupportCounts, UnallocatedRegion,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, QuantityDimension};

use super::reason;
use crate::measured_kinds::{interval, refused, resolution_error};
use crate::support::Unavailable;

/// Measures the space-validation values and lists.
pub(crate) struct SpaceMeasures;

const CAP: &str = "space_cap";
const DUPLICATES: &str = "space_duplicates";
const HEIGHT: &str = "space_height";
const INTERSECTIONS: &str = "space_intersections";
const SUPPORTS: &str = "space_supports";
const UNCOVERED: &str = "space_uncovered_boundary";
const OVERLAPS: &str = "space_overlaps";
const REGIONS: &str = "unallocated_regions";
const STOREYS: &str = "unallocated_storeys";

fn service<'c>(context: &'c RuleContext<'_>) -> Result<&'c dyn SpaceService, Unavailable> {
    context
        .services
        .get::<SpaceServiceHandle>()
        .map(SpaceServiceHandle::get)
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "space service is not registered".to_owned(),
            )
        })
}

/// The service's evidence, asked once per run: every value cites it.
fn evidence(service: &dyn SpaceService, context: &RuleContext<'_>) -> Arc<Evidence> {
    #[derive(Hash, PartialEq, Eq)]
    struct Key;
    MeasuredMemo::of(context.services, Key, || Arc::new(service.evidence()))
}

fn refusal(error: &SpaceError) -> Unavailable {
    (reason(error), error.to_string())
}

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => *value,
        _ => 0.0,
    }
}

fn truth(call: &MeasuredCall, key: &str) -> bool {
    matches!(call.argument(key), Some(MeasuredArgument::Truth(true)))
}

/// What the elements a call names come to.
enum Elements<'c> {
    /// None named: the service's own.
    Default,
    /// The elements picked, every one decided.
    Picked(&'c Arc<MeasuredSelection>),
    /// The selection names nothing: nothing to judge by.
    Nothing,
}

/// The objects `picked` picks, as a request names them.
fn picked(picked: &MeasuredSelection) -> Vec<ObjectId> {
    picked.matched.iter().cloned().collect()
}

/// The elements `key` names, `name` wording a selection that cannot
/// decide them (`boundary elements could not be selected: …`).
fn elements<'c>(
    call: &'c MeasuredCall,
    key: &str,
    name: &str,
) -> Result<Elements<'c>, Unavailable> {
    let picked = match call.argument(key) {
        None => return Ok(Elements::Default),
        Some(MeasuredArgument::Objects(picked)) => picked,
        Some(_) => {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                format!("`{key}` names no selection"),
            ));
        }
    };
    if !picked.undecided.is_empty() {
        let (reason, message) = picked.first_undecided.clone().unwrap_or((
            NotEvaluatedReason::IncompleteEvidence,
            "an object's selection is undecided".to_owned(),
        ));
        return Err((
            reason,
            format!("{name} elements could not be selected: {message}"),
        ));
    }
    Ok(if picked.matched.is_empty() {
        Elements::Nothing
    } else {
        Elements::Picked(picked)
    })
}

/// The model's supports, asked of the service once per run.
fn supports(context: &RuleContext<'_>) -> Result<SupportCounts, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Key;
    MeasuredMemo::of(context.services, Key, || {
        service(context)?
            .measure_support_counts()
            .map_err(|error| refusal(&error))
    })
}

/// A space's reportable overlaps under `call`'s elements and tolerance,
/// measured once per run for the value and the list; `None` where the
/// selection names nothing.
fn overlaps(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    space: &ObjectId,
) -> Result<Option<Arc<Vec<axioval_engine::SpaceOverlap>>>, Unavailable> {
    // The elements by the selection's identity: a rule binds one selection
    // into each value naming it.
    #[derive(Hash, PartialEq, Eq)]
    struct Key(ObjectId, Option<SelectionIdentity>, u64);
    let named = match elements(call, "elements", "intersection")? {
        Elements::Nothing => return Ok(None),
        Elements::Default => None,
        Elements::Picked(picked) => Some(picked),
    };
    let tolerance = length(call, "tolerance");
    MeasuredMemo::of(
        context.services,
        Key(
            space.clone(),
            named.map(|named| SelectionIdentity(Arc::clone(named))),
            tolerance.to_bits(),
        ),
        || {
            let mut request = OverlapRequest::new();
            if let Some(named) = named {
                request = request.with_elements(picked(named));
            }
            let overlaps = service(context)?
                .measure_overlaps(space, &request)
                .map_err(|error| refusal(&error))?;
            // Contact, not intersection, as `SpaceOverlap::intersects` reads it.
            Ok(Arc::new(
                overlaps
                    .into_iter()
                    .filter(|overlap| overlap.intersects(tolerance))
                    .collect(),
            ))
        },
    )
    .map(Some)
}

/// A count, exact as the service's evidence is.
#[allow(clippy::cast_precision_loss)]
fn count(value: usize, evidence: &Evidence) -> Measurement {
    let value = value as f64;
    interval(
        (value, value),
        None,
        evidence.exact,
        evidence.locator.clone(),
    )
}

/// The value `call` names of `space`, with what it cites.
#[allow(clippy::too_many_lines)]
fn value(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    space: &ObjectId,
) -> Result<(Measurement, Citation), Unavailable> {
    let service = service(context)?;
    let evidence = evidence(service, context);
    let absent = || {
        Ok((
            Measurement::Absent {
                locator: evidence.locator.clone(),
            },
            Citation::default(),
        ))
    };
    let cited = |related: Vec<ObjectId>| Citation {
        related,
        ..Citation::default()
    };
    match call.name() {
        DUPLICATES => {
            let duplicates = service
                .measure_duplicates(space)
                .map_err(|error| refusal(&error))?;
            Ok((count(duplicates.len(), &evidence), cited(duplicates)))
        }
        HEIGHT => {
            let height = service
                .measure_clear_height(space)
                .map_err(|error| refusal(&error))?;
            Ok((
                interval(
                    height.bounds_metres(),
                    Some(QuantityDimension::Length),
                    evidence.exact,
                    evidence.locator.clone(),
                ),
                Citation::default(),
            ))
        }
        UNCOVERED => {
            let request = match elements(call, "elements", "boundary")? {
                Elements::Nothing => return absent(),
                Elements::Default => BoundaryRequest::new(),
                Elements::Picked(named) => BoundaryRequest::new().with_elements(picked(named)),
            };
            let gaps = service
                .measure_boundary_gaps(space, &request)
                .map_err(|error| refusal(&error))?;
            // Only gaps at least as long as the declared segment count.
            let segment = length(call, "segment");
            let counted = || gaps.iter().filter(|gap| gap.length_metres() >= segment);
            let total: f64 = counted()
                .map(axioval_engine::BoundaryGap::length_metres)
                .sum();
            Ok((
                interval(
                    (total, total),
                    Some(QuantityDimension::Length),
                    evidence.exact,
                    evidence.locator.clone(),
                ),
                cited(
                    counted()
                        .flat_map(|gap| gap.elements().iter().cloned())
                        .collect(),
                ),
            ))
        }
        INTERSECTIONS => match overlaps(context, call, space)? {
            None => absent(),
            Some(overlaps) => Ok((count(overlaps.len(), &evidence), Citation::default())),
        },
        CAP => {
            if !truth(call, "check") {
                return absent();
            }
            let cap = if call.choice("cap") == Some("bottom") {
                Cap::Bottom
            } else {
                Cap::Top
            };
            let name = if cap == Cap::Top {
                "top cap"
            } else {
                "bottom cap"
            };
            let request = match elements(call, "elements", name)? {
                Elements::Nothing => return absent(),
                Elements::Picked(named) => CapRequest::new(cap).with_elements(picked(named)),
                // Without elements of its own, only a model with elements
                // that could form the cap checks it.
                Elements::Default => {
                    let counts = supports(context)?;
                    let available = match cap {
                        Cap::Top => counts.slabs() > 0 || counts.roofs() > 0,
                        Cap::Bottom => counts.slabs() > 0,
                    };
                    if !available {
                        return absent();
                    }
                    CapRequest::new(cap)
                }
            };
            let coverage = service
                .measure_cap_coverage(space, &request)
                .map_err(|error| refusal(&error))?;
            Ok((
                interval(
                    coverage.covered_ratio_bounds(),
                    None,
                    evidence.exact,
                    evidence.locator.clone(),
                ),
                cited(coverage.elements().to_vec()),
            ))
        }
        _ => Err((
            NotEvaluatedReason::InvalidDeclaration,
            format!("`{}` is measured of the project", call.name()),
        )),
    }
}

/// The supports of the project: asked only where a checked cap names no
/// elements of its own, zero otherwise.
fn project_value(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
) -> Result<Measurement, Unavailable> {
    let evidence = evidence(service(context)?, context);
    let needed = (truth(call, "top") && call.argument("top_elements").is_none())
        || (truth(call, "bottom") && call.argument("bottom_elements").is_none());
    if !needed {
        return Ok(count(0, &evidence));
    }
    Ok(count(supports(context)?.slabs(), &evidence))
}

fn objects(objects: Vec<ObjectId>) -> MemberValue {
    MemberValue::Objects { objects }
}

fn number(
    value: Option<(f64, f64)>,
    dimension: Option<QuantityDimension>,
    locator: String,
) -> MemberValue {
    MemberValue::Measured(match value {
        Some((lower, upper)) => Measurement::Value {
            lower,
            upper,
            dimension,
            locator,
        },
        None => Measurement::Absent { locator },
    })
}

fn truth_of(value: bool, locator: String) -> MemberValue {
    MemberValue::Truth { value, locator }
}

const AREA: Option<QuantityDimension> = Some(QuantityDimension::Area);

/// The reportable overlaps of `space`.
fn overlap_members(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    space: &ObjectId,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    use axioval_engine::Containment;
    let exact = evidence(service(context)?, context).exact;
    let Some(overlaps) = overlaps(context, call, space)? else {
        return Ok(Vec::new());
    };
    Ok(overlaps
        .iter()
        .enumerate()
        .map(|(index, overlap)| {
            let at = |field: &str| format!("{OVERLAPS}:{space}#{}:{field}", index + 1);
            let containment = overlap.containment();
            let partial = containment == Containment::Partial;
            let area = overlap.area_square_metres();
            MeasuredMember {
                certain: true,
                exact,
                evidence: Vec::new(),
                fields: BTreeMap::from([
                    (
                        "inside",
                        truth_of(containment == Containment::SubjectInsideOther, at("inside")),
                    ),
                    (
                        "contains",
                        truth_of(
                            containment == Containment::OtherInsideSubject,
                            at("contains"),
                        ),
                    ),
                    ("partial", truth_of(partial, at("partial"))),
                    (
                        "space",
                        truth_of(partial && overlap.other_is_space(), at("space")),
                    ),
                    ("area", number(Some((area, area)), AREA, at("area"))),
                    ("other", objects(vec![overlap.other().clone()])),
                ]),
            }
        })
        .collect())
}

/// The model's unallocated regions, asked of the service once per run.
fn regions(context: &RuleContext<'_>) -> Result<Arc<Vec<UnallocatedRegion>>, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Key;
    MeasuredMemo::of(context.services, Key, || {
        service(context)?
            .measure_unallocated_regions()
            .map(Arc::new)
            .map_err(|error| refusal(&error))
    })
}

/// The project's list `call` names, with the service's evidence.
fn project_members(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
    let evidence = evidence(service(context)?, context);
    let exact = evidence.exact;
    let regions = regions(context)?;
    let members = if call.name() == REGIONS {
        regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                let at = |field: &str| format!("{REGIONS}#{}:{field}", index + 1);
                let area = region.area_square_metres();
                MeasuredMember {
                    certain: true,
                    exact,
                    evidence: Vec::new(),
                    fields: BTreeMap::from([
                        ("storey", objects(vec![region.storey().clone()])),
                        ("area", number(Some((area, area)), AREA, at("area"))),
                        ("elements", objects(region.elements().to_vec())),
                    ]),
                }
            })
            .collect()
    } else {
        let mut per_storey: BTreeMap<&ObjectId, Vec<&UnallocatedRegion>> = BTreeMap::new();
        for region in regions.iter() {
            per_storey.entry(region.storey()).or_default().push(region);
        }
        per_storey
            .into_iter()
            .map(|(storey, regions)| {
                let at = |field: &str| format!("{STOREYS}:{storey}:{field}");
                // The share `axioval:measured` `unallocated_share` answers too.
                let share = UnallocatedRegion::storey_share(&regions);
                MeasuredMember {
                    certain: true,
                    exact,
                    evidence: Vec::new(),
                    fields: BTreeMap::from([
                        ("storey", objects(vec![storey.clone()])),
                        (
                            "share",
                            number(share.map(|share| share.share()), None, at("share")),
                        ),
                        (
                            "area",
                            number(
                                share.map(|share| {
                                    let (lower, _) = share.area_square_metres();
                                    (lower, lower)
                                }),
                                AREA,
                                at("area"),
                            ),
                        ),
                        (
                            "gross",
                            number(
                                share.map(|share| {
                                    let gross = share.gross_floor_area_square_metres();
                                    (gross, gross)
                                }),
                                AREA,
                                at("gross"),
                            ),
                        ),
                        (
                            "elements",
                            objects(
                                regions
                                    .iter()
                                    .flat_map(|region| region.elements().iter().cloned())
                                    .collect(),
                            ),
                        ),
                    ]),
                }
            })
            .collect()
    };
    Ok((members, vec![(*evidence).clone()]))
}

impl MeasuredProvider for SpaceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[CAP, DUPLICATES, HEIGHT, INTERSECTIONS, SUPPORTS, UNCOVERED]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[OVERLAPS, REGIONS, STOREYS]
    }

    /// A rule reads each value of a space once, as the capability measured
    /// it once per rule; what several values read (the overlaps, the
    /// supports, the unallocated regions) is kept for the run here, and
    /// nothing a second time.
    fn memoizes(&self) -> bool {
        true
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

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        value(context, call, object).map_err(refused(call.name(), object))
    }

    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        project_value(context, call)
            .map(|measured| (measured, Citation::default()))
            .map_err(resolution_error)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        overlap_members(context, call, object).map_err(refused(call.name(), object))
    }

    fn members_of_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        project_members(context, call).map_err(resolution_error)
    }
}
