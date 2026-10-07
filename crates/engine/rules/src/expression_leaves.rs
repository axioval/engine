//! The leaves of an expression evaluated for one object of a rule: its
//! properties, derived values, and the rule's parameters and tables.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::expression::{
    EvaluationBudget, ExpressionContext, Interval, Leaf, Member, RuleRead, Unit, Value,
    derived_value, evaluate, evaluate_untraced,
};
use axioval_engine::{
    MeasuredMember, MeasuredRead, Measurement, MemberValue, ObjectVerdict, RuleContext,
    RuleOutcomes,
};
use axioval_ir::contract::{
    AggregateSource, Expression, ParameterValue, ScalarValue, Selector, TableRow,
};
use axioval_ir::{Evidence, NotEvaluatedReason, Object, ObjectId};

use crate::measured_arguments::{Arguments, bind};

use crate::selection::{Selection, object_by_id, selector_matches};
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{PropertyRef, Resolved, Traversal, resolve};

/// A stated property read: its set, its name and its value as stated.
type StatedRead = (
    Option<Arc<str>>,
    Arc<str>,
    Option<axioval_ir::PropertyValue>,
);

/// What the source states for a property: its value, or `None` where it
/// states the property absent.
#[derive(Clone, Debug)]
pub(crate) struct Stated(pub(crate) Option<axioval_ir::PropertyValue>);

/// A candidate member: the object, whether it surely belongs, and the
/// evidence that reached it.
pub(crate) type Candidate<'a> = (&'a Object, bool, Vec<Evidence>);

/// A measured value of the object in scope read ahead of its read, by set
/// and name: what [`resolve`] would answer for it, read for many objects
/// together through the run's `MeasuredValues`.
pub(crate) type Prefetched = (
    Option<Arc<str>>,
    Arc<str>,
    Result<MeasuredRead, (NotEvaluatedReason, String)>,
);

/// An object's values read ahead, on the heap: the object's leaves and
/// judgement move them several times, which an inline list would copy.
pub(crate) type Prefetch = Vec<Prefetched>;

/// A measured value naming the rule's parameters, read ahead with its
/// arguments bound, by its name as written.
pub(crate) type BoundPrefetched = (
    Arc<str>,
    Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)>,
);

/// Answers an expression's leaves for one selected object.
pub(crate) struct ObjectLeaves<'a> {
    context: &'a RuleContext<'a>,
    /// The run's evaluation budget, looked up once for the object.
    budget: Option<&'a Arc<EvaluationBudget>>,
    object: &'a Object,
    /// The rule's checked object: `object`, except in an aggregate's
    /// member scope.
    subject: &'a Object,
    /// The rule's parameters; a selector reads none.
    parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    /// Why each unreadable leaf was unreadable, in reading order.
    reasons: RefCell<smallvec::SmallVec<[NotEvaluatedReason; 2]>>,
    /// Every stated property read, by set and name, as the source states
    /// it (`None` where it states the property absent): a template words a
    /// value that is not of the kind it needs as the source states it.
    stated: RefCell<smallvec::SmallVec<[StatedRead; 4]>>,
    /// The measured member in scope, whose fields `axioval:member` reads.
    fields: Option<&'a MeasuredMember>,
    /// The evidence of the measured member list last listed.
    listed: Vec<Evidence>,
    /// Properties of the object resolved ahead in a batch, each read once.
    prefetched: Prefetch,
    /// Measured values naming the rule's parameters, read ahead in a batch.
    bound: Vec<BoundPrefetched>,
    /// What the rule's measured values bind, read once per rule.
    arguments: Option<&'a Arguments>,
    /// The objects measured values bound from the rule were measured
    /// against, as their providers cite them, in reading order.
    related: RefCell<Vec<ObjectId>>,
    /// What the measured values bound from the rule noted, as their
    /// providers cite it, in reading order.
    notes: RefCell<Vec<String>>,
    /// The sources they were measured against (a reference source), as
    /// their providers cite them, in reading order.
    sources: RefCell<Vec<axioval_ir::SourceId>>,
    /// Members the caller states for one aggregate source in place of
    /// reaching them: a template's members of an anchor, or the objects
    /// selected in a scope.
    supplied: Vec<(AggregateSource, Vec<Candidate<'a>>)>,
    /// Member lists a template's checks read, by their name as written,
    /// each measured once for the object.
    lists: RefCell<Vec<(Arc<str>, Arc<Listed>)>>,
    /// Properties of the object resolved now, each resolved once however
    /// often an expression reads it, where `cached`.
    resolved: RefCell<Vec<ResolvedRead>>,
    /// Whether properties are kept once resolved: where values compose
    /// reads of what other values read.
    cached: bool,
    /// Measured values naming the rule's parameters read for an object,
    /// each measured once however often the object's values read it.
    measured: RefCell<Vec<(ObjectId, Arc<str>, BoundPrefetched)>>,
}

/// A property resolved for an object (the object in scope, or the subject
/// read through it): the object, its set and name, and what the source
/// states (`None`: absent) with its evidence, or why it cannot be read.
type ResolvedRead = (
    ObjectId,
    Option<Arc<str>>,
    Arc<str>,
    Result<(Option<axioval_ir::PropertyValue>, Vec<Evidence>), (NotEvaluatedReason, String)>,
);

/// A measured member list as a template's checks read it: its members and
/// the evidence of the measurement they come from, or why it cannot be
/// measured.
pub(crate) type Listed = Result<(Vec<MeasuredMember>, Vec<Evidence>), (NotEvaluatedReason, String)>;

