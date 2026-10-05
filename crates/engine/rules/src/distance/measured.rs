//! Distances as measured values: the nearest or farthest counterpart's
//! distance, and how many counterparts lie within a radius, measured
//! exactly as `distance` measures them for its verdicts.
//!
//! The counterparts are the objects of the source kinds `to` names. The
//! search reuses the broad phase within `within` (or the `radius`), so a
//! counterpart it does not propose is proven beyond it. An undecided
//! counterpart, one whose distance could not be read or whose interval
//! straddles, widens the value towards the bound it could move, as
//! `distance` counts it as unknown.

use std::collections::BTreeMap;

use axioval_engine::{
    CompiledRule, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{COUNT_WITHIN, DISTANCE, MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, RuleId};

use super::{Candidate, Scope, candidates, declaration};
use crate::pairs::prepare;

/// Measures `distance` and `count_within`.
pub struct DistanceMeasures;

/// The distance declaration `call` stands for, around one subject.
fn rule(
    call: &MeasuredCall,
    object: &ObjectId,
    counterparts: std::collections::BTreeSet<ObjectId>,
    reach: f64,
) -> CompiledRule {
    let text = |value: &str| ParameterValue::String {
        value: value.to_owned(),
    };
    let mut parameters = BTreeMap::from([
        (
            "counterparts".to_owned(),
            ParameterValue::Selector {
                value: Box::new(Selector::Objects {
                    objects: counterparts,
                }),
            },
        ),
        (
            "maximum_metres".to_owned(),
            ParameterValue::Number { value: reach },
        ),
    ]);
    for (key, parameter) in [
        ("projection", "projection"),
        ("direction", "vertical_direction"),
        ("subject_surface", "subject_surface"),
        ("counterpart_surface", "counterpart_surface"),
    ] {
        if let Some(choice) = call.choice(key) {
            parameters.insert(parameter.to_owned(), text(choice));
        }
    }
    CompiledRule {
        id: RuleId::new("axioval-measured-distance").expect("a valid rule id"),
        capability: "axioval:capability.distance".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: std::collections::BTreeSet::from([object.clone()]),
        },
        parameters,
    }
}

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

/// The interval a candidate's distance surely lies in: unknown when it
/// could not be measured. An infinite lower bound is no relation in the
/// projection at all.
fn interval(candidate: &Candidate) -> (f64, f64) {
    candidate.interval().unwrap_or((0.0, f64::INFINITY))
}

/// The nearest (or farthest) counterpart's distance from what is known of
/// every candidate.
fn extreme(all: &[&Candidate], farthest: bool) -> Result<Measurement, String> {
    let related: Vec<&&Candidate> = all
        .iter()
        .filter(|candidate| interval(candidate).0.is_finite())
        .collect();
    let possible: Vec<(f64, f64)> = related
        .iter()
        .filter(|candidate| candidate.possibly_in_scope())
        .map(|candidate| interval(candidate))
        .collect();
    let certain: Vec<(f64, f64)> = related
        .iter()
        .filter(|candidate| candidate.certainly_in_scope() && candidate.interval().is_some())
        .map(|candidate| interval(candidate))
        .collect();
    if possible.is_empty() {
        return Ok(Measurement::Absent {
            locator: "no counterpart within the search".into(),
        });
    }
    let (lower, upper) = if farthest {
        (
            certain
                .iter()
                .map(|(lower, _)| *lower)
                .fold(f64::NEG_INFINITY, f64::max),
            possible
                .iter()
                .map(|(_, upper)| *upper)
                .fold(f64::NEG_INFINITY, f64::max),
        )
    } else {
        (
            possible
                .iter()
                .map(|(lower, _)| *lower)
                .fold(f64::INFINITY, f64::min),
            certain
                .iter()
                .map(|(_, upper)| *upper)
                .fold(f64::INFINITY, f64::min),
        )
    };
    if !(lower.is_finite() && upper.is_finite()) {
        return Err(format!(
            "the {} counterpart is undecided: one may or may not count",
            if farthest { "farthest" } else { "nearest" }
        ));
    }
    Ok(Measurement::Cited {
        lower: lower.max(0.0),
        upper,
        dimension: Some(QuantityDimension::Length),
        locator: String::new(),
        // Exact as the counterparts that may be the nearest (or farthest)
        // were measured; one surely beyond it decides nothing.
        exact: measured_exactly(
            related
                .iter()
                .map(|candidate| **candidate)
                .filter(|candidate| {
                    let (low, high) = interval(candidate);
                    if farthest {
                        high >= lower
                    } else {
                        low <= upper
                    }
                }),
        ),
    })
}

