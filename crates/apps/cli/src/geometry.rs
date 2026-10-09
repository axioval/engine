//! IFC body geometry as Axiolid meshes, registered into a check's session.
//!
//! This is host composition, not an adapter: `axioval-ifc` must not know
//! Axiolid and `axioval-axiolid` must not know IFC, so the CLI reads the same
//! model bytes a second time, meshes each product with `ifc-geometry`, and
//! hands the meshes to the geometry services under the object identities the
//! IFC session already uses.
//!
//! With several models, every source is meshed into one geometry set under
//! its own source-qualified identities, and every service is bound to all
//! the session's snapshots. A clash between objects of two files is then an
//! ordinary pair of the set.
//!
//! Geometry is never re-aligned: each source is meshed in its own model
//! coordinates, so the set is one frame only for sources that share the
//! first source's coordinate system (`compare_coordinate_systems` with the
//! default tolerances: the same world frame, and the same map conversion or
//! none on either side). A source that does not, or whose coordinate system
//! cannot be read, contributes its physical objects as unmeasured with the
//! reason, so every geometric check touching them is not evaluated rather
//! than measured in the wrong place.
//!
//! Every object ends up in exactly one of three states, because the geometry
//! services treat them differently:
//!
//! - **meshed**, either exactly (every face planar, so the mesh is the shape)
//!   or as a tessellation within the deviation the compiler certifies;
//! - **no body**: it occupies no material (a storey, a zone, an opening);
//! - **unmeasured**: it is physical but could not be meshed. Measurements it
//!   could affect refuse rather than act as if it were not there.
//!
//! A group (a zone) has no body, but its plan footprint is the union of its
//! members', so the bridge also declares every group's membership.
//!
//! A physical product with no body of its own that is decomposed into parts
//! (`IfcRelAggregates`, at any depth: a stair into flights and landings, a
//! roof into slabs, a wall into layers) is measured as the union of its
//! parts' bodies (`AxiolidGeometry::compose`): exact when every part is,
//! tessellated within the largest deviation of its parts otherwise, with an
//! exact body where every part has one. If any part is unmeasured, the whole
//! is too, with a reason naming the first such part. A product with no body
//! and no parts stays unmeasured: as `no shape representation` when it has
//! no representation at all, which is model data its author can fix and is
//! also reported once per object as the integrity warning
//! [`NO_SHAPE_REPRESENTATION`], or as `no body representation; it has …`
//! naming the identifiers of the representations it has, none of which the
//! bridge measures as a body. The whole
//! keeps its own identity, and a whole and its own parts share material, so
//! no pairwise rule ever pairs them (`ProximityService::shares_body`).
//!
//! With [`Options::exact_boundaries`], a body whose lowered graph has an
//! exact construction (a rigidly placed extrusion, revolution or swept disk,
//! one less its extruded openings or clipped by half-spaces, or several
//! such items; see
//! `axioval::axiolid::exact_boundary`) also gets its exact boundary
//! registered beside the mesh, built from the same graph the mesh is compiled
//! from, so placement and mirroring are the mesh's own. The proximity service
//! then certifies distances between curved bodies the chord deviation would
//! leave open. A boundary is registered only when its extent agrees with the
//! mesh's within the chord deviation, and only when some tessellated body
//! has one: two exact meshes are never certified, so boundaries of planar
//! bodies alone would be built for nothing.
//!
//! A product on an IFC4X3 `IfcLinearPlacement` is placed where its linear
//! expression puts it: every lowering derives the frame from the basis
//! curve through `ifc-geometry`'s evaluator-taking entry points
//! (`LoweringSession::with_curve_evaluator`,
//! `product_world_transform_with_evaluator`, openbimrs/ifc#353) with the
//! Axiolid reference evaluator, in IFC4.3's (tangent, left, up) frame
//! (#355). A cached `CartesianPosition` is checked against the derived one
//! (`CachedPositionPolicy::Verify`, #354): farther apart than the model's
//! tolerance leaves the product unmeasured with both positions named. An
//! `IfcParameterValue` along an alignment is refused by name (#347). Where
//! the derivation would ignore a frame the file states (a `PlacementRelTo`
//! or a basis curve placed off the identity, openbimrs/ifc#357), the
//! product is unmeasured with that reason ([`Linear`]), never misplaced.
//!
//! Relationships derived from geometry (`axioval:derived.*`) need to know
//! which objects are spaces and which are doors, windows or openings. An
//! opening occupies no material, so its void is meshed separately and handed
//! to the derivation alone.
//!
//! A host is meshed net of its openings (`lower_product_net_with`). Since
//! `ifc-geometry` 0.13 (openbimrs/ifc#388) the openings are subtracted in
//! the host's own frame and one `Instance` with the host's world transform
//! sits above the result, so a flush opening keeps the coincidence its file
//! states however far out a georeferenced site lies; the mesh and the exact
//! boundary walk through that `Instance` like any placement. In an
//! IFC4 or IFC4X3 file, an `IfcOpeningElement` whose every representation is
//! `Reference` is taken as already applied (`ReferenceOnlyOpenings::
//! TakeAsApplied`, openbimrs/ifc#351): IFC4 states that such a
//! representation "is not subtracted, it is provided in addition to the hole
//! in the Body shape representation of the voided element", as Reference
//! View exports author it. The host is measured from its `Body` as authored,
//! its evidence names the openings (`;applied-openings:`), and the result
//! lists each with the reason. Such an opening's void is its `Reference`
//! solid. An opening with no representation, or with any other
//! representation, is refused as before and leaves its host unmeasured; an
//! IFC2X3 file states no such reading, so none is taken there.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use axiolid_contracts::{ExecutionOptions, GeomError};
use axiolid_core::{BooleanOperator, Tolerance};
use axiolid_curve::{Curve2, Curve3};
use axiolid_evaluate::ReferenceCurveEvaluator;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_compile::{DeviationBound, DeviationReport, ReferenceMeshCompiler};
use axiolid_mesh_compile_contract::MeshCompiler;
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation, SurfaceRelation};
use axiolid_primitive::Primitive;
use axiolid_profile::{Profile, SectionProfile};
use axiolid_surface::Surface;
use axioval::axiolid::{
    AxiolidBoundaryCoverageService, AxiolidContactService, AxiolidDerivedRelationshipService,
    AxiolidEnvelopeMembershipService, AxiolidFacadeAreaService, AxiolidFreeSpaceService,
    AxiolidGeometry, AxiolidGuardService, AxiolidLinearQuantityService,
    AxiolidMetricRoutingService, AxiolidPlanAreaService, AxiolidPlanSpanService,
    AxiolidProximityService, AxiolidSightService, AxiolidSpaceService, AxiolidTriangleCountService,
    AxiolidVerticalExtentService, AxiolidWalkabilityService, AxiolidWalkingSurfaceService,
    ExactBoundary,
};
use axioval::engine::{
    AlignmentServiceHandle, BodyContainment, BoundaryCoverageServiceHandle, ContactServiceHandle,
    CoordinateSystemServiceHandle, DerivedRelationshipServiceHandle, EnvelopeMembershipError,
    EnvelopeMembershipEvidence, EnvelopeMembershipRequest, EnvelopeMembershipService,
    EnvelopeMembershipServiceHandle, EvidenceSession, FacadeAreaServiceHandle,
    FreeSpaceServiceHandle, GuardServiceHandle, LinearQuantityServiceHandle,
    MetricRoutingServiceHandle, PlanAreaServiceHandle, PlanSpanServiceHandle, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, ProximityRequest, ProximityService,
    ProximityServiceHandle, RelationshipEdgesRequest, RelationshipQuery,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, SemanticRelationship,
    SightServiceHandle, SourceSnapshot, SpaceServiceHandle, TraversalDirection,
    TriangleCountServiceHandle, TypeHierarchyServiceHandle, VerticalExtentServiceHandle,
    WalkabilityServiceHandle, WalkingSurfaceServiceHandle,
};
use axioval::ir::{ObjectId, PropertyValue, Report, SourceId};
use axioval::rules::{CoordinateTolerance, compare_coordinate_systems};
use axioval::{bcf, bcf_snapshot};
use ifc_geometry::constraint::local::PlacementResolver;
use ifc_geometry::lower::{
    AppliedReason, LoweringSession, NetOptions, ReferenceOnlyOpenings, lower_connection_surface,
    lower_product_net_with, lower_product_representation, lower_representation_item,
};
use ifc_geometry::{CachedPositionPolicy, GeometryError, RepresentationPurpose, Transform};
use ifc_model::{EntityId, Model};
use ifc_spatial::relation::boundary::{ConnectionGeometryAnomaly, SpaceBoundary};
use ifc_spatial::{SpatialAnomaly, SpatialKind, SpatialTree};
use std::sync::{Arc, OnceLock};

mod alignment;
mod authored_faces;
pub mod timings;

/// Linear tolerance handed to the mesh compiler: with no explicit chord
/// budget, its chord budget. Each tessellated mesh is declared with the
/// deviation the compiler certifies for it (`compile_mesh_with_deviation`,
/// axiolid/kernel#232): this budget where its construction proves it, the
/// bound it computed otherwise, and none at all (the body unmeasured)
/// where it has no bound.
const TOLERANCE: Tolerance = Tolerance::MILLIMETRE;

/// The mesh compiler `ifc-geometry` lowers for, whose deviation reports
/// are its own claim about its meshes.
type Compiler = ReferenceMeshCompiler<BoolmeshBoolean>;

/// Graph nodes the planarity check visits before calling a product curved.
const NODE_BUDGET: usize = 100_000;

/// Entity types that occupy no material. Names from both IFC2X3 and IFC4;
/// a name a release does not declare simply matches nothing.
const NO_BODY: &[&str] = &[
    "IfcSpatialElement",
    "IfcSpatialStructureElement",
    "IfcFeatureElementSubtraction",
    "IfcVirtualElement",
    "IfcAnnotation",
    "IfcGrid",
    "IfcPort",
    "IfcStructuralItem",
    "IfcStructuralActivity",
];

/// What the bridge did, for the host to report.
#[derive(Debug, Default)]
pub struct GeometryReport {
    pub exact: usize,
    pub tessellated: usize,
    pub no_body: usize,
    /// Meshed objects registered with their exact boundary as well.
    pub exact_boundaries: usize,
    /// Wholes with no body of their own measured as the union of their
    /// parts; each is counted as exact or tessellated as well.
    pub composed: usize,
    /// Physical objects that could not be meshed, with the reason.
    pub unmeasured: Vec<(ObjectId, String)>,
    /// Facts about the model data found while meshing, once per object:
    /// every physical product with no shape representation and no parts
    /// ([`NO_SHAPE_REPRESENTATION`]), every product unmeasured for a
    /// face whose boundary crosses or runs back along itself
    /// ([`SELF_INTERSECTING_FACE`]); every product whose openings remove its
    /// whole body ([`VOIDED_BODY`]); and every measured product whose body
    /// is open because its faces, as authored, leave edges bounding one
    /// face only ([`OPEN_SURFACE`]). Their measurements that need what is
    /// missing stay not evaluated.
    pub model_data: Vec<ModelData>,
    /// Openings taken as already applied to a measured host's `Body`:
    /// host, opening and the reason, in identity order.
    pub applied_openings: Vec<(ObjectId, ObjectId, String)>,
    /// Every opening voiding a whole measured through its parts that was
    /// decided by geometry: whole, opening and the parts it was subtracted
    /// from (none where they are already cut by it, or it misses them), in
    /// the order decided.
    pub whole_openings: Vec<(ObjectId, ObjectId, Vec<ObjectId>)>,
    /// Every meshed object's triangles, kept only when asked for, to draw
    /// BCF snapshots from.
    pub meshes: BTreeMap<ObjectId, bcf_snapshot::Mesh>,
}

/// The integrity code of a physical product with no shape representation
/// and no parts: it has no geometry at all, so it is unmeasured, never
/// measured as empty.
pub const NO_SHAPE_REPRESENTATION: &str = "shape.no-representation";

/// The integrity code of a product whose body has a face the mesh compiler
/// refuses because the face's boundary, as written, crosses itself or runs
/// back along itself: that boundary bounds no region, so no surface fills
/// it (#298).
pub const SELF_INTERSECTING_FACE: &str = "shape.self-intersecting-face";

/// The integrity code of a product whose openings remove its whole body:
/// its `Body` meshes to a solid, and nothing is left once the openings
/// voiding it are subtracted, as when a frame is voided by the opening
/// meant for the wall around it.
pub const VOIDED_BODY: &str = "shape.voided-body";

/// Why a mesh is refused when compilation leaves no triangle.
const NO_TRIANGLES: &str = "mesh compilation produced no triangles";

/// The reason a product is unmeasured when its openings remove its whole
/// body, `count` of them subtracted.
fn voided_body_reason(count: usize) -> String {
    format!(
        "its openings remove its whole body: nothing is left once the {count} opening(s) \
         voiding it are subtracted"
    )
}

/// Whether `reason` is [`voided_body_reason`]'s.
fn voided_body(reason: &str) -> bool {
    reason
        .strip_prefix("its openings remove its whole body: nothing is left once the ")
        .and_then(|rest| rest.strip_suffix(" opening(s) voiding it are subtracted"))
        .is_some_and(|count| count.parse::<usize>().is_ok())
}
/// The integrity code of a measured product whose mesh is no closed solid
/// because its `Body` items' faces, as authored, leave edges that bound one
/// face only (an `IfcOpenShell`, a face set stated not closed, or a shell
/// whose faces do not meet): its body is an open surface, which bounds no
/// inside. Measurements that need one (a clash between two such bodies, the
/// room a body takes up above a floor) stay not evaluated (#311).
pub const OPEN_SURFACE: &str = "shape.open-surface";

/// A fact about the model data the bridge found, for the host to report as
/// an integrity warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelData {
    /// The integrity code, such as [`NO_SHAPE_REPRESENTATION`].
    pub code: &'static str,
    pub message: String,
    /// `ifc:<fingerprint>:<detail>`, as the IFC adapter locates its own.
    pub locator: String,
}

/// Why a product with no `Body` of its own is unmeasured when it has no
/// parts either.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Bodiless {
    /// No representation at all (`Representation` is `$`, or lists none):
    /// model data.
    Shapeless,
    /// Representations none of which is measured as a body, by identifier
    /// (`unidentified` for one stating none), or none when they cannot be
    /// read.
    Shapes(Vec<String>),
}

impl Bodiless {
    /// Reads `product`'s representations.
    fn of(model: &Model, product: EntityId) -> Self {
        let Some(entity) = model.get(product) else {
            return Self::Shapes(Vec::new());
        };
        let Some(shape) = ifc_geometry::Slots::new(product, entity).opt_ref(PRODUCT_REPRESENTATION)
        else {
            return Self::Shapeless;
        };
        let Some(representations) = model.get(shape).and_then(|entity| {
            ifc_geometry::ProductShape::new(shape, entity)
                .representations()
                .ok()
        }) else {
            return Self::Shapes(Vec::new());
        };
        if representations.is_empty() {
            return Self::Shapeless;
        }
        let mut identifiers: Vec<String> = representations
            .into_iter()
            .map(|id| {
                model
                    .get(id)
                    .and_then(|entity| ifc_geometry::Representation::new(id, entity).identifier())
                    .filter(|identifier| !identifier.trim().is_empty())
                    .unwrap_or_else(|| "unidentified".to_owned())
            })
            .collect();
        identifiers.sort();
        identifiers.dedup();
        Self::Shapes(identifiers)
    }

    /// The unmeasured reason.
    fn reason(&self) -> String {
        match self {
            Self::Shapeless => "no shape representation".to_owned(),
            Self::Shapes(identifiers) if identifiers.is_empty() => {
                "no body representation".to_owned()
            }
            Self::Shapes(identifiers) => {
                format!("no body representation; it has {}", identifiers.join(", "))
            }
        }
    }
}

/// How [`attach`] meshes the model.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Keep every meshed object's triangles in [`GeometryReport::meshes`],
    /// for BCF snapshots.
    pub keep_meshes: bool,
    /// Build and register exact boundaries where the construction is exact.
    pub exact_boundaries: bool,
    /// Print each phase's time on stderr (`--timings`).
    pub timings: bool,
}

impl Options {
    /// Meshes only, keeping them when `keep` is set.
    pub fn meshes(keep: bool) -> Self {
        Self {
            keep_meshes: keep,
            exact_boundaries: false,
            timings: false,
        }
    }

    /// The same, printing each phase's time on stderr when `on` is set.
    #[must_use]
    pub fn with_timings(self, on: bool) -> Self {
        Self {
            timings: on,
            ..self
        }
    }

    /// The same, with exact boundaries when `exact` is set.
    #[must_use]
    pub fn with_exact_boundaries(self, exact: bool) -> Self {
        Self {
            exact_boundaries: exact,
            ..self
        }
    }
}

/// Each source's model bytes, keyed by the source the session imported them as.
pub type ModelBytes = BTreeMap<SourceId, Vec<u8>>;

/// One parsed model and its unit scale, per source.
struct Parsed {
    model: Model,
    units: ifc_geometry::units::UnitScale,
    /// How the host's openings are subtracted: `Reference`-only openings
    /// taken as applied in an IFC4 or IFC4X3 file ([`net_options`]).
    net: NetOptions,
    /// Linear placements whose derivation would ignore a stated frame.
    linear: Linear,
}

/// The curve evaluator every `IfcLinearPlacement` is derived with: the
/// Axiolid reference evaluator, global `+Z` up.
static EVALUATOR: ReferenceCurveEvaluator = ReferenceCurveEvaluator::new();

/// What a cached `CartesianPosition` is: checked against the position the
/// linear expression derives, and a mismatch beyond the model's tolerance
/// refused (`GeometryError::CachedPlacementMismatch`), never placed.
const CACHED_POSITIONS: CachedPositionPolicy = CachedPositionPolicy::Verify;

/// A lowering session deriving linear placements through [`EVALUATOR`].
fn session<'a>(model: &'a Model, units: &'a ifc_geometry::units::UnitScale) -> LoweringSession<'a> {
    LoweringSession::new(model, units)
        .with_curve_evaluator(&EVALUATOR)
        .with_cached_position_policy(CACHED_POSITIONS)
}

/// `product`'s world transform in metres, a linear placement derived
/// through [`EVALUATOR`].
fn world_transform(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    product: EntityId,
) -> Result<Transform, GeometryError> {
    ifc_geometry::product_world_transform_with_evaluator(
        model,
        units,
        product,
        &EVALUATOR,
        CACHED_POSITIONS,
    )
}

/// `IfcObjectPlacement.PlacementRelTo`, `IfcLinearPlacement.RelativePlacement`,
/// `IfcAxis2PlacementLinear.Location` and
/// `IfcPointByDistanceExpression.BasisCurve`.
const PLACEMENT_REL_TO: usize = 0;
const RELATIVE_PLACEMENT: usize = 1;
const LINEAR_LOCATION: usize = 0;
const BASIS_CURVE: usize = 4;

/// `IfcProduct.ObjectPlacement`.
const OBJECT_PLACEMENT: usize = 5;

/// IFC4X3 positioning elements whose representations hold alignment curves.
const ALIGNMENTS: &[&str] = &[
    "IFCALIGNMENT",
    "IFCALIGNMENTHORIZONTAL",
    "IFCALIGNMENTVERTICAL",
    "IFCALIGNMENTCANT",
    "IFCALIGNMENTSEGMENT",
    "IFCLINEARPOSITIONINGELEMENT",
];

/// How far, in metres and in each axis component, a frame may lie from the
/// identity and still be the identity: floating-point rounding.
const IDENTITY: f64 = 1e-9;

/// The `IfcLinearPlacement`s of one model whose derivation would ignore a
/// frame the file states, each with the reason.
///
/// `ifc-geometry` 0.10 evaluates the basis curve in world coordinates and
/// ignores the placement's `PlacementRelTo` (openbimrs/ifc#357). Both are
/// harmless at the identity, so only these refuse: a `PlacementRelTo` whose
/// world frame is not the identity or cannot be resolved; a basis curve
/// held by a representation of a product placed off the identity (the
/// representation context's world coordinate system above the product's
/// placement); and a basis curve held by no representation (an alignment's
/// curve nested under another one) while some alignment of the model is
/// placed off the identity, since its frame is then not known. A product
/// on such a placement, or voided by an opening on one, is unmeasured with
/// the reason, never placed where the derivation alone puts it.
#[derive(Debug, Default)]
struct Linear {
    refused: BTreeMap<EntityId, String>,
}

impl Linear {
    /// Every linear placement of `model` refused, with the reason.
    fn scan(model: &Model, units: &ifc_geometry::units::UnitScale) -> Self {
        let placements = model.ids_of_type("IFCLINEARPLACEMENT");
        if placements.is_empty() {
            return Self::default();
        }
        let curves: BTreeMap<EntityId, EntityId> = placements
            .iter()
            .filter_map(|&placement| Some((placement, basis_curve(model, placement)?)))
            .collect();
        let owners = curve_owners(model, &curves.values().copied().collect());
        let refused = placements
            .iter()
            .filter_map(|&placement| {
                let reason = relative_to(model, units, placement).err().or_else(|| {
                    let curve = curves.get(&placement)?;
                    curve_frame(model, units, *curve, owners.get(curve)).err()
                })?;
                Some((
                    placement,
                    format!(
                        "{placement} (IFCLINEARPLACEMENT): {reason}, which deriving the \
                         placement does not compose (openbimrs/ifc#357), so the product is not \
                         placed"
                    ),
                ))
            })
            .collect();
        Self { refused }
    }