impl<'a> ObjectLeaves<'a> {
    /// Leaves of `object`, reading the rule's `parameters` where given.
    pub(crate) fn new(
        context: &'a RuleContext<'a>,
        object: &'a Object,
        parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    ) -> Self {
        Self {
            context,
            budget: context.services.get::<Arc<EvaluationBudget>>(),
            object,
            subject: object,
            parameters,
            reasons: RefCell::new(smallvec::SmallVec::new()),
            stated: RefCell::new(smallvec::SmallVec::new()),
            resolved: RefCell::new(Vec::new()),
            cached: false,
            measured: RefCell::new(Vec::new()),
            fields: None,
            listed: Vec::new(),
            prefetched: Prefetch::new(),
            supplied: Vec::new(),
            bound: Vec::new(),
            arguments: None,
            related: RefCell::new(Vec::new()),
            notes: RefCell::new(Vec::new()),
            sources: RefCell::new(Vec::new()),
            lists: RefCell::new(Vec::new()),
        }
    }

    /// The same leaves, an aggregate over `over` reading `members` (each
    /// with whether it surely belongs and the evidence that reached it)
    /// instead of the objects `over` reaches.
    pub(crate) fn supplying(mut self, over: AggregateSource, members: Vec<Candidate<'a>>) -> Self {
        self.supplied.push((over, members));
        self
    }

    /// The leaves of `member`, in an aggregate's member scope: the same
    /// subject and parameters.
    fn member(&self, member: &'a Object) -> Self {
        Self {
            context: self.context,
            budget: self.budget,
            object: member,
            subject: self.subject,
            parameters: self.parameters,
            reasons: RefCell::new(smallvec::SmallVec::new()),
            stated: RefCell::new(smallvec::SmallVec::new()),
            resolved: RefCell::new(Vec::new()),
            cached: false,
            measured: RefCell::new(Vec::new()),
            fields: None,
            listed: Vec::new(),
            prefetched: Prefetch::new(),
            supplied: Vec::new(),
            bound: Vec::new(),
            arguments: self.arguments,
            related: RefCell::new(Vec::new()),
            notes: RefCell::new(Vec::new()),
            sources: RefCell::new(Vec::new()),
            lists: RefCell::new(Vec::new()),
        }
    }

    /// The leaves of a measured member of the object in scope: the same
    /// object, its fields read in `axioval:member`.
    fn measured_member(&self, member: &'a MeasuredMember) -> Self {
        Self {
            context: self.context,
            budget: self.budget,
            object: self.object,
            subject: self.subject,
            parameters: self.parameters,
            reasons: RefCell::new(smallvec::SmallVec::new()),
            stated: RefCell::new(smallvec::SmallVec::new()),
            resolved: RefCell::new(Vec::new()),
            cached: false,
            measured: RefCell::new(Vec::new()),
            fields: Some(member),
            listed: Vec::new(),
            prefetched: Prefetch::new(),
            supplied: Vec::new(),
            bound: Vec::new(),
            arguments: self.arguments,
            related: RefCell::new(Vec::new()),
            notes: RefCell::new(Vec::new()),
            sources: RefCell::new(Vec::new()),
            lists: RefCell::new(Vec::new()),
        }
    }

    /// The leaves of the object, with properties resolved ahead in a batch
    /// (`support::resolve_batch`): a read of one takes its answer instead
    /// of resolving it again, and reads it exactly as a resolved one.
    pub(crate) fn with_prefetched(mut self, prefetched: Prefetch) -> Self {
        self.prefetched = prefetched;
        self
    }

    /// Leaves of `object` as a template judges it: properties and measured
    /// values naming the rule's parameters read ahead in a batch, binding
    /// through `arguments`, and, where `cached`, each property kept once
    /// resolved, for values that read again what other values read. Built
    /// in place, the leaves being large to move.
    pub(crate) fn read_ahead(
        context: &'a RuleContext<'a>,
        (object, parameters): (&'a Object, &'a BTreeMap<String, ParameterValue>),
        arguments: &'a Arguments,
        (prefetched, bound): (Prefetch, Vec<BoundPrefetched>),
        cached: bool,
    ) -> Self {
        let mut leaves = Self::new(context, object, Some(parameters));
        leaves.prefetched = prefetched;
        leaves.bound = bound;
        leaves.arguments = Some(arguments);
        leaves.cached = cached;
        leaves
    }

    /// The same leaves of a part of `subject`, the rule's checked object a
    /// measured value names as `@anchor`.
    pub(crate) fn with_subject(mut self, subject: &'a Object) -> Self {
        self.subject = subject;
        self
    }

    /// The same leaves, binding measured values' references through
    /// `arguments`, read once per rule.
    pub(crate) fn with_arguments(mut self, arguments: &'a Arguments) -> Self {
        self.arguments = Some(arguments);
        self
    }

    /// The member list `list` (written as a measured value is, `@`
    /// references bound to the rule's parameters and the anchor) measured
    /// of the object, once per object however many checks read it. A
    /// refusal states why without the list's name and the object.
    pub(crate) fn bound_members(&self, list: &str) -> Arc<Listed> {
        if let Some((_, listed)) = self
            .lists
            .borrow()
            .iter()
            .find(|(written, _)| &**written == list)
        {
            return Arc::clone(listed);
        }
        let listed = Arc::new(self.measure_members(list));
        self.lists
            .borrow_mut()
            .push((Arc::from(list), Arc::clone(&listed)));
        listed
    }

