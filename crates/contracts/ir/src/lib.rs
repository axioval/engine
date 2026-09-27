//! Source-neutral semantic model and declarative package contracts.
#![forbid(unsafe_code)]
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::return_self_not_must_use
)]

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical normalized package contract emitted by `axioval/schema`.
pub mod contract;
pub use contract::{DefinitionPackage, RuleSetPackage};

/// Calendar dates and date-times with a UTC offset.
pub mod temporal;
pub use temporal::{Date, DateTime, TemporalError, TemporalPrecision};

/// Named tables of measured values reported beside findings.
pub mod table;
pub use table::{
    ReportColumn, ReportColumnKind, ReportRow, ReportTable, ReportTableError, ReportValue,
};

/// Validation error for source-neutral contracts.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IrError {
    /// An identity component was blank.
    #[error("{kind} must not be blank")]
    Blank { kind: &'static str },
    /// A project contains an ambiguous identity.
    #[error("duplicate object id: {0}")]
    DuplicateObject(ObjectId),
    /// One object states two identities in the same external scheme.
    #[error("object {object} has more than one `{scheme}` identity")]
    ConflictingExternalId { object: ObjectId, scheme: String },
    /// Two objects of one source claim the same external identity.
    #[error("external id {} is claimed by both {} and {}", .0.id, .0.first, .0.second)]
    DuplicateExternalId(Box<ExternalIdClash>),
    /// A discipline name is not a lowercase token.
    #[error(
        "invalid discipline `{0}`: use 1 to 64 lowercase ASCII letters, digits, `-` or `_`, starting with a letter or digit"
    )]
    InvalidDiscipline(String),
}

/// Two objects of one source claiming one external id, in identity order.
#[derive(Debug, PartialEq, Eq)]
pub struct ExternalIdClash {
    pub id: ExternalId,
    pub first: ObjectId,
    pub second: ObjectId,
}

fn required(value: impl Into<String>, kind: &'static str) -> Result<String, IrError> {
    let value = value.into();
    if value.trim().is_empty() {
        Err(IrError::Blank { kind })
    } else {
        Ok(value)
    }
}

/// Stable, source-qualified input identity.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceId {
    pub system: String,
    pub document: String,
}
impl SourceId {
    /// Creates a source identity.
    pub fn new(system: impl Into<String>, document: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            system: required(system, "source system")?,
            document: required(document, "source document")?,
        })
    }
}
impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.system, self.document)
    }
}

/// Stable identity of an object within a source.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectId {
    pub source: SourceId,
    pub local_id: String,
}
impl ObjectId {
    /// Creates a source-qualified object identity.
    pub fn new(source: SourceId, local_id: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            source,
            local_id: required(local_id, "object local id")?,
        })
    }
}
impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.source, self.local_id)
    }
}

/// An identity an object also carries in a scheme outside this engine.
///
/// An alias, never a replacement: the engine keys everything on [`ObjectId`].
/// External ids exist so output formats and cross-revision tools can name an
/// object the way other software does. The scheme is an adapter-defined label;
/// the IR attaches no meaning to it beyond uniqueness within one source.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalId {
    pub scheme: String,
    pub value: String,
}
impl ExternalId {
    /// Creates an external identity.
    pub fn new(scheme: impl Into<String>, value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            scheme: required(scheme, "external id scheme")?,
            value: required(value, "external id value")?,
        })
    }
}
impl fmt::Display for ExternalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.scheme, self.value)
    }
}

