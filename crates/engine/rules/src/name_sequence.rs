//! Numbered names that must count up in a declared order.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ObjectFrameServiceHandle,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
#[cfg(feature = "parity-reference")]
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::support::{Parameters, PropertyRef, Traversal, Unavailable, invalid, resolve};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::SequenceMeasures;

pub(crate) const NAME: &str = "name-sequence";

/// Requires the members of each anchor to be numbered consecutively in order.
///
/// Storeys of a building named `1`, `2`, `3` from the lowest up: the anchors
/// are the rule's selection (buildings), the members are the objects
/// `member_selector` picks that the declared `relationship` reaches from an
/// anchor (or, with none, every such object in the anchor's source), ordered
/// by the numeric `order` property (elevation), ties broken by name. The
/// first numbered member must be `first` (default 1), and each next one the
/// previous plus `increment` (default 1).
///
/// A name counts as a number only when it is one exactly: an optional sign
/// and digits, nothing else, so ` 1` and `1a` are not numbers. A member
/// without a numeric name, or with a number below `first`, gets its own
/// finding and does not interrupt the sequence; a number out of order is
/// reported against the member below it. Ordering needs every member's order
/// value, so a member without one makes the whole anchor not evaluated.
///
/// With `order_fallback` `placement_height`, a member with no order value
/// (absent or null) is ordered by the height of its placement origin, in
/// metres, from the object-frame service instead: a storey without an
/// `Elevation` by where it is placed. A member with neither, or with a
/// placement the service cannot read exactly, still leaves the anchor not
/// evaluated, and without the service it is a missing service. A present
/// order value that is not a number is never replaced.
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `name_sequence` list, each member in order with its number and
/// the number the sequence expects of it, judged on the member.
pub struct NameSequence;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

pub(crate) struct Member<'a> {
    pub(crate) object: &'a Object,
    pub(crate) order: f64,
    pub(crate) name: Option<PropertyValue>,
    pub(crate) evidence: Vec<Evidence>,
}

impl RuleCapability for NameSequence {
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

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("member_selector", ParameterType::Selector),
        ParameterDescriptor::required("name", ParameterType::PropertyReference),
        ParameterDescriptor::required("order", ParameterType::PropertyReference),
        ParameterDescriptor::optional("first", ParameterType::Integer),
        ParameterDescriptor::optional("increment", ParameterType::Integer),
        ParameterDescriptor::optional("order_fallback", ParameterType::String),
    ]
    .into_iter()
    .chain(crate::support::traversal_parameters())
    .collect()
}

pub(crate) struct Config<'a> {
    /// The member selector, where the declaration was read from a rule.
    #[cfg(feature = "parity-reference")]
    pub(crate) members: Option<&'a Selector>,
    pub(crate) name: PropertyRef<'a>,
    pub(crate) order: PropertyRef<'a>,
    pub(crate) first: i64,
    pub(crate) increment: i64,
    pub(crate) traversal: Option<Traversal>,
    /// Order a member without an order value by its placement height.
    pub(crate) placement_fallback: bool,
}

impl<'a> Config<'a> {
    /// The declaration, read and refused as the capability always read it;
    /// the member selector too where `selector` holds (a measured list is
    /// handed the objects it picks instead).
    pub(crate) fn read(parameters: &Parameters<'a>, selector: bool) -> Result<Self, Unavailable> {
        let increment = parameters.integer("increment")?.unwrap_or(1);
        if increment <= 0 {
            return Err(invalid("increment must be positive"));
        }
        let placement_fallback = match parameters.string("order_fallback")? {
            None => false,
            Some("placement_height") => true,
            Some(other) => {
                return Err(invalid(format!(
                    "order_fallback `{other}` is unsupported; use `placement_height`"
                )));
            }
        };
        let members = if selector {
            Some(parameters.required_selector("member_selector")?)
        } else {
            None
        };
        // Only the parity reference keeps the member selector; the measured
        // values take it as an argument.
        #[cfg(not(feature = "parity-reference"))]
        let _ = members;
        Ok(Config {
            #[cfg(feature = "parity-reference")]
            members,
            name: parameters.required_property("name")?,
            order: parameters.required_property("order")?,
            first: parameters.integer("first")?.unwrap_or(1),
            increment,
            traversal: parameters.traversal()?,
            placement_fallback,
        })
    }
}

