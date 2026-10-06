//! What a space's declared boundaries cover of its body's surface, as
//! measured values, measured exactly as `space-boundary-coverage` measures
//! it: one boundary-coverage request per space and plane tolerance, kept
//! for the run, read by every value.
//!
//! - `boundary_coverage_off`: how many declared boundaries lie on no face
//!   of the body, citing the elements they bound against and noting the
//!   boundaries (`b1, b2`);
//! - `boundary_coverage_share`: the share of the surface they cover;
//! - `boundary_coverage_uncovered` and `boundary_coverage_surface`: the
//!   area no boundary covers, and the whole surface;
//! - `boundary_coverage_overlap`: the area they cover twice, citing the
//!   elements of the boundaries that surely overlap and noting those pairs
//!   (`b1 and b2; b3 and b4`).
//!
//! A space the service cannot measure refuses every value, as the
//! capability left it open: `space-boundary coverage: …`, for the reason
//! [`coverage_error`] gives.

use std::sync::Arc;

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageRequest, BoundaryCoverageServiceHandle, Citation,
    MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::coverage_error;
use crate::measured_kinds::{interval, refused};
use crate::support::Unavailable;

/// Measures the `boundary_coverage_*` values.
pub(crate) struct BoundaryMeasures;

const OFF: &str = "boundary_coverage_off";
const OVERLAP: &str = "boundary_coverage_overlap";
const SHARE: &str = "boundary_coverage_share";
const SURFACE: &str = "boundary_coverage_surface";
const UNCOVERED: &str = "boundary_coverage_uncovered";

/// One space's coverage, as its values read it.
struct Covered {
    surface: (f64, f64),
    share: (f64, f64),
    uncovered: (f64, f64),
    overlap: (f64, f64),
    exact: bool,
    /// The declared boundaries off the surface: how many, the elements
    /// they bound against (sorted, each once) and the boundaries named.
    off: usize,
    off_elements: Vec<ObjectId>,
    off_named: Option<String>,
    /// The pairs that surely overlap: their boundaries' elements (sorted,
    /// each once) and the pairs named.
    sure_elements: Vec<ObjectId>,
    sure_named: Option<String>,
}

/// The elements the boundaries bound against, each once, in identity order.
fn elements<'a>(
    boundaries: impl Iterator<Item = &'a axioval_engine::MeasuredBoundary>,
) -> Vec<ObjectId> {
    let mut elements: Vec<ObjectId> = boundaries
        .filter_map(|boundary| boundary.element().cloned())
        .collect();
    elements.sort();
    elements.dedup();
    elements
}

fn summarise(measured: &BoundaryCoverage) -> Covered {
    let span = |area: axioval_engine::SurfaceAreaInterval| {
        (area.lower_square_metres(), area.upper_square_metres())
    };
    let off: Vec<&axioval_engine::MeasuredBoundary> = measured.off_surface().collect();
    let off_named = (!off.is_empty()).then(|| {
        off.iter()
            .map(|boundary| boundary.boundary().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    });
    let sure: Vec<&axioval_engine::BoundaryOverlap> = measured
        .overlaps()
        .iter()
        .filter(|pair| pair.area().lower_square_metres() > 0.0)
        .collect();
    let sure_named = (!sure.is_empty()).then(|| {
        sure.iter()
            .map(|pair| format!("{} and {}", pair.first(), pair.second()))
            .collect::<Vec<_>>()
            .join("; ")
    });
    let involved = sure
        .iter()
        .flat_map(|pair| [pair.first(), pair.second()])
        .filter_map(|boundary| {
            measured
                .boundaries()
                .iter()
                .find(|measured| measured.boundary() == boundary)
        });
    let share = measured.covered_share();
    Covered {
        surface: span(measured.surface_area()),
        share: (share.lower(), share.upper()),
        uncovered: span(measured.uncovered_area()),
        overlap: span(measured.overlap_area()),
        exact: measured.evidence().exact,
        off: off.len(),
        off_elements: elements(off.iter().copied()),
        off_named,
        sure_elements: elements(involved),
        sure_named,
    }
}

/// The plane tolerance a call states, in metres.
fn plane(call: &MeasuredCall) -> f64 {
    match call.argument("plane") {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => *value,
        _ => 0.0,
    }
}

/// `space`'s coverage under `call`'s plane tolerance, measured once per
/// run for every value reading it.
fn covered(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    space: &ObjectId,
) -> Result<Arc<Covered>, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Key(ObjectId, u64);
    let tolerance = plane(call);
    MeasuredMemo::of(
        context.services,
        Key(space.clone(), tolerance.to_bits()),
        || {
            let service = context
                .services
                .get::<BoundaryCoverageServiceHandle>()
                .ok_or_else(|| {
                    (
                        NotEvaluatedReason::MissingService,
                        "space-boundary coverage service is not registered".to_owned(),
                    )
                })?;
            BoundaryCoverageRequest::try_new(space.clone(), tolerance)
                .and_then(|request| service.measure_boundary_coverage(&request))
                .map(|measured| Arc::new(summarise(&measured)))
                .map_err(|error| coverage_error(&error))
        },
    )
}

/// The value `call` names of `space`, with what it cites.
fn value(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    space: &ObjectId,
) -> Result<(Measurement, Citation), Unavailable> {
    let covered = covered(context, call, space)?;
    let locator = format!("boundary-coverage:{space}");
    let area = Some(QuantityDimension::Area);
    let plain = |span, dimension| {
        (
            interval(span, dimension, covered.exact, locator.clone()),
            Citation::default(),
        )
    };
    Ok(match call.name() {
        OFF => {
            #[allow(clippy::cast_precision_loss)]
            let count = covered.off as f64;
            (
                interval((count, count), None, covered.exact, locator.clone()),
                Citation {
                    related: covered.off_elements.clone(),
                    evidence: Vec::new(),
                    notes: covered.off_named.iter().cloned().collect(),
                    sources: Vec::new(),
                },
            )
        }
        OVERLAP => (
            interval(covered.overlap, area, covered.exact, locator.clone()),
            Citation {
                related: covered.sure_elements.clone(),
                evidence: Vec::new(),
                notes: covered.sure_named.iter().cloned().collect(),
                sources: Vec::new(),
            },
        ),
        SHARE => plain(covered.share, None),
        SURFACE => plain(covered.surface, area),
        _ => plain(covered.uncovered, area),
    })
}

impl MeasuredProvider for BoundaryMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[OFF, OVERLAP, SHARE, SURFACE, UNCOVERED]
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

    /// Each space is measured once per run, whichever value reads it.
    fn memoizes(&self) -> bool {
        true
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        value(context, call, object).map_err(refused(call.name(), object))
    }
}
