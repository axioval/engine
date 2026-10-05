//! A differential parity harness: a capability rule and its re-expression
//! (an expression rule, or a template composed of measured values,
//! expressions and generic judges), run over the same models, must judge
//! every object, source and project alike before the capability's decision
//! moves.
//!
//! Each side is read into [`Observations`]: every scope a rule reported
//! about (an object, a source, the project), its findings with their
//! severity, evidence exactness, categories, messages and related objects,
//! its not-evaluated outcomes with their reasons, the graded deviation of
//! each finding, the rule's measured values (its report tables, and values
//! the caller measured itself), and, where known, the rule's selection, so
//! that an object a rule passed and one it did not select are told apart.
//!
//! A [`Parity`] says which of those must agree: [`Parity::outcomes`] what
//! an expression rewrite must reproduce (verdict, severity, exactness,
//! reason, finding count and categories, at every scope), and
//! [`Parity::contract`] everything a template standing in for a built-in
//! capability must reproduce (its messages and related objects, its graded
//! deviations too). Measured values are compared by name, each within the
//! rounding it is declared with ([`Parity::value`]). [`Parity::compare`]
//! lines two sides up scope by scope and returns [`ParityEvidence`], which
//! a migration ledger records as proof that a re-expression was checked.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axioval_engine::{CapabilityEvaluation, ObjectVerdict, RuleOutcomes};
use axioval_ir::{
    ColumnExactness, Finding, NotEvaluatedReason, ObjectId, PropertyValue, Report, ReportTable,
    ReportValue, RuleStatus, Scope, Severity,
};
use serde::{Serialize, Serializer};

/// How one rule judged one scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum Outcome {
    /// Found: how severe, and whether every finding's evidence is exact.
    Finding {
        /// The most severe finding's severity.
        severity: Severity,
        /// Whether every finding's evidence is exact.
        exact: bool,
    },
    /// Left open, and why.
    NotEvaluated {
        /// Why the rule could not decide.
        reason: NotEvaluatedReason,
    },
    /// Selected, and nothing found or left open about it.
    Passed,
    /// Surely not selected, and nothing reported about it.
    NotSelected,
    /// Nothing reported about it, but whether it was selected is
    /// undecided, or it lies in a source or project the rule reported about
    /// as a whole.
    Undecided,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finding { severity, exact } => write!(
                f,
                "finding ({severity:?}, {} evidence)",
                if *exact { "exact" } else { "inexact" }
            ),
            Self::NotEvaluated { reason } => write!(f, "not evaluated ({reason:?})"),
            Self::Passed => f.write_str("passed"),
            Self::NotSelected => f.write_str("not selected"),
            Self::Undecided => f.write_str("undecided"),
        }
    }
}

/// One measured value: an interval in a unit, a text, a value the source
/// states absent (`null`), or one that could not be measured. `null` and
/// not evaluated never compare equal.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Measure {
    /// A number within `lower..=upper`, in `unit` (`m`, `m²`; empty for a
    /// plain number). `exact` says whether it rests on exact evidence,
    /// where the side knows.
    Number {
        /// The least value it may take.
        lower: f64,
        /// The greatest value it may take.
        upper: f64,
        /// The coherent SI unit symbol, empty for a plain number.
        unit: String,
        /// Whether it rests on exact evidence; `None` when not known.
        #[serde(skip_serializing_if = "Option::is_none")]
        exact: Option<bool>,
    },
    /// A text.
    Text {
        /// The text.
        value: String,
    },
    /// Stated absent.
    Null,
    /// Could not be measured.
    NotEvaluated,
}

impl Measure {
    /// An exact number in `unit`.
    #[must_use]
    pub fn exact(value: f64, unit: &str) -> Self {
        Self::Number {
            lower: value,
            upper: value,
            unit: unit.to_owned(),
            exact: Some(true),
        }
    }

    /// A number known to lie within `lower..=upper`, in `unit`, of unknown
    /// exactness.
    #[must_use]
    pub fn interval(lower: f64, upper: f64, unit: &str) -> Self {
        Self::Number {
            lower,
            upper,
            unit: unit.to_owned(),
            exact: None,
        }
    }

