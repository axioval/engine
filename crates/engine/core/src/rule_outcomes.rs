//! Rules that depend on other rules' outcomes: gates and `ruleOutcome`
//! selectors, the order they impose on a plan, and the outcomes the runtime
//! records for them.
//!
//! Capabilities never feed one another. A rule instance may instead read
//! how another rule of its ruleset fared, as a whole (a gate that runs it
//! only if the other rule passed or failed) or per object (a selector on
//! the objects the other rule passed or failed). The runtime runs the other
//! rule first, records its refined outcomes, and answers from them. What
//! the other rule left undecided stays undecided: never a match, never a
//! non-match.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::contract::{GateCondition, ParameterValue, RuleOutcomeKind, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, Object, ObjectId, RuleId, Scope, SourceId};

use crate::{CapabilityEvaluation, CompiledRule, OutcomeRefiner, ResourceObjects, RuleContext};

/// How a selector judged one object, as the host's outcome refiner
/// evaluates it ([`crate::OutcomeRefiner::evaluate_selector`]).
#[derive(Clone, Debug, PartialEq)]
pub enum SelectorVerdict {
    /// Selected, with the facts that decided it.
    Match(Vec<Evidence>),
    /// Not selected, with the facts that decided it.
    NoMatch(Vec<Evidence>),
    /// Cannot be decided, and why.
    Undecided(NotEvaluatedReason, String),
}

/// How a completed rule fared as a whole.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleVerdict {
    /// No finding and nothing left not evaluated.
    Passed,
    /// At least one finding.
    Failed,
    /// No finding, but something left not evaluated.
    Undecided,
    /// Its own gate skipped it.
    Skipped,
}

/// How a completed rule judged one object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectVerdict {
    /// Selected, and nothing found or left open about it.
    Passed,
    /// The subject of at least one finding.
    Failed,
    /// Surely not selected, and nothing reported about it.
    NotSelected,
    /// Left not evaluated, or its selection undecided, and why.
    Undecided(String),
}

/// One completed rule's refined outcomes, as dependent rules read them.
#[derive(Clone, Debug, Default)]
pub struct RuleRecord {
    skipped: bool,
    found: bool,
    open: bool,
    /// Subjects of findings.
    failed: BTreeSet<ObjectId>,
    /// Objects left not evaluated, with the first reason.
    undecided: BTreeMap<ObjectId, String>,
    /// Sources reported about as a whole, with the first message.
    sources: BTreeMap<SourceId, String>,
    /// The first outcome about the project as a whole.
    project: Option<String>,
    /// The rule's selection, when a dependent reads it per object: every
    /// object surely selected, and every object whose selection is
    /// undecided with why.
    selection: Option<(BTreeSet<ObjectId>, BTreeMap<ObjectId, String>)>,
    /// Sources where the rule's selection reached resource objects that
    /// could not be listed, with why.
    unread: BTreeSet<(SourceId, String)>,
}

impl RuleRecord {
    /// The record of a rule its gate skipped.
    pub(crate) fn skipped() -> Self {
        Self {
            skipped: true,
            ..Self::default()
        }
    }

    /// The record of a rule left not evaluated as a whole.
    pub(crate) fn undecided(message: &str) -> Self {
        Self {
            open: true,
            project: Some(message.to_owned()),
            ..Self::default()
        }
    }

