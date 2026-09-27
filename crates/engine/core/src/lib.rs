//! Trusted capability compilation and deterministic runtime.
#![forbid(unsafe_code)]
#![allow(missing_docs, clippy::missing_errors_doc)]

use std::{collections::BTreeMap, sync::Arc};

pub use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract as schema;
use axioval_ir::{
    Finding, NotEvaluated, ObjectId, Project, Report, ReportTable, RuleId, Scope, SourceId,
};
use thiserror::Error;

mod session;

/// Errors while compiling untrusted declarations into a trusted execution plan.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    /// A package declares a schema version this compiler does not implement.
    #[error(
        "unsupported schema version `{version}` for {package_kind} `{package_id}`; supported: {supported}"
    )]
    UnsupportedSchemaVersion {
        package_kind: &'static str,
        package_id: String,
        version: String,
        supported: &'static str,
    },
    /// Multiple supplied definition packages declared the same package identity.
    #[error("duplicate definition package `{0}`")]
    DuplicateDefinitionPackage(String),
    /// A capability was not registered by the host.
    #[error("unknown capability `{0}`")]
    UnknownCapability(String),
    /// Two trusted implementations claimed an ID.
    #[error("duplicate capability `{0}`")]
    DuplicateCapability(String),
    /// A package supplied a non-declared parameter.
    #[error("capability `{capability}` does not declare parameter `{parameter}`")]
    UnknownParameter {
        capability: String,
        parameter: String,
    },
    /// A required parameter was absent.
    #[error("capability `{capability}` requires parameter `{parameter}`")]
    MissingParameter {
        capability: String,
        parameter: String,
    },
    /// A binding type did not conform to its descriptor.
    #[error("capability `{capability}` parameter `{parameter}` has invalid type")]
    InvalidParameterType {
        capability: String,
        parameter: String,
    },
    /// A row of a table-valued binding does not fit the declared columns.
    #[error("capability `{capability}` parameter `{parameter}` row {row}: {detail}")]
    InvalidTableRow {
        capability: String,
        parameter: String,
        /// Zero-based row index.
        row: usize,
        detail: String,
    },
    /// A rule binds a parameter more than once.
    #[error("rule has duplicate parameter binding `{0}`")]
    DuplicateBinding(String),
    /// A rule ID violates the engine identity contract.
    #[error("invalid rule id `{0}`")]
    InvalidRuleId(String),
    /// A rule references no loaded definition.
    #[error("unknown rule definition `{0}`")]
    UnknownDefinition(String),
    /// A ruleset references a definition package that was not supplied.
    #[error("missing definition package `{0}`")]
    MissingDefinitionPackage(String),
    /// A trusted capability descriptor conflicts with its portable definition.
    #[error("definition `{definition}` conflicts with capability `{capability}`: {detail}")]
    CapabilityContract {
        definition: String,
        capability: String,
        detail: String,
    },
    /// Rule IDs must be unique throughout the recursive folder tree.
    #[error("duplicate rule id `{0}`")]
    DuplicateRule(String),
    /// Two rulesets compiled together share a package ID.
    #[error("duplicate ruleset package `{0}`")]
    DuplicateRuleSet(String),
    /// No ruleset was given to compile.
    #[error("no ruleset to compile")]
    NoRuleSet,
    /// Two loaded definition packages declare the same concept identity.
    #[error("duplicate concept `{0}` across definition packages")]
    DuplicateConcept(String),
    /// A rule names a concept no loaded definition package declares.
    #[error("rule `{rule}` references unknown {kind} concept `{concept}`")]
    UnknownConcept {
        rule: String,
        kind: String,
        concept: String,
    },
    /// One rule reported two tables of one name.
    #[error("rule `{rule}` reported table `{table}` twice")]
    DuplicateReportTable { rule: String, table: String },
    /// A rule refines its outcomes in a way the package or the capability
    /// cannot support, such as severity bands on a capability that reports
    /// no deviation.
    #[error("rule `{rule}`: {detail}")]
    InvalidRefinement { rule: String, detail: String },
}

