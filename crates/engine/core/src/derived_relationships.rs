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
//! Edges always run from a subject to a space (a level, for `spans-level`),
//! so `forward` from an element reaches its spaces and `backward` from a
//! space reaches its subjects. `adjacent-across` runs from a separating
//! element (a wall or a slab) to the spaces on its faces, and the session
//! answers it from the space boundaries a source states wherever an element
//! has any. `intersects` is symmetric: it runs both ways
//! between two bodies sharing volume, so either direction reaches the same
//! objects.

use std::fmt;
use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::relationships::{
    CompleteRelationshipSelection, RelationshipKind, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    SemanticRelationship, TraversalDirection, validate_selection,
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
    /// `axioval:derived.spans-level`: a space to every level (storey)
    /// whose height band its vertical extent reaches at least `overlap`
    /// metres into (default 1), or at least half its own height, so a space
    /// counts in its own storey and a two-storey atrium in both. A level's
    /// band runs from its elevation up to the next level's.
    SpansLevel {
        /// Smallest vertical overlap with a level's band, in metres.
        overlap_metres: f64,
    },
    /// `axioval:derived.intersects`: an element to every object of the
    /// request's universe whose body shares volume with its own, such as a
    /// duct to the beam or wall it passes through with no void modelled.
    /// It takes no parameters and runs both ways.
    ///
    /// Bodies that only touch share no volume. A pair the geometry cannot
    /// decide (an unmeasured body, a tessellation within its chord
    /// deviation of the other body, two open surfaces) refuses the answer.
    Intersects,
    /// `axioval:derived.adjacent-across`: a separating element, a wall or a
    /// slab, to the spaces lying within `tolerance` metres of either of its
    /// faces (default 0.05) along at least `overlap` metres of it (default
    /// 0.3, positive). A wall's faces are the two long sides of its plan;
    /// along it is the wall's length, measured in the height the wall and
    /// the space share. A slab's faces are its top and its bottom, and a
    /// space beside one must share at least `overlap`² square metres of
    /// footprint with it.
    ///
    /// Each edge records the face its space lies on (see [`across_side`]),
    /// so two spaces adjacent across the element lie on opposite faces. A
    /// space within or beyond the tolerance only as far as the geometry's
    /// rounding or chord deviation can tell refuses the answer.
    AdjacentAcross {
        /// Largest gap between a face and a space's boundary, in metres.
        tolerance_metres: f64,
        /// Least length (for a slab, square root of the least area) the
        /// space must share with the face, in metres.
        overlap_metres: f64,
    },
}

const CONTAINED_IN_SPACE: &str = "contained-in-space";
const ADJACENT_SPACE: &str = "adjacent-space";
const OVERLAPPING_GROUP_SPACE: &str = "overlapping-group-space";
const SPANS_LEVEL: &str = "spans-level";
const INTERSECTS: &str = "intersects";
const ADJACENT_ACROSS: &str = "adjacent-across";

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
            SPANS_LEVEL => &["overlap"],
            INTERSECTS => &[],
            ADJACENT_ACROSS => &["tolerance", "overlap"],
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
            SPANS_LEVEL => Self::SpansLevel {
                overlap_metres: get("overlap", 1.0),
            },
            INTERSECTS => Self::Intersects,
            ADJACENT_ACROSS => Self::AdjacentAcross {
                tolerance_metres: get("tolerance", 0.05),
                overlap_metres: get("overlap", 0.3),
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
            Self::SpansLevel { overlap_metres } | Self::AdjacentAcross { overlap_metres, .. }
                if overlap_metres <= 0.0 =>
            {
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
            Self::SpansLevel { .. } => SPANS_LEVEL,
            Self::Intersects => INTERSECTS,
            Self::AdjacentAcross { .. } => ADJACENT_ACROSS,
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
            Self::SpansLevel { overlap_metres } => write!(f, ";overlap={overlap_metres}"),
            Self::Intersects => Ok(()),
            Self::AdjacentAcross {
                tolerance_metres,
                overlap_metres,
            } => write!(f, ";tolerance={tolerance_metres};overlap={overlap_metres}"),
        }
    }
}

