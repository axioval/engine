//! IFC body geometry as Axiolid meshes, registered into a check's session.
//!
//! This is host composition, not an adapter: `axioval-ifc` must not know
//! Axiolid and `axioval-axiolid` must not know IFC, so the CLI reads the same
//! model bytes a second time, meshes each product with `ifc-geometry`, and
//! hands the meshes to the geometry services under the object identities the
//! IFC session already uses.
//!
//! Every object ends up in exactly one of three states, because the geometry
//! services treat them differently:
//!
//! - **meshed**, either exactly (every face planar, so the mesh is the shape)
//!   or as a tessellation within the compiler's chord budget;
//! - **no body**: it occupies no material (a storey, a zone, an opening);
//! - **unmeasured**: it is physical but could not be meshed. Measurements it
//!   could affect refuse rather than act as if it were not there.

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
    AxiolidContactService, AxiolidEnvelopeMembershipService, AxiolidFreeSpaceService,
    AxiolidGeometry, AxiolidLinearQuantityService, AxiolidProximityService, AxiolidSpaceService,
};
use axioval::engine::{
    ContactServiceHandle, EnvelopeMembershipServiceHandle, EvidenceSession, FreeSpaceServiceHandle,
    LinearQuantityServiceHandle, PropertyRequest, PropertyResolution,
    PropertyResolutionServiceHandle, ProximityServiceHandle, RelationshipQuery,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, SemanticRelationship,
    SourceSnapshot, SpaceServiceHandle, TraversalDirection, TypeHierarchyServiceHandle,
};
use axioval::ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, SourceId};
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

/// Policy choices IFC does not state, supplied explicitly by the caller.
///
/// Each one decides what a geometric service treats as given. A service whose
/// profile is missing is not registered, so its rules report
/// `missing-service` instead of acting on a guess.
#[derive(Debug, Default)]
pub struct Profiles {
    /// Name of the `IfcZone` whose spaces make up the building's envelope.
    pub envelope_zone: Option<String>,
}

