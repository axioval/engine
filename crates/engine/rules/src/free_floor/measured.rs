//! The free-floor search as measured member lists, both answered by
//! [`Search::place`]:
//!
//! - `free_floor_fit`, the capabilities' own declaration under their
//!   parameters' names: one item, whether the shape fits (`fits`, undecided
//!   where the selections leave it open), the spaces and entrances a proof
//!   relates and whether the shape fits where no path from an entrance
//!   reaches it. A search refused for another reason refuses the list.
//! - `free_placements`, for expressions: a found placement is a sure
//!   member, a proven absence none, and a search the selections leave open
//!   one undecided member, so `count >= 1` is the three-valued fit.
//!
//! Each run reads the obstacles, the door swings and the entrances once
//! per declaration, then searches each space.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, PlacementOrientation, PlacementShape, PropertyResolutionError, RuleContext,
};
use axioval_engine::{BoxClearance, CylinderClearance};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use super::measure::Measured;
use super::{Declared, Obstacles, Placement, Search, band, entrance};
use crate::door_swing::Swings;
use crate::measured_kinds::{refused, resolution_error, selection};
use crate::space_access::{AccessDeclaration, Pick};
use crate::support::{Traversal, Unavailable, invalid};

/// Measures `free_floor_fit`.
pub(crate) struct FitMeasures;

/// Measures `free_placements`.
pub(crate) struct PlacementMeasures;

/// The keys `free_placements` and `free_floor_fit` name each part of the
/// declaration by.
struct Keys {
    diameter: &'static str,
    width: &'static str,
    length: &'static str,
    height: &'static str,
    band_from: &'static str,
    band_to: &'static str,
    merge: &'static str,
    obstacles: &'static str,
    swings: &'static str,
    entrance_width: &'static str,
    entrance_tolerance: Option<&'static str>,
    access: &'static str,
    doors: &'static str,
    openings: &'static str,
    spaces: Option<&'static str>,
}

const FIT: Keys = Keys {
    diameter: "diameter_metres",
    width: "width_metres",
    length: "length_metres",
    height: "height_metres",
    band_from: "band_from_metres",
    band_to: "band_to_metres",
    merge: "merge_path",
    obstacles: "obstacles",
    swings: "subtract_door_swings",
    entrance_width: "entrance_path_width",
    entrance_tolerance: Some("entrance_tolerance_metres"),
    access: "access_path",
    doors: "door_selector",
    openings: "opening_selector",
    spaces: Some("space_selector"),
};

const PLACEMENTS: Keys = Keys {
    diameter: "diameter",
    width: "width",
    length: "length",
    height: "height",
    band_from: "band_from",
    band_to: "band_to",
    merge: "merge",
    obstacles: "obstacles",
    swings: "swings",
    entrance_width: "entrance_width",
    entrance_tolerance: None,
    access: "access",
    doors: "doors",
    openings: "openings",
    spaces: None,
};

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

