//! Relationships derived from geometry rather than stated by a source.
//!
//! A model often leaves out relationships a check needs: which space a
//! component stands in, which spaces a door connects, which larger space a
//! room belongs to. A geometry provider can derive them. It answers the same
//! [`RelationshipSelectionRequest`]s as the semantic relationship service, so
//! every capability that takes a `relationship` or a `path` can use a derived
//! relationship without change: the session routes each request whose
//! identity starts with [`DERIVED_RELATIONSHIP_PREFIX`] to the registered
//! [`DerivedRelationshipServiceHandle`] (see
//! [`crate::EvidenceSession::with_derived_relationships`]).
//!
//! The identity names the derivation and fixes its tolerances, so two rules
//! asking with different tolerances ask different questions, and the evidence
//! of every answer names the derivation it came from. Parameters follow the
//! name as `;key=value` pairs in metres (or a plain ratio); an omitted
//! parameter takes its documented default, and an unknown or repeated key, a
//! negative, non-finite or out-of-range value is an invalid request.
//!
//! Edges always run from a subject to a space, so `forward` from an element
//! reaches its spaces and `backward` from a space reaches its subjects.

use std::fmt;
use std::sync::Arc;

use axioval_ir::ObjectId;

use crate::relationships::{
    CompleteRelationshipSelection, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    SemanticRelationship, validate_selection,
};
use crate::session::{SnapshotBoundService, SourceSnapshot};

/// Every derived relationship identity starts with this prefix.
pub const DERIVED_RELATIONSHIP_PREFIX: &str = "axioval:derived.";

/// A derived relationship and the tolerances it was asked with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Derivation {
    /// `axioval:derived.contained-in-space`: an element to the spaces whose
    /// body contains its reference point, or, when none does, to the one
    /// nearest space whose surface lies within `horizontal` metres in plan
    /// and `vertical` metres in elevation of it. Both default to zero, which
    /// asks for containment alone.
    ContainedInSpace {
        /// Largest plan offset to a space's nearest surface point, in metres.
        horizontal_metres: f64,
        /// Largest elevation offset to that point, in metres.
        vertical_metres: f64,
    },
    /// `axioval:derived.adjacent-space`: a door, window or opening to the
    /// spaces a probe first enters on each side of it within `reach` metres
    /// (default 1). A side that enters no space is outside.
    AdjacentSpace {
        /// How far each probe sweeps from the element's face, in metres.
        reach_metres: f64,
    },
    /// `axioval:derived.overlapping-group-space`: a space to every larger
    /// space whose footprint covers at least `ratio` of its own (default
    /// 0.5) and whose vertical extent lies within `vertical` metres of its
    /// own (default 0, touching or overlapping).
    OverlappingGroupSpace {
        /// Smallest share of the space's footprint the group must cover.
        minimum_ratio: f64,
        /// Largest vertical gap between the two extents, in metres.
        vertical_metres: f64,
    },
}

const CONTAINED_IN_SPACE: &str = "contained-in-space";
const ADJACENT_SPACE: &str = "adjacent-space";
const OVERLAPPING_GROUP_SPACE: &str = "overlapping-group-space";

