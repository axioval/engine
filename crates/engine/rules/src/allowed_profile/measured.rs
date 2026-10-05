//! Profile values: each dimension and slope of a member's swept profile,
//! read as `allowed-profile` reads them, and the section area and elastic
//! section modulus where the profile's family defines them. The profile's
//! type and name are stated, and read as properties of the body set.
//!
//! Section values are computed over intervals rounded outward, so each
//! holds the exact value of the stated dimensions. A radius the formula
//! reads but the source leaves unset is unknown, never zero: the value
//! widens over every radius the profile type allows, and its evidence is no
//! longer exact. A flange slope left unset leaves the value not evaluated.

use axioval_engine::expression::{Interval, IntervalFailure};
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

/// [`needed`], as an interval.
fn stated(
    profile: &Profile,
    column: &str,
    body: &mut BodyFacts<'_>,
) -> Result<Interval, Unavailable> {
    needed(profile, column, body).map(Interval::point)
}

/// Passes when `column` is stated as zero; refused when it is stated as
/// anything else (the formula leaves it out) or unset (it may be anything).
fn zero(profile: &Profile, column: &str, body: &mut BodyFacts<'_>) -> Result<(), Unavailable> {
    match dimension_value(profile, column, body)? {
        Some(0.0) => Ok(()),
        Some(_) => Err(incomplete(format!(
            "the `{}` profile has a {column}, which the section formula leaves out",
            profile.family
        ))),
        None => Err(incomplete(format!(
            "the `{}` profile states no {column}, so the section it shapes is unknown",
            profile.family
        ))),
    }
}

/// The radius `column` of `profile` as stated, or, left unset, every
/// radius from zero to `most`, the largest the profile type allows; and
/// whether it was stated.
fn radius(
    profile: &Profile,
    column: &str,
    most: Interval,
    body: &mut BodyFacts<'_>,
) -> Result<(Interval, bool), Unavailable> {
    Ok(match dimension_value(profile, column, body)? {
        Some(value) => (Interval::point(value), true),
        None => (
            Interval {
                lower: 0.0,
                upper: most.upper.max(0.0),
            },
            false,
        ),
    })
}

/// A section value, and whether it rests on stated dimensions alone.
struct Section {
    value: Interval,
    stated: bool,
}

fn no_section(_: IntervalFailure) -> Unavailable {
    incomplete("the profile's dimensions give no section")
}

fn plus(a: Interval, b: Interval) -> Result<Interval, Unavailable> {
    a.plus(b).map_err(no_section)
}

fn minus(a: Interval, b: Interval) -> Result<Interval, Unavailable> {
    a.minus(b).map_err(no_section)
}

fn times(a: Interval, b: Interval) -> Result<Interval, Unavailable> {
    a.times(b).map_err(no_section)
}

fn over(a: Interval, b: f64) -> Result<Interval, Unavailable> {
    a.divided_by(Interval::point(b)).map_err(no_section)
}

/// π, between the two doubles around it.
fn pi() -> Interval {
    Interval {
        lower: std::f64::consts::PI,
        upper: std::f64::consts::PI.next_up(),
    }
}

/// `4 − π`: four rounded corners of radius `r` take `(4 − π)·r²` from the
/// square corners they replace.
fn corners() -> Result<Interval, Unavailable> {
    minus(Interval::point(4.0), pi())
}

