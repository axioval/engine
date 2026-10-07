//! The vertical connectors a walking capability may climb, and how a climb
//! counts: `stair_selector`, `ramp_selector` and `lift_selector` pick the
//! connectors by kind, `stair_length` (`slope`, the default, or
//! `horizontal-plus-vertical`) and `vertical_factor` (default one) say how
//! the metric-routing service counts a climb.

use std::collections::BTreeMap;

use axioval_engine::{
    ClimbLength, ConnectorRouting, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleContext, StairLength, VerticalConnector, VerticalConnectorKind,
};
use axioval_ir::ObjectId;
use axioval_ir::contract::Selector;
use axioval_ir::measured::MeasuredSelection;

use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, Unavailable, invalid};

/// The connector parameters a walking capability takes.
pub(crate) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("stair_selector", ParameterType::Selector),
        ParameterDescriptor::optional("ramp_selector", ParameterType::Selector),
        ParameterDescriptor::optional("lift_selector", ParameterType::Selector),
        ParameterDescriptor::optional("stair_length", ParameterType::String),
        ParameterDescriptor::optional("vertical_factor", ParameterType::Number),
    ]
}

/// What picks the connectors of one kind: a rule's selector, evaluated
/// per object, or the objects a measured value's argument bound from it.
#[derive(Clone, Copy)]
enum Pick<'a> {
    Selector(&'a Selector),
    Selected(&'a MeasuredSelection),
}

/// A rule's connector declaration.
pub(crate) struct Climbing<'a> {
    selectors: [(VerticalConnectorKind, Option<Pick<'a>>); 3],
    climb: ClimbLength,
}

impl<'a> Climbing<'a> {
    /// The declaration, or `None` when the rule selects no connector: its
    /// walks then stay on one level.
    pub(crate) fn parse(parameters: &Parameters<'a>) -> Result<Option<Self>, Unavailable> {
        let measure = match parameters.string("stair_length")? {
            None | Some("slope") => StairLength::Slope,
            Some("horizontal-plus-vertical") => StairLength::HorizontalPlusVertical,
            Some(other) => {
                return Err(invalid(format!(
                    "`stair_length` `{other}` is unsupported (slope, horizontal-plus-vertical)"
                )));
            }
        };
        let factor = parameters.number("vertical_factor")?.unwrap_or(1.0);
        let climb = ClimbLength::try_new(measure, factor)
            .map_err(|_| invalid("`vertical_factor` must be finite and not negative"))?;
        let found = [
            (
                VerticalConnectorKind::Stair,
                parameters.selector("stair_selector")?,
            ),
            (
                VerticalConnectorKind::Ramp,
                parameters.selector("ramp_selector")?,
            ),
            (
                VerticalConnectorKind::Lift,
                parameters.selector("lift_selector")?,
            ),
        ];
        if found.iter().all(|(_, selector)| selector.is_none()) {
            if parameters.string("stair_length")?.is_some()
                || parameters.number("vertical_factor")?.is_some()
            {
                return Err(invalid(
                    "`stair_length` and `vertical_factor` need a connector selector",
                ));
            }
            return Ok(None);
        }
        Ok(Some(Self {
            selectors: found.map(|(kind, selector)| (kind, selector.map(Pick::Selector))),
            climb,
        }))
    }

    /// The same declaration, its connectors the objects measured values'
    /// arguments bound from its selectors (stairs, ramps, lifts): the
    /// connectors a template's search reads. A kind the rule does not
    /// select stays unselected.
    pub(crate) fn selected(self, bound: [Option<&'a MeasuredSelection>; 3]) -> Self {
        let [stairs, ramps, lifts] = bound;
        let [stair, ramp, lift] = self.selectors;
        let pick = |(kind, declared): (VerticalConnectorKind, Option<Pick<'a>>),
                    bound: Option<&'a MeasuredSelection>| {
            (kind, declared.and(bound.map(Pick::Selected)))
        };
        Self {
            selectors: [pick(stair, stairs), pick(ramp, ramps), pick(lift, lifts)],
            climb: self.climb,
        }
    }

    /// The connectors the rule selects, as a routing: every connector
    /// decided, one kind per object.
    ///
    /// # Errors
    ///
    /// Not evaluated when a connector's selection is undecided (it could
    /// shorten a walk or lengthen it), and an invalid declaration when one
    /// object is selected as two kinds.
    pub(crate) fn routing(
        &self,
        context: &RuleContext<'_>,
    ) -> Result<ConnectorRouting, Unavailable> {
        let mut kinds: BTreeMap<ObjectId, VerticalConnectorKind> = BTreeMap::new();
        for (kind, pick) in &self.selectors {
            let Some(pick) = pick else { continue };
            for object in context.project.objects() {
                let picked = match pick {
                    Pick::Selector(selector) => {
                        selector_matches(context, selector, object, &mut Vec::new())
                    }
                    Pick::Selected(selected) if selected.matched.contains(&object.id) => {
                        Selection::Match
                    }
                    Pick::Selected(selected) if selected.undecided.contains(&object.id) => {
                        let (reason, message) =
                            selected.reasons.get(&object.id).cloned().unwrap_or((
                                NotEvaluatedReason::IncompleteEvidence,
                                "its selection is undecided".to_owned(),
                            ));
                        Selection::NotEvaluated(reason, message)
                    }
                    Pick::Selected(_) => Selection::NoMatch,
                };
                match picked {
                    Selection::Match => {
                        if let Some(other) = kinds.insert(object.id.clone(), *kind)
                            && other != *kind
                        {
                            return Err(invalid(format!(
                                "{} is selected as both a {} and a {}",
                                object.id,
                                other.as_str(),
                                kind.as_str()
                            )));
                        }
                    }
                    Selection::NoMatch => {}
                    Selection::NotEvaluated(_, message) => {
                        return Err((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!(
                                "whether {} is a {} a walk may climb is undecided: {message}",
                                object.id,
                                kind.as_str()
                            ),
                        ));
                    }
                }
            }
        }
        ConnectorRouting::try_new(
            kinds
                .into_iter()
                .map(|(object, kind)| VerticalConnector::new(object, kind))
                .collect(),
            self.climb,
        )
        .map_err(|error| invalid(error.to_string()))
    }
}
