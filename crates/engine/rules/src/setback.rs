//! Setback areas in front of external walls (#326): `setback_depth`,
//! `setback_area`, `setback_outside_area` and `setback_overlap_area`.
//!
//! The depth is `factor × H`, at least `minimum`, where `H` is the wall's
//! top above the terrain (`above_terrain;of=top`, the mean along its own
//! outline, the terrain's accuracy included) plus `roof_share` of the rise
//! above the wall's top of every `roof` object of its source standing over
//! it in plan. A roof that may or may not stand over the wall raises only
//! the most. Every bound is rounded outward, so the terrain's accuracy
//! widens `H` and the depth with it; nothing here is a stated constant of
//! any code, every number is the rule's.
//!
//! The setback sweeps the wall's footprint straight out along each plan
//! direction its steep faces look outside (`measure_facade_faces`, as the
//! facade area classifies them), by the depth, and leaves the footprint
//! out (`PlanAreaServiceHandle::measure_setback`): faces partly outside
//! add only to the most. Which side faces out must be decided first, as
//! `azimuth` decides it: one direction must surely hold the most area. A
//! wall that looks out both ways, or several ways none surely largest, a
//! tessellated wall (its directions approximate), an unreadable height and
//! an unplaced zone leave the value not evaluated. A wall no steep face of
//! which looks outside (an internal wall) has no setback, exactly.

use std::collections::BTreeSet;

use axioval_engine::expression::Interval;
use axioval_engine::{
    Citation, FacadeAreaServiceHandle, FacadeFaceSide, MeasuredMemo, MeasuredProvider,
    MeasuredRead, MeasuredValues, Measurement, NotEvaluatedReason, PropertyResolutionError,
    RuleContext, SetbackArea, SetbackDirection, SetbackOther, SetbackRequest, SetbackStrip,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, PropertyValue, QuantityDimension};

use crate::facade_orientation::{Part, dominant, facade_unavailable};
use crate::measured_kinds::{every_object_of_kinds, interval, refused, selection};
use crate::support::Unavailable;

const DEPTH: &str = "setback_depth";
const AREA: &str = "setback_area";
const OUTSIDE: &str = "setback_outside_area";
const OVERLAP: &str = "setback_overlap_area";

/// Two outward plan directions are one when their unit vectors differ by
/// no more than this.
const SAME_DIRECTION: f64 = 1e-9;

/// Measures the setback values.
pub(crate) struct SetbackMeasures;

/// The memo key of a wall's setback depth.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DepthKey;

/// The memo key of a wall's outward plan directions.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DirectionsKey;

/// The memo key of a measured setback.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SetbackKey;

/// What a setback value's call states of the depth.
struct Parameters {
    terrain: &'static str,
    factor: f64,
    minimum: Option<f64>,
    roof: Option<(String, f64)>,
}

impl Parameters {
    fn of(call: &MeasuredCall) -> Result<Self, Unavailable> {
        let invalid = |why: &str| (NotEvaluatedReason::InvalidDeclaration, why.to_owned());
        let terrain = call
            .choice("terrain")
            .ok_or_else(|| invalid("no terrain is named"))?;
        let Some(MeasuredArgument::Number(factor)) = call.argument("factor") else {
            return Err(invalid("no factor is stated"));
        };
        let minimum = match call.argument("minimum") {
            Some(MeasuredArgument::Length(minimum)) => Some(*minimum),
            _ => None,
        };
        let roof = match (call.argument("roof"), call.argument("roof_share")) {
            (Some(MeasuredArgument::SourceKind(kinds)), Some(MeasuredArgument::Number(share))) => {
                Some((kinds.clone(), *share))
            }
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "`roof` is stated without `roof_share`, or `roof_share` without `roof`",
                ));
            }
        };
        Ok(Self {
            terrain,
            factor: *factor,
            minimum,
            roof,
        })
    }

    /// The parameters as one key, the same for the same depth.
    fn key(&self) -> String {
        format!(
            "{};{};{:?};{:?}",
            self.terrain,
            self.factor.to_bits(),
            self.minimum.map(f64::to_bits),
            self.roof
                .as_ref()
                .map(|(kinds, share)| (kinds, share.to_bits()))
        )
    }
}

