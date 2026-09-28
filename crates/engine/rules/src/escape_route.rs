//! `escape-route`: travel distance, number of exits and exit widths per
//! space use.
//!
//! Each selected space takes the first row of `uses` whose `spaces`
//! selector picks it. The row states, each optionally:
//!
//! - `maximum_travel`: the longest walk, in metres, from the start to the
//!   nearest exit. With `route_start` `farthest-point` (the default) the
//!   start is every point of the space's walkable area, so the farthest one
//!   counts; with `door` it is each of the space's own doors (`door_path`
//!   among `door_selector` objects). Walking follows the walking line of a
//!   point (`walking_height`, `walking_step`) through the metric-routing
//!   service; exits are targets at their representative points (the centre
//!   of the footprint, inside it, on the exit's floor).
//! - `exits`: how many exits the space needs.
//! - `area_per_occupant`: the plan area, in square metres, per occupant.
//!   The occupant load is the space's measured footprint divided by it,
//!   rounded up, and every exit must be at least as wide as the first row of
//!   `widths` whose `occupants` covers that load (`width`), and all exits
//!   together as wide as its `total_width` where stated. An exit's width is
//!   its `clear_width_property`, a length; where none is stated, the
//!   geometry decides only a failure: a footprint whose longest plan
//!   diagonal is narrower than required bounds the clear width from above.
//!
//! Exits are the `exit_selector` objects `exit_path` reaches from the space.
//! Every measure is an interval: the travel distance is bounded from above
//! through the exits that surely are exits and from below through every one
//! that might be, and a verdict stands only when what is unknown cannot
//! change it. An occupant load straddling two rows of `widths` requires
//! either row's width, so only what both decide stands.
//!
//! **Multiplied sections.** Each row of `sections` names objects (`objects`,
//! a stair, say) on which a walked metre counts `factor` times, at least
//! one; with `shared_by`, only a section that many checked spaces reach
//! along `section_path` multiplies; where sections overlap, the largest
//! factor counts. The multiplied travel, the least multiplied length of any
//! walk, is bracketed: at least the plain walk's lower bound, at most the
//! multiplied length of any one walk. From a door the routing answer names
//! a walk no longer than the plain upper bound `U`, traced over the
//! sections: `U` plus each section's length on it times its factor less
//! one. Otherwise, or where smaller, `U` times the largest factor of a
//! section the walk may cross: a walk of at most `U` metres stays within
//! `U` of its start in plan, so a section whose horizontal distance from
//! the start (the space, or the door) surely exceeds `U` is left out; every
//! other one, and one that might be shared, may be crossed.
//!
//! **Common path.** With `common_path_factor`, the stretch a space's
//! routes share before they part counts that many times, on top of the
//! sections: from a door, the named walk's length over the passages the
//! walks to every other sure target may cross too; the whole walk where
//! there is no other route, or no walk is named. Only the upper bound
//! grows.
//!
//! **Passages.** With `passage_selector`, the passages of a checked space
//! are the `passage_selector` objects `passage_path` reaches from it, and
//! the space itself where `passage_selector` picks it. A passage carries the
//! occupants of every checked space it serves, and must be as wide as the
//! `passage_width` of the rows of `widths` for that load. Its width is its
//! `passage_width_property`, a length; without one, only a failure decides:
//! no clear width exceeds the shorter side of the rectangle of least area
//! enclosing its footprint. A space whose load is unknown (no
//! `area_per_occupant`, an undecided use or selection, an unmeasured
//! footprint) leaves every passage it may serve not evaluated.
//!
//! **Walked passages.** With `walked_passages`, a space's passages come
//! from the walks out of its doors instead: the walk a routing answer names
//! is one shortest walk among perhaps several, so it only picks candidates.
//! A passage is surely relied on when, from every door, the walk to every
//! possible exit keeping out of it is longer than the plain walk to the
//! sure exits, or reaches none; perhaps relied on unless the plan distances
//! from the door to it and on to the nearest exit already exceed the plain
//! walk. A passage carries its sure loads at least and its possible ones at
//! most, and needs one space surely relying on it to be found too narrow.
//!
//! **Doors on routes.** With `route_door_selector`, the doors a space's
//! walks rely on are derived as walked passages are and carry the summed
//! loads, each needing the `door_width` of its load's rows; its width is
//! the `clear_width_property`, else at most the footprint's diagonal.
//! `total_door_width` asks a space's own doors together to be that wide.
//!
//! **Exit door direction.** With `exit_door_direction`, every exit door
//! must open in the direction of escape: out of the space. Its leaves come
//! from the object-frame service and which side the space lies on from the
//! free-space service's containment probes (`door_swing::relation`). An
//! exit door swinging into the space is a finding; one opening away from
//! it, or double-acting, passes. An exit that is no door has no leaf and is
//! skipped; a door without a hinged leaf, unknown leaves, and a space
//! neither probe lies in are not evaluated.
//!
//! **Not usable for escape.** With `no_escape_selector`, what it picks
//! (locked or staff-only doors) is no exit and no start, and every walk
//! keeps out of it. What it may pick is only a possible exit or start, and
//! avoided by the walks bounding the travel from above only. The farthest
//! point is measured on the plain walk, which bounds the walk around
//! anything from below, and from above only where every avoided object lies
//! surely farther from the space than that bound.
//!
//! **Compartments.** With `compartment_selector`, a space lies in the
//! compartments `compartment_path` reaches from it, or in each covering at
//! least `compartment_overlap` of its footprint. Travel ends at the nearest
//! exit or door out of the start's one compartment: the doors out are found
//! by walking the compartment's spaces through their doors (`door_path`,
//! and back from each door to its spaces); a door reaching a space outside,
//! or no other space, leads out. A project without any compartment is an
//! inadequate-information finding per source of a checked space.
//!
//! **Independent routes.** With `exit_count: routes`, `exits` counts
//! routes to distinct targets, walked from the space's representative
//! point; routes sharing a passage count once. At most every reachable
//! target, one where a passage cuts every walk; at least the sure targets
//! whose traced walks share no passage.
//!
//! **Along the route.** With `route_door_direction` and
//! `minimum_clear_height`, the walk named from each door is traced over
//! the doors and passages: every single-swing door it crosses must open
//! along it (read from where the walk crosses the closed leaf's line), and
//! every door, opening and space on it, the start door and space included,
//! must be as high as the minimum. A failure is a finding only when every
//! shortest walk crosses the object.
//!
//! **Zones.** Each object takes the rank of the first row of `zones`
//! picking it, the start its space's or compartment's; every walk keeps out
//! of what ranks above the start, as it keeps out of what is not usable for
//! escape.
//!
//! **Across levels.** With `stair_selector`, `ramp_selector` or
//! `lift_selector`, every walk may climb the selected connectors (read by
//! the `climbing` module), a climb counting by `stair_length` and
//! `vertical_factor`. A walk that climbs is never traced over sections or
//! passages, since a trace measures in plan: sections bound it by the
//! largest factor and its common path is the whole walk.
//!
//! Not checked: passages walked from the farthest point rather than the
//! doors.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, ConnectorRouting, DoorLeaves, DoorLeavesError,
    FarthestPointOutcome, FarthestPointRequest, FreeSpaceServiceHandle, LengthInterval,
    MetricPoint, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, NotEvaluatedReason, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, PathTraceRequest, PlanArea, PlanAreaServiceHandle, PlanSpanServiceHandle,
    ProximityProjection, ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext,
    SpaceServiceHandle, TableColumn, VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, QuantityDimension, Scope};

use crate::climbing::{self, Climbing};
use crate::door_swing::{self, Relation};
use crate::exit_separation::Candidates;
use crate::keyed_limit::door_clear_height;
use crate::plan_area::{footprint, shown};
use crate::selection::{Selection, select_objects, selector_matches};
use crate::space_distance::representative_point;
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, finding, invalid, resolve,
};

const USES: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("spaces", ColumnKind::Selector),
    TableColumn::optional("maximum_travel", ColumnKind::Number),
    TableColumn::optional("route_start", ColumnKind::String),
    TableColumn::optional("exits", ColumnKind::Integer),
    TableColumn::optional("area_per_occupant", ColumnKind::Number),
];

const WIDTHS: &[TableColumn] = &[
    TableColumn::required("occupants", ColumnKind::Integer),
    TableColumn::required("width", ColumnKind::Number),
    TableColumn::optional("total_width", ColumnKind::Number),
    TableColumn::optional("passage_width", ColumnKind::Number),
    TableColumn::optional("door_width", ColumnKind::Number),
    TableColumn::optional("total_door_width", ColumnKind::Number),
];

const SECTIONS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("objects", ColumnKind::Selector),
    TableColumn::required("factor", ColumnKind::Number),
    TableColumn::optional("shared_by", ColumnKind::Integer),
];

const ZONES: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("objects", ColumnKind::Selector),
    TableColumn::required("rank", ColumnKind::Integer),
];

/// How narrow the farthest-point bracket is asked to become, in metres.
const TOLERANCE: f64 = 0.01;

/// Requires each selected space's escape routes to fit its use.
pub struct EscapeRoute;

/// Where travel is measured from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    FarthestPoint,
    Door,
}

struct Use<'a> {
    name: String,
    spaces: &'a Selector,
    maximum_travel: Option<f64>,
    start: Start,
    exits: Option<usize>,
    area_per_occupant: Option<f64>,
}

/// One row of `widths`.
struct WidthRow {
    /// The row covers loads up to this many occupants.
    occupants: u64,
    /// The least width of each exit.
    width: f64,
    /// The least width of all exits together.
    total: Option<f64>,
    /// The least clear width of each passage.
    passage: Option<f64>,
    /// The least clear width of each door on a route.
    door: Option<f64>,
    /// The least width of a space's own doors together.
    total_door: Option<f64>,
}

/// One row of `sections`: objects on which a walked metre counts `factor`
/// times.
struct SectionKind<'a> {
    name: String,
    objects: &'a Selector,
    factor: f64,
    /// Only a section at least this many checked spaces reach multiplies.
    shared_by: Option<usize>,
}

/// The passages a space's occupants rely on.
struct Passages<'a> {
    path: Option<Traversal>,
    selector: &'a Selector,
    width: Option<PropertyRef<'a>>,
    /// Whether the passages are those the walks from each space's doors
    /// cross, rather than declared along `path`.
    walked: bool,
    /// Whether their widths are checked: every row of `widths` states a
    /// `passage_width`. Otherwise they only merge routes.
    judged: bool,
}

/// How a space is assigned to its compartments.
enum Membership {
    /// The compartments the path reaches from the space.
    Path(Traversal),
    /// The compartments covering at least this share of its footprint.
    Overlap(f64),
}

/// Fire compartments: travel ends at the start compartment's boundary.
struct Compartments<'a> {
    selector: &'a Selector,
    membership: Membership,
}

/// One row of `zones`: objects of a rank.
struct Zone<'a> {
    objects: &'a Selector,
    rank: i64,
}

