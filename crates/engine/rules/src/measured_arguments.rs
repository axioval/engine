//! Binding a measured value's references to the rule reading it: each
//! `@name` to the rule's parameter of that name, each `@anchor` to the
//! object the rule checks ([`axioval_ir::measured`]).
//!
//! A selector parameter binds to the objects it picks, surely or not, read
//! once per rule through the run's one selection ([`select_shared`], shared between its rules) and
//! sorted by source-qualified identity; a length, path, text, property
//! reference or table binds to the value the parameter states, checked
//! against the measured parameter's kind. A reference that cannot be bound (a parameter the rule does not
//! state, of another kind or not realisable, or a selection whose objects
//! cannot all be listed) leaves the value not evaluated, never a default.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axioval_engine::{NotEvaluatedReason, RuleContext};
use axioval_ir::ObjectId;
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{
    MeasuredArgument, MeasuredCall, MeasuredParameterKind, MeasuredSelection,
};

use crate::selection::select_shared;
use crate::support::{Unavailable, invalid, si_quantity};

/// What one rule's measured values bind, read once per rule: the objects
/// each selector parameter picks, and each measured name naming no anchor,
/// parsed and bound (it binds alike for every object).
#[derive(Default)]
pub(crate) struct Arguments {
    /// The rule's own selector, which `@selection` names in a template.
    selector: Option<Selector>,
    /// What the rule's plan keeps of its measured names for every run.
    planned: Option<Arc<Planned>>,
    selections: RefCell<BTreeMap<String, Result<Arc<MeasuredSelection>, Unavailable>>>,
    /// Each name read through [`Arguments::call`]: `None` where it is not
    /// prepared once for the rule (it names the anchor, binds nothing or
    /// does not parse), kept so it is parsed once.
    calls: RefCell<BTreeMap<String, Prepared>>,
    /// Each name naming a reference read through [`Arguments::anchored`],
    /// as a value (`[0]`) and as a list (`[1]`): the rule's parameters
    /// bound, the anchor left for each object.
    anchored: RefCell<[BTreeMap<String, Anchored>; 2]>,
    /// Each member list naming a selection, bound for the rule and shared
    /// by every object reading it ([`Arguments::planned_list`]).
    lists: RefCell<BTreeMap<String, Option<Arc<MeasuredCall>>>>,
}

/// What a template's bound plan keeps of each measured name its values and
/// lists read, for every run of the rule: each name parsed once, and bound
/// (a value also prepared) once where none of its references needs the
/// run, as a selection does. Filled as names are first read; like the plan,
/// a pure function of the names and the rule's parameters.
#[derive(Default)]
pub(crate) struct Planned {
    /// Each name read as a value.
    values: Mutex<BTreeMap<Box<str>, Kept>>,
    /// Each name read as a member list.
    lists: Mutex<BTreeMap<Box<str>, Kept>>,
    /// Each selector parameter as its shared selection is kept by
    /// ([`crate::selection::shared_as`]), written once.
    selectors: Mutex<BTreeMap<Box<str>, Option<Arc<str>>>>,
}

/// What a plan keeps of one measured name.
#[derive(Clone)]
enum Kept {
    /// It does not parse, or names no reference: read as written.
    Unread,
    /// A value naming no anchor, bound from the rule's parameters alone and
    /// prepared.
    Prepared(Result<axioval_engine::PreparedRead, Unavailable>),
    /// Bound from the rule's parameters alone, the anchor (where named)
    /// left for each object.
    Anchored(
        Result<Arc<MeasuredCall>, Unavailable>,
        Option<axioval_engine::PreparedRead>,
    ),
    /// Parsed, naming a selection, which each run binds.
    Parsed(Arc<MeasuredCall>),
}

impl Planned {
    /// The selector parameter `parameter` (stating `selector`) as its
    /// shared selection is kept by, written once for the plan.
    fn written(&self, parameter: &str, selector: &Selector) -> Option<Arc<str>> {
        if let Some(written) = self
            .selectors
            .lock()
            .ok()
            .and_then(|kept| kept.get(parameter).cloned())
        {
            return written;
        }
        let written: Option<Arc<str>> = crate::selection::shared_as(selector).map(Arc::from);
        if let Ok(mut kept) = self.selectors.lock() {
            kept.insert(parameter.into(), written.clone());
        }
        written
    }

