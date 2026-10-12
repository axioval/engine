//! The cover of embedded bodies to their host's faces and the height of an
//! edge above the surfaces beneath it, measured by Axiolid on real meshes.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval::engine::{
    CapabilityRegistry, CompleteRelationshipSelection, PropertyResolution, ProximityServiceHandle,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, ServiceRegistry,
    TraversalDirection, measured_value,
};
use axioval::ir::{Evidence, Object, ObjectId, Project, PropertyValue, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented prism over the counter-clockwise plan
/// polygon `plan`, from `bottom` up to the height `top` gives at each
/// corner.
fn prism(plan: &[[f64; 2]], bottom: f64, top: impl Fn(f64, f64) -> f64) -> TriMesh {
    let n = u32::try_from(plan.len()).unwrap();
    let mut points: Vec<Point3> = plan
        .iter()
        .map(|[x, y]| Point3::new(*x, *y, bottom))
        .collect();
    points.extend(plan.iter().map(|[x, y]| Point3::new(*x, *y, top(*x, *y))));
    let mut indices = Vec::new();
    for i in 1..n - 1 {
        indices.extend([0, i + 1, i]);
        indices.extend([n, n + i, n + i + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    TriMesh::new(points, indices)
}

/// An axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    prism(&rect(min[0], min[1], max[0], max[1]), min[2], |_, _| max[2])
}

/// An axis-aligned plan rectangle.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

#[derive(Default)]
struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    hosts: Vec<(ObjectId, ObjectId)>,
}

impl Scene {
    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    /// Replaces `local`'s mesh by the same mesh read as a tessellation of
    /// curved faces, within a millimetre of the true surface.
    fn tessellated(mut self, local: &str) -> Self {
        let mesh = self.geometry.mesh(&id(local)).unwrap().clone();
        self.geometry = self.geometry.with_tessellated_mesh(id(local), mesh, 0.001);
        self
    }

    fn hosted(mut self, embedded: &str, host: &str) -> Self {
        self.hosts.push((id(embedded), id(host)));
        self
    }

    /// The measured value `name` of `local`: its bounds and whether its
    /// evidence is exact, `None` where it is absent, or why it is not
    /// evaluated.
    fn measure(self, local: &str, name: &str) -> Result<Option<(f64, f64, bool)>, String> {
        let project = Project::new(self.objects.clone()).unwrap();
        let mut services = ServiceRegistry::new();
        services
            .register(ProximityServiceHandle::new(Arc::new(
                AxiolidProximityService::new(self.geometry.clone()),
            )))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(Arc::new(self)))
            .unwrap();
        axioval::rules::register_builtins(CapabilityRegistry::new())
            .unwrap()
            .install_measured(&mut services, &project);
        match measured_value(&services, &project, &id(local), name) {
            Ok(PropertyResolution::Present(resolved)) => {
                let property = resolved.property();
                let exact = property
                    .evidence
                    .as_ref()
                    .is_some_and(|evidence| evidence.exact);
                Ok(Some(match property.value() {
                    PropertyValue::Quantity { value, .. } => (*value, *value, exact),
                    PropertyValue::Measured { lower, upper, .. } => (*lower, *upper, exact),
                    other => panic!("{name} of {local} is {other:?}"),
                }))
            }
            Ok(PropertyResolution::Absent(_)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl RelationshipSelectionService for Scene {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related {
            relationship,
            direction: TraversalDirection::Forward,
            ..
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        if relationship.as_str() != "hosts" {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "no {} in this source",
                relationship.as_str()
            )));
        }
        let reached = self
            .hosts
            .iter()
            .filter(|(embedded, host)| {
                embedded == request.anchor() && request.candidate_universe().contains(host)
            })
            .map(|(_, host)| host.clone())
            .collect();
        CompleteRelationshipSelection::try_new(
            request.clone(),
            reached,
            vec![Evidence::exact(source(), "scan:hosts")],
        )
    }
}

fn near(measured: Option<(f64, f64, bool)>, value: f64, within: f64) -> bool {
    measured.is_some_and(|(lower, upper, _)| {
        lower <= value + within && value - within <= upper && upper - lower <= 2.0 * within
    })
}

/// A 2 m x 1 m slab 0.2 m deep holding a 20 mm bar 35 mm above its
/// soffit (`b`), one falling from there to 25 mm (`f`), one reaching out of
/// its end (`o`) and one hosted by two slabs (`d`).
fn slab() -> Scene {
    let falling = TriMesh::new(
        [
            [0.1, 0.29, 0.035],
            [1.9, 0.29, 0.025],
            [1.9, 0.31, 0.025],
            [0.1, 0.31, 0.035],
            [0.1, 0.29, 0.055],
            [1.9, 0.29, 0.045],
            [1.9, 0.31, 0.045],
            [0.1, 0.31, 0.055],
        ]
        .map(|[x, y, z]| Point3::new(x, y, z))
        .to_vec(),
        cuboid([0.0; 3], [1.0; 3]).indices,
    );
    Scene::default()
        .body("s", "slab", cuboid([0.0, 0.0, 0.0], [2.0, 1.0, 0.2]))
        .body("t", "slab", cuboid([3.0, 0.0, 0.0], [4.0, 1.0, 0.2]))
        .body("b", "bar", cuboid([0.1, 0.49, 0.035], [1.9, 0.51, 0.055]))
        .body("f", "bar", falling)
        .body("o", "bar", cuboid([1.5, 0.69, 0.09], [2.5, 0.71, 0.11]))
        .body("d", "bar", cuboid([0.2, 0.69, 0.09], [0.4, 0.71, 0.11]))
        .hosted("b", "s")
        .hosted("f", "s")
        .hosted("o", "s")
        .hosted("d", "s")
        .hosted("d", "t")
}

const COVER: &str = "cover;host_path=hosts";

#[test]
fn a_cover_is_the_least_distance_to_the_host_faces_over_the_whole_body() {
    let cover = slab().measure("b", COVER).unwrap();
    assert!(near(cover, 0.035, 1e-9), "{cover:?}");
    assert!(cover.unwrap().2, "{cover:?}");
    let cover = slab().measure("f", COVER).unwrap();
    assert!(near(cover, 0.025, 1e-9), "{cover:?}");
    // To the top faces only, the bar keeps 0.145 m.
    let top = slab().measure("b", &format!("{COVER};faces=top")).unwrap();
    assert!(near(top, 0.145, 1e-9), "{top:?}");
    // Reaching out of its host, a bar has no cover; hosted twice, its host
    // is unknown; hosted by none, it has none.
    let why = slab().measure("o", COVER).unwrap_err();
    assert!(why.contains("not wholly inside its host"), "{why}");
    let why = slab().measure("d", COVER).unwrap_err();
    assert!(why.contains("several hosts"), "{why}");
    assert_eq!(slab().measure("s", COVER).unwrap(), None);
}

#[test]
fn a_cover_measured_on_a_tessellation_is_inexact() {
    let cover = slab().tessellated("b").measure("b", COVER).unwrap();
    let (lower, upper, exact) = cover.unwrap();
    assert!(!exact, "{cover:?}");
    assert!(lower <= 0.035 && 0.035 <= upper, "{cover:?}");
    // A tessellated host's faces are not its true faces.
    let why = slab().tessellated("s").measure("b", COVER).unwrap_err();
    assert!(why.contains("tessellated"), "{why}");
}
