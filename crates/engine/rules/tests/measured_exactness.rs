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
        "containment_items",
        "tests/containment.rs",
        "a_cover_measured_on_a_tessellation_is_inexact",
    ),
    (
        "distance_items",
        "tests/distance.rs",
        "a_distance_measured_on_a_tessellation_is_inexact",
    ),
    (
        "parking_bay",
        "../../facade/axioval/tests/axiolid_bays_and_spacing.rs",
        "a_bay_measured_on_a_tessellation_is_inexact",
    ),
    (
        "wall_spacing",
        "../../facade/axioval/tests/axiolid_bays_and_spacing.rs",
        "a_spacing_measured_on_a_tessellation_is_inexact",
    ),
    (
        "numbering",
        "tests/semantic.rs",
        "a_gap_and_a_different_prefix_are_reported_per_storey",
    ),
    (
        "name_sequence",
        "tests/semantic.rs",
        "members_without_an_order_value_are_ordered_by_their_placement_height",
    ),
    (
        "exit_separation",
        "tests/exit_separation.rs",
        "a_separation_measured_approximately_is_inexact",
    ),
    (
        "clash_pairs",
        "tests/clash.rs",
        "pairs_measured_on_tessellated_geometry_are_inexact",
    ),
    (
        "sight_view",
        "tests/component_visibility.rs",
        "a_view_measured_approximately_is_inexact",
    ),
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
        "limited_values",
        "tests/keyed_limit.rs",
        "a_judged_sill_height_on_a_tessellation_is_inexact",
    ),
    (
        "corridor_end_openings",
        "tests/corridor_end_openings.rs",
        "openings_searched_at_approximate_ends_are_never_exact",
    ),
    (
        "swing_spaces",
        "tests/door_swing.rs",
        "spaces_probed_approximately_are_never_exact",
    ),
    (
        "space_connections",
        "tests/space_connection.rs",
        "connections_read_from_approximate_adjacency_are_never_exact",
    ),
    (
        "connected_spaces",
        "tests/opening_spaces.rs",
        "connected_spaces_read_from_approximate_adjacency_are_never_exact",
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
        "run_count",
        "tests/stair_geometry.rs",
        "a_tessellated_ramps_items_measure_inexactly",
    ),
    (
        "guard_edges",
        "tests/horizontal_guard.rs",
        "approximate_guard_edges_are_never_measured",
    ),
    (
        "space_height",
        "tests/space_validation.rs",
        "space_aspects_measured_approximately_are_inexact",
    ),
    (
        "boundary_coverage_share",
        "tests/space_boundary_coverage.rs",
        "coverage_measured_approximately_is_never_exact",
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
        "plan_coverage",
        "tests/plan_area.rs",
        "an_area_measured_on_a_tessellation_is_inexact",
    ),
    (
        "ratio_area",
        "tests/plan_area.rs",
        "an_area_measured_on_a_tessellation_is_inexact",
    ),
    (
        "counterpart_uncovered_share",
        "tests/counterpart_coverage.rs",
        "a_share_measured_on_a_tessellation_is_inexact",
    ),
    (
        "effective_share",
        "tests/effective_coverage.rs",
        "an_effect_measured_inexactly_is_inexact",
    ),
    (
        "coordinate_shift",
        "tests/coordinate_consistency.rs",
        "a_departure_is_exact_only_as_stated_and_rounded_outward",
    ),
    // Measured from stated facts only (body facts, storey elevations, an
    // opening's stated light area and size), which reach a provider only
    // exactly: the property service refuses a stated value cited
    // approximate.
    (
        "light_area",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
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
    // Placed from the same stated bodies; a support's contact cites the
    // proximity service's evidence on the item as it states it.
    (
        "zone_checks",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
    (
        "levels_above",
        "tests/measured_exactness.rs",
        "a_stated_value_cited_approximate_is_refused",
    ),
    // A contact service's evidence must be exact and reviewable: one
    // cited approximate is refused before it reaches a value.
    (
        "contact_share",
        "tests/slab_contact.rs",
        "a_contact_cited_approximate_is_refused",
    ),
    // An envelope service's evidence must be exact and reviewable: one
    // cited approximate is refused before it reaches a value.
    (
        "on_envelope",
        "tests/external_wall_validation.rs",
        "an_envelope_cited_approximate_is_refused",
    ),
    // Counted over a selection, which nothing measures.
    (
        "undecided_count",
        "tests/measured_exactness.rs",
        "an_undecided_count_is_counted_exactly",
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

/// A count of the objects a selection cannot decide is counted, never
/// measured: exact, nothing named counting none.
#[test]
fn an_undecided_count_is_counted_exactly() {
    let project =
        axioval_ir::Project::new(vec![axioval_ir::Object::new(common::id("wall"), "wall")])
            .unwrap();
    let services = axioval_engine::ServiceRegistry::new();
    for name in ["undecided_count;objects=wall", "undecided_count"] {
        assert_eq!(
            common::measured_cited(&services, &project, &common::id("wall"), name),
            Ok(Some(((0.0, 0.0), true))),
            "{name}"
        );
    }
}
