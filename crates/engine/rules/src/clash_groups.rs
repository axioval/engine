//! Grouping clash findings into reviewable issues.
//!
//! `clash` and `clash-matrix` report one finding per pair. A duct through
//! forty identical walls is then forty findings about one problem. With
//! `group_by`, reported pairs sharing a key become one finding that relates
//! every object involved and carries each pair's evidence:
//!
//! - `subject`: every pair of one subject;
//! - `type_pair`: every pair of the same two object types, either way round;
//! - `similar`: pairs of one class between the same two object types (and
//!   values of `group_property`), whose intersection extents round to the
//!   same multiples of `group_tolerance_metres`.
//!
//! With `per_storey`, the key also holds the storeys `storey_path` reaches
//! from the two members together, so the same clash on two storeys is two
//! issues. A clash matrix never groups pairs judged by different cells.
//!
//! Grouping only arranges findings; it never decides one. A pair whose key
//! cannot be read (a storey walk the source refuses, an unreadable property,
//! unmeasured extents or extents straddling a rounding step) is reported on
//! its own and says why, never folded into a group it might not belong to.

use std::collections::BTreeMap;

use axioval_engine::{
    LengthInterval, ParameterDescriptor, ParameterType, ProximityEvidence, RuleContext,
};
use axioval_ir::{Object, ObjectId};

use crate::clash::Class;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, invalid, resolve, undefined,
};

/// How reported pairs are grouped.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupBy {
    TypePair,
    Subject,
    Similar,
}

/// A declared grouping.
pub(crate) struct Grouping<'a> {
    pub(crate) by: GroupBy,
    /// The relationship path from an object to its storey, with `per_storey`.
    storey_path: Option<Vec<String>>,
    /// A property whose values `similar` pairs must share.
    property: Option<PropertyRef<'a>>,
    /// The rounding step of `similar` extents.
    step: f64,
}

/// The grouping parameters `clash` and `clash-matrix` share.
pub(crate) fn grouping_parameters() -> [ParameterDescriptor; 5] {
    [
        ParameterDescriptor::optional("group_by", ParameterType::String),
        ParameterDescriptor::optional("per_storey", ParameterType::Boolean),
        ParameterDescriptor::optional("storey_path", ParameterType::String),
        ParameterDescriptor::optional("group_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("group_tolerance_metres", ParameterType::Number),
    ]
}

/// Reads the declared grouping; `None` when pairs are not grouped.
pub(crate) fn grouping<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<Grouping<'a>>, Unavailable> {
    let by = parameters
        .string("group_by")?
        .map(|by| match by {
            "type_pair" => Ok(GroupBy::TypePair),
            "subject" => Ok(GroupBy::Subject),
            "similar" => Ok(GroupBy::Similar),
            other => Err(invalid(format!(
                "`group_by` `{other}` is not `type_pair`, `subject` or `similar`"
            ))),
        })
        .transpose()?;
    let per_storey = parameters.boolean("per_storey")?.unwrap_or(false);
    let storey_path = parameters.string("storey_path")?;
    let property = parameters.property("group_property")?;
    let step = parameters.number("group_tolerance_metres")?;
    let Some(by) = by else {
        if per_storey || storey_path.is_some() || property.is_some() || step.is_some() {
            return Err(invalid(
                "`per_storey`, `storey_path`, `group_property` and `group_tolerance_metres` \
                 arrange groups, but no `group_by` is declared",
            ));
        }
        return Ok(None);
    };
    let storey_path = match (per_storey, storey_path) {
        (true, Some(path)) => {
            let path: Vec<String> = path.split_whitespace().map(str::to_owned).collect();
            if path.is_empty() {
                return Err(invalid("`storey_path` has no steps"));
            }
            Traversal::path(&path)?;
            Some(path)
        }
        (true, None) => {
            return Err(invalid(
                "`per_storey` needs `storey_path`: the relationship path from an object to its storey",
            ));
        }
        (false, Some(_)) => {
            return Err(invalid(
                "`storey_path` is declared, but `per_storey` is not on",
            ));
        }
        (false, None) => None,
    };
    let step = match (by, step) {
        (GroupBy::Similar, Some(step)) if step > 0.0 && step.is_finite() => step,
        (GroupBy::Similar, _) => {
            return Err(invalid(
                "`group_by` `similar` needs a positive `group_tolerance_metres`",
            ));
        }
        (_, Some(_)) => {
            return Err(invalid(
                "`group_tolerance_metres` rounds the extents of `similar` groups only",
            ));
        }
        (_, None) => 0.0,
    };
    if property.is_some() && by != GroupBy::Similar {
        return Err(invalid("`group_property` keys `similar` groups only"));
    }
    Ok(Some(Grouping {
        by,
        storey_path,
        property,
        step,
    }))
}

/// What groups a pair, beyond the pair itself.
pub(crate) struct Context<'a> {
    /// The measurement, when the pair was measured.
    pub(crate) measured: Option<&'a ProximityEvidence>,
    /// The clash matrix cell that judged the pair.
    pub(crate) cell: Option<usize>,
}

/// A group's key. Fields a grouping does not use stay `None`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub(crate) cell: Option<usize>,
    /// The class of a `similar` group's pairs, set by who judged them.
    pub(crate) class: Option<Class>,
    pub(crate) subject: Option<ObjectId>,
    /// The two sides' types (and property values), in order.
    pub(crate) sides: Option<(String, String)>,
    /// Extents in rounding steps: the narrower and the wider plan axis,
    /// then the vertical.
    pub(crate) extents: Option<[i64; 3]>,
    pub(crate) storeys: Option<Vec<ObjectId>>,
}

