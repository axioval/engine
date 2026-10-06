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
//!
//! A parameter whose kind takes one ([`MeasuredParameterKind::reference`](crate::measured::MeasuredParameterKind::reference))
//! may instead name a parameter of the rule reading the value, written
//! `@name` (`shelf_length;doors=@door_selector`), and an
//! [`Objects`](crate::measured::MeasuredParameterKind::Objects) parameter the anchor,
//! `@anchor`: the object the rule checks, whose members a value may be
//! read on. The registry parses a reference as written
//! ([`Parameter`](crate::measured::MeasuredArgument::Parameter), [`Anchor`](crate::measured::MeasuredArgument::Anchor)); the
//! rule reading the value binds it ([`MeasuredCall::bind`](crate::measured::MeasuredCall::bind)) before
//! anything is measured, and a reference it cannot bind leaves the value
//! not evaluated, never a default.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde::ser::SerializeStruct;

mod members;
mod registry;

pub use members::{
    MEASURED_MEMBERS, MemberDescriptor, MemberField, MemberFieldKind, is_member_field,
    member_descriptor, members_of, parse_members,
};

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
/// The `face` that selects the pieces facing a direction.
pub const FACING: &str = "facing";
/// A face's pieces, one by one: the measured member list.
pub const FACE_PIECES: &str = "face_pieces";

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
///
/// Serialized with `references`, the kind of rule parameter `@name` may
/// name in its place, where its kind takes one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredParameter {
    pub key: &'static str,
    pub kind: MeasuredParameterKind,
    /// Whether the name must state it.
    pub required: bool,
    /// The value an optional parameter takes when not stated, as written.
    pub default: Option<&'static str>,
    pub help: &'static [LocalizedText],
}

impl Serialize for MeasuredParameter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let reference = self.kind.reference();
        let fields = 4 + usize::from(self.default.is_some()) + usize::from(reference.is_some());
        let mut state = serializer.serialize_struct("MeasuredParameter", fields)?;
        state.serialize_field("key", self.key)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("required", &self.required)?;
        if let Some(default) = self.default {
            state.serialize_field("default", default)?;
        }
        state.serialize_field("help", self.help)?;
        if let Some(reference) = reference {
            state.serialize_field("references", &reference)?;
        }
        state.end()
    }
}

/// The rule parameter a measured parameter's `@name` may name: what the
/// compiler requires the rule to state, and what the rule's value binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterReference {
    /// A `number` or `integer` of metres, or a `quantity` of length.
    Length,
    /// A `stringList` of relationship steps.
    StringList,
    /// A `string`.
    String,
    /// A `selector`, bound to the objects it picks; or `@anchor`.
    Selector,
    /// A `propertyReference`, bound to the property it names.
    Property,
    /// A `table`, bound to its rows as stated.
    Table,
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
    /// A property the object states, written `set/name` or `name`.
    Property,
    /// A word naming something the source declares, such as a discipline,
    /// as written.
    Text,
    /// A simple polygon in a section plane: at least three `lateral:up`
    /// vertices in metres, `,`-separated, enclosing an area and never
    /// crossing or touching itself.
    Polygon,
    /// The objects measured against: source kinds, `,`-separated (subtypes
    /// match), the objects a selector parameter of the rule picks
    /// (`@name`), or the anchor the rule checks (`@anchor`).
    Objects,
    /// A table of the rule reading the value, named only as `@name`: its
    /// rows as the rule states them.
    Table,
}

impl MeasuredParameterKind {
    /// The rule parameter `@name` may name in place of a value of this
    /// kind; `None` where a value must be written.
    #[must_use]
    pub const fn reference(self) -> Option<ParameterReference> {
        match self {
            Self::Length { .. } => Some(ParameterReference::Length),
            Self::Path => Some(ParameterReference::StringList),
            Self::Choice { .. } | Self::Text | Self::SourceKind => Some(ParameterReference::String),
            Self::Objects => Some(ParameterReference::Selector),
            Self::Property => Some(ParameterReference::Property),
            Self::Table => Some(ParameterReference::Table),
            Self::Vector | Self::Polygon => None,
        }
    }
}

