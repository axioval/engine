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
//!
//! A bodiless group the host declares (a zone) has the union of its members'
//! footprints. Membership is semantic, so the host states it; a member without
//! a body, an unmeasured member or undecided membership refuses, never zero.

use axioval_engine::{GeometryFidelity, PlanArea, PlanAreaError, PlanAreaService};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{footprint_measure, plan_overlap_area};

/// Overlay tolerance: tight, because exact evidence must not be laundered
/// through a loose one.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// A measured footprint and the triangles it was measured from.
struct Footprint {
    soup: Vec<Triangle>,
    area: f64,
    perimeter: f64,
    deviation: f64,
}

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

    /// The object's plan triangles, footprint area and perimeter, and the
    /// largest chord deviation among the meshes they came from.
    fn measure(&self, object: &ObjectId) -> Result<Footprint, PlanAreaError> {
        let mut soup = Vec::new();
        let mut deviation = 0.0_f64;
        self.collect(object, None, &mut Vec::new(), &mut soup, &mut deviation)?;
        let (area, perimeter) = footprint_measure(&soup, tolerance()?).ok_or_else(|| {
            PlanAreaError::Unavailable(format!("the footprint of {object} cannot be computed"))
        })?;
        Ok(Footprint {
            soup,
            area,
            perimeter,
            deviation,
        })
    }

    /// Gathers the triangles whose plan union is `object`'s footprint.
    ///
    /// A declared group contributes its members' triangles: the overlay
    /// unions them, so members that overlap count once. The union's true
    /// boundary lies within the largest member deviation of the measured one,
    /// so that deviation bounds the whole group's band.
    fn collect(
        &self,
        object: &ObjectId,
        group: Option<&ObjectId>,
        visiting: &mut Vec<ObjectId>,
        soup: &mut Vec<Triangle>,
        deviation: &mut f64,
    ) -> Result<(), PlanAreaError> {
        let named = group.map_or_else(
            || object.to_string(),
            |group| format!("{object} (a member of {group})"),
        );
        if let Some(members) = self.geometry.group_members(object) {
            let members = members.map_err(|reason| {
                PlanAreaError::Unavailable(format!(
                    "the members of {named} are undecided: {reason}"
                ))
            })?;
            if visiting.contains(object) {
                return Err(PlanAreaError::Unavailable(format!(
                    "{object} is a member of itself, so it has no footprint"
                )));
            }
            // An empty group is not an empty footprint: nothing states where it is.
            if members.is_empty() {
                return Err(PlanAreaError::Unavailable(format!(
                    "{named} groups nothing, so it has no footprint"
                )));
            }
            visiting.push(object.clone());
            for member in members {
                self.collect(member, Some(object), visiting, soup, deviation)?;
            }
            visiting.pop();
            return Ok(());
        }
        if self.geometry.has_no_body(object) {
            // A declared bodiless object (a storey) covers nothing, exactly.
            // A group member without a body leaves the group's extent unknown.
            return match group {
                None => Ok(()),
                Some(_) => Err(PlanAreaError::Unavailable(format!(
                    "{named} has no body to give it a footprint"
                ))),
            };
        }
        // An unmeasured body exists with an unknown extent: never zero.
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(PlanAreaError::Unavailable(format!(
                "{named} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self.geometry.mesh(object).ok_or_else(|| match group {
            None => PlanAreaError::UnknownObject(object.clone()),
            Some(_) => PlanAreaError::Unavailable(format!("{named} has no described geometry")),
        })?;
        soup.extend(triangles(mesh));
        *deviation = deviation.max(self.deviation(object)?);
        Ok(())
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
        let footprint = self.measure(object)?;
        self.area(
            footprint.area,
            band(footprint.perimeter, footprint.deviation),
            f64::INFINITY,
            format!("footprint:{object}"),
        )
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let one = self.measure(first)?;
        let other = self.measure(second)?;
        let overlap = plan_overlap_area(&one.soup, &other.soup, tolerance()?).ok_or_else(|| {
            PlanAreaError::Unavailable(format!(
                "the overlap of {first} and {second} cannot be computed"
            ))
        })?;
        // The overlap's boundary runs along both footprints' boundaries, so
        // either one's band can move it.
        let slack = band(one.perimeter, one.deviation) + band(other.perimeter, other.deviation);
        // An overlap never exceeds either footprint.
        let cap = (one.area + band(one.perimeter, one.deviation))
            .min(other.area + band(other.perimeter, other.deviation));
        self.area(
            overlap.min(one.area).min(other.area),
            slack,
            cap,
            format!("plan-overlap:{first}:{second}"),
        )
    }
}