pub use schema::ColumnKind;

/// One trusted column of a table parameter.
///
/// A definition's columns must match the descriptor's by ID, kind and
/// requirement, in any order; their names and descriptions are presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableColumn {
    pub id: &'static str,
    pub kind: ColumnKind,
    pub required: bool,
}
impl TableColumn {
    /// A column every row must fill.
    #[must_use]
    pub const fn required(id: &'static str, kind: ColumnKind) -> Self {
        Self {
            id,
            kind,
            required: true,
        }
    }
    /// A column a row may leave empty.
    #[must_use]
    pub const fn optional(id: &'static str, kind: ColumnKind) -> Self {
        Self {
            id,
            kind,
            required: false,
        }
    }
}

/// Supported declarative parameter types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterType {
    Boolean,
    Integer,
    Number,
    String,
    Quantity,
    Enum,
    /// An ISO 8601 calendar date, validated when the package is read.
    Date,
    /// An ISO 8601 date-time with a UTC offset, validated when the package
    /// is read.
    DateTime,
    Reference,
    ObjectTypeReference,
    PropertyReference,
    Selector,
    StringList,
    ReferenceList,
    /// Rows of typed cells in the given columns.
    Table(&'static [TableColumn]),
}
impl ParameterType {
    /// The kind's spelling in a definition package.
    #[must_use]
    pub fn package_kind(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::String => "string",
            Self::Quantity => "quantity",
            Self::Enum => "enum",
            Self::Date => "date",
            Self::DateTime => "dateTime",
            Self::Reference => "reference",
            Self::ObjectTypeReference => "objectTypeReference",
            Self::PropertyReference => "propertyReference",
            Self::Selector => "selector",
            Self::StringList => "stringList",
            Self::ReferenceList => "referenceList",
            Self::Table(_) => "table",
        }
    }
    fn accepts(self, value: &schema::ParameterValue) -> bool {
        matches!(
            (self, value),
            (Self::Boolean, schema::ParameterValue::Boolean { .. })
                | (Self::Integer, schema::ParameterValue::Integer { .. })
                | (Self::Number, schema::ParameterValue::Number { .. })
                | (Self::String, schema::ParameterValue::String { .. })
                | (Self::Quantity, schema::ParameterValue::Quantity { .. })
                | (Self::Enum, schema::ParameterValue::Enum { .. })
                | (Self::Date, schema::ParameterValue::Date { .. })
                | (Self::DateTime, schema::ParameterValue::DateTime { .. })
                | (Self::Reference, schema::ParameterValue::Reference { .. })
                | (
                    Self::ObjectTypeReference,
                    schema::ParameterValue::ObjectTypeReference { .. }
                )
                | (
                    Self::PropertyReference,
                    schema::ParameterValue::PropertyReference { .. }
                )
                | (Self::Selector, schema::ParameterValue::Selector { .. })
                | (Self::StringList, schema::ParameterValue::StringList { .. })
                | (
                    Self::ReferenceList,
                    schema::ParameterValue::ReferenceList { .. }
                )
                | (Self::Table(_), schema::ParameterValue::Table { .. })
        )
    }
}
/// Trusted capability parameter descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterDescriptor {
    pub name: String,
    pub parameter_type: ParameterType,
    pub required: bool,
}
impl ParameterDescriptor {
    /// Required parameter descriptor.
    pub fn required(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: true,
        }
    }
    /// Optional parameter descriptor.
    pub fn optional(name: impl Into<String>, parameter_type: ParameterType) -> Self {
        Self {
            name: name.into(),
            parameter_type,
            required: false,
        }
    }
}

/// One validated portable rule bound to trusted executable capability code.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledRule {
    /// Package-local stable rule ID.
    pub id: RuleId,
    /// Registered capability ID.
    pub capability: String,
    /// Rule severity.
    pub severity: schema::Severity,
    /// Source-neutral applicability selector.
    pub selector: schema::Selector,
    /// Strictly validated parameter bindings.
    pub parameters: BTreeMap<String, schema::ParameterValue>,
}

