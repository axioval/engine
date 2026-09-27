//! Source-neutral circulation maps: where a path of a given width can run
//! within one space, and which entrances and components it comes near.
//!
//! A circulation map is a measurement, not a verdict. The free area of a
//! space (its floor less what the obstacles occupy in a band above it) is
//! eroded by half the path's width: every point left is a centre at which
//! a disc as wide as the path fits. A backend cannot compute that erosion
//! exactly, so it reports two bounds:
//!
//! - **pieces**, the connected parts of a region *inside* the exact
//!   erosion. A path provably runs between any two points of one piece;
//! - **possible pieces**, the connected parts of a region *containing* the
//!   exact erosion. A path between two points of different possible pieces
//!   provably does not exist.
//!
//! Each entrance and component the request names is a subject. Its contact
//! lists the pieces that provably come within half the width plus the
//! request's tolerance of its plan footprint, and the possible pieces that
//! may. The skeleton of the pieces (nodes along their middle, joined by
//! edges) tells where paths run, where they meet and where they end. Node
//! positions are approximate; that each node lies in its piece, and the
//! bounds on the free half width around it, are proven.
//!
//! Whether a component that no entrance reaches, or an end without a free
//! area, fails a rule is the capability's decision.

use axioval_ir::{Evidence, ObjectId};

use crate::LengthInterval;
use crate::door_leaves::{SweptDoor, tidy_swept};
use crate::free_space::{ElevationBand, FreeSpaceError};
use crate::services::reviewable_exact_evidence;

/// A circulation map request for one space.
///
/// Entrances are never obstacles, and the scope is neither an entrance, a
/// component nor an obstacle: the constructor removes them, as walkability
/// does. The lists are sorted and deduplicated.
#[derive(Clone, Debug, PartialEq)]
pub struct CirculationRequest {
    scope: ObjectId,
    entrances: Vec<ObjectId>,
    components: Vec<ObjectId>,
    obstacles: Vec<ObjectId>,
    width_metres: f64,
    height_metres: f64,
    tolerance_metres: f64,
    swept: Vec<SweptDoor>,
}

impl CirculationRequest {
    /// A path `width_metres` wide, free `height_metres` above the scope's
    /// floor, in `scope`; a subject is near a piece within half the width
    /// plus `tolerance_metres`.
    ///
    /// # Errors
    ///
    /// [`FreeSpaceError::InvalidClearanceShape`] for a width or height that
    /// is not positive and finite, or a tolerance that is negative or not
    /// finite.
    pub fn try_new(
        scope: ObjectId,
        entrances: Vec<ObjectId>,
        components: Vec<ObjectId>,
        obstacles: Vec<ObjectId>,
        width_metres: f64,
        height_metres: f64,
        tolerance_metres: f64,
    ) -> Result<Self, FreeSpaceError> {
        let positive = |value: f64| value.is_finite() && value > 0.0;
        if !(positive(width_metres)
            && positive(height_metres)
            && tolerance_metres.is_finite()
            && tolerance_metres >= 0.0)
        {
            return Err(FreeSpaceError::InvalidClearanceShape);
        }
        let tidy = |mut list: Vec<ObjectId>| {
            list.retain(|object| object != &scope);
            list.sort();
            list.dedup();
            list
        };
        let entrances = tidy(entrances);
        let components = tidy(components);
        let mut obstacles = tidy(obstacles);
        obstacles.retain(|object| entrances.binary_search(object).is_err());
        Ok(Self {
            scope,
            entrances,
            components,
            obstacles,
            width_metres,
            height_metres,
            tolerance_metres,
            swept: Vec::new(),
        })
    }

