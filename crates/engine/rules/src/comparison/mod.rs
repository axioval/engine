//! Comparison of two revisions of a model.
//!
//! Two sessions are compared through [`compare_sessions`], a host entry
//! point; two sources of one session through the `model-comparison`
//! capability ([`CompareModels`]), which runs beside other rules.
//!
//! Objects are matched across revisions by an ordered chain of
//! [`Matcher`]s, never by [`ObjectId`]: an object's local id is whatever its
//! source file numbered it, and a re-export renumbers everything. A matcher
//! is an external identity scheme, or a property each side states; the
//! engine attaches no meaning to a scheme, so an IFC session supplies
//! `GlobalId`s under its adapter's scheme, another source its own.
//!
//! The semantic facets (kind, classifications, properties, relationships) are
//! always compared; whole property sets are listed through each revision's
//! property enumeration when the request names them. The spatial facets are opt-in, each with its tolerance,
//! and read typed host services only:
//!
//! - **placement** compares object frames ([`ObjectFrameServiceHandle`]): the
//!   distance between origins and the rotation between axis triples;
//! - **geometry** compares measured extents ([`ProximityServiceHandle`]):
//!   the largest shift of any face of the axis-aligned bounds;
//! - **coordinate systems** compare each pair of sources
//!   ([`CoordinateSystemServiceHandle`]): world frame, true north and map
//!   conversion.
//!
//! A measurement is an interval. It is changed when its whole interval lies
//! above the tolerance, unchanged when its whole interval lies within it, and
//! otherwise **undetermined**, never rounded either way.
//!
//! Nothing is silently dropped. An object no matcher can key cannot be
//! matched and is reported as unidentified. An identity held by two objects
//! of one revision is ambiguous and matches nothing, and an identity that
//! cannot be read leaves its object, and every object it might match,
//! undecided. A facet that
//! cannot be read exactly on both sides -- a property resolver error, a
//! relationship target without an identity, a missing service or an
//! unmeasured body -- is reported unresolved rather than compared as if it
//! were empty.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{EvidenceSession, RuleContext};
use axioval_ir::contract::SourceField;
use axioval_ir::{
    Evidence, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, PropertyValue, Report,
    RuleId, Scope, Severity, SourceId,
};

mod capability;
mod facets;
mod matching;

pub use capability::CompareModels;

/// A declaration the comparison cannot run with.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComparisonError {
    /// The identity scheme is blank.
    #[error("comparison identity scheme must not be blank")]
    BlankScheme,
    /// A requested property name or set is blank.
    #[error("compared property names must not be blank")]
    BlankProperty,
    /// A tolerance is negative or not finite.
    #[error("comparison tolerances must be finite and non-negative")]
    InvalidTolerance,
    /// No matcher is given.
    #[error("a comparison needs at least one matcher")]
    NoMatcher,
    /// An overlap ratio is not above zero and at most one.
    #[error("an overlap ratio must lie above 0 and at most 1")]
    InvalidRatio,
    /// A related matcher names no relationship path.
    #[error("a related matcher needs a relationship path")]
    EmptyPath,
}

/// How far a spatial facet may move before it counts as changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComparisonTolerance {
    length_metres: f64,
    angle_radians: f64,
}

impl ComparisonTolerance {
    /// A tolerance for lengths, in metres, and angles, in radians.
    ///
    /// # Errors
    ///
    /// Returns an error when either is negative or not finite.
    pub fn try_new(length_metres: f64, angle_radians: f64) -> Result<Self, ComparisonError> {
        let valid = |value: f64| value.is_finite() && value >= 0.0;
        if !valid(length_metres) || !valid(angle_radians) {
            return Err(ComparisonError::InvalidTolerance);
        }
        Ok(Self {
            length_metres,
            angle_radians,
        })
    }
    /// Largest length difference that counts as unchanged, in metres.
    #[must_use]
    pub fn length_metres(&self) -> f64 {
        self.length_metres
    }
    /// Largest angle difference that counts as unchanged, in radians.
    #[must_use]
    pub fn angle_radians(&self) -> f64 {
        self.angle_radians
    }
}

/// One property compared through each session's property resolver.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComparedProperty {
    property_set: Option<String>,
    name: String,
}

