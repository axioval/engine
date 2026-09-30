//! Pushing findings to, and pulling decisions from, a BCF API 3.0 server:
//! an in-process test server, never a real one.
#![allow(missing_docs, clippy::doc_markdown)]

mod support {
    pub mod mock;
}

use std::cell::RefCell;
use std::collections::BTreeMap;

use axioval_bcf::{IFC_GLOBAL_ID_SCHEME, Options, import_topics};
use axioval_bcf_api::{ApiError, Auth, Client};
use axioval_ir::{
    Decision, DecisionStatus, Decisions, Evidence, ExternalId, Finding, NotEvaluated,
    NotEvaluatedReason, Object, ObjectId, Project, Report, RuleId, Scope, Severity, SourceId,
};
use support::mock::{self, Mock};

fn id(local: u64) -> ObjectId {
    ObjectId::new(
        SourceId::new("ifc-step", "a.ifc").unwrap(),
        format!("#{local}"),
    )
    .unwrap()
}

fn model() -> Project {
    let alias = |value: &str| ExternalId::new(IFC_GLOBAL_ID_SCHEME, value).unwrap();
    Project::new(vec![
        Object::new(id(1), "IFCWALL").with_external_id(alias("2O2Fr$t4X7Zf8NOew3FLOH")),
        Object::new(id(2), "IFCSLAB").with_external_id(alias("0000000000000000000001")),
        Object::new(id(3), "IFCDOOR").with_external_id(alias("0000000000000000000003")),
    ])
    .unwrap()
}

/// A wall finding, a door finding and a rule that could not run.
fn report() -> Report {
    let source = SourceId::new("ifc-step", "a.ifc").unwrap();
    let finding = |rule: &str, object, severity, message: &str| {
        Finding::new(RuleId::new(rule).unwrap(), id(object), severity, message)
            .with_evidence([Evidence::exact(source.clone(), "e")])
    };
    let mut report = Report {
        findings: vec![
            finding(
                "slab-contact",
                1,
                Severity::Error,
                "Wall has insufficient contact with the slab below",
            )
            .with_related([id(2)]),
            finding(
                "door-fire-rating",
                3,
                Severity::Warning,
                "FireRating is missing",
            ),
        ],
        not_evaluated: vec![NotEvaluated {
            rule_id: RuleId::new("stair-headroom").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry service is registered".into(),
            location: None,
        }],
        ..Report::default()
    };
    report
        .identify_findings(&model(), IFC_GLOBAL_ID_SCHEME)
        .unwrap();
    report
}

fn options() -> Options {
    Options::new("axioval", "2026-09-30T09:00:00Z")
}

fn credentials() -> Auth {
    Auth::ClientCredentials {
        client_id: mock::CLIENT_ID.into(),
        client_secret: mock::CLIENT_SECRET.into(),
        token_url: None,
    }
}

#[test]
fn pushing_a_run_creates_its_topics_and_pushing_again_updates_them() {
    let server = Mock::start();
    let client = Client::connect(&server.url, &credentials()).unwrap();
    assert_eq!(client.projects().unwrap()[0].project_id, mock::PROJECT);
    let report = report();

    let pushed = client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    assert_eq!((pushed.created, pushed.updated), (3, 0));
    // The wall and the door each have one viewpoint; the project-level
    // outcome has none.
    assert_eq!(pushed.viewpoints, 2);
    assert_eq!(server.topic_count(), 3);
    let wall = report.findings[0].id.unwrap().to_string();
    let topic = server.topic(&wall);
    assert_eq!(
        topic["title"],
        "Wall has insufficient contact with the slab below"
    );
    assert_eq!(topic["topic_status"], "Open");
    assert_eq!(topic["priority"], "High");

    let again = client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    assert_eq!((again.created, again.updated), (0, 3));
    assert_eq!((again.viewpoints, again.comments), (0, 0));
    assert_eq!(server.topic_count(), 3);
    assert_eq!(server.viewpoint_count(), 2);
}