    fn measure_members(&self, list: &str) -> Listed {
        // A list naming no anchor is read as the plan keeps it, uncopied.
        let shared = self
            .arguments
            .and_then(|arguments| arguments.planned_list(self.context, self.parameters, list))
            .filter(|call| call.is_bound());
        let owned;
        let call: &axioval_ir::measured::MeasuredCall = if let Some(call) = &shared {
            call
        } else {
            let mut call = match self.anchored(list, true) {
                Some(call) => call?,
                None => axioval_ir::measured::parse_members(list)
                    .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?,
            };
            bind(
                self.context,
                self.parameters,
                self.arguments,
                &self.subject.id,
                &mut call,
            )?;
            owned = call;
            &owned
        };
        axioval_engine::measured_members_bound(self.context.services, &self.object.id, call)
            .map_err(|error| {
                let (reason, message) = crate::selection::property_error(error);
                let message = message
                    .strip_prefix("property evidence conflicts: ")
                    .unwrap_or(&message);
                let message = message
                    .strip_prefix("`")
                    .and_then(|rest| rest.strip_prefix(axioval_ir::MEASURED_SET))
                    .and_then(|rest| rest.strip_prefix("` value "))
                    .unwrap_or(message);
                // The list's own words: after `` `<name>` of <subject>: ``
                // (an object, or a source for a list of a source), matched
                // without formatting the subject.
                let why = message
                    .strip_prefix('`')
                    .and_then(|rest| rest.strip_prefix(call.name()))
                    .and_then(|rest| rest.strip_prefix("` of "))
                    .and_then(|rest| rest.split_once(": "))
                    .map_or(message, |(_, why)| why);
                (reason, why.to_owned())
            })
    }

    /// The measured name `name` (a list where `list`) parsed and the
    /// rule's parameters bound once per rule, the anchor left to bind:
    /// `None` outside a rule or where it does not parse or binds nothing.
    fn anchored(
        &self,
        name: &str,
        list: bool,
    ) -> Option<Result<axioval_ir::measured::MeasuredCall, (NotEvaluatedReason, String)>> {
        self.arguments?
            .anchored(self.context, self.parameters, name, list)
    }

    /// Whether the rule's selector parameter `parameter` leaves objects
    /// undecided, read once per rule; a selection that cannot be listed
    /// may leave any object undecided.
    pub(crate) fn leaves_undecided(&self, parameter: &str) -> bool {
        let Some(ParameterValue::Selector { value: selector }) = self
            .parameters
            .and_then(|parameters| parameters.get(parameter))
        else {
            return false;
        };
        let read = match self.arguments {
            Some(arguments) => arguments.selection_of(self.context, parameter, selector),
            None => crate::measured_arguments::selection_of(self.context, parameter, selector),
        };
        read.map_or(true, |selection| !selection.undecided.is_empty())
    }

    /// The objects the measured values read since the last call were
    /// measured against, as their providers cite them.
    pub(crate) fn take_related(&self) -> Vec<ObjectId> {
        std::mem::take(&mut *self.related.borrow_mut())
    }