/// Checks the rule parameters the measured `name_sequence` is handed, as
/// the rule states them: the capability's declaration, in its order and
/// words.
pub(crate) fn check_arguments(
    arguments: &std::collections::BTreeMap<String, axioval_ir::contract::ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    Config::read(&Parameters(&rule), true).map(|_| ())
}

/// The anchor's members in order, among `candidates` (the objects the member
/// selector picks, in the project's order), or why they cannot be ordered:
/// first, the member selection's first undecided object.
pub(crate) fn members<'a>(
    context: &RuleContext<'a>,
    config: &Config<'_>,
    anchor: &Object,
    (candidates, undecided): (&[ObjectId], Option<Unavailable>),
) -> Result<Vec<Member<'a>>, Unavailable> {
    if let Some((reason, message)) = undecided {
        return Err((reason, format!("member selection is undecided: {message}")));
    }
    let (reached, relation_evidence): (Vec<ObjectId>, Vec<Evidence>) = match &config.traversal {
        Some(traversal) => traversal.related_ids(context, &anchor.id, candidates)?,
        None => (
            candidates
                .iter()
                .filter(|member| member.source == anchor.id.source && **member != anchor.id)
                .cloned()
                .collect(),
            Vec::new(),
        ),
    };
    let mut members = Vec::new();
    for id in reached {
        let object = context
            .project
            .object(&id)
            .ok_or_else(|| invalid(format!("member {id} is not in the project")))?;
        let order = resolve(context, object, config.order)?;
        let mut evidence = relation_evidence.clone();
        evidence.extend(order.evidence());
        let order_value = match order.value() {
            Some(PropertyValue::Integer(value)) => {
                #[allow(clippy::cast_precision_loss)]
                let value = *value as f64;
                value
            }
            Some(PropertyValue::Decimal(value) | PropertyValue::Quantity { value, .. }) => *value,
            None | Some(PropertyValue::Null) if config.placement_fallback => {
                let (height, placement) = placement_height(context, &id)?;
                evidence.push(placement);
                height
            }
            other => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{id} has no numeric {} ({}), so the order is unknown",
                        config.order,
                        crate::support::display(other)
                    ),
                ));
            }
        };
        let name = resolve(context, object, config.name)?;
        evidence.extend(name.evidence());
        members.push(Member {
            object,
            order: order_value,
            name: name.value().cloned(),
            evidence,
        });
    }
    members.sort_by(|left, right| {
        left.order
            .total_cmp(&right.order)
            .then_with(|| name_text(left).cmp(&name_text(right)))
            .then_with(|| left.object.id.cmp(&right.object.id))
    });
    Ok(members)
}

/// The height of `member`'s placement origin in metres, with its evidence.
fn placement_height(
    context: &RuleContext<'_>,
    member: &ObjectId,
) -> Result<(f64, Evidence), Unavailable> {
    let frames = context
        .services
        .get::<ObjectFrameServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                format!(
                    "{member} has no order value, and ordering it by its placement height \
                     needs the object-frame service"
                ),
            )
        })?;
    let frame = frames.object_frame(member).map_err(|error| {
        let (reason, message) = crate::body_extent::frame_error(&error);
        (
            reason,
            format!("{member} has no order value, and its placement height is unknown: {message}"),
        )
    })?;
    Ok((
        frame.frame().origin().coordinates_metres()[2],
        frame.evidence().clone(),
    ))
}

fn name_text(member: &Member<'_>) -> String {
    match &member.name {
        Some(PropertyValue::String(text)) => text.clone(),
        other => crate::support::display(other.as_ref()),
    }
}

/// A strict whole number: optional sign and ASCII digits, nothing else.
pub(crate) fn number(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// What a member's name states of its number.
pub(crate) enum Numbered {
    /// Nothing (`undefined`).
    Unset,
    /// Something that is no whole number.
    Word,
    /// A whole number.
    Number(i64),
}

/// A member's number, as its name states it.
pub(crate) fn numbered(name: Option<&PropertyValue>) -> Numbered {
    if crate::support::undefined(name) {
        return Numbered::Unset;
    }
    match name {
        Some(PropertyValue::String(text)) => number(text).map_or(Numbered::Word, Numbered::Number),
        Some(PropertyValue::Integer(value)) => Numbered::Number(*value),
        _ => Numbered::Word,
    }
}