impl ComparedProperty {
    /// A property by name, optionally within a set.
    ///
    /// # Errors
    ///
    /// Returns an error when the name or a given set is blank.
    pub fn new(property_set: Option<&str>, name: &str) -> Result<Self, ComparisonError> {
        if name.trim().is_empty() || property_set.is_some_and(|set| set.trim().is_empty()) {
            return Err(ComparisonError::BlankProperty);
        }
        Ok(Self {
            property_set: property_set.map(str::to_owned),
            name: name.to_owned(),
        })
    }
    /// Property set, or `None` for a property unambiguous by name.
    pub fn property_set(&self) -> Option<&str> {
        self.property_set.as_deref()
    }
    /// Property name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for ComparedProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.property_set {
            Some(set) => write!(f, "{set}.{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// One way of telling that an object of the base revision and one of the
/// revised revision are the same object.
#[derive(Clone, Debug, PartialEq)]
pub enum Matcher {
    /// The same identity in an external identity scheme, such as the IFC
    /// adapter's `GlobalId` scheme.
    Scheme(String),
    /// The same value of a property: `base` read on the base revision,
    /// `revised` on the revised one, such as a door number. Values are
    /// compared exactly; an absent, null or blank value is no key.
    Property {
        /// The property read on the base revision.
        base: ComparedProperty,
        /// The property read on the revised revision.
        revised: ComparedProperty,
    },
    /// The same kind and the same body: the certified Hausdorff distance
    /// between the two surfaces within `tolerance_metres`. Needs both
    /// revisions in one session, so their bodies can be measured together.
    Geometry {
        /// The largest distance between the surfaces, in metres.
        tolerance_metres: f64,
    },
    /// As [`Matcher::Geometry`], and placed alike: frame origins within the
    /// length tolerance and axes within the angle tolerance, or both
    /// unplaced.
    Placement {
        /// The tolerance for the frames and, in length, the surfaces.
        tolerance: ComparisonTolerance,
    },
    /// The same kind and a shared volume at least `minimum_ratio` of the
    /// larger body's, certified.
    Overlap {
        /// The least share of the larger body's volume, above 0, at most 1.
        minimum_ratio: f64,
    },
    /// The same kind, each reaching exactly one object along `path` (a door
    /// its opening), the two reached objects already matched or their
    /// surfaces coinciding within `tolerance_metres`.
    Related {
        /// The relationship steps, as a traversal `path` names them.
        path: Vec<String>,
        /// The largest distance between the reached objects' surfaces.
        tolerance_metres: f64,
    },
}

impl Matcher {
    /// How reports name the matcher: the scheme, or `property` and the
    /// property (both, when they differ).
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Scheme(scheme) => scheme.clone(),
            Self::Property { base, revised } if base == revised => format!("property {base}"),
            Self::Property { base, revised } => format!("property {base}/{revised}"),
            Self::Geometry { .. } => "geometry".to_owned(),
            Self::Placement { .. } => "placement".to_owned(),
            Self::Overlap { .. } => "overlap".to_owned(),
            Self::Related { path, .. } => format!("related {}", path.join(" > ")),
        }
    }

    fn validate(&self) -> Result<(), ComparisonError> {
        let path_given = |matcher: &Self| match matcher {
            Self::Related { path, .. } => path.iter().any(|step| !step.trim().is_empty()),
            _ => true,
        };
        let length = |value: f64| {
            if value.is_finite() && value >= 0.0 {
                Ok(())
            } else {
                Err(ComparisonError::InvalidTolerance)
            }
        };
        match self {
            Self::Scheme(scheme) if scheme.trim().is_empty() => Err(ComparisonError::BlankScheme),
            Self::Scheme(_) | Self::Property { .. } | Self::Placement { .. } => Ok(()),
            Self::Geometry { tolerance_metres }
            | Self::Related {
                tolerance_metres, ..
            } if path_given(self) => length(*tolerance_metres),
            Self::Overlap { minimum_ratio } if *minimum_ratio > 0.0 && *minimum_ratio <= 1.0 => {
                Ok(())
            }
            Self::Overlap { .. } => Err(ComparisonError::InvalidRatio),
            Self::Geometry { .. } | Self::Related { .. } => Err(ComparisonError::EmptyPath),
        }
    }
}

/// What to compare and how to match objects.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonRequest {
    matchers: Vec<Matcher>,
    properties: BTreeSet<ComparedProperty>,
    property_sets: BTreeSet<String>,
    all_property_sets: bool,
    timestamps: bool,
    placement: Option<ComparisonTolerance>,
    geometry: Option<ComparisonTolerance>,
    coordinate_systems: Option<ComparisonTolerance>,
}

impl ComparisonRequest {
    /// Matches objects by their identity in `scheme`.
    ///
    /// # Errors
    ///
    /// Returns an error when `scheme` is blank.
    pub fn new(scheme: impl Into<String>) -> Result<Self, ComparisonError> {
        Self::matching(vec![Matcher::Scheme(scheme.into())])
    }

    /// Matches objects by each of `matchers` in turn, each over the objects
    /// the ones before it left unmatched.
    ///
    /// # Errors
    ///
    /// Returns an error when `matchers` is empty, a scheme is blank, a
    /// tolerance negative or not finite, an overlap ratio not in `(0, 1]`,
    /// or a relationship path empty.
    pub fn matching(matchers: Vec<Matcher>) -> Result<Self, ComparisonError> {
        if matchers.is_empty() {
            return Err(ComparisonError::NoMatcher);
        }
        for matcher in &matchers {
            matcher.validate()?;
        }
        Ok(Self {
            matchers,
            properties: BTreeSet::new(),
            property_sets: BTreeSet::new(),
            all_property_sets: false,
            timestamps: false,
            placement: None,
            geometry: None,
            coordinate_systems: None,
        })
    }

    /// Also compares one property, resolved through each session's resolver.
    ///
    /// Sources that answer properties only on request, rather than carrying
    /// them on the object, need this or a property set
    /// ([`Self::with_property_set`]).
    ///
    /// # Errors
    ///
    /// Returns an error when the name or a given set is blank.
    pub fn with_property(
        mut self,
        property_set: Option<&str>,
        name: &str,
    ) -> Result<Self, ComparisonError> {
        self.properties
            .insert(ComparedProperty::new(property_set, name)?);
        Ok(self)
    }

    /// Also compares every property of a set, listed through each session's
    /// property enumeration.
    ///
    /// # Errors
    ///
    /// Returns an error when the set is blank.
    pub fn with_property_set(mut self, property_set: &str) -> Result<Self, ComparisonError> {
        if property_set.trim().is_empty() {
            return Err(ComparisonError::BlankProperty);
        }
        self.property_sets.insert(property_set.to_owned());
        Ok(self)
    }

    /// Also compares every property of every set, listed through each
    /// session's property enumeration.
    #[must_use]
    pub fn with_all_property_sets(mut self) -> Self {
        self.all_property_sets = true;
        self
    }