struct Declaration<'a> {
    uses: Vec<Use<'a>>,
    /// By occupants.
    widths: Vec<WidthRow>,
    sections: Vec<SectionKind<'a>>,
    section_path: Option<Traversal>,
    passages: Option<Passages<'a>>,
    exits: Traversal,
    exit_selector: &'a Selector,
    doors: Option<(Traversal, &'a Selector)>,
    clear_width: Option<PropertyRef<'a>>,
    profile: Option<MobilityProfile>,
    /// Whether exit doors must open in the direction of escape.
    door_direction: bool,
    /// Objects not usable for escape: never exits, starts or passed.
    no_escape: Option<&'a Selector>,
    /// Whether `exits` counts independent routes rather than exits.
    count_routes: bool,
    /// The doors on routes whose loads are checked.
    route_doors: Option<&'a Selector>,
    /// How many times a metre of the common path counts, at least one.
    common_path: Option<f64>,
    /// Whether every single-swing door a walk crosses must open along it.
    route_door_direction: bool,
    /// The least clear height of what a walk crosses, in metres.
    minimum_height: Option<f64>,
    /// A door's clear height, as `keyed-limit`'s `clear-height` reads it:
    /// stated, else overall less lining and threshold.
    clear_height: [Option<PropertyRef<'a>>; 4],
    compartments: Option<Compartments<'a>>,
    /// By row; the first row picking an object ranks it.
    zones: Vec<Zone<'a>>,
    /// The connectors walks climb between levels.
    climbing: Option<Climbing<'a>>,
}

fn positive(name: &str, column: &str, value: Option<f64>) -> Result<Option<f64>, Unavailable> {
    match value {
        Some(value) if value <= 0.0 => Err(invalid(format!("{name}: `{column}` must be positive"))),
        other => Ok(other),
    }
}

#[allow(clippy::too_many_lines)]
fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let table = parameters
        .table("uses")?
        .ok_or_else(|| invalid("parameter `uses` is required"))?;
    let mut uses = Vec::new();
    for (index, row) in table.into_iter().enumerate() {
        let name = match row.text("label")? {
            Some(label) => format!("use {index} ({label})"),
            None => format!("use {index}"),
        };
        let maximum_travel = match row.number("maximum_travel")? {
            Some(value) if value < 0.0 => {
                return Err(invalid(format!(
                    "{name}: `maximum_travel` must not be negative"
                )));
            }
            other => other,
        };
        let route_start = row.text("route_start")?;
        if route_start.is_some() && maximum_travel.is_none() {
            return Err(invalid(format!(
                "{name}: `route_start` needs `maximum_travel`"
            )));
        }
        let start = match route_start {
            None | Some("farthest-point") => Start::FarthestPoint,
            Some("door") => Start::Door,
            Some(other) => {
                return Err(invalid(format!(
                    "{name}: route start `{other}` is unsupported (farthest-point, door)"
                )));
            }
        };
        let exits = row
            .integer("exits")?
            .map(|exits| {
                usize::try_from(exits)
                    .ok()
                    .filter(|exits| *exits > 0)
                    .ok_or_else(|| invalid(format!("{name}: `exits` must be at least one")))
            })
            .transpose()?;
        let area_per_occupant =
            positive(&name, "area_per_occupant", row.number("area_per_occupant")?)?;
        if maximum_travel.is_none() && exits.is_none() && area_per_occupant.is_none() {
            return Err(invalid(format!("{name} states no requirement")));
        }
        uses.push(Use {
            spaces: row
                .selector("spaces")?
                .ok_or_else(|| invalid(format!("{name} has no `spaces`")))?,
            maximum_travel,
            start,
            exits,
            area_per_occupant,
            name,
        });
    }
    let mut widths = Vec::new();
    for (index, row) in parameters
        .table("widths")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = format!("width {index}");
        let occupants = row
            .integer("occupants")?
            .and_then(|occupants| u64::try_from(occupants).ok())
            .filter(|occupants| *occupants > 0)
            .ok_or_else(|| invalid(format!("{name}: `occupants` must be at least one")))?;
        let width = positive(&name, "width", row.number("width")?)?
            .ok_or_else(|| invalid(format!("{name} has no `width`")))?;
        let total = positive(&name, "total_width", row.number("total_width")?)?;
        let passage = positive(&name, "passage_width", row.number("passage_width")?)?;
        let door = positive(&name, "door_width", row.number("door_width")?)?;
        let total_door = positive(&name, "total_door_width", row.number("total_door_width")?)?;
        widths.push(WidthRow {
            occupants,
            width,
            total,
            passage,
            door,
            total_door,
        });
    }
    widths.sort_by_key(|row| row.occupants);
    if widths
        .windows(2)
        .any(|pair| pair[0].occupants == pair[1].occupants)
    {
        return Err(invalid("two rows of `widths` state the same `occupants`"));
    }
    let loads = uses.iter().any(|use_| use_.area_per_occupant.is_some());
    if loads && widths.is_empty() {
        return Err(invalid("`area_per_occupant` needs `widths`"));
    }
    if !loads && !widths.is_empty() {
        return Err(invalid("`widths` needs a use stating `area_per_occupant`"));
    }
    let doors = match (
        parameters.strings("door_path")?,
        parameters.selector("door_selector")?,
    ) {
        (Some(path), Some(selector)) => Some((Traversal::path(path)?, selector)),
        (None, None) => None,
        _ => return Err(invalid("`door_path` and `door_selector` go together")),
    };
    if doors.is_none() && uses.iter().any(|use_| use_.start == Start::Door) {
        return Err(invalid(
            "route start `door` needs `door_path` and `door_selector`",
        ));
    }
    let profile = match (
        parameters.number("walking_height")?,
        parameters.number("walking_step")?,
    ) {
        (Some(height), Some(step)) => Some(
            MobilityProfile::try_new(0.0, height, step, 0.0)
                .map_err(|error| invalid(error.to_string()))?,
        ),
        (None, None) => None,
        _ => return Err(invalid("`walking_height` and `walking_step` go together")),
    };
    if profile.is_none() && uses.iter().any(|use_| use_.maximum_travel.is_some()) {
        return Err(invalid(
            "`maximum_travel` needs `walking_height` and `walking_step`",
        ));
    }
    let (sections, section_path) = sections(&parameters, &uses)?;
    let count_routes = match parameters.string("exit_count")? {
        None | Some("exits") => false,
        Some("routes") => true,
        Some(other) => {
            return Err(invalid(format!(
                "exit count `{other}` is unsupported (exits, routes)"
            )));
        }
    };
    let common_path = match parameters.number("common_path_factor")? {
        Some(factor) if !factor.is_finite() || factor < 1.0 => {
            return Err(invalid("`common_path_factor` must be at least 1"));
        }
        other => other,
    };
    if common_path.is_some() && uses.iter().all(|use_| use_.maximum_travel.is_none()) {
        return Err(invalid(
            "`common_path_factor` needs a use stating `maximum_travel`",
        ));
    }
    let passages = passages(&parameters, &widths, count_routes || common_path.is_some())?;
    if count_routes
        && (passages.is_none() || profile.is_none() || uses.iter().all(|use_| use_.exits.is_none()))
    {
        return Err(invalid(
            "`exit_count: routes` needs `passage_selector`, `walking_height`, `walking_step` \
             and a use stating `exits`",
        ));
    }
    if passages.as_ref().is_some_and(|passages| passages.walked)
        && (doors.is_none() || profile.is_none())
    {
        return Err(invalid(
            "`walked_passages` needs `door_path`, `door_selector`, `walking_height` and \
             `walking_step`",
        ));
    }
    let compartments = compartments(&parameters)?;
    if compartments.is_some() && doors.is_none() {
        return Err(invalid(
            "`compartment_selector` needs `door_path` and `door_selector`, which lead out of \
             the compartment",
        ));
    }
    let zones = zones(&parameters)?;
    let route_doors = parameters.selector("route_door_selector")?;
    if route_doors.is_some()
        && (widths.is_empty()
            || widths.iter().any(|row| row.door.is_none())
            || doors.is_none()
            || profile.is_none())
    {
        return Err(invalid(
            "`route_door_selector` needs every row of `widths` to state `door_width`, and \
             `door_path`, `door_selector`, `walking_height` and `walking_step`",
        ));
    }
    if route_doors.is_none() && widths.iter().any(|row| row.door.is_some()) {
        return Err(invalid("`door_width` needs `route_door_selector`"));
    }
    if doors.is_none() && widths.iter().any(|row| row.total_door.is_some()) {
        return Err(invalid(
            "`total_door_width` needs `door_path` and `door_selector`",
        ));
    }
    let route_door_direction = parameters.boolean("route_door_direction")?.unwrap_or(false);
    let minimum_height = positive(
        "`minimum_clear_height`",
        "minimum_clear_height",
        parameters.number("minimum_clear_height")?,
    )?;
    let clear_height = [
        parameters.property("clear_height_property")?,
        parameters.property("overall_height")?,
        parameters.property("lining_thickness")?,
        parameters.property("threshold_thickness")?,
    ];
    if clear_height.iter().any(Option::is_some) && minimum_height.is_none() {
        return Err(invalid(
            "`clear_height_property`, `overall_height`, `lining_thickness` and \
             `threshold_thickness` need `minimum_clear_height`",
        ));
    }
    if clear_height[1].is_none() && (clear_height[2].is_some() || clear_height[3].is_some()) {
        return Err(invalid(
            "`lining_thickness` and `threshold_thickness` are deducted from `overall_height`, \
             which is not declared",
        ));
    }
    if (route_door_direction || minimum_height.is_some()) && (doors.is_none() || profile.is_none())
    {
        return Err(invalid(
            "`route_door_direction` and `minimum_clear_height` need `door_path`, \
             `door_selector`, `walking_height` and `walking_step`",
        ));
    }
    if (compartments.is_some() || !zones.is_empty())
        && uses.iter().all(|use_| use_.maximum_travel.is_none())
    {
        return Err(invalid(
            "`compartment_selector` and `zones` need a use stating `maximum_travel`",
        ));
    }
    Ok(Declaration {
        uses,
        widths,
        sections,
        section_path,
        passages,
        exits: Traversal::path(
            parameters
                .strings("exit_path")?
                .ok_or_else(|| invalid("parameter `exit_path` is required"))?,
        )?,
        exit_selector: parameters.required_selector("exit_selector")?,
        doors,
        clear_width: parameters.property("clear_width_property")?,
        profile,
        door_direction: parameters.boolean("exit_door_direction")?.unwrap_or(false),
        no_escape: parameters.selector("no_escape_selector")?,
        count_routes,
        route_doors,
        common_path,
        route_door_direction,
        minimum_height,
        clear_height,
        compartments,
        zones,
        climbing: Climbing::parse(&parameters)?,
    })
}

fn compartments<'a>(parameters: &Parameters<'a>) -> Result<Option<Compartments<'a>>, Unavailable> {
    let path = parameters.strings("compartment_path")?;
    let overlap = parameters.number("compartment_overlap")?;
    let Some(selector) = parameters.selector("compartment_selector")? else {
        if path.is_some() || overlap.is_some() {
            return Err(invalid(
                "`compartment_path` and `compartment_overlap` need `compartment_selector`",
            ));
        }
        return Ok(None);
    };
    let membership = match (path, overlap) {
        (Some(path), None) => Membership::Path(Traversal::path(path)?),
        (None, Some(share)) if share > 0.0 && share <= 1.0 => Membership::Overlap(share),
        (None, Some(_)) => {
            return Err(invalid(
                "`compartment_overlap` must be a share above 0 and at most 1",
            ));
        }
        _ => {
            return Err(invalid(
                "`compartment_selector` needs either `compartment_path` or \
                 `compartment_overlap`",
            ));
        }
    };
    Ok(Some(Compartments {
        selector,
        membership,
    }))
}

fn zones<'a>(parameters: &Parameters<'a>) -> Result<Vec<Zone<'a>>, Unavailable> {
    let mut zones = Vec::new();
    for (index, row) in parameters
        .table("zones")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        zones.push(Zone {
            objects: row
                .selector("objects")?
                .ok_or_else(|| invalid(format!("zone {index} has no `objects`")))?,
            rank: row
                .integer("rank")?
                .ok_or_else(|| invalid(format!("zone {index} has no `rank`")))?,
        });
    }
    Ok(zones)
}

type Sections<'a> = (Vec<SectionKind<'a>>, Option<Traversal>);

fn sections<'a>(
    parameters: &Parameters<'a>,
    uses: &[Use<'_>],
) -> Result<Sections<'a>, Unavailable> {
    let mut sections = Vec::new();
    for (index, row) in parameters
        .table("sections")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = match row.text("label")? {
            Some(label) => format!("section {index} ({label})"),
            None => format!("section {index}"),
        };
        let objects = row
            .selector("objects")?
            .ok_or_else(|| invalid(format!("{name} has no `objects`")))?;
        let factor = row
            .number("factor")?
            .filter(|factor| factor.is_finite() && *factor >= 1.0)
            .ok_or_else(|| invalid(format!("{name}: `factor` must be at least 1")))?;
        let shared_by = row
            .integer("shared_by")?
            .map(|count| {
                usize::try_from(count)
                    .ok()
                    .filter(|count| *count >= 2)
                    .ok_or_else(|| invalid(format!("{name}: `shared_by` must be at least 2")))
            })
            .transpose()?;
        sections.push(SectionKind {
            name,
            objects,
            factor,
            shared_by,
        });
    }
    let path = parameters
        .strings("section_path")?
        .map(Traversal::path)
        .transpose()?;
    let shared = sections.iter().any(|kind| kind.shared_by.is_some());
    if shared && path.is_none() {
        return Err(invalid("`shared_by` needs `section_path`"));
    }
    if !shared && path.is_some() {
        return Err(invalid(
            "`section_path` needs a section stating `shared_by`",
        ));
    }
    if !sections.is_empty() && uses.iter().all(|use_| use_.maximum_travel.is_none()) {
        return Err(invalid("`sections` needs a use stating `maximum_travel`"));
    }
    Ok((sections, path))
}

fn passages<'a>(
    parameters: &Parameters<'a>,
    widths: &[WidthRow],
    merges: bool,
) -> Result<Option<Passages<'a>>, Unavailable> {
    let path = parameters.strings("passage_path")?;
    let width = parameters.property("passage_width_property")?;
    let walked = parameters.boolean("walked_passages")?.unwrap_or(false);
    let Some(selector) = parameters.selector("passage_selector")? else {
        if path.is_some()
            || width.is_some()
            || walked
            || widths.iter().any(|row| row.passage.is_some())
        {
            return Err(invalid(
                "`passage_path`, `walked_passages`, `passage_width_property` and \
                 `passage_width` need `passage_selector`",
            ));
        }
        return Ok(None);
    };
    if walked && path.is_some() {
        return Err(invalid(
            "`walked_passages` and `passage_path` exclude each other",
        ));
    }
    let judged = !widths.is_empty() && widths.iter().all(|row| row.passage.is_some());
    let stated = widths.iter().any(|row| row.passage.is_some());
    if !judged && (!merges || stated) {
        return Err(invalid(
            "`passage_selector` needs every row of `widths` to state `passage_width`, unless \
             it only traces routes (`exit_count: routes` or `common_path_factor`, no \
             `passage_width`)",
        ));
    }
    if !judged && (path.is_some() || width.is_some() || walked) {
        return Err(invalid(
            "`passage_path`, `walked_passages` and `passage_width_property` need `widths` \
             stating `passage_width`",
        ));
    }
    Ok(Some(Passages {
        path: path.map(Traversal::path).transpose()?,
        selector,
        width,
        walked,
        judged,
    }))
}

impl RuleCapability for EscapeRoute {
    fn id(&self) -> &'static str {
        "axioval:capability.escape-route"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("uses", ParameterType::Table(USES)),
            ParameterDescriptor::optional("widths", ParameterType::Table(WIDTHS)),
            ParameterDescriptor::required("exit_path", ParameterType::StringList),
            ParameterDescriptor::required("exit_selector", ParameterType::Selector),
            ParameterDescriptor::optional("door_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("clear_width_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("walking_height", ParameterType::Number),
            ParameterDescriptor::optional("walking_step", ParameterType::Number),
            ParameterDescriptor::optional("sections", ParameterType::Table(SECTIONS)),
            ParameterDescriptor::optional("section_path", ParameterType::StringList),
            ParameterDescriptor::optional("passage_path", ParameterType::StringList),
            ParameterDescriptor::optional("passage_selector", ParameterType::Selector),
            ParameterDescriptor::optional(
                "passage_width_property",
                ParameterType::PropertyReference,
            ),
            ParameterDescriptor::optional("exit_door_direction", ParameterType::Boolean),
            ParameterDescriptor::optional("walked_passages", ParameterType::Boolean),
            ParameterDescriptor::optional("no_escape_selector", ParameterType::Selector),
            ParameterDescriptor::optional("compartment_selector", ParameterType::Selector),
            ParameterDescriptor::optional("compartment_path", ParameterType::StringList),
            ParameterDescriptor::optional("compartment_overlap", ParameterType::Number),
            ParameterDescriptor::optional("zones", ParameterType::Table(ZONES)),
            ParameterDescriptor::optional("exit_count", ParameterType::String),
            ParameterDescriptor::optional("route_door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("common_path_factor", ParameterType::Number),
            ParameterDescriptor::optional("route_door_direction", ParameterType::Boolean),
            ParameterDescriptor::optional("minimum_clear_height", ParameterType::Number),
            ParameterDescriptor::optional(
                "clear_height_property",
                ParameterType::PropertyReference,
            ),
            ParameterDescriptor::optional("overall_height", ParameterType::PropertyReference),
            ParameterDescriptor::optional("lining_thickness", ParameterType::PropertyReference),
            ParameterDescriptor::optional("threshold_thickness", ParameterType::PropertyReference),
        ];
        parameters.extend(climbing::descriptors());
        parameters
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("escape-route: {message}"),
                );
            }
        };
        let exits = Candidates::select(context, declared.exit_selector);
        let doors = declared
            .doors
            .as_ref()
            .map(|(_, selector)| Candidates::select(context, selector));
        let passages = declared
            .passages
            .as_ref()
            .map(|passages| Candidates::select(context, passages.selector));
        let judge = Judge {
            context,
            rule,
            declared: &declared,
            exits,
            doors,
            sections: possible_sections(context, rule, &declared),
            passages,
            no_escape: declared
                .no_escape
                .map(|selector| Candidates::select(context, selector)),
            route_doors: declared
                .route_doors
                .map(|selector| Candidates::select(context, selector)),
            compartments: declared
                .compartments
                .as_ref()
                .map(|compartments| Candidates::select(context, compartments.selector)),
            zones: declared
                .zones
                .iter()
                .map(|zone| Candidates::select(context, zone.objects))
                .collect(),
            membership: RefCell::new(BTreeMap::new()),
            walks: RefCell::new(BTreeMap::new()),
            connectors: declared
                .climbing
                .as_ref()
                .map(|climbing| climbing.routing(context)),
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        judge.no_compartment(&spaces, &mut evaluation);
        let mut served = Served::default();
        let mut served_doors = Served::default();
        let mut results: Vec<(ObjectId, String, Checked)> = Vec::new();
        for space in spaces {
            let matched =
                match_rows(
                    &declared.uses,
                    RowSelection::First,
                    |use_| match selector_matches(context, use_.spaces, space, &mut Vec::new()) {
                        Selection::Match => RowTest::Match(0),
                        Selection::NoMatch => RowTest::NoMatch,
                        Selection::NotEvaluated(..) => RowTest::Undecided,
                    },
                );
            let use_ = match matched {
                Matched::Rows(rows) if !rows.is_empty() => rows[0].1,
                Matched::Rows(_) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "escape-route: no row of `uses` picks this space",
                    );
                    judge.serve(&space.id, Err("no row of `uses` picks it"), &mut served);
                    judge.serve_doors(
                        &space.id,
                        Err("no row of `uses` picks it"),
                        &mut served_doors,
                    );
                    continue;
                }
                Matched::Undecided | Matched::Ambiguous(_) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "escape-route: whether a row of `uses` picks this space is undecided",
                    );
                    judge.serve(
                        &space.id,
                        Err("whether a row of `uses` picks it is undecided"),
                        &mut served,
                    );
                    judge.serve_doors(
                        &space.id,
                        Err("whether a row of `uses` picks it is undecided"),
                        &mut served_doors,
                    );
                    continue;
                }
            };
            let mut checked = Checked::default();
            let load = judge.space(space, use_, &mut checked);
            let load = match &load {
                Some(Ok(load)) => Ok(load),
                Some(Err(_)) => Err("its footprint is not measured"),
                None => Err("its use states no `area_per_occupant`"),
            };
            if let Some(doubt) = judge.serve(&space.id, load, &mut served) {
                checked.doubts.push(doubt);
            }
            judge.serve_doors(&space.id, load, &mut served_doors);
            results.push((
                space.id.clone(),
                format!("escape-route {}", use_.name),
                checked,
            ));
        }
        if declared
            .passages
            .as_ref()
            .is_some_and(|passages| passages.judged)
        {
            // A space the rule may select brings occupants nobody counted.
            for space in Candidates::select(context, &rule.selector).undecided.keys() {
                judge.serve(
                    space,
                    Err("whether the rule selects it is undecided"),
                    &mut served,
                );
            }
            judge.judge_passages(served, Class::Passage, &mut results);
        }
        if judge.route_doors.is_some() {
            for space in Candidates::select(context, &rule.selector).undecided.keys() {
                judge.serve_doors(
                    space,
                    Err("whether the rule selects it is undecided"),
                    &mut served_doors,
                );
            }
            judge.judge_passages(served_doors, Class::Door, &mut results);
        }
        for (subject, prefix, checked) in results {
            emit(&mut evaluation, subject, &prefix, checked);
        }
        evaluation
    }
}

