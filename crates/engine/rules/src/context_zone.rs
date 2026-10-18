//! Context zones (#327): measured values relating objects to the zones
//! supplied beside the model (`SuppliedZones`), each zone placed in the
//! object's source coordinates first (`ContextZone::place`, refused where
//! the transform cannot be established).
//!
//! `zone_inside_area` and `zone_outside_area` are the object's footprint
//! inside and outside the zone (`PlanAreaServiceHandle::measure_zone_area`),
//! `zone_distance` the least plan distance from the footprint to the
//! zone's line or boundary (`ProximityServiceHandle::measure_line_distance`
//! per segment, its ends boxed by the zone's accuracy) and `zone_area` the
//! zone's own area. `zone_footprint_area` (the union of the selected
//! elements' footprints inside the zone, for GRZ) and `zone_floor_area`
//! (each element's footprint inside it, summed, for GFZ) are values of the
//! project: elements the selection surely picks make the least, every one
//! it may pick the most. Nothing is defaulted: an unsupplied zone, a zone
//! that cannot be placed and a footprint that cannot be read are not
//! evaluated, never inside.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::PointBox;
use axioval_engine::expression::{Leaf, Value as ExpressionValue};
use axioval_engine::{
    Citation, CoordinateSystemServiceHandle, LineDistanceRequest, LineProjection, MeasuredMember,
    MeasuredMemo, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason, PlacedZone,
    PlanPolygon, PropertyResolutionError, ProximityError, ProximityServiceHandle, RuleContext,
    SuppliedZones, ZONE_LOCATOR_PREFIX, ZoneArea, ZoneAreaRequest, ZoneReference, ZoneShape,
    ZoneValue,
};
use axioval_ir::contract::ScalarValue;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, SourceId};

use crate::measured_kinds::{interval, refused, resolution_error, selection};
use crate::support::Unavailable;

const ZONE_AREA: &str = "zone_area";
const ZONE_DISTANCE: &str = "zone_distance";
const ZONE_FLOOR_AREA: &str = "zone_floor_area";
const ZONE_FOOTPRINT_AREA: &str = "zone_footprint_area";
const ZONE_INSIDE_AREA: &str = "zone_inside_area";
const ZONE_OUTSIDE_AREA: &str = "zone_outside_area";
const ZONE_NEIGHBOURS: &str = "zone_neighbours";
const ZONE_ANGLE: &str = "zone_angle";
const ZONE_LENGTH: &str = "zone_length";
const ZONE_VALUE: &str = "zone_value";

/// Measures the values of context zones.
pub(crate) struct ZoneMeasures;

/// The memo key of a zone placed in one source.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PlacedKey;

/// The memo key of objects measured against a zone.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AreaKey;

/// The memo key of an object's distance to a zone.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DistanceKey;

/// The zone a call names by id.
fn zone_id(call: &MeasuredCall) -> Result<&str, Unavailable> {
    match call.argument("zone") {
        Some(MeasuredArgument::Text(id)) => Ok(id),
        _ => Err((
            NotEvaluatedReason::InvalidDeclaration,
            "no context zone is named".to_owned(),
        )),
    }
}

/// The run's supplied zones, or why there are none.
fn supplied<'a>(context: &RuleContext<'a>, wanted: &str) -> Result<&'a SuppliedZones, Unavailable> {
    context
        .services
        .get::<SuppliedZones>()
        .filter(|supplied| supplied.is_supplied())
        .ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!("no context zones were supplied beside the model, so {wanted} is unknown"),
            )
        })
}

/// The zone `id`, placed in the model coordinates of `source`.
pub(crate) fn placed_id(
    context: &RuleContext<'_>,
    id: &str,
    source: &SourceId,
) -> Result<PlacedZone, Unavailable> {
    MeasuredMemo::of(
        context.services,
        (PlacedKey, id.to_owned(), source.clone()),
        || {
            let supplied = supplied(context, &format!("`{id}`"))?;
            let (file, zone) = supplied.zone(id).ok_or_else(|| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("no supplied context zone is `{id}`"),
                )
            })?;
            let system = match file.reference() {
                ZoneReference::Model => None,
                ZoneReference::Map(name) => {
                    let handle = context
                        .services
                        .get::<CoordinateSystemServiceHandle>()
                        .ok_or_else(|| {
                            (
                                NotEvaluatedReason::MissingService,
                                "coordinate-system service is not registered".to_owned(),
                            )
                        })?;
                    Some(handle.coordinate_system(source).map_err(|error| {
                        (
                            NotEvaluatedReason::IncompleteEvidence,
                            format!(
                                "zone `{id}` is stated in `{name}`, and the coordinate system of \
                                 {source} cannot be read: {error}"
                            ),
                        )
                    })?)
                }
            };
            // In the local frame the source's geometry is measured in.
            let origin = context
                .services
                .get::<axioval_engine::LocalOrigins>()
                .map(|origins| origins.of(source))
                .unwrap_or_default();
            zone.place_in(file, system.as_ref(), origin)
                .map_err(|why| (NotEvaluatedReason::IncompleteEvidence, why))
        },
    )
}

