//! The `source` selector: objects by what their source states about itself.
#![allow(missing_docs)]

mod common;

use axioval_engine::{SourceMetadata, SourceMetadataIndex};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Quantifier, Selector, SourceField};
use axioval_ir::{NotEvaluatedReason, SourceId};
use axioval_rules::ManualIssue;
use common::{Model, rule, string, unevaluated};

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

/// One object in each of `arch`, `struct`, `both`, `silent` and `unread`.
fn model() -> Model {
    ["arch", "struct", "both", "silent", "unread"]
        .into_iter()
        .fold(Model::default(), |model, document| {
            model.object_in(document, document, "thing")
        })
}

/// `arch` and `struct` written by one application each, `both` by two,
/// `silent` stating none, `unread` never read.
fn index() -> SourceMetadataIndex {
    let written = |applications: &[&str]| {
        SourceMetadata::new().with(SourceField::Application, applications.iter().copied())
    };
    SourceMetadataIndex::new([
        (source("arch"), written(&["Modeller Architecture 2024"])),
        (source("struct"), written(&["Modeller Structure 2024"])),
        (
            source("both"),
            written(&["Modeller Architecture 2024", "Converter"]),
        ),
        (source("silent"), written(&[])),
        (
            source("unread"),
            SourceMetadata::new().with(SourceField::FileName, ["unread.ifc"]),
        ),
    ])
}

fn select(selector: Selector) -> (Vec<String>, Vec<(String, NotEvaluatedReason)>) {
    let evaluation = model().evaluate_with(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            selector,
            vec![("title", string("selected"))],
        ),
        |services| services.register(index()).unwrap(),
    );
    let mut chosen: Vec<String> = evaluation
        .findings()
        .iter()
        .flat_map(|finding| finding.object_id().into_iter().chain(&finding.related))
        .map(|id| id.local_id.clone())
        .collect();
    chosen.sort();
    (chosen, unevaluated(&evaluation))
}

fn application(
    operator: ComparisonOperator,
    value: Option<ParameterValue>,
    quantifier: Option<Quantifier>,
) -> Selector {
    Selector::Source {
        field: SourceField::Application,
        operator,
        value,
        case_sensitive: true,
        trim: false,
        quantifier,
    }
}

#[test]
fn application_like_selects_the_objects_of_models_a_matching_application_wrote() {
    let (chosen, undecided) = select(application(
        ComparisonOperator::Like,
        Some(string("*Architecture*")),
        Some(Quantifier::Any),
    ));
    assert_eq!(chosen, ["arch", "both"]);
    // Never read: unknown, never a non-match.
    assert_eq!(
        undecided,
        [("unread".to_owned(), NotEvaluatedReason::NotRecorded)]
    );
}

#[test]
fn several_applications_need_a_quantifier() {
    let (chosen, undecided) = select(application(
        ComparisonOperator::Like,
        Some(string("*Architecture*")),
        None,
    ));
    assert_eq!(chosen, ["arch"]);
    assert_eq!(
        undecided,
        [
            ("both".to_owned(), NotEvaluatedReason::InvalidEvidence),
            ("unread".to_owned(), NotEvaluatedReason::NotRecorded),
        ]
    );
    let (chosen, _) = select(application(
        ComparisonOperator::Like,
        Some(string("*Architecture*")),
        Some(Quantifier::All),
    ));
    assert_eq!(chosen, ["arch"]);
}

#[test]
fn a_source_stating_no_application_matches_nothing() {
    let (chosen, _) = select(Selector::Not {
        operand: Box::new(application(ComparisonOperator::Exists, None, None)),
    });
    assert_eq!(chosen, ["silent"]);
    let (chosen, _) = select(application(ComparisonOperator::IsEmpty, None, None));
    assert!(chosen.is_empty(), "{chosen:?}");
}

#[test]
fn other_fields_compare_alike() {
    let (chosen, undecided) = select(Selector::Source {
        field: SourceField::FileName,
        operator: ComparisonOperator::Equals,
        value: Some(string("UNREAD.IFC")),
        case_sensitive: false,
        trim: false,
        quantifier: None,
    });
    assert_eq!(chosen, ["unread"]);
    assert_eq!(undecided.len(), 4, "{undecided:?}");
    assert!(
        undecided
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::NotRecorded)
    );
}

#[test]
fn a_comparison_that_does_not_fit_its_operator_is_an_invalid_declaration() {
    let (chosen, undecided) = select(application(
        ComparisonOperator::Like,
        Some(ParameterValue::Integer { value: 1 }),
        None,
    ));
    assert!(chosen.is_empty());
    assert!(
        undecided
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration),
        "{undecided:?}"
    );
}

#[test]
fn outside_a_session_source_metadata_is_unavailable() {
    let evaluation = model().evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            application(ComparisonOperator::Exists, None, None),
            vec![("title", string("selected"))],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::MissingService)
    );
}
