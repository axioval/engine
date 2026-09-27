//! Direct access between spaces, and from a space to the outside, through
//! the doors and openings on their boundaries.
//!
//! Shared by `space-connection`, `space-distance` and `effective-coverage`.
//! Each door or opening reaches the spaces it connects through `access_path`: a relationship the
//! model states (such as `IfcRelSpaceBoundary` backward, from the element to
//! the spaces it bounds) or `axioval:derived.adjacent-space`, whose evidence
//! records the face of the element each space lies on and which face opens
//! to the outside. Two spaces have direct access through an element when it
//! reaches both, on opposite faces when faces are recorded; a space opens to
//! the outside through an element whose other face enters no space, which
//! only a sided derivation can say.
//!
//! Every answer is three-valued. A link through an element that is surely
//! a door or opening of the asked type stands; one through an element whose
//! type is undecided, or an element whose spaces could not be read (it might
//! connect anything), leaves the answer unknown rather than "no".

use std::collections::BTreeMap;

use axioval_engine::{AdjacentSide, NotEvaluatedReason, RuleContext, TraversalDirection};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::counts::Population;
use crate::opening_spaces::is_adjacency;
use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// Which kinds of element may give access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessType {
    Any,
    Doors,
    Openings,
}

impl AccessType {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, Unavailable> {
        match value.unwrap_or("any") {
            "any" => Ok(Self::Any),
            "doors" => Ok(Self::Doors),
            "openings" => Ok(Self::Openings),
            other => Err(invalid(format!(
                "access type `{other}` is unsupported (any, doors, openings)"
            ))),
        }
    }

    /// How findings name the elements of this type.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Any => "door or opening",
            Self::Doors => "door",
            Self::Openings => "opening",
        }
    }
}

/// Whether an object is a member of a selection: surely, surely not, or
/// undecided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Member {
    Yes,
    No,
    Undecided,
}

impl Member {
    fn of(selection: &Selection) -> Self {
        match selection {
            Selection::Match => Self::Yes,
            Selection::NoMatch => Self::No,
            Selection::NotEvaluated(..) => Self::Undecided,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            (Self::No, Self::No) => Self::No,
            _ => Self::Undecided,
        }
    }
}

/// What an element reaches: its spaces with the face each lies on (when
/// faces are recorded), its faces that open to the outside, and the
/// relationship evidence.
struct Reach {
    spaces: Vec<(ObjectId, Option<AdjacentSide>)>,
    outside: Vec<AdjacentSide>,
    evidence: Vec<Evidence>,
}

/// A door or opening and what it connects.
struct Element {
    id: ObjectId,
    door: Member,
    opening: Member,
    reach: Result<Reach, String>,
}

impl Element {
    fn member(&self, access: AccessType) -> Member {
        match access {
            AccessType::Any => self.door.or(self.opening),
            AccessType::Doors => self.door,
            AccessType::Openings => self.opening,
        }
    }
}

/// A space linked to another, or to the outside, through an element.
#[derive(Clone, Debug)]
pub(crate) enum Link {
    /// Through `via`, surely a door or opening of the asked type.
    Sure {
        via: ObjectId,
        evidence: Vec<Evidence>,
    },
    /// Possibly, for the reason given.
    Maybe(String),
}

/// The spaces directly linked with one space, and why other links may exist
/// unseen.
pub(crate) struct Partners {
    pub(crate) linked: BTreeMap<ObjectId, Link>,
    /// Elements whose spaces could not be read: any space may be linked
    /// through them.
    pub(crate) unknown: Vec<String>,
}

impl Partners {
    /// Whether `other` is directly linked: `Ok(Some)` surely, `Ok(None)`
    /// surely not, `Err` unknown.
    pub(crate) fn with(&self, other: &ObjectId) -> Result<Option<Link>, String> {
        match self.linked.get(other) {
            Some(Link::Sure { via, evidence }) => Ok(Some(Link::Sure {
                via: via.clone(),
                evidence: evidence.clone(),
            })),
            Some(Link::Maybe(why)) => Err(why.clone()),
            None if self.unknown.is_empty() => Ok(None),
            None => Err(self.unknown.join("; ")),
        }
    }
}