/// The zone `call` names for `object`, placed in its source's model
/// coordinates: the zone of id `zone`, or the zone of kind `stands_in` the
/// object stands in ([`stands_in`]), with the evidence of that choice.
pub(crate) fn zone_for(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<(PlacedZone, Option<Evidence>), Unavailable> {
    if let Some(argument) = call.argument("stands_in") {
        let MeasuredArgument::Text(kind) = argument else {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                "no kind of context zone is named".to_owned(),
            ));
        };
        let (id, evidence) = stands_in(context, kind, object)?;
        return Ok((placed_id(context, &id, &object.source)?, Some(evidence)));
    }
    Ok((placed_id(context, zone_id(call)?, &object.source)?, None))
}

/// The zone `call` names for `object`, placed in its source's model
/// coordinates ([`zone_for`]).
pub(crate) fn placed(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<PlacedZone, Unavailable> {
    zone_for(context, call, object).map(|(zone, _)| zone)
}

/// The memo key of the zone of a kind an object stands in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StandsKey;

/// The memo key of an object's footprint extent.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ExtentKey;

/// The plan extent `[west, south, east, north]` of `object`'s footprint,
/// grown by its outline's deviation; `None` where the plan-area service
/// traces no outline of it, so nothing is ruled out by it.
fn footprint_extent(context: &RuleContext<'_>, object: &ObjectId) -> Option<[f64; 4]> {
    MeasuredMemo::of(context.services, (ExtentKey, object.clone()), || {
        let outline = crate::plan_area::service(context).ok().and_then(|service| {
            service
                .measure_footprint_outline(std::slice::from_ref(object))
                .ok()
        });
        Ok::<_, Unavailable>(outline.map(|outline| {
            let grow = outline.deviation_metres();
            outline.rings().iter().flatten().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |[west, south, east, north], [x, y]| {
                    [
                        west.min(x - grow),
                        south.min(y - grow),
                        east.max(x + grow),
                        north.max(y + grow),
                    ]
                },
            )
        }))
    })
    .ok()
    .flatten()
}

/// Whether `zone` surely lies apart from the extent `[west, south, east,
/// north]`: every boundary its accuracy allows keeps clear of it.
fn apart(zone: &PlacedZone, polygons: &[PlanPolygon], extent: [f64; 4]) -> bool {
    let mut scale = extent
        .iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    let [mut west, mut south, mut east, mut north] = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in polygons.iter().flat_map(PlanPolygon::rings).flatten() {
        scale = scale.max(x.abs()).max(y.abs());
        west = west.min(*x);
        south = south.min(*y);
        east = east.max(*x);
        north = north.max(*y);
    }
    let margin = zone.accuracy_metres() + 64.0 * f64::EPSILON * scale;
    west - margin > extent[2]
        || east + margin < extent[0]
        || south - margin > extent[3]
        || north + margin < extent[1]
}

