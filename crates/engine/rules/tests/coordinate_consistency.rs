//! `coordinate-consistency`: discipline models share one coordinate system.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CoordinateFrame, CoordinateSystemError, CoordinateSystemService,
    CoordinateSystemServiceHandle, MapConversion, MetricDirection, SitePlacement,
    SourceCoordinateSystem, SourceDisciplines, SourceSnapshot,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Discipline, Evidence, NotEvaluatedReason, Scope, SourceId};
use axioval_rules::CoordinateConsistencyCheck;
use common::{Model, boolean, kind, number, rule, string};

const ID: &str = "axioval:capability.coordinate-consistency";

fn document(name: &str) -> SourceId {
    SourceId::new("test", name).unwrap()
}

fn frame(origin: [f64; 3]) -> CoordinateFrame {
    let axis = |v| MetricDirection::try_new(v).unwrap();
    CoordinateFrame::try_new(
        origin,
        axis([1.0, 0.0, 0.0]),
        axis([0.0, 1.0, 0.0]),
        axis([0.0, 0.0, 1.0]),
    )
    .unwrap()
}

/// A model georeferenced onto EPSG:25832 at `easting`, in millimetre map
/// units when `millimetres`, its site at the world origin.
fn georeferenced(name: &str, easting: f64, millimetres: bool) -> SourceCoordinateSystem {
    let unit = if millimetres { 0.001 } else { 1.0 };
    let map = MapConversion::try_new(
        Some("EPSG:25832".into()),
        [easting / unit, 5_600_000.0 / unit, 50.0 / unit],
        [1.0, 0.0],
        1.0,
        Some(unit),
    )
    .unwrap();
    system(name, Some(map))
}

fn system(name: &str, map: Option<MapConversion>) -> SourceCoordinateSystem {
    SourceCoordinateSystem::try_new(
        document(name),
        Some(frame([0.0; 3])),
        Some([0.0, 1.0]),
        map,
        Evidence::exact(document(name), format!("crs:{name}")),
    )
    .unwrap()
    .with_site(SitePlacement::Stated(frame([0.0; 3])))
}

struct Systems(
    Vec<SourceSnapshot>,
    BTreeMap<SourceId, Result<SourceCoordinateSystem, CoordinateSystemError>>,
);

impl CoordinateSystemService for Systems {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn coordinate_system(
        &self,
        source: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        self.1[source].clone()
    }
}

fn evaluate(
    systems: Vec<(&str, Result<SourceCoordinateSystem, CoordinateSystemError>)>,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut model = Model::default();
    let mut snapshots = Vec::new();
    let mut disciplines = Vec::new();
    for (name, _) in &systems {
        model = model.object_in(name, "#1", "wall");
        snapshots
            .push(SourceSnapshot::try_new(document(name), "r", format!("sha256:{name}")).unwrap());
        disciplines.push((document(name), Discipline::new(*name).unwrap()));
    }
    let systems = systems
        .into_iter()
        .map(|(name, system)| (document(name), system))
        .collect();
    model.evaluate_with(
        &CoordinateConsistencyCheck,
        &rule(ID, kind("wall"), parameters),
        |services| {
            services
                .register(CoordinateSystemServiceHandle::new(Arc::new(Systems(
                    snapshots, systems,
                ))))
                .unwrap();
            services
                .register(SourceDisciplines::new(disciplines))
                .unwrap();
        },
    )
}

fn by_architecture() -> Vec<(&'static str, ParameterValue)> {
    vec![("reference", string("architecture"))]
}

/// `(source, message)` of every finding.
fn found(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    evaluation
        .findings()
        .iter()
        .map(|finding| match &finding.scope {
            Scope::Source(source) => (source.document.clone(), finding.message.clone()),
            other => panic!("a finding against {other:?}"),
        })
        .collect()
}

/// `(source, reason)` of every not-evaluated outcome.
fn unevaluated(evaluation: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason)> {
    evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| match outcome.scope() {
            Scope::Source(source) => (source.document.clone(), outcome.reason().clone()),
            _ => ("-".into(), outcome.reason().clone()),
        })
        .collect()
}