    /// Why `product`'s own placement is refused, `None` when it is not.
    fn own_refusal(&self, model: &Model, product: EntityId) -> Option<String> {
        let placement =
            ifc_geometry::Slots::new(product, model.get(product)?).opt_ref(OBJECT_PLACEMENT)?;
        self.refused.get(&placement).cloned()
    }

    /// Why `product` cannot be placed: its own placement, or that of an
    /// opening voiding it, is refused. `None` otherwise.
    fn refusal(&self, model: &Model, product: EntityId) -> Option<String> {
        if self.refused.is_empty() {
            return None;
        }
        std::iter::once(product)
            .chain(ifc_geometry::openings_of(model, product))
            .find_map(|object| {
                let placement = ifc_geometry::Slots::new(object, model.get(object)?)
                    .opt_ref(OBJECT_PLACEMENT)?;
                let reason = self.refused.get(&placement)?;
                Some(if object == product {
                    reason.clone()
                } else {
                    format!("its opening {object} is not placed: {reason}")
                })
            })
    }
}

/// The basis curve of a linear placement's `IfcPointByDistanceExpression`,
/// if it states one; anything else is the lowering's to refuse.
fn basis_curve(model: &Model, placement: EntityId) -> Option<EntityId> {
    let relative =
        ifc_geometry::Slots::new(placement, model.get(placement)?).opt_ref(RELATIVE_PLACEMENT)?;
    let location =
        ifc_geometry::Slots::new(relative, model.get(relative)?).opt_ref(LINEAR_LOCATION)?;
    let point = model.get(location)?;
    point
        .is_type("IFCPOINTBYDISTANCEEXPRESSION")
        .then(|| ifc_geometry::Slots::new(location, point).opt_ref(BASIS_CURVE))
        .flatten()
}

/// Whether a linear placement's `PlacementRelTo`, if any, is the identity.
fn relative_to(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    placement: EntityId,
) -> Result<(), String> {
    let entity = model
        .get(placement)
        .ok_or_else(|| format!("{placement} does not exist"))?;
    let Some(relative) = ifc_geometry::Slots::new(placement, entity).opt_ref(PLACEMENT_REL_TO)
    else {
        return Ok(());
    };
    let frame = PlacementResolver::new()
        .world_transform(model, relative)
        .map_err(|error| format!("its PlacementRelTo {relative} cannot be resolved ({error})"))?
        .to_metres(units);
    if frame.is_identity(IDENTITY) {
        Ok(())
    } else {
        Err(format!(
            "its PlacementRelTo {relative} places it off the identity"
        ))
    }
}

/// For each of `curves`, the products and representations holding it as
/// an item.
fn curve_owners(
    model: &Model,
    curves: &BTreeSet<EntityId>,
) -> BTreeMap<EntityId, Vec<(EntityId, EntityId)>> {
    let mut holding: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    for type_name in ["IFCSHAPEREPRESENTATION", "IFCTOPOLOGYREPRESENTATION"] {
        for (id, entity) in model.of_type(type_name) {
            let items = ifc_geometry::Representation::new(id, entity)
                .items()
                .unwrap_or_default();
            let held: Vec<EntityId> = items
                .into_iter()
                .filter(|item| curves.contains(item))
                .collect();
            if !held.is_empty() {
                holding.insert(id, held);
            }
        }
    }
    let mut owners: BTreeMap<EntityId, Vec<(EntityId, EntityId)>> = BTreeMap::new();
    if holding.is_empty() {
        return owners;
    }
    for product in ifc_geometry::geometric_products(model) {
        let Some(shape) = model.get(product).and_then(|entity| {
            ifc_geometry::Slots::new(product, entity).opt_ref(PRODUCT_REPRESENTATION)
        }) else {
            continue;
        };
        let Some(representations) = model.get(shape).and_then(|entity| {
            ifc_geometry::ProductShape::new(shape, entity)
                .representations()
                .ok()
        }) else {
            continue;
        };
        for representation in representations {
            for curve in holding.get(&representation).into_iter().flatten() {
                owners
                    .entry(*curve)
                    .or_default()
                    .push((product, representation));
            }
        }
    }
    owners
}

/// Whether `curve` lies in world coordinates: every product holding it is
/// placed at the identity, or, held by none, every alignment of the model.
fn curve_frame(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    curve: EntityId,
    owners: Option<&Vec<(EntityId, EntityId)>>,
) -> Result<(), String> {
    if let Some(owners) = owners.filter(|owners| !owners.is_empty()) {
        for &(product, representation) in owners {
            let placement = ifc_geometry::product_world_transform(model, units, product)
                .map_err(|error| {
                    format!("the placement of {product}, which holds its basis curve {curve}, cannot be resolved ({error})")
                })?;
            let frame = context_frame(model, units, representation).ok_or_else(|| {
                format!("the context of {representation}, which holds its basis curve {curve}, cannot be read")
            })?;
            if !frame.compose(&placement).is_identity(IDENTITY) {
                return Err(format!(
                    "its basis curve {curve} is held by {product}, which is placed off the identity"
                ));
            }
        }
        return Ok(());
    }
    for type_name in ALIGNMENTS {
        for (alignment, entity) in model.of_type(type_name) {
            if ifc_geometry::Slots::new(alignment, entity)
                .opt_ref(OBJECT_PLACEMENT)
                .is_none()
            {
                continue;
            }
            let placed = ifc_geometry::product_world_transform(model, units, alignment)
                .is_ok_and(|placement| placement.is_identity(IDENTITY));
            if !placed {
                return Err(format!(
                    "its basis curve {curve} is held by no representation, and the alignment \
                     {alignment} is placed off the identity, so the curve's frame is not known"
                ));
            }
        }
    }
    Ok(())
}

/// The net lowering options for a file of `schema`: an
/// `IfcOpeningElement` whose every representation is `Reference` is taken
/// as already applied to its host's `Body` where the release states that
/// such a representation is not subtracted (IFC4 ADD2 TC1 and IFC4X3,
/// `IfcOpeningElement`; openbimrs/ifc#351). IFC2X3 states no such reading,
/// and an unknown release none either, so there it is refused as before.
fn net_options(schema: Option<&str>) -> NetOptions {
    let states_reference_openings = matches!(schema, Some("IFC4" | "IFC4X3"));
    NetOptions::default().with_reference_only_openings(if states_reference_openings {
        ReferenceOnlyOpenings::TakeAsApplied
    } else {
        ReferenceOnlyOpenings::Refuse
    })
}

/// Why `reason` lets an opening be taken as applied, as the result states
/// it; `None` for a reason this bridge does not know, which is refused.
fn applied_reason(reason: AppliedReason) -> Option<&'static str> {
    match reason {
        AppliedReason::ReferenceRepresentationOnly => Some(
            "every representation of the opening is 'Reference', which IFC4 states is \
             provided in addition to the hole in the host's Body, not subtracted",
        ),
        _ => None,
    }
}

/// The sources whose geometry is not in the first source's frame, with
/// the reason. Empty for a single source.
fn unaligned(
    session: &EvidenceSession,
    snapshots: &[SourceSnapshot],
) -> BTreeMap<SourceId, String> {
    let Some((first, others)) = snapshots.split_first() else {
        return BTreeMap::new();
    };
    let reference = first.source();
    let others = others.iter().map(SourceSnapshot::source);
    let Some(service) = session.service::<CoordinateSystemServiceHandle>() else {
        return others
            .map(|source| {
                (
                    source.clone(),
                    "no coordinate-system service states whether its geometry shares the frame of the other models".to_owned(),
                )
            })
            .collect();
    };
    let base = service.coordinate_system(reference);
    others
        .filter_map(|source| {
            let base = match &base {
                Ok(base) => base,
                Err(error) => {
                    return Some((
                        source.clone(),
                        format!(
                            "the coordinate system of `{reference}` cannot be read, so its frame and this model's are not known to be one: {error}"
                        ),
                    ));
                }
            };
            let reason = match service.coordinate_system(source) {
                Ok(system) => {
                    compare_coordinate_systems(base, &system, CoordinateTolerance::default())
                        .shares_frame()
                        .err()?
                }
                Err(error) => format!("its coordinate system cannot be read: {error}"),
            };
            Some((
                source.clone(),
                format!("not in the coordinate system of `{reference}`: {reason}"),
            ))
        })
        .collect()
}

/// Parses the model of every snapshot's source.
fn parse(
    snapshots: &[SourceSnapshot],
    models: &ModelBytes,
) -> Result<BTreeMap<SourceId, Parsed>, Box<dyn Error>> {
    let mut parsed = BTreeMap::new();
    for snapshot in snapshots {
        let source = snapshot.source();
        let bytes = models
            .get(source)
            .ok_or_else(|| format!("no model bytes for source `{source}`"))?;
        // An ifcXML source is read by the adapter's reader, with its
        // refusals, into the model its STEP form parses to.
        let model = if axioval::ifc::is_ifc_xml(bytes) {
            axioval::ifc::read_ifc_xml(bytes).map_err(|error| error.to_string())
        } else {
            axioval::ifc::read_ifc_step(bytes).map_err(|error| error.to_string())
        }
        .map_err(|error| format!("{}: {error}", source.document))?;
        let units = ifc_geometry::units::resolve(&model);
        let net = net_options(snapshot.schema());
        let linear = Linear::scan(&model, &units);
        parsed.insert(
            source.clone(),
            Parsed {
                model,
                units,
                net,
                linear,
            },
        );
    }
    Ok(parsed)
}

/// Meshes every source's model and registers geometry services for
/// `session`, bound to all its snapshots.
///
/// `models` holds each source's bytes, keyed by the source the session
/// imported them as. `options` says whether meshes are kept and exact
/// boundaries registered. Policy choices IFC does not state, such as which
/// surfaces are walkable or which spaces bound the envelope, are the rules'
/// own selections, carried in each request; the bridge declares none of them.
///
/// # Errors
///
/// Returns an error when a source has no bytes, the bytes do not parse (the
/// session already parsed them, so this means they changed) or a service
/// cannot be registered.
// One pass over the objects, each ending in exactly one state; splitting
// it would scatter that invariant.
#[allow(clippy::too_many_lines)]
pub fn attach(
    session: EvidenceSession,
    models: &ModelBytes,
    options: Options,
) -> Result<(EvidenceSession, GeometryReport), Box<dyn Error>> {
    let snapshots: Vec<SourceSnapshot> = session.snapshots().cloned().collect();
    let Some(first) = snapshots.first() else {
        return Err("geometry needs a session over at least one source".into());
    };
    // Set-level evidence (free space, guard, envelope, storey residuals) is
    // cited under one source; evidence about one object cites its own.
    let source = first.source().clone();
    let mut clock = timings::Stopwatch::new(options.timings);
    let parsed = parse(&snapshots, models)?;
    clock.lap("geometry: parse");
    let hierarchy = session
        .service::<TypeHierarchyServiceHandle>()
        .ok_or("the session has no type hierarchy to classify objects with")?
        .clone();
    let is_a = |id: &ObjectId, ancestor: &str, kinds: &BTreeMap<ObjectId, String>| {
        kinds
            .get(id)
            .is_some_and(|kind| hierarchy.is_a(&id.source, kind, ancestor).unwrap_or(false))
    };
    let kinds: BTreeMap<ObjectId, String> = session
        .project()
        .objects()
        .map(|object| (object.id.clone(), object.kind().to_owned()))
        .collect();
    let is_a = |id: &ObjectId, ancestor: &str| is_a(id, ancestor, &kinds);

    let unaligned = unaligned(&session, &snapshots);
    let backend = ifc_geometry::compile::default_backend();
    let mut geometry = AxiolidGeometry::new();
    let mut report = GeometryReport::default();
    let mut voids: Vec<(ObjectId, Void)> = Vec::new();
    // Each built boundary, and whether its object's mesh is tessellated.
    let mut boundaries: Vec<(ObjectId, ExactBoundary, bool)> = Vec::new();
    // Physical products with no body of their own, measured through their
    // parts once every part is.
    let mut wholes: Vec<ObjectId> = Vec::new();
    // Why each whole is unmeasured if it has no parts.
    let mut unbodied: BTreeMap<ObjectId, Bodiless> = BTreeMap::new();
    // The box each whole's `Box` representation states, if any.
    let mut stated: BTreeMap<ObjectId, ([f64; 3], [f64; 3])> = BTreeMap::new();
    // Openings some measured host's `Body` already carries.
    let mut applied_openings: BTreeSet<ObjectId> = BTreeSet::new();
    // Bodies meshed, registered once every whole's openings are subtracted
    // from the parts they cut.
    let mut bodies: BTreeMap<ObjectId, Body> = BTreeMap::new();
    // Measured products whose authored faces leave edges open, with how
    // many.
    let mut open_surfaces: Vec<(ObjectId, usize)> = Vec::new();

    for object in session.project().objects() {
        let id = object.id.clone();
        let Some(Parsed {
            model,
            units,
            net,
            linear,
        }) = parsed.get(&id.source)
        else {
            return Err(format!("no model for source `{}`", id.source).into());
        };
        let is_space = is_a(&id, "IfcSpace");
        let bodiless = !is_a(&id, "IfcProduct")
            || (!is_space && NO_BODY.iter().any(|ancestor| is_a(&id, ancestor)));
        let unaligned = unaligned.get(&id.source);
        if bodiless {
            if let (true, Some(reason)) = (is_a(&id, "IfcOpeningElement"), unaligned) {
                voids.push((id.clone(), Err(reason.clone())));
            } else if is_a(&id, "IfcOpeningElement") {
                let void = entity_id(&id)
                    .ok_or_else(|| "not a STEP instance id".to_owned())
                    .and_then(|entity| {
                        mesh(
                            &backend,
                            model,
                            units,
                            linear,
                            entity,
                            false,
                            NetOptions::default(),
                        )
                    })
                    .and_then(|meshed| {
                        meshed
                            .map(|body| (body.mesh, body.fit))
                            .ok_or_else(|| "no body representation".into())
                    });
                voids.push((id.clone(), void));
            }
            geometry = geometry.with_no_body(id);
            report.no_body += 1;
            continue;
        }
        if let Some(reason) = unaligned {
            report.unmeasured.push((id.clone(), reason.clone()));
            geometry = geometry.with_unmeasured(id, reason.clone());
            continue;
        }
        let Some(entity) = entity_id(&id) else {
            report
                .unmeasured
                .push((id.clone(), "not a STEP instance id".into()));
            geometry = geometry.with_unmeasured(id, "not a STEP instance id");
            continue;
        };
        let meshed = mesh(
            &backend,
            model,
            units,
            linear,
            entity,
            options.exact_boundaries,
            *net,
        )
        .and_then(|body| applied(&id, body));
        match meshed {
            Ok(Some(body)) => {
                if !axiolid_mesh::audit_mesh(&body.mesh, TOLERANCE).is_closed_two_manifold()
                    && let Some(open) = authored_faces::open_edges(model, entity)
                    && open > 0
                {
                    open_surfaces.push((id.clone(), open));
                }
                bodies.insert(id, body);
            }
            // A space without a body is still no material; it cannot be
            // measured itself, but it obstructs nothing.
            Ok(None) if is_space => {
                geometry = geometry.with_no_body(id);
                report.no_body += 1;
            }
            Ok(None) => {
                // Kept for the whole in case its parts cannot measure it.
                if let Some(bound) = stated_box(model, units, linear, entity) {
                    stated.insert(id.clone(), bound);
                }
                unbodied.insert(id.clone(), Bodiless::of(model, entity));
                wholes.push(id);
            }
            Err(error) => {
                report.unmeasured.push((id.clone(), error.clone()));
                geometry = geometry.with_unmeasured(id.clone(), error);
                if let Some((min, max)) = stated_box(model, units, linear, entity) {
                    geometry = geometry.with_unmeasured_bound(id, min, max);
                }
            }
        }
    }

    clock.lap("geometry: mesh");
    let relationships = session.service::<RelationshipSelectionServiceHandle>();
    let parts = decompositions(relationships, &kinds);
    let mut cuts = WholeOpenings::decide(
        &backend,
        &parsed,
        &wholes,
        parts.as_ref().ok(),
        &bodies,
        &voids,
    );
    cuts.subtract(
        &backend,
        &parsed,
        &mut bodies,
        options.exact_boundaries,
        &mut report,
    );
    for (part, (reason, bound)) in std::mem::take(&mut cuts.unmeasured_parts) {
        bodies.remove(&part);
        report.unmeasured.push((part.clone(), reason.clone()));
        geometry = geometry.with_unmeasured(part.clone(), reason);
        if let Some((min, max)) = bound {
            geometry = geometry.with_unmeasured_bound(part, min, max);
        }
    }
    for (id, body) in bodies {
        if !body.applied.is_empty() {
            let openings = body.applied.iter().map(|(opening, _)| opening.clone());
            geometry = geometry.with_applied_openings(id.clone(), openings.collect());
            for (opening, reason) in &body.applied {
                applied_openings.insert(opening.clone());
                report
                    .applied_openings
                    .push((id.clone(), opening.clone(), (*reason).to_owned()));
            }
        }
        if let Some(openings) = cuts.subtracted.get(&id) {
            geometry = geometry.with_whole_openings(id.clone(), openings.clone());
        }
        if options.keep_meshes
            && let Some(kept) = snapshot_mesh(&body.mesh)
        {
            report.meshes.insert(id.clone(), kept);
        }
        if let Some(boundary) = body.boundary {
            boundaries.push((id.clone(), boundary, body.fit != Fit::Exact));
        }
        match body.fit {
            Fit::Exact => {
                geometry = geometry.with_mesh(id, body.mesh);
                report.exact += 1;
            }
            Fit::Within(deviation) => {
                geometry = geometry.with_tessellated_mesh(id, body.mesh, deviation);
                report.tessellated += 1;
            }
        }
    }
    // The openings a whole's parts are taken to carry, by the file's word.
    for openings in cuts.applied.values() {
        applied_openings.extend(openings.iter().map(|(opening, _)| opening.clone()));
    }
    clock.lap("geometry: whole openings");
    geometry = with_boundaries(geometry, boundaries, &mut report);
    clock.lap("geometry: exact boundaries");
    report.applied_openings.sort();
    // An opening taken as applied has no `Body` to mesh its void from; its
    // `Reference` solid is the void the file authors.
    for (id, void) in &mut voids {
        if void.is_err()
            && applied_openings.contains(id)
            && let (Some(parsed), Some(entity)) = (parsed.get(&id.source), entity_id(id))
        {
            *void = reference_void(
                &backend,
                &parsed.model,
                &parsed.units,
                &parsed.linear,
                entity,
            );
        }
    }

    geometry = Composer {
        geometry,
        parts,
        wholes: wholes.iter().cloned().collect(),
        bodiless: unbodied,
        stated,
        decided: BTreeSet::new(),
        openings: cuts,
        report: &mut report,
        keep_meshes: options.keep_meshes,
    }
    .compose_all(&wholes);
    report.applied_openings.sort();
    clock.lap("geometry: compose wholes");
    report.unmeasured.sort_by(|a, b| a.0.cmp(&b.0));
    open_surfaces.sort();
    report.model_data = model_data_notes(&report.unmeasured, &open_surfaces, &snapshots, &kinds);
    for (group, members) in groups(relationships, &kinds, &is_a) {
        geometry = match members {
            Ok(members) => geometry.with_group(group, members),
            Err(reason) => geometry.with_undecided_group(group, reason),
        };
    }
    clock.lap("geometry: groups");
    let envelope = envelope_service(&session, &geometry, &source, &kinds, options.timings)?;
    let space = (
        space_service(&parsed, &geometry, &source, &kinds, &is_a),
        linear_service(&geometry, &voids),
        boundary_service(&backend, &parsed, &geometry, &kinds, &is_a),
        plan_area_service(&geometry, &source, &voids),
    );
    clock.lap("geometry: space services");
    let routes = route_services(&geometry, &source, &kinds, &is_a, &voids);
    clock.lap("geometry: route services");
    let derived = derived_service(&geometry, &parsed, &kinds, &is_a, voids);
    clock.lap("geometry: derived relationships");
    // Sections cut, and envelopes are swept past, the same bodies every
    // other service measures.
    let alignments = alignment::alignment_service(
        &parsed,
        &kinds.keys().cloned().collect::<Vec<_>>(),
        geometry.clone(),
    );
    let facade = facade_service(&geometry, &kinds, &is_a);
    clock.lap("geometry: alignments and facades");
    let session = register(session, &snapshots, geometry, space, envelope, routes)?
        .with_host_service(FacadeAreaServiceHandle::new(Arc::new(facade)), &snapshots)?
        .with_derived_relationships(
            DerivedRelationshipServiceHandle::new(Arc::new(derived)),
            &snapshots,
        )?
        // Stations, offsets and heights of reference points along the
        // sources' alignments.
        .with_host_service(
            AlignmentServiceHandle::new(Arc::new(alignments)),
            &snapshots,
        )?;
    clock.lap("geometry: register services");
    Ok((session, report))
}

