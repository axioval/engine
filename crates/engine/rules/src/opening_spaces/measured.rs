//! What `opening-spaces` reads, as measured values:
//!
//! - `connected_spaces` (members, of an element): one item, the element's
//!   host walls and their exposure, how many spaces it relates to (decided
//!   and possible) against how many the exposure needs, and, for the
//!   derived adjacency, whether those spaces lie on the faces it needs. An
//!   element whose hosts or spaces cannot be read is refused, worded as the
//!   capability left it open.
//! - `any_external`, `host_walls`, `undeclared_hosts`, `possible_hosts`
//!   (of a source): whether any of the source's walls declares itself
//!   external (read in identity order up to the first that does), how
//!   many walls it holds, how many of them declare nothing where none
//!   declares itself external, and how many more objects may be walls.
//!
//! The selections are the rule's, bound into the call and borrowed, and
//! each host's declaration is read once per run for the property.

use std::sync::Arc;

use axioval_engine::{
    ArgumentsKey, Citation, MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement,
    MemberValue, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, ObjectId, SourceId};

use super::{Declaration, declaration, host, needs, sides, space_traversal, universe};
use crate::counts::{Population, every_object};
use crate::measured_kinds::{interval, resolution_error, selection_cow};
use crate::opening_area::Picks;
use crate::support::{PropertyRef, Traversal, Unavailable, invalid};

/// The member list measured here.
const CONNECTED_SPACES: &str = "connected_spaces";
/// Whether any of a source's walls declares itself external.
const ANY_EXTERNAL: &str = "any_external";
/// How many walls a source holds.
const HOST_WALLS: &str = "host_walls";
/// How many of a source's walls declare nothing usable.
const UNDECLARED_HOSTS: &str = "undeclared_hosts";
/// How many more of a source's objects may be walls.
const POSSIBLE_HOSTS: &str = "possible_hosts";

/// Measures what `opening-spaces` reads.
pub(crate) struct HostMeasures;

/// The objects a selection argument picks: the selection bound into the
/// call, borrowed, or every object where the call names none.
enum Picked<'c> {
    Selected(std::borrow::Cow<'c, MeasuredSelection>),
    Every(Arc<Population>),
}

impl Picked<'_> {
    fn picks(&self) -> Picks<'_> {
        match self {
            Self::Selected(selection) => Picks::selected(selection),
            Self::Every(population) => Picks::of(population),
        }
    }
}

/// The objects the selection argument `key` picks, every object where the
/// call names none (made once per run).
fn picked<'c>(
    call: &'c MeasuredCall,
    key: &str,
    context: &RuleContext<'_>,
) -> Result<Picked<'c>, Unavailable> {
    Ok(
        match selection_cow(context, call, key).map_err(crate::selection::property_error)? {
            Some(selection) => Picked::Selected(selection),
            None => Picked::Every(every_object(context)),
        },
    )
}

/// The property a host declares its exposure under.
fn external(call: &MeasuredCall) -> Result<PropertyRef<'_>, Unavailable> {
    match call.argument("external_property") {
        Some(MeasuredArgument::Property { set, name }) => Ok(PropertyRef {
            set: set.as_deref(),
            name,
        }),
        _ => Err(invalid("parameter `external_property` is required")),
    }
}

/// The key of a host's declaration in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct DeclarationKey(ArgumentsKey, ObjectId);

/// Whether `host` declares itself external, read once per run.
fn declared(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    key: &ArgumentsKey,
    host: &ObjectId,
) -> Declaration {
    let property = external(call)?;
    MeasuredMemo::of(
        context.services,
        DeclarationKey(key.clone(), host.clone()),
        || declaration(context, property, host),
    )
}

