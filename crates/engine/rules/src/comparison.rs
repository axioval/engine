//! Comparison of two evidence sessions.
//!
//! Objects are matched across sessions by an external identity scheme, never
//! by [`ObjectId`]: an object's local id is whatever its source file numbered
//! it, and a re-export renumbers everything. The scheme is a parameter because
//! the engine attaches no meaning to it; an IFC session supplies `GlobalId`s
//! under its adapter's scheme, another source its own.
//!
//! The semantic facets (kind, classifications, properties, relationships) are
//! always compared. The spatial facets are opt-in, each with its tolerance,
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
//! Nothing is silently dropped. An object without an identity in the scheme
//! cannot be matched and is reported as unidentified. An identity held by two
//! objects of one session is ambiguous and matches nothing. A facet that
//! cannot be read exactly on both sides -- a property resolver error, a
//! relationship target without an identity, a missing service or an
//! unmeasured body -- is reported unresolved rather than compared as if it
//! were empty.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    ClassificationServiceHandle, CoordinateFrame, CoordinateSystemServiceHandle, EvidenceSession,
    MetricDirection, ObjectFrameError, ObjectFrameServiceHandle, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, ProximityError, ProximityServiceHandle,
    SourceCoordinateSystem,
};
use axioval_ir::{
    Evidence, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, PropertyValue, Report,
    RuleId, Scope, Severity, SourceId,
};

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

/// What to compare and how to match objects.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonRequest {
    scheme: String,
    properties: BTreeSet<ComparedProperty>,
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
        let scheme = scheme.into();
        if scheme.trim().is_empty() {
            return Err(ComparisonError::BlankScheme);
        }
        Ok(Self {
            scheme,
            properties: BTreeSet::new(),
            placement: None,
            geometry: None,
            coordinate_systems: None,
        })
    }

    /// Also compares one property, resolved through each session's resolver.
    ///
    /// Sources that answer properties only on request, rather than carrying
    /// them on the object, need this: there is no way to list what such a
    /// source holds, so the properties that matter are named.
    ///
    /// # Errors
    ///
    /// Returns an error when the name or a given set is blank.
    pub fn with_property(
        mut self,
        property_set: Option<&str>,
        name: &str,
    ) -> Result<Self, ComparisonError> {
        if name.trim().is_empty() || property_set.is_some_and(|set| set.trim().is_empty()) {
            return Err(ComparisonError::BlankProperty);
        }
        self.properties.insert(ComparedProperty {
            property_set: property_set.map(str::to_owned),
            name: name.to_owned(),
        });
        Ok(self)
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

    /// Also compares the coordinate systems of each pair of sources.
    #[must_use]
    pub fn with_coordinate_systems(mut self, tolerance: ComparisonTolerance) -> Self {
        self.coordinate_systems = Some(tolerance);
        self
    }

    /// The identity scheme objects are matched by.
    pub fn scheme(&self) -> &str {
        &self.scheme
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

/// Which session an object belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// The earlier session.
    Base,
    /// The later session.
    Revised,
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
            Self::Property { .. } => Facet::Property,
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

/// One external identity and what happened to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparedObject {
    /// The identity's value in the comparison scheme.
    pub identity: String,
    /// What happened to it.
    pub change: ObjectChange,
}

/// Several objects of one session claiming one identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmbiguousIdentity {
    /// The session holding the claims.
    pub side: Side,
    /// The contested identity value.
    pub identity: String,
    /// The claimants, or the one object on the other side the ambiguity
    /// left unmatched.
    pub objects: Vec<ObjectId>,
}

/// The coordinate systems of one base source and its revised counterpart.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceComparison {
    /// The base source.
    pub base: SourceId,
    /// The revised source.
    pub revised: SourceId,
    /// Coordinate-system differences.
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
    sources: Vec<SourceComparison>,
    unpaired_sources: Vec<(Side, SourceId)>,
}