/// The entrances of a space.
#[derive(Default)]
pub(crate) struct Entrances {
    /// Surely an entrance, with the relationship evidence.
    pub(crate) sure: Vec<(ObjectId, Vec<Evidence>)>,
    /// Possibly an entrance, with why it is not sure.
    pub(crate) maybe: Vec<(ObjectId, String)>,
}

/// A door or opening joining a space to another.
pub(crate) struct Connection {
    pub(crate) via: ObjectId,
    pub(crate) space: ObjectId,
    /// Whether `via` is surely a door or opening.
    pub(crate) certain: bool,
    pub(crate) evidence: Vec<Evidence>,
}

/// Whether a space opens directly to the outside.
pub(crate) enum Exit {
    Sure {
        via: ObjectId,
        evidence: Vec<Evidence>,
    },
    None,
    Unknown(String),
}

/// Every door and opening of the model with the spaces it connects.
pub(crate) struct AccessIndex {
    elements: Vec<Element>,
    /// Whether the path records faces (the derived adjacency).
    pub(crate) sided: bool,
    pub(crate) relationship: String,
}

/// The declaration an access index is built from.
pub(crate) struct AccessDeclaration<'a> {
    path: Traversal<'a>,
    doors: Option<&'a Selector>,
    openings: Option<&'a Selector>,
    spaces: &'a Selector,
    pub(crate) sided: bool,
}

impl<'a> AccessDeclaration<'a> {
    /// Reads `access_path`, `door_selector`, `opening_selector` and
    /// `space_selector`; `None` when `access_path` is not declared.
    pub(crate) fn parse(parameters: &Parameters<'a>) -> Result<Option<Self>, Unavailable> {
        let doors = parameters.selector("door_selector")?;
        let openings = parameters.selector("opening_selector")?;
        let spaces = parameters.selector("space_selector")?;
        let Some(path) = parameters.strings("access_path")? else {
            if doors.is_some() || openings.is_some() || spaces.is_some() {
                return Err(invalid(
                    "`door_selector`, `opening_selector` and `space_selector` need `access_path`",
                ));
            }
            return Ok(None);
        };
        if doors.is_none() && openings.is_none() {
            return Err(invalid(
                "`access_path` needs `door_selector`, `opening_selector` or both",
            ));
        }
        let path = Traversal::path(path)?;
        let adjacency: Vec<bool> = path
            .steps()
            .map(|(relationship, _)| is_adjacency(relationship))
            .collect();
        let sided = adjacency.contains(&true);
        if sided
            && (adjacency.len() != 1
                || path
                    .steps()
                    .any(|(_, direction)| direction != TraversalDirection::Forward))
        {
            return Err(invalid(
                "`axioval:derived.adjacent-space` must be the only `access_path` step, forward, \
                 so the faces it records are the element's",
            ));
        }
        Ok(Some(Self {
            path,
            doors,
            openings,
            spaces: spaces.unwrap_or(&Selector::All),
            sided,
        }))
    }

    /// Checks that `access` can be told apart with the declared selectors.
    pub(crate) fn admits(&self, access: AccessType) -> Result<(), Unavailable> {
        match access {
            AccessType::Doors if self.doors.is_none() => {
                Err(invalid("access type `doors` needs `door_selector`"))
            }
            AccessType::Openings if self.openings.is_none() => {
                Err(invalid("access type `openings` needs `opening_selector`"))
            }
            _ => Ok(()),
        }
    }