/// A wall's setback depth and what it was measured from.
#[derive(Clone)]
struct Depth {
    bounds: (f64, f64),
    exact: bool,
    evidence: Vec<Evidence>,
    roofs: Vec<ObjectId>,
    notes: Vec<String>,
}

/// A value read: its bounds, whether exact, and its evidence.
type Read = ((f64, f64), bool, Option<Evidence>);

/// The value `name` of `object` as the run measures it.
fn read(context: &RuleContext<'_>, name: &str, object: &ObjectId) -> Result<Read, Unavailable> {
    let measure = |values: &MeasuredValues| {
        values
            .read_batch(name, &[object])
            .pop()
            .unwrap_or(Err(PropertyResolutionError::InvalidRequest))
    };
    let read = match context.services.get::<MeasuredValues>() {
        Some(values) => measure(values),
        None => measure(&MeasuredValues::of(context.services, context.project)),
    }
    .map_err(crate::selection::property_error)?;
    match read {
        MeasuredRead::Value(PropertyValue::Quantity { value, .. }, evidence) => {
            let exact = evidence.as_ref().is_none_or(|evidence| evidence.exact);
            Ok(((value, value), exact, evidence))
        }
        MeasuredRead::Value(PropertyValue::Measured { lower, upper, .. }, evidence) => {
            let exact = evidence.as_ref().is_some_and(|evidence| evidence.exact);
            Ok(((lower, upper), exact, evidence))
        }
        MeasuredRead::Value(other, _) => Err((
            NotEvaluatedReason::InvalidEvidence,
            format!("`{name}` of {object} is no length: {other:?}"),
        )),
        MeasuredRead::Absent(_) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{object} has no `{name}`"),
        )),
    }
}

fn span((lower, upper): (f64, f64)) -> Interval {
    Interval { lower, upper }
}

fn overflow<T>(_: T) -> Unavailable {
    (
        NotEvaluatedReason::InvalidEvidence,
        "the setback depth overflows".to_owned(),
    )
}

/// The rise above `wall`'s top of the `roof` objects standing over it:
/// the least of the sure ones' greatest, the most of every possible one's.
fn roof_rise(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    wall: &ObjectId,
    depth: &mut Depth,
) -> Result<Interval, Unavailable> {
    let candidates =
        every_object_of_kinds(context, call, "roof").map_err(crate::selection::property_error)?;
    let plan = crate::plan_area::service(context)?;
    let (top, top_exact, top_evidence) = read(context, "top", wall)?;
    let mut rise = Interval::point(0.0);
    for roof in candidates
        .iter()
        .filter(|roof| *roof != wall && roof.source == wall.source)
    {
        let overlap = plan.measure_plan_overlap(wall, roof).map_err(|error| {
            let (reason, why) = crate::plan_area::unavailable(error);
            (
                reason,
                format!("whether {roof} stands over the wall: {why}"),
            )
        })?;
        if overlap.upper_square_metres() <= 0.0 {
            continue;
        }
        let sure = overlap.lower_square_metres() > 0.0;
        let (roof_top, roof_exact, roof_evidence) = read(context, "top", roof)?;
        let above = span(roof_top).minus(span(top)).map_err(overflow)?;
        let above = Interval {
            lower: above.lower.max(0.0),
            upper: above.upper.max(0.0),
        };
        rise = if sure {
            rise.max(above)
        } else {
            depth.notes.push(format!(
                "{roof} may stand over the wall, so it raises only the most"
            ));
            Interval {
                lower: rise.lower,
                upper: rise.upper.max(above.upper),
            }
        };
        depth.exact &= sure && roof_exact && overlap.is_exact();
        depth.evidence.extend(roof_evidence);
        depth.evidence.push(overlap.evidence().clone());
        depth.roofs.push(roof.clone());
    }
    depth.exact &= top_exact;
    depth.evidence.extend(top_evidence);
    Ok(rise)
}