    /// Counts the sectors `swept` doors sweep as obstacles (see
    /// [`SweptDoor`]): a piece stays clear of their circumscribed
    /// polygons, a possible piece only of their inscribed ones. An entrance
    /// is walked through, so its own swing is dropped, as the entrance is
    /// from the obstacles.
    ///
    /// # Errors
    ///
    /// [`FreeSpaceError::ConflictingSweptDoors`] when one door is given
    /// twice with different sectors.
    pub fn with_swept_doors(mut self, swept: Vec<SweptDoor>) -> Result<Self, FreeSpaceError> {
        let mut swept = tidy_swept(swept).ok_or(FreeSpaceError::ConflictingSweptDoors)?;
        swept.retain(|door| self.entrances.binary_search(door.door()).is_err());
        self.swept = swept;
        Ok(self)
    }

    /// The doors whose swept sectors are obstacles, sorted by door.
    #[must_use]
    pub fn swept_doors(&self) -> &[SweptDoor] {
        &self.swept
    }

    /// The space the path runs in.
    #[must_use]
    pub fn scope(&self) -> &ObjectId {
        &self.scope
    }

    /// The entrances the path starts from.
    #[must_use]
    pub fn entrances(&self) -> &[ObjectId] {
        &self.entrances
    }

    /// The components the path must come near.
    #[must_use]
    pub fn components(&self) -> &[ObjectId] {
        &self.components
    }

    /// The objects that occupy floor, counted by what they occupy in the
    /// band.
    #[must_use]
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }

    /// The path's width.
    #[must_use]
    pub fn width_metres(&self) -> f64 {
        self.width_metres
    }

    /// The height above the floor the path must be free to.
    #[must_use]
    pub fn height_metres(&self) -> f64 {
        self.height_metres
    }

    /// How much farther than half the width a subject may be from a piece
    /// and still be near it.
    #[must_use]
    pub fn tolerance_metres(&self) -> f64 {
        self.tolerance_metres
    }

    /// The band obstacles count in: the floor up by the height.
    #[must_use]
    pub fn band(&self) -> ElevationBand {
        ElevationBand::try_new(0.0, self.height_metres)
            .unwrap_or_else(|_| unreachable!("the height is positive and finite"))
    }

    /// The subjects a map reports contacts for: the entrances and the
    /// components, sorted and deduplicated.
    #[must_use]
    pub fn subjects(&self) -> Vec<ObjectId> {
        let mut subjects: Vec<ObjectId> = self
            .entrances
            .iter()
            .chain(&self.components)
            .cloned()
            .collect();
        subjects.sort();
        subjects.dedup();
        subjects
    }
}

/// What a skeleton node is, by its number of neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CirculationNodeKind {
    /// Where a path ends: one neighbour.
    End,
    /// Along a path: two neighbours.
    Path,
    /// Where paths meet: three or more.
    Junction,
    /// No neighbour: a piece too small to hold a path.
    Isolated,
}

impl CirculationNodeKind {
    fn admits(self, degree: usize) -> bool {
        match self {
            Self::End => degree == 1,
            Self::Path => degree == 2,
            Self::Junction => degree >= 3,
            Self::Isolated => degree == 0,
        }
    }
}

/// One skeleton node.
#[derive(Clone, Debug, PartialEq)]
pub struct CirculationNode {
    point: [f64; 3],
    kind: CirculationNodeKind,
    piece: usize,
    half_width: LengthInterval,
}

impl CirculationNode {
    /// A node at `point` (on the floor), in `piece`, whose distance to the
    /// nearest wall or obstacle of the free area lies in `half_width`.
    ///
    /// # Errors
    ///
    /// [`FreeSpaceError::Unavailable`] for a coordinate that is not finite.
    pub fn try_new(
        point: [f64; 3],
        kind: CirculationNodeKind,
        piece: usize,
        half_width: LengthInterval,
    ) -> Result<Self, FreeSpaceError> {
        if !point.iter().all(|value| value.is_finite()) {
            return Err(FreeSpaceError::Unavailable(
                "a circulation node's coordinates must be finite".into(),
            ));
        }
        Ok(Self {
            point,
            kind,
            piece,
            half_width,
        })
    }