    /// Reads every door and opening of the project and what it connects.
    pub(crate) fn index(&self, context: &RuleContext<'_>) -> AccessIndex {
        let population = Population::of(context, self.spaces);
        let universe: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| population.contains(&object.id))
            .collect();
        let member = |selector: Option<&Selector>, object: &Object| {
            selector.map_or(Member::No, |selector| {
                Member::of(&selector_matches(
                    context,
                    selector,
                    object,
                    &mut Vec::new(),
                ))
            })
        };
        let mut elements = Vec::new();
        for object in context.project.objects() {
            let door = member(self.doors, object);
            let opening = member(self.openings, object);
            if door == Member::No && opening == Member::No {
                continue;
            }
            let reach = self
                .path
                .related(context, &object.id, &universe)
                .map_err(|(_, why)| {
                    format!("the spaces {} connects cannot be read: {why}", object.id)
                })
                .and_then(|(spaces, evidence)| self.reach(&object.id, spaces, evidence));
            elements.push(Element {
                id: object.id.clone(),
                door,
                opening,
                reach,
            });
        }
        AccessIndex {
            elements,
            sided: self.sided,
            relationship: self.path.relationship.clone(),
        }
    }

    fn reach(
        &self,
        element: &ObjectId,
        spaces: Vec<ObjectId>,
        evidence: Vec<Evidence>,
    ) -> Result<Reach, String> {
        if !self.sided {
            return Ok(Reach {
                spaces: spaces.into_iter().map(|space| (space, None)).collect(),
                outside: Vec::new(),
                evidence,
            });
        }
        let mut placed = Vec::new();
        for space in spaces {
            let sides: Vec<AdjacentSide> = evidence
                .iter()
                .filter_map(|item| {
                    axioval_engine::adjacent_side(&item.locator, element, Some(&space))
                })
                .collect();
            match sides.as_slice() {
                [side] => placed.push((space, Some(*side))),
                [] => {
                    return Err(format!(
                        "the adjacency evidence of {element} records no face for space {space}"
                    ));
                }
                // One space on both faces links no two spaces through it.
                _ => {}
            }
        }
        let outside = evidence
            .iter()
            .filter_map(|item| axioval_engine::adjacent_side(&item.locator, element, None))
            .collect();
        Ok(Reach {
            spaces: placed,
            outside,
            evidence,
        })
    }
}

impl AccessIndex {
    /// The relationship evidence of every element that reaches `space`: what
    /// a finding of a missing link rests on.
    pub(crate) fn cited(&self, space: &ObjectId) -> (Vec<ObjectId>, Vec<Evidence>) {
        let mut elements = Vec::new();
        let mut evidence = Vec::new();
        for element in &self.elements {
            if let Ok(reach) = &element.reach
                && reach.spaces.iter().any(|(reached, _)| reached == space)
            {
                elements.push(element.id.clone());
                evidence.extend(reach.evidence.iter().cloned());
            }
        }
        (elements, evidence)
    }

    /// Every door and opening that reaches `space`, with its relationship
    /// evidence; `Err` when an element whose spaces cannot be read, or whose
    /// type is undecided and that reaches the space, might be one more.
    pub(crate) fn reaching(
        &self,
        space: &ObjectId,
    ) -> Result<(Vec<ObjectId>, Vec<Evidence>), String> {
        let mut elements = Vec::new();
        let mut evidence = Vec::new();
        for element in &self.elements {
            let member = element.member(AccessType::Any);
            if member == Member::No {
                continue;
            }
            let reach = element.reach.as_ref().map_err(Clone::clone)?;
            if !reach.spaces.iter().any(|(reached, _)| reached == space) {
                continue;
            }
            if member == Member::Undecided {
                return Err(format!(
                    "whether {} is a door or opening is undecided",
                    element.id
                ));
            }
            elements.push(element.id.clone());
            evidence.extend(reach.evidence.iter().cloned());
        }
        Ok((elements, evidence))
    }

    /// The spaces `space` has direct access to through an element of the
    /// `access` type.
    pub(crate) fn partners(&self, space: &ObjectId, access: AccessType) -> Partners {
        let mut linked: BTreeMap<ObjectId, Link> = BTreeMap::new();
        let mut unknown = Vec::new();
        for element in &self.elements {
            let member = element.member(access);
            if member == Member::No {
                continue;
            }
            let reach = match &element.reach {
                Ok(reach) => reach,
                Err(why) => {
                    unknown.push(why.clone());
                    continue;
                }
            };
            let Some(side) = reach
                .spaces
                .iter()
                .find(|(reached, _)| reached == space)
                .map(|(_, side)| *side)
            else {
                continue;
            };
            for (other, other_side) in &reach.spaces {
                if other == space || (self.sided && *other_side == side) {
                    continue;
                }
                let link = if member == Member::Yes {
                    Link::Sure {
                        via: element.id.clone(),
                        evidence: reach.evidence.clone(),
                    }
                } else {
                    Link::Maybe(format!(
                        "whether {} is a {} is undecided",
                        element.id,
                        access.describe()
                    ))
                };
                match (linked.get(other), &link) {
                    (Some(Link::Sure { .. }), _) | (Some(Link::Maybe(_)), Link::Maybe(_)) => {}
                    _ => {
                        linked.insert(other.clone(), link);
                    }
                }
            }
        }
        Partners { linked, unknown }
    }

