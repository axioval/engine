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
//! A host is meshed net of its openings (`lower_product_net_with`). In an
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
use axiolid_core::Tolerance;
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
    AlignmentServiceHandle, BoundaryCoverageServiceHandle, ContactServiceHandle,
    CoordinateSystemServiceHandle, DerivedRelationshipServiceHandle,
    EnvelopeMembershipServiceHandle, EvidenceSession, FacadeAreaServiceHandle,
    FreeSpaceServiceHandle, GuardServiceHandle, LinearQuantityServiceHandle,
    MetricRoutingServiceHandle, PlanAreaServiceHandle, PlanSpanServiceHandle, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, ProximityServiceHandle,
    RelationshipEdgesRequest, RelationshipQuery, RelationshipSelectionRequest,
    RelationshipSelectionServiceHandle, SemanticRelationship, SightServiceHandle, SourceSnapshot,
    SpaceServiceHandle, TraversalDirection, TriangleCountServiceHandle, TypeHierarchyServiceHandle,
    VerticalExtentServiceHandle, WalkabilityServiceHandle, WalkingSurfaceServiceHandle,
};
use axioval::ir::{ObjectId, PropertyValue, Report, SourceId};
use axioval::rules::{CoordinateTolerance, compare_coordinate_systems};
use axioval::{bcf, bcf_snapshot};
use ifc_geometry::constraint::local::PlacementResolver;
use ifc_geometry::lower::{
    AppliedReason, LoweringSession, NetOptions, ReferenceOnlyOpenings, lower_connection_surface,
    lower_product_net_with, lower_representation_item,
};
use ifc_geometry::{CachedPositionPolicy, GeometryError, RepresentationPurpose, Transform};
use ifc_model::{EntityId, Model};
use ifc_spatial::relation::boundary::{ConnectionGeometryAnomaly, SpaceBoundary};
use ifc_spatial::{SpatialAnomaly, SpatialKind, SpatialTree};
use std::sync::Arc;

mod alignment;

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
    /// ([`NO_SHAPE_REPRESENTATION`]). Its measurements stay not evaluated.
    pub model_data: Vec<ModelData>,
    /// Openings taken as already applied to a measured host's `Body`:
    /// host, opening and the reason, in identity order.
    pub applied_openings: Vec<(ObjectId, ObjectId, String)>,
    /// Every meshed object's triangles, kept only when asked for, to draw
    /// BCF snapshots from.
    pub meshes: BTreeMap<ObjectId, bcf_snapshot::Mesh>,
}

/// The integrity code of a physical product with no shape representation
/// and no parts: it has no geometry at all, so it is unmeasured, never
/// measured as empty.
pub const NO_SHAPE_REPRESENTATION: &str = "shape.no-representation";

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
}