impl Derivation {
    /// The derivation a relationship identity names, `None` for an identity
    /// outside [`DERIVED_RELATIONSHIP_PREFIX`].
    ///
    /// # Errors
    ///
    /// [`RelationshipSelectionError::InvalidRequest`] for an unknown
    /// derivation or a malformed, unknown or repeated parameter.
    pub fn parse(
        relationship: &SemanticRelationship,
    ) -> Result<Option<Self>, RelationshipSelectionError> {
        let Some(rest) = relationship
            .as_str()
            .strip_prefix(DERIVED_RELATIONSHIP_PREFIX)
        else {
            return Ok(None);
        };
        let mut parts = rest.split(';');
        let name = parts.next().unwrap_or_default();
        let mut parameters: Vec<(&str, f64)> = Vec::new();
        for part in parts {
            let (key, value) = part
                .split_once('=')
                .ok_or(RelationshipSelectionError::InvalidRequest)?;
            let value: f64 = value
                .trim()
                .parse()
                .map_err(|_| RelationshipSelectionError::InvalidRequest)?;
            let key = key.trim();
            if !value.is_finite() || value < 0.0 || parameters.iter().any(|(seen, _)| *seen == key)
            {
                return Err(RelationshipSelectionError::InvalidRequest);
            }
            parameters.push((key, value));
        }
        let allowed: &[&str] = match name {
            CONTAINED_IN_SPACE => &["horizontal", "vertical"],
            ADJACENT_SPACE => &["reach"],
            OVERLAPPING_GROUP_SPACE => &["ratio", "vertical"],
            _ => return Err(RelationshipSelectionError::InvalidRequest),
        };
        if parameters.iter().any(|(key, _)| !allowed.contains(key)) {
            return Err(RelationshipSelectionError::InvalidRequest);
        }
        let get = |key: &str, default: f64| {
            parameters
                .iter()
                .find(|(seen, _)| *seen == key)
                .map_or(default, |(_, value)| *value)
        };
        let derivation = match name {
            CONTAINED_IN_SPACE => Self::ContainedInSpace {
                horizontal_metres: get("horizontal", 0.0),
                vertical_metres: get("vertical", 0.0),
            },
            ADJACENT_SPACE => Self::AdjacentSpace {
                reach_metres: get("reach", 1.0),
            },
            _ => Self::OverlappingGroupSpace {
                minimum_ratio: get("ratio", 0.5),
                vertical_metres: get("vertical", 0.0),
            },
        };
        match derivation {
            Self::AdjacentSpace { reach_metres } if reach_metres <= 0.0 => {
                Err(RelationshipSelectionError::InvalidRequest)
            }
            Self::OverlappingGroupSpace { minimum_ratio, .. }
                if minimum_ratio <= 0.0 || minimum_ratio > 1.0 =>
            {
                Err(RelationshipSelectionError::InvalidRequest)
            }
            derivation => Ok(Some(derivation)),
        }
    }

    /// The derivation's name, the identity without parameters, such as
    /// `axioval:derived.contained-in-space`. Every evidence locator of an
    /// answer starts with it.
    #[must_use]
    pub fn name(&self) -> String {
        let name = match self {
            Self::ContainedInSpace { .. } => CONTAINED_IN_SPACE,
            Self::AdjacentSpace { .. } => ADJACENT_SPACE,
            Self::OverlappingGroupSpace { .. } => OVERLAPPING_GROUP_SPACE,
        };
        format!("{DERIVED_RELATIONSHIP_PREFIX}{name}")
    }
}

/// The canonical identity: the name and every parameter, defaults included.
impl fmt::Display for Derivation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())?;
        match self {
            Self::ContainedInSpace {
                horizontal_metres,
                vertical_metres,
            } => write!(
                f,
                ";horizontal={horizontal_metres};vertical={vertical_metres}"
            ),
            Self::AdjacentSpace { reach_metres } => write!(f, ";reach={reach_metres}"),
            Self::OverlappingGroupSpace {
                minimum_ratio,
                vertical_metres,
            } => write!(f, ";ratio={minimum_ratio};vertical={vertical_metres}"),
        }
    }
}

/// The face of a door, window or opening an `adjacent-space` probe starts
/// from: along the element's through-thickness normal (`+`) or against it
/// (`-`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdjacentSide {
    /// Along the normal, written `+`.
    Positive,
    /// Against the normal, written `-`.
    Negative,
}

impl AdjacentSide {
    /// The sign the evidence writes for this side.
    #[must_use]
    pub fn symbol(self) -> char {
        match self {
            Self::Positive => '+',
            Self::Negative => '-',
        }
    }

    /// The other face.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }

    fn from_symbol(symbol: char) -> Option<Self> {
        match symbol {
            '+' => Some(Self::Positive),
            '-' => Some(Self::Negative),
            _ => None,
        }
    }
}

