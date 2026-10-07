//! The spaces a door, window or opening connects, by its host wall's
//! declared exposure.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    AdjacentSide, CapabilityEvaluation, CompiledRule, Derivation, NotEvaluatedReason,
    ParameterDescriptor, RuleCapability, RuleContext, SemanticRelationship, TraversalDirection,
    adjacent_side,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::opening_area::Picks;
use crate::support::{Parameters, PropertyRef, Resolved, Traversal, Unavailable, invalid, resolve};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::HostMeasures;

/// Requires each selected door, window or opening to relate to the spaces
/// its host wall calls for: two, one on each side, in an internal wall; one
/// in an external wall, the other side being outside.
///
/// The host is what `host_path` reaches among the `host_selector` objects
/// (with IFC, `IfcRelFillsElement` then `IfcRelVoidsElement` backward from a
/// door or window, `IfcRelVoidsElement` backward from an opening). Its
/// boolean `external_property` decides internal or external; a host that
/// does not declare it, or hosts that disagree, leave the element not
/// evaluated. The spaces are what `space_path` reaches among the
/// `space_selector` objects (every object by default): a relationship the
/// model states, or `axioval:derived.adjacent-space`, whose evidence records
/// which face of the element each space lies on, so two spaces on the same
/// face are reported rather than counted as connected. Which kinds are
/// checked (openings, doors, windows) is the rule's selection.
///
/// Each source holding a selected element or a host is also checked as a
/// whole: a source in which no host declares itself external is reported
/// against the source, since a building has an envelope.
///
/// It runs as a template ([`axioval_engine::template`]): each element by
/// the measured list `connected_spaces` (its hosts' exposure and the spaces
/// it relates to), each source by how many of its walls declare themselves
/// external (`any_external`).
pub struct OpeningSpaces;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for OpeningSpaces {
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

/// A host's declared exposure and the facts behind it.
pub(crate) type Declaration = Result<(bool, Vec<Evidence>), Unavailable>;

/// The declaration a rule states, read as the capability read it. Only
/// the parity reference keeps it; `check_arguments` reads it to refuse a
/// bad declaration.
pub(crate) struct Config<'a> {
    #[cfg(feature = "parity-reference")]
    pub(crate) hosts: Traversal,
    #[cfg(feature = "parity-reference")]
    pub(crate) host_selector: &'a Selector,
    #[cfg(feature = "parity-reference")]
    pub(crate) external: PropertyRef<'a>,
    #[cfg(feature = "parity-reference")]
    pub(crate) spaces: Traversal,
    #[cfg(feature = "parity-reference")]
    pub(crate) space_selector: &'a Selector,
    /// Whether the spaces come from the derived adjacency, whose evidence
    /// records sides.
    #[cfg(feature = "parity-reference")]
    pub(crate) sided: bool,
    /// The declaration's lifetime, which only the parity reference reads.
    marker: std::marker::PhantomData<&'a Selector>,
}

impl<'a> Config<'a> {
    pub(crate) fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let host_path = parameters
            .strings("host_path")?
            .ok_or_else(|| invalid("parameter `host_path` is required"))?;
        let space_path = parameters
            .strings("space_path")?
            .ok_or_else(|| invalid("parameter `space_path` is required"))?;
        let (spaces, sided) = space_traversal(space_path)?;
        let hosts = Traversal::path(host_path)?;
        let host_selector = parameters.required_selector("host_selector")?;
        let external = parameters.required_property("external_property")?;
        let space_selector = parameters
            .selector("space_selector")?
            .unwrap_or(&Selector::All);
        #[cfg(not(feature = "parity-reference"))]
        let _ = (
            hosts,
            host_selector,
            external,
            spaces,
            space_selector,
            sided,
        );
        Ok(Self {
            #[cfg(feature = "parity-reference")]
            hosts,
            #[cfg(feature = "parity-reference")]
            host_selector,
            #[cfg(feature = "parity-reference")]
            external,
            #[cfg(feature = "parity-reference")]
            spaces,
            #[cfg(feature = "parity-reference")]
            space_selector,
            #[cfg(feature = "parity-reference")]
            sided,
            marker: std::marker::PhantomData,
        })
    }
}

/// The spaces' traversal and whether it is the derived adjacency, refused
/// as the capability refused a path it cannot read sides from.
pub(crate) fn space_traversal(space_path: &[String]) -> Result<(Traversal, bool), Unavailable> {
    let spaces = Traversal::path(space_path)?;
    let adjacency: Vec<bool> = spaces
        .steps()
        .iter()
        .map(|step| {
            step.relationships()
                .iter()
                .any(|r| is_adjacency(r.as_str()))
        })
        .collect();
    let sided = adjacency.contains(&true);
    if sided
        && (adjacency.len() != 1
            || spaces.steps().iter().any(|step| {
                step.relationships().len() != 1 || step.direction() != TraversalDirection::Forward
            }))
    {
        return Err(invalid(
            "`axioval:derived.adjacent-space` must be the only `space_path` step, forward, \
             so the sides it records are the checked element's",
        ));
    }
    Ok((spaces, sided))
}

