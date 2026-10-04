//! Profile values: each dimension and slope of a member's swept profile,
//! read as `allowed-profile` reads them, and the section area and elastic
//! section modulus where the profile's family defines them. The profile's
//! type and name are stated, and read as properties of the body set.

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Profile, dimension_value, read_profile};
use crate::body_facts::BodyFacts;
use crate::support::Unavailable;

/// The names measured here.
pub(crate) const NAMES: &[&str] = &[
    "profile_dimension",
    "profile_slope",
    "section_area",
    "section_modulus",
];

/// Measures profile values.
pub(crate) struct ProfileMeasures;

fn incomplete(message: impl Into<String>) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message.into())
}

/// The dimension `column` of `profile`, required.
fn needed(profile: &Profile, column: &str, body: &mut BodyFacts<'_>) -> Result<f64, Unavailable> {
    dimension_value(profile, column, body)?.ok_or_else(|| {
        incomplete(format!(
            "the `{}` profile states no {column}",
            profile.family
        ))
    })
}

/// Zero when `column` is unset, refused when it is set to anything else:
/// the formula ignores it.
fn zero(profile: &Profile, column: &str, body: &mut BodyFacts<'_>) -> Result<(), Unavailable> {
    match dimension_value(profile, column, body)? {
        Some(value) if value != 0.0 => Err(incomplete(format!(
            "the `{}` profile has a {column}, which the section formula leaves out",
            profile.family
        ))),
        _ => Ok(()),
    }
}

/// The section area of `profile`.
fn area(profile: &Profile, body: &mut BodyFacts<'_>) -> Result<f64, Unavailable> {
    use std::f64::consts::PI;
    Ok(match profile.family.as_str() {
        "rectangle" => needed(profile, "width", body)? * needed(profile, "depth", body)?,
        "rectangle-hollow" => {
            zero(profile, "fillet_radius", body)?;
            zero(profile, "outer_fillet_radius", body)?;
            let (b, d) = (
                needed(profile, "width", body)?,
                needed(profile, "depth", body)?,
            );
            let t = needed(profile, "wall_thickness", body)?;
            b * d - (b - 2.0 * t) * (d - 2.0 * t)
        }
        "circle" => PI * needed(profile, "radius", body)?.powi(2),
        "circle-hollow" => {
            let r = needed(profile, "radius", body)?;
            let t = needed(profile, "wall_thickness", body)?;
            PI * (r * r - (r - t) * (r - t))
        }
        "ellipse" => {
            PI * needed(profile, "semi_axis_1", body)? * needed(profile, "semi_axis_2", body)?
        }
        "i-shape" => {
            zero(profile, "flange_slope", body)?;
            zero(profile, "edge_radius", body)?;
            let (b, d) = (
                needed(profile, "width", body)?,
                needed(profile, "depth", body)?,
            );
            let (tw, tf) = (
                needed(profile, "web_thickness", body)?,
                needed(profile, "flange_thickness", body)?,
            );
            let r = dimension_value(profile, "fillet_radius", body)?.unwrap_or(0.0);
            2.0 * b * tf + (d - 2.0 * tf) * tw + (4.0 - PI) * r * r
        }
        family => {
            return Err(incomplete(format!(
                "a `{family}` profile defines no section area here"
            )));
        }
    })
}

/// The elastic section modulus of `profile` about its strong or weak axis.
fn modulus(profile: &Profile, strong: bool, body: &mut BodyFacts<'_>) -> Result<f64, Unavailable> {
    use std::f64::consts::PI;
    Ok(match profile.family.as_str() {
        "rectangle" => {
            let (b, d) = (
                needed(profile, "width", body)?,
                needed(profile, "depth", body)?,
            );
            let (across, deep) = if (d >= b) == strong { (b, d) } else { (d, b) };
            across * deep * deep / 6.0
        }
        "circle" => PI * needed(profile, "radius", body)?.powi(3) / 4.0,
        "circle-hollow" => {
            let r = needed(profile, "radius", body)?;
            let inner = r - needed(profile, "wall_thickness", body)?;
            PI * (r.powi(4) - inner.powi(4)) / (4.0 * r)
        }
        family => {
            return Err(incomplete(format!(
                "a `{family}` profile defines no section modulus here"
            )));
        }
    })
}

impl ProfileMeasures {
    fn value(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let mut body = BodyFacts::of(context, object)?;
        let profile = read_profile(&mut body)?.map_err(incomplete)?;
        let (value, dimension) = match call.name() {
            "profile_dimension" | "profile_slope" => {
                let column = call.choice("name").unwrap_or("width");
                let Some(value) = dimension_value(&profile, column, &mut body)? else {
                    return Ok(Measurement::Absent {
                        locator: format!("the `{}` profile has no {column}", profile.family),
                    });
                };
                let dimension = if call.name() == "profile_slope" {
                    QuantityDimension::PlaneAngle
                } else {
                    QuantityDimension::Length
                };
                (value, dimension)
            }
            "section_area" => (area(&profile, &mut body)?, QuantityDimension::Area),
            _ => (
                modulus(&profile, call.choice("axis") != Some("weak"), &mut body)?,
                QuantityDimension::Volume,
            ),
        };
        if !value.is_finite() || value < 0.0 {
            return Err(incomplete("the profile's dimensions give no section"));
        }
        Ok(Measurement::Value {
            lower: value,
            upper: value,
            dimension: Some(dimension),
            locator: format!("{}:{}:{}", call.name(), object.id, profile.family),
        })
    }
}

impl MeasuredProvider for ProfileMeasures {
    fn names(&self) -> &'static [&'static str] {
        NAMES
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let object = context.project.object(object).ok_or_else(|| {
            PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
        })?;
        Self::value(call, object, context).map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{}` of {}: {why}", call.name(), object.id),
            ))
        })
    }
}
