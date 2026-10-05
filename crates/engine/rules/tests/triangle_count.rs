//! `triangle-count`: the triangles of each element's mesh against a maximum.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, TriangleCount, TriangleCountError, TriangleCountService,
    TriangleCountServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::TriangleCountLimit;
use common::{Model, findings, id, integer, kind, rule, source, unevaluated};

const ID: &str = "axioval:capability.triangle-count";

/// Per object: the count and whether the mesh is the exact shape; absent
/// objects are unmeasured.
#[derive(Default)]
struct Counts(BTreeMap<ObjectId, (u64, bool)>);

impl Counts {
    fn with(mut self, local: &str, triangles: u64, exact: bool) -> Self {
        self.0.insert(id(local), (triangles, exact));
        self
    }
}

impl TriangleCountService for Counts {
    fn count_triangles(&self, object: &ObjectId) -> Result<TriangleCount, TriangleCountError> {
        let (triangles, exact) = *self
            .0
            .get(object)
            .ok_or_else(|| TriangleCountError::Unavailable("not meshed".into()))?;
        let mut evidence = Evidence::exact(source(), format!("triangles:{}", object.local_id));
        evidence.exact = exact;
        TriangleCount::try_new(object.clone(), triangles, evidence)
    }
}

fn model() -> Model {
    Model::default()
        .object("box", "column")
        .object("round", "column")
        .object("dense", "column")
        .object("broken", "column")
}

fn run(counts: Counts, maximum: ParameterValue) -> CapabilityEvaluation {
    model().evaluate_with(
        &TriangleCountLimit,
        &rule(ID, kind("column"), vec![("maximum", maximum)]),
        |services| {
            services
                .register(TriangleCountServiceHandle::new(Arc::new(counts)))
                .unwrap();
        },
    )
}

#[test]
fn a_mesh_with_more_triangles_than_allowed_is_found() {
    let evaluation = run(
        Counts::default()
            .with("box", 12, true)
            .with("round", 480, false)
            .with("dense", 5000, true),
        integer(500),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "dense".into(),
            "mesh has 5000 triangles; at most 500 allowed".into()
        )]
    );
    assert_eq!(
        evaluation.findings()[0].evidence[0].locator,
        "triangles:dense"
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("broken".into(), NotEvaluatedReason::BackendUnavailable)]
    );
}

/// The measured count of a tessellation's triangles is never exact, a
/// point though it is; the count of a mesh that is the exact shape is.
#[test]
fn a_tessellated_count_is_measured_inexactly() {
    let (project, mut services) = model().services();
    services
        .register(TriangleCountServiceHandle::new(Arc::new(
            Counts::default()
                .with("box", 12, true)
                .with("round", 480, false),
        )))
        .unwrap();
    let read = |local: &str| {
        common::measured_cited(&services, &project, &id(local), "triangle_count").unwrap()
    };
    assert_eq!(read("round"), Some(((480.0, 480.0), false)));
    assert_eq!(read("box"), Some(((12.0, 12.0), true)));
}

/// A tessellation's count follows the host's chord budget, and says so.
#[test]
fn a_tessellated_count_says_it_depends_on_the_tessellation() {
    let evaluation = run(
        Counts::default()
            .with("box", 12, true)
            .with("round", 480, false)
            .with("dense", 50, true)
            .with("broken", 0, true),
        integer(100),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "round".into(),
            "mesh has 480 triangles; at most 100 allowed; the mesh tessellates curved faces, \
             so the count depends on the host's tessellation"
                .into()
        )]
    );
    assert!(!evaluation.findings()[0].evidence[0].exact);
}

#[test]
fn a_negative_maximum_or_no_service_judges_nothing() {
    let evaluation = run(Counts::default(), integer(-1));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let evaluation = model().evaluate(
        &TriangleCountLimit,
        &rule(ID, kind("column"), vec![("maximum", integer(10))]),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
    );
}

/// `triangle_count` at most the maximum reaches the capability's verdicts,
/// a tessellation's count cited as approximate evidence as the capability
/// cites it.
mod as_expressions {
    use super::*;
    use common::expressions::{
        assert_parity, at_most, integer as count, measured, rule as expression,
    };

    fn rewrite(counts: fn() -> Counts, maximum: i64) -> CapabilityEvaluation {
        model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &expression(
                kind("column"),
                &at_most(measured("triangle_count"), count(maximum)),
            ),
            |services| {
                services
                    .register(TriangleCountServiceHandle::new(Arc::new(counts())))
                    .unwrap();
            },
        )
    }

    fn mixed() -> Counts {
        Counts::default()
            .with("box", 12, true)
            .with("round", 480, false)
            .with("dense", 5000, true)
    }

    fn all_measured() -> Counts {
        Counts::default()
            .with("box", 12, true)
            .with("round", 480, false)
            .with("dense", 50, true)
            .with("broken", 0, true)
    }

    #[test]
    fn the_count_against_the_maximum_reaches_the_verdicts() {
        for (counts, maximum) in [
            (mixed as fn() -> Counts, 500),
            (all_measured, 100),
            (all_measured, 480),
            (mixed, 11),
        ] {
            let capability = run(counts(), integer(maximum));
            assert_parity(ID, &capability, &rewrite(counts, maximum));
        }
    }
}
