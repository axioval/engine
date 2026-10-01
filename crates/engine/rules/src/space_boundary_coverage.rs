//! `space-boundary-coverage`: how much of each space's body surface its
//! declared space boundaries cover, the gaps they leave and where they
//! overlap.

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest,
    BoundaryCoverageServiceHandle, BoundaryOverlap, CapabilityEvaluation, CompiledRule,
    MeasuredBoundary, NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::{Finding, ObjectId, QuantityDimension};

use crate::plan_area::{Verdict, judge};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Requires each selected space's declared boundaries to cover its body's
/// surface: at least `minimum_covered_share` of it, leaving at most
/// `maximum_uncovered_area` uncovered and overlapping each other over at most
/// `maximum_overlap_area`. At least one of the three is required.
///
/// The boundaries are the ones the source declares for the space, with the
/// connection surfaces it states; the rule selects spaces, never boundaries.
/// A boundary surface counts on a face of the body when it lies within
/// `plane_tolerance` of the face's plane (default zero: on the plane, up to
/// a micrometre). A boundary lying on no face plane covers nothing and is
/// always its own finding, relating the element it bounds against: it is
/// misplaced whatever the thresholds are. A space whose boundary surfaces
/// cannot all be read (a surface form the host does not lower, or a
/// boundary stating none), whose body is curved or missing is not
/// evaluated.
///
/// Areas are intervals: a turned space measures within the rounding of its
/// projection, a tessellated boundary within its chord deviation. A check
/// is a finding only when the whole interval breaks its bound; one
/// straddling it is not evaluated. An overlap finding relates the elements
/// of the boundaries that surely overlap.
pub struct SpaceBoundaryCoverage;

struct Config {
    minimum_share: Option<f64>,
    maximum_uncovered: Option<f64>,
    maximum_overlap: Option<f64>,
    plane_tolerance: f64,
}

fn quantity(
    parameters: &Parameters<'_>,
    name: &str,
    dimension: QuantityDimension,
    unit: &str,
) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, found)) if found == dimension && value >= 0.0 => Ok(Some(value)),
        Some((_, found)) if found == dimension => Err(invalid(format!("`{name}` is negative"))),
        Some(_) => Err(invalid(format!("`{name}` is not {unit}"))),
    }
}

impl Config {
    fn parse(rule: &CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let minimum_share = parameters.number("minimum_covered_share")?;
        if minimum_share.is_some_and(|share| !(0.0..=1.0).contains(&share)) {
            return Err(invalid("`minimum_covered_share` lies outside 0 to 1"));
        }
        let area = QuantityDimension::Area;
        let maximum_uncovered = quantity(&parameters, "maximum_uncovered_area", area, "an area")?;
        let maximum_overlap = quantity(&parameters, "maximum_overlap_area", area, "an area")?;
        if minimum_share.is_none() && maximum_uncovered.is_none() && maximum_overlap.is_none() {
            return Err(invalid(
                "`minimum_covered_share`, `maximum_uncovered_area` or `maximum_overlap_area` \
                 is required",
            ));
        }
        let plane_tolerance = quantity(
            &parameters,
            "plane_tolerance",
            QuantityDimension::Length,
            "a length",
        )?
        .unwrap_or(0.0);
        Ok(Self {
            minimum_share,
            maximum_uncovered,
            maximum_overlap,
            plane_tolerance,
        })
    }
}

impl RuleCapability for SpaceBoundaryCoverage {
    fn id(&self) -> &'static str {
        "axioval:capability.space-boundary-coverage"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("minimum_covered_share", ParameterType::Number),
            ParameterDescriptor::optional("maximum_uncovered_area", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum_overlap_area", ParameterType::Quantity),
            ParameterDescriptor::optional("plane_tolerance", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("space-boundary-coverage: {message}"),
                );
            }
        };
        let Some(coverage) = context.services.get::<BoundaryCoverageServiceHandle>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "space-boundary coverage service is not registered",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let measured =
                BoundaryCoverageRequest::try_new(object.id.clone(), config.plane_tolerance)
                    .and_then(|request| coverage.measure_boundary_coverage(&request))
                    .map_err(|error| coverage_error(&error));
            let measured = match measured {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let (findings, undecided) = check(rule, &config, &measured);
            for found in findings {
                evaluation.push_finding(found);
            }
            if !undecided.is_empty() {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    undecided.join("; "),
                );
            }
        }
        evaluation
    }
}