/// Every whole's parts, from the session's `IfcRelAggregates` edges, or
/// why they cannot be read.
fn decompositions(
    relationships: Option<&RelationshipSelectionServiceHandle>,
    kinds: &BTreeMap<ObjectId, String>,
) -> Result<BTreeMap<ObjectId, Vec<ObjectId>>, String> {
    let relationships = relationships
        .ok_or("the session has no relationship service to read decompositions with")?;
    let request = RelationshipEdgesRequest::try_new(
        kinds.keys().cloned().collect(),
        SemanticRelationship::try_new("IfcRelAggregates").map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let listing = relationships
        .edges(&request)
        .map_err(|error| error.to_string())?;
    let mut parts: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    for edge in listing.edges() {
        parts
            .entry(edge.relating.clone())
            .or_default()
            .push(edge.related.clone());
    }
    Ok(parts)
}

/// Measures every physical product with no body of its own through its
/// parts, innermost wholes first.
struct Composer<'r> {
    geometry: AxiolidGeometry,
    /// Every whole's parts, or why decompositions cannot be read.
    parts: Result<BTreeMap<ObjectId, Vec<ObjectId>>, String>,
    /// The products with no body of their own.
    wholes: BTreeSet<ObjectId>,
    /// Why each of them is unmeasured if it has no parts.
    bodiless: BTreeMap<ObjectId, Bodiless>,
    /// The box a whole's `Box` representation states.
    stated: BTreeMap<ObjectId, ([f64; 3], [f64; 3])>,
    /// Wholes measured or left unmeasured, and those being decided.
    decided: BTreeSet<ObjectId>,
    /// What the wholes' own openings did to their parts.
    openings: WholeOpenings,
    report: &'r mut GeometryReport,
    keep_meshes: bool,
}

impl Composer<'_> {
    fn compose_all(mut self, wholes: &[ObjectId]) -> AxiolidGeometry {
        for whole in wholes {
            self.compose(whole);
        }
        self.geometry
    }

    /// Decides `whole` after every part of it that is a whole itself. A
    /// part still being decided (a decomposition cycle) has no body yet, so
    /// the compose refuses it by name.
    fn compose(&mut self, whole: &ObjectId) {
        if !self.decided.insert(whole.clone()) {
            return;
        }
        let bodiless = self
            .bodiless
            .get(whole)
            .map_or_else(|| "no body representation".to_owned(), Bodiless::reason);
        let parts = match &self.parts {
            Ok(decompositions) => decompositions.get(whole).cloned().unwrap_or_default(),
            Err(error) => {
                let reason = format!(
                    "{bodiless}, and whether it decomposes into parts that carry its body \
                     cannot be read: {error}"
                );
                self.unmeasured(whole, reason, None);
                return;
            }
        };
        if parts.is_empty() {
            self.unmeasured(whole, bodiless, None);
            return;
        }
        for part in &parts {
            if self.wholes.contains(part) {
                self.compose(part);
            }
        }
        if let Some(reason) = self.openings.refused.get(whole).cloned() {
            // Cutting only takes material away, so its parts' box still
            // bounds it.
            let bound = self.geometry.parts_bound(&parts);
            self.unmeasured(
                whole,
                format!("no body representation of its own, and {reason}"),
                bound,
            );
            return;
        }
        match self.geometry.compose(&parts) {
            Ok(body) => {
                if body.is_exact() {
                    self.report.exact += 1;
                } else {
                    self.report.tessellated += 1;
                }
                if body.has_exact_body() {
                    self.report.exact_boundaries += 1;
                }
                if self.keep_meshes
                    && let Some(kept) = snapshot_mesh(body.mesh())
                {
                    self.report.meshes.insert(whole.clone(), kept);
                }
                self.report.composed += 1;
                let mut geometry =
                    std::mem::take(&mut self.geometry).with_composed_body(whole.clone(), body);
                if let Some(applied) = self.openings.applied.get(whole) {
                    let openings = applied.iter().map(|(opening, _)| opening.clone());
                    geometry = geometry.with_applied_openings(whole.clone(), openings.collect());
                    for (opening, reason) in applied {
                        self.report.applied_openings.push((
                            whole.clone(),
                            opening.clone(),
                            (*reason).to_owned(),
                        ));
                    }
                }
                if let Some(openings) = self.openings.subtracted.get(whole) {
                    geometry = geometry.with_whole_openings(whole.clone(), openings.clone());
                }
                self.geometry = geometry;
            }
            Err(error) => {
                // Its body is the union of its parts', so the box around
                // their bodies and declared boxes bounds it.
                let bound = self.geometry.parts_bound(&parts);
                self.unmeasured(
                    whole,
                    format!("no body representation of its own, and {error}"),
                    bound,
                );
            }
        }
    }

    /// Leaves `whole` unmeasured, bounded by its parts' box when they give
    /// one, else by the box its file states, else nowhere.
    fn unmeasured(
        &mut self,
        whole: &ObjectId,
        reason: String,
        parts: Option<([f64; 3], [f64; 3])>,
    ) {
        let mut geometry = std::mem::take(&mut self.geometry);
        geometry = geometry.with_unmeasured(whole.clone(), reason.clone());
        if let Some((min, max)) = parts.or_else(|| self.stated.get(whole).copied()) {
            geometry = geometry.with_unmeasured_bound(whole.clone(), min, max);
        }
        self.geometry = geometry;
        self.report.unmeasured.push((whole.clone(), reason));
    }
}

/// A shared volume no larger than this, in cubic metres, is the rounding of
/// two bodies that only touch, as the derived `intersects` reads it.
const TOUCHING_VOLUME: f64 = 1e-12;

/// Surfaces closer than this, in metres, touch.
const ON_SURFACE: f64 = 1e-9;

/// A box `(min, max)` in world metres.
type Extent = ([f64; 3], [f64; 3]);

/// What the openings voiding a whole measured through its parts do to
/// those parts (#223).
///
/// An `IfcRelVoidsElement` on a whole voids its material, which is its
/// parts'. Where the parts are already cut (a Reference View export) the
/// opening's body touches them at most; where they are uncut (a design
/// transfer export) it reaches into their material. Each opening is
/// subtracted from each part whose material it is shown to reach into
/// deeper than [`TOUCHING_DEPTH`] ([`cuts`]), and from no other: a part it
/// only touches or misses keeps its body. A part
/// for which that cannot be decided, or which lies wholly within the
/// opening, is unmeasured naming the opening, and so is every whole it is
/// a part of; a whole one of whose openings has no body to decide with is
/// unmeasured naming it. An opening the file states is already applied
/// (`Reference` only, as for hosts) is taken as applied to the whole.
#[derive(Debug, Default)]
struct WholeOpenings {
    /// The wholes' openings that cut each part, with the whole each voids.
    cut: BTreeMap<ObjectId, Vec<(ObjectId, ObjectId)>>,
    /// Parts left unmeasured by a whole's opening, with the reason and
    /// the box their uncut body gives.
    unmeasured_parts: BTreeMap<ObjectId, (String, Option<Extent>)>,
    /// Wholes left unmeasured by an opening of their own, with the reason.
    refused: BTreeMap<ObjectId, String>,
    /// Openings of a whole taken as already applied to its parts, with the
    /// reason.
    applied: BTreeMap<ObjectId, Vec<(ObjectId, &'static str)>>,
    /// The wholes' openings subtracted from each body: a part's, and each
    /// whole's that part is a piece of, at any depth.
    subtracted: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// The measured parts of each whole, at any depth.
    leaves: BTreeMap<ObjectId, BTreeSet<ObjectId>>,
    /// Each whole's opening decided by geometry, with the parts it cuts.
    decided: Vec<(ObjectId, ObjectId, Vec<ObjectId>)>,
}

impl WholeOpenings {
    /// Decides, for each whole's opening, which of its measured parts it
    /// cuts. Nothing when decompositions cannot be read: every whole is
    /// left unmeasured then.
    fn decide(
        backend: &Compiler,
        parsed: &BTreeMap<SourceId, Parsed>,
        wholes: &[ObjectId],
        parts: Option<&BTreeMap<ObjectId, Vec<ObjectId>>>,
        bodies: &BTreeMap<ObjectId, Body>,
        voids: &[(ObjectId, Void)],
    ) -> Self {
        let mut decided = Self::default();
        let Some(parts) = parts else {
            return decided;
        };
        let voids: BTreeMap<&ObjectId, &Void> = voids.iter().map(|(id, void)| (id, void)).collect();
        let wholes_set: BTreeSet<&ObjectId> = wholes.iter().collect();
        for whole in wholes {
            let mut leaves = BTreeSet::new();
            collect_leaves(whole, parts, &wholes_set, &mut BTreeSet::new(), &mut leaves);
            leaves.retain(|leaf| bodies.contains_key(leaf));
            decided.leaves.insert(whole.clone(), leaves);
        }
        for whole in wholes {
            let (Some(source), Some(entity)) = (parsed.get(&whole.source), entity_id(whole)) else {
                continue;
            };
            for opening in ifc_geometry::openings_of(&source.model, entity) {
                decided.decide_opening(backend, source, whole, opening, bodies, &voids);
                if decided.refused.contains_key(whole) {
                    break;
                }
            }
        }
        decided
    }

    fn decide_opening(
        &mut self,
        backend: &Compiler,
        source: &Parsed,
        whole: &ObjectId,
        opening: EntityId,
        bodies: &BTreeMap<ObjectId, Body>,
        voids: &BTreeMap<&ObjectId, &Void>,
    ) {
        let id = match ObjectId::new(whole.source.clone(), format!("#{}", opening.0)) {
            Ok(id) => id,
            Err(error) => {
                self.refused.insert(
                    whole.clone(),
                    format!("its opening #{} cannot be named: {error}", opening.0),
                );
                return;
            }
        };
        if source.net.reference_only_openings == ReferenceOnlyOpenings::TakeAsApplied
            && reference_only(&source.model, opening)
            && let Some(reason) = applied_reason(AppliedReason::ReferenceRepresentationOnly)
        {
            self.applied
                .entry(whole.clone())
                .or_default()
                .push((id, reason));
            return;
        }
        let void = match voids.get(&id) {
            Some(void) => (*void).clone(),
            None => mesh(
                backend,
                &source.model,
                &source.units,
                &source.linear,
                opening,
                false,
                NetOptions::default(),
            )
            .and_then(|meshed| {
                meshed
                    .map(|body| (body.mesh, body.fit))
                    .ok_or_else(|| "no body representation".into())
            }),
        };
        let (void, fit) = match void {
            Ok(void) => void,
            Err(reason) => {
                self.refused.insert(
                    whole.clone(),
                    format!(
                        "whether its opening {id} cuts its parts cannot be decided, so it \
                         cannot be subtracted from them: {reason}"
                    ),
                );
                return;
            }
        };
        let mut cut = Vec::new();
        let leaves = self.leaves.get(whole).cloned().unwrap_or_default();
        for leaf in leaves {
            if self.unmeasured_parts.contains_key(&leaf) {
                continue;
            }
            let Some(body) = bodies.get(&leaf) else {
                continue;
            };
            let refuse = |reason: String| (reason, body_box(body));
            match cuts(&leaf, body, &id, &void, fit) {
                Cut::Cuts => {
                    self.cut
                        .entry(leaf.clone())
                        .or_default()
                        .push((id.clone(), whole.clone()));
                    cut.push(leaf);
                }
                Cut::Clear => {}
                Cut::Inside => {
                    let reason = format!(
                        "it lies wholly within the opening {id} voiding {whole}, which it is a \
                         part of"
                    );
                    self.unmeasured_parts.insert(leaf, refuse(reason));
                }
                Cut::Undecided(why) => {
                    let reason = format!(
                        "whether the opening {id} voiding {whole}, which it is a part of, cuts \
                         it is undecided: {why}"
                    );
                    self.unmeasured_parts.insert(leaf, refuse(reason));
                }
            }
        }
        self.decided.push((whole.clone(), id, cut));
    }

    /// Re-meshes each part cut by a whole's opening with those openings
    /// subtracted, or leaves it unmeasured naming them, and records which
    /// openings each body had subtracted, the wholes' included.
    fn subtract(
        &mut self,
        backend: &Compiler,
        parsed: &BTreeMap<SourceId, Parsed>,
        bodies: &mut BTreeMap<ObjectId, Body>,
        boundary: bool,
        report: &mut GeometryReport,
    ) {
        let mut done: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
        for (part, openings) in &self.cut {
            if self.unmeasured_parts.contains_key(part) {
                continue;
            }
            let Some(bound) = bodies.get(part).map(body_box) else {
                continue;
            };
            let mut ids: Vec<ObjectId> = openings
                .iter()
                .map(|(opening, _)| opening.clone())
                .collect();
            ids.sort();
            ids.dedup();
            let names = ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let entities: Vec<EntityId> = ids.iter().filter_map(entity_id).collect();
            let cut = match (parsed.get(&part.source), entity_id(part)) {
                (Some(source), Some(entity)) if entities.len() == ids.len() => mesh_less(
                    backend,
                    &source.model,
                    &source.units,
                    &source.linear,
                    entity,
                    boundary,
                    source.net,
                    &entities,
                )
                .and_then(|body| applied(part, body)),
                _ => Err("not a STEP instance id".to_owned()),
            };
            match cut {
                Ok(Some(body)) => {
                    bodies.insert(part.clone(), body);
                    done.insert(part.clone(), ids);
                }
                Ok(None) => {}
                Err(error) => {
                    let reason = format!(
                        "the whole's opening(s) {names} cut it but cannot be subtracted from \
                         it: {error}"
                    );
                    self.unmeasured_parts.insert(part.clone(), (reason, bound));
                }
            }
        }
        for (whole, leaves) in &self.leaves {
            let openings: BTreeSet<&ObjectId> = leaves
                .iter()
                .filter_map(|leaf| done.get(leaf))
                .flatten()
                .collect();
            if !openings.is_empty() {
                self.subtracted
                    .insert(whole.clone(), openings.into_iter().cloned().collect());
            }
        }
        for (whole, opening, parts) in std::mem::take(&mut self.decided) {
            let parts = parts
                .into_iter()
                .filter(|part| done.get(part).is_some_and(|ids| ids.contains(&opening)))
                .collect();
            report.whole_openings.push((whole, opening, parts));
        }
        self.subtracted.extend(done);
    }
}

/// Every part of `whole` that is no whole itself, at any depth.
fn collect_leaves(
    whole: &ObjectId,
    parts: &BTreeMap<ObjectId, Vec<ObjectId>>,
    wholes: &BTreeSet<&ObjectId>,
    visited: &mut BTreeSet<ObjectId>,
    leaves: &mut BTreeSet<ObjectId>,
) {
    if !visited.insert(whole.clone()) {
        return;
    }
    for part in parts.get(whole).into_iter().flatten() {
        if wholes.contains(part) {
            collect_leaves(part, parts, wholes, visited, leaves);
        } else {
            leaves.insert(part.clone());
        }
    }
}

/// Whether a whole's opening cuts a part's material.
#[derive(Debug, PartialEq)]
enum Cut {
    /// Shown to reach into the part's material beyond the touching depth.
    Cuts,
    /// Shown to touch it at most.
    Clear,
    /// The part lies wholly within the opening.
    Inside,
    /// Neither, with why.
    Undecided(String),
}

/// Whether `opening`'s void cuts `part`'s material deeper than
/// [`TOUCHING_DEPTH`].
///
/// Apart beyond both deviations is [`Cut::Clear`]. Two exact bodies are
/// decided cell by cell of the opening ([`OpeningCell`], [`cell_cut`]):
/// each connected piece of its mesh is one convex cell when it is convex
/// (an extruded rectangle, as exporters write most openings), one prism
/// per cap triangle when it is an extrusion of any other profile, and
/// otherwise only bounded by a convex cell holding it. What that leaves
/// open, and every tessellated pair, is read from the certified volume the
/// two share ([`shared_volume_cut`]). Anything still open is undecided,
/// never a guess either way.
fn cuts(
    part: &ObjectId,
    body: &Body,
    opening: &ObjectId,
    void: &axiolid_mesh::TriMesh,
    fit: Fit,
) -> Cut {
    let deviation = |fit: Fit| match fit {
        Fit::Exact => 0.0,
        Fit::Within(deviation) => deviation,
    };
    let slack = deviation(body.fit) + deviation(fit);
    if let (Some(first), Some(second)) = (mesh_box(&body.mesh), mesh_box(void))
        && (0..3).any(|axis| {
            first.0[axis] - second.1[axis] > slack + ON_SURFACE
                || second.0[axis] - first.1[axis] > slack + ON_SURFACE
        })
    {
        return Cut::Clear;
    }
    if slack == 0.0 {
        let mut open = false;
        for cell in OpeningCell::all(void) {
            match cell_cut(&body.mesh, &cell) {
                Some(Cut::Clear) => {}
                Some(decided) => return decided,
                None => open = true,
            }
        }
        if !open {
            return Cut::Clear;
        }
    }
    shared_volume_cut(part, body, opening, void, fit)
}

/// A convex region of an opening, as the half-spaces bounding it.
#[derive(Debug)]
struct OpeningCell {
    /// Outward unit normals and offsets: `n · x <= d` inside.
    planes: Vec<([f64; 3], f64)>,
    /// Whether the cell lies within the opening; otherwise it only holds
    /// a piece of it.
    within: bool,
    /// A point inside: the mean of the corners it was built from.
    centre: [f64; 3],
    /// The corners it was built from, which span its width along each of
    /// its planes.
    corners: Vec<[f64; 3]>,
}

impl OpeningCell {
    /// The cells of every connected piece of `mesh` (triangles joined by
    /// shared indices).
    fn all(mesh: &axiolid_mesh::TriMesh) -> Vec<Self> {
        let count = mesh.positions.len();
        let mut parent: Vec<usize> = (0..count).collect();
        let triangles: Vec<[usize; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|corners| [corners[0], corners[1], corners[2]].map(|corner| corner as usize))
            .filter(|corners| corners.iter().all(|corner| *corner < count))
            .collect();
        for corners in &triangles {
            for other in &corners[1..] {
                let (a, b) = (
                    union_root(&mut parent, corners[0]),
                    union_root(&mut parent, *other),
                );
                parent[a] = b;
            }
        }
        let mut pieces: BTreeMap<usize, Vec<[usize; 3]>> = BTreeMap::new();
        for corners in triangles {
            let key = union_root(&mut parent, corners[0]);
            pieces.entry(key).or_default().push(corners);
        }
        let points: Vec<[f64; 3]> = mesh.positions.iter().map(|p| [p.x, p.y, p.z]).collect();
        pieces
            .into_values()
            .flat_map(|triangles| Self::piece(&points, &triangles))
            .collect()
    }

    /// The cells of one closed piece.
    fn piece(points: &[[f64; 3]], triangles: &[[usize; 3]]) -> Vec<Self> {
        let corners: Vec<[f64; 3]> = triangles
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<usize>>()
            .into_iter()
            .map(|index| points[index])
            .collect();
        let volume: f64 = triangles
            .iter()
            .map(|[a, b, c]| dot(points[*a], cross(points[*b], points[*c])))
            .sum();
        let sign = volume.signum();
        let planes: Vec<([f64; 3], f64)> = triangles
            .iter()
            .filter_map(|[a, b, c]| {
                let normal = cross(sub(points[*b], points[*a]), sub(points[*c], points[*a]));
                let length = dot(normal, normal).sqrt();
                (length > MIN_FACE_AREA && volume != 0.0).then(|| {
                    let unit = normal.map(|value| sign * value / length);
                    (unit, dot(unit, points[*a]))
                })
            })
            .collect();
        let slack = CONVEX_SLACK + rounding(&corners);
        let convex = !planes.is_empty()
            && planes.iter().all(|(normal, offset)| {
                corners
                    .iter()
                    .all(|corner| dot(*normal, *corner) - offset <= slack)
            });
        if convex {
            return vec![Self {
                centre: mean(&corners),
                planes,
                within: true,
                corners,
            }];
        }
        if let Some(prisms) = Self::prisms(points, triangles, &planes, &corners, slack) {
            return prisms;
        }
        // Bounded by the slabs its own faces and the axes span.
        let mut normals: Vec<[f64; 3]> = planes.iter().map(|(normal, _)| *normal).collect();
        normals.extend([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let mut bounds = Vec::with_capacity(2 * normals.len());
        for normal in normals {
            let (low, high) =
                corners
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), corner| {
                        let value = dot(normal, *corner);
                        (low.min(value), high.max(value))
                    });
            bounds.push((normal, high));
            bounds.push((normal.map(|value| -value), -low));
        }
        vec![Self {
            planes: bounds,
            within: false,
            centre: mean(&corners),
            corners,
        }]
    }

