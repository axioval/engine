//! Neutral executable-check specifications.
//!
//! This module is the semantic hand-off between external format codecs and the
//! runtime `rules` crate. Types here describe *what* to check in IFC terms; they
//! deliberately contain no CSET class names, parameter keys, Java descriptors,
//! model access, or executable rule objects.

use serde::{Deserialize, Serialize};

use super::assertion::{IdentifierRef, Severity};
use super::basic_checks::{
    ConditionalPresencePlanSpec, ElementDimensionPlanSpec, ElementValidationPlanSpec,
    TypeGroupSizeOutlierPlanSpec, WindowFloorRatioPlanSpec,
};
use super::building_storey::BuildingStoreyPlanSpec;
use super::clash_matrix::ClashMatrixPlanSpec;
use super::comparison::PropertyComparisonPlanSpec;
use super::component_clearance::ComponentClearancePlanSpec;
use super::daylight::FloorOpeningRatioPlanSpec;
use super::door_accessibility::DoorAccessibilityPlanSpec;
use super::effective_coverage::EffectiveCoveragePlanSpec;
use super::escape_route::EscapeRoutePlanSpec;
use super::exit_access_doorway::ExitAccessDoorwayPlanSpec;
use super::fire_compartment_membership::FireCompartmentMembershipPlanSpec;
use super::free_floor_space::FreeFloorSpacePlanSpec;
use super::front_clearance::FrontClearancePlanSpec;
use super::horizontal_guard::HorizontalGuardPlanSpec;
use super::local_circulation::LocalCirculationPlanSpec;
use super::manual_issue::ManualIssuePlanSpec;
use super::model::{
    FireCompartmentAreaPlanSpec, RequiredComponentsPlanSpec, SpaceGroupContainmentPlanSpec,
    SpacesInDerivedGroupsPlanSpec, StoreyNameSequencePlanSpec,
};
use super::model_architecture::ModelArchitecturePlanSpec;
use super::model_comparison::ModelComparisonPlanSpec;
use super::opening_sill::OpeningSillPlanSpec;
use super::parking::ParkingPlanSpec;
use super::profile::AllowedProfilePlanSpec;
use super::ramp::RampPlanSpec;
use super::relation::{
    RelationPlanSpec, SelectionCardinalityPlanSpec, SpaceComponentCountPlanSpec,
};
use super::shelf_capacity::ShelfCapacityPlanSpec;
use super::space_connection::SpaceConnectionPlanSpec;
use super::space_distance::SpaceDistancePlanSpec;
use super::spatial::{
    AccessibleSpacePlanSpec, BeamIntersectionPlanSpec, CirculationPlanSpec,
    ComponentDistancePlanSpec, FloorDistancePlanSpec, PairwisePlanSpec, PathPlanSpec,
    RouteComponentCompliancePlanSpec, WallDistancePlanSpec,
};
use super::stair::StairPlanSpec;
use super::structure_architecture_conformity::StructureArchitectureConformityPlanSpec;
use crate::rule::containment::ComponentContainmentPlanSpec;
use crate::rule::coverage::CoverageComparisonPlanSpec;
use crate::rule::envelope::BuildingEnvelopePlanSpec;
use crate::rule::external_wall::ExternalWallValidationPlanSpec;
use crate::rule::fire_wall_components::FireWallComponentsPlanSpec;
use crate::rule::layer_agreement::LayerAgreementPlanSpec;
use crate::rule::opening::ElementHolePlanSpec;
use crate::rule::slab_contact::SlabContactPlanSpec;
use crate::rule::space_validation::SpaceValidationPlanSpec;
use crate::rule::visibility::ComponentVisibilityPlanSpec;
use crate::rule::wall_validation::WallValidationPlanSpec;

/// A format-neutral ruleset ready for runtime compilation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<CheckRuleSpec>,
}

/// One configured semantic check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRuleSpec {
    /// Stable neutral rule-definition identity used by the runtime compiler.
    pub definition_id: String,
    /// Stable instance identity. Source adapters may derive this from native ids,
    /// but runtime compilation never interprets that source-specific scheme.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub semantics: CheckSemantics,
}

/// One named assertion inside a composite semantic rule.
///
/// The identifier is stable within its parent rule and becomes finding
/// provenance at runtime. It is semantic identity, not a source-format control
/// name or Java parameter handle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCheckSpec {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub semantics: CheckSemantics,
}

mod validation;

