//! What `opening-zone` checks of an opening, as the measured list
//! `zone_checks`: each placement in a host its path reaches, placed and
//! measured as the capability placed and measured it, and what it checks
//! there, in the capability's order, host by host (`check` names each):
//!
//! - `placement`: whether the opening is placed (undecided, with the
//!   reason, where its host or body cannot be read or it may be below the
//!   minimum area), whether it lies inside the host and its outline, its
//!   clear distances from the host's ends and edges (or flanges) and its
//!   distances from the bottom and top edges. A distance measured to a
//!   free outline from the box the opening may lie in is known only from
//!   below: up to infinity.
//! - `zones`: against `zones`, none missed, or the side the opening misses
//!   most of the zone it misses least, or undecided where it may lie in a
//!   zone.
//! - `dimension`: each row of `dimensions` whose `source` selects the
//!   opening, with the row's bounds and its distance.
//! - `support`: each requirement on the host's supports
//!   (`support_distance`, then `support_clearance`), whether a support
//!   surely misses it.
//! - `spacing`: the least clear distance to another opening of the host
//!   surely closer than `opening_spacing`, or why one may be.
//!
//! The order is the capability's, so an opening left open for several
//! reasons is left open for the first as the capability left it.
//!
//! Every opening the rule may select is placed once per rule, so each
//! opening's neighbours are the capability's; the checks of each opening
//! the rule selects are measured when its list is read, so only one
//! opening's checks are held at a time.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, CompiledRule, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement,
    MemberValue, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use super::dimensions::{Applied, Selections};
use super::supports::{Decided, Opening, Supports, SupportsRead};
use super::{Bodies, Clearance, Config, Host, Judge, Placed, Placement, ROUNDING, Span, list};
use crate::counts::Population;
use crate::level_spacing::metres;
use crate::measured_kinds::{population, resolution_error, stated_rule};
use crate::support::Unavailable;

const ZONE_CHECKS: &str = "zone_checks";

/// Measures what `opening-zone` checks.
pub(crate) struct ZoneMeasures;

/// The checks of an opening, or why it cannot be placed.
type Checks = Result<Vec<MeasuredMember>, Unavailable>;

/// What a rule's openings share, measured once per rule: the declaration,
/// the populations, every host's body and every opening's placement (each
/// opening's neighbours are among them), the dimensioning table's
/// selections and the supports read so far. Each opening's checks are
/// measured from it when its list is read, so neither time nor heap grows
/// with the checks of the rule's other openings.
struct Run {
    rule: CompiledRule,
    openings: Arc<Population>,
    hosts: Arc<Population>,
    supported: Option<Arc<Population>>,
    bodies: Bodies,
    placed: BTreeMap<ObjectId, Placement>,
    dimensions: Selections,
    supports: SupportsRead,
}

#[derive(Hash, PartialEq, Eq)]
struct RunKey(ArgumentsKey);

/// What the rule's openings share, measured once per rule and kept for it
/// alone.
fn run(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Arc<Run>, Unavailable> {
    MeasuredMemo::of_rule(context.services, RunKey(ArgumentsKey::of(call)), || {
        measure(call, context).map(Arc::new)
    })
}

/// Places every opening the rule may select, so spacing sees the
/// undecided ones too.
fn measure(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Run, Unavailable> {
    let rule = stated_rule(call, &["minimum_opening_area"]);
    let openings = population(call, "openings", context)?;
    let hosts = population(call, "host_selector", context)?;
    let (supported, dimensions, bodies, placed) = {
        let config = Config::parse(&rule)?;
        let supported = match config.supports {
            Some(_) => Some(population(call, "support_selector", context)?),
            None => None,
        };
        let dimensions = Selections::of(context, &config.dimensions);
        let mut judge = Judge {
            dimensions: &dimensions,
            context,
            #[cfg(feature = "parity-reference")]
            rule: &rule,
            config: &config,
            hosts: &hosts,
            bodies: Cow::Owned(BTreeMap::new()),
            placed: Cow::Owned(BTreeMap::new()),
            supports: None,
        };
        for opening in context.project.objects() {
            if openings.contains(&opening.id) {
                let placed = judge.place(opening);
                judge.placed.to_mut().insert(opening.id.clone(), placed);
            }
        }
        let (bodies, placed) = (judge.bodies.into_owned(), judge.placed.into_owned());
        (supported, dimensions, bodies, placed)
    };
    Ok(Run {
        rule,
        openings,
        hosts,
        supported,
        bodies,
        placed,
        dimensions,
        supports: SupportsRead::default(),
    })
}

/// The checks of `opening`, one the rule selects, measured from what the
/// rule's openings share.
fn checks(run: &Run, opening: &Object, context: &RuleContext<'_>) -> Checks {
    let config = Config::parse(&run.rule)?;
    let judge =
        Judge {
            dimensions: &run.dimensions,
            context,
            #[cfg(feature = "parity-reference")]
            rule: &run.rule,
            config: &config,
            hosts: &run.hosts,
            bodies: Cow::Borrowed(&run.bodies),
            placed: Cow::Borrowed(&run.placed),
            supports: run.supported.as_deref().zip(config.supports.as_ref()).map(
                |(population, config)| Supports::new(context, config, population, &run.supports),
            ),
        };
    judge.checks(opening, &run.openings)
}

fn absent(locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Absent {
        locator: locator.to_owned(),
    })
}

fn text(text: impl Into<String>) -> MemberValue {
    MemberValue::Text { text: text.into() }
}

fn truth(value: bool, locator: &str) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: locator.to_owned(),
    }
}

