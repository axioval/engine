//! Semantic comparison of two evidence sessions.
//!
//! Objects are matched across sessions by an external identity scheme, never
//! by [`ObjectId`]: an object's local id is whatever its source file numbered
//! it, and a re-export renumbers everything. The scheme is a parameter because
//! the engine attaches no meaning to it; an IFC session supplies `GlobalId`s
//! under its adapter's scheme, another source its own.
//!
//! The comparison is purely semantic. It reads kinds, classifications,
//! properties and relationships, and never geometry.
//!
//! Nothing is silently dropped. An object without an identity in the scheme
//! cannot be matched and is reported as unidentified. An identity held by two
//! objects of one session is ambiguous and matches nothing. A facet that
//! cannot be read exactly on both sides -- a property resolver error, or a
//! relationship target without an identity -- is reported unresolved rather
//! than compared as if it were empty.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    ClassificationServiceHandle, EvidenceSession, PropertyRequest, PropertyResolution,
    PropertyResolutionServiceHandle,
};
use axioval_ir::{
    Evidence, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, PropertyValue, Report,
    RuleId, Severity,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonRequest {
    scheme: String,
    properties: BTreeSet<ComparedProperty>,
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

    /// The identity scheme objects are matched by.
    pub fn scheme(&self) -> &str {
        &self.scheme
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

/// One semantic difference between matched objects.
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
        Some(PropertyValue::String(value)) => format!("{value:?}"),
    }
}

/// A facet of a matched pair that could not be compared.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unresolved {
    /// What was not compared, such as `property Pset.Name`.
    pub facet: String,
    /// Why it could not be.
    pub reason: String,
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
    /// Present in both. Unchanged when both lists are empty.
    Matched {
        /// The object in the base session.
        base: ObjectId,
        /// The object in the revised session.
        revised: ObjectId,
        /// Semantic differences, in facet order.
        differences: Vec<Difference>,
        /// Facets that could not be compared.
        unresolved: Vec<Unresolved>,
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

/// The result of comparing two sessions, ordered by identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelComparison {
    scheme: String,
    objects: Vec<ComparedObject>,
    unidentified: Vec<(Side, ObjectId)>,
    ambiguous: Vec<AmbiguousIdentity>,
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
    /// Whether the sessions agree completely on everything compared.
    pub fn is_identical(&self) -> bool {
        self.unidentified.is_empty()
            && self.ambiguous.is_empty()
            && self.objects.iter().all(|object| {
                matches!(&object.change, ObjectChange::Matched { differences, unresolved, .. }
                    if differences.is_empty() && unresolved.is_empty())
            })
    }

    /// Not-evaluated outcomes for objects that could not be matched at all.
    fn identity_gaps(&self, rule_id: &RuleId) -> Vec<NotEvaluated> {
        let mut not_evaluated = Vec::new();
        for (side, object) in &self.unidentified {
            not_evaluated.push(NotEvaluated {
                rule_id: rule_id.clone(),
                object_id: Some(object.clone()),
                reason: NotEvaluatedReason::IncompleteEvidence,
                message: format!(
                    "{side:?} object has no `{}` identity and cannot be matched",
                    self.scheme
                ),
            });
        }
        for ambiguous in &self.ambiguous {
            for object in &ambiguous.objects {
                not_evaluated.push(NotEvaluated {
                    rule_id: rule_id.clone(),
                    object_id: Some(object.clone()),
                    reason: NotEvaluatedReason::InvalidEvidence,
                    message: format!(
                        "{:?} identity {}:{} is claimed by {} objects",
                        ambiguous.side,
                        self.scheme,
                        ambiguous.identity,
                        ambiguous.objects.len()
                    ),
                });
            }
        }
        not_evaluated
    }

    /// Projects the comparison into a report, one finding per added, removed
    /// or changed object, so it can travel through any report sink.
    ///
    /// Everything the comparison could not decide becomes not-evaluated, not
    /// silence: an unidentified object may be the one that changed.
    #[must_use]
    pub fn report(&self, rule_id: &RuleId, severity: &Severity) -> Report {
        let mut findings = Vec::new();
        let mut not_evaluated = Vec::new();
        let evidence = |object: &ObjectId, identity: &str| Evidence {
            source: object.source.clone(),
            locator: format!("comparison:{}:{identity}", self.scheme),
            exact: true,
        };
        let mut finding =
            |object: &ObjectId, related: Option<&ObjectId>, message: String, identity: &str| {
                findings.push(
                    Finding {
                        rule_id: rule_id.clone(),
                        object_id: object.clone(),
                        severity: severity.clone(),
                        message,
                        related: Vec::new(),
                        evidence: vec![evidence(object, identity)],
                    }
                    .with_related(related.cloned()),
                );
            };
        for compared in &self.objects {
            let identity = compared.identity.as_str();
            match &compared.change {
                ObjectChange::Added { revised } => {
                    finding(
                        revised,
                        None,
                        format!("added ({}:{identity})", self.scheme),
                        identity,
                    );
                }
                ObjectChange::Removed { base } => {
                    finding(
                        base,
                        None,
                        format!("removed ({}:{identity})", self.scheme),
                        identity,
                    );
                }
                ObjectChange::Matched {
                    base,
                    revised,
                    differences,
                    unresolved,
                } => {
                    if !differences.is_empty() {
                        let changes: Vec<String> =
                            differences.iter().map(ToString::to_string).collect();
                        finding(
                            revised,
                            Some(base),
                            format!("changed: {}", changes.join("; ")),
                            identity,
                        );
                    }
                    for gap in unresolved {
                        not_evaluated.push(NotEvaluated {
                            rule_id: rule_id.clone(),
                            object_id: Some(revised.clone()),
                            reason: NotEvaluatedReason::IncompleteEvidence,
                            message: format!("{} not compared: {}", gap.facet, gap.reason),
                        });
                    }
                }
            }
        }
        not_evaluated.extend(self.identity_gaps(rule_id));
        findings.sort_by(|a, b| {
            a.rule_id
                .cmp(&b.rule_id)
                .then_with(|| a.object_id.cmp(&b.object_id))
                .then_with(|| a.message.cmp(&b.message))
        });
        not_evaluated.sort();
        Report {
            findings,
            not_evaluated,
        }
    }
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

struct Pair<'a> {
    base: &'a Indexed<'a>,
    revised: &'a Indexed<'a>,
    request: &'a ComparisonRequest,
    differences: Vec<Difference>,
    unresolved: Vec<Unresolved>,
}

impl Pair<'_> {
    fn unresolved(&mut self, facet: impl Into<String>, reason: impl Into<String>) {
        self.unresolved.push(Unresolved {
            facet: facet.into(),
            reason: reason.into(),
        });
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
                "classifications",
                "the sessions state classifications through different services",
            );
            return;
        }
        match (
            base_basis.statements(base),
            revised_basis.statements(revised),
        ) {
            (Ok(before), Ok(after)) if before != after => {
                self.differences.push(Difference::Classifications {
                    removed: before.difference(&after).cloned().collect(),
                    added: after.difference(&before).cloned().collect(),
                });
            }
            (Ok(_), Ok(_)) => {}
            (Err(reason), _) | (_, Err(reason)) => self.unresolved("classifications", reason),
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
                self.unresolved(format!("property {key}"), "stated more than once");
                continue;
            };
            if !same_optional(old, new) {
                self.differences.push(Difference::Property {
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
            let facet = format!("property {property}");
            let (Some(base_resolver), Some(revised_resolver)) = resolvers else {
                self.unresolved(facet, "a session has no property resolver");
                continue;
            };
            match (
                resolve(base_resolver, base, property),
                resolve(revised_resolver, revised, property),
            ) {
                (Ok(old), Ok(new)) => {
                    if !same_optional(old.as_ref(), new.as_ref()) {
                        self.differences.push(Difference::Property {
                            property: property.clone(),
                            base: old,
                            revised: new,
                        });
                    }
                }
                (Err(reason), _) | (_, Err(reason)) => self.unresolved(facet, reason),
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
                        self.differences.push(Difference::Relationship {
                            name: name.clone(),
                            removed: before.difference(&after).cloned().collect(),
                            added: after.difference(&before).cloned().collect(),
                        });
                    }
                }
                _ => self.unresolved(
                    format!("relationship {name}"),
                    "a target has no unique identity in the scheme",
                ),
            }
        }
    }
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
            (Some(old), Some(new)) => {
                let mut pair = Pair {
                    base: &base_index,
                    revised: &revised_index,
                    request,
                    differences: Vec::new(),
                    unresolved: Vec::new(),
                };
                if old.kind != new.kind {
                    pair.differences.push(Difference::Kind {
                        base: old.kind.clone(),
                        revised: new.kind.clone(),
                    });
                }
                pair.classifications(old, new);
                pair.carried_properties(old, new);
                pair.requested_properties(old, new);
                pair.relationships(old, new);
                ObjectChange::Matched {
                    base: old.id.clone(),
                    revised: new.id.clone(),
                    differences: pair.differences,
                    unresolved: pair.unresolved,
                }
            }
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
    ModelComparison {
        scheme: request.scheme.clone(),
        objects,
        unidentified,
        ambiguous,
    }
}