/// The ids of `ids`, quoted and joined.
fn named(ids: &[&str]) -> String {
    ids.iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The id of the supplied area of `kind` that `object` stands in, and the
/// evidence of that choice: of every supplied area of the kind, the one
/// it surely lies in by [`locate_in`] while surely lying in no other.
/// Areas surely apart from its footprint's traced extent are ruled out
/// before anything else is measured.
///
/// # Errors
///
/// No zones supplied or none of the kind; a zone of the kind that cannot
/// be placed, or that the object may or may not lie in, or which does not
/// locate it; an object surely lying in several or in none. Which zone it
/// stands in is never guessed.
pub(crate) fn stands_in(
    context: &RuleContext<'_>,
    kind: &str,
    object: &ObjectId,
) -> Result<(String, Evidence), Unavailable> {
    let key = (StandsKey, kind.to_owned(), object.clone());
    MeasuredMemo::of(context.services, key, || {
        let supplied = supplied(context, &format!("the zone of kind `{kind}` it stands in"))?;
        let candidates: Vec<String> = supplied
            .of_kind(kind)
            .filter(|(_, zone)| matches!(zone.shape(), ZoneShape::Area(_)))
            .map(|(_, zone)| zone.id().to_owned())
            .collect();
        if candidates.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("no supplied context zone is an area of kind `{kind}`"),
            ));
        }
        let extent = footprint_extent(context, object);
        let (mut surely, mut possibly, mut unlocated) = (Vec::new(), Vec::new(), Vec::new());
        let mut exact = true;
        for id in &candidates {
            let zone = placed_id(context, id, &object.source).map_err(|(reason, why)| {
                (
                    reason,
                    format!("which zone of kind `{kind}` it stands in is undecided: {why}"),
                )
            })?;
            if extent.is_some_and(|extent| apart(&zone, polygons(&zone).unwrap_or(&[]), extent)) {
                continue;
            }
            match locate_in(context, &zone, object)? {
                Location::Located(InZone::Surely, sure) => {
                    exact &= sure;
                    surely.push(id.as_str());
                }
                Location::Located(InZone::Possibly, _) => possibly.push(id.as_str()),
                Location::Located(InZone::Not, sure) => exact &= sure,
                Location::Unlocated(why) => unlocated.push(format!("`{id}`: {why}")),
            }
        }
        if !unlocated.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "which zone of kind `{kind}` it stands in is undecided, since it cannot be \
                     located against {}",
                    unlocated.join("; ")
                ),
            ));
        }
        match (surely.as_slice(), possibly.as_slice()) {
            ([one], []) => Ok((
                (*one).to_owned(),
                Evidence {
                    source: object.source.clone(),
                    locator: format!("{ZONE_LOCATOR_PREFIX};stands_in={kind};id={one}:{object}"),
                    exact,
                    witness: None,
                },
            )),
            ([], []) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("it stands in no supplied context zone of kind `{kind}`"),
            )),
            (_, []) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it stands in {} zones of kind `{kind}`, {}, so which one is undecided",
                    surely.len(),
                    named(&surely)
                ),
            )),
            _ => {
                let all: Vec<&str> = surely.iter().chain(&possibly).copied().collect();
                Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "it may stand in {} of kind `{kind}`: its footprint straddles a \
                         boundary or lies within its accuracy of one, so which zone it stands \
                         in is undecided",
                        named(&all)
                    ),
                ))
            }
        }
    })
}

/// The zone's polygons; a line encloses no area.
pub(crate) fn polygons(zone: &PlacedZone) -> Result<&[PlanPolygon], Unavailable> {
    match zone.shape() {
        ZoneShape::Area(polygons) => Ok(polygons),
        ZoneShape::Line(_) => Err((
            NotEvaluatedReason::InvalidDeclaration,
            format!(
                "zone `{}` is a {}, a line, which encloses no area",
                zone.id(),
                zone.kind()
            ),
        )),
    }
}

/// The zone's statement, as evidence beside a measurement of `source`.
pub(crate) fn statement(zone: &PlacedZone, source: &SourceId) -> Evidence {
    Evidence {
        source: source.clone(),
        locator: zone.locator().to_owned(),
        exact: zone.is_exact(),
        witness: None,
    }
}

/// The union of `objects`' footprints against the zone, once per run.
fn zone_area(
    context: &RuleContext<'_>,
    zone: &PlacedZone,
    objects: Vec<ObjectId>,
) -> Result<ZoneArea, Unavailable> {
    let key = (AreaKey, objects.clone(), zone.locator().to_owned());
    MeasuredMemo::of(context.services, key, || {
        let polygons = polygons(zone)?.to_vec();
        let request = ZoneAreaRequest::try_new(objects, polygons, zone.accuracy_metres())
            .map_err(crate::plan_area::unavailable)?;
        crate::plan_area::service(context)?
            .measure_zone_area(&request)
            .map_err(crate::plan_area::unavailable)
    })
}

/// Why a proximity measurement failed, in a report's words.
fn proximity_error(error: &ProximityError) -> Unavailable {
    let reason = match error {
        ProximityError::NoBody
        | ProximityError::Unavailable
        | ProximityError::Refused(_)
        | ProximityError::UnsupportedProjection => NotEvaluatedReason::BackendUnavailable,
        _ => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("proximity: {error}"))
}

