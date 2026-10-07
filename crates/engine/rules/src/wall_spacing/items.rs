//! What `wall-spacing` judges of a storey, as a measured member list
//! (`wall_spacing`), measured as the capability measures it: the members
//! the path reaches among those the bound member selection picks, each pair
//! parallel and facing as `Storey::pairs` finds it, and the bands between
//! pairs at most the maximum apart over each footprint the storey reaches.
//!
//! Its items: each pair surely parallel, facing and selected with its plan
//! distance; why some pair may stand closer than the minimum, the words of
//! the capability's one doubt; and each footprint's area outside every
//! band, or why it cannot be measured. The template judges the distances
//! against the minimum and the areas against the area allowed.

use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{
    MeasuredMember, MeasuredMemo, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PropertyResolutionError, RuleContext,
};
#[cfg(feature = "parity-reference")]
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection, SelectionIdentity};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use super::{Bands, Config, Coverage, Members, Services, Storey, uncovered};
use crate::measured_kinds::{interval, refused};
use crate::orientation::Tri;
use crate::plan_area::shown;
use crate::support::{Traversal, Unavailable, everything, invalid};

/// Measures what `wall-spacing` judges of a storey.
pub(crate) struct SpacingItems;

const LIST: &str = "wall_spacing";

fn number(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => Some(*value),
        _ => None,
    }
}

fn path(call: &MeasuredCall, key: &str) -> Result<Option<Traversal>, Unavailable> {
    match call.argument(key) {
        Some(MeasuredArgument::Path(steps)) => Traversal::path(steps).map(Some),
        _ => Ok(None),
    }
}

fn selection<'c>(call: &'c MeasuredCall, key: &str) -> Option<&'c Arc<MeasuredSelection>> {
    match call.argument(key) {
        Some(MeasuredArgument::Objects(selection)) => Some(selection),
        _ => None,
    }
}

fn truth(value: bool) -> MemberValue {
    MemberValue::Truth {
        value,
        locator: LIST.to_owned(),
    }
}

fn text(text: String) -> MemberValue {
    MemberValue::Text { text }
}

fn member(exact: bool, fields: Vec<(&'static str, MemberValue)>) -> MeasuredMember {
    MeasuredMember {
        certain: true,
        exact,
        evidence: Vec::new(),
        fields: fields.into_iter().collect(),
    }
}

fn exact<'e>(evidence: impl IntoIterator<Item = &'e Evidence>) -> bool {
    evidence.into_iter().all(|evidence| evidence.exact)
}

/// A footprint's item that cannot be measured, for its reason.
fn refusal((reason, why): Unavailable) -> MeasuredMember {
    member(
        true,
        [("cover", truth(true))]
            .into_iter()
            .chain(crate::measured_kinds::refused_field(
                "uncovered",
                (reason, why),
            ))
            .collect(),
    )
}

/// The memo key of a selection's objects in the project's order.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Listed(SelectionIdentity, bool);

/// The objects a selection picks (and, with `undecided`, those it leaves
/// undecided), in the project's order, listed once per selection.
fn listed(
    context: &RuleContext<'_>,
    selection: &Arc<MeasuredSelection>,
    undecided: bool,
) -> Arc<Vec<ObjectId>> {
    MeasuredMemo::of(
        context.services,
        Listed(SelectionIdentity(selection.clone()), undecided),
        || {
            Arc::new(
                context
                    .project
                    .objects()
                    .filter(|object| {
                        selection.matched.contains(&object.id)
                            || (undecided && selection.undecided.contains(&object.id))
                    })
                    .map(|object| object.id.clone())
                    .collect(),
            )
        },
    )
}

