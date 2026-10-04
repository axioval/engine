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

use crate::{
    MEASURED_AREA, MEASURED_BOTTOM, MEASURED_BOTTOM_ABOVE_LEVEL, MEASURED_BOUNDARY_AREA,
    MEASURED_EXTENT_X, MEASURED_EXTENT_Y, MEASURED_EXTENT_Z, MEASURED_LEVEL_HEIGHT, MEASURED_SET,
    MEASURED_TOP, MEASURED_VOLUME, MEASURED_X, MEASURED_Y, MEASURED_Z, QuantityDimension,
};

/// One measured value the engine answers in [`MEASURED_SET`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredDescriptor {
    /// The name, lowercase, without parameters.
    pub name: &'static str,
    /// The parameters it takes, in documentation order.
    pub parameters: &'static [MeasuredParameter],
    /// The dimension of the value; it is answered in the coherent SI unit
    /// ([`MeasuredDescriptor::unit`]).
    pub dimension: QuantityDimension,
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
            QuantityDimension::Length => "m",
            QuantityDimension::Area => "m2",
            QuantityDimension::Volume => "m3",
            QuantityDimension::PlaneAngle => "rad",
            QuantityDimension::Other { .. } => "1",
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

const fn en_de(en: &'static str, de: &'static str) -> [LocalizedText; 2] {
    [
        LocalizedText {
            language: "en",
            text: en,
        },
        LocalizedText {
            language: "de",
            text: de,
        },
    ]
}

const VERTICAL: &[&str] = &["vertical-extent"];
const NO_GEOMETRY: &str = "the object has no body the service can measure";

macro_rules! plain {
    ($name:expr, $dimension:expr, $services:expr, $exactness:expr, $not:expr,
     $label:expr, $help:expr) => {
        MeasuredDescriptor {
            name: $name,
            parameters: &[],
            dimension: $dimension,
            services: $services,
            exactness: $exactness,
            not_evaluated: $not,
            label: &$label,
            help: &$help,
        }
    };
}

