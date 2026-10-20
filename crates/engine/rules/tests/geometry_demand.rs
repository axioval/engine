//! What a plan's rules may ask of a host's geometry (#410, #448): nothing
//! for rules reading properties only, the spaces, obstacles and doors a
//! free-floor rule selects, the bodies a clash rule pairs, and everything
//! for a rule that reads a measured value or whose capability records why
//! it may; none of a class the source's schema does not declare (#449).
#![allow(missing_docs)]

mod common;

use std::any::TypeId;
use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CompiledRule, FreeSpaceServiceHandle, GeometryDemand, GeometryReach,
    LocalOrigins, ObjectFrameServiceHandle, ParameterType, ProximityServiceHandle, SourceSnapshot,
    TypeHierarchyError, TypeHierarchyService, TypeHierarchyServiceHandle, geometry_demand,
    services_asked,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{DefinitionPackage, MEASURED_SET, RuleId};
use axioval_rules::reach::{CONDITIONAL_REACH, UNBOUNDED_REACH};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, ruleset, session, snapshot};
use common::{Model, id};
use serde_json::{Value, json};

const CIRCLE: &str = "axioval:capability.free-floor-circle";
const COUNT: &str = "axioval:capability.object-count";
const CLASH: &str = "axioval:capability.clash";
const EXPRESSION: &str = "axioval:capability.expression";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    definitions(
        registry,
        &[CIRCLE, COUNT, CLASH, EXPRESSION],
        &["Space", "Furniture", "Door", "Pipe"],
        &["Name"],
        &["Pset"],
    )
}

/// A space, a wardrobe, a door and a pipe.
fn model() -> Model {
    Model::default()
        .object("s1", "Space")
        .object("f1", "Furniture")
        .object("d1", "Door")
        .object("p1", "Pipe")
}

/// The demand of a plan of `rules` over [`model`].
fn demand(rules: Vec<Value>) -> GeometryDemand {
    let registry = registry();
    let plan = plan(&registry, &vocabulary(&registry), rules).unwrap();
    geometry_demand(&plan, &registry, &session(model()))
}

fn count(applicability: Value) -> Value {
    rule(
        "count",
        COUNT,
        "error",
        applicability,
        json!({"minimum": {"type": "integer", "value": 1}}),
        json!({}),
    )
}

fn circle(parameters: &Value) -> Value {
    let mut stated = json!({
        "diameter_metres": {"type": "number", "value": 1.5},
        "height_metres": {"type": "number", "value": 2.0},
    });
    for (name, value) in parameters.as_object().into_iter().flatten() {
        stated[name] = value.clone();
    }
    rule(
        "circle",
        CIRCLE,
        "error",
        entity("Space"),
        stated,
        json!({}),
    )
}

#[test]
fn rules_reading_properties_only_ask_nothing() {
    assert_eq!(
        demand(vec![count(entity("Furniture"))]),
        GeometryDemand::Nothing
    );
}

#[test]
fn a_free_floor_rule_asks_for_its_spaces_obstacles_and_doors_through_the_free_space_service() {
    let demand = demand(vec![
        count(entity("Pipe")),
        circle(&json!({
            "obstacles": {"type": "selector", "value": entity("Furniture")},
            "subtract_door_swings": {"type": "selector", "value": entity("Door")},
        })),
    ]);
    assert_eq!(
        demand,
        GeometryDemand::Objects {
            objects: BTreeSet::from([id("s1"), id("f1"), id("d1")]),
            services: BTreeSet::from([
                TypeId::of::<FreeSpaceServiceHandle>(),
                TypeId::of::<ObjectFrameServiceHandle>(),
                TypeId::of::<LocalOrigins>(),
            ]),
        }
    );
}

#[test]
fn a_free_floor_rule_without_selected_obstacles_or_with_a_path_may_ask_anything() {
    // Every physical object is an obstacle.
    assert_eq!(demand(vec![circle(&json!({}))]), GeometryDemand::Everything);
    // Merged spaces are reached by a path, not a selector.
    assert_eq!(
        demand(vec![circle(&json!({
            "obstacles": {"type": "selector", "value": entity("Furniture")},
            "merge_path": {"type": "stringList", "value": ["IfcRelAggregates:forward"]},
        }))]),
        GeometryDemand::Everything
    );
}

fn clash() -> Value {
    rule(
        "clash",
        CLASH,
        "error",
        entity("Furniture"),
        json!({"counterparts": {"type": "selector", "value": entity("Pipe")},
               "penetration_tolerance_metres": {"type": "number", "value": 0.01}}),
        json!({}),
    )
}

