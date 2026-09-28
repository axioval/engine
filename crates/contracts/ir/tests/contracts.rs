//! IR contract tests.
#![allow(missing_docs)]

use axioval_ir::{
    Classification, Evidence, ExternalId, ExternalIdClash, IrError, Object, ObjectId, Project,
    Property, PropertyValue, Selector, SourceId,
};

#[test]
fn project_object_ids_are_source_qualified_and_ordered() {
    let source = SourceId::new("ifc", "model-a").unwrap();
    let id = ObjectId::new(source.clone(), "Wall-7").unwrap();
    let project = Project::new(vec![Object::new(id.clone(), "Wall")]).unwrap();
    assert_eq!(project.object(&id).unwrap().kind(), "Wall");
    assert!(id.to_string().contains("ifc:model-a"));
}

#[test]
fn selector_filters_by_kind_and_classification() {
    let source = SourceId::new("source", "a").unwrap();
    let object = Object::new(ObjectId::new(source, "1").unwrap(), "Wall")
        .with_classification(Classification::new("Uniclass", "EF_25").unwrap());
    let selector = Selector::by_kind("Wall").with_classification("Uniclass", "EF_25");
    assert!(selector.matches(&object));
}

#[test]
fn semantic_property_values_preserve_provenance() {
    let source = SourceId::new("source", "a").unwrap();
    let owner = ObjectId::new(source.clone(), "1").unwrap();
    let property = Property::new(
        "Pset_WallCommon",
        "FireRating",
        PropertyValue::String("60".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(source, "property:1"));
    let object = Object::new(owner, "Wall").with_property(property);
    assert_eq!(
        object
            .property("Pset_WallCommon", "FireRating")
            .unwrap()
            .value(),
        &PropertyValue::String("60".into())
    );
}

#[test]
fn legacy_report_without_not_evaluated_field_remains_readable() {
    let report: axioval_ir::Report = serde_json::from_str(r#"{"findings":[]}"#).unwrap();
    assert!(report.not_evaluated().is_empty());
}

fn object(source: &SourceId, local: &str) -> Object {
    Object::new(ObjectId::new(source.clone(), local).unwrap(), "Wall")
}

#[test]
fn external_ids_are_aliases_looked_up_by_scheme() {
    let source = SourceId::new("source", "a").unwrap();
    let wall = object(&source, "1")
        .with_external_id(ExternalId::new("scheme-b", "B").unwrap())
        .with_external_id(ExternalId::new("scheme-a", "A").unwrap());
    let schemes: Vec<_> = wall.external_ids.iter().map(|id| &*id.scheme).collect();
    assert_eq!(schemes, ["scheme-a", "scheme-b"]);
    assert_eq!(wall.external_id("scheme-b"), Some("B"));
    assert_eq!(wall.external_id("scheme-c"), None);
    assert!(ExternalId::new(" ", "A").is_err());
    assert!(ExternalId::new("scheme-a", "").is_err());
}

#[test]
fn an_object_with_two_ids_in_one_scheme_is_rejected() {
    let source = SourceId::new("source", "a").unwrap();
    let wall = object(&source, "1")
        .with_external_id(ExternalId::new("scheme", "A").unwrap())
        .with_external_id(ExternalId::new("scheme", "B").unwrap());
    assert_eq!(
        Project::new(vec![wall]),
        Err(IrError::ConflictingExternalId {
            object: ObjectId::new(source, "1").unwrap(),
            scheme: "scheme".into(),
        })
    );
    // Deserialized objects need not be sorted; the check must not rely on order.
    let unsorted: Object = serde_json::from_str(
        r#"{"id":{"source":{"system":"source","document":"a"},"local_id":"1"},"kind":"Wall",
            "external_ids":[{"scheme":"x","value":"A"},{"scheme":"y","value":"B"},
                            {"scheme":"x","value":"C"}],
            "properties":[],"classifications":[],"relationships":{}}"#,
    )
    .unwrap();
    assert!(matches!(
        Project::new(vec![unsorted]),
        Err(IrError::ConflictingExternalId { .. })
    ));
}

#[test]
fn a_shared_external_id_is_rejected_within_a_source_only() {
    let a = SourceId::new("source", "a").unwrap();
    let b = SourceId::new("source", "b").unwrap();
    let id = ExternalId::new("scheme", "same").unwrap();
    let duplicated = Project::new(vec![
        object(&a, "2").with_external_id(id.clone()),
        object(&a, "1").with_external_id(id.clone()),
    ]);
    assert_eq!(
        duplicated,
        Err(IrError::DuplicateExternalId(Box::new(ExternalIdClash {
            id: id.clone(),
            first: ObjectId::new(a.clone(), "1").unwrap(),
            second: ObjectId::new(a.clone(), "2").unwrap(),
        })))
    );
    // Two revisions of one model legitimately carry the same alias.
    assert!(
        Project::new(vec![
            object(&a, "1").with_external_id(id.clone()),
            object(&b, "1").with_external_id(id),
        ])
        .is_ok()
    );
}

#[test]
fn objects_without_external_ids_keep_their_serialized_shape() {
    let source = SourceId::new("source", "a").unwrap();
    let json = serde_json::to_string(&object(&source, "1")).unwrap();
    assert!(!json.contains("external_ids"));
    let back: Object = serde_json::from_str(&json).unwrap();
    assert!(back.external_ids.is_empty());
}

#[test]
fn a_property_carries_its_declared_data_type_only_when_reported() {
    let untyped = Property::new("Pset", "Code", PropertyValue::String("A".into())).unwrap();
    let json = serde_json::to_string(&untyped).unwrap();
    // Properties without a reported type keep their serialized shape.
    assert!(!json.contains("data_type"), "{json}");
    assert_eq!(untyped.data_type(), None);

    let typed = untyped.with_data_type("IFCLABEL").unwrap();
    let back: Property = serde_json::from_str(&serde_json::to_string(&typed).unwrap()).unwrap();
    assert_eq!(back.data_type(), Some("IFCLABEL"));
    assert_eq!(back, typed);

    let blank = Property::new("Pset", "Code", PropertyValue::Null)
        .unwrap()
        .with_data_type(" ");
    assert!(matches!(blank, Err(IrError::Blank { .. })));
}

mod scope {
    use axioval_ir::{
        Evidence, Finding, NotEvaluated, NotEvaluatedReason, ObjectId, RuleId, Scope, Severity,
        SourceId,
    };
    use serde_json::{Value, json};

    fn source() -> SourceId {
        SourceId::new("ifc-step", "model.ifc").unwrap()
    }

    fn finding(scope: Scope) -> Finding {
        Finding::new(RuleId::new("r").unwrap(), scope, Severity::Error, "m").with_evidence([
            Evidence::exact(source(), "b"),
            Evidence::exact(source(), "a"),
        ])
    }

    #[test]
    fn an_object_finding_serializes_exactly_as_before_scopes() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let value = serde_json::to_value(finding(Scope::Object(object))).unwrap();
        assert_eq!(
            value,
            json!({
                "rule_id": "r",
                "object_id": {"source": {"system": "ifc-step", "document": "model.ifc"},
                              "local_id": "#1"},
                "severity": "error",
                "message": "m",
                "evidence": [
                    {"source": {"system": "ifc-step", "document": "model.ifc"},
                     "locator": "a", "exact": true},
                    {"source": {"system": "ifc-step", "document": "model.ifc"},
                     "locator": "b", "exact": true},
                ],
            })
        );
    }

    #[test]
    fn a_located_finding_writes_its_storeys_and_spaces_and_reads_back() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let mut located = finding(Scope::Object(object));
        located.location = Some(axioval_ir::Location {
            storeys: vec![axioval_ir::Place {
                id: ObjectId::new(source(), "#10").unwrap(),
                name: Some("Level 1".into()),
            }],
            spaces: vec![],
            unresolved: Some("why".into()),
        });
        let value = serde_json::to_value(&located).unwrap();
        assert_eq!(
            value["location"],
            json!({
                "storeys": [{"id": {"source": {"system": "ifc-step", "document": "model.ifc"},
                                    "local_id": "#10"},
                             "name": "Level 1"}],
                "unresolved": "why",
            })
        );
        assert_eq!(serde_json::from_value::<Finding>(value).unwrap(), located);
    }

    #[test]
    fn a_categorised_finding_writes_its_levels_and_reads_back() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let plain = finding(Scope::Object(object));
        assert!(
            serde_json::to_value(&plain)
                .unwrap()
                .get("categories")
                .is_none()
        );
        let mut categorised = plain;
        categorised.categories = vec!["F90".into(), "Office".into()];
        let value = serde_json::to_value(&categorised).unwrap();
        assert_eq!(value["categories"], json!(["F90", "Office"]));
        assert_eq!(
            serde_json::from_value::<Finding>(value).unwrap(),
            categorised
        );
    }

    #[test]
    fn source_and_project_findings_round_trip() {
        for scope in [Scope::Source(source()), Scope::Project] {
            let original = finding(scope);
            let value = serde_json::to_value(&original).unwrap();
            assert!(value.get("object_id").is_none(), "{value}");
            assert_eq!(
                value.get("source").is_some(),
                matches!(original.scope, Scope::Source(_))
            );
            let read: Finding = serde_json::from_value(value).unwrap();
            assert_eq!(read, original);
            assert_eq!(read.object_id(), None);
        }
    }

    #[test]
    fn a_record_naming_both_an_object_and_a_source_is_rejected() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let mut value = serde_json::to_value(finding(Scope::Object(object))).unwrap();
        value["source"] = serde_json::to_value(source()).unwrap();
        let error = serde_json::from_value::<Finding>(value.clone()).unwrap_err();
        assert!(error.to_string().contains("both object"), "{error}");

        value["reason"] = json!("missing_service");
        for field in ["severity", "evidence"] {
            value.as_object_mut().unwrap().remove(field);
        }
        assert!(serde_json::from_value::<NotEvaluated>(value).is_err());
    }

    #[test]
    fn a_rule_level_outcome_still_writes_a_null_object() {
        let outcome = NotEvaluated {
            rule_id: RuleId::new("r").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "m".into(),
            location: None,
        };
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["object_id"], Value::Null);
        assert!(value.get("source").is_none());
        let scoped = NotEvaluated {
            scope: Scope::Source(source()),
            ..outcome
        };
        let value = serde_json::to_value(&scoped).unwrap();
        assert_eq!(value["source"]["document"], "model.ifc");
        assert_eq!(
            serde_json::from_value::<NotEvaluated>(value).unwrap(),
            scoped
        );
    }

    #[test]
    fn scopes_order_project_then_sources_then_objects() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let mut scopes = vec![
            Scope::Object(object.clone()),
            Scope::Source(source()),
            Scope::Project,
        ];
        scopes.sort();
        assert_eq!(
            scopes,
            [
                Scope::Project,
                Scope::Source(source()),
                Scope::Object(object)
            ]
        );
        assert_eq!(scopes[1].source(), Some(&source()));
        assert_eq!(scopes[2].source(), Some(&source()));
        assert_eq!(scopes[1].to_string(), "source ifc-step:model.ifc");
    }

    #[test]
    fn an_object_finding_never_relates_its_subject() {
        let object = ObjectId::new(source(), "#1").unwrap();
        let other = ObjectId::new(source(), "#2").unwrap();
        let scoped = finding(Scope::Source(source())).with_related([other.clone(), object.clone()]);
        assert_eq!(scoped.related, [object.clone(), other.clone()]);
        let own = finding(Scope::Object(object.clone())).with_related([other.clone(), object]);
        assert_eq!(own.related, [other]);
    }
}