/// Reusable semantic kernels represented by the first executable migration.
///
/// These variants are intentionally capabilities, not vendor rule classes: one
/// capability may eventually compile many native templates or neutral catalog
/// definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckSemantics {
    /// Several independently evaluable assertions that retain one authored rule
    /// identity and one reported result.
    Composite {
        checks: Vec<SemanticCheckSpec>,
    },
    /// Per-source cardinality over an indexed IFC relation graph.
    RelationCardinality {
        plan: RelationPlanSpec,
    },
    /// Cardinality of one filtered component selection over a model.
    SelectionCardinality {
        plan: SelectionCardinalityPlanSpec,
    },
    /// Count provider-nearest classified components for matched spaces.
    SpaceComponentCount {
        plan: SpaceComponentCountPlanSpec,
    },
    /// Ratio of provider-resolved effective light-opening area to space area.
    FloorOpeningRatio {
        plan: FloorOpeningRatioPlanSpec,
    },
    /// Compare counts from two independently selected component sets.
    RelativeCount {
        plan: super::aggregate::RelativeCountPlanSpec,
    },
    /// Required component rows evaluated across selected federation members.
    RequiredComponents {
        plan: RequiredComponentsPlanSpec,
    },
    /// Ordinary spaces must belong to an accepted derived space-group overlay.
    SpacesInDerivedGroups {
        plan: SpacesInDerivedGroupsPlanSpec,
    },
    SpaceGroupContainment {
        plan: SpaceGroupContainmentPlanSpec,
    },
    /// Maximum area of provider-derived fire compartments.
    FireCompartmentArea {
        plan: FireCompartmentAreaPlanSpec,
    },
    /// Numeric storey names must form a sequence in ascending elevation order.
    StoreyNameSequence {
        plan: StoreyNameSequencePlanSpec,
    },
    /// Require each selected component's native TYPE designation to match one
    /// of the source-ordered patterns configured for its native component class.
    AgreedTypeValues {
        /// Native `cpFilterParameter`, lowered to the shared neutral component
        /// selector. Runtime applies it before exact table-class matching.
        selector: ElementScopeSpec,
        rows: Vec<AgreedTypeValueSpec>,
        case_sensitive: bool,
        severity: Severity,
    },
    /// Native `SpaceTypesFromAgreedListConstraint`: every space must match at
    /// least one row of an agreed list keyed on type/name/number.
    AgreedSpaceTypes {
        rows: Vec<AgreedSpaceRowSpec>,
        case_sensitive: bool,
        /// Native `cpAllowWhiteSpaces` — trims model values before comparing.
        allow_whitespace: bool,
        /// Which native `SSpace` populations participate in the check.
        #[serde(default)]
        group_mode: SpaceGroupCheckMode,
        severity: Severity,
    },
    /// Native `ComponentSimilarityConstraint`: among same-class components in
    /// the same scope, those sharing `compared` must also share `identical`.
    ConsistentProperty {
        pairs: Vec<ConsistentPropertySpec>,
        scope: SimilarityScope,
        element_scope: Option<ElementScopeSpec>,
        severity: Severity,
    },
    RequiredProperties {
        requirements: Vec<RequiredPropertySpec>,
    },
    UniqueIdentifier {
        applies_to: String,
        source: IdentifierRef,
        require_present: bool,
    },
    /// If the trigger population exists, the required population must exist.
    ConditionalPresence {
        plan: ConditionalPresencePlanSpec,
    },
    /// Every selected element must have non-degenerate body geometry.
    ElementValidation {
        plan: ElementValidationPlanSpec,
    },
    /// A selected nominal attribute/property dimension must satisfy its bounds.
    ElementDimension {
        plan: ElementDimensionPlanSpec,
    },
    /// Members of one type group must stay within a relative size tolerance.
    TypeGroupSizeOutlier {
        plan: TypeGroupSizeOutlierPlanSpec,
    },
    WindowFloorRatio {
        plan: WindowFloorRatioPlanSpec,
    },
    Interference {
        intersecting: ElementScopeSpec,
        intersected: ElementScopeSpec,
        min_depth_mm: f64,
    },
    /// Generic pairwise geometry intent. Vendor-specific rule classes compile
    /// into this composition before runtime execution.
    PairwiseGeometry {
        plan: PairwisePlanSpec,
    },
    /// Exact native `ComponentDistance` profile, including aggregate cardinality,
    /// projected modes, containment/container restriction, and door swing facts.
    ComponentDistance {
        plan: ComponentDistancePlanSpec,
    },
    /// Exact native `FloorDistance` profile, including cross-pair equality groups.
    FloorDistance {
        plan: FloorDistancePlanSpec,
    },
    /// Exact native `WallDistance` minimum and gross-area maximum profile.
    WallDistance {
        plan: WallDistancePlanSpec,
    },
    /// Generic field/path geometry intent over a sampled walkable domain.
    PathGeometry {
        plan: PathPlanSpec,
    },
    /// Compliance of selected route components: clear widths, obstructions,
    /// connection gaps, and stair/ramp endpoint connectivity.
    RouteComponentCompliance {
        plan: RouteComponentCompliancePlanSpec,
    },
    /// Clearance-shape fitting, vertical free-space, accessible-path, and door
    /// entrance checks inside selected spaces.
    AccessibleSpace {
        plan: AccessibleSpacePlanSpec,
    },
    /// Beam-axis-relative allowed penetration regions, support/connection
    /// exclusions, and exact residual intersection checking.
    BeamIntersectionCompliance {
        plan: BeamIntersectionPlanSpec,
    },
    /// Components whose extracted parametric profiles must match one of the
    /// configured neutral family/dimension rows.
    AllowedProfiles {
        plan: AllowedProfilePlanSpec,
    },
    /// Bidirectional architecture/structure geometric coverage.
    ArchitectureStructureCoverage {
        plan: CoverageComparisonPlanSpec,
    },
    /// Exterior-wall recess and vertical-airwell envelope compliance.
    BuildingEnvelope {
        plan: BuildingEnvelopePlanSpec,
    },
    /// Opening placement within an extruded parametric host profile.
    ElementHolePlacement {
        plan: ElementHolePlanSpec,
    },
    /// Exact component-inside-component classification, cardinality, and surface clearances.
    ComponentContainment {
        plan: ComponentContainmentPlanSpec,
    },
    /// Exact provider-resolved panoramic visibility and occlusion checking.
    ComponentVisibility {
        plan: ComponentVisibilityPlanSpec,
    },
    /// Exit-access doorway cardinality and separation relative to the space diagonal.
    ExitAccessDoorwayArrangement {
        plan: ExitAccessDoorwayPlanSpec,
    },
    /// Exact fire-wall, door, window, and opening type validation.
    FireWallComponents {
        plan: FireWallComponentsPlanSpec,
    },
    /// Typed agreed component/construction/layer associations.
    LayerAgreement {
        plan: LayerAgreementPlanSpec,
    },
    /// Exact compartmentation-derived exterior-wall assignment validation.
    ExternalWallValidation {
        plan: ExternalWallValidationPlanSpec,
    },
    /// Exact provider-resolved space enclosure, height, and unallocated-area validation.
    SpaceValidation {
        plan: SpaceValidationPlanSpec,
    },
    SlabContact {
        plan: SlabContactPlanSpec,
    },
    /// Exact wall geometry, dimensional, opening, and area consistency validation.
    WallValidation {
        plan: WallValidationPlanSpec,
    },
    /// Directional obstruction clearance in front of selected components.
    FrontClearance {
        plan: FrontClearancePlanSpec,
    },
    /// Directional accessible area, elevation, and protrusion checks around components.
    ComponentClearance {
        plan: ComponentClearancePlanSpec,
    },
    /// Geometric and optional property-derived effective coverage.
    EffectiveCoverage {
        plan: EffectiveCoveragePlanSpec,
    },
    /// Parking-space dimensions, orientation, aisle, and obstruction checks.
    Parking {
        plan: ParkingPlanSpec,
    },
    /// Local accessible paths, entrances, endpoint clearance, and selected-component connectivity.
    LocalCirculation {
        plan: LocalCirculationPlanSpec,
    },
    /// Ramp geometry, gradient, landing, stair, and optional handrail requirements.
    Ramp {
        plan: RampPlanSpec,
    },
    Stair {
        plan: StairPlanSpec,
    },
    ModelComparison {
        plan: ModelComparisonPlanSpec,
    },
    /// Exact provider-backed architectural model integrity checks.
    ModelArchitecture {
        plan: ModelArchitecturePlanSpec,
    },
    BuildingStorey {
        plan: BuildingStoreyPlanSpec,
    },

    ClashMatrix {
        plan: ClashMatrixPlanSpec,
    },
    EscapeRoute {
        plan: EscapeRoutePlanSpec,
    },
    /// Accessible free-area requirements within spaces and around furniture.
    FreeFloorSpace {
        plan: FreeFloorSpacePlanSpec,
    },
    /// Structural elements checked for horizontal and vertical architectural coverage.
    StructureArchitectureConformity {
        plan: StructureArchitectureConformityPlanSpec,
    },
    /// Per-row distance and access requirements between selected spaces.
    SpaceDistances {
        plan: SpaceDistancePlanSpec,
    },
    /// Maximum bottom-elevation difference between an opening and its unique
    /// adjacent space.
    OpeningSill {
        plan: OpeningSillPlanSpec,
    },
    /// Ordered accessibility requirements over doors connecting classified spaces.
    DoorAccessibility {
        plan: DoorAccessibilityPlanSpec,
    },
    /// Spaces on storeys must overlap one fire compartment above the native ratio.
    FireCompartmentMembership {
        plan: FireCompartmentMembershipPlanSpec,
    },
    /// Horizontal walking surfaces with native-equivalent fall-risk guard findings.
    HorizontalGuard {
        plan: HorizontalGuardPlanSpec,
    },
    /// Authored model-level findings from a manual checking table.
    ManualIssues {
        plan: ManualIssuePlanSpec,
    },
    /// Exact selected-component property to typed constant comparison.
    PropertyComparison {
        plan: PropertyComparisonPlanSpec,
    },
    /// Exact per-space shelf running-length requirements. Geometry-derived
    /// capacity is supplied by a neutral resolved-fact provider.
    ShelfCapacity {
        plan: ShelfCapacityPlanSpec,
    },
    /// Exact space-to-space and space-to-outside portal topology.
    SpaceConnection {
        plan: SpaceConnectionPlanSpec,
    },
    /// Whole-building circulation intent over per-space free-space layers and
    /// relation-grounded, geometry-verified portals.
    CirculationGeometry {
        plan: CirculationPlanSpec,
    },
    SpaceArea {
        min_area_m2: Option<f64>,
        max_area_m2: Option<f64>,
    },
    /// Require selected native identity properties on ordinary spaces. Native
    /// `SSpace.name` maps to `IfcSpace.LongName`, number maps to `IfcSpace.Name`,
    /// and type maps to the related `IfcSpaceType.Name` imported as `SType.code`.
    SpacePropertyPresence {
        #[serde(default)]
        check_space_groups: bool,
        require_type: bool,
        require_name: bool,
        require_number: bool,
    },
    /// Assign ordinary spaces to the first source-ordered requirement whose
    /// name pattern and rounded individual area both match, then compare exact
    /// group counts. Duplicate identical rows are combined by the codec.
    SpaceTypeSizeCount {
        /// Native classification scheme selected by `cpSpaceClassification`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        classification_name: Option<String>,
        #[serde(default)]
        group_mode: SpaceGroupCheckMode,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        group_types: Vec<String>,
        #[serde(default)]
        categorization: SpaceCategorizationSpec,
        #[serde(default)]
        total_area_mode: bool,
        requirements: Vec<SpaceTypeSizeCountSpec>,
    },
    /// Sum ordinary spaces directly contained in each storey and compare the
    /// total with the last matching configured name row.
    StoreySpaceAreaAggregate {
        limits: Vec<StoreyAreaLimitSpec>,
    },
    /// Count ordinary spaces directly contained in each storey. Distinct rows
    /// retain source order; duplicate identical rows are combined by the codec.
    StoreySpaceCountAggregate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        classification_name: Option<String>,
        #[serde(default)]
        group_mode: SpaceGroupCheckMode,
        requirements: Vec<StoreySpaceCountSpec>,
    },
    /// Every selected component must satisfy every applicable property
    /// predicate. One failed predicate produces one finding on that component.
    PropertyPredicates {
        selector: ElementScopeSpec,
        fallback_type: String,
        requirements: Vec<PropertyPredicateSpec>,
        /// Configured requirement rows whose source semantics deliberately skip
        /// property evaluation. Keeping these distinct from an absent parameter
        /// prevents adapters from inventing a predicate merely to compile.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        non_evaluating_requirements: Vec<NonEvaluatingRequirementSpec>,
        severity: Severity,
    },
}