/// The discipline a source plays in a check, such as `architecture` or
/// `structure`.
///
/// A host declaration about a source, never read from it: IFC carries no
/// discipline. The name is a lowercase token (`[a-z0-9][a-z0-9_-]{0,63}`), so
/// two spellings of one discipline cannot silently differ by case or
/// whitespace, and it compares exactly. The engine attaches no vocabulary;
/// hosts and packages agree on the names.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Discipline(String);
impl Discipline {
    /// Longest accepted discipline name, in bytes.
    pub const MAX_LEN: usize = 64;
    /// Validates a discipline name.
    pub fn new(name: impl Into<String>) -> Result<Self, IrError> {
        let name = name.into();
        let mut bytes = name.bytes();
        let valid = name.len() <= Self::MAX_LEN
            && bytes
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            });
        if valid {
            Ok(Self(name))
        } else {
            Err(IrError::InvalidDiscipline(name))
        }
    }
    /// The discipline name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Discipline {
    type Error = IrError;
    fn try_from(name: String) -> Result<Self, IrError> {
        Self::new(name)
    }
}
impl From<Discipline> for String {
    fn from(discipline: Discipline) -> Self {
        discipline.0
    }
}
impl fmt::Display for Discipline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Physical dimension of a canonical SI quantity.
///
/// The value of a quantity is always in the coherent SI unit of its
/// dimension: metres, square metres, cubic metres, radians, and for
/// [`QuantityDimension::Other`] the product of SI base units its exponents
/// name (kilogram, second, kelvin, ...). Two quantities compare only when
/// their dimensions are equal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantityDimension {
    Length,
    Area,
    Volume,
    /// Radians. Dimensionless in SI, but never compared with a plain ratio.
    PlaneAngle,
    /// Any other dimension, as SI base-unit exponents in the order length,
    /// mass, time, electric current, temperature, amount of substance,
    /// luminous intensity. A thermal transmittance in W/(m²·K) is
    /// `[0, 1, -3, 0, -1, 0, 0]`.
    Other {
        exponents: [i8; 7],
    },
}

impl QuantityDimension {
    /// The dimension with these SI base-unit exponents, named when it has a name.
    ///
    /// All-zero exponents are not a dimension: a dimensionless value is a
    /// plain number, and a plane angle must be named explicitly.
    #[must_use]
    pub fn from_exponents(exponents: [i8; 7]) -> Option<Self> {
        Some(match exponents {
            [0, 0, 0, 0, 0, 0, 0] => return None,
            [1, 0, 0, 0, 0, 0, 0] => Self::Length,
            [2, 0, 0, 0, 0, 0, 0] => Self::Area,
            [3, 0, 0, 0, 0, 0, 0] => Self::Volume,
            exponents => Self::Other { exponents },
        })
    }

    /// The coherent SI unit symbol, e.g. `m²` or `kg·s⁻³·K⁻¹`.
    #[must_use]
    pub fn unit_symbol(self) -> String {
        const BASE: [&str; 7] = ["m", "kg", "s", "A", "K", "mol", "cd"];
        let exponents = match self {
            Self::Length => return "m".into(),
            Self::Area => return "m²".into(),
            Self::Volume => return "m³".into(),
            Self::PlaneAngle => return "rad".into(),
            Self::Other { exponents } => exponents,
        };
        let superscript = |digit: char| match digit {
            '-' => '⁻',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            _ => '⁰',
        };
        BASE.iter()
            .zip(exponents)
            .filter(|(_, exponent)| *exponent != 0)
            .map(|(base, exponent)| {
                if exponent == 1 {
                    (*base).to_owned()
                } else {
                    format!(
                        "{base}{}",
                        exponent
                            .to_string()
                            .chars()
                            .map(superscript)
                            .collect::<String>()
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("·")
    }
}

/// A value supplied by a source adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PropertyValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Decimal(f64),
    Quantity {
        value: f64,
        dimension: QuantityDimension,
    },
    String(String),
    /// A calendar day without a time zone, `YYYY-MM-DD` on the wire.
    Date(Date),
    /// An instant with the UTC offset it was stated in,
    /// `YYYY-MM-DDThh:mm:ss[.f]±hh:mm` (or `Z`) on the wire, tagged
    /// `dateTime` like the camelCase package kinds. A date-time without an
    /// offset is not representable.
    #[serde(rename = "dateTime")]
    DateTime(DateTime),
    /// Several values of one property, in the order the source states them
    /// (the presentation layers of an object). Elements are scalar values:
    /// never `Null` and never a nested list. A comparison against a list
    /// states whether any or every element must satisfy it; a list is never
    /// compared as if it were one of its elements.
    List(Vec<PropertyValue>),
}

/// Provenance and exactness of evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: SourceId,
    pub locator: String,
    pub exact: bool,
}
impl Evidence {
    /// Creates evidence asserted exact by its adapter.
    pub fn exact(source: SourceId, locator: impl Into<String>) -> Self {
        Self {
            source,
            locator: locator.into(),
            exact: true,
        }
    }
}

/// A namespace/code classification.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub system: String,
    pub code: String,
}
impl Classification {
    /// Creates a classification.
    pub fn new(system: impl Into<String>, code: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            system: required(system, "classification system")?,
            code: required(code, "classification code")?,
        })
    }
}