    /// A property value as a measure: a number, a quantity in its SI unit,
    /// a measured interval (inexact), a text or `null`. Any other value
    /// (a list, a table, a date) is no single measure.
    #[must_use]
    pub fn of_property(value: &PropertyValue) -> Option<Self> {
        #[allow(clippy::cast_precision_loss)]
        Some(match value {
            PropertyValue::Null => Self::Null,
            PropertyValue::Integer(value) => Self::exact(*value as f64, ""),
            PropertyValue::Decimal(value) => Self::exact(*value, ""),
            PropertyValue::Quantity { value, dimension } => {
                Self::exact(*value, &dimension.unit_symbol())
            }
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            } => Self::Number {
                lower: *lower,
                upper: *upper,
                unit: dimension
                    .map(axioval_ir::QuantityDimension::unit_symbol)
                    .unwrap_or_default(),
                exact: Some(false),
            },
            PropertyValue::String(value) => Self::Text {
                value: value.clone(),
            },
            PropertyValue::Boolean(value) => Self::Text {
                value: value.to_string(),
            },
            _ => return None,
        })
    }

    fn of_cell(value: &ReportValue, unit: &str, exactness: Option<ColumnExactness>) -> Self {
        let exact = exactness.map(|exactness| exactness == ColumnExactness::Exact);
        match value {
            ReportValue::Unknown => Self::NotEvaluated,
            ReportValue::Exact { value } => Self::Number {
                lower: *value,
                upper: *value,
                unit: unit.to_owned(),
                exact,
            },
            ReportValue::Interval { lower, upper } => Self::Number {
                lower: *lower,
                upper: *upper,
                unit: unit.to_owned(),
                exact,
            },
            ReportValue::Text { value } => Self::Text {
                value: value.clone(),
            },
        }
    }

    /// Whether `self` and `other` agree within `step` on each bound, in the
    /// same unit, of the same exactness where both know it.
    fn agrees(&self, other: &Self, step: f64) -> bool {
        match (self, other) {
            (
                Self::Number {
                    lower,
                    upper,
                    unit,
                    exact,
                },
                Self::Number {
                    lower: other_lower,
                    upper: other_upper,
                    unit: other_unit,
                    exact: other_exact,
                },
            ) => {
                unit == other_unit
                    && within(*lower, *other_lower, step)
                    && within(*upper, *other_upper, step)
                    && match (exact, other_exact) {
                        (Some(exact), Some(other)) => exact == other,
                        _ => true,
                    }
            }
            _ => self == other,
        }
    }
}

/// Whether `a` and `b` differ by at most `step`, allowing a few units in
/// the last place of either.
fn within(a: f64, b: f64, step: f64) -> bool {
    #[allow(clippy::float_cmp)]
    if a == b {
        return true;
    }
    (a - b).abs() <= step + 4.0 * f64::EPSILON * a.abs().max(b.abs())
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number {
                lower,
                upper,
                unit,
                exact,
            } => {
                #[allow(clippy::float_cmp)]
                if lower == upper {
                    write!(f, "{lower}")?;
                } else {
                    write!(f, "{lower}..{upper}")?;
                }
                if !unit.is_empty() {
                    write!(f, " {unit}")?;
                }
                match exact {
                    Some(true) => f.write_str(" (exact)"),
                    Some(false) => f.write_str(" (inexact)"),
                    None => Ok(()),
                }
            }
            Self::Text { value } => write!(f, "`{value}`"),
            Self::Null => f.write_str("null"),
            Self::NotEvaluated => f.write_str("not evaluated"),
        }
    }
}

/// One finding, as the harness compares it.
#[derive(Clone, Debug, PartialEq)]
struct Reported {
    severity: Severity,
    exact: bool,
    message: String,
    categories: Vec<String>,
    related: Vec<ObjectId>,
    deviation: Option<(f64, f64)>,
}

impl Reported {
    fn of(finding: &Finding, deviation: Option<(f64, f64)>) -> Self {
        Self {
            severity: finding.severity.clone(),
            exact: finding.evidence.iter().all(|evidence| evidence.exact),
            message: finding.message.clone(),
            categories: finding.categories.clone(),
            related: finding.related.clone(),
            deviation,
        }
    }
}

/// Everything one rule reported about one scope.
#[derive(Clone, Debug, Default, PartialEq)]
struct Observed {
    findings: Vec<Reported>,
    /// Not-evaluated outcomes: reason and message, in report order.
    open: Vec<(NotEvaluatedReason, String)>,
    values: BTreeMap<String, Measure>,
}

impl Observed {
    /// The verdict its outcomes reach: a finding (the most severe, exact
    /// only when every finding is) outranks an open outcome, which keeps
    /// its first reason. `None` when it reported none.
    fn reported(&self) -> Option<Outcome> {
        if let Some(first) = self.findings.first() {
            let severity = self
                .findings
                .iter()
                .map(|finding| finding.severity.clone())
                // `Severity` orders the most severe first.
                .min()
                .unwrap_or_else(|| first.severity.clone());
            let exact = self.findings.iter().all(|finding| finding.exact);
            return Some(Outcome::Finding { severity, exact });
        }
        self.open.first().map(|(reason, _)| Outcome::NotEvaluated {
            reason: reason.clone(),
        })
    }
}