    /// The record of a rule's refined `evaluation`, with its applicability
    /// selection when a dependent reads it per object.
    pub(crate) fn of(evaluation: &CapabilityEvaluation, selection: Option<Selection>) -> Self {
        let mut record = Self {
            found: !evaluation.findings().is_empty(),
            open: !evaluation.not_evaluated_outcomes().is_empty(),
            ..Self::default()
        };
        for finding in evaluation.findings() {
            let message = format!("it reported `{}` about it", finding.message);
            match &finding.scope {
                Scope::Object(object) => {
                    record.failed.insert(object.clone());
                }
                scope => record.note_whole(scope, message),
            }
        }
        for outcome in evaluation.not_evaluated_outcomes() {
            match outcome.scope() {
                Scope::Object(object) => {
                    record
                        .undecided
                        .entry(object.clone())
                        .or_insert_with(|| outcome.message().to_owned());
                }
                scope => record.note_whole(scope, outcome.message().to_owned()),
            }
        }
        let selection = selection.map(|selection| {
            record.unread = selection.unread.into_iter().collect();
            selection.verdicts
        });
        record.selection = selection.map(|verdicts| {
            let mut selected = BTreeSet::new();
            let mut open = BTreeMap::new();
            for (object, verdict) in verdicts {
                match verdict {
                    SelectorVerdict::Match(_) => {
                        selected.insert(object);
                    }
                    SelectorVerdict::NoMatch(_) => {}
                    SelectorVerdict::Undecided(_, why) => {
                        open.insert(object, why);
                    }
                }
            }
            (selected, open)
        });
        record
    }

    fn note_whole(&mut self, scope: &Scope, message: String) {
        match scope {
            Scope::Source(source) => {
                self.sources.entry(source.clone()).or_insert(message);
            }
            _ => {
                self.project.get_or_insert(message);
            }
        }
    }

    /// Every object and resource object the rule's outcomes or recorded
    /// selection name, sorted, possibly more than once.
    pub fn named(&self) -> impl Iterator<Item = &ObjectId> {
        let (selected, open) = self
            .selection
            .as_ref()
            .map(|(selected, open)| (Some(selected), Some(open)))
            .unwrap_or_default();
        self.failed
            .iter()
            .chain(self.undecided.keys())
            .chain(selected.into_iter().flatten())
            .chain(open.into_iter().flat_map(BTreeMap::keys))
    }

    /// Sources where the rule's selection reached resource objects that
    /// could not be listed, with why.
    pub fn unread_resources(&self) -> impl Iterator<Item = &(SourceId, String)> {
        self.unread.iter()
    }

    /// How the rule fared as a whole.
    #[must_use]
    pub fn verdict(&self) -> RuleVerdict {
        if self.skipped {
            RuleVerdict::Skipped
        } else if self.found {
            RuleVerdict::Failed
        } else if self.open {
            RuleVerdict::Undecided
        } else {
            RuleVerdict::Passed
        }
    }

    /// How the rule judged `object`.
    ///
    /// A finding about the object fails it. Otherwise an object left not
    /// evaluated, or whose source or project the rule reported about as a
    /// whole, or whose selection the rule could not decide, is undecided;
    /// an object the rule surely did not select is not selected, and one it
    /// selected passed. A skipped rule selected nothing. Without a recorded
    /// selection every object is undecided.
    #[must_use]
    pub fn object(&self, object: &Object) -> ObjectVerdict {
        if self.failed.contains(&object.id) {
            return ObjectVerdict::Failed;
        }
        if self.skipped {
            return ObjectVerdict::NotSelected;
        }
        if let Some(why) = self.undecided.get(&object.id) {
            return ObjectVerdict::Undecided(format!("it was not evaluated: {why}"));
        }
        let Some((selected, open)) = &self.selection else {
            return ObjectVerdict::Undecided("its selection was not recorded".into());
        };
        let chosen = selected.contains(&object.id);
        let doubt = open.get(&object.id);
        if !chosen && doubt.is_none() {
            return ObjectVerdict::NotSelected;
        }
        if let Some(why) = &self.project {
            return ObjectVerdict::Undecided(format!("it reported about the whole model: {why}"));
        }
        if let Some(why) = self.sources.get(&object.id.source) {
            return ObjectVerdict::Undecided(format!(
                "it reported about source `{}` as a whole: {why}",
                object.id.source
            ));
        }
        match doubt {
            Some(why) => ObjectVerdict::Undecided(format!("its selection is undecided: {why}")),
            None => ObjectVerdict::Passed,
        }
    }
}

