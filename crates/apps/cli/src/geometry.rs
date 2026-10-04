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
//! is too, with a reason naming the first such part; a product with no body
//! and no parts stays unmeasured as `no body representation`. The whole
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
//! Relationships derived from geometry (`axioval:derived.*`) need to know
//! which objects are spaces and which are doors, windows or openings. An
//! opening occupies no material, so its void is meshed separately and handed
//! to the derivation alone.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use axiolid_contracts::{ExecutionOptions, GeomError};
use axiolid_core::Tolerance;
use axiolid_curve::{Curve2, Curve3};
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
    BoundaryCoverageServiceHandle, ContactServiceHandle, CoordinateSystemServiceHandle,
    DerivedRelationshipServiceHandle, EnvelopeMembershipServiceHandle, EvidenceSession,
    FacadeAreaServiceHandle, FreeSpaceServiceHandle, GuardServiceHandle,
    LinearQuantityServiceHandle, MetricRoutingServiceHandle, PlanAreaServiceHandle,
    PlanSpanServiceHandle, PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle,
    ProximityServiceHandle, RelationshipEdgesRequest, RelationshipQuery,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, SemanticRelationship,
    SightServiceHandle, SourceSnapshot, SpaceServiceHandle, TraversalDirection,
    TriangleCountServiceHandle, TypeHierarchyServiceHandle, VerticalExtentServiceHandle,
    WalkabilityServiceHandle, WalkingSurfaceServiceHandle,
};
use axioval::ir::{ObjectId, PropertyValue, Report, SourceId};
use axioval::rules::{CoordinateTolerance, compare_coordinate_systems};
use axioval::{bcf, bcf_snapshot};
use ifc_geometry::lower::{LoweringSession, lower_connection_surface, lower_product_net};
use ifc_geometry::{RepresentationPurpose, Transform};
use ifc_model::{EntityId, Model};
use ifc_spatial::relation::boundary::{ConnectionGeometryAnomaly, SpaceBoundary};
use ifc_spatial::{SpatialAnomaly, SpatialKind, SpatialTree};
use std::sync::Arc;

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
    /// Every meshed object's triangles, kept only when asked for, to draw
    /// BCF snapshots from.
    pub meshes: BTreeMap<ObjectId, bcf_snapshot::Mesh>,
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
        parsed.insert(source.clone(), Parsed { model, units });
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
    // The box each whole's `Box` representation states, if any.
    let mut stated: BTreeMap<ObjectId, ([f64; 3], [f64; 3])> = BTreeMap::new();

    for object in session.project().objects() {
        let id = object.id.clone();
        let Some(Parsed { model, units }) = parsed.get(&id.source) else {
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
                    .and_then(|entity| mesh(&backend, model, units, entity, false))
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
        let meshed = mesh(&backend, model, units, entity, options.exact_boundaries);
        match keep(&mut report, options.keep_meshes.then_some(&id), meshed) {
            Ok(Some(body)) => {
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
                if let Some(bound) = stated_box(model, units, entity) {
                    stated.insert(id.clone(), bound);
                }
                wholes.push(id);
            }
            Err(error) => {
                report.unmeasured.push((id.clone(), error.clone()));
                geometry = geometry.with_unmeasured(id.clone(), error);
                if let Some((min, max)) = stated_box(model, units, entity) {
                    geometry = geometry.with_unmeasured_bound(id, min, max);
                }
            }
        }
    }

    geometry = with_boundaries(geometry, boundaries, &mut report);

    let relationships = session.service::<RelationshipSelectionServiceHandle>();
    geometry = Composer {
        geometry,
        parts: decompositions(relationships, &kinds),
        wholes: wholes.iter().cloned().collect(),
        stated,
        decided: BTreeSet::new(),
        report: &mut report,
        keep_meshes: options.keep_meshes,
    }
    .compose_all(&wholes);
    report.unmeasured.sort_by(|a, b| a.0.cmp(&b.0));
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
    let facade = facade_service(&geometry, &kinds, &is_a);
    let session = register(session, &snapshots, geometry, space, envelope, routes)?
        .with_host_service(FacadeAreaServiceHandle::new(Arc::new(facade)), &snapshots)?
        .with_derived_relationships(
            DerivedRelationshipServiceHandle::new(Arc::new(derived)),
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
        let parts = match &self.parts {
            Ok(decompositions) => decompositions.get(whole).cloned().unwrap_or_default(),
            Err(error) => {
                let reason = format!(
                    "no body representation, and whether it decomposes into parts that                      carry its body cannot be read: {error}"
                );
                self.unmeasured(whole, reason, None);
                return;
            }
        };
        if parts.is_empty() {
            self.unmeasured(whole, "no body representation".to_owned(), None);
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
        let (Some(entity), Some(Parsed { model, units })) = (entity_id(id), parsed.get(&id.source))
        else {
            continue;
        };
        let parent = trees
            .get(&id.source)
            .and_then(|tree| tree.node(entity))
            .and_then(|node| node.parent);
        let height = ifc_geometry::product_world_transform(model, units, entity)
            .map_err(|error| error.to_string())
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
    for (source, Parsed { model, units }) in parsed {
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
            service = match boundary_surface(backend, model, units, &boundary, &space) {
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
    let frame = space_frame(model, units, space)?;
    let mut session = LoweringSession::new(model, units);
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
/// Authored polygon faces warped off their plane by more than the
/// tolerance ([`warp`]) make either kind tessellated, declared within
/// twice their warp, and leave a body that cuts or is cut by one
/// unmeasured.
fn compile(
    backend: &Compiler,
    graph: &GeometryGraph,
    root: NodeId,
) -> Result<(axiolid_mesh::TriMesh, Fit), String> {
    let options = ExecutionOptions::new(TOLERANCE);
    let warp = warp(graph, root)?;
    if planar(graph, root, &mut NODE_BUDGET.clone()) {
        let mesh = backend
            .compile_mesh(graph, root, &options)
            .map_err(|error| compilation_refused(&error))?;
        if mesh.triangle_count() == 0 {
            return Err("mesh compilation produced no triangles".into());
        }
        return Ok((mesh, warp.map_or(Fit::Exact, Fit::Within)));
    }
    let (outcome, report) = backend
        .compile_mesh_with_deviation(graph, root, &options)
        .map_err(|error| compilation_refused(&error))?;
    let mesh = outcome.mesh;
    if mesh.triangle_count() == 0 {
        return Err("mesh compilation produced no triangles".into());
    }
    match report.bound {
        Some(bound) if bound.is_finite() && bound >= 0.0 => {
            Ok((mesh, Fit::Within(bound.max(warp.unwrap_or(0.0)))))
        }
        _ => Err(uncertified(&report)),
    }
}

/// How far the true surface of the authored polygon faces under `root`
/// (`PolygonMesh` faces, and B-rep faces given only by their loops) may
/// lie from their mesh, in metres, or `None` when every such face is
/// within the tolerance of its plane, which the mesh compiler counts as
/// planar.
///
/// A face whose corners leave its plane has no single true surface
/// (axiolid/kernel#254): the two triangulations of a quad, a bilinear
/// patch and the face flattened onto its plane are all readings of it.
/// The mesh compiler reports a polygon mesh face's warp `w` as the largest
/// distance of a corner from the face's fit plane (the outer ring's
/// centroid and Newell normal), which bounds the distance to the face
/// flattened onto that plane. Every reading through the face's corners
/// that keeps within their hull lies within the corners' spread across
/// that plane, which can be `2 w` (the diagonals of a quad warped by `w`
/// pass `w` above and below it), so the face is declared within `2 w`,
/// never `w`. A faceted B-rep face carries no surface, and the compiler
/// reports it planar however far its corners leave its plane, so the warp
/// is computed here for both kinds, as the compiler computes it, and
/// scaled by each placement's largest stretch.
///
/// A boolean cut by or cutting a warped face moves its section by more
/// than any bound on its operands covers (#235), so such a body is
/// refused, and so is a graph too large to walk.
fn warp(graph: &GeometryGraph, root: NodeId) -> Result<Option<f64>, String> {
    let mut walk = WarpWalk {
        budget: NODE_BUDGET,
        bound: None,
        under_boolean: None,
    };
    walk.visit(graph, root, 1.0, false)?;
    if let Some(warp) = walk.under_boolean {
        return Err(format!(
            "an authored polygon face warped {:.3} m off its plane is an operand of a \
             boolean, and nothing bounds how far the boolean's result lies from its mesh",
            warp / 2.0
        ));
    }
    Ok(walk.bound)
}

/// Why a face's corners could not be read to bound its warp.
const UNREADABLE_FACE: &str =
    "an authored polygon face's corners could not be read to bound how far it is warped";

/// The walk behind [`warp`]: authored faces enter a body as its items,
/// instances, collection members and boolean operands.
struct WarpWalk {
    budget: usize,
    /// The largest declared bound (twice the warp) outside booleans.
    bound: Option<f64>,
    /// The largest declared bound under a boolean.
    under_boolean: Option<f64>,
}

impl WarpWalk {
    fn visit(
        &mut self,
        graph: &GeometryGraph,
        id: NodeId,
        stretch: f64,
        boolean: bool,
    ) -> Result<(), String> {
        if self.budget == 0 {
            return Err(
                "the geometry graph is too large to bound the warp of its authored faces".into(),
            );
        }
        self.budget -= 1;
        let warp = match graph.get(id) {
            Some(GeometryNode::Instance(instance)) => {
                return self.visit(
                    graph,
                    instance.source,
                    stretch * largest_stretch(instance.transform),
                    boolean,
                );
            }
            Some(GeometryNode::Collection(children)) => {
                for child in children {
                    self.visit(graph, *child, stretch, boolean)?;
                }
                return Ok(());
            }
            Some(GeometryNode::SolidOperation(SolidOperation::Boolean { left, right, .. })) => {
                self.visit(graph, *left, stretch, true)?;
                return self.visit(graph, *right, stretch, true);
            }
            Some(GeometryNode::PolygonMesh(mesh)) => {
                let mut worst = None;
                for face in &mesh.faces {
                    let rings = std::iter::once(&face.outer)
                        .chain(&face.holes)
                        .map(|ring| {
                            ring.iter()
                                .map(|index| mesh.positions.get(*index as usize).copied())
                                .collect::<Option<Vec<_>>>()
                        })
                        .collect::<Option<Vec<_>>>()
                        .ok_or(UNREADABLE_FACE)?;
                    if let Some(warp) = face_warp(&rings) {
                        max_warp(&mut worst, warp);
                    }
                }
                worst
            }
            Some(GeometryNode::BRep(brep)) => {
                let mut worst = None;
                for face in brep.faces().iter().filter(|face| face.surface.is_none()) {
                    // The outer bound first: the fit plane is the outer
                    // ring's.
                    let mut rings = Vec::with_capacity(face.bounds.len());
                    for bound in face
                        .bounds
                        .iter()
                        .filter(|bound| bound.outer)
                        .chain(face.bounds.iter().filter(|bound| !bound.outer))
                    {
                        let wire = brep
                            .loops()
                            .get(bound.loop_id.index())
                            .ok_or(UNREADABLE_FACE)?;
                        // Each edge use's end shared with the next one's,
                        // so no orientation flag is read.
                        let ends = wire
                            .edges
                            .iter()
                            .map(|edge_use| {
                                let edge = brep.edges().get(edge_use.edge.index())?;
                                Some((edge.start.index(), edge.end.index()))
                            })
                            .collect::<Option<Vec<_>>>()
                            .ok_or(UNREADABLE_FACE)?;
                        let mut corners = Vec::with_capacity(ends.len());
                        for (index, &(start, end)) in ends.iter().enumerate() {
                            let (next_start, next_end) = ends[(index + 1) % ends.len()];
                            let shared = if start == next_start || start == next_end {
                                start
                            } else if end == next_start || end == next_end {
                                end
                            } else {
                                return Err(UNREADABLE_FACE.into());
                            };
                            let vertex = brep.vertices().get(shared).ok_or(UNREADABLE_FACE)?;
                            corners.push(vertex.position);
                        }
                        rings.push(corners);
                    }
                    if let Some(warp) = face_warp(&rings) {
                        max_warp(&mut worst, warp);
                    }
                }
                worst
            }
            _ => None,
        };
        if let Some(warp) = warp {
            let declared = 2.0 * warp * stretch;
            if boolean {
                max_warp(&mut self.under_boolean, declared);
            } else {
                max_warp(&mut self.bound, declared);
            }
        }
        Ok(())
    }
}

fn max_warp(worst: &mut Option<f64>, warp: f64) {
    *worst = Some(worst.map_or(warp, |worst| worst.max(warp)));
}

/// The largest distance of any corner of `rings` (the outer ring first)
/// from the plane through the outer ring's centroid along its Newell
/// normal, with the rounding of computing it, when it exceeds the
/// tolerance; `None` for a face within it, or one whose outer ring
/// encloses no area (the compiler's to refuse).
fn face_warp(rings: &[Vec<axiolid_core::Vec3>]) -> Option<f64> {
    let outer = rings.first().filter(|outer| outer.len() >= 3)?;
    let mut normal = axiolid_core::Vec3::ZERO;
    for (index, current) in outer.iter().enumerate() {
        let next = outer[(index + 1) % outer.len()];
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    let normal = normal.try_normalize()?;
    #[allow(clippy::cast_precision_loss)]
    let centroid = outer.iter().copied().sum::<axiolid_core::Vec3>() / outer.len() as f64;
    let (mut worst, mut reach) = (0.0_f64, 0.0_f64);
    for corner in rings.iter().flatten() {
        let offset = *corner - centroid;
        let distance = offset.dot(normal).abs();
        if !distance.is_finite() {
            return None;
        }
        worst = worst.max(distance);
        reach = reach.max(offset.length());
    }
    (worst > TOLERANCE.linear()).then_some(worst + 16.0 * f64::EPSILON * reach)
}

/// An upper bound on how much `transform` stretches a length: the square
/// root of the largest absolute row sum of `MᵀM` (Gershgorin), one for a
/// rotation up to the rounding the margin covers.
fn largest_stretch(transform: axiolid_core::Transform3) -> f64 {
    let m = transform.matrix3;
    let gram = m.transpose() * m;
    let largest = [gram.row(0), gram.row(1), gram.row(2)]
        .iter()
        .map(|row| row.abs().element_sum())
        .fold(0.0_f64, f64::max);
    largest.sqrt() * (1.0 + 1e-12)
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
    space: &ObjectId,
) -> Result<Transform, String> {
    let entity = entity_id(space).ok_or("the space is not a STEP instance")?;
    match ifc_geometry::product_representation_frame(
        model,
        units,
        entity,
        RepresentationPurpose::Body,
    ) {
        Ok(Some(frame)) => Ok(frame),
        Ok(None) => ifc_geometry::product_world_transform(model, units, entity)
            .map_err(|error| error.to_string()),
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
    product: EntityId,
) -> Option<([f64; 3], [f64; 3])> {
    let shape =
        ifc_geometry::Slots::new(product, model.get(product)?).opt_ref(PRODUCT_REPRESENTATION)?;
    let representations = ifc_geometry::ProductShape::new(shape, model.get(shape)?)
        .representations()
        .ok()?;
    let placement = ifc_geometry::product_world_transform(model, units, product).ok()?;
    let mut bound: Option<([f64; 3], [f64; 3])> = None;
    for id in representations {
        let representation = ifc_geometry::Representation::new(id, model.get(id)?);
        if !representation
            .identifier()
            .is_some_and(|identifier| identifier.eq_ignore_ascii_case("Box"))
        {
            continue;
        }
        let context = match ifc_geometry::context_of(model, id)
            .and_then(|context| context.world_coordinate_system(model))
        {
            Some(system) => ifc_geometry::resource::placement::axis_placement_transform(
                model,
                system,
                model.get(system)?,
            )
            .ok()?
            .to_metres(units),
            None => Transform::identity(),
        };
        let frame = context.compose(&placement);
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

/// One product's net body (openings subtracted), whether it is exact, and
/// with `boundary` its exact boundary where the lowered graph has one.
fn mesh(
    backend: &Compiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    product: EntityId,
    boundary: bool,
) -> Meshed {
    let mut session = LoweringSession::new(model, units);
    let Some(net) = lower_product_net(&mut session, product).map_err(|e| e.to_string())? else {
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
    }))
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
    use ifc_geometry::lower::{LoweringSession, lower_product_net};

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

    /// A unit box `[0, 1]^3` whose corner `(1, 1, 1)` is lifted by `lift`
    /// metres, as an `IfcFacetedBrep` (`faceted`) or an
    /// `IfcPolygonalFaceSet`, the body of proxy `#90`. The three faces
    /// meeting at the lifted corner are warped quads.
    fn lifted_box(lift: f64, faceted: bool) -> String {
        use std::fmt::Write as _;
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0 + lift],
            [0.0, 1.0, 1.0],
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

    /// How `lifted_box`'s body is meshed.
    fn lifted_fit(lift: f64, faceted: bool) -> Result<super::Fit, String> {
        let model = axioval::ifc::read_ifc_step(lifted_box(lift, faceted).as_bytes()).unwrap();
        let units = ifc_geometry::units::resolve(&model);
        let backend = ifc_geometry::compile::default_backend();
        super::mesh(&backend, &model, &units, ifc_model::EntityId(90), false)
            .map(|body| body.expect("a body").fit)
    }

    #[test]
    fn a_warped_authored_face_is_declared_within_twice_its_warp() {
        // axiolid/kernel#254: the top, front and side quads meeting at the
        // lifted corner are warped by `w` (about 1.25 cm) about their fit
        // planes. The polygon mesh compiles since axiolid-mesh-compile
        // 0.3.13 and the faceted B-rep always did (its faces carry no
        // surface, so the compiler reports them planar); either reading of
        // a quad (its two diagonals) is a triangulation through the same
        // corners, and they lie up to `2 w` apart across the plane.
        let lift: f64 = 0.05;
        // The top quad's two triangulations at its centre: the diagonal
        // through the lifted corner passes `lift / 2` above the other's
        // triangle `(1,0,1) (1,1,1+lift) (0,1,1)`, whose unit normal is
        // `(-lift, -lift, 1)` normalised.
        let apart = (lift / 2.0) / (1.0 + 2.0 * lift * lift).sqrt();
        for faceted in [true, false] {
            let Ok(super::Fit::Within(bound)) = lifted_fit(lift, faceted) else {
                panic!("{faceted}: {:?}", lifted_fit(lift, faceted));
            };
            assert!(
                apart <= bound && bound < lift,
                "{faceted}: {apart} <= {bound}"
            );
            // Within the tolerance a face counts as planar, as the
            // compiler counts it, and the body stays exact.
            for lift in [0.0, 5e-4] {
                assert_eq!(
                    lifted_fit(lift, faceted),
                    Ok(super::Fit::Exact),
                    "{faceted}"
                );
            }
        }
    }

    #[test]
    fn a_face_six_centimetres_out_of_plane_is_declared_within_twice_that() {
        // engine#213: a corner lifted 25 cm puts the warped quads about
        // 6 cm out of their fit planes (the compiler's own reading, which
        // its report states for the polygon mesh); the body is declared
        // within twice that, above 5 cm, the faceted B-rep alike.
        use axiolid_mesh_compile::DeviationPath;
        let lift = 0.25;
        let model = axioval::ifc::read_ifc_step(lifted_box(lift, false).as_bytes()).unwrap();
        let units = ifc_geometry::units::resolve(&model);
        let mut session = LoweringSession::new(&model, &units);
        let net = lower_product_net(&mut session, ifc_model::EntityId(90))
            .unwrap()
            .unwrap();
        let lowered = session.finish(net.root).unwrap();
        let (_, report) = ifc_geometry::compile::default_backend()
            .compile_mesh_with_deviation(
                &lowered.graph,
                lowered.root,
                &axiolid_contracts::ExecutionOptions::new(super::TOLERANCE),
            )
            .unwrap();
        let reported = report
            .contributions
            .iter()
            .filter(|contribution| contribution.path == DeviationPath::AuthoredMesh)
            .filter_map(|contribution| contribution.bound.value())
            .fold(0.0_f64, f64::max);
        assert!(reported > 0.05, "{report:?}");
        for faceted in [true, false] {
            let Ok(super::Fit::Within(bound)) = lifted_fit(lift, faceted) else {
                panic!("{faceted}: {:?}", lifted_fit(lift, faceted));
            };
            assert!(
                (bound - 2.0 * reported).abs() <= 1e-12,
                "{faceted}: {bound} is not twice {reported}"
            );
        }
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
    fn a_warp_grows_with_its_placement_and_refuses_a_boolean() {
        use axiolid_core::{Transform3, Vec3};
        let warp = |transform, cut| {
            let (graph, root) = warped_graph(transform, cut);
            super::warp(&graph, root)
        };
        let plain = warp(Transform3::IDENTITY, None).unwrap().unwrap();
        // A rotation keeps it (up to the stretch bound's margin), a scale
        // by two doubles it.
        let turned = warp(Transform3::from_rotation_z(0.6), None)
            .unwrap()
            .unwrap();
        assert!(
            plain <= turned && turned <= plain * (1.0 + 1e-9),
            "{plain} {turned}"
        );
        let scaled = warp(Transform3::from_scale(Vec3::splat(2.0)), None)
            .unwrap()
            .unwrap();
        assert!(2.0 * plain <= scaled && scaled <= 2.0 * plain * (1.0 + 1e-9));
        // Cutting or being cut, the warped box leaves the body unmeasured.
        for subject in [true, false] {
            let refusal = warp(Transform3::IDENTITY, Some(subject)).unwrap_err();
            assert!(refusal.contains("operand of a boolean"), "{refusal}");
        }
    }
}
