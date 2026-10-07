//! Allowed zones: rows of insets from a host's ends and edges, each the
//! larger of a fraction of the host's span or depth and a minimum length.
//! An opening must lie within one of them.

use axioval_engine::{ColumnKind, Deviation, NotEvaluatedReason, TableColumn};
use axioval_ir::QuantityDimension;

use super::face::{Host, ROUNDING, Span};
use super::{Judge, Placed};
use crate::level_spacing::metres;
use crate::support::table::Row;
use crate::support::{Parameters, Unavailable, invalid, si_quantity};

/// The columns of `zones`: a name, and per side (`end`, `top`, `bottom`) a
/// fraction, what it is a fraction of, and a minimum length.
pub(super) const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("name", ColumnKind::String),
    TableColumn::optional("end_fraction", ColumnKind::Number),
    TableColumn::optional("end_of", ColumnKind::String),
    TableColumn::optional("end_minimum", ColumnKind::Quantity),
    TableColumn::optional("top_fraction", ColumnKind::Number),
    TableColumn::optional("top_of", ColumnKind::String),
    TableColumn::optional("top_minimum", ColumnKind::Quantity),
    TableColumn::optional("bottom_fraction", ColumnKind::Number),
    TableColumn::optional("bottom_of", ColumnKind::String),
    TableColumn::optional("bottom_minimum", ColumnKind::Quantity),
];

/// What an inset's fraction is taken of.
#[derive(Clone, Copy)]
enum Reference {
    /// The host's length along `length_axis`.
    Span,
    /// The host's height along `height_axis`.
    Depth,
}

/// How far a zone keeps from one side: the larger of `fraction` of the
/// reference and `minimum`.
#[derive(Clone, Copy)]
struct Inset {
    fraction: f64,
    of: Reference,
    minimum: f64,
}

impl Inset {
    fn length(self, span: f64, depth: f64) -> f64 {
        let reference = match self.of {
            Reference::Span => span,
            Reference::Depth => depth,
        };
        (self.fraction * reference).max(self.minimum)
    }
}

/// One allowed zone.
pub(super) struct Zone {
    label: String,
    /// From both ends, the bottom and the top.
    end: Inset,
    bottom: Inset,
    top: Inset,
}

/// Reads `zones`; none when it is absent.
pub(super) fn parse(parameters: &Parameters<'_>) -> Result<Vec<Zone>, Unavailable> {
    let Some(rows) = parameters.table("zones")? else {
        return Ok(Vec::new());
    };
    if rows.is_empty() {
        return Err(invalid(
            "`zones` has no rows; leave it out to allow the whole host",
        ));
    }
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let number = index + 1;
            Ok(Zone {
                label: match row.text("name")? {
                    Some(name) if !name.trim().is_empty() => format!("zone `{}`", name.trim()),
                    _ => format!("zone {number}"),
                },
                end: inset(*row, number, "end", Reference::Span)?,
                bottom: inset(*row, number, "bottom", Reference::Depth)?,
                top: inset(*row, number, "top", Reference::Depth)?,
            })
        })
        .collect()
}

fn inset(
    row: Row<'_>,
    number: usize,
    side: &str,
    default: Reference,
) -> Result<Inset, Unavailable> {
    let fraction = match row.number(&format!("{side}_fraction"))? {
        None => 0.0,
        Some(fraction) if fraction >= 0.0 => fraction,
        Some(_) => {
            return Err(invalid(format!(
                "`zones` row {number}'s `{side}_fraction` is negative"
            )));
        }
    };
    let of = match row.text(&format!("{side}_of"))? {
        None => default,
        Some("span") => Reference::Span,
        Some("depth") => Reference::Depth,
        Some(other) => {
            return Err(invalid(format!(
                "`zones` row {number}'s `{side}_of` `{other}` is unsupported; use `span` or \
                 `depth`"
            )));
        }
    };
    let minimum = match row.quantity(&format!("{side}_minimum"))? {
        None => 0.0,
        Some((value, unit)) => match si_quantity(value, unit)? {
            (value, QuantityDimension::Length) if value >= 0.0 => value,
            _ => {
                return Err(invalid(format!(
                    "`zones` row {number}'s `{side}_minimum` is not a non-negative length"
                )));
            }
        },
    };
    Ok(Inset {
        fraction,
        of,
        minimum,
    })
}

/// A side's clear distance from an opening: the distance (negative when
/// the opening reaches into a flange), whether it is exact or only a lower
/// bound, and the side's name.
struct Side {
    clear: f64,
    exact: bool,
    name: &'static str,
}

/// A side an opening is too near: its index in the sides, the clear
/// distance and the inset needed.
type Miss = (usize, f64, f64);

/// How one zone holds an opening.
enum Held {
    Inside,
    /// Surely outside, too near each of these sides.
    Outside(Vec<Miss>, Deviation),
    /// Its distance from a side is known only as a lower bound under the
    /// inset.
    Undecided,
}

impl Zone {
    /// Where the opening lies against this zone.
    fn holds(&self, sides: &[Side; 4], span: f64, depth: f64) -> Held {
        let insets = [self.end, self.end, self.bottom, self.top];
        let mut misses = Vec::new();
        let mut open = false;
        for (index, (side, inset)) in sides.iter().zip(insets).enumerate() {
            let needed = inset.length(span, depth);
            if side.clear >= needed - ROUNDING {
                continue;
            }
            if side.exact {
                misses.push((index, side.clear, needed));
            } else {
                open = true;
            }
        }
        let deviation = misses
            .iter()
            .map(|(_, clear, needed)| Deviation::below(*needed, *clear, *clear))
            .reduce(Deviation::worst);
        match deviation {
            Some(deviation) => Held::Outside(misses, deviation),
            None if open => Held::Undecided,
            None => Held::Inside,
        }
    }
}

