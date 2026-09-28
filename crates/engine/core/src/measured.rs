//! Values the engine measures from geometry services and answers as the
//! reserved property set [`axioval_ir::MEASURED_SET`].
//!
//! Each value is an interval sure to hold the exact value: a point is a
//! quantity with exact evidence, anything wider a
//! [`PropertyValue::Measured`] whose evidence is never exact. A missing
//! service is a fact of the run, never of the object.

use axioval_ir::{
    Evidence, MEASURED_AREA, MEASURED_BOTTOM, MEASURED_EXTENT_X, MEASURED_EXTENT_Y,
    MEASURED_EXTENT_Z, MEASURED_SET, MEASURED_TOP, MEASURED_VOLUME, ObjectId, Property,
    PropertyValue, QuantityDimension,
};

use crate::ServiceRegistry;
use crate::free_space::MetricDirection;
use crate::plan_area::PlanAreaServiceHandle;
use crate::properties::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, ResolvedProperty,
};
use crate::proximity::ProximityServiceHandle;
use crate::vertical_extent::VerticalExtentServiceHandle;

/// The geometry services a run measures with, as the host registered them.
#[derive(Clone, Default)]
pub(crate) struct Measures {
    vertical: Option<VerticalExtentServiceHandle>,
    plan: Option<PlanAreaServiceHandle>,
    proximity: Option<ProximityServiceHandle>,
}

impl Measures {
    pub(crate) fn of(services: &ServiceRegistry) -> Self {
        Self {
            vertical: services.get::<VerticalExtentServiceHandle>().cloned(),
            plan: services.get::<PlanAreaServiceHandle>().cloned(),
            proximity: services.get::<ProximityServiceHandle>().cloned(),
        }
    }

    /// Answers one request in the measured set.
    pub(crate) fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let name = request.property().to_ascii_lowercase();
        let object = request.object_id();
        let (lower, upper, dimension, locator) = self.measure(&name, object)?;
        if !(lower.is_finite() && upper.is_finite() && lower <= upper) {
            return Err(PropertyResolutionError::InvalidValue);
        }
        let exact = lower.to_bits() == upper.to_bits();
        let value = if exact {
            PropertyValue::Quantity {
                value: lower,
                dimension,
            }
        } else {
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            }
        };
        let mut evidence = Evidence::exact(
            object.source.clone(),
            format!("{MEASURED_SET}/{name}: {locator}"),
        );
        evidence.exact = exact;
        let property = Property::new(MEASURED_SET, request.property(), value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?
            .with_evidence(evidence);
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
    }

    fn measure(
        &self,
        name: &str,
        object: &ObjectId,
    ) -> Result<(f64, f64, QuantityDimension, String), PropertyResolutionError> {
        let missing = |service: &str| {
            PropertyResolutionError::MissingService(format!(
                "no {service} service is registered, so `{MEASURED_SET}` value `{name}` \
                 cannot be measured"
            ))
        };
        let unavailable = |error: String| {
            PropertyResolutionError::Unavailable(format!(
                "`{MEASURED_SET}` value `{name}` of {object}: {error}"
            ))
        };
        let length = QuantityDimension::Length;
        match name {
            MEASURED_BOTTOM | MEASURED_TOP | MEASURED_EXTENT_Z => {
                let service = self
                    .vertical
                    .as_ref()
                    .ok_or_else(|| missing("vertical-extent"))?;
                let extent = service
                    .measure_vertical_extent(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                let locator = extent.evidence().locator.clone();
                let (lower, upper) = match name {
                    MEASURED_BOTTOM => (
                        extent.bottom().lower_metres(),
                        extent.bottom().upper_metres(),
                    ),
                    MEASURED_TOP => (extent.top().lower_metres(), extent.top().upper_metres()),
                    _ => extent.height_metres(),
                };
                Ok((lower, upper, length, locator))
            }
            MEASURED_EXTENT_X | MEASURED_EXTENT_Y => {
                let service = self
                    .vertical
                    .as_ref()
                    .ok_or_else(|| missing("vertical-extent"))?;
                let axis = if name == MEASURED_EXTENT_X {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let direction = MetricDirection::try_new(axis)
                    .map_err(|error| unavailable(error.to_string()))?;
                let extent = service
                    .measure_directional_extent(object, direction)
                    .map_err(|error| unavailable(error.to_string()))?;
                let (lower, upper) = extent.length_metres();
                Ok((lower, upper, length, extent.evidence().locator.clone()))
            }
            MEASURED_AREA => {
                let service = self.plan.as_ref().ok_or_else(|| missing("plan-area"))?;
                let area = service
                    .measure_footprint(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok((
                    area.lower_square_metres(),
                    area.upper_square_metres(),
                    QuantityDimension::Area,
                    area.evidence().locator.clone(),
                ))
            }
            MEASURED_VOLUME => {
                let service = self
                    .proximity
                    .as_ref()
                    .ok_or_else(|| missing("proximity"))?;
                let body = service
                    .measure_body_volume(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok((
                    body.volume().lower_cubic_metres(),
                    body.volume().upper_cubic_metres(),
                    QuantityDimension::Volume,
                    body.evidence().locator.clone(),
                ))
            }
            _ => Err(PropertyResolutionError::InvalidRequest),
        }
    }
}
