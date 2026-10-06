//! Template features judging sources, held to what their documentation
//! says on small templates of their own: values whose subject is a source
//! or the project, values read once per rule, a judgement joining the
//! words of a list's items, every source judged without selecting, the
//! sources holding a selected object judged before each object, checks
//! passing where a condition holds and checks leaving a refusal to
//! another, refusals after selecting, and list options judged as stated.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::template::{
    Applies, Check, Condition, Decision, Form, FormCheck, Joined, Needed, Once, Operand,
    ParameterDefault, Refusals, ScopeMessages, ScopeSources, Scopes, Template, TemplateValue, Term,
    Text,
};
use axioval_engine::{
    CapabilityEvaluation, CoordinateFrame, CoordinateSystemError, CoordinateSystemService,
    CoordinateSystemServiceHandle, EnvelopeMembershipError, EnvelopeMembershipEvidence,
    EnvelopeMembershipRequest, EnvelopeMembershipService, EnvelopeMembershipServiceHandle,
    MapConversion, MetricDirection, ParameterDescriptor, ParameterType, ServiceRegistry,
    SessionSources, SitePlacement, SourceCoordinateSystem, SourceSnapshot,
};
use axioval_ir::contract::{Expression, ParameterValue, ScalarValue};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Scope, SourceId};
use axioval_rules::templates::{ForkError, Templated, fork};
use common::{Model, kind, rule, selector, strings};

fn document(name: &str) -> SourceId {
    SourceId::new("test", name).unwrap()
}

fn measured(name: &'static str, property: &'static str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: property.to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

fn messages() -> ScopeMessages {
    ScopeMessages {
        source: "in `{source}`",
        project: "in the project",
        no_source: "nowhere to judge",
        no_discipline: "no source plays {disciplines}",
        undeclared: "`{source}` declares no discipline",
        undeclared_member: "`{source}` declares no discipline",
        unlisted: "unlisted: {why}",
        no_disciplines: "no disciplines",
    }
}

/// Every source judged against a reference read once per rule, by the
/// words of the differences its coordinate system shows.
fn federation() -> Template {
    Template {
        id: "test:federation",
        parameters: vec![ParameterDescriptor::optional(
            "length",
            ParameterType::Number,
        )],
        grades: false,
        name: "federation",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "length",
            value: ScalarValue::Number { value: 0.001 },
            from: &[],
        }],
        declaration: Vec::new(),
        services: None,
        texts: vec![
            Text {
                name: "lead",
                when: Some(Condition::Scope { value: "reference" }),
                text: "the reference",
            },
            Text {
                name: "lead",
                when: None,
                text: "`{source}` against `{reference:source}`",
            },
        ],
        forms: vec![Form {
            when: &[],
            values: Vec::new(),
            decision: Decision::Joined(Joined {
                list: "coordinate_differences;length=@length;require_map=true",
                found: "found",
                words: "finding",
                recorded: Some("recorded"),
                separator: " | ",
                refused: "unread: {why}",
            }),
            fail: "{lead}: {found}",
            undecided: "{lead} open: {open}",
            members: None,
            table: None,
            scope: Some(Scopes {
                across: "",
                disciplines: None,
                sources: ScopeSources::Every,
                needs: None,
                messages: messages(),
            }),
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            unless: Vec::new(),
            grading: None,
            once: vec![Once {
                value: measured("reference", "coordinate_reference"),
                applies: None,
                refused: "federation: {why}",
                required: true,
            }],
        }],
    }
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

fn system(name: &str, x: f64, map: bool, site: SitePlacement) -> SourceCoordinateSystem {
    let map = map.then(|| {
        MapConversion::try_new(
            Some("EPSG:25832".into()),
            [500_000.0, 0.0, 0.0],
            [1.0, 0.0],
            1.0,
            Some(1.0),
        )
        .unwrap()
    });
    SourceCoordinateSystem::try_new(
        document(name),
        Some(frame([x, 0.0, 0.0])),
        Some([0.0, 1.0]),
        map,
        Evidence::exact(document(name), format!("crs:{name}")),
    )
    .unwrap()
    .with_site(site)
}

