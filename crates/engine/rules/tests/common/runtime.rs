//! Whole runs of the in-memory source: packages compiled against the
//! registry and executed by the runtime over one session, so what a rule
//! instance declares beside its parameters is exercised end to end.

#![allow(clippy::needless_pass_by_value)]

use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CompleteRelationshipSelection, EngineError, EvidenceSession, ExecutionPlan,
    ParameterType, PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, Runtime, SourceSnapshot,
    compile,
};
use axioval_ir::{DefinitionPackage, Project, Report, RuleSetPackage};
use serde_json::{Map, Value, json};

use super::{Model, source};

/// The type system the in-memory source declares: concept `t.<name>` binds
/// to the source name `<name>`.
pub const TYPE_SYSTEM: &str = "axioval:test";

pub fn snapshot() -> SourceSnapshot {
    SourceSnapshot::try_new(source(), "r1", "sha256:test")
        .unwrap()
        .with_type_system(TYPE_SYSTEM)
        .unwrap()
}

/// The model bound to [`snapshot`].
struct Bound(Arc<Model>, Vec<SourceSnapshot>);

impl PropertyResolutionService for Bound {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        self.0.resolve(request)
    }
    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        self.0.enumerate(request)
    }
}

impl RelationshipSelectionService for Bound {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        self.0.select(request)
    }
}

/// A session over `model` with its property and relationship services.
pub fn session(model: Model) -> EvidenceSession {
    let project = Project::new(model.objects.clone()).unwrap();
    let model = Arc::new(model);
    let bound = || Arc::new(Bound(model.clone(), vec![snapshot()]));
    EvidenceSession::try_new(project, [snapshot()])
        .unwrap()
        .with_service(PropertyResolutionServiceHandle::new(bound()))
        .unwrap()
        .with_service(RelationshipSelectionServiceHandle::new(bound()))
        .unwrap()
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn concept(name: &str) -> (String, Value) {
    let id = format!("t.{name}");
    let value = json!({
        "id": id,
        "name": text(name),
        "externalNames": [{ "typeSystem": TYPE_SYSTEM, "name": name }],
    });
    (id, value)
}

/// The definition `t.def.<suffix>` of every capability in `capabilities`,
/// its signature (a table's columns too) taken from the registry, and the concepts `t.<name>` of
/// the named object types, properties and property sets.
pub fn definitions(
    registry: &CapabilityRegistry,
    capabilities: &[&str],
    object_types: &[&str],
    properties: &[&str],
    property_sets: &[&str],
) -> DefinitionPackage {
    let mut definitions = Map::new();
    for capability in capabilities {
        let parameters: Map<String, Value> = registry
            .get(capability)
            .unwrap()
            .parameters()
            .into_iter()
            .map(|descriptor| {
                let mut declared = json!({
                    "id": descriptor.name,
                    "name": text(&descriptor.name),
                    "kind": descriptor.parameter_type.package_kind(),
                    "required": descriptor.required,
                });
                // A table declares its columns as the capability does.
                if let ParameterType::Table(columns) = &descriptor.parameter_type {
                    declared["columns"] = columns
                        .iter()
                        .map(|column| {
                            json!({
                                "id": column.id,
                                "name": text(column.id),
                                "kind": column.kind.as_str(),
                                "required": column.required,
                            })
                        })
                        .collect();
                }
                (descriptor.name.clone(), declared)
            })
            .collect();
        let id = definition(capability);
        definitions.insert(
            id.clone(),
            json!({
                "id": id,
                "name": text(capability),
                "capability": capability,
                "parameters": parameters,
            }),
        );
    }
    let concepts = |names: &[&str]| {
        names
            .iter()
            .map(|name| concept(name))
            .collect::<Map<_, _>>()
    };
    // Every property is text: the in-memory source states what it states.
    let mut properties = concepts(properties);
    for property in properties.values_mut() {
        property["valueKind"] = json!("string");
    }
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "t.definitions",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "objectTypes": concepts(object_types),
        "properties": properties,
        "propertySets": concepts(property_sets),
        "definitions": definitions,
    }))
    .unwrap()
}

/// The definition id [`definitions`] gives `capability`.
pub fn definition(capability: &str) -> String {
    format!(
        "t.def.{}",
        capability.trim_start_matches("axioval:capability.")
    )
}

/// An entity-type selector of the concept `t.<name>`.
pub fn entity(name: &str) -> Value {
    json!({ "kind": "entityType", "objectType": format!("t.{name}"), "includeSubtypes": false })
}

/// One rule of `capability` over `applicability`, with `extra` fields (such
/// as `severityBands`) merged in.
pub fn rule(
    id: &str,
    capability: &str,
    severity: &str,
    applicability: Value,
    parameters: Value,
    extra: Value,
) -> Value {
    let mut rule = json!({
        "id": id,
        "definitionId": definition(capability),
        "name": text(id),
        "severity": severity,
        "applicability": applicability,
        "parameters": parameters,
    });
    if let (Value::Object(rule), Value::Object(extra)) = (&mut rule, extra) {
        rule.extend(extra);
    }
    rule
}

pub fn ruleset(rules: Vec<Value>) -> RuleSetPackage {
    serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": {
            "id": "t.ruleset",
            "name": text("test"),
            "version": "0.1.0",
            "authors": [],
        },
        "definitionPackages": ["t.definitions"],
        "root": { "id": "root", "name": text("root"), "folders": [], "rules": rules },
    }))
    .unwrap()
}

/// Compiles `rules` against `definitions` and the registry.
pub fn plan(
    registry: &CapabilityRegistry,
    definitions: &DefinitionPackage,
    rules: Vec<Value>,
) -> Result<ExecutionPlan, EngineError> {
    compile(registry, std::slice::from_ref(definitions), &ruleset(rules))
}

/// Runs `plan` over `session` with a runtime `configure` sets up.
pub fn run(
    registry: CapabilityRegistry,
    plan: ExecutionPlan,
    session: &EvidenceSession,
    configure: impl FnOnce(Runtime) -> Runtime,
) -> Result<Report, EngineError> {
    configure(Runtime::new(registry)).run_session(session, plan)
}
