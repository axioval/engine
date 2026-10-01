//! The facets of one matched pair and of one pair of sources.

use std::collections::{BTreeMap, BTreeSet};

use std::sync::Arc;

use axioval_engine::{
    ClassificationServiceHandle, CoordinateFrame, CoordinateSystemServiceHandle, MetricDirection,
    ObjectFrameError, ObjectFrameServiceHandle, PropertyResolutionServiceHandle, ProximityError,
    ProximityServiceHandle, RelationshipEdgesRequest, RelationshipKind,
    RelationshipSelectionServiceHandle, SourceCoordinateSystem, SurfaceDirection,
    SurfaceDistanceRequest,
};
use axioval_ir::{DateTime, Object, ObjectId, Property, PropertyValue, SourceId};

use super::{
    ComparedProperty, ComparisonRequest, ComparisonTolerance, Difference, Facet, GeometryMode,
    Measure, Measurement, ObjectChange, Revision, Side, SourceComparison, Unresolved, Witness,
};
use crate::body_facts::BodyFacts;
use crate::selection::{NameSpec, enumerate};
use crate::support::{PropertyRef, display, resolve};

/// Where a revision's classification statements come from.
enum ClassificationBasis<'a> {
    Service(&'a ClassificationServiceHandle),
    Carried,
}