/// Property set that names an object's own intrinsic attributes.
///
/// Sources describe an object partly through named property sets and partly
/// through fields of the object itself: its name, its long name, its type
/// label. A property request in this set asks for such a field by the
/// source's own attribute name (`Name`, `LongName` for IFC), so rules can
/// check both through one resolver. It is reserved: it names no property set
/// of any source, and package concept binding passes it through unchanged.
pub const ATTRIBUTE_SET: &str = "axioval:attributes";

/// Property set that names the attributes of an object's type object.
///
/// Where a source types its occurrences by a shared type object (a door
/// type, a space type), this set reads that object's attributes: `Name` in
/// this set is the construction type name. An object with no type is exactly
/// absent; an object with several is a conflict, not a choice. Reserved like
/// [`ATTRIBUTE_SET`].
pub const TYPE_ATTRIBUTE_SET: &str = "axioval:type-attributes";

/// Property set that names how an object is presented in its source.
///
/// Reserved like [`ATTRIBUTE_SET`]. Property names are matched ignoring
/// ASCII case:
///
/// - [`PRESENTATION_LAYER`] lists the names of the presentation (CAD) layers
///   the object's shape is assigned to, as a [`PropertyValue::List`] of
///   strings: every distinct layer, sorted. An object on no layer has none
///   (an exact absence), unless its source assigns no layer to any object at
///   all: then absence says nothing about the object, and the source answers
///   that it records no layers ([`NotEvaluatedReason::NotRecorded`]).
/// - [`PRESENTATION_TRANSPARENCY`] lists how transparent the object's styled
///   surfaces are, as a [`PropertyValue::List`] of decimals from `0.0`
///   (opaque) to `1.0` (fully transparent): every distinct value, ascending.
///   A comparison states whether `any` or `all` surfaces must satisfy it. An
///   object with no styled surface has none (an exact absence).
pub const PRESENTATION_SET: &str = "axioval:presentation";

/// The layer list property in [`PRESENTATION_SET`].
pub const PRESENTATION_LAYER: &str = "Layer";

/// The surface transparency list property in [`PRESENTATION_SET`].
pub const PRESENTATION_TRANSPARENCY: &str = "Transparency";

/// Property set that names the material an object is made of.
///
/// The material is the object's own, or else the one its type object
/// carries. An object with no material has none of these properties (an exact
/// absence); an object with several material assignments is a conflict.
/// Reserved like [`ATTRIBUTE_SET`]. Property names are matched ignoring ASCII
/// case:
///
/// - [`MATERIAL_KIND`]: how the material is composed, one of
///   [`MATERIAL_KIND_SINGLE`], [`MATERIAL_KIND_LAYER_SET`],
///   [`MATERIAL_KIND_CONSTITUENT_SET`], [`MATERIAL_KIND_PROFILE_SET`] or
///   [`MATERIAL_KIND_LIST`].
/// - [`MATERIAL_NAME`]: the name of a single material, or of the set.
/// - [`MATERIAL_CATEGORY`]: the category of a single material.
/// - [`MATERIAL_TOTAL_THICKNESS`]: the summed layer thickness of a layer set,
///   a length.
/// - [`MATERIAL_COUNT`]: the number of layers, constituents, profiles or
///   listed materials.
/// - Members, numbered from 1 in the source's order: `Layer<n>.Material`,
///   `Layer<n>.Thickness` (a length), `Layer<n>.Name` and `Layer<n>.Category`;
///   `Constituent<n>.Material`, `Constituent<n>.Name`,
///   `Constituent<n>.Category` and `Constituent<n>.Fraction` (a decimal);
///   `Profile<n>.Material`, `Profile<n>.Name` and `Profile<n>.Category`;
///   `Material<n>.Name` and `Material<n>.Category` for a list. `.Material` is
///   the name of the member's material.
/// - [`MATERIAL_NAMES`]: every name the material goes by, as a
///   [`PropertyValue::List`] of strings, distinct and sorted: the material's
///   or the set's name and category, and each member's name and category and
///   its material's name and category. Empty names are left out. A selector
///   asks whether `any` of them is a given name without enumerating members.
pub const MATERIAL_SET: &str = "axioval:material";