fn path(call: &MeasuredCall, key: &str) -> Result<Traversal, Unavailable> {
    match call.argument(key) {
        Some(MeasuredArgument::Path(steps)) => Traversal::path(steps),
        _ => Err(invalid(format!("parameter `{key}` is required"))),
    }
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn number(value: usize, locator: &str) -> MemberValue {
    #[allow(clippy::cast_precision_loss)]
    let value = value as f64;
    MemberValue::Measured(interval((value, value), None, true, locator.to_owned()))
}

impl HostMeasures {
    /// The element's one item, and the evidence its hosts and spaces were
    /// read from.
    fn element(
        call: &MeasuredCall,
        element: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let hosts = picked(call, "host_selector", context)?;
        let spaces = picked(call, "space_selector", context)?;
        let key = ArgumentsKey::of_keys(call, &["external_property"]);
        let (hosts_reached, exposed, mut evidence) = host(
            context,
            (&path(call, "host_path")?, hosts.picks()),
            external(call)?,
            element,
            &mut |host| declared(call, context, &key, host),
        )?;
        let Some(MeasuredArgument::Path(steps)) = call.argument("space_path") else {
            return Err(invalid("parameter `space_path` is required"));
        };
        let (traversal, sided) = space_traversal(steps)?;
        let (related, cited) =
            traversal.related(context, element, &universe(context, spaces.picks()))?;
        let decided: Vec<ObjectId> = related
            .iter()
            .filter(|id| spaces.picks().matched.contains(*id))
            .cloned()
            .collect();
        let undecided = related.len() - decided.len();
        let count = decided.len();
        let via = &traversal.relationship;
        let (wall, expected, requirement) = needs(exposed);
        // The faces are read only where the spaces are exactly those the
        // requirement counts.
        let placed = if sided && count == expected && undecided == 0 {
            sides(element, &decided, &cited, exposed)?
        } else {
            None
        };
        let locator = format!("{CONNECTED_SPACES}:{element}");
        let exact = evidence.iter().chain(&cited).all(|evidence| evidence.exact);
        let mut related_objects = hosts_reached.clone();
        related_objects.extend(decided);
        related_objects.sort();
        related_objects.dedup();
        let item = MeasuredMember {
            certain: true,
            exact,
            fields: [
                (
                    "relates",
                    text(format!("relates to {count} space(s) via {via}")),
                ),
                ("count", number(count, &locator)),
                ("possible", number(count + undecided, &locator)),
                ("expected", number(expected, &locator)),
                (
                    "known",
                    if undecided == 0 {
                        MemberValue::Truth {
                            value: true,
                            locator: locator.clone(),
                        }
                    } else {
                        MemberValue::Undecided {
                            why: format!(
                                "relates to {count} space(s) via {via} and {undecided} more \
                                 that may be spaces"
                            ),
                        }
                    },
                ),
                (
                    "sides",
                    MemberValue::Truth {
                        value: placed.is_none(),
                        locator: locator.clone(),
                    },
                ),
                ("placed", text(placed.unwrap_or_default())),
                ("wall", text(wall.to_owned())),
                (
                    "hosts",
                    text(
                        hosts_reached
                            .iter()
                            .map(|host| host.local_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ),
                ("requirement", text(requirement.to_owned())),
                (
                    "related",
                    MemberValue::Objects {
                        objects: related_objects,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            evidence: Vec::new(),
        };
        evidence.extend(cited);
        Ok((vec![item], evidence))
    }

    /// What a source's walls declare, as the value `call` names.
    fn source(
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let hosts = picked(call, "host_selector", context)?;
        let hosts = hosts.picks();
        let walls = || hosts.matched.iter().filter(|host| host.source == *source);
        let possible = || {
            hosts
                .undecided
                .iter()
                .filter(|host| host.source == *source)
                .count()
        };
        let counted = |count: usize, name: &str| {
            #[allow(clippy::cast_precision_loss)]
            let count = count as f64;
            interval((count, count), None, true, format!("{name}: {source}"))
        };
        let name = call.name();
        match name {
            HOST_WALLS => return Ok((counted(walls().count(), name), Citation::default())),
            POSSIBLE_HOSTS => return Ok((counted(possible(), name), Citation::default())),
            _ => {}
        }
        // The walls are read in identity order up to the first declared
        // external, as the capability read them: past it nothing counts.
        let key = ArgumentsKey::of_keys(call, &["external_property"]);
        let mut unknown = 0_usize;
        let mut evidence = Vec::new();
        let mut any = false;
        for wall in walls() {
            match declared(call, context, &key, wall) {
                Ok((true, cited)) => {
                    any = true;
                    evidence = cited;
                    break;
                }
                Ok((false, cited)) => evidence.extend(cited),
                Err(_) => unknown += 1,
            }
        }
        let exact = evidence.iter().all(|evidence| evidence.exact);
        if name == UNDECLARED_HOSTS {
            let undeclared = if any { 0 } else { unknown };
            return Ok((counted(undeclared, name), Citation::default()));
        }
        let (lower, upper) = match (any, unknown + possible()) {
            (true, _) => (1.0, 1.0),
            (false, 0) => (0.0, 0.0),
            (false, _) => (0.0, 1.0),
        };
        Ok((
            interval((lower, upper), None, exact, format!("{name}: {source}")),
            Citation {
                // A finding relates the source's walls.
                related: if any {
                    Vec::new()
                } else {
                    walls().cloned().collect()
                },
                evidence,
                ..Citation::default()
            },
        ))
    }
}

impl MeasuredProvider for HostMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[ANY_EXTERNAL, HOST_WALLS, POSSIBLE_HOSTS, UNDECLARED_HOSTS]
    }

    /// Each host's declaration is kept once per run, which every value of
    /// a source counts again cheaply; the values are not kept twice.
    fn memoizes(&self) -> bool {
        true
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[CONNECTED_SPACES]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn measure_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::source(call, source, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {source}: {why}", call.name())))
        })
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    /// The item's findings cite its hosts' declarations and the spaces'
    /// relationship evidence.
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        Self::element(call, object, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