fn describe(clear: f64, side: &str) -> String {
    if clear < 0.0 {
        format!("reaches {} into {side}", metres(-clear))
    } else {
        format!("is {} from {side}", metres(clear))
    }
}

impl Judge<'_, '_> {
    /// The opening's clear distances from both ends, the bottom and the
    /// top (or the flanges, with `zone` `web`); `None` where the host's
    /// outline passes through it, which is found apart.
    fn sides(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
    ) -> Result<Option<[Side; 4]>, Unavailable> {
        let axes = self.config.axes;
        let Some(ends) = host.clearance(axes.length, placed.length, rect) else {
            return Ok(None);
        };
        let ends_exact = placed.section_exact || !host.outlined(axes.length);
        let (edges, edges_exact, names) = if self.config.web {
            let web = host.web.ok_or_else(|| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "its host {}'s `{}` profile has no web between flanges",
                        placed.host.local_id, host.family
                    ),
                )
            })?;
            (
                (placed.height.0 - web.0, web.1 - placed.height.1),
                true,
                ["the lower flange", "the upper flange"],
            )
        } else {
            let Some(edges) = host.clearance(axes.height, placed.height, rect) else {
                return Ok(None);
            };
            (
                edges,
                placed.section_exact || !host.outlined(axes.height),
                ["the bottom edge", "the top edge"],
            )
        };
        let side = |clear, exact, name| Side { clear, exact, name };
        Ok(Some([
            side(ends.0, ends_exact, "an end"),
            side(ends.1, ends_exact, "an end"),
            side(edges.0, edges_exact, names[0]),
            side(edges.1, edges_exact, names[1]),
        ]))
    }

    /// Where the opening lies against the allowed zones: inside one is
    /// none, surely outside every one the miss of the zone it misses least,
    /// and an error where it may lie in one.
    pub(super) fn zone_miss(
        &self,
        host: &Host,
        placed: &Placed,
        rect: [Span; 2],
    ) -> Result<Option<ZoneMiss>, Unavailable> {
        let zones = &self.config.zones;
        if zones.is_empty() {
            return Ok(None);
        }
        let Some(sides) = self.sides(host, placed, rect)? else {
            return Ok(None);
        };
        let (_, length_bounds) = host.axis(self.config.axes.length);
        let (_, height_bounds) = host.axis(self.config.axes.height);
        let (span, depth) = (
            length_bounds.1 - length_bounds.0,
            height_bounds.1 - height_bounds.0,
        );
        let mut nearest: Option<(&Zone, Vec<Miss>, Deviation)> = None;
        let mut undecided = Vec::new();
        for zone in zones {
            match zone.holds(&sides, span, depth) {
                Held::Inside => return Ok(None),
                Held::Undecided => undecided.push(zone.label.as_str()),
                Held::Outside(misses, deviation) => {
                    nearest = Some(match nearest {
                        Some((least, kept, graded)) if graded.lower() <= deviation.lower() => {
                            (least, kept, graded.least(deviation))
                        }
                        Some((_, _, graded)) => (zone, misses, graded.least(deviation)),
                        None => (zone, misses, deviation),
                    });
                }
            }
        }
        if !undecided.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it may lie in {} of its host {}: where it lies in the host's section is \
                     known only within bounds",
                    undecided.join(" or "),
                    placed.host.local_id
                ),
            ));
        }
        let Some((zone, misses, deviation)) = nearest else {
            return Ok(None);
        };
        // The side it misses most decides how far it misses the zone.
        let below = |(clear, needed): (f64, f64)| Deviation::below(needed, clear, clear).lower();
        let (clear, needed) = misses
            .iter()
            .map(|(_, clear, needed)| (*clear, *needed))
            .reduce(|most, miss| {
                if below(miss) > below(most) {
                    miss
                } else {
                    most
                }
            })
            .expect("a zone missed is missed at a side");
        let described = misses
            .iter()
            .map(|(index, clear, needed)| {
                format!(
                    "{} ({} required)",
                    describe(*clear, sides[*index].name),
                    metres(*needed)
                )
            })
            .collect::<Vec<_>>()
            .join(" and ");
        let within = if zones.len() == 1 {
            format!("its host {}'s allowed zone", placed.host.local_id)
        } else {
            format!(
                "any of the {} allowed zones of its host {}",
                zones.len(),
                placed.host.local_id
            )
        };
        Ok(Some(ZoneMiss {
            message: format!(
                "opening lies outside {within}: nearest is {}, where it {described}",
                zone.label
            ),
            clear,
            needed,
            deviation,
        }))
    }
}

/// An opening surely outside every allowed zone: the finding's message,
/// the clear distance and inset of the side it misses most in the zone it
/// misses least, and how far it misses that zone.
pub(super) struct ZoneMiss {
    pub(super) message: String,
    pub(super) clear: f64,
    pub(super) needed: f64,
    #[cfg_attr(not(feature = "parity-reference"), allow(dead_code))]
    pub(super) deviation: Deviation,
}
