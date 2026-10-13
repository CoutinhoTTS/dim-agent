//! Native aimux model hosts for the Agent runtime.
pub mod config;
mod driver;
mod models;
mod plugin;
mod request;
mod stream;

pub use models::AimuxModels;
pub use plugin::{AimuxPlugin, ConfiguredModelPlugin, ConfiguredModelsPlugin};