mod tables {
    use axioval_ir::{
        Finding, NotEvaluated, NotEvaluatedReason, ObjectId, QuantityDimension, Report,
        ReportColumn, ReportTable, ReportTableError, ReportValue, RuleId, Scope, Severity,
        SourceId,
    };
    use serde_json::json;

    fn source() -> SourceId {
        SourceId::new("ifc-step", "model.ifc").unwrap()
    }

    fn object(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn levels() -> ReportTable {
        ReportTable::new(
            RuleId::new("storey-height").unwrap(),
            "levels",
            vec![
                ReportColumn::quantity("height", QuantityDimension::Length),
                ReportColumn::number("ratio"),
                ReportColumn::text("level"),
            ],
        )
        .unwrap()
    }

    fn report(tables: Vec<ReportTable>) -> Report {
        Report {
            stale_decisions: Vec::new(),
            findings: vec![Finding::new(
                RuleId::new("r").unwrap(),
                object("#1"),
                Severity::Error,
                "m",
            )],
            not_evaluated: vec![NotEvaluated {
                rule_id: RuleId::new("r").unwrap(),
                scope: Scope::Project,
                reason: NotEvaluatedReason::MissingService,
                message: "m".into(),
                location: None,
            }],
            tables,
            rules: Vec::new(),
        }
    }

    #[test]
    fn a_report_without_tables_serializes_byte_for_byte_as_before() {
        let text = serde_json::to_string(&report(vec![])).unwrap();
        assert_eq!(
            text,
            concat!(
                r#"{"findings":[{"rule_id":"r","object_id":{"source":{"system":"ifc-step","#,
                r##""document":"model.ifc"},"local_id":"#1"},"severity":"error","message":"m","##,
                r#""evidence":[]}],"not_evaluated":[{"rule_id":"r","object_id":null,"#,
                r#""reason":"missing_service","message":"m"}]}"#
            )
        );
        let read: Report = serde_json::from_str(&text).unwrap();
        assert!(read.tables().is_empty());
        assert_eq!(read, report(vec![]));
    }

    #[test]
    fn tables_round_trip_with_rows_in_scope_order() {
        let mut table = levels();
        assert!(table.is_empty());
        table
            .push_row(
                object("#9"),
                vec![
                    ReportValue::measured(2.9, 3.1),
                    ReportValue::exact(0.25),
                    ReportValue::text("EG"),
                ],
            )
            .unwrap();
        table
            .push_row(
                Scope::Project,
                vec![
                    ReportValue::Unknown,
                    ReportValue::measured(1.0, f64::INFINITY),
                    ReportValue::Unknown,
                ],
            )
            .unwrap();
        table
            .push_row(
                source(),
                vec![
                    ReportValue::measured(3.0, 3.0),
                    ReportValue::Unknown,
                    ReportValue::Unknown,
                ],
            )
            .unwrap();
        let scopes: Vec<_> = table.rows().iter().map(|row| row.scope().clone()).collect();
        assert_eq!(
            scopes,
            [
                Scope::Project,
                Scope::Source(source()),
                Scope::Object(object("#9"))
            ]
        );
        let original = report(vec![table]);
        let value = serde_json::to_value(&original).unwrap();
        assert_eq!(
            value["tables"],
            json!([{
                "rule_id": "storey-height",
                "name": "levels",
                "columns": [
                    {"id": "height", "kind": "quantity", "dimension": "length"},
                    {"id": "ratio", "kind": "number"},
                    {"id": "level", "kind": "text"},
                ],
                "rows": [
                    {"values": [{"type": "unknown"}, {"type": "unknown"}, {"type": "unknown"}]},
                    {"source": {"system": "ifc-step", "document": "model.ifc"},
                     "values": [{"type": "exact", "value": 3.0}, {"type": "unknown"},
                                {"type": "unknown"}]},
                    {"object_id": {"source": {"system": "ifc-step", "document": "model.ifc"},
                                   "local_id": "#9"},
                     "values": [{"type": "interval", "lower": 2.9, "upper": 3.1},
                                {"type": "exact", "value": 0.25},
                                {"type": "text", "value": "EG"}]},
                ],
            }])
        );
        let read: Report = serde_json::from_value(value).unwrap();
        assert_eq!(read, original);
        let rule = RuleId::new("storey-height").unwrap();
        let table = read.table(&rule, "levels").unwrap();
        assert_eq!(
            table.row(&Scope::Object(object("#9"))).unwrap().values()[2],
            ReportValue::text("EG")
        );
        assert!(read.table(&rule, "spaces").is_none());
    }

    #[test]
    fn rows_read_out_of_order_are_sorted() {
        let rows = |first: &str, second: &str| {
            json!({
                "rule_id": "r", "name": "t",
                "columns": [{"id": "a", "kind": "number"}],
                "rows": [
                    {"object_id": {"source": {"system": "s", "document": "d"}, "local_id": first},
                     "values": [{"type": "exact", "value": 1.0}]},
                    {"object_id": {"source": {"system": "s", "document": "d"}, "local_id": second},
                     "values": [{"type": "exact", "value": 2.0}]},
                ],
            })
        };
        let sorted: ReportTable = serde_json::from_value(rows("#1", "#2")).unwrap();
        let shuffled: ReportTable = serde_json::from_value(rows("#2", "#1")).unwrap();
        assert_eq!(sorted.rows()[0].scope(), shuffled.rows()[0].scope());
        assert!(serde_json::from_value::<ReportTable>(rows("#1", "#1")).is_err());
    }

    #[test]
    fn invalid_tables_and_rows_are_refused() {
        let rule = RuleId::new("r").unwrap();
        for name in ["", "Levels", "-x", "a b"] {
            assert!(matches!(
                ReportTable::new(rule.clone(), name, vec![ReportColumn::number("a")]),
                Err(ReportTableError::InvalidName { .. })
            ));
        }
        assert!(matches!(
            ReportTable::new(rule.clone(), "t", vec![]),
            Err(ReportTableError::NoColumns(_))
        ));
        assert!(matches!(
            ReportTable::new(
                rule.clone(),
                "t",
                vec![ReportColumn::number("a"), ReportColumn::text("a")]
            ),
            Err(ReportTableError::DuplicateColumn { .. })
        ));
        let mut table = levels();
        let refused = |table: &mut ReportTable, values: Vec<ReportValue>| {
            let error = table.push_row(object("#1"), values).unwrap_err();
            assert!(matches!(error, ReportTableError::InvalidRow { .. }));
            error.to_string()
        };
        assert!(refused(&mut table, vec![ReportValue::Unknown]).contains("1 value(s) for 3"));
        let text_as_number = vec![
            ReportValue::text("3 m"),
            ReportValue::Unknown,
            ReportValue::Unknown,
        ];
        assert!(refused(&mut table, text_as_number).contains("column `height`"));
        let number_as_text = vec![
            ReportValue::Unknown,
            ReportValue::Unknown,
            ReportValue::exact(1.0),
        ];
        assert!(refused(&mut table, number_as_text).contains("text column"));
        let point = vec![
            ReportValue::Interval {
                lower: 1.0,
                upper: 1.0,
            },
            ReportValue::Unknown,
            ReportValue::Unknown,
        ];
        assert!(refused(&mut table, point).contains("a point is exact"));
        let infinite = vec![
            ReportValue::Exact {
                value: f64::INFINITY,
            },
            ReportValue::Unknown,
            ReportValue::Unknown,
        ];
        assert!(refused(&mut table, infinite).contains("not finite"));
        assert!(table.is_empty());
        table
            .push_row(object("#1"), vec![ReportValue::Unknown; 3])
            .unwrap();
        assert!(refused(&mut table, vec![ReportValue::Unknown; 3]).contains("exists already"));
        assert_eq!(table.rows().len(), 1);

        let column = |value| serde_json::from_value::<ReportColumn>(value);
        assert!(column(json!({"id": "a", "kind": "quantity"})).is_err());
        assert!(column(json!({"id": "a", "kind": "number", "dimension": "area"})).is_err());
        assert_eq!(
            column(json!({"id": "a", "kind": "quantity", "dimension": "area"})).unwrap(),
            ReportColumn::quantity("a", QuantityDimension::Area)
        );
    }

    #[test]
    fn measured_values_are_exact_only_as_points() {
        assert_eq!(
            ReportValue::measured(2.0, 2.0),
            ReportValue::Exact { value: 2.0 }
        );
        assert_eq!(ReportValue::measured(2.0, 1.0), ReportValue::Unknown);
        assert_eq!(ReportValue::exact(f64::NAN), ReportValue::Unknown);
        assert_eq!(ReportValue::measured(1.0, 2.0).to_string(), "1..2");
    }
}

#[test]
fn gates_and_rule_outcome_selectors_read_and_write_their_package_form() {
    use axioval_ir::contract::{
        GateCondition, RuleFolder, RuleGate, RuleOutcomeKind, Selector as Schema,
    };
    use serde_json::json;

    let selector: Schema = serde_json::from_value(
        json!({"kind": "ruleOutcome", "rule": "door-type", "outcome": "failed"}),
    )
    .unwrap();
    assert_eq!(
        selector,
        Schema::RuleOutcome {
            rule: "door-type".into(),
            outcome: RuleOutcomeKind::Failed
        }
    );
    let folder: RuleFolder = serde_json::from_value(json!({
        "id": "hardware",
        "name": {"default": "Hardware", "translations": {}},
        "description": null,
        "gate": {"rule": "door-type", "condition": "failedObjects"},
    }))
    .unwrap();
    assert_eq!(
        folder.gate,
        Some(RuleGate {
            rule: "door-type".into(),
            condition: GateCondition::FailedObjects
        })
    );
    let written = serde_json::to_value(&folder).unwrap();
    assert_eq!(written["gate"]["condition"], "failedObjects");
    // An ungated folder writes no gate.
    let ungated = RuleFolder {
        gate: None,
        ..folder
    };
    assert!(
        serde_json::to_value(&ungated)
            .unwrap()
            .get("gate")
            .is_none()
    );
    assert!(
        serde_json::from_value::<RuleGate>(json!({"rule": "a", "condition": "sometimes"})).is_err()
    );
}

#[test]
fn a_measured_value_reads_and_writes_its_interval() {
    use axioval_ir::QuantityDimension;
    use serde_json::json;

    let wire = json!({"type": "measured", "value": {"lower": 0.045, "upper": 0.055, "dimension": "length"}});
    let value: PropertyValue = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        value,
        PropertyValue::Measured {
            lower: 0.045,
            upper: 0.055,
            dimension: QuantityDimension::Length
        }
    );
    assert!(value.is_scalar());
    assert!(value.stated_values().is_none());
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    assert!(axioval_ir::is_derived_set(axioval_ir::MEASURED_SET));
    assert!(axioval_ir::is_reserved_set(axioval_ir::MEASURED_SET));
}

#[test]
fn an_auxiliary_rule_reads_and_writes_its_flag_only_when_set() {
    use axioval_ir::contract::RuleInstance;
    use serde_json::json;

    let written = json!({
        "id": "door-type",
        "definitionId": "d",
        "name": {"default": "Door type", "translations": {}},
        "description": null,
        "message": null,
        "auxiliary": true,
    });
    let rule: RuleInstance = serde_json::from_value(written).unwrap();
    assert!(rule.auxiliary);
    assert_eq!(serde_json::to_value(&rule).unwrap()["auxiliary"], true);
    // A reported rule writes no flag, and reads as reported without one.
    let reported = RuleInstance {
        auxiliary: false,
        ..rule
    };
    let value = serde_json::to_value(&reported).unwrap();
    assert!(value.get("auxiliary").is_none());
    assert!(
        !serde_json::from_value::<RuleInstance>(value)
            .unwrap()
            .auxiliary
    );
}
