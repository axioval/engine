//! Forms deciding once per source, or once for the whole project, over the
//! objects the rule selects there ([`Scopes`]).

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::template::{Decision, Scopes};
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext, SourceDisciplines};
use axioval_ir::contract::{ParameterValue, ScalarValue};
use axioval_ir::{
    Discipline, Evidence, Finding, NotEvaluatedReason, Object, ObjectId, Scope, SourceId,
};

use super::{Constant, Outcome, Plan, Read, candidates, ranged, read_values, render, within};
use crate::expression_leaves::ObjectLeaves;
use crate::selection::{Selection, population, selector_matches};
use crate::support::sources;

/// The objects selected in one scope: surely, with what selected them, and
/// possibly, with why their selection is undecided.
#[derive(Default)]
struct Tally {
    sure: Vec<ObjectId>,
    evidence: Vec<Evidence>,
    possible: Vec<(ObjectId, NotEvaluatedReason, String)>,
}

/// Sources a declared discipline list leaves out, and sources declaring
/// none; both empty without the list.
#[derive(Default)]
struct Membership {
    left_out: BTreeSet<SourceId>,
    unknown: BTreeSet<SourceId>,
}

/// `message` rendered with a source's and the disciplines' names.
fn worded(
    plan: &Plan<'_>,
    message: &str,
    source: Option<&SourceId>,
    extra: &[(&'static str, String)],
) -> String {
    let mut read = Read::default();
    if let Some(source) = source {
        read.named.insert("source", source.to_string());
    }
    for (name, text) in extra {
        if *name == "why" {
            read.why = Some(text.clone());
        } else {
            read.named.insert(name, text.clone());
        }
    }
    render(plan, &read, message)
}

/// The declared disciplines, as `` `mep` or `hvac` ``.
fn listed(disciplines: &BTreeSet<Discipline>) -> String {
    disciplines
        .iter()
        .map(|discipline| format!("`{discipline}`"))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Runs a form deciding per scope.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    plan: &Plan<'_>,
    decision: &Decision,
    scopes: &Scopes,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let across = matches!(
        plan.constants.get(scopes.across),
        Some(Constant::Scalar(ScalarValue::Boolean { value: true }))
    );
    let disciplines: Option<BTreeSet<Discipline>> =
        scopes
            .disciplines
            .and_then(|parameter| match plan.constants.get(parameter) {
                Some(Constant::Other(ParameterValue::StringList { value })) => Some(
                    value
                        .iter()
                        .filter_map(|name| Discipline::new(name.as_str()).ok())
                        .collect(),
                ),
                _ => None,
            });
    let mut membership = Membership::default();
    if let Some(disciplines) = &disciplines {
        let Some(declared) = context.services.get::<SourceDisciplines>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                scopes.messages.no_disciplines,
            );
        };
        for source in sources(context) {
            match declared.of(&source) {
                Some(discipline) if disciplines.contains(discipline) => {}
                Some(_) => {
                    membership.left_out.insert(source);
                }
                None => {
                    membership.unknown.insert(source);
                }
            }
        }
    }
    let mut evaluation = CapabilityEvaluation::default();
    // Every scope is judged, including one where nothing is selected:
    // that is the case a scope form exists to report.
    let mut tallies: BTreeMap<Scope, Tally> = BTreeMap::new();
    if across {
        tallies.insert(Scope::Project, Tally::default());
    } else {
        for source in sources(context) {
            if membership.left_out.contains(&source) {
                continue;
            }
            if membership.unknown.contains(&source) {
                evaluation.push_source_not_evaluated(
                    source.clone(),
                    NotEvaluatedReason::NotRecorded,
                    worded(plan, scopes.messages.undeclared, Some(&source), &[]),
                );
                continue;
            }
            tallies.insert(Scope::Source(source), Tally::default());
        }
    }
    let (selection, unreadable) = population(context, &rule.selector);
    for (source, why) in unreadable {
        let message = worded(
            plan,
            scopes.messages.unlisted,
            Some(&source),
            &[("why", why)],
        );
        evaluation.push_source_not_evaluated(
            source,
            NotEvaluatedReason::IncompleteEvidence,
            message,
        );
    }
    for object in selection {
        let source = &object.id.source;
        let unknown = membership.unknown.contains(source);
        if membership.left_out.contains(source) || (!across && unknown) {
            continue;
        }
        let scope = if across {
            Scope::Project
        } else {
            Scope::Source(source.clone())
        };
        let tally = tallies.entry(scope).or_default();
        let mut evidence = Vec::new();
        match (
            selector_matches(context, &rule.selector, object, &mut evidence),
            unknown,
        ) {
            (Selection::NoMatch, _) => {}
            (Selection::Match, false) => {
                tally.sure.push(object.id.clone());
                tally.evidence.extend(evidence);
            }
            // Across sources, a selected object of a source declaring no
            // discipline may count or not.
            (Selection::Match, true) => tally.possible.push((
                object.id.clone(),
                NotEvaluatedReason::NotRecorded,
                worded(plan, scopes.messages.undeclared_member, Some(source), &[]),
            )),
            (Selection::NotEvaluated(reason, message), _) => {
                tally.possible.push((object.id.clone(), reason, message));
            }
        }
    }
    if tallies.is_empty() && membership.unknown.is_empty() {
        // Per source, and no source counts: there is no scope to judge,
        // and saying nothing would read as a pass.
        let message = match &disciplines {
            Some(disciplines) if !membership.left_out.is_empty() => worded(
                plan,
                scopes.messages.no_discipline,
                None,
                &[("disciplines", listed(disciplines))],
            ),
            _ => worded(plan, scopes.messages.no_source, None, &[]),
        };
        evaluation.push_not_evaluated(NotEvaluatedReason::IncompleteEvidence, message);
    }
    for (scope, tally) in tallies {
        judge(
            plan,
            decision,
            scopes,
            context,
            rule,
            scope,
            tally,
            &mut evaluation,
        );
    }
    evaluation
}