/// The least plan distance from `object`'s footprint to the zone's line or
/// boundary, the nearest of its segments, each end boxed by the zone's
/// accuracy.
fn distance(
    context: &RuleContext<'_>,
    zone: &PlacedZone,
    object: &ObjectId,
) -> Result<(f64, f64, bool, Vec<Evidence>), Unavailable> {
    let key = (DistanceKey, object.clone(), zone.locator().to_owned());
    MeasuredMemo::of(context.services, key, || {
        let service = context
            .services
            .get::<ProximityServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "proximity service is not registered".to_owned(),
                )
            })?;
        let reach = zone.accuracy_metres();
        let boxed = |[x, y]: [f64; 2]| {
            PointBox::try_new([x - reach, y - reach, 0.0], [x + reach, y + reach, 0.0]).map_err(
                |error| {
                    (
                        NotEvaluatedReason::InvalidEvidence,
                        format!("zone `{}`: {error}", zone.id()),
                    )
                },
            )
        };
        let (mut lower, mut upper) = (f64::INFINITY, f64::INFINITY);
        let mut exact = zone.is_exact();
        let mut evidence = Vec::new();
        for (start, end) in zone.shape().segments() {
            let request = LineDistanceRequest::new(
                object.clone(),
                boxed(start)?,
                boxed(end)?,
                LineProjection::Plan,
            );
            let measured = service
                .measure_line_distance(&request)
                .map_err(|error| proximity_error(&error))?;
            lower = lower.min(measured.lower_metres());
            upper = upper.min(measured.upper_metres());
            exact &= measured.evidence().exact;
            if evidence.is_empty() || !measured.evidence().exact {
                evidence.push(measured.evidence().clone());
            }
        }
        if !upper.is_finite() {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                format!("zone `{}` has no segment", zone.id()),
            ));
        }
        Ok((lower, upper, exact, evidence))
    })
}

/// A value of `object` and what it was measured from.
fn of_object(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let (zone, chosen) = zone_for(context, call, object)?;
    let zone_evidence = statement(&zone, &object.source);
    let locator = format!("{}:{object}:zone={}", call.name(), zone.id());
    let (bounds, dimension, exact, mut evidence) = match call.name() {
        ZONE_AREA => {
            let bounds = zone.area_square_metres().ok_or_else(|| {
                (
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("zone `{}` is a line, which encloses no area", zone.id()),
                )
            })?;
            (bounds, QuantityDimension::Area, zone.is_exact(), Vec::new())
        }
        ZONE_DISTANCE => {
            let (lower, upper, exact, evidence) = distance(context, &zone, object)?;
            ((lower, upper), QuantityDimension::Length, exact, evidence)
        }
        _ => {
            let area = zone_area(context, &zone, vec![object.clone()])?;
            let bounds = if call.name() == ZONE_INSIDE_AREA {
                area.inside_square_metres()
            } else {
                area.outside_square_metres()
            };
            (
                bounds,
                QuantityDimension::Area,
                area.evidence().exact && zone.is_exact(),
                vec![area.evidence().clone()],
            )
        }
    };
    evidence.push(zone_evidence);
    evidence.extend(chosen);
    Ok((
        interval(bounds, Some(dimension), exact, locator),
        Citation {
            related: Vec::new(),
            evidence,
            notes: Vec::new(),
            sources: Vec::new(),
        },
    ))
}

/// A project value over the selected elements: the footprint they cover
/// in the zone together, or the sum of each one's footprint in it.
fn of_project(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let picked = selection(context, call, "elements", None)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?
        .ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                "no elements are named".to_owned(),
            )
        })?;
    let every: BTreeSet<&ObjectId> = picked.matched.iter().chain(&picked.undecided).collect();
    let sources: BTreeSet<&SourceId> = every.iter().map(|object| &object.source).collect();
    let id = zone_id(call)?;
    let locator = format!(
        "{}:zone={id}:elements={}",
        call.name(),
        picked.matched.len()
    );
    let mut notes = Vec::new();
    if !picked.undecided.is_empty() {
        notes.push(format!(
            "{} elements the selection leaves undecided may add to it",
            picked.undecided.len()
        ));
    }
    let source = match sources.into_iter().collect::<Vec<_>>().as_slice() {
        [] => {
            // Nothing is selected: no footprint, exactly.
            return Ok((
                interval((0.0, 0.0), Some(QuantityDimension::Area), true, locator),
                Citation::default(),
            ));
        }
        [source] => (*source).clone(),
        several => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the elements lie in {} sources, whose footprints are not measured together",
                    several.len()
                ),
            ));
        }
    };
    let zone = placed_id(context, id, &source)?;
    let mut evidence = vec![statement(&zone, &source)];
    let mut exact = zone.is_exact() && picked.undecided.is_empty();
    let (mut lower, mut upper) = (0.0_f64, 0.0_f64);
    if call.name() == ZONE_FOOTPRINT_AREA {
        let matched: Vec<ObjectId> = picked.matched.iter().cloned().collect();
        if !matched.is_empty() {
            let sure = zone_area(context, &zone, matched)?;
            lower = sure.inside_square_metres().0;
            upper = sure.inside_square_metres().1;
            exact &= sure.evidence().exact;
            evidence.push(sure.evidence().clone());
        }
        if !picked.undecided.is_empty() {
            let all = zone_area(context, &zone, every.into_iter().cloned().collect())?;
            upper = upper.max(all.inside_square_metres().1);
            evidence.push(all.evidence().clone());
        }
    } else {
        let elements = picked
            .matched
            .iter()
            .map(|element| (element, true))
            .chain(picked.undecided.iter().map(|element| (element, false)));
        for (element, sure) in elements {
            let area = zone_area(context, &zone, vec![element.clone()])?;
            let (least, most) = area.inside_square_metres();
            if sure {
                lower += least;
            }
            upper += most;
            exact &= area.evidence().exact;
            evidence.push(area.evidence().clone());
        }
        // The running sums rounded outward.
        let count = f64::from(u32::try_from(evidence.len()).unwrap_or(u32::MAX));
        let slack = 8.0 * f64::EPSILON * count;
        lower = (lower * (1.0 - slack)).next_down().max(0.0);
        upper = (upper * (1.0 + slack)).next_up();
    }
    Ok((
        interval(
            (lower, upper.max(lower)),
            Some(QuantityDimension::Area),
            exact,
            locator,
        ),
        Citation {
            related: Vec::new(),
            evidence,
            notes,
            sources: Vec::new(),
        },
    ))
}

