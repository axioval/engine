//! Typed host-service registry behavior.
#![allow(missing_docs)]
use axioval_engine::{ServiceRegistry, ServiceRegistryError};
#[derive(Debug, PartialEq)]
struct UnitScale(f64);
#[test]
fn services_are_type_safe_and_duplicate_registration_fails() {
    let mut services = ServiceRegistry::new();
    services.register(UnitScale(0.001)).unwrap();
    assert_eq!(services.get::<UnitScale>(), Some(&UnitScale(0.001)));
    assert_eq!(
        services.register(UnitScale(1.0)),
        Err(ServiceRegistryError::Duplicate)
    );
}