/// The composition property in [`MATERIAL_SET`].
pub const MATERIAL_KIND: &str = "Kind";
/// The name property in [`MATERIAL_SET`].
pub const MATERIAL_NAME: &str = "Name";
/// The category property in [`MATERIAL_SET`].
pub const MATERIAL_CATEGORY: &str = "Category";
/// The total layer thickness property in [`MATERIAL_SET`].
pub const MATERIAL_TOTAL_THICKNESS: &str = "TotalThickness";
/// The member count property in [`MATERIAL_SET`].
pub const MATERIAL_COUNT: &str = "Count";
/// The list of every name and category in [`MATERIAL_SET`].
pub const MATERIAL_NAMES: &str = "Names";
/// [`MATERIAL_KIND`] of one homogeneous material.
pub const MATERIAL_KIND_SINGLE: &str = "material";
/// [`MATERIAL_KIND`] of a set of layers with thicknesses.
pub const MATERIAL_KIND_LAYER_SET: &str = "layer-set";
/// [`MATERIAL_KIND`] of a set of named constituents.
pub const MATERIAL_KIND_CONSTITUENT_SET: &str = "constituent-set";
/// [`MATERIAL_KIND`] of a set of materials with cross-section profiles.
pub const MATERIAL_KIND_PROFILE_SET: &str = "profile-set";
/// [`MATERIAL_KIND`] of an unstructured list of materials.
pub const MATERIAL_KIND_LIST: &str = "list";

/// Whether `set` is one of the reserved sets, which bind to themselves.
#[must_use]
pub fn is_reserved_set(set: &str) -> bool {
    set == ATTRIBUTE_SET
        || set == TYPE_ATTRIBUTE_SET
        || set == PRESENTATION_SET
        || set == MATERIAL_SET
}

/// A named semantic property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub property_set: String,
    pub name: String,
    pub value: PropertyValue,
    /// The value's type as the source declares it, in the source's own
    /// vocabulary (an IFC property: `IFCLABEL`, `IFCLENGTHMEASURE`). `None`
    /// when the source declares none or the adapter does not report it,
    /// which is never evidence of any particular type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    pub evidence: Option<Evidence>,
}
impl Property {
    /// Creates a property without provenance.
    pub fn new(
        property_set: impl Into<String>,
        name: impl Into<String>,
        value: PropertyValue,
    ) -> Result<Self, IrError> {
        Ok(Self {
            property_set: required(property_set, "property set")?,
            name: required(name, "property name")?,
            value,
            data_type: None,
            evidence: None,
        })
    }
    /// Records the value's type as the source declares it.
    ///
    /// # Errors
    ///
    /// Returns an error when `data_type` is blank.
    pub fn with_data_type(mut self, data_type: impl Into<String>) -> Result<Self, IrError> {
        self.data_type = Some(required(data_type, "property data type")?);
        Ok(self)
    }
    /// Attaches source evidence.
    pub fn with_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence = Some(evidence);
        self
    }
    /// The value's type as the source declares it, if reported.
    pub fn data_type(&self) -> Option<&str> {
        self.data_type.as_deref()
    }
    /// Returns the typed property value.
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