struct Systems(
    Vec<SourceSnapshot>,
    BTreeMap<SourceId, Result<SourceCoordinateSystem, CoordinateSystemError>>,
);

impl Systems {
    /// The systems of the sources held, each covered.
    fn of(held: BTreeMap<SourceId, Result<SourceCoordinateSystem, CoordinateSystemError>>) -> Self {
        let snapshots = held
            .keys()
            .map(|source| {
                SourceSnapshot::try_new(source.clone(), "r", format!("sha256:{source}")).unwrap()
            })
            .collect();
        Self(snapshots, held)
    }
}

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

/// `federation()` over sources `a` (the reference) and those given, the
/// last one holding no object.
fn judge_federation(
    others: Vec<(&str, Result<SourceCoordinateSystem, CoordinateSystemError>)>,
) -> CapabilityEvaluation {
    let mut model = Model::default();
    let mut held = BTreeMap::new();
    let mut session = Vec::new();
    let count = others.len();
    for (index, (name, system)) in [("a", Ok(system("a", 0.0, false, SitePlacement::Absent)))]
        .into_iter()
        .chain(others)
        .enumerate()
    {
        if index < count {
            model = model.object_in(name, "#1", "wall");
        }
        held.insert(document(name), system);
        session.push(document(name));
    }
    let systems = Arc::new(Systems::of(held));
    model.evaluate_measured(
        &Templated::new(federation()),
        &rule("test:federation", kind("wall"), Vec::new()),
        |services: &mut ServiceRegistry| {
            services
                .register(CoordinateSystemServiceHandle::new(systems.clone()))
                .unwrap();
            services
                .register(SessionSources::new(session.clone()))
                .unwrap();
        },
    )
}

/// `(source, message)` of every finding and not-evaluated outcome.
fn worded(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    let scope = |scope: &Scope| match scope {
        Scope::Source(source) => source.document.clone(),
        Scope::Object(object) => object.local_id.clone(),
        Scope::Project => "-".to_owned(),
    };
    evaluation
        .findings()
        .iter()
        .map(|finding| (scope(&finding.scope), finding.message.clone()))
        .chain(
            evaluation
                .not_evaluated_outcomes()
                .iter()
                .map(|outcome| (scope(outcome.scope()), outcome.message().to_owned())),
        )
        .collect()
}