    /// What the measured values read since the last call noted, as their
    /// providers cite it.
    pub(crate) fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.borrow_mut())
    }

    /// The measured value `name` (written as a rule writes it, any `@`
    /// reference bound) read with what it was measured against, which
    /// [`Self::take_related`] and [`Self::take_sources`] then give: as a
    /// template reads a value once per rule, whatever it names.
    pub(crate) fn measured_cited(&mut self, name: &str) -> Leaf {
        // Prepared once for the rule where it names no anchor.
        if let Some(bound) = self
            .arguments
            .and_then(|arguments| arguments.call(self.context, self.parameters, name))
        {
            return self.bound_measured_with(name, bound);
        }
        match axioval_ir::measured::parse(name) {
            Ok(call) => self.bound_measured(name, call),
            Err(error) => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::InvalidDeclaration);
                Leaf::unreadable(error.to_string())
            }
        }
    }

    /// The sources the measured values read since the last call were
    /// measured against, as their providers cite them.
    pub(crate) fn take_sources(&self) -> Vec<axioval_ir::SourceId> {
        std::mem::take(&mut *self.sources.borrow_mut())
    }

    /// The measured value `name` as `call` names it, its references bound
    /// and read now, through the run's measured values.
    fn read_bound(
        &self,
        name: &str,
        call: &mut axioval_ir::measured::MeasuredCall,
    ) -> Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)> {
        if let Err((reason, why)) = bind(
            self.context,
            self.parameters,
            self.arguments,
            &self.subject.id,
            call,
        ) {
            return Err((
                reason,
                format!("`{}` of {}: {why}", call.name(), self.object.id),
            ));
        }
        self.measure_bound(name, call)
    }

    /// The prepared read measured for the object.
    fn measure_prepared(
        &self,
        prepared: &axioval_engine::PreparedRead,
    ) -> Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)> {
        let measure = |values: &axioval_engine::MeasuredValues| {
            values
                .read_prepared(prepared, &[&self.object.id])
                .pop()
                .unwrap_or(Err(axioval_engine::PropertyResolutionError::InvalidRequest))
        };
        let read = match self
            .context
            .services
            .get::<axioval_engine::MeasuredValues>()
        {
            Some(values) => measure(values),
            None => measure(&axioval_engine::MeasuredValues::of(
                self.context.services,
                self.context.project,
            )),
        };
        read.map_err(crate::selection::property_error)
    }

    /// The bound `call` of `name` measured for the object.
    fn measure_bound(
        &self,
        name: &str,
        call: &axioval_ir::measured::MeasuredCall,
    ) -> Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)> {
        let measure = |values: &axioval_engine::MeasuredValues| {
            values
                .read_bound_batch(name, call, &[&self.object.id])
                .pop()
                .unwrap_or(Err(axioval_engine::PropertyResolutionError::InvalidRequest))
        };
        let read = match self
            .context
            .services
            .get::<axioval_engine::MeasuredValues>()
        {
            Some(values) => measure(values),
            None => measure(&axioval_engine::MeasuredValues::of(
                self.context.services,
                self.context.project,
            )),
        };
        read.map_err(crate::selection::property_error)
    }

    /// The measured value `name`, its references bound to the rule's
    /// parameters and the anchor (the rule's checked object) before it is
    /// measured through the run's measured values; one that cannot be
    /// bound leaves it unread for its reason.
    fn bound_measured(&mut self, name: &str, mut call: axioval_ir::measured::MeasuredCall) -> Leaf {
        let known = self
            .measured
            .borrow()
            .iter()
            .find(|(object, read, _)| *object == self.object.id && &**read == name)
            .map(|(_, _, (_, read))| read.clone());
        let read = if let Some(read) = known {
            read
        } else {
            let read = match self.bound.iter().position(|(read, _)| &**read == name) {
                Some(index) => self.bound.swap_remove(index).1,
                None => self.read_bound(name, &mut call),
            };
            self.measured.borrow_mut().push((
                self.object.id.clone(),
                Arc::from(name),
                (Arc::from(name), read.clone()),
            ));
            read
        };
        self.bound_leaf(name, read)
    }

    /// [`Self::bound_measured`] of a call the rule bound already, or its
    /// refusal to bind.
    fn bound_measured_with(
        &mut self,
        name: &str,
        bound: Result<axioval_engine::PreparedRead, (NotEvaluatedReason, String)>,
    ) -> Leaf {
        let (written, read) = match self.bound.iter().position(|(read, _)| &**read == name) {
            Some(index) => {
                // Kept for a value read again, as by several checks.
                let (written, read) = self.bound[index].clone();
                (Some(written), read)
            }
            None => match bound {
                Ok(prepared) => {
                    // Kept for a value read again (a truth composing it and
                    // the value worded), as one read ahead is: a provider
                    // keeping nothing for the run measures it once.
                    let read = self.measure_prepared(&prepared);
                    let written: Arc<str> = Arc::from(name);
                    self.bound.push((Arc::clone(&written), read.clone()));
                    (Some(written), read)
                }
                Err((reason, why)) => (
                    None,
                    Err((
                        reason,
                        format!(
                            "`{}` of {}: {why}",
                            name.split(';').next().unwrap_or(name),
                            self.object.id
                        ),
                    )),
                ),
            },
        };
        self.bound_leaf_named(name, written, read)
    }

    /// The measured value `name`, prepared once for the rule with its
    /// anchor left, read of the object, the anchor bound to the rule's
    /// checked object; or the rule's parameters' refusal to bind.
    fn anchored_measured(
        &mut self,
        name: &str,
        prepared: Result<axioval_engine::PreparedRead, (NotEvaluatedReason, String)>,
    ) -> Leaf {
        let read = match prepared {
            Ok(prepared) => {
                let measure = |values: &axioval_engine::MeasuredValues| {
                    values
                        .read_anchored(&prepared, &self.subject.id, &[&self.object.id])
                        .pop()
                        .unwrap_or(Err(axioval_engine::PropertyResolutionError::InvalidRequest))
                };
                match self
                    .context
                    .services
                    .get::<axioval_engine::MeasuredValues>()
                {
                    Some(values) => measure(values),
                    None => measure(&axioval_engine::MeasuredValues::of(
                        self.context.services,
                        self.context.project,
                    )),
                }
                .map_err(crate::selection::property_error)
            }
            Err((reason, why)) => Err((
                reason,
                format!(
                    "`{}` of {}: {why}",
                    name.split(';').next().unwrap_or(name),
                    self.object.id
                ),
            )),
        };
        self.bound_leaf(name, read)
    }

    /// The leaf of a measured value read with bound arguments.
    fn bound_leaf(
        &mut self,
        name: &str,
        read: Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)>,
    ) -> Leaf {
        self.bound_leaf_named(name, None, read)
    }

    /// [`Self::bound_leaf`], the name shared where it was read ahead.
    fn bound_leaf_named(
        &mut self,
        name: &str,
        written: Option<Arc<str>>,
        read: Result<axioval_engine::BoundRead, (NotEvaluatedReason, String)>,
    ) -> Leaf {
        let ((read, citation), name) = match read {
            Ok(read) => (read, name),
            Err((reason, message)) => {
                self.reasons.borrow_mut().push(reason);
                return Leaf::unreadable(message);
            }
        };
        let (stated, mut evidence): (_, Vec<Evidence>) = match read {
            MeasuredRead::Value(value, evidence) => (Some(value), evidence.into_iter().collect()),
            MeasuredRead::Absent(evidence) => (None, vec![evidence]),
        };
        evidence.extend(citation.evidence);
        self.related.borrow_mut().extend(citation.related);
        self.notes.borrow_mut().extend(citation.notes);
        self.sources.borrow_mut().extend(citation.sources);
        let value = match &stated {
            // A stated absence is `null`, never a value not read.
            None => Ok(Value::Null),
            Some(value) => Value::from_property(value),
        };
        self.stated.borrow_mut().push((
            Some(measured_set()),
            written.unwrap_or_else(|| Arc::from(name)),
            stated,
        ));
        if value.is_err() {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::InvalidEvidence);
        }
        Leaf { value, evidence }
    }

    /// The field `name` of the measured member in scope.
    #[allow(clippy::too_many_lines)]
    fn member_field(&self, name: &str) -> Leaf {
        let Some(member) = self.fields else {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::InvalidEvidence);
            return Leaf::unreadable(format!(
                "`{}` `{name}` is read only inside an aggregate over measured members",
                axioval_ir::MEMBER_SET
            ));
        };
        let source = self.object.id.source.clone();
        // Exact as the member states, unless the field itself is cited as
        // an approximation.
        let exact = member.exact
            && !matches!(
                member.fields.get(name),
                Some(MemberValue::Measured(Measurement::Cited {
                    exact: false,
                    ..
                }))
            );
        let cited = |locator: &str| {
            let locator = format!("{}/{name}:{locator}", axioval_ir::MEMBER_SET);
            vec![Evidence {
                source: source.clone(),
                locator,
                exact,
            }]
        };
        match member.fields.get(name) {
            None => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::InvalidEvidence);
                Leaf::unreadable(format!("the members state no `{name}`"))
            }
            Some(MemberValue::Undecided { why }) => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::IncompleteEvidence);
                Leaf::unreadable(format!("`{name}` is undecided: {why}"))
            }
            Some(MemberValue::Truth { value, locator }) => Leaf {
                value: Ok(Value::Boolean(*value)),
                evidence: cited(locator),
            },
            Some(MemberValue::Text { text }) => Leaf {
                value: Ok(Value::Text(text.clone())),
                evidence: cited(name),
            },
            Some(MemberValue::Objects { objects }) => Leaf {
                value: Ok(Value::Text(
                    objects
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                )),
                evidence: cited(name),
            },
            Some(MemberValue::Measured(Measurement::Absent { locator })) => Leaf {
                value: Ok(Value::Null),
                evidence: cited(locator),
            },
            Some(MemberValue::Measured(
                Measurement::Value {
                    lower,
                    upper,
                    dimension,
                    locator,
                }
                | Measurement::Rounded {
                    lower,
                    upper,
                    dimension,
                    locator,
                }
                | Measurement::Cited {
                    lower,
                    upper,
                    dimension,
                    locator,
                    ..
                },
            )) => {
                let value = Value::from_property(&axioval_ir::PropertyValue::Measured {
                    lower: *lower,
                    upper: *upper,
                    dimension: *dimension,
                });
                if value.is_err() {
                    self.reasons
                        .borrow_mut()
                        .push(NotEvaluatedReason::InvalidEvidence);
                }
                Leaf {
                    value,
                    evidence: cited(locator),
                }
            }
        }
    }

    /// The members of `list` measured of the object in scope, each
    /// evaluated with `value`.
    fn measured_members(
        &mut self,
        list: &str,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        // Parsed and its parameters bound once per rule where it can be.
        let anchored = self.anchored(list, true);
        let mut call = match anchored {
            Some(Ok(call)) => call,
            Some(Err((reason, why))) => {
                self.reasons.borrow_mut().push(reason);
                return Err(format!("`{list}` of {}: {why}", self.object.id));
            }
            None => axioval_ir::measured::parse_members(list).map_err(|error| {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::InvalidDeclaration);
                error.to_string()
            })?,
        };
        // The rule's parameters and the anchor the list names, bound as a
        // measured value's are.
        if let Err((reason, why)) = bind(
            self.context,
            self.parameters,
            self.arguments,
            &self.subject.id,
            &mut call,
        ) {
            self.reasons.borrow_mut().push(reason);
            return Err(format!("`{list}` of {}: {why}", self.object.id));
        }
        let (measured, listed) =
            axioval_engine::measured_members_bound(self.context.services, &self.object.id, &call)
                .map_err(|error| {
                self.reasons
                    .borrow_mut()
                    .push(crate::selection::property_error(error.clone()).0);
                format!("`{list}` of {}: {error}", self.object.id)
            })?;
        self.listed = listed;
        let mut members = Vec::new();
        for member in &measured {
            let (value, evidence) = match value {
                None => (Ok(Value::Null), Vec::new()),
                Some(value) => {
                    let mut leaves = self.measured_member(member);
                    let evaluation = evaluate_untraced(value, path, &mut leaves);
                    self.adopt_reason(&leaves, &evaluation.outcome);
                    (
                        evaluation.outcome,
                        evaluation
                            .reads
                            .iter()
                            .flat_map(|read| read.leaf.evidence.iter().cloned())
                            .collect(),
                    )
                }
            };
            members.push(Member {
                certain: member.certain,
                value,
                evidence,
            });
        }
        Ok(members)
    }

    /// The candidate members `over` reaches from the object in scope, each
    /// with whether it surely belongs and the evidence that reached it.
    fn candidates(&self, over: &AggregateSource) -> Result<Vec<Candidate<'a>>, String> {
        if let Some((_, members)) = self.supplied.iter().find(|(supplied, _)| supplied == over) {
            return Ok(members.clone());
        }
        let context = self.context;
        match over {
            AggregateSource::Path { path } => {
                let traversal = Traversal::path(path).map_err(|(_, why)| why)?;
                let everything: Vec<&Object> = context.project.objects().collect();
                let (reached, cited) = traversal
                    .related(context, &self.object.id, &everything)
                    .map_err(|(_, why)| format!("via {}: {why}", traversal.relationship))?;
                reached
                    .iter()
                    .map(|id| {
                        object_by_id(context, id)
                            .map(|object| (object, true, cited.clone()))
                            .ok_or_else(|| {
                                format!("the path reached {id}, which is not in the run")
                            })
                    })
                    .collect()
            }
            AggregateSource::Group { grouping } => {
                let groups = context
                    .services
                    .get::<std::sync::Arc<axioval_engine::DerivedGroups>>()
                    .ok_or("no derived groups are available outside a run")?;
                let derived = groups
                    .grouping(grouping)
                    .ok_or_else(|| format!("the run derives no grouping `{grouping}`"))?;
                let group = match derived.group(&self.object.id) {
                    Some(group) => group,
                    None => match derived.membership(&self.object.id) {
                        Some(axioval_engine::Membership::Grouped(group, _)) => derived
                            .group(group)
                            .ok_or_else(|| format!("group {group} is not derived"))?,
                        Some(axioval_engine::Membership::Undecided(_, why)) => {
                            return Err(format!(
                                "whether {} belongs to a group of `{grouping}` is undecided: {why}",
                                self.object.id
                            ));
                        }
                        _ => return Ok(Vec::new()),
                    },
                };
                if let Some(why) = group.undecided() {
                    return Err(format!("the members of the group are not all known: {why}"));
                }
                group
                    .members()
                    .iter()
                    .map(|id| {
                        object_by_id(context, id)
                            .map(|object| (object, true, Vec::new()))
                            .ok_or_else(|| format!("member {id} is not in the run"))
                    })
                    .collect()
            }
            AggregateSource::Measured { .. } => Err("measured members are no objects".into()),
            AggregateSource::Selector { selector } => {
                let mut candidates = Vec::new();
                for object in context.project.objects() {
                    let mut evidence = Vec::new();
                    match selector_matches(context, selector, object, &mut evidence) {
                        Selection::Match => candidates.push((object, true, evidence)),
                        Selection::NoMatch => {}
                        Selection::NotEvaluated(..) => candidates.push((object, false, evidence)),
                    }
                }
                Ok(candidates)
            }
        }
    }

    /// What the source states for the property `set`/`name` of the object
    /// in scope, as last read; `None` where it was not read or could not be.
    pub(crate) fn stated(&self, set: Option<&str>, name: &str) -> Option<Stated> {
        self.stated
            .borrow()
            .iter()
            .rev()
            .find(|(read_set, read_name, _)| read_set.as_deref() == set && &**read_name == name)
            .map(|(_, _, value)| Stated(value.clone()))
    }

    /// Why the first unreadable leaf was unreadable.
    pub(crate) fn first_reason(&self) -> Option<NotEvaluatedReason> {
        self.reasons.borrow().first().cloned()
    }

    /// Takes over why a member's value was not evaluated, so an aggregate
    /// left open by a member is open for the member's reason, as reading
    /// the value on the object itself would leave it.
    fn adopt_reason<T, E>(&self, member: &Self, outcome: &Result<T, E>) {
        if outcome.is_err()
            && let Some(reason) = member.first_reason()
        {
            self.reasons.borrow_mut().push(reason);
        }
    }
}

