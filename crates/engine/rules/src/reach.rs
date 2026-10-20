//! What each built-in capability may ask of a host's geometry
//! ([`axioval_engine::RuleCapability::geometry_reach`], #410, #448).
//!
//! A capability states the bodies it may measure (those of the objects its
//! subjects and named selector parameters may select) and the service
//! handles it may ask for them. A host meshes only those bodies, and a run
//! releases what a geometry service built once no rule still to run may
//! ask that service (#440). Stating less than a capability asks is a
//! correctness bug: the host would answer from bodies it left out. So a
//! capability that may reach bodies no selector bounds states
//! [`GeometryReach::Unbounded`], and why is recorded here:
//! [`UNBOUNDED_REACH`] for every rule of a capability,
//! [`CONDITIONAL_REACH`] for the rules stating what a capability cannot
//! bound. The tests hold every registered capability to a bounded reach or
//! one of these records.

use std::any::TypeId;

use axioval_engine::{CompiledRule, GeometryReach, LocalOrigins};
use axioval_ir::contract::ParameterValue;

/// The [`TypeId`]s of the named service handles of `axioval_engine`.
macro_rules! of {
    ($($handle:ident),* $(,)?) => {
        &[$(::std::any::TypeId::of::<::axioval_engine::$handle>()),*]
    };
}
pub(crate) use of;

/// The bodies of the objects `rule`'s subjects and its selector parameters
/// `names` may select, through the service handles `services`, and the
/// local frames a position is reported in.
pub(crate) fn selected(rule: &CompiledRule, names: &[&str], services: &[TypeId]) -> GeometryReach {
    let mut services = services.to_vec();
    services.push(TypeId::of::<LocalOrigins>());
    GeometryReach::Selected {
        selectors: axioval_engine::rule_selectors(rule, names),
        services,
    }
}

/// Whether `rule` states the parameter `name`.
pub(crate) fn states(rule: &CompiledRule, name: &str) -> bool {
    rule.parameters.contains_key(name)
}

/// Whether `rule` states `name` as a selector.
pub(crate) fn selects(rule: &CompiledRule, name: &str) -> bool {
    matches!(
        rule.parameters.get(name),
        Some(ParameterValue::Selector { .. })
    )
}

/// The string `rule` states as `name`, if it states one.
pub(crate) fn text<'r>(rule: &'r CompiledRule, name: &str) -> Option<&'r str> {
    match rule.parameters.get(name)? {
        ParameterValue::String { value } => Some(value),
        _ => None,
    }
}

/// Whether `rule` states `value` as any of its string parameters `names`.
pub(crate) fn chooses(rule: &CompiledRule, names: &[&str], value: &str) -> bool {
    names.iter().any(|name| text(rule, name) == Some(value))
}

/// Every built-in geometric capability whose reach no selector bounds, with
/// why: it states [`GeometryReach::Unbounded`] for every rule.
pub const UNBOUNDED_REACH: &[(&str, &str)] = &[
    (
        "axioval:capability.escape-route",
        "walks are routed through the whole model's walkable space, every other body an \
         obstacle, and most of its selectors sit in table rows",
    ),
    (
        "axioval:capability.external-wall-validation",
        "the envelope is derived from, and tested against, every body of the model",
    ),
    (
        "axioval:capability.horizontal-guard",
        "the guard search weighs every body near an edge, whatever the role selectors state",
    ),
    (
        "axioval:capability.level-spacing",
        "it reads the built-in measured `bottom` and `top`, which may ask any service, and \
         reaches contents by path",
    ),
    (
        "axioval:capability.light-well",
        "the well's spaces are those its `member_path` reaches, which no selector bounds",
    ),
    (
        "axioval:capability.space-distance",
        "its destinations are selected in the `distances` table's rows, and walking \
         distances are routed through the whole model",
    ),
    (
        "axioval:capability.space-validation",
        "duplicates, supports and unallocated floor are measured against every space and \
         slab of the model",
    ),
];

/// Every built-in capability bounded for some rules only, with the rules
/// it is not bounded for and why: it states [`GeometryReach::Unbounded`]
/// for those.
pub const CONDITIONAL_REACH: &[(&str, &str)] = &[
    (
        "axioval:capability.accessible-route",
        "a rule stating a `passing_…` parameter: passing spaces are routed over the whole \
         level, every other body an obstacle",
    ),
    (
        "axioval:capability.area-ratio",
        "a rule measuring a `facade`: a facade area depends on every body against it",
    ),
    (
        "axioval:capability.component-clearance",
        "a rule stating `space_path`: the spaces it reaches are measured, and no selector \
         bounds them",
    ),
    (
        "axioval:capability.effective-coverage",
        "a rule stating `access_path`: the spaces it joins are measured, every object one \
         unless `space_selector` says otherwise",
    ),
    (
        "axioval:capability.free-floor-circle",
        "a rule without an `obstacles` selector, whose obstacles are every physical object, \
         or stating `merge_path` or `access_path`, which reach spaces no selector bounds",
    ),
    (
        "axioval:capability.free-floor-rectangle",
        "a rule without an `obstacles` selector, whose obstacles are every physical object, \
         or stating `merge_path` or `access_path`, which reach spaces no selector bounds",
    ),
    (
        "axioval:capability.keyed-limit",
        "a rule limiting `sill-height` or `threshold-step`, whose floors are reached by \
         `floor_path`, or a `measured` value, which may be any",
    ),
    (
        "axioval:capability.local-circulation",
        "a rule without an `obstacles` selector, whose obstacles are every object, or \
         stating `merge_path`, which reaches spaces no selector bounds",
    ),
    (
        "axioval:capability.model-comparison",
        "a rule matching by `related` hosts, whose bodies are those `match_path` reaches",
    ),
    (
        "axioval:capability.opening-zone",
        "a rule stating `support_gap` without both a `host_selector` and a \
         `support_selector`: either defaults to every object",
    ),
    (
        "axioval:capability.plan-area",
        "a rule measuring a `facade`: a facade area depends on every body against it",
    ),
    (
        "axioval:capability.property-requirements",
        "a rule whose requirements are not stated inline, or are judged per measured face \
         area, which depends on every body against it",
    ),
    (
        "axioval:capability.slab-contact",
        "a rule without `counterparts`: a face may rest on every other object",
    ),
];

/// Why the built-in capability `id` may ask any body of a model through
/// any service: for every rule ([`UNBOUNDED_REACH`]) or for some
/// ([`CONDITIONAL_REACH`]). `None` for a capability whose every rule
/// states a bounded reach.
#[must_use]
pub fn unbounded_reason(id: &str) -> Option<&'static str> {
    UNBOUNDED_REACH
        .iter()
        .chain(CONDITIONAL_REACH)
        .find(|(capability, _)| *capability == id)
        .map(|(_, why)| *why)
}
