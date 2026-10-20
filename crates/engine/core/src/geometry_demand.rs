//! What a plan's rules may ask of a host's geometry (#410).
//!
//! Meshing every object of a model is most of what attaching geometry
//! costs, and a check whose rules read properties only needs none of it.
//! Each capability states what its rules may ask
//! ([`crate::RuleCapability::geometry_reach`]): nothing, only the bodies of
//! the objects some of their selectors may select (and only through the
//! services named), or anything. [`geometry_demand`] joins the plan's rules
//! into one [`GeometryDemand`] a host meshes for.
//!
//! The demand is conservative: a capability that states nothing may ask
//! anything, and so may any rule, value, classification, grouping, relation
//! or refinement that names a measured value ([`axioval_ir::MEASURED_SET`])
//! or list, a relationship derived from geometry
//! ([`crate::DERIVED_RELATIONSHIP_PREFIX`]) or compartments, whose joins
//! are measured. A selector is bounded by the
//! object types it requires; one that cannot be bounded that way (a
//! property, a relationship, a negation) may select every object. Where a
//! type cannot be told (an unbound concept, a kind the type hierarchy does
//! not know) the object is kept; a class the source's schema does not
//! declare, asked of an object whose kind it does, selects none (#449). Being wrong here would turn missing
//! evidence into a verdict, so a host meshing less than everything also
//! watches the services it left out ([`crate::ServiceRegistry::watch`])
//! and runs again with every object meshed when a rule asked for one.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::contract::{self as schema, Selector};
use axioval_ir::{MEASURED_SET, Object, ObjectId, SourceId};

use crate::concepts::{ConceptBindings, TypeHierarchyError};
use crate::derived_relationships::DERIVED_RELATIONSHIP_PREFIX;
use crate::{
    CapabilityRegistry, CompiledRule, EvidenceSession, ExecutionPlan, TypeHierarchyServiceHandle,
};

/// What one rule may ask of a host's geometry, as its capability states it
/// ([`crate::RuleCapability::geometry_reach`]).
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryReach {
    /// Nothing: the rule asks no geometry service and reads no measured
    /// value.
    None,
    /// Only the bodies of the objects `selectors` may select, through
    /// `services` (the service handles' types) and no other geometry
    /// service.
    Selected {
        /// The selectors whose objects' bodies the rule may measure: its
        /// subjects' and those of its selector parameters.
        selectors: Vec<Selector>,
        /// The types of the host service handles the rule may ask.
        services: Vec<TypeId>,
    },
    /// Anything: every body, through any service.
    Unbounded,
}

/// What a plan's rules may ask of a host's geometry, joined over its rules
/// ([`geometry_demand`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryDemand {
    /// No rule asks anything of geometry: a host need mesh nothing.
    Nothing,
    /// Only the bodies of `objects` (and whatever a host needs to measure
    /// them as it would among all the others), through `services`.
    Objects {
        /// Every object some rule may ask about, in identity order.
        objects: BTreeSet<ObjectId>,
        /// The types of the host service handles some rule may ask.
        services: BTreeSet<TypeId>,
    },
    /// Some rule may ask anything.
    Everything,
}

/// How an aggregate over a measured list writes its source, which names
/// the list without the measured set (`{"kind": "measured", "name": …}`).
const MEASURED_SOURCE: &str = r#""kind":"measured""#;

/// How a grouping by compartments writes its key, whose joins are measured
/// through the derived adjacency and the proximity service
/// (`GroupingKey::Compartment`).
const COMPARTMENT_KEY: &str = r#""kind":"compartment""#;

/// Whether `text` names a measured value, a measured list, a derived
/// relationship or compartments, any of which a host's geometry may
/// answer.
fn reads_geometry(text: &str) -> bool {
    text.contains(MEASURED_SET)
        || text.contains(MEASURED_SOURCE)
        || text.contains(DERIVED_RELATIONSHIP_PREFIX)
        || text.contains(COMPARTMENT_KEY)
}

/// Whether `value`, written as JSON, may read geometry; anything that cannot
/// be written may.
fn writes_geometry(value: &impl serde::Serialize) -> bool {
    serde_json::to_string(value).map_or(true, |text| reads_geometry(&text))
}

/// Whether anything of `plan` that every rule may come to read (its
/// values, classifications, groupings and relations) may read geometry.
fn plan_wide_reads_geometry(plan: &ExecutionPlan) -> bool {
    writes_geometry(plan.values.as_ref())
        || writes_geometry(&plan.classifications)
        || writes_geometry(&plan.groupings)
        || writes_geometry(&plan.relations)
}

/// Whether `rule`'s own selector, parameters or refinement may read
/// geometry.
fn rule_reads_geometry(plan: &ExecutionPlan, rule: &CompiledRule) -> bool {
    writes_geometry(&(&rule.selector, &rule.parameters))
        || plan.refinements.get(&rule.id).is_some_and(|refinement| {
            writes_geometry(&refinement.severity_overrides)
                || writes_geometry(&refinement.categories)
        })
}

/// Whether anything of `plan` beside its rules' capabilities (its values,
/// classifications, groupings, relations, refinements, and every rule's
/// selector and parameters) may read geometry.
fn plan_reads_geometry(plan: &ExecutionPlan) -> bool {
    plan_wide_reads_geometry(plan)
        || plan
            .rules
            .iter()
            .any(|rule| rule_reads_geometry(plan, rule))
}