    /// The prisms of a piece extruded from a profile of any shape: its
    /// corners lie on two parallel planes, each on the first one moved by
    /// one extrusion vector onto the second, and each cap triangle on the
    /// first plane sweeps one convex prism. `None` for anything else.
    fn prisms(
        points: &[[f64; 3]],
        triangles: &[[usize; 3]],
        planes: &[([f64; 3], f64)],
        corners: &[[f64; 3]],
        slack: f64,
    ) -> Option<Vec<Self>> {
        for (normal, _) in planes {
            let values: Vec<f64> = corners.iter().map(|corner| dot(*normal, *corner)).collect();
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if high - low <= slack
                || values
                    .iter()
                    .any(|value| value - low > slack && high - value > slack)
            {
                continue;
            }
            let on = |point: [f64; 3], level: f64| (dot(*normal, point) - level).abs() <= slack;
            let bottom: Vec<[f64; 3]> = corners
                .iter()
                .copied()
                .filter(|corner| on(*corner, low))
                .collect();
            let top: Vec<[f64; 3]> = corners
                .iter()
                .copied()
                .filter(|corner| on(*corner, high))
                .collect();
            if bottom.len() != top.len() || bottom.len() < 3 {
                continue;
            }
            let near =
                |a: [f64; 3], b: [f64; 3]| (0..3).all(|axis| (a[axis] - b[axis]).abs() <= slack);
            let Some(vector) = top.iter().map(|end| sub(*end, bottom[0])).find(|vector| {
                bottom.iter().all(|start| {
                    let moved = std::array::from_fn(|axis| start[axis] + vector[axis]);
                    top.iter().any(|end| near(moved, *end))
                })
            }) else {
                continue;
            };
            let mut prisms = Vec::new();
            for [a, b, c] in triangles {
                let cap = [points[*a], points[*b], points[*c]];
                if !cap.iter().all(|corner| on(*corner, low)) {
                    continue;
                }
                if let Some(prism) = Self::prism(cap, vector) {
                    prisms.push(prism);
                }
            }
            if !prisms.is_empty() {
                return Some(prisms);
            }
        }
        None
    }

    /// The triangle `cap` swept by `vector`; `None` when it bounds no
    /// volume.
    fn prism(cap: [[f64; 3]; 3], vector: [f64; 3]) -> Option<Self> {
        let unit = |normal: [f64; 3]| {
            let length = dot(normal, normal).sqrt();
            (length > MIN_FACE_AREA).then(|| normal.map(|value| value / length))
        };
        let [a, b, c] = cap;
        let mut base = unit(cross(sub(b, a), sub(c, a)))?;
        if dot(base, vector) > 0.0 {
            base = base.map(|value| -value);
        }
        if dot(base, vector).abs() <= MIN_FACE_AREA {
            return None;
        }
        let top = std::array::from_fn(|axis| a[axis] + vector[axis]);
        let mut planes = vec![
            (base, dot(base, a)),
            (base.map(|value| -value), -dot(base, top)),
        ];
        for (start, end, other) in [(a, b, c), (b, c, a), (c, a, b)] {
            let mut side = unit(cross(sub(end, start), vector))?;
            if dot(side, sub(other, start)) > 0.0 {
                side = side.map(|value| -value);
            }
            planes.push((side, dot(side, start)));
        }
        let centre =
            std::array::from_fn(|axis| (a[axis] + b[axis] + c[axis]) / 3.0 + 0.5 * vector[axis]);
        let shifted = |point: [f64; 3]| std::array::from_fn(|axis| point[axis] + vector[axis]);
        Some(Self {
            planes,
            within: true,
            centre,
            corners: vec![a, b, c, shifted(a), shifted(b), shifted(c)],
        })
    }

    /// How deep `point` lies inside the cell's planes; negative outside.
    fn depth(&self, point: [f64; 3]) -> f64 {
        self.planes
            .iter()
            .map(|(normal, offset)| offset - dot(*normal, point))
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether the cell's core (its planes moved in by `depth`) holds no
    /// ball: the cell is no wider than `2 depth` across one of its planes,
    /// and a ball inside it is no wider than the cell.
    fn thinner_than(&self, depth: f64) -> bool {
        self.planes.iter().any(|(normal, offset)| {
            self.corners
                .iter()
                .map(|corner| offset - dot(*normal, *corner))
                .fold(0.0_f64, f64::max)
                <= 2.0 * depth
        })
    }

    /// Whether some point of the triangle lies at least `depth` inside
    /// the planes: the triangle clipped by each plane moved in by it.
    fn reached(&self, triangle: [[f64; 3]; 3], depth: f64) -> bool {
        let mut polygon: Vec<[f64; 3]> = triangle.to_vec();
        for (normal, offset) in &self.planes {
            let limit = offset - depth;
            let mut kept = Vec::with_capacity(polygon.len() + 1);
            for (index, point) in polygon.iter().enumerate() {
                let next = polygon[(index + 1) % polygon.len()];
                let (here, there) = (dot(*normal, *point) - limit, dot(*normal, next) - limit);
                if here <= 0.0 {
                    kept.push(*point);
                }
                if (here < 0.0 && there > 0.0) || (here > 0.0 && there < 0.0) {
                    let t = here / (here - there);
                    kept.push(std::array::from_fn(|axis| {
                        point[axis] + t * (next[axis] - point[axis])
                    }));
                }
            }
            if kept.is_empty() {
                return false;
            }
            polygon = kept;
        }
        true
    }
}

/// The mean of some points.
fn mean(points: &[[f64; 3]]) -> [f64; 3] {
    let count = points.iter().fold(0.0, |count, _| count + 1.0);
    std::array::from_fn(|axis| points.iter().map(|point| point[axis]).sum::<f64>() / count)
}

/// The rounding of a plane's offset at the coordinates of `points`.
fn rounding(points: &[[f64; 3]]) -> f64 {
    64.0 * f64::EPSILON
        * points
            .iter()
            .flatten()
            .fold(1.0_f64, |largest, value| largest.max(value.abs()))
}

/// The representative of `at` in a union-find forest, halving the path.
fn union_root(parent: &mut [usize], mut at: usize) -> usize {
    while parent[at] != at {
        parent[at] = parent[parent[at]];
        at = parent[at];
    }
    at
}

/// Planes of an opening piece whose corners lie off them by less than this,
/// in metres, still bound a convex piece.
const CONVEX_SLACK: f64 = 1e-9;

/// Twice the area, in square metres, below which a face of an opening
/// piece states no plane: its normal is rounding.
const MIN_FACE_AREA: f64 = 1e-12;

/// How deep, in metres, an opening may reach into a part and still only
/// touch it: the coordinate precision IFC exporters state for their
/// models (`IfcGeometricRepresentationContext.Precision`, commonly 1e-5),
/// within which a hole a part already carries and the opening that made it
/// are written apart.
const TOUCHING_DEPTH: f64 = 1e-5;

/// Whether one cell of an opening cuts an exact part deeper than
/// [`TOUCHING_DEPTH`]; `None` where this cannot tell.
///
/// The cell's core (its planes moved in by the depth) is convex. The part
/// reaches into it with its surface, so material of the part lies in it
/// ([`Cut::Cuts`]); or its surface stays out of the core, which then lies
/// wholly inside the part ([`Cut::Cuts`]: a cavity) or wholly outside
/// ([`Cut::Clear`]), as the winding number of the core's centre tells. A
/// part all of whose corners lie in a cell's core lies within the opening
/// ([`Cut::Inside`]). For a cell that only holds a piece of the opening,
/// only a part clear of its core is decided. Material of the part left in
/// the opening is then at most `2 TOUCHING_DEPTH` thick: within the depth
/// of its faces, or of the faces between its cells.
fn cell_cut(part: &axiolid_mesh::TriMesh, cell: &OpeningCell) -> Option<Cut> {
    if cell.thinner_than(TOUCHING_DEPTH) {
        return Some(Cut::Clear);
    }
    if cell.within
        && !part.positions.is_empty()
        && part
            .positions
            .iter()
            .all(|p| cell.depth([p.x, p.y, p.z]) > TOUCHING_DEPTH)
    {
        return Some(Cut::Inside);
    }
    let point = |index: u32| {
        let p = part.positions[index as usize];
        [p.x, p.y, p.z]
    };
    let reached = part.indices.chunks_exact(3).any(|corners| {
        cell.reached(
            [point(corners[0]), point(corners[1]), point(corners[2])],
            TOUCHING_DEPTH,
        )
    });
    if reached {
        return cell.within.then_some(Cut::Cuts);
    }
    if cell.depth(cell.centre) <= TOUCHING_DEPTH {
        return None;
    }
    let winding = winding_number(part, cell.centre);
    let whole = winding.round();
    if (winding - whole).abs() > 0.1 {
        return None;
    }
    if whole == 0.0 {
        Some(Cut::Clear)
    } else {
        cell.within.then_some(Cut::Cuts)
    }
}

/// The generalized winding number of a closed mesh about `point`, by the
/// solid angle of each triangle (Van Oosterom and Strackee).
fn winding_number(mesh: &axiolid_mesh::TriMesh, point: [f64; 3]) -> f64 {
    let corner = |index: u32| {
        let p = mesh.positions[index as usize];
        sub([p.x, p.y, p.z], point)
    };
    let angle: f64 = mesh
        .indices
        .chunks_exact(3)
        .map(|corners| {
            let (a, b, c) = (corner(corners[0]), corner(corners[1]), corner(corners[2]));
            let (la, lb, lc) = (dot(a, a).sqrt(), dot(b, b).sqrt(), dot(c, c).sqrt());
            let numerator = dot(a, cross(b, c));
            let denominator = la * lb * lc + dot(a, b) * lc + dot(b, c) * la + dot(c, a) * lb;
            2.0 * numerator.atan2(denominator)
        })
        .sum();
    angle / (4.0 * std::f64::consts::PI)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Whether `opening`'s void cuts `part`'s body by the volume they share,
/// as the proximity service certifies it: the part inside the opening is
/// [`Cut::Inside`], the opening inside the part or a shared volume beyond
/// `TOUCHING_VOLUME` is [`Cut::Cuts`], apart or a shared volume within it
/// is [`Cut::Clear`]; anything else is undecided. A penetration witness is
/// no proof here: a point on a face an opening shares with a hole already
/// cut reads deep.
fn shared_volume_cut(
    part: &ObjectId,
    body: &Body,
    opening: &ObjectId,
    void: &axiolid_mesh::TriMesh,
    fit: Fit,
) -> Cut {
    let register =
        |geometry: AxiolidGeometry, id: &ObjectId, mesh: &axiolid_mesh::TriMesh, fit| match fit {
            Fit::Exact => geometry.with_mesh(id.clone(), mesh.clone()),
            Fit::Within(deviation) => {
                geometry.with_tessellated_mesh(id.clone(), mesh.clone(), deviation)
            }
        };
    let probe = register(
        register(AxiolidGeometry::new(), part, &body.mesh, body.fit),
        opening,
        void,
        fit,
    );
    let measured = match ProximityRequest::try_new(part.clone(), opening.clone())
        .and_then(|request| AxiolidProximityService::new(probe).measure_proximity(&request))
    {
        Ok(measured) => measured,
        Err(error) => return Cut::Undecided(format!("they cannot be measured: {error:?}")),
    };
    match measured.containment() {
        Some(BodyContainment::SubjectInsideCounterpart) => return Cut::Inside,
        Some(BodyContainment::CounterpartInsideSubject) => return Cut::Cuts,
        None => {}
    }
    if measured.separation_interval_metres().0 > ON_SURFACE {
        return Cut::Clear;
    }
    match measured.intersection_volume() {
        Some(volume) if volume.shared().lower_cubic_metres() > TOUCHING_VOLUME => Cut::Cuts,
        Some(volume) if volume.shared().upper_cubic_metres() <= TOUCHING_VOLUME => Cut::Clear,
        Some(volume) => Cut::Undecided(format!(
            "they meet, and the volume they share, {:.3e} to {:.3e} m³, is not bounded away \
             from the rounding of bodies that only touch",
            volume.shared().lower_cubic_metres(),
            volume.shared().upper_cubic_metres()
        )),
        None => Cut::Undecided("they meet, and the volume they share cannot be measured".into()),
    }
}

/// A mesh's box; `None` for an empty one.
fn mesh_box(mesh: &axiolid_mesh::TriMesh) -> Option<Extent> {
    let mut points = mesh.positions.iter();
    let first = points.next()?;
    let start = ([first.x, first.y, first.z], [first.x, first.y, first.z]);
    Some(points.fold(start, |(min, max), point| {
        let point = [point.x, point.y, point.z];
        (
            std::array::from_fn(|axis| min[axis].min(point[axis])),
            std::array::from_fn(|axis| max[axis].max(point[axis])),
        )
    }))
}

/// The box a body's true shape lies within: its mesh's, grown by its chord
/// deviation. Cutting only takes material away, so it bounds the body cut
/// too.
fn body_box(body: &Body) -> Option<Extent> {
    let (min, max) = mesh_box(&body.mesh)?;
    let grow = match body.fit {
        Fit::Exact => 0.0,
        Fit::Within(deviation) => deviation,
    };
    Some((min.map(|value| value - grow), max.map(|value| value + grow)))
}

/// Whether `opening` is an `IfcOpeningElement` (or `IfcOpeningStandardCase`)
/// whose representations are all, and only, `Reference`: the reading
/// `ifc-geometry` takes an opening as applied by (openbimrs/ifc#351), read
/// here for a whole, which lowering never sees as a host. Anything
/// unreadable is no such statement.
fn reference_only(model: &Model, opening: EntityId) -> bool {
    let Some(entity) = model.get(opening) else {
        return false;
    };
    if !(entity.type_name.eq_ignore_ascii_case("IFCOPENINGELEMENT")
        || entity
            .type_name
            .eq_ignore_ascii_case("IFCOPENINGSTANDARDCASE"))
    {
        return false;
    }
    let Some(shape) = ifc_geometry::Slots::new(opening, entity).opt_ref(PRODUCT_REPRESENTATION)
    else {
        return false;
    };
    let Some(representations) = model.get(shape).and_then(|entity| {
        ifc_geometry::ProductShape::new(shape, entity)
            .representations()
            .ok()
    }) else {
        return false;
    };
    !representations.is_empty()
        && representations.into_iter().all(|id| {
            model.get(id).is_some_and(|entity| {
                ifc_geometry::Representation::new(id, entity)
                    .identifier()
                    .as_deref()
                    == Some("Reference")
            })
        })
}

/// Every product left unmeasured for a fact about the model data, once
/// each, as an integrity warning: no shape representation (and no parts),
/// a face whose boundary crosses or runs back along itself, or openings
/// that remove the whole body; then every measured product in
/// `open_surfaces`, whose authored faces leave edges bounding one face only.
fn model_data_notes(
    unmeasured: &[(ObjectId, String)],
    open_surfaces: &[(ObjectId, usize)],
    snapshots: &[SourceSnapshot],
    kinds: &BTreeMap<ObjectId, String>,
) -> Vec<ModelData> {
    let shapeless = Bodiless::Shapeless.reason();
    let fingerprint = |id: &ObjectId| {
        snapshots
            .iter()
            .find(|snapshot| *snapshot.source() == id.source)
            .map_or("", SourceSnapshot::fingerprint)
            .to_owned()
    };
    let kind = |id: &ObjectId| kinds.get(id).map_or("", String::as_str).to_owned();
    let open = open_surfaces.iter().map(|(id, edges)| {
        let (fingerprint, kind, local) = (fingerprint(id), kind(id), &id.local_id);
        let edges = if *edges == 1 {
            "an edge that bounds".to_owned()
        } else {
            format!("{edges} edges that bound")
        };
        ModelData {
            code: OPEN_SURFACE,
            message: format!(
                "{local} {kind} has faces that, as authored, leave {edges} one face only, so \
                 its body is an open surface with no inside; measurements that need one are \
                 not evaluated"
            ),
            locator: format!("ifc:{fingerprint}:open-surface:{local}"),
        }
    });
    unmeasured
        .iter()
        .filter_map(|(id, reason)| {
            let (fingerprint, kind) = (fingerprint(id), kind(id));
            let local = &id.local_id;
            if *reason == shapeless {
                return Some(ModelData {
                    code: NO_SHAPE_REPRESENTATION,
                    message: format!(
                        "{local} {kind} has no shape representation and no parts; every \
                         measurement of it is not evaluated"
                    ),
                    locator: format!("ifc:{fingerprint}:no-shape:{local}"),
                });
            }
            if voided_body(reason) {
                return Some(ModelData {
                    code: VOIDED_BODY,
                    message: format!(
                        "{local} {kind} has openings that remove its whole body; every \
                         measurement of it is not evaluated"
                    ),
                    locator: format!("ifc:{fingerprint}:voided-body:{local}"),
                });
            }
            let defect = self_intersecting_face(reason)?;
            Some(ModelData {
                code: SELF_INTERSECTING_FACE,
                message: format!(
                    "{local} {kind} has a face whose boundary {defect}, so it bounds no \
                     region; every measurement of it is not evaluated"
                ),
                locator: format!("ifc:{fingerprint}:self-intersecting-face:{local}"),
            })
        })
        .chain(open)
        .collect()
}

/// What is wrong with a face's boundary, when `reason` is the mesh
/// compiler refusing to triangulate a face because one of its rings, as
/// written, crosses itself or runs back along itself (#298).
///
/// The compiler's ring checks are exact on the face's projection onto its
/// plane. On the corpus behind #298 every such crossing lies at least
/// 10 um inside the other edge, far above rounding, or follows from a
/// boundary that runs back along itself. A ring that overlaps itself is
/// not read as model data: a hole joined to the outer boundary by a seam
/// traversed both ways bounds a region the compiler does not yet accept
/// (axiolid/kernel#270). Neither is a ring refused for other reasons.
fn self_intersecting_face(reason: &str) -> Option<&'static str> {
    let refusal = reason.strip_prefix("mesh compilation refused: invalid geometry input: ")?;
    let (face, ring) = refusal.split_once(" cannot be triangulated: ")?;
    if face != "planar face" && !face.starts_with("authored polygon face ") {
        return None;
    }
    let ring = ring
        .strip_prefix("its rings do not bound a region: ")
        .unwrap_or(ring);
    let defect = if let Some(defect) = ring.strip_prefix("profile outer ring ") {
        defect
    } else {
        let (hole, defect) = ring.strip_prefix("profile hole ")?.split_once(' ')?;
        hole.parse::<usize>().ok()?;
        defect
    };
    if defect == "intersects itself" {
        return Some("crosses itself");
    }
    let vertex = defect.strip_prefix("folds back on itself at vertex ")?;
    vertex
        .parse::<usize>()
        .ok()
        .map(|_| "runs back along itself")
}

/// Registers the boundaries that agree with their meshes, when one of them
/// is a tessellated body's: only a tessellated pair is ever certified.
fn with_boundaries(
    mut geometry: AxiolidGeometry,
    boundaries: Vec<(ObjectId, ExactBoundary, bool)>,
    report: &mut GeometryReport,
) -> AxiolidGeometry {
    let agreeing: Vec<(ObjectId, ExactBoundary, bool)> = boundaries
        .into_iter()
        .filter(|(id, boundary, _)| geometry.check_exact_boundary(id, boundary).is_ok())
        .collect();
    if !agreeing.iter().any(|(_, _, tessellated)| *tessellated) {
        return geometry;
    }
    for (id, boundary, tessellated) in agreeing {
        // The construction bounds a tessellated body's extent closer than
        // its chord deviation does (a slab's top stays at the floor).
        if tessellated {
            let (outer, inner) = boundary.extent_bounds();
            geometry = geometry.with_extent_bounds(id.clone(), outer, Some(inner));
        }
        geometry = geometry.with_exact_body(id, boundary.into_body());
        report.exact_boundaries += 1;
    }
    geometry
}

/// Registers every geometry service over `geometry`, bound to `snapshots`.
fn register(
    session: EvidenceSession,
    snapshots: &[SourceSnapshot],
    geometry: AxiolidGeometry,
    (space, shelves, boundaries, plan_areas): (
        AxiolidSpaceService,
        AxiolidLinearQuantityService,
        AxiolidBoundaryCoverageService,
        AxiolidPlanAreaService,
    ),
    envelope: LazyEnvelope,
    (walkability, routing): (AxiolidWalkabilityService, AxiolidMetricRoutingService),
) -> Result<EvidenceSession, Box<dyn Error>> {
    let source = snapshots
        .first()
        .ok_or("geometry needs a session over at least one source")?
        .source()
        .clone();
    let bound = snapshots;
    let session = session
        .with_host_service(
            ContactServiceHandle::new(Arc::new(AxiolidContactService::new(geometry.clone()))),
            bound,
        )?
        .with_host_service(
            FreeSpaceServiceHandle::new(Arc::new(AxiolidFreeSpaceService::new(
                geometry.clone(),
                source.clone(),
            ))),
            bound,
        )?
        .with_host_service(SpaceServiceHandle::new(Arc::new(space)), bound)?
        // Spaces and their declared boundaries are IFC facts.
        .with_host_service(
            BoundaryCoverageServiceHandle::new(Arc::new(boundaries)),
            bound,
        )?
        // Walking surfaces are the guard rule's selection, carried in each
        // request; the host declares none of its own.
        .with_host_service(
            GuardServiceHandle::new(Arc::new(AxiolidGuardService::new(
                geometry.clone(),
                source.clone(),
            ))),
            bound,
        )?
        // Doors and openings are the shelf rule's selection, carried in each
        // request; the bridge hands over the voids of bodiless openings.
        .with_host_service(LinearQuantityServiceHandle::new(Arc::new(shelves)), bound)?
        .with_host_service(
            // Effects continue through the voids of bodiless openings.
            PlanAreaServiceHandle::new(Arc::new(plan_areas)),
            bound,
        )?
        .with_host_service(
            PlanSpanServiceHandle::new(Arc::new(AxiolidPlanSpanService::new(
                geometry.clone(),
                source.clone(),
            ))),
            bound,
        )?
        // Counts the triangles of the meshes this bridge produced, so a
        // count follows its chord budget for curved bodies.
        .with_host_service(
            TriangleCountServiceHandle::new(Arc::new(AxiolidTriangleCountService::new(
                geometry.clone(),
            ))),
            bound,
        )?
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(AxiolidVerticalExtentService::new(
                geometry.clone(),
            ))),
            bound,
        )?
        // Treads, ramp runs and headroom from the same meshes; obstacles are
        // the rule's selection, carried in each request.
        .with_host_service(
            WalkingSurfaceServiceHandle::new(Arc::new(AxiolidWalkingSurfaceService::new(
                geometry.clone(),
            ))),
            bound,
        )?
        // Eyes, targets and blockers are the visibility rule's selection,
        // carried in each request.
        .with_host_service(
            SightServiceHandle::new(Arc::new(AxiolidSightService::new(geometry.clone()))),
            bound,
        )?
        .with_host_service(
            ProximityServiceHandle::new(Arc::new(AxiolidProximityService::new(geometry))),
            bound,
        )?
        // Bounding spaces are the envelope rule's selection, carried in each
        // request; the host declares only what the model states external.
        .with_host_service(
            EnvelopeMembershipServiceHandle::new(Arc::new(envelope)),
            bound,
        )?
        // Walkable surfaces, entrances and obstacles are the walkability
        // rule's selection; metric routing's are IFC classes.
        .with_host_service(WalkabilityServiceHandle::new(Arc::new(walkability)), bound)?
        .with_host_service(MetricRoutingServiceHandle::new(Arc::new(routing)), bound)?;
    Ok(session)
}