impl Options {
    /// Meshes only, keeping them when `keep` is set.
    pub fn meshes(keep: bool) -> Self {
        Self {
            keep_meshes: keep,
            exact_boundaries: false,
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
    let parsed = parse(&snapshots, models)?;
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
        match keep(&mut report, options.keep_meshes.then_some(&id), meshed) {
            Ok(Some(body)) => {
                if !body.applied.is_empty() {
                    let openings = body.applied.iter().map(|(opening, _)| opening.clone());
                    geometry = geometry.with_applied_openings(id.clone(), openings.collect());
                    for (opening, reason) in &body.applied {
                        applied_openings.insert(opening.clone());
                        report.applied_openings.push((
                            id.clone(),
                            opening.clone(),
                            (*reason).to_owned(),
                        ));
                    }
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

    geometry = with_boundaries(geometry, boundaries, &mut report);
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

    let relationships = session.service::<RelationshipSelectionServiceHandle>();
    geometry = Composer {
        geometry,
        parts: decompositions(relationships, &kinds),
        wholes: wholes.iter().cloned().collect(),
        bodiless: unbodied,
        stated,
        decided: BTreeSet::new(),
        report: &mut report,
        keep_meshes: options.keep_meshes,
    }
    .compose_all(&wholes);
    report.unmeasured.sort_by(|a, b| a.0.cmp(&b.0));
    report.model_data = shapeless_records(&report.unmeasured, &snapshots, &kinds);
    for (group, members) in groups(relationships, &kinds, &is_a) {
        geometry = match members {
            Ok(members) => geometry.with_group(group, members),
            Err(reason) => geometry.with_undecided_group(group, reason),
        };
    }
    let envelope = envelope_service(&session, &geometry, &source, &kinds)?;
    let space = (
        space_service(&parsed, &geometry, &source, &kinds, &is_a),
        linear_service(&geometry, &voids),
        boundary_service(&backend, &parsed, &geometry, &kinds, &is_a),
        plan_area_service(&geometry, &source, &voids),
    );
    let routes = route_services(&geometry, &source, &kinds, &is_a, &voids);
    let derived = derived_service(&geometry, &parsed, &kinds, &is_a, voids);
    // Sections cut, and envelopes are swept past, the same bodies every
    // other service measures.
    let alignments = alignment::alignment_service(
        &parsed,
        &kinds.keys().cloned().collect::<Vec<_>>(),
        geometry.clone(),
    );
    let facade = facade_service(&geometry, &kinds, &is_a);
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
                let geometry = std::mem::take(&mut self.geometry);
                self.geometry = geometry.with_composed_body(whole.clone(), body);
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

/// Every product left unmeasured for having no shape representation (and
/// no parts), once each, as an integrity warning about the model data.
fn shapeless_records(
    unmeasured: &[(ObjectId, String)],
    snapshots: &[SourceSnapshot],
    kinds: &BTreeMap<ObjectId, String>,
) -> Vec<ModelData> {
    let shapeless = Bodiless::Shapeless.reason();
    unmeasured
        .iter()
        .filter(|(_, reason)| *reason == shapeless)
        .map(|(id, _)| {
            let fingerprint = snapshots
                .iter()
                .find(|snapshot| *snapshot.source() == id.source)
                .map_or("", SourceSnapshot::fingerprint);
            let kind = kinds.get(id).map_or("", String::as_str);
            ModelData {
                code: NO_SHAPE_REPRESENTATION,
                message: format!(
                    "{} {kind} has no shape representation and no parts; every \
                     measurement of it is not evaluated",
                    id.local_id
                ),
                locator: format!("ifc:{fingerprint}:no-shape:{}", id.local_id),
            }
        })
        .collect()
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
    for (id, boundary, _) in agreeing {
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
    envelope: AxiolidEnvelopeMembershipService,
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
fn envelope_service(
    session: &EvidenceSession,
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
) -> Result<AxiolidEnvelopeMembershipService, Box<dyn Error>> {
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .ok_or("the session has no property service to read declarations with")?;
    let value = |object: &ObjectId, name: &str| -> Option<PropertyValue> {
        let request = PropertyRequest::try_new(object.clone(), None, name).ok()?;
        match properties.resolve(&request).ok()? {
            PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
            PropertyResolution::Absent(_) => None,
        }
    };

    let mut service = AxiolidEnvelopeMembershipService::new(geometry.clone(), source.clone());
    for object in kinds.keys() {
        if geometry.mesh(object).is_none() {
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
    Ok(service)
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
                return Err("mesh compilation produced no triangles".into());
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
            return Err("mesh compilation produced no triangles".into());
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
        return Err("mesh compilation produced no triangles".into());
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

/// `meshed`, its triangles kept in `report` under `id` when given.
fn keep(report: &mut GeometryReport, id: Option<&ObjectId>, meshed: Meshed) -> Meshed {
    if let (Some(id), Ok(Some(body))) = (id, &meshed)
        && let Some(kept) = snapshot_mesh(&body.mesh)
    {
        report.meshes.insert(id.clone(), kept);
    }
    meshed
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
    if let Some(reason) = linear.refusal(model, product) {
        return Err(reason);
    }
    let mut session = session(model, units);
    let Some(net) =
        lower_product_net_with(&mut session, product, net).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let lowered = session.finish(net.root).map_err(|e| e.to_string())?;
    let (mesh, fit) = compile(backend, &lowered.graph, lowered.root)?;
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
}
