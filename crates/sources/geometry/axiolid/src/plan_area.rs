//! Plan-projected footprint and overlap areas.
//!
//! ADR 0004: this module measures areas; whether a storey's spaces cover
//! enough of it, or a space lies within a compartment, is a rule's decision.
//!
//! A planar mesh is the object's shape, so its footprint measures exactly. A
//! tessellated mesh lies within its declared chord deviation `d` of the true
//! surface, so the true footprint boundary lies within `d` of the measured
//! one. The two regions then differ only inside a band of width `2d` along
//! the measured boundary, whose area is at most `2·P·d + π·d²` for a
//! boundary of length `P`. The area is reported as that interval, never as a
//! point.

use axioval_engine::{GeometryFidelity, PlanArea, PlanAreaError, PlanAreaService};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, triangles};
use crate::planar::{footprint_measure, plan_overlap_area};

/// Overlay tolerance: tight, because exact evidence must not be laundered
/// through a loose one.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// Measures plan areas of registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidPlanAreaService {
    geometry: AxiolidGeometry,
    source: SourceId,
}

impl AxiolidPlanAreaService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self { geometry, source }
    }

    fn deviation(&self, object: &ObjectId) -> Result<f64, PlanAreaError> {
        match self.geometry.fidelity(object) {
            Ok(GeometryFidelity::Exact) => Ok(0.0),
            Ok(GeometryFidelity::Tessellated {
                chord_deviation_metres,
            }) => Ok(chord_deviation_metres),
            Err(_) => Err(PlanAreaError::InvalidMeasurement),
        }
    }

    fn measure(
        &self,
        object: &ObjectId,
    ) -> Result<(Vec<crate::geometry::Triangle>, f64, f64, f64), PlanAreaError> {
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let soup = triangles(mesh);
        let (area, perimeter) = footprint_measure(&soup, tolerance()?).ok_or_else(|| {
            PlanAreaError::Unavailable(format!("the footprint of {object} cannot be computed"))
        })?;
        Ok((soup, area, perimeter, self.deviation(object)?))
    }

    fn area(
        &self,
        measured: f64,
        slack: f64,
        cap: f64,
        locator: String,
    ) -> Result<PlanArea, PlanAreaError> {
        if slack == 0.0 {
            return PlanArea::try_new(
                measured,
                measured,
                Evidence::exact(self.source.clone(), locator),
            );
        }
        let evidence = Evidence {
            source: self.source.clone(),
            locator,
            exact: false,
        };
        PlanArea::try_new(
            (measured - slack).max(0.0),
            (measured + slack).min(cap),
            evidence,
        )
    }
}

fn tolerance() -> Result<axiolid_core::Tolerance, PlanAreaError> {
    axiolid_core::Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE)
        .map_err(|_| PlanAreaError::Unavailable("invalid overlay tolerance".into()))
}

/// Area of the band of width `2d` along a boundary of length `perimeter`.
fn band(perimeter: f64, deviation: f64) -> f64 {
    2.0 * perimeter * deviation + std::f64::consts::PI * deviation * deviation
}

impl PlanAreaService for AxiolidPlanAreaService {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (_, area, perimeter, deviation) = self.measure(object)?;
        self.area(
            area,
            band(perimeter, deviation),
            f64::INFINITY,
            format!("footprint:{object}"),
        )
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let (first_soup, first_area, first_perimeter, first_deviation) = self.measure(first)?;
        let (second_soup, second_area, second_perimeter, second_deviation) =
            self.measure(second)?;
        let overlap =
            plan_overlap_area(&first_soup, &second_soup, tolerance()?).ok_or_else(|| {
                PlanAreaError::Unavailable(format!(
                    "the overlap of {first} and {second} cannot be computed"
                ))
            })?;
        // The overlap's boundary runs along both footprints' boundaries, so
        // either one's band can move it.
        let slack =
            band(first_perimeter, first_deviation) + band(second_perimeter, second_deviation);
        // An overlap never exceeds either footprint.
        let cap = (first_area + band(first_perimeter, first_deviation))
            .min(second_area + band(second_perimeter, second_deviation));
        self.area(
            overlap.min(first_area).min(second_area),
            slack,
            cap,
            format!("plan-overlap:{first}:{second}"),
        )
    }
}