    /// What the plan keeps of `name` (a list where `list`), kept now if it
    /// was not.
    fn kept(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
        list: bool,
    ) -> Kept {
        let map = if list { &self.lists } else { &self.values };
        if let Some(kept) = map.lock().ok().and_then(|kept| kept.get(name).cloned()) {
            return kept;
        }
        let kept = Self::keep(context, parameters, name, list);
        if let Ok(mut map) = map.lock() {
            map.insert(name.into(), kept.clone());
        }
        kept
    }

    fn keep(
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
        list: bool,
    ) -> Kept {
        let Some(mut call) = parsed(name, list) else {
            return Kept::Unread;
        };
        if call.is_bound() {
            return Kept::Unread;
        }
        let mut anchor = false;
        let mut selection = false;
        for (key, argument) in call.references() {
            match argument {
                MeasuredArgument::Anchor => anchor = true,
                MeasuredArgument::Parameter(parameter) => {
                    selection |= parameter == axioval_engine::template::SELECTION
                        || call.parameter(key).is_some_and(|declared| {
                            matches!(declared.kind, MeasuredParameterKind::Objects)
                        });
                }
                _ => {}
            }
        }
        if selection {
            // Every other parameter bound now where all of them bind: a run
            // binds only the selections, refusing as it would have.
            let mut bound = call.clone();
            if bind_values(context, parameters, &mut bound).is_ok() {
                return Kept::Parsed(Arc::new(bound));
            }
            return Kept::Parsed(Arc::new(call));
        }
        // No selection named: the rule's parameters bind alike in every
        // run, whatever the model.
        let bound = bind_references(context, parameters, None, None, &mut call);
        match (list, anchor) {
            (false, false) => {
                Kept::Prepared(bound.map(|()| axioval_engine::PreparedRead::of_owned(name, call)))
            }
            // A value read of each object, prepared once with its anchor
            // left for each read to bind.
            (false, true) => {
                let prepared = bound
                    .is_ok()
                    .then(|| axioval_engine::PreparedRead::anchored(name, call.clone()));
                Kept::Anchored(bound.map(|()| Arc::new(call)), prepared)
            }
            (true, _) => Kept::Anchored(bound.map(|()| Arc::new(call)), None),
        }
    }
}

/// A name prepared once for the rule; `None` where it is not.
type Prepared = Option<Result<axioval_engine::PreparedRead, Unavailable>>;

/// A name parsed with the rule's parameters bound, the anchor left; `None`
/// where it does not parse or names no reference.
type Anchored = Option<Result<MeasuredCall, Unavailable>>;

/// How many parsed names [`parsed`] keeps before it starts over.
const PARSED_KEPT: usize = 1024;

/// The measured name `name` (a member list where `list`) as parsed, its
/// references unbound; `None` where it does not parse. Parsing is a pure
/// function of the name, so each is parsed once and kept for every run
/// (at most [`PARSED_KEPT`], all dropped when full).
pub(crate) fn parsed(name: &str, list: bool) -> Option<MeasuredCall> {
    type Kept = std::collections::HashMap<(bool, String), Option<MeasuredCall>>;
    static KEPT: std::sync::LazyLock<std::sync::Mutex<Kept>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Kept::new()));
    let key = (list, name.to_owned());
    if let Ok(kept) = KEPT.lock()
        && let Some(call) = kept.get(&key)
    {
        return call.clone();
    }
    let call = if list {
        axioval_ir::measured::parse_members(name)
    } else {
        axioval_ir::measured::parse(name)
    }
    .ok();
    if let Ok(mut kept) = KEPT.lock() {
        if kept.len() >= PARSED_KEPT {
            kept.clear();
        }
        kept.insert(key, call.clone());
    }
    call
}

