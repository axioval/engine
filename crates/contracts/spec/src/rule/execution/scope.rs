use serde::{Deserialize, Serialize};

/// A full IFC-facing component scope with an explicit include-combination mode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ElementScopeSpec {
    #[serde(default)]
    pub combine: ElementScopeCombine,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clauses: Vec<ElementScopeClause>,
    /// IFC recursive type roots the runtime should enumerate. Empty means use
    /// the semantic check's documented fallback type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_types: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementScopeCombine {
    /// Compatibility mode used by the original four CSET bindings.
    #[default]
    AnyClause,
    /// Source `TotalFilter`: AND include clauses inside one IFC-type group,
    /// then OR the groups. Excludes remain negations after group selection.
    AllPerType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementScopeClause {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifc_type: Option<String>,
    pub include: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<ElementField>,
    pub op: ElementScopeOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<ElementRelation>,
}

/// Which neutral IFC value a scope predicate reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum ElementField {
    IfcEntity,
    /// IFC `IfcRoot.GlobalId`.
    SpaceGroupType,
    GlobalId,
    /// Discipline assigned to the model containing this element.
    ///
    /// This is import/federation metadata, not an IFC entity attribute. A
    /// standalone model may therefore leave it undefined.
    ModelDomain,
    /// Material name resolved from IFC material associations, directly or
    /// through the associated IFC type object.
    MaterialName,
    /// `IfcTypeObject.Name`, reached through `IfcRelDefinesByType` for an
    /// occurrence. Distinct from occurrence `Name` and `ObjectType`.
    ConstructionTypeName,
    /// Native-neutral component type designation: a non-empty
    /// `Pset_*Common.Reference`, otherwise the related `IfcTypeObject.Name`.
    /// Distinct from IFC class, occurrence `Name`, `ObjectType`, and predefined type.
    TypeDesignation,
    /// Raw IFC `PredefinedType`, preferring an occurrence's explicit enum and
    /// otherwise inheriting from its related type object.
    PredefinedType,
    /// Value assigned by an imported IFC classification association, keyed by
    /// the exact `IfcClassification.Name` used as the scheme identity.
    Classification {
        scheme: String,
    },
    /// Non-empty `IfcProject.Phase` inherited through spatial containment and
    /// decomposition from the component's containing project.
    ProjectPhase,
    /// Presence of at least one successfully resolved product geometry item.
    Geometry,
    /// Native-neutral global X used by location checks: placement origin unless
    /// it lies over 0.5 length units outside the component AABB, then AABB min X.
    GlobalPositionX,
    /// Native-neutral global Y, with the same origin/AABB rule as X.
    GlobalPositionY,
    /// Native-neutral global lower elevation from the component AABB minimum Z;
    /// components without usable bounds use the source-proven zero fallback.
    GlobalBottomElevation,
    /// Fully-supported world AABB height, normalized to metres.
    BoundingBoxHeight,
    /// Native non-negative geometry volume at a source-proven zero-height
    /// boundary. Other geometry-volume shapes remain undefined.
    GeometryBottomArea,
    GeometryVolume,
    /// Exact truth value of native `SSpace.doorArea > 0`. This deliberately
    /// preserves the grounded predicate rather than claiming a display area.
    HasPositiveSpaceDoorArea,
    Name,
    ObjectType,
    LongName,
    Description,
    Property {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        property_set: Option<String>,
        name: String,
    },
    /// Typed IFCBOOLEAN property. Unlike the textual property selector this
    /// never treats arbitrary strings such as `"true"` as booleans.
    BooleanProperty {
        property_set: String,
        name: String,
    },
    /// A native `PropertySetPropertyReference`: property-set name, property
    /// name, and the authored quantity/value type, all of which are part of the
    /// reference's identity in native code.
    ///
    /// Both names are native `Operators.MATCHES` patterns — wildcards, `rx:`
    /// regex, and case-insensitive literals — so this is deliberately NOT the
    /// same thing as [`Self::Property`], whose names are exact strings and whose
    /// property-set lookup also accepts the internal set key.
    NativeProperty {
        property_set: String,
        name: String,
        value_type: NativePropertyValueType,
    },
}

/// The `quantityType` class carried by a native `PropertySetPropertyReference`.
///
/// Native code selects operators, editors, and renderers from this class, and
/// includes it in the reference's `equals`/`hashCode`. Two references with the
/// same names but different quantity types are different references, so this
/// must not be discarded when lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePropertyValueType {
    /// `java.lang.String`
    Text,
    /// `java.lang.Boolean`
    Boolean,
    /// `java.lang.Integer` / `java.lang.Long`
    Integer,
    /// `java.lang.Double` / `java.lang.Float` and unit-bearing quantities.
    Number,
}