/// Source-neutral data and typed host services visible during one rule evaluation.
pub struct RuleContext<'a> {
    /// Immutable composed project view.
    pub project: &'a Project,
    /// Adapter-provided semantic and computational capabilities.
    pub services: &'a ServiceRegistry,
}

/// Fail-closed output from one trusted capability evaluation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapabilityEvaluation {
    findings: Vec<Finding>,
    not_evaluated: Vec<CapabilityNotEvaluated>,
    tables: Vec<ReportTable>,
    /// The deviation of each graded finding, by its index in `findings`.
    graded: Vec<(usize, Deviation)>,
}
/// A not-evaluated outcome before the runtime binds its compiled rule ID.
#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityNotEvaluated {
    scope: Scope,
    reason: NotEvaluatedReason,
    message: String,
}
impl CapabilityNotEvaluated {
    /// The object that could not be evaluated, if the outcome is about one.
    #[must_use]
    pub fn object_id(&self) -> Option<&ObjectId> {
        self.scope.object()
    }
    /// What could not be evaluated: an object, a source, or the whole rule.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    #[must_use]
    pub fn reason(&self) -> &NotEvaluatedReason {
        &self.reason
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl CapabilityEvaluation {
    /// Conclusive findings emitted by this capability.
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Explicit fail-closed outcomes emitted by this capability.
    #[must_use]
    pub fn not_evaluated_outcomes(&self) -> &[CapabilityNotEvaluated] {
        &self.not_evaluated
    }
    /// Tables of measured values emitted by this capability.
    #[must_use]
    pub fn tables(&self) -> &[ReportTable] {
        &self.tables
    }
    /// Creates a conclusive evaluation from zero or more findings.
    #[must_use]
    pub fn evaluated(findings: Vec<Finding>) -> Self {
        Self {
            findings,
            ..Self::default()
        }
    }
    /// Adds a table of measured values, reported beside the findings.
    ///
    /// A table without rows is dropped: it measured nothing, and the
    /// not-evaluated outcomes already say why. Tables are informative only;
    /// they never stand in for a finding or a not-evaluated outcome.
    pub fn push_table(&mut self, table: ReportTable) {
        if !table.is_empty() {
            self.tables.push(table);
        }
    }
    /// Creates a rule-level not-evaluated outcome.
    #[must_use]
    pub fn not_evaluated(reason: NotEvaluatedReason, message: impl Into<String>) -> Self {
        let mut outcome = Self::default();
        outcome.push_not_evaluated(reason, message);
        outcome
    }
    /// Adds a conclusive finding, about an object, a source or the project
    /// as its [`Scope`] says.
    pub fn push_finding(&mut self, finding: Finding) {
        self.findings.push(finding);
    }
    /// Adds a finding of a value missing its bound by `deviation`, so a rule
    /// declaring severity bands grades it. The finding keeps its own
    /// severity otherwise; the deviation is never reported.
    pub fn push_graded_finding(&mut self, finding: Finding, deviation: Deviation) {
        self.graded.push((self.findings.len(), deviation));
        self.findings.push(finding);
    }
    /// Adds a finding, graded when `deviation` is known.
    pub fn push_finding_deviating(&mut self, finding: Finding, deviation: Option<Deviation>) {
        match deviation {
            Some(deviation) => self.push_graded_finding(finding, deviation),
            None => self.push_finding(finding),
        }
    }
    /// The deviation a graded finding was pushed with, by its index in
    /// [`Self::findings`]; `None` for an ungraded one.
    #[must_use]
    pub fn deviation(&self, finding: usize) -> Option<Deviation> {
        self.graded
            .iter()
            .find(|(index, _)| *index == finding)
            .map(|(_, deviation)| *deviation)
    }
    /// Grades every graded finding's severity by `bands`, the finding's own
    /// severity standing beyond the last band.
    fn grade(&mut self, bands: &[schema::SeverityBand]) {
        for (index, deviation) in std::mem::take(&mut self.graded) {
            let finding = &mut self.findings[index];
            let (severity, mixed) = refinement::grade(bands, &finding.severity, deviation);
            if mixed {
                use std::fmt::Write as _;
                let _ = write!(
                    finding.message,
                    "; deviation between {} and {}, graded {} by its most severe band",
                    percent(deviation.lower()),
                    percent(deviation.upper()),
                    refinement::label(&severity)
                );
            }
            finding.severity = severity;
        }
    }
    /// Adds a rule-level not-evaluated outcome.
    pub fn push_not_evaluated(&mut self, reason: NotEvaluatedReason, message: impl Into<String>) {
        self.push_unavailable(Scope::Project, reason, message);
    }
    /// Adds a not-evaluated outcome about one source as a whole, such as a
    /// count over that source that undecided objects could still change.
    pub fn push_source_not_evaluated(
        &mut self,
        source: SourceId,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.push_unavailable(Scope::Source(source), reason, message);
    }
    /// Adds an object-specific not-evaluated outcome.
    pub fn push_object_not_evaluated(
        &mut self,
        object_id: ObjectId,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.push_unavailable(Scope::Object(object_id), reason, message);
    }
    fn push_unavailable(
        &mut self,
        scope: Scope,
        reason: NotEvaluatedReason,
        message: impl Into<String>,
    ) {
        self.not_evaluated.push(CapabilityNotEvaluated {
            scope,
            reason,
            message: message.into(),
        });
    }
}

/// A relative deviation as a reviewer reads it, `12.5 %`.
fn percent(value: f64) -> String {
    if value.is_finite() {
        format!("{} %", (value * 1e4).round() / 1e2)
    } else {
        "unbounded".to_owned()
    }
}

/// Trusted code selected by a package capability ID; packages never supply executable code.
pub trait RuleCapability: Send + Sync {
    /// Stable trusted capability ID.
    fn id(&self) -> &'static str;
    /// Strict accepted parameters.
    fn parameters(&self) -> Vec<ParameterDescriptor>;
    /// Whether findings report how far a value misses its bound
    /// ([`CapabilityEvaluation::push_graded_finding`]), so a rule may grade
    /// them with severity bands. A rule declaring bands on a capability that
    /// answers `false` fails compilation.
    fn grades_deviation(&self) -> bool {
        false
    }
    /// Evaluates an already-validated rule request.
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation;
}

/// Host-controlled registry of trusted capabilities.
#[derive(Clone, Default)]
pub struct CapabilityRegistry {
    capabilities: BTreeMap<String, Arc<dyn RuleCapability>>,
}
impl CapabilityRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers a capability; duplicate IDs are rejected.
    pub fn register<C: RuleCapability + 'static>(
        mut self,
        capability: C,
    ) -> Result<Self, EngineError> {
        let id = capability.id().to_owned();
        if self
            .capabilities
            .insert(id.clone(), Arc::new(capability))
            .is_some()
        {
            return Err(EngineError::DuplicateCapability(id));
        }
        Ok(self)
    }
    /// Gets trusted code by exact ID.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn RuleCapability>> {
        self.capabilities.get(id)
    }
}

