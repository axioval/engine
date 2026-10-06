//! The voided area of a host as a measured value: the summed section areas
//! of the openings its path reaches, on the host's middle plane, measured
//! as `opening-area` measures them, so `gross_area − net_area` against it
//! is that capability's comparison; and, as `empty-host` compares them, how
//! many openings it counts and the area of the face they void.
//!
//! A host's face is measured once per run for its axes. Its openings are
//! placed where a value reads them, once per object and rule: keeping each
//! placement (or each value, `MeasuredProvider::memoizes`) for the run
//! holds more than placing it again costs, as a run of many rules over
//! many hosts shows.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Citation, CompiledRule, MeasuredMemo, MeasuredProvider, Measurement, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, RuleId};

use crate::counts::Population;
use crate::empty_host::face_area;
use crate::measured_kinds::{refused, selection_cow};
use crate::opening_area::{Openings, Picks, voided};
use crate::opening_zone::face::{FaceAxes, read_host};
use crate::support::{Parameters, Unavailable};

/// The name measured.
pub(crate) const OPENING_AREA: &str = "opening_area";
/// One opening's section area on its host's middle plane.
pub(crate) const OPENING_SECTION_AREA: &str = "opening_section_area";

/// How many openings take area from the host's middle plane.
const OPENING_COUNT: &str = "opening_count";
/// The host's face on its middle plane, as `empty-host` measures it.
const MIDDLE_FACE_AREA: &str = "middle_face_area";

/// Measures `opening_area` and what `empty-host` compares.
pub(crate) struct OpeningMeasures;

/// The face `call`'s axes name on the middle plane of `host`.
fn middle_face(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    host: &ObjectId,
) -> Result<Measurement, Unavailable> {
    let length = call.choice("length_axis").unwrap_or("extrusion");
    let height = call.choice("height_axis").unwrap_or("profile-y");
    let key = FaceKey(host.clone(), format!("{length};{height}"));
    let (area, exact) = MeasuredMemo::of(context.services, key, || {
        let axes = FaceAxes::parse(length, height)?;
        let face = read_host(context, host)?;
        Ok::<_, Unavailable>((face_area(&face, axes)?, exact(&face.evidence)))
    })?;
    Ok(crate::measured_kinds::interval(
        (area, area),
        Some(QuantityDimension::Area),
        exact,
        format!("{MIDDLE_FACE_AREA}:{host}"),
    ))
}

/// The key of a host's middle face in the run's memo: the host and its
/// axes as written.
#[derive(Hash, PartialEq, Eq)]
struct FaceKey(ObjectId, String);

/// Whether every evidence a value was measured from is exact: the body
/// facts are stated, so this holds unless a source cites an estimate.
fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

/// A sum of `terms` non-negative doubles, widened by the most its
/// rounding may have moved it, so it holds the exact sum of the terms.
fn summed(sum: f64, terms: usize) -> (f64, f64) {
    if terms < 2 || sum == 0.0 {
        return (sum, sum);
    }
    #[allow(clippy::cast_precision_loss)]
    let margin = (terms - 1) as f64 * f64::EPSILON * sum;
    (
        (sum - margin).next_down().max(0.0),
        (sum + margin).next_up(),
    )
}

/// The openings `call` names as `opening-area` reads them: its path, axes
/// and minimum, as a rule of that capability would state them.
fn openings_rule(call: &MeasuredCall, path_key: &str) -> Result<CompiledRule, Unavailable> {
    let text = |value: &str| ParameterValue::String {
        value: value.to_owned(),
    };
    let Some(MeasuredArgument::Path(steps)) = call.argument(path_key) else {
        return Err(crate::support::invalid(format!("`{path_key}` is required")));
    };
    let mut parameters = BTreeMap::from([
        (
            "opening_path".to_owned(),
            ParameterValue::StringList {
                value: steps.clone(),
            },
        ),
        (
            "length_axis".to_owned(),
            text(call.choice("length_axis").unwrap_or("extrusion")),
        ),
        (
            "height_axis".to_owned(),
            text(call.choice("height_axis").unwrap_or("profile-y")),
        ),
    ]);
    if let Some(MeasuredArgument::Number(minimum)) = call.argument("minimum") {
        parameters.insert(
            "minimum_opening_area".to_owned(),
            ParameterValue::Quantity {
                value: *minimum,
                unit: "m2".into(),
            },
        );
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-opening-area").expect("a valid rule id"),
        capability: "axioval:capability.opening-area".into(),
        severity: Severity::Info,
        selector: Selector::All,
        parameters,
    })
}

/// The declaration of a host's openings a rule hands `opening_area` and
/// `opening_count` (`length_axis=@length_axis;…;minimum=@minimum_opening_area`),
/// checked as `opening-area` and `empty-host` checked theirs: the face's two
/// axes, then the minimum opening area, worded by the rule parameters'
/// names.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    // Unstated, an axis takes the value's default.
    let axis = |key: &str, default| match stated.get(key) {
        Some(ParameterValue::String { value }) => value.as_str(),
        _ => default,
    };
    FaceAxes::parse(
        axis("length_axis", "extrusion"),
        axis("height_axis", "profile-y"),
    )?;
    let rule = CompiledRule {
        id: RuleId::new("axioval-measured-opening-area").expect("a valid rule id"),
        capability: "axioval:capability.opening-area".into(),
        severity: Severity::Info,
        selector: Selector::All,
        parameters: stated
            .get("minimum")
            .map(|minimum| ("minimum_opening_area".to_owned(), minimum.clone()))
            .into_iter()
            .collect(),
    };
    crate::opening_area::minimum_area(&Parameters(&rule)).map(|_| ())
}