impl NativePropertyValueType {
    /// Map a decoded Java simple class name onto the neutral value type.
    ///
    /// Unknown classes return `None`: native operator selection is driven by
    /// this class, so guessing would silently change which comparisons a rule
    /// is allowed to make.
    pub fn from_java_class(name: &str) -> Option<Self> {
        match name {
            "String" | "java.lang.String" => Some(Self::Text),
            "Boolean" | "java.lang.Boolean" | "IFCBOOLEAN" => Some(Self::Boolean),
            "Integer" | "Long" | "java.lang.Integer" | "java.lang.Long" => Some(Self::Integer),
            "Double" | "Float" | "java.lang.Double" | "java.lang.Float" => Some(Self::Number),
            _ => None,
        }
    }
}

impl ElementField {
    pub(super) fn is_global_location(&self) -> bool {
        matches!(
            self,
            Self::GlobalPositionX | Self::GlobalPositionY | Self::GlobalBottomElevation
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "targets", rename_all = "snake_case")]
pub enum ElementScopeOp {
    Equals(Vec<String>),
    NotEquals(Vec<String>),
    Matches(Vec<String>),
    /// Native `Operators.MATCHES_CASE` (ordinal 9).
    MatchesCase(Vec<String>),
    /// Case-insensitive glob-aware containment. Patterns are normalized by the
    /// source adapter to carry leading and trailing `*` as required.
    Contains(Vec<String>),
    OneOf(Vec<String>),
    NoneOf(Vec<String>),
    DomainOneOf(Vec<ModelDomain>),
    /// Ordered numeric comparison for source-grounded scalar selectors.
    AtMost(f64),
    AtLeast(f64),
    Greater(f64),
    Smaller(f64),
    IsDefined,
    IsUndefined,
    IsNotEmpty,
    IsEmpty,
    ClassOnly,
}

/// Format-neutral model disciplines used by component applicability filters.
///
/// These are semantic values rather than serialized native ordinals. Source
/// codecs own any positional mapping from their on-disk representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDomain {
    Architecture,
    AirConditioning,
    BuildingServices,
    Electrical,
    Heat,
    StructuralElements,
    Ventilation,
    Plumbing,
    Sprinkler,
    Inventory,
    FacilityManagement,
    Landscape,
    Any,
    PrefabConcrete,
    SteelStructure,
    SiteOperations,
    Cooling,
    SpecialPiping,
    Process,
    Hvac,
    Geotechnical,
    Bridge,
    Road,
    Railway,
    PortsAndWaterways,
}

impl ModelDomain {
    /// Stable identifier shared with neutral model metadata.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::AirConditioning => "air_conditioning",
            Self::BuildingServices => "building_services",
            Self::Electrical => "electrical",
            Self::Heat => "heat",
            Self::StructuralElements => "structural_elements",
            Self::Ventilation => "ventilation",
            Self::Plumbing => "plumbing",
            Self::Sprinkler => "sprinkler",
            Self::Inventory => "inventory",
            Self::FacilityManagement => "facility_management",
            Self::Landscape => "landscape",
            Self::Any => "any",
            Self::PrefabConcrete => "prefab_concrete",
            Self::SteelStructure => "steel_structure",
            Self::SiteOperations => "site_operations",
            Self::Cooling => "cooling",
            Self::SpecialPiping => "special_piping",
            Self::Process => "process",
            Self::Hvac => "hvac",
            Self::Geotechnical => "geotechnical",
            Self::Bridge => "bridge",
            Self::Road => "road",
            Self::Railway => "railway",
            Self::PortsAndWaterways => "ports_and_waterways",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "architecture" => Self::Architecture,
            "air_conditioning" => Self::AirConditioning,
            "building_services" => Self::BuildingServices,
            "electrical" => Self::Electrical,
            "heat" => Self::Heat,
            "structural_elements" => Self::StructuralElements,
            "ventilation" => Self::Ventilation,
            "plumbing" => Self::Plumbing,
            "sprinkler" => Self::Sprinkler,
            "inventory" => Self::Inventory,
            "facility_management" => Self::FacilityManagement,
            "landscape" => Self::Landscape,
            "any" => Self::Any,
            "prefab_concrete" => Self::PrefabConcrete,
            "steel_structure" => Self::SteelStructure,
            "site_operations" => Self::SiteOperations,
            "cooling" => Self::Cooling,
            "special_piping" => Self::SpecialPiping,
            "process" => Self::Process,
            "hvac" => Self::Hvac,
            "geotechnical" => Self::Geotechnical,
            "bridge" => Self::Bridge,
            "road" => Self::Road,
            "railway" => Self::Railway,
            "ports_and_waterways" => Self::PortsAndWaterways,
            _ => return None,
        })
    }
}

/// Neutral relationship traversal supported by the migrated runtime scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "relation", rename_all = "snake_case")]
pub enum ElementRelation {
    SpaceBoundary {
        forward: bool,
        #[serde(default)]
        either: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_scope: Option<Box<ElementScopeSpec>>,
    },
    /// Native `SDecomposes` aggregation relation. Recursive traversal follows
    /// the same directed edge repeatedly and is cycle-safe at runtime.
    Decomposition {
        forward: bool,
        #[serde(default)]
        either: bool,
        #[serde(default)]
        recurse: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        related_scope: Option<Box<ElementScopeSpec>>,
    },
}