/// A compiled rule the engine cannot execute as authored.
///
/// Carried in the plan so every run reports it as not evaluated; dropping it
/// would make an unexecuted requirement indistinguishable from a satisfied one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeferredRule {
    /// Package-local stable rule ID.
    pub id: RuleId,
    /// Registered capability ID.
    pub capability: String,
    /// Why the rule cannot run as authored.
    pub reason: String,
}

/// Validated, deterministic request plan.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    rules: Vec<CompiledRule>,
    deferred: Vec<DeferredRule>,
    concepts: Arc<ConceptCatalog>,
    refinements: BTreeMap<RuleId, RuleRefinement>,
}
impl ExecutionPlan {
    /// Rules ordered by stable rule ID.
    pub fn rules(&self) -> &[CompiledRule] {
        &self.rules
    }
    /// Rules compiled but not executable as authored, ordered by rule ID.
    pub fn deferred(&self) -> &[DeferredRule] {
        &self.deferred
    }
    /// Canonical concepts declared by the ruleset's definition packages.
    pub fn concepts(&self) -> &ConceptCatalog {
        &self.concepts
    }
    /// How `rule` refines its outcomes; `None` when it declares nothing.
    pub fn refinement(&self, rule: &RuleId) -> Option<&RuleRefinement> {
        self.refinements.get(rule)
    }
}

