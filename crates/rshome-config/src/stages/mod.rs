//! 10-stage validation pipeline — individual stage modules.
//!
//! Each stage module exports exactly one public function named `stage_N_*`.
//! Stages accumulate errors into a `Vec<ValidationError>`; fatal errors stop
//! pipeline progression in the orchestrator ([`pipeline`](crate::pipeline)).

pub mod s10_pin_conflicts;
pub mod s11_exclusive_groups;
pub mod s12_profile_validation;
pub mod s13_secret_fields;
pub mod s1_package_merge;
pub mod s2_substitutions;
pub mod s3_5_variant_resolution;
pub mod s3_extend_remove;
pub mod s4_external_components;
pub mod s5_preload;
pub mod s6_load_components;
pub mod s7_schema_validation;
pub mod s8_id_resolution;
pub mod s9_final_validation;