#[test]
fn a_clash_rule_asks_for_the_bodies_it_pairs_through_the_proximity_service() {
    assert_eq!(
        demand(vec![clash()]),
        GeometryDemand::Objects {
            objects: BTreeSet::from([id("f1"), id("p1")]),
            services: BTreeSet::from([
                TypeId::of::<ProximityServiceHandle>(),
                TypeId::of::<ObjectFrameServiceHandle>(),
                TypeId::of::<LocalOrigins>(),
            ]),
        }
    );
}

#[test]
fn a_measured_value_or_list_asks_for_everything() {
    let measured = json!({"kind": "property", "propertySet": MEASURED_SET,
                          "property": "area", "operator": "exists"});
    assert_eq!(demand(vec![count(measured)]), GeometryDemand::Everything);
    // An aggregate over a measured list names the list alone (#448).
    let over_list = rule(
        "expression",
        EXPRESSION,
        "error",
        entity("Space"),
        json!({"requirement": {"type": "expression", "value": {
            "kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "aggregate", "function": "count",
                     "over": {"kind": "measured", "name": "steps"}},
            "right": {"kind": "literal", "value": {"type": "number", "value": 1}}}}}),
        json!({}),
    );
    assert_eq!(demand(vec![over_list]), GeometryDemand::Everything);
}

/// A check of free-floor rules, then clash rules: no clash rule asks the
/// free-space service, so the scenes read through it are released after the
/// last free-floor rule (#448).
#[test]
fn no_rule_after_the_last_free_floor_rule_asks_the_free_space_service() {
    let registry = registry();
    let rules = vec![
        circle(&json!({"obstacles": {"type": "selector", "value": entity("Furniture")}})),
        rule(
            "circle-2",
            CIRCLE,
            "error",
            entity("Space"),
            json!({"diameter_metres": {"type": "number", "value": 1.2},
                   "height_metres": {"type": "number", "value": 2.0},
                   "obstacles": {"type": "selector", "value": entity("Furniture")}}),
            json!({}),
        ),
        clash(),
    ];
    let plan = plan(&registry, &vocabulary(&registry), rules).unwrap();
    let asked = services_asked(&plan, &registry);
    let free_space = TypeId::of::<FreeSpaceServiceHandle>();
    let proximity = TypeId::of::<ProximityServiceHandle>();
    let last_free_floor = asked
        .iter()
        .rposition(|services| {
            services
                .as_ref()
                .is_none_or(|asked| asked.contains(&free_space))
        })
        .unwrap();
    let first_clash = asked
        .iter()
        .position(|services| {
            services
                .as_ref()
                .is_none_or(|asked| asked.contains(&proximity))
        })
        .unwrap();
    assert!(last_free_floor < first_clash, "{asked:?}");
    assert!(
        asked[first_clash..].iter().all(|services| services
            .as_ref()
            .is_some_and(|asked| !asked.contains(&free_space))),
        "{asked:?}"
    );
}

/// A rule of `capability` stating every selector parameter it takes and
/// nothing else: what its reach is bounded by at most.
fn probe(registry: &CapabilityRegistry, capability: &str) -> CompiledRule {
    let space = Selector::EntityType {
        object_type: "t.Space".into(),
        include_subtypes: false,
    };
    CompiledRule {
        id: RuleId::new("probe").unwrap(),
        capability: capability.to_owned(),
        severity: Severity::Error,
        selector: space.clone(),
        parameters: registry
            .get(capability)
            .unwrap()
            .parameters()
            .into_iter()
            .filter(|parameter| matches!(parameter.parameter_type, ParameterType::Selector))
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    ParameterValue::Selector {
                        value: Box::new(space.clone()),
                    },
                )
            })
            .collect(),
    }
}

/// Every built-in capability states a bounded reach for a rule selecting
/// all it may, or the reason it may ask anything is recorded (#448): for
/// every rule (`UNBOUNDED_REACH`), or for the rules it names
/// (`CONDITIONAL_REACH`). A recorded capability is registered, and one
/// recorded unbounded for every rule is.
#[test]
fn every_capability_bounds_its_reach_or_records_why_not() {
    let registry = registry();
    let ids: BTreeSet<&str> = registry.ids().collect();
    for (capability, why) in UNBOUNDED_REACH.iter().chain(CONDITIONAL_REACH) {
        assert!(ids.contains(capability), "{capability} is not registered");
        assert!(!why.is_empty(), "{capability} records no reason");
    }
    for capability in &ids {
        let reach = registry
            .get(capability)
            .unwrap()
            .geometry_reach(&probe(&registry, capability));
        let always = UNBOUNDED_REACH.iter().any(|(id, _)| id == capability);
        let sometimes = CONDITIONAL_REACH.iter().any(|(id, _)| id == capability);
        assert!(!(always && sometimes), "{capability} is recorded twice");
        if always {
            assert_eq!(reach, GeometryReach::Unbounded, "{capability}");
        } else if !sometimes {
            assert_ne!(
                reach,
                GeometryReach::Unbounded,
                "{capability} may ask anything, and no reason is recorded"
            );
        }
    }
}