/// The outcomes of every rule completed so far in a run, by compiled rule
/// id. The runtime installs it before every rule, replacing any host copy,
/// so a `ruleOutcome` selector reads the rules the plan ran first.
#[derive(Clone, Debug, Default)]
pub struct RuleOutcomes(BTreeMap<RuleId, Arc<RuleRecord>>);

impl RuleOutcomes {
    /// The record of the completed rule `rule`, if it ran before.
    #[must_use]
    pub fn get(&self, rule: &str) -> Option<&RuleRecord> {
        let id = RuleId::new(rule).ok()?;
        self.0.get(&id).map(AsRef::as_ref)
    }

    pub(crate) fn insert(&mut self, rule: RuleId, record: RuleRecord) {
        self.0.insert(rule, Arc::new(record));
    }

    /// Whether whole-rule `gates` let their rule run: closed when any is,
    /// else undecided when any is.
    pub(crate) fn gate(&self, gates: &[(RuleId, GateCondition)]) -> Gate {
        let mut state = Gate::Open;
        for (parent, condition) in gates {
            let verdict = self
                .0
                .get(parent)
                .map_or(RuleVerdict::Undecided, |record| record.verdict());
            match gate(parent, *condition, verdict) {
                Gate::Closed => return Gate::Closed,
                undecided @ Gate::Undecided(_) if state == Gate::Open => state = undecided,
                _ => {}
            }
        }
        state
    }

    /// Whether `rule` judged `object` as `outcome` asks, for a
    /// `ruleOutcome` selector; `Err` with a reason when that cannot be
    /// decided.
    pub fn selects(
        &self,
        rule: &str,
        outcome: RuleOutcomeKind,
        object: &Object,
    ) -> Result<bool, (NotEvaluatedReason, String)> {
        let Some(record) = self.get(rule) else {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                format!("the outcomes of rule `{rule}` are not available: it has not run before"),
            ));
        };
        match record.object(object) {
            ObjectVerdict::Passed => Ok(outcome == RuleOutcomeKind::Passed),
            ObjectVerdict::Failed => Ok(outcome == RuleOutcomeKind::Failed),
            ObjectVerdict::NotSelected => Ok(false),
            ObjectVerdict::Undecided(why) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("rule `{rule}` left {} undecided: {why}", object.id),
            )),
        }
    }
}

/// How a rule's applicability selector judged its population.
pub(crate) struct Selection {
    verdicts: Vec<(ObjectId, SelectorVerdict)>,
    unread: Vec<(SourceId, String)>,
}

impl From<Vec<(ObjectId, SelectorVerdict)>> for Selection {
    /// A selection that reached no unlisted resource objects.
    fn from(verdicts: Vec<(ObjectId, SelectorVerdict)>) -> Self {
        Self {
            verdicts,
            unread: Vec::new(),
        }
    }
}

/// How `rule`'s applicability selector judges every object of the
/// project and every resource object it reaches, as a dependent reading it
/// per object needs.
pub(crate) fn selection(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> Selection {
    let reached = context
        .services
        .get::<ResourceObjects>()
        .map(|resources| resources.reached(&rule.selector, context.services.get::<RuleOutcomes>()))
        .unwrap_or_default();
    let verdicts = context
        .project
        .objects()
        .chain(reached.objects)
        .map(|object| {
            let verdict = refiner.evaluate_selector(context, &rule.selector, object);
            (object.id.clone(), verdict)
        })
        .collect();
    Selection {
        verdicts,
        unread: reached.unreadable,
    }
}

/// Whether a whole-rule gate lets its rule run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Gate {
    Open,
    Closed,
    Undecided(String),
}

/// The state of the whole-rule gate `condition` on the rule `parent` whose
/// verdict is `verdict`. A skipped parent keeps every gate closed.
pub(crate) fn gate(parent: &RuleId, condition: GateCondition, verdict: RuleVerdict) -> Gate {
    let wants = match condition {
        GateCondition::AllIfPassed => RuleVerdict::Passed,
        GateCondition::AllIfFailed => RuleVerdict::Failed,
        // Object conditions narrow the selection; they never close a rule.
        GateCondition::PassedObjects | GateCondition::FailedObjects => return Gate::Open,
    };
    match verdict {
        RuleVerdict::Undecided => Gate::Undecided(format!(
            "the rule runs only if rule `{parent}` {}, and `{parent}` left outcomes not \
             evaluated without a finding, so whether it passed is undecided",
            if wants == RuleVerdict::Passed {
                "passed"
            } else {
                "failed"
            }
        )),
        verdict if verdict == wants => Gate::Open,
        _ => Gate::Closed,
    }
}

