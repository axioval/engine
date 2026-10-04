//! The registry of measured values: every name of [`MEASURED_SET`] with
//! its typed parameters, result, the service it needs, its exactness and
//! what leaves it not evaluated.
//!
//! The registry is the one place a measured name is parsed and validated.
//! The engine resolves only what [`parse`](crate::measured::parse) accepts, the compiler refuses a
//! rule reading anything else with the registry's message, and catalogues
//! for editors are emitted from the same descriptors.
//!
//! A name is written `name[;key=value…]`, matched ignoring ASCII case, its
//! parameter keys too.
use std::collections::BTreeMap;

use serde::Serialize;

mod registry;

/// The nearest or farthest counterpart's distance.
pub const DISTANCE: &str = "distance";
/// How many counterparts lie within a radius.
pub const COUNT_WITHIN: &str = "count_within";
/// The least vertical clearance above a walking surface.
pub const HEADROOM: &str = "headroom";
/// The least clearance below a flight or ramp over the floors beneath.
pub const CLEARANCE_BELOW: &str = "clearance_below";
/// The narrowest clear width along a flight or ramp.
pub const CLEAR_WIDTH: &str = "clear_width";
/// A space's clear height.
pub const CLEAR_HEIGHT: &str = "clear_height";
/// The body's extent along an own axis or a direction.
pub const EXTENT: &str = "extent";
/// A member's length along its own axis.
pub const LENGTH: &str = "length";
/// How thick the body is along a direction or across a face.
pub const THICKNESS: &str = "thickness";
/// The length of the footprint's boundary.
pub const PERIMETER: &str = "perimeter";
/// The angle between the object and the objects a path reaches.
pub const ANGLE_TO: &str = "angle_to";
/// The plan bearing of an axis from north.
pub const BEARING: &str = "bearing";
/// How far a long axis is from square to a reference's.
pub const SKEW: &str = "skew";
/// The steepest gradient of a face, as an angle.
pub const SLOPE: &str = "slope";
/// A face's gradient in a plan direction, as a signed angle.
pub const SLOPE_ALONG: &str = "slope_along";
/// A face's gradient across a plan axis, unsigned, as an angle.
pub const CROSS_FALL: &str = "cross_fall";
/// The tilt of an own axis of the object's placement.
pub const INCLINATION: &str = "inclination";
/// The plan bearing of a face's steepest descent.
pub const GRADIENT_DIRECTION: &str = "gradient_direction";

use crate::{MEASURED_SET, QuantityDimension};

/// One measured value the engine answers in [`MEASURED_SET`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredDescriptor {
    /// The name, lowercase, without parameters.
    pub name: &'static str,
    /// The parameters it takes, in documentation order.
    pub parameters: &'static [MeasuredParameter],
    /// The dimension of the value; it is answered in the coherent SI unit
    /// ([`MeasuredDescriptor::unit`]). `None` for a plain number, such as
    /// a count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<QuantityDimension>,
    /// The services a run needs to answer it, by their service names.
    pub services: &'static [&'static str],
    /// How exact an answer can be.
    pub exactness: MeasuredExactness,
    /// What leaves it not evaluated for an object, in plain words.
    pub not_evaluated: &'static [&'static str],
    /// A short name for editors.
    pub label: &'static [LocalizedText],
    /// What it measures.
    pub help: &'static [LocalizedText],
}

impl MeasuredDescriptor {
    /// The coherent SI unit the value is answered in.
    #[must_use]
    pub fn unit(&self) -> &'static str {
        match self.dimension {
            Some(QuantityDimension::Length) => "m",
            Some(QuantityDimension::Area) => "m2",
            Some(QuantityDimension::Volume) => "m3",
            Some(QuantityDimension::PlaneAngle) => "rad",
            Some(QuantityDimension::Other { .. }) | None => "1",
        }
    }

    /// The parameter `key`, if the value takes it.
    #[must_use]
    pub fn parameter(&self, key: &str) -> Option<&'static MeasuredParameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.key == key)
    }
}

/// A text in one language, by BCP 47 tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LocalizedText {
    pub language: &'static str,
    pub text: &'static str,
}

/// One parameter of a measured value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredParameter {
    pub key: &'static str,
    pub kind: MeasuredParameterKind,
    /// Whether the name must state it.
    pub required: bool,
    /// The value an optional parameter takes when not stated, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<&'static str>,
    pub help: &'static [LocalizedText],
}

/// What a measured parameter's value is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MeasuredParameterKind {
    /// Relationship steps, `,`-separated, each written as a `related`
    /// selector's step. The engine parses the steps.
    Path,
    /// A source kind, such as an entity name; subtypes match.
    SourceKind,
    /// A length in metres, at least `minimum`.
    Length { minimum: f64 },
    /// One of `options`, matched ignoring ASCII case.
    Choice { options: &'static [&'static str] },
    /// A direction in world coordinates, written `x,y,z`, not zero.
    Vector,
}

