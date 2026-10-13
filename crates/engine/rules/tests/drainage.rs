//! Whether a surface drains to its outlets: `undrained_area` and
//! `drainage_path` over the drainage the proximity service traces.
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityRegistry, DrainageEvidence, DrainagePath, DrainageRequest, ObjectBounds,
    PropertyResolution, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle, ServiceRegistry, measured_value,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::{Model, id, source};

/// A roof traced as engine#335's: 240 m² in plan, of which a dished 40 m²
/// holds its water while both drains take the rest; one drain alone
/// takes half. Every request is recorded.
struct Roof {
    exact: bool,
    asked: Mutex<Vec<DrainageRequest>>,
}

impl ProximityService for Roof {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_drainage(
        &self,
        request: &DrainageRequest,
    ) -> Result<DrainageEvidence, ProximityError> {
        self.asked.lock().unwrap().push(request.clone());
        let held = if request.minimum_fall() > 0.04 {
            240.0
        } else {
            match request.outlets().len() {
                0 => 240.0,
                1 => 140.0,
                _ => 40.0,
            }
        };
        let path = if held >= 240.0 {
            DrainagePath::NoOutlet
        } else {
            DrainagePath::Bounded {
                lower_metres: 6.86,
                upper_metres: 7.07,
            }
        };
        let mut evidence = Evidence::exact(source(), "drainage");
        evidence.exact = self.exact;
        let undrained = if self.exact {
            (held, held)
        } else {
            (0.0, 240.0)
        };
        DrainageEvidence::try_new(request.clone(), (240.0, 240.0), undrained, path, evidence)
    }
}

fn session(exact: bool) -> (Project, ServiceRegistry, Arc<Roof>) {
    let model = Model::default()
        .object("roof", "slab")
        .object("d1", "drain")
        .object("d2", "drain");
    let roof = Arc::new(Roof {
        exact,
        asked: Mutex::new(Vec::new()),
    });
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(roof.clone()))
        .unwrap();
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services, roof)
}

/// `name` of the roof: its bounds and whether its evidence is exact.
fn value(
    (project, services, _): &(Project, ServiceRegistry, Arc<Roof>),
    name: &str,
) -> Result<(f64, f64, bool), String> {
    match measured_value(services, project, &id("roof"), name) {
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.into_property();
            let exact = property.evidence.iter().all(|evidence| evidence.exact);
            match property.value {
                PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => {
                    Ok((value, value, exact))
                }
                PropertyValue::Measured { lower, upper, .. } => Ok((lower, upper, exact)),
                other => Err(format!("{other:?}")),
            }
        }
        Ok(PropertyResolution::Absent(_)) => Err("absent".into()),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn a_dished_area_is_reported_with_its_area() {
    let roof = session(true);
    let (lower, upper, exact) = value(&roof, "undrained_area;outlets=drain").unwrap();
    assert!((lower - 40.0).abs() < 1e-9 && (upper - 40.0).abs() < 1e-9);
    assert!(exact);
    let asked = roof.2.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].surface(), &id("roof"));
    assert_eq!(asked[0].outlets(), &[id("d1"), id("d2")]);
    assert!(asked[0].minimum_fall().abs() < f64::EPSILON);
    // The path's bounds differ, so it is no exact value.
    let (lower, upper, exact) = value(&roof, "drainage_path;outlets=drain").unwrap();
    assert!((lower - 6.86).abs() < 1e-9 && (upper - 7.07).abs() < 1e-9);
    assert!(!exact);
}

#[test]
fn the_minimum_fall_is_handed_to_the_service() {
    let roof = session(true);
    let (lower, ..) = value(&roof, "undrained_area;outlets=drain;minimum_fall=0.05").unwrap();
    assert!((lower - 240.0).abs() < 1e-9);
    let asked = roof.2.asked.lock().unwrap().clone();
    assert!((asked[0].minimum_fall() - 0.05).abs() < 1e-12);
    // Nothing drains, so there is no path.
    let why = value(&roof, "drainage_path;outlets=drain;minimum_fall=0.05").unwrap_err();
    assert_eq!(why, "absent");
}

#[test]
fn a_drainage_measured_on_a_tessellation_is_inexact() {
    let roof = session(false);
    let (lower, upper, exact) = value(&roof, "undrained_area;outlets=drain").unwrap();
    assert!(lower <= 40.0 && 40.0 <= upper);
    assert!(!exact);
    let (.., exact) = value(&roof, "drainage_path;outlets=drain").unwrap();
    assert!(!exact);
}

#[test]
fn without_the_service_nothing_is_traced() {
    let model = Model::default().object("roof", "slab");
    let project = model.project();
    let mut services = ServiceRegistry::new();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    let why = match measured_value(
        &services,
        &project,
        &id("roof"),
        "undrained_area;outlets=drain",
    ) {
        Err(error) => error.to_string(),
        Ok(other) => format!("{other:?}"),
    };
    assert!(why.contains("proximity service is not registered"), "{why}");
}