/// A value read once per rule whose refusal is required leaves the rule
/// open, once, and nothing else is judged.
#[test]
fn a_required_value_refused_leaves_only_the_rule_open() {
    let evaluation = judge_federation(Vec::new());
    assert_eq!(
        worded(&evaluation),
        [(
            "-".to_owned(),
            "federation: the run checks 1 source(s); coordinate consistency compares at least \
             two"
            .to_owned()
        )]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

/// Every source is judged, one holding no object too, by the words of the
/// differences its list states joined into one message; the reference is
/// the source the value read once per rule cites.
#[test]
fn every_source_is_judged_by_the_joined_words_of_its_list() {
    let evaluation = judge_federation(vec![
        // Two differences, joined.
        ("b", Ok(system("b", 2.0, false, SitePlacement::Absent))),
        // Open for a statement only it makes, and one not recorded.
        (
            "c",
            Ok(system(
                "c",
                0.0,
                true,
                SitePlacement::Stated(frame([0.0; 3])),
            )),
        ),
        // Unreadable: the list refused.
        (
            "d",
            Err(CoordinateSystemError::Ambiguous("2 contexts".into())),
        ),
        // Holding no object: judged all the same.
        ("e", Ok(system("e", 3.0, true, SitePlacement::Absent))),
    ]);
    assert_eq!(
        worded(&evaluation),
        [
            (
                "a".to_owned(),
                "the reference: states no map conversion; the federation requires one".to_owned()
            ),
            (
                "b".to_owned(),
                "`test:b` against `test:a`: world frame moved by 2.0000 m | states no map \
                 conversion"
                    .to_owned()
            ),
            (
                "e".to_owned(),
                "`test:e` against `test:a`: world frame moved by 3.0000 m".to_owned()
            ),
            (
                "c".to_owned(),
                "`test:c` against `test:a` open: a site is stated only by this source".to_owned()
            ),
            (
                "d".to_owned(),
                "unread: coordinate system is stated ambiguously: 2 contexts".to_owned()
            ),
        ]
    );
    let reasons: Vec<NotEvaluatedReason> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.reason().clone())
        .collect();
    assert_eq!(
        reasons,
        [
            NotEvaluatedReason::IncompleteEvidence,
            NotEvaluatedReason::IncompleteEvidence
        ]
    );
}

/// Where every item left open is not recorded, the source is open as not
/// recorded.
#[test]
fn items_left_open_for_what_is_not_recorded_leave_the_source_not_recorded() {
    let mut template = federation();
    template.forms[0].decision = Decision::Joined(Joined {
        list: "coordinate_differences",
        found: "found",
        words: "finding",
        recorded: Some("recorded"),
        separator: "; ",
        refused: "unread: {why}",
    });
    let model = Model::default()
        .object_in("a", "#1", "wall")
        .object_in("b", "#1", "wall");
    let systems = Arc::new(Systems::of(BTreeMap::from([
        (
            document("a"),
            Ok(system("a", 0.0, true, SitePlacement::Absent)),
        ),
        (
            document("b"),
            Ok(system("b", 0.0, false, SitePlacement::Absent)),
        ),
    ])));
    let evaluation = model.evaluate_measured(
        &Templated::new(template),
        &rule("test:federation", kind("wall"), Vec::new()),
        |services: &mut ServiceRegistry| {
            services
                .register(CoordinateSystemServiceHandle::new(systems.clone()))
                .unwrap();
        },
    );
    assert_eq!(
        worded(&evaluation),
        [(
            "b".to_owned(),
            "`test:b` against `test:a` open: this source states no map conversion, so whether \
             the georeferences agree is unknown"
                .to_owned()
        )]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::NotRecorded
    );
}

/// Without a scope, a joined judgement judges each selected object by its
/// own list: here its source's, a value of a source read on an object.
#[test]
fn a_joined_judgement_judges_an_object_by_its_list() {
    let mut template = federation();
    template.forms[0].scope = None;
    template.forms[0].once = Vec::new();
    template.texts = Vec::new();
    template.forms[0].fail = "{found}";
    let model = Model::default()
        .object_in("a", "#1", "wall")
        .object_in("b", "#1", "wall")
        .object_in("b", "#2", "wall");
    let systems = Arc::new(Systems::of(BTreeMap::from([
        (
            document("a"),
            Ok(system("a", 0.0, true, SitePlacement::Absent)),
        ),
        (
            document("b"),
            Ok(system("b", 2.0, true, SitePlacement::Absent)),
        ),
    ])));
    let evaluation = model.evaluate_measured(
        &Templated::new(template),
        &rule("test:federation", kind("wall"), Vec::new()),
        |services: &mut ServiceRegistry| {
            services
                .register(CoordinateSystemServiceHandle::new(systems.clone()))
                .unwrap();
        },
    );
    assert_eq!(
        worded(&evaluation),
        [
            ("#1".to_owned(), "world frame moved by 2.0000 m".to_owned()),
            ("#2".to_owned(), "world frame moved by 2.0000 m".to_owned()),
        ]
    );
    assert!(
        evaluation
            .findings()
            .iter()
            .all(|finding| finding.scope != Scope::Source(document("b")))
    );
}

/// A form judging sources, or reading a value once per rule, is never
/// forked: an expression rule judges each object on its own.
#[test]
fn a_form_judging_sources_is_not_forked() {
    let federation = Templated::new(federation());
    assert!(matches!(
        fork(
            &federation,
            &rule("test:federation", kind("wall"), Vec::new())
        ),
        Err(ForkError::Inexpressible(_))
    ));
}

/// Answers every derivation with the same sets, or an error.
struct Envelope(Result<(Vec<ObjectId>, Vec<ObjectId>), EnvelopeMembershipError>);

impl EnvelopeMembershipService for Envelope {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        let (declared, derived) = self.0.clone()?;
        EnvelopeMembershipEvidence::try_new(
            request.clone(),
            declared,
            derived,
            1,
            Evidence::exact(document("a"), "envelope"),
        )
    }
}

