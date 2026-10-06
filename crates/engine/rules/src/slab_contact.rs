//! `slab-contact`: whether enough of a face rests on another element, and
//! the storey search that leaves out the top or bottom storey.
//!
//! ADR 0004: contact areas are measured by a
//! [`ContactServiceHandle`](axioval_engine::ContactServiceHandle), as the
//! measured values `contact_share`, `contact_area` and `contact_gap`
//! ([`ContactMeasures`]); whether enough of the face is in contact, and how
//! serious a shortfall is, is the template's policy: a shortfall graded by
//! how far the measured share falls beneath the declared minimum, a total
//! absence of contact by how far away the nearest candidate is.
//!
//! Scope is policy too. Which objects the face may rest on is the
//! `counterparts` selector, bound into the values, so the adapter measures
//! exactly those. Leaving out the top or bottom storey is decided from each
//! storey's `Elevation` attribute ([`storeys`], [`ends`]), as the measured
//! value `storey_end`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{ATTRIBUTE_SET, Object, ObjectId, PropertyValue, QuantityDimension, SourceId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::{ContactMeasures, StoreyMeasures};

use crate::selection::{object_by_id, select_objects};
use crate::support::{PropertyRef, Traversal, Unavailable, invalid, resolve};

/// Requires a minimum fraction of a face to rest on another element.
///
/// `counterparts` selects what the face may rest on; without it, every other
/// project object is a candidate. `skip_top_storey` and `skip_bottom_storey`
/// leave out subjects on the highest or lowest `storey_selector` object of
/// their source, ordered by the `Elevation` attribute. The subject's storey is
/// the one the declared traversal (`relationship` or `path`) reaches from it.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `contact_share` at least `minimum_contact_ratio`, a shortfall graded by
/// the measured `contact_area`, `contact_gap` and the share's part of the
/// minimum, and the measured `storey_end` leaving a subject unjudged.
pub struct SlabContact;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SlabContact {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The attribute storeys are ordered by.
const ELEVATION: PropertyRef<'static> = PropertyRef {
    set: Some(ATTRIBUTE_SET),
    name: "Elevation",
};

/// Every selected storey's elevation, and the lowest and highest per source.
#[derive(Clone)]
pub(crate) struct Storeys {
    pub(crate) universe: Vec<ObjectId>,
    pub(crate) elevations: BTreeMap<ObjectId, f64>,
    pub(crate) extremes: BTreeMap<SourceId, (f64, f64)>,
}

/// Every storey `selector` picks, with its elevation. Any undecided storey
/// or unknown elevation fails closed: without it, which storey is highest
/// or lowest is not known.
pub(crate) fn storeys(
    context: &RuleContext<'_>,
    selector: &Selector,
) -> Result<Storeys, Unavailable> {
    let (universe, outcomes) = select_objects(context, selector);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            outcome.reason().clone(),
            format!("storey selection is undecided: {}", outcome.message()),
        ));
    }
    ordered(context, &universe)
}

/// The elevations of `universe`, every storey decided.
pub(crate) fn ordered(
    context: &RuleContext<'_>,
    universe: &[&Object],
) -> Result<Storeys, Unavailable> {
    let mut elevations = BTreeMap::new();
    let mut extremes: BTreeMap<SourceId, (f64, f64)> = BTreeMap::new();
    for storey in universe {
        let resolved = resolve(context, storey, ELEVATION)?;
        let Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }) = resolved.value()
        else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} has no length Elevation ({})",
                    storey.id,
                    crate::support::display(resolved.value())
                ),
            ));
        };
        if !value.is_finite() {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!("{} has a non-finite Elevation", storey.id),
            ));
        }
        elevations.insert(storey.id.clone(), *value);
        extremes
            .entry(storey.id.source.clone())
            .and_modify(|(low, high)| {
                *low = low.min(*value);
                *high = high.max(*value);
            })
            .or_insert((*value, *value));
    }
    Ok(Storeys {
        universe: universe.iter().map(|storey| storey.id.clone()).collect(),
        elevations,
        extremes,
    })
}

/// Whether `subject` lies on the highest and on the lowest storey of its
/// source, through the one storey `traversal` reaches from it.
///
/// The subject must reach exactly one storey: none or several leave its
/// position in the building unknown, which is not evaluated rather than
/// guessed.
pub(crate) fn ends(
    context: &RuleContext<'_>,
    traversal: &Traversal,
    storeys: &Storeys,
    subject: &ObjectId,
) -> Result<(bool, bool), Unavailable> {
    let universe: Vec<&Object> = storeys
        .universe
        .iter()
        .filter_map(|storey| object_by_id(context, storey))
        .collect();
    let (reached, _) = traversal.related(context, subject, &universe)?;
    let [storey] = reached.as_slice() else {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} reaches {} storeys through {}, so whether it is on the top or bottom \
                 storey is unknown",
                subject,
                reached.len(),
                traversal.relationship
            ),
        ));
    };
    let (Some(elevation), Some((low, high))) = (
        storeys.elevations.get(storey),
        storeys.extremes.get(&storey.source),
    ) else {
        return Err(invalid(format!("storey {storey} was not selected")));
    };
    Ok((
        elevation.total_cmp(high).is_eq(),
        elevation.total_cmp(low).is_eq(),
    ))
}
