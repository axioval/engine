//! Concept binding: package concepts reach source data only through a name
//! declared for the source's own type system.
//!
//! These tests pin the fail-open this module closes. Before binding existed,
//! a ruleset selecting `axioval:example.ifc.wall` over a model whose objects
//! were kinded `IFCWALL` selected nothing and returned an empty report, which
//! a caller reads as a pass.
#![allow(missing_docs)]

use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CompletePropertyAbsenceEvidence, EvidenceSession, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty, Runtime, SourceSnapshot, TypeHierarchyError,
    TypeHierarchyService, TypeHierarchyServiceHandle, compile,
};
use axioval_ir::contract::{ExternalName, RuleApplicability};
use axioval_ir::{
    DefinitionPackage, Evidence, NotEvaluatedReason, Object, ObjectId, Project, Property,
    PropertyValue, RuleSetPackage, SourceId,
};
use axioval_rules::register_builtins;

/// Type system of the fixture's wall concept.
const IFC43: &str = "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3";

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn packages() -> (DefinitionPackage, RuleSetPackage) {
    let definitions: DefinitionPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let mut rules: RuleSetPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    let rule = &mut rules.root.rules[0];
    if let RuleApplicability::Groups(groups) = &rule.applicability {
        rule.applicability = RuleApplicability::Selector(groups.groups["walls"].selector.clone());
    }
    rule.requirements.clear();
    (definitions, rules)
}

/// Gives the property and set concepts names in IFC4.3 too, so one declared
/// type system binds every concept the rule uses.
fn bind_properties_in_ifc43(mut definitions: DefinitionPackage) -> DefinitionPackage {
    definitions
        .properties
        .get_mut("axioval:example.ifc.reference")
        .unwrap()
        .external_names
        .push(ExternalName {
            type_system: IFC43.into(),
            name: "Reference".into(),
        });
    definitions
        .property_sets
        .get_mut("axioval:example.ifc.pset-wall-common")
        .unwrap()
        .external_names
        .push(ExternalName {
            type_system: IFC43.into(),
            name: "Pset_WallCommon".into(),
        });
    definitions
}

fn snapshot(type_system: Option<&str>) -> SourceSnapshot {
    let snapshot = SourceSnapshot::try_new(source(), "r1", "sha256:binding").unwrap();
    match type_system {
        Some(system) => snapshot.with_type_system(system).unwrap(),
        None => snapshot,
    }
}

fn wall(kind: &str) -> Object {
    Object::new(ObjectId::new(source(), "#1").unwrap(), kind)
}

fn wall_numbered(number: usize) -> Object {
    Object::new(
        ObjectId::new(source(), format!("#{number:03}")).unwrap(),
        "IFCWALL",
    )
}

/// Records every request, so a test can prove which source names were asked for.
struct Recording {
    snapshots: Vec<SourceSnapshot>,
    seen: std::sync::Mutex<Vec<(Option<String>, String)>>,
}
impl PropertyResolutionService for Recording {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        self.seen.lock().unwrap().push((
            request.property_set().map(ToOwned::to_owned),
            request.property().to_owned(),
        ));
        if request.property_set() == Some("Pset_WallCommon") && request.property() == "Reference" {
            let property = Property::new(
                "Pset_WallCommon",
                "Reference",
                PropertyValue::String("W-1".into()),
            )
            .unwrap()
            .with_evidence(Evidence::exact(source(), "#4/#2"));
            return Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                property,
            )?));
        }
        Ok(PropertyResolution::Absent(
            CompletePropertyAbsenceEvidence::try_new(
                request.clone(),
                Evidence::exact(source(), "complete property table"),
            )
            .unwrap(),
        ))
    }
}

/// IFC-style subtype table: `IfcWallStandardCase` is a wall.
struct Hierarchy(Vec<SourceSnapshot>);
impl TypeHierarchyService for Hierarchy {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, TypeHierarchyError> {
        let kind = kind.to_ascii_uppercase();
        let ancestor = ancestor.to_ascii_uppercase();
        Ok(kind == ancestor || (kind == "IFCWALLSTANDARDCASE" && ancestor == "IFCWALL"))
    }
}