/// How a rule's selection judged an object it reported nothing about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selected {
    Passed,
    Undecided,
}

/// Everything one side of a comparison reported: one rule of a report,
/// one capability evaluation, or several merged (a capability rewritten as
/// one rule per check or severity band).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observations {
    scopes: BTreeMap<Scope, Observed>,
    /// The objects the rule selected without reporting about them, when
    /// its selection is known: every other object was not selected.
    selection: Option<BTreeMap<ObjectId, Selected>>,
    /// The rule's summary, when the report carries one.
    summary: Option<RuleFacts>,
}

/// How a rule fared as a whole, as its report summary states it: how many
/// objects it checked, and its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFacts {
    /// How many objects its selection decided it checks.
    pub checked: usize,
    /// How it fared.
    pub status: RuleStatus,
}

impl fmt::Display for RuleFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} checked, {:?}", self.checked, self.status)
    }
}

impl Observations {
    /// What the rule `rule` of `report` reported: its findings, its
    /// not-evaluated outcomes and its tables, at every scope, and its rule
    /// summary when the run made one (`Runtime::with_rule_summaries`). Its
    /// selection is not known, so an object it reported nothing about
    /// passed or was not selected, which the comparison reads as one; the
    /// summaries still tell how many objects each rule checked.
    #[must_use]
    pub fn of_report(report: &Report, rule: &str) -> Self {
        let mut observations = Self {
            summary: report
                .rules()
                .iter()
                .find(|summary| summary.rule_id.to_string() == rule)
                .map(|summary| RuleFacts {
                    checked: summary.checked,
                    status: summary.status,
                }),
            ..Self::default()
        };
        for finding in report
            .findings()
            .iter()
            .filter(|finding| finding.rule_id.to_string() == rule)
        {
            observations
                .at(&finding.scope)
                .findings
                .push(Reported::of(finding, None));
        }
        for outcome in report
            .not_evaluated()
            .iter()
            .filter(|outcome| outcome.rule_id.to_string() == rule)
        {
            observations
                .at(&outcome.scope)
                .open
                .push((outcome.reason.clone(), outcome.message.clone()));
        }
        for table in report
            .tables()
            .iter()
            .filter(|table| table.rule_id().to_string() == rule)
        {
            observations.tabulate(table);
        }
        observations
    }

    /// What the rule `rule` of `report` reported, as [`Self::of_report`]
    /// reads it, with its selection from the run's `outcomes`
    /// ([`axioval_engine::Runtime::run_session_recorded`]): an object it
    /// selected and reported nothing about passed, and every object it
    /// neither selected nor reported about was not selected. A rule
    /// missing from `outcomes` (it did not run) leaves the selection
    /// unknown.
    #[must_use]
    pub fn of_recorded(report: &Report, outcomes: &RuleOutcomes, rule: &str) -> Self {
        let mut observations = Self::of_report(report, rule);
        if let Some(record) = outcomes.get(rule) {
            let named: BTreeSet<&ObjectId> = record.named().collect();
            let mut selection = BTreeMap::new();
            for object in named {
                let selected = match record.object_verdict(object) {
                    ObjectVerdict::Passed => Selected::Passed,
                    ObjectVerdict::Undecided(_) => Selected::Undecided,
                    ObjectVerdict::Failed | ObjectVerdict::NotSelected => continue,
                };
                selection.insert(object.clone(), selected);
            }
            observations.selection = Some(selection);
        }
        observations
    }

    /// What one capability evaluation reported, as [`Self::of_report`]
    /// reads a rule of a report, with each graded finding's deviation.
    /// Its selection is not known; see [`Self::selecting`].
    #[must_use]
    pub fn of_evaluation(evaluation: &CapabilityEvaluation) -> Self {
        let mut observations = Self::default();
        for (index, finding) in evaluation.findings().iter().enumerate() {
            let deviation = evaluation
                .deviation(index)
                .map(|deviation| (deviation.lower(), deviation.upper()));
            observations
                .at(&finding.scope)
                .findings
                .push(Reported::of(finding, deviation));
        }
        for outcome in evaluation.not_evaluated_outcomes() {
            observations
                .at(outcome.scope())
                .open
                .push((outcome.reason().clone(), outcome.message().to_owned()));
        }
        for table in evaluation.tables() {
            observations.tabulate(table);
        }
        observations
    }