/// The selector an object gate narrows its rule's applicability by.
pub(crate) fn gate_selector(rule: &str, condition: GateCondition) -> Option<Selector> {
    let outcome = match condition {
        GateCondition::PassedObjects => RuleOutcomeKind::Passed,
        GateCondition::FailedObjects => RuleOutcomeKind::Failed,
        GateCondition::AllIfPassed | GateCondition::AllIfFailed => return None,
    };
    Some(Selector::RuleOutcome {
        rule: rule.to_owned(),
        outcome,
    })
}

/// Every rule a `ruleOutcome` selector in `selector` names.
pub(crate) fn selector_references<'a>(selector: &'a Selector, out: &mut BTreeSet<&'a str>) {
    match selector {
        Selector::RuleOutcome { rule, .. } => {
            out.insert(rule);
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                selector_references(operand, out);
            }
        }
        Selector::Not { operand } => selector_references(operand, out),
        Selector::Related { selector, .. } => selector_references(selector, out),
        Selector::All
        | Selector::EntityType { .. }
        | Selector::Property { .. }
        | Selector::PropertyPattern { .. }
        | Selector::Classification { .. }
        | Selector::Discipline { .. }
        | Selector::Source { .. } => {}
    }
}

/// Every rule a selector within `value` names.
pub(crate) fn value_references<'a>(value: &'a ParameterValue, out: &mut BTreeSet<&'a str>) {
    match value {
        ParameterValue::Selector { value } => selector_references(value, out),
        ParameterValue::Table { value: rows } => {
            for cell in rows.iter().flat_map(TableRow::values) {
                value_references(cell, out);
            }
        }
        _ => {}
    }
}

/// Renames every rule a `ruleOutcome` selector in `selector` names.
pub(crate) fn rename_selector(selector: &mut Selector, rename: &dyn Fn(&str) -> String) {
    match selector {
        Selector::RuleOutcome { rule, .. } => *rule = rename(rule),
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                rename_selector(operand, rename);
            }
        }
        Selector::Not { operand } => rename_selector(operand, rename),
        Selector::Related { selector, .. } => rename_selector(selector, rename),
        Selector::All
        | Selector::EntityType { .. }
        | Selector::Property { .. }
        | Selector::PropertyPattern { .. }
        | Selector::Classification { .. }
        | Selector::Discipline { .. }
        | Selector::Source { .. } => {}
    }
}

/// Renames every rule a selector within `value` names.
pub(crate) fn rename_value(value: &mut ParameterValue, rename: &dyn Fn(&str) -> String) {
    match value {
        ParameterValue::Selector { value } => rename_selector(value, rename),
        ParameterValue::Table { value: rows } => {
            for cell in rows.iter_mut().flat_map(|row| row.values_mut()) {
                rename_value(cell, rename);
            }
        }
        _ => {}
    }
}

