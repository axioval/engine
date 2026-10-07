//! Grouping clash findings: pairs sharing a key become one finding that
//! relates every object involved and carries each pair's evidence.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, GeometryFidelity, LengthInterval,
    NotEvaluatedReason, ObjectBounds, OverlapExtents, ProximityError, ProximityEvidence,
    ProximityRequest, ProximityService, ProximityServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, Scope};
use axioval_rules::{Clash, ClashMatrix};

use common::{
    Model, boolean, id, kind, number, property, rule, selector, source, string, unevaluated,
};

/// Unit boxes along x; every declared pair overlaps 0.05 m deep, its
/// intersection reaching `(x, y, z)` metres.
#[derive(Default)]
struct Stub {
    boxes: BTreeMap<String, f64>,
    pairs: BTreeMap<(String, String), [f64; 3]>,
}

impl Stub {
    fn object(mut self, local: &str, x: f64) -> Self {
        self.boxes.insert(local.into(), x);
        self
    }
    fn overlap(mut self, a: &str, b: &str, extents: [f64; 3]) -> Self {
        self.pairs.insert((a.into(), b.into()), extents);
        self
    }
}

impl ProximityService for Stub {
    fn bounds(&self, object: &axioval_ir::ObjectId) -> Result<ObjectBounds, ProximityError> {
        let x = *self
            .boxes
            .get(&object.local_id)
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let extents = *self
            .pairs
            .get(&(a.clone(), b.clone()))
            .or_else(|| self.pairs.get(&(b.clone(), a.clone())))
            .unwrap_or_else(|| panic!("{a}/{b} should not have been measured"));
        let [along_x, along_y, along_z] =
            extents.map(|extent| LengthInterval::exact(extent).unwrap());
        ProximityEvidence::try_new(
            request.clone(),
            0.0,
            Some(0.05),
            Some(0.0),
            None,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("proximity:{a}:{b}")),
        )?
        .with_hausdorff(LengthInterval::try_new(1.0, 1.0).unwrap())?
        .with_overlap_extents(OverlapExtents::new(along_x, along_y, along_z))
    }
}

const THROUGH: [f64; 3] = [0.2, 0.5, 0.3];
const WALLS: [&str; 5] = ["wall-1", "wall-2", "wall-3", "wall-4", "wall-5"];

/// A duct on storey `s1` through five identical walls; `upper` of them
/// stand on storey `s2` instead.
fn storeys(upper: usize) -> Model {
    let mut model = Model::default()
        .object("s1", "storey")
        .object("s2", "storey")
        .object("duct", "duct")
        .edge("contains", "s1", "duct");
    for (index, wall) in WALLS.iter().enumerate() {
        let storey = if index < WALLS.len() - upper {
            "s1"
        } else {
            "s2"
        };
        model = model.object(wall, "wall").edge("contains", storey, wall);
    }
    model
}

fn through_walls() -> Stub {
    let mut stub = Stub::default().object("duct", 0.0);
    for (index, wall) in WALLS.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = 0.1 * (index + 1) as f64;
        stub = stub.object(wall, x).overlap("duct", wall, THROUGH);
    }
    stub
}

fn clash(extra: Vec<(&str, ParameterValue)>) -> CompiledRule {
    let mut parameters = vec![
        ("counterparts", selector(kind("wall"))),
        ("penetration_tolerance_metres", number(0.0)),
    ];
    parameters.extend(extra);
    rule("axioval:capability.clash", kind("duct"), parameters)
}

fn similar_per_storey() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("group_by", string("similar")),
        ("group_tolerance_metres", number(0.01)),
        ("per_storey", boolean(true)),
        ("storey_path", string("contains:backward")),
    ]
}

fn run(
    capability: &dyn axioval_engine::RuleCapability,
    model: Model,
    stub: Stub,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let stub = Arc::new(stub);
    common::clash_held(model, capability, rule, |services| {
        services
            .register(ProximityServiceHandle::new(stub.clone()))
            .unwrap();
    })
}

fn locals(ids: &[axioval_ir::ObjectId]) -> Vec<&str> {
    ids.iter().map(|id| id.local_id.as_str()).collect()
}

#[test]
fn a_duct_through_five_identical_walls_on_one_storey_is_one_issue() {
    let evaluation = run(
        &Clash,
        storeys(0),
        through_walls(),
        &clash(similar_per_storey()),
    );
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    let [finding] = evaluation.findings() else {
        panic!("one finding expected: {:?}", evaluation.findings());
    };
    assert_eq!(finding.scope, Scope::Object(id("duct")));
    assert_eq!(locals(&finding.related), WALLS);
    assert_eq!(finding.evidence.len(), 5, "{:?}", finding.evidence);
    assert!(
        finding.message.starts_with(
            "5 similar intersection clashes of duct with wall on test:model/s1: \
             [test:model/duct] hard clash with test:model/wall-1:"
        ),
        "{}",
        finding.message
    );
}