    /// The same observations, knowing the rule selected exactly the
    /// objects `selected` (and those it reported about): an object it
    /// reported nothing about passed when selected, and was not selected
    /// otherwise.
    #[must_use]
    pub fn selecting(mut self, selected: impl IntoIterator<Item = ObjectId>) -> Self {
        let selection = self.selection.get_or_insert_with(BTreeMap::new);
        for object in selected {
            selection.entry(object).or_insert(Selected::Passed);
        }
        self
    }

    /// The same observations with the measured value `name` of `scope`:
    /// a value the caller measured itself, such as a measured value the
    /// re-expression reads, or a measurement the capability makes. A value
    /// of that name already observed is replaced.
    #[must_use]
    pub fn with_value(
        mut self,
        scope: impl Into<Scope>,
        name: impl Into<String>,
        value: Measure,
    ) -> Self {
        self.at(&scope.into()).values.insert(name.into(), value);
        self
    }

    /// The same observations, keeping only the measured values `keep`
    /// accepts by scope and name: a value compared only where both sides
    /// measure it by contract, such as the levels a check judges.
    #[must_use]
    pub fn retain_values(mut self, keep: impl Fn(&Scope, &str) -> bool) -> Self {
        for (scope, observed) in &mut self.scopes {
            observed.values.retain(|name, _| keep(scope, name));
        }
        self
    }

    /// Both sides' observations as one: a capability rewritten as several
    /// rules (one per check or severity band) is compared with their
    /// observations merged. An object either selected without reporting
    /// passed; the selection is known when either knows it. Several rules
    /// have no one summary, so the merge carries none.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        self.summary = None;
        for (scope, observed) in other.scopes {
            let own = self.at(&scope);
            own.findings.extend(observed.findings);
            own.open.extend(observed.open);
            for (name, value) in observed.values {
                own.values.entry(name).or_insert(value);
            }
        }
        if let Some(theirs) = other.selection {
            let selection = self.selection.get_or_insert_with(BTreeMap::new);
            for (object, selected) in theirs {
                let entry = selection.entry(object).or_insert(selected);
                if selected == Selected::Undecided {
                    *entry = Selected::Undecided;
                }
            }
        }
        self
    }

    /// How the rule judged `scope`: what it reported, else, for an object
    /// where its selection is known, whether it passed, was not selected or
    /// is undecided. `None` when it reported nothing and its selection is
    /// not known, and for a source or the project it reported nothing
    /// about.
    #[must_use]
    pub fn outcome(&self, scope: &Scope) -> Option<Outcome> {
        if let Some(outcome) = self.scopes.get(scope).and_then(Observed::reported) {
            return Some(outcome);
        }
        // A rule selects objects, never a source or the project: about
        // those it either reported or did not.
        let object = scope.object()?;
        Some(match self.selection.as_ref()?.get(object) {
            Some(Selected::Passed) => Outcome::Passed,
            Some(Selected::Undecided) => Outcome::Undecided,
            None => Outcome::NotSelected,
        })
    }

    /// Every scope it reported about or selected.
    fn scopes(&self) -> impl Iterator<Item = Scope> + '_ {
        self.scopes.keys().cloned().chain(
            self.selection
                .iter()
                .flat_map(BTreeMap::keys)
                .cloned()
                .map(Scope::Object),
        )
    }

    fn at(&mut self, scope: &Scope) -> &mut Observed {
        self.scopes.entry(scope.clone()).or_default()
    }

    /// Each row's numeric and text cells as values named
    /// `<table>.<column>`, a grouped row's as `<table>[<group>].<column>`.
    fn tabulate(&mut self, table: &ReportTable) {
        for row in table.rows() {
            let prefix = if row.group().is_empty() {
                table.name().to_owned()
            } else {
                format!("{}[{}]", table.name(), row.group().join(", "))
            };
            for (column, value) in table.columns().iter().zip(row.values()) {
                let unit = column.unit_symbol().unwrap_or_default();
                self.at(row.scope()).values.insert(
                    format!("{prefix}.{}", column.id),
                    Measure::of_cell(value, &unit, column.exactness),
                );
            }
        }
    }
}

/// An aspect of a scope's findings a comparison may hold to parity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Aspect {
    /// How many findings each scope has.
    Counts,
    /// The findings' categories.
    Categories,
    /// The findings' and not-evaluated outcomes' messages, word for word.
    Messages,
    /// The objects each finding relates.
    Related,
}

/// What must agree between the two sides of a comparison, beyond what
/// always must: every scope's verdict, the most severe finding's severity,
/// evidence exactness, and the not-evaluated reason; passed and not
/// selected wherever both sides know their selection.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parity {
    /// The aspects of each scope's findings compared.
    pub aspects: BTreeSet<Aspect>,
    /// The greatest graded deviation of each scope's findings, within this
    /// rounding; `None` leaves deviations uncompared.
    pub deviations: Option<f64>,
    /// The measured values compared, by name, each within its rounding:
    /// the bounds of the two intervals may differ by at most the step.
    pub values: BTreeMap<String, f64>,
}

