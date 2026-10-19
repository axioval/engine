//! What a plan's rules may ask of a host's geometry (#410): nothing for
//! rules reading properties only, the spaces, obstacles and doors a
//! free-floor rule selects, and everything for a rule that states nothing
//! less or reads a measured value.
#![allow(missing_docs)]

mod common;

use std::any::TypeId;
use std::collections::BTreeSet;

use axioval_engine::{CapabilityRegistry, FreeSpaceServiceHandle, GeometryDemand, geometry_demand};
use axioval_ir::{DefinitionPackage, MEASURED_SET};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, session};
use common::{Model, id};
use serde_json::{Value, json};

const CIRCLE: &str = "axioval:capability.free-floor-circle";
const COUNT: &str = "axioval:capability.object-count";
const CLASH: &str = "axioval:capability.clash";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    definitions(
        registry,
        &[CIRCLE, COUNT, CLASH],
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
            services: BTreeSet::from([TypeId::of::<FreeSpaceServiceHandle>()]),
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

#[test]
fn a_measured_value_or_a_capability_stating_nothing_asks_for_everything() {
    let measured = json!({"kind": "property", "propertySet": MEASURED_SET,
                          "property": "area", "operator": "exists"});
    assert_eq!(demand(vec![count(measured)]), GeometryDemand::Everything);
    let clash = rule(
        "clash",
        CLASH,
        "error",
        entity("Furniture"),
        json!({"counterparts": {"type": "selector", "value": entity("Pipe")},
               "penetration_tolerance_metres": {"type": "number", "value": 0.01}}),
        json!({}),
    );
    assert_eq!(demand(vec![clash]), GeometryDemand::Everything);
}