/// What placing a host's openings found: their summed area on its middle
/// plane, the openings reached and those taking area, and the evidence.
struct Voids {
    sum: f64,
    reached: Vec<ObjectId>,
    counted: Vec<ObjectId>,
    /// Whether every evidence placed from is exact.
    exact: bool,
}

/// The key of every object as candidate openings in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct EveryObject;

/// The openings of `host` the call names, placed.
fn voids(
    call: &MeasuredCall,
    host: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Voids, Unavailable> {
    let rule = openings_rule(call, "path")?;
    let openings = Openings::parse(&Parameters(&rule))?;
    let every;
    let picks = match selection_cow(context, call, "openings") {
        Ok(Some(selection)) => {
            every = selection;
            Picks {
                matched: &every.matched,
                undecided: &every.undecided,
            }
        }
        Ok(None) => {
            let population = MeasuredMemo::of(context.services, EveryObject, || {
                Arc::new(Population::of(context, &Selector::All))
            });
            let subject = crate::selection::object_by_id(context, host)
                .ok_or_else(|| crate::support::invalid(format!("{host} is not in the project")))?;
            let mut evidence = Vec::new();
            let placed = voided(
                context,
                &openings,
                Picks::of(&population),
                subject,
                &mut evidence,
            )?;
            return Ok(Voids {
                sum: placed.sum,
                reached: placed.reached,
                counted: placed.counted,
                exact: exact(&evidence),
            });
        }
        Err(error) => {
            return Err((
                axioval_engine::NotEvaluatedReason::IncompleteEvidence,
                error.to_string(),
            ));
        }
    };
    let subject = crate::selection::object_by_id(context, host)
        .ok_or_else(|| crate::support::invalid(format!("{host} is not in the project")))?;
    let mut evidence = Vec::new();
    let placed = voided(context, &openings, picks, subject, &mut evidence)?;
    Ok(Voids {
        sum: placed.sum,
        reached: placed.reached,
        counted: placed.counted,
        exact: exact(&evidence),
    })
}

impl MeasuredProvider for OpeningMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            MIDDLE_FACE_AREA,
            OPENING_AREA,
            OPENING_COUNT,
            OPENING_SECTION_AREA,
        ]
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

    /// An opening area cites every opening its host reaches; an opening
    /// count, the openings taking area from the middle plane.
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let refused = refused(call.name(), object);
        match call.name() {
            MIDDLE_FACE_AREA => middle_face(call, context, object)
                .map(|measurement| (measurement, Citation::default()))
                .map_err(refused),
            OPENING_SECTION_AREA => {
                let rule = openings_rule(call, "host_path").map_err(&refused)?;
                let openings = Openings::parse(&Parameters(&rule)).map_err(&refused)?;
                let subject = context.project.object(object).ok_or_else(|| {
                    PropertyResolutionError::Unavailable(format!(
                        "`{OPENING_AREA}` of {object}: it is not in the project"
                    ))
                })?;
                section(context, &openings, subject)
                    .map(|measurement| (measurement, Citation::default()))
                    .map_err(refused)
            }
            name => {
                let placed = voids(call, object, context).map_err(refused)?;
                let exact = placed.exact;
                if name == OPENING_COUNT {
                    #[allow(clippy::cast_precision_loss)]
                    let counted = placed.counted.len() as f64;
                    return Ok((
                        crate::measured_kinds::interval(
                            (counted, counted),
                            None,
                            exact,
                            format!("{OPENING_COUNT}:{object}"),
                        ),
                        Citation {
                            related: placed.counted,
                            evidence: Vec::new(),
                            notes: Vec::new(),
                            ..Citation::default()
                        },
                    ));
                }
                Ok((
                    crate::measured_kinds::interval(
                        summed(placed.sum, placed.counted.len()),
                        Some(QuantityDimension::Area),
                        exact,
                        format!("{OPENING_AREA}:{object}"),
                    ),
                    Citation {
                        related: if call.choice("cites") == Some("counted") {
                            placed.counted
                        } else {
                            placed.reached
                        },
                        evidence: Vec::new(),
                        notes: Vec::new(),
                        ..Citation::default()
                    },
                ))
            }
        }
    }

    /// A host's face is kept once per run; its values are not, since each
    /// is read once per object and rule, and keeping them holds more than
    /// measuring them again costs.
    fn memoizes(&self) -> bool {
        true
    }
}

/// The section area `opening` takes from the middle plane of the one host
/// `openings`' path reaches from it.
fn section(
    context: &RuleContext<'_>,
    openings: &Openings<'_>,
    opening: &axioval_ir::Object,
) -> Result<Measurement, Unavailable> {
    let everything: Vec<&axioval_ir::Object> = context.project.objects().collect();
    let (hosts, _) = openings.path().related(context, &opening.id, &everything)?;
    let [host] = &hosts[..] else {
        return if hosts.is_empty() {
            Ok(Measurement::Absent {
                locator: format!("{OPENING_SECTION_AREA}:{}: it voids no host", opening.id),
            })
        } else {
            Err((
                axioval_engine::NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "it voids {} hosts, so its section is ambiguous",
                    hosts.len()
                ),
            ))
        };
    };
    let face = crate::opening_zone::face::read_host(context, host)?;
    let mut evidence = Vec::new();
    // `None` is an opening surely below the declared minimum, which takes
    // nothing from the plane as `opening-area` counts it: a decided zero,
    // not a missing area (an area that cannot be measured is an error).
    let area = crate::opening_area::opening_area(context, openings, &face, opening, &mut evidence)?
        .map_or(0.0, |(area, _)| area);
    evidence.extend(face.evidence.iter().cloned());
    Ok(crate::measured_kinds::interval(
        (area, area),
        Some(QuantityDimension::Area),
        exact(&evidence),
        format!("{OPENING_SECTION_AREA}:{}:{host}", opening.id),
    ))
}