    /// Every door or opening joining `space` to another space, with that
    /// space and whether the element surely is one, and why elements whose
    /// spaces cannot be read might join more. Shared with
    /// `effective-coverage`, whose effects continue through them.
    pub(crate) fn connections(&self, space: &ObjectId) -> (Vec<Connection>, Vec<String>) {
        let mut joined = Vec::new();
        let mut unknown = Vec::new();
        for element in &self.elements {
            let member = element.member(AccessType::Any);
            if member == Member::No {
                continue;
            }
            let reach = match &element.reach {
                Ok(reach) => reach,
                Err(why) => {
                    unknown.push(why.clone());
                    continue;
                }
            };
            let Some(side) = reach
                .spaces
                .iter()
                .find(|(reached, _)| reached == space)
                .map(|(_, side)| *side)
            else {
                continue;
            };
            for (other, other_side) in &reach.spaces {
                if other == space || (self.sided && *other_side == side) {
                    continue;
                }
                joined.push(Connection {
                    via: element.id.clone(),
                    space: other.clone(),
                    certain: member == Member::Yes,
                    evidence: reach.evidence.clone(),
                });
            }
        }
        (joined, unknown)
    }

    /// The doors and openings of the `access` type that reach `space`:
    /// surely, with their relationship evidence, or possibly, with why. An
    /// element whose spaces cannot be read may reach any space.
    pub(crate) fn entrances(&self, space: &ObjectId, access: AccessType) -> Entrances {
        let mut entrances = Entrances::default();
        for element in &self.elements {
            let member = element.member(access);
            if member == Member::No {
                continue;
            }
            match &element.reach {
                Err(why) => entrances.maybe.push((element.id.clone(), why.clone())),
                Ok(reach) if reach.spaces.iter().any(|(reached, _)| reached == space) => {
                    if member == Member::Yes {
                        entrances
                            .sure
                            .push((element.id.clone(), reach.evidence.clone()));
                    } else {
                        entrances.maybe.push((
                            element.id.clone(),
                            format!(
                                "whether {} is a {} is undecided",
                                element.id,
                                access.describe()
                            ),
                        ));
                    }
                }
                Ok(_) => {}
            }
        }
        entrances
    }

    /// Whether `space` opens directly to the outside through an element of
    /// the `access` type. Only a sided path records the outside.
    pub(crate) fn exit(&self, space: &ObjectId, access: AccessType) -> Exit {
        let mut unknown = Vec::new();
        for element in &self.elements {
            let member = element.member(access);
            if member == Member::No {
                continue;
            }
            let reach = match &element.reach {
                Ok(reach) => reach,
                Err(why) => {
                    unknown.push(why.clone());
                    continue;
                }
            };
            let opens = reach.spaces.iter().any(|(reached, side)| {
                reached == space
                    && side.is_some_and(|side| reach.outside.contains(&side.opposite()))
            });
            if !opens {
                continue;
            }
            if member == Member::Yes {
                return Exit::Sure {
                    via: element.id.clone(),
                    evidence: reach.evidence.clone(),
                };
            }
            unknown.push(format!(
                "whether {} is a {} is undecided",
                element.id,
                access.describe()
            ));
        }
        if unknown.is_empty() {
            Exit::None
        } else {
            Exit::Unknown(unknown.join("; "))
        }
    }
}

/// The not-evaluated reason for an unknown link.
pub(crate) fn unknown(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}