/// Every measured value, sorted by name.
pub static MEASURED_VALUES: &[MeasuredDescriptor] = &[
    plain!(
        MEASURED_AREA,
        QuantityDimension::Area,
        &["plan-area"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Footprint area", "Grundfläche"),
        en_de(
            "The area of the object's footprint in plan, overlaps counted once.",
            "Die Fläche des Grundrisses des Objekts, Überlappungen einmal gezählt."
        )
    ),
    plain!(
        MEASURED_BOTTOM,
        QuantityDimension::Length,
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Bottom elevation", "Unterkante"),
        en_de(
            "The elevation of the object's lowest point.",
            "Die Höhe des tiefsten Punkts des Objekts."
        )
    ),
    MeasuredDescriptor {
        name: MEASURED_BOTTOM_ABOVE_LEVEL,
        parameters: &[MeasuredParameter {
            key: "path",
            kind: MeasuredParameterKind::Path,
            required: true,
            default: None,
            help: &en_de(
                "The relationship steps from the object to its level.",
                "Die Beziehungsschritte vom Objekt zu seinem Geschoss.",
            ),
        }],
        dimension: QuantityDimension::Length,
        services: &["relationship-selection", "object-frame", "vertical-extent"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            NO_GEOMETRY,
            "a reached level's placement is not stated exactly",
            "the path reaches levels at different elevations",
        ],
        label: &en_de("Bottom above level", "Unterkante über Geschoss"),
        help: &en_de(
            "The object's bottom above the placement origin of the one level the path \
             reaches; none when it reaches no level.",
            "Die Unterkante des Objekts über dem Ursprung des einen Geschosses, das der \
             Pfad erreicht; keine, wenn er kein Geschoss erreicht.",
        ),
    },
    MeasuredDescriptor {
        name: MEASURED_BOUNDARY_AREA,
        parameters: &[
            MeasuredParameter {
                key: "kind",
                kind: MeasuredParameterKind::SourceKind,
                required: true,
                default: None,
                help: &en_de(
                    "The kind of bounding element, subtypes included.",
                    "Die Art des begrenzenden Bauteils, Untertypen eingeschlossen.",
                ),
            },
            MeasuredParameter {
                key: "plane",
                kind: MeasuredParameterKind::Length { minimum: 0.0 },
                required: false,
                default: Some("0"),
                help: &en_de(
                    "How far from the body's face planes a boundary still counts, in metres.",
                    "Wie weit von den Flächenebenen des Körpers eine Begrenzung noch zählt, \
                     in Metern.",
                ),
            },
        ],
        dimension: QuantityDimension::Area,
        services: &["boundary-coverage", "type-hierarchy"],
        exactness: MeasuredExactness::Measured,
        not_evaluated: &[
            "a boundary names no bounding element",
            "a bounding element's kind cannot be decided",
            NO_GEOMETRY,
        ],
        label: &en_de("Boundary area", "Begrenzungsfläche"),
        help: &en_de(
            "A space's summed space-boundary area against elements of one kind.",
            "Die summierte Raumbegrenzungsfläche eines Raums gegen Bauteile einer Art.",
        ),
    },
    plain!(
        MEASURED_EXTENT_X,
        QuantityDimension::Length,
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Extent along x", "Ausdehnung in x"),
        en_de(
            "The body's extent along the world x axis.",
            "Die Ausdehnung des Körpers entlang der x-Achse."
        )
    ),
    plain!(
        MEASURED_EXTENT_Y,
        QuantityDimension::Length,
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Extent along y", "Ausdehnung in y"),
        en_de(
            "The body's extent along the world y axis.",
            "Die Ausdehnung des Körpers entlang der y-Achse."
        )
    ),
    plain!(
        MEASURED_EXTENT_Z,
        QuantityDimension::Length,
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Height", "Höhe"),
        en_de(
            "The body's vertical extent.",
            "Die vertikale Ausdehnung des Körpers."
        )
    ),
    plain!(
        MEASURED_LEVEL_HEIGHT,
        QuantityDimension::Length,
        &["property-resolution"],
        MeasuredExactness::Stated,
        &["the source does not state storey elevations"],
        en_de("Storey height", "Geschosshöhe"),
        en_de(
            "A storey's height to the next storey of the same spatial parent, as the \
             source states it; none for the highest storey.",
            "Die Höhe eines Geschosses bis zum nächsten Geschoss desselben räumlichen \
             Elternteils, wie die Quelle sie angibt; keine für das oberste Geschoss."
        )
    ),
    plain!(
        MEASURED_TOP,
        QuantityDimension::Length,
        VERTICAL,
        MeasuredExactness::Measured,
        &[NO_GEOMETRY],
        en_de("Top elevation", "Oberkante"),
        en_de(
            "The elevation of the object's highest point.",
            "Die Höhe des höchsten Punkts des Objekts."
        )
    ),
    plain!(
        MEASURED_VOLUME,
        QuantityDimension::Volume,
        &["proximity"],
        MeasuredExactness::Measured,
        &[NO_GEOMETRY, "the body is not closed"],
        en_de("Volume", "Volumen"),
        en_de(
            "The volume the object's body encloses.",
            "Das vom Körper des Objekts umschlossene Volumen."
        )
    ),
    plain!(
        MEASURED_X,
        QuantityDimension::Length,
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin x", "Ursprung x"),
        en_de(
            "The world x coordinate of the object's placement origin.",
            "Die x-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
    plain!(
        MEASURED_Y,
        QuantityDimension::Length,
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin y", "Ursprung y"),
        en_de(
            "The world y coordinate of the object's placement origin.",
            "Die y-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
    plain!(
        MEASURED_Z,
        QuantityDimension::Length,
        &["object-frame"],
        MeasuredExactness::Stated,
        &["the placement is not stated exactly"],
        en_de("Origin z", "Ursprung z"),
        en_de(
            "The world z coordinate of the object's placement origin.",
            "Die z-Koordinate des Platzierungsursprungs des Objekts."
        )
    ),
];

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
        let argument = match parameter.kind {
            MeasuredParameterKind::Path => {
                let steps: Vec<String> = value
                    .split(',')
                    .map(|step| step.trim().to_owned())
                    .collect();
                if steps.iter().any(String::is_empty) {
                    return Err(invalid("a step is empty".into()));
                }
                MeasuredArgument::Path(steps)
            }
            MeasuredParameterKind::SourceKind => {
                if value.is_empty() {
                    return Err(invalid("it is empty".into()));
                }
                MeasuredArgument::SourceKind(value.to_owned())
            }
            MeasuredParameterKind::Length { minimum } => MeasuredArgument::Length(
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|length| length.is_finite() && *length >= minimum)
                    .ok_or_else(|| {
                        invalid(format!("`{value}` is no length of at least {minimum} m"))
                    })?,
            ),
        };
        arguments.insert(parameter.key, argument);
    }
    Ok(MeasuredCall {
        descriptor,
        arguments,
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
            (
                "height",
                "`height` is no measured value; known: area, bottom,",
            ),
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
