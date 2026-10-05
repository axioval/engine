//! Exact source-neutral property-resolution host-service contracts.

use axioval_ir::{Evidence, ObjectId, Property, PropertyValue, is_reserved_set};
use regex::Regex;
use std::sync::Arc;
use thiserror::Error;

use crate::session::{SnapshotBoundService, SourceSnapshot};

/// Failure to resolve a property conclusively.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PropertyResolutionError {
    /// The requested property reference is malformed.
    #[error("property request is invalid")]
    InvalidRequest,
    /// Returned data names another object or property.
    #[error("property response does not match its request")]
    ResponseRequestMismatch,
    /// A conclusive answer lacks exact, reviewable provenance.
    #[error("property evidence is not exact and reviewable")]
    InexactEvidence,
    /// A conclusive answer contains a value that is not a valid value of its
    /// kind: a non-finite number, or text that is not the date its source
    /// type declares.
    #[error("property value is invalid for its type")]
    InvalidValue,
    /// The source could answer only part of the request scope.
    #[error("property source coverage is incomplete: {0}")]
    Incomplete(String),
    /// Mutually incompatible exact facts were returned.
    #[error("property evidence conflicts: {0}")]
    Conflicting(String),
    /// The source cannot currently provide a conclusive answer.
    #[error("property resolution unavailable: {0}")]
    Unavailable(String),
    /// The source records this kind of property for no object at all (a
    /// model with no presentation layers), so an object's missing value is
    /// no evidence of absence. The message describes the source, never the
    /// object, so every object of the source answers identically.
    #[error("{0}")]
    NotRecorded(String),
    /// The run registered no service that could answer the request, for any
    /// object (a measured value without geometry). The message describes
    /// the run, never the object.
    #[error("{0}")]
    MissingService(String),
    /// The property is present and states a value of a type the source
    /// declares exactly, but the value cannot be read exactly (a measure
    /// whose unit does not resolve). What depends on the declared type and
    /// on presence alone is decided; the value is not.
    #[error("{}", .0.reason())]
    UnreadableValue(Box<UnreadableValue>),
    /// A measured value's argument, bound from a rule's parameter, is not
    /// one it can measure with (a parameter the rule does not state, of
    /// another kind, or not realisable): the rule's declaration is invalid,
    /// whatever the object.
    #[error("{0}")]
    InvalidArgument(String),
}