/// The items of one storey.
#[allow(clippy::too_many_lines)]
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let member_path =
        path(call, "member_path")?.ok_or_else(|| invalid("`member_path` is required"))?;
    let minimum = number(call, "minimum");
    let coverage = match (
        number(call, "maximum"),
        path(call, "footprint_path")?,
        number(call, "uncovered_above"),
    ) {
        (Some(maximum), Some(footprint_path), Some(threshold)) => {
            #[cfg(not(feature = "parity-reference"))]
            let _ = threshold;
            Some(Coverage {
                maximum,
                #[cfg(feature = "parity-reference")]
                footprints: &Selector::All,
                footprint_path,
                #[cfg(feature = "parity-reference")]
                threshold,
                marker: std::marker::PhantomData,
            })
        }
        _ => None,
    };
    let config = Config {
        #[cfg(feature = "parity-reference")]
        members: &Selector::All,
        member_path,
        tolerance: number(call, "angle_tolerance").unwrap_or(0.0),
        minimum,
        coverage,
    };
    let services = Services::of(context, &config)?;
    let picked = selection(call, "members").ok_or_else(|| invalid("`members` is required"))?;
    let storey_object = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    let universe = listed(context, picked, true);
    // The pairs read only which members are matched.
    let members = Members {
        matched: &picked.matched,
        #[cfg(feature = "parity-reference")]
        universe: &[],
    };
    #[cfg(not(feature = "parity-reference"))]
    let _ = storey_object;
    let storey = Storey {
        #[cfg(feature = "parity-reference")]
        context,
        config: &config,
        services: &services,
        #[cfg(feature = "parity-reference")]
        object: storey_object,
    };
    let (reached, mut evidence) =
        config
            .member_path
            .related_among(context, object, &universe, &everything(context))?;
    let reach = config
        .minimum
        .unwrap_or(0.0)
        .max(config.coverage.as_ref().map_or(0.0, |c| c.maximum));
    let (pairs, blind) = storey.pairs(&reached, &members, reach)?;
    for pair in &pairs {
        evidence.extend(pair.evidence.iter().cloned());
    }
    let mut items = Vec::new();
    if let Some(minimum) = config.minimum {
        let mut unknown: Vec<String> = blind.clone();
        for pair in &pairs {
            let (low, high) = pair.distance;
            if pair.paired == Tri::Yes && high.is_finite() {
                items.push(member(
                    exact(&pair.evidence),
                    vec![
                        ("close", truth(true)),
                        (
                            "distance",
                            MemberValue::Measured(interval(
                                (low, high),
                                Some(QuantityDimension::Length),
                                exact(&pair.evidence),
                                format!("{LIST}:{}:{}", pair.first, pair.second),
                            )),
                        ),
                        ("apart", text(shown(low, high))),
                        (
                            "pair",
                            MemberValue::Objects {
                                objects: vec![pair.first.clone(), pair.second.clone()],
                            },
                        ),
                    ],
                ));
            }
            // Why the pair may stand closer, as the capability words it.
            if pair.paired.and(Tri::of(high < minimum, low >= minimum)) == Tri::Maybe {
                let mut why = pair.why.clone();
                if why.is_empty() {
                    why.push(format!(
                        "{} and {} may be closer than {minimum} m ({} m)",
                        pair.first,
                        pair.second,
                        shown(low, high)
                    ));
                }
                unknown.extend(why);
            }
        }
        if !unknown.is_empty() {
            items.push(member(
                true,
                vec![
                    ("doubt", truth(true)),
                    (
                        "spacing",
                        MemberValue::Undecided {
                            why: format!(
                                "minimum spacing: whether every parallel pair stands {minimum} m \
                                 apart is unknown: {}",
                                unknown.join("; ")
                            ),
                        },
                    ),
                ],
            ));
        }
    }
    let (Some(coverage), Some(areas)) = (&config.coverage, services.areas) else {
        return Ok(items);
    };
    let maximum = coverage.maximum;
    let Bands {
        least,
        most,
        mut unknown,
        related,
    } = Bands::of(&pairs, maximum);
    unknown.splice(0..0, blind.iter().cloned());
    let footprints_picked =
        selection(call, "footprints").ok_or_else(|| invalid("`footprints` is required"))?;
    if let Some((reason, message)) = &footprints_picked.first_undecided {
        items.push(refusal((
            reason.clone(),
            format!("the storey's footprint objects are undecided: {message}"),
        )));
        return Ok(items);
    }
    let matched = listed(context, footprints_picked, false);
    let (footprints, mut cited) =
        match coverage
            .footprint_path
            .related_among(context, object, &matched, &everything(context))
        {
            Ok(found) => found,
            Err(unavailable) => {
                items.push(refusal(unavailable));
                return Ok(items);
            }
        };
    if footprints.is_empty() {
        items.push(refusal((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{object} reaches no footprint object, so it has no gross footprint to cover"),
        )));
        return Ok(items);
    }
    cited.extend(evidence.iter().cloned());
    let doubts: String = unknown
        .iter()
        .flat_map(|reason| ["; ", reason.as_str()])
        .collect();
    for footprint in footprints {
        let (lower, upper) = match uncovered(areas, &footprint, (&least, &most), unknown.is_empty())
        {
            Ok((area, measured)) => {
                cited.extend(measured);
                area
            }
            Err(unavailable) => {
                items.push(refusal(unavailable));
                continue;
            }
        };
        let mut objects: BTreeSet<ObjectId> = related.iter().cloned().collect();
        objects.insert(footprint.clone());
        items.push(member(
            exact(&cited),
            vec![
                ("cover", truth(true)),
                (
                    "uncovered",
                    MemberValue::Measured(interval(
                        (lower, upper),
                        None,
                        exact(&cited),
                        format!("{LIST}:{footprint}:uncovered"),
                    )),
                ),
                (
                    "what",
                    text(format!(
                        "{} m² of {footprint} lies outside every band between parallel members \
                         at most {maximum} m apart",
                        shown(lower, upper)
                    )),
                ),
                ("unknown", text(doubts.clone())),
                (
                    "related",
                    MemberValue::Objects {
                        objects: objects.into_iter().collect(),
                    },
                ),
            ],
        ));
    }
    Ok(items)
}

impl MeasuredProvider for SpacingItems {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LIST]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        items(call, object, context).map_err(refused(call.name(), object))
    }
}