/// `wall`'s setback depth, once per run for the parameters.
fn depth(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
) -> Result<Depth, Unavailable> {
    let key = (DepthKey, wall.clone(), parameters.key());
    MeasuredMemo::of(context.services, key, || {
        let height_name = format!("above_terrain;terrain={};of=top", parameters.terrain);
        let (height, exact, evidence) = read(context, &height_name, wall)?;
        let mut depth = Depth {
            bounds: (0.0, 0.0),
            exact,
            evidence: evidence.into_iter().collect(),
            roofs: Vec::new(),
            notes: Vec::new(),
        };
        let mut h = span(height);
        if let Some((_, share)) = &parameters.roof {
            let rise = roof_rise(context, call, wall, &mut depth)?;
            h = h
                .plus(rise.times(Interval::point(*share)).map_err(overflow)?)
                .map_err(overflow)?;
        }
        let h = Interval {
            lower: h.lower.max(0.0),
            upper: h.upper.max(0.0),
        };
        let mut reach = h
            .times(Interval::point(parameters.factor))
            .map_err(overflow)?;
        if let Some(minimum) = parameters.minimum {
            reach = reach.max(Interval::point(minimum));
        }
        depth.bounds = (reach.lower, reach.upper);
        Ok(depth)
    })
}

/// The plan directions `wall` looks out along, each surely outward where
/// a face of it lies wholly outside; `None` where no steep face looks
/// outside.
fn directions(
    context: &RuleContext<'_>,
    wall: &ObjectId,
) -> Result<Option<(Vec<SetbackDirection>, Evidence)>, Unavailable> {
    MeasuredMemo::of(context.services, (DirectionsKey, wall.clone()), || {
        let faces = context
            .services
            .get::<FacadeAreaServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "facade-area service is not registered".to_owned(),
                )
            })?
            .measure_facade_faces(wall)
            .map_err(|error| facade_unavailable(&error))?;
        let parts: Vec<Part> = faces
            .faces()
            .iter()
            .filter_map(|face| {
                Part::of(
                    face.normal(),
                    face.area_square_metres(),
                    face.centroid(),
                    face.side(),
                )
            })
            .collect();
        // Steep: the normal leans nearer the horizontal than the vertical.
        let steep: Vec<(&Part, [f64; 2])> = parts
            .iter()
            .filter_map(|part| {
                let [x, y, z] = part.unit;
                let across = x.hypot(y);
                (across > z.abs()).then(|| (part, [x / across, y / across]))
            })
            .collect();
        if steep.is_empty() {
            return Ok(None);
        }
        if !faces.evidence().exact {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the wall is a tessellation, so the directions it looks out along are \
                 approximate"
                    .to_owned(),
            ));
        }
        // Which side faces out is decided as the azimuth decides it.
        dominant(&parts)?;
        let mut grouped: Vec<([f64; 2], bool)> = Vec::new();
        for (part, direction) in steep {
            let sure = part.side == FacadeFaceSide::Outside;
            match grouped.iter_mut().find(|(held, _)| {
                (held[0] - direction[0]).abs() <= SAME_DIRECTION
                    && (held[1] - direction[1]).abs() <= SAME_DIRECTION
            }) {
                Some((_, held)) => *held |= sure,
                None => grouped.push((direction, sure)),
            }
        }
        let directions = grouped
            .into_iter()
            .map(|(direction, sure)| SetbackDirection::try_new(direction, sure))
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::plan_area::unavailable)?;
        Ok(Some((directions, faces.evidence().clone())))
    })
}

