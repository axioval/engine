//! Strict compiler contract tests.
#![allow(missing_docs)]

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, EngineError, EvidenceSession,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext, Runtime,
    ServiceRegistry, SessionSources, SnapshotBoundService, SourceSnapshot, compile,
};
use axioval_ir::{
    DefinitionPackage, Finding, Object, ObjectId, Project, QuantityDimension, ReportColumn,
    ReportTable, ReportValue, RuleId, RuleSetPackage, Scope, Severity, SourceId,
};

struct Stub;
impl RuleCapability for Stub {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        CapabilityEvaluation::evaluated(vec![])
    }
}
fn packages() -> (DefinitionPackage, RuleSetPackage) {
    (
        serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/definitions.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap(),
    )
}
#[test]
fn canonical_packages_compile() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert_eq!(
        compile(&registry, &[definitions], &rules)
            .unwrap()
            .rules()
            .len(),
        1
    );
}
#[test]
fn compiler_fails_closed_for_missing_required_parameter() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].parameters.clear();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn compiler_rejects_unsupported_definition_schema_version() {
    let (mut definitions, rules) = packages();
    definitions.schema_version = "999.0.0".into();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn compiler_rejects_unsupported_ruleset_schema_version() {
    let (definitions, mut rules) = packages();
    rules.schema_version = "999.0.0".into();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn runtime_rejects_capability_registry_drift() {
    let (definitions, rules) = packages();
    let compiler_registry = CapabilityRegistry::new().register(Stub).unwrap();
    let plan = compile(&compiler_registry, &[definitions], &rules).unwrap();

    let error = Runtime::new(CapabilityRegistry::new())
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap_err();
    assert!(matches!(error, EngineError::UnknownCapability(_)));
}

#[test]
fn compiler_rejects_duplicate_definition_package_ids() {
    let (definitions, rules) = packages();
    let duplicate = definitions.clone();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();

    let error = compile(&registry, &[definitions, duplicate], &rules).unwrap_err();
    assert!(matches!(error, EngineError::DuplicateDefinitionPackage(_)));
}

struct Unavailable;
impl RuleCapability for Unavailable {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, "z diagnostic");
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, "a diagnostic");
        evaluation
    }
}

struct SessionMarker {
    label: &'static str,
    snapshots: Vec<SourceSnapshot>,
}
impl SnapshotBoundService for SessionMarker {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
}

struct SessionServiceCapability;
impl RuleCapability for SessionServiceCapability {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, context: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        let mut evaluation = CapabilityEvaluation::default();
        let marker = context.services.get::<SessionMarker>().unwrap();
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, marker.label);
        evaluation
    }
}

#[test]
fn runtime_reports_capability_unavailability_without_false_pass() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(Unavailable).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap();
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 2);
    assert_eq!(report.not_evaluated()[0].message, "a diagnostic");
    assert_eq!(report.not_evaluated()[1].message, "z diagnostic");
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::MissingService
    );
    assert_eq!(
        report.not_evaluated()[0].rule_id.to_string(),
        "wall-reference-required"
    );
}

/// Pushes object, source and project outcomes in reverse of report order.
struct Scoped;
impl RuleCapability for Scoped {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let source = |document: &str| SourceId::new("test", document).unwrap();
        let object = ObjectId::new(source("a"), "1").unwrap();
        let finding = |scope: Scope| Finding::new(rule.id.clone(), scope, Severity::Error, "m");
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_finding(finding(Scope::Object(object.clone())));
        evaluation.push_finding(finding(Scope::Source(source("b"))));
        evaluation.push_finding(finding(Scope::Source(source("a"))));
        evaluation.push_finding(finding(Scope::Project));
        evaluation.push_object_not_evaluated(object, NotEvaluatedReason::BackendUnavailable, "m");
        evaluation.push_source_not_evaluated(
            source("a"),
            NotEvaluatedReason::IncompleteEvidence,
            "m",
        );
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, "m");
        evaluation
    }
}

#[test]
fn source_and_project_outcomes_order_before_object_outcomes() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(Scoped).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap();
    let scopes: Vec<String> = report
        .findings()
        .iter()
        .map(|finding| finding.scope.to_string())
        .collect();
    assert_eq!(
        scopes,
        ["project", "source test:a", "source test:b", "test:a/1"]
    );
    let scopes: Vec<String> = report
        .not_evaluated()
        .iter()
        .map(|outcome| outcome.scope.to_string())
        .collect();
    assert_eq!(scopes, ["project", "source test:a", "test:a/1"]);
}