const SAME_LEVEL: &str = "same-level";

/// `axioval:derived.same-level`: levels of different sources that stand for
/// one federated level, such as the architecture model's and the MEP
/// model's storeys at one elevation.
///
/// Unlike a [`Derivation`] it needs no geometry: a capability matches two
/// containers it has already climbed to by what their sources state, their
/// elevation or their name, so it is never routed to a geometry provider.
/// Parameters follow the name as for a derivation: `by` is `elevation` (the
/// default) or `name`, and `tolerance` (metres, default 0, elevation only)
/// is the largest elevation difference of one level. A container is always
/// on its own level; two containers of one source never share one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LevelMatch {
    /// Elevations at most `tolerance_metres` apart.
    Elevation {
        /// Largest elevation difference of one level, in metres.
        tolerance_metres: f64,
    },
    /// Names that are equal as stated.
    Name,
}

/// What a level states that [`LevelMatch`] compares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelFacts {
    /// The level's elevation in metres, when stated.
    pub elevation_metres: Option<f64>,
    /// The level's name, when stated.
    pub name: Option<String>,
}

impl LevelMatch {
    /// The level match a relationship identity names, `None` for any other
    /// identity.
    ///
    /// # Errors
    ///
    /// [`RelationshipSelectionError::InvalidRequest`] for a malformed,
    /// unknown or repeated parameter, a negative or non-finite tolerance, or
    /// a tolerance with `by=name`.
    pub fn parse(relationship: &str) -> Result<Option<Self>, RelationshipSelectionError> {
        let Some(rest) = relationship.strip_prefix(DERIVED_RELATIONSHIP_PREFIX) else {
            return Ok(None);
        };
        let mut parts = rest.split(';');
        if parts.next() != Some(SAME_LEVEL) {
            return Ok(None);
        }
        let (mut by, mut tolerance) = (None, None);
        for part in parts {
            let (key, value) = part
                .split_once('=')
                .ok_or(RelationshipSelectionError::InvalidRequest)?;
            match (key.trim(), value.trim()) {
                ("by", value) if by.is_none() => by = Some(value.to_owned()),
                ("tolerance", value) if tolerance.is_none() => {
                    let metres: f64 = value
                        .parse()
                        .map_err(|_| RelationshipSelectionError::InvalidRequest)?;
                    if !metres.is_finite() || metres < 0.0 {
                        return Err(RelationshipSelectionError::InvalidRequest);
                    }
                    tolerance = Some(metres);
                }
                _ => return Err(RelationshipSelectionError::InvalidRequest),
            }
        }
        match (by.as_deref(), tolerance) {
            (None | Some("elevation"), tolerance) => Ok(Some(Self::Elevation {
                tolerance_metres: tolerance.unwrap_or(0.0),
            })),
            (Some("name"), None) => Ok(Some(Self::Name)),
            _ => Err(RelationshipSelectionError::InvalidRequest),
        }
    }

    /// Whether two levels of different sources are one level: `None` when
    /// either does not state what the match compares.
    ///
    /// Elevations compare with the rounding of one unit conversion allowed
    /// beyond the tolerance, so levels stated as `3000 mm` and `3 m` match
    /// with none.
    #[must_use]
    pub fn same(&self, left: &LevelFacts, right: &LevelFacts) -> Option<bool> {
        match self {
            Self::Elevation { tolerance_metres } => {
                let (left, right) = (left.elevation_metres?, right.elevation_metres?);
                if !left.is_finite() || !right.is_finite() {
                    return None;
                }
                let rounding = 1e-9 * left.abs().max(right.abs()).max(1.0);
                Some((left - right).abs() <= tolerance_metres + rounding)
            }
            Self::Name => Some(left.name.as_ref()? == right.name.as_ref()?),
        }
    }
}