    /// Where it is, on the floor, in metres. Approximate: the node lies in
    /// its piece, but how close it is to the piece's middle is not proven.
    #[must_use]
    pub fn point(&self) -> [f64; 3] {
        self.point
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> CirculationNodeKind {
        self.kind
    }

    /// The piece it lies in.
    #[must_use]
    pub fn piece(&self) -> usize {
        self.piece
    }

    /// Proven bounds on its distance to the free area's boundary: half the
    /// width of the free area around it.
    #[must_use]
    pub fn half_width(&self) -> LengthInterval {
        self.half_width
    }
}

/// Where one subject meets the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CirculationContact {
    subject: ObjectId,
    reached: Vec<(usize, Option<usize>)>,
    possible: Vec<usize>,
}

impl CirculationContact {
    /// `reached` pairs each piece proven near the subject with the node of
    /// that piece nearest to it, if the piece has one; `possible` lists the
    /// possible pieces that may be near it.
    #[must_use]
    pub fn new(
        subject: ObjectId,
        mut reached: Vec<(usize, Option<usize>)>,
        mut possible: Vec<usize>,
    ) -> Self {
        reached.sort_unstable();
        reached.dedup_by_key(|(piece, _)| *piece);
        possible.sort_unstable();
        possible.dedup();
        Self {
            subject,
            reached,
            possible,
        }
    }

    /// The entrance or component.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// The pieces proven near it, each with its nearest node.
    #[must_use]
    pub fn reached(&self) -> &[(usize, Option<usize>)] {
        &self.reached
    }

    /// Whether `piece` is proven near it.
    #[must_use]
    pub fn reaches(&self, piece: usize) -> bool {
        self.reached.iter().any(|(reached, _)| *reached == piece)
    }

    /// The possible pieces that may be near it. Every other possible piece
    /// is proven farther away.
    #[must_use]
    pub fn possible(&self) -> &[usize] {
        &self.possible
    }
}

/// The circulation map of one space for one path width.
#[derive(Clone, Debug, PartialEq)]
pub struct CirculationMap {
    request: CirculationRequest,
    pieces: usize,
    possible_pieces: usize,
    nodes: Vec<CirculationNode>,
    edges: Vec<(usize, usize)>,
    spacing_metres: f64,
    contacts: Vec<CirculationContact>,
    unmapped: Vec<(usize, String)>,
    evidence: Evidence,
}