impl ExpressionContext for ObjectLeaves<'_> {
    fn spend(&mut self) -> bool {
        let spent = self.budget.is_none_or(|budget| budget.spend());
        if !spent {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::ResourceLimit);
        }
        spent
    }

    #[allow(clippy::too_many_lines)]
    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        if set == Some(axioval_ir::MEMBER_SET) {
            return self.member_field(name);
        }
        if set == Some(axioval_ir::VALUE_SET) {
            return self.derived(name);
        }
        // Only a name naming a reference (`@`) is bound; every other is
        // read as it is, unparsed here. One naming no anchor is bound once
        // for the rule.
        if set == Some(axioval_ir::MEASURED_SET) && name.contains('@') {
            // Read ahead for the object: nothing to bind.
            if let Some(index) = self.bound.iter().position(|(read, _)| &**read == name) {
                let (written, read) = self.bound[index].clone();
                return self.bound_leaf_named(name, Some(written), read);
            }
            if let Some(arguments) = self.arguments {
                if let Some(bound) = arguments.call(self.context, self.parameters, name) {
                    return self.bound_measured_with(name, bound);
                }
                if let Some(prepared) = arguments.anchored_read(self.context, self.parameters, name)
                {
                    return self.anchored_measured(name, prepared);
                }
            }
            match self.anchored(name, false) {
                Some(Ok(call)) => return self.bound_measured(name, call),
                Some(Err((reason, why))) => {
                    let read = Err((
                        reason,
                        format!(
                            "`{}` of {}: {why}",
                            name.split(';').next().unwrap_or(name),
                            self.object.id
                        ),
                    ));
                    return self.bound_leaf(name, read);
                }
                None => {
                    if let Ok(call) = axioval_ir::measured::parse(name)
                        && !call.is_bound()
                    {
                        return self.bound_measured(name, call);
                    }
                }
            }
        }
        // The value as the source states it (`None`: stated absent) and
        // the evidence cited, read ahead in a batch or resolved now.
        // A property this object read already is read again as it was,
        // its exact evidence cited once.
        let known = self
            .resolved
            .borrow()
            .iter()
            .filter(|_| self.cached)
            .find(|(read_object, read_set, read_name, _)| {
                *read_object == self.object.id && read_set.as_deref() == set && &**read_name == name
            })
            .map(|(_, _, _, read)| match read {
                Ok((stated, evidence)) => Ok((
                    stated
                        .as_ref()
                        .map_or(Ok(Value::Null), Value::from_property),
                    if evidence.iter().all(|evidence| evidence.exact) {
                        Vec::new()
                    } else {
                        evidence.clone()
                    },
                )),
                Err(error) => Err(error.clone()),
            });
        if let Some(known) = known {
            return match known {
                Ok((value, evidence)) => Leaf { value, evidence },
                Err((reason, message)) => {
                    self.reasons.borrow_mut().push(reason);
                    Leaf::unreadable(message)
                }
            };
        }
        let (key, read) =
            if let Some(index) = self.prefetched.iter().position(|(read_set, read_name, _)| {
                read_set.as_deref() == set && &**read_name == name
            }) {
                {
                    let (read_set, read_name, read) = self.prefetched.swap_remove(index);
                    let read = read.map(|read| match read {
                        MeasuredRead::Value(value, evidence) => {
                            (Some(value), evidence.into_iter().collect())
                        }
                        MeasuredRead::Absent(evidence) => (None, vec![evidence]),
                    });
                    ((read_set, read_name), read)
                }
            } else {
                {
                    let key: (Option<Arc<str>>, Arc<str>) = (set.map(Arc::from), Arc::from(name));
                    let read = resolve(self.context, self.object, PropertyRef { set, name }).map(
                        |resolved| match resolved {
                            Resolved::Present(property) => (
                                Some(property.value),
                                property.evidence.into_iter().collect(),
                            ),
                            Resolved::Absent(evidence) => (None, vec![evidence]),
                        },
                    );
                    if self.cached {
                        // Exact evidence is cited once: none is kept.
                        let kept = match &read {
                            Ok((stated, evidence)) => Ok((
                                stated.clone(),
                                if evidence.iter().all(|evidence| evidence.exact) {
                                    Vec::new()
                                } else {
                                    evidence.clone()
                                },
                            )),
                            Err(error) => Err(error.clone()),
                        };
                        self.resolved.borrow_mut().push((
                            self.object.id.clone(),
                            key.0.clone(),
                            key.1.clone(),
                            kept,
                        ));
                    }
                    (key, read)
                }
            };
        match read {
            Ok((stated, evidence)) => {
                let value = match &stated {
                    // A stated absence is `null`, never a value not read.
                    None => Ok(Value::Null),
                    Some(value) => Value::from_property(value),
                };
                self.stated.borrow_mut().push((key.0, key.1, stated));
                if value.is_err() {
                    self.reasons
                        .borrow_mut()
                        .push(NotEvaluatedReason::InvalidEvidence);
                }
                Leaf { value, evidence }
            }
            Err((reason, message)) => {
                self.reasons.borrow_mut().push(reason);
                Leaf::unreadable(message)
            }
        }
    }

    fn derived(&mut self, name: &str) -> Leaf {
        let leaf = derived_value(self.context.services, &self.object.id, name);
        if leaf.value.is_err() {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::IncompleteEvidence);
        }
        leaf
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        let Some(parameters) = self.parameters else {
            return Leaf::unreadable(format!("a selector reads no rule parameter, not `{name}`"));
        };
        match parameters.get(name).cloned().map(ScalarValue::try_from) {
            Some(Ok(scalar)) => match Value::from_literal(&scalar) {
                Ok(value) => Leaf::stated(value),
                Err(why) => Leaf::unreadable(why),
            },
            Some(Err(_)) => Leaf::unreadable(format!("parameter `{name}` is no single value")),
            None => Leaf::unreadable(format!("the rule has no parameter `{name}`")),
        }
    }

    fn subject_property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        let object = self.object;
        self.object = self.subject;
        let leaf = self.property(set, name);
        self.object = object;
        leaf
    }

    fn rule(&mut self, rule: &str, read: RuleRead) -> Leaf {
        let Some(outcomes) = self.context.services.get::<RuleOutcomes>() else {
            return Leaf::unreadable("no rule outcomes are available outside a run");
        };
        let Some(record) = outcomes.get(rule) else {
            return Leaf::unreadable(format!("rule `{rule}` has not run"));
        };
        let verdict = if read == RuleRead::Selected {
            // Whether it was selected, however it was then judged.
            match record.selected(self.object) {
                Ok(true) => ObjectVerdict::Passed,
                Ok(false) => ObjectVerdict::NotSelected,
                Err(why) => ObjectVerdict::Undecided(why),
            }
        } else {
            record.object(self.object)
        };
        let (count, deviation) = record.findings_about(&self.object.id);
        let value = match (read, verdict) {
            (_, ObjectVerdict::Undecided(why)) => {
                self.reasons
                    .borrow_mut()
                    .push(NotEvaluatedReason::IncompleteEvidence);
                let what = if read == RuleRead::Selected {
                    "cannot tell whether it selected it"
                } else {
                    "left it open"
                };
                return Leaf::unreadable(format!("rule `{rule}` {what}: {why}"));
            }
            (RuleRead::Outcome, ObjectVerdict::Passed) => Value::Boolean(true),
            (RuleRead::Outcome, ObjectVerdict::Failed) => Value::Boolean(false),
            (RuleRead::Outcome, ObjectVerdict::NotSelected) => Value::Null,
            (RuleRead::Selected, verdict) => {
                Value::Boolean(!matches!(verdict, ObjectVerdict::NotSelected))
            }
            (RuleRead::FindingCount, _) => Value::integer(i64::try_from(count).unwrap_or(i64::MAX)),
            (RuleRead::Deviation, _) => match deviation {
                Some((lower, upper)) => Value::Number {
                    value: Interval { lower, upper },
                    unit: Unit::NONE,
                },
                None => Value::Null,
            },
        };
        Leaf {
            value: Ok(value),
            evidence: vec![Evidence::exact(
                self.object.id.source.clone(),
                format!("rule:{rule}#{}", self.object.id.local_id),
            )],
        }
    }

    fn members(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        self.listed_members(over, filter, value, path, false)
    }

    fn members_until_unreadable(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        self.listed_members(over, filter, value, path, true)
    }

    fn declared_types(&self) -> Option<&axioval_engine::expression::DeclaredTypes> {
        self.context
            .services
            .get::<std::sync::Arc<axioval_engine::expression::DeclaredTypes>>()
            .map(AsRef::as_ref)
    }

    fn listing_evidence(&mut self) -> Vec<Evidence> {
        std::mem::take(&mut self.listed)
    }

    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        let Some(ParameterValue::Table { value: rows }) =
            self.parameters.and_then(|parameters| parameters.get(table))
        else {
            return Leaf::unreadable(format!("the rule has no table `{table}`"));
        };
        lookup(rows, keys, column).map_or_else(Leaf::unreadable, Leaf::stated)
    }
}

