//! `related` selectors: objects selected by the objects a path reaches.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, RelatedQuantifier, Selector};
use axioval_rules::ManualIssue;
use common::{Model, kind, rule, string, unevaluated};

type Outcome = (Vec<String>, Vec<(String, NotEvaluatedReason)>);

/// `(selected, not evaluated)` for a selector over the model.
fn select(model: Model, selector: Selector) -> Outcome {
    let evaluation = model.evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            selector,
            vec![("title", string("selected"))],
        ),
    );
    let mut chosen: Vec<String> = evaluation
        .findings()
        .iter()
        .flat_map(|finding| finding.object_id().into_iter().chain(&finding.related))
        .map(|id| id.local_id.clone())
        .collect();
    chosen.sort();
    chosen.dedup();
    (chosen, unevaluated(&evaluation))
}

fn outcome(selected: &[&str], undecided: &[(&str, NotEvaluatedReason)]) -> Outcome {
    (
        selected.iter().map(|name| (*name).to_owned()).collect(),
        undecided
            .iter()
            .map(|(name, reason)| ((*name).to_owned(), reason.clone()))
            .collect(),
    )
}

fn related(path: &[&str], quantifier: RelatedQuantifier, selector: Selector) -> Selector {
    Selector::Related {
        path: path.iter().map(|step| (*step).to_owned()).collect(),
        quantifier,
        selector: Box::new(selector),
    }
}

fn walls_whose_doors(quantifier: RelatedQuantifier) -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("wall"),
            related(
                &["voids", "fills:forward"],
                quantifier,
                Selector::property(
                    Some("Pset".into()),
                    "Rating",
                    ComparisonOperator::Like,
                    Some(string("EI*")),
                ),
            ),
        ],
    }
}

/// Wall `w1` holds a rated and a plain door, `w2` only a rated one, `w3`
/// none. `w4`'s one door cannot be read; `w5` holds a rated door and an
/// unreadable one.
fn walls() -> Model {
    let mut model = Model::default();
    for (wall, doors) in [
        ("w1", &["d1", "d2"][..]),
        ("w2", &["d3"]),
        ("w3", &[]),
        ("w4", &["d4"]),
        ("w5", &["d5", "d6"]),
    ] {
        model = model.object(wall, "wall");
        for door in doors {
            let opening = format!("o{door}");
            model = model
                .object(&opening, "opening")
                .object(door, "door")
                .edge("voids", wall, &opening)
                .edge("fills", &opening, door);
        }
    }
    model
        .text("d1", "Pset", "Rating", "EI30")
        .text("d2", "Pset", "Rating", "none")
        .text("d3", "Pset", "Rating", "EI60")
        .unreadable("d4")
        .text("d5", "Pset", "Rating", "EI90")
        .unreadable("d6")
}

const UNAVAILABLE: NotEvaluatedReason = NotEvaluatedReason::BackendUnavailable;

#[test]
fn any_selects_an_object_with_one_matching_relative() {
    assert_eq!(
        select(walls(), walls_whose_doors(RelatedQuantifier::Any)),
        // `w5`'s rated door settles it despite the unreadable one.
        outcome(&["w1", "w2", "w5"], &[("w4", UNAVAILABLE)])
    );
}

#[test]
fn all_needs_every_relative_and_at_least_one() {
    assert_eq!(
        select(walls(), walls_whose_doors(RelatedQuantifier::All)),
        // `w3` reaches no door, so `all` does not hold vacuously.
        outcome(&["w2"], &[("w4", UNAVAILABLE), ("w5", UNAVAILABLE)])
    );
}

#[test]
fn none_selects_an_object_without_a_matching_relative() {
    assert_eq!(
        select(walls(), walls_whose_doors(RelatedQuantifier::None)),
        outcome(&["w3"], &[("w4", UNAVAILABLE)])
    );
}

#[test]
fn a_backward_path_selects_doors_by_their_wall() {
    let model = walls()
        .value(
            "w1",
            "Pset",
            "Compartmentation",
            PropertyValue::Boolean(true),
        )
        .value(
            "w2",
            "Pset",
            "Compartmentation",
            PropertyValue::Boolean(false),
        );
    let selector = Selector::AllOf {
        operands: vec![
            kind("door"),
            related(
                &["fills:backward", "voids:backward"],
                RelatedQuantifier::Any,
                Selector::property(
                    Some("Pset".into()),
                    "Compartmentation",
                    ComparisonOperator::Equals,
                    Some(ParameterValue::Boolean { value: true }),
                ),
            ),
        ],
    };
    // `w4` and `w5` state no compartmentation, exactly.
    assert_eq!(select(model, selector), outcome(&["d1", "d2"], &[]));
}

#[test]
fn a_refused_relationship_leaves_the_object_undecided() {
    let selector = Selector::AllOf {
        operands: vec![
            kind("wall"),
            related(&["voids", "hosts"], RelatedQuantifier::None, kind("door")),
        ],
    };
    // Every wall but `w3` reaches an opening, whose `hosts` answer is
    // refused; `w3` reaches nothing, so the second step is never asked.
    assert_eq!(
        select(walls(), selector),
        outcome(
            &["w3"],
            &[
                ("w1", UNAVAILABLE),
                ("w2", UNAVAILABLE),
                ("w4", UNAVAILABLE),
                ("w5", UNAVAILABLE),
            ]
        )
    );
}

#[test]
fn a_chained_step_reaches_every_whole_above() {
    // `b1` is part of `a1`, which is part of `s1`; `b2` is part of nothing.
    let model = || {
        Model::default()
            .object("s1", "site")
            .object("a1", "assembly")
            .object("b1", "beam")
            .object("b2", "beam")
            .edge("aggregates", "s1", "a1")
            .edge("aggregates", "a1", "b1")
    };
    let beams_in = |step: &str| Selector::AllOf {
        operands: vec![
            kind("beam"),
            related(&[step], RelatedQuantifier::Any, kind("site")),
        ],
    };
    assert_eq!(
        select(model(), beams_in("aggregates:backward")),
        outcome(&[], &[])
    );
    assert_eq!(
        select(model(), beams_in("aggregates:backward+")),
        outcome(&["b1"], &[])
    );
    // The anchor is never its own relative, even around a chain.
    let sites = Selector::AllOf {
        operands: vec![
            kind("site"),
            related(
                &["aggregates:either+"],
                RelatedQuantifier::Any,
                kind("site"),
            ),
        ],
    };
    assert_eq!(select(model(), sites), outcome(&[], &[]));
}

#[test]
fn a_malformed_path_is_an_invalid_declaration() {
    for path in [&[][..], &["voids:sideways"]] {
        let selector = related(path, RelatedQuantifier::Any, kind("door"));
        let (chosen, undecided) = select(Model::default().object("w1", "wall"), selector);
        assert!(chosen.is_empty());
        assert_eq!(
            undecided,
            [("w1".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
