//! What must be true — the rule half of the spec.
//!
//! A [`definition::RuleDefinition`] is a reusable parameterized check;
//! binding it to values yields an [`instance::RuleInstance`]; a tree of those
//! is an [`instance::RuleSetPackage`], the unit a backend compiles into one
//! output document.
//!
//! [`target::Fidelity`] per [`target::Target`] is the honesty mechanism: a rule
//! declares how completely it maps to each output format, so a backend can
//! refuse to emit rather than silently degrade.

pub mod aggregate;
pub mod applicability;
pub mod assertion;
pub mod basic_checks;
pub mod building_storey;
pub mod clash_matrix;
pub mod comparison;
pub mod component_clearance;
pub mod containment;
pub mod coverage;
pub mod daylight;
pub mod definition;
pub mod door_accessibility;
pub mod effective_coverage;
pub mod envelope;
pub mod escape_route;
pub mod execution;
pub mod exit_access_doorway;
pub mod external_wall;
pub mod fire_compartment_membership;
pub mod fire_wall_components;
pub mod free_floor_space;
pub mod front_clearance;
pub mod horizontal_guard;
pub mod instance;
pub mod layer_agreement;
pub mod local_circulation;
pub mod manual_issue;
pub mod model;
pub mod model_architecture;
pub mod model_comparison;
pub mod opening;
pub mod opening_sill;
pub mod param;
pub mod parking;
pub mod profile;
pub mod ramp;
pub mod relation;
pub mod shelf_capacity;
pub mod slab_contact;
pub mod space_connection;
pub mod space_distance;
pub mod space_validation;
pub mod spatial;
pub mod stair;
pub mod structure_architecture_conformity;
pub mod target;
pub mod text;
pub mod visibility;
pub mod wall_validation;