impl Key {
    /// The two sides as a group's finding names them: `A with B`.
    pub(crate) fn sides_words(&self) -> String {
        self.sides
            .as_ref()
            .map(|(a, b)| format!("{a} with {b}"))
            .unwrap_or_default()
    }

    /// Where a group lies, as its finding names it: ` on s1, s2`, ` on no
    /// storey`, or nothing without `per_storey`.
    pub(crate) fn on_words(&self) -> String {
        match self.storeys.as_deref() {
            None => String::new(),
            Some([]) => " on no storey".to_owned(),
            Some(storeys) => format!(
                " on {}",
                storeys
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// The key as one text, its class left out: two keys are equal
    /// exactly where their texts are.
    pub(crate) fn encoded(&self) -> String {
        format!(
            "{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}",
            self.cell, self.subject, self.sides, self.extents, self.storeys
        )
    }
}

type Storeys = Result<Vec<ObjectId>, Unavailable>;

/// Reads the keys of pairs, every storey walk and property read cached per
/// object.
pub(crate) struct GroupKeys<'r> {
    context: &'r RuleContext<'r>,
    pub(crate) grouping: &'r Grouping<'r>,
    storey: Option<Traversal>,
    everything: Vec<&'r Object>,
    storeys: BTreeMap<ObjectId, Storeys>,
    values: BTreeMap<ObjectId, Result<String, Unavailable>>,
}

impl<'r> GroupKeys<'r> {
    pub(crate) fn new(
        context: &'r RuleContext<'r>,
        grouping: &'r Grouping<'r>,
    ) -> Result<Self, Unavailable> {
        Ok(Self {
            context,
            grouping,
            storey: grouping
                .storey_path
                .as_deref()
                .map(Traversal::path)
                .transpose()?,
            everything: context.project.objects().collect(),
            storeys: BTreeMap::new(),
            values: BTreeMap::new(),
        })
    }

    fn storeys_of(&mut self, object: &ObjectId) -> Storeys {
        let Self {
            context,
            storey,
            everything,
            storeys,
            ..
        } = self;
        let Some(traversal) = storey else {
            return Ok(Vec::new());
        };
        storeys
            .entry(object.clone())
            .or_insert_with(|| {
                traversal
                    .related(context, object, everything)
                    .map(|(found, _)| found.into_iter().collect())
            })
            .clone()
    }

    /// An object's type, and its `group_property` value when declared.
    fn side(&mut self, object: &ObjectId) -> Result<String, String> {
        let Some(found) = self.context.project.object(object) else {
            return Err(format!("{object} is not in the project"));
        };
        let kind = found.kind.clone();
        let Some(property) = self.grouping.property else {
            return Ok(kind);
        };
        let context = self.context;
        let value = self
            .values
            .entry(object.clone())
            .or_insert_with(|| {
                let resolved = resolve(context, found, property)?;
                Ok(match resolved.value() {
                    value if undefined(value) => format!("{property} absent"),
                    value => format!("{property} {}", display(value)),
                })
            })
            .clone();
        value
            .map(|value| format!("{kind} ({value})"))
            .map_err(|(_, message)| format!("{property} of {object} cannot be read: {message}"))
    }

    /// The key of the pair `(subject, counterpart)`, its class unset; an
    /// `unmatched` pair (no matrix cell covers it) needs no extents. Why it
    /// cannot be read, where it cannot.
    pub(crate) fn key(
        &mut self,
        pair: &Context<'_>,
        (subject, counterpart): (&ObjectId, &ObjectId),
        unmatched: bool,
    ) -> Result<Key, String> {
        let mut key = Key {
            cell: pair.cell,
            class: None,
            subject: None,
            sides: None,
            extents: None,
            storeys: None,
        };
        match self.grouping.by {
            GroupBy::Subject => key.subject = Some(subject.clone()),
            GroupBy::TypePair | GroupBy::Similar => {
                let (a, b) = (self.side(subject)?, self.side(counterpart)?);
                key.sides = Some(if a <= b { (a, b) } else { (b, a) });
            }
        }
        if self.grouping.by == GroupBy::Similar && !unmatched {
            let extents = pair
                .measured
                .and_then(ProximityEvidence::overlap_extents)
                .ok_or("its intersection extents were not measured")?;
            let step = |interval: LengthInterval| -> Result<i64, String> {
                #[allow(clippy::cast_possible_truncation)]
                let [lower, upper] =
                    [interval.lower_metres(), interval.upper_metres()].map(|value| {
                        (value / self.grouping.step).round().clamp(-9.0e15, 9.0e15) as i64
                    });
                if lower == upper {
                    Ok(lower)
                } else {
                    Err("its intersection extents straddle a rounding step".to_owned())
                }
            };
            let (x, y) = (step(extents.x())?, step(extents.y())?);
            key.extents = Some([x.min(y), x.max(y), step(extents.z())?]);
        }
        if self.storey.is_some() {
            let mut storeys = Vec::new();
            for member in [subject, counterpart] {
                match self.storeys_of(member) {
                    Ok(found) => storeys.extend(found),
                    Err((_, message)) => {
                        return Err(format!("the storey of {member} cannot be read: {message}"));
                    }
                }
            }
            storeys.sort();
            storeys.dedup();
            key.storeys = Some(storeys);
        }
        Ok(key)
    }
}