/// Records what checking one object found.
fn emit(evaluation: &mut CapabilityEvaluation, subject: ObjectId, prefix: &str, checked: Checked) {
    for found in checked.findings {
        evaluation.push_finding(found);
    }
    if checked.doubts.is_empty() {
        return;
    }
    let reason = if checked
        .doubts
        .iter()
        .any(|(why, _)| *why == NotEvaluatedReason::MissingService)
    {
        NotEvaluatedReason::MissingService
    } else {
        checked.doubts[0].0.clone()
    };
    let mut messages: Vec<String> = checked
        .doubts
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    messages.dedup();
    evaluation.push_object_not_evaluated(
        subject,
        reason,
        format!("{prefix}: {}", messages.join("; ")),
    );
}

/// A section object the walk may cross, and its factor.
struct Section {
    object: ObjectId,
    factor: f64,
    /// Its row of `sections`.
    kind: usize,
}

/// Every object that may be a multiplying section: picked (or perhaps
/// picked) by a row of `sections`, and, with `shared_by`, reached from
/// enough checked spaces that it may be shared. A factor of one changes
/// nothing and is left out.
fn possible_sections(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
) -> Vec<Section> {
    if declared.sections.is_empty() {
        return Vec::new();
    }
    let candidates: Vec<Candidates<'_>> = declared
        .sections
        .iter()
        .map(|kind| Candidates::select(context, kind.objects))
        .collect();
    // How many checked spaces may reach each shared section; a space whose
    // sections cannot be read may reach any.
    let mut reaching: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let mut unread = 0;
    if let Some(path) = &declared.section_path {
        let mut universe: Vec<&Object> = Vec::new();
        for (kind, candidates) in declared.sections.iter().zip(&candidates) {
            if kind.shared_by.is_some() {
                for object in &candidates.universe {
                    if !universe.iter().any(|known| known.id == object.id) {
                        universe.push(object);
                    }
                }
            }
        }
        for space in Candidates::select(context, &rule.selector).universe {
            match path.related(context, &space.id, &universe) {
                Ok((reached, _)) => {
                    for object in reached {
                        *reaching.entry(object).or_default() += 1;
                    }
                }
                Err(_) => unread += 1,
            }
        }
    }
    let mut sections = Vec::new();
    for (index, (kind, candidates)) in declared.sections.iter().zip(&candidates).enumerate() {
        if kind.factor <= 1.0 {
            continue;
        }
        for object in &candidates.universe {
            if let Some(needed) = kind.shared_by
                && reaching.get(&object.id).copied().unwrap_or(0) + unread < needed
            {
                continue;
            }
            sections.push(Section {
                object: object.id.clone(),
                factor: kind.factor,
                kind: index,
            });
        }
    }
    sections
}

/// A space's occupant load, as an interval, and the area it comes from.
struct Load {
    least: u64,
    most: u64,
    area: PlanArea,
}

/// The occupants relying on one passage.
#[derive(Default)]
struct Reliance {
    least: u64,
    most: u64,
    /// Why more occupants may rely on it than `most`.
    unbounded: Vec<String>,
    /// The checked spaces it surely serves, counted in `least`.
    spaces: Vec<ObjectId>,
    /// The checked spaces it may serve, counted only in `most`.
    perhaps: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

/// The passages a space's walks cross.
#[derive(Default)]
struct Walked {
    /// Crossed by every shortest walk from every door, with the proof.
    sure: BTreeMap<ObjectId, Vec<Evidence>>,
    /// Perhaps crossed, and not surely.
    perhaps: BTreeSet<ObjectId>,
}

/// The occupants relying on every passage reached.
#[derive(Default)]
struct Served {
    passages: BTreeMap<ObjectId, Reliance>,
    /// Why any passage may serve more occupants: a space whose passages
    /// cannot be read.
    anywhere: Vec<String>,
}

/// What checking one space found: findings that stand, and what could not
/// be decided.
#[derive(Default)]
struct Checked {
    findings: Vec<Finding>,
    doubts: Vec<Unavailable>,
}

fn incomplete(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}

fn missing(service: &str) -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        format!("{service} service is not registered"),
    )
}

/// The least and the largest of `values`.
fn span(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, 0.0_f64), |(low, high), value| {
        (low.min(value), high.max(value))
    })
}

/// An occupant load for a message.
fn occupants(least: u64, most: u64) -> String {
    if least == most {
        format!("{least} occupant(s)")
    } else {
        format!("between {least} and {most} occupants")
    }
}

/// Objects reached from a space, split by whether their selection is sure.
#[derive(Clone, Default)]
struct Reached {
    sure: Vec<ObjectId>,
    maybe: Vec<ObjectId>,
    /// Why each of `maybe` is not sure.
    why: BTreeMap<ObjectId, String>,
    evidence: Vec<Evidence>,
}

impl Reached {
    fn doubts(&self, what: &str) -> Vec<String> {
        self.maybe
            .iter()
            .map(|object| {
                format!(
                    "whether {object} is {what} is undecided: {}",
                    self.why[object]
                )
            })
            .collect()
    }

    /// Takes out what `excluded` surely picks, and leaves what it may pick
    /// only possible.
    fn without(&mut self, excluded: &Candidates<'_>, what: &str) {
        let picked = |object: &ObjectId| excluded.universe.iter().any(|known| known.id == *object);
        let surely = |object: &ObjectId| picked(object) && !excluded.undecided.contains_key(object);
        self.sure.retain(|object| !surely(object));
        self.maybe.retain(|object| !surely(object));
        let mut doubted = Vec::new();
        self.sure.retain(|object| {
            if picked(object) {
                doubted.push(object.clone());
                false
            } else {
                true
            }
        });
        for object in self.maybe.iter().chain(&doubted) {
            if let Some(why) = excluded.undecided.get(object) {
                let doubt = format!("whether it is {what} is undecided ({why})");
                let entry = self.why.entry(object.clone()).or_default();
                *entry = if entry.is_empty() {
                    doubt
                } else {
                    format!("{entry}; {doubt}")
                };
            }
        }
        self.maybe.extend(doubted);
        self.maybe.sort();
        let maybe = &self.maybe;
        self.why.retain(|object, _| maybe.contains(object));
    }
}

impl Reached {
    fn contains(&self, object: &ObjectId) -> bool {
        self.sure.contains(object) || self.maybe.contains(object)
    }

    /// Adds `other`'s objects; an object sure in either is sure.
    fn merge(&mut self, other: Self) {
        for object in other.sure {
            if !self.sure.contains(&object) {
                self.sure.push(object);
            }
        }
        for object in other.maybe {
            if !self.sure.contains(&object) && !self.maybe.contains(&object) {
                self.why.insert(object.clone(), other.why[&object].clone());
                self.maybe.push(object);
            }
        }
        let sure = &self.sure;
        self.maybe.retain(|object| !sure.contains(object));
        let maybe = &self.maybe;
        self.why.retain(|object, _| maybe.contains(object));
        self.evidence.extend(other.evidence);
    }
}

/// Objects every walk from a start keeps out of: surely, and perhaps.
#[derive(Clone, Default)]
struct Avoid {
    sure: BTreeSet<ObjectId>,
    maybe: BTreeSet<ObjectId>,
}

impl Avoid {
    /// What every walk from `origin` surely keeps out of: for its lower
    /// bound.
    fn least(&self, origin: &ObjectId) -> Vec<ObjectId> {
        self.sure
            .iter()
            .filter(|object| *object != origin)
            .cloned()
            .collect()
    }

    /// What a walk from `origin` may have to keep out of: for its upper
    /// bound.
    fn most(&self, origin: &ObjectId) -> Vec<ObjectId> {
        self.sure
            .iter()
            .chain(&self.maybe)
            .filter(|object| *object != origin)
            .cloned()
            .collect()
    }
}

/// Widths required of each exit and of all together, as intervals.
struct Required {
    each: (f64, f64),
    total: Option<(f64, f64)>,
    /// Of a space's own doors together.
    total_door: Option<(f64, f64)>,
}

/// A stated or bounded exit width.
#[derive(Clone)]
enum Width {
    /// A stated clear width, and its evidence.
    Stated(f64, Vec<Evidence>),
    /// The footprint's longest plan diagonal, which no clear width exceeds.
    AtMost(f64, Evidence),
    Unknown(String),
}

/// The compartments a space lies in: surely, and perhaps (with why).
#[derive(Clone, Default)]
struct Assigned {
    sure: BTreeSet<ObjectId>,
    maybe: BTreeMap<ObjectId, String>,
}

/// Whether a space lies in a compartment.
enum Within {
    Yes,
    No,
    Unknown(String),
}

/// The doors out of a compartment, found by walking its spaces through
/// their doors.
struct Boundary {
    doors: Reached,
    /// Whether no door out may be missing.
    known: bool,
    doubts: Vec<Unavailable>,
}

/// How one thing on a route stands against a requirement.
enum Verdict {
    Good,
    /// Fails, with what the finding says and cites.
    Bad(String, Vec<Evidence>),
    /// Undecided, and why.
    Open(String),
}

/// A clear height: bracketed (as a door states it), measured exactly,
/// bounded from above, or unknown.
enum Height {
    Stated(f64, f64, String, Vec<Evidence>),
    Measured(f64, Evidence),
    AtMost(f64, Evidence),
    Unknown(String),
}

/// Which way `path` crosses the closed hinged leaves of `leaves` in plan:
/// along their opening direction (`Some(true)`), against it
/// (`Some(false)`), or undecided (no crossing, or crossings both ways).
///
/// A crossing is where a segment passes from behind a leaf's closed line
/// to its swing side or back, within a leaf's width of the leaf, beyond
/// which another wall's door would stand.
fn crossing(leaves: &DoorLeaves, path: &[MetricPoint]) -> Option<bool> {
    let (mut along, mut against) = (false, false);
    for leaf in leaves.hinged() {
        let (a, b) = leaf.closed_edge();
        let [ox, oy, _] = leaf.opening().components();
        let norm = ox.hypot(oy);
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let length = ex.hypot(ey);
        if norm < 0.5 || length <= 0.0 {
            return None;
        }
        let (nx, ny, ux, uy) = (ox / norm, oy / norm, ex / length, ey / length);
        let margin = leaf.width_metres();
        let side = |point: [f64; 3]| (point[0] - a[0]) * nx + (point[1] - a[1]) * ny;
        for pair in path.windows(2) {
            let (p, q) = (pair[0].coordinates_metres(), pair[1].coordinates_metres());
            let (sp, sq) = (side(p), side(q));
            if (sp < 0.0) == (sq < 0.0) {
                continue;
            }
            let share = sp / (sp - sq);
            let at = [p[0] + share * (q[0] - p[0]), p[1] + share * (q[1] - p[1])];
            let offset = (at[0] - a[0]) * ux + (at[1] - a[1]) * uy;
            if offset >= -margin && offset <= length + margin {
                if sq > sp {
                    along = true;
                } else {
                    against = true;
                }
            }
        }
    }
    match (along, against) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Where the walks out of one space end, and what they keep out of.
struct Escape {
    /// The space's exits usable for escape.
    exits: Reached,
    /// Every target a walk may end at: the exits, and more.
    targets: Reached,
    /// Whether no target is missing from `targets`.
    known: bool,
    placed: Placed,
    avoid: Avoid,
    /// The compartment whose boundary also ends the walks.
    compartment: Option<ObjectId>,
}

impl Escape {
    /// Where the walks end, for messages.
    fn goal(&self) -> String {
        match &self.compartment {
            None => "the nearest exit".to_owned(),
            Some(compartment) => {
                format!("the nearest exit or door out of compartment {compartment}")
            }
        }
    }
}

/// A space's exits placed as walking targets.
struct Placed {
    /// The sure exits with a point.
    sure: Vec<Target>,
    /// Every possible exit with a point.
    all: Vec<Target>,
    /// Whether every possible exit has a point.
    complete: bool,
    /// Why an exit has no point, or may not be one.
    doubts: Vec<Unavailable>,
}

/// A start (a door, or none for the farthest point), whether it surely is
/// one, and its travel bounded from below and from above.
type Measured = (Option<ObjectId>, bool, [Result<Travel, Unavailable>; 2]);

/// A bound on the travel from one start, and what supports it.
#[derive(Clone)]
struct Travel {
    lower: f64,
    upper: f64,
    /// Where the lower bound is attained, for a finding's message.
    at: Option<[f64; 2]>,
    /// A walk no longer than `upper`, where the routing answer names one.
    path: Option<Vec<MetricPoint>>,
    /// The target that walk reaches.
    target: Option<ObjectId>,
    evidence: Vec<Evidence>,
}

impl Travel {
    /// Nothing measured: the travel is at least `lower`, perhaps unbounded.
    fn unbounded(lower: f64) -> Self {
        Self {
            lower,
            upper: f64::INFINITY,
            at: None,
            path: None,
            target: None,
            evidence: Vec::new(),
        }
    }
}

/// A walk from a door (or a space) to the nearest of some exits, around
/// some objects: `(origin, exits, avoided)`, the avoided sorted.
type WalkKey = (ObjectId, Vec<ObjectId>, Vec<ObjectId>);

/// An exit and the point a walk reaches it at.
type Target = (ObjectId, MetricPoint);

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    declared: &'r Declaration<'r>,
    exits: Candidates<'c>,
    doors: Option<Candidates<'c>>,
    sections: Vec<Section>,
    passages: Option<Candidates<'c>>,
    /// Objects not usable for escape.
    no_escape: Option<Candidates<'c>>,
    /// The doors on routes whose loads are checked.
    route_doors: Option<Candidates<'c>>,
    /// Every compartment.
    compartments: Option<Candidates<'c>>,
    /// Per row of `zones`, what it picks.
    zones: Vec<Candidates<'c>>,
    /// The compartments of each space asked about.
    membership: RefCell<BTreeMap<ObjectId, Result<Assigned, Unavailable>>>,
    /// Walks already measured: travel and walked passages share them.
    walks: RefCell<BTreeMap<WalkKey, Result<Travel, Unavailable>>>,
    /// The connectors walks may climb, when the rule selects any.
    connectors: Option<Result<ConnectorRouting, Unavailable>>,
}

