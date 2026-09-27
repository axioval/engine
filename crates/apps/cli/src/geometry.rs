//! IFC body geometry as Axiolid meshes, registered into a check's session.
//!
//! This is host composition, not an adapter: `axioval-ifc` must not know
//! Axiolid and `axioval-axiolid` must not know IFC, so the CLI reads the same
//! model bytes a second time, meshes each product with `ifc-geometry`, and
//! hands the meshes to the geometry services under the object identities the
//! IFC session already uses.
//!
//! With several models, every source is meshed into one geometry set under
//! its own source-qualified identities, in one shared coordinate system, and
//! every service is bound to all the session's snapshots. A clash between
//! objects of two files is then an ordinary pair of the set.
//!
//! Every object ends up in exactly one of three states, because the geometry
//! services treat them differently:
//!
//! - **meshed**, either exactly (every face planar, so the mesh is the shape)
//!   or as a tessellation within the compiler's chord budget;
//! - **no body**: it occupies no material (a storey, a zone, an opening);
//! - **unmeasured**: it is physical but could not be meshed. Measurements it
//!   could affect refuse rather than act as if it were not there.
//!
//! A group (a zone) has no body, but its plan footprint is the union of its
//! members', so the bridge also declares every group's membership.
//!
//! Relationships derived from geometry (`axioval:derived.*`) need to know
//! which objects are spaces and which are doors, windows or openings. An
//! opening occupies no material, so its void is meshed separately and handed
//! to the derivation alone.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use axiolid_contracts::ExecutionOptions;
use axiolid_core::Tolerance;
use axiolid_curve::{Curve2, Curve3};
use axiolid_mesh_compile_contract::MeshCompiler;
use axiolid_model::{GeometryGraph, GeometryNode, NodeId, SolidOperation};
use axiolid_primitive::Primitive;
use axiolid_profile::Profile;
use axiolid_surface::Surface;
use axioval::axiolid::{
    AxiolidContactService, AxiolidDerivedRelationshipService, AxiolidEnvelopeMembershipService,
    AxiolidFacadeAreaService, AxiolidFreeSpaceService, AxiolidGeometry, AxiolidGuardService,
    AxiolidLinearQuantityService, AxiolidMetricRoutingService, AxiolidPlanAreaService,
    AxiolidPlanSpanService, AxiolidProximityService, AxiolidSpaceService,
    AxiolidTriangleCountService, AxiolidVerticalExtentService, AxiolidWalkabilityService,
};
use axioval::engine::{
    ContactServiceHandle, DerivedRelationshipServiceHandle, EnvelopeMembershipServiceHandle,
    EvidenceSession, FacadeAreaServiceHandle, FreeSpaceServiceHandle, GuardServiceHandle,
    LinearQuantityServiceHandle, MetricRoutingServiceHandle, PlanAreaServiceHandle,
    PlanSpanServiceHandle, PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle,
    ProximityServiceHandle, RelationshipQuery, RelationshipSelectionRequest,
    RelationshipSelectionServiceHandle, SemanticRelationship, SourceSnapshot, SpaceServiceHandle,
    TraversalDirection, TriangleCountServiceHandle, TypeHierarchyServiceHandle,
    VerticalExtentServiceHandle, WalkabilityServiceHandle,
};
use axioval::ir::{ObjectId, PropertyValue, SourceId};
use ifc_geometry::lower::{LoweringSession, lower_product_net};
use ifc_model::{Codec, EntityId, Model};
use ifc_spatial::{SpatialAnomaly, SpatialKind, SpatialTree};
use ifc_step::StepCodec;
use std::sync::Arc;

/// Linear tolerance handed to the mesh compiler. With no explicit chord
/// budget, curved geometry stays within this distance of the true surface,
/// which is the deviation declared for every tessellated mesh.
const TOLERANCE: Tolerance = Tolerance::MILLIMETRE;
const CHORD_DEVIATION_METRES: f64 = 1e-3;

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
    /// Physical objects that could not be meshed, with the reason.
    pub unmeasured: Vec<(ObjectId, String)>,
}

/// Each source's model bytes, keyed by the source the session imported them as.
pub type ModelBytes = BTreeMap<SourceId, Vec<u8>>;