#[test]
fn a_status_changed_on_the_server_is_pulled_back_as_a_decision() {
    let server = Mock::start();
    let client = Client::connect(&server.url, &credentials()).unwrap();
    let report = report();
    client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    let wall = report.findings[0].id.unwrap();
    server.set_status(&wall.to_string(), "Closed");

    let markups = client.pull(mock::PROJECT).unwrap();
    assert_eq!(markups.len(), 3);
    let imported = import_topics(&markups, &report, &model(), &BTreeMap::new()).unwrap();
    let decision = imported.decisions.get(wall).unwrap();
    assert_eq!(decision.status, DecisionStatus::Accepted);
    assert_eq!(decision.author, "C. Reviewer");
    assert_eq!(decision.date.to_string(), "2026-09-30T12:00:00Z");
    assert_eq!(imported.decisions.decisions().len(), 1);
    assert_eq!(imported.unmatched.len(), 1, "the not-evaluated outcome");

    // Pushing the undecided report again keeps the reviewer's status.
    client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    assert_eq!(server.topic(&wall.to_string())["topic_status"], "Closed");
}

#[test]
fn a_decision_with_assignee_due_date_and_comments_round_trips_through_the_server() {
    let server = Mock::start();
    let client = Client::connect(&server.url, &credentials()).unwrap();
    let mut report = report();
    let wall = report.findings[0].id.unwrap();
    let decisions = Decisions::new([Decision::new(
        wall,
        DecisionStatus::Rejected,
        "A. Reviewer",
        "2026-09-27T08:00:00Z".parse().unwrap(),
    )
    .unwrap()
    .with_comment("a lining")
    .with_assignee("C. Engineer")
    .with_due_date("2026-10-15T17:00:00+02:00".parse().unwrap())])
    .unwrap();
    report.apply_decisions(&decisions).unwrap();
    let pushed = client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    assert_eq!(pushed.comments, 1);
    let topic = server.topic(&wall.to_string());
    assert_eq!(topic["topic_status"], "Rejected");
    assert_eq!(topic["assigned_to"], "C. Engineer");
    assert_eq!(topic["due_date"], "2026-10-15T17:00:00+02:00");
    // The server attributes the comment to the signed-in user.
    assert_eq!(server.comments(&wall.to_string())[0]["author"], mock::USER);

    let markups = client.pull(mock::PROJECT).unwrap();
    let imported = import_topics(&markups, &report, &model(), &BTreeMap::new()).unwrap();
    let back = imported.decisions.get(wall).unwrap();
    assert_eq!(back.status, DecisionStatus::Rejected);
    assert_eq!(back.assigned_to.as_deref(), Some("C. Engineer"));
    assert_eq!(back.due_date, decisions.decisions()[0].due_date);
    assert_eq!(back.comments[0].text, "a lining");

    // Pushing again adds no second copy of the comment.
    let again = client
        .push(mock::PROJECT, &report, &model(), &options())
        .unwrap();
    assert_eq!(again.comments, 0);
    assert_eq!(server.comments(&wall.to_string()).len(), 1);
}

#[test]
fn the_device_flow_polls_until_the_person_signed_in() {
    let server = Mock::start();
    let shown = std::rc::Rc::new(RefCell::new(None));
    let seen = std::rc::Rc::clone(&shown);
    let auth = Auth::Device {
        client_id: mock::CLIENT_ID.into(),
        device_authorization_url: format!("{}/device", server.url),
        token_url: None,
        show: Box::new(move |code| *seen.borrow_mut() = Some(code.user_code.clone())),
    };
    let client = Client::connect(&server.url, &auth).unwrap();
    assert_eq!(shown.borrow().as_deref(), Some("ABCD-EFGH"));
    assert_eq!(server.state.lock().unwrap().device_polls, 2);
    assert!(client.projects().is_ok());
}

#[test]
fn a_bearer_token_signs_in_and_a_missing_one_is_refused() {
    let server = Mock::start();
    let client = Client::connect(&server.url, &Auth::Bearer(mock::TOKEN.into())).unwrap();
    assert!(client.projects().is_ok());
    let anonymous = Client::connect(&server.url, &Auth::None).unwrap();
    assert!(matches!(
        anonymous.projects(),
        Err(ApiError::Status { status: 401, .. })
    ));
    let wrong = Auth::ClientCredentials {
        client_id: mock::CLIENT_ID.into(),
        client_secret: "wrong".into(),
        token_url: None,
    };
    let error = Client::connect(&server.url, &wrong).unwrap_err();
    assert!(
        matches!(&error, ApiError::Auth(why) if why.contains("invalid_client")),
        "{error}"
    );
    // Credentials are never printed.
    assert!(!format!("{wrong:?}").contains("wrong"));
}

#[test]
#[cfg(not(feature = "native-tls"))]
fn an_https_server_is_refused_without_tls() {
    assert!(matches!(
        Client::connect("https://bcf.example.com", &Auth::None),
        Err(ApiError::TlsUnavailable(_))
    ));
}