/// A present property whose declared type is known exactly and whose
/// stated value cannot be read exactly, bound to the request it answers.
///
/// The source states a value (never `$`): the property exists and holds
/// one. Only the value itself is unknown, so a capability may decide
/// presence, non-emptiness and the declared type, and must leave every
/// comparison of the value not evaluated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableValue {
    request: PropertyRequest,
    data_type: String,
    evidence: Evidence,
    reason: String,
}
impl UnreadableValue {
    /// Creates a request-bound answer: the property holds a value of
    /// `data_type`, in the source's own vocabulary, that cannot be read
    /// exactly for `reason`.
    ///
    /// # Errors
    ///
    /// [`PropertyResolutionError::InvalidRequest`] for a blank type or
    /// reason, and [`PropertyResolutionError::InexactEvidence`] for evidence
    /// that is not exact and reviewable or not from the requested object's
    /// source.
    pub fn try_new(
        request: PropertyRequest,
        data_type: impl Into<String>,
        evidence: Evidence,
        reason: impl Into<String>,
    ) -> Result<Self, PropertyResolutionError> {
        let (data_type, reason) = (data_type.into(), reason.into());
        if data_type.trim().is_empty() || reason.trim().is_empty() {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        if !reviewable(&evidence) || evidence.source != request.object_id().source {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        Ok(Self {
            request,
            data_type,
            evidence,
            reason,
        })
    }
    /// Bound request.
    pub fn request(&self) -> &PropertyRequest {
        &self.request
    }
    /// The value's type as the source declares it (an IFC property:
    /// `IFCMASSMEASURE`).
    pub fn data_type(&self) -> &str {
        &self.data_type
    }
    /// Exact reviewable provenance of the property.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
    /// Why the value cannot be read exactly.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Request for one direct property on one source-qualified object.
///
/// This contract covers occurrence/type inheritance owned by the source adapter,
/// but never traverses semantic relationships to other objects. Related-object
/// selection requires a separately complete relationship service.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PropertyRequest {
    object_id: ObjectId,
    property_set: Option<String>,
    property: String,
}
impl PropertyRequest {
    /// Creates a request. An omitted set requests an unambiguous property by name.
    pub fn try_new(
        object_id: ObjectId,
        property_set: Option<String>,
        property: impl Into<String>,
    ) -> Result<Self, PropertyResolutionError> {
        let property = property.into();
        if property.trim().is_empty()
            || property_set
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Ok(Self {
            object_id,
            property_set,
            property,
        })
    }
    /// Requested object.
    pub fn object_id(&self) -> &ObjectId {
        &self.object_id
    }
    /// Optional requested property set.
    pub fn property_set(&self) -> Option<&str> {
        self.property_set.as_deref()
    }
    /// Requested property name.
    pub fn property(&self) -> &str {
        &self.property
    }
    fn matches(&self, property: &Property) -> bool {
        property.name == self.property
            && self
                .property_set
                .as_ref()
                .is_none_or(|set| property.property_set == *set)
    }
}

/// Exact proof that a requested property is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletePropertyAbsenceEvidence {
    request: PropertyRequest,
    evidence: Evidence,
}
impl CompletePropertyAbsenceEvidence {
    /// Creates request-bound exact absence evidence.
    pub fn try_new(
        request: PropertyRequest,
        evidence: Evidence,
    ) -> Result<Self, PropertyResolutionError> {
        if !reviewable(&evidence) || evidence.source != request.object_id().source {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        Ok(Self { request, evidence })
    }
    /// Bound request.
    pub fn request(&self) -> &PropertyRequest {
        &self.request
    }
    /// Exact reviewable provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Exact property value bound to the request that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProperty {
    request: PropertyRequest,
    property: Property,
}
impl ResolvedProperty {
    /// Creates an exact request-bound property value.
    pub fn try_new(
        request: PropertyRequest,
        property: Property,
    ) -> Result<Self, PropertyResolutionError> {
        if !request.matches(&property) {
            return Err(PropertyResolutionError::ResponseRequestMismatch);
        }
        if !admissible_evidence(&request, &property) {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        if !valid_property(&property) {
            return Err(PropertyResolutionError::InvalidValue);
        }
        Ok(Self { request, property })
    }
    /// Bound request, including the source-qualified object identity.
    pub fn request(&self) -> &PropertyRequest {
        &self.request
    }
    /// Exact typed property and its reviewable provenance.
    pub fn property(&self) -> &Property {
        &self.property
    }
    /// The property, taken out of its binding once it has been checked.
    #[must_use]
    pub fn into_property(self) -> Property {
        self.property
    }
}

/// Conclusive property result from a trusted source adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyResolution {
    /// The exact request-bound property value and its provenance.
    Present(ResolvedProperty),
    /// Exact proof that the requested property is absent.
    Absent(CompletePropertyAbsenceEvidence),
}

/// A regular expression over whole property-set or property names.
///
/// The syntax is the `regex` crate's; callers holding an XML Schema pattern
/// translate it first (`axioval_rules::translate_xsd_pattern`). The pattern
/// always matches a whole name. Two patterns are equal when their text is.
#[derive(Clone, Debug)]
pub struct NamePattern {
    pattern: String,
    regex: Regex,
}
impl NamePattern {
    /// Compiles `pattern` to match whole names.
    ///
    /// # Errors
    ///
    /// [`PropertyResolutionError::InvalidRequest`] when the pattern does not
    /// compile.
    pub fn new(pattern: impl Into<String>) -> Result<Self, PropertyResolutionError> {
        let pattern = pattern.into();
        let regex = Regex::new(&format!(r"\A(?:{pattern})\z"))
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        Ok(Self { pattern, regex })
    }
    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        &self.pattern
    }
    /// Whether the whole of `name` matches.
    pub fn is_match(&self, name: &str) -> bool {
        self.regex.is_match(name)
    }
}
impl PartialEq for NamePattern {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
    }
}
impl Eq for NamePattern {}
impl PartialOrd for NamePattern {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for NamePattern {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.pattern.cmp(&other.pattern)
    }
}