/// `rules` ordered so every rule follows the rules it depends on, ties by
/// id; `Err` with the rules of a cycle.
pub(crate) fn dependency_order(
    rules: Vec<CompiledRule>,
    dependencies: &BTreeMap<RuleId, BTreeSet<RuleId>>,
) -> Result<Vec<CompiledRule>, Vec<RuleId>> {
    let mut pending: BTreeMap<RuleId, CompiledRule> = rules
        .into_iter()
        .map(|rule| (rule.id.clone(), rule))
        .collect();
    let mut ordered = Vec::with_capacity(pending.len());
    loop {
        let ready: Option<RuleId> = pending
            .keys()
            .find(|id| {
                dependencies
                    .get(*id)
                    .is_none_or(|needs| needs.iter().all(|need| !pending.contains_key(need)))
            })
            .cloned();
        match ready {
            Some(id) => ordered.push(pending.remove(&id).expect("a ready rule is pending")),
            None if pending.is_empty() => return Ok(ordered),
            None => return Err(pending.into_keys().collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::{Finding, Severity};

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    fn object(local: &str) -> Object {
        Object::new(id(local), "door")
    }

    /// d1 selected and clean, d2 found, d3 left open, d4 not selected, d5
    /// of undecided selection.
    fn record() -> RuleRecord {
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_finding(Finding::new(
            RuleId::new("type").unwrap(),
            id("d2"),
            Severity::Error,
            "no rating",
        ));
        evaluation.push_object_not_evaluated(
            id("d3"),
            NotEvaluatedReason::BackendUnavailable,
            "unreadable",
        );
        let selection = vec![
            (id("d1"), SelectorVerdict::Match(Vec::new())),
            (id("d2"), SelectorVerdict::Match(Vec::new())),
            (id("d3"), SelectorVerdict::Match(Vec::new())),
            (id("d4"), SelectorVerdict::NoMatch(Vec::new())),
            (
                id("d5"),
                SelectorVerdict::Undecided(NotEvaluatedReason::InvalidEvidence, "list".into()),
            ),
        ];
        RuleRecord::of(&evaluation, Some(selection.into()))
    }

    #[test]
    fn an_object_is_judged_by_its_finding_its_outcome_and_its_selection() {
        let record = record();
        assert_eq!(record.verdict(), RuleVerdict::Failed);
        assert_eq!(record.object(&object("d1")), ObjectVerdict::Passed);
        assert_eq!(record.object(&object("d2")), ObjectVerdict::Failed);
        assert!(matches!(
            record.object(&object("d3")),
            ObjectVerdict::Undecided(_)
        ));
        assert_eq!(record.object(&object("d4")), ObjectVerdict::NotSelected);
        assert!(matches!(
            record.object(&object("d5")),
            ObjectVerdict::Undecided(_)
        ));
    }

    #[test]
    fn an_outcome_about_the_source_leaves_its_selected_objects_undecided() {
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_source_not_evaluated(
            SourceId::new("test", "model").unwrap(),
            NotEvaluatedReason::NotRecorded,
            "no layers",
        );
        let record = RuleRecord::of(
            &evaluation,
            Some(
                vec![
                    (id("d1"), SelectorVerdict::Match(Vec::new())),
                    (id("d4"), SelectorVerdict::NoMatch(Vec::new())),
                ]
                .into(),
            ),
        );
        assert_eq!(record.verdict(), RuleVerdict::Undecided);
        assert!(matches!(
            record.object(&object("d1")),
            ObjectVerdict::Undecided(_)
        ));
        assert_eq!(record.object(&object("d4")), ObjectVerdict::NotSelected);
        // Without a recorded selection nothing is decided.
        let unrecorded = RuleRecord::of(&CapabilityEvaluation::default(), None);
        assert!(matches!(
            unrecorded.object(&object("d1")),
            ObjectVerdict::Undecided(_)
        ));
    }

    #[test]
    fn whole_gates_open_close_or_stay_undecided_with_the_parent() {
        let parent = RuleId::new("type").unwrap();
        let passed = GateCondition::AllIfPassed;
        assert_eq!(gate(&parent, passed, RuleVerdict::Passed), Gate::Open);
        assert_eq!(gate(&parent, passed, RuleVerdict::Failed), Gate::Closed);
        assert_eq!(gate(&parent, passed, RuleVerdict::Skipped), Gate::Closed);
        assert!(matches!(
            gate(&parent, GateCondition::AllIfFailed, RuleVerdict::Undecided),
            Gate::Undecided(_)
        ));
        assert_eq!(
            gate(
                &parent,
                GateCondition::FailedObjects,
                RuleVerdict::Undecided
            ),
            Gate::Open
        );
    }
}
