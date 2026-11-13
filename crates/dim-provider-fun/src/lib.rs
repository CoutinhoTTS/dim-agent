//! Fun-owned model providers for the built-in dim agent.
//!
//! Fun writes `providers.json`; this crate validates the snapshot, resolves
//! selections and connections from it, and exposes the `fun.providers.*`
//! management methods. A sync swaps the catalog in place, so the next request
//! uses the new provider/model without restarting the daemon.

mod catalog;
mod plugin;
mod snapshot;

pub use catalog::{
    CatalogSummary, FunCatalog, FunCatalogState, FunModel, LoadedCatalog, ModelIdSnapshotOut, load,
};
pub use plugin::{
    FunConnection, FunContextModel, FunModelPlugin, FunProviderPlugin, install, register_names,
};
pub use snapshot::{
    CapabilitiesSnapshot, ModelIdSnapshot, ModelSnapshot, ProviderSnapshot, Snapshot,
};