    /// Also compares each matched object's placement frame: the distance
    /// between origins against the length tolerance, the rotation between
    /// axes against the angle tolerance.
    #[must_use]
    pub fn with_placement(mut self, tolerance: ComparisonTolerance) -> Self {
        self.placement = Some(tolerance);
        self
    }

    /// Also compares each matched object's measured bounds: the largest shift
    /// of any bound against the length tolerance.
    #[must_use]
    pub fn with_geometry(mut self, tolerance: ComparisonTolerance) -> Self {
        self.geometry = Some(tolerance);
        self
    }

    /// Also compares the header timestamps of each pair of sources: a
    /// revised source written before its base source is a finding.
    #[must_use]
    pub fn with_timestamps(mut self) -> Self {
        self.timestamps = true;
        self
    }

    /// Whether the header timestamps of each pair of sources are compared.
    #[must_use]
    pub fn compares_timestamps(&self) -> bool {
        self.timestamps
    }

    /// Also compares the coordinate systems of each pair of sources.
    #[must_use]
    pub fn with_coordinate_systems(mut self, tolerance: ComparisonTolerance) -> Self {
        self.coordinate_systems = Some(tolerance);
        self
    }

    /// The matchers objects are matched by, in order.
    pub fn matchers(&self) -> &[Matcher] {
        &self.matchers
    }
    /// The property sets compared whole, unless every set is.
    pub fn property_sets(&self) -> impl Iterator<Item = &str> {
        self.property_sets.iter().map(String::as_str)
    }
    /// Whether every property set is compared whole.
    #[must_use]
    pub fn compares_all_property_sets(&self) -> bool {
        self.all_property_sets
    }
    /// Whether the sources of each side are paired and compared.
    fn compares_sources(&self) -> bool {
        self.coordinate_systems.is_some() || self.timestamps
    }
    /// The placement tolerance, when placement is compared.
    #[must_use]
    pub fn placement(&self) -> Option<ComparisonTolerance> {
        self.placement
    }
    /// The geometry tolerance, when geometry is compared.
    #[must_use]
    pub fn geometry(&self) -> Option<ComparisonTolerance> {
        self.geometry
    }
    /// The coordinate-system tolerance, when coordinate systems are compared.
    #[must_use]
    pub fn coordinate_systems(&self) -> Option<ComparisonTolerance> {
        self.coordinate_systems
    }
}

/// Which revision an object belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// The earlier revision.
    Base,
    /// The later revision.
    Revised,
}

impl Side {
    /// `base` or `revised`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Revised => "revised",
        }
    }
}

/// What a difference, a gap or a measurement is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    /// The object's semantic kind.
    Kind,
    /// Classification statements.
    Classifications,
    /// A carried or requested property.
    Property,
    /// Relationship targets.
    Relationship,
    /// The object's placement frame.
    Placement,
    /// The object's measured body.
    Geometry,
    /// A source's coordinate system.
    CoordinateSystem,
    /// A source's header timestamp.
    Timestamp,
}

impl Facet {
    /// Stable lowercase name, used in rule ids and reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Classifications => "classifications",
            Self::Property => "property",
            Self::Relationship => "relationship",
            Self::Placement => "placement",
            Self::Geometry => "geometry",
            Self::CoordinateSystem => "coordinate-system",
            Self::Timestamp => "timestamp",
        }
    }
}

/// A quantity measured on both sides and compared against a tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Measure {
    /// Distance between placement origins, in metres.
    Origin,
    /// Rotation between placement axes, in radians.
    Orientation,
    /// Largest shift of any face of the bounds, in metres.
    Bounds,
    /// Distance between world-frame origins, in metres.
    WorldOrigin,
    /// Rotation between world-frame axes, in radians.
    WorldOrientation,
    /// Angle between true-north directions, in radians.
    TrueNorth,
    /// Distance between map offsets, in metres.
    MapOffset,
    /// Angle between map rotations, in radians.
    MapRotation,
    /// Difference between map scales; compared exactly.
    MapScale,
}

impl Measure {
    /// The facet the measure belongs to.
    #[must_use]
    pub fn facet(self) -> Facet {
        match self {
            Self::Origin | Self::Orientation => Facet::Placement,
            Self::Bounds => Facet::Geometry,
            Self::WorldOrigin
            | Self::WorldOrientation
            | Self::TrueNorth
            | Self::MapOffset
            | Self::MapRotation
            | Self::MapScale => Facet::CoordinateSystem,
        }
    }

    /// Stable lowercase name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Orientation => "orientation",
            Self::Bounds => "bounds",
            Self::WorldOrigin => "world-origin",
            Self::WorldOrientation => "world-orientation",
            Self::TrueNorth => "true-north",
            Self::MapOffset => "map-offset",
            Self::MapRotation => "map-rotation",
            Self::MapScale => "map-scale",
        }
    }

    /// The unit differences of this measure are stated in: `m`, `rad`, or
    /// empty for the unitless map scale.
    #[must_use]
    pub fn unit(self) -> &'static str {
        if self.is_angle() {
            "rad"
        } else if self.is_length() {
            "m"
        } else {
            ""
        }
    }

    fn is_angle(self) -> bool {
        matches!(
            self,
            Self::Orientation | Self::WorldOrientation | Self::TrueNorth | Self::MapRotation
        )
    }

    fn is_length(self) -> bool {
        matches!(
            self,
            Self::Origin | Self::Bounds | Self::WorldOrigin | Self::MapOffset
        )
    }

    /// Formats a value of this measure: metres to the millimetre, angles in
    /// degrees.
    fn format(self, value: f64) -> String {
        if self.is_angle() {
            format!("{:.3}°", value.to_degrees())
        } else if self.is_length() {
            format!("{value:.4} m")
        } else {
            format!("{value}")
        }
    }
}