impl<'a> ClassificationBasis<'a> {
    fn of(revision: &Revision<'a>) -> Self {
        revision
            .context
            .services
            .get::<ClassificationServiceHandle>()
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
pub(super) struct Outcome {
    pub(super) differences: Vec<Difference>,
    pub(super) unresolved: Vec<Unresolved>,
    pub(super) undetermined: Vec<Measurement>,
}

impl Outcome {
    pub(super) fn unresolved(
        &mut self,
        facet: Facet,
        subject: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.unresolved.push(Unresolved {
            facet,
            subject: subject.into(),
            reason: reason.into(),
        });
    }

    pub(super) fn stated(&mut self, facet: Facet, subject: &str, base: &str, revised: &str) {
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
    pub(super) fn measured(&mut self, measure: Measure, lower: f64, upper: f64, tolerance: f64) {
        self.witnessed(measure, (lower, upper), tolerance, None);
    }

    /// As [`Self::measured`], with where the difference is realised.
    pub(super) fn witnessed(
        &mut self,
        measure: Measure,
        (lower, upper): (f64, f64),
        tolerance: f64,
        witness: Option<Witness>,
    ) {
        let measurement = Measurement {
            measure,
            lower,
            upper,
            tolerance,
            witness,
        };
        if lower > tolerance {
            self.differences.push(Difference::Measured(measurement));
        } else if upper > tolerance {
            self.undetermined.push(measurement);
        }
    }

    /// Judges an exactly known difference.
    pub(super) fn exact(&mut self, measure: Measure, value: f64, tolerance: f64) {
        self.measured(measure, value, value, tolerance);
    }
}

/// How the objects of each revision are named when they are the target of a
/// relationship: by the identity of the pair they belong to, or else by
/// their identity in a scheme the comparison matches by.
pub(super) struct TargetNames<'o> {
    pub(super) base: BTreeMap<&'o ObjectId, String>,
    pub(super) revised: BTreeMap<&'o ObjectId, String>,
}

/// How a related object is named when two related-object sets are compared.
#[derive(Clone, Debug)]
pub(super) enum RelatedName {
    /// The same name on both sides: a matched pair's identity, or the
    /// scheme identity of an object the comparison does not match.
    Same(String),
    /// An object matched with nothing, held by its own side only.
    Unmatched(String),
}

/// What one side's objects relate to, one set per object at either end.
type Related = BTreeMap<ObjectId, BTreeSet<ObjectId>>;

/// One side's related objects per object, for every relationship kind.
pub(super) struct KindSide {
    /// Per kind, every object's related objects, or why the kind could not
    /// be listed.
    related: BTreeMap<RelationshipKind, Result<Related, String>>,
    /// The names related objects are compared by; an object missing here
    /// has no decided match.
    names: BTreeMap<ObjectId, RelatedName>,
}

/// The objects related to one object through one kind, by how they are
/// named: matched, unmatched, and undecided.
type Sorted = (BTreeSet<String>, Vec<String>, Vec<String>);

impl KindSide {
    /// Lists every kind's edges among `revision`'s objects once, through the
    /// revision's relationship service.
    pub(super) fn of(revision: &Revision<'_>, names: BTreeMap<ObjectId, RelatedName>) -> Self {
        let universe: Vec<ObjectId> = revision
            .objects
            .iter()
            .map(|object| object.id.clone())
            .collect();
        let service = revision
            .context
            .services
            .get::<RelationshipSelectionServiceHandle>();
        let related = RelationshipKind::ALL
            .into_iter()
            .map(|kind| {
                let listed = service
                    .ok_or_else(|| "a session has no relationship-selection service".to_owned())
                    .and_then(|service| {
                        RelationshipEdgesRequest::try_new(universe.clone(), kind.relationship())
                            .and_then(|request| service.edges(&request))
                            .map_err(|error| error.to_string())
                    })
                    .map(|listing| {
                        let mut related = Related::new();
                        for edge in listing.edges() {
                            if edge.relating == edge.related {
                                continue;
                            }
                            related
                                .entry(edge.relating.clone())
                                .or_default()
                                .insert(edge.related.clone());
                            related
                                .entry(edge.related.clone())
                                .or_default()
                                .insert(edge.relating.clone());
                        }
                        related
                    });
                (kind, listed)
            })
            .collect();
        Self { related, names }
    }

    fn sorted(&self, kind: RelationshipKind, object: &ObjectId) -> Result<Sorted, String> {
        let related = match self.related.get(&kind) {
            Some(Ok(related)) => related.get(object),
            Some(Err(reason)) => return Err(reason.clone()),
            None => None,
        };
        let (mut same, mut unmatched, mut undecided) = (BTreeSet::new(), Vec::new(), Vec::new());
        for target in related.into_iter().flatten() {
            match self.names.get(target) {
                Some(RelatedName::Same(name)) => {
                    same.insert(name.clone());
                }
                Some(RelatedName::Unmatched(name)) => unmatched.push(name.clone()),
                None => undecided.push(target.to_string()),
            }
        }
        Ok((same, unmatched, undecided))
    }
}

/// Both sides' related objects per relationship kind.
pub(super) struct KindRelations {
    pub(super) base: KindSide,
    pub(super) revised: KindSide,
}

struct Pair<'r, 'a> {
    base: &'r Revision<'a>,
    revised: &'r Revision<'a>,
    request: &'r ComparisonRequest,
    names: &'r TargetNames<'a>,
    kinds: Option<&'r KindRelations>,
    outcome: Outcome,
}

impl Pair<'_, '_> {
    fn unresolved(&mut self, facet: Facet, subject: impl Into<String>, reason: impl Into<String>) {
        self.outcome.unresolved(facet, subject, reason);
    }

    fn differ(&mut self, difference: Difference) {
        self.outcome.differences.push(difference);
    }

    fn classifications(&mut self, base: &Object, revised: &Object) {
        let (base_basis, revised_basis) = (
            ClassificationBasis::of(self.base),
            ClassificationBasis::of(self.revised),
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

    /// Whether `key` is compared some other way: named, or in a set compared
    /// whole through enumeration.
    fn compared_otherwise(&self, key: &ComparedProperty) -> bool {
        self.request.properties.contains(key)
            || key.property_set.as_ref().is_some_and(|set| {
                self.request.all_property_sets || self.request.property_sets.contains(set)
            })
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
            if self.compared_otherwise(key) {
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
                .context
                .services
                .get::<PropertyResolutionServiceHandle>(),
            self.revised
                .context
                .services
                .get::<PropertyResolutionServiceHandle>(),
        );
        let request = self.request;
        for property in &request.properties {
            if resolvers.0.is_none() || resolvers.1.is_none() {
                self.unresolved(
                    Facet::Property,
                    property.to_string(),
                    "a session has no property resolver",
                );
                continue;
            }
            let named = PropertyRef {
                set: property.property_set(),
                name: property.name(),
            };
            let value = |revision: &Revision<'_>, object: &Object| {
                resolve(&revision.context, object, named)
                    .map(|resolved| resolved.value().cloned())
                    .map_err(|(_, message)| message)
            };
            match (value(self.base, base), value(self.revised, revised)) {
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

    /// Properties of whole sets, listed through each revision's enumeration:
    /// every set with `all_property_sets`, otherwise the named ones.
    fn property_sets(&mut self, base: &Object, revised: &Object) {
        let request = self.request;
        let named: Vec<Option<&str>> = if request.all_property_sets {
            vec![None]
        } else {
            request
                .property_sets
                .iter()
                .map(|set| Some(set.as_str()))
                .collect()
        };
        for set in named {
            let subject = set.unwrap_or("*");
            let spec = set.map_or(NameSpec::Any, NameSpec::Exact);
            let listed = |revision: &Revision<'_>, object: &Object| {
                enumerate(&revision.context, object, spec, NameSpec::Any)
                    .map(|enumeration| enumeration.properties().to_vec())
                    .map_err(|(_, message)| message)
            };
            let (before, after) = match (listed(self.base, base), listed(self.revised, revised)) {
                (Ok(before), Ok(after)) => (before, after),
                (Err(reason), _) | (_, Err(reason)) => {
                    self.unresolved(Facet::Property, subject, reason);
                    continue;
                }
            };
            self.compare_listed(&before, &after);
        }
    }

    /// Compares two listings property by property.
    fn compare_listed(&mut self, before: &[Property], after: &[Property]) {
        let collect = |listed: &[Property]| {
            let mut values: BTreeMap<ComparedProperty, Vec<PropertyValue>> = BTreeMap::new();
            for property in listed {
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
        let (before, after) = (collect(before), collect(after));
        let sets = |values: &BTreeMap<ComparedProperty, Vec<PropertyValue>>| -> BTreeSet<String> {
            values
                .keys()
                .filter_map(|key| key.property_set.clone())
                .collect()
        };
        let (sets_before, sets_after) = (sets(&before), sets(&after));
        // A set on one side only is one difference, not one per property.
        for set in sets_before.symmetric_difference(&sets_after) {
            self.differ(Difference::PropertySet {
                property_set: set.clone(),
                added: sets_after.contains(set),
            });
        }
        let keys: BTreeSet<&ComparedProperty> = before.keys().chain(after.keys()).collect();
        for key in keys {
            // A property also named is compared through the resolver alone.
            if self.request.properties.contains(key)
                || key
                    .property_set
                    .as_ref()
                    .is_some_and(|set| sets_before.contains(set) != sets_after.contains(set))
            {
                continue;
            }
            let (Ok(old), Ok(new)) = (single(before.get(key)), single(after.get(key))) else {
                self.unresolved(Facet::Property, key.to_string(), "listed more than once");
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

    fn relationships(&mut self, base: &Object, revised: &Object) {
        let names: BTreeSet<&String> = base
            .relationships
            .keys()
            .chain(revised.relationships.keys())
            .collect();
        for name in names {
            let targets = |named: &BTreeMap<&ObjectId, String>,
                           object: &Object|
             -> Option<BTreeSet<String>> {
                object
                    .relationships
                    .get(name)
                    .into_iter()
                    .flatten()
                    .map(|target| named.get(target).cloned())
                    .collect()
            };
            match (
                targets(&self.names.base, base),
                targets(&self.names.revised, revised),
            ) {
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

    /// Related objects per relationship kind, mapped through the matching.
    ///
    /// Matched related objects are compared by their pair's identity, so a
    /// difference among them is a change. A related object matched with
    /// nothing (added or removed itself) is listed as unmatched, and one
    /// whose match is undecided as undecided: neither is a change of the
    /// relationship, and both leave the kind not compared beyond the
    /// matched objects.
    fn relationship_kinds(&mut self, base: &Object, revised: &Object) {
        let Some(kinds) = self.kinds else {
            return;
        };
        for kind in RelationshipKind::ALL {
            let (before, after) = match (
                kinds.base.sorted(kind, &base.id),
                kinds.revised.sorted(kind, &revised.id),
            ) {
                (Ok(before), Ok(after)) => (before, after),
                (Err(reason), _) | (_, Err(reason)) => {
                    self.unresolved(Facet::Relationship, kind.name(), reason);
                    continue;
                }
            };
            let ((before, base_unmatched, base_undecided), (after, unmatched, undecided)) =
                (before, after);
            if before != after {
                self.differ(Difference::Relationship {
                    name: kind.name().to_owned(),
                    removed: before.difference(&after).cloned().collect(),
                    added: after.difference(&before).cloned().collect(),
                });
            }
            let mut gaps = Vec::new();
            for (what, base_list, revised_list) in [
                ("unmatched", &base_unmatched, &unmatched),
                ("undecided", &base_undecided, &undecided),
            ] {
                let mut sides = Vec::new();
                if !base_list.is_empty() {
                    sides.push(format!("base {}", base_list.join(", ")));
                }
                if !revised_list.is_empty() {
                    sides.push(format!("revised {}", revised_list.join(", ")));
                }
                if !sides.is_empty() {
                    gaps.push(format!("{what} related objects ({})", sides.join("; ")));
                }
            }
            if !gaps.is_empty() {
                self.unresolved(
                    Facet::Relationship,
                    kind.name(),
                    format!("{}; compared through matched objects only", gaps.join(", ")),
                );
            }
        }
    }

    /// Placement frames: origin distance and axis rotation, both exact.
    fn placement(&mut self, base: &Object, revised: &Object, tolerance: ComparisonTolerance) {
        let (Some(base_frames), Some(revised_frames)) = (
            self.base.context.services.get::<ObjectFrameServiceHandle>(),
            self.revised
                .context
                .services
                .get::<ObjectFrameServiceHandle>(),
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
                self.mirroring(base, revised);
            }
            (Err(error), _) | (_, Err(error)) => {
                self.unresolved(Facet::Placement, "", error.to_string());
            }
        }
    }

    /// Mirrored against rotated: the sign of the determinant of the
    /// transform placing each body, as the body facts state it. A frame is
    /// right-handed by contract, so a mirroring is the body's, never the
    /// frame's; a rotation cannot turn one into the other.
    fn mirroring(&mut self, base: &Object, revised: &Object) {
        let word = |mirrored: bool| if mirrored { "mirrored" } else { "not mirrored" };
        match (mirrored(self.base, base), mirrored(self.revised, revised)) {
            (Ok(before), Ok(after)) if before != after => {
                self.outcome
                    .stated(Facet::Placement, "mirroring", word(before), word(after));
            }
            (Ok(_), Ok(_)) => {}
            (Err(reason), _) | (_, Err(reason)) => {
                self.unresolved(Facet::Placement, "mirroring", reason);
            }
        }
    }

    /// The geometry facet in the request's mode.
    fn geometry(&mut self, base: &Object, revised: &Object, tolerance: ComparisonTolerance) {
        let (Some(base_bodies), Some(revised_bodies)) = (
            self.base.context.services.get::<ProximityServiceHandle>(),
            self.revised
                .context
                .services
                .get::<ProximityServiceHandle>(),
        ) else {
            self.unresolved(Facet::Geometry, "", "a session has no geometry service");
            return;
        };
        match self.request.geometry_mode {
            GeometryMode::Bounds => {
                self.bounds(base, revised, (base_bodies, revised_bodies), tolerance);
            }
            GeometryMode::Mesh => {
                self.mesh(base, revised, (base_bodies, revised_bodies), tolerance);
            }
        }
    }

    /// The certified two-sided Hausdorff distance between the two surfaces,
    /// in world coordinates: the revised body's surface, handed out by its
    /// session, measured against the base body by the base session's
    /// service. Only exact bodies are measured; a tessellation leaves the
    /// facet unresolved, never unchanged.
    fn mesh(
        &mut self,
        base: &Object,
        revised: &Object,
        (base_bodies, revised_bodies): (&ProximityServiceHandle, &ProximityServiceHandle),
        tolerance: ComparisonTolerance,
    ) {
        let facet = Facet::Geometry;
        match (
            base_bodies.bounds(&base.id),
            revised_bodies.body_surface(&revised.id),
        ) {
            (Err(ProximityError::NoBody), Err(ProximityError::NoBody)) => {}
            (Ok(_), Err(ProximityError::NoBody)) => {
                self.outcome.stated(facet, "body", "present", "none");
            }
            (Err(ProximityError::NoBody), Ok(_)) => {
                self.outcome.stated(facet, "body", "none", "present");
            }
            (Ok(before), Ok(after)) => {
                for (side, exact) in [
                    (Side::Base, before.fidelity().is_exact()),
                    (Side::Revised, after.fidelity().is_exact()),
                ] {
                    if !exact {
                        self.unresolved(
                            facet,
                            "mesh",
                            format!(
                                "the {} body is a tessellation, so its surface distance cannot be certified",
                                side.name()
                            ),
                        );
                        return;
                    }
                }
                let length = tolerance.length_metres;
                let measured = SurfaceDistanceRequest::try_new(
                    base.id.clone(),
                    Arc::new(after),
                    mesh_accuracy(length),
                )
                .and_then(|request| base_bodies.measure_surface_distance(&request));
                match measured {
                    Ok(measured) => {
                        let distance = measured.distance();
                        let (direction, directed) = measured.witness();
                        let witness = Witness {
                            side: match direction {
                                SurfaceDirection::FromSubject => Side::Base,
                                SurfaceDirection::FromCounterpart => Side::Revised,
                            },
                            from: directed.from(),
                            to: directed.to(),
                        };
                        self.outcome.witnessed(
                            Measure::Mesh,
                            (distance.lower_metres(), distance.upper_metres()),
                            length,
                            Some(witness),
                        );
                    }
                    Err(error) => self.unresolved(facet, "mesh", error.to_string()),
                }
            }
            (Err(error), _) | (_, Err(error)) => {
                self.unresolved(facet, "", error.to_string());
            }
        }
    }

    /// Measured bounds: the largest shift of any bound, widened by both
    /// tessellations' chord deviations.
    fn bounds(
        &mut self,
        base: &Object,
        revised: &Object,
        (base_bodies, revised_bodies): (&ProximityServiceHandle, &ProximityServiceHandle),
        tolerance: ComparisonTolerance,
    ) {
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
                let shift = bounds_shift(&before.bounds(), &after.bounds());
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

/// Whether the transform placing `object`'s body mirrors it
/// (`axioval:body.Mirrored`); an object without body facts has no body to
/// mirror.
fn mirrored(revision: &Revision<'_>, object: &Object) -> Result<bool, String> {
    let mut facts = BodyFacts::of(&revision.context, object).map_err(|(_, message)| message)?;
    match facts.value("Mirrored").map_err(|(_, message)| message)? {
        None => Ok(false),
        Some(PropertyValue::Boolean(mirrored)) => Ok(mirrored),
        Some(other) => Err(format!(
            "`axioval:body.Mirrored` is {}, not a boolean",
            display(Some(&other))
        )),
    }
}

/// How tight a surface distance is asked for: a tenth of the tolerance, so
/// an interval rarely straddles it, and no tighter than rounding allows.
fn mesh_accuracy(tolerance_metres: f64) -> f64 {
    (tolerance_metres / 10.0).max(1e-9)
}

/// The largest shift of any face of two axis-aligned bounds.
pub(super) fn bounds_shift(a: &axioval_engine::Bounds3, b: &axioval_engine::Bounds3) -> f64 {
    (0..3)
        .flat_map(|axis| {
            [
                (a.min()[axis] - b.min()[axis]).abs(),
                (a.max()[axis] - b.max()[axis]).abs(),
            ]
        })
        .fold(0.0_f64, f64::max)
}

pub(super) fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The rotation angle between two orthonormal axis triples.
///
/// For a rotation by θ, the Frobenius norm of the difference of the two
/// rotation matrices is `2·√2·sin(θ/2)`; unlike the trace formula this stays
/// accurate for the small angles a tolerance is about.
pub(super) fn rotation(a: [MetricDirection; 3], b: [MetricDirection; 3]) -> f64 {
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

/// Compares what two paired sources state about themselves: their coordinate
/// systems, when `request` asks.
pub(super) fn sources(
    base: (&Revision<'_>, &SourceId),
    revised: (&Revision<'_>, &SourceId),
    request: &ComparisonRequest,
) -> SourceComparison {
    let mut outcome = Outcome::default();
    if let Some(tolerance) = request.coordinate_systems {
        coordinate_systems(base, revised, tolerance, &mut outcome);
    }
    if request.timestamps {
        timestamps(base, revised, &mut outcome);
    }
    SourceComparison {
        base: base.1.clone(),
        revised: revised.1.clone(),
        differences: outcome.differences,
        unresolved: outcome.unresolved,
        undetermined: outcome.undetermined,
    }
}

/// A header timestamp as the nanoseconds since 1970 it may state: one
/// instant with a UTC offset, or, without one, every instant the local time
/// may be in any zone (offsets run to 14 hours either way).
fn instants(written: &str) -> Option<(i128, i128)> {
    let nanoseconds = |instant: DateTime| {
        let (seconds, nanoseconds) = instant.unix_instant();
        i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds)
    };
    if let Ok(instant) = written.parse::<DateTime>() {
        let at = nanoseconds(instant);
        return Some((at, at));
    }
    let local = nanoseconds(format!("{written}Z").parse::<DateTime>().ok()?);
    let zones = 14 * 3600 * 1_000_000_000_i128;
    Some((local - zones, local + zones))
}

/// The one timestamp a source states, as written, with its instants.
fn timestamp(
    revision: &Revision<'_>,
    source: &SourceId,
    side: &str,
) -> Result<(String, (i128, i128)), String> {
    let Some(Some(values)) = revision.timestamps.get(source) else {
        return Err(format!("the {side} source's timestamp was not read"));
    };
    let [written] = values.as_slice() else {
        return Err(if values.is_empty() {
            format!("the {side} source states no timestamp")
        } else {
            format!("the {side} source states {} timestamps", values.len())
        });
    };
    let instants = instants(written)
        .ok_or_else(|| format!("the {side} source's timestamp `{written}` is no date-time"))?;
    Ok((written.clone(), instants))
}

/// A revised source written surely before its base source is a
/// difference; one that may have been is unresolved.
fn timestamps(
    base: (&Revision<'_>, &SourceId),
    revised: (&Revision<'_>, &SourceId),
    outcome: &mut Outcome,
) {
    let facet = Facet::Timestamp;
    match (
        timestamp(base.0, base.1, "base"),
        timestamp(revised.0, revised.1, "revised"),
    ) {
        (Ok((before, (base_lower, base_upper))), Ok((after, (revised_lower, revised_upper)))) => {
            if revised_upper < base_lower {
                outcome.differences.push(Difference::OlderTimestamp {
                    base: before,
                    revised: after,
                });
            } else if revised_lower < base_upper {
                outcome.unresolved(
                    facet,
                    "",
                    format!(
                        "`{after}` and `{before}` may be in either order: a timestamp without a UTC offset may be in any zone"
                    ),
                );
            }
        }
        (Err(reason), _) | (_, Err(reason)) => outcome.unresolved(facet, "", reason),
    }
}

/// Compares the coordinate systems of two sources.
fn coordinate_systems(
    base: (&Revision<'_>, &SourceId),
    revised: (&Revision<'_>, &SourceId),
    tolerance: ComparisonTolerance,
    outcome: &mut Outcome,
) {
    let facet = Facet::CoordinateSystem;
    match (
        base.0
            .context
            .services
            .get::<CoordinateSystemServiceHandle>(),
        revised
            .0
            .context
            .services
            .get::<CoordinateSystemServiceHandle>(),
    ) {
        (Some(before), Some(after)) => {
            match (
                before.coordinate_system(base.1),
                after.coordinate_system(revised.1),
            ) {
                (Ok(before), Ok(after)) => {
                    compare_systems(&before, &after, tolerance, outcome);
                }
                (Err(error), _) | (_, Err(error)) => {
                    outcome.unresolved(facet, "", error.to_string());
                }
            }
        }
        _ => outcome.unresolved(facet, "", "a session has no coordinate-system service"),
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

fn single(values: Option<&Vec<PropertyValue>>) -> Result<Option<&PropertyValue>, ()> {
    match values.map(Vec::as_slice) {
        None | Some([]) => Ok(None),
        Some([value]) => Ok(Some(value)),
        Some(_) => Err(()),
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
pub(super) fn matched<'a>(
    (base, revised): (&Revision<'a>, &Revision<'a>),
    request: &ComparisonRequest,
    (names, kinds): (&TargetNames<'a>, Option<&KindRelations>),
    old: &Object,
    new: &Object,
) -> ObjectChange {
    let mut pair = Pair {
        base,
        revised,
        request,
        names,
        kinds,
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
    if request.all_property_sets || !request.property_sets.is_empty() {
        pair.property_sets(old, new);
    }
    pair.relationships(old, new);
    pair.relationship_kinds(old, new);
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