/// Meshes the model in `bytes` and registers geometry services for `session`.
///
/// # Errors
///
/// Returns an error when the bytes do not parse (the session already parsed
/// them, so this means they changed), a service cannot be registered, or a
/// profile names something the model does not contain.
pub fn attach(
    session: EvidenceSession,
    bytes: &[u8],
    profiles: &Profiles,
) -> Result<(EvidenceSession, GeometryReport), Box<dyn Error>> {
    let model = StepCodec.read_bytes(bytes)?;
    let snapshots: Vec<_> = session.snapshots().cloned().collect();
    let [snapshot] = snapshots.as_slice() else {
        return Err("geometry needs a session over exactly one source".into());
    };
    let source = snapshot.source().clone();
    let hierarchy = session
        .service::<TypeHierarchyServiceHandle>()
        .ok_or("the session has no type hierarchy to classify objects with")?
        .clone();
    let is_a =
        |kind: &str, ancestor: &str| hierarchy.is_a(&source, kind, ancestor).unwrap_or(false);

    let backend = ifc_geometry::compile::default_backend();
    let units = ifc_geometry::units::resolve(&model);
    let mut geometry = AxiolidGeometry::new();
    let mut report = GeometryReport::default();
    let mut kinds: BTreeMap<ObjectId, String> = BTreeMap::new();

    for object in session.project().objects() {
        let id = object.id.clone();
        let kind = object.kind().to_owned();
        kinds.insert(id.clone(), kind.clone());
        let is_space = is_a(&kind, "IfcSpace");
        let bodiless = !is_a(&kind, "IfcProduct")
            || (!is_space && NO_BODY.iter().any(|ancestor| is_a(&kind, ancestor)));
        if bodiless {
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
        match mesh(&backend, &model, &units, entity) {
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

    if let Some(relationships) = session.service::<RelationshipSelectionServiceHandle>() {
        for (space, count) in doorways(relationships, &kinds, &is_a) {
            geometry = geometry.with_doorways(space, count);
        }
    }
    let envelope = match &profiles.envelope_zone {
        Some(zone) => Some(envelope_service(
            &session, &geometry, &source, &kinds, &is_a, zone,
        )?),
        None => None,
    };
    let space = space_service(&model, &geometry, &source, &kinds, &is_a);
    let session = register(session, snapshot, geometry, space, envelope)?;
    Ok((session, report))
}

/// Registers every geometry service over `geometry` for `snapshot`.
fn register(
    session: EvidenceSession,
    snapshot: &SourceSnapshot,
    geometry: AxiolidGeometry,
    space: AxiolidSpaceService,
    envelope: Option<AxiolidEnvelopeMembershipService>,
) -> Result<EvidenceSession, Box<dyn Error>> {
    let source = snapshot.source().clone();
    let bound = std::slice::from_ref(snapshot);
    let session = session
        .with_host_service(
            ContactServiceHandle::new(Arc::new(AxiolidContactService::new(
                geometry.clone(),
                source.clone(),
            ))),
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
        .with_host_service(
            LinearQuantityServiceHandle::new(Arc::new(AxiolidLinearQuantityService::new(
                geometry.clone(),
                source.clone(),
            ))),
            bound,
        )?
        .with_host_service(
            ProximityServiceHandle::new(Arc::new(AxiolidProximityService::new(geometry))),
            bound,
        )?;
    let session = match envelope {
        Some(envelope) => session.with_host_service(
            EnvelopeMembershipServiceHandle::new(Arc::new(envelope)),
            bound,
        )?,
        None => session,
    };
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
    is_a: &impl Fn(&str, &str) -> bool,
) -> Vec<(ObjectId, usize)> {
    let of_kind = |ancestor: &str| -> Vec<ObjectId> {
        kinds
            .iter()
            .filter(|(_, kind)| is_a(kind, ancestor))
            .map(|(id, _)| id.clone())
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

/// Envelope membership over the spaces of one named `IfcZone`.
///
/// The zone is the only envelope statement IFC makes explicitly, so the caller
/// names it rather than this bridge guessing which spaces are conditioned.
/// Its spaces bound both derivations. Each other object's `IsExternal`, from
/// whichever property set states it, is its declaration: `true` external,
/// `false` internal. An object without exactly one such boolean stays
/// undeclared and its rule reports not evaluated; absent is not internal.
fn envelope_service(
    session: &EvidenceSession,
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&str, &str) -> bool,
    zone_name: &str,
) -> Result<AxiolidEnvelopeMembershipService, Box<dyn Error>> {
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .ok_or("the session has no property service to read declarations with")?;
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .ok_or("the session has no relationship service to read zones with")?;
    let value = |object: &ObjectId, set: Option<&str>, name: &str| -> Option<PropertyValue> {
        let request =
            PropertyRequest::try_new(object.clone(), set.map(str::to_owned), name).ok()?;
        match properties.resolve(&request).ok()? {
            PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
            PropertyResolution::Absent(_) => None,
        }
    };

    let zones: Vec<&ObjectId> = kinds
        .iter()
        .filter(|(id, kind)| {
            is_a(kind, "IfcZone")
                && value(id, Some(ATTRIBUTE_SET), "Name")
                    == Some(PropertyValue::String(zone_name.to_owned()))
        })
        .map(|(id, _)| id)
        .collect();
    let [zone] = zones.as_slice() else {
        return Err(format!(
            "--envelope-zone names {} IfcZone(s) called `{zone_name}`, not exactly one",
            zones.len()
        )
        .into());
    };
    let spaces: Vec<ObjectId> = kinds
        .iter()
        .filter(|(_, kind)| is_a(kind, "IfcSpace"))
        .map(|(id, _)| id.clone())
        .collect();
    let query = RelationshipQuery::Related {
        relationship: SemanticRelationship::try_new("IfcRelAssignsToGroup")?,
        direction: TraversalDirection::Forward,
        follow_chain: false,
    };
    let request = RelationshipSelectionRequest::try_new((*zone).clone(), spaces, query)?;
    let members = relationships
        .select(&request)
        .map_err(|error| format!("zone `{zone_name}`: {error}"))?;
    if members.candidates().is_empty() {
        return Err(format!("zone `{zone_name}` groups no IfcSpace").into());
    }

    let mut service = AxiolidEnvelopeMembershipService::new(geometry.clone(), source.clone());
    for space in members.candidates() {
        service = service.with_gross_area_space(space.clone());
    }
    for object in kinds.keys() {
        if geometry.mesh(object).is_none() || members.candidates().contains(object) {
            continue;
        }
        match value(object, None, "IsExternal") {
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
/// Storeys come from the spatial tree. An element the file places twice, or
/// anything under a structure aggregated twice, gets no storey: the tree keeps
/// one of the two parents, and a guessed storey would move floor area between
/// storeys without saying so.
fn space_service(
    model: &Model,
    geometry: &AxiolidGeometry,
    source: &SourceId,
    kinds: &BTreeMap<ObjectId, String>,
    is_a: &impl Fn(&str, &str) -> bool,
) -> AxiolidSpaceService {
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
    let storey_of = |entity: EntityId| -> Option<EntityId> {
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
    for (id, kind) in kinds {
        let Some(entity) = entity_id(id) else {
            continue;
        };
        if is_a(kind, "IfcSpace") {
            service = service.with_space(id.clone());
        } else if is_a(kind, "IfcSlab") {
            service = service.with_slab(id.clone());
        } else if is_a(kind, "IfcRoof") {
            service = service.with_roof(id.clone());
        } else if is_a(kind, "IfcBuilding") {
            service = service.with_building(id.clone());
        }
        let is_structure = tree.node(entity).is_some();
        if (!is_structure || is_a(kind, "IfcSpace"))
            && !geometry.has_no_body(id)
            && let Some(storey) = storey_of(entity)
        {
            let storey = ObjectId {
                source: source.clone(),
                local_id: storey.to_string(),
            };
            service = service.with_storey(id.clone(), storey);
        }
    }
    service
}