impl Parity {
    /// What an expression rewrite must reproduce of the capability it
    /// rewrites: every scope's verdict, severity, exactness and reason,
    /// and each scope's finding count and categories. An expression words
    /// its own findings and relates no other objects, so messages and
    /// related objects are not compared.
    #[must_use]
    pub fn outcomes() -> Self {
        Self {
            aspects: BTreeSet::from([Aspect::Counts, Aspect::Categories]),
            deviations: None,
            values: BTreeMap::new(),
        }
    }

    /// The whole outside contract a template standing in for a built-in
    /// capability must reproduce: [`Self::outcomes`], the messages and
    /// related objects, and the graded deviations to a few units in the
    /// last place.
    #[must_use]
    pub fn contract() -> Self {
        Self {
            aspects: BTreeSet::from([
                Aspect::Counts,
                Aspect::Categories,
                Aspect::Messages,
                Aspect::Related,
            ]),
            deviations: Some(0.0),
            values: BTreeMap::new(),
        }
    }

    /// The same comparison without finding counts: a rewrite judging each
    /// object once where the capability reports each failed check.
    #[must_use]
    pub fn uncounted(mut self) -> Self {
        self.aspects.remove(&Aspect::Counts);
        self
    }

    /// The same comparison, also comparing the measured value `name` of
    /// every scope within `step` (in the value's unit): a declared
    /// rounding of `0.0` allows only a few units in the last place.
    #[must_use]
    pub fn value(mut self, name: impl Into<String>, step: f64) -> Self {
        self.values.insert(name.into(), step);
        self
    }

    /// Lines the two sides up scope by scope: every scope either reported
    /// about or selected, its outcome on each side, and what else differs
    /// where the outcomes agree. Measured values are compared whatever the
    /// outcomes.
    #[must_use]
    pub fn compare(
        &self,
        (capability, left): (&str, &Observations),
        (expression, right): (&str, &Observations),
    ) -> ParityEvidence {
        let mut scopes: BTreeSet<Scope> = left.scopes().chain(right.scopes()).collect();
        // Rules that checked a different number of objects, or fared
        // differently as a whole, differ about the project.
        let rule = match (left.summary, right.summary) {
            (Some(own), Some(theirs)) if own != theirs => Some(Detail::Rule {
                capability: own,
                expression: theirs,
            }),
            _ => None,
        };
        if rule.is_some() {
            scopes.insert(Scope::Project);
        }
        let mut differences = Vec::new();
        let (mut found, mut open, mut values) = (0, 0, 0);
        let empty = Observed::default();
        for scope in &scopes {
            let (own, theirs) = (left.outcome(scope), right.outcome(scope));
            match own {
                Some(Outcome::Finding { .. }) => found += 1,
                Some(Outcome::NotEvaluated { .. }) => open += 1,
                _ => {}
            }
            let (own, theirs) = comparable(own, theirs);
            let observed = (
                left.scopes.get(scope).unwrap_or(&empty),
                right.scopes.get(scope).unwrap_or(&empty),
            );
            let mut details = if own == theirs {
                self.details(observed)
            } else {
                Vec::new()
            };
            if *scope == Scope::Project {
                details.extend(rule.clone());
            }
            for (name, step) in &self.values {
                let pair = (observed.0.values.get(name), observed.1.values.get(name));
                if pair == (None, None) {
                    continue;
                }
                values += 1;
                let agree = match pair {
                    (Some(a), Some(b)) => a.agrees(b, *step),
                    _ => false,
                };
                if !agree {
                    details.push(Detail::Value {
                        name: name.clone(),
                        capability: pair.0.cloned(),
                        expression: pair.1.cloned(),
                        step: *step,
                    });
                }
            }
            if own != theirs || !details.is_empty() {
                differences.push(Difference {
                    scope: scope.clone(),
                    capability: own,
                    expression: theirs,
                    details,
                });
            }
        }
        ParityEvidence {
            capability: capability.to_owned(),
            expression: expression.to_owned(),
            objects: scopes.len(),
            found,
            open,
            values,
            differences,
        }
    }

    /// Compares two capability evaluations run on the same model, read as
    /// [`Observations::of_evaluation`] reads them.
    #[must_use]
    pub fn compare_evaluations(
        &self,
        (capability, evaluated): (&str, &CapabilityEvaluation),
        (expression, rewritten): (&str, &CapabilityEvaluation),
    ) -> ParityEvidence {
        self.compare(
            (capability, &Observations::of_evaluation(evaluated)),
            (expression, &Observations::of_evaluation(rewritten)),
        )
    }