impl ObjectLeaves<'_> {
    /// The members `over` lists, each with `value` evaluated with it in
    /// scope; with `until_unreadable`, none after the first whose value
    /// cannot be read.
    fn listed_members(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
        until_unreadable: bool,
    ) -> Result<Vec<Member>, String> {
        if let AggregateSource::Measured { name } = over {
            return self.measured_members(name, value, path);
        }
        let mut members = Vec::new();
        for (object, mut certain, mut evidence) in self.candidates(over)? {
            if let Some(filter) = filter {
                match selector_matches(self.context, filter, object, &mut evidence) {
                    Selection::Match => {}
                    Selection::NoMatch => continue,
                    Selection::NotEvaluated(..) => certain = false,
                }
            }
            let value = match value {
                None => Ok(Value::Null),
                Some(value) => {
                    let mut leaves = self.member(object);
                    let evaluation = evaluate(value, path, &mut leaves);
                    self.adopt_reason(&leaves, &evaluation.outcome);
                    evidence.extend(
                        evaluation
                            .reads
                            .iter()
                            .flat_map(|read| read.leaf.evidence.iter().cloned()),
                    );
                    evaluation.outcome
                }
            };
            let unreadable = value.is_err();
            members.push(Member {
                certain,
                value,
                evidence,
            });
            if until_unreadable && unreadable {
                break;
            }
        }
        Ok(members)
    }
}