/// The id of the zone `call` names for `object`, without placing it where
/// it is named by id, with the evidence of the choice where it is the zone
/// of a kind the object stands in.
fn zone_id_for(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<(String, Option<Evidence>), Unavailable> {
    if let Some(argument) = call.argument("stands_in") {
        let MeasuredArgument::Text(kind) = argument else {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                "no kind of context zone is named".to_owned(),
            ));
        };
        let (id, evidence) = stands_in(context, kind, object)?;
        return Ok((id, Some(evidence)));
    }
    Ok((zone_id(call)?.to_owned(), None))
}

/// A value the zone `call` names states (`zone_value`, `zone_length`,
/// `zone_angle`, #321): as stated and cited as the zone's statement, an
/// angle stated in degrees in radians rounded outward; `null` where the
/// zone states none; not evaluated where it states it in a way that is
/// not read.
fn stated(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let key = call.choice("value").ok_or_else(|| {
        (
            NotEvaluatedReason::InvalidDeclaration,
            "no stated value is named".to_owned(),
        )
    })?;
    let (id, chosen) = zone_id_for(context, call, object)?;
    let supplied = supplied(context, &format!("`{id}`"))?;
    let (file, zone) = supplied.zone(&id).ok_or_else(|| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("no supplied context zone is `{id}`"),
        )
    })?;
    let mut locator = format!("{};value={key}", zone.locator(file));
    let mut evidence = Vec::new();
    evidence.extend(chosen);
    let (value, degrees) = match zone.values().get(key) {
        None => {
            return Ok((
                Measurement::Absent { locator },
                Citation {
                    evidence,
                    ..Citation::default()
                },
            ));
        }
        Some(ZoneValue::Unread { why }) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("zone `{id}` states `{key}` in a way that is not read: {why}"),
            ));
        }
        Some(ZoneValue::Stated {
            value,
            degrees,
            statement,
        }) => {
            locator.push_str(";stated=");
            locator.push_str(statement);
            (*value, *degrees)
        }
        Some(ZoneValue::Text { statement, .. } | ZoneValue::Quantity { statement, .. }) => {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "zone `{id}` states `{key}` ({statement}) as text or a quantity of its own \
                     unit, not as `{}` reads it; read it with a `zone` expression",
                    call.name()
                ),
            ));
        }
    };
    let dimension = call.descriptor.dimension;
    let bounds = if degrees && dimension == Some(QuantityDimension::PlaneAngle) {
        let radians = value.to_radians();
        let slack = 4.0 * f64::EPSILON * radians.abs();
        ((radians - slack).next_down(), (radians + slack).next_up())
    } else {
        (value, value)
    };
    evidence.push(Evidence {
        source: object.source.clone(),
        locator: locator.clone(),
        exact: true,
        witness: None,
    });
    Ok((
        interval(bounds, dimension, true, locator),
        Citation {
            related: Vec::new(),
            evidence,
            notes: Vec::new(),
            sources: Vec::new(),
        },
    ))
}

