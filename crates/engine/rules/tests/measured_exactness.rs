//! Every built-in measured-value provider is held to tessellated evidence:
//! a value or member measured from approximate evidence is never exact,
//! however its interval looks. Exactness is the provider's statement, never
//! inferred from a point (`axioval_engine::Measurement`).
//!
//! Each provider's exact path is exercised where its fixtures live; this
//! table names the test that does it, and fails for a provider it does not
//! name, so a new provider cannot be registered without one.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ResolvedProperty,
};
use axioval_ir::{Evidence, Property, PropertyValue, QuantityDimension};

/// Where a provider's exact path meets approximate evidence: one name or
/// member list the provider measures, the test file (from this crate's
/// root) and the test.
const COVERED: &[(&str, &str, &str)] = &[
    (
        "face_pieces",
        "tests/face_pieces.rs",
        "pieces_measured_from_approximate_normals_are_inexact",
    ),
    (
        "distance",
        "tests/distance.rs",
        "tessellated_counterparts_measure_inexactly",
    ),
    (
        "sill_height",
        "tests/keyed_limit.rs",
        "a_sill_height_measured_on_a_tessellation_is_inexact",
    ),
    (
        "swing_spaces",
        "tests/door_swing.rs",
        "spaces_probed_approximately_are_never_exact",
    ),
    (
        "section_area",
        "tests/allowed_profile.rs",
        "an_unset_fillet_radius_widens_the_section_area",
    ),
    (
        "flight_width",
        "tests/stair_geometry.rs",
        "a_tessellated_flight_measures_inexactly",
    ),
    (
        "guard_edges",
        "tests/horizontal_guard.rs",
        "approximate_guard_edges_are_never_measured",
    ),
    (
        "plan_diameter",
        "../../facade/axioval/tests/axiolid_bays_and_spacing.rs",
        "values_measured_on_tessellations_are_never_exact",
    ),
    (
        "travel_distance",
        "tests/escape_route.rs",
        "a_travel_distance_walked_on_a_tessellation_is_inexact",
    ),
    (
        "free_placements",
        "tests/free_floor_circle.rs",
        "only_a_placement_found_exactly_is_exact",
    ),
    (
        "rectangle_side",
        "../../facade/axioval/tests/axiolid_bays_and_spacing.rs",
        "values_measured_on_tessellations_are_never_exact",
    ),
    (
        "band_uncovered_area",
        "../../facade/axioval/tests/axiolid_bays_and_spacing.rs",
        "values_measured_on_tessellations_are_never_exact",
    ),
    (
        "level_rise",
        "tests/level_spacing_geometry.rs",
        "a_rise_measured_on_a_tessellation_is_inexact",
    ),
    (
        "shelf_length",
        "tests/shelf_capacity.rs",
        "an_estimated_run_is_never_measured_exactly",
    ),
    (
        "stack_distance",
        "tests/slab_stack.rs",
        "a_distance_to_a_tessellated_slab_is_inexact",
    ),
    (
        "triangle_count",
        "tests/triangle_count.rs",
        "a_tessellated_count_is_measured_inexactly",
    ),
    (
        "body_extent",
        "tests/body_extent.rs",
        "an_extent_measured_on_a_tessellation_is_inexact",
    ),
    (
        "plan_area",
        "tests/plan_area.rs",
        "an_area_measured_on_a_tessellation_is_inexact",
    ),
    (
        "counterpart_uncovered_share",
        "tests/counterpart_coverage.rs",
        "a_share_measured_on_a_tessellation_is_inexact",
    ),
    (
        "coordinate_shift",
        "tests/coordinate_consistency.rs",
        "a_departure_is_exact_only_as_stated_and_rounded_outward",
    ),
    // Measured from stated facts only (body facts, storey elevations),
    // which reach a provider only exactly: the property service refuses a
    // stated value cited approximate.
    (
        "opening_area",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
    (
        "opening_placements",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
    (
        "levels_above",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
];

/// Every registered provider is named, by one of its names or lists, and
/// every name the table holds is registered.
#[test]
fn every_provider_is_held_to_tessellated_evidence() {
    let registry = axioval_rules::register_builtins(CapabilityRegistry::new()).unwrap();
    let mut measured = BTreeSet::new();
    for (names, lists) in registry.measured_providers() {
        let all: Vec<&str> = names.iter().chain(lists).copied().collect();
        assert!(
            all.iter()
                .any(|name| COVERED.iter().any(|(covered, ..)| covered == name)),
            "no test holds the provider measuring {all:?} to tessellated evidence: add one and \
             name it in COVERED"
        );
        measured.extend(all);
    }
    for (name, ..) in COVERED {
        assert!(
            measured.contains(name),
            "`{name}` is measured by no provider"
        );
    }
}

/// Every test the table names exists where it says.
#[test]
fn every_named_test_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (name, file, test) in COVERED {
        let source = std::fs::read_to_string(root.join(file))
            .unwrap_or_else(|error| panic!("{name}: {file}: {error}"));
        assert!(
            source.contains(&format!("fn {test}(")),
            "{name}: {file} has no test `{test}`"
        );
    }
}

/// A source stating a value cited as approximate.
struct Approximate;

impl PropertyResolutionService for Approximate {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let mut evidence = Evidence::exact(common::source(), "estimate");
        evidence.exact = false;
        let property = Property::new(
            request.property_set().unwrap_or("Pset"),
            request.property(),
            PropertyValue::Quantity {
                value: 0.3,
                dimension: QuantityDimension::Length,
            },
        )
        .unwrap()
        .with_evidence(evidence);
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
    }
}

/// A stated value reaches a provider only exactly: one cited approximate
/// is refused, so nothing measured from stated facts becomes exact on an
/// estimate.
#[test]
fn a_stated_value_cited_approximate_is_refused() {
    let handle = PropertyResolutionServiceHandle::new(Arc::new(Approximate));
    let request = PropertyRequest::try_new(
        common::id("wall"),
        Some(axioval_ir::BODY_SET.to_owned()),
        "Profile.XDim",
    )
    .unwrap();
    assert_eq!(
        handle.resolve(&request).err(),
        Some(PropertyResolutionError::InexactEvidence)
    );
}