/// The `column` cell of the most specific row whose key cells match `keys`:
/// a text cell is a wildcard pattern matched against the key's text, any
/// other cell must equal the key, and a blank cell accepts any key. No
/// matching row, or one leaving `column` blank, is `null`; tied or undecided
/// rows have no value.
fn lookup(
    rows: &[TableRow],
    keys: &BTreeMap<String, Value>,
    column: &str,
) -> Result<Value, String> {
    let mut problem = None;
    let matched = match_rows(rows, RowSelection::MostSpecific, |row| {
        let mut verdict = RowTest::Match(0);
        for (key, value) in keys {
            let Some(cell) = row.get(key) else {
                continue;
            };
            let outcome = match (cell, value) {
                (_, Value::Null) => RowTest::NoMatch,
                (ParameterValue::String { value: pattern }, value) => match key_text(value) {
                    Some(text) => match TextPattern::new(pattern, true) {
                        Ok(pattern) => pattern.test(&text),
                        Err(why) => {
                            problem.get_or_insert(why);
                            RowTest::Undecided
                        }
                    },
                    None => RowTest::NoMatch,
                },
                (cell, value) => match ScalarValue::try_from(cell.clone())
                    .ok()
                    .and_then(|cell| Value::from_literal(&cell).ok())
                {
                    Some(cell) => equal(&cell, value),
                    None => RowTest::NoMatch,
                },
            };
            verdict = verdict.and(outcome);
        }
        verdict
    });
    match matched {
        Matched::Rows(found) => Ok(match found.first() {
            None => Value::Null,
            Some((_, row)) => match row.get(column) {
                None => Value::Null,
                Some(cell) => {
                    let scalar = ScalarValue::try_from(cell.clone())
                        .map_err(|_| format!("column `{column}` holds no single value"))?;
                    Value::from_literal(&scalar)?
                }
            },
        }),
        Matched::Undecided => Err(problem.unwrap_or_else(|| "a row cannot be decided".into())),
        Matched::Ambiguous(tied) => Err(format!(
            "rows {} tie for the most specific",
            tied.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A key read as text, as `keyed-limit` reads keys.
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) | Value::Enum(text) => Some(text.clone()),
        Value::Boolean(value) => Some(value.to_string()),
        Value::Number { value, unit } if unit.is_plain() && value.is_point() => {
            Some(value.lower.to_string())
        }
        _ => None,
    }
}

fn equal(cell: &Value, value: &Value) -> RowTest {
    match (cell, value) {
        (
            Value::Number {
                value: cell,
                unit: cell_unit,
            },
            Value::Number { value, unit },
        ) if cell_unit == unit => {
            if cell.upper < value.lower || cell.lower > value.upper {
                RowTest::NoMatch
            } else if cell.is_point() && value.is_point() {
                RowTest::Match(1)
            } else {
                RowTest::Undecided
            }
        }
        (cell, value) if cell == value => RowTest::Match(1),
        _ => RowTest::NoMatch,
    }
}

/// The measured set's name, shared by every read of it.
fn measured_set() -> Arc<str> {
    static SET: std::sync::LazyLock<Arc<str>> =
        std::sync::LazyLock::new(|| Arc::from(axioval_ir::MEASURED_SET));
    SET.clone()
}