/// The canonical identity: the name and every parameter, defaults included.
impl fmt::Display for LevelMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{DERIVED_RELATIONSHIP_PREFIX}{SAME_LEVEL}")?;
        match self {
            Self::Elevation { tolerance_metres } => {
                write!(f, ";by=elevation;tolerance={tolerance_metres}")
            }
            Self::Name => write!(f, ";by=name"),
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
    let tail = side_record(locator, ADJACENT_SPACE, subject, space)?;
    let side = AdjacentSide::from_symbol(tail.chars().next()?)?;
    if space.is_none() && !tail.contains(":outside") {
        return None;
    }
    Some(side)
}

/// The face an `axioval:derived.adjacent-across` edge evidence locator
/// records for `space` beside `element`.
///
/// A provider of the derivation cites every edge as
/// `{element}->{space}:side={+|-}…` after the canonical identity and a
/// colon, and every face beside which no space lies as
/// `{element}:side={+|-}…:none…`. A wall's `+` face lies along the normal
/// the record writes after the sign; a slab's `+` face is its top. A space
/// may lie on both faces, in two records. `None` when the locator is no
/// such edge record, such as the router's record of an element answered
/// from its stated boundaries ([`across_stated`]), which records no face.
#[must_use]
pub fn across_side(locator: &str, element: &ObjectId, space: &ObjectId) -> Option<AdjacentSide> {
    let tail = side_record(locator, ADJACENT_ACROSS, element, Some(space))?;
    AdjacentSide::from_symbol(tail.chars().next()?)
}

/// Whether an `axioval:derived.adjacent-across` evidence locator records
/// that `element` was answered from the space boundaries its source
/// states, not from geometry: `{identity}:stated:{element}`.
#[must_use]
pub fn across_stated(locator: &str, element: &ObjectId) -> bool {
    let Some(rest) = locator
        .strip_prefix(DERIVED_RELATIONSHIP_PREFIX)
        .and_then(|rest| rest.strip_prefix(ADJACENT_ACROSS))
    else {
        return false;
    };
    rest.split_once(':').is_some_and(|(tolerances, record)| {
        (tolerances.is_empty() || tolerances.starts_with(';'))
            && record
                .strip_prefix(STATED)
                .is_some_and(|tail| tail == element.to_string())
    })
}

const STATED: &str = "stated:";

/// What follows `side=` in a side record of the derivation `name` about
/// `subject` (and `space`, for an edge).
fn side_record<'l>(
    locator: &'l str,
    name: &str,
    subject: &ObjectId,
    space: Option<&ObjectId>,
) -> Option<&'l str> {
    let rest = locator
        .strip_prefix(DERIVED_RELATIONSHIP_PREFIX)?
        .strip_prefix(name)?;
    // Tolerances are `;key=number` pairs and never hold a colon.
    let (tolerances, record) = rest.split_once(':')?;
    if !(tolerances.is_empty() || tolerances.starts_with(';')) {
        return None;
    }
    let head = match space {
        Some(space) => format!("{subject}->{space}:side="),
        None => format!("{subject}:side="),
    };
    record.strip_prefix(&head)
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
    /// Every object of the session's project: the universe a stated
    /// boundary is looked for in, whatever universe a request names.
    pub(crate) objects: Arc<[ObjectId]>,
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
            if let (Some(semantic), Some(derivation @ Derivation::AdjacentAcross { .. })) = (
                &self.semantic,
                Derivation::parse(request.query().relationship())?,
            ) {
                return StatedFirst {
                    semantic,
                    derived: &self.derived,
                    objects: &self.objects,
                    request,
                    identity: derivation.to_string(),
                }
                .select();
            }
            return self.derived.select(request);
        }
        match &self.semantic {
            Some(semantic) => semantic.select(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no semantic relationship service is registered".into(),
            )),
        }
    }

    fn edges(
        &self,
        request: &crate::RelationshipEdgesRequest,
    ) -> Result<crate::CompleteRelationshipEdges, RelationshipSelectionError> {
        if request
            .relationship()
            .as_str()
            .starts_with(DERIVED_RELATIONSHIP_PREFIX)
        {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "derived relationships are answered per object; the edges of `{}` are not listed",
                request.relationship().as_str()
            )));
        }
        match &self.semantic {
            Some(semantic) => semantic.edges(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no semantic relationship service is registered".into(),
            )),
        }
    }
}