fn undecided(why: impl Into<String>) -> MemberValue {
    MemberValue::Undecided { why: why.into() }
}

/// A length known within `[lower, upper]`.
fn length((lower, upper): (f64, f64), locator: &str) -> MemberValue {
    MemberValue::Measured(Measurement::Value {
        lower,
        upper,
        dimension: Some(QuantityDimension::Length),
        locator: locator.to_owned(),
    })
}

/// A distance exact, or known only from below.
fn distance(clear: f64, exact: bool, locator: &str) -> MemberValue {
    length((clear, if exact { clear } else { f64::INFINITY }), locator)
}

/// A reason as a report writes it.
fn reason(reason: &NotEvaluatedReason) -> MemberValue {
    text(match serde_json::to_value(reason) {
        Ok(serde_json::Value::String(reason)) => reason,
        _ => String::new(),
    })
}

/// How a field an item leaves out is stated.
#[derive(Clone, Copy)]
enum Kind {
    /// Absent: a number or truth not measured.
    Number,
    /// No words.
    Text,
    /// No objects.
    Objects,
}

/// Every field of an item, and how one it leaves out is stated.
const FIELDS: &[(&str, Kind)] = &[
    ("check", Kind::Text),
    ("placed", Kind::Number),
    ("host", Kind::Text),
    ("inside", Kind::Number),
    ("outside", Kind::Text),
    ("end", Kind::Number),
    ("end_shown", Kind::Text),
    ("edge", Kind::Number),
    ("edge_words", Kind::Text),
    ("bottom", Kind::Number),
    ("bottom_shown", Kind::Text),
    ("bottom_name", Kind::Text),
    ("top", Kind::Number),
    ("top_shown", Kind::Text),
    ("top_name", Kind::Text),
    ("far_open", Kind::Number),
    ("zone", Kind::Number),
    ("needed", Kind::Number),
    ("distance", Kind::Number),
    ("minimum", Kind::Number),
    ("maximum", Kind::Number),
    ("slack", Kind::Number),
    ("words", Kind::Text),
    ("required", Kind::Text),
    ("label", Kind::Text),
    ("open", Kind::Text),
    ("fails", Kind::Number),
    ("message", Kind::Text),
    ("spacing", Kind::Number),
    ("related", Kind::Objects),
    ("reason", Kind::Text),
];

/// An item checking `check` of `fields`, every field it leaves out stated
/// absent, empty or none.
fn item(
    check: &str,
    mut fields: BTreeMap<&'static str, MemberValue>,
    evidence: Vec<Evidence>,
) -> MeasuredMember {
    fields.insert("check", text(check));
    for (name, kind) in FIELDS {
        fields.entry(name).or_insert_with(|| match kind {
            Kind::Number => absent(""),
            Kind::Text => text(""),
            Kind::Objects => MemberValue::Objects {
                objects: Vec::new(),
            },
        });
    }
    MeasuredMember {
        certain: true,
        exact: evidence.iter().all(|evidence| evidence.exact),
        fields,
        evidence,
    }
}

fn related(objects: Vec<ObjectId>) -> MemberValue {
    MemberValue::Objects { objects }
}