#[test]
fn an_architectural_and_a_structural_model_with_the_same_georeference_pass() {
    let evaluation = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            // The same georeference stated in millimetres.
            (
                "structural",
                Ok(georeferenced("structural", 500_000.0, true)),
            ),
        ],
        by_architecture(),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_structural_model_shifted_by_one_metre_is_a_finding_naming_it() {
    let evaluation = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            (
                "structural",
                Ok(georeferenced("structural", 500_001.0, false)),
            ),
        ],
        by_architecture(),
    );
    assert_eq!(
        found(&evaluation),
        vec![(
            "structural".into(),
            "`test:structural` does not share the coordinate system of `test:architecture`: map offset moved by 1.0000 m".into()
        )]
    );
    let evidence = &evaluation.findings()[0].evidence;
    assert_eq!(
        evidence
            .iter()
            .map(|e| e.locator.as_str())
            .collect::<Vec<_>>(),
        vec!["crs:architecture", "crs:structural"],
        "the finding cites both coordinate systems"
    );
    let within = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            (
                "structural",
                Ok(georeferenced("structural", 500_001.0, false)),
            ),
        ],
        vec![
            ("reference", string("architecture")),
            ("length_tolerance", number(1.5)),
        ],
    );
    assert!(within.findings().is_empty(), "within an explicit tolerance");
}

/// A map unit the source leaves to its standard's default compares in
/// metres, and the finding says the unit was not stated.
#[test]
fn a_default_map_unit_is_compared_and_named_as_the_default() {
    let by_default = |name: &str, easting: f64| {
        let map = MapConversion::try_new(
            Some("EPSG:25832".into()),
            [easting * 1000.0, 5_600_000_000.0, 50_000.0],
            [1.0, 0.0],
            1.0,
            Some(0.001),
        )
        .unwrap()
        .with_map_unit_by_default();
        system(name, Some(map))
    };
    let same = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            ("structural", Ok(by_default("structural", 500_000.0))),
        ],
        by_architecture(),
    );
    assert!(same.findings().is_empty(), "{same:?}");
    assert!(unevaluated(&same).is_empty(), "compared, never unknown");
    let shifted = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            ("structural", Ok(by_default("structural", 500_001.0))),
        ],
        by_architecture(),
    );
    assert_eq!(
        found(&shifted),
        vec![(
            "structural".into(),
            "`test:structural` does not share the coordinate system of `test:architecture`: map offset moved by 1.0000 m (this source's map unit is the standard's default, not stated)".into()
        )]
    );
}

#[test]
fn the_reference_defaults_to_the_first_source_and_names_the_other() {
    let evaluation = evaluate(
        vec![
            (
                "a-architecture",
                Ok(georeferenced("a-architecture", 500_000.0, false)),
            ),
            (
                "b-structural",
                Ok(georeferenced("b-structural", 500_000.0, false)
                    .with_site(SitePlacement::Stated(frame([0.0, 2.0, 0.0])))),
            ),
        ],
        Vec::new(),
    );
    assert_eq!(found(&evaluation).len(), 1);
    assert_eq!(found(&evaluation)[0].0, "b-structural");
    assert!(
        found(&evaluation)[0]
            .1
            .ends_with("site placement moved by 2.0000 m")
    );
}

#[test]
fn a_model_without_georeferencing_is_not_evaluated_or_a_finding_when_required() {
    let systems = || {
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            ("structural", Ok(system("structural", None))),
        ]
    };
    let evaluation = evaluate(systems(), by_architecture());
    assert!(
        evaluation.findings().is_empty(),
        "never assumed identical or different"
    );
    assert_eq!(
        unevaluated(&evaluation),
        vec![("structural".into(), NotEvaluatedReason::NotRecorded)]
    );
    let required = evaluate(
        systems(),
        vec![
            ("reference", string("architecture")),
            ("require_map_conversion", boolean(true)),
        ],
    );
    assert_eq!(
        found(&required),
        vec![(
            "structural".into(),
            "`test:structural` does not share the coordinate system of `test:architecture`: states no map conversion".into()
        )]
    );
}