/// Members of every group (`IfcGroup`: zones, systems), so a bodiless zone
/// has the union of its members' footprints.
///
/// Membership is `IfcRelAssignsToGroup`, read through the session's
/// relationship service over every object. A refused answer, or no service
/// at all, leaves the membership undecided: the group's footprint is then
/// unavailable, never the empty footprint of a group that groups nothing.
fn groups(
    relationships: Option<&RelationshipSelectionServiceHandle>,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> Vec<(ObjectId, Result<Vec<ObjectId>, String>)> {
    let everything: Vec<ObjectId> = kinds.keys().cloned().collect();
    let members = |group: &ObjectId| -> Result<Vec<ObjectId>, String> {
        let relationships =
            relationships.ok_or("the session has no relationship service to read groups with")?;
        let query = RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelAssignsToGroup")
                .map_err(|error| error.to_string())?,
            direction: TraversalDirection::Forward,
            follow_chain: false,
        };
        let request =
            RelationshipSelectionRequest::try_new(group.clone(), everything.clone(), query)
                .map_err(|error| error.to_string())?;
        let selection = relationships
            .select(&request)
            .map_err(|error| error.to_string())?;
        Ok(selection.candidates().to_vec())
    };
    kinds
        .keys()
        .filter(|id| is_a(id, "IfcGroup"))
        .map(|group| (group.clone(), members(group)))
        .collect()
}

/// Envelope membership declarations: what the model states external.
///
/// Which spaces bound the envelope is the envelope rule's selection, carried
/// in each request, so the bridge declares none. Each meshed object's
/// `IsExternal`, from whichever property set states it, is its declaration:
/// `true` external, `false` internal. An object without exactly one such
/// boolean stays undeclared and its rule reports not evaluated; absent is not
/// internal. A bounding space's own declaration is ignored by the adapter.
///
/// The declarations are read on the first request ([`LazyEnvelope`]): a run
/// whose rules never ask for envelope membership resolves no `IsExternal`.
fn envelope_service(
    session: &EvidenceSession,
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    timings: bool,
) -> Result<LazyEnvelope, Box<dyn Error>> {
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .ok_or("the session has no property service to read declarations with")?
        .clone();
    Ok(LazyEnvelope {
        properties,
        geometry: geometry.clone(),
        source: source.clone(),
        objects: kinds.keys().cloned().collect(),
        timings,
        declared: OnceLock::new(),
    })
}

/// The envelope membership service, its declarations read once, on the
/// first request ([`envelope_service`]).
struct LazyEnvelope {
    properties: PropertyResolutionServiceHandle,
    geometry: AxiolidGeometry,
    source: SourceId,
    objects: Vec<ObjectId>,
    /// Whether reading the declarations prints its time (`--timings`).
    timings: bool,
    declared: OnceLock<AxiolidEnvelopeMembershipService>,
}

impl LazyEnvelope {
    /// The service with every meshed object's declaration, in one pass over
    /// the objects: the property service indexes the model's property
    /// relationships once, so each object reads only its own sets.
    fn declare(&self) -> AxiolidEnvelopeMembershipService {
        let mut clock = timings::Stopwatch::new(self.timings);
        let value = |object: &ObjectId, name: &str| -> Option<PropertyValue> {
            let request = PropertyRequest::try_new(object.clone(), None, name).ok()?;
            match self.properties.resolve(&request).ok()? {
                PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
                PropertyResolution::Absent(_) => None,
            }
        };
        let mut service =
            AxiolidEnvelopeMembershipService::new(self.geometry.clone(), self.source.clone());
        for object in &self.objects {
            if self.geometry.mesh(object).is_none() {
                continue;
            }
            match value(object, "IsExternal") {
                Some(PropertyValue::Boolean(true)) => {
                    service = service.with_declared_external(object.clone());
                }
                Some(PropertyValue::Boolean(false)) => {
                    service = service.with_declared_internal(object.clone());
                }
                _ => {}
            }
        }
        clock.lap("geometry: envelope declarations (first request)");
        service
    }
}

impl EnvelopeMembershipService for LazyEnvelope {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        self.declared
            .get_or_init(|| self.declare())
            .measure_envelope_membership(request)
    }
}

/// An opening's meshed void and how it stands for the void, or why it has
/// none.
type Void = Result<(axiolid_mesh::TriMesh, Fit), String>;

/// Relationships derived from geometry, over the model's spaces, its
/// doors, windows and openings, and its walls and slabs.
///
/// Every `IfcSpace` is a space, every `IfcDoor`, `IfcWindow` and
/// `IfcOpeningElement` an opening, and every `IfcWall` and `IfcSlab` (with
/// their subtypes) a separating element for `adjacent-across`; all are IFC
/// facts. A void that could not
/// be meshed is declared unmeasured, so the derivation refuses it rather than
/// finding no space beside it.
fn derived_service(
    geometry: &AxiolidGeometry,
    parsed: &BTreeMap<SourceId, Parsed>,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
    voids: Vec<(ObjectId, Void)>,
) -> AxiolidDerivedRelationshipService {
    let mut service = AxiolidDerivedRelationshipService::new(geometry.clone());
    for id in kinds.keys() {
        if is_a(id, "IfcSpace") {
            service = service.with_space(id.clone());
        } else if is_a(id, "IfcDoor") || is_a(id, "IfcWindow") {
            service = service.with_opening(id.clone());
        } else if is_a(id, "IfcWall") || is_a(id, "IfcSlab") {
            service = service.with_separating_element(id.clone());
        }
    }
    for (id, void) in voids {
        service = match void {
            Ok((mesh, Fit::Exact)) => service.with_opening_void(id, mesh),
            Ok((mesh, Fit::Within(deviation))) => {
                service.with_tessellated_opening_void(id, mesh, deviation)
            }
            Err(reason) => service.with_unmeasured_opening_void(id, reason),
        };
    }
    with_levels(service, parsed, kinds, is_a)
}

/// A storey's world height, or why it has none.
type Height = Result<f64, String>;

/// A storey's source and spatial parent: the storeys whose heights order
/// its band.
type Parent = (SourceId, Option<EntityId>);