/// Which property-set or property names an enumeration selects.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameMatch {
    /// Every name.
    Any,
    /// One exact name, compared as the source spells it.
    Exact(String),
    /// Every name the pattern matches as a whole.
    Pattern(NamePattern),
}
impl NameMatch {
    /// Whether `name` is selected.
    pub fn matches(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(exact) => exact == name,
            Self::Pattern(pattern) => pattern.is_match(name),
        }
    }
}
impl std::fmt::Display for NameMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::Exact(name) => f.write_str(name),
            Self::Pattern(pattern) => write!(f, "/{}/", pattern.as_str()),
        }
    }
}

/// Request for every property of one object whose set and name match.
///
/// Enumeration covers the source's own property sets, as
/// [`PropertyRequest`] does with occurrence/type inheritance, and never the
/// reserved sets ([`axioval_ir::is_reserved_set`]), which name engine
/// vocabulary and are resolved by name only: an exact reserved set is an
/// invalid request, and a pattern never selects one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PropertyEnumerationRequest {
    object_id: ObjectId,
    property_set: NameMatch,
    property: NameMatch,
}
impl PropertyEnumerationRequest {
    /// Creates a request.
    ///
    /// # Errors
    ///
    /// [`PropertyResolutionError::InvalidRequest`] for a blank exact name or
    /// an exact reserved set.
    pub fn try_new(
        object_id: ObjectId,
        property_set: NameMatch,
        property: NameMatch,
    ) -> Result<Self, PropertyResolutionError> {
        let blank =
            |name: &NameMatch| matches!(name, NameMatch::Exact(name) if name.trim().is_empty());
        if blank(&property_set)
            || blank(&property)
            || matches!(&property_set, NameMatch::Exact(set) if is_reserved_set(set))
        {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Ok(Self {
            object_id,
            property_set,
            property,
        })
    }
    /// Requested object.
    pub fn object_id(&self) -> &ObjectId {
        &self.object_id
    }
    /// Which property sets are searched.
    pub fn property_set(&self) -> &NameMatch {
        &self.property_set
    }
    /// Which properties of those sets are selected.
    pub fn property(&self) -> &NameMatch {
        &self.property
    }
    /// Whether `property` is one the request selects.
    pub fn selects(&self, property: &Property) -> bool {
        !is_reserved_set(&property.property_set)
            && self.property_set.matches(&property.property_set)
            && self.property.matches(&property.name)
    }
}

/// Every property an enumeration request selects, exactly, with the evidence
/// that nothing else is selected.
///
/// Properties are sorted by set and name; no set and name occurs twice. An
/// empty enumeration is an exact proof that the object has no selected
/// property, as [`CompletePropertyAbsenceEvidence`] is for one name.
///
/// A set the object carries without any member holds no property, so it
/// leaves no trace among the properties. A source that can tell such a set
/// apart from no set at all names it in [`Self::empty_sets`], so a rule
/// requiring a property in every selected set fails on it, as IDS requires.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyEnumeration {
    request: PropertyEnumerationRequest,
    properties: Vec<Property>,
    evidence: Evidence,
    empty_sets: Vec<String>,
}
impl PropertyEnumeration {
    /// Creates a request-bound enumeration.
    ///
    /// # Errors
    ///
    /// [`PropertyResolutionError::InexactEvidence`] when the completeness
    /// evidence or a property's evidence is not exact, reviewable and from
    /// the object's source; [`PropertyResolutionError::ResponseRequestMismatch`]
    /// for a property the request does not select;
    /// [`PropertyResolutionError::InvalidValue`] for an invalid value; and
    /// [`PropertyResolutionError::Conflicting`] when a set and name occur
    /// twice.
    pub fn try_new(
        request: PropertyEnumerationRequest,
        mut properties: Vec<Property>,
        evidence: Evidence,
    ) -> Result<Self, PropertyResolutionError> {
        let source = &request.object_id().source;
        if !reviewable(&evidence) || evidence.source != *source {
            return Err(PropertyResolutionError::InexactEvidence);
        }
        for property in &properties {
            if !request.selects(property) {
                return Err(PropertyResolutionError::ResponseRequestMismatch);
            }
            if !property
                .evidence
                .as_ref()
                .is_some_and(|evidence| reviewable(evidence) && evidence.source == *source)
            {
                return Err(PropertyResolutionError::InexactEvidence);
            }
            if !valid_property(property) {
                return Err(PropertyResolutionError::InvalidValue);
            }
        }
        properties.sort_by(|a, b| (&a.property_set, &a.name).cmp(&(&b.property_set, &b.name)));
        if let Some(pair) = properties.windows(2).find(|pair| {
            pair[0].property_set == pair[1].property_set && pair[0].name == pair[1].name
        }) {
            return Err(PropertyResolutionError::Conflicting(format!(
                "{}.{} is enumerated twice",
                pair[0].property_set, pair[0].name
            )));
        }
        Ok(Self {
            request,
            properties,
            evidence,
            empty_sets: Vec::new(),
        })
    }
    /// The same enumeration, also naming the selected sets the object carries
    /// without any member, sorted and each once.
    ///
    /// # Errors
    ///
    /// [`PropertyResolutionError::ResponseRequestMismatch`] for a set the
    /// request does not select, or of a reserved name; and
    /// [`PropertyResolutionError::Conflicting`] for a set an enumerated
    /// property is in, which is not empty.
    pub fn with_empty_sets(
        mut self,
        sets: impl IntoIterator<Item = String>,
    ) -> Result<Self, PropertyResolutionError> {
        let mut sets: Vec<String> = sets.into_iter().collect();
        for set in &sets {
            if is_reserved_set(set) || !self.request.property_set().matches(set) {
                return Err(PropertyResolutionError::ResponseRequestMismatch);
            }
            if self
                .properties
                .iter()
                .any(|property| property.property_set == *set)
            {
                return Err(PropertyResolutionError::Conflicting(format!(
                    "set {set} is reported empty and holds a property"
                )));
            }
        }
        sets.sort();
        sets.dedup();
        self.empty_sets = sets;
        Ok(self)
    }
    /// Bound request.
    pub fn request(&self) -> &PropertyEnumerationRequest {
        &self.request
    }
    /// The selected properties, sorted by set and name.
    pub fn properties(&self) -> &[Property] {
        &self.properties
    }
    /// Exact reviewable evidence that the enumeration is complete.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
    /// The selected sets the object carries without any member, sorted. The
    /// same evidence proves there are no others; a source that cannot tell
    /// an empty set from none reports none.
    pub fn empty_sets(&self) -> &[String] {
        &self.empty_sets
    }
}