/// How much a measure differs between the sides: the true difference lies
/// in `[lower, upper]`, a point when both measurements were exact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    /// What was measured.
    pub measure: Measure,
    /// Smallest possible difference.
    pub lower: f64,
    /// Largest possible difference.
    pub upper: f64,
    /// The tolerance it was compared with, in the measure's unit.
    pub tolerance: f64,
}

impl Measurement {
    /// Whether the difference is a single value.
    #[must_use]
    #[allow(clippy::float_cmp)] // A point interval is the question asked.
    pub fn is_exact(&self) -> bool {
        self.lower == self.upper
    }
}

impl std::fmt::Display for Measurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let measure = self.measure;
        let facet = measure.facet().name();
        let name = measure.name();
        if self.is_exact() {
            write!(
                f,
                "{facet} {name} differs by {}",
                measure.format(self.upper)
            )?;
        } else {
            write!(
                f,
                "{facet} {name} differs by {} to {}",
                measure.format(self.lower),
                measure.format(self.upper)
            )?;
        }
        write!(f, " (tolerance {})", measure.format(self.tolerance))
    }
}

/// One difference between matched objects or matched sources.
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
    /// The semantic kind changed.
    Kind {
        /// Kind in the base session.
        base: String,
        /// Kind in the revised session.
        revised: String,
    },
    /// Classification statements present on one side only.
    Classifications {
        /// Statements only the base object carries.
        removed: Vec<String>,
        /// Statements only the revised object carries.
        added: Vec<String>,
    },
    /// A property value changed, appeared (`base: None`) or disappeared
    /// (`revised: None`).
    Property {
        /// The property compared.
        property: ComparedProperty,
        /// Value in the base session.
        base: Option<PropertyValue>,
        /// Value in the revised session.
        revised: Option<PropertyValue>,
    },
    /// A property set present on one side only: every property in it
    /// appeared or disappeared with it.
    PropertySet {
        /// The set's name.
        property_set: String,
        /// Whether the revised object holds it (added) or the base object
        /// (removed).
        added: bool,
    },
    /// The revised source states it was written before the base source.
    OlderTimestamp {
        /// The base source's timestamp, as written.
        base: String,
        /// The revised source's timestamp, as written.
        revised: String,
    },
    /// Relationship targets, named by their external identity, present on
    /// one side only.
    Relationship {
        /// Relationship name.
        name: String,
        /// Target identities only the base object relates to.
        removed: Vec<String>,
        /// Target identities only the revised object relates to.
        added: Vec<String>,
    },
    /// A measure differs by more than its tolerance.
    Measured(Measurement),
    /// Something stated on one side differs from the other side's statement,
    /// or is stated on one side only: a placement, a body, a map conversion,
    /// a target reference system.
    Stated {
        /// The facet it belongs to.
        facet: Facet,
        /// What is stated, such as `placement` or `map target`.
        subject: String,
        /// The base statement.
        base: String,
        /// The revised statement.
        revised: String,
    },
}

impl Difference {
    /// The facet this difference belongs to.
    #[must_use]
    pub fn facet(&self) -> Facet {
        match self {
            Self::Kind { .. } => Facet::Kind,
            Self::Classifications { .. } => Facet::Classifications,
            Self::Property { .. } | Self::PropertySet { .. } => Facet::Property,
            Self::OlderTimestamp { .. } => Facet::Timestamp,
            Self::Relationship { .. } => Facet::Relationship,
            Self::Measured(measurement) => measurement.measure.facet(),
            Self::Stated { facet, .. } => *facet,
        }
    }
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |items: &[String]| items.join(", ");
        match self {
            Self::Kind { base, revised } => write!(f, "kind {base} -> {revised}"),
            Self::Classifications { removed, added } => {
                write!(f, "classifications -[{}] +[{}]", list(removed), list(added))
            }
            Self::Property {
                property,
                base,
                revised,
            } => write!(
                f,
                "property {property} {} -> {}",
                display_value(base.as_ref()),
                display_value(revised.as_ref())
            ),
            Self::PropertySet {
                property_set,
                added,
            } => write!(
                f,
                "property set {property_set} {}",
                if *added { "added" } else { "removed" }
            ),
            Self::OlderTimestamp { base, revised } => write!(
                f,
                "the revised file is older than the base: written {revised}, the base {base}"
            ),
            Self::Relationship {
                name,
                removed,
                added,
            } => write!(f, "{name} -[{}] +[{}]", list(removed), list(added)),
            Self::Measured(measurement) => measurement.fmt(f),
            Self::Stated {
                facet,
                subject,
                base,
                revised,
            } => write!(f, "{} {subject} {base} -> {revised}", facet.name()),
        }
    }
}