/// The reference naming the anchor in an
/// [`Objects`](MeasuredParameterKind::Objects) parameter, `@anchor`.
pub const ANCHOR: &str = "anchor";

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

    /// Every argument still naming a rule parameter or the anchor, by key,
    /// in key order.
    pub fn references(&self) -> impl Iterator<Item = (&'static str, &MeasuredArgument)> {
        self.arguments
            .iter()
            .filter(|(_, argument)| argument.is_reference())
            .map(|(key, argument)| (*key, argument))
    }

    /// Whether every reference is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.references().next().is_none()
    }

    /// Binds the reference of `key` to `argument`, a value of the
    /// parameter's kind: what the rule's parameter (or anchor) states.
    ///
    /// # Errors
    ///
    /// `key` is no reference of the call, or `argument` is a reference or
    /// not of the parameter's kind.
    pub fn bind(&mut self, key: &str, argument: MeasuredArgument) -> Result<(), MeasuredError> {
        let invalid = |detail: &str| MeasuredError::Invalid {
            name: self.descriptor.name.to_owned(),
            key: key.to_owned(),
            detail: detail.to_owned(),
        };
        let Some(parameter) = self.descriptor.parameter(key) else {
            return Err(invalid("the value takes no such parameter"));
        };
        if !self
            .arguments
            .get(parameter.key)
            .is_some_and(MeasuredArgument::is_reference)
        {
            return Err(invalid("it names no rule parameter to bind"));
        }
        let fits = matches!(
            (parameter.kind, &argument),
            (
                MeasuredParameterKind::Length { .. },
                MeasuredArgument::Length(_)
            ) | (MeasuredParameterKind::Path, MeasuredArgument::Path(_))
                | (
                    MeasuredParameterKind::Choice { .. },
                    MeasuredArgument::Choice(_)
                )
                | (MeasuredParameterKind::Text, MeasuredArgument::Text(_))
                | (
                    MeasuredParameterKind::SourceKind | MeasuredParameterKind::Objects,
                    MeasuredArgument::SourceKind(_)
                )
                | (MeasuredParameterKind::Objects, MeasuredArgument::Objects(_))
                | (
                    MeasuredParameterKind::Property,
                    MeasuredArgument::Property { .. }
                )
                | (MeasuredParameterKind::Table, MeasuredArgument::Table(_))
        );
        if !fits {
            return Err(invalid("the bound value is not of the parameter's kind"));
        }
        self.arguments.insert(parameter.key, argument);
        Ok(())
    }

    /// The parameter `key` declares.
    #[must_use]
    pub fn parameter(&self, key: &str) -> Option<&'static MeasuredParameter> {
        self.descriptor.parameter(key)
    }
}

/// The objects a reference picked, bound into a measured call: the objects
/// a rule's selector parameter surely picks and those it cannot decide,
/// each sorted by source-qualified identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasuredSelection {
    /// The rule parameter it was bound from, or [`ANCHOR`].
    pub parameter: String,
    /// The objects surely picked.
    pub matched: BTreeSet<crate::ObjectId>,
    /// The objects that may or may not be picked.
    pub undecided: BTreeSet<crate::ObjectId>,
}

impl MeasuredSelection {
    /// The anchor alone, surely picked.
    #[must_use]
    pub fn anchor(anchor: crate::ObjectId) -> Self {
        Self {
            parameter: ANCHOR.to_owned(),
            matched: BTreeSet::from([anchor]),
            undecided: BTreeSet::new(),
        }
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
    /// A word, as written.
    Text(String),
    /// A property, by its set (when written) and name.
    Property {
        /// The property set, `None` when only the name is written.
        set: Option<String>,
        /// The property's name.
        name: String,
    },
    /// A simple polygon's `(lateral, up)` vertices, as written.
    Polygon(Vec<[f64; 2]>),
    /// A rule parameter named in the value's place, `@name`, not yet bound.
    Parameter(String),
    /// The anchor, `@anchor`, not yet bound.
    Anchor,
    /// The objects a reference picked, bound.
    Objects(MeasuredSelection),
    /// The rows of a table a reference named, bound.
    Table(Vec<crate::contract::TableRow>),
}

impl MeasuredArgument {
    /// Whether it still names a rule parameter or the anchor.
    #[must_use]
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Parameter(_) | Self::Anchor)
    }

    /// The reference as written, `@name` or `@anchor`.
    #[must_use]
    pub fn written(&self) -> Option<String> {
        match self {
            Self::Parameter(name) => Some(format!("@{name}")),
            Self::Anchor => Some(format!("@{ANCHOR}")),
            _ => None,
        }
    }
}