/// The objects `selector`, the rule's parameter `parameter`, picks: those
/// surely picked and those it cannot decide, each sorted. A source whose
/// objects cannot all be listed leaves the whole selection unknown.
fn selection(
    context: &RuleContext<'_>,
    parameter: &str,
    selector: &Selector,
) -> Result<MeasuredSelection, Unavailable> {
    selection_of_selected(parameter, select_shared(context, selector))
}

/// [`selection`] of the objects selected already, with the outcomes of
/// those the selection could not decide.
fn selection_of_selected(
    parameter: &str,
    (matched, outcomes): (
        Vec<&axioval_ir::Object>,
        axioval_engine::CapabilityEvaluation,
    ),
) -> Result<MeasuredSelection, Unavailable> {
    let mut undecided = BTreeSet::new();
    let mut first_undecided = None;
    let mut reasons = BTreeMap::new();
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => {
                undecided.insert(object.clone());
                first_undecided.get_or_insert_with(|| {
                    (outcome.reason().clone(), outcome.message().to_owned())
                });
                reasons.insert(
                    object.clone(),
                    (outcome.reason().clone(), outcome.message().to_owned()),
                );
            }
            None => {
                return Err((
                    outcome.reason().clone(),
                    format!(
                        "the objects `@{parameter}` selects cannot all be listed: {}",
                        outcome.message()
                    ),
                ));
            }
        }
    }
    Ok(MeasuredSelection {
        parameter: parameter.to_owned(),
        matched: matched
            .into_iter()
            .map(|object| object.id.clone())
            .collect(),
        undecided,
        first_undecided,
        reasons,
    })
}

/// The objects `selector`, the rule's parameter `parameter`, picks, read
/// outside a rule's run.
pub(crate) fn selection_of(
    context: &RuleContext<'_>,
    parameter: &str,
    selector: &Selector,
) -> Result<MeasuredSelection, Unavailable> {
    selection(context, parameter, selector)
}

impl Arguments {
    /// What `rule`'s measured values bind: `@selection` the objects the
    /// rule itself selects, where it states no parameter of that name.
    pub(crate) fn of_rule(rule: &axioval_engine::CompiledRule) -> Self {
        Self {
            selector: Some(rule.selector.clone()),
            ..Self::default()
        }
    }