fn run(
    definitions: DefinitionPackage,
    objects: Vec<Object>,
    snapshot: SourceSnapshot,
    hierarchy: bool,
) -> (axioval_ir::Report, Vec<(Option<String>, String)>) {
    let (_, rules) = packages();
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let service = Arc::new(Recording {
        snapshots: vec![snapshot.clone()],
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let mut session = EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot.clone()])
        .unwrap()
        .with_service(PropertyResolutionServiceHandle::new(service.clone()))
        .unwrap();
    if hierarchy {
        session = session
            .with_service(TypeHierarchyServiceHandle::new(Arc::new(Hierarchy(vec![
                snapshot,
            ]))))
            .unwrap();
    }
    let report = Runtime::new(registry).run_session(&session, plan).unwrap();
    let seen = service.seen.lock().unwrap().clone();
    (report, seen)
}

#[test]
fn unbound_type_system_is_not_evaluated_never_a_clean_pass() {
    // The exact configuration that used to return findings=0, not_evaluated=0.
    let (definitions, _) = packages();
    let (report, seen) = run(definitions, vec![wall("IFCWALL")], snapshot(None), true);
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::UnboundConcept
    );
    assert!(
        seen.is_empty(),
        "no property may be requested for an unbound concept"
    );
}

#[test]
fn concept_without_a_name_in_the_declared_system_is_not_evaluated() {
    // The wall binds in IFC4.3, but the property and set concepts carry only
    // the examples namespace, so the property request cannot be formed.
    let (definitions, _) = packages();
    let (report, seen) = run(
        definitions,
        vec![wall("IFCWALL")],
        snapshot(Some(IFC43)),
        true,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert!(
        report.not_evaluated()[0]
            .message
            .contains("axioval:example.ifc.reference")
    );
    assert!(seen.is_empty());
}

#[test]
fn bound_concepts_are_translated_to_source_names() {
    let (definitions, _) = packages();
    let (report, seen) = run(
        bind_properties_in_ifc43(definitions),
        vec![wall("IFCWALL")],
        snapshot(Some(IFC43)),
        true,
    );
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert!(report.findings().is_empty(), "the wall has the property");
    assert_eq!(
        seen,
        [(Some("Pset_WallCommon".to_owned()), "Reference".to_owned())],
        "the source must be asked in its own vocabulary, never for the concept ID"
    );
}

#[test]
fn bound_type_selects_subtypes_through_the_hierarchy_service() {
    let (definitions, _) = packages();
    let (report, seen) = run(
        bind_properties_in_ifc43(definitions),
        vec![wall("IFCWALLSTANDARDCASE")],
        snapshot(Some(IFC43)),
        true,
    );
    assert!(
        report.not_evaluated().is_empty(),
        "{:?}",
        report.not_evaluated()
    );
    assert_eq!(seen.len(), 1, "the subtype occurrence must be selected");
}

#[test]
fn subtype_selection_without_a_hierarchy_service_is_not_evaluated() {
    let (definitions, _) = packages();
    let (report, seen) = run(
        bind_properties_in_ifc43(definitions),
        vec![wall("IFCWALLSTANDARDCASE")],
        snapshot(Some(IFC43)),
        false,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::MissingService
    );
    assert!(seen.is_empty());
}

#[test]
fn unrelated_kinds_are_not_selected() {
    let (definitions, _) = packages();
    let (report, seen) = run(
        bind_properties_in_ifc43(definitions),
        vec![wall("IFCSLAB")],
        snapshot(Some(IFC43)),
        true,
    );
    assert!(report.findings().is_empty());
    assert!(report.not_evaluated().is_empty());
    assert!(seen.is_empty(), "a slab is not a wall");
}

#[test]
fn unknown_concepts_are_rejected_at_compile_time() {
    let (definitions, mut rules) = packages();
    if let RuleApplicability::Selector(axioval_ir::contract::Selector::EntityType {
        object_type,
        ..
    }) = &mut rules.root.rules[0].applicability
    {
        *object_type = "axioval:example.ifc.never-declared".into();
    }
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    assert!(matches!(
        compile(&registry, &[definitions], &rules),
        Err(axioval_engine::EngineError::UnknownConcept { .. })
    ));
}

#[test]
fn single_target_group_runs_and_multi_group_is_reported_not_dropped() {
    // Real MCS output: the minimal example names one target group. That is
    // exactly one population, so it compiles to an executable rule.
    let definitions: DefinitionPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let mut rules: RuleSetPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions.clone()], &rules).unwrap();
    assert_eq!(plan.rules().len(), 1);
    assert!(plan.deferred().is_empty());

    // Two groups name two populations. A one-selector capability would have
    // to pick one or their union, so the rule must be reported as not
    // evaluated rather than silently vanishing from the report.
    let RuleApplicability::Groups(groups) = &mut rules.root.rules[0].applicability else {
        panic!("the MCS minimal example uses target-group applicability");
    };
    let mut second = groups.groups["walls"].clone();
    second.id = "more-walls".into();
    groups.groups.insert("more-walls".into(), second);
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    assert!(plan.rules().is_empty());
    assert_eq!(plan.deferred().len(), 1);
    let session = EvidenceSession::try_new(
        Project::new(vec![wall("IFCWALL")]).unwrap(),
        [snapshot(Some(IFC43))],
    )
    .unwrap();
    let report = Runtime::new(registry).run_session(&session, plan).unwrap();
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].rule_id.to_string(),
        "wall-reference-required"
    );
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::InvalidDeclaration
    );
}