fn display_value(value: Option<&PropertyValue>) -> String {
    match value {
        None => "absent".to_owned(),
        Some(PropertyValue::Null) => "null".to_owned(),
        Some(PropertyValue::Boolean(value)) => value.to_string(),
        Some(PropertyValue::Integer(value)) => value.to_string(),
        Some(PropertyValue::Decimal(value)) => value.to_string(),
        Some(PropertyValue::Quantity { value, dimension }) => format!("{value} ({dimension:?})"),
        Some(PropertyValue::Measured {
            lower,
            upper,
            dimension,
        }) => format!("{lower}..{upper} ({dimension:?})"),
        Some(PropertyValue::String(value)) => format!("{value:?}"),
        Some(PropertyValue::Date(value)) => value.to_string(),
        Some(PropertyValue::DateTime(value)) => value.to_string(),
        Some(PropertyValue::Reference(target)) => format!("-> {target}"),
        Some(PropertyValue::List(values)) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| display_value(Some(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Some(PropertyValue::Bounded {
            lower,
            upper,
            set_point,
        }) => {
            let shown = |part: &Option<Box<PropertyValue>>| {
                part.as_deref().map(|value| display_value(Some(value)))
            };
            let range = match (shown(lower), shown(upper)) {
                (Some(lower), Some(upper)) => format!("from {lower} to {upper}"),
                (Some(lower), None) => format!("from {lower}, open above"),
                (None, Some(upper)) => format!("up to {upper}, open below"),
                (None, None) => "open".to_owned(),
            };
            match shown(set_point) {
                Some(point) => format!("[range {range}, set point {point}]"),
                None => format!("[range {range}]"),
            }
        }
        Some(PropertyValue::Table(rows)) => format!(
            "{{{}}}",
            rows.iter()
                .map(|row| format!(
                    "{} -> {}",
                    display_value(Some(&row.defining)),
                    display_value(Some(&row.defined))
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A facet of a matched pair that could not be compared.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unresolved {
    /// The facet not compared.
    pub facet: Facet,
    /// What within the facet, such as a property name; empty for the whole
    /// facet.
    pub subject: String,
    /// Why it could not be.
    pub reason: String,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.facet.name())?;
        if !self.subject.is_empty() {
            write!(f, " {}", self.subject)?;
        }
        write!(f, " not compared: {}", self.reason)
    }
}

/// How one identity fared between the sessions.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectChange {
    /// Only the revised session holds the identity.
    Added {
        /// The object in the revised session.
        revised: ObjectId,
    },
    /// Only the base session holds the identity.
    Removed {
        /// The object in the base session.
        base: ObjectId,
    },
    /// Present in both. Unchanged when all three lists are empty.
    Matched {
        /// The object in the base session.
        base: ObjectId,
        /// The object in the revised session.
        revised: ObjectId,
        /// Differences, in facet order.
        differences: Vec<Difference>,
        /// Facets that could not be compared.
        unresolved: Vec<Unresolved>,
        /// Measures whose difference interval straddles the tolerance.
        undetermined: Vec<Measurement>,
    },
}

/// One identity and what happened to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparedObject {
    /// The identity: its value in the scheme, or the value of the matched
    /// property. An added or removed object is named by its first key.
    pub identity: String,
    /// The label of the matcher the identity is read by ([`Matcher::label`]).
    pub matcher: String,
    /// What happened to it.
    pub change: ObjectChange,
}

/// Several objects of one revision claiming one identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbiguousIdentity {
    /// The revision holding the claims.
    pub side: Side,
    /// The contested identity value.
    pub identity: String,
    /// The label of the matcher that read it.
    pub matcher: String,
    /// The claimants, or the one object on the other side the ambiguity
    /// left unmatched.
    pub objects: Vec<ObjectId>,
}

/// An object whose match cannot be decided: its identity cannot be read, or
/// it may match an object whose identity cannot be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndecidedMatch {
    /// The revision holding the object.
    pub side: Side,
    /// The object.
    pub object: ObjectId,
    /// Why it is undecided.
    pub reason: NotEvaluatedReason,
    /// What could not be read.
    pub message: String,
}

/// What one base source and its revised counterpart state about themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceComparison {
    /// The base source.
    pub base: SourceId,
    /// The revised source.
    pub revised: SourceId,
    /// Differences between the two sources' statements.
    pub differences: Vec<Difference>,
    /// Statements that could not be compared.
    pub unresolved: Vec<Unresolved>,
    /// Measures whose difference interval straddles the tolerance.
    pub undetermined: Vec<Measurement>,
}

/// The result of comparing two sessions, ordered by identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelComparison {
    scheme: String,
    objects: Vec<ComparedObject>,
    unidentified: Vec<(Side, ObjectId)>,
    ambiguous: Vec<AmbiguousIdentity>,
    undecided: Vec<UndecidedMatch>,
    sources: Vec<SourceComparison>,
    unpaired_sources: Vec<(Side, SourceId)>,
}

impl ModelComparison {
    /// The labels of the matchers objects were matched by, joined by `, `.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    /// Every matched, added and removed identity, in identity order.
    pub fn objects(&self) -> &[ComparedObject] {
        &self.objects
    }
    /// Objects without an identity in the scheme, which cannot be matched.
    pub fn unidentified(&self) -> &[(Side, ObjectId)] {
        &self.unidentified
    }
    /// Identities claimed by more than one object of a revision.
    pub fn ambiguous(&self) -> &[AmbiguousIdentity] {
        &self.ambiguous
    }
    /// Objects whose match could not be decided.
    pub fn undecided(&self) -> &[UndecidedMatch] {
        &self.undecided
    }
    /// Source comparisons, one per paired source, when requested.
    pub fn sources(&self) -> &[SourceComparison] {
        &self.sources
    }
    /// Sources whose coordinate system had no counterpart to compare with.
    pub fn unpaired_sources(&self) -> &[(Side, SourceId)] {
        &self.unpaired_sources
    }
    /// Whether the sessions agree completely on everything compared.
    pub fn is_identical(&self) -> bool {
        self.unidentified.is_empty()
            && self.ambiguous.is_empty()
            && self.undecided.is_empty()
            && self.unpaired_sources.is_empty()
            && self.sources.iter().all(|source| {
                source.differences.is_empty()
                    && source.unresolved.is_empty()
                    && source.undetermined.is_empty()
            })
            && self.objects.iter().all(|object| {
                matches!(&object.change, ObjectChange::Matched { differences, unresolved, undetermined, .. }
                    if differences.is_empty() && unresolved.is_empty() && undetermined.is_empty())
            })
    }