/// What the context zone `id` states under `value` (its id without one),
/// as a `zone` expression reads it (#324), for an object of `source`: as
/// stated and cited as the zone's statement (`…;value=<key>;stated=…`),
/// beside the evidence of how the zone was chosen; a key the zone does not
/// state `null`; one it states in a way that is not read unreadable. A
/// zone named by id is not placed to read what it states.
pub(crate) fn zone_statement(
    context: &RuleContext<'_>,
    id: &str,
    value: Option<&str>,
    source: &SourceId,
    chosen: Option<Evidence>,
) -> Result<Leaf, Unavailable> {
    use axioval_ir::measured::{ZONE_ANGLES, ZONE_LENGTHS};
    let supplied = supplied(context, &format!("`{id}`"))?;
    let (file, zone) = supplied.zone(id).ok_or_else(|| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("no supplied context zone is `{id}`"),
        )
    })?;
    let mut evidence: Vec<Evidence> = chosen.into_iter().collect();
    let cite = |locator: String, evidence: &mut Vec<Evidence>| {
        evidence.push(Evidence {
            source: source.clone(),
            locator,
            exact: true,
            witness: None,
        });
    };
    let Some(key) = value else {
        cite(zone.locator(file), &mut evidence);
        return Ok(Leaf {
            value: Ok(ExpressionValue::Text(id.to_owned())),
            evidence,
        });
    };
    let locator = format!("{};value={key}", zone.locator(file));
    let quantity = |value: f64, unit: &str| {
        ExpressionValue::from_literal(&ScalarValue::Quantity {
            value,
            unit: unit.to_owned(),
        })
    };
    let (read, statement) = match zone.values().get(key) {
        None => {
            cite(locator, &mut evidence);
            return Ok(Leaf {
                value: Ok(ExpressionValue::Null),
                evidence,
            });
        }
        Some(ZoneValue::Unread { why }) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("zone `{id}` states `{key}` in a way that is not read: {why}"),
            ));
        }
        Some(ZoneValue::Stated {
            value,
            degrees,
            statement,
        }) => {
            let read = if *degrees {
                quantity(*value, "deg")
            } else if ZONE_LENGTHS.contains(&key) {
                quantity(*value, "m")
            } else if ZONE_ANGLES.contains(&key) {
                quantity(*value, "rad")
            } else {
                ExpressionValue::from_literal(&ScalarValue::Number { value: *value })
            };
            (read, statement)
        }
        Some(ZoneValue::Text { text, statement }) => {
            (Ok(ExpressionValue::Text(text.clone())), statement)
        }
        Some(ZoneValue::Quantity {
            value,
            unit,
            statement,
        }) => (quantity(*value, unit), statement),
    };
    cite(format!("{locator};stated={statement}"), &mut evidence);
    Ok(Leaf {
        value: read,
        evidence,
    })
}

/// A refusal of `name` of the project, as a property resolution states it.
fn refused_project(name: &str) -> impl Fn(Unavailable) -> PropertyResolutionError + '_ {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of the project: {why}")))
}

/// How `other` borders `zone`, both placed in one source: `Some(true)`
/// where their stated boundaries share a stretch of some length (the
/// stretch is one statement of both), `Some(false)` where they share none
/// but come within the sum of their accuracies of each other, `None`
/// where they lie farther apart, or both are exact and meet at points
/// only.
fn borders(zone: &PlacedZone, other: &PlacedZone) -> Option<bool> {
    let (ours, theirs) = (zone.shape().segments(), other.shape().segments());
    let scale = ours
        .iter()
        .chain(&theirs)
        .flat_map(|(start, end)| [start, end])
        .fold(0.0_f64, |scale, [x, y]| scale.max(x.abs()).max(y.abs()));
    let tolerance = 64.0 * f64::EPSILON * scale;
    let reach = zone.accuracy_metres() + other.accuracy_metres();
    let mut nearest = f64::INFINITY;
    for &(p, q) in &ours {
        for &(r, s) in &theirs {
            if shares_stretch((p, q), (r, s), tolerance) {
                return Some(true);
            }
            nearest = nearest.min(segment_distance((p, q), (r, s)));
        }
    }
    (reach > 0.0 && nearest <= reach + tolerance).then_some(false)
}

/// Whether segment `r s` lies along segment `p q`, within `tolerance`,
/// overlapping it by more than `tolerance`.
fn shares_stretch(
    ([px, py], [qx, qy]): ([f64; 2], [f64; 2]),
    ([rx, ry], [sx, sy]): ([f64; 2], [f64; 2]),
    tolerance: f64,
) -> bool {
    let length = (qx - px).hypot(qy - py);
    if length <= tolerance {
        return false;
    }
    let (ux, uy) = ((qx - px) / length, (qy - py) / length);
    let across = |x: f64, y: f64| (ux * (y - py) - uy * (x - px)).abs();
    let along = |x: f64, y: f64| ux.mul_add(x - px, uy * (y - py));
    if across(rx, ry) > tolerance || across(sx, sy) > tolerance {
        return false;
    }
    let (a, b) = (along(rx, ry), along(sx, sy));
    length.min(a.max(b)) - a.min(b).max(0.0) > tolerance
}