/// The findings on one space, and the checks its intervals leave undecided.
fn check(
    rule: &CompiledRule,
    config: &Config,
    measured: &BoundaryCoverage,
) -> (Vec<Finding>, Vec<String>) {
    let space = measured.space();
    let evidence = || vec![measured.evidence().clone()];
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut judged = |outcome: Outcome, related: Vec<ObjectId>| match outcome {
        Outcome::Pass => {}
        Outcome::Fail(message) => {
            findings.push(finding(rule, space, message, evidence(), related));
        }
        Outcome::Undecided(message) => undecided.push(message),
    };

    let off: Vec<&MeasuredBoundary> = measured.off_surface().collect();
    if !off.is_empty() {
        let names: Vec<String> = off.iter().map(|b| b.boundary().to_string()).collect();
        judged(
            Outcome::Fail(format!(
                "space boundary {} lies on no face of the space's body, so it covers nothing",
                names.join(", ")
            )),
            elements(off.iter().copied()),
        );
    }

    let surface = measured.surface_area();
    let surface = shown_area(surface.lower_square_metres(), surface.upper_square_metres());
    let uncovered = measured.uncovered_area();
    let (gap_lower, gap_upper) = (
        uncovered.lower_square_metres(),
        uncovered.upper_square_metres(),
    );
    if let Some(minimum) = config.minimum_share {
        let share = measured.covered_share();
        let text = format!(
            "declared boundaries cover {} of the {surface} surface, leaving {} uncovered",
            shown_share(share.lower(), share.upper()),
            shown_area(gap_lower, gap_upper),
        );
        let outcome = match judge(share.lower(), share.upper(), Some(minimum), None) {
            Verdict::Pass => Outcome::Pass,
            Verdict::Fail(_) => {
                Outcome::Fail(format!("{text}; at least {} required", percent(minimum)))
            }
            Verdict::Undecided(_) => Outcome::Undecided(format!(
                "{text}, which straddles the required {}",
                percent(minimum)
            )),
        };
        judged(outcome, vec![]);
    }
    if let Some(maximum) = config.maximum_uncovered {
        let text = format!(
            "declared boundaries leave {} of the {surface} surface uncovered",
            shown_area(gap_lower, gap_upper)
        );
        judged(at_most(gap_lower, gap_upper, maximum, &text), vec![]);
    }
    if let Some(maximum) = config.maximum_overlap {
        let overlap = measured.overlap_area();
        let (lower, upper) = (overlap.lower_square_metres(), overlap.upper_square_metres());
        let sure: Vec<&BoundaryOverlap> = measured
            .overlaps()
            .iter()
            .filter(|pair| pair.area().lower_square_metres() > 0.0)
            .collect();
        let between = if sure.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = sure
                .iter()
                .map(|pair| format!("{} and {}", pair.first(), pair.second()))
                .collect();
            format!(" (boundaries {})", pairs.join("; "))
        };
        let text = format!(
            "declared boundaries overlap over {} of the surface{between}",
            shown_area(lower, upper)
        );
        let involved = sure
            .iter()
            .flat_map(|pair| [pair.first(), pair.second()])
            .filter_map(|boundary| {
                measured
                    .boundaries()
                    .iter()
                    .find(|measured| measured.boundary() == boundary)
            });
        judged(at_most(lower, upper, maximum, &text), elements(involved));
    }
    (findings, undecided)
}

/// What one check concludes.
enum Outcome {
    Pass,
    Fail(String),
    Undecided(String),
}

/// An area interval against a maximum.
fn at_most(lower: f64, upper: f64, maximum: f64, text: &str) -> Outcome {
    match judge(lower, upper, None, Some(maximum)) {
        Verdict::Pass => Outcome::Pass,
        Verdict::Fail(_) => Outcome::Fail(format!(
            "{text}; at most {} allowed",
            square_metres(maximum)
        )),
        Verdict::Undecided(_) => Outcome::Undecided(format!(
            "{text}, which straddles the allowed {}",
            square_metres(maximum)
        )),
    }
}

/// The elements the boundaries bound against, each once, in identity order.
fn elements<'a>(boundaries: impl Iterator<Item = &'a MeasuredBoundary>) -> Vec<ObjectId> {
    let mut elements: Vec<ObjectId> = boundaries
        .filter_map(|boundary| boundary.element().cloned())
        .collect();
    elements.sort();
    elements.dedup();
    elements
}

pub(crate) fn coverage_error(error: &BoundaryCoverageError) -> Unavailable {
    let reason = match error {
        BoundaryCoverageError::UnknownSpace(_)
        | BoundaryCoverageError::NoBody(_)
        | BoundaryCoverageError::Unavailable(_) => NotEvaluatedReason::BackendUnavailable,
        BoundaryCoverageError::Unsupported => NotEvaluatedReason::MissingService,
        BoundaryCoverageError::InvalidRequest(_) => NotEvaluatedReason::InvalidDeclaration,
        BoundaryCoverageError::InvalidMeasurement => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("space-boundary coverage: {error}"))
}

fn square_metres(value: f64) -> String {
    format!("{} m²", (value * 1e6).round() / 1e6)
}

fn shown_area(lower: f64, upper: f64) -> String {
    let (low, high) = (square_metres(lower), square_metres(upper));
    if low == high {
        low
    } else {
        format!("between {low} and {high}")
    }
}

fn percent(share: f64) -> String {
    format!("{}%", (share * 1e4).round() / 1e2)
}

fn shown_share(lower: f64, upper: f64) -> String {
    let (low, high) = (percent(lower), percent(upper));
    if low == high {
        low
    } else {
        format!("between {low} and {high}")
    }
}