/// `adjacent-across` answered from stated space boundaries first.
///
/// An element whose source states a space boundary to it (the
/// [`RelationshipKind::SpaceBoundary`] kind, from a space to its bounding
/// elements) within the request's universe is answered from those spaces
/// alone; only an element stating none is answered from geometry. Walking
/// backward from a space reaches the elements its stated boundaries name
/// and the elements geometry places beside it that state no boundary of
/// their own. A refused stated answer refuses the whole answer: the stated
/// boundary it could hold would win.
struct StatedFirst<'r> {
    semantic: &'r RelationshipSelectionServiceHandle,
    derived: &'r DerivedRelationshipServiceHandle,
    objects: &'r [ObjectId],
    request: &'r RelationshipSelectionRequest,
    identity: String,
}

impl StatedFirst<'_> {
    fn select(&self) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related {
            direction,
            follow_chain,
            ..
        } = self.request.query()
        else {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "`{}` relates elements to the spaces beside them; it forms no groups",
                self.identity
            )));
        };
        let anchor = self.request.anchor();
        let mut evidence = vec![Evidence::exact(
            anchor.source.clone(),
            format!("{}:stated-first:{anchor}", self.identity),
        )];
        let mut reached = std::collections::BTreeSet::new();
        let mut seen = std::collections::BTreeSet::from([anchor.clone()]);
        let mut frontier = vec![anchor.clone()];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for current in &frontier {
                let mut step = Vec::new();
                if matches!(
                    direction,
                    TraversalDirection::Forward | TraversalDirection::Either
                ) {
                    step.extend(self.forward(current, &mut evidence)?);
                }
                if matches!(
                    direction,
                    TraversalDirection::Backward | TraversalDirection::Either
                ) {
                    step.extend(self.backward(current, &mut evidence)?);
                }
                for object in step {
                    reached.insert(object.clone());
                    if seen.insert(object.clone()) {
                        next.push(object);
                    }
                }
            }
            if !follow_chain {
                break;
            }
            frontier = next;
        }
        reached.remove(anchor);
        reached.retain(|object| {
            self.request
                .candidate_universe()
                .binary_search(object)
                .is_ok()
        });
        let mut unique = std::collections::BTreeSet::new();
        evidence.retain(|item| unique.insert((item.source.clone(), item.locator.clone())));
        CompleteRelationshipSelection::try_new(
            self.request.clone(),
            reached.into_iter().collect(),
            evidence,
        )
    }

    /// One step of a stated or derived relationship from `from`.
    fn ask(
        &self,
        from: &ObjectId,
        relationship: &SemanticRelationship,
        direction: TraversalDirection,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        // Every step looks through the whole project: an element's stated
        // boundaries win wherever they lead, and a chain passes through
        // objects outside the request's universe.
        let mut universe = self.objects.to_vec();
        universe.extend(self.request.candidate_universe().iter().cloned());
        universe.sort();
        universe.dedup();
        let request = RelationshipSelectionRequest::try_new(
            from.clone(),
            universe,
            RelationshipQuery::Related {
                relationship: relationship.clone(),
                direction,
                follow_chain: false,
            },
        )?
        .with_absent_ends(self.request.absent_ends());
        let answer = if relationship
            .as_str()
            .starts_with(DERIVED_RELATIONSHIP_PREFIX)
        {
            self.derived.select(&request)?
        } else {
            self.semantic.select(&request)?
        };
        evidence.extend(answer.evidence().iter().cloned());
        Ok(answer.candidates().to_vec())
    }

    /// The spaces whose stated boundaries name `element`.
    fn stated(
        &self,
        element: &ObjectId,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        self.ask(
            element,
            &RelationshipKind::SpaceBoundary.relationship(),
            TraversalDirection::Backward,
            evidence,
        )
    }

    fn forward(
        &self,
        element: &ObjectId,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        let stated = self.stated(element, evidence)?;
        if !stated.is_empty() {
            evidence.push(Evidence::exact(
                element.source.clone(),
                format!("{}:{STATED}{element}", self.identity),
            ));
            return Ok(stated);
        }
        self.ask(
            element,
            self.request.query().relationship(),
            TraversalDirection::Forward,
            evidence,
        )
    }

    fn backward(
        &self,
        space: &ObjectId,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        let mut elements = self.ask(
            space,
            &RelationshipKind::SpaceBoundary.relationship(),
            TraversalDirection::Forward,
            evidence,
        )?;
        for element in self.ask(
            space,
            self.request.query().relationship(),
            TraversalDirection::Backward,
            evidence,
        )? {
            // An element stating boundaries is answered by them alone, and
            // they do not name this space, or it would be listed already.
            if !elements.contains(&element) && self.stated(&element, evidence)?.is_empty() {
                elements.push(element);
            }
        }
        Ok(elements)
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
        assert_eq!(
            parse("axioval:derived.spans-level"),
            Ok(Some(Derivation::SpansLevel {
                overlap_metres: 1.0
            }))
        );
        assert_eq!(
            parse("axioval:derived.spans-level;overlap=0.5")
                .unwrap()
                .unwrap()
                .to_string(),
            "axioval:derived.spans-level;overlap=0.5"
        );
        assert_eq!(
            parse("axioval:derived.spans-level;overlap=0"),
            Err(RelationshipSelectionError::InvalidRequest)
        );
        assert_eq!(
            parse("axioval:derived.spans-level;reach=1"),
            Err(RelationshipSelectionError::InvalidRequest)
        );
        let intersects = parse("axioval:derived.intersects").unwrap().unwrap();
        assert_eq!(intersects, Derivation::Intersects);
        assert_eq!(intersects.to_string(), "axioval:derived.intersects");
        assert_eq!(intersects.name(), "axioval:derived.intersects");
        assert_eq!(
            parse("axioval:derived.intersects;depth=0.1"),
            Err(RelationshipSelectionError::InvalidRequest)
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

    #[test]
    fn adjacency_across_fixes_its_tolerances_and_records_faces() {
        let across = parse("axioval:derived.adjacent-across;overlap=0.5")
            .unwrap()
            .unwrap();
        assert_eq!(
            across,
            Derivation::AdjacentAcross {
                tolerance_metres: 0.05,
                overlap_metres: 0.5
            }
        );
        assert_eq!(
            across.to_string(),
            "axioval:derived.adjacent-across;tolerance=0.05;overlap=0.5"
        );
        for invalid in [
            "axioval:derived.adjacent-across;overlap=0",
            "axioval:derived.adjacent-across;reach=1",
            "axioval:derived.adjacent-across;tolerance=-0.1",
        ] {
            assert_eq!(
                parse(invalid),
                Err(RelationshipSelectionError::InvalidRequest),
                "{invalid}"
            );
        }
        let source = axioval_ir::SourceId::new("ifc-step", "model.ifc").unwrap();
        let wall = ObjectId::new(source.clone(), "#40").unwrap();
        let room = ObjectId::new(source, "#16").unwrap();
        let identity = across.to_string();
        let edge =
            format!("{identity}:{wall}->{room}:side=-(1.000000,0.000000,0.000000):overlap=4");
        assert_eq!(
            across_side(&edge, &wall, &room),
            Some(AdjacentSide::Negative)
        );
        assert_eq!(across_side(&edge, &room, &wall), None);
        let opening = format!("axioval:derived.adjacent-space;reach=1:{wall}->{room}:side=+");
        assert_eq!(across_side(&opening, &wall, &room), None);
        assert!(across_stated(&format!("{identity}:stated:{wall}"), &wall));
        assert!(!across_stated(&format!("{identity}:stated:{wall}"), &room));
        assert!(!across_stated(&edge, &wall));
    }
}