    /// Not-evaluated outcomes for objects that could not be matched at all.
    fn identity_gaps(&self, rule_id: &RuleId) -> Vec<NotEvaluated> {
        let mut not_evaluated = Vec::new();
        for (side, object) in &self.unidentified {
            not_evaluated.push(NotEvaluated {
                rule_id: rule_id.clone(),
                scope: Scope::Object(object.clone()),
                reason: NotEvaluatedReason::IncompleteEvidence,
                message: format!(
                    "{side:?} object has no `{}` identity and cannot be matched",
                    self.scheme
                ),
                location: None,
            });
        }
        for ambiguous in &self.ambiguous {
            for object in &ambiguous.objects {
                not_evaluated.push(NotEvaluated {
                    rule_id: rule_id.clone(),
                    scope: Scope::Object(object.clone()),
                    reason: NotEvaluatedReason::InvalidEvidence,
                    message: format!(
                        "{:?} identity {}:{} is claimed by {} objects",
                        ambiguous.side,
                        ambiguous.matcher,
                        ambiguous.identity,
                        ambiguous.objects.len()
                    ),
                    location: None,
                });
            }
        }
        for undecided in &self.undecided {
            not_evaluated.push(NotEvaluated {
                rule_id: rule_id.clone(),
                scope: Scope::Object(undecided.object.clone()),
                reason: undecided.reason.clone(),
                message: format!(
                    "{:?} object cannot be matched: {}",
                    undecided.side, undecided.message
                ),
                location: None,
            });
        }
        not_evaluated
    }

    /// Projects the comparison into a report, so it can travel through any
    /// report sink.
    ///
    /// Each entry's rule id is `rule_id` followed by what it is about:
    /// `.added`, `.removed`, `.identity` for unidentified and ambiguous
    /// objects, or `.` and the facet name (`.property`, `.placement`,
    /// `.coordinate-system`, ...). A changed object has one finding per
    /// changed facet, naming its base object in `related`; a changed source
    /// has one `coordinate-system` finding scoped to the revised source.
    ///
    /// Everything the comparison could not decide becomes not-evaluated, not
    /// silence: an unidentified object may be the one that changed, and an
    /// undetermined measurement may be a move.
    #[must_use]
    pub fn report(&self, rule_id: &RuleId, severity: &Severity) -> Report {
        self.project(Naming::Suffixed(rule_id), severity, |_| None)
    }

    /// Projects the comparison into report entries named as `naming` says.
    ///
    /// `out_of_scope` names the compared objects whose outcome the caller
    /// cannot stand behind, with why: they become one not-evaluated outcome
    /// on their object instead.
    fn project(
        &self,
        naming: Naming<'_>,
        severity: &Severity,
        out_of_scope: impl Fn(&ComparedObject) -> Option<(ObjectId, NotEvaluatedReason, String)>,
    ) -> Report {
        let mut projection = Projection {
            scheme: &self.scheme,
            naming,
            severity,
            findings: Vec::new(),
            not_evaluated: Vec::new(),
        };
        for compared in &self.objects {
            match out_of_scope(compared) {
                Some((object, reason, message)) => {
                    projection.not_evaluated.push(NotEvaluated {
                        rule_id: projection.rule("identity"),
                        scope: Scope::Object(object),
                        reason,
                        message,
                        location: None,
                    });
                }
                None => projection.object(compared),
            }
        }
        for source in &self.sources {
            projection.source(source);
        }
        for (side, source) in &self.unpaired_sources {
            projection.gap(
                Facet::CoordinateSystem,
                Scope::Source(source.clone()),
                format!(
                    "coordinate-system not compared: the {} source has no counterpart",
                    match side {
                        Side::Base => "base",
                        Side::Revised => "revised",
                    }
                ),
            );
        }
        let identity = projection.rule("identity");
        projection
            .not_evaluated
            .extend(self.identity_gaps(&identity));
        projection.finish()
    }
}

/// How report entries name what they are about.
#[derive(Clone, Copy)]
enum Naming<'a> {
    /// In the rule id: `RULE.added`, `RULE.<facet>`, as `axioval compare`
    /// reports.
    Suffixed(&'a RuleId),
    /// In the message, under the rule's own id, as a capability reports.
    Prefixed(&'a RuleId),
}

/// Builds a comparison's report, entry by entry.
struct Projection<'a> {
    scheme: &'a str,
    naming: Naming<'a>,
    severity: &'a Severity,
    findings: Vec<Finding>,
    not_evaluated: Vec<NotEvaluated>,
}