/// What a placement's findings relate and cite: its host and placement,
/// and with them the openings it is surely too close to, as the capability
/// related every finding of the placement to them.
struct Cited {
    related: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

impl Judge<'_, '_> {
    /// The checks of one opening; an error where its hosts cannot be told.
    fn checks(
        &self,
        opening: &Object,
        openings: &Population,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let placements = match self.placed.get(&opening.id) {
            Some(Ok(placements)) => placements,
            Some(Err(error)) => return Err(error.clone()),
            None => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    "the opening was not placed".to_owned(),
                ));
            }
        };
        let mut checks = Vec::new();
        for placement in placements {
            match placement {
                Err((why, message)) => checks.push(item(
                    "placement",
                    BTreeMap::from([
                        ("placed", undecided(message.clone())),
                        ("reason", reason(why)),
                    ]),
                    Vec::new(),
                )),
                Ok(placed) if placed.may_be_small => checks.push(item(
                    "placement",
                    BTreeMap::from([(
                        "placed",
                        undecided(format!(
                            "whether its area in its host {} is below `minimum_opening_area` \
                             is undecided: it is not extruded through the host",
                            placed.host.local_id
                        )),
                    )]),
                    Vec::new(),
                )),
                Ok(placed) => self.check_in(opening, placed, openings, &mut checks),
            }
        }
        Ok(checks)
    }

    /// The checks of an opening placed in one of its hosts.
    fn check_in(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        checks: &mut Vec<MeasuredMember>,
    ) {
        // Placing the opening read its host.
        let Some(Ok(host)) = self.bodies.get(&placed.host).cloned() else {
            return;
        };
        let locator = format!("{ZONE_CHECKS}:{}:{}", opening.id, placed.host);
        let rect = placed.section_rect(&host, self.config.axes);
        let (outside_length, outside_height) = self.beyond(&host, placed);
        // The openings it is surely too close to, which every finding of the
        // placement relates.
        let (spacing, cited) = self.spacing_check(opening, placed, openings, &locator);
        let (fields, crosses) = self.placement_check(placed, &host, rect, &locator);
        let mut placement = fields;
        placement.insert("related", related(cited.related.clone()));
        checks.push(item("placement", placement, cited.evidence.clone()));
        if !outside_length && !outside_height && !crosses {
            if let Some(zones) = self.zone_check(placed, &host, rect, &cited, &locator) {
                checks.push(zones);
            }
            self.dimension_checks(opening, placed, &host, rect, &locator, checks);
        }
        self.support_checks(placed, &host, &locator, checks);
        if let Some(mut spacing) = spacing {
            spacing.insert("related", related(cited.related.clone()));
            checks.push(item("spacing", spacing, cited.evidence));
        }
    }

    /// Whether the opening is inside its host, and its distances from the
    /// host's ends and edges; and whether it crosses the outline (or may),
    /// which nothing else is then measured past.
    fn placement_check(
        &self,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        locator: &str,
    ) -> (BTreeMap<&'static str, MemberValue>, bool) {
        let axes = self.config.axes;
        let (_, length_bounds) = host.axis(axes.length);
        let (_, height_bounds) = host.axis(axes.height);
        let (outside_length, outside_height) = self.beyond(host, placed);
        let outside: Vec<String> = [
            ("length", outside_length, placed.length, length_bounds),
            ("height", outside_height, placed.height, height_bounds),
        ]
        .into_iter()
        .filter(|(_, out, _, _)| *out)
        .map(|(name, _, extent, bounds)| {
            format!(
                "along its {name} it spans {} to {}, the host {} to {}",
                metres(extent.0),
                metres(extent.1),
                metres(bounds.0),
                metres(bounds.1)
            )
        })
        .collect();
        let host_name = placed.host.local_id.as_str();
        let mut fields =
            BTreeMap::from([("placed", truth(true, locator)), ("host", text(host_name))]);
        let crossing = if outside.is_empty() {
            Self::crossing(host, placed, rect)
        } else {
            Some(false)
        };
        let crosses = outside.is_empty() && crossing != Some(false);
        let inside = if outside.is_empty() {
            match crossing {
                Some(false) => truth(true, locator),
                Some(true) => {
                    fields.insert("outside", text("it crosses the edge of the host's outline"));
                    truth(false, locator)
                }
                None => undecided(format!(
                    "it may cross the edge of its host {host_name}'s outline: it is not \
                     extruded straight through the host, so where it lies in the host's \
                     section is known only within bounds"
                )),
            }
        } else {
            fields.insert("outside", text(outside.join("; ")));
            truth(false, locator)
        };
        fields.insert("inside", inside);
        if !outside_length
            && !crosses
            && let Some((clear, exact)) = self.end_clearance(host, placed, rect)
        {
            fields.insert("end", distance(clear, exact, locator));
            fields.insert("end_shown", text(metres(clear.max(0.0))));
        }
        if !outside_height && !crosses {
            match self.edge_clearance(host, placed, rect) {
                Ok(Some(Clearance {
                    clear,
                    exact,
                    outline,
                    what,
                })) => {
                    fields.insert("edge", distance(clear, exact, locator));
                    let words = if outline {
                        format!("is {} from an edge", metres(clear.max(0.0)))
                    } else if clear < 0.0 {
                        format!("reaches {} into {what}", metres(-clear))
                    } else {
                        format!("is {} from {what}", metres(clear))
                    };
                    fields.insert("edge_words", text(words));
                }
                Ok(None) => {}
                Err((_, why)) => {
                    fields.insert("edge", undecided(why));
                }
            }
            self.far(placed, host, rect, locator, &mut fields);
        }
        (fields, crosses)
    }

    /// The distances from the host's bottom and top edges (or flanges) the
    /// rule bounds, and where one known only from below may exceed the
    /// bound, that both are open in one outcome.
    fn far(
        &self,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        locator: &str,
        fields: &mut BTreeMap<&'static str, MemberValue>,
    ) {
        let Some((maximum, low, high)) = self.config.edge_maximum else {
            return;
        };
        fields.insert("far_open", truth(false, locator));
        let ((bottom, top), exact) = match self.far_clearance(host, placed, rect) {
            Ok(Some(far)) => far,
            Ok(None) => return,
            Err((_, why)) => {
                fields.insert("far_open", undecided(why));
                return;
            }
        };
        let names = if self.config.web {
            ["the lower flange", "the upper flange"]
        } else {
            ["the bottom edge", "the top edge"]
        };
        let mut open = Vec::new();
        for (checked, clear, name, [value, shown, named]) in [
            (
                low,
                bottom,
                names[0],
                ["bottom", "bottom_shown", "bottom_name"],
            ),
            (high, top, names[1], ["top", "top_shown", "top_name"]),
        ] {
            if !checked {
                continue;
            }
            fields.insert(value, distance(clear, exact, locator));
            fields.insert(shown, text(metres(clear)));
            fields.insert(named, text(name));
            if !exact && clear <= maximum + ROUNDING {
                open.push(name);
            }
        }
        if !open.is_empty() {
            fields.insert(
                "far_open",
                undecided(format!(
                    "its distance from {} of its host {}'s outline may exceed {}: where it lies \
                     in the host's section is known only within bounds",
                    open.join(" and "),
                    placed.host.local_id,
                    metres(maximum)
                )),
            );
        }
    }

    /// The least clear distance to an opening of the host surely too
    /// close, or why one may be; and what every finding of the placement
    /// then relates and cites.
    fn spacing_check(
        &self,
        opening: &Object,
        placed: &Placed,
        openings: &Population,
        locator: &str,
    ) -> (Option<BTreeMap<&'static str, MemberValue>>, Cited) {
        let mut cited = Cited {
            related: Vec::new(),
            evidence: placed.evidence.clone(),
        };
        let mut fields = None;
        if let Some(required) = self.config.spacing {
            let spacing = self.spacing_of(opening, placed, openings, required);
            let host = ("host", text(placed.host.local_id.as_str()));
            if !spacing.sure.is_empty() {
                let nearest = spacing.nearest();
                fields = Some(BTreeMap::from([
                    host,
                    ("spacing", length((nearest, nearest), locator)),
                ]));
                for (other, _, neighbour) in spacing.sure {
                    cited.evidence.extend(neighbour.evidence.iter().cloned());
                    cited.related.push(other);
                }
            } else if !spacing.unknown.is_empty() {
                fields = Some(BTreeMap::from([
                    host,
                    (
                        "spacing",
                        undecided(format!(
                            "its clear distance to {} may be under {}: their outlines or hosts \
                             are not known exactly",
                            list(&spacing.unknown),
                            metres(required)
                        )),
                    ),
                ]));
            }
        }
        cited.related.push(placed.host.clone());
        (fields, cited)
    }

    /// Where the opening lies against the allowed zones.
    fn zone_check(
        &self,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        cited: &Cited,
        locator: &str,
    ) -> Option<MeasuredMember> {
        if self.config.zones.is_empty() {
            return None;
        }
        let mut fields = BTreeMap::from([("related", related(cited.related.clone()))]);
        match self.zone_miss(host, placed, rect) {
            Ok(None) => {}
            Ok(Some(miss)) => {
                fields.insert("zone", length((miss.clear, miss.clear), locator));
                fields.insert("needed", length((miss.needed, miss.needed), locator));
                fields.insert("words", text(miss.message));
            }
            Err((_, why)) => {
                fields.insert("zone", undecided(why));
            }
        }
        Some(item("zones", fields, cited.evidence.clone()))
    }

    /// The rows of the dimensioning table applying to the opening.
    fn dimension_checks(
        &self,
        opening: &Object,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        locator: &str,
        checks: &mut Vec<MeasuredMember>,
    ) {
        for applied in self.applied(opening, placed, host, rect) {
            let mut objects = Vec::new();
            let mut evidence = placed.evidence.clone();
            let mut fields = BTreeMap::new();
            match applied {
                Applied::Undecided(why) | Applied::Measured(_, Err((_, why))) => {
                    fields.insert("distance", undecided(why));
                }
                Applied::Measured(index, Ok(measure)) => {
                    let dimension = &self.config.dimensions[index];
                    let (minimum, maximum) = dimension.bound.limits();
                    fields.insert("distance", length(measure.distance, locator));
                    for (name, bound) in [("minimum", minimum), ("maximum", maximum)] {
                        if let Some(bound) = bound {
                            fields.insert(name, length((bound, bound), locator));
                        }
                    }
                    let slack = dimension.tolerance + ROUNDING;
                    fields.insert("slack", length((slack, slack), locator));
                    fields.insert("words", text(measure.words));
                    fields.insert(
                        "required",
                        text(dimension.bound.required(dimension.tolerance)),
                    );
                    fields.insert("label", text(dimension.label.clone()));
                    fields.insert("open", text(measure.open));
                    if let Some((target, neighbour)) = measure.target {
                        evidence.extend(neighbour.evidence.iter().cloned());
                        objects.push(target.clone());
                    }
                }
            }
            objects.push(placed.host.clone());
            fields.insert("related", related(objects));
            checks.push(item("dimension", fields, evidence));
        }
    }

    /// How the opening meets each requirement on its host's supports.
    fn support_checks(
        &self,
        placed: &Placed,
        host: &Host,
        locator: &str,
        checks: &mut Vec<MeasuredMember>,
    ) {
        let Some(supports) = &self.supports else {
            return;
        };
        let face = Opening {
            host: &placed.host,
            length: placed.length,
            height: placed.height,
            exact: placed.exact,
        };
        for decided in supports.judge(&face, host, self.config.axes) {
            let mut objects = Vec::new();
            let mut evidence = Vec::new();
            let mut fields = BTreeMap::new();
            match decided {
                Decided::Unfound((why, message)) => {
                    fields.insert("fails", undecided(message));
                    fields.insert("reason", reason(&why));
                }
                Decided::Pass => {
                    fields.insert("fails", truth(false, locator));
                }
                Decided::Open(why) => {
                    fields.insert("fails", undecided(why));
                }
                Decided::Finding((message, cited, members)) => {
                    fields.insert("fails", truth(true, locator));
                    fields.insert("message", text(message));
                    evidence.clone_from(&placed.evidence);
                    evidence.extend(cited);
                    objects = members;
                }
            }
            objects.push(placed.host.clone());
            fields.insert("related", related(objects));
            checks.push(item("support", fields, evidence));
        }
    }
}

impl MeasuredProvider for ZoneMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[ZONE_CHECKS]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    /// An opening whose hosts cannot be told refuses the list, as the
    /// capability left it open.
    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let refused = |(reason, why): Unavailable| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        };
        let run = run(call, context).map_err(refused)?;
        // Only the openings the rule selects are checked.
        if !run.openings.matched.contains(object) {
            return Ok(Vec::new());
        }
        let Some(opening) = context.project.object(object) else {
            return Ok(Vec::new());
        };
        checks(&run, opening, context).map_err(refused)
    }
}
