//! Hooks: the documents, their sources and the commands they run
//! (`docs/design/plugins/Hooks.md`).
//!
//! This module holds what both owners need: the document format and its
//! validation, the effective catalog of one hook point, and the Session port
//! that reads the sources. The configuration owner is HooksSettings in App
//! Server; it registers the documents and binds [`HookSourcesHost`].
pub mod document;
mod sources;

pub use document::{
    HookEntry, HookEvent, HookHandler, HookState, HooksDocument, OnFailure, entries, handlers,
};
pub use sources::{
    HookSourceId, HookSourceSnapshot, HookSourceState, HookSourcesHost, HookSourcesSnapshot,
    NoHookSources, ReadHookSources,
};