impl Judge<'_, '_> {
    /// The connectors every walk climbs through, if any.
    fn routing(&self) -> Result<Option<&ConnectorRouting>, Unavailable> {
        match &self.connectors {
            None => Ok(None),
            Some(Ok(routing)) => Ok(Some(routing)),
            Some(Err(why)) => Err(why.clone()),
        }
    }

    /// Whether a walk may climb a connector: its waypoints stand on one.
    fn climbs(&self, path: Option<&[MetricPoint]>) -> bool {
        let Some(Ok(routing)) = &self.connectors else {
            return false;
        };
        path.is_none_or(|path| {
            path.iter().any(|point| {
                routing
                    .connectors()
                    .iter()
                    .any(|connector| connector.object() == point.subject())
            })
        })
    }
}

/// What a relied-on object is: a passage, or a door on a route.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Passage,
    Door,
}

impl Class {
    fn noun(self) -> &'static str {
        match self {
            Self::Passage => "passage",
            Self::Door => "door",
        }
    }
}

/// What bounds a clear width from above where none is stated.
#[derive(Clone, Copy)]
enum Bound {
    /// The footprint's longest plan diagonal.
    Diagonal,
    /// The shorter side of the rectangle of least area enclosing the
    /// footprint.
    ShortSide,
}

impl Judge<'_, '_> {
    fn reached(
        &self,
        traversal: &Traversal,
        candidates: &Candidates<'_>,
        space: &ObjectId,
    ) -> Result<Reached, Unavailable> {
        let (reached, evidence) = traversal.related(self.context, space, &candidates.universe)?;
        let (maybe, sure): (Vec<ObjectId>, Vec<ObjectId>) = reached
            .into_iter()
            .partition(|object| candidates.undecided.contains_key(object));
        let why = maybe
            .iter()
            .map(|object| (object.clone(), candidates.undecided[object].clone()))
            .collect();
        Ok(Reached {
            sure,
            maybe,
            why,
            evidence,
        })
    }

    /// The exits of `space` usable for escape.
    fn exits_of(&self, space: &ObjectId) -> Result<Reached, Unavailable> {
        let mut exits = self.reached(&self.declared.exits, &self.exits, space)?;
        if let Some(no_escape) = &self.no_escape {
            exits.without(no_escape, "not usable for escape");
        }
        Ok(exits)
    }

    /// The doors of `space` a walk may start from: none surely unusable for
    /// escape.
    fn start_doors(&self, space: &ObjectId) -> Result<Reached, Unavailable> {
        let (Some((traversal, _)), Some(candidates)) =
            (self.declared.doors.as_ref(), self.doors.as_ref())
        else {
            return Err(invalid("no `door_path` and `door_selector` are declared"));
        };
        let mut doors = self.reached(traversal, candidates, space)?;
        if let Some(no_escape) = &self.no_escape {
            doors.without(no_escape, "not usable for escape");
        }
        Ok(doors)
    }

    /// What every walk out of `space` keeps out of: whatever is not usable
    /// for escape, and every zone ranked above the space's, surely or
    /// perhaps. The space itself, and its compartment, are never avoided.
    fn avoid(&self, space: &ObjectId) -> Result<Avoid, Unavailable> {
        let mut avoid = Avoid::default();
        if let Some(no_escape) = &self.no_escape {
            for object in &no_escape.universe {
                if object.id == *space {
                    continue;
                }
                if no_escape.undecided.contains_key(&object.id) {
                    avoid.maybe.insert(object.id.clone());
                } else {
                    avoid.sure.insert(object.id.clone());
                }
            }
        }
        if !self.zones.is_empty() {
            let own = self.start_rank(space)?;
            let compartment = self.compartment(space).ok();
            let ranked: BTreeSet<&ObjectId> = self
                .zones
                .iter()
                .flat_map(|zone| zone.universe.iter().map(|object| &object.id))
                .collect();
            for object in ranked {
                if object == space || compartment.as_ref() == Some(object) {
                    continue;
                }
                let (sure, possible) = self.rank(object);
                let ranks: Vec<i64> = possible.iter().map(|(rank, _)| *rank).chain(sure).collect();
                if sure.is_some() && ranks.iter().all(|rank| *rank > own) {
                    avoid.sure.insert(object.clone());
                } else if ranks.iter().any(|rank| *rank > own) {
                    avoid.maybe.insert(object.clone());
                }
            }
        }
        let sure = &avoid.sure;
        avoid.maybe.retain(|object| !sure.contains(object));
        Ok(avoid)
    }

    /// The rank of the first row of `zones` surely picking `object`, if
    /// any, and of every earlier row that may pick it, with why.
    fn rank(&self, object: &ObjectId) -> (Option<i64>, Vec<(i64, String)>) {
        let mut possible = Vec::new();
        for (zone, candidates) in self.declared.zones.iter().zip(&self.zones) {
            if !candidates.universe.iter().any(|known| known.id == *object) {
                continue;
            }
            match candidates.undecided.get(object) {
                Some(why) => possible.push((zone.rank, why.clone())),
                None => return (Some(zone.rank), possible),
            }
        }
        (None, possible)
    }

    /// The rank of the zone `space` lies in: its own, else its
    /// compartment's.
    fn start_rank(&self, space: &ObjectId) -> Result<i64, Unavailable> {
        let mut ranked = self.rank(space);
        if ranked.0.is_none()
            && ranked.1.is_empty()
            && let Ok(compartment) = self.compartment(space)
        {
            ranked = self.rank(&compartment);
        }
        match ranked {
            (Some(rank), possible) if possible.is_empty() => Ok(rank),
            (_, possible) if !possible.is_empty() => {
                let why: Vec<&str> = possible.iter().map(|(_, why)| why.as_str()).collect();
                Err(incomplete(format!(
                    "the zone it lies in is undecided: {}",
                    why.join("; ")
                )))
            }
            _ => Err(incomplete("no row of `zones` ranks it".to_owned())),
        }
    }

    /// The compartments `space` lies in, each space asked once.
    fn assigned(&self, space: &ObjectId) -> Result<Assigned, Unavailable> {
        if let Some(known) = self.membership.borrow().get(space) {
            return known.clone();
        }
        let assigned = self.assign(space);
        self.membership
            .borrow_mut()
            .insert(space.clone(), assigned.clone());
        assigned
    }

    fn assign(&self, space: &ObjectId) -> Result<Assigned, Unavailable> {
        let (Some(declared), Some(candidates)) = (
            self.declared.compartments.as_ref(),
            self.compartments.as_ref(),
        ) else {
            return Ok(Assigned::default());
        };
        let mut assigned = Assigned::default();
        if candidates.universe.is_empty() {
            return Ok(assigned);
        }
        let mut assign = |compartment: &ObjectId, doubt: Option<String>| match (
            candidates.undecided.get(compartment),
            doubt,
        ) {
            (None, None) => {
                assigned.sure.insert(compartment.clone());
            }
            (Some(why), _) => {
                assigned.maybe.insert(
                    compartment.clone(),
                    format!("whether {compartment} is a compartment is undecided: {why}"),
                );
            }
            (None, Some(doubt)) => {
                assigned.maybe.insert(compartment.clone(), doubt);
            }
        };
        match &declared.membership {
            Membership::Path(path) => {
                let (reached, _) = path.related(self.context, space, &candidates.universe)?;
                for compartment in &reached {
                    assign(compartment, None);
                }
            }
            Membership::Overlap(share) => {
                let Some(areas) = self.context.services.get::<PlanAreaServiceHandle>() else {
                    return Err(missing("plan-area"));
                };
                let area = footprint(self.context, space)?;
                for compartment in &candidates.universe {
                    let compartment = &compartment.id;
                    if compartment == space {
                        continue;
                    }
                    let overlap = match areas.measure_plan_overlap(space, compartment) {
                        Ok(overlap) => overlap,
                        Err(error) => {
                            assign(
                                compartment,
                                Some(format!(
                                    "its overlap with {compartment} is unknown: {error}"
                                )),
                            );
                            continue;
                        }
                    };
                    let least = overlap.lower_square_metres() / area.upper_square_metres();
                    let most = if area.lower_square_metres() > 0.0 {
                        overlap.upper_square_metres() / area.lower_square_metres()
                    } else {
                        f64::INFINITY
                    };
                    if most < *share {
                        continue;
                    }
                    let doubt = (least < *share).then(|| {
                        format!(
                            "{compartment} covers between {} and {} of its footprint, and \
                             {share} is asked",
                            shown(least, least),
                            shown(most, most)
                        )
                    });
                    assign(compartment, doubt);
                }
            }
        }
        Ok(assigned)
    }

    /// Whether `space` lies in `compartment`.
    fn within(&self, space: &ObjectId, compartment: &ObjectId) -> Within {
        match self.assigned(space) {
            Ok(assigned) if assigned.sure.contains(compartment) => Within::Yes,
            Ok(assigned) => match assigned.maybe.get(compartment) {
                Some(why) => Within::Unknown(format!(
                    "whether {space} lies in {compartment} is undecided: {why}"
                )),
                None => Within::No,
            },
            Err((_, message)) => Within::Unknown(format!(
                "the compartments of {space} cannot be read: {message}"
            )),
        }
    }

    /// The one compartment `space` surely lies in.
    fn compartment(&self, space: &ObjectId) -> Result<ObjectId, Unavailable> {
        let assigned = self.assigned(space)?;
        if !assigned.maybe.is_empty() {
            let why: Vec<&str> = assigned.maybe.values().map(String::as_str).collect();
            return Err(incomplete(format!(
                "the compartment it lies in is undecided: {}",
                why.join("; ")
            )));
        }
        let mut sure = assigned.sure.into_iter();
        match (sure.next(), sure.next()) {
            (Some(compartment), None) => Ok(compartment),
            (None, _) => Err(incomplete("it lies in no compartment".to_owned())),
            (Some(first), Some(second)) => Err(incomplete(format!(
                "it lies in several compartments ({first}, {second})"
            ))),
        }
    }

    /// The doors out of `compartment`, walking its spaces from `space`
    /// through their doors: a door reaching a space outside it, or no other
    /// space (it leads outside), leads out, unless only into what every walk
    /// avoids. Whatever is undecided on the way makes a door only a
    /// possible one, and a door or space that cannot be read may hide more.
    #[allow(clippy::too_many_lines)]
    fn boundary(&self, space: &ObjectId, compartment: &ObjectId, avoid: &Avoid) -> Boundary {
        let mut boundary = Boundary {
            doors: Reached::default(),
            known: true,
            doubts: Vec::new(),
        };
        let (Some((path, _)), Some(candidates)) =
            (self.declared.doors.as_ref(), self.doors.as_ref())
        else {
            boundary.known = false;
            return boundary;
        };
        let back = path.reversed();
        let everything: Vec<&Object> = self.context.project.objects().collect();
        let mut seen = BTreeSet::from([space.clone()]);
        let mut done = BTreeSet::new();
        // Members surely in the compartment are walked first, so a door is
        // first met from the surest side.
        let mut surely = vec![space.clone()];
        let mut perhaps: Vec<(ObjectId, String)> = Vec::new();
        let mut sure = BTreeSet::new();
        let mut maybe: BTreeMap<ObjectId, String> = BTreeMap::new();
        loop {
            let (member, doubt) = match surely.pop() {
                Some(member) => (member, None),
                None => match perhaps.pop() {
                    Some((member, why)) => (member, Some(why)),
                    None => break,
                },
            };
            let doors = match self.reached(path, candidates, &member) {
                Ok(doors) => doors,
                Err((_, message)) => {
                    boundary.known = false;
                    boundary.doubts.push(incomplete(format!(
                        "the doors of {member} cannot be read: {message}"
                    )));
                    continue;
                }
            };
            boundary
                .doors
                .evidence
                .extend(doors.evidence.iter().cloned());
            for door in doors.sure.iter().chain(&doors.maybe) {
                if !done.insert(door.clone()) {
                    continue;
                }
                let mut why: Vec<String> = doubt.iter().cloned().collect();
                if let Some(undecided) = doors.why.get(door) {
                    why.push(format!(
                        "whether {door} is a door is undecided: {undecided}"
                    ));
                }
                let (spaces, cited) = match back.related(self.context, door, &everything) {
                    Ok(found) => found,
                    Err((_, message)) => {
                        boundary.known = false;
                        maybe.insert(
                            door.clone(),
                            format!("the spaces of {door} cannot be read: {message}"),
                        );
                        continue;
                    }
                };
                boundary.doors.evidence.extend(cited);
                let others: Vec<ObjectId> = spaces
                    .into_iter()
                    .filter(|other| other != &member && other != door)
                    .collect();
                let mut out = others.is_empty();
                let mut open: Vec<String> = Vec::new();
                for other in others {
                    if avoid.sure.contains(&other) {
                        continue;
                    }
                    let avoided = avoid
                        .maybe
                        .contains(&other)
                        .then(|| format!("a walk may have to keep out of {other}"));
                    match self.within(&other, compartment) {
                        Within::Yes => {
                            if seen.insert(other.clone()) {
                                if why.is_empty() {
                                    surely.push(other);
                                } else {
                                    perhaps.push((other, why.join("; ")));
                                }
                            }
                        }
                        Within::No => match avoided {
                            None => out = true,
                            Some(avoided) => open.push(avoided),
                        },
                        Within::Unknown(unknown) => {
                            open.push(unknown.clone());
                            if seen.insert(other.clone()) {
                                let mut doubts = why.clone();
                                doubts.push(unknown);
                                perhaps.push((other, doubts.join("; ")));
                            }
                        }
                    }
                }
                if out && why.is_empty() {
                    sure.insert(door.clone());
                } else if out || !open.is_empty() {
                    why.extend(open);
                    maybe.insert(door.clone(), why.join("; "));
                }
            }
        }
        maybe.retain(|door, _| !sure.contains(door));
        boundary.doors.sure = sure.into_iter().collect();
        boundary.doors.maybe = maybe.keys().cloned().collect();
        boundary.doors.why = maybe;
        boundary
    }

    /// A project without any compartment cannot say where travel ends: an
    /// inadequate-information finding on every source of a checked space.
    fn no_compartment(&self, spaces: &[&Object], evaluation: &mut CapabilityEvaluation) {
        let Some(candidates) = &self.compartments else {
            return;
        };
        if !candidates.universe.is_empty() {
            return;
        }
        let mut done = BTreeSet::new();
        for space in spaces {
            if !done.insert(space.id.source.clone()) {
                continue;
            }
            let mut found = finding(
                self.rule,
                &space.id,
                "inadequate information: no compartment is modelled (`compartment_selector` \
                 picks nothing), so where escape travel ends at a compartment boundary is unknown"
                    .to_owned(),
                Vec::new(),
                Vec::new(),
            );
            found.scope = Scope::Source(space.id.source.clone());
            evaluation.push_finding(found);
        }
    }

    /// Where the walks out of `space` end, and what they keep out of.
    fn escape(&self, space: &ObjectId) -> Result<Escape, Unavailable> {
        let exits = self.exits_of(space)?;
        let avoid = self.avoid(space)?;
        let mut targets = exits.clone();
        let mut known = true;
        let mut doubts = Vec::new();
        let mut compartment = None;
        let mut what = "an exit";
        if self.declared.compartments.is_some() {
            let inside = self.compartment(space)?;
            let boundary = self.boundary(space, &inside, &avoid);
            known = boundary.known;
            doubts = boundary.doubts;
            targets.merge(boundary.doors);
            if let Some(no_escape) = &self.no_escape {
                targets.without(no_escape, "not usable for escape");
            }
            compartment = Some(inside);
            what = "an exit or a door out of its compartment";
        }
        let mut placed = self.placed(&targets, what);
        placed.complete &= known;
        placed.doubts.extend(doubts);
        Ok(Escape {
            exits,
            targets,
            known,
            placed,
            avoid,
            compartment,
        })
    }