    /// The same, `@selection` the objects the rule selected already and
    /// those whose selection `outcomes` leave undecided.
    pub(crate) fn selected(
        self,
        selected: &[&axioval_ir::Object],
        outcomes: &axioval_engine::CapabilityEvaluation,
    ) -> Self {
        let mut undecided = BTreeSet::new();
        let mut first_undecided = None;
        let mut reasons = BTreeMap::new();
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => {
                    undecided.insert(object.clone());
                    first_undecided.get_or_insert_with(|| {
                        (outcome.reason().clone(), outcome.message().to_owned())
                    });
                    reasons.insert(
                        object.clone(),
                        (outcome.reason().clone(), outcome.message().to_owned()),
                    );
                }
                // The selection cannot be listed whole: read it as bound.
                None => return self,
            }
        }
        self.selections.borrow_mut().insert(
            axioval_engine::template::SELECTION.to_owned(),
            Ok(Arc::new(MeasuredSelection {
                parameter: axioval_engine::template::SELECTION.to_owned(),
                matched: selected.iter().map(|object| object.id.clone()).collect(),
                undecided,
                first_undecided,
                reasons,
            })),
        );
        self
    }

    /// The measured name `name` parsed, bound and prepared for the rule,
    /// once per rule: `None` where it does not parse, binds nothing or
    /// names the anchor, which binds per object.
    pub(crate) fn call(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
    ) -> Option<Result<axioval_engine::PreparedRead, Unavailable>> {
        let parsed = match self
            .planned
            .as_ref()
            .map(|planned| planned.kept(context, parameters, name, false))
        {
            Some(Kept::Unread | Kept::Anchored(..)) => return None,
            Some(Kept::Prepared(read)) => return Some(read),
            Some(Kept::Parsed(call)) => Some(call),
            None => None,
        };
        if let Some(bound) = self.calls.borrow().get(name) {
            return bound.clone();
        }
        let bound = match parsed {
            Some(call) => Self::prepared(context, parameters, self, name, (*call).clone()),
            None => self::parsed(name, false)
                .filter(|call| !call.is_bound())
                .and_then(|call| Self::prepared(context, parameters, self, name, call)),
        };
        self.calls
            .borrow_mut()
            .insert(name.to_owned(), bound.clone());
        bound
    }

    /// `call`, written `name`, bound and prepared for the rule: `None`
    /// where it names the anchor, which binds per object.
    fn prepared(
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        arguments: &Self,
        name: &str,
        mut call: MeasuredCall,
    ) -> Option<Result<axioval_engine::PreparedRead, Unavailable>> {
        if call
            .references()
            .any(|(_, argument)| *argument == MeasuredArgument::Anchor)
        {
            return None;
        }
        // No anchor named: any object binds it alike.
        let anchor = context.project.objects().next()?.id.clone();
        Some(
            bind(context, parameters, Some(arguments), &anchor, &mut call)
                .map(|()| axioval_engine::PreparedRead::of_owned(name, call)),
        )
    }

    /// The measured value `name` naming the anchor, prepared once by the
    /// rule's plan with the anchor left for each object to bind
    /// ([`axioval_engine::PreparedRead::anchored`]), or why the rule's
    /// parameters do not bind; `None` where the plan prepared none (a value
    /// naming a selection binds per run, as [`Self::anchored`] does).
    pub(crate) fn anchored_read(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
    ) -> Option<Result<axioval_engine::PreparedRead, Unavailable>> {
        match self
            .planned
            .as_ref()?
            .kept(context, parameters, name, false)
        {
            Kept::Anchored(Err(why), _) => Some(Err(why)),
            Kept::Anchored(Ok(_), Some(prepared)) => Some(Ok(prepared)),
            _ => None,
        }
    }

    /// The member list `name` as the rule's plan keeps it, its rule
    /// parameters bound, shared rather than copied: `None` where the plan
    /// keeps none so (it names a selection, or does not parse).
    pub(crate) fn planned_list(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
    ) -> Option<Arc<MeasuredCall>> {
        match self.planned.as_ref()?.kept(context, parameters, name, true) {
            Kept::Anchored(Ok(call), _) => Some(call),
            // Its selections bound once for the rule; one that does not bind
            // is read as before, to be refused there.
            Kept::Parsed(call) => {
                if let Some(bound) = self.lists.borrow().get(name) {
                    return bound.clone();
                }
                let mut bound = (*call).clone();
                let bound = bind_references(context, parameters, Some(self), None, &mut bound)
                    .ok()
                    .map(|()| Arc::new(bound));
                self.lists
                    .borrow_mut()
                    .insert(name.to_owned(), bound.clone());
                bound
            }
            _ => None,
        }
    }

    /// The same, its measured names kept by the rule's plan for every run
    /// ([`Planned`]).
    pub(crate) fn planned(mut self, planned: &Arc<Planned>) -> Self {
        self.planned = Some(Arc::clone(planned));
        self
    }

    /// The measured name `name` (a member list where `list`) parsed, the
    /// rule's parameters it names bound, once per rule; each object binds
    /// the anchor it names ([`bind`]) alike. `None` where it does not
    /// parse or names no reference; a parameter that does not bind is
    /// refused as [`bind`] refuses it.
    pub(crate) fn anchored(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
        list: bool,
    ) -> Option<Result<MeasuredCall, Unavailable>> {
        let parsed = match self
            .planned
            .as_ref()
            .map(|planned| planned.kept(context, parameters, name, list))
        {
            Some(Kept::Unread) => return None,
            Some(Kept::Anchored(call, _)) => return Some(call.map(|call| (*call).clone())),
            Some(Kept::Parsed(call)) => Some(call),
            Some(Kept::Prepared(_)) | None => None,
        };
        let kind = usize::from(list);
        if let Some(call) = self.anchored.borrow()[kind].get(name) {
            return call.clone();
        }
        let call = match parsed {
            Some(call) => Some((*call).clone()),
            None => self::parsed(name, list).filter(|call| !call.is_bound()),
        }
        .map(|mut call| {
            bind_references(context, parameters, Some(self), None, &mut call).map(|()| call)
        });
        self.anchored.borrow_mut()[kind].insert(name.to_owned(), call.clone());
        call
    }

    /// The objects the selector parameter `parameter` picks, read once.
    pub(crate) fn selection_of(
        &self,
        context: &RuleContext<'_>,
        parameter: &str,
        selector: &Selector,
    ) -> Result<MeasuredSelection, Unavailable> {
        self.selection(context, parameter, selector)
            .map(|selection| selection.as_ref().clone())
    }

    /// The objects the selector parameter `parameter` picks, read once.
    fn selection(
        &self,
        context: &RuleContext<'_>,
        parameter: &str,
        selector: &Selector,
    ) -> Result<Arc<MeasuredSelection>, Unavailable> {
        if let Some(read) = self.selections.borrow().get(parameter) {
            return read.clone();
        }
        // A selector parameter's selection, as the plan wrote it once (the
        // rule's own selection is no parameter of the plan).
        let written = self
            .planned
            .as_ref()
            .filter(|_| parameter != axioval_engine::template::SELECTION)
            .map(|planned| planned.written(parameter, selector));
        let selected = match &written {
            Some(written) => {
                crate::selection::select_shared_as(context, selector, written.as_deref())
            }
            None => select_shared(context, selector),
        };
        let read = selection_of_selected(parameter, selected).map(Arc::new);
        self.selections
            .borrow_mut()
            .insert(parameter.to_owned(), read.clone());
        read
    }
}