impl ModelComparison {
    /// The identity scheme objects were matched by.
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
    /// Identities claimed by more than one object of a session.
    pub fn ambiguous(&self) -> &[AmbiguousIdentity] {
        &self.ambiguous
    }
    /// Coordinate-system comparisons, one per paired source, when requested.
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
                        self.scheme,
                        ambiguous.identity,
                        ambiguous.objects.len()
                    ),
                    location: None,
                });
            }
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
        let mut projection = Projection {
            scheme: &self.scheme,
            rule_id,
            severity,
            findings: Vec::new(),
            not_evaluated: Vec::new(),
        };
        for compared in &self.objects {
            projection.object(compared);
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

/// Builds a comparison's report, entry by entry.
struct Projection<'a> {
    scheme: &'a str,
    rule_id: &'a RuleId,
    severity: &'a Severity,
    findings: Vec<Finding>,
    not_evaluated: Vec<NotEvaluated>,
}

impl Projection<'_> {
    fn rule(&self, suffix: &str) -> RuleId {
        RuleId::new(format!("{}.{suffix}", self.rule_id)).unwrap_or_else(|_| self.rule_id.clone())
    }

    fn finding(
        &mut self,
        suffix: &str,
        scope: Scope,
        related: Option<&ObjectId>,
        message: String,
        (source, identity): (&SourceId, &str),
    ) {
        let finding = Finding {
            rule_id: self.rule(suffix),
            scope,
            severity: self.severity.clone(),
            message,
            related: Vec::new(),
            evidence: vec![Evidence {
                source: source.clone(),
                locator: format!("comparison:{}:{identity}", self.scheme),
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
        let scheme = self.scheme;
        match &compared.change {
            ObjectChange::Added { revised } => self.finding(
                "added",
                Scope::Object(revised.clone()),
                None,
                format!("added ({scheme}:{identity})"),
                (&revised.source, identity),
            ),
            ObjectChange::Removed { base } => self.finding(
                "removed",
                Scope::Object(base.clone()),
                None,
                format!("removed ({scheme}:{identity})"),
                (&base.source, identity),
            ),
            ObjectChange::Matched {
                base,
                revised,
                differences,
                unresolved,
                undetermined,
            } => {
                for (facet, changes) in by_facet(differences) {
                    self.finding(
                        facet.name(),
                        Scope::Object(revised.clone()),
                        Some(base),
                        format!("changed: {}", changes.join("; ")),
                        (&revised.source, identity),
                    );
                }
                self.gaps(&Scope::Object(revised.clone()), unresolved, undetermined);
            }
        }
    }

    fn source(&mut self, source: &SourceComparison) {
        let scope = Scope::Source(source.revised.clone());
        let base = source.base.to_string();
        for (_, changes) in by_facet(&source.differences) {
            self.finding(
                Facet::CoordinateSystem.name(),
                scope.clone(),
                None,
                format!("changed from `{base}`: {}", changes.join("; ")),
                (&source.revised, &base),
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

/// One session, indexed by identity.
struct Indexed<'a> {
    session: &'a EvidenceSession,
    by_identity: BTreeMap<&'a str, &'a Object>,
    identity_of: BTreeMap<&'a ObjectId, &'a str>,
}

fn index<'a>(
    session: &'a EvidenceSession,
    side: Side,
    scheme: &str,
    unidentified: &mut Vec<(Side, ObjectId)>,
    ambiguous: &mut Vec<AmbiguousIdentity>,
) -> Indexed<'a> {
    let mut claims: BTreeMap<&str, Vec<&Object>> = BTreeMap::new();
    for object in session.project().objects() {
        match object.external_id(scheme) {
            Some(identity) => claims.entry(identity).or_default().push(object),
            None => unidentified.push((side, object.id.clone())),
        }
    }
    let mut by_identity = BTreeMap::new();
    let mut identity_of = BTreeMap::new();
    for (identity, objects) in claims {
        // `Project` rejects this within one source; a multi-source session
        // can still hold one identity twice, for instance a federated copy.
        if let [object] = objects[..] {
            by_identity.insert(identity, object);
            identity_of.insert(&object.id, identity);
        } else {
            ambiguous.push(AmbiguousIdentity {
                side,
                identity: identity.to_owned(),
                objects: objects.iter().map(|object| object.id.clone()).collect(),
            });
        }
    }
    Indexed {
        session,
        by_identity,
        identity_of,
    }
}

/// Where a session's classification statements come from.
enum ClassificationBasis<'a> {
    Service(&'a ClassificationServiceHandle),
    Carried,
}

impl<'a> ClassificationBasis<'a> {
    fn of(session: &'a EvidenceSession) -> Self {
        session
            .service::<ClassificationServiceHandle>()
            .map_or(Self::Carried, Self::Service)
    }

    fn statements(&self, object: &Object) -> Result<BTreeSet<String>, String> {
        match self {
            Self::Carried => Ok(object
                .classifications
                .iter()
                .map(|c| format!("{}:{}", c.system, c.code))
                .collect()),
            Self::Service(service) => service
                .classifications(&object.id)
                .map(|assignments| {
                    assignments
                        .iter()
                        .map(|assignment| {
                            let codes: Vec<&str> = assignment
                                .codes
                                .iter()
                                .map(|code| code.as_deref().unwrap_or("?"))
                                .collect();
                            format!(
                                "{}:{}",
                                assignment.system.as_deref().unwrap_or("?"),
                                codes.join("/")
                            )
                        })
                        .collect()
                })
                .map_err(|error| error.to_string()),
        }
    }
}

/// Differences, gaps and undetermined measures collected for one pair.
#[derive(Default)]
struct Outcome {
    differences: Vec<Difference>,
    unresolved: Vec<Unresolved>,
    undetermined: Vec<Measurement>,
}

impl Outcome {
    fn unresolved(&mut self, facet: Facet, subject: impl Into<String>, reason: impl Into<String>) {
        self.unresolved.push(Unresolved {
            facet,
            subject: subject.into(),
            reason: reason.into(),
        });
    }

    fn stated(&mut self, facet: Facet, subject: &str, base: &str, revised: &str) {
        self.differences.push(Difference::Stated {
            facet,
            subject: subject.to_owned(),
            base: base.to_owned(),
            revised: revised.to_owned(),
        });
    }

    /// Judges a difference known to lie in `[lower, upper]`: changed when
    /// all of it exceeds the tolerance, unchanged when none of it does,
    /// otherwise undetermined.
    fn measured(&mut self, measure: Measure, lower: f64, upper: f64, tolerance: f64) {
        let measurement = Measurement {
            measure,
            lower,
            upper,
            tolerance,
        };
        if lower > tolerance {
            self.differences.push(Difference::Measured(measurement));
        } else if upper > tolerance {
            self.undetermined.push(measurement);
        }
    }

    /// Judges an exactly known difference.
    fn exact(&mut self, measure: Measure, value: f64, tolerance: f64) {
        self.measured(measure, value, value, tolerance);
    }
}

struct Pair<'a> {
    base: &'a Indexed<'a>,
    revised: &'a Indexed<'a>,
    request: &'a ComparisonRequest,
    outcome: Outcome,
}

impl Pair<'_> {
    fn unresolved(&mut self, facet: Facet, subject: impl Into<String>, reason: impl Into<String>) {
        self.outcome.unresolved(facet, subject, reason);
    }

    fn differ(&mut self, difference: Difference) {
        self.outcome.differences.push(difference);
    }

    fn classifications(&mut self, base: &Object, revised: &Object) {
        let (base_basis, revised_basis) = (
            ClassificationBasis::of(self.base.session),
            ClassificationBasis::of(self.revised.session),
        );
        if matches!(base_basis, ClassificationBasis::Service(_))
            != matches!(revised_basis, ClassificationBasis::Service(_))
        {
            // One side resolves inherited chains, the other lists what the
            // object carries: every difference would be an artefact.
            self.unresolved(
                Facet::Classifications,
                "",
                "the sessions state classifications through different services",
            );
            return;
        }
        match (
            base_basis.statements(base),
            revised_basis.statements(revised),
        ) {
            (Ok(before), Ok(after)) if before != after => {
                self.differ(Difference::Classifications {
                    removed: before.difference(&after).cloned().collect(),
                    added: after.difference(&before).cloned().collect(),
                });
            }
            (Ok(_), Ok(_)) => {}
            (Err(reason), _) | (_, Err(reason)) => {
                self.unresolved(Facet::Classifications, "", reason);
            }
        }
    }

    fn carried_properties(&mut self, base: &Object, revised: &Object) {
        let collect = |object: &Object| {
            let mut values: BTreeMap<ComparedProperty, Vec<PropertyValue>> = BTreeMap::new();
            for property in &object.properties {
                values
                    .entry(ComparedProperty {
                        property_set: Some(property.property_set.clone()),
                        name: property.name.clone(),
                    })
                    .or_default()
                    .push(property.value.clone());
            }
            values
        };
        let (before, after) = (collect(base), collect(revised));
        let keys: BTreeSet<&ComparedProperty> = before.keys().chain(after.keys()).collect();
        for key in keys {
            // Named properties are compared through the resolvers instead.
            if self.request.properties.contains(key) {
                continue;
            }
            let (Ok(old), Ok(new)) = (single(before.get(key)), single(after.get(key))) else {
                self.unresolved(Facet::Property, key.to_string(), "stated more than once");
                continue;
            };
            if !same_optional(old, new) {
                self.differ(Difference::Property {
                    property: key.clone(),
                    base: old.cloned(),
                    revised: new.cloned(),
                });
            }
        }
    }

    fn requested_properties(&mut self, base: &Object, revised: &Object) {
        let resolvers = (
            self.base
                .session
                .service::<PropertyResolutionServiceHandle>(),
            self.revised
                .session
                .service::<PropertyResolutionServiceHandle>(),
        );
        for property in &self.request.properties.clone() {
            let (Some(base_resolver), Some(revised_resolver)) = resolvers else {
                self.unresolved(
                    Facet::Property,
                    property.to_string(),
                    "a session has no property resolver",
                );
                continue;
            };
            match (
                resolve(base_resolver, base, property),
                resolve(revised_resolver, revised, property),
            ) {
                (Ok(old), Ok(new)) => {
                    if !same_optional(old.as_ref(), new.as_ref()) {
                        self.differ(Difference::Property {
                            property: property.clone(),
                            base: old,
                            revised: new,
                        });
                    }
                }
                (Err(reason), _) | (_, Err(reason)) => {
                    self.unresolved(Facet::Property, property.to_string(), reason);
                }
            }
        }
    }

    fn relationships(&mut self, base: &Object, revised: &Object) {
        let names: BTreeSet<&String> = base
            .relationships
            .keys()
            .chain(revised.relationships.keys())
            .collect();
        for name in names {
            let targets = |indexed: &Indexed<'_>, object: &Object| -> Option<BTreeSet<String>> {
                object
                    .relationships
                    .get(name)
                    .into_iter()
                    .flatten()
                    .map(|target| indexed.identity_of.get(target).map(|id| (*id).to_owned()))
                    .collect()
            };
            match (targets(self.base, base), targets(self.revised, revised)) {
                (Some(before), Some(after)) => {
                    if before != after {
                        self.differ(Difference::Relationship {
                            name: name.clone(),
                            removed: before.difference(&after).cloned().collect(),
                            added: after.difference(&before).cloned().collect(),
                        });
                    }
                }
                _ => self.unresolved(
                    Facet::Relationship,
                    name.clone(),
                    "a target has no unique identity in the scheme",
                ),
            }
        }
    }

    /// Placement frames: origin distance and axis rotation, both exact.
    fn placement(&mut self, base: &Object, revised: &Object, tolerance: ComparisonTolerance) {
        let (Some(base_frames), Some(revised_frames)) = (
            self.base.session.service::<ObjectFrameServiceHandle>(),
            self.revised.session.service::<ObjectFrameServiceHandle>(),
        ) else {
            self.unresolved(
                Facet::Placement,
                "",
                "a session has no object-frame service",
            );
            return;
        };
        match (
            base_frames.object_frame(&base.id),
            revised_frames.object_frame(&revised.id),
        ) {
            (Err(ObjectFrameError::NotPlaced(_)), Err(ObjectFrameError::NotPlaced(_))) => {}
            (Ok(_), Err(ObjectFrameError::NotPlaced(_))) => {
                self.outcome
                    .stated(Facet::Placement, "placement", "placed", "not placed");
            }
            (Err(ObjectFrameError::NotPlaced(_)), Ok(_)) => {
                self.outcome
                    .stated(Facet::Placement, "placement", "not placed", "placed");
            }
            (Ok(before), Ok(after)) => {
                let (before, after) = (before.frame(), after.frame());
                let axes = |frame: &axioval_engine::MetricFrame| {
                    [frame.right(), frame.forward(), frame.up()]
                };
                self.outcome.exact(
                    Measure::Origin,
                    distance(
                        before.origin().coordinates_metres(),
                        after.origin().coordinates_metres(),
                    ),
                    tolerance.length_metres,
                );
                self.outcome.exact(
                    Measure::Orientation,
                    rotation(axes(before), axes(after)),
                    tolerance.angle_radians,
                );
            }
            (Err(error), _) | (_, Err(error)) => {
                self.unresolved(Facet::Placement, "", error.to_string());
            }
        }
    }

    /// Measured bounds: the largest shift of any bound, widened by both
    /// tessellations' chord deviations.
    fn geometry(&mut self, base: &Object, revised: &Object, tolerance: ComparisonTolerance) {
        let (Some(base_bodies), Some(revised_bodies)) = (
            self.base.session.service::<ProximityServiceHandle>(),
            self.revised.session.service::<ProximityServiceHandle>(),
        ) else {
            self.unresolved(Facet::Geometry, "", "a session has no geometry service");
            return;
        };
        match (
            base_bodies.bounds(&base.id),
            revised_bodies.bounds(&revised.id),
        ) {
            (Err(ProximityError::NoBody), Err(ProximityError::NoBody)) => {}
            (Ok(_), Err(ProximityError::NoBody)) => {
                self.outcome
                    .stated(Facet::Geometry, "body", "present", "none");
            }
            (Err(ProximityError::NoBody), Ok(_)) => {
                self.outcome
                    .stated(Facet::Geometry, "body", "none", "present");
            }
            (Ok(before), Ok(after)) => {
                let (a, b) = (before.bounds(), after.bounds());
                let shift = (0..3)
                    .flat_map(|axis| {
                        [
                            (a.min()[axis] - b.min()[axis]).abs(),
                            (a.max()[axis] - b.max()[axis]).abs(),
                        ]
                    })
                    .fold(0.0_f64, f64::max);
                // Each true bound lies within its body's chord deviation of
                // the measured one, so the true shift lies within their sum.
                let widening =
                    before.fidelity().deviation_metres() + after.fidelity().deviation_metres();
                self.outcome.measured(
                    Measure::Bounds,
                    (shift - widening).max(0.0),
                    shift + widening,
                    tolerance.length_metres,
                );
            }
            (Err(error), _) | (_, Err(error)) => {
                self.unresolved(Facet::Geometry, "", error.to_string());
            }
        }
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The rotation angle between two orthonormal axis triples.
///
/// For a rotation by θ, the Frobenius norm of the difference of the two
/// rotation matrices is `2·√2·sin(θ/2)`; unlike the trace formula this stays
/// accurate for the small angles a tolerance is about.
fn rotation(a: [MetricDirection; 3], b: [MetricDirection; 3]) -> f64 {
    let squared: f64 = a
        .iter()
        .zip(&b)
        .map(|(x, y)| {
            let (x, y) = (x.components(), y.components());
            (x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)
        })
        .sum();
    2.0 * (squared.sqrt() / (2.0 * std::f64::consts::SQRT_2))
        .min(1.0)
        .asin()
}

/// The angle between two unit plan directions.
fn plan_angle(a: [f64; 2], b: [f64; 2]) -> f64 {
    let cross = a[0] * b[1] - a[1] * b[0];
    let dot = a[0] * b[0] + a[1] * b[1];
    cross.abs().atan2(dot)
}

fn stated(value: bool) -> &'static str {
    if value { "stated" } else { "not stated" }
}

/// Compares the coordinate systems of two sources.
fn coordinate_systems(
    base: (&EvidenceSession, &SourceId),
    revised: (&EvidenceSession, &SourceId),
    tolerance: ComparisonTolerance,
) -> SourceComparison {
    let mut outcome = Outcome::default();
    let facet = Facet::CoordinateSystem;
    match (
        base.0.service::<CoordinateSystemServiceHandle>(),
        revised.0.service::<CoordinateSystemServiceHandle>(),
    ) {
        (Some(before), Some(after)) => {
            match (
                before.coordinate_system(base.1),
                after.coordinate_system(revised.1),
            ) {
                (Ok(before), Ok(after)) => {
                    compare_systems(&before, &after, tolerance, &mut outcome);
                }
                (Err(error), _) | (_, Err(error)) => {
                    outcome.unresolved(facet, "", error.to_string());
                }
            }
        }
        _ => outcome.unresolved(facet, "", "a session has no coordinate-system service"),
    }
    SourceComparison {
        base: base.1.clone(),
        revised: revised.1.clone(),
        differences: outcome.differences,
        unresolved: outcome.unresolved,
        undetermined: outcome.undetermined,
    }
}

fn compare_systems(
    before: &SourceCoordinateSystem,
    after: &SourceCoordinateSystem,
    tolerance: ComparisonTolerance,
    outcome: &mut Outcome,
) {
    let facet = Facet::CoordinateSystem;
    let frame_axes = |frame: &CoordinateFrame| frame.axes();
    match (before.world(), after.world()) {
        (Some(a), Some(b)) => {
            outcome.exact(
                Measure::WorldOrigin,
                distance(a.origin_metres(), b.origin_metres()),
                tolerance.length_metres,
            );
            outcome.exact(
                Measure::WorldOrientation,
                rotation(frame_axes(a), frame_axes(b)),
                tolerance.angle_radians,
            );
        }
        (None, None) => {}
        (a, b) => outcome.stated(
            facet,
            "world frame",
            stated(a.is_some()),
            stated(b.is_some()),
        ),
    }
    match (before.true_north(), after.true_north()) {
        (Some(a), Some(b)) => {
            outcome.exact(
                Measure::TrueNorth,
                plan_angle(a, b),
                tolerance.angle_radians,
            );
        }
        (None, None) => {}
        (a, b) => outcome.stated(
            facet,
            "true north",
            stated(a.is_some()),
            stated(b.is_some()),
        ),
    }
    match (before.map(), after.map()) {
        (Some(a), Some(b)) => {
            let name = |map: &axioval_engine::MapConversion| {
                map.target().unwrap_or("(unnamed)").to_owned()
            };
            if a.target() != b.target() {
                outcome.stated(facet, "map target", &name(a), &name(b));
            }
            match (a.offset_metres(), b.offset_metres()) {
                (Some(x), Some(y)) => {
                    outcome.exact(Measure::MapOffset, distance(x, y), tolerance.length_metres);
                }
                // Equal statements in one unit are equal whatever the unit.
                #[allow(clippy::float_cmp)] // Identical statements, not measurements.
                _ if a.offset() == b.offset()
                    && a.metres_per_map_unit() == b.metres_per_map_unit() => {}
                _ => outcome.unresolved(
                    facet,
                    "map offset",
                    "the map unit is not stated exactly, so the offsets cannot be measured in metres",
                ),
            }
            outcome.exact(
                Measure::MapRotation,
                plan_angle(a.x_axis(), b.x_axis()),
                tolerance.angle_radians,
            );
            outcome.exact(Measure::MapScale, (a.scale() - b.scale()).abs(), 0.0);
        }
        (None, None) => {}
        (a, b) => outcome.stated(
            facet,
            "map conversion",
            stated(a.is_some()),
            stated(b.is_some()),
        ),
    }
}

/// Pairs each base source with its revised counterpart: the only source of
/// each side, or else the one source of each side declaring a discipline.
/// Paired base and revised sources, and the sources left without a pair.
type SourcePairs = (Vec<(SourceId, SourceId)>, Vec<(Side, SourceId)>);

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

fn single(values: Option<&Vec<PropertyValue>>) -> Result<Option<&PropertyValue>, ()> {
    match values.map(Vec::as_slice) {
        None | Some([]) => Ok(None),
        Some([value]) => Ok(Some(value)),
        Some(_) => Err(()),
    }
}

fn resolve(
    resolver: &PropertyResolutionServiceHandle,
    object: &Object,
    property: &ComparedProperty,
) -> Result<Option<PropertyValue>, String> {
    let request = PropertyRequest::try_new(
        object.id.clone(),
        property.property_set.clone(),
        property.name.clone(),
    )
    .map_err(|error| error.to_string())?;
    match resolver.resolve(&request) {
        Ok(PropertyResolution::Present(present)) => Ok(Some(present.property().value.clone())),
        Ok(PropertyResolution::Absent(_)) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Value equality in which a value equals itself, NaN included, and the two
/// zeros are equal: a comparison must not report a property as changed
/// against its own unchanged value.
fn same_value(a: &PropertyValue, b: &PropertyValue) -> bool {
    #[allow(clippy::float_cmp)] // Exact equality is the question asked.
    let same = |x: f64, y: f64| x == y || (x.is_nan() && y.is_nan());
    match (a, b) {
        (PropertyValue::Decimal(x), PropertyValue::Decimal(y)) => same(*x, *y),
        (
            PropertyValue::Quantity {
                value: x,
                dimension: dx,
            },
            PropertyValue::Quantity {
                value: y,
                dimension: dy,
            },
        ) => dx == dy && same(*x, *y),
        _ => a == b,
    }
}

fn same_optional(a: Option<&PropertyValue>, b: Option<&PropertyValue>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_value(a, b),
        _ => false,
    }
}

/// Compares one matched pair on every requested facet.
fn matched(
    base: &Indexed<'_>,
    revised: &Indexed<'_>,
    request: &ComparisonRequest,
    old: &Object,
    new: &Object,
) -> ObjectChange {
    let mut pair = Pair {
        base,
        revised,
        request,
        outcome: Outcome::default(),
    };
    if old.kind != new.kind {
        pair.differ(Difference::Kind {
            base: old.kind.clone(),
            revised: new.kind.clone(),
        });
    }
    pair.classifications(old, new);
    pair.carried_properties(old, new);
    pair.requested_properties(old, new);
    pair.relationships(old, new);
    if let Some(tolerance) = request.placement {
        pair.placement(old, new, tolerance);
    }
    if let Some(tolerance) = request.geometry {
        pair.geometry(old, new, tolerance);
    }
    ObjectChange::Matched {
        base: old.id.clone(),
        revised: new.id.clone(),
        differences: pair.outcome.differences,
        unresolved: pair.outcome.unresolved,
        undetermined: pair.outcome.undetermined,
    }
}

/// Compares two sessions object by object, matched by `request`'s scheme.
#[must_use]
pub fn compare_sessions(
    base: &EvidenceSession,
    revised: &EvidenceSession,
    request: &ComparisonRequest,
) -> ModelComparison {
    let mut unidentified = Vec::new();
    let mut ambiguous = Vec::new();
    let base_index = index(
        base,
        Side::Base,
        &request.scheme,
        &mut unidentified,
        &mut ambiguous,
    );
    let revised_index = index(
        revised,
        Side::Revised,
        &request.scheme,
        &mut unidentified,
        &mut ambiguous,
    );
    // An identity ambiguous on either side matches nothing on the other.
    let blocked: BTreeSet<String> = ambiguous
        .iter()
        .map(|entry| entry.identity.clone())
        .collect();

    let identities: BTreeSet<&str> = base_index
        .by_identity
        .keys()
        .chain(revised_index.by_identity.keys())
        .copied()
        .filter(|identity| !blocked.contains(*identity))
        .collect();
    let mut objects = Vec::new();
    for identity in identities {
        let change = match (
            base_index.by_identity.get(identity),
            revised_index.by_identity.get(identity),
        ) {
            (Some(old), Some(new)) => matched(&base_index, &revised_index, request, old, new),
            (Some(old), None) => ObjectChange::Removed {
                base: old.id.clone(),
            },
            (None, Some(new)) => ObjectChange::Added {
                revised: new.id.clone(),
            },
            (None, None) => continue,
        };
        objects.push(ComparedObject {
            identity: identity.to_owned(),
            change,
        });
    }
    // Objects blocked by an ambiguity on the other side are not silently lost:
    // they are reported with the ambiguity itself.
    for side_index in [(&base_index, Side::Base), (&revised_index, Side::Revised)] {
        let (indexed, side) = side_index;
        for (identity, object) in &indexed.by_identity {
            if blocked.contains(*identity) {
                ambiguous.push(AmbiguousIdentity {
                    side,
                    identity: (*identity).to_owned(),
                    objects: vec![object.id.clone()],
                });
            }
        }
    }
    unidentified.sort();
    ambiguous.sort_by(|a, b| {
        a.identity
            .cmp(&b.identity)
            .then_with(|| a.side.cmp(&b.side))
    });
    let (sources, unpaired_sources) = match request.coordinate_systems {
        Some(tolerance) => {
            let (pairs, unpaired) = pair_sources(base, revised);
            let compared = pairs
                .iter()
                .map(|(a, b)| coordinate_systems((base, a), (revised, b), tolerance))
                .collect();
            (compared, unpaired)
        }
        None => (Vec::new(), Vec::new()),
    };
    ModelComparison {
        scheme: request.scheme.clone(),
        objects,
        unidentified,
        ambiguous,
        sources,
        unpaired_sources,
    }
}