/// Pushes tables out of name order, one without rows, and optionally one
/// name twice.
struct Tabled {
    duplicate: bool,
}
impl RuleCapability for Tabled {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        let object = |local: &str| ObjectId::new(SourceId::new("test", "a").unwrap(), local);
        // The capability names another rule; the runtime binds the compiled one.
        let table = |name: &str| {
            let mut table = ReportTable::new(
                RuleId::new("elsewhere").unwrap(),
                name,
                vec![ReportColumn::quantity("height", QuantityDimension::Length)],
            )
            .unwrap();
            for (local, height) in [("2", 3.5), ("1", 3.0)] {
                table
                    .push_row(object(local).unwrap(), vec![ReportValue::exact(height)])
                    .unwrap();
            }
            table
        };
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_table(table("spaces"));
        evaluation.push_table(
            ReportTable::new(
                RuleId::new("elsewhere").unwrap(),
                "empty",
                vec![ReportColumn::number("ratio")],
            )
            .unwrap(),
        );
        evaluation.push_table(table("levels"));
        if self.duplicate {
            evaluation.push_table(table("levels"));
        }
        assert_eq!(evaluation.tables().len(), 2 + usize::from(self.duplicate));
        evaluation
    }
}

#[test]
fn tables_are_bound_to_their_rule_and_ordered_by_rule_and_name() {
    let (definitions, mut rules) = packages();
    let mut earlier = rules.root.rules[0].clone();
    earlier.id = "a-first-rule".into();
    rules.root.rules.push(earlier);
    let registry = CapabilityRegistry::new()
        .register(Tabled { duplicate: false })
        .unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap();
    let tables: Vec<(String, &str)> = report
        .tables()
        .iter()
        .map(|table| (table.rule_id().to_string(), table.name()))
        .collect();
    assert_eq!(
        tables,
        [
            ("a-first-rule".to_owned(), "levels"),
            ("a-first-rule".to_owned(), "spaces"),
            ("wall-reference-required".to_owned(), "levels"),
            ("wall-reference-required".to_owned(), "spaces"),
        ]
    );
    let rows: Vec<String> = report.tables()[0]
        .rows()
        .iter()
        .map(|row| row.scope().to_string())
        .collect();
    assert_eq!(rows, ["test:a/1", "test:a/2"]);
}

#[test]
fn a_rule_reporting_one_table_twice_fails_the_run() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new()
        .register(Tabled { duplicate: true })
        .unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let error = Runtime::new(registry)
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap_err();
    assert_eq!(
        error,
        EngineError::DuplicateReportTable {
            rule: "wall-reference-required".into(),
            table: "levels".into(),
        }
    );
}

#[test]
fn session_services_are_authoritative_for_session_runs() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new()
        .register(SessionServiceCapability)
        .unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let source = SourceId::new("test", "runtime-session").unwrap();
    let snapshot =
        SourceSnapshot::try_new(source.clone(), "revision-1", "sha256:runtime-session").unwrap();
    let project = Project::new(vec![Object::new(
        ObjectId::new(source, "object-1").unwrap(),
        "wall",
    )])
    .unwrap();
    let mut runtime_services = ServiceRegistry::new();
    runtime_services
        .register(SessionMarker {
            label: "runtime",
            snapshots: vec![snapshot.clone()],
        })
        .unwrap();
    let session = EvidenceSession::try_new(project, [snapshot.clone()])
        .unwrap()
        .with_service(SessionMarker {
            label: "session",
            snapshots: vec![snapshot],
        })
        .unwrap();

    let report = Runtime::new(registry)
        .with_services(runtime_services)
        .run_session(&session, plan)
        .unwrap();

    assert_eq!(report.not_evaluated()[0].message, "session");
}

/// Reports the run's sources as one project outcome.
struct ListsSources;
impl RuleCapability for ListsSources {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, context: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        let sources = context.services.get::<SessionSources>().unwrap();
        let listed: Vec<String> = sources.iter().map(ToString::to_string).collect();
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, listed.join(" "));
        evaluation
    }
}

#[test]
fn the_runtime_lists_every_session_source_including_an_empty_one() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(ListsSources).unwrap();
    let session_plan = compile(&registry, &[definitions.clone()], &rules).unwrap();
    let bare_plan = compile(&registry, &[definitions], &rules).unwrap();
    let source = |document: &str| SourceId::new("test", document).unwrap();
    let snapshot =
        |document: &str| SourceSnapshot::try_new(source(document), "r1", "sha256:0").unwrap();
    let project = || {
        Project::new(vec![Object::new(
            ObjectId::new(source("full"), "1").unwrap(),
            "wall",
        )])
        .unwrap()
    };
    // A host copy naming only one source is replaced, never trusted.
    let mut host = ServiceRegistry::new();
    host.register(SessionSources::new([source("full")]))
        .unwrap();
    let runtime = Runtime::new(registry).with_services(host);

    let session =
        EvidenceSession::try_new(project(), [snapshot("full"), snapshot("empty")]).unwrap();
    let report = runtime.run_session(&session, session_plan).unwrap();
    assert_eq!(report.not_evaluated()[0].message, "test:empty test:full");

    // A bare project knows only the sources its objects name.
    let report = runtime.run(&project(), bare_plan).unwrap();
    assert_eq!(report.not_evaluated()[0].message, "test:full");
}