/// Declares every `IfcBuildingStorey` a level for `spans-level`: its band
/// runs from its placement's height up to the next storey's of the same
/// parent (open above for the highest). A storey whose placement cannot be
/// read, is tilted, or shares its height with a sibling has no band, so a
/// request it could answer refuses.
fn with_levels(
    mut service: AxiolidDerivedRelationshipService,
    parsed: &BTreeMap<SourceId, Parsed>,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> AxiolidDerivedRelationshipService {
    let trees: BTreeMap<&SourceId, SpatialTree> = parsed
        .iter()
        .map(|(source, Parsed { model, .. })| (source, SpatialTree::build(model)))
        .collect();
    let mut siblings: BTreeMap<Parent, Vec<(ObjectId, Height)>> = BTreeMap::new();
    for id in kinds.keys().filter(|id| is_a(id, "IfcBuildingStorey")) {
        let (
            Some(entity),
            Some(Parsed {
                model,
                units,
                linear,
                ..
            }),
        ) = (entity_id(id), parsed.get(&id.source))
        else {
            continue;
        };
        let parent = trees
            .get(&id.source)
            .and_then(|tree| tree.node(entity))
            .and_then(|node| node.parent);
        let height = linear
            .refusal(model, entity)
            .map_or_else(|| Ok(()), Err)
            .and_then(|()| world_transform(model, units, entity).map_err(|e| e.to_string()))
            .and_then(|frame| {
                let up = frame.basis[2];
                if up[0].abs() > 1e-12 || up[1].abs() > 1e-12 || (up[2] - 1.0).abs() > 1e-12 {
                    Err("the storey's placement is tilted".to_owned())
                } else {
                    Ok(frame.origin[2])
                }
            });
        siblings
            .entry((id.source.clone(), parent))
            .or_default()
            .push((id.clone(), height));
    }
    for levels in siblings.into_values() {
        let mut heights: Vec<f64> = levels
            .iter()
            .filter_map(|(_, height)| height.as_ref().ok().copied())
            .collect();
        heights.sort_by(f64::total_cmp);
        for (level, height) in levels {
            service = match height {
                Err(reason) => service.with_unmeasured_level(level, reason),
                #[allow(clippy::float_cmp)]
                Ok(height) if heights.iter().filter(|other| **other == height).count() > 1 => {
                    service.with_unmeasured_level(level, "another storey shares its elevation")
                }
                Ok(height) => {
                    let top = heights.iter().copied().find(|other| *other > height);
                    service.with_level(level, height, top)
                }
            };
        }
    }
    service
}

/// Plan areas over `geometry`, with the exact voids of bodiless openings so
/// an effect can continue through them; any other void is unmeasured.
fn plan_area_service(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    voids: &[(ObjectId, Void)],
) -> AxiolidPlanAreaService {
    voids.iter().fold(
        AxiolidPlanAreaService::new(geometry.clone(), source.clone()),
        |service, (id, void)| match void {
            Ok((mesh, Fit::Exact)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((_, Fit::Within(_))) | Err(_) => service.with_unmeasured_opening_void(id.clone()),
        },
    )
}

/// Shelf lengths over `geometry`, with the voids of bodiless openings so a
/// shelf rule can place their clearances.
fn linear_service(
    geometry: &AxiolidGeometry,
    voids: &[(ObjectId, Void)],
) -> AxiolidLinearQuantityService {
    voids.iter().fold(
        AxiolidLinearQuantityService::new(geometry.clone()),
        |service, (id, void)| match void {
            Ok((mesh, Fit::Exact)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((mesh, Fit::Within(deviation))) => {
                service.with_tessellated_opening_void(id.clone(), mesh.clone(), *deviation)
            }
            Err(_) => service.with_unmeasured_opening_void(id.clone()),
        },
    )
}

/// The walkability and metric-routing services over `geometry`.
fn route_services(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
    voids: &[(ObjectId, Void)],
) -> (AxiolidWalkabilityService, AxiolidMetricRoutingService) {
    (
        walkability_service(geometry, source, voids),
        routing_service(geometry, source, kinds, is_a, voids),
    )
}

/// Walkable regions over the surfaces, entrances and obstacles each request
/// selects. The bridge only hands over the voids of opening elements, which
/// have no body of their own. IFC states no door clear width the bridge can
/// trust (a door's overall width includes its lining), so none is declared:
/// a door bounds route widths from above only.
fn walkability_service(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    voids: &[(ObjectId, Void)],
) -> AxiolidWalkabilityService {
    voids.iter().fold(
        AxiolidWalkabilityService::new(geometry.clone(), source.clone()),
        |service, (id, void)| match void {
            Ok((mesh, Fit::Exact)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((mesh, Fit::Within(_))) => {
                service.with_tessellated_opening_void(id.clone(), mesh.clone())
            }
            Err(reason) => service.with_unmeasured_opening_void(id.clone(), reason.clone()),
        },
    )
}

/// Metric routes over every `IfcSpace` as a walkable surface, every `IfcDoor`
/// and opening element as a portal, and every stair, ramp (and their flights)
/// and transport element as a vertical connector; all are IFC facts. Every
/// other body obstructs. Windows are not portals: a window filling an opening
/// obstructs it.
fn routing_service(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
    voids: &[(ObjectId, Void)],
) -> AxiolidMetricRoutingService {
    let mut service = AxiolidMetricRoutingService::new(geometry.clone(), source.clone());
    for id in kinds.keys() {
        if is_a(id, "IfcSpace") {
            service = service.with_surface(id.clone());
        } else if is_a(id, "IfcDoor") {
            service = service.with_portal(id.clone());
        } else if CONNECTORS.iter().any(|connector| is_a(id, connector)) {
            service = service.with_connector(id.clone());
        }
    }
    for (id, void) in voids {
        service = match void {
            Ok((mesh, Fit::Exact)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((mesh, Fit::Within(_))) => {
                service.with_tessellated_opening_void(id.clone(), mesh.clone())
            }
            Err(reason) => service.with_unmeasured_opening_void(id.clone(), reason.clone()),
        };
    }
    service
}

/// Entity types that join levels. Names from both IFC2X3 and IFC4.
const CONNECTORS: &[&str] = &[
    "IfcStair",
    "IfcStairFlight",
    "IfcRamp",
    "IfcRampFlight",
    "IfcTransportElement",
];

/// Facade areas, with every `IfcSpace` as the interior a face may look into.
///
/// Which objects are spaces is an IFC fact. Whether a wall is external is
/// not this bridge's to decide: the rule selects the walls it measures.
fn facade_service(
    geometry: &AxiolidGeometry,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> AxiolidFacadeAreaService {
    kinds.keys().filter(|id| is_a(id, "IfcSpace")).fold(
        AxiolidFacadeAreaService::new(geometry.clone()),
        |service, id| service.with_space(id.clone()),
    )
}

/// Space-boundary coverage over every `IfcSpace` and the space boundaries
/// the model declares for it (`IfcRelSpaceBoundary` and its subtypes).
///
/// Which boundaries a space has is an IFC fact, so every declared boundary
/// is registered: with its connection surface meshed in the space body's
/// coordinates, or unmeasured with the reason, so its space is refused
/// rather than measured without it. A boundary stating no connection
/// geometry, one that is not a surface, and any surface the lowering or
/// compiler refuses are unmeasured.
fn boundary_service(
    backend: &Compiler,
    parsed: &BTreeMap<SourceId, Parsed>,
    geometry: &AxiolidGeometry,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> AxiolidBoundaryCoverageService {
    let mut service = AxiolidBoundaryCoverageService::new(geometry.clone());
    for id in kinds.keys().filter(|id| is_a(id, "IfcSpace")) {
        service = service.with_space(id.clone());
    }
    for (
        source,
        Parsed {
            model,
            units,
            linear,
            ..
        },
    ) in parsed
    {
        let object = |entity: EntityId| ObjectId {
            source: source.clone(),
            local_id: entity.to_string(),
        };
        for boundary in ifc_spatial::relation::boundary::all(model) {
            let Some(space) = boundary.space.map(object).filter(|id| is_a(id, "IfcSpace")) else {
                continue;
            };
            let element = boundary
                .element
                .map(object)
                .filter(|id| kinds.contains_key(id));
            let id = object(boundary.id);
            service = match boundary_surface(backend, model, units, linear, &boundary, &space) {
                Ok((mesh, Fit::Exact)) => service.with_boundary(space, id, element, mesh),
                Ok((mesh, Fit::Within(deviation))) => {
                    service.with_tessellated_boundary(space, id, element, mesh, deviation)
                }
                Err(reason) => service.with_unmeasured_boundary(space, id, element, reason),
            };
        }
    }
    service
}

/// One boundary's connection surface as a mesh in the coordinates of its
/// space's body, and how it stands for the surface.
///
/// `ifc-spatial` reads the boundary's `ConnectionGeometry` and `ifc-geometry`
/// lowers its `SurfaceOnRelatingElement`: a surface, a face surface or a
/// face-based surface model; point, curve and volume connections are refused.
/// The surface is stated in the relating space's object coordinates, so it
/// is lowered in the frame `ifc-geometry` places the space's body in: the
/// body context's world coordinate system above the space's placement
/// (`product_representation_frame`, openbimrs/ifc#164). A curve-bounded
/// plane takes that frame on its basis plane only, its boundaries staying
/// in the plane's parameters (openbimrs/ifc#163).
fn boundary_surface(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    boundary: &SpaceBoundary,
    space: &ObjectId,
) -> Result<(axiolid_mesh::TriMesh, Fit), String> {
    let connection = match boundary.connection_geometry(model) {
        Ok(Some(connection)) => connection,
        Ok(None) => return Err("the boundary states no connection geometry".into()),
        Err(ConnectionGeometryAnomaly::Dangling { target, .. }) => {
            return Err(format!("the connection geometry {target} does not exist"));
        }
        Err(ConnectionGeometryAnomaly::WrongKind {
            target, type_name, ..
        }) => {
            return Err(format!(
                "{target} is a {type_name}, not a connection geometry"
            ));
        }
        Err(anomaly) => return Err(format!("unreadable connection geometry: {anomaly:?}")),
    };
    let frame = space_frame(model, units, linear, space)?;
    let mut session = session(model, units);
    let root = lower_connection_surface(&mut session, connection, frame)
        .map_err(|error| error.to_string())?;
    let lowered = session.finish(root).map_err(|error| error.to_string())?;
    compile(backend, &lowered.graph, lowered.root)
}

/// How a compiled mesh stands for the surface it was compiled from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fit {
    /// Every face is planar: the mesh is the shape.
    Exact,
    /// Curved: every point of the true surface lies within this many
    /// metres of the mesh, as the compiler certifies it.
    Within(f64),
}

/// `root` compiled into a mesh, and how it stands for the surface.
///
/// A planar body is exact, and compiled without a deviation report, which
/// would only measure what its planarity already proves. A curved one is
/// declared with the deviation the compiler certifies (axiolid/kernel#232):
/// the chord budget where its construction proves it (`Proven`), the bound
/// it computed for this mesh where that is larger or smaller (`Certified`),
/// as returned. A boolean is certified by measuring its mesh against the
/// exact compiler's result (#235: a wall with a round window, a beam cut by
/// round holes, a wall clipped by its roof), which costs time per curved
/// boolean. A mesh it cannot bound (`Unbounded`: a boolean the exact
/// compiler refuses, a tapered extrusion, a sectioned spine, ...) is
/// refused with the paths that have no bound, so the object is unmeasured
/// rather than declared within a tolerance nothing proves.
///
/// Authored faces warped off their plane by more than the tolerance
/// (polygon mesh faces, and B-rep faces given only by their loops) make
/// either kind tessellated, declared within the slab width the compiler
/// reports for them ([`warped_width`]), and leave a body that cuts or is
/// cut by one unmeasured ([`authored_leaves`]).
fn compile(
    backend: &Compiler,
    graph: &GeometryGraph,
    root: NodeId,
) -> Result<(axiolid_mesh::TriMesh, Fit), String> {
    let options = ExecutionOptions::new(TOLERANCE);
    let leaves = authored_leaves(graph, root)?;
    for &leaf in &leaves.under_boolean {
        let (_, report) = backend
            .compile_mesh_with_deviation(graph, leaf, &options)
            .map_err(|error| compilation_refused(&error))?;
        if let Some(width) = warped_width(&report) {
            return Err(format!(
                "an authored face warped off its plane (its corners span a slab {width:.3} m \
                 wide) is an operand of a boolean, and nothing bounds how far the boolean's \
                 result lies from its mesh"
            ));
        }
    }
    if planar(graph, root, &mut NODE_BUDGET.clone()) {
        if !leaves.outside_boolean {
            let mesh = backend
                .compile_mesh(graph, root, &options)
                .map_err(|error| compilation_refused(&error))?;
            if mesh.triangle_count() == 0 {
                return Err(NO_TRIANGLES.into());
            }
            return Ok((mesh, Fit::Exact));
        }
        // Planar but for warped faces, which only the report finds. Its
        // booleans have planar operands, none of them warped (above), so
        // their meshes are exact whatever the report says of them.
        let (outcome, report) = backend
            .compile_mesh_with_deviation(graph, root, &options)
            .map_err(|error| compilation_refused(&error))?;
        if outcome.mesh.triangle_count() == 0 {
            return Err(NO_TRIANGLES.into());
        }
        return Ok((
            outcome.mesh,
            warped_width(&report).map_or(Fit::Exact, Fit::Within),
        ));
    }
    let (outcome, report) = backend
        .compile_mesh_with_deviation(graph, root, &options)
        .map_err(|error| compilation_refused(&error))?;
    let mesh = outcome.mesh;
    if mesh.triangle_count() == 0 {
        return Err(NO_TRIANGLES.into());
    }
    match report.bound {
        Some(bound) if bound.is_finite() && bound >= 0.0 => Ok((mesh, Fit::Within(bound))),
        _ => Err(uncertified(&report)),
    }
}

/// The details under which the mesh compiler reports a face warped off
/// its plane beyond the tolerance: an authored polygon face (#254) and a
/// B-rep face that declares no surface (#257).
const WARPED_FACES: [&str; 2] = [
    "non-planar authored face",
    "non-planar face without a surface",
];

/// The largest bound the compiler reports for warped faces, `None` when
/// it reports none.
///
/// A face whose corners leave its plane has no single true surface
/// (axiolid/kernel#254): the two triangulations of a quad, a bilinear
/// patch and the face flattened onto its fit plane are all readings of
/// it. Since axiolid-mesh-compile 0.3.14 (#257, #261) the compiler reports
/// such a face, authored or a faceted B-rep face, `Certified` with the
/// width of the slab its corners (holes included) span about its fit
/// plane, which holds every reading and the mesh: a saddle with corners
/// at `±h` reports `2 h`, a square with one corner lifted by `h` about
/// `h / 2`, the gap between its two triangulations. The bound is scaled
/// by every placement above the face. Faces within the tolerance count
/// as planar and report nothing here.
fn warped_width(report: &DeviationReport) -> Option<f64> {
    report
        .contributions
        .iter()
        .filter(|contribution| WARPED_FACES.contains(&contribution.detail))
        .filter_map(|contribution| contribution.bound.value())
        .reduce(f64::max)
}

/// Where the faces that can be warped (polygon meshes, and B-reps with a
/// face given only by its loops) enter a body.
#[derive(Debug, Default)]
struct AuthoredLeaves {
    /// Whether one enters other than as a boolean operand.
    outside_boolean: bool,
    /// The nodes entering a boolean as (part of) an operand.
    under_boolean: Vec<NodeId>,
}

/// The faces under `root` that can be warped, walked through instances,
/// collections and boolean operands.
///
/// A boolean cut by or cutting a warped face moves its section by more
/// than any bound on its operands covers (#235), and the compiler reports
/// no bound for it: the exact compiler, which the report measures a
/// boolean against, refuses polygon meshes and B-reps. So each such node
/// under a boolean is compiled alone and the body refused when it is
/// warped. A graph too large to walk is refused too.
fn authored_leaves(graph: &GeometryGraph, root: NodeId) -> Result<AuthoredLeaves, String> {
    fn visit(
        graph: &GeometryGraph,
        id: NodeId,
        boolean: bool,
        budget: &mut usize,
        leaves: &mut AuthoredLeaves,
    ) -> Result<(), String> {
        if *budget == 0 {
            return Err(
                "the geometry graph is too large to find the authored faces that may be warped"
                    .into(),
            );
        }
        *budget -= 1;
        let authored = match graph.get(id) {
            Some(GeometryNode::Instance(instance)) => {
                return visit(graph, instance.source, boolean, budget, leaves);
            }
            Some(GeometryNode::Collection(children)) => {
                for child in children {
                    visit(graph, *child, boolean, budget, leaves)?;
                }
                return Ok(());
            }
            Some(GeometryNode::SolidOperation(SolidOperation::Boolean { left, right, .. })) => {
                visit(graph, *left, true, budget, leaves)?;
                return visit(graph, *right, true, budget, leaves);
            }
            Some(GeometryNode::PolygonMesh(_)) => true,
            Some(GeometryNode::BRep(brep)) => {
                brep.faces().iter().any(|face| face.surface.is_none())
            }
            _ => false,
        };
        if authored {
            if boolean {
                if !leaves.under_boolean.contains(&id) {
                    leaves.under_boolean.push(id);
                }
            } else {
                leaves.outside_boolean = true;
            }
        }
        Ok(())
    }
    let mut leaves = AuthoredLeaves::default();
    visit(graph, root, false, &mut NODE_BUDGET.clone(), &mut leaves)?;
    Ok(leaves)
}

/// Why a curved mesh has no certified deviation: the paths the compiler
/// names unbounded, each with its reason.
fn uncertified(report: &DeviationReport) -> String {
    let paths: Vec<String> = report
        .contributions
        .iter()
        .filter_map(|contribution| match contribution.bound {
            DeviationBound::Unbounded(reason) => Some(reason.to_owned()),
            _ => None,
        })
        .collect();
    format!(
        "the mesh compiler certifies no bound on how far the curved surface lies from its \
         mesh ({}), so it is not declared within the {} m chord tolerance",
        if paths.is_empty() {
            "no path reported one".to_owned()
        } else {
            paths.join("; ")
        },
        TOLERANCE.linear()
    )
}

/// Why the mesh compiler refused a body, which leaves it unmeasured.
///
/// A body whose surface needs more steps round an axis than the kernel
/// takes (4096 a turn) to stay within the chord tolerance is refused with
/// `BudgetExceeded` rather than meshed more coarsely (axiolid/kernel#231):
/// its mesh would break the deviation declared for it, so it is not
/// measured at all.
fn compilation_refused(error: &GeomError) -> String {
    match error {
        GeomError::BudgetExceeded { resource } => format!(
            "mesh compilation refused: keeping the surface within the {} m chord tolerance \
             needs more {resource} than the kernel's budget allows, and a coarser mesh \
             would break that tolerance",
            TOLERANCE.linear()
        ),
        error => format!("mesh compilation refused: {error}"),
    }
}

/// The frame a space's body is placed in, as lowering places it: the world
/// coordinate system of its body representation's context above its
/// placement. A space with no body representation has only its placement;
/// its coverage is refused for want of a body anyway.
fn space_frame(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    space: &ObjectId,
) -> Result<Transform, String> {
    let entity = entity_id(space).ok_or("the space is not a STEP instance")?;
    if let Some(reason) = linear.refusal(model, entity) {
        return Err(reason);
    }
    match ifc_geometry::product_representation_frame_with_evaluator(
        model,
        units,
        entity,
        RepresentationPurpose::Body,
        &EVALUATOR,
        CACHED_POSITIONS,
    ) {
        Ok(Some(frame)) => Ok(frame),
        Ok(None) => world_transform(model, units, entity).map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// One product's meshed body.
struct Body {
    mesh: axiolid_mesh::TriMesh,
    /// How the mesh stands for the body.
    fit: Fit,
    /// The exact boundary built from the same graph, when asked for and
    /// exactly constructible.
    boundary: Option<ExactBoundary>,
    /// Openings taken as already applied to the body, with the reason
    /// ([`applied`]); empty when every opening was subtracted.
    applied: Vec<(ObjectId, &'static str)>,
    /// What lowering reported as taken as applied, before [`applied`]
    /// names it.
    taken: Vec<ifc_geometry::lower::TakenAsApplied>,
}

/// `body` with the openings taken as applied named under the source of
/// `host`, each with its reason. An opening taken for a reason this bridge
/// does not know leaves the host unmeasured, naming it.
fn applied(host: &ObjectId, body: Option<Body>) -> Meshed {
    let Some(mut body) = body else {
        return Ok(None);
    };
    for taken in std::mem::take(&mut body.taken) {
        let opening = format!("#{}", taken.opening.0);
        let Some(reason) = applied_reason(taken.reason) else {
            return Err(format!(
                "the opening {opening} is taken as already applied for a reason this \
                 bridge does not know: {:?}",
                taken.reason
            ));
        };
        let opening = ObjectId::new(host.source.clone(), opening)
            .map_err(|error| format!("the opening #{}: {error}", taken.opening.0))?;
        body.applied.push((opening, reason));
    }
    Ok(Some(body))
}

/// One product's body, as [`mesh`] returns it.
type Meshed = Result<Option<Body>, String>;

/// A mesh as the snapshot renderer takes it: positions and triangles.
fn snapshot_mesh(mesh: &axiolid_mesh::TriMesh) -> Option<bcf_snapshot::Mesh> {
    bcf_snapshot::Mesh::new(
        mesh.positions.iter().map(|p| [p.x, p.y, p.z]).collect(),
        mesh.triangles().collect(),
    )
}

fn entity_id(id: &ObjectId) -> Option<EntityId> {
    id.local_id.strip_prefix('#')?.parse().ok().map(EntityId)
}

/// `IfcProduct.Representation`.
const PRODUCT_REPRESENTATION: usize = 6;

/// The world box, in metres, that a product's `Box` representation states:
/// every `IfcBoundingBox` item's eight corners placed as the product's
/// representations are (the context's `WorldCoordinateSystem` above the
/// placement chain) and enclosed. A box is authored in the
/// representation's own axes, so its corners are placed one by one, never
/// its corner and extents as world axes.
///
/// `None` when the product states no box or any part of it cannot be read:
/// an unmeasured product without one may be anywhere.
fn stated_box(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    product: EntityId,
) -> Option<([f64; 3], [f64; 3])> {
    if linear.refusal(model, product).is_some() {
        return None;
    }
    let shape =
        ifc_geometry::Slots::new(product, model.get(product)?).opt_ref(PRODUCT_REPRESENTATION)?;
    let representations = ifc_geometry::ProductShape::new(shape, model.get(shape)?)
        .representations()
        .ok()?;
    let placement = world_transform(model, units, product).ok()?;
    let mut bound: Option<([f64; 3], [f64; 3])> = None;
    for id in representations {
        let representation = ifc_geometry::Representation::new(id, model.get(id)?);
        if !representation
            .identifier()
            .is_some_and(|identifier| identifier.eq_ignore_ascii_case("Box"))
        {
            continue;
        }
        let frame = context_frame(model, units, id)?.compose(&placement);
        for item in representation.items().ok()? {
            let entity = model.get(item)?;
            if !entity.type_name.eq_ignore_ascii_case("IFCBOUNDINGBOX") {
                return None;
            }
            let item = ifc_geometry::solid::BoundingBox::new(item, entity);
            let corner = item.corner_point(model).ok()?.coordinates_3d().ok()?;
            let size = item.checked_dimensions().ok()?;
            for index in 0..8 {
                let local: [f64; 3] = std::array::from_fn(|axis| {
                    let far = (index >> axis) & 1 == 1;
                    units.length(corner[axis] + if far { size[axis] } else { 0.0 })
                });
                let world = frame.apply(local);
                if world.iter().any(|value| !value.is_finite()) {
                    return None;
                }
                bound = Some(match bound {
                    None => (world, world),
                    Some((min, max)) => (
                        std::array::from_fn(|axis| min[axis].min(world[axis])),
                        std::array::from_fn(|axis| max[axis].max(world[axis])),
                    ),
                });
            }
        }
    }
    bound
}

/// One product's net body (openings subtracted, or taken as applied as
/// `net` says), whether it is exact, and with `boundary` its exact boundary
/// where the lowered graph has one.
fn mesh(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    product: EntityId,
    boundary: bool,
    net: NetOptions,
) -> Meshed {
    mesh_less(backend, model, units, linear, product, boundary, net, &[])
}

/// [`mesh`], with the `Body` of each of `openings` subtracted from the net
/// body as well, in the order given: the openings voiding a whole the
/// product is a part of, which cut it (#223). Each is lowered by its own
/// placement and subtracted in world coordinates; a product whose net body
/// is several solids, or an opening whose `Body` is, is refused by the
/// graph by name, never cut in part.
#[allow(clippy::too_many_arguments)] // `mesh`'s inputs and the openings
fn mesh_less(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    product: EntityId,
    boundary: bool,
    net: NetOptions,
    openings: &[EntityId],
) -> Meshed {
    if let Some(reason) = std::iter::once(&product)
        .chain(openings)
        .find_map(|object| linear.refusal(model, *object))
    {
        return Err(reason);
    }
    let net_options = net;
    let mut session = session(model, units);
    let Some(net) =
        lower_product_net_with(&mut session, product, net).map_err(|e| e.to_string())?
    else {
        return if openings.is_empty() {
            Ok(None)
        } else {
            Err("it has no body to subtract the openings from".into())
        };
    };
    let mut root = net.root;
    for &opening in openings {
        let tool = lower_product_representation(&mut session, opening, RepresentationPurpose::Body)
            .map_err(|error| format!("the opening #{}: {error}", opening.0))?
            .ok_or_else(|| format!("the opening #{} has no body representation", opening.0))?;
        root = session
            .node_for(
                opening,
                GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left: root,
                    right: tool,
                    operator: BooleanOperator::Difference,
                }),
            )
            .map_err(|error| format!("the opening #{} cannot be subtracted: {error}", opening.0))?;
    }
    let lowered = session.finish(root).map_err(|e| e.to_string())?;
    let (mesh, fit) = match compile(backend, &lowered.graph, lowered.root) {
        // Removed whole only by the openings of a whole it is part of (its
        // own net body was measured before): not its own openings' voided
        // body (#310); the caller names the whole's openings.
        Err(reason) if reason == NO_TRIANGLES && !openings.is_empty() => {
            return Err("nothing of it is left outside the openings".into());
        }
        Err(reason) if reason == NO_TRIANGLES && !net.subtractions.is_empty() => {
            return Err(
                if gross_has_triangles(backend, model, units, product, net_options) {
                    voided_body_reason(net.subtractions.len())
                } else {
                    reason
                },
            );
        }
        compiled => compiled?,
    };
    // Built from the graph the mesh was compiled from, so it carries the
    // same placement; anything without an exact construction keeps its
    // mesh alone.
    let boundary = boundary
        .then(|| axioval::axiolid::exact_boundary(&lowered.graph, lowered.root).ok())
        .flatten();
    Ok(Some(Body {
        mesh,
        fit,
        boundary,
        applied: Vec::new(),
        taken: net.taken_as_applied,
    }))
}

/// Whether `product`'s gross `Body`, before its openings are subtracted,
/// compiles to a mesh with triangles: then a net body that compiles to none
/// was removed whole by the openings (model data, [`VOIDED_BODY`]). The
/// exact compiler that found the net body empty is the one every other
/// measurement trusts; anything refused here keeps the plain reason.
fn gross_has_triangles(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    product: EntityId,
    net: NetOptions,
) -> bool {
    let mut session = session(model, units);
    let Ok(Some(net)) = lower_product_net_with(&mut session, product, net) else {
        return false;
    };
    let Ok(lowered) = session.finish(net.gross) else {
        return false;
    };
    compile(backend, &lowered.graph, lowered.root).is_ok_and(|(mesh, _)| mesh.triangle_count() > 0)
}

/// The void of an opening taken as already applied: its `Reference`
/// solid, every item of every `Reference` representation placed as its
/// representations are (the context's world coordinate system above the
/// placement chain) and compiled as a body is. An opening with any other
/// representation was never taken as applied, so one here is refused.
fn reference_void(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    linear: &Linear,
    opening: EntityId,
) -> Void {
    if let Some(reason) = linear.refusal(model, opening) {
        return Err(reason);
    }
    let entity = model
        .get(opening)
        .ok_or_else(|| format!("the opening {opening} does not exist"))?;
    let shape = ifc_geometry::Slots::new(opening, entity)
        .opt_ref(PRODUCT_REPRESENTATION)
        .ok_or("the opening has no representation")?;
    let representations = ifc_geometry::ProductShape::new(
        shape,
        model
            .get(shape)
            .ok_or_else(|| format!("the representation {shape} does not exist"))?,
    )
    .representations()
    .map_err(|error| error.to_string())?;
    let placement = world_transform(model, units, opening).map_err(|error| error.to_string())?;
    let mut session = session(model, units);
    let mut roots = Vec::new();
    for id in representations {
        let representation = ifc_geometry::Representation::new(
            id,
            model
                .get(id)
                .ok_or_else(|| format!("the representation {id} does not exist"))?,
        );
        if representation.identifier().as_deref() != Some("Reference") {
            return Err(format!(
                "the representation {id} of the opening is not 'Reference'"
            ));
        }
        let frame = context_frame(model, units, id)
            .ok_or_else(|| format!("the context of the representation {id} is unreadable"))?
            .compose(&placement);
        for item in representation.items().map_err(|error| error.to_string())? {
            roots.push(
                lower_representation_item(&mut session, item, frame)
                    .map_err(|error| error.to_string())?,
            );
        }
    }
    let root = match roots.as_slice() {
        [] => return Err("the opening's Reference representation holds no item".into()),
        [root] => *root,
        _ => session
            .node_for(opening, GeometryNode::Collection(roots))
            .map_err(|error| error.to_string())?,
    };
    let lowered = session.finish(root).map_err(|error| error.to_string())?;
    compile(backend, &lowered.graph, lowered.root)
}

/// The frame a representation's context places it in: the context's
/// `WorldCoordinateSystem` in metres, the identity when it states none;
/// `None` when it cannot be read.
fn context_frame(
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    representation: EntityId,
) -> Option<Transform> {
    match ifc_geometry::context_of(model, representation)
        .and_then(|context| context.world_coordinate_system(model))
    {
        Some(system) => Some(
            ifc_geometry::resource::placement::axis_placement_transform(
                model,
                system,
                model.get(system)?,
            )
            .ok()?
            .to_metres(units),
        ),
        None => Some(Transform::identity()),
    }
}

/// Whether every face under `id` is planar, so its mesh is its exact shape.
///
/// Conservative: only structures known to stay planar count, and anything
/// else, including an exhausted budget, is treated as curved. A planar
/// product mis-called curved only loses exactness; the reverse would present
/// a chord approximation as exact.
fn planar(graph: &GeometryGraph, id: NodeId, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let Some(node) = graph.get(id) else {
        return false;
    };
    match node {
        GeometryNode::TriMesh(_)
        | GeometryNode::PolygonMesh(_)
        | GeometryNode::BoundingBox(_)
        | GeometryNode::HalfSpace(_)
        | GeometryNode::Primitive(Primitive::Block { .. }) => true,
        // An affine placement maps planes to planes.
        GeometryNode::Instance(instance) => planar(graph, instance.source, budget),
        GeometryNode::Collection(children) => {
            children.iter().all(|child| planar(graph, *child, budget))
        }
        GeometryNode::SolidOperation(SolidOperation::Extrusion { profile, .. }) => {
            planar(graph, *profile, budget)
        }
        GeometryNode::SolidOperation(SolidOperation::Boolean { left, right, .. }) => {
            planar(graph, *left, budget) && planar(graph, *right, budget)
        }
        // A plane cut down to the prism of a polygon in it: planes only.
        GeometryNode::SolidOperation(SolidOperation::BoundedHalfSpace {
            half_space,
            boundary,
            ..
        }) => {
            planar(graph, *half_space, budget)
                && matches!(
                    graph.get(*boundary),
                    Some(GeometryNode::Curve2(Curve2::Line(_) | Curve2::Polyline(_)))
                )
        }
        GeometryNode::Profile(profile) => polygonal(profile),
        // A plane bounded by straight boundaries (a space boundary's
        // connection surface): its triangles are the region.
        GeometryNode::SurfaceRelation(SurfaceRelation::CurveBounded {
            basis, boundaries, ..
        }) => {
            matches!(
                graph.get(*basis),
                Some(GeometryNode::Surface(Surface::Plane(_)))
            ) && boundaries.iter().all(|boundary| {
                matches!(
                    graph.get(*boundary),
                    Some(
                        GeometryNode::Curve3(Curve3::Line(_) | Curve3::Polyline(_))
                            | GeometryNode::Curve2(Curve2::Line(_) | Curve2::Polyline(_))
                    )
                )
            })
        }
        // A faceted B-rep: every face on a plane (or given only by its
        // polygon, which IFC requires to be planar) and every edge straight.
        GeometryNode::BRep(brep) => {
            brep.faces().iter().all(|face| {
                face.surface.is_none_or(|surface| {
                    matches!(
                        graph.get(surface),
                        Some(GeometryNode::Surface(Surface::Plane(_)))
                    )
                })
            }) && brep.edges().iter().all(|edge| {
                edge.curve.is_none_or(|curve| {
                    matches!(
                        graph.get(curve),
                        Some(GeometryNode::Curve3(Curve3::Line(_) | Curve3::Polyline(_)))
                    )
                })
            })
        }
        _ => false,
    }
}

fn polygonal(profile: &Profile) -> bool {
    let straight = |curve: &Curve2| matches!(curve, Curve2::Line(_) | Curve2::Polyline(_));
    let sharp = |radius: Option<f64>| radius.is_none_or(|r| r == 0.0);
    match profile {
        Profile::Rectangle(rectangle) => {
            sharp(rectangle.outer_radius) && sharp(rectangle.inner_radius)
        }
        // A section meshes from its exact contour (axiolid-mesh-compile
        // 0.3.5): straight edges, sloped ones included, are exact and only
        // its fillets and rounded edges are chorded.
        Profile::Section(section) => match section {
            SectionProfile::I {
                fillet_radius,
                flange_edge_radius,
                ..
            } => sharp(*fillet_radius) && sharp(*flange_edge_radius),
            SectionProfile::AsymmetricI {
                bottom_fillet_radius,
                bottom_flange_edge_radius,
                top_fillet_radius,
                top_flange_edge_radius,
                ..
            } => [
                bottom_fillet_radius,
                bottom_flange_edge_radius,
                top_fillet_radius,
                top_flange_edge_radius,
            ]
            .into_iter()
            .all(|radius| sharp(*radius)),
            SectionProfile::L {
                fillet_radius,
                edge_radius,
                ..
            }
            | SectionProfile::U {
                fillet_radius,
                edge_radius,
                ..
            }
            | SectionProfile::Z {
                fillet_radius,
                edge_radius,
                ..
            } => sharp(*fillet_radius) && sharp(*edge_radius),
            SectionProfile::T {
                fillet_radius,
                flange_edge_radius,
                web_edge_radius,
                ..
            } => sharp(*fillet_radius) && sharp(*flange_edge_radius) && sharp(*web_edge_radius),
            SectionProfile::C {
                internal_fillet_radius,
                ..
            } => sharp(*internal_fillet_radius),
            SectionProfile::Trapezium { .. } => true,
            _ => false,
        },
        Profile::Contour(contour) => std::iter::once(&contour.outer)
            .chain(&contour.holes)
            .flat_map(|ring| &ring.segments)
            .all(|segment| straight(&segment.curve)),
        Profile::Derived { basis, .. } => polygonal(basis),
        Profile::Composite(parts) => parts.iter().all(polygonal),
        _ => false,
    }
}

/// Space validation needs roles and storeys, which are IFC facts.
///
/// Storeys come from each source's own spatial tree, so an object's storey
/// is always one of its own model's. An element the file places twice, or
/// anything under a structure aggregated twice, gets no storey: the tree
/// keeps one of the two parents, and a guessed storey would move floor area
/// between storeys without saying so.
fn space_service(
    parsed: &BTreeMap<SourceId, Parsed>,
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> AxiolidSpaceService {
    let trees: BTreeMap<&SourceId, (SpatialTree, BTreeSet<EntityId>)> = parsed
        .iter()
        .map(|(source, Parsed { model, .. })| {
            let tree = SpatialTree::build(model);
            let ambiguous: BTreeSet<EntityId> = tree
                .anomalies()
                .iter()
                .filter_map(|anomaly| match anomaly {
                    SpatialAnomaly::ContainedTwice { element, .. } => Some(*element),
                    SpatialAnomaly::AggregatedTwice { child, .. } => Some(*child),
                    _ => None,
                })
                .collect();
            (source, (tree, ambiguous))
        })
        .collect();
    let storey_of = |tree: &SpatialTree,
                     ambiguous: &BTreeSet<EntityId>,
                     entity: EntityId|
     -> Option<EntityId> {
        let mut chain = vec![entity];
        chain.extend(tree.container_of(entity));
        let start = *chain.last()?;
        chain.extend(tree.ancestors(start));
        if chain.iter().any(|link| ambiguous.contains(link)) {
            return None;
        }
        chain.into_iter().skip(1).find(|link| {
            tree.node(*link)
                .is_some_and(|node| node.kind == SpatialKind::Storey)
        })
    };

    let mut service = AxiolidSpaceService::new(geometry.clone(), source.clone());
    for id in kinds.keys() {
        let (Some(entity), Some((tree, ambiguous))) = (entity_id(id), trees.get(&id.source)) else {
            continue;
        };
        if is_a(id, "IfcSpace") {
            service = service.with_space(id.clone());
        } else if is_a(id, "IfcSlab") {
            service = service.with_slab(id.clone());
        } else if is_a(id, "IfcRoof") {
            service = service.with_roof(id.clone());
        } else if is_a(id, "IfcBuilding") {
            service = service.with_building(id.clone());
        }
        let is_structure = tree.node(entity).is_some();
        if (!is_structure || is_a(id, "IfcSpace"))
            && !geometry.has_no_body(id)
            && let Some(storey) = storey_of(tree, ambiguous, entity)
        {
            let storey = ObjectId {
                source: id.source.clone(),
                local_id: storey.to_string(),
            };
            service = service.with_storey(id.clone(), storey);
        }
    }
    service
}

/// The measured extent of every object `report` names, for fitting BCF
/// cameras: each session's proximity service answers for its own objects.
///
/// An object no session measured is left out, so its viewpoint gets no
/// camera rather than a guessed one. The extent is the box that encloses the
/// true body (the mesh box grown by its chord deviation).
pub fn bounds(sessions: &[&EvidenceSession], report: &Report) -> BTreeMap<ObjectId, bcf::Bounds> {
    let mut named: BTreeSet<&ObjectId> = BTreeSet::new();
    for finding in report.findings() {
        named.extend(finding.object_id());
        named.extend(&finding.related);
    }
    for outcome in report.not_evaluated() {
        named.extend(outcome.object_id());
    }
    let mut bounds = BTreeMap::new();
    for object in named {
        let measured = sessions
            .iter()
            .filter(|session| session.project().object(object).is_some())
            .filter_map(|session| session.service::<ProximityServiceHandle>())
            .find_map(|service| service.bounds(object).ok())
            .map(|measured| measured.enclosing())
            .and_then(|extent| bcf::Bounds::new(extent.min(), extent.max()));
        if let Some(measured) = measured {
            bounds.insert(object.clone(), measured);
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::attach;
    use axioval::engine::{
        MetricPoint, MetricRouteOutcome, MetricRouteRequest, MetricRoutingServiceHandle,
        MobilityProfile, WalkabilityRequest, WalkabilityRouteOutcome, WalkabilityServiceHandle,
    };
    use axioval::ifc::import_ifc_session;
    use axioval::ir::ObjectId;

    /// The reason a host's openings remove its whole body is told apart
    /// from an empty compilation and from anything else (#310).
    #[test]
    fn only_openings_removing_the_whole_body_read_as_a_voided_body() {
        assert!(super::voided_body(&super::voided_body_reason(1)));
        assert!(super::voided_body(&super::voided_body_reason(12)));
        assert_eq!(
            super::voided_body_reason(2),
            "its openings remove its whole body: nothing is left once the 2 opening(s) \
             voiding it are subtracted"
        );
        for other in [
            super::NO_TRIANGLES,
            "its openings remove its whole body: nothing is left once the x opening(s) \
             voiding it are subtracted",
            "its openings remove its whole body",
        ] {
            assert!(!super::voided_body(other), "{other}");
        }
    }

    /// Only the compiler's refusals of a face ring that crosses or runs
    /// back along itself are model data (#298); a ring overlapping itself
    /// (a keyhole, axiolid/kernel#270), a triangulation finding no ear
    /// (axiolid/kernel#269) and refusals of anything but a face are not.
    #[test]
    fn only_a_face_ring_crossing_or_folding_back_is_model_data() {
        use super::self_intersecting_face as defect;
        let refused = "mesh compilation refused: invalid geometry input: ";
        for (reason, expected) in [
            (
                "planar face cannot be triangulated: profile outer ring intersects itself",
                Some("crosses itself"),
            ),
            (
                "authored polygon face 0 cannot be triangulated: its rings do not bound a \
                 region: profile outer ring intersects itself",
                Some("crosses itself"),
            ),
            (
                "planar face cannot be triangulated: profile outer ring folds back on itself \
                 at vertex 6",
                Some("runs back along itself"),
            ),
            (
                "authored polygon face 3 cannot be triangulated: its rings do not bound a \
                 region: profile hole 2 intersects itself",
                Some("crosses itself"),
            ),
            (
                "authored polygon face 21 cannot be triangulated: its rings do not bound a \
                 region: profile outer ring overlaps itself",
                None,
            ),
            (
                "planar face cannot be triangulated: profile hole 0 touches or crosses the \
                 outer ring",
                None,
            ),
            ("profile outer ring intersects itself", None),
            (
                "planar face cannot be triangulated: profile outer ring folds back on itself \
                 at vertex x",
                None,
            ),
        ] {
            assert_eq!(defect(&format!("{refused}{reason}")), expected, "{reason}");
        }
        assert_eq!(
            defect(
                "mesh compilation refused: numerically degenerate input: planar face cannot be \
                 triangulated: profile triangulation found no ear among 5 remaining vertices"
            ),
            None
        );
        assert_eq!(
            defect("profile outer ring intersects itself"),
            None,
            "not a mesh compilation refusal"
        );
    }

    /// Rooms `#16` (x 0..4) and `#26` (x 4.2..8.2) with a 0.9 m door body
    /// `#40` in the gap between them, and nothing else.
    fn rooms_and_door() -> String {
        let body = |first: u32, x: f64, y: f64, dx: f64, dy: f64, depth: f64| {
            let [point, position, profile, solid, shape] = [0, 1, 2, 3, 4].map(|o| first + o);
            format!(
                "#{point}=IFCCARTESIANPOINT(({x},{y}));\n\
                 #{position}=IFCAXIS2PLACEMENT2D(#{point},$);\n\
                 #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{position},{dx},{dy});\n\
                 #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth});\n\
                 #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
                 #{}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n",
                first + 5
            )
        };
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#2);\n\
             #4=IFCDIRECTION((0.,0.,1.));\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             {}#16=IFCSPACE('0000000000000000000016',$,$,$,$,#3,#15,$,.ELEMENT.,$,$);\n\
             {}#26=IFCSPACE('0000000000000000000026',$,$,$,$,#3,#25,$,.ELEMENT.,$,$);\n\
             {}#40=IFCDOOR('0000000000000000000040',$,$,$,$,#3,#35,$,2.1,0.9,$,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;\n",
            body(10, 2.0, 2.0, 4.0, 4.0, 3.0),
            body(20, 6.2, 2.0, 4.0, 4.0, 3.0),
            body(30, 4.1, 2.0, 0.1, 0.9, 2.1),
        )
    }

    #[test]
    fn geometry_registers_walkability_and_metric_routing() {
        let bytes = rooms_and_door();
        let session = import_ifc_session("rooms.ifc", bytes.as_bytes()).unwrap();
        let models: super::ModelBytes = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), bytes.as_bytes().to_vec()))
            .collect();
        let (session, report) = attach(session, &models, super::Options::default()).unwrap();
        assert_eq!(report.exact, 3, "{report:?}");
        let id = |local: &str| ObjectId {
            source: session.snapshots().next().unwrap().source().clone(),
            local_id: local.to_owned(),
        };

        let walkability = session.service::<WalkabilityServiceHandle>().unwrap();
        let request = |width: f64| {
            WalkabilityRequest::try_new(
                vec![id("#16"), id("#26")],
                vec![id("#40")],
                Vec::new(),
                width,
                None,
                true,
                false,
            )
            .unwrap()
        };
        // IFC states no clear width the bridge trusts, so a door that could
        // pass stays undecided, and one too narrow blocks.
        let wide_enough = walkability.snapshot(&request(0.8)).unwrap();
        assert_eq!(
            wide_enough.route_between(&id("#16"), &id("#26")).unwrap(),
            WalkabilityRouteOutcome::Indeterminate
        );
        let too_narrow = walkability.snapshot(&request(1.0)).unwrap();
        assert_eq!(
            too_narrow.route_between(&id("#16"), &id("#26")).unwrap(),
            WalkabilityRouteOutcome::Unreachable
        );

        let routing = session.service::<MetricRoutingServiceHandle>().unwrap();
        let route = MetricRouteRequest::new(
            MetricPoint::try_new(id("#16"), [1.0, 2.0, 0.0]).unwrap(),
            MetricPoint::try_new(id("#26"), [7.0, 2.0, 0.0]).unwrap(),
            MobilityProfile::try_new(0.5, 2.0, 0.02, 0.06).unwrap(),
        );
        assert!(matches!(
            routing.route(&route).unwrap(),
            MetricRouteOutcome::Blocked(_)
        ));
    }

    /// A section without fillets or rounded edges meshes to its exact
    /// contour, so its mesh is exact; any radius makes it a tessellation.
    #[test]
    fn only_sharp_sections_are_polygonal() {
        use super::polygonal;
        use axiolid_profile::{Profile, SectionProfile};

        let i = |fillet_radius| {
            Profile::Section(SectionProfile::I {
                depth: 0.3,
                width: 0.3,
                web_thickness: 0.011,
                flange_thickness: 0.019,
                fillet_radius,
                flange_edge_radius: None,
                flange_slope: Some(0.1),
            })
        };
        assert!(polygonal(&i(None)));
        assert!(polygonal(&i(Some(0.0))));
        assert!(!polygonal(&i(Some(0.027))));
        let c = |internal_fillet_radius| {
            Profile::Section(SectionProfile::C {
                depth: 0.2,
                width: 0.1,
                wall_thickness: 0.004,
                girth: 0.02,
                internal_fillet_radius,
            })
        };
        assert!(polygonal(&c(None)));
        assert!(!polygonal(&c(Some(0.004))));
        assert!(polygonal(&Profile::Section(SectionProfile::Trapezium {
            bottom_x: 0.4,
            top_x: 0.2,
            y: 0.3,
            top_offset: -0.05,
        })));
    }

    /// A unit box `[0, 1]^3` whose top corners `(0, 0)`, `(1, 0)`, `(1, 1)`
    /// and `(0, 1)` are raised by `top` metres, as an `IfcFacetedBrep`
    /// (`faceted`) or an `IfcPolygonalFaceSet`, the body of proxy `#90`.
    /// Its side faces stay planar (each keeps its two top corners in its
    /// vertical plane); its top quad is warped unless `top` is planar.
    fn top_box(top: [f64; 4], faceted: bool) -> String {
        use std::fmt::Write as _;
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0 + top[0]],
            [1.0, 0.0, 1.0 + top[1]],
            [1.0, 1.0, 1.0 + top[2]],
            [0.0, 1.0, 1.0 + top[3]],
        ];
        // Outward: bottom, top, front, right, back, left.
        let faces: [[usize; 4]; 6] = [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut data = String::new();
        let item = if faceted {
            for (index, [x, y, z]) in corners.iter().enumerate() {
                writeln!(
                    data,
                    "#{}=IFCCARTESIANPOINT(({x:?},{y:?},{z:?}));",
                    20 + index
                )
                .unwrap();
            }
            let mut face_ids = Vec::new();
            for (index, face) in faces.iter().enumerate() {
                let [poly, bound, id] = [0, 1, 2].map(|o| 30 + 3 * index + o);
                let points: Vec<String> = face.iter().map(|c| format!("#{}", 20 + c)).collect();
                writeln!(
                    data,
                    "#{poly}=IFCPOLYLOOP(({}));\n#{bound}=IFCFACEOUTERBOUND(#{poly},.T.);\n\
                     #{id}=IFCFACE((#{bound}));",
                    points.join(",")
                )
                .unwrap();
                face_ids.push(format!("#{id}"));
            }
            writeln!(
                data,
                "#60=IFCCLOSEDSHELL(({}));\n#61=IFCFACETEDBREP(#60);",
                face_ids.join(",")
            )
            .unwrap();
            "#62=IFCSHAPEREPRESENTATION(#5,'Body','Brep',(#61));\n"
        } else {
            let points: Vec<String> = corners
                .iter()
                .map(|[x, y, z]| format!("({x:?},{y:?},{z:?})"))
                .collect();
            writeln!(
                data,
                "#20=IFCCARTESIANPOINTLIST3D(({}),$);",
                points.join(",")
            )
            .unwrap();
            let mut face_ids = Vec::new();
            for (index, face) in faces.iter().enumerate() {
                let indices: Vec<String> = face.iter().map(|c| (c + 1).to_string()).collect();
                writeln!(
                    data,
                    "#{}=IFCINDEXEDPOLYGONALFACE(({}));",
                    30 + index,
                    indices.join(",")
                )
                .unwrap();
                face_ids.push(format!("#{}", 30 + index));
            }
            writeln!(
                data,
                "#61=IFCPOLYGONALFACESET(#20,.T.,({}),$);",
                face_ids.join(",")
            )
            .unwrap();
            "#62=IFCSHAPEREPRESENTATION(#5,'Body','Tessellation',(#61));\n"
        };
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#2);\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
             #7=IFCUNITASSIGNMENT((#6));\n\
             #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
             {data}{item}#63=IFCPRODUCTDEFINITIONSHAPE($,$,(#62));\n\
             #90=IFCBUILDINGELEMENTPROXY('0000000000000000000090',$,$,$,$,#3,#63,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    /// How `top_box`'s body is meshed.
    fn top_fit(top: [f64; 4], faceted: bool) -> Result<super::Fit, String> {
        let model = axioval::ifc::read_ifc_step(top_box(top, faceted).as_bytes()).unwrap();
        let units = ifc_geometry::units::resolve(&model);
        let backend = ifc_geometry::compile::default_backend();
        super::mesh(
            &backend,
            &model,
            &units,
            &super::Linear::default(),
            ifc_model::EntityId(90),
            false,
            super::NetOptions::default(),
        )
        .map(|body| body.expect("a body").fit)
    }

    /// The box's top with its corner `(1, 1)` lifted by `lift`.
    fn lifted(lift: f64) -> [f64; 4] {
        [0.0, 0.0, lift, 0.0]
    }

    #[test]
    fn a_lifted_corner_is_declared_within_the_gap_between_its_readings() {
        // axiolid/kernel#254, #257, #261: the top quad with one corner
        // lifted 5 cm is warped. Its two triangulations (one per diagonal)
        // are both readings of it; at the centre the diagonal through the
        // lifted corner passes `lift / 2` above the other's triangle
        // `(1,0,1) (1,1,1+lift) (0,1,1)`, whose unit normal is
        // `(-lift, -lift, 1)` normalised. The compiler reports the slab
        // width of the corners about the fit plane, about `lift / 2`, for
        // the polygon mesh and the faceted B-rep alike; it must cover that
        // gap, and stay below the twice-the-warp the bridge declared before.
        for lift in [0.05_f64, 0.25] {
            let apart = (lift / 2.0) / (1.0 + 2.0 * lift * lift).sqrt();
            for faceted in [true, false] {
                let Ok(super::Fit::Within(bound)) = top_fit(lifted(lift), faceted) else {
                    panic!("{faceted}: {:?}", top_fit(lifted(lift), faceted));
                };
                assert!(
                    apart <= bound && bound <= lift / 2.0 + 1e-12,
                    "{faceted}: {apart} <= {bound} <= {}",
                    lift / 2.0
                );
            }
        }
        // Within the tolerance a face counts as planar, as the compiler
        // counts it, and the body stays exact.
        for faceted in [true, false] {
            for lift in [0.0, 5e-4] {
                assert_eq!(
                    top_fit(lifted(lift), faceted),
                    Ok(super::Fit::Exact),
                    "{faceted}"
                );
            }
        }
    }

    #[test]
    fn a_saddle_is_declared_within_the_full_spread_of_its_corners() {
        // A top whose corners alternate `+h` and `-h`: its two
        // triangulations pass `h` above and below the fit plane at the
        // centre, `2 h` apart, which the bound must cover (the largest
        // corner distance `h` would not).
        let h = 0.02;
        for faceted in [true, false] {
            let Ok(super::Fit::Within(bound)) = top_fit([h, -h, h, -h], faceted) else {
                panic!("{faceted}: {:?}", top_fit([h, -h, h, -h], faceted));
            };
            assert!(
                2.0 * h <= bound && bound <= 2.0 * h * (1.0 + 1e-9),
                "{faceted}: {bound}"
            );
        }
    }

    /// A unit box as an `IfcFacetedBrep` (proxy `#90`) with a triangular
    /// pocket half a metre deep in its top. The pocket's rim corner
    /// `(1, 0.5, 1)` lies inside the top face's outer edge
    /// `(1, 0, 1)-(1, 1, 1)`, which the right face shares, when `rim` is
    /// one; below one the pocket stands clear of it.
    fn pocket_box(rim: f64) -> String {
        use std::fmt::Write as _;
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            [rim, 0.5, 1.0],
            [0.5, 0.75, 1.0],
            [0.5, 0.25, 1.0],
            [rim, 0.5, 0.5],
            [0.5, 0.75, 0.5],
            [0.5, 0.25, 0.5],
        ];
        // Outward, each with its holes: bottom, front, right, back, left,
        // the top round the pocket's rim, the pocket's walls and floor.
        let faces: [(&[usize], &[usize]); 10] = [
            (&[0, 3, 2, 1], &[]),
            (&[0, 1, 5, 4], &[]),
            (&[1, 2, 6, 5], &[]),
            (&[2, 3, 7, 6], &[]),
            (&[3, 0, 4, 7], &[]),
            (&[4, 5, 6, 7], &[8, 10, 9]),
            (&[10, 8, 11, 13], &[]),
            (&[9, 10, 13, 12], &[]),
            (&[8, 9, 12, 11], &[]),
            (&[11, 12, 13], &[]),
        ];
        let mut data = String::new();
        for (index, [x, y, z]) in points.iter().enumerate() {
            writeln!(
                data,
                "#{}=IFCCARTESIANPOINT(({x:?},{y:?},{z:?}));",
                100 + index
            )
            .unwrap();
        }
        let mut next = 200;
        let mut face_ids = Vec::new();
        for (outer, hole) in faces {
            let mut bounds = Vec::new();
            for (ring, kind) in [(outer, "IFCFACEOUTERBOUND"), (hole, "IFCFACEBOUND")] {
                if ring.is_empty() {
                    continue;
                }
                let corners: Vec<String> = ring.iter().map(|c| format!("#{}", 100 + c)).collect();
                writeln!(
                    data,
                    "#{next}=IFCPOLYLOOP(({}));\n#{}={kind}(#{next},.T.);",
                    corners.join(","),
                    next + 1
                )
                .unwrap();
                bounds.push(format!("#{}", next + 1));
                next += 2;
            }
            writeln!(data, "#{next}=IFCFACE(({}));", bounds.join(",")).unwrap();
            face_ids.push(format!("#{next}"));
            next += 1;
        }
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#2);\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
             #7=IFCUNITASSIGNMENT((#6));\n\
             #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
             {data}#60=IFCCLOSEDSHELL(({}));\n#61=IFCFACETEDBREP(#60);\n\
             #62=IFCSHAPEREPRESENTATION(#5,'Body','Brep',(#61));\n\
             #63=IFCPRODUCTDEFINITIONSHAPE($,$,(#62));\n\
             #90=IFCBUILDINGELEMENTPROXY('0000000000000000000090',$,$,$,$,#3,#63,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;\n",
            face_ids.join(",")
        )
    }

    #[test]
    fn a_t_junction_left_by_a_vertex_on_another_rings_edge_is_no_closed_solid() {
        // axiolid-mesh-compile 0.3.14 triangulates a B-rep face whose
        // rings touch (#262): the pocket's rim corner is inserted into the
        // top face's outer edge, but the right face, which shares that
        // edge, keeps it whole. The mesh then has a T-junction there,
        // although the B-rep's topology is closed and the compiler calls
        // it a solid. The mesh audit every service reads finds the edge
        // open, so the body is measured as an exact surface, never as a
        // closed solid: no volume, containment or inside-of-solid reading
        // trusts it.
        let health = |rim: f64| {
            let model = axioval::ifc::read_ifc_step(pocket_box(rim).as_bytes()).unwrap();
            let units = ifc_geometry::units::resolve(&model);
            let backend = ifc_geometry::compile::default_backend();
            let body = super::mesh(
                &backend,
                &model,
                &units,
                &super::Linear::default(),
                ifc_model::EntityId(90),
                false,
                super::NetOptions::default(),
            )
            .unwrap()
            .expect("a body");
            assert_eq!(body.fit, super::Fit::Exact);
            axiolid_mesh::audit_mesh(&body.mesh, super::TOLERANCE)
        };
        let touching = health(1.0);
        assert!(touching.is_surface_usable(), "{touching:?}");
        assert!(!touching.is_closed_two_manifold(), "{touching:?}");
        assert!(touching.boundary_edges > 0, "{touching:?}");
        // The same pocket clear of the edge closes.
        let clear = health(0.9);
        assert!(clear.is_closed_two_manifold(), "{clear:?}");
    }

    /// A polygon-mesh unit box with its corner `(1, 1, 1)` lifted 5 cm,
    /// placed by `transform`, in a graph with a block when `cut` names
    /// which operand of a difference the box is.
    fn warped_graph(
        transform: axiolid_core::Transform3,
        cut: Option<bool>,
    ) -> (axiolid_model::GeometryGraph, axiolid_model::NodeId) {
        use axiolid_core::Vec3;
        use axiolid_mesh::{PolygonFace, PolygonMesh};
        use axiolid_model::{GeometryGraphBuilder, GeometryNode, Instance, SolidOperation};
        let mut corners = [Vec3::ZERO; 8];
        for (index, corner) in corners.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let bit = |shift: usize| ((index >> shift) & 1) as f64;
            *corner = Vec3::new(bit(0), bit(1), bit(2));
        }
        corners[7].z += 0.05;
        let face = |outer: [u32; 4]| PolygonFace {
            outer: outer.to_vec(),
            holes: Vec::new(),
        };
        let mesh = PolygonMesh {
            positions: corners.to_vec(),
            faces: vec![
                face([0, 2, 3, 1]),
                face([4, 5, 7, 6]),
                face([0, 1, 5, 4]),
                face([1, 3, 7, 5]),
                face([3, 2, 6, 7]),
                face([2, 0, 4, 6]),
            ],
        };
        let mut builder = GeometryGraphBuilder::new();
        let mesh = builder.push(GeometryNode::PolygonMesh(mesh)).unwrap();
        let mut root = builder
            .push(GeometryNode::Instance(Instance {
                source: mesh,
                transform,
            }))
            .unwrap();
        if let Some(subject) = cut {
            let block = builder
                .push(GeometryNode::Primitive(
                    axiolid_primitive::Primitive::Block {
                        x: 0.5,
                        y: 0.5,
                        z: 0.5,
                    },
                ))
                .unwrap();
            let (left, right) = if subject {
                (root, block)
            } else {
                (block, root)
            };
            root = builder
                .push(GeometryNode::SolidOperation(SolidOperation::Boolean {
                    left,
                    right,
                    operator: axiolid_core::BooleanOperator::Difference,
                }))
                .unwrap();
        }
        (builder.finish(vec![root]).unwrap(), root)
    }

    #[test]
    fn a_reported_warp_grows_with_its_placement_and_refuses_a_boolean() {
        use axiolid_core::{Transform3, Vec3};
        let backend = ifc_geometry::compile::default_backend();
        let fit = |transform, cut| {
            let (graph, root) = warped_graph(transform, cut);
            super::compile(&backend, &graph, root).map(|(_, fit)| fit)
        };
        let within = |fit: Result<super::Fit, String>| match fit {
            Ok(super::Fit::Within(bound)) => bound,
            other => panic!("{other:?}"),
        };
        let plain = within(fit(Transform3::IDENTITY, None));
        assert!(0.02 < plain && plain <= 0.025 + 1e-12, "{plain}");
        // A rotation keeps it (up to the stretch bound's margin), a scale
        // by two doubles it.
        let turned = within(fit(Transform3::from_rotation_z(0.6), None));
        assert!(
            plain * (1.0 - 1e-9) <= turned && turned <= plain * (1.0 + 1e-9),
            "{plain} {turned}"
        );
        let scaled = within(fit(Transform3::from_scale(Vec3::splat(2.0)), None));
        assert!(
            2.0 * plain * (1.0 - 1e-9) <= scaled && scaled <= 2.0 * plain * (1.0 + 1e-9),
            "{plain} {scaled}"
        );
        // Cutting or being cut, the warped box leaves the body unmeasured:
        // the compiler bounds no boolean of a polygon mesh, since the exact
        // compiler it measures booleans against refuses one.
        for subject in [true, false] {
            let (graph, root) = warped_graph(Transform3::IDENTITY, Some(subject));
            if let Ok((_, report)) = backend.compile_mesh_with_deviation(
                &graph,
                root,
                &super::ExecutionOptions::new(super::TOLERANCE),
            ) {
                assert_eq!(report.bound, None, "{report:?}");
            }
            let refusal = fit(Transform3::IDENTITY, Some(subject)).unwrap_err();
            assert!(refusal.contains("operand of a boolean"), "{refusal}");
        }
    }

    /// engine#224, openbimrs/ifc#355: a product on a linear placement with
    /// a positive `OffsetLateral` lies to the LEFT of the basis curve's
    /// direction, and its local Z is up. The curve runs along +Y, so left
    /// is -X: a 1 m square column 3 m tall, 5 m along and 2 m left, spans
    /// x -2.5..-1.5, y 4.5..5.5 and z 0..3. Before 0.10 it moved right and
    /// its Z lay along the lateral.
    #[test]
    fn a_lateral_offset_places_a_product_left_of_its_curve_and_upright() {
        let model = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4X3_ADD2'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #4=IFCDIRECTION((0.,0.,1.));\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             #90=IFCPOLYLINE((#1,#91));\n\
             #91=IFCCARTESIANPOINT((0.,20.,0.));\n\
             #92=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(5.),2.,$,$,#90);\n\
             #93=IFCAXIS2PLACEMENTLINEAR(#92,$,$);\n\
             #94=IFCLINEARPLACEMENT($,#93,$);\n\
             #95=IFCCARTESIANPOINT((0.,0.));\n\
             #96=IFCAXIS2PLACEMENT2D(#95,$);\n\
             #97=IFCRECTANGLEPROFILEDEF(.AREA.,$,#96,1.,1.);\n\
             #98=IFCEXTRUDEDAREASOLID(#97,#2,#4,3.);\n\
             #99=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#98));\n\
             #100=IFCPRODUCTDEFINITIONSHAPE($,$,(#99));\n\
             #101=IFCBUILDINGELEMENTPROXY('0000000000000000000101',$,$,$,$,#94,#100,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;\n";
        let model = axioval::ifc::read_ifc_step(model.as_bytes()).unwrap();
        let units = ifc_geometry::units::resolve(&model);
        let linear = super::Linear::scan(&model, &units);
        assert!(linear.refused.is_empty(), "{linear:?}");
        let backend = ifc_geometry::compile::default_backend();
        let body = super::mesh(
            &backend,
            &model,
            &units,
            &linear,
            ifc_model::EntityId(101),
            false,
            super::NetOptions::default(),
        )
        .unwrap()
        .expect("a body");
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for point in &body.mesh.positions {
            for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        for (got, want) in min
            .into_iter()
            .chain(max)
            .zip([-2.5, 4.5, 0.0, -1.5, 5.5, 3.0])
        {
            assert!((got - want).abs() < 1e-9, "{min:?} {max:?}");
        }
    }

    /// engine#218: only IFC4 and IFC4X3 state that a `Reference`
    /// representation of an opening is not subtracted.
    #[test]
    fn reference_openings_are_taken_as_applied_only_where_the_release_says_so() {
        use ifc_geometry::lower::ReferenceOnlyOpenings;
        for (schema, policy) in [
            (Some("IFC4"), ReferenceOnlyOpenings::TakeAsApplied),
            (Some("IFC4X3"), ReferenceOnlyOpenings::TakeAsApplied),
            (Some("IFC2X3"), ReferenceOnlyOpenings::Refuse),
            (Some("IFC4X1"), ReferenceOnlyOpenings::Refuse),
            (None, ReferenceOnlyOpenings::Refuse),
        ] {
            assert_eq!(
                super::net_options(schema).reference_only_openings,
                policy,
                "{schema:?}"
            );
        }
    }

    /// An opening taken as applied has its `Reference` solid as its void,
    /// placed as its representation is: a 1 m prism at x 1.5..2.5 lifted
    /// 2 m by the opening's placement.
    #[test]
    fn an_applied_openings_void_is_its_reference_solid() {
        let model = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #4=IFCDIRECTION((0.,0.,1.));\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             #6=IFCCARTESIANPOINT((0.,0.,2.));\n\
             #7=IFCAXIS2PLACEMENT3D(#6,$,$);\n\
             #8=IFCLOCALPLACEMENT($,#7);\n\
             #200=IFCRECTANGLEPROFILEDEF(.AREA.,$,#201,1.,1.);\n\
             #201=IFCAXIS2PLACEMENT2D(#202,$);\n\
             #202=IFCCARTESIANPOINT((2.,0.));\n\
             #205=IFCEXTRUDEDAREASOLID(#200,#2,#4,1.);\n\
             #206=IFCSHAPEREPRESENTATION(#5,'Reference','SweptSolid',(#205));\n\
             #207=IFCPRODUCTDEFINITIONSHAPE($,$,(#206));\n\
             #208=IFCOPENINGELEMENT('0000000000000000000208',$,$,$,$,#8,#207,$,.OPENING.);\n\
             #210=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#205));\n\
             #211=IFCPRODUCTDEFINITIONSHAPE($,$,(#206,#210));\n\
             #212=IFCOPENINGELEMENT('0000000000000000000212',$,$,$,$,#8,#211,$,.OPENING.);\n\
             ENDSEC;\nEND-ISO-10303-21;\n";
        let model = axioval::ifc::read_ifc_step(model.as_bytes()).unwrap();
        let units = ifc_geometry::units::resolve(&model);
        let backend = ifc_geometry::compile::default_backend();
        let (mesh, fit) = super::reference_void(
            &backend,
            &model,
            &units,
            &super::Linear::default(),
            ifc_model::EntityId(208),
        )
        .unwrap();
        assert_eq!(fit, super::Fit::Exact);
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for point in &mesh.positions {
            for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        for (got, want) in min
            .into_iter()
            .chain(max)
            .zip([1.5, -0.5, 2.0, 2.5, 0.5, 3.0])
        {
            assert!((got - want).abs() < 1e-9, "{min:?} {max:?}");
        }
        // An opening with a `Body` beside its `Reference` was never taken
        // as applied; its void is refused here.
        let refusal = super::reference_void(
            &backend,
            &model,
            &units,
            &super::Linear::default(),
            ifc_model::EntityId(212),
        )
        .unwrap_err();
        assert!(refusal.contains("not 'Reference'"), "{refusal}");
    }

    /// A closed, outward box from `min` to `max`.
    fn cuboid([x0, y0, z0]: [f64; 3], [x1, y1, z1]: [f64; 3]) -> axiolid_mesh::TriMesh {
        use axiolid_core::Point3;
        axiolid_mesh::TriMesh::new(
            vec![
                Point3::new(x0, y0, z0),
                Point3::new(x1, y0, z0),
                Point3::new(x1, y1, z0),
                Point3::new(x0, y1, z0),
                Point3::new(x0, y0, z1),
                Point3::new(x1, y0, z1),
                Point3::new(x1, y1, z1),
                Point3::new(x0, y1, z1),
            ],
            vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0,
                7, 3, 1, 2, 6, 1, 6, 5,
            ],
        )
    }

    /// Two meshes as one, unjoined.
    fn joined(
        first: &axiolid_mesh::TriMesh,
        second: &axiolid_mesh::TriMesh,
    ) -> axiolid_mesh::TriMesh {
        let offset = u32::try_from(first.positions.len()).unwrap();
        let mut positions = first.positions.clone();
        positions.extend(second.positions.iter().copied());
        let mut indices = first.indices.clone();
        indices.extend(second.indices.iter().map(|index| index + offset));
        axiolid_mesh::TriMesh::new(positions, indices)
    }

    /// Whether `opening` cuts a part meshed as `part` within `fit`.
    fn decide(
        part: axiolid_mesh::TriMesh,
        fit: super::Fit,
        opening: &axiolid_mesh::TriMesh,
    ) -> super::Cut {
        use axioval::ir::SourceId;
        let source = SourceId::new("ifc-step", "model.ifc").unwrap();
        let body = super::Body {
            mesh: part,
            fit,
            boundary: None,
            applied: Vec::new(),
            taken: Vec::new(),
        };
        super::cuts(
            &ObjectId::new(source.clone(), "#1").unwrap(),
            &body,
            &ObjectId::new(source, "#2").unwrap(),
            opening,
            super::Fit::Exact,
        )
    }

    /// A whole's opening cuts a part only where it reaches into its
    /// material beyond the touching depth (#223): an opening through an
    /// uncut part or inside it cuts; one flush with a hole already cut,
    /// within the depth of it, or apart, does not; a part inside the
    /// opening is told apart; and a tessellated part merely touching the
    /// opening is undecided, never taken as cut or clear.
    #[test]
    fn a_wholes_opening_cuts_a_part_only_where_it_reaches_into_its_material() {
        use super::{Cut, Fit};
        let door = cuboid([1.5, -0.2, 0.0], [2.5, 0.2, 2.1]);

        // An uncut layer through the door, exact or tessellated.
        let layer = cuboid([0.0, -0.1, 0.0], [4.0, 0.0, 3.0]);
        assert_eq!(decide(layer.clone(), Fit::Exact, &door), Cut::Cuts);
        assert_eq!(decide(layer, Fit::Within(0.001), &door), Cut::Cuts);
        // The layer's piece left of the hole already cut: flush.
        let left = cuboid([0.0, -0.1, 0.0], [1.5, 0.0, 3.0]);
        assert_eq!(decide(left.clone(), Fit::Exact, &door), Cut::Clear);
        // Apart, and a block within the door.
        let apart = cuboid([3.0, -0.1, 0.0], [4.0, 0.0, 3.0]);
        assert_eq!(decide(apart, Fit::Exact, &door), Cut::Clear);
        let block = cuboid([1.9, -0.1, 0.5], [2.1, 0.0, 1.5]);
        assert_eq!(decide(block, Fit::Exact, &door), Cut::Inside);
        // A tessellated piece flush with the hole: its true surface may
        // reach into the door by its deviation.
        assert!(
            matches!(decide(left, Fit::Within(0.001), &door), Cut::Undecided(_)),
            "a tessellated touch is undecided"
        );
        // Written 5 µm into the hole's jamb, the opening only touches the
        // piece; 50 µm into it, it cuts.
        for (reach, expected) in [(5e-6, Cut::Clear), (5e-5, Cut::Cuts)] {
            let piece = cuboid([0.0, -0.1, 0.0], [1.5 + reach, 0.0, 3.0]);
            assert_eq!(decide(piece, Fit::Exact, &door), expected, "{reach}");
        }
        // A sliver of an opening across the jamb, nowhere 20 µm thick,
        // holds no core: it cuts nothing beyond the touching depth.
        let sliver = axiolid_mesh::TriMesh::new(
            vec![
                axiolid_core::Point3::new(1.499_995, -0.1, 0.0),
                axiolid_core::Point3::new(1.499_995, 0.0, 0.0),
                axiolid_core::Point3::new(1.499_995, -0.05, 1.0),
                axiolid_core::Point3::new(1.500_005, -0.05, 0.5),
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3],
        );
        let left = cuboid([0.0, -0.1, 0.0], [1.5, 0.0, 3.0]);
        assert_eq!(decide(left, Fit::Exact, &sliver), Cut::Clear);
        // An opening wholly inside a part's material (a cavity) cuts it.
        let cavity = cuboid([1.0, -0.15, 0.5], [1.2, -0.05, 0.7]);
        let solid = cuboid([0.0, -0.2, 0.0], [2.0, 0.0, 1.0]);
        assert_eq!(decide(solid, Fit::Exact, &cavity), Cut::Cuts);
    }

    /// An opening of several pieces is read piece by piece, and one of a
    /// profile that is not convex prism by prism (#223): a frame and leaf
    /// written as two boxes, and an L-shaped opening whose notch a part
    /// fills flush, are decided as their shapes are, not as their boxes.
    #[test]
    fn a_wholes_opening_of_several_pieces_or_any_profile_is_read_by_its_shape() {
        use super::{Cut, Fit};
        use axiolid_core::Point3;

        let framed = joined(
            &cuboid([1.5, -0.2, 0.0], [1.6, 0.2, 2.1]),
            &cuboid([1.6, -0.2, 0.0], [2.5, 0.2, 2.1]),
        );
        let left = cuboid([0.0, -0.1, 0.0], [1.5, 0.0, 3.0]);
        assert_eq!(decide(left, Fit::Exact, &framed), Cut::Clear);
        let layer = cuboid([0.0, -0.1, 0.0], [4.0, 0.0, 3.0]);
        assert_eq!(decide(layer.clone(), Fit::Exact, &framed), Cut::Cuts);

        // The L of x 1.5..2.5, y -0.2..0.2 less x 1.5..2.0, y 0..0.2,
        // extruded 2.1 m up; caps triangulated, sides as quads.
        let profile = [
            [1.5, -0.2],
            [2.5, -0.2],
            [2.5, 0.2],
            [2.0, 0.2],
            [2.0, 0.0],
            [1.5, 0.0],
        ];
        let mut positions: Vec<Point3> = profile
            .iter()
            .map(|[x, y]| Point3::new(*x, *y, 0.0))
            .collect();
        positions.extend(profile.iter().map(|[x, y]| Point3::new(*x, *y, 2.1)));
        let mut indices = Vec::new();
        for [a, b, c] in [[0u32, 1, 4], [1, 2, 4], [2, 3, 4], [0, 4, 5]] {
            indices.extend([a, c, b, a + 6, b + 6, c + 6]);
        }
        for i in 0..6u32 {
            let j = (i + 1) % 6;
            indices.extend([i, j, j + 6, i, j + 6, i + 6]);
        }
        let notched = axiolid_mesh::TriMesh::new(positions, indices);
        // One prism per cap triangle, each within the opening.
        let cells = super::OpeningCell::all(&notched);
        assert_eq!(cells.len(), 4, "{cells:?}");
        assert!(cells.iter().all(|cell| cell.within), "{cells:?}");
        // A layer filling the notch, flush with the L's inner faces, is
        // clear of every prism; its box would hold it.
        let filling = cuboid([0.0, 0.0, 0.0], [2.0, 0.1, 3.0]);
        for cell in &cells {
            assert_eq!(
                super::cell_cut(&filling, cell),
                Some(Cut::Clear),
                "{cell:?}"
            );
        }
        assert_eq!(decide(filling, Fit::Exact, &notched), Cut::Clear);
        assert_eq!(decide(layer, Fit::Exact, &notched), Cut::Cuts);
    }
}