/// A source-neutral object and its semantic facts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: ObjectId,
    pub kind: String,
    /// Aliases in external schemes, at most one per scheme, sorted by scheme.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_ids: Vec<ExternalId>,
    pub properties: Vec<Property>,
    pub classifications: Vec<Classification>,
    pub relationships: BTreeMap<String, Vec<ObjectId>>,
}
impl Object {
    /// Creates an object.
    pub fn new(id: ObjectId, kind: impl Into<String>) -> Self {
        Self {
            id,
            kind: kind.into(),
            external_ids: vec![],
            properties: vec![],
            classifications: vec![],
            relationships: BTreeMap::new(),
        }
    }
    /// Adds an external identity, keeping `external_ids` sorted by scheme.
    ///
    /// A second identity in the same scheme is kept and rejected when the
    /// object enters a [`Project`], so the conflict cannot pass unnoticed.
    pub fn with_external_id(mut self, id: ExternalId) -> Self {
        let at = self.external_ids.partition_point(|held| held <= &id);
        self.external_ids.insert(at, id);
        self
    }
    /// The object's identity in `scheme`, if the source states one.
    pub fn external_id(&self, scheme: &str) -> Option<&str> {
        self.external_ids
            .iter()
            .find(|id| id.scheme == scheme)
            .map(|id| id.value.as_str())
    }
    /// Adds a property.
    pub fn with_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }
    /// Adds a classification.
    pub fn with_classification(mut self, classification: Classification) -> Self {
        self.classifications.push(classification);
        self
    }
    /// Object semantic kind.
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// Finds a property by namespace and name.
    pub fn property(&self, set: &str, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.property_set == set && p.name == name)
    }
}

/// Deterministically indexed source-neutral project graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    objects: BTreeMap<ObjectId, Object>,
}
impl Project {
    /// Builds a project, rejecting ambiguous IDs.
    ///
    /// Ambiguity covers external ids too: an object with two ids in one
    /// scheme, or two objects of one source sharing an external id, would
    /// make any consumer that resolves the alias pick one silently.
    pub fn new(objects: Vec<Object>) -> Result<Self, IrError> {
        let mut result = Self::default();
        let mut claimed: BTreeMap<(&SourceId, &ExternalId), &ObjectId> = BTreeMap::new();
        for object in &objects {
            // Not a sorted-neighbour check: deserialized objects need not be sorted.
            let mut schemes = std::collections::BTreeSet::new();
            if let Some(id) = object
                .external_ids
                .iter()
                .find(|id| !schemes.insert(id.scheme.as_str()))
            {
                return Err(IrError::ConflictingExternalId {
                    object: object.id.clone(),
                    scheme: id.scheme.clone(),
                });
            }
            for id in &object.external_ids {
                if let Some(first) = claimed.insert((&object.id.source, id), &object.id) {
                    let (first, second) = if first <= &object.id {
                        (first, &object.id)
                    } else {
                        (&object.id, first)
                    };
                    return Err(IrError::DuplicateExternalId(Box::new(ExternalIdClash {
                        id: id.clone(),
                        first: first.clone(),
                        second: second.clone(),
                    })));
                }
            }
        }
        for object in objects {
            if result
                .objects
                .insert(object.id.clone(), object.clone())
                .is_some()
            {
                return Err(IrError::DuplicateObject(object.id));
            }
        }
        Ok(result)
    }
    /// Finds an object by source-qualified ID.
    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }
    /// Iterates objects in stable identity order.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }
}

/// A source-neutral object selector.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    pub kinds: Vec<String>,
    pub classification: Option<Classification>,
}
impl Selector {
    /// Selects a semantic kind.
    pub fn by_kind(kind: impl Into<String>) -> Self {
        Self {
            kinds: vec![kind.into()],
            classification: None,
        }
    }
    /// Requires a classification.
    pub fn with_classification(
        mut self,
        system: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        self.classification = Classification::new(system, code).ok();
        self
    }
    /// Whether an object matches all selector terms.
    pub fn matches(&self, object: &Object) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|k| k == &object.kind))
            && self
                .classification
                .as_ref()
                .is_none_or(|c| object.classifications.contains(c))
    }
}

/// Stable package-local rule identity.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(String);
impl RuleId {
    /// Creates an ID.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(required(value, "rule id")?))
    }
}
impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Severity of a validation finding.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}
/// What a finding or not-evaluated outcome is about.
///
/// Most outcomes are about one object. Some are about a whole source ("this
/// model has no building") or the whole project ("no storey anywhere has a
/// fire compartment"): there is no object to report them against, and
/// reporting nothing would read as a pass.
///
/// Ordered project first, then sources, then objects, each by identity, so
/// report ordering stays deterministic.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Scope {
    /// The whole project, every source together.
    Project,
    /// One source as a whole.
    Source(SourceId),
    /// One object.
    Object(ObjectId),
}