/// `wall`'s setback as asked of the service, with what it was measured
/// from; `None` where the wall looks outside nowhere.
fn strip(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
) -> Result<Option<(SetbackStrip, Depth)>, Unavailable> {
    let Some((directions, faces)) = directions(context, wall)? else {
        return Ok(None);
    };
    let mut depth = depth(context, call, parameters, wall)?;
    depth.evidence.push(faces);
    let strip = SetbackStrip::try_new(wall.clone(), directions, depth.bounds)
        .map_err(crate::plan_area::unavailable)?;
    Ok(Some((strip, depth)))
}

/// The service's measure of `request`, once per run for `key`.
fn measured(
    context: &RuleContext<'_>,
    key: String,
    request: &SetbackRequest,
) -> Result<SetbackArea, Unavailable> {
    MeasuredMemo::of(context.services, (SetbackKey, key), || {
        crate::plan_area::service(context)?
            .measure_setback(request)
            .map_err(crate::plan_area::unavailable)
    })
}

/// The setbacks of the call's `others`, each measured as the wall's.
fn others(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
) -> Result<(Vec<SetbackOther>, Vec<Evidence>), Unavailable> {
    let picked = selection(context, call, "others", Some(wall))
        .map_err(crate::selection::property_error)?
        .ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                "no walls are named to compare".to_owned(),
            )
        })?;
    let undecided: BTreeSet<&ObjectId> = picked.undecided.iter().collect();
    let (mut others, mut evidence) = (Vec::new(), Vec::new());
    for other in picked.matched.iter().chain(&picked.undecided) {
        if other == wall {
            continue;
        }
        let compared = strip(context, call, parameters, other).map_err(|(reason, why)| {
            (
                reason,
                format!("the setback of {other}, compared, cannot be measured: {why}"),
            )
        })?;
        if let Some((strip, depth)) = compared {
            evidence.extend(depth.evidence);
            others.push(SetbackOther::new(strip, !undecided.contains(other)));
        }
    }
    Ok((others, evidence))
}