fn picked(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<Option<MeasuredSelection>, Unavailable> {
    selection(context, call, key, None).map_err(crate::selection::property_error)
}

/// The search `call` declares, read under `keys`.
fn search(
    call: &MeasuredCall,
    keys: &Keys,
    context: &RuleContext<'_>,
) -> Result<Search, Unavailable> {
    let required =
        |key: &str| length(call, key).ok_or_else(|| invalid(format!("`{key}` is required")));
    let height = required(keys.height)?;
    let shape = if call.choice("shape") == Some("rectangle") {
        let shape = BoxClearance::try_new(required(keys.width)?, required(keys.length)?, height)
            .map_err(|_| invalid("free-floor rectangle dimensions must be positive and finite"))?;
        PlacementShape::Box {
            shape,
            orientation: PlacementOrientation::Any,
        }
    } else {
        PlacementShape::Cylinder(
            CylinderClearance::try_new(required(keys.diameter)? / 2.0, height)
                .map_err(|_| invalid("free-floor circle dimensions must be positive and finite"))?,
        )
    };
    let band = band(
        length(call, keys.band_from),
        length(call, keys.band_to),
        height,
    )?;
    let merge = match call.argument(keys.merge) {
        Some(MeasuredArgument::Path(steps)) => Some(Traversal::path(steps)?),
        _ => None,
    };
    let (doors, openings) = (
        picked(context, call, keys.doors)?,
        picked(context, call, keys.openings)?,
    );
    let spaces = match keys.spaces {
        Some(key) => picked(context, call, key)?,
        None => None,
    };
    let access = match call.argument(keys.access) {
        Some(MeasuredArgument::Path(steps)) => Some(AccessDeclaration::of(
            steps,
            doors.as_ref().map(Pick::Selected),
            openings.as_ref().map(Pick::Selected),
            spaces.as_ref().map(Pick::Selected),
        )?),
        _ => None,
    };
    let entrance = entrance(
        access.is_some(),
        length(call, keys.entrance_width),
        keys.entrance_tolerance.and_then(|key| length(call, key)),
    )?;
    let index = match (&access, entrance) {
        (Some(access), Some(_)) => Some(access.index(context)),
        _ => None,
    };
    let obstacles = Obstacles::of(context, picked(context, call, keys.obstacles)?.as_ref());
    let swings = match picked(context, call, keys.swings)? {
        Some(doors) => Swings::of_selection(context, &doors),
        None => Swings::default(),
    };
    Ok(Search {
        shape,
        declared: Declared {
            band,
            merge,
            entrance,
        },
        obstacles,
        swings,
        index,
    })
}

#[derive(Hash, PartialEq, Eq)]
struct SearchKey(ArgumentsKey);

/// The search `call` declares, read once per run.
fn memoized(
    call: &MeasuredCall,
    keys: &Keys,
    context: &RuleContext<'_>,
) -> Result<Arc<Search>, Unavailable> {
    MeasuredMemo::of(context.services, SearchKey(ArgumentsKey::of(call)), || {
        search(call, keys, context).map(Arc::new)
    })
}

/// What the search answers for `object`.
fn placed(
    call: &MeasuredCall,
    keys: &Keys,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Placement, Unavailable> {
    let space = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    Ok(memoized(call, keys, context)?.place(context, space))
}

fn truth(value: bool, locator: &str) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: locator.to_owned(),
    }
}