    /// Checks one space, and answers its occupant load where its use
    /// states one.
    fn space(
        &self,
        space: &Object,
        use_: &Use<'_>,
        checked: &mut Checked,
    ) -> Option<Result<Load, Unavailable>> {
        let load = use_
            .area_per_occupant
            .map(|per_occupant| Self::load(self.context, &space.id, per_occupant));
        let exits = match self.exits_of(&space.id) {
            Ok(exits) => exits,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return load;
            }
        };
        if let Some(required) = use_.exits {
            if self.declared.count_routes {
                self.routes(space, use_, required, checked);
            } else {
                self.count(space, use_, required, &exits, checked);
            }
        }
        if let (Some(per_occupant), Some(load)) = (use_.area_per_occupant, &load) {
            match load {
                Ok(load) => self.widths(space, use_, per_occupant, load, &exits, checked),
                Err(unavailable) => checked.doubts.push(unavailable.clone()),
            }
        }
        if let Some(maximum) = use_.maximum_travel {
            self.travel(space, use_, maximum, checked);
        }
        if self.declared.door_direction {
            self.door_direction(space, &exits, checked);
        }
        if self.declared.route_door_direction || self.declared.minimum_height.is_some() {
            self.route(space, checked);
        }
        load
    }

    /// Every exit door must open out of `space`, in the direction of
    /// escape. An exit that may not be one decides only a doubt.
    fn door_direction(&self, space: &Object, exits: &Reached, checked: &mut Checked) {
        let (Some(frames), Some(free)) = (
            self.context.services.get::<ObjectFrameServiceHandle>(),
            self.context.services.get::<FreeSpaceServiceHandle>(),
        ) else {
            checked.doubts.push(missing("object-frame or free-space"));
            return;
        };
        let doors = exits
            .sure
            .iter()
            .map(|exit| (exit, true))
            .chain(exits.maybe.iter().map(|exit| (exit, false)));
        for (exit, sure) in doors {
            let leaves = match frames.leaves(exit) {
                Ok(leaves) => leaves,
                // An opening or a passage has no leaf to open.
                Err(DoorLeavesError::NotADoor(_)) => continue,
                Err(error) => {
                    checked.doubts.push((
                        door_swing::reason(&error),
                        format!("the leaves of exit {exit} are unknown: {error}"),
                    ));
                    continue;
                }
            };
            if leaves.hinged().next().is_none() {
                if sure {
                    checked.doubts.push(incomplete(format!(
                        "exit door {exit} has no hinged leaf ({}), so the direction it opens in \
                         is not defined",
                        leaves.operation()
                    )));
                }
                continue;
            }
            match door_swing::relation(free, &leaves, &space.id) {
                Ok((Relation::Into, evidence)) if sure => checked.findings.push(finding(
                    self.rule,
                    &space.id,
                    format!(
                        "exit door {exit} opens into the space, against the direction of escape"
                    ),
                    evidence,
                    vec![exit.clone()],
                )),
                Ok((Relation::Into, _)) => checked.doubts.push(incomplete(format!(
                    "{exit} opens into the space, and whether it is an exit is undecided: {}",
                    exits.why[exit]
                ))),
                Ok((Relation::Apart, _)) if sure => checked.doubts.push(incomplete(format!(
                    "neither side of exit door {exit} lies in the space at its probes, so the \
                     direction it opens in is not decided"
                ))),
                Ok(_) => {}
                Err(unavailable) => checked.doubts.push(unavailable),
            }
        }
    }

    /// The footprint over the area per occupant, rounded up at both ends.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn load(
        context: &RuleContext<'_>,
        space: &ObjectId,
        per_occupant: f64,
    ) -> Result<Load, Unavailable> {
        let area = footprint(context, space)?;
        let load = |area: f64| (area / per_occupant).ceil().max(1.0) as u64;
        Ok(Load {
            least: load(area.lower_square_metres()),
            most: load(area.upper_square_metres()),
            area,
        })
    }

    /// Adds `space`'s occupants to every passage it reaches; answers why its
    /// passages cannot be read.
    fn serve(
        &self,
        space: &ObjectId,
        load: Result<&Load, &str>,
        served: &mut Served,
    ) -> Option<Unavailable> {
        let (Some(declared), Some(candidates)) =
            (self.declared.passages.as_ref(), self.passages.as_ref())
        else {
            return None;
        };
        if !declared.judged {
            return None;
        }
        if declared.walked {
            let walked = self.walked(space, candidates);
            Self::rely(space, load, &walked, served);
            return None;
        }
        let mut reached = Vec::new();
        let mut evidence = Vec::new();
        if candidates.universe.iter().any(|object| object.id == *space) {
            reached.push(space.clone());
        }
        if let Some(path) = &declared.path {
            match path.related(self.context, space, &candidates.universe) {
                Ok((found, cited)) => {
                    reached.extend(found);
                    evidence = cited;
                }
                Err((_, message)) => {
                    served
                        .anywhere
                        .push(format!("the passages of {space} cannot be read: {message}"));
                    return Some(incomplete(format!(
                        "its passages cannot be read: {message}"
                    )));
                }
            }
        }
        for passage in reached {
            let reliance = served.passages.entry(passage).or_default();
            match load {
                Ok(load) => {
                    reliance.least += load.least;
                    reliance.most += load.most;
                    reliance.evidence.push(load.area.evidence().clone());
                }
                Err(why) => reliance.unbounded.push(format!("{space}: {why}")),
            }
            reliance.spaces.push(space.clone());
            reliance.evidence.extend(evidence.iter().cloned());
        }
        None
    }

    /// Adds `space`'s load to every object its walks rely on: surely to
    /// the least and most, perhaps to the most only.
    fn rely(space: &ObjectId, load: Result<&Load, &str>, walked: &Walked, served: &mut Served) {
        let empty = Vec::new();
        for (passage, proof) in walked
            .sure
            .iter()
            .map(|(passage, proof)| (passage, Some(proof)))
            .chain(walked.perhaps.iter().map(|passage| (passage, None)))
        {
            let reliance = served.passages.entry(passage.clone()).or_default();
            match (load, proof) {
                (Ok(load), Some(_)) => {
                    reliance.least += load.least;
                    reliance.most += load.most;
                    reliance.evidence.push(load.area.evidence().clone());
                }
                (Ok(load), None) => reliance.most += load.most,
                (Err(why), _) => reliance.unbounded.push(format!("{space}: {why}")),
            }
            if proof.is_some() {
                reliance.spaces.push(space.clone());
            } else {
                reliance.perhaps.push(space.clone());
            }
            reliance
                .evidence
                .extend(proof.unwrap_or(&empty).iter().cloned());
        }
    }

    /// Adds `space`'s occupants to every door on a route its walks rely on.
    fn serve_doors(&self, space: &ObjectId, load: Result<&Load, &str>, served: &mut Served) {
        if let Some(candidates) = self.route_doors.as_ref() {
            let walked = self.walked(space, candidates);
            Self::rely(space, load, &walked, served);
        }
    }

    /// The passages `space`'s occupants rely on, from the walks out of its
    /// doors: surely those every shortest walk from every door crosses, and
    /// perhaps every other one not proven off each door's shortest walks.
    /// What cannot be walked may use any passage.
    fn walked(&self, space: &ObjectId, candidates: &Candidates<'_>) -> Walked {
        let all: BTreeSet<ObjectId> = candidates
            .universe
            .iter()
            .map(|object| object.id.clone())
            .collect();
        let mut walked = Walked::default();
        // A space that is a passage carries its own occupants.
        if all.contains(space) {
            walked.sure.insert(space.clone(), Vec::new());
        }
        let anything = |mut walked: Walked| {
            walked.perhaps = all
                .iter()
                .filter(|passage| !walked.sure.contains_key(*passage))
                .cloned()
                .collect();
            walked
        };
        let (Some(routes), Some(profile)) = (
            self.context.services.get::<MetricRoutingServiceHandle>(),
            self.declared.profile,
        ) else {
            return anything(walked);
        };
        let Ok(escape) = self.escape(space) else {
            return anything(walked);
        };
        let doors = match self.start_doors(space) {
            Ok(doors) if !doors.sure.is_empty() || !doors.maybe.is_empty() => doors,
            _ => return anything(walked),
        };
        let mut sure: Option<BTreeMap<ObjectId, Vec<Evidence>>> = None;
        let mut perhaps = BTreeSet::new();
        for door in doors.sure.iter().chain(&doors.maybe) {
            let (door_sure, door_perhaps) =
                self.door_passages(routes, profile, door, &escape, &all);
            perhaps.extend(door_perhaps);
            sure = Some(match sure {
                None => door_sure,
                Some(mut known) => {
                    known.retain(|passage, _| door_sure.contains_key(passage));
                    for (passage, proof) in door_sure {
                        if let Some(cited) = known.get_mut(&passage) {
                            cited.extend(proof);
                        }
                    }
                    known
                }
            });
        }
        for (passage, proof) in sure.unwrap_or_default() {
            walked.sure.entry(passage).or_default().extend(proof);
        }
        walked.perhaps = perhaps
            .into_iter()
            .filter(|passage| !walked.sure.contains_key(passage))
            .collect();
        walked
    }

    /// The passages every shortest walk from `door` to the nearest exit
    /// surely crosses, with the proof, and those it may cross.
    ///
    /// The plain walk to the sure exits is at most `U` long. A passage is
    /// crossed by every shortest walk to whichever exits there are when the
    /// walk to every possible exit around it is longer than `U`, or reaches
    /// none: such a walk to the actual exits is longer than the shortest one.
    /// A passage is off every shortest walk when the plan distance from the
    /// door to it and on from it to the nearest possible exit already
    /// exceeds `U`. The walk the routing answer names crosses a passage it
    /// lies over; only those are tried around.
    #[allow(clippy::type_complexity)]
    fn door_passages(
        &self,
        routes: &MetricRoutingServiceHandle,
        profile: MobilityProfile,
        door: &ObjectId,
        escape: &Escape,
        all: &BTreeSet<ObjectId>,
    ) -> (BTreeMap<ObjectId, Vec<Evidence>>, BTreeSet<ObjectId>) {
        let mut sure = BTreeMap::new();
        let Some(walk) = self.witness(routes, profile, door, escape) else {
            return (sure, all.clone());
        };
        // Every walk from a door starts in it.
        if all.contains(door) {
            sure.insert(door.clone(), walk.evidence.clone());
        }
        let crossed = Self::crossed(routes, &walk, all);
        let targets: Vec<&ObjectId> = escape
            .targets
            .sure
            .iter()
            .chain(&escape.targets.maybe)
            .collect();
        let perhaps: BTreeSet<ObjectId> = all
            .iter()
            .filter(|passage| {
                crossed
                    .as_ref()
                    .is_some_and(|crossed| crossed.contains_key(*passage))
                    || !escape.known
                    || !self.off_every_walk(door, passage, &targets, walk.upper)
            })
            .cloned()
            .collect();
        let tried: Vec<&ObjectId> = match &crossed {
            Some(crossed) => crossed.keys().collect(),
            None => perhaps.iter().collect(),
        };
        for passage in tried {
            if let Some(proof) = self.on_every_walk(routes, profile, door, escape, &walk, passage) {
                sure.insert(passage.clone(), proof);
            }
        }
        (sure, perhaps)
    }

    /// The walk from `origin` to the nearest sure target around everything
    /// it may have to avoid: one shortest walk, no longer than its upper
    /// bound, which is finite.
    fn witness(
        &self,
        routes: &MetricRoutingServiceHandle,
        profile: MobilityProfile,
        origin: &ObjectId,
        escape: &Escape,
    ) -> Option<Travel> {
        if escape.placed.sure.is_empty() {
            return None;
        }
        self.nearest(
            routes,
            origin,
            &escape.placed.sure,
            &escape.avoid.most(origin),
            profile,
        )
        .ok()
        .filter(|walk| walk.upper.is_finite())
    }

    /// The objects of `among` the walk may lie over, each with the traced
    /// length (`None` where unmeasured); `None` when the walk cannot be
    /// traced.
    fn crossed(
        routes: &MetricRoutingServiceHandle,
        walk: &Travel,
        among: &BTreeSet<ObjectId>,
    ) -> Option<BTreeMap<ObjectId, Option<LengthInterval>>> {
        let path = walk.path.as_ref()?;
        if among.is_empty() {
            return Some(BTreeMap::new());
        }
        let request =
            PathTraceRequest::try_new(path.clone(), among.iter().cloned().collect()).ok()?;
        let trace = routes.trace_path(&request).ok()?;
        Some(
            request
                .objects()
                .iter()
                .zip(trace.lengths())
                .filter(|(_, length)| {
                    length
                        .as_ref()
                        .map_or(true, |length| length.upper_metres() > 0.0)
                })
                .map(|(object, length)| (object.clone(), length.as_ref().ok().copied()))
                .collect(),
        )
    }

    /// The proof that every shortest walk from `origin` to whichever
    /// targets there are enters `object`: with every possible target
    /// placed, the walk to them around it (and around only what surely is
    /// avoided) is longer than `walk`, the witness, or reaches none.
    fn on_every_walk(
        &self,
        routes: &MetricRoutingServiceHandle,
        profile: MobilityProfile,
        origin: &ObjectId,
        escape: &Escape,
        walk: &Travel,
        object: &ObjectId,
    ) -> Option<Vec<Evidence>> {
        if !escape.placed.complete || object == origin {
            return None;
        }
        let mut avoided = escape.avoid.least(origin);
        avoided.push(object.clone());
        let around = self
            .nearest(routes, origin, &escape.placed.all, &avoided, profile)
            .ok()?;
        (around.lower > walk.upper).then(|| {
            let mut proof = walk.evidence.clone();
            proof.extend(around.evidence);
            proof
        })
    }

    /// Whether every walk from `door` through `passage` to any of `exits`
    /// is surely longer than `upper`, by the plan distances alone.
    fn off_every_walk(
        &self,
        door: &ObjectId,
        passage: &ObjectId,
        exits: &[&ObjectId],
        upper: f64,
    ) -> bool {
        let Some(proximity) = self.context.services.get::<ProximityServiceHandle>() else {
            return false;
        };
        let apart = |from: &ObjectId, to: &ObjectId| {
            if from == to {
                return 0.0;
            }
            ProximityRequest::projected(from.clone(), to.clone(), ProximityProjection::Horizontal)
                .and_then(|request| proximity.measure_distance(&request))
                .map_or(0.0, |distance| distance.interval_metres().0)
        };
        let onward = exits
            .iter()
            .map(|exit| apart(passage, exit))
            .fold(f64::INFINITY, f64::min);
        apart(door, passage) + onward > upper
    }

    /// Judges every passage a checked space reaches, adding the outcome to
    /// that of the passage where it is a checked space itself.
    fn judge_passages(
        &self,
        served: Served,
        class: Class,
        results: &mut Vec<(ObjectId, String, Checked)>,
    ) {
        for (passage, reliance) in served.passages {
            let mut checked = Checked::default();
            self.passage(&passage, class, &reliance, &served.anywhere, &mut checked);
            if let Some((_, _, own)) = results.iter_mut().find(|(id, _, _)| *id == passage) {
                own.findings.extend(checked.findings);
                own.doubts.extend(checked.doubts);
            } else {
                results.push((passage, format!("escape-route {}", class.noun()), checked));
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn passage(
        &self,
        passage: &ObjectId,
        class: Class,
        reliance: &Reliance,
        anywhere: &[String],
        checked: &mut Checked,
    ) {
        let noun = class.noun();
        let (candidates, property, bound) = match class {
            Class::Passage => {
                let (Some(declared), Some(candidates)) =
                    (self.declared.passages.as_ref(), self.passages.as_ref())
                else {
                    return;
                };
                (candidates, declared.width, Bound::ShortSide)
            }
            Class::Door => {
                let Some(candidates) = self.route_doors.as_ref() else {
                    return;
                };
                (candidates, self.declared.clear_width, Bound::Diagonal)
            }
        };
        let unknown: Vec<&str> = reliance
            .unbounded
            .iter()
            .chain(anywhere)
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            checked.doubts.push(incomplete(format!(
                "the occupants relying on {noun} {passage} are unknown: {}",
                unknown.join("; ")
            )));
            return;
        }
        let (least, most) = (reliance.least, reliance.most);
        let required = self.rows(least, most).and_then(|rows| {
            rows.iter()
                .map(|row| match class {
                    Class::Passage => row.passage,
                    Class::Door => row.door,
                })
                .collect::<Option<Vec<f64>>>()
                .map(|widths| span(widths.into_iter()))
                .ok_or_else(|| format!("a row of `widths` states no `{noun}_width`"))
        });
        let (low, high) = match required {
            Ok(required) => required,
            Err(why) => {
                checked.doubts.push(incomplete(why));
                return;
            }
        };
        let names =
            |spaces: &[ObjectId]| spaces.iter().map(ToString::to_string).collect::<Vec<_>>();
        let from = match (reliance.spaces.is_empty(), reliance.perhaps.is_empty()) {
            (_, true) => format!("from {}", names(&reliance.spaces).join(", ")),
            (true, false) => format!("perhaps from {}", names(&reliance.perhaps).join(", ")),
            (false, false) => format!(
                "from {}; perhaps also from {}",
                names(&reliance.spaces).join(", "),
                names(&reliance.perhaps).join(", ")
            ),
        };
        let basis = format!(
            "{} relying on it ({from}) require at least {} m",
            occupants(least, most),
            shown(low, high)
        );
        let bounded_by = match bound {
            Bound::ShortSide => "the shorter side of the rectangle enclosing its footprint",
            Bound::Diagonal => "its whole footprint's longest plan diagonal",
        };
        let (what, cited) = match self.width(passage, property, bound) {
            Width::Stated(width, cited) if width < low => (
                format!("{noun} {passage} is {width} m wide (stated clear width)"),
                cited,
            ),
            Width::Stated(width, _) if width >= high => return,
            Width::Stated(width, _) => {
                checked.doubts.push(incomplete(format!(
                    "{noun} {passage} is {width} m wide, and {basis}"
                )));
                return;
            }
            Width::AtMost(bound, cited) if bound < low => (
                format!(
                    "{noun} {passage} is at most {} m wide ({bounded_by})",
                    shown(bound, bound)
                ),
                vec![cited],
            ),
            Width::AtMost(..) => {
                checked.doubts.push(incomplete(format!(
                    "the clear width of {noun} {passage} is not stated, and {basis}"
                )));
                return;
            }
            Width::Unknown(why) => {
                checked.doubts.push(incomplete(format!(
                    "the width of {noun} {passage} is unknown: {why}"
                )));
                return;
            }
        };
        if let Some(why) = candidates.undecided.get(passage) {
            checked.doubts.push(incomplete(format!(
                "whether {passage} is a {noun} is undecided ({why}), and it may be too narrow"
            )));
            return;
        }
        // A passage no one surely walks through requires no width.
        if reliance.spaces.is_empty() {
            checked.doubts.push(incomplete(format!(
                "{what}, and no walk surely crosses it; {basis}"
            )));
            return;
        }
        let mut evidence = reliance.evidence.clone();
        evidence.extend(cited);
        checked.findings.push(finding(
            self.rule,
            passage,
            format!("{what}; {basis}"),
            evidence,
            reliance.spaces.clone(),
        ));
    }

    /// Judges what the walks from `space`'s doors cross: each single-swing
    /// door's direction, and the clear height of each door, opening or
    /// space, the space itself included.
    ///
    /// The walk judged from each door is the one the routing answer names,
    /// among perhaps several shortest ones. What it crosses is judged; a
    /// failure is a finding only when every shortest walk crosses the
    /// object too (`on_every_walk`), else the space is not evaluated. The
    /// door a walk starts from is judged against the space, as exit doors
    /// are.
    #[allow(clippy::too_many_lines)]
    fn route(&self, space: &Object, checked: &mut Checked) {
        let (Some(routes), Some(profile), Some(doors)) = (
            self.context.services.get::<MetricRoutingServiceHandle>(),
            self.declared.profile,
            self.doors.as_ref(),
        ) else {
            checked.doubts.push(missing("metric-routing"));
            return;
        };
        let escape = match self.escape(&space.id) {
            Ok(escape) => escape,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        let starts = match self.start_doors(&space.id) {
            Ok(starts) if !starts.sure.is_empty() || !starts.maybe.is_empty() => starts,
            Ok(_) => {
                checked.doubts.push(incomplete(format!(
                    "{} reaches no door usable for escape to start from",
                    space.id
                )));
                return;
            }
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        let door_ids: BTreeSet<ObjectId> = doors
            .universe
            .iter()
            .map(|object| object.id.clone())
            .collect();
        let mut among = door_ids.clone();
        if let Some(passages) = &self.passages {
            among.extend(passages.universe.iter().map(|object| object.id.clone()));
        }
        let mut reported: BTreeMap<String, Finding> = BTreeMap::new();
        let mut report =
            |verdict: Verdict, sure: bool, proof: &[Evidence], checked: &mut Checked| match verdict
            {
                Verdict::Good => {}
                Verdict::Bad(message, cited) if sure => {
                    let mut evidence = proof.to_vec();
                    evidence.extend(cited);
                    reported.entry(message.clone()).or_insert_with(|| {
                        finding(self.rule, &space.id, message, evidence, Vec::new())
                    });
                }
                Verdict::Bad(message, _) => checked.doubts.push(incomplete(format!(
                    "{message}, and not every shortest walk is proven to cross it"
                ))),
                Verdict::Open(why) => checked.doubts.push(incomplete(why)),
            };
        if let Some(minimum) = self.declared.minimum_height {
            report(self.height(&space.id, minimum), true, &[], checked);
        }
        let undecided = |object: &ObjectId| {
            doors.undecided.contains_key(object)
                || self
                    .passages
                    .as_ref()
                    .is_some_and(|passages| passages.undecided.contains_key(object))
        };
        for (start, sure_start) in starts
            .sure
            .iter()
            .map(|door| (door, true))
            .chain(starts.maybe.iter().map(|door| (door, false)))
        {
            if self.declared.route_door_direction {
                report(self.start_way(start, &space.id), sure_start, &[], checked);
            }
            if let Some(minimum) = self.declared.minimum_height {
                report(self.height(start, minimum), sure_start, &[], checked);
            }
            let Some(walk) = self.witness(routes, profile, start, &escape) else {
                checked.doubts.push(incomplete(format!(
                    "no walk from door {start} to an exit is known to judge its route by"
                )));
                continue;
            };
            let (Some(crossed), Some(path)) = (Self::crossed(routes, &walk, &among), &walk.path)
            else {
                checked.doubts.push(incomplete(format!(
                    "the walk from door {start} cannot be traced over its route"
                )));
                continue;
            };
            for object in crossed.keys() {
                if object == start || *object == space.id {
                    continue;
                }
                let proof = self.on_every_walk(routes, profile, start, &escape, &walk, object);
                let sure = sure_start && proof.is_some() && !undecided(object);
                let proof = proof.unwrap_or_default();
                if self.declared.route_door_direction && door_ids.contains(object) {
                    report(self.door_way(object, path), sure, &proof, checked);
                }
                if let Some(minimum) = self.declared.minimum_height {
                    report(self.height(object, minimum), sure, &proof, checked);
                }
            }
        }
        checked.findings.extend(reported.into_values());
    }

    /// Which way a door on a route opens along the walk `path`.
    fn door_way(&self, door: &ObjectId, path: &[MetricPoint]) -> Verdict {
        let leaves = match self.route_leaves(door) {
            Ok(Some(leaves)) => leaves,
            Ok(None) => return Verdict::Good,
            Err(verdict) => return verdict,
        };
        match crossing(&leaves, path) {
            Some(true) => Verdict::Good,
            Some(false) => Verdict::Bad(
                format!("door {door} on its route opens against the direction of escape"),
                vec![leaves.evidence().clone()],
            ),
            None => Verdict::Open(format!(
                "which way the walk crosses door {door} on its route is not decided"
            )),
        }
    }

    /// Which way the door a walk starts from opens, against the space.
    fn start_way(&self, door: &ObjectId, space: &ObjectId) -> Verdict {
        let leaves = match self.route_leaves(door) {
            Ok(Some(leaves)) => leaves,
            Ok(None) => return Verdict::Good,
            Err(verdict) => return verdict,
        };
        let Some(free) = self.context.services.get::<FreeSpaceServiceHandle>() else {
            return Verdict::Open("free-space service is not registered".to_owned());
        };
        match door_swing::relation(free, &leaves, space) {
            Ok((Relation::Into, evidence)) => Verdict::Bad(
                format!(
                    "door {door} it is left by opens into the space, against the direction of \
                     escape"
                ),
                evidence,
            ),
            Ok((Relation::Away | Relation::BothWays, _)) => Verdict::Good,
            Ok((Relation::Apart, _)) => Verdict::Open(format!(
                "neither side of door {door} lies in the space at its probes"
            )),
            Err((_, message)) => Verdict::Open(message),
        }
    }

    /// The leaves of a single-swing door on a route: `None` for anything
    /// that swings in no one direction (no door, a sliding door, a
    /// double-acting one); a door whose operation is not stated fails, as
    /// its direction is undefined.
    fn route_leaves(&self, door: &ObjectId) -> Result<Option<DoorLeaves>, Verdict> {
        let Some(frames) = self.context.services.get::<ObjectFrameServiceHandle>() else {
            return Err(Verdict::Open(
                "object-frame service is not registered".to_owned(),
            ));
        };
        match frames.leaves(door) {
            Ok(leaves) => {
                let single = leaves.hinged().any(|leaf| {
                    leaf.swing()
                        .is_some_and(|sector| !sector.is_double_acting())
                });
                Ok(single.then_some(leaves))
            }
            Err(DoorLeavesError::NotADoor(_)) => Ok(None),
            Err(DoorLeavesError::NotStated(what)) => Err(Verdict::Bad(
                format!("door {door} on its route opens in an undefined direction ({what})"),
                Vec::new(),
            )),
            Err(error) => Err(Verdict::Open(format!(
                "the leaves of door {door} are unknown: {error}"
            ))),
        }
    }

    /// How the clear height of `object` stands against `minimum`.
    fn height(&self, object: &ObjectId, minimum: f64) -> Verdict {
        let low = |what: String, cited| {
            Verdict::Bad(
                format!("{what}; its route needs at least {minimum} m of clear height"),
                cited,
            )
        };
        match self.clear_height(object) {
            Height::Stated(lower, upper, what, cited) if upper < minimum => low(
                format!(
                    "{object} on its route is {} m high ({what})",
                    shown(lower, upper)
                ),
                cited,
            ),
            Height::Stated(lower, upper, what, _) if lower < minimum => Verdict::Open(format!(
                "{object} on its route is {} m high ({what}), and its route needs at least \
                 {minimum} m",
                shown(lower, upper)
            )),
            Height::Measured(height, cited) if height < minimum => low(
                format!(
                    "{object} on its route is {} m high (its measured clear height)",
                    shown(height, height)
                ),
                vec![cited],
            ),
            Height::AtMost(height, cited) if height < minimum => low(
                format!(
                    "{object} on its route is at most {} m high (its vertical extent)",
                    shown(height, height)
                ),
                vec![cited],
            ),
            Height::Stated(..) | Height::Measured(..) => Verdict::Good,
            Height::AtMost(..) => Verdict::Open(format!(
                "the clear height of {object} on its route is not stated"
            )),
            Height::Unknown(why) => Verdict::Open(format!(
                "the clear height of {object} on its route is unknown: {why}"
            )),
        }
    }

    /// A clear height: as `keyed-limit`'s `clear-height` reads a door's
    /// (stated, else overall less lining and threshold), else a space's
    /// measured clear height, else no more than the vertical extent.
    fn clear_height(&self, object: &ObjectId) -> Height {
        let [stated, overall, lining, threshold] = self.declared.clear_height;
        if (stated.is_some() || overall.is_some())
            && let Some(found) = self.context.project.object(object)
            && let Ok((lower, upper, what, cited)) =
                door_clear_height(self.context, found, stated, overall, lining, threshold)
        {
            return Height::Stated(lower, upper, what, cited);
        }
        if let Some(spaces) = self.context.services.get::<SpaceServiceHandle>()
            && let Ok(measured) = spaces.get().measure_clear_height(object)
            && measured.space() == object
        {
            return Height::Measured(measured.metres(), measured.evidence().clone());
        }
        let Some(extents) = self.context.services.get::<VerticalExtentServiceHandle>() else {
            return Height::Unknown("the vertical-extent service is not registered".to_owned());
        };
        match extents.measure_vertical_extent(object) {
            Ok(extent) => Height::AtMost(
                extent.top().upper_metres() - extent.bottom().lower_metres(),
                extent.evidence().clone(),
            ),
            Err(error) => Height::Unknown(error.to_string()),
        }
    }

    /// Counts the independent routes out of `space`: routes to distinct
    /// targets, two sharing a passage (within the start's compartment)
    /// counting once.
    ///
    /// At most, every target some walk may reach, and one when a passage
    /// the nearest walk crosses cuts every walk (the walk around it reaches
    /// no target). At least, as many sure targets as have walks crossing
    /// pairwise no common passage, as traced; a walk that cannot be traced
    /// may share every passage.
    #[allow(clippy::too_many_lines)]
    fn routes(&self, space: &Object, use_: &Use<'_>, required: usize, checked: &mut Checked) {
        let needed = format!("{} requires at least {required}", use_.name);
        let (Some(routes), Some(profile), Some(candidates)) = (
            self.context.services.get::<MetricRoutingServiceHandle>(),
            self.declared.profile,
            self.passages.as_ref(),
        ) else {
            checked.doubts.push(missing("metric-routing"));
            return;
        };
        let escape = match self.escape(&space.id) {
            Ok(escape) => escape,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        let origin = &space.id;
        // The passages two routes may share (within the compartment, if
        // any), and those surely passages surely within it.
        let mut shared = BTreeSet::new();
        let mut surely = BTreeSet::new();
        for passage in &candidates.universe {
            let passage = &passage.id;
            if passage == origin || escape.targets.contains(passage) {
                continue;
            }
            let inside = match &escape.compartment {
                None => Within::Yes,
                Some(compartment) => self.within(passage, compartment),
            };
            match inside {
                Within::Yes => {
                    shared.insert(passage.clone());
                    if !candidates.undecided.contains_key(passage) {
                        surely.insert(passage.clone());
                    }
                }
                Within::Unknown(_) => {
                    shared.insert(passage.clone());
                }
                Within::No => {}
            }
        }
        let mut evidence = escape.targets.evidence.clone();
        // At most: every target a walk may reach.
        let mut reachable = 0_usize;
        for (target, point) in &escape.placed.all {
            let alone = [(target.clone(), point.clone())];
            match self.nearest(routes, origin, &alone, &escape.avoid.least(origin), profile) {
                Ok(walk) if walk.lower.is_infinite() => evidence.extend(walk.evidence),
                _ => reachable += 1,
            }
        }
        // One passage every walk crosses leaves one route.
        let mut cut = None;
        if escape.placed.complete
            && let Some(walk) = self.witness(routes, profile, origin, &escape)
            && let Some(crossed) = Self::crossed(routes, &walk, &surely)
        {
            for passage in crossed.keys() {
                let mut avoided = escape.avoid.least(origin);
                avoided.push(passage.clone());
                if let Ok(around) =
                    self.nearest(routes, origin, &escape.placed.all, &avoided, profile)
                    && around.lower.is_infinite()
                {
                    evidence.extend(walk.evidence.iter().cloned());
                    evidence.extend(around.evidence);
                    cut = Some(passage.clone());
                    break;
                }
            }
        }
        let most = if !escape.placed.complete {
            None
        } else if cut.is_some() {
            Some(reachable.min(1))
        } else {
            Some(reachable)
        };
        // At least: sure targets walked to over pairwise distinct passages,
        // the shorter walks first.
        let mut walks = Vec::new();
        for (target, point) in &escape.placed.sure {
            let alone = [(target.clone(), point.clone())];
            if let Ok(walk) =
                self.nearest(routes, origin, &alone, &escape.avoid.most(origin), profile)
                && walk.upper.is_finite()
            {
                let crosses: BTreeSet<ObjectId> = match Self::crossed(routes, &walk, &shared) {
                    Some(crossed) => crossed.into_keys().collect(),
                    None => shared.clone(),
                };
                walks.push((walk.upper, crosses));
            }
        }
        walks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut taken = BTreeSet::new();
        let mut least = 0_usize;
        for (_, crosses) in walks {
            if crosses.is_disjoint(&taken) {
                least += 1;
                taken.extend(crosses);
            }
        }
        let related: Vec<ObjectId> = escape
            .targets
            .sure
            .iter()
            .chain(&escape.targets.maybe)
            .cloned()
            .collect();
        match most {
            Some(most) if most < required => {
                let message = match &cut {
                    Some(passage) => format!(
                        "every walk from it to an exit passes through {passage}, so it has at \
                         most {most} independent route(s); {needed}"
                    ),
                    None => format!(
                        "it reaches at most {most} exit(s) walking, so it has at most {most} \
                         independent route(s); {needed}"
                    ),
                };
                let mut related = related;
                related.extend(cut);
                checked
                    .findings
                    .push(finding(self.rule, &space.id, message, evidence, related));
            }
            _ if least >= required => {}
            most => {
                checked.doubts.extend(escape.placed.doubts);
                checked.doubts.push(incomplete(format!(
                    "it has at least {least} and {} independent route(s), and {needed}",
                    most.map_or_else(
                        || "perhaps more".to_owned(),
                        |most| format!("at most {most}")
                    )
                )));
            }
        }
    }

    fn count(
        &self,
        space: &Object,
        use_: &Use<'_>,
        required: usize,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let (sure, maybe) = (exits.sure.len(), exits.maybe.len());
        if sure + maybe < required {
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "has {} exit(s) via {}; {} requires at least {required}",
                    sure + maybe,
                    self.declared.exits.relationship,
                    use_.name
                ),
                exits.evidence.clone(),
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
        } else if sure < required {
            checked.doubts.push(incomplete(format!(
                "{sure} certain exit(s), at least {required} required: {}",
                exits.doubts("an exit").join("; ")
            )));
        }
    }

    /// An exit's or passage's clear width: stated in `property` (and the
    /// parameter declaring it), else bounded by its footprint.
    fn width(&self, exit: &ObjectId, property: Option<PropertyRef<'_>>, bound: Bound) -> Width {
        let mut why = match (property, bound) {
            (Some(_), _) => String::new(),
            (None, Bound::Diagonal) => "no `clear_width_property` is declared".to_owned(),
            (None, Bound::ShortSide) => "no `passage_width_property` is declared".to_owned(),
        };
        if let Some(property) = property {
            match self.context.project.object(exit) {
                None => why = format!("{exit} is not in the project"),
                Some(object) => match resolve(self.context, object, property) {
                    Ok(resolved) => match resolved.value() {
                        None => why = format!("{property} of {exit} is not stated"),
                        Some(PropertyValue::Quantity {
                            value,
                            dimension: QuantityDimension::Length,
                        }) if value.is_finite() && *value > 0.0 => {
                            return Width::Stated(*value, resolved.evidence());
                        }
                        Some(other) => {
                            return Width::Unknown(format!(
                                "{property} of {exit} is {}, not a positive length",
                                display(Some(other))
                            ));
                        }
                    },
                    Err((_, message)) => {
                        return Width::Unknown(format!(
                            "{property} of {exit} cannot be read: {message}"
                        ));
                    }
                },
            }
        }
        let Some(spans) = self.context.services.get::<PlanSpanServiceHandle>() else {
            return Width::Unknown(format!(
                "{why}, and the plan-span service is not registered"
            ));
        };
        match bound {
            Bound::Diagonal => match spans.measure_diameter(exit) {
                Ok(diameter) => Width::AtMost(diameter.upper_metres(), diameter.evidence().clone()),
                Err(error) => Width::Unknown(format!("{why}, and its footprint: {error}")),
            },
            // A body that passes stands on a disc of its width inside the
            // footprint, so the footprint is at least that wide across in
            // every direction, and so is any rectangle enclosing it.
            Bound::ShortSide => match spans.measure_rectangle(exit) {
                Ok(rectangle) => match rectangle.width_and_length() {
                    Ok([(_, width), _]) => Width::AtMost(width, rectangle.evidence().clone()),
                    Err(reason) => Width::Unknown(format!("{why}, and {reason}")),
                },
                Err(error) => Width::Unknown(format!("{why}, and its footprint: {error}")),
            },
        }
    }

    /// The rows of `widths` that may apply to a load between `least` and
    /// `most` occupants.
    fn rows(&self, least: u64, most: u64) -> Result<&[WidthRow], String> {
        let rows = &self.declared.widths;
        let from = rows
            .iter()
            .position(|row| row.occupants >= least)
            .ok_or_else(|| format!("no row of `widths` covers {least} occupant(s)"))?;
        let to = rows
            .iter()
            .position(|row| row.occupants >= most)
            .ok_or_else(|| format!("no row of `widths` covers {most} occupant(s)"))?;
        Ok(&rows[from..=to])
    }

    /// The widths `widths` requires for any load between `least` and
    /// `most` occupants: of each exit, and of all together where the rows
    /// state it.
    fn required_width(&self, least: u64, most: u64) -> Result<Required, String> {
        let rows = self.rows(least, most)?;
        let together = |column: &str, totals: Vec<f64>| {
            if totals.is_empty() {
                Ok(None)
            } else if totals.len() == rows.len() {
                Ok(Some(span(totals.into_iter())))
            } else {
                Err(format!(
                    "the rows of `widths` for {least} to {most} occupants state `{column}` \
                     only in part"
                ))
            }
        };
        Ok(Required {
            each: span(rows.iter().map(|row| row.width)),
            total: together(
                "total_width",
                rows.iter().filter_map(|row| row.total).collect(),
            )?,
            total_door: together(
                "total_door_width",
                rows.iter().filter_map(|row| row.total_door).collect(),
            )?,
        })
    }

    fn widths(
        &self,
        space: &Object,
        use_: &Use<'_>,
        per_occupant: f64,
        load: &Load,
        exits: &Reached,
        checked: &mut Checked,
    ) {
        let Load { least, most, area } = load;
        let (least, most) = (*least, *most);
        let occupants = occupants(least, most);
        let required = match self.required_width(least, most) {
            Ok(required) => required,
            Err(why) => {
                checked.doubts.push(incomplete(why));
                return;
            }
        };
        let (low, high) = required.each;
        let width = |exit| self.width(exit, self.declared.clear_width, Bound::Diagonal);
        let sure: Vec<Width> = exits.sure.iter().map(width).collect();
        let maybe: Vec<Width> = exits.maybe.iter().map(width).collect();
        let basis = format!(
            "{occupants} ({} m² at {per_occupant} m² each) require at least {} m ({})",
            shown(area.lower_square_metres(), area.upper_square_metres()),
            shown(low, high),
            use_.name
        );
        for (exit, width) in exits.sure.iter().zip(&sure) {
            let mut evidence = exits.evidence.clone();
            evidence.push(area.evidence().clone());
            match width.clone() {
                Width::Stated(width, cited) if width < low => {
                    evidence.extend(cited);
                    checked.findings.push(finding(
                        self.rule,
                        &space.id,
                        format!("exit {exit} is {width} m wide (stated clear width); {basis}"),
                        evidence,
                        vec![exit.clone()],
                    ));
                }
                Width::Stated(width, _) if width >= high => {}
                Width::Stated(width, _) => checked.doubts.push(incomplete(format!(
                    "exit {exit} is {width} m wide, and {basis}"
                ))),
                Width::AtMost(bound, cited) if bound < low => {
                    evidence.push(cited);
                    checked.findings.push(finding(
                        self.rule,
                        &space.id,
                        format!(
                            "exit {exit} is at most {} m wide (its whole footprint's longest \
                             plan diagonal); {basis}",
                            shown(bound, bound)
                        ),
                        evidence,
                        vec![exit.clone()],
                    ));
                }
                Width::AtMost(..) => checked.doubts.push(incomplete(format!(
                    "the clear width of exit {exit} is not stated, and {basis}"
                ))),
                Width::Unknown(why) => checked.doubts.push(incomplete(format!(
                    "the width of exit {exit} is unknown: {why}"
                ))),
            }
        }
        // An undecided exit needs a width only if it is one.
        for (exit, width) in exits.maybe.iter().zip(&maybe) {
            if !matches!(width, Width::Stated(width, _) if *width >= high) {
                checked.doubts.push(incomplete(format!(
                    "whether {exit} is an exit is undecided ({}), and it may be too narrow",
                    exits.why[exit]
                )));
            }
        }
        if let Some(total) = required.total {
            let widths = (sure.as_slice(), maybe.as_slice());
            self.total_width(
                space, use_, &occupants, area, total, exits, widths, "exit", checked,
            );
        }
        if let Some(total) = required.total_door {
            match self.start_doors(&space.id) {
                Ok(doors) => {
                    let sure: Vec<Width> = doors.sure.iter().map(width).collect();
                    let maybe: Vec<Width> = doors.maybe.iter().map(width).collect();
                    let widths = (sure.as_slice(), maybe.as_slice());
                    self.total_width(
                        space, use_, &occupants, area, total, &doors, widths, "door", checked,
                    );
                }
                Err(unavailable) => checked.doubts.push(unavailable),
            }
        }
    }

    /// Judges the exits' widths together against `total`.
    #[allow(clippy::too_many_arguments)]
    fn total_width(
        &self,
        space: &Object,
        use_: &Use<'_>,
        occupants: &str,
        area: &PlanArea,
        (least_total, most_total): (f64, f64),
        exits: &Reached,
        (sure, maybe): (&[Width], &[Width]),
        noun: &str,
        checked: &mut Checked,
    ) {
        // Together the sure exits are at least as wide as their stated
        // widths; all that might be exits at most as wide as their bounds.
        let stated: f64 = sure
            .iter()
            .map(|width| match width {
                Width::Stated(width, _) => *width,
                _ => 0.0,
            })
            .sum();
        let bound: f64 = sure
            .iter()
            .chain(maybe)
            .map(|width| match width {
                Width::Stated(width, _) | Width::AtMost(width, _) => *width,
                Width::Unknown(_) => f64::INFINITY,
            })
            .sum();
        let together = format!(
            "{occupants} require at least {} m of {noun} width together ({})",
            shown(least_total, most_total),
            use_.name
        );
        if bound < least_total {
            let mut evidence = exits.evidence.clone();
            evidence.push(area.evidence().clone());
            for width in sure.iter().chain(maybe) {
                match width {
                    Width::Stated(_, cited) => evidence.extend(cited.iter().cloned()),
                    Width::AtMost(_, cited) => evidence.push(cited.clone()),
                    Width::Unknown(_) => {}
                }
            }
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "its {} {noun}(s) are at most {} m wide together; {together}",
                    sure.len() + maybe.len(),
                    shown(bound, bound)
                ),
                evidence,
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
        } else if stated < most_total {
            checked.doubts.push(incomplete(format!(
                "its {noun}s are {} m wide together by their stated widths, and {together}",
                shown(stated, stated)
            )));
        }
    }

    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_lines)]
    fn travel(&self, space: &Object, use_: &Use<'_>, maximum: f64, checked: &mut Checked) {
        let allows = format!("{} allows at most {maximum} m of travel", use_.name);
        let escape = match self.escape(&space.id) {
            Ok(escape) => escape,
            Err(unavailable) => {
                checked.doubts.push(unavailable);
                return;
            }
        };
        let exits = &escape.exits;
        if escape.known && escape.targets.sure.is_empty() && escape.targets.maybe.is_empty() {
            let door_out = escape
                .compartment
                .as_ref()
                .map_or_else(String::new, |compartment| {
                    format!(" and no door out of compartment {compartment}")
                });
            checked.findings.push(finding(
                self.rule,
                &space.id,
                format!(
                    "has no exit via {}{door_out} to walk to; {allows}",
                    self.declared.exits.relationship
                ),
                escape.targets.evidence.clone(),
                Vec::new(),
            ));
            return;
        }
        let Some(routes) = self.context.services.get::<MetricRoutingServiceHandle>() else {
            checked.doubts.push(missing("metric-routing"));
            return;
        };
        let Some(profile) = self.declared.profile else {
            checked
                .doubts
                .push(invalid("`maximum_travel` needs a walking profile"));
            return;
        };
        // What could change the verdict, reported only if it stays open.
        let Placed {
            sure,
            all,
            complete: placed,
            doubts,
        } = &escape.placed;
        let mut doubts = doubts.clone();
        let bounds = |door: Option<&ObjectId>| -> [Result<Travel, Unavailable>; 2] {
            let measure = |targets: &[Target], avoided: Vec<ObjectId>| {
                if targets.is_empty() {
                    return Ok(Travel::unbounded(f64::INFINITY));
                }
                match door {
                    None => self.farthest(routes, &space.id, targets, &avoided, profile),
                    Some(door) => self.nearest(routes, door, targets, &avoided, profile),
                }
            };
            let from = door.unwrap_or(&space.id);
            let upper = measure(sure, escape.avoid.most(from));
            // The farthest point is measured on the plain walk, which no
            // walk around anything undercuts: a lower bound either way.
            let least = if door.is_some() {
                escape.avoid.least(from)
            } else {
                Vec::new()
            };
            let lower = if !placed {
                // A target without a point might lie anywhere.
                Ok(Travel::unbounded(0.0))
            } else if sure.len() == all.len() && least == escape.avoid.most(from) {
                upper.clone()
            } else {
                measure(all, least)
            };
            [lower, upper]
        };
        let measured: Vec<Measured> = match use_.start {
            Start::FarthestPoint => vec![(None, true, bounds(None))],
            Start::Door => {
                let doors = match self.start_doors(&space.id) {
                    Ok(doors) => doors,
                    Err(unavailable) => {
                        checked.doubts.push(unavailable);
                        return;
                    }
                };
                if doors.sure.is_empty() && doors.maybe.is_empty() {
                    checked.doubts.push(incomplete(format!(
                        "{} reaches no door usable for escape to start from",
                        space.id
                    )));
                    return;
                }
                doubts.extend(doors.doubts("a door of it").into_iter().map(incomplete));
                doors
                    .sure
                    .iter()
                    .map(|door| (door, true))
                    .chain(doors.maybe.iter().map(|door| (door, false)))
                    .map(|(door, sure_start)| (Some(door.clone()), sure_start, bounds(Some(door))))
                    .collect()
            }
        };
        // Travel is judged at its worst start: a finding needs one sure
        // start whose every route is too long, a pass every possible start
        // within the maximum.
        // Sections multiply the walk by at least one, so the plain walk's
        // lower bound stands. Its upper bound grows by the largest factor
        // of a section it may cross, or, where the routing answer names a
        // walk, to that walk's own multiplied length: any walk's cost
        // bounds the least cost from above.
        let mut most = 0.0_f64;
        let mut multiplied = 1.0_f64;
        let mut crossed = BTreeSet::new();
        let shared = self.declared.common_path.unwrap_or(1.0);
        let mut common_counted = false;
        let mut worst: Option<(Option<ObjectId>, Travel)> = None;
        for (start, sure_start, [lower, upper]) in measured {
            match upper {
                Ok(upper) => {
                    let from = start.as_ref().unwrap_or(&space.id);
                    let (factor, mut kinds) = self.factor(from, upper.upper, maximum);
                    let mut bound = upper.upper * factor;
                    // The common path is at most the whole walk.
                    let mut common = (shared - 1.0) * upper.upper;
                    if bound + common > maximum
                        && let Some(path) = &upper.path
                    {
                        if let Some((cost, traced)) = self.walked_cost(routes, path, upper.upper)
                            && cost < bound
                        {
                            bound = cost;
                            kinds = traced;
                        }
                        if shared > 1.0
                            && let Some(door) = &start
                            && let Some(length) =
                                self.common_length(routes, profile, door, &upper, &escape)
                        {
                            common = common.min((shared - 1.0) * length);
                        }
                    }
                    common_counted |= common > 0.0;
                    most = most.max(bound + common);
                    for kind in kinds {
                        multiplied = multiplied.max(self.declared.sections[kind].factor);
                        crossed.insert(kind);
                    }
                }
                Err(unavailable) => {
                    most = f64::INFINITY;
                    doubts.push(unavailable);
                }
            }
            match lower {
                Ok(lower) if sure_start => {
                    if worst
                        .as_ref()
                        .is_none_or(|(_, known)| lower.lower > known.lower)
                    {
                        worst = Some((start, lower));
                    }
                }
                Ok(_) => {}
                Err(unavailable) => doubts.push(unavailable),
            }
        }
        let from = |start: &Option<ObjectId>| match start {
            Some(door) => format!("door {door}"),
            None => "its farthest point".to_owned(),
        };
        if let Some((start, travel)) = &worst
            && travel.lower > maximum
        {
            let place = travel
                .at
                .map(|[x, y]| format!(", around ({x:.2}, {y:.2}),"))
                .unwrap_or_default();
            let none = match &escape.compartment {
                None => "no exit".to_owned(),
                Some(compartment) => format!("no exit or door out of compartment {compartment}"),
            };
            let message = if travel.lower.is_infinite() {
                if start.is_none() {
                    format!("part of it{place} reaches {none} walking; {allows}")
                } else {
                    format!("{} reaches {none} walking; {allows}", from(start))
                }
            } else {
                format!(
                    "{}{place} lies {} m from {} walking; {allows}",
                    from(start),
                    if travel.upper.is_finite() {
                        shown(travel.lower, travel.upper)
                    } else {
                        format!("at least {}", shown(travel.lower, travel.lower))
                    },
                    escape.goal()
                )
            };
            let mut evidence = exits.evidence.clone();
            evidence.extend(travel.evidence.iter().cloned());
            checked.findings.push(finding(
                self.rule,
                &space.id,
                message,
                evidence,
                exits.sure.iter().chain(&exits.maybe).cloned().collect(),
            ));
            // A travel finding stands; what is undecided cannot withdraw it.
            return;
        }
        if most <= maximum {
            return;
        }
        let least = worst.map_or(0.0, |(_, travel)| travel.lower);
        let common = if common_counted {
            format!(", its common path counting {shared} times")
        } else {
            String::new()
        };
        let counted = if crossed.is_empty() {
            common
        } else {
            let names: Vec<&str> = crossed
                .iter()
                .map(|kind: &usize| self.declared.sections[*kind].name.as_str())
                .collect();
            format!(
                ", counting the walk on {} up to {multiplied} times{common}",
                names.join(", ")
            )
        };
        checked.doubts.extend(doubts);
        checked.doubts.push(incomplete(format!(
            "the longest travel to {} is {} m walking{counted}, and {allows}",
            escape.goal(),
            if most.is_finite() {
                shown(least, most)
            } else {
                format!("at least {}", shown(least, least))
            }
        )));
    }

    /// The representative points of `exits`: of the sure ones, of all, and
    /// whether every one has a point.
    fn placed(&self, exits: &Reached, what: &str) -> Placed {
        let mut placed = Placed {
            sure: Vec::new(),
            all: Vec::new(),
            complete: true,
            doubts: Vec::new(),
        };
        for exit in exits.sure.iter().chain(&exits.maybe) {
            match representative_point(self.context, exit) {
                Ok((point, _)) => {
                    if exits.sure.contains(exit) {
                        placed.sure.push((exit.clone(), point.clone()));
                    }
                    placed.all.push((exit.clone(), point));
                }
                Err(unavailable) => {
                    placed.complete = false;
                    placed.doubts.push(unavailable);
                }
            }
        }
        placed
            .doubts
            .extend(exits.doubts(what).into_iter().map(incomplete));
        placed
    }

    /// The largest factor a walk of at most `reach` metres from `from` may
    /// count its metres by, and the rows of `sections` it may cross.
    ///
    /// Such a walk stays within `reach` of `from` in plan, so a section
    /// surely farther away than that is not crossed. A section that cannot
    /// be measured may be.
    fn factor(&self, from: &ObjectId, reach: f64, maximum: f64) -> (f64, BTreeSet<usize>) {
        let largest = self
            .sections
            .iter()
            .map(|section| section.factor)
            .fold(1.0_f64, f64::max);
        if !reach.is_finite() || reach * largest <= maximum {
            // Unbounded either way, or within the maximum at any factor.
            return (largest, BTreeSet::new());
        }
        if reach > maximum {
            // No factor makes this start pass: every section may count.
            return (
                largest,
                self.sections.iter().map(|section| section.kind).collect(),
            );
        }
        let mut factor = 1.0_f64;
        let mut kinds = BTreeSet::new();
        for section in &self.sections {
            if !self.farther(from, &section.object, reach) {
                factor = factor.max(section.factor);
                kinds.insert(section.kind);
            }
        }
        (factor, kinds)
    }

    /// The common path of `walk` from `start`, bounded from above: its
    /// length over the passages the walk to every other sure target may
    /// cross too, as traced (a walk that cannot be traced may share every
    /// passage). With no other route, the whole walk. `None` when the walk
    /// cannot be traced, or no passages are declared.
    fn common_length(
        &self,
        routes: &MetricRoutingServiceHandle,
        profile: MobilityProfile,
        start: &ObjectId,
        walk: &Travel,
        escape: &Escape,
    ) -> Option<f64> {
        let reached = walk.target.as_ref()?;
        let candidates = self.passages.as_ref()?;
        let all: BTreeSet<ObjectId> = candidates
            .universe
            .iter()
            .map(|object| object.id.clone())
            .collect();
        let mut common = Self::crossed(routes, walk, &all)?;
        let mut others = 0_usize;
        for other in escape
            .placed
            .sure
            .iter()
            .filter(|(exit, _)| exit != reached)
        {
            let route = self.nearest(
                routes,
                start,
                std::slice::from_ref(other),
                &escape.avoid.most(start),
                profile,
            );
            match route {
                // Proven unreachable: no route to share.
                Ok(route) if route.lower.is_infinite() => {}
                Ok(route) => {
                    others += 1;
                    if let Some(shares) = Self::crossed(routes, &route, &all) {
                        common.retain(|passage, _| shares.contains_key(passage));
                    }
                }
                Err(_) => others += 1,
            }
        }
        // A climb is longer than its trace in plan: the whole walk bounds it.
        if others == 0 || self.climbs(walk.path.as_deref()) {
            return Some(walk.upper);
        }
        let length: f64 = common
            .values()
            .map(|length| length.map_or(walk.upper, |length| length.upper_metres()))
            .sum();
        Some(length.min(walk.upper))
    }

    /// The multiplied length of `path`, a walk of at most `plain` metres,
    /// bounded from above, and the rows of `sections` it may cross; `None`
    /// when the walk cannot be traced.
    ///
    /// Each metre counts by the largest factor of the sections it lies on,
    /// so the excess over the plain length is at most the sum, over the
    /// sections, of the length on each times its factor less one. A
    /// section whose length is unknown may hold the whole walk.
    fn walked_cost(
        &self,
        routes: &MetricRoutingServiceHandle,
        path: &[MetricPoint],
        plain: f64,
    ) -> Option<(f64, BTreeSet<usize>)> {
        // A trace measures in plan, and a climb is longer than its plan
        // length: a climbing walk is bounded by the largest factor instead.
        if self.climbs(Some(path)) {
            return None;
        }
        let objects = self
            .sections
            .iter()
            .map(|section| section.object.clone())
            .collect();
        let request = PathTraceRequest::try_new(path.to_vec(), objects).ok()?;
        let trace = routes.trace_path(&request).ok()?;
        let mut cost = plain;
        let mut kinds = BTreeSet::new();
        for (object, length) in request.objects().iter().zip(trace.lengths()) {
            let over = length
                .as_ref()
                .map_or(plain, |length| length.upper_metres().min(plain));
            if over <= 0.0 {
                continue;
            }
            let mut factor = 1.0_f64;
            for section in self
                .sections
                .iter()
                .filter(|section| section.object == *object)
            {
                factor = factor.max(section.factor);
                kinds.insert(section.kind);
            }
            cost += (factor - 1.0) * over;
        }
        Some((cost, kinds))
    }

    /// The farthest point of `space` from the nearest of `targets`,
    /// keeping out of `avoided`.
    ///
    /// The routing service measures the farthest point on the plain walk
    /// only. A walk of at most `U` metres from a point of the space stays
    /// within `U` of the space in plan, so the plain answer stands for the
    /// walk around every avoided object surely farther than its upper bound
    /// from the space; any other avoided object leaves the travel unknown.
    fn farthest(
        &self,
        routes: &MetricRoutingServiceHandle,
        space: &ObjectId,
        targets: &[Target],
        avoided: &[ObjectId],
        profile: MobilityProfile,
    ) -> Result<Travel, Unavailable> {
        let plain = Self::plain_farthest(
            routes,
            space,
            targets.iter().map(|(_, point)| point.clone()).collect(),
            profile,
            self.routing()?,
        )?;
        let near: Vec<String> = avoided
            .iter()
            .filter(|object| !self.farther(space, object, plain.upper))
            .map(ToString::to_string)
            .collect();
        if near.is_empty() {
            return Ok(plain);
        }
        Err(incomplete(format!(
            "the farthest point of {space} is measured on the plain walk only, and a walk may \
             have to keep out of {}",
            near.join(", ")
        )))
    }

    /// Whether `object` lies surely farther than `reach` from `from` in
    /// plan.
    fn farther(&self, from: &ObjectId, object: &ObjectId, reach: f64) -> bool {
        reach.is_finite()
            && object != from
            && self
                .context
                .services
                .get::<ProximityServiceHandle>()
                .is_some_and(|proximity| {
                    ProximityRequest::projected(
                        from.clone(),
                        object.clone(),
                        ProximityProjection::Horizontal,
                    )
                    .and_then(|request| proximity.measure_distance(&request))
                    .is_ok_and(|distance| distance.interval_metres().0 > reach)
                })
    }

    fn plain_farthest(
        routes: &MetricRoutingServiceHandle,
        space: &ObjectId,
        targets: Vec<MetricPoint>,
        profile: MobilityProfile,
        routing: Option<&ConnectorRouting>,
    ) -> Result<Travel, Unavailable> {
        let mut request = FarthestPointRequest::try_new(space.clone(), targets, profile, TOLERANCE)
            .map_err(|error| incomplete(error.to_string()))?;
        if let Some(routing) = routing {
            request = request.with_connectors(routing.clone());
        }
        match routes.farthest_point(&request) {
            Ok(FarthestPointOutcome::Bounded(bounded)) => {
                let [x, y, _] = bounded.witness().coordinates_metres();
                Ok(Travel {
                    lower: bounded.distance().lower_metres(),
                    upper: bounded.distance().upper_metres(),
                    at: Some([x, y]),
                    // The witness is a point, not a walk: a walk from one
                    // point bounds only that point's travel.
                    path: None,
                    target: None,
                    evidence: vec![bounded.evidence().clone()],
                })
            }
            Ok(FarthestPointOutcome::Unreachable(cut_off)) => {
                let [x, y, _] = cut_off.witness().coordinates_metres();
                Ok(Travel {
                    lower: f64::INFINITY,
                    upper: f64::INFINITY,
                    at: Some([x, y]),
                    path: None,
                    target: None,
                    evidence: vec![cut_off.completeness().evidence().clone()],
                })
            }
            Err(error) => Err(incomplete(format!(
                "the farthest point of {space} from an exit: {error}"
            ))),
        }
    }

    /// The walk from `origin` (a door, or a space's representative point)
    /// to the nearest of `targets`, keeping out of `avoided`. Each walk is
    /// measured once.
    fn nearest(
        &self,
        routes: &MetricRoutingServiceHandle,
        origin: &ObjectId,
        targets: &[Target],
        avoided: &[ObjectId],
        profile: MobilityProfile,
    ) -> Result<Travel, Unavailable> {
        let mut avoided = avoided.to_vec();
        avoided.sort();
        avoided.dedup();
        let key = (
            origin.clone(),
            targets.iter().map(|(exit, _)| exit.clone()).collect(),
            avoided,
        );
        if let Some(known) = self.walks.borrow().get(&key) {
            return known.clone();
        }
        let walked = self.routing().and_then(|routing| {
            Self::walk(
                self.context,
                routes,
                origin,
                targets,
                &key.2,
                profile,
                routing,
            )
        });
        self.walks.borrow_mut().insert(key, walked.clone());
        walked
    }

    fn walk(
        context: &RuleContext<'_>,
        routes: &MetricRoutingServiceHandle,
        from: &ObjectId,
        targets: &[Target],
        avoided: &[ObjectId],
        profile: MobilityProfile,
        routing: Option<&ConnectorRouting>,
    ) -> Result<Travel, Unavailable> {
        let (origin, cited) = representative_point(context, from)?;
        let points = targets.iter().map(|(_, point)| point.clone()).collect();
        let mut request = NearestTargetRequest::try_new(origin, points, profile)
            .map_err(|error| incomplete(error.to_string()))?
            .with_avoided(avoided.to_vec());
        if let Some(routing) = routing {
            request = request.with_connectors(routing.clone());
        }
        match routes.nearest_target(&request) {
            Ok(NearestTargetOutcome::Reached(reached)) => {
                let mut evidence = cited;
                evidence.push(reached.evidence().clone());
                Ok(Travel {
                    lower: reached.shortest_distance().lower_metres(),
                    upper: reached.shortest_distance().upper_metres(),
                    at: None,
                    path: Some(reached.waypoints().to_vec()),
                    target: targets.get(reached.target()).map(|(exit, _)| exit.clone()),
                    evidence,
                })
            }
            Ok(NearestTargetOutcome::Unreachable(unreachable)) => {
                let mut evidence = cited;
                evidence.push(unreachable.completeness().evidence().clone());
                Ok(Travel {
                    lower: f64::INFINITY,
                    upper: f64::INFINITY,
                    at: None,
                    path: None,
                    target: None,
                    evidence,
                })
            }
            Err(error) => Err(incomplete(if avoided.is_empty() {
                format!("walking from {from} to an exit: {error}")
            } else {
                let names: Vec<String> = avoided.iter().map(ToString::to_string).collect();
                format!(
                    "walking from {from} to an exit around {}: {error}",
                    names.join(", ")
                )
            })),
        }
    }
}