/// The least plan distance between two segments.
fn segment_distance((p, q): ([f64; 2], [f64; 2]), (r, s): ([f64; 2], [f64; 2])) -> f64 {
    let turn = |[ax, ay]: [f64; 2], [bx, by]: [f64; 2], [cx, cy]: [f64; 2]| {
        (bx - ax).mul_add(cy - ay, -((by - ay) * (cx - ax)))
    };
    let crossing = turn(p, q, r) * turn(p, q, s) < 0.0 && turn(r, s, p) * turn(r, s, q) < 0.0;
    if crossing {
        return 0.0;
    }
    let to_segment = |[x, y]: [f64; 2], [ax, ay]: [f64; 2], [bx, by]: [f64; 2]| {
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx.mul_add(dx, dy * dy);
        let along = if length > 0.0 {
            ((x - ax).mul_add(dx, (y - ay) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (x - along.mul_add(dx, ax)).hypot(y - along.mul_add(dy, ay))
    };
    to_segment(p, r, s)
        .min(to_segment(q, r, s))
        .min(to_segment(r, p, q))
        .min(to_segment(s, p, q))
}

/// The supplied areas beside the zone `call` names for `object`
/// (`zone_neighbours`, #319), in the order supplied, of the `kinds` it
/// lists: surely beside where [`borders`] finds a shared stretch, possibly
/// beside where it finds them within their accuracies. Every zone of the
/// kinds is placed in the object's source, and one that cannot be leaves
/// the whole list not evaluated, never shorter.
fn neighbours(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
    let (zone, chosen) = zone_for(context, call, object)?;
    polygons(&zone)?;
    let kinds: Option<Vec<&str>> = match call.argument("kinds") {
        Some(MeasuredArgument::Text(kinds)) => Some(
            kinds
                .split(',')
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
                .collect(),
        ),
        _ => None,
    };
    let candidates: Vec<String> = supplied(context, "a neighbouring zone")?
        .all()
        .filter(|(_, other)| {
            other.id() != zone.id()
                && matches!(other.shape(), ZoneShape::Area(_))
                && kinds
                    .as_ref()
                    .is_none_or(|kinds| kinds.contains(&other.kind()))
        })
        .map(|(_, other)| other.id().to_owned())
        .collect();
    let mut members = Vec::new();
    for id in candidates {
        let other = placed_id(context, &id, &object.source)?;
        let Some(sure) = borders(&zone, &other) else {
            continue;
        };
        let exact = zone.is_exact() && other.is_exact();
        let area = other.area_square_metres().ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                format!("zone `{id}` encloses no area"),
            )
        })?;
        let locator = format!(
            "{ZONE_NEIGHBOURS}:{object}:zone={}:neighbour={id}",
            zone.id()
        );
        let fields = BTreeMap::from([
            ("zone", MemberValue::Text { text: id.clone() }),
            (
                "kind",
                MemberValue::Text {
                    text: other.kind().to_owned(),
                },
            ),
            (
                "area",
                MemberValue::Measured(interval(
                    area,
                    Some(QuantityDimension::Area),
                    exact,
                    locator,
                )),
            ),
        ]);
        members.push(MeasuredMember {
            certain: sure,
            exact,
            fields,
            evidence: vec![statement(&other, &object.source)],
        });
    }
    let mut evidence = vec![statement(&zone, &object.source)];
    evidence.extend(chosen);
    Ok((members, evidence))
}

impl MeasuredProvider for ZoneMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            ZONE_AREA,
            ZONE_DISTANCE,
            ZONE_FLOOR_AREA,
            ZONE_FOOTPRINT_AREA,
            ZONE_INSIDE_AREA,
            ZONE_OUTSIDE_AREA,
            ZONE_ANGLE,
            ZONE_LENGTH,
            ZONE_VALUE,
        ]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ZONE_NEIGHBOURS]
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        neighbours(call, object, context).map_err(refused(call.name(), object))
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        match call.name() {
            ZONE_FLOOR_AREA | ZONE_FOOTPRINT_AREA => {
                of_project(call, context).map_err(refused(call.name(), object))
            }
            ZONE_ANGLE | ZONE_LENGTH | ZONE_VALUE => {
                stated(call, object, context).map_err(refused(call.name(), object))
            }
            _ => of_object(call, object, context).map_err(refused(call.name(), object)),
        }
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

    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        of_project(call, context).map_err(refused_project(call.name()))
    }
}

/// The memo key of where an object lies against a zone.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct LocatedKey;

/// Whether an object lies in a context zone (#360).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InZone {
    /// Wholly inside under every boundary the zone's accuracy allows.
    Surely,
    /// Straddling the boundary, or within its accuracy of it.
    Possibly,
    /// Wholly outside under every boundary its accuracy allows.
    Not,
}

