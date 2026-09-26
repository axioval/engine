//! Measure values converted to canonical SI quantities.
//!
//! A measure in an IFC file is a number in some unit: the explicit unit of a
//! property, otherwise the project's default unit of that kind. Reading the
//! bare number would compare millimetres with metres, so every measure goes
//! through `ifc_properties::exact_unit`, which resolves the effective unit to
//! an exact SI scale (and offset, for degrees Celsius) or refuses. A refusal
//! is reported, never replaced by a guess.

use axioval_engine::PropertyResolutionError;
use axioval_ir::{PropertyValue, QuantityDimension};
use ifc_model::{EntityId, Model};
use ifc_properties::{ExactUnitError, exact_unit};

/// `value` of declared type `measure_type` in SI, or `None` when the type is
/// not a measure at all.
///
/// Dimensionless measures (ratios, counts) become plain decimals; plane
/// angles become radians; everything else becomes a quantity of its SI
/// dimension.
pub(crate) fn si_value(
    model: &Model,
    measure_type: &str,
    unit: Option<EntityId>,
    value: f64,
) -> Result<Option<PropertyValue>, PropertyResolutionError> {
    let measure_type = measure_type.to_ascii_uppercase();
    let resolved = match exact_unit(model, &measure_type, unit) {
        Ok(resolved) => resolved,
        Err(ExactUnitError::NotAMeasure { .. }) => return Ok(None),
        Err(error) => {
            return Err(PropertyResolutionError::Incomplete(format!(
                "the unit of a {measure_type} cannot be resolved exactly: {error}"
            )));
        }
    };
    let si = value * resolved.scale + resolved.offset;
    if !si.is_finite() {
        return Err(PropertyResolutionError::InvalidValue);
    }
    let mut exponents = [0_i8; 7];
    for (target, exponent) in exponents.iter_mut().zip(resolved.dimensions) {
        *target = i8::try_from(exponent).map_err(|_| {
            PropertyResolutionError::Incomplete(format!(
                "{measure_type} has a dimension exponent out of range"
            ))
        })?;
    }
    Ok(Some(match QuantityDimension::from_exponents(exponents) {
        Some(dimension) => PropertyValue::Quantity {
            value: si,
            dimension,
        },
        None if measure_type.contains("PLANEANGLE") => PropertyValue::Quantity {
            value: si,
            dimension: QuantityDimension::PlaneAngle,
        },
        None => PropertyValue::Decimal(si),
    }))
}