/// A number of SI units or a quantity of `dimension`, at least `minimum`,
/// as `reference` binds it: `plain` and `quantity` name what it must be,
/// and `unit` its unit.
fn measure(
    reference: &Reference<'_>,
    value: &ParameterValue,
    (dimension, minimum): (QuantityDimension, f64),
    (plain, quantity, unit): (&str, &str, &str),
) -> Result<f64, Unavailable> {
    let not = |what: &str| invalid(format!("{reference} is not {what}"));
    #[allow(clippy::cast_precision_loss)]
    let measured = match value {
        ParameterValue::Number { value } => *value,
        ParameterValue::Integer { value } => *value as f64,
        ParameterValue::Quantity { value, unit } => match si_quantity(*value, unit) {
            Ok((measured, stated)) if stated == dimension => measured,
            _ => return Err(not(quantity)),
        },
        _ => return Err(not(plain)),
    };
    if !(measured.is_finite() && measured >= minimum) {
        return Err(invalid(format!(
            "{reference} is {measured} {unit}, not {quantity} of at least {minimum} {unit}"
        )));
    }
    Ok(measured)
}

/// The value the rule's parameter `parameter` binds for a measured
/// parameter of `kind`.
#[allow(clippy::too_many_lines)]
fn bound(
    context: &RuleContext<'_>,
    arguments: Option<&Arguments>,
    kind: MeasuredParameterKind,
    parameter: &str,
    value: &ParameterValue,
) -> Result<MeasuredArgument, Unavailable> {
    // Worded only when a refusal needs it.
    let reference = Reference(parameter);
    let not = |what: &str| invalid(format!("{reference} is not {what}"));
    Ok(match (kind, value) {
        (MeasuredParameterKind::Objects, ParameterValue::Selector { value: selector }) => {
            MeasuredArgument::Objects(match arguments {
                Some(arguments) => arguments.selection(context, parameter, selector)?,
                None => Arc::new(selection(context, parameter, selector)?),
            })
        }
        (MeasuredParameterKind::Objects, _) => return Err(not("a selector")),
        (MeasuredParameterKind::Length { minimum }, value) => MeasuredArgument::Length(measure(
            &reference,
            value,
            (QuantityDimension::Length, minimum),
            ("a number of metres or a length", "a length", "m"),
        )?),
        (MeasuredParameterKind::Path, ParameterValue::StringList { value: steps }) => {
            if steps.is_empty() || steps.iter().any(|step| step.trim().is_empty()) {
                return Err(invalid(format!("{reference} holds an empty step")));
            }
            MeasuredArgument::Path(steps.iter().map(|step| step.trim().to_owned()).collect())
        }
        (MeasuredParameterKind::Choices { options }, ParameterValue::StringList { value }) => {
            MeasuredArgument::Choices(
                axioval_ir::measured::choices(options, value.iter().map(String::as_str))
                    .map_err(|why| invalid(format!("{reference}: {why}")))?,
            )
        }
        // Each path as the rule states it; an empty list states none.
        (MeasuredParameterKind::Paths, ParameterValue::StringList { value: paths }) => {
            MeasuredArgument::Path(paths.iter().map(|path| path.trim().to_owned()).collect())
        }
        (
            MeasuredParameterKind::Path
            | MeasuredParameterKind::Paths
            | MeasuredParameterKind::Choices { .. },
            _,
        ) => {
            return Err(not("a string list"));
        }
        // A pattern binds exactly as stated: its spaces match spaces.
        (MeasuredParameterKind::Pattern, ParameterValue::String { value: text }) => {
            if text.is_empty() {
                return Err(invalid(format!("{reference} is empty")));
            }
            MeasuredArgument::Text(text.clone())
        }
        (
            MeasuredParameterKind::Choice { .. }
            | MeasuredParameterKind::Text
            | MeasuredParameterKind::SourceKind,
            ParameterValue::String { value: text },
        ) => match kind {
            MeasuredParameterKind::Choice { options } => MeasuredArgument::Choice(
                options
                    .iter()
                    .find(|option| option.eq_ignore_ascii_case(text.trim()))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{reference} `{text}` is none of {}",
                            options.join(", ")
                        ))
                    })?,
            ),
            _ if text.trim().is_empty() => return Err(invalid(format!("{reference} is empty"))),
            MeasuredParameterKind::Text => MeasuredArgument::Text(text.trim().to_owned()),
            _ => MeasuredArgument::SourceKind(text.trim().to_owned()),
        },
        (
            MeasuredParameterKind::Choice { .. }
            | MeasuredParameterKind::Text
            | MeasuredParameterKind::Pattern
            | MeasuredParameterKind::SourceKind,
            _,
        ) => return Err(not("a string")),
        (
            MeasuredParameterKind::Property,
            ParameterValue::PropertyReference {
                property_set,
                property,
            },
        ) => MeasuredArgument::Property {
            set: property_set.clone(),
            name: property.clone(),
        },
        (MeasuredParameterKind::Property, _) => return Err(not("a property reference")),
        (MeasuredParameterKind::Table, ParameterValue::Table { value: rows }) => {
            MeasuredArgument::Table(rows.clone())
        }
        (MeasuredParameterKind::Table, _) => return Err(not("a table")),
        (MeasuredParameterKind::Area { minimum }, value) => MeasuredArgument::Number(measure(
            &reference,
            value,
            (QuantityDimension::Area, minimum),
            ("a number of square metres or an area", "an area", "m²"),
        )?),
        (MeasuredParameterKind::Number { minimum }, value) => {
            #[allow(clippy::cast_precision_loss)]
            let number = match value {
                ParameterValue::Number { value } => *value,
                ParameterValue::Integer { value } => *value as f64,
                _ => return Err(not("a number")),
            };
            if !(number.is_finite() && number >= minimum) {
                return Err(invalid(format!(
                    "{reference} is {number}, not a number of at least {minimum}"
                )));
            }
            MeasuredArgument::Number(number)
        }
        (MeasuredParameterKind::Truth, ParameterValue::Boolean { value }) => {
            MeasuredArgument::Truth(*value)
        }
        (MeasuredParameterKind::Truth, _) => return Err(not("a boolean")),
        (MeasuredParameterKind::Angle { below }, value) => {
            let ParameterValue::Quantity { value, unit } = value else {
                return Err(not("a plane angle"));
            };
            let degrees = match si_quantity(*value, unit) {
                Ok((radians, QuantityDimension::PlaneAngle)) => radians.to_degrees(),
                _ => return Err(not("a plane angle")),
            };
            if !(0.0..below).contains(&degrees) {
                return Err(invalid(format!(
                    "{reference} is {degrees} degrees, not an angle of at least 0 and below \
                     {below} degrees"
                )));
            }
            MeasuredArgument::Number(degrees)
        }
        (MeasuredParameterKind::Vector | MeasuredParameterKind::Polygon, _) => {
            return Err(invalid(format!("{reference} names no value of a rule")));
        }
    })
}