impl Projection<'_> {
    fn rule(&self, suffix: &str) -> RuleId {
        match self.naming {
            Naming::Suffixed(rule_id) => {
                RuleId::new(format!("{rule_id}.{suffix}")).unwrap_or_else(|_| rule_id.clone())
            }
            Naming::Prefixed(rule_id) => rule_id.clone(),
        }
    }

    /// The message of a change of `facet`: the facet is named in the
    /// message only where the rule id does not name it.
    fn changed(&self, facet: Facet, rest: &str) -> String {
        match self.naming {
            Naming::Suffixed(_) => format!("changed{rest}"),
            Naming::Prefixed(_) => format!("{} changed{rest}", facet.name()),
        }
    }

    fn finding(
        &mut self,
        suffix: &str,
        scope: Scope,
        related: Option<&ObjectId>,
        message: String,
        (source, matcher, identity): (&SourceId, &str, &str),
    ) {
        // A revision older than its base inverts the whole comparison.
        let severity = if suffix == Facet::Timestamp.name() {
            Severity::Error
        } else {
            self.severity.clone()
        };
        let finding = Finding {
            id: None,
            decision: None,
            rule_id: self.rule(suffix),
            scope,
            severity,
            message,
            related: Vec::new(),
            evidence: vec![Evidence {
                source: source.clone(),
                locator: format!("comparison:{matcher}:{identity}"),
                exact: true,
            }],
            location: None,
            categories: Vec::new(),
        }
        .with_related(related.cloned());
        self.findings.push(finding);
    }

    fn gap(&mut self, facet: Facet, scope: Scope, message: String) {
        self.not_evaluated.push(NotEvaluated {
            rule_id: self.rule(facet.name()),
            scope,
            reason: NotEvaluatedReason::IncompleteEvidence,
            message,
            location: None,
        });
    }

    fn gaps(&mut self, scope: &Scope, unresolved: &[Unresolved], undetermined: &[Measurement]) {
        for entry in unresolved {
            self.gap(entry.facet, scope.clone(), entry.to_string());
        }
        for measurement in undetermined {
            self.gap(
                measurement.measure.facet(),
                scope.clone(),
                format!("{measurement}: undetermined"),
            );
        }
    }

    fn object(&mut self, compared: &ComparedObject) {
        let identity = compared.identity.as_str();
        let matcher = compared.matcher.as_str();
        match &compared.change {
            ObjectChange::Added { revised } => self.finding(
                "added",
                Scope::Object(revised.clone()),
                None,
                format!("added ({matcher}:{identity})"),
                (&revised.source, matcher, identity),
            ),
            ObjectChange::Removed { base } => self.finding(
                "removed",
                Scope::Object(base.clone()),
                None,
                format!("removed ({matcher}:{identity})"),
                (&base.source, matcher, identity),
            ),
            ObjectChange::Matched {
                base,
                revised,
                differences,
                unresolved,
                undetermined,
            } => {
                for (facet, changes) in by_facet(differences) {
                    let message = self.changed(facet, &format!(": {}", changes.join("; ")));
                    self.finding(
                        facet.name(),
                        Scope::Object(revised.clone()),
                        Some(base),
                        message,
                        (&revised.source, matcher, identity),
                    );
                }
                self.gaps(&Scope::Object(revised.clone()), unresolved, undetermined);
            }
        }
    }

    fn source(&mut self, source: &SourceComparison) {
        let scope = Scope::Source(source.revised.clone());
        let base = source.base.to_string();
        for (facet, changes) in by_facet(&source.differences) {
            let message = self.changed(facet, &format!(" from `{base}`: {}", changes.join("; ")));
            let scheme = self.scheme;
            self.finding(
                facet.name(),
                scope.clone(),
                None,
                message,
                (&source.revised, scheme, &base),
            );
        }
        self.gaps(&scope, &source.unresolved, &source.undetermined);
    }

    fn finish(mut self) -> Report {
        self.findings.sort_by(|a, b| {
            a.rule_id
                .cmp(&b.rule_id)
                .then_with(|| a.scope.cmp(&b.scope))
                .then_with(|| a.message.cmp(&b.message))
        });
        self.not_evaluated.sort();
        Report {
            resources: Vec::new(),
            stale_decisions: Vec::new(),
            findings: self.findings,
            not_evaluated: self.not_evaluated,
            tables: Vec::new(),
            rules: Vec::new(),
        }
    }
}

/// Differences grouped by facet, in facet order, each rendered.
fn by_facet(differences: &[Difference]) -> BTreeMap<Facet, Vec<String>> {
    let mut grouped: BTreeMap<Facet, Vec<String>> = BTreeMap::new();
    for difference in differences {
        grouped
            .entry(difference.facet())
            .or_default()
            .push(difference.to_string());
    }
    grouped
}

/// One side of a comparison: the objects compared and where to read about
/// them.
pub(crate) struct Revision<'a> {
    /// The project and services the revision's objects are read through.
    pub(crate) context: RuleContext<'a>,
    /// The objects compared.
    pub(crate) candidates: Vec<&'a Object>,
    /// Every object of the revision's sources, compared or not: relationship
    /// targets are named among them.
    pub(crate) objects: Vec<&'a Object>,
    /// Each source's header timestamps as written; `None` when unread.
    pub(crate) timestamps: BTreeMap<SourceId, Option<Vec<String>>>,
}

impl<'a> Revision<'a> {
    /// Every object of `session`, compared.
    fn of(session: &'a EvidenceSession) -> Self {
        let objects: Vec<&Object> = session.project().objects().collect();
        Self {
            context: RuleContext {
                project: session.project(),
                services: session.services(),
            },
            candidates: objects.clone(),
            objects,
            timestamps: session
                .snapshots()
                .map(|snapshot| {
                    let source = snapshot.source();
                    let values = session
                        .source_metadata(source)
                        .and_then(|metadata| metadata.values(SourceField::Timestamp))
                        .map(<[String]>::to_vec);
                    (source.clone(), values)
                })
                .collect(),
        }
    }
}

/// Paired base and revised sources, and the sources left without a pair.
type SourcePairs = (Vec<(SourceId, SourceId)>, Vec<(Side, SourceId)>);

