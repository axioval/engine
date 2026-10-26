//! The facade's `mcs` feature: a `.mcs` archive read through
//! `axioval::mcs` yields the facade's own `axioval-ir` types, and the
//! default registry compiles them.
#![cfg(feature = "mcs")]
#![allow(missing_docs)]

use axioval::engine::compile;

const MINIMAL: &[u8] = include_bytes!("../../../../fixtures/mcs-v0.1.0/minimal.mcs");

#[test]
fn archive_read_through_the_facade_compiles_with_the_default_registry() {
    let (ruleset, definitions) = axioval::mcs::read(MINIMAL).unwrap().into_ruleset().unwrap();
    let registry = axioval::default_registry().unwrap();
    let plan = compile(&registry, &definitions, &ruleset).unwrap();
    assert!(!plan.rules().is_empty());
}