#[test]
fn walls_on_two_storeys_are_two_issues() {
    let evaluation = run(
        &Clash,
        storeys(2),
        through_walls(),
        &clash(similar_per_storey()),
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let mut groups: Vec<Vec<&str>> = evaluation
        .findings()
        .iter()
        .map(|finding| locals(&finding.related))
        .collect();
    groups.sort();
    assert_eq!(
        groups,
        vec![vec!["wall-1", "wall-2", "wall-3"], vec!["wall-4", "wall-5"]]
    );

    // Without `per_storey` they are one.
    let mut extra = similar_per_storey();
    extra.truncate(2);
    let evaluation = run(&Clash, storeys(2), through_walls(), &clash(extra));
    assert_eq!(evaluation.findings().len(), 1);
}

#[test]
fn without_grouping_each_pair_is_its_own_finding() {
    let evaluation = run(&Clash, storeys(2), through_walls(), &clash(vec![]));
    assert_eq!(evaluation.findings().len(), 5);
    for finding in evaluation.findings() {
        assert_eq!(finding.scope, Scope::Object(id("duct")));
        assert_eq!(finding.related.len(), 1);
        assert_eq!(finding.evidence.len(), 1);
        assert!(finding.message.starts_with("hard clash with"));
    }
}

/// A thicker wall is not similar; grouping by type pair or by subject still
/// takes it in.
#[test]
fn similar_pairs_share_their_rounded_extents() {
    let stub = through_walls().overlap("duct", "wall-5", [0.3, 0.5, 0.3]);
    let similar = vec![
        ("group_by", string("similar")),
        ("group_tolerance_metres", number(0.01)),
    ];
    let evaluation = run(&Clash, storeys(0), stub, &clash(similar));
    let mut sizes: Vec<usize> = evaluation
        .findings()
        .iter()
        .map(|finding| finding.related.len())
        .collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![1, 4]);

    // Plan extents compare regardless of their axis.
    let stub = through_walls().overlap("duct", "wall-5", [0.5, 0.2, 0.3]);
    let similar = vec![
        ("group_by", string("similar")),
        ("group_tolerance_metres", number(0.01)),
    ];
    let evaluation = run(&Clash, storeys(0), stub, &clash(similar));
    assert_eq!(evaluation.findings().len(), 1);

    for by in ["type_pair", "subject"] {
        let stub = through_walls().overlap("duct", "wall-5", [0.3, 0.5, 0.3]);
        let evaluation = run(
            &Clash,
            storeys(0),
            stub,
            &clash(vec![("group_by", string(by))]),
        );
        let [finding] = evaluation.findings() else {
            panic!("{by}: one finding expected: {:?}", evaluation.findings());
        };
        assert_eq!(finding.related.len(), 5);
    }
}

/// Values of `group_property` split similar groups.
#[test]
fn a_group_property_splits_similar_pairs() {
    let model = storeys(0).text("wall-1", "Pset", "FireRating", "F90").text(
        "wall-2",
        "Pset",
        "FireRating",
        "F90",
    );
    let rule = clash(vec![
        ("group_by", string("similar")),
        ("group_tolerance_metres", number(0.01)),
        ("group_property", property(Some("Pset"), "FireRating")),
    ]);
    let evaluation = run(&Clash, model, through_walls(), &rule);
    let mut sizes: Vec<usize> = evaluation
        .findings()
        .iter()
        .map(|finding| finding.related.len())
        .collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![2, 3]);
}

/// A pair whose storey cannot be read is reported on its own and says why;
/// grouping never hides or merges it.
#[test]
fn a_pair_of_unknown_storey_is_reported_on_its_own() {
    let mut rule = clash(similar_per_storey());
    rule.parameters.insert(
        "storey_path".into(),
        string("contains:backward unknown:backward"),
    );
    let evaluation = run(&Clash, storeys(0), through_walls(), &rule);
    assert_eq!(evaluation.findings().len(), 5);
    assert!(
        evaluation
            .findings()
            .iter()
            .all(|finding| finding.message.contains("(not grouped: the storey of")),
        "{:?}",
        evaluation.findings()
    );
}

#[test]
fn invalid_groupings_refuse_the_rule() {
    for extra in [
        vec![("group_by", string("storey"))],
        vec![("group_by", string("similar"))],
        vec![
            ("group_by", string("subject")),
            ("group_tolerance_metres", number(0.01)),
        ],
        vec![
            ("group_by", string("subject")),
            ("per_storey", boolean(true)),
        ],
        vec![("storey_path", string("contains:backward"))],
        vec![
            ("group_by", string("type_pair")),
            ("group_property", property(None, "Name")),
        ],
    ] {
        let evaluation = run(&Clash, storeys(0), through_walls(), &clash(extra));
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            vec![("duct".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

fn cell(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect()
}

/// A clash matrix never groups pairs its cells judge differently.
#[test]
fn a_matrix_groups_within_one_cell() {
    let model = storeys(0)
        .text("wall-1", "Pset", "Kind", "masonry")
        .text("wall-2", "Pset", "Kind", "masonry");
    let rule = rule(
        "axioval:capability.clash-matrix",
        kind("duct"),
        vec![
            ("counterparts", selector(kind("wall"))),
            (
                "cells",
                ParameterValue::Table {
                    value: vec![
                        cell(&[("penetration_tolerance_metres", number(0.0))]),
                        cell(&[
                            ("counterpart_key_1", string("masonry")),
                            ("penetration_tolerance_metres", number(0.0)),
                            ("severity", string("warning")),
                        ]),
                    ],
                },
            ),
            ("key_1", property(Some("Pset"), "Kind")),
            ("symmetric", boolean(false)),
            ("exclude_same_system", boolean(false)),
            ("group_by", string("type_pair")),
        ],
    );
    let evaluation = run(&ClashMatrix, model, through_walls(), &rule);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let mut groups: Vec<(Vec<&str>, axioval_ir::Severity)> = evaluation
        .findings()
        .iter()
        .map(|finding| (locals(&finding.related), finding.severity.clone()))
        .collect();
    groups.sort();
    assert_eq!(
        groups,
        vec![
            (vec!["wall-1", "wall-2"], axioval_ir::Severity::Warning),
            (
                vec!["wall-3", "wall-4", "wall-5"],
                axioval_ir::Severity::Error
            ),
        ]
    );
}