/// Where an object lies against a context zone, or why it cannot be told.
#[derive(Clone, Debug)]
pub(crate) enum Location {
    /// Located, with whether everything that placed it is exact.
    Located(InZone, bool),
    /// Neither its footprint nor its placement locates it.
    Unlocated(String),
}

/// Where `object` lies against the zone `call` names: by its footprint
/// (`PlanAreaServiceHandle::measure_zone_area`), surely in where some of it
/// is surely inside and none of it possibly outside, surely out where none
/// of it is possibly inside; or, where it has no footprint or one of no
/// area (a tree stated as a point, an element placed without a body), by
/// its placement point, surely in or out where the point lies farther than
/// the zone's accuracy from its boundary. A zero-area footprint is never
/// read as outside: it encloses nothing inside either.
///
/// # Errors
///
/// An unsupplied, unplaceable or line zone, and a missing plan-area or
/// (where the placement is read) object-frame service: the value is not
/// evaluated, never zero. An object neither its footprint nor its
/// placement locates is [`Location::Unlocated`], left to the caller.
pub(crate) fn locate(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<Location, Unavailable> {
    locate_in(context, &placed(context, call, object)?, object)
}

/// Where `object` lies against `zone`, placed in its source, as
/// [`locate`] locates it.
///
/// # Errors
///
/// As [`locate`].
pub(crate) fn locate_in(
    context: &RuleContext<'_>,
    zone: &PlacedZone,
    object: &ObjectId,
) -> Result<Location, Unavailable> {
    let zone_polygons = polygons(zone)?.to_vec();
    let key = (LocatedKey, object.clone(), zone.locator().to_owned());
    MeasuredMemo::of(context.services, key, || {
        let service = crate::plan_area::service(context)?;
        let request = ZoneAreaRequest::try_new(
            vec![object.clone()],
            zone_polygons.clone(),
            zone.accuracy_metres(),
        )
        .map_err(crate::plan_area::unavailable)?;
        let footprint = match service.measure_zone_area(&request) {
            Ok(area) if area.footprint_square_metres().1 > 0.0 => {
                let exact = area.evidence().exact && zone.is_exact();
                return Ok(Location::Located(by_area(&area), exact));
            }
            Ok(_) => "its footprint has no area",
            Err(axioval_engine::PlanAreaError::UnknownObject(_)) => "it has no footprint",
            Err(error) => {
                return Ok(Location::Unlocated(format!(
                    "its footprint cannot be measured: {error}"
                )));
            }
        };
        let frames = context
            .services
            .get::<axioval_engine::ObjectFrameServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    format!(
                        "{object}: {footprint}, and the object-frame service that places it \
                         is not registered"
                    ),
                )
            })?;
        Ok(match frames.object_frame(object) {
            Ok(frame) => {
                let [x, y, _] = frame.frame().origin().coordinates_metres();
                let exact = frame.evidence().exact && zone.is_exact();
                Location::Located(by_point(zone, &zone_polygons, [x, y]), exact)
            }
            Err(error) => Location::Unlocated(format!(
                "{footprint}, and its placement cannot be read: {error}"
            )),
        })
    })
}

/// Where a footprint of some area lies, from its areas inside and outside.
fn by_area(area: &ZoneArea) -> InZone {
    let (inside, outside) = (area.inside_square_metres(), area.outside_square_metres());
    if inside.1 <= 0.0 {
        InZone::Not
    } else if inside.0 > 0.0 && outside.1 <= 0.0 {
        InZone::Surely
    } else {
        InZone::Possibly
    }
}

/// Where a point lies: inside the zone's polygons by the crossings of its
/// rings, decided only farther than the zone's accuracy (and the rounding
/// of these computations) from every boundary segment.
fn by_point(zone: &PlacedZone, polygons: &[PlanPolygon], [x, y]: [f64; 2]) -> InZone {
    let mut inside = false;
    let mut nearest = f64::INFINITY;
    let mut scale = x.abs() + y.abs();
    for ring in polygons.iter().flat_map(PlanPolygon::rings) {
        for (&[ax, ay], &[bx, by]) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            scale = scale.max(ax.abs() + ay.abs()).max(bx.abs() + by.abs());
            if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
                inside = !inside;
            }
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx.mul_add(dx, dy * dy);
            let along = if length > 0.0 {
                ((x - ax).mul_add(dx, (y - ay) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            nearest = nearest.min((x - along.mul_add(dx, ax)).hypot(y - along.mul_add(dy, ay)));
        }
    }
    let rounding = 64.0 * f64::EPSILON * scale;
    if nearest.is_nan() || nearest <= zone.accuracy_metres() + rounding {
        InZone::Possibly
    } else if inside {
        InZone::Surely
    } else {
        InZone::Not
    }
}
