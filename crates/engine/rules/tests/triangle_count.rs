//! `triangle-count`: the triangles of each element's mesh against a maximum.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ServiceRegistry, TriangleCount, TriangleCountError, TriangleCountService,
    TriangleCountServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::TriangleCountLimit;
use axioval_rules::parity::{Measure, Observations, Parity};
use common::{Model, findings, id, integer, kind, rule, source, string, unevaluated};

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

/// The capability's evaluation as a run evaluates it: `triangle-count` runs
/// as its template, reading the measured `triangle_count`. Every fixture
/// also holds the template to the implementation it replaced under the
/// whole outside contract (findings word for word, counts, evidence
/// exactness, not-evaluated outcomes and messages), and each object's
/// count as the template reads it against the count the service gave the
/// replaced implementation, exactly.
fn run(counts: Counts, maximum: ParameterValue) -> CapabilityEvaluation {
    let rule = rule(ID, kind("column"), vec![("maximum", maximum)]);
    let counts = Arc::new(counts);
    let register = |services: &mut ServiceRegistry| {
        services
            .register(TriangleCountServiceHandle::new(counts.clone()))
            .unwrap();
    };
    let template = model().evaluate_measured(&TriangleCountLimit, &rule, register);
    let reference = model().evaluate_with(
        &axioval_rules::reference::TriangleCountLimit,
        &rule,
        register,
    );
    let objects: Vec<ObjectId> = model()
        .project()
        .objects()
        .map(|object| object.id.clone())
        .collect();
    let right = model()
        .measure("triangle_count", &objects, register)
        .into_iter()
        .fold(
            Observations::of_evaluation(&template),
            |side, (object, count)| side.with_value(object, "count", count),
        );
    let left = objects
        .iter()
        .fold(Observations::of_evaluation(&reference), |side, object| {
            let count = counts
                .count_triangles(object)
                .map_or(Measure::NotEvaluated, |count| {
                    #[allow(clippy::cast_precision_loss)]
                    let triangles = count.triangles() as f64;
                    Measure::interval(triangles, triangles, "")
                });
            side.with_value(object.clone(), "count", count)
        });
    let parity = Parity::contract()
        .value("count", 0.0)
        .compare((ID, &left), ("template", &right));
    assert!(parity.holds(), "{}", parity.diff());
    assert!(parity.values > 0, "no count was compared");
    template
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
    // The count is cited as the measured value read from the host's count.
    assert!(
        evaluation.findings()[0].evidence[0]
            .locator
            .ends_with("triangles:dense"),
        "{:?}",
        evaluation.findings()[0].evidence
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
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "triangle-count: `maximum` is negative"
    );
    let bound = rule(ID, kind("column"), vec![("maximum", integer(10))]);
    let evaluation = model().evaluate_measured(&TriangleCountLimit, &bound, |_| {});
    let reference = model().evaluate(&axioval_rules::reference::TriangleCountLimit, &bound);
    let parity = Parity::contract().compare(
        (ID, &Observations::of_evaluation(&reference)),
        ("template", &Observations::of_evaluation(&evaluation)),
    );
    assert!(parity.holds(), "{}", parity.diff());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "triangle-count service is not registered"
    );
}

/// A rule without its required `maximum` is refused as the capability
/// refused it.
#[test]
fn a_missing_or_mistyped_maximum_is_refused() {
    let evaluation = run(Counts::default().with("box", 12, true), string("ten"));
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let bound = rule(ID, kind("column"), vec![]);
    let register = |services: &mut ServiceRegistry| {
        services
            .register(TriangleCountServiceHandle::new(Arc::new(Counts::default())))
            .unwrap();
    };
    let evaluation = model().evaluate_measured(&TriangleCountLimit, &bound, register);
    let reference = model().evaluate_with(
        &axioval_rules::reference::TriangleCountLimit,
        &bound,
        register,
    );
    let parity = Parity::contract().compare(
        (ID, &Observations::of_evaluation(&reference)),
        ("template", &Observations::of_evaluation(&evaluation)),
    );
    assert!(parity.holds(), "{}", parity.diff());
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "triangle-count: parameter `maximum` is required"
    );
}

/// The rule forked from the template, an `expression` rule evaluated by
/// the expression capability, reaches the template's verdicts on every
/// fixture. Its findings are worded as an expression rule's, so only
/// outcomes are compared.
#[test]
fn the_forked_rule_reaches_the_templates_verdicts() {
    use axioval_rules::templates::{Fork, fork};
    let fixtures: [fn() -> Counts; 3] = [
        Counts::default,
        || {
            Counts::default()
                .with("box", 12, true)
                .with("round", 480, false)
                .with("dense", 5000, true)
        },
        || {
            Counts::default()
                .with("box", 12, true)
                .with("round", 480, false)
                .with("dense", 50, true)
                .with("broken", 0, true)
        },
    ];
    for counts in fixtures {
        for maximum in [0, 11, 12, 100, 480, 500] {
            let bound = rule(ID, kind("column"), vec![("maximum", integer(maximum))]);
            let forked = fork(&TriangleCountLimit, &bound).unwrap();
            let mut expression_rule = bound.clone();
            expression_rule.capability = Fork::CAPABILITY.into();
            expression_rule.parameters = forked.parameters();
            let template = run(counts(), integer(maximum));
            let forked = model().evaluate_measured(
                &axioval_rules::ExpressionRequirement,
                &expression_rule,
                |services| {
                    services
                        .register(TriangleCountServiceHandle::new(Arc::new(counts())))
                        .unwrap();
                },
            );
            let parity = axioval_rules::parity::compare_evaluations(
                ("template", &template),
                ("fork", &forked),
            );
            assert!(parity.holds(), "{maximum}\n{}", parity.diff());
        }
    }
    let refused = rule(ID, kind("column"), vec![("maximum", integer(-1))]);
    assert_eq!(
        fork(&TriangleCountLimit, &refused).unwrap_err().to_string(),
        "triangle-count: `maximum` is negative"
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

/// Generated meshes: random counts, each the exact shape or a
/// tessellation, or unmeasured, against a random maximum. The template
/// holds the replaced implementation's whole contract on every one.
mod generated {
    use super::*;
    use proptest::prelude::*;

    /// One object's mesh: its count and exactness, or none.
    type Mesh = Option<(u64, bool)>;

    fn counts(meshes: &[Mesh; 4]) -> Counts {
        ["box", "round", "dense", "broken"]
            .into_iter()
            .zip(meshes)
            .fold(Counts::default(), |counts, (local, mesh)| match mesh {
                Some((triangles, exact)) => counts.with(local, *triangles, *exact),
                None => counts,
            })
    }

    fn mesh() -> impl Strategy<Value = Mesh> {
        proptest::option::of((0u64..2000, any::<bool>()))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn generated_meshes_hold_parity(
            meshes in [mesh(), mesh(), mesh(), mesh()],
            maximum in 0i64..2000,
        ) {
            run(counts(&meshes), integer(maximum));
        }
    }
}