#[test]
fn a_concept_binds_only_through_the_declared_type_system() {
    // The property and set concepts carry a name only in the examples
    // namespace. A source declaring IFC4.3 must not borrow that name.
    let (definitions, _) = packages();
    let (report, seen) = run(
        definitions,
        vec![wall("IFCWALL")],
        snapshot(Some(IFC43)),
        true,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert!(
        seen.is_empty(),
        "no request may use a foreign type system's name"
    );
}

#[test]
fn a_concept_with_two_names_for_one_source_is_ambiguous_not_first_wins() {
    // The source declares two type systems and the wall concept names a
    // different entity in each. Picking either would select a population
    // the author did not name, so the rule is not evaluated.
    const OTHER: &str = "https://example.org/other-type-system";
    let (mut definitions, _) = packages();
    definitions
        .object_types
        .get_mut("axioval:example.ifc.wall")
        .unwrap()
        .external_names
        .push(ExternalName {
            type_system: OTHER.into(),
            name: "IfcSlab".into(),
        });
    let two_systems = snapshot(Some(IFC43)).with_type_system(OTHER).unwrap();
    let (report, seen) = run(
        bind_properties_in_ifc43(definitions),
        vec![wall("IFCWALL")],
        two_systems,
        true,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert!(
        report.not_evaluated()[0].message.contains("ambiguous"),
        "{}",
        report.not_evaluated()[0].message
    );
    assert!(seen.is_empty());
}

#[test]
fn an_unbound_concept_is_reported_once_per_source_not_per_object() {
    let (definitions, _) = packages();
    let walls = (1..=50).map(wall_numbered).collect();
    let (report, _) = run(definitions, walls, snapshot(None), true);
    assert!(report.findings().is_empty());
    assert_eq!(
        report.not_evaluated().len(),
        1,
        "{:?}",
        report.not_evaluated()
    );
    let outcome = &report.not_evaluated()[0];
    assert_eq!(outcome.reason, NotEvaluatedReason::UnboundConcept);
    assert_eq!(
        outcome.object_id, None,
        "a source-level cause names no object"
    );
    assert!(
        outcome.message.contains("50 object(s) of source"),
        "{}",
        outcome.message
    );
    assert!(outcome.message.contains("+47 more"), "{}", outcome.message);
}