/// Compartments join their members across measured faces, so a plan
/// grouping by them may ask anything, whatever its rules state (#448).
#[test]
fn a_grouping_by_compartments_asks_for_everything() {
    let registry = registry();
    let mut package = serde_json::to_value(ruleset(vec![clash()])).unwrap();
    package["groupings"] = json!({"compartments": {
        "id": "compartments",
        "name": {"default": "compartments", "translations": {}},
        "members": entity("Space"),
        "by": {"kind": "compartment", "separators": entity("Pipe"), "boundary": entity("Pipe")},
    }});
    let plan = axioval_engine::compile(
        &registry,
        std::slice::from_ref(&vocabulary(&registry)),
        &serde_json::from_value(package).unwrap(),
    )
    .unwrap();
    assert_eq!(
        geometry_demand(&plan, &registry, &session(model())),
        GeometryDemand::Everything
    );
    assert_eq!(services_asked(&plan, &registry), vec![None]);
}

/// A type hierarchy over a schema declaring `declared` kinds, each its own
/// only ancestor, refusing as an adapter does (#449): an undeclared kind as
/// unknown, an undeclared ancestor of a declared kind as such.
struct Schema(Vec<SourceSnapshot>, &'static [&'static str]);

impl TypeHierarchyService for Schema {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, TypeHierarchyError> {
        if !self.1.contains(&kind) {
            return Err(TypeHierarchyError::UnknownType(kind.to_owned()));
        }
        if !self.1.contains(&ancestor) {
            return Err(TypeHierarchyError::UnknownAncestor(ancestor.to_owned()));
        }
        Ok(kind == ancestor)
    }
}

/// The demand of a clash of furniture against `counterpart`, both with
/// their subtypes, over `model` and a schema declaring `declared`.
fn clash_over(
    model: Model,
    declared: &'static [&'static str],
    counterpart: &str,
) -> GeometryDemand {
    let registry = registry();
    let subtypes = |name: &str| json!({"kind": "entityType", "objectType": format!("t.{name}"), "includeSubtypes": true});
    let rule = rule(
        "clash",
        CLASH,
        "error",
        subtypes("Furniture"),
        json!({"counterparts": {"type": "selector", "value": subtypes(counterpart)},
               "penetration_tolerance_metres": {"type": "number", "value": 0.01}}),
        json!({}),
    );
    let plan = plan(&registry, &vocabulary(&registry), vec![rule]).unwrap();
    let hierarchy = TypeHierarchyServiceHandle::new(Arc::new(Schema(vec![snapshot()], declared)));
    let session = session(model).with_service(hierarchy).unwrap();
    geometry_demand(&plan, &registry, &session)
}

/// A clash against a class the source's schema does not declare, as an
/// IFC4 class asked of an IFC2X3 model: no object of the source can be one,
/// so only the furniture is meshed (#449).
#[test]
fn a_class_the_schema_does_not_declare_selects_no_object() {
    let model = Model::default()
        .object("s1", "Space")
        .object("f1", "Furniture")
        .object("d1", "Door");
    assert_eq!(
        clash_over(model, &["Space", "Furniture", "Door"], "Pipe"),
        GeometryDemand::Objects {
            objects: BTreeSet::from([id("f1")]),
            services: BTreeSet::from([
                TypeId::of::<ProximityServiceHandle>(),
                TypeId::of::<ObjectFrameServiceHandle>(),
                TypeId::of::<LocalOrigins>(),
            ]),
        }
    );
}

/// An object whose own kind the hierarchy cannot place may be anything, so
/// it stays in the demand, whether the class asked for is declared or not
/// (#449).
#[test]
fn an_object_of_an_unknown_kind_may_still_be_selected() {
    let model = || {
        Model::default()
            .object("s1", "Space")
            .object("f1", "Furniture")
            .object("d1", "Door")
            .object("x1", "Mystery")
    };
    let declared: &'static [&'static str] = &["Space", "Furniture", "Door"];
    for counterpart in ["Door", "Pipe"] {
        let GeometryDemand::Objects { objects, .. } = clash_over(model(), declared, counterpart)
        else {
            panic!("a clash is bounded by its selectors");
        };
        let mut expected = BTreeSet::from([id("f1"), id("x1")]);
        if counterpart == "Door" {
            expected.insert(id("d1"));
        }
        assert_eq!(objects, expected, "{counterpart}");
    }
}
