//! Format-neutral composition of pairwise and path spatial checks.
//!
//! These types describe rule intent only. Model selection, geometry lowering,
//! acceleration, query algorithms, and issue rendering belong to runtime crates.

use serde::{Deserialize, Serialize};

use super::assertion::Severity;
use super::execution::ElementScopeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Length(f64);

impl Length {
    pub fn millimetres(value: f64) -> Result<Self, String> {
        let length = Self(value);
        length.validate()?;
        Ok(length)
    }

    pub fn as_millimetres(self) -> f64 {
        self.0
    }

    pub fn validate(self) -> Result<(), String> {
        if !self.0.is_finite() || self.0 < 0.0 {
            return Err("length must be finite and non-negative millimetres".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Volume(f64);

impl Volume {
    pub fn cubic_millimetres(value: f64) -> Result<Self, String> {
        let volume = Self(value);
        volume.validate()?;
        Ok(volume)
    }

    pub fn as_cubic_millimetres(self) -> f64 {
        self.0
    }

    pub fn validate(self) -> Result<(), String> {
        if !self.0.is_finite() || self.0 < 0.0 {
            return Err("volume must be finite and non-negative cubic millimetres".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operator", content = "value", rename_all = "snake_case")]
pub enum NumericRequirement {
    AtLeast(Length),
    AtMost(Length),
    GreaterThan(Length),
    LessThan(Length),
}

impl NumericRequirement {
    pub fn threshold(self) -> Length {
        match self {
            Self::AtLeast(value)
            | Self::AtMost(value)
            | Self::GreaterThan(value)
            | Self::LessThan(value) => value,
        }
    }

    pub fn accepts(self, measured_mm: f64) -> bool {
        if !measured_mm.is_finite() {
            return false;
        }
        let threshold = self.threshold().as_millimetres();
        match self {
            Self::AtLeast(_) => measured_mm >= threshold,
            Self::AtMost(_) => measured_mm <= threshold,
            Self::GreaterThan(_) => measured_mm > threshold,
            Self::LessThan(_) => measured_mm < threshold,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingPolicy {
    WithinSelection,
    CrossSelections,
    EachLeftAgainstAnyRight,
    NearestRight,
    /// Adjacent elements after sorting by bottom elevation, admitted only when
    /// their horizontal AABB footprints overlap. This is a neutral structural
    /// pairing policy; exact projected-area thresholds remain provider-specific.
    AdjacentVerticalOverlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    SurfaceEuclidean,
    HorizontalSeparation,
    VerticalSeparation,
    VerticalTopToTop,
    VerticalBottomToBottom,
    Centroid,
    AabbLowerBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    BoundaryInside,
    BoundaryOutside,
    BoundaryViolation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "relation", rename_all = "snake_case")]
pub enum SpatialPredicateSpec {
    Penetrates {
        min_depth: Length,
    },
    Intersects {
        tolerance: Length,
    },
    Distance {
        metric: DistanceMetric,
        requirement: NumericRequirement,
    },
    Clearance {
        minimum: Length,
        /// Distances at or below this tolerance belong to contact/clash checks,
        /// not the open-clearance interval. Omit when every sub-minimum
        /// distance, including contact, is a violation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contact_tolerance: Option<Length>,
    },
    Touches {
        tolerance: Length,
    },
    Contains {
        boundary: BoundaryPolicy,
    },
    Overlap {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum_volume: Option<Volume>,
    },
}

impl SpatialPredicateSpec {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Penetrates { min_depth } => {
                min_depth.validate()?;
                if min_depth.as_millimetres() == 0.0 {
                    return Err(
                        "penetration depth must be positive; use intersects for zero-depth contact"
                            .into(),
                    );
                }
                Ok(())
            }
            Self::Intersects { tolerance } | Self::Touches { tolerance } => tolerance.validate(),
            Self::Distance { requirement, .. } => requirement.threshold().validate(),
            Self::Clearance {
                minimum,
                contact_tolerance,
            } => {
                minimum.validate()?;
                if let Some(tolerance) = contact_tolerance {
                    tolerance.validate()?;
                    if tolerance.as_millimetres() >= minimum.as_millimetres() {
                        return Err(
                            "clearance contact tolerance must be below the minimum clearance"
                                .into(),
                        );
                    }
                }
                Ok(())
            }
            Self::Contains { .. } => Ok(()),
            Self::Overlap { minimum_volume } => {
                if let Some(volume) = minimum_volume {
                    volume.validate()?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "quantifier", content = "count", rename_all = "snake_case")]
pub enum PairQuantifier {
    EveryPair,
    EveryLeftHasMatch,
    NoPair,
    AtLeast(usize),
    AtMost(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairSuppression {
    SameElement,
    OpeningFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairEvidencePolicy {
    EveryViolatingPair,
    FirstViolation,
    PerLeftSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResultPolicy {
    pub evidence: PairEvidencePolicy,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwisePlanSpec {
    pub left: ElementScopeSpec,
    pub right: ElementScopeSpec,
    pub pairing: PairingPolicy,
    pub predicate: SpatialPredicateSpec,
    pub quantifier: PairQuantifier,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressions: Vec<PairSuppression>,
    pub result: PairResultPolicy,
}

impl PairwisePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.predicate.validate()?;
        if matches!(self.quantifier, PairQuantifier::AtLeast(0)) {
            return Err("at-least quantifier must be positive".into());
        }
        if self.quantifier == PairQuantifier::EveryLeftHasMatch
            && !matches!(
                self.pairing,
                PairingPolicy::EachLeftAgainstAnyRight | PairingPolicy::NearestRight
            )
        {
            return Err("every-left-has-match requires each-left or nearest-right pairing".into());
        }
        if self.pairing == PairingPolicy::WithinSelection && self.left != self.right {
            return Err("within-selection pairing requires identical selectors".into());
        }
        if matches!(
            self.quantifier,
            PairQuantifier::EveryLeftHasMatch
                | PairQuantifier::AtLeast(_)
                | PairQuantifier::AtMost(_)
        ) && self.suppressions.contains(&PairSuppression::OpeningFill)
        {
            return Err(
                "aggregate/directional quantifiers cannot use relation-dependent opening-fill suppression"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Native `ComponentDistanceRule.rpCheckMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentCheckMode {
    MaximumAllowed,
    MinimumRequired,
    Range,
}

/// Native `rpOverlappingProjection` / distance-calculation branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentProjectionMode {
    Minimum3d,
    Above,
    Below,
    AboveWithinOffsetFootprint,
    BelowWithinOffsetFootprint,
    Horizontal,
    Minimum2d,
    Overlapping2d,
}

/// Native vertical surface pairing used by projected modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentSurfaces {
    TopFootprintToClosestOverlap,
    BottomFootprintToTopFootprint,
    TopFootprintToTopFootprint,
    BottomFootprintToClosestOverlap,
    TopFootprintToBottomFootprint,
    BottomFootprintToBottomFootprint,
    TopToBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentMode {
    Ignore,
    Space,
    SpaceGroup,
}

/// Complete runtime contract for native `ComponentDistanceRule`.
///
/// Providers return exact, threshold-free geometry/relation facts. Candidate
/// restriction, native mode dispatch, threshold comparisons, and the per-source
/// `minimum_amount` policy remain runtime-owned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentDistancePlanSpec {
    pub source: ElementScopeSpec,
    pub target: ElementScopeSpec,
    pub check_mode: ComponentCheckMode,
    pub distance_min_mm: f64,
    pub distance_max_mm: f64,
    pub minimum_amount: usize,
    pub projection: ComponentProjectionMode,
    pub surfaces: ComponentSurfaces,
    pub use_container_filter: bool,
    pub container_filter: ElementScopeSpec,
    pub containment: ContainmentMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub space_group_type_names: Vec<String>,
    pub use_door_swing_footprint: bool,
    pub horizontal_footprint_offset_mm: f64,
    pub horizontal_between_footprint_elevation_offset_mm: f64,
    pub result: PairResultPolicy,
}

impl ComponentDistancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("minimum distance", self.distance_min_mm),
            ("maximum distance", self.distance_max_mm),
            (
                "horizontal footprint offset",
                self.horizontal_footprint_offset_mm,
            ),
            (
                "between-footprint elevation offset",
                self.horizontal_between_footprint_elevation_offset_mm,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite non-negative millimetres"));
            }
        }
        if self.minimum_amount == 0 {
            return Err("component distance minimum_amount must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorSurfaceMode {
    TopToTop,
    BottomToBottom,
    TopToBottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorDistanceModeSpec {
    pub surface: FloorSurfaceMode,
    pub distances_equal: bool,
    pub minimum_mm: f64,
    pub maximum_mm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorDistancePlanSpec {
    pub selector: ElementScopeSpec,
    pub modes: Vec<FloorDistanceModeSpec>,
    pub result: PairResultPolicy,
}

impl FloorDistancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.modes.is_empty() {
            return Err("floor distance requires at least one enabled surface mode".into());
        }
        let mut surfaces = std::collections::BTreeSet::new();
        for mode in &self.modes {
            if !surfaces.insert(mode.surface) {
                return Err("floor distance surface modes must be unique".into());
            }
            if !mode.minimum_mm.is_finite()
                || mode.minimum_mm < 0.0
                || !mode.maximum_mm.is_finite()
                || mode.maximum_mm < mode.minimum_mm
            {
                return Err(
                    "floor distance bounds must be finite, non-negative, and ordered".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallDistancePlanSpec {
    pub selector: ElementScopeSpec,
    pub minimum_mm: f64,
    pub maximum_mm: f64,
    pub result: PairResultPolicy,
}

impl WallDistancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.minimum_mm.is_finite()
            || self.minimum_mm < 0.0
            || !self.maximum_mm.is_finite()
            || self.maximum_mm < 0.0
        {
            return Err("wall distance bounds must be finite non-negative millimetres".into());
        }
        if self.minimum_mm == 0.0 && self.maximum_mm == 0.0 {
            return Err("wall distance must enable a minimum or maximum check".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ratio(f64);

impl Ratio {
    pub fn new(value: f64) -> Result<Self, String> {
        if !value.is_finite() || value < 0.0 {
            return Err("ratio must be finite and non-negative".into());
        }
        Ok(Self(value))
    }

    pub fn as_fraction(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MobilityProfileSpec {
    pub body_width: Length,
    pub body_height: Length,
    pub maximum_step: Length,
    pub maximum_slope: Ratio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteWidthRequirementsSpec {
    pub route: Length,
    pub stair: Length,
    pub ramp: Length,
    pub door: Length,
}

impl RouteWidthRequirementsSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, width) in [
            ("route width", self.route),
            ("stair width", self.stair),
            ("ramp width", self.ramp),
            ("door width", self.door),
        ] {
            width.validate()?;
            if width.as_millimetres() == 0.0 {
                return Err(format!("{name} must be positive"));
            }
        }
        Ok(())
    }
}

/// Native-neutral compliance contract for a connected chain of route components.
///
/// Unlike [`PathPlanSpec`], this checks the selected components themselves: clear
/// footprint width, obstacle penetration, gaps to accessible spaces/elevators,
/// door clear width, and stair/ramp connectivity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteComponentCompliancePlanSpec {
    pub route_components: ElementScopeSpec,
    pub accessible_spaces: ElementScopeSpec,
    pub accessible_elevators: ElementScopeSpec,
    pub obstacles: ElementScopeSpec,
    pub widths: RouteWidthRequirementsSpec,
    pub allowed_obstruction_depth: Length,
    pub allowed_gap: Length,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl RouteComponentCompliancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.widths.validate()?;
        self.allowed_obstruction_depth.validate()?;
        self.allowed_gap.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathEndpointPolicy {
    /// XY center of the selected element's bounds at its lower elevation.
    BoundsFootprintCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathSamplingSpec {
    pub cell_size: Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "requirement", content = "value", rename_all = "snake_case")]
pub enum PathRequirementSpec {
    Reachable,
    MaximumLength(Length),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathEvidencePolicy {
    EveryDestination,
    FirstFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathResultPolicy {
    pub evidence: PathEvidencePolicy,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

/// Optional route-compliance features beyond basic reachability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassingSpaceSpec {
    pub minimum_width: Length,
    pub maximum_spacing: Length,
}

impl PassingSpaceSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.minimum_width.validate()?;
        self.maximum_spacing.validate()?;
        if self.minimum_width.as_millimetres() == 0.0
            || self.maximum_spacing.as_millimetres() == 0.0
        {
            return Err("passing-space width and spacing must be positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathComplianceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevators: Option<ElementScopeSpec>,
    #[serde(default)]
    pub subtract_door_swings: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passing_spaces: Option<PassingSpaceSpec>,
}

impl PathComplianceSpec {
    pub fn is_default(&self) -> bool {
        self.elevators.is_none() && !self.subtract_door_swings && self.passing_spaces.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(scope) = &self.elevators {
            if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
                return Err("elevator scope has no candidate types or clauses".into());
            }
        }
        if let Some(passing) = &self.passing_spaces {
            passing.validate()?;
        }
        Ok(())
    }
}

/// A field/path analysis plan over a traversable domain and route endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathPlanSpec {
    pub origins: ElementScopeSpec,
    pub destinations: ElementScopeSpec,
    pub walkable: ElementScopeSpec,
    pub obstacles: ElementScopeSpec,
    #[serde(default, skip_serializing_if = "PathComplianceSpec::is_default")]
    pub compliance: PathComplianceSpec,
    pub endpoints: PathEndpointPolicy,
    pub mobility: MobilityProfileSpec,
    pub sampling: PathSamplingSpec,
    pub requirement: PathRequirementSpec,
    pub result: PathResultPolicy,
}

impl PathPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, scope) in [
            ("origin", &self.origins),
            ("destination", &self.destinations),
            ("walkable", &self.walkable),
            ("obstacle", &self.obstacles),
        ] {
            if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
                return Err(format!(
                    "path {name} scope has no candidate types or clauses"
                ));
            }
        }

        self.compliance.validate()?;
        validate_path_contract("path", self.mobility, self.sampling, self.requirement)
    }
}

/// A building-network path plan. Each selected space contributes its own 2.5D
/// free-space layer; relation-grounded, geometry-verified door portals connect
/// those layers. A one-sided portal is usable as an exit only when its door is
/// explicitly selected by `exits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CirculationPlanSpec {
    pub origin_spaces: ElementScopeSpec,
    pub exits: ElementScopeSpec,
    pub obstacles: ElementScopeSpec,
    pub mobility: MobilityProfileSpec,
    pub sampling: PathSamplingSpec,
    pub maximum_landing_distance: Length,
    pub aperture_tolerance: Length,
    pub requirement: PathRequirementSpec,
    pub result: PathResultPolicy,
}

impl CirculationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, scope) in [
            ("origin", &self.origin_spaces),
            ("exit", &self.exits),
            ("obstacle", &self.obstacles),
        ] {
            if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
                return Err(format!(
                    "circulation {name} scope has no candidate types or clauses"
                ));
            }
        }
        validate_path_contract(
            "circulation",
            self.mobility,
            self.sampling,
            self.requirement,
        )?;
        self.maximum_landing_distance.validate()?;
        if self.maximum_landing_distance.as_millimetres() == 0.0 {
            return Err("circulation maximum landing distance must be positive".into());
        }
        self.aperture_tolerance.validate()?;
        Ok(())
    }
}

fn validate_path_contract(
    family: &str,
    mobility: MobilityProfileSpec,
    sampling: PathSamplingSpec,
    requirement: PathRequirementSpec,
) -> Result<(), String> {
    for (name, value, positive) in [
        ("body width", mobility.body_width, true),
        ("body height", mobility.body_height, true),
        ("maximum step", mobility.maximum_step, false),
        ("cell size", sampling.cell_size, true),
    ] {
        value.validate()?;
        if positive && value.as_millimetres() == 0.0 {
            return Err(format!("{family} {name} must be positive"));
        }
    }
    if sampling.cell_size.as_millimetres() > mobility.body_width.as_millimetres() * 0.5 {
        return Err(format!(
            "{family} cell size must not exceed half the body width"
        ));
    }
    if let PathRequirementSpec::MaximumLength(maximum) = requirement {
        maximum.validate()?;
        if maximum.as_millimetres() == 0.0 {
            return Err("maximum path length must be positive".into());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum AccessibleSpaceShapeSpec {
    Cylinder { diameter: Length },
    Cuboid { width: Length, length: Length },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibleSpacePlanSpec {
    pub spaces: ElementScopeSpec,
    #[serde(default)]
    pub merge_spaces: bool,
    pub obstacles: ElementScopeSpec,
    pub shape: AccessibleSpaceShapeSpec,
    pub lower_elevation: Length,
    pub upper_elevation: Length,
    #[serde(default)]
    pub check_accessible_path: bool,
    pub doors: ElementScopeSpec,
    pub path_width: Length,
    pub path_tolerance: Length,
    #[serde(default)]
    pub subtract_door_swings: bool,
    #[serde(default)]
    pub check_door_width: bool,
    pub minimum_door_width: Length,
    pub fitting_tolerance: Length,
    pub elevation_tolerance: Length,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl AccessibleSpacePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for value in [
            self.lower_elevation,
            self.upper_elevation,
            self.path_width,
            self.path_tolerance,
            self.minimum_door_width,
            self.fitting_tolerance,
            self.elevation_tolerance,
        ] {
            value.validate()?;
        }
        match self.shape {
            AccessibleSpaceShapeSpec::Cylinder { diameter } => {
                positive_accessible_length(diameter, "cylinder diameter")?;
            }
            AccessibleSpaceShapeSpec::Cuboid { width, length } => {
                positive_accessible_length(width, "cuboid width")?;
                positive_accessible_length(length, "cuboid length")?;
            }
        }
        if self.upper_elevation.as_millimetres() <= self.lower_elevation.as_millimetres() {
            return Err("upper elevation must exceed lower elevation".into());
        }
        if self.check_accessible_path
            && self.path_width.as_millimetres() <= 2.0 * self.path_tolerance.as_millimetres()
        {
            return Err("path width must exceed twice the path tolerance".into());
        }
        if self.check_door_width {
            positive_accessible_length(self.minimum_door_width, "minimum door width")?;
        }
        Ok(())
    }
}

fn positive_accessible_length(value: Length, name: &str) -> Result<(), String> {
    value.validate()?;
    if value.as_millimetres() <= 0.0 {
        Err(format!("{name} must be positive"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeamClearanceReferenceSpec {
    Height,
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RelativeClearanceSpec {
    pub numerator: u32,
    pub denominator: u32,
    pub reference: BeamClearanceReferenceSpec,
    pub minimum: Length,
}

impl RelativeClearanceSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.minimum.validate()?;
        if self.denominator == 0 {
            return Err("relative clearance denominator must be non-zero".into());
        }
        if self.numerator == 0 {
            return Err("relative clearance numerator must be non-zero".into());
        }
        Ok(())
    }
    pub fn resolve(&self, reference: Length) -> Length {
        let relative =
            reference.as_millimetres() * f64::from(self.numerator) / f64::from(self.denominator);
        Length::millimetres(relative.max(self.minimum.as_millimetres()))
            .expect("validated finite clearance")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeamIntersectionAllowanceRowSpec {
    pub beam_end: RelativeClearanceSpec,
    pub beam_top: RelativeClearanceSpec,
    pub beam_bottom: RelativeClearanceSpec,
    pub connection: RelativeClearanceSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeamIntersectionPlanSpec {
    pub checked_beams: ElementScopeSpec,
    pub connecting_beams: ElementScopeSpec,
    pub supporting_components: ElementScopeSpec,
    pub allowed_intersections: ElementScopeSpec,
    pub allowance_rows: Vec<BeamIntersectionAllowanceRowSpec>,
    #[serde(default)]
    pub allow_only_through_beam: bool,
    pub tolerance: Length,
    #[serde(default)]
    pub check_inclined: bool,
    #[serde(default)]
    pub show_allowed: bool,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl BeamIntersectionPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.tolerance.validate()?;
        if self.allowance_rows.is_empty() {
            return Err("beam intersection allowance table must contain at least one row".into());
        }
        for row in &self.allowance_rows {
            for value in [row.beam_end, row.beam_top, row.beam_bottom, row.connection] {
                value.validate()?;
            }
        }
        Ok(())
    }
}