/// Pairs each base source with its revised counterpart: the only source of
/// each side, or else the one source of each side declaring a discipline.
fn pair_sources(base: &EvidenceSession, revised: &EvidenceSession) -> SourcePairs {
    let sources = |session: &EvidenceSession| -> Vec<SourceId> {
        session
            .snapshots()
            .map(|snapshot| snapshot.source().clone())
            .collect()
    };
    let (before, after) = (sources(base), sources(revised));
    if let ([a], [b]) = (before.as_slice(), after.as_slice()) {
        return (vec![(a.clone(), b.clone())], Vec::new());
    }
    let by_discipline = |session: &EvidenceSession, list: &[SourceId]| {
        let mut map: BTreeMap<String, Vec<SourceId>> = BTreeMap::new();
        for source in list {
            if let Some(discipline) = session.discipline(source) {
                map.entry(discipline.to_string())
                    .or_default()
                    .push(source.clone());
            }
        }
        map
    };
    let (base_map, revised_map) = (by_discipline(base, &before), by_discipline(revised, &after));
    let mut pairs = Vec::new();
    for (discipline, sources) in &base_map {
        if let ([a], Some([b])) = (
            sources.as_slice(),
            revised_map.get(discipline).map(Vec::as_slice),
        ) {
            pairs.push((a.clone(), b.clone()));
        }
    }
    let paired_base: BTreeSet<&SourceId> = pairs.iter().map(|(a, _)| a).collect();
    let paired_revised: BTreeSet<&SourceId> = pairs.iter().map(|(_, b)| b).collect();
    let mut unpaired: Vec<(Side, SourceId)> = before
        .iter()
        .filter(|source| !paired_base.contains(source))
        .map(|source| (Side::Base, source.clone()))
        .chain(
            after
                .iter()
                .filter(|source| !paired_revised.contains(source))
                .map(|source| (Side::Revised, source.clone())),
        )
        .collect();
    unpaired.sort();
    (pairs, unpaired)
}

/// Names relationship targets: a matched object by its pair's identity, any
/// other by its identity in the first scheme the request matches by, when no
/// other object of its revision claims it.
fn target_names<'a>(
    (base, revised): (&Revision<'a>, &Revision<'a>),
    request: &ComparisonRequest,
    matching: &matching::Matching<'a>,
) -> facets::TargetNames<'a> {
    let mut names = facets::TargetNames {
        base: BTreeMap::new(),
        revised: BTreeMap::new(),
    };
    for pair in &matching.pairs {
        names.base.insert(&pair.base.id, pair.identity.clone());
        names
            .revised
            .insert(&pair.revised.id, pair.identity.clone());
    }
    let scheme = request.matchers.iter().find_map(|matcher| match matcher {
        Matcher::Scheme(scheme) => Some(scheme.as_str()),
        _ => None,
    });
    if let Some(scheme) = scheme {
        for (revision, named) in [(base, &mut names.base), (revised, &mut names.revised)] {
            let mut claims: BTreeMap<&str, Vec<&'a ObjectId>> = BTreeMap::new();
            for object in &revision.objects {
                if let Some(identity) = object.external_id(scheme) {
                    claims.entry(identity).or_default().push(&object.id);
                }
            }
            for (identity, objects) in claims {
                if let [object] = objects[..] {
                    named.entry(object).or_insert_with(|| identity.to_owned());
                }
            }
        }
    }
    names
}

/// Compares two sessions object by object, matched by `request`'s matchers.
#[must_use]
pub fn compare_sessions(
    base: &EvidenceSession,
    revised: &EvidenceSession,
    request: &ComparisonRequest,
) -> ModelComparison {
    let sources = if request.compares_sources() {
        pair_sources(base, revised)
    } else {
        (Vec::new(), Vec::new())
    };
    compare(
        &Revision::of(base),
        &Revision::of(revised),
        request,
        sources,
    )
}

/// Compares two revisions: matches their candidates, compares every pair
/// and every paired source.
fn compare<'a>(
    base: &Revision<'a>,
    revised: &Revision<'a>,
    request: &ComparisonRequest,
    (pairs, unpaired_sources): SourcePairs,
) -> ModelComparison {
    let matching = matching::match_objects(base, revised, &request.matchers);
    let names = target_names((base, revised), request, &matching);
    let mut objects: Vec<ComparedObject> = Vec::new();
    for pair in &matching.pairs {
        objects.push(ComparedObject {
            identity: pair.identity.clone(),
            matcher: pair.matcher.clone(),
            change: facets::matched((base, revised), request, &names, pair.base, pair.revised),
        });
    }
    for single in &matching.removed {
        objects.push(ComparedObject {
            identity: single.identity.clone(),
            matcher: single.matcher.clone(),
            change: ObjectChange::Removed {
                base: single.object.id.clone(),
            },
        });
    }
    for single in &matching.added {
        objects.push(ComparedObject {
            identity: single.identity.clone(),
            matcher: single.matcher.clone(),
            change: ObjectChange::Added {
                revised: single.object.id.clone(),
            },
        });
    }
    let subject = |compared: &ComparedObject| match &compared.change {
        ObjectChange::Removed { base } | ObjectChange::Matched { base, .. } => base.clone(),
        ObjectChange::Added { revised } => revised.clone(),
    };
    objects.sort_by(|a, b| {
        (&a.identity, &a.matcher)
            .cmp(&(&b.identity, &b.matcher))
            .then_with(|| subject(a).cmp(&subject(b)))
    });
    let sources = pairs
        .iter()
        .map(|(a, b)| facets::sources((base, a), (revised, b), request))
        .collect();
    let matching::Matching {
        unidentified,
        ambiguous,
        undecided,
        ..
    } = matching;
    ModelComparison {
        scheme: request
            .matchers
            .iter()
            .map(Matcher::label)
            .collect::<Vec<_>>()
            .join(", "),
        objects,
        unidentified,
        ambiguous,
        undecided,
        sources,
        unpaired_sources,
    }
}