    /// What differs between two scopes whose outcomes agree.
    fn details(&self, (left, right): (&Observed, &Observed)) -> Vec<Detail> {
        let mut details = Vec::new();
        let sorted = |mut items: Vec<String>| {
            items.sort();
            items
        };
        if self.aspects.contains(&Aspect::Counts) && left.findings.len() != right.findings.len() {
            details.push(Detail::Count {
                capability: left.findings.len(),
                expression: right.findings.len(),
            });
        }
        if self.aspects.contains(&Aspect::Categories) {
            let categories = |observed: &Observed| {
                sorted(
                    observed
                        .findings
                        .iter()
                        .filter(|finding| !finding.categories.is_empty())
                        .map(|finding| finding.categories.join(" / "))
                        .collect(),
                )
            };
            let (own, theirs) = (categories(left), categories(right));
            if own != theirs {
                details.push(Detail::Categories {
                    capability: own,
                    expression: theirs,
                });
            }
        }
        if self.aspects.contains(&Aspect::Messages) {
            let messages = |observed: &Observed| {
                sorted(
                    observed
                        .findings
                        .iter()
                        .map(|finding| finding.message.clone())
                        .chain(observed.open.iter().map(|(_, message)| message.clone()))
                        .collect(),
                )
            };
            let (own, theirs) = (messages(left), messages(right));
            if own != theirs {
                details.push(Detail::Messages {
                    capability: own,
                    expression: theirs,
                });
            }
        }
        if self.aspects.contains(&Aspect::Related) {
            let related = |observed: &Observed| {
                sorted(
                    observed
                        .findings
                        .iter()
                        .filter(|finding| !finding.related.is_empty())
                        .map(|finding| {
                            finding
                                .related
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .collect(),
                )
            };
            let (own, theirs) = (related(left), related(right));
            if own != theirs {
                details.push(Detail::Related {
                    capability: own,
                    expression: theirs,
                });
            }
        }
        if let Some(step) = self.deviations {
            let greatest = |observed: &Observed| {
                observed
                    .findings
                    .iter()
                    .filter_map(|finding| finding.deviation)
                    .reduce(|(a, b), (c, d)| (a.max(c), b.max(d)))
                    .map(|(lower, upper)| Measure::interval(lower, upper, ""))
            };
            let (own, theirs) = (greatest(left), greatest(right));
            let agree = match (&own, &theirs) {
                (Some(a), Some(b)) => a.agrees(b, step),
                (None, None) => true,
                _ => false,
            };
            if !agree {
                details.push(Detail::Deviation {
                    capability: own,
                    expression: theirs,
                    step,
                });
            }
        }
        details
    }
}

/// The two outcomes as they can be compared: passed and not selected are
/// told apart only where both sides know their selection; where one does
/// not, either reads as "reported nothing".
fn comparable(own: Option<Outcome>, theirs: Option<Outcome>) -> (Option<Outcome>, Option<Outcome>) {
    let silent = |outcome: &Option<Outcome>| {
        matches!(outcome, None | Some(Outcome::Passed | Outcome::NotSelected))
    };
    if (own.is_none() || theirs.is_none()) && silent(&own) && silent(&theirs) {
        return (None, None);
    }
    (own, theirs)
}

/// What else differs about a scope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "aspect", rename_all = "camelCase")]
pub enum Detail {
    /// How many findings.
    Count {
        /// The capability's.
        capability: usize,
        /// The re-expression's.
        expression: usize,
    },
    /// The findings' categories, each finding's levels joined by ` / `,
    /// sorted.
    Categories {
        /// The capability's.
        capability: Vec<String>,
        /// The re-expression's.
        expression: Vec<String>,
    },
    /// The findings' and not-evaluated outcomes' messages, sorted.
    Messages {
        /// The capability's.
        capability: Vec<String>,
        /// The re-expression's.
        expression: Vec<String>,
    },
    /// The objects each finding relating any relates, sorted.
    Related {
        /// The capability's.
        capability: Vec<String>,
        /// The re-expression's.
        expression: Vec<String>,
    },
    /// The greatest graded deviation.
    Deviation {
        /// The capability's; `None` when ungraded.
        capability: Option<Measure>,
        /// The re-expression's.
        expression: Option<Measure>,
        /// The rounding they had to agree within.
        step: f64,
    },
    /// The rule as a whole: how many objects it checked, and its status.
    Rule {
        /// The capability rule's.
        capability: RuleFacts,
        /// The re-expression's.
        expression: RuleFacts,
    },
    /// A measured value.
    Value {
        /// Its name.
        name: String,
        /// The capability's; `None` when it has none.
        capability: Option<Measure>,
        /// The re-expression's.
        expression: Option<Measure>,
        /// The rounding they had to agree within.
        step: f64,
    },
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let measure = |value: &Option<Measure>| {
            value
                .as_ref()
                .map_or_else(|| "none".to_owned(), ToString::to_string)
        };
        match self {
            Self::Count {
                capability,
                expression,
            } => write!(f, "{capability} findings against {expression}"),
            Self::Categories {
                capability,
                expression,
            } => write!(f, "categories {capability:?} against {expression:?}"),
            Self::Messages {
                capability,
                expression,
            } => write!(f, "messages {capability:?} against {expression:?}"),
            Self::Related {
                capability,
                expression,
            } => write!(f, "related {capability:?} against {expression:?}"),
            Self::Deviation {
                capability,
                expression,
                step,
            } => write!(
                f,
                "deviation {} against {} (within {step})",
                measure(capability),
                measure(expression)
            ),
            Self::Rule {
                capability,
                expression,
            } => write!(f, "rule {capability} against {expression}"),
            Self::Value {
                name,
                capability,
                expression,
                step,
            } => write!(
                f,
                "`{name}` {} against {} (within {step})",
                measure(capability),
                measure(expression)
            ),
        }
    }
}

/// One scope two sides judged differently.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    /// The object, source or project judged differently.
    #[serde(serialize_with = "scope_text")]
    pub scope: Scope,
    /// The capability's outcome; `None` when it reported nothing and
    /// passed and not selected cannot be told apart.
    pub capability: Option<Outcome>,
    /// The re-expression's outcome.
    pub expression: Option<Outcome>,
    /// What else differs: where the outcomes agree, every compared aspect
    /// that does not; and every measured value that does not agree.
    pub details: Vec<Detail>,
}