/// One parsed model and its unit scale, per source.
struct Parsed {
    model: Model,
    units: ifc_geometry::units::UnitScale,
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
        let model = StepCodec
            .read_bytes(bytes)
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
/// imported them as. Policy choices IFC does not state, such as which
/// surfaces are walkable or which spaces bound the envelope, are the rules'
/// own selections, carried in each request; the bridge declares none of them.
///
/// # Errors
///
/// Returns an error when a source has no bytes, the bytes do not parse (the
/// session already parsed them, so this means they changed) or a service
/// cannot be registered.
pub fn attach(
    session: EvidenceSession,
    models: &ModelBytes,
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

    let backend = ifc_geometry::compile::default_backend();
    let mut geometry = AxiolidGeometry::new();
    let mut report = GeometryReport::default();
    let mut voids: Vec<(ObjectId, Void)> = Vec::new();

    for object in session.project().objects() {
        let id = object.id.clone();
        let Some(Parsed { model, units }) = parsed.get(&id.source) else {
            return Err(format!("no model for source `{}`", id.source).into());
        };
        let is_space = is_a(&id, "IfcSpace");
        let bodiless = !is_a(&id, "IfcProduct")
            || (!is_space && NO_BODY.iter().any(|ancestor| is_a(&id, ancestor)));
        if bodiless {
            if is_a(&id, "IfcOpeningElement") {
                let void = entity_id(&id)
                    .ok_or_else(|| "not a STEP instance id".to_owned())
                    .and_then(|entity| mesh(&backend, model, units, entity))
                    .and_then(|meshed| meshed.ok_or_else(|| "no body representation".into()));
                voids.push((id.clone(), void));
            }
            geometry = geometry.with_no_body(id);
            report.no_body += 1;
            continue;
        }
        let Some(entity) = entity_id(&id) else {
            report
                .unmeasured
                .push((id.clone(), "not a STEP instance id".into()));
            geometry = geometry.with_unmeasured(id, "not a STEP instance id");
            continue;
        };
        match mesh(&backend, model, units, entity) {
            Ok(Some((mesh, true))) => {
                geometry = geometry.with_mesh(id, mesh);
                report.exact += 1;
            }
            Ok(Some((mesh, false))) => {
                geometry = geometry.with_tessellated_mesh(id, mesh, CHORD_DEVIATION_METRES);
                report.tessellated += 1;
            }
            // A space without a body is still no material; it cannot be
            // measured itself, but it obstructs nothing.
            Ok(None) if is_space => {
                geometry = geometry.with_no_body(id);
                report.no_body += 1;
            }
            Ok(None) => {
                let reason = "no body representation";
                report.unmeasured.push((id.clone(), reason.into()));
                geometry = geometry.with_unmeasured(id, reason);
            }
            Err(error) => {
                report.unmeasured.push((id.clone(), error.clone()));
                geometry = geometry.with_unmeasured(id, error);
            }
        }
    }

    let relationships = session.service::<RelationshipSelectionServiceHandle>();
    if let Some(relationships) = relationships {
        for (space, count) in doorways(relationships, &kinds, &is_a) {
            geometry = geometry.with_doorways(space, count);
        }
    }
    for (group, members) in groups(relationships, &kinds, &is_a) {
        geometry = match members {
            Ok(members) => geometry.with_group(group, members),
            Err(reason) => geometry.with_undecided_group(group, reason),
        };
    }
    let envelope = envelope_service(&session, &geometry, &source, &kinds)?;
    let space = space_service(&parsed, &geometry, &source, &kinds, &is_a);
    let routes = route_services(&geometry, &source, &kinds, &is_a, &voids);
    let derived = derived_service(&geometry, &kinds, &is_a, voids);
    let facade = facade_service(&geometry, &kinds, &is_a);
    let session = register(session, &snapshots, geometry, space, envelope, routes)?
        .with_host_service(FacadeAreaServiceHandle::new(Arc::new(facade)), &snapshots)?
        .with_derived_relationships(
            DerivedRelationshipServiceHandle::new(Arc::new(derived)),
            &snapshots,
        )?;
    Ok((session, report))
}

/// Registers every geometry service over `geometry`, bound to `snapshots`.
fn register(
    session: EvidenceSession,
    snapshots: &[SourceSnapshot],
    geometry: AxiolidGeometry,
    space: AxiolidSpaceService,
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
        // Walking surfaces are the guard rule's selection, carried in each
        // request; the host declares none of its own.
        .with_host_service(
            GuardServiceHandle::new(Arc::new(AxiolidGuardService::new(
                geometry.clone(),
                source.clone(),
            ))),
            bound,
        )?
        .with_host_service(
            LinearQuantityServiceHandle::new(Arc::new(AxiolidLinearQuantityService::new(
                geometry.clone(),
            ))),
            bound,
        )?
        .with_host_service(
            PlanAreaServiceHandle::new(Arc::new(AxiolidPlanAreaService::new(
                geometry.clone(),
                source.clone(),
            ))),
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

/// Doorways per space, from the space boundaries the model states.
///
/// A doorway is a door that bounds the space, directly or through an opening
/// it fills (`IfcRelFillsElement`). Only spaces whose count is known are
/// returned; the linear-quantity service refuses the rest rather than count
/// zero, which would credit wall a door interrupts. A space is left out when:
///
/// - it has no space boundary at all, so the model says nothing about its doors;
/// - a bounding opening is filled by nothing, since it may be a doorless
///   passage or a niche and the model does not say which;
/// - any relationship answer is refused, including for a boundary instance
///   that omits a required end anywhere in the model.
fn doorways(
    relationships: &RelationshipSelectionServiceHandle,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&ObjectId, &str) -> bool,
) -> Vec<(ObjectId, usize)> {
    let of_kind = |ancestor: &str| -> Vec<ObjectId> {
        kinds
            .keys()
            .filter(|id| is_a(id, ancestor))
            .cloned()
            .collect()
    };
    let doors = of_kind("IfcDoor");
    let openings = of_kind("IfcOpeningElement");
    let everything: Vec<ObjectId> = kinds.keys().cloned().collect();
    let related = |anchor: &ObjectId, relationship: &str, universe: &[ObjectId]| {
        let query = RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new(relationship).ok()?,
            direction: TraversalDirection::Forward,
            follow_chain: false,
        };
        let request =
            RelationshipSelectionRequest::try_new(anchor.clone(), universe.to_vec(), query).ok()?;
        let selection = relationships.select(&request).ok()?;
        Some(selection.candidates().to_vec())
    };
    let count = |space: &ObjectId| -> Option<usize> {
        let bounding = related(space, "IfcRelSpaceBoundary", &everything)?;
        if bounding.is_empty() {
            return None;
        }
        let mut found: BTreeSet<ObjectId> = BTreeSet::new();
        for element in &bounding {
            if doors.binary_search(element).is_ok() {
                found.insert(element.clone());
            } else if openings.binary_search(element).is_ok() {
                let fillings = related(element, "IfcRelFillsElement", &everything)?;
                if fillings.is_empty() {
                    return None;
                }
                found.extend(
                    fillings
                        .into_iter()
                        .filter(|filling| doors.binary_search(filling).is_ok()),
                );
            }
        }
        Some(found.len())
    };
    of_kind("IfcSpace")
        .into_iter()
        .filter_map(|space| count(&space).map(|n| (space, n)))
        .collect()
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

/// An opening's meshed void and whether it is exact, or why it has none.
type Void = Result<(axiolid_mesh::TriMesh, bool), String>;

/// Relationships derived from geometry, over the model's spaces and its
/// doors, windows and openings.
///
/// Every `IfcSpace` is a space and every `IfcDoor`, `IfcWindow` and
/// `IfcOpeningElement` an opening; both are IFC facts. A void that could not
/// be meshed is declared unmeasured, so the derivation refuses it rather than
/// finding no space beside it.
fn derived_service(
    geometry: &AxiolidGeometry,
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
        }
    }
    for (id, void) in voids {
        service = match void {
            Ok((mesh, true)) => service.with_opening_void(id, mesh),
            Ok((mesh, false)) => {
                service.with_tessellated_opening_void(id, mesh, CHORD_DEVIATION_METRES)
            }
            Err(reason) => service.with_unmeasured_opening_void(id, reason),
        };
    }
    service
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
            Ok((mesh, true)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((mesh, false)) => service.with_tessellated_opening_void(id.clone(), mesh.clone()),
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
            Ok((mesh, true)) => service.with_opening_void(id.clone(), mesh.clone()),
            Ok((mesh, false)) => service.with_tessellated_opening_void(id.clone(), mesh.clone()),
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

fn entity_id(id: &ObjectId) -> Option<EntityId> {
    id.local_id.strip_prefix('#')?.parse().ok().map(EntityId)
}

/// One product's net body (openings subtracted) and whether it is exact.
fn mesh(
    backend: &impl MeshCompiler,
    model: &Model,
    units: &ifc_geometry::units::UnitScale,
    product: EntityId,
) -> Result<Option<(axiolid_mesh::TriMesh, bool)>, String> {
    let mut session = LoweringSession::new(model, units);
    let Some(net) = lower_product_net(&mut session, product).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let lowered = session.finish(net.root).map_err(|e| e.to_string())?;
    let exact = planar(&lowered.graph, lowered.root, &mut NODE_BUDGET.clone());
    let mesh = backend
        .compile_mesh(
            &lowered.graph,
            lowered.root,
            &ExecutionOptions::new(TOLERANCE),
        )
        .map_err(|e| format!("mesh compilation refused: {e}"))?;
    if mesh.triangle_count() == 0 {
        return Err("mesh compilation produced no triangles".into());
    }
    Ok(Some((mesh, exact)))
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
        GeometryNode::Profile(profile) => polygonal(profile),
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
    match profile {
        Profile::Rectangle(rectangle) => {
            let sharp = |radius: Option<f64>| radius.is_none_or(|r| r == 0.0);
            sharp(rectangle.outer_radius) && sharp(rectangle.inner_radius)
        }
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
        let (session, report) = attach(session, &models).unwrap();
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
}