/// The section area of `profile`.
fn area(profile: &Profile, body: &mut BodyFacts<'_>) -> Result<Section, Unavailable> {
    let two = Interval::point(2.0);
    Ok(match profile.family.as_str() {
        "rectangle" => Section {
            value: times(
                stated(profile, "width", body)?,
                stated(profile, "depth", body)?,
            )?,
            stated: true,
        },
        "rectangle-hollow" => {
            let (b, d) = (
                stated(profile, "width", body)?,
                stated(profile, "depth", body)?,
            );
            let wall = times(two, stated(profile, "wall_thickness", body)?)?;
            let (inner_b, inner_d) = (minus(b, wall)?, minus(d, wall)?);
            // The outline's rounded corners take area away, the hole's give
            // it back; each fits half the narrower side.
            let (outer, outer_stated) =
                radius(profile, "outer_fillet_radius", over(b.min(d), 2.0)?, body)?;
            let (inner, inner_stated) = radius(
                profile,
                "fillet_radius",
                over(inner_b.min(inner_d), 2.0)?,
                body,
            )?;
            let rounding = times(
                corners()?,
                minus(times(inner, inner)?, times(outer, outer)?)?,
            )?;
            Section {
                value: plus(minus(times(b, d)?, times(inner_b, inner_d)?)?, rounding)?,
                stated: outer_stated && inner_stated,
            }
        }
        "circle" => {
            let r = stated(profile, "radius", body)?;
            Section {
                value: times(pi(), times(r, r)?)?,
                stated: true,
            }
        }
        "circle-hollow" => {
            let r = stated(profile, "radius", body)?;
            let inner = minus(r, stated(profile, "wall_thickness", body)?)?;
            Section {
                value: times(pi(), minus(times(r, r)?, times(inner, inner)?)?)?,
                stated: true,
            }
        }
        "ellipse" => Section {
            value: times(
                times(pi(), stated(profile, "semi_axis_1", body)?)?,
                stated(profile, "semi_axis_2", body)?,
            )?,
            stated: true,
        },
        "i-shape" => {
            zero(profile, "flange_slope", body)?;
            let (b, d) = (
                stated(profile, "width", body)?,
                stated(profile, "depth", body)?,
            );
            let (tw, tf) = (
                stated(profile, "web_thickness", body)?,
                stated(profile, "flange_thickness", body)?,
            );
            let web_height = minus(d, times(two, tf)?)?;
            let plain = plus(times(times(two, b)?, tf)?, times(web_height, tw)?)?;
            // A fillet fits beside the web within the flange's outstand and
            // within half the web's clear height; a flange edge's rounding
            // within the outstand and the flange's thickness.
            let outstand = over(minus(b, tw)?, 2.0)?;
            let (fillet, fillet_stated) = radius(
                profile,
                "fillet_radius",
                outstand.min(over(web_height, 2.0)?),
                body,
            )?;
            let (edge, edge_stated) = radius(profile, "edge_radius", outstand.min(tf), body)?;
            let rounding = times(
                corners()?,
                minus(times(fillet, fillet)?, times(edge, edge)?)?,
            )?;
            Section {
                value: plus(plain, rounding)?,
                stated: fillet_stated && edge_stated,
            }
        }
        family => {
            return Err(incomplete(format!(
                "a `{family}` profile defines no section area here"
            )));
        }
    })
}

/// The elastic section modulus of `profile` about its strong or weak axis.
fn modulus(
    profile: &Profile,
    strong: bool,
    body: &mut BodyFacts<'_>,
) -> Result<Section, Unavailable> {
    let value = match profile.family.as_str() {
        "rectangle" => {
            let (b, d) = (
                needed(profile, "width", body)?,
                needed(profile, "depth", body)?,
            );
            let (across, deep) = if (d >= b) == strong { (b, d) } else { (d, b) };
            let deep = Interval::point(deep);
            over(times(times(Interval::point(across), deep)?, deep)?, 6.0)?
        }
        "circle" => {
            let r = stated(profile, "radius", body)?;
            over(times(times(times(pi(), r)?, r)?, r)?, 4.0)?
        }
        "circle-hollow" => {
            let r = stated(profile, "radius", body)?;
            let inner = minus(r, stated(profile, "wall_thickness", body)?)?;
            let fourth = |value: Interval| times(times(value, value)?, times(value, value)?);
            let ring = times(pi(), minus(fourth(r)?, fourth(inner)?)?)?;
            ring.divided_by(times(Interval::point(4.0), r)?)
                .map_err(no_section)?
        }
        family => {
            return Err(incomplete(format!(
                "a `{family}` profile defines no section modulus here"
            )));
        }
    };
    Ok(Section {
        value,
        stated: true,
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
        let (section, dimension) = match call.name() {
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
                (
                    Section {
                        value: Interval::point(value),
                        stated: true,
                    },
                    dimension,
                )
            }
            "section_area" => (area(&profile, &mut body)?, QuantityDimension::Area),
            _ => (
                modulus(&profile, call.choice("axis") != Some("weak"), &mut body)?,
                QuantityDimension::Volume,
            ),
        };
        let Interval { lower, upper } = section.value;
        if !(lower.is_finite() && upper.is_finite()) || upper < 0.0 {
            return Err(incomplete("the profile's dimensions give no section"));
        }
        // No section is smaller than nothing, whatever an unset radius allows.
        let lower = lower.max(0.0);
        let locator = format!("{}:{}:{}", call.name(), object.id, profile.family);
        // Stated dimensions are exact, so the interval holds only the
        // arithmetic's rounding; an unset radius makes it an estimate.
        Ok(crate::measured_kinds::interval(
            (lower, upper),
            Some(dimension),
            section.stated,
            locator,
        ))
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