/// Whether every possible counterpart measured was measured exactly, as
/// `distance` cites its verdicts: an unmeasurable counterpart widens the
/// value without evidence of its own.
fn measured_exactly<'a>(candidates: impl Iterator<Item = &'a Candidate>) -> bool {
    candidates
        .filter(|candidate| candidate.possibly_in_scope())
        .all(|candidate| {
            candidate.measured.as_ref().map_or(true, |measured| {
                measured.evidence.iter().all(|evidence| evidence.exact)
            })
        })
}

/// How many counterparts surely and possibly lie within `[from, radius]`.
/// Cited as exact where every possible one measured was measured exactly,
/// as `distance` cites a shortfall: an unmeasurable counterpart widens the
/// count without evidence of its own.
fn count(all: &[&Candidate], from: f64, radius: f64) -> Measurement {
    let mut sure = 0_u32;
    let mut possible = 0_u32;
    let mut exact = true;
    for candidate in all {
        let (lower, upper) = interval(candidate);
        if !lower.is_finite() || !candidate.possibly_in_scope() {
            continue;
        }
        let surely = candidate.certainly_in_scope()
            && candidate.interval().is_some()
            && from <= lower
            && upper <= radius;
        if surely {
            sure += 1;
        }
        if upper >= from && lower <= radius {
            possible += 1;
            exact &= measured_exactly(std::iter::once(*candidate));
        }
    }
    Measurement::Cited {
        lower: f64::from(sure),
        upper: f64::from(possible),
        dimension: None,
        locator: String::new(),
        exact,
    }
}

impl MeasuredProvider for DistanceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[COUNT_WITHIN, DISTANCE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |why: String| {
            PropertyResolutionError::Unavailable(format!("`{name}` of {object}: {why}"))
        };
        let counterparts = crate::measured_kinds::objects_of_kinds(context, call, "to", object)?;
        let reach = if name == COUNT_WITHIN {
            length(call, "radius")
        } else {
            length(call, "within")
        }
        .ok_or(PropertyResolutionError::InvalidRequest)?;
        let rule = rule(call, object, counterparts, reach);
        let declared = declaration(&rule).map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        })?;
        let prepared = prepare(context, &rule, Some(declared.margin()), declared.projection)
            .map_err(|refused| {
                unavailable(refused.not_evaluated_outcomes().first().map_or_else(
                    || "the search could not run".into(),
                    |outcome| outcome.message().to_owned(),
                ))
            })?;
        if !prepared.subjects.contains(object) || prepared.unmeasurable_subjects.contains(object) {
            return Err(unavailable("its extent could not be read".into()));
        }
        let heights = context.services.get::<VerticalExtentServiceHandle>();
        let mut scope = Scope::new(&declared, heights, context);
        let (measured, unmeasurable) = candidates(
            &prepared,
            &declared,
            &mut scope,
            object,
            &std::collections::BTreeSet::new(),
        );
        let all: Vec<&Candidate> = measured.iter().chain(&unmeasurable).collect();
        let locator = format!("{name}:{object}:{}", all.len());
        let mut measurement = if name == COUNT_WITHIN {
            count(&all, length(call, "from").unwrap_or(0.0), reach)
        } else {
            extreme(&all, call.choice("mode") == Some("farthest")).map_err(unavailable)?
        };
        let (Measurement::Value { locator: cited, .. }
        | Measurement::Rounded { locator: cited, .. }
        | Measurement::Cited { locator: cited, .. }
        | Measurement::Absent { locator: cited }) = &mut measurement;
        *cited = if cited.is_empty() {
            locator
        } else {
            format!("{locator}: {cited}")
        };
        Ok(measurement)
    }
}