/// Binds every reference of `call`: `@anchor` to `anchor`, `@name` to the
/// rule's parameter `name` in `parameters` (none outside a rule).
///
/// # Errors
///
/// A reference that cannot be bound, for its reason: an invalid
/// declaration for a parameter the rule does not state or of another kind,
/// incomplete evidence for a selection that cannot be listed.
pub(crate) fn bind(
    context: &RuleContext<'_>,
    parameters: Option<&BTreeMap<String, ParameterValue>>,
    arguments: Option<&Arguments>,
    anchor: &ObjectId,
    call: &mut MeasuredCall,
) -> Result<(), Unavailable> {
    bind_references(context, parameters, arguments, Some(anchor), call)
}

/// [`bind`], the anchor left unbound where none is given.
fn bind_references(
    context: &RuleContext<'_>,
    parameters: Option<&BTreeMap<String, ParameterValue>>,
    arguments: Option<&Arguments>,
    anchor: Option<&ObjectId>,
    call: &mut MeasuredCall,
) -> Result<(), Unavailable> {
    let references: Vec<(&'static str, MeasuredArgument)> = call
        .references()
        .map(|(key, argument)| (key, argument.clone()))
        .collect();
    for (key, reference) in references {
        let argument = match &reference {
            MeasuredArgument::Anchor => match anchor {
                Some(anchor) => {
                    MeasuredArgument::Objects(Arc::new(MeasuredSelection::anchor(anchor.clone())))
                }
                None => continue,
            },
            MeasuredArgument::Parameter(parameter) => {
                let Some(parameters) = parameters else {
                    return Err(invalid(format!(
                        "`@{parameter}` names a rule parameter, which a selector never reads"
                    )));
                };
                // A template's own selection, as `@selection`.
                if parameter == axioval_engine::template::SELECTION
                    && !parameters.contains_key(parameter)
                    && let Some(arguments) = arguments
                    && let Some(selector) = &arguments.selector
                {
                    let selection = arguments.selection(context, parameter, selector)?;
                    call.bind(key, MeasuredArgument::Objects(selection))
                        .map_err(|error| {
                            (NotEvaluatedReason::InvalidDeclaration, error.to_string())
                        })?;
                    continue;
                }
                let value = parameters.get(parameter).ok_or_else(|| {
                    invalid(format!("the rule states no parameter `{parameter}`"))
                })?;
                let kind = call
                    .parameter(key)
                    .map(|declared| declared.kind)
                    .ok_or_else(|| invalid(format!("`{key}` is no parameter")))?;
                bound(context, arguments, kind, parameter, value)?
            }
            _ => continue,
        };
        call.bind(key, argument)
            .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    }
    Ok(())
}

/// Binds every reference of `call` naming a rule parameter of a value (no
/// selection, no anchor), as [`bind`] binds it.
fn bind_values(
    context: &RuleContext<'_>,
    parameters: Option<&BTreeMap<String, ParameterValue>>,
    call: &mut MeasuredCall,
) -> Result<(), Unavailable> {
    let Some(parameters) = parameters else {
        return Err(invalid("no rule parameters".to_owned()));
    };
    let references: Vec<(&'static str, String)> = call
        .references()
        .filter_map(|(key, argument)| match argument {
            MeasuredArgument::Parameter(parameter) => Some((key, parameter.clone())),
            _ => None,
        })
        .collect();
    for (key, parameter) in references {
        let kind = call
            .parameter(key)
            .map(|declared| declared.kind)
            .ok_or_else(|| invalid(format!("`{key}` is no parameter")))?;
        if parameter == axioval_engine::template::SELECTION
            || matches!(kind, MeasuredParameterKind::Objects)
        {
            continue;
        }
        let value = parameters
            .get(&parameter)
            .ok_or_else(|| invalid(format!("the rule states no parameter `{parameter}`")))?;
        let argument = bound(context, None, kind, &parameter, value)?;
        call.bind(key, argument)
            .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    }
    Ok(())
}

/// A reference as a refusal words it, `` `@name` ``.
struct Reference<'a>(&'a str);

impl std::fmt::Display for Reference<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "`@{}`", self.0)
    }
}

