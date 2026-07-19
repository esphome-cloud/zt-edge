//! Public project-generation API for the rshome SDK.

pub mod board_yaml;
pub mod brookesia;
pub mod cmake;
pub mod error;
pub mod generator;
pub mod ha_adapter;
pub mod sdkconfig;

pub use error::CodegenError;
pub use generator::{GeneratedProject, ProjectGenerator};