/// Why a name is no measured value.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeasuredError {
    #[error("`{name}` is no {what}; known: {known}")]
    Unknown {
        name: String,
        what: &'static str,
        known: String,
    },
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

pub use registry::{MEASURED_VALUES, en_de};

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
    parse_in(text, MEASURED_VALUES.iter(), "measured value")
}

/// Parses `name[;key=value…]` against `descriptors`, which name `what`.
fn parse_in(
    text: &str,
    descriptors: impl Iterator<Item = &'static MeasuredDescriptor> + Clone,
    what: &'static str,
) -> Result<MeasuredCall, MeasuredError> {
    let mut parts = text.split(';');
    let base = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let descriptor = descriptors
        .clone()
        .find(|descriptor| descriptor.name == base)
        .ok_or_else(|| MeasuredError::Unknown {
            name: base.clone(),
            what,
            known: descriptors
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
    facing(descriptor, &arguments)?;
    Ok(MeasuredCall {
        descriptor,
        arguments,
    })
}

/// Where a value's `face` may face a direction, `face=facing` states its
/// `direction` and a `tolerance` of at most 180 degrees, and no other face
/// states either.
fn facing(
    descriptor: &MeasuredDescriptor,
    arguments: &BTreeMap<&'static str, MeasuredArgument>,
) -> Result<(), MeasuredError> {
    let offered = descriptor.parameter("face").is_some_and(|face| {
        matches!(face.kind, MeasuredParameterKind::Choice { options } if options.contains(&FACING))
    });
    if !offered {
        return Ok(());
    }
    let name = || descriptor.name.to_owned();
    let facing = matches!(
        arguments.get("face"),
        Some(MeasuredArgument::Choice(FACING))
    );
    for key in ["direction", "tolerance"] {
        match (facing, arguments.contains_key(key)) {
            (true, false) => {
                return Err(MeasuredError::Missing {
                    name: name(),
                    key: key.to_owned(),
                });
            }
            (false, true) => {
                return Err(MeasuredError::Invalid {
                    name: name(),
                    key: key.to_owned(),
                    detail: "only `face=facing` takes it".into(),
                });
            }
            _ => {}
        }
    }
    if let Some(MeasuredArgument::Length(degrees)) = arguments.get("tolerance")
        && *degrees > 180.0
    {
        return Err(MeasuredError::Invalid {
            name: name(),
            key: "tolerance".into(),
            detail: format!("{degrees} degrees is more than 180"),
        });
    }
    Ok(())
}

/// The reference `value` (`@name`) states for a parameter of `kind`: the
/// anchor, or the rule parameter `name`, or why it is none.
fn reference(
    kind: MeasuredParameterKind,
    value: &str,
    name: &str,
) -> Result<MeasuredArgument, String> {
    if name == ANCHOR {
        if kind == MeasuredParameterKind::Objects {
            return Ok(MeasuredArgument::Anchor);
        }
        return Err(format!(
            "`{value}` names the anchor, which only objects measured against take"
        ));
    }
    if kind.reference().is_none() {
        return Err(format!(
            "`{value}` names a rule parameter, which it never takes"
        ));
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(format!("`{value}` names no rule parameter"));
    }
    Ok(MeasuredArgument::Parameter(name.to_owned()))
}

/// The argument `value` states for a parameter of `kind`, or why it is
/// none.
fn argument(kind: MeasuredParameterKind, value: &str) -> Result<MeasuredArgument, String> {
    if let Some(name) = value.strip_prefix('@') {
        return reference(kind, value, name.trim());
    }
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
        MeasuredParameterKind::SourceKind | MeasuredParameterKind::Objects => {
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
        MeasuredParameterKind::Text => {
            if value.is_empty() {
                return Err("it is empty".into());
            }
            MeasuredArgument::Text(value.to_owned())
        }
        MeasuredParameterKind::Property => {
            let (set, name) = match value.split_once('/') {
                Some((set, name)) => (Some(set.trim().to_owned()), name.trim().to_owned()),
                None => (None, value.trim().to_owned()),
            };
            if name.is_empty() || set.as_ref().is_some_and(String::is_empty) {
                return Err(format!("`{value}` is no `set/name` property"));
            }
            MeasuredArgument::Property { set, name }
        }
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
        MeasuredParameterKind::Polygon => {
            let vertices: Vec<[f64; 2]> = value
                .split(',')
                .map(|vertex| {
                    let (lateral, up) = vertex.split_once(':')?;
                    let lateral = lateral.trim().parse::<f64>().ok()?;
                    let up = up.trim().parse::<f64>().ok()?;
                    Some([lateral, up])
                })
                .collect::<Option<_>>()
                .ok_or_else(|| {
                    format!("`{value}` is no polygon of `lateral:up` vertices, `,`-separated")
                })?;
            if let Some(problem) = polygon_problem(&vertices) {
                return Err(format!("`{value}` is no simple polygon: {problem}"));
            }
            MeasuredArgument::Polygon(vertices)
        }
        MeasuredParameterKind::Table => {
            return Err(format!(
                "`{value}` is no table; a table is named only as a rule parameter, `@name`"
            ));
        }
    })
}

/// Why `vertices` is no simple polygon (at least three finite vertices,
/// enclosing an area, no edge crossing or touching another but its two
/// neighbours at their shared vertex), or `None` when it is one.
#[must_use]
pub fn polygon_problem(vertices: &[[f64; 2]]) -> Option<String> {
    let count = vertices.len();
    if count < 3 {
        return Some(format!("it has {count} vertices, fewer than three"));
    }
    if vertices.iter().flatten().any(|value| !value.is_finite()) {
        return Some("a coordinate is not finite".into());
    }
    let twice_area: f64 = (0..count)
        .map(|i| {
            let ([ax, ay], [bx, by]) = (vertices[i], vertices[(i + 1) % count]);
            ax * by - bx * ay
        })
        .sum();
    if twice_area == 0.0 || !twice_area.is_finite() {
        return Some("it encloses no area".into());
    }
    let edge = |i: usize| (vertices[i], vertices[(i + 1) % count]);
    for i in 0..count {
        let (a, b) = edge(i);
        if a[0].to_bits() == b[0].to_bits() && a[1].to_bits() == b[1].to_bits() {
            return Some(format!(
                "vertex {} repeats the one before it",
                (i + 1) % count + 1
            ));
        }
        for j in i + 1..count {
            let adjacent = j == i + 1 || (i == 0 && j == count - 1);
            let (c, d) = edge(j);
            if adjacent {
                // Neighbours share one vertex; they may not fold back
                // onto each other.
                let (shared, other_a, other_b) = if j == i + 1 { (b, a, d) } else { (a, b, c) };
                let cross = (other_a[0] - shared[0]) * (other_b[1] - shared[1])
                    - (other_a[1] - shared[1]) * (other_b[0] - shared[0]);
                let dot = (other_a[0] - shared[0]) * (other_b[0] - shared[0])
                    + (other_a[1] - shared[1]) * (other_b[1] - shared[1]);
                if cross == 0.0 && dot > 0.0 {
                    return Some(format!("edges {} and {} overlap", i + 1, j + 1));
                }
            } else if segments_meet(a, b, c, d) {
                return Some(format!("edges {} and {} cross or touch", i + 1, j + 1));
            }
        }
    }
    None
}

/// Whether the closed segments `ab` and `cd` share a point.
fn segments_meet(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        let value = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    };
    let within = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        r[0] >= p[0].min(q[0])
            && r[0] <= p[0].max(q[0])
            && r[1] >= p[1].min(q[1])
            && r[1] <= p[1].max(q[1])
    };
    let (o1, o2, o3, o4) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    if o1 * o2 < 0 && o3 * o4 < 0 {
        return true;
    }
    (o1 == 0 && within(a, b, c))
        || (o2 == 0 && within(a, b, d))
        || (o3 == 0 && within(c, d, a))
        || (o4 == 0 && within(c, d, b))
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

    const SHELVING: &str = "shelf_length;depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;\
                            top=2;clearance=0.9;access=bounds:forward";

    #[test]
    fn a_parameter_named_with_at_is_a_reference_the_rule_binds() {
        let mut call = parse(&format!(
            "{SHELVING};doors=@door_selector;openings=@anchor;spaces=@space_selector"
        ))
        .unwrap();
        assert_eq!(
            call.argument("doors"),
            Some(&MeasuredArgument::Parameter("door_selector".into()))
        );
        assert_eq!(call.argument("openings"), Some(&MeasuredArgument::Anchor));
        assert_eq!(
            call.references().map(|(key, _)| key).collect::<Vec<_>>(),
            ["doors", "openings", "spaces"]
        );
        assert!(!call.is_bound());
        // Bound only to a value of the parameter's kind.
        assert!(
            call.bind("doors", MeasuredArgument::Text("x".into()))
                .is_err()
        );
        assert!(call.bind("access", MeasuredArgument::Length(1.0)).is_err());
        call.bind("spaces", MeasuredArgument::SourceKind("IfcSpace".into()))
            .unwrap();
        let depth = parse(&format!("{SHELVING};depth=@depth_metres"));
        assert!(depth.is_err(), "`depth` is stated twice");
        let door = crate::ObjectId::new(crate::SourceId::new("a", "b").unwrap(), "d").unwrap();
        for key in ["doors", "openings"] {
            call.bind(
                key,
                MeasuredArgument::Objects(MeasuredSelection::anchor(door.clone())),
            )
            .unwrap();
        }
        assert!(call.is_bound());
        // Literal source kinds still read as kinds.
        let call = parse(&format!("{SHELVING};doors=IfcDoor")).unwrap();
        assert_eq!(
            call.argument("doors"),
            Some(&MeasuredArgument::SourceKind("IfcDoor".into()))
        );
    }

    #[test]
    fn a_property_of_the_rule_binds_as_stated() {
        let mut call =
            parse("door_clear_width;stated=@clear_width;overall=Attributes/OverallWidth").unwrap();
        assert_eq!(
            call.argument("stated"),
            Some(&MeasuredArgument::Parameter("clear_width".into()))
        );
        assert_eq!(
            call.argument("overall"),
            Some(&MeasuredArgument::Property {
                set: Some("Attributes".into()),
                name: "OverallWidth".into()
            })
        );
        // Bound only to a property.
        assert!(
            call.bind("stated", MeasuredArgument::Text("x".into()))
                .is_err()
        );
        call.bind(
            "stated",
            MeasuredArgument::Property {
                set: None,
                name: "ClearWidth".into(),
            },
        )
        .unwrap();
        assert!(call.is_bound());
    }

    #[test]
    fn a_reference_is_refused_where_its_kind_takes_none() {
        for (name, message) in [
            (
                "slope;face=facing;direction=@axis;tolerance=10".to_owned(),
                "`slope` parameter `direction`: `@axis` names a rule parameter",
            ),
            (
                format!("{SHELVING};doors=@"),
                "`shelf_length` parameter `doors`: `@` names no rule parameter",
            ),
            (
                "contact_area;with=IfcSlab;gap=@anchor".to_owned(),
                "`contact_area` parameter `gap`: `@anchor` names the anchor",
            ),
            (
                "face_pieces;face=@face".to_owned(),
                "`face_pieces` parameter `face`: `@face` names a rule parameter, which a \
                 member list never takes",
            ),
        ] {
            let error = if name.starts_with(FACE_PIECES) {
                parse_members(&name)
            } else {
                parse(&name)
            };
            let error = error.map(|call| call.arguments).unwrap_err().to_string();
            assert!(error.starts_with(message), "{name}: {error}");
        }
    }

    #[test]
    fn a_parameter_lists_the_reference_it_takes() {
        let call = parse(SHELVING).unwrap();
        let json = |key: &str| serde_json::to_value(call.parameter(key).unwrap()).unwrap();
        assert_eq!(json("doors")["references"], "selector");
        assert_eq!(json("doors")["kind"]["type"], "objects");
        assert_eq!(json("depth")["references"], "length");
        assert_eq!(json("access")["references"], "stringList");
        let direction = parse("slope;face=facing;direction=1,0,0;tolerance=10").unwrap();
        assert!(
            serde_json::to_value(direction.parameter("direction").unwrap())
                .unwrap()
                .get("references")
                .is_none()
        );
    }

    #[test]
    fn a_face_facing_a_direction_states_its_direction_and_tolerance() {
        let call = parse("slope;face=Facing;direction=1,0,1;tolerance=30").unwrap();
        assert_eq!(call.choice("face"), Some(FACING));
        assert_eq!(
            call.argument("direction"),
            Some(&MeasuredArgument::Vector([1.0, 0.0, 1.0]))
        );
        let members = parse_members("face_pieces;face=facing;direction=0,0,1;tolerance=180");
        assert!(members.is_ok(), "{members:?}");
        assert!(parse("cross_fall;axis=x;face=bottom").is_ok());
        for (name, message) in [
            (
                "slope;face=facing;tolerance=10",
                "`slope` needs `direction`",
            ),
            (
                "gradient_direction;face=facing;direction=1,0,0",
                "`gradient_direction` needs `tolerance`",
            ),
            (
                "slope;direction=1,0,0;tolerance=10",
                "`slope` parameter `direction`: only `face=facing` takes it",
            ),
            (
                "face_pieces;face=facing;direction=1,0,0;tolerance=181",
                "`face_pieces` parameter `tolerance`: 181 degrees is more than 180",
            ),
            (
                "slope_along;direction=x;face=facing",
                "`slope_along` parameter `face`",
            ),
        ] {
            let error = if name.starts_with(FACE_PIECES) {
                parse_members(name)
            } else {
                parse(name)
            }
            .unwrap_err()
            .to_string();
            assert!(error.starts_with(message), "{name}: {error}");
        }
    }

    #[test]
    fn a_clearance_envelope_is_a_simple_polygon_of_lateral_up_vertices() {
        let call = parse(
            "envelope_intrusions;bodies=IfcWall;envelope=-2:0, 2:0,2:5,-2:5;from=0;to=10;step=1",
        )
        .unwrap();
        assert_eq!(
            call.argument("envelope"),
            Some(&MeasuredArgument::Polygon(vec![
                [-2.0, 0.0],
                [2.0, 0.0],
                [2.0, 5.0],
                [-2.0, 5.0]
            ]))
        );
        for (envelope, reason) in [
            ("0:0,1:0", "fewer than three"),
            ("0:0,1:0,2:0", "encloses no area"),
            ("0:0,3:2,3:0,0:3", "cross or touch"),
            ("0:0,1:0,1:1,1:0.5", "overlap"),
            ("0:0;1:0,1:1", "not `key=value`"),
            ("0:0,1,1:1", "no polygon"),
            ("0:0,1:0,inf:1", "not finite"),
        ] {
            let name = format!(
                "envelope_intrusions;bodies=IfcWall;envelope={envelope};from=0;to=1;step=1"
            );
            let error = parse(&name).unwrap_err().to_string();
            assert!(error.contains(reason), "{envelope}: {error}");
        }
        assert!(polygon_problem(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]).is_none());
        // A step of zero is refused when the name is read.
        assert!(
            parse("envelope_intrusions;bodies=IfcWall;envelope=0:0,1:0,1:1;from=0;to=1;step=0")
                .is_err()
        );
    }
}