mod properties;
pub use properties::*;

mod scope;
pub use scope::*;

mod diagnostics;
pub use diagnostics::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_check_spec_roundtrips_without_source_format_types() {
        let spec = CheckSetSpec {
            name: Some("QA".into()),
            rules: vec![CheckRuleSpec {
                definition_id: "space.unique_identifier".into(),
                id: "ruleset.1".into(),
                name: Some("Space numbers".into()),
                semantics: CheckSemantics::UniqueIdentifier {
                    applies_to: "IFCSPACE".into(),
                    source: IdentifierRef::Name,
                    require_present: true,
                },
            }],
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(!json.contains("Constraint"));
        assert!(!json.contains("ruleLogicClassName"));
        assert_eq!(serde_json::from_str::<CheckSetSpec>(&json).unwrap(), spec);
    }

    #[test]
    fn space_property_presence_validation_accepts_a_noop_selection() {
        let rule = CheckRuleSpec {
            definition_id: "space.required_identity".into(),
            id: "space-identity.1".into(),
            name: None,
            semantics: CheckSemantics::SpacePropertyPresence {
                check_space_groups: false,
                require_type: false,
                require_name: false,
                require_number: false,
            },
        };
        rule.validate().unwrap();
    }

    /// An inactive Rule 230 plan — no selector clauses and no requirements —
    /// is a legal native no-op, not a malformed plan.
    ///
    /// Native `TotalFilter` only builds match branches for INCLUDE rows, so an
    /// empty or exclude-only filter authors an empty selector. Rejecting that
    /// at the spec gate would refuse check sets that source tools load and
    /// runs (to zero findings).
    #[test]
    fn property_predicate_validation_accepts_an_inactive_plan_as_a_no_op() {
        let rule = CheckRuleSpec {
            definition_id: "data.property_predicates".into(),
            id: "property.1".into(),
            name: None,
            semantics: CheckSemantics::PropertyPredicates {
                selector: ElementScopeSpec::default(),
                fallback_type: "IFCROOT".into(),
                requirements: Vec::new(),
                non_evaluating_requirements: Vec::new(),
                severity: Severity::Warning,
            },
        };
        rule.validate()
            .expect("an inactive Rule 230 plan is a legal no-op");
    }

    /// Java-regex values are represented losslessly in the neutral plan. Their
    /// exact compilation and invalid-pattern fail-closed behavior belongs to
    /// the runtime, which uses a Java-compatible engine.
    #[test]
    fn agreed_type_values_accepts_java_regex_patterns_at_the_spec_gate() {
        let rule = CheckRuleSpec {
            definition_id: "element.agreed_type_values".into(),
            id: "agreed-type.1".into(),
            name: None,
            semantics: CheckSemantics::AgreedTypeValues {
                selector: ElementScopeSpec::default(),
                rows: vec![AgreedTypeValueSpec {
                    applies_to: "IFCWALL".into(),
                    checked_property: ElementField::TypeDesignation,
                    allowed_values: vec!["rx:.*".into()],
                }],
                case_sensitive: false,
                severity: Severity::Warning,
            },
        };
        assert!(rule.validate().is_ok());
    }

    /// Native `rx:` rows remain executable when a neutral specification is
    /// deserialized directly instead of arriving through the CSET codec.
    #[test]
    fn agreed_space_types_accepts_java_regex_at_the_spec_gate() {
        let rule = CheckRuleSpec {
            definition_id: "space.agreed_type_values".into(),
            id: "agreed-space.1".into(),
            name: None,
            semantics: CheckSemantics::AgreedSpaceTypes {
                rows: vec![AgreedSpaceRowSpec {
                    space_name: Some("rx:.*".into()),
                    ..AgreedSpaceRowSpec::default()
                }],
                case_sensitive: false,
                allow_whitespace: true,
                group_mode: SpaceGroupCheckMode::NoSpaceGroups,
                severity: Severity::Warning,
            },
        };
        assert!(rule.validate().is_ok());
    }

    /// A row with every cell blank matches every space, silently neutralising
    /// the check. Native `getMatchingRow` skips such rows.
    #[test]
    fn agreed_space_types_refuses_a_fully_blank_row() {
        let rule = CheckRuleSpec {
            definition_id: "space.agreed_type_values".into(),
            id: "agreed-space.2".into(),
            name: None,
            semantics: CheckSemantics::AgreedSpaceTypes {
                rows: vec![AgreedSpaceRowSpec {
                    space_type: Some("   ".into()),
                    ..AgreedSpaceRowSpec::default()
                }],
                case_sensitive: false,
                allow_whitespace: true,
                group_mode: SpaceGroupCheckMode::NoSpaceGroups,
                severity: Severity::Warning,
            },
        };
        assert!(rule.validate().unwrap_err().contains("no populated column"));
    }

    #[test]
    fn diagnostic_is_structured_by_stage_and_code() {
        let diagnostic = RuleDiagnostic {
            source_type: "NativeUnknown".into(),
            source_id: Some("7".into()),
            definition_id: None,
            stage: DiagnosticStage::CodecToSpec,
            code: DiagnosticCode::UnsupportedRule,
            message: "no neutral binding".into(),
        };
        let value = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(value["stage"], "codec_to_spec");
        assert_eq!(value["code"], "unsupported_rule");
    }
}