impl fmt::Display for AdjacentSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// The side an `axioval:derived.adjacent-space` evidence locator records.
///
/// A provider of the adjacency derivation cites every face it probed, in
/// one of two forms after the canonical identity (the derivation's name and
/// its `;key=value` tolerances) and a colon:
///
/// - `{subject}->{space}:side={+|-}…`: the probe from that face of
///   `subject` first entered `space`;
/// - `{subject}:side={+|-}…:outside…`: the probe from that face entered no
///   space within reach.
///
/// Free detail (a normal, a distance) may follow the sign. With `space`,
/// this returns the side of the edge from `subject` to `space`; without, a
/// side `subject` records as outside. `None` when the locator is no such
/// record. Capabilities use it to tell a door with a space on each face from
/// one with two spaces on the same face.
#[must_use]
pub fn adjacent_side(
    locator: &str,
    subject: &ObjectId,
    space: Option<&ObjectId>,
) -> Option<AdjacentSide> {
    let rest = locator
        .strip_prefix(DERIVED_RELATIONSHIP_PREFIX)?
        .strip_prefix(ADJACENT_SPACE)?;
    // Tolerances are `;key=number` pairs and never hold a colon.
    let (tolerances, record) = rest.split_once(':')?;
    if !(tolerances.is_empty() || tolerances.starts_with(';')) {
        return None;
    }
    let head = match space {
        Some(space) => format!("{subject}->{space}:side="),
        None => format!("{subject}:side="),
    };
    let tail = record.strip_prefix(&head)?;
    let side = AdjacentSide::from_symbol(tail.chars().next()?)?;
    if space.is_none() && !tail.contains(":outside") {
        return None;
    }
    Some(side)
}

/// Trusted provider of relationships derived from geometry.
///
/// It answers exactly what a [`RelationshipSelectionService`] answers for a
/// derived identity: the complete selection within the request's universe,
/// with exact evidence, or a refusal. An undecided case (an unmeasured body,
/// a tessellation near a boundary it decides) refuses the whole answer.
pub trait DerivedRelationshipService: Send + Sync {
    /// Exact source snapshots used to construct this service.
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &[]
    }
    /// Derives the selection `request` asks for under `derivation`.
    ///
    /// # Errors
    ///
    /// A refusal when the answer cannot be decided exactly.
    fn derive(
        &self,
        derivation: &Derivation,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError>;
}

/// Cloneable, type-erased derived-relationship service.
#[derive(Clone)]
pub struct DerivedRelationshipServiceHandle(Arc<dyn DerivedRelationshipService>);

impl DerivedRelationshipServiceHandle {
    /// Wraps a trusted derived-relationship service.
    #[must_use]
    pub fn new(service: Arc<dyn DerivedRelationshipService>) -> Self {
        Self(service)
    }

    /// Derives a selection and validates it as the relationship handle does,
    /// and that every evidence locator names the derivation.
    ///
    /// # Errors
    ///
    /// [`RelationshipSelectionError::InvalidRequest`] for an identity that
    /// names no derivation, the provider's refusal, or a response that is
    /// not bound to the request or does not cite its derivation.
    pub fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let derivation = Derivation::parse(request.query().relationship())?
            .ok_or(RelationshipSelectionError::InvalidRequest)?;
        let selection = validate_selection(request, self.0.derive(&derivation, request)?)?;
        let name = derivation.name();
        if selection
            .evidence()
            .iter()
            .any(|item| !item.locator.starts_with(&name))
        {
            return Err(RelationshipSelectionError::InexactEvidence);
        }
        Ok(selection)
    }
}

impl SnapshotBoundService for DerivedRelationshipServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

/// Routes derived identities to the derived service, the rest to the
/// semantic service the source registered.
pub(crate) struct RoutedRelationships {
    pub(crate) semantic: Option<RelationshipSelectionServiceHandle>,
    pub(crate) derived: DerivedRelationshipServiceHandle,
    pub(crate) snapshots: Vec<SourceSnapshot>,
}

impl RelationshipSelectionService for RoutedRelationships {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        if is_derived(request.query()) {
            return self.derived.select(request);
        }
        match &self.semantic {
            Some(semantic) => semantic.select(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no semantic relationship service is registered".into(),
            )),
        }
    }
}