/// How exact a measured value can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasuredExactness {
    /// As the source states it: exact, or not answered.
    Stated,
    /// Measured on geometry: a point with exact evidence when the service
    /// certifies it, otherwise an interval sure to hold the exact value
    /// (a tessellated body's is never a point).
    Measured,
}

/// A measured name with its arguments, validated against its descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredCall {
    pub descriptor: &'static MeasuredDescriptor,
    /// Every parameter the descriptor declares that the name states or
    /// defaults, by key.
    pub arguments: BTreeMap<&'static str, MeasuredArgument>,
}

impl MeasuredCall {
    /// The argument of `key`, if stated or defaulted.
    #[must_use]
    pub fn argument(&self, key: &str) -> Option<&MeasuredArgument> {
        self.arguments.get(key)
    }

    /// The option chosen for `key`, if it is a choice stated or defaulted.
    #[must_use]
    pub fn choice(&self, key: &str) -> Option<&'static str> {
        match self.arguments.get(key)? {
            MeasuredArgument::Choice(option) => Some(option),
            _ => None,
        }
    }

    /// The name of the value measured, as the registry spells it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.descriptor.name
    }
}

/// One parsed argument of a measured name.
#[derive(Clone, Debug, PartialEq)]
pub enum MeasuredArgument {
    /// Relationship steps as written, in order, trimmed.
    Path(Vec<String>),
    /// A source kind, as written.
    SourceKind(String),
    /// A length in metres.
    Length(f64),
    /// The option chosen, as the registry spells it.
    Choice(&'static str),
    /// A direction's components, as written.
    Vector([f64; 3]),
}

/// Why a name is no measured value.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeasuredError {
    #[error("`{name}` is no measured value; known: {known}")]
    Unknown { name: String, known: String },
    #[error("`{part}` of `{name}` is not `key=value`")]
    NotKeyValue { name: String, part: String },
    #[error("`{name}` states `{key}` twice")]
    Repeated { name: String, key: String },
    #[error("`{name}` takes no parameter `{key}`{}", takes(.accepted))]
    UnknownParameter {
        name: String,
        key: String,
        accepted: String,
    },
    #[error("`{name}` needs `{key}`")]
    Missing { name: String, key: String },
    #[error("`{name}` parameter `{key}`: {detail}")]
    Invalid {
        name: String,
        key: String,
        detail: String,
    },
}

fn takes(accepted: &str) -> String {
    if accepted.is_empty() {
        String::new()
    } else {
        format!("; it takes {accepted}")
    }
}

pub use registry::MEASURED_VALUES;

/// The descriptor of `name` (without parameters), ignoring ASCII case.
#[must_use]
pub fn descriptor(name: &str) -> Option<&'static MeasuredDescriptor> {
    MEASURED_VALUES
        .iter()
        .find(|descriptor| descriptor.name.eq_ignore_ascii_case(name.trim()))
}

/// Parses `name[;key=value…]` of [`MEASURED_SET`] against the registry.
///
/// # Errors
///
/// An unknown name, a parameter stated twice, not taken or not given when
/// required, or a value of the wrong kind.
pub fn parse(text: &str) -> Result<MeasuredCall, MeasuredError> {
    let mut parts = text.split(';');
    let base = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let descriptor = descriptor(&base).ok_or_else(|| MeasuredError::Unknown {
        name: base.clone(),
        known: MEASURED_VALUES
            .iter()
            .map(|descriptor| descriptor.name)
            .collect::<Vec<_>>()
            .join(", "),
    })?;
    let name = || descriptor.name.to_owned();
    let mut stated = BTreeMap::new();
    for part in parts {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| MeasuredError::NotKeyValue {
                name: name(),
                part: part.to_owned(),
            })?;
        let key = key.trim().to_ascii_lowercase();
        let parameter =
            descriptor
                .parameter(&key)
                .ok_or_else(|| MeasuredError::UnknownParameter {
                    name: name(),
                    key: key.clone(),
                    accepted: descriptor
                        .parameters
                        .iter()
                        .map(|parameter| format!("`{}`", parameter.key))
                        .collect::<Vec<_>>()
                        .join(", "),
                })?;
        if stated.insert(parameter.key, value.trim()).is_some() {
            return Err(MeasuredError::Repeated { name: name(), key });
        }
    }
    let mut arguments = BTreeMap::new();
    for parameter in descriptor.parameters {
        let Some(value) = stated.get(parameter.key).copied().or(parameter.default) else {
            if parameter.required {
                return Err(MeasuredError::Missing {
                    name: name(),
                    key: parameter.key.to_owned(),
                });
            }
            continue;
        };
        let invalid = |detail: String| MeasuredError::Invalid {
            name: name(),
            key: parameter.key.to_owned(),
            detail,
        };
        let argument = argument(parameter.kind, value).map_err(invalid)?;
        arguments.insert(parameter.key, argument);
    }
    Ok(MeasuredCall {
        descriptor,
        arguments,
    })
}