fn wall(document_name: &str, local: &str) -> ObjectId {
    ObjectId::new(document(document_name), local).unwrap()
}

/// The sources holding a selected wall judged by how many it declares
/// external, then each wall: on the envelope where declared (a check
/// passing where its source declares none, and a quiet one beside it),
/// all gated by an envelope read once per rule where `modes` lists
/// `envelope`; refusals reported after selecting.
#[allow(clippy::too_many_lines)]
fn walls() -> Template {
    let derived = "on_envelope;derivation=all-spaces;bounding=@spaces";
    let declared = "declared_external;derivation=all-spaces;bounding=@spaces";
    let counted = "external_declarations;derivations=all-spaces;bounding=@spaces;\
                   objects=@selection";
    let applies = || {
        Some(Applies {
            when: &[],
            any: &[],
            condition: Some(Condition::Measured { value: "envelope" }),
        })
    };
    let check = |minimum: Option<&'static str>, maximum: Option<&'static str>, fail, quiet| {
        let bound = |name: &'static str| vec![Term::plus(Operand::Value(name))];
        FormCheck {
            values: vec![
                measured("declared", declared),
                measured("on", derived),
                measured("source", counted),
            ],
            decision: Decision::Within {
                value: "on",
                minimum: minimum.map(bound),
                maximum: maximum.map(bound),
                rounding: Vec::new(),
            },
            fail,
            undecided: fail,
            related: None,
            grading: None,
            applies: applies(),
            unless: Some(Condition::Below {
                value: "source",
                than: 0.5,
            }),
            quiet,
            derived: Vec::new(),
        }
    };
    Template {
        id: "test:walls",
        parameters: vec![
            ParameterDescriptor::required("modes", ParameterType::StringList),
            ParameterDescriptor::optional("spaces", ParameterType::Selector),
        ],
        grades: false,
        name: "walls",
        refusals: Refusals::Selected,
        defaults: Vec::new(),
        declaration: vec![
            Check::Listed {
                parameter: "modes",
                options: &["envelope", "nothing"],
                unknown: "mode `{value}` is unknown",
                repeated: "mode `{value}` twice",
            },
            Check::ListedNeeds {
                parameter: "modes",
                needs: &[Needed {
                    value: "envelope",
                    parameters: &["spaces"],
                    message: "the envelope needs `spaces`",
                }],
            },
        ],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![
                measured("counted", counted),
                TemplateValue {
                    name: "one",
                    expression: Expression::Literal {
                        value: ScalarValue::Integer { value: 1 },
                        label: None,
                    },
                    expect: None,
                    absent: None,
                    mismatch: None,
                },
            ],
            decision: Decision::Within {
                value: "counted",
                minimum: Some(vec![Term::plus(Operand::Value("one"))]),
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "declares nothing external",
            undecided: "{counted:upper0} may be",
            members: None,
            table: None,
            scope: Some(Scopes {
                across: "",
                disciplines: None,
                sources: ScopeSources::Occupied,
                needs: Some(Condition::Measured { value: "envelope" }),
                messages: messages(),
            }),
            derived: Vec::new(),
            related: Some("counted"),
            checks: vec![
                check(Some("declared"), None, "declared but not on it", false),
                check(None, Some("declared"), "on it but not declared", true),
            ],
            unless: Vec::new(),
            grading: None,
            once: vec![Once {
                value: measured(
                    "envelope",
                    "envelope_size;derivation=all-spaces;bounding=@spaces",
                ),
                applies: Some(Applies {
                    when: &[],
                    any: &[],
                    condition: Some(Condition::Lists {
                        parameter: "modes",
                        value: "envelope",
                    }),
                }),
                refused: "envelope: {why}",
                required: false,
            }],
        }],
    }
}