fn is_derived(query: &RelationshipQuery) -> bool {
    query
        .relationship()
        .as_str()
        .starts_with(DERIVED_RELATIONSHIP_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(identity: &str) -> Result<Option<Derivation>, RelationshipSelectionError> {
        Derivation::parse(&SemanticRelationship::try_new(identity).unwrap())
    }

    #[test]
    fn identities_name_the_derivation_and_fix_its_tolerances() {
        assert_eq!(parse("IfcRelAggregates"), Ok(None));
        assert_eq!(
            parse("axioval:derived.contained-in-space"),
            Ok(Some(Derivation::ContainedInSpace {
                horizontal_metres: 0.0,
                vertical_metres: 0.0
            }))
        );
        let nearest = parse("axioval:derived.contained-in-space;vertical=0.5;horizontal=0.25")
            .unwrap()
            .unwrap();
        assert_eq!(
            nearest.to_string(),
            "axioval:derived.contained-in-space;horizontal=0.25;vertical=0.5"
        );
        assert_eq!(
            parse("axioval:derived.adjacent-space")
                .unwrap()
                .unwrap()
                .to_string(),
            "axioval:derived.adjacent-space;reach=1"
        );
        assert_eq!(
            parse("axioval:derived.overlapping-group-space;ratio=0.9")
                .unwrap()
                .unwrap()
                .to_string(),
            "axioval:derived.overlapping-group-space;ratio=0.9;vertical=0"
        );
    }

    #[test]
    fn adjacency_locators_record_the_side_of_each_space_and_each_outside_face() {
        let source = axioval_ir::SourceId::new("ifc-step", "model.ifc").unwrap();
        let door = ObjectId::new(source.clone(), "#76").unwrap();
        let room = ObjectId::new(source.clone(), "#16").unwrap();
        let other = ObjectId::new(source, "#26").unwrap();
        let identity = "axioval:derived.adjacent-space;reach=1";
        // The forms the geometry adapter writes.
        let edge = format!("{identity}:{door}->{room}:side=+(1.000000,0.000000):entered=0.000000");
        let outside = format!("{identity}:{door}:side=-(1.000000,0.000000):outside:reach=1");
        assert_eq!(
            adjacent_side(&edge, &door, Some(&room)),
            Some(AdjacentSide::Positive)
        );
        assert_eq!(adjacent_side(&edge, &door, Some(&other)), None);
        assert_eq!(adjacent_side(&edge, &door, None), None);
        assert_eq!(
            adjacent_side(&outside, &door, None),
            Some(AdjacentSide::Negative)
        );
        assert_eq!(adjacent_side(&outside, &door, Some(&room)), None);
        assert_eq!(adjacent_side(&outside, &room, None), None);
        // Another derivation, or a scan locator, records no side.
        let contained = format!(
            "axioval:derived.contained-in-space;horizontal=0;vertical=0:{door}->{room}:side=+"
        );
        assert_eq!(adjacent_side(&contained, &door, Some(&room)), None);
        let scan = format!("{identity}:derived-from:{door}:2 space(s)");
        assert_eq!(adjacent_side(&scan, &door, None), None);
        assert_eq!(AdjacentSide::Positive.opposite(), AdjacentSide::Negative);
    }

    #[test]
    fn a_malformed_identity_is_an_invalid_request() {
        for identity in [
            "axioval:derived.",
            "axioval:derived.nearest-thing",
            "axioval:derived.adjacent-space;reach",
            "axioval:derived.adjacent-space;reach=0",
            "axioval:derived.adjacent-space;reach=-1",
            "axioval:derived.adjacent-space;reach=inf",
            "axioval:derived.adjacent-space;ratio=0.5",
            "axioval:derived.contained-in-space;vertical=1;vertical=2",
            "axioval:derived.overlapping-group-space;ratio=1.5",
            "axioval:derived.overlapping-group-space;ratio=0",
        ] {
            assert_eq!(
                parse(identity),
                Err(RelationshipSelectionError::InvalidRequest),
                "{identity}"
            );
        }
    }
}