/// The polygons of the setback at the most, as a note a reviewer reads.
fn polygon_note(setback: &SetbackArea) -> Option<String> {
    let rings: Vec<String> = setback
        .most_polygons()
        .iter()
        .map(|polygon| {
            polygon
                .outer()
                .iter()
                .map(|[x, y]| format!("{x:.3} {y:.3}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .map(|ring| format!("({ring})"))
        .collect();
    (!rings.is_empty()).then(|| format!("setback polygon: {}", rings.join(" ")))
}

/// `setback_depth` of `wall` and what it was measured from.
fn of_depth(
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let depth = depth(context, call, parameters, wall)?;
    let locator = format!("{DEPTH}:{wall}:factor={}", parameters.factor);
    Ok((
        interval(
            depth.bounds,
            Some(QuantityDimension::Length),
            depth.exact,
            locator,
        ),
        Citation {
            related: depth.roofs,
            evidence: depth.evidence,
            notes: depth.notes,
            sources: Vec::new(),
        },
    ))
}

/// No area, exactly, with `evidence` and why.
fn no_area(locator: String, evidence: Vec<Evidence>, why: String) -> (Measurement, Citation) {
    (
        interval((0.0, 0.0), Some(QuantityDimension::Area), true, locator),
        Citation {
            related: Vec::new(),
            evidence,
            notes: vec![why],
            sources: Vec::new(),
        },
    )
}

/// What a setback request was built from: its memo key, whether the
/// statements it rests on are exact, and their citation.
struct Asked {
    request: SetbackRequest,
    key: String,
    exact: bool,
    citation: Citation,
}

/// The request `call` asks of `wall`'s setback; `Err(Ok(..))` where the
/// answer is no area without asking.
fn asked(
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
    context: &RuleContext<'_>,
    locator: &str,
) -> Result<Result<Asked, (Measurement, Citation)>, Unavailable> {
    let zone = if call.name() == OUTSIDE {
        Some(crate::context_zone::placed(context, call, wall)?)
    } else {
        None
    };
    let zone_evidence: Vec<Evidence> = zone
        .iter()
        .map(|zone| crate::context_zone::statement(zone, &wall.source))
        .collect();
    let Some((subject, depth)) = strip(context, call, parameters, wall)? else {
        return Ok(Err(no_area(
            locator.to_owned(),
            zone_evidence,
            format!("no steep face of {wall} looks outside, so it has no setback"),
        )));
    };
    let mut key = format!("{wall}|{}", parameters.key());
    let mut request = SetbackRequest::new(subject);
    let mut evidence = depth.evidence;
    let mut exact = depth.exact;
    if let Some(zone) = &zone {
        let polygons = crate::context_zone::polygons(zone)?.to_vec();
        request = request
            .with_zone(polygons, zone.accuracy_metres())
            .map_err(crate::plan_area::unavailable)?;
        key.push_str("|zone=");
        key.push_str(zone.locator());
        exact &= zone.is_exact();
    }
    evidence.extend(zone_evidence);
    if call.name() == OVERLAP {
        let (compared, compared_evidence) = others(context, call, parameters, wall)?;
        if compared.is_empty() {
            return Ok(Err(no_area(
                locator.to_owned(),
                evidence,
                "no compared wall looks outside".to_owned(),
            )));
        }
        for other in &compared {
            key.push('|');
            key.push_str(&other.strip().object().to_string());
            if !other.is_sure() {
                key.push('?');
            }
        }
        request = request
            .with_others(compared)
            .map_err(crate::plan_area::unavailable)?;
        evidence.extend(compared_evidence);
    }
    Ok(Ok(Asked {
        request,
        key,
        exact,
        citation: Citation {
            related: depth.roofs,
            evidence,
            notes: depth.notes,
            sources: Vec::new(),
        },
    }))
}

/// A setback area of `wall` and what it was measured from.
fn of_area(
    call: &MeasuredCall,
    parameters: &Parameters,
    wall: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let name = call.name();
    let mut locator = format!("{name}:{wall}:factor={}", parameters.factor);
    let Asked {
        request,
        key,
        mut exact,
        mut citation,
    } = match asked(call, parameters, wall, context, &locator)? {
        Ok(asked) => asked,
        Err(none) => return Ok(none),
    };
    let setback = measured(context, key, &request)?;
    exact &= setback.evidence().exact;
    citation.evidence.push(setback.evidence().clone());
    let missing = |what: &str| {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!("the service measured no {what}"),
        )
    };
    let bounds = match name {
        AREA => setback.area_square_metres(),
        OUTSIDE => setback
            .outside_square_metres()
            .ok_or_else(|| missing("part outside the zone"))?,
        _ => {
            // Named in the value's own evidence too, so a plain read
            // without the citation still says whose setbacks overlap.
            if let Some((first, rest)) = setback.overlapping().split_first() {
                locator.push_str(":overlapped-by=");
                locator.push_str(&first.to_string());
                for other in rest {
                    locator.push(',');
                    locator.push_str(&other.to_string());
                }
            }
            citation
                .related
                .extend(setback.overlapping().iter().cloned());
            setback
                .overlap_square_metres()
                .ok_or_else(|| missing("overlap"))?
        }
    };
    citation.notes.extend(polygon_note(&setback));
    citation.related.sort();
    citation.related.dedup();
    Ok((
        interval(bounds, Some(QuantityDimension::Area), exact, locator),
        citation,
    ))
}

/// A value of `wall` and what it was measured from.
fn of_wall(
    call: &MeasuredCall,
    wall: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let parameters = Parameters::of(call)?;
    if call.name() == DEPTH {
        of_depth(call, &parameters, wall, context)
    } else {
        of_area(call, &parameters, wall, context)
    }
}

impl MeasuredProvider for SetbackMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[AREA, DEPTH, OUTSIDE, OVERLAP]
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        of_wall(call, object, context).map_err(refused(call.name(), object))
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
}