/// Trusted adapter seam for property resolution.
pub trait PropertyResolutionService: Send + Sync {
    /// Exact source snapshots used to construct this resolver.
    ///
    /// The default is intentionally unbound for services used only through a
    /// raw [`crate::ServiceRegistry`]; an [`crate::EvidenceSession`] rejects it.
    // gate: reads
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &[]
    }
    /// Resolves one request or reports why it is not conclusive.
    // gate: reads
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError>;
    /// Enumerates every property of one object the request selects, or
    /// reports why the enumeration is not conclusive.
    ///
    /// The default refuses: a source that cannot list an object's
    /// properties never answers with an empty enumeration, which would be a
    /// proof of absence.
    // gate: reads
    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        let _ = request;
        Err(PropertyResolutionError::Unavailable(
            "this property source cannot enumerate an object's properties".into(),
        ))
    }
}

/// Cloneable, type-erased property service registered by the host.
#[derive(Clone)]
pub struct PropertyResolutionServiceHandle {
    service: Arc<dyn PropertyResolutionService>,
}
impl PropertyResolutionServiceHandle {
    /// Wraps a trusted service for use outside an evidence session.
    pub fn new(service: Arc<dyn PropertyResolutionService>) -> Self {
        Self { service }
    }
    /// Resolves and validates request binding and exact provenance.
    ///
    /// A [`PropertyResolutionError::UnreadableValue`] is an answer too: it
    /// is bound to this very request and its evidence checked like a
    /// present value's.
    pub fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let resolution = match self.service.resolve(request) {
            Err(PropertyResolutionError::UnreadableValue(unreadable)) => {
                if unreadable.request() != request {
                    return Err(PropertyResolutionError::ResponseRequestMismatch);
                }
                if !reviewable(unreadable.evidence())
                    || unreadable.evidence().source != request.object_id().source
                {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
                return Err(PropertyResolutionError::UnreadableValue(unreadable));
            }
            other => other?,
        };
        match &resolution {
            PropertyResolution::Present(resolved) => {
                if resolved.request() != request || !request.matches(resolved.property()) {
                    return Err(PropertyResolutionError::ResponseRequestMismatch);
                }
                if !admissible_evidence(request, resolved.property()) {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
                if !valid_property(resolved.property()) {
                    return Err(PropertyResolutionError::InvalidValue);
                }
            }
            PropertyResolution::Absent(evidence) => {
                if evidence.request() != request {
                    return Err(PropertyResolutionError::ResponseRequestMismatch);
                }
                if !reviewable(evidence.evidence())
                    || evidence.evidence().source != request.object_id().source
                {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
            }
        }
        Ok(resolution)
    }
    /// Enumerates and validates request binding and exact provenance.
    ///
    /// The enumeration's own constructor already checked every property
    /// against the request; this binds the answer to this very request.
    pub fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        let enumeration = self.service.enumerate(request)?;
        if enumeration.request() != request {
            return Err(PropertyResolutionError::ResponseRequestMismatch);
        }
        Ok(enumeration)
    }
}

impl SnapshotBoundService for PropertyResolutionServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.service.source_snapshots()
    }
}