/// The one item of `free_floor_fit`.
fn fit(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<MeasuredMember, Unavailable> {
    let locator = format!("free_floor_fit:{object}");
    let item = |fits: MemberValue,
                related: Vec<ObjectId>,
                unreached: bool,
                evidence: Vec<Evidence>,
                measured: Option<&Measured>| {
        // An open search proves nothing exactly; an answer is as exact as
        // what it rests on.
        let exact = !matches!(fits, MemberValue::Undecided { .. })
            && evidence.iter().all(|evidence| evidence.exact);
        let mut fields = BTreeMap::from([
            ("fits", fits),
            ("related", MemberValue::Objects { objects: related }),
            ("unreached", truth(unreached, &locator)),
        ]);
        fields.extend(measured_fields(&locator, measured));
        MeasuredMember {
            certain: true,
            exact,
            fields,
            evidence,
        }
    };
    match placed(call, &FIT, object, context)? {
        Placement::Found => Ok(item(
            truth(true, &locator),
            Vec::new(),
            false,
            Vec::new(),
            None,
        )),
        Placement::Absent(absent) => {
            let mut evidence = absent.evidence;
            evidence.extend(absent.measured.evidence.iter().cloned());
            evidence.extend(summary(object, &absent.measured));
            Ok(item(
                truth(false, &locator),
                absent.related,
                absent.unreached,
                evidence,
                Some(&absent.measured),
            ))
        }
        Placement::Open((NotEvaluatedReason::IncompleteEvidence, why)) => Ok(item(
            MemberValue::Undecided { why },
            Vec::new(),
            false,
            Vec::new(),
            None,
        )),
        Placement::Open(refusal) => Err(refusal),
    }
}

/// A bracket measured on exact evidence, `null` where not measured.
fn cited(
    locator: &str,
    name: &str,
    value: Option<(f64, f64)>,
    dimension: QuantityDimension,
) -> MemberValue {
    let locator = format!("{locator}:{name}");
    MemberValue::Measured(match value {
        Some((lower, upper)) => Measurement::Cited {
            lower,
            upper,
            dimension: Some(dimension),
            locator,
            exact: true,
        },
        None => Measurement::Absent { locator },
    })
}

/// The fields of what a proven absence measured (#391), `null` and empty
/// for any other answer.
fn measured_fields(locator: &str, measured: Option<&Measured>) -> Vec<(&'static str, MemberValue)> {
    let largest = measured.and_then(|measured| measured.largest.as_ref());
    let size = |index: usize, of: usize| {
        largest
            .filter(|largest| largest.sizes.len() == of)
            .map(|largest| largest.sizes[index])
    };
    let centre = |axis: usize| {
        largest
            .and_then(|largest| largest.centre)
            .map(|centre| (centre[axis], centre[axis]))
    };
    let objects = |pick: fn(&Measured) -> &Vec<ObjectId>| MemberValue::Objects {
        objects: measured
            .map(|measured| pick(measured).clone())
            .unwrap_or_default(),
    };
    let length = QuantityDimension::Length;
    vec![
        ("obstacles", objects(|measured| &measured.obstacles)),
        ("possible_obstacles", objects(|measured| &measured.possible)),
        ("largest", cited(locator, "largest", size(0, 1), length)),
        (
            "largest_width",
            cited(locator, "largest_width", size(0, 2), length),
        ),
        (
            "largest_length",
            cited(locator, "largest_length", size(1, 2), length),
        ),
        ("centre_x", cited(locator, "centre_x", centre(0), length)),
        ("centre_y", cited(locator, "centre_y", centre(1), length)),
        ("centre_z", cited(locator, "centre_z", centre(2), length)),
        (
            "free_area",
            cited(
                locator,
                "free_area",
                measured.and_then(|measured| measured.area),
                QuantityDimension::Area,
            ),
        ),
    ]
}

/// The evidence naming each measured value with its certified bounds, in
/// metres and square metres, for hosts to read: `axioval:free-floor:
/// <space>:largest=[lower,upper]` (a rectangle's `[w0,w1]x[l0,l1]`) with
/// `:centre=[x,y,z]` in model coordinates where a shape was placed, and
/// `axioval:free-floor:<space>:area=[lower,upper]`. Exact, as the
/// searches they rest on are.
fn summary(object: &ObjectId, measured: &Measured) -> Vec<Evidence> {
    let bounds = |(lower, upper): (f64, f64)| format!("[{lower},{upper}]");
    let mut evidence = Vec::new();
    if let Some(largest) = &measured.largest {
        let sizes: Vec<String> = largest.sizes.iter().copied().map(bounds).collect();
        let mut locator = format!("axioval:free-floor:{object}:largest={}", sizes.join("x"));
        if let Some([x, y, z]) = largest.centre {
            let _ = write!(locator, ":centre=[{x},{y},{z}]");
        }
        evidence.push(Evidence::exact(object.source.clone(), locator));
    }
    if let Some(area) = measured.area {
        evidence.push(Evidence::exact(
            object.source.clone(),
            format!("axioval:free-floor:{object}:area={}", bounds(area)),
        ));
    }
    evidence
}

/// The placements `free_placements` lists.
fn placements(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    // A sure placement rests on an exact witness and exact support, as the
    // free-space contract requires of a found placement; an open search
    // proves nothing exactly.
    let member = |certain| MeasuredMember {
        certain,
        exact: certain,
        fields: BTreeMap::new(),
        evidence: Vec::new(),
    };
    match placed(call, &PLACEMENTS, object, context)? {
        Placement::Found => Ok(vec![member(true)]),
        Placement::Absent(_) => Ok(Vec::new()),
        // The selections leave the fit open: perhaps a placement.
        Placement::Open((NotEvaluatedReason::IncompleteEvidence, _)) => Ok(vec![member(false)]),
        Placement::Open(refusal) => Err(refusal),
    }
}

fn unmeasured() -> Result<Measurement, PropertyResolutionError> {
    Err(PropertyResolutionError::InvalidRequest)
}

impl MeasuredProvider for FitMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["free_floor_fit"]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        unmeasured()
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        fit(call, object, context)
            .map(|item| vec![item])
            .map_err(refused(call.name(), object))
    }
}

impl MeasuredProvider for PlacementMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["free_placements"]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        unmeasured()
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        placements(call, object, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