/// Reads the plan's values over one scope's objects and decides.
#[allow(clippy::too_many_arguments)]
fn judge(
    plan: &Plan<'_>,
    decision: &Decision,
    scopes: &Scopes,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    scope: Scope,
    tally: Tally,
    evaluation: &mut CapabilityEvaluation,
) {
    let source = match &scope {
        Scope::Source(source) => Some(source.clone()),
        Scope::Project | Scope::Object(_) => None,
    };
    // The scope itself is no object of the model: the values read only
    // its objects, which the leaves supply.
    let stand_in = Object::new(
        ObjectId::new(
            source
                .clone()
                .unwrap_or_else(|| SourceId::new("axioval", "project").expect("a valid source id")),
            axioval_engine::template::SELECTION,
        )
        .expect("a valid object id"),
        axioval_engine::template::SELECTION,
    );
    let possible: Vec<ObjectId> = tally.possible.iter().map(|(id, _, _)| id.clone()).collect();
    let mut leaves = ObjectLeaves::new(context, &stand_in, Some(&rule.parameters)).supplying(
        Scopes::source(),
        candidates(context, &tally.sure, &possible),
    );
    let mut read = Read::default();
    read.named.insert(
        "place",
        match &source {
            Some(source) => worded(plan, scopes.messages.source, Some(source), &[]),
            None => worded(plan, scopes.messages.project, None, &[]),
        },
    );
    read.named
        .insert("undecided", tally.possible.len().to_string());
    // Whether the possible objects are what leaves the scope undecided.
    let mut straddled = false;
    let outcome = match read_values(
        plan,
        plan.values(),
        &|name| super::judges_stated(decision, name),
        context,
        &stand_in,
        &mut leaves,
        &mut read,
    ) {
        Some(outcome) => outcome,
        None => match within(plan, &read, decision) {
            Some(judged) => {
                read.bounds = Some((judged.minimum, judged.maximum));
                read.evidence = tally.evidence;
                straddled = matches!(judged.verdict, crate::plan_area::Verdict::Undecided(_));
                ranged(plan, read, &judged, tally.sure)
            }
            None => Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{}: a value the decision reads is no number",
                    plan.template.name
                ),
            ),
        },
    };
    match outcome {
        Outcome::Passed => {}
        Outcome::Finding {
            message,
            evidence,
            related,
            deviation,
        } => {
            let mut finding = Finding::new(
                rule.id.clone(),
                scope,
                crate::pairs::severity(rule),
                message,
            )
            .with_evidence(evidence)
            .with_related(related);
            finding
                .evidence
                .sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
            finding.evidence.dedup();
            evaluation.push_finding_deviating(finding, deviation);
        }
        Outcome::Open(reason, message) => {
            // A scope its undecided objects leave open leaves them open
            // too, each for its own reason.
            if straddled {
                for (object, reason, detail) in tally.possible {
                    evaluation.push_object_not_evaluated(object, reason, detail);
                }
            }
            match scope {
                Scope::Source(source) => {
                    evaluation.push_source_not_evaluated(source, reason, message);
                }
                Scope::Project | Scope::Object(_) => evaluation.push_not_evaluated(reason, message),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axioval_engine::template::{
        Decision, Form, Operand, ScopeMessages, Scopes, Template, TemplateValue, Term,
    };
    use axioval_engine::{
        CompiledRule, ParameterDescriptor, ParameterType, RuleContext, ServiceRegistry,
    };
    use axioval_ir::contract::{AggregateFunction, Expression, ParameterValue, Selector, Severity};
    use axioval_ir::{Object, ObjectId, Project, RuleId, Scope, SourceId};

    /// At least `minimum` objects selected per source, or across sources.
    fn template() -> Template {
        Template {
            id: "test:scoped",
            parameters: vec![
                ParameterDescriptor::required("minimum", ParameterType::Integer),
                ParameterDescriptor::optional("across", ParameterType::Boolean),
            ],
            grades: false,
            name: "scoped",
            refusals: axioval_engine::template::Refusals::Rule,
            defaults: Vec::new(),
            declaration: Vec::new(),
            services: None,
            texts: Vec::new(),
            forms: vec![Form {
                when: &[],
                values: vec![TemplateValue {
                    name: "count",
                    expression: Expression::Aggregate {
                        function: AggregateFunction::Count,
                        over: Scopes::source(),
                        filter: None,
                        value: None,
                        label: None,
                    },
                    expect: None,
                    absent: None,
                    mismatch: None,
                }],
                decision: Decision::Within {
                    value: "count",
                    minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
                    maximum: None,
                    rounding: Vec::new(),
                },
                fail: "{count:least} {place}; required {required}",
                undecided: "{count:least} and {undecided} more {place}",
                members: None,
                table: None,
                scope: Some(Scopes {
                    across: "across",
                    disciplines: None,
                    messages: ScopeMessages {
                        source: "in `{source}`",
                        project: "in the project",
                        no_source: "nowhere to count",
                        no_discipline: "no source plays {disciplines}",
                        undeclared: "`{source}` declares no discipline",
                        undeclared_member: "`{source}` declares no discipline",
                        unlisted: "unlisted: {why}",
                        no_disciplines: "no disciplines",
                    },
                }),
                derived: Vec::new(),
                related: None,
                checks: Vec::new(),
            }],
        }
    }

    fn rule(minimum: i64, across: bool) -> CompiledRule {
        CompiledRule {
            id: RuleId::new("scoped").unwrap(),
            capability: "test:scoped".into(),
            severity: Severity::Error,
            selector: Selector::All,
            parameters: [
                (
                    "minimum".to_owned(),
                    ParameterValue::Integer { value: minimum },
                ),
                (
                    "across".to_owned(),
                    ParameterValue::Boolean { value: across },
                ),
            ]
            .into(),
        }
    }

    fn object(document: &str, local: &str) -> Object {
        Object::new(
            ObjectId::new(SourceId::new("test", document).unwrap(), local).unwrap(),
            "wall",
        )
    }

    /// Each source is judged on its own objects, the project on all, and a
    /// finding is scoped to the source or the project it is about.
    #[test]
    fn a_scope_is_judged_on_the_objects_selected_in_it() {
        let project =
            Project::new(vec![object("a", "1"), object("a", "2"), object("b", "3")]).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let per_source = super::super::run(
            (&template(), &super::super::Plans::new()),
            &context,
            &rule(2, false),
        );
        let [finding] = per_source.findings() else {
            panic!("{:?}", per_source.findings());
        };
        assert_eq!(
            finding.scope,
            Scope::Source(SourceId::new("test", "b").unwrap())
        );
        assert_eq!(finding.message, "1 in `test:b`; required at least 2");
        assert_eq!(finding.related.len(), 1);
        let across = super::super::run(
            (&template(), &super::super::Plans::new()),
            &context,
            &rule(3, true),
        );
        assert!(across.findings().is_empty() && across.not_evaluated_outcomes().is_empty());
        let across = super::super::run(
            (&template(), &super::super::Plans::new()),
            &context,
            &rule(4, true),
        );
        assert_eq!(across.findings()[0].scope, Scope::Project);
        assert_eq!(
            across.findings()[0].message,
            "3 in the project; required at least 4"
        );
    }

    /// With no source at all there is nothing to judge, and the rule says
    /// so rather than passing.
    #[test]
    fn no_scope_leaves_the_rule_open() {
        let project = Project::new(Vec::new()).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let evaluation = super::super::run(
            (&template(), &super::super::Plans::new()),
            &context,
            &rule(1, false),
        );
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].message(),
            "nowhere to count"
        );
    }
}