/// Whether a property's value is well formed: `valid_value`, and a
/// complex property declares no type, since it holds no value of one.
fn valid_property(property: &Property) -> bool {
    valid_value(&property.value)
        && !(matches!(property.value, PropertyValue::Complex)
            && (property.data_type().is_some() || property.column_types().is_some()))
}

fn valid_value(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::Decimal(value) | PropertyValue::Quantity { value, .. } => value.is_finite(),
        PropertyValue::Null
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::String(_)
        | PropertyValue::Date(_)
        | PropertyValue::DateTime(_)
        | PropertyValue::Reference(_)
        | PropertyValue::Complex => true,
        // A list holds scalar values only: no null, no nested composite.
        PropertyValue::List(elements) => elements.iter().all(valid_scalar),
        // A range states at least one scalar, and one kind of value.
        PropertyValue::Bounded { .. } => value.stated_values().is_some_and(|stated| {
            !stated.is_empty()
                && stated.iter().all(|part| valid_scalar(part))
                && stated
                    .windows(2)
                    .all(|pair| std::mem::discriminant(pair[0]) == std::mem::discriminant(pair[1]))
        }),
        PropertyValue::Measured { lower, upper, .. } => {
            lower.is_finite() && upper.is_finite() && lower <= upper
        }
        PropertyValue::Table(rows) => {
            !rows.is_empty()
                && rows
                    .iter()
                    .all(|row| valid_scalar(&row.defining) && valid_scalar(&row.defined))
        }
    }
}

fn valid_scalar(value: &PropertyValue) -> bool {
    value.is_scalar() && valid_value(value)
}

/// Whether `property` carries evidence from the requested object's source
/// that a reviewer can follow: exact, or for a measured interval (which is
/// never exact) at least located.
fn admissible_evidence(request: &PropertyRequest, property: &Property) -> bool {
    let interval = matches!(property.value, PropertyValue::Measured { .. });
    // A reference names an instance of the requested object's own source.
    if let PropertyValue::Reference(target) = &property.value
        && target.source != request.object_id().source
    {
        return false;
    }
    property.evidence.as_ref().is_some_and(|evidence| {
        (reviewable(evidence) || interval && !evidence.locator.trim().is_empty())
            && evidence.source == request.object_id().source
    })
}

fn reviewable(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