impl Scope {
    /// The object this scope names, if it names one.
    #[must_use]
    pub fn object(&self) -> Option<&ObjectId> {
        match self {
            Self::Object(object) => Some(object),
            Self::Project | Self::Source(_) => None,
        }
    }
    /// The source this scope lies in: the named source, or the object's own
    /// source. `None` for the project.
    #[must_use]
    pub fn source(&self) -> Option<&SourceId> {
        match self {
            Self::Project => None,
            Self::Source(source) => Some(source),
            Self::Object(object) => Some(&object.source),
        }
    }
    /// Splits the scope into its wire fields, `object_id` and `source`; at
    /// most one is set.
    fn into_wire(self) -> (Option<ObjectId>, Option<SourceId>) {
        match self {
            Self::Project => (None, None),
            Self::Source(source) => (None, Some(source)),
            Self::Object(object) => (Some(object), None),
        }
    }
    fn from_wire(object_id: Option<ObjectId>, source: Option<SourceId>) -> Result<Self, String> {
        match (object_id, source) {
            (None, None) => Ok(Self::Project),
            (None, Some(source)) => Ok(Self::Source(source)),
            (Some(object), None) => Ok(Self::Object(object)),
            (Some(object), Some(source)) => Err(format!(
                "outcome names both object {object} and source {source}; an object already names its source"
            )),
        }
    }
}

impl From<ObjectId> for Scope {
    fn from(object: ObjectId) -> Self {
        Self::Object(object)
    }
}

impl From<SourceId> for Scope {
    fn from(source: SourceId) -> Self {
        Self::Source(source)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Project => f.write_str("project"),
            Self::Source(source) => write!(f, "source {source}"),
            Self::Object(object) => object.fmt(f),
        }
    }
}

/// A deterministic, source-qualified validation outcome.
///
/// On the wire an object finding carries `object_id`, a source finding
/// `source`, and a project finding neither; a record with both is rejected.
/// An object finding therefore serializes exactly as before scopes existed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "FindingWire", into = "FindingWire")]
pub struct Finding {
    pub rule_id: RuleId,
    /// What the finding is reported against: an object, a source, or the
    /// project. Evidence rules do not depend on it: every finding carries
    /// the exact source evidence that decided it.
    pub scope: Scope,
    pub severity: Severity,
    pub message: String,
    /// Other objects that participate in the finding -- the slab a wall rests
    /// on, the body a space intersects, the objects a count found.
    ///
    /// A finding a reviewer cannot act on is a finding that gets ignored:
    /// "this wall has insufficient contact" is only useful alongside *what*
    /// it fails to rest on. Empty when the subject alone explains the finding.
    pub related: Vec<ObjectId>,
    pub evidence: Vec<Evidence>,
}

impl Finding {
    /// A finding with no related objects and no evidence yet.
    #[must_use]
    pub fn new(
        rule_id: RuleId,
        scope: impl Into<Scope>,
        severity: Severity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id,
            scope: scope.into(),
            severity,
            message: message.into(),
            related: Vec::new(),
            evidence: Vec::new(),
        }
    }
    /// The object the finding is reported against, if it is about one.
    #[must_use]
    pub fn object_id(&self) -> Option<&ObjectId> {
        self.scope.object()
    }
    /// Attaches evidence, sorted by source and locator and deduplicated so
    /// ordering never depends on evaluation order.
    #[must_use]
    pub fn with_evidence(mut self, evidence: impl IntoIterator<Item = Evidence>) -> Self {
        self.evidence = evidence.into_iter().collect();
        self.evidence
            .sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
        self.evidence.dedup();
        self
    }
    /// Attaches the other objects that participate in this finding, sorted and
    /// deduplicated so ordering never depends on adapter traversal order.
    #[must_use]
    pub fn with_related(mut self, related: impl IntoIterator<Item = ObjectId>) -> Self {
        self.related = related.into_iter().collect();
        self.related.sort();
        self.related.dedup();
        // The subject is already named by the scope; repeating it adds noise.
        if let Scope::Object(subject) = &self.scope {
            let subject = subject.clone();
            self.related.retain(|candidate| candidate != &subject);
        }
        self
    }
}