/// The argument `value` states for a parameter of `kind`, or why it is
/// none.
fn argument(kind: MeasuredParameterKind, value: &str) -> Result<MeasuredArgument, String> {
    Ok(match kind {
        MeasuredParameterKind::Path => {
            let steps: Vec<String> = value
                .split(',')
                .map(|step| step.trim().to_owned())
                .collect();
            if steps.iter().any(String::is_empty) {
                return Err("a step is empty".into());
            }
            MeasuredArgument::Path(steps)
        }
        MeasuredParameterKind::SourceKind => {
            if value.is_empty() {
                return Err("it is empty".into());
            }
            MeasuredArgument::SourceKind(value.to_owned())
        }
        MeasuredParameterKind::Length { minimum } => MeasuredArgument::Length(
            value
                .parse::<f64>()
                .ok()
                .filter(|length| length.is_finite() && *length >= minimum)
                .ok_or_else(|| format!("`{value}` is no length of at least {minimum} m"))?,
        ),
        MeasuredParameterKind::Choice { options } => MeasuredArgument::Choice(
            options
                .iter()
                .find(|option| option.eq_ignore_ascii_case(value))
                .ok_or_else(|| format!("`{value}` is none of {}", options.join(", ")))?,
        ),
        MeasuredParameterKind::Vector => {
            let components: Vec<f64> = value
                .split(',')
                .map(|component| component.trim().parse::<f64>())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("`{value}` is no `x,y,z` direction"))?;
            match components[..] {
                [x, y, z]
                    if [x, y, z].iter().all(|c| c.is_finite())
                        && [x, y, z].iter().any(|c| *c != 0.0) =>
                {
                    MeasuredArgument::Vector([x, y, z])
                }
                _ => return Err(format!("`{value}` is no `x,y,z` direction")),
            }
        }
    })
}

/// Whether `set` and `name` read a measured value the registry accepts.
#[must_use]
pub fn is_measured(set: &str, name: &str) -> bool {
    set == MEASURED_SET && parse(name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_sorted_and_names_are_unique_and_lowercase() {
        let names: Vec<_> = MEASURED_VALUES.iter().map(|d| d.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
        assert!(names.iter().all(|name| *name == name.to_ascii_lowercase()));
    }

    #[test]
    fn every_descriptor_is_labelled_in_english_and_german() {
        for descriptor in MEASURED_VALUES {
            for texts in [descriptor.label, descriptor.help]
                .into_iter()
                .chain(descriptor.parameters.iter().map(|p| p.help))
            {
                let languages: Vec<_> = texts.iter().map(|t| t.language).collect();
                assert_eq!(languages, ["en", "de"], "{}", descriptor.name);
            }
            assert!(!descriptor.services.is_empty(), "{}", descriptor.name);
            assert!(!descriptor.not_evaluated.is_empty(), "{}", descriptor.name);
        }
    }

    #[test]
    fn every_earlier_name_resolves_through_the_registry() {
        for name in crate::MEASURED_NAMES {
            let call = parse(&name.to_ascii_uppercase()).unwrap();
            assert_eq!(call.descriptor.name, name);
            assert!(call.arguments.is_empty());
        }
        let call = parse("Boundary_Area; Kind = IfcWall").unwrap();
        assert_eq!(
            call.argument("kind"),
            Some(&MeasuredArgument::SourceKind("IfcWall".into()))
        );
        assert_eq!(call.argument("plane"), Some(&MeasuredArgument::Length(0.0)));
        let call = parse("bottom_above_level;path=A:backward, B+").unwrap();
        assert_eq!(
            call.argument("path"),
            Some(&MeasuredArgument::Path(vec![
                "A:backward".into(),
                "B+".into()
            ]))
        );
    }

    #[test]
    fn wrong_names_and_parameters_are_refused_with_a_reason() {
        for (name, message) in [
            ("height", "`height` is no measured value; known: "),
            ("extent_x;path=a", "`extent_x` takes no parameter `path`"),
            (
                "bottom_above_level;path=a;PATH=b",
                "`bottom_above_level` states `path` twice",
            ),
            (
                "bottom_above_level;path=a,,b",
                "`bottom_above_level` parameter `path`: a step",
            ),
            (
                "boundary_area;kind=",
                "`boundary_area` parameter `kind`: it is empty",
            ),
            (
                "boundary_area;kind=w;plane=x",
                "`boundary_area` parameter `plane`: `x` is no",
            ),
            ("area;flat", "`flat` of `area` is not `key=value`"),
        ] {
            let error = parse(name).unwrap_err().to_string();
            assert!(error.starts_with(message), "{name}: {error}");
        }
    }
}