#[cfg(test)]
mod tests {
    use axioval_engine::{RuleContext, ServiceRegistry};
    use axioval_ir::contract::{ParameterValue, TableRow};
    use axioval_ir::measured::{MeasuredArgument, MeasuredParameterKind};
    use axioval_ir::{NotEvaluatedReason, Project};

    use super::bound;

    /// A property reference and a table bind as the rule states them, and
    /// a parameter of another kind is the rule's invalid declaration.
    #[test]
    fn a_property_or_a_table_binds_as_stated() {
        let project = Project::new(Vec::new()).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let property = ParameterValue::PropertyReference {
            property: "Width".into(),
            property_set: Some("Attributes".into()),
        };
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Property,
                "overall",
                &property
            ),
            Ok(MeasuredArgument::Property {
                set: Some("Attributes".into()),
                name: "Width".into()
            })
        );
        let rows = vec![TableRow::from([(
            "width".to_owned(),
            ParameterValue::Number { value: 1.0 },
        )])];
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Table,
                "rows",
                &ParameterValue::Table {
                    value: rows.clone()
                }
            ),
            Ok(MeasuredArgument::Table(rows))
        );
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Table,
                "overall",
                &property
            ),
            Err((
                NotEvaluatedReason::InvalidDeclaration,
                "`@overall` is not a table".into()
            ))
        );
    }

    /// A number binds at least its minimum, an integer as a number, and a
    /// boolean as a truth.
    #[test]
    fn a_number_or_a_truth_binds_as_stated() {
        let project = Project::new(Vec::new()).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let number = MeasuredParameterKind::Number { minimum: 0.0 };
        assert_eq!(
            bound(
                &context,
                None,
                number,
                "share",
                &ParameterValue::Integer { value: 1 }
            ),
            Ok(MeasuredArgument::Number(1.0))
        );
        assert_eq!(
            bound(
                &context,
                None,
                number,
                "share",
                &ParameterValue::Number { value: -0.5 }
            ),
            Err((
                NotEvaluatedReason::InvalidDeclaration,
                "`@share` is -0.5, not a number of at least 0".into()
            ))
        );
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Truth,
                "chain",
                &ParameterValue::Boolean { value: true }
            ),
            Ok(MeasuredArgument::Truth(true))
        );
    }

    /// A bound selection keeps, for each undecided object, the reason and
    /// the words the selection left it undecided with.
    #[test]
    fn an_undecided_object_keeps_its_reason() {
        let object =
            axioval_ir::ObjectId::new(axioval_ir::SourceId::new("model", "ifc").unwrap(), "pipe")
                .unwrap();
        let mut outcomes = axioval_engine::CapabilityEvaluation::default();
        outcomes.push_object_not_evaluated(
            object.clone(),
            NotEvaluatedReason::MissingService,
            "no service",
        );
        let arguments = super::Arguments::default().selected(&[], &outcomes);
        let selections = arguments.selections.borrow();
        let selection = selections[axioval_engine::template::SELECTION]
            .as_ref()
            .unwrap();
        assert_eq!(
            selection.reasons.get(&object),
            Some(&(NotEvaluatedReason::MissingService, "no service".to_owned()))
        );
    }
}