/// The serialized form of a [`Finding`], compatible with reports written
/// before scopes existed.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingWire {
    rule_id: RuleId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<SourceId>,
    severity: Severity,
    message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

impl From<Finding> for FindingWire {
    fn from(finding: Finding) -> Self {
        let (object_id, source) = finding.scope.into_wire();
        Self {
            rule_id: finding.rule_id,
            object_id,
            source,
            severity: finding.severity,
            message: finding.message,
            related: finding.related,
            evidence: finding.evidence,
        }
    }
}

impl TryFrom<FindingWire> for Finding {
    type Error = String;
    fn try_from(wire: FindingWire) -> Result<Self, String> {
        Ok(Self {
            rule_id: wire.rule_id,
            scope: Scope::from_wire(wire.object_id, wire.source)?,
            severity: wire.severity,
            message: wire.message,
            related: wire.related,
            evidence: wire.evidence,
        })
    }
}
/// Why an object or rule instance could not be evaluated conclusively.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotEvaluatedReason {
    MissingService,
    BackendUnavailable,
    IncompleteEvidence,
    InvalidEvidence,
    InvalidDeclaration,
    ResourceLimit,
    /// The package names a concept the source's declared vocabulary cannot
    /// express. A fact about the package and the source, never about one
    /// object, so the runtime reports it once per rule and source.
    UnboundConcept,
    /// The source records the consulted kind of fact for no object at all
    /// (a model with no presentation layers), so neither a value nor an
    /// absence can be stated and the rule does not apply to that source. A
    /// fact about the source, never about one object, so the runtime reports
    /// it once per rule and source.
    NotRecorded,
}
/// Explicit fail-closed evaluation outcome. This is not a compliance finding.
///
/// On the wire `object_id` is always written (`null` unless the outcome is
/// about one object) and `source` only for a source-scoped outcome, so object-
/// and rule-level outcomes serialize exactly as before scopes existed.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "NotEvaluatedWire", into = "NotEvaluatedWire")]
pub struct NotEvaluated {
    pub rule_id: RuleId,
    /// What could not be evaluated: one object, one source, or the rule as a
    /// whole (`Scope::Project`).
    pub scope: Scope,
    pub reason: NotEvaluatedReason,
    pub message: String,
}

impl NotEvaluated {
    /// The object that could not be evaluated, if the outcome is about one.
    #[must_use]
    pub fn object_id(&self) -> Option<&ObjectId> {
        self.scope.object()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotEvaluatedWire {
    rule_id: RuleId,
    #[serde(default)]
    object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<SourceId>,
    reason: NotEvaluatedReason,
    message: String,
}

impl From<NotEvaluated> for NotEvaluatedWire {
    fn from(outcome: NotEvaluated) -> Self {
        let (object_id, source) = outcome.scope.into_wire();
        Self {
            rule_id: outcome.rule_id,
            object_id,
            source,
            reason: outcome.reason,
            message: outcome.message,
        }
    }
}

impl TryFrom<NotEvaluatedWire> for NotEvaluated {
    type Error = String;
    fn try_from(wire: NotEvaluatedWire) -> Result<Self, String> {
        Ok(Self {
            rule_id: wire.rule_id,
            scope: Scope::from_wire(wire.object_id, wire.source)?,
            reason: wire.reason,
            message: wire.message,
        })
    }
}
/// Ordered report from a plan execution.
///
/// `tables` holds the measured values rules report beside their findings,
/// ordered by rule and table name. It is omitted from the serialized form
/// when empty, so a report without tables serializes byte for byte as
/// before tables existed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub not_evaluated: Vec<NotEvaluated>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<ReportTable>,
}
impl Report {
    /// Findings in deterministic order.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Fail-closed rule or object evaluations in deterministic order.
    pub fn not_evaluated(&self) -> &[NotEvaluated] {
        &self.not_evaluated
    }
    /// Tables of measured values, by rule and table name.
    pub fn tables(&self) -> &[ReportTable] {
        &self.tables
    }
    /// The table `name` of `rule_id`, if the report has it.
    pub fn table(&self, rule_id: &RuleId, name: &str) -> Option<&ReportTable> {
        self.tables
            .iter()
            .find(|table| table.rule_id() == rule_id && table.name() == name)
    }
}