fn scope_text<S: Serializer>(scope: &Scope, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(scope)
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = |outcome: &Option<Outcome>| {
            outcome
                .as_ref()
                .map_or_else(|| "reported nothing".to_owned(), ToString::to_string)
        };
        write!(
            f,
            "{}: capability {}, expression {}",
            self.scope,
            shown(&self.capability),
            shown(&self.expression)
        )?;
        for detail in &self.details {
            write!(f, "; {detail}")?;
        }
        Ok(())
    }
}

/// What one parity check compared and found.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityEvidence {
    /// The capability rule's id.
    pub capability: String,
    /// The re-expression's rule id.
    pub expression: String,
    /// How many scopes (objects, sources, the project) either side
    /// reported about or selected.
    pub objects: usize,
    /// How many scopes the capability found.
    pub found: usize,
    /// How many scopes the capability left open.
    pub open: usize,
    /// How many measured values were compared.
    pub values: usize,
    /// Every scope judged differently; empty at parity.
    pub differences: Vec<Difference>,
}

impl ParityEvidence {
    /// Whether the two sides judged every scope alike.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.differences.is_empty()
    }

    /// The differences, one line each, for a failing test.
    #[must_use]
    pub fn diff(&self) -> String {
        self.differences
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Compares how the rule `capability` and the rule `expression` of
/// `report` judged every scope, under [`Parity::outcomes`].
#[must_use]
pub fn compare(report: &Report, capability: &str, expression: &str) -> ParityEvidence {
    Parity::outcomes().compare(
        (capability, &Observations::of_report(report, capability)),
        (expression, &Observations::of_report(report, expression)),
    )
}

/// Compares the two rules as [`compare`] does, telling passed from not
/// selected through the run's recorded `outcomes`.
#[must_use]
pub fn compare_recorded(
    report: &Report,
    outcomes: &RuleOutcomes,
    capability: &str,
    expression: &str,
) -> ParityEvidence {
    Parity::outcomes().compare(
        (
            capability,
            &Observations::of_recorded(report, outcomes, capability),
        ),
        (
            expression,
            &Observations::of_recorded(report, outcomes, expression),
        ),
    )
}

/// Compares how a capability's evaluation and its rewrite's, run on the
/// same model, judged every scope, under [`Parity::outcomes`]: the harness
/// over capabilities run directly, as their fixture tests run them.
#[must_use]
pub fn compare_evaluations(
    (capability, evaluated): (&str, &CapabilityEvaluation),
    (expression, rewritten): (&str, &CapabilityEvaluation),
) -> ParityEvidence {
    Parity::outcomes().compare_evaluations((capability, evaluated), (expression, rewritten))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::{Evidence, RuleId, SourceId};

    fn source() -> SourceId {
        SourceId::new("test", "model").unwrap()
    }

    fn object(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn finding(local: &str, message: &str) -> Finding {
        Finding::new(
            RuleId::new("r").unwrap(),
            object(local),
            Severity::Error,
            message,
        )
        .with_evidence([Evidence::exact(source(), "#1")])
    }

    fn evaluation(findings: Vec<Finding>) -> Observations {
        Observations::of_evaluation(&CapabilityEvaluation::evaluated(findings))
    }

    #[test]
    fn passed_and_not_selected_are_told_apart_where_both_know_their_selection() {
        let passed = evaluation(vec![]).selecting([object("a")]);
        let unselected = evaluation(vec![]).selecting([]);
        let parity = Parity::outcomes().compare(("c", &passed), ("e", &unselected));
        assert_eq!(parity.differences.len(), 1, "{}", parity.diff());
        assert_eq!(
            parity.diff(),
            "test:model/a: capability passed, expression not selected"
        );
        // Unknown on one side: both read as reported nothing.
        let unknown = evaluation(vec![]);
        assert!(
            Parity::outcomes()
                .compare(("c", &passed), ("e", &unknown))
                .holds()
        );
    }

    #[test]
    fn source_and_project_outcomes_are_compared() {
        let mut whole = CapabilityEvaluation::default();
        whole.push_finding(Finding::new(
            RuleId::new("r").unwrap(),
            source(),
            Severity::Warning,
            "no building",
        ));
        let parity = Parity::outcomes().compare(
            ("c", &Observations::of_evaluation(&whole)),
            ("e", &evaluation(vec![])),
        );
        assert_eq!(
            parity.diff(),
            "source test:model: capability finding (Warning, exact evidence), \
             expression reported nothing"
        );
    }

    #[test]
    fn counts_messages_and_values_are_compared_where_declared() {
        let capability = evaluation(vec![finding("a", "too high"), finding("a", "too wide")])
            .with_value(object("a"), "height", Measure::exact(3.0, "m"));
        let rewrite = evaluation(vec![finding("a", "requirement does not hold")]).with_value(
            object("a"),
            "height",
            Measure::interval(2.9995, 3.0004, "m"),
        );
        let outcomes = Parity::outcomes().compare(("c", &capability), ("e", &rewrite));
        assert_eq!(outcomes.differences[0].details.len(), 1);
        assert!(outcomes.diff().ends_with("; 2 findings against 1"));
        assert!(
            Parity::outcomes()
                .uncounted()
                .compare(("c", &capability), ("e", &rewrite))
                .holds()
        );
        // Within a millimetre, not within a tenth of one.
        let coarse = Parity::outcomes()
            .uncounted()
            .value("height", 0.001)
            .compare(("c", &capability), ("e", &rewrite));
        assert!(coarse.holds(), "{}", coarse.diff());
        assert_eq!(coarse.values, 1);
        let fine = Parity::outcomes()
            .uncounted()
            .value("height", 0.0001)
            .compare(("c", &capability), ("e", &rewrite));
        assert_eq!(
            fine.diff(),
            "test:model/a: capability finding (Error, exact evidence), expression finding \
             (Error, exact evidence); `height` 3 m (exact) against 2.9995..3.0004 m (within 0.0001)"
        );
        let contract = Parity::contract().compare(("c", &capability), ("e", &rewrite));
        assert!(matches!(
            contract.differences[0].details[..],
            [Detail::Count { .. }, Detail::Messages { .. }]
        ));
    }

    #[test]
    fn null_and_not_evaluated_values_never_agree() {
        assert!(!Measure::Null.agrees(&Measure::NotEvaluated, 1.0));
        assert!(Measure::Null.agrees(&Measure::Null, 0.0));
        assert!(!Measure::exact(1.0, "m").agrees(&Measure::exact(1.0, "m²"), 1.0));
        assert!(
            !Measure::exact(1.0, "m").agrees(
                &Measure::of_property(&PropertyValue::Measured {
                    lower: 1.0,
                    upper: 1.0,
                    dimension: Some(axioval_ir::QuantityDimension::Length),
                })
                .unwrap(),
                0.0
            ),
            "exact against inexact"
        );
    }

    #[test]
    fn evidence_serializes_scopes_as_text() {
        let parity = Parity::outcomes().compare(
            ("c", &evaluation(vec![finding("a", "x")])),
            ("e", &evaluation(vec![])),
        );
        let json = serde_json::to_value(&parity).unwrap();
        assert_eq!(json["differences"][0]["scope"], "test:model/a");
        assert_eq!(
            json["differences"][0]["expression"],
            serde_json::Value::Null
        );
        assert_eq!(json["objects"], 1);
    }
}