fn judge_walls(
    envelope: Envelope,
    parameters: Vec<(&str, ParameterValue)>,
    selected: &str,
) -> CapabilityEvaluation {
    let model = Model::default()
        .object_in("a", "w1", "wall")
        .object_in("a", "w2", "wall")
        .object_in("b", "w3", "wall")
        .object_in("a", "s1", "space");
    let envelope = Arc::new(envelope);
    model.evaluate_measured(
        &Templated::new(walls()),
        &rule("test:walls", kind(selected), parameters),
        |services: &mut ServiceRegistry| {
            services
                .register(EnvelopeMembershipServiceHandle::new(envelope.clone()))
                .unwrap();
            services
                .register(SessionSources::new([
                    document("a"),
                    document("b"),
                    document("c"),
                ]))
                .unwrap();
        },
    )
}

fn envelope_modes() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("modes", strings(&["envelope"])),
        ("spaces", selector(kind("space"))),
    ]
}

/// One measurement leads to outcomes at source and at object level: only
/// the sources holding a selected object are judged, and a check passes
/// where its condition holds over its values (`b` declares nothing, so its
/// wall on the envelope is its source's finding alone).
#[test]
fn one_measurement_judges_sources_and_objects() {
    let evaluation = judge_walls(
        Envelope(Ok((
            vec![wall("a", "w1")],
            vec![wall("a", "w2"), wall("b", "w3")],
        ))),
        envelope_modes(),
        "wall",
    );
    assert_eq!(
        worded(&evaluation),
        [
            ("b".to_owned(), "declares nothing external".to_owned()),
            ("w1".to_owned(), "declared but not on it".to_owned()),
            ("w2".to_owned(), "on it but not declared".to_owned()),
        ]
    );
    assert!(evaluation.findings()[0].related.is_empty());
}

/// A value read once per rule that is not required leaves the rule open
/// where it is refused, and what reads it is not judged.
#[test]
fn a_refused_value_read_once_gates_what_reads_it() {
    let evaluation = judge_walls(
        Envelope(Err(EnvelopeMembershipError::Unavailable)),
        envelope_modes(),
        "wall",
    );
    assert_eq!(
        worded(&evaluation),
        [(
            "-".to_owned(),
            "envelope: envelope membership is unavailable for the requested derivation".to_owned()
        )]
    );
    // Not listed: never read, nothing judged.
    let evaluation = judge_walls(
        Envelope(Err(EnvelopeMembershipError::Unavailable)),
        vec![("modes", strings(&["nothing"]))],
        "wall",
    );
    assert!(worded(&evaluation).is_empty());
}

/// Refusals wait for the selection: nothing selected says nothing, and a
/// list's options are judged as stated, in the list's order.
#[test]
fn refusals_after_selecting_judge_listed_options_as_stated() {
    let refused = |modes: &[&str]| {
        worded(&judge_walls(
            Envelope(Ok((Vec::new(), Vec::new()))),
            vec![("modes", strings(modes))],
            "wall",
        ))
    };
    assert_eq!(
        refused(&[" envelope"]),
        [(
            "-".to_owned(),
            "walls: mode ` envelope` is unknown".to_owned()
        )]
    );
    assert_eq!(
        refused(&["nothing", "nothing", "other"]),
        [("-".to_owned(), "walls: mode `nothing` twice".to_owned())]
    );
    assert_eq!(
        refused(&["nothing", "envelope"]),
        [(
            "-".to_owned(),
            "walls: the envelope needs `spaces`".to_owned()
        )]
    );
    let nothing = judge_walls(
        Envelope(Ok((Vec::new(), Vec::new()))),
        vec![("modes", strings(&["other"]))],
        "door",
    );
    assert!(worded(&nothing).is_empty());
}