/// The declaration the capability refused, in its order and words.
/// `stated` holds the rule's parameters the list names, by the list's keys
/// (the parameters' own names).
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(stated.clone());
    Config::parse(&rule).map(|_| ())
}

/// Whether a relationship identity names the derived adjacency. A malformed
/// derived identity is left to the relationship service to refuse.
pub(crate) fn is_adjacency(relationship: &str) -> bool {
    SemanticRelationship::try_new(relationship)
        .ok()
        .and_then(|relationship| Derivation::parse(&relationship).ok().flatten())
        .is_some_and(|derivation| matches!(derivation, Derivation::AdjacentSpace { .. }))
}

/// Whether `host` declares itself external, as `external` states it.
pub(crate) fn declaration(
    context: &RuleContext<'_>,
    external: PropertyRef<'_>,
    host: &ObjectId,
) -> Declaration {
    match context.project.object(host) {
        Some(object) => match resolve(context, object, external)? {
            Resolved::Present(property) => match &property.value {
                PropertyValue::Boolean(value) => Ok((
                    *value,
                    property.evidence.iter().cloned().collect::<Vec<_>>(),
                )),
                PropertyValue::Null => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("host {host} states no value for `{external}`"),
                )),
                _ => Err((
                    NotEvaluatedReason::InvalidEvidence,
                    format!("`{external}` of host {host} is not a boolean"),
                )),
            },
            Resolved::Absent(_) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("host {host} does not declare `{external}`"),
            )),
        },
        None => Err(invalid(format!("host {host} is not in the project"))),
    }
}

/// The objects `picks` may pick, the universe a traversal may reach.
pub(crate) fn universe<'c>(context: &RuleContext<'c>, picks: Picks<'_>) -> Vec<&'c Object> {
    context
        .project
        .objects()
        .filter(|object| picks.contains(&object.id))
        .collect()
}

/// The element's host walls, whether they are external, and the facts
/// behind both, each host's declaration read through `declared`.
pub(crate) fn host(
    context: &RuleContext<'_>,
    (hosts, population): (&Traversal, Picks<'_>),
    external: PropertyRef<'_>,
    element: &ObjectId,
    declared: &mut dyn FnMut(&ObjectId) -> Declaration,
) -> Result<(Vec<ObjectId>, bool, Vec<Evidence>), Unavailable> {
    let (reached, mut evidence) =
        hosts.related(context, element, &universe(context, population))?;
    if let Some(undecided) = reached.iter().find(|id| !population.matched.contains(*id)) {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("whether {undecided} is a host wall is undecided"),
        ));
    }
    if reached.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "no host wall is reached via {}, so the spaces it needs are unknown",
                hosts.relationship
            ),
        ));
    }
    let mut exposure = BTreeSet::new();
    for host in &reached {
        let (external, cited) = declared(host)?;
        exposure.insert(external);
        evidence.extend(cited);
    }
    if exposure.len() != 1 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("its host walls disagree on `{external}`"),
        ));
    }
    Ok((reached, exposure.contains(&true), evidence))
}

/// What a host's exposure asks of an element's spaces: the wall's words,
/// how many spaces, and the requirement's words.
pub(crate) fn needs(external: bool) -> (&'static str, usize, &'static str) {
    if external {
        ("an external wall", 1, "one space, the other side outside")
    } else {
        ("an internal wall", 2, "two spaces, one on each side")
    }
}

/// Why the derived adjacency's recorded faces fail the host's requirement,
/// `None` when they meet it: an internal host needs one space on each face,
/// an external one its space on one face and the other face outside. It
/// reads exactly the spaces the requirement counts: two for an internal
/// host, one for an external one.
pub(crate) fn sides(
    element: &ObjectId,
    spaces: &[ObjectId],
    cited: &[Evidence],
    external: bool,
) -> Result<Option<String>, Unavailable> {
    let mut sides: BTreeMap<&ObjectId, BTreeSet<AdjacentSide>> = BTreeMap::new();
    for space in spaces {
        let found: BTreeSet<AdjacentSide> = cited
            .iter()
            .filter_map(|item| adjacent_side(&item.locator, element, Some(space)))
            .collect();
        if found.is_empty() {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!("the adjacency evidence records no side for space {space}"),
            ));
        }
        sides.insert(space, found);
    }
    let placed = sides
        .iter()
        .map(|(space, faces)| {
            let faces: Vec<String> = faces.iter().map(ToString::to_string).collect();
            format!("{} on side {}", space.local_id, faces.join(" and "))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let faces: Vec<&BTreeSet<AdjacentSide>> = sides.values().collect();
    if !external {
        let opposite = faces.iter().all(|face| face.len() == 1) && faces[0] != faces[1];
        return Ok((!opposite).then(|| format!("its spaces are not on opposite sides ({placed})")));
    }
    let face: Vec<AdjacentSide> = faces[0].iter().copied().collect();
    let [face] = face[..] else {
        return Ok(Some(format!("its one space lies on both sides ({placed})")));
    };
    let outside = cited
        .iter()
        .any(|item| adjacent_side(&item.locator, element, None) == Some(face.opposite()));
    if !outside {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{placed}; the other side enters an object outside the space selection"),
        ));
    }
    Ok(None)
}