/// The host service handles (their types) each rule of `plan` may ask, in
/// the plan's order, as `registry`'s capabilities state it
/// ([`crate::RuleCapability::geometry_reach`]): none for
/// [`GeometryReach::None`], the stated ones for
/// [`GeometryReach::Selected`], and `None` (any service) for an unknown
/// capability, [`GeometryReach::Unbounded`], or a rule whose selector,
/// parameters or refinement, or the plan's values, classifications,
/// groupings or relations, name a measured value or a derived
/// relationship. The run releases a derivation after the last rule that
/// may ask a service it is read through (#440); being conservative here
/// only keeps a value longer.
#[must_use]
pub fn services_asked(
    plan: &ExecutionPlan,
    registry: &CapabilityRegistry,
) -> Vec<Option<BTreeSet<TypeId>>> {
    let everywhere = plan_wide_reads_geometry(plan);
    plan.rules
        .iter()
        .map(|rule| {
            if everywhere || rule_reads_geometry(plan, rule) {
                return None;
            }
            match registry.get(&rule.capability)?.geometry_reach(rule) {
                GeometryReach::None => Some(BTreeSet::new()),
                GeometryReach::Selected { services, .. } => Some(services.into_iter().collect()),
                GeometryReach::Unbounded => None,
            }
        })
        .collect()
}

/// What `plan`'s rules may ask of the geometry a host attaches to
/// `session`, each as `registry`'s capability states it.
///
/// [`GeometryDemand::Everything`] when any rule's capability is unknown or
/// states [`GeometryReach::Unbounded`], or anything of the plan names a
/// measured value or a derived relationship; [`GeometryDemand::Nothing`]
/// when every rule states [`GeometryReach::None`]; otherwise the objects
/// the selected rules' selectors may select, bounded by type.
#[must_use]
pub fn geometry_demand(
    plan: &ExecutionPlan,
    registry: &CapabilityRegistry,
    session: &EvidenceSession,
) -> GeometryDemand {
    if plan_reads_geometry(plan) {
        return GeometryDemand::Everything;
    }
    let mut selectors: Vec<Selector> = Vec::new();
    let mut services: BTreeSet<TypeId> = BTreeSet::new();
    for rule in &plan.rules {
        let Some(capability) = registry.get(&rule.capability) else {
            return GeometryDemand::Everything;
        };
        match capability.geometry_reach(rule) {
            GeometryReach::None => {}
            GeometryReach::Selected {
                selectors: reached,
                services: asked,
            } => {
                selectors.extend(reached);
                services.extend(asked);
            }
            GeometryReach::Unbounded => return GeometryDemand::Everything,
        }
    }
    if selectors.is_empty() {
        return GeometryDemand::Nothing;
    }
    let type_systems: BTreeMap<SourceId, Vec<Arc<str>>> = session
        .snapshots()
        .map(|snapshot| (snapshot.source().clone(), snapshot.type_systems().to_vec()))
        .collect();
    let bound = Bound {
        bindings: ConceptBindings::new(plan.concepts.clone(), type_systems),
        hierarchy: session.service::<TypeHierarchyServiceHandle>(),
    };
    let mut objects = BTreeSet::new();
    for object in session.project().objects() {
        if selectors
            .iter()
            .any(|selector| bound.may_select(selector, object))
        {
            objects.insert(object.id.clone());
        }
    }
    GeometryDemand::Objects { objects, services }
}

/// What bounds a selector by type: the plan's concepts bound to each
/// source's type systems, and the session's type hierarchy.
struct Bound<'s> {
    bindings: ConceptBindings,
    hierarchy: Option<&'s TypeHierarchyServiceHandle>,
}

impl Bound<'_> {
    /// Whether `selector` may select `object`, or leave it undecided: false
    /// only where its types surely exclude it.
    fn may_select(&self, selector: &Selector, object: &Object) -> bool {
        match selector {
            Selector::EntityType {
                object_type,
                include_subtypes,
            } => self.may_be(object, object_type, *include_subtypes),
            Selector::AllOf { operands } => operands
                .iter()
                .all(|operand| self.may_select(operand, object)),
            Selector::AnyOf { operands } => operands
                .iter()
                .any(|operand| self.may_select(operand, object)),
            Selector::Objects { objects } => objects.contains(&object.id),
            // Bounded by nothing a host can read before the run.
            _ => true,
        }
    }

    /// Whether `object` may be of `object_type` (a package concept bound
    /// to its source's type system), or of a subtype when `subtypes`.
    fn may_be(&self, object: &Object, object_type: &str, subtypes: bool) -> bool {
        let Ok(name) = self.bindings.object_type(object_type, &object.id.source) else {
            // Unbound: the rule leaves it open, so it may still be asked.
            return true;
        };
        if object.kind().eq_ignore_ascii_case(name) {
            return true;
        }
        if !subtypes {
            return false;
        }
        // A class the source's schema does not declare has no member among
        // the kinds it does (#449); any other refusal (a kind the hierarchy
        // cannot place, an uncovered source) leaves the object open.
        self.hierarchy.is_none_or(|hierarchy| {
            match hierarchy.is_a(&object.id.source, object.kind(), name) {
                Ok(is) => is,
                Err(TypeHierarchyError::UnknownAncestor(_)) => false,
                Err(_) => true,
            }
        })
    }
}

/// The selectors of `rule`'s parameters among `names`, each stated as a
/// selector, after its subjects' selector.
#[must_use]
pub fn rule_selectors(rule: &crate::CompiledRule, names: &[&str]) -> Vec<Selector> {
    let mut selectors = vec![rule.selector.clone()];
    selectors.extend(
        names
            .iter()
            .filter_map(|name| match rule.parameters.get(*name) {
                Some(schema::ParameterValue::Selector { value }) => Some(value.as_ref().clone()),
                _ => None,
            }),
    );
    selectors
}