impl CirculationMap {
    /// Validates a map for `request`: every node in a piece, every edge
    /// within one piece, kinds matching the number of neighbours, one
    /// contact per subject in order, and exact evidence. `spacing_metres`
    /// is the boundary sample spacing the skeleton was built with.
    /// `unmapped` names the pieces whose skeleton could not be built, with
    /// why; they have no nodes, and where their paths end is unknown.
    ///
    /// # Errors
    ///
    /// [`FreeSpaceError::InexactPlacementEvidence`] for approximate or
    /// blank evidence, [`FreeSpaceError::Unavailable`] for a malformed map.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        request: CirculationRequest,
        pieces: usize,
        possible_pieces: usize,
        nodes: Vec<CirculationNode>,
        mut edges: Vec<(usize, usize)>,
        spacing_metres: f64,
        contacts: Vec<CirculationContact>,
        mut unmapped: Vec<(usize, String)>,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        let malformed = |why: &str| FreeSpaceError::Unavailable(format!("circulation map: {why}"));
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactPlacementEvidence);
        }
        if !(spacing_metres.is_finite() && spacing_metres > 0.0) {
            return Err(malformed("the sample spacing must be positive"));
        }
        if nodes.iter().any(|node| node.piece >= pieces) {
            return Err(malformed("a node lies in no piece"));
        }
        unmapped.sort();
        unmapped.dedup_by_key(|(piece, _)| *piece);
        if unmapped
            .iter()
            .any(|(piece, _)| *piece >= pieces || nodes.iter().any(|node| node.piece == *piece))
        {
            return Err(malformed("an unmapped piece is not there or has nodes"));
        }
        edges.sort_unstable();
        edges.dedup();
        let mut degree = vec![0usize; nodes.len()];
        for &(a, b) in &edges {
            if a >= b || b >= nodes.len() || nodes[a].piece != nodes[b].piece {
                return Err(malformed("an edge joins no two nodes of one piece"));
            }
            degree[a] += 1;
            degree[b] += 1;
        }
        if nodes
            .iter()
            .zip(&degree)
            .any(|(node, &degree)| !node.kind.admits(degree))
        {
            return Err(malformed("a node's kind does not match its neighbours"));
        }
        let subjects = request.subjects();
        if contacts.len() != subjects.len()
            || contacts
                .iter()
                .zip(&subjects)
                .any(|(contact, subject)| &contact.subject != subject)
        {
            return Err(malformed("the contacts are not the request's subjects"));
        }
        for contact in &contacts {
            for &(piece, node) in &contact.reached {
                if piece >= pieces
                    || node.is_some_and(|node| node >= nodes.len() || nodes[node].piece != piece)
                {
                    return Err(malformed(
                        "a contact names a piece or node that is not there",
                    ));
                }
            }
            if contact
                .possible
                .iter()
                .any(|&piece| piece >= possible_pieces)
            {
                return Err(malformed(
                    "a contact names a possible piece that is not there",
                ));
            }
        }
        Ok(Self {
            request,
            pieces,
            possible_pieces,
            nodes,
            edges,
            spacing_metres,
            contacts,
            unmapped,
            evidence,
        })
    }

    /// The request this map answers.
    #[must_use]
    pub fn request(&self) -> &CirculationRequest {
        &self.request
    }

    /// How many pieces there are: connected parts of a region inside the
    /// free area eroded by half the width.
    #[must_use]
    pub fn pieces(&self) -> usize {
        self.pieces
    }

    /// How many possible pieces there are: connected parts of a region
    /// containing that erosion.
    #[must_use]
    pub fn possible_pieces(&self) -> usize {
        self.possible_pieces
    }

    /// The skeleton nodes.
    #[must_use]
    pub fn nodes(&self) -> &[CirculationNode] {
        &self.nodes
    }

    /// The skeleton edges, as node index pairs, lower first, sorted.
    #[must_use]
    pub fn edges(&self) -> &[(usize, usize)] {
        &self.edges
    }

    /// The neighbours of `node`, ascending.
    #[must_use]
    pub fn neighbours(&self, node: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .edges
            .iter()
            .filter_map(|&(a, b)| match (a == node, b == node) {
                (true, _) => Some(b),
                (_, true) => Some(a),
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// The boundary sample spacing the skeleton was built with: how far
    /// node positions may stray is of this order, but not proven.
    #[must_use]
    pub fn spacing_metres(&self) -> f64 {
        self.spacing_metres
    }

    /// One contact per subject, in subject order.
    #[must_use]
    pub fn contacts(&self) -> &[CirculationContact] {
        &self.contacts
    }

    /// The contact of `subject`, if the request names it.
    #[must_use]
    pub fn contact(&self, subject: &ObjectId) -> Option<&CirculationContact> {
        self.contacts
            .iter()
            .find(|contact| &contact.subject == subject)
    }

    /// The pieces without a skeleton, with why: where their paths end is
    /// unknown.
    #[must_use]
    pub fn unmapped(&self) -> &[(usize, String)] {
        &self.unmapped
    }

    /// Why `piece` has no skeleton, if it has none.
    #[must_use]
    pub fn unmapped_reason(&self, piece: usize) -> Option<&str> {
        self.unmapped
            .iter()
            .find(|(unmapped, _)| *unmapped == piece)
            .map(|(_, why)| why.as_str())
    }

    /// Provenance.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}