#[test]
fn a_statement_only_one_side_makes_or_an_unreadable_system_is_not_evaluated() {
    let unknown_site = georeferenced("structural", 500_000.0, false)
        .with_site(SitePlacement::Unknown("2 sites".into()));
    let evaluation = evaluate(
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            ("structural", Ok(unknown_site)),
            (
                "mep",
                Err(CoordinateSystemError::Ambiguous("2 model contexts".into())),
            ),
        ],
        by_architecture(),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        vec![
            ("mep".into(), NotEvaluatedReason::IncompleteEvidence),
            ("structural".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn one_source_or_no_service_is_not_evaluated() {
    let evaluation = evaluate(
        vec![(
            "architecture",
            Ok(georeferenced("architecture", 500_000.0, false)),
        )],
        Vec::new(),
    );
    assert_eq!(
        unevaluated(&evaluation),
        vec![("-".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let evaluation = Model::default()
        .object_in("a", "#1", "wall")
        .object_in("b", "#1", "wall")
        .evaluate(
            &CoordinateConsistencyCheck,
            &rule(ID, kind("wall"), Vec::new()),
        );
    assert_eq!(
        unevaluated(&evaluation),
        vec![("-".into(), NotEvaluatedReason::MissingService)]
    );
}

/// Each statement's `coordinate_shift`, `coordinate_turn`,
/// `map_scale_change` and `map_target_change` within the tolerances, and
/// `map_conversion` where one is required, reach `coordinate-consistency`'s
/// verdicts. The capability judges a source and the rewrite each object of
/// it; the comparison reads the capability's outcome for a source as its
/// one object's.
mod as_expressions {
    use super::*;
    use axioval_engine::CapabilityEvaluation;
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{
        and, assert_parity, at_most, compare, integer, m, measured, plain, quantity,
        rule as expression, unless_null,
    };
    use serde_json::{Value, json};

    type Systems = Vec<(
        &'static str,
        Result<SourceCoordinateSystem, CoordinateSystemError>,
    )>;

    /// The capability's outcomes for each source, as outcomes for the
    /// source's objects: its one wall.
    fn by_object(evaluation: &CapabilityEvaluation) -> CapabilityEvaluation {
        let wall = |source: &SourceId| axioval_ir::ObjectId::new(source.clone(), "#1").unwrap();
        let mut objects = CapabilityEvaluation::default();
        for finding in evaluation.findings() {
            if let Scope::Source(source) = &finding.scope {
                let mut found = finding.clone();
                found.scope = Scope::Object(wall(source));
                objects.push_finding(found);
            }
        }
        for outcome in evaluation.not_evaluated_outcomes() {
            if let Scope::Source(source) = outcome.scope() {
                objects.push_object_not_evaluated(
                    wall(source),
                    outcome.reason().clone(),
                    outcome.message(),
                );
            }
        }
        objects
    }

    fn rewrite(systems: &dyn Fn() -> Systems, requirement: &Value) -> CapabilityEvaluation {
        let mut model = Model::default();
        for (name, _) in systems() {
            model = model.object_in(name, "#1", "wall");
        }
        model.evaluate_measured(
            &ExpressionRequirement,
            &expression(kind("wall"), requirement),
            |services| {
                let mut snapshots = Vec::new();
                let mut disciplines = Vec::new();
                let mut held = BTreeMap::new();
                for (name, system) in systems() {
                    snapshots.push(
                        SourceSnapshot::try_new(document(name), "r", format!("sha256:{name}"))
                            .unwrap(),
                    );
                    disciplines.push((document(name), Discipline::new(name).unwrap()));
                    held.insert(document(name), system);
                }
                services
                    .register(CoordinateSystemServiceHandle::new(Arc::new(Systems(
                        snapshots, held,
                    ))))
                    .unwrap();
                services
                    .register(SourceDisciplines::new(disciplines))
                    .unwrap();
            },
        )
    }

    /// Every statement within the tolerances; a missing map conversion is
    /// a finding with `required`.
    fn consistent(reference: &str, length: f64, required: bool) -> Value {
        let value = |name: &str| measured(&format!("{name}{reference}"));
        let degrees = || quantity(0.01, "deg");
        let metres = || m(length);
        let maps = and(vec![
            compare("equals", value("map_target_change"), integer(0)),
            at_most(value("coordinate_shift;of=map"), metres()),
            at_most(value("coordinate_turn;of=map"), degrees()),
            at_most(value("map_scale_change"), plain(0.0)),
        ]);
        let mut checks = vec![
            at_most(value("coordinate_shift;of=world"), metres()),
            at_most(value("coordinate_turn;of=world"), degrees()),
        ];
        for (name, bound) in [
            ("coordinate_turn;of=north", degrees()),
            ("coordinate_shift;of=site", metres()),
            ("coordinate_turn;of=site", degrees()),
        ] {
            let read = value(name);
            checks.push(unless_null(&read, at_most(read.clone(), bound)));
        }
        if required {
            checks.insert(
                0,
                compare("equals", value("map_conversion;of=own"), integer(1)),
            );
            checks.push(json!({"kind": "if",
                "branches": [{"when": compare("equals",
                    value("map_conversion;of=reference"), integer(1)), "then": maps}],
                "else": {"kind": "literal", "value": {"type": "boolean", "value": true}}}));
        } else {
            // Last, so a missing map conversion is not recorded only when
            // nothing else is unknown.
            checks.push(maps);
        }
        and(checks)
    }

    fn parity(
        systems: &dyn Fn() -> Systems,
        parameters: Vec<(&str, ParameterValue)>,
        requirement: &Value,
    ) {
        let found = by_object(&evaluate(systems(), parameters));
        assert_parity(ID, &found, &rewrite(systems, requirement));
    }

    const BY_ARCHITECTURE: &str = ";reference=architecture";

    fn shifted(easting: f64, millimetres: bool) -> Systems {
        vec![
            (
                "architecture",
                Ok(georeferenced("architecture", 500_000.0, false)),
            ),
            (
                "structural",
                Ok(georeferenced("structural", easting, millimetres)),
            ),
        ]
    }

    #[test]
    fn map_offsets_within_the_tolerance_reach_the_verdicts() {
        for (easting, millimetres) in [
            (500_000.0, true),
            (500_001.0, false),
            (500_000.000_5, false),
        ] {
            let systems = move || shifted(easting, millimetres);
            parity(
                &systems,
                by_architecture(),
                &consistent(BY_ARCHITECTURE, 0.001, false),
            );
            parity(
                &systems,
                vec![
                    ("reference", string("architecture")),
                    ("length_tolerance", number(1.5)),
                ],
                &consistent(BY_ARCHITECTURE, 1.5, false),
            );
        }
    }

    #[test]
    fn a_default_map_unit_reaches_the_verdicts() {
        for easting in [500_000.0, 500_001.0] {
            let systems = move || -> Systems {
                let map = MapConversion::try_new(
                    Some("EPSG:25832".into()),
                    [easting * 1000.0, 5_600_000_000.0, 50_000.0],
                    [1.0, 0.0],
                    1.0,
                    Some(0.001),
                )
                .unwrap()
                .with_map_unit_by_default();
                vec![
                    (
                        "architecture",
                        Ok(georeferenced("architecture", 500_000.0, false)),
                    ),
                    ("structural", Ok(system("structural", Some(map)))),
                ]
            };
            parity(
                &systems,
                by_architecture(),
                &consistent(BY_ARCHITECTURE, 0.001, false),
            );
        }
    }

    #[test]
    fn a_moved_site_against_the_first_source_reaches_the_verdicts() {
        let systems = || -> Systems {
            vec![
                (
                    "a-architecture",
                    Ok(georeferenced("a-architecture", 500_000.0, false)),
                ),
                (
                    "b-structural",
                    Ok(georeferenced("b-structural", 500_000.0, false)
                        .with_site(SitePlacement::Stated(frame([0.0, 2.0, 0.0])))),
                ),
            ]
        };
        parity(&systems, Vec::new(), &consistent("", 0.001, false));
    }

    #[test]
    fn a_missing_map_conversion_reaches_the_verdicts() {
        let systems = || -> Systems {
            vec![
                (
                    "architecture",
                    Ok(georeferenced("architecture", 500_000.0, false)),
                ),
                ("structural", Ok(system("structural", None))),
            ]
        };
        parity(
            &systems,
            by_architecture(),
            &consistent(BY_ARCHITECTURE, 0.001, false),
        );
        parity(
            &systems,
            vec![
                ("reference", string("architecture")),
                ("require_map_conversion", boolean(true)),
            ],
            &consistent(BY_ARCHITECTURE, 0.001, true),
        );
        // The reference itself without one is its own finding.
        let reversed = || -> Systems {
            vec![
                ("architecture", Ok(system("architecture", None))),
                (
                    "structural",
                    Ok(georeferenced("structural", 500_000.0, false)),
                ),
            ]
        };
        parity(
            &reversed,
            vec![
                ("reference", string("architecture")),
                ("require_map_conversion", boolean(true)),
            ],
            &consistent(BY_ARCHITECTURE, 0.001, true),
        );
    }

    #[test]
    fn one_sided_statements_and_unreadable_systems_reach_the_verdicts() {
        let systems = || -> Systems {
            vec![
                (
                    "architecture",
                    Ok(georeferenced("architecture", 500_000.0, false)),
                ),
                (
                    "structural",
                    Ok(georeferenced("structural", 500_000.0, false)
                        .with_site(SitePlacement::Unknown("2 sites".into()))),
                ),
                (
                    "mep",
                    Err(CoordinateSystemError::Ambiguous("2 model contexts".into())),
                ),
            ]
        };
        parity(
            &systems,
            by_architecture(),
            &consistent(BY_ARCHITECTURE, 0.001, false),
        );
        // A site unknown beside a missing map conversion is incomplete, not
        // merely unrecorded.
        let both = || -> Systems {
            vec![
                (
                    "architecture",
                    Ok(georeferenced("architecture", 500_000.0, false)),
                ),
                (
                    "structural",
                    Ok(system("structural", None)
                        .with_site(SitePlacement::Unknown("2 sites".into()))),
                ),
            ]
        };
        parity(
            &both,
            by_architecture(),
            &consistent(BY_ARCHITECTURE, 0.001, false),
        );
    }
}
