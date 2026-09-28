//! The facets of one matched pair and of one pair of sources.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    ClassificationServiceHandle, CoordinateFrame, CoordinateSystemServiceHandle, MetricDirection,
    ObjectFrameError, ObjectFrameServiceHandle, PropertyResolutionServiceHandle, ProximityError,
    ProximityServiceHandle, SourceCoordinateSystem,
};
use axioval_ir::{Object, ObjectId, Property, PropertyValue, SourceId};

use super::{
    ComparedProperty, ComparisonRequest, ComparisonTolerance, Difference, Facet, Measure,
    Measurement, ObjectChange, Revision, SourceComparison, Unresolved,
};
use crate::selection::{NameSpec, enumerate};
use crate::support::{PropertyRef, resolve};

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

struct Pair<'r, 'a> {
    base: &'r Revision<'a>,
    revised: &'r Revision<'a>,
    request: &'r ComparisonRequest,
    names: &'r TargetNames<'a>,
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
        let keys: BTreeSet<&ComparedProperty> = before.keys().chain(after.keys()).collect();
        for key in keys {
            // A property also named is compared through the resolver alone.
            if self.request.properties.contains(key) {
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
            self.base.context.services.get::<ProximityServiceHandle>(),
            self.revised
                .context
                .services
                .get::<ProximityServiceHandle>(),
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
    SourceComparison {
        base: base.1.clone(),
        revised: revised.1.clone(),
        differences: outcome.differences,
        unresolved: outcome.unresolved,
        undetermined: outcome.undetermined,
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
    names: &TargetNames<'a>,
    old: &Object,
    new: &Object,
) -> ObjectChange {
    let mut pair = Pair {
        base,
        revised,
        request,
        names,
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
