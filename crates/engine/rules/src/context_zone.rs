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

use std::collections::BTreeSet;

use axioval_engine::PointBox;
use axioval_engine::{
    Citation, CoordinateSystemServiceHandle, LineDistanceRequest, LineProjection, MeasuredMemo,
    MeasuredProvider, Measurement, NotEvaluatedReason, PlacedZone, PlanPolygon,
    PropertyResolutionError, ProximityError, ProximityServiceHandle, RuleContext, SuppliedZones,
    ZoneArea, ZoneAreaRequest, ZoneReference, ZoneShape,
};
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

/// The zone a call names.
fn zone_id(call: &MeasuredCall) -> Result<&str, Unavailable> {
    match call.argument("zone") {
        Some(MeasuredArgument::Text(id)) => Ok(id),
        _ => Err((
            NotEvaluatedReason::InvalidDeclaration,
            "no context zone is named".to_owned(),
        )),
    }
}

/// The zone `call` names, placed in the model coordinates of `source`.
fn placed(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    source: &SourceId,
) -> Result<PlacedZone, Unavailable> {
    let id = zone_id(call)?.to_owned();
    MeasuredMemo::of(
        context.services,
        (PlacedKey, id.clone(), source.clone()),
        || {
            let supplied = context
                .services
                .get::<SuppliedZones>()
                .filter(|supplied| supplied.is_supplied())
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "no context zones were supplied beside the model, so `{id}` is unknown"
                        ),
                    )
                })?;
            let (file, zone) = supplied.zone(&id).ok_or_else(|| {
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
            zone.place(file, system.as_ref())
                .map_err(|why| (NotEvaluatedReason::IncompleteEvidence, why))
        },
    )
}

/// The zone's polygons; a line encloses no area.
fn polygons(zone: &PlacedZone) -> Result<&[PlanPolygon], Unavailable> {
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
fn statement(zone: &PlacedZone, source: &SourceId) -> Evidence {
    Evidence {
        source: source.clone(),
        locator: zone.locator().to_owned(),
        exact: zone.is_exact(),
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
    let zone = placed(context, call, &object.source)?;
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
    let zone = placed(context, call, &source)?;
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

/// A refusal of `name` of the project, as a property resolution states it.
fn refused_project(name: &str) -> impl Fn(Unavailable) -> PropertyResolutionError + '_ {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of the project: {why}")))
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
        ]
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
    let zone = placed(context, call, &object.source)?;
    let zone_polygons = polygons(&zone)?.to_vec();
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
                Location::Located(by_point(&zone, &zone_polygons, [x, y]), exact)
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