mod boundary_coverage;
mod circulation;
mod classifications;
mod compiler;
mod concepts;
mod contact;
mod coordinate_system;
mod corridor_end;
mod coverage;
mod derived_relationships;
mod discipline_map;
mod door_leaves;
mod envelope_membership;
mod facade_area;
mod federation;
mod free_space;
mod guard;
mod integrity;
mod linear_quantity;
mod metric_routing;
mod object_frame;
mod pairwise;
mod plan_area;
mod plan_region;
mod plan_span;
mod properties;
mod proximity;
mod refinement;
mod relationships;
mod services;
mod side_distance;
mod sight;
mod source_metadata;
mod space;
mod topology;
mod triangle_count;
mod vertical_extent;
mod walkability;
mod walking_surface;
pub use boundary_coverage::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryCoverageServiceHandle, BoundaryOverlap, BoundaryPlacement, CoverageAreas,
    MeasuredBoundary, ShareInterval, SurfaceAreaInterval,
};
pub use circulation::{
    CirculationContact, CirculationMap, CirculationNode, CirculationNodeKind, CirculationRequest,
};
pub use classifications::{
    ClassificationAssignment, ClassificationError, ClassificationService,
    ClassificationServiceHandle,
};
pub use compiler::{QUALIFIED_RULE_SEPARATOR, SUPPORTED_SCHEMA_VERSION, compile, compile_rulesets};
pub use concepts::{
    BindingError, ConceptBindings, ConceptCatalog, ConceptKind, TypeHierarchyError,
    TypeHierarchyService, TypeHierarchyServiceHandle,
};
pub use contact::{
    ContactError, ContactEvidence, ContactRequest, ContactService, ContactServiceHandle,
    ContactSide, ContactTolerance,
};
pub use coordinate_system::{
    CoordinateFrame, CoordinateSystemError, CoordinateSystemService, CoordinateSystemServiceHandle,
    MapConversion, SourceCoordinateSystem,
};
pub use corridor_end::{CorridorEnd, CorridorEndRequest, CorridorEnds, EndWall, WallContact};
pub use coverage::{CoverageEvidence, CoverageRequest, EffectMeets, EffectReach, Participant};
pub use derived_relationships::{
    AdjacentSide, DERIVED_RELATIONSHIP_PREFIX, Derivation, DerivedRelationshipService,
    DerivedRelationshipServiceHandle, adjacent_side,
};
pub use discipline_map::{
    DisciplineMap, DisciplineMapError, DisciplineOrigin, DisciplineRule, UnmappedReason,
    wildcard_regex,
};
pub use door_leaves::{
    DoorLeaf, DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, PlanRing,
    SwingSector,
};
pub use envelope_membership::{
    EnvelopeDerivation, EnvelopeMembershipError, EnvelopeMembershipEvidence,
    EnvelopeMembershipRequest, EnvelopeMembershipService, EnvelopeMembershipServiceHandle,
};
pub use facade_area::{FacadeArea, FacadeAreaError, FacadeAreaService, FacadeAreaServiceHandle};
pub use free_space::{
    AreaInterval, BoxClearance, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest,
    ClearanceShape, CompleteClearanceEvidence, CompletePlacementEvidence, CompleteSupportEvidence,
    ContainmentEvidence, ContainmentOutcome, ContainmentRequest, CylinderClearance, ElevationBand,
    FrameOffsetPlacement, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService,
    FreeSpaceServiceHandle, MetricDirection, MetricFrame, ObstructionEvidence, PlacementDomain,
    PlacementOrientation, PlacementOutcome, PlacementRequest, PlacementShape,
    SignedDistanceInterval, SupportCoverageEvidence, SupportCoverageOutcome,
    SupportCoverageRequest, SupportedPlacement,
};
pub use guard::{
    ClimbableCandidate, GuardCandidate, GuardEdge, GuardError, GuardEvidence, GuardSearch,
    GuardService, GuardServiceHandle,
};
pub use integrity::{
    IntegrityError, IntegrityIssue, IntegritySeverity, SourceIntegrityService,
    SourceIntegrityServiceHandle,
};
pub use linear_quantity::{
    LinearInterval, LinearQuantityError, LinearQuantityEvidence, LinearQuantityKind,
    LinearQuantityRequest, LinearQuantityService, LinearQuantityServiceHandle, ShelfGeometry,
};
pub use metric_routing::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, FarthestPointEvidence,
    FarthestPointOutcome, FarthestPointRequest, LengthInterval, MetricPoint, MetricRouteEvidence,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingError, MetricRoutingService,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetEvidence, NearestTargetOutcome,
    NearestTargetRequest, PathTrace, PathTraceRequest, ThresholdVerdict, UnreachableRegionEvidence,
    UnreachableTargetsEvidence,
};
pub use object_frame::{
    ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront,
};
pub use pairwise::{
    CandidatePair, CandidateSearchError, candidate_pairs, projected_candidate_pairs,
};
pub use plan_area::{PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle, PlanBand};
pub use plan_region::ConvexPlanRegion;
pub use plan_span::{
    CentrePlacement, PlanCentre, PlanLength, PlanRecess, PlanRecesses, PlanRectangle, PlanSection,
    PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle, RectangleOrientation,
};
pub use properties::{
    CompletePropertyAbsenceEvidence, NameMatch, NamePattern, PropertyEnumeration,
    PropertyEnumerationRequest, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ResolvedProperty,
};
pub use proximity::{
    BodyContainment, Bounds3, FaceClass, FaceDistanceError, FaceDistanceEvidence,
    FaceDistanceRequest, GeometryFidelity, IntersectionVolume, ObjectBounds, OverlapAlongEvidence,
    OverlapAlongRequest, OverlapExtents, ProjectedDistanceEvidence, ProximityError,
    ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle, RegionDistanceEvidence, RegionDistanceRequest, VerticalDirection,
    VolumeInterval,
};
pub use refinement::{Deviation, RuleRefinement};
pub use relationships::{
    AbsentEndPolicy, CompleteRelationshipSelection, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    SemanticRelationship, TraversalDirection,
};
pub use services::{ServiceRegistry, ServiceRegistryError};
pub use session::{
    EvidenceSession, EvidenceSessionError, SessionSources, SnapshotBoundService, SourceDisciplines,
    SourceSnapshot,
};
pub use side_distance::{
    RectangleSide, SideDistance, SideDistanceRequest, SideDistances, SidePresence,
};
pub use sight::{
    SightError, SightEvidence, SightOutcome, SightRequest, SightService, SightServiceHandle,
};
pub use source_metadata::{SourceMetadata, SourceMetadataIndex};
pub use space::{
    BoundaryGap, Cap, CapCoverage, CapRequest, ClearHeightEvidence, Containment, SpaceError,
    SpaceOverlap, SpaceService, SpaceServiceHandle, StoreyResidual, SupportCounts,
};
pub use topology::{
    CompleteTopologyEvidence, ConnectivityGraph, RouteOutcome, TopologyError, VerifiedConnection,
};
pub use triangle_count::{
    TriangleCount, TriangleCountError, TriangleCountService, TriangleCountServiceHandle,
};
pub use vertical_extent::{
    DirectionalExtent, ElevationInterval, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
pub use walkability::{
    PassageAdmission, VerifiedWalkablePassage, VerticalConnector, VerticalConnectorKind,
    WalkabilityError, WalkabilityRegion, WalkabilityRegionId, WalkabilityRequest,
    WalkabilityRouteOutcome, WalkabilityService, WalkabilityServiceHandle, WalkabilitySnapshot,
};
pub use walking_surface::{
    ClearanceBelow, ClearanceBelowRequest, HandrailEvidence, HandrailRequest, Headroom,
    HeadroomRequest, Landing, LandingEvidence, LandingExtent, LandingRequest, MeasuredInterval,
    PlanSegment, RailMeasurement, RailSide, RiserClosure, SlopedRun, SlopedSurface, StretchPart,
    Tread, TreadFlight, TreadFlightRequest, WalkingEnd, WalkingLine, WalkingLinePlacement,
    WalkingStretch, WalkingSurfaceError, WalkingSurfaceService, WalkingSurfaceServiceHandle,
    across,
};

/// Binds a rule's outcomes to it, reporting each source-wide cause once.
///
/// An unbound concept depends on the package and the source, and an
/// unrecorded fact on the source alone, never on the object, so every object
/// of that source fails identically. Listing each one buries the single
/// cause under thousands of copies. Object-level outcomes with either reason
/// are merged per source, reason and message into one rule-level outcome
/// scoped to that source, naming the count and a few examples. Every other
/// outcome keeps its scope.
fn collapse_source_wide(
    rule_id: &RuleId,
    outcomes: Vec<CapabilityNotEvaluated>,
) -> Vec<NotEvaluated> {
    const EXAMPLES: usize = 3;
    let mut merged: BTreeMap<(axioval_ir::SourceId, NotEvaluatedReason, String), Vec<ObjectId>> =
        BTreeMap::new();
    let mut kept = Vec::new();
    for outcome in outcomes {
        match (outcome.reason, outcome.scope) {
            (
                reason @ (NotEvaluatedReason::UnboundConcept | NotEvaluatedReason::NotRecorded),
                Scope::Object(object),
            ) => merged
                .entry((object.source.clone(), reason, outcome.message))
                .or_default()
                .push(object),
            (reason, scope) => kept.push(NotEvaluated {
                rule_id: rule_id.clone(),
                scope,
                reason,
                message: outcome.message,
            }),
        }
    }
    kept.extend(merged.into_iter().map(|((source, reason, message), mut objects)| {
        objects.sort();
        let examples: Vec<&str> = objects
            .iter()
            .take(EXAMPLES)
            .map(|object| object.local_id.as_str())
            .collect();
        let more = objects.len().saturating_sub(EXAMPLES);
        let tail = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        NotEvaluated {
            rule_id: rule_id.clone(),
            scope: Scope::Source(source.clone()),
            reason,
            message: format!(
                "{message}; {} object(s) of source `{source}` not evaluated (e.g. {}{tail})",
                objects.len(),
                examples.join(", ")
            ),
        }
    }));
    kept
}

/// Deterministic runtime that invokes only registered trusted capabilities.
pub struct Runtime {
    registry: CapabilityRegistry,
    services: ServiceRegistry,
}
impl Runtime {
    /// Creates a runtime from a host-controlled registry.
    pub fn new(registry: CapabilityRegistry) -> Self {
        Self {
            registry,
            services: ServiceRegistry::new(),
        }
    }
    /// Adds adapter-provided host services to subsequent evaluations.
    #[must_use]
    pub fn with_services(mut self, services: ServiceRegistry) -> Self {
        self.services = services;
        self
    }
    /// Executes a plan and returns deterministically sorted findings.
    ///
    /// Execution fails closed if the host registry no longer contains any capability
    /// that was present when the plan was compiled.
    ///
    /// A bare project carries no source type-system declarations, so package
    /// concepts bind to nothing and concept-based selection is reported as not
    /// evaluated. Hosts that want concept binding run an [`EvidenceSession`].
    pub fn run(&self, project: &Project, plan: ExecutionPlan) -> Result<Report, EngineError> {
        self.run_with_services(
            project,
            &self.services,
            BTreeMap::new(),
            (
                SessionSources::new(project.objects().map(|object| object.id.source.clone())),
                SourceDisciplines::default(),
                SourceMetadataIndex::default(),
            ),
            plan,
        )
    }

    /// Executes a plan against one immutable source/evidence snapshot.
    pub fn run_session(
        &self,
        session: &EvidenceSession,
        plan: ExecutionPlan,
    ) -> Result<Report, EngineError> {
        let type_systems = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), snapshot.type_systems().to_vec()))
            .collect();
        self.run_with_services(
            session.project(),
            session.services(),
            type_systems,
            (
                SessionSources::new(
                    session
                        .snapshots()
                        .map(|snapshot| snapshot.source().clone()),
                ),
                session.source_disciplines(),
                session.metadata_index(),
            ),
            plan,
        )
    }

    fn run_with_services(
        &self,
        project: &Project,
        services: &ServiceRegistry,
        type_systems: BTreeMap<axioval_ir::SourceId, Vec<Arc<str>>>,
        (sources, disciplines, metadata): (SessionSources, SourceDisciplines, SourceMetadataIndex),
        plan: ExecutionPlan,
    ) -> Result<Report, EngineError> {
        // Bindings are per run: they join this plan's package concepts to this
        // project's declared type systems, so they cannot be registered once by
        // a host. A host-registered `ConceptBindings` is overridden rather than
        // trusted, because it could bind concepts the packages never declared.
        let mut services = services.clone();
        services.replace(ConceptBindings::new(plan.concepts.clone(), type_systems));
        // Disciplines are the session's declarations; a host-registered copy
        // could claim roles the session never declared.
        services.replace(disciplines);
        // So is what the session knows about each source as a whole.
        services.replace(metadata);
        // So are the sources: a host copy could hide an empty source.
        services.replace(sources);
        let services = &services;
        let context = RuleContext { project, services };
        let mut findings = Vec::new();
        let mut tables: Vec<ReportTable> = Vec::new();
        let mut not_evaluated: Vec<NotEvaluated> = plan
            .deferred
            .into_iter()
            .map(|rule| NotEvaluated {
                rule_id: rule.id,
                scope: Scope::Project,
                reason: NotEvaluatedReason::InvalidDeclaration,
                message: rule.reason,
            })
            .collect();
        for rule in plan.rules {
            let capability = self
                .registry
                .get(&rule.capability)
                .ok_or_else(|| EngineError::UnknownCapability(rule.capability.clone()))?;
            let rule_id = rule.id.clone();
            let mut evaluation = capability.evaluate(&context, &rule);
            if let Some(refinement) = plan.refinements.get(&rule_id) {
                evaluation.grade(&refinement.severity_bands);
            }
            findings.extend(evaluation.findings);
            // The compiled rule is the table's identity, whatever the capability named.
            tables.extend(
                evaluation
                    .tables
                    .into_iter()
                    .map(|table| table.with_rule_id(rule_id.clone())),
            );
            not_evaluated.extend(collapse_source_wide(&rule_id, evaluation.not_evaluated));
        }
        findings.sort_by(|a, b| {
            a.rule_id
                .cmp(&b.rule_id)
                // Project, then sources, then objects, each by identity.
                .then_with(|| a.scope.cmp(&b.scope))
                .then_with(|| a.message.cmp(&b.message))
        });
        not_evaluated.sort();
        tables.sort_by(|a, b| (a.rule_id(), a.name()).cmp(&(b.rule_id(), b.name())));
        if let Some(pair) = tables
            .windows(2)
            .find(|pair| (pair[0].rule_id(), pair[0].name()) == (pair[1].rule_id(), pair[1].name()))
        {
            return Err(EngineError::DuplicateReportTable {
                rule: pair[0].rule_id().to_string(),
                table: pair[0].name().to_owned(),
            });
        }
        Ok(Report {
            findings,
            not_evaluated,
            tables,
        })
    }
}
