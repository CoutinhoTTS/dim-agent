//! The plugin-supplied OCR target's description (Modality A.2.1): the reply
//! `ocrHelper.describe` returns, and the contract version this build reads.

use serde::Deserialize;
use std::path::PathBuf;

/// The `providerId` of the plugin-supplied target; with [`PLUGIN_MODEL`] it
/// names `local/pp-ocr`.
pub const PLUGIN_PROVIDER: &str = "local";
/// The `modelId` of the plugin-supplied target.
pub const PLUGIN_MODEL: &str = "pp-ocr";
/// The helper-contract version Modality implements; a description reporting
/// another version marks the target as needing an update.
pub const HELPER_CONTRACT: u32 = 1;

/// The `describe` reply: what Modality starts for one task, and what the
/// helper must be able to read. Modality trusts it per call and stores none
/// of it in the task record.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperDescription {
    /// The contract version the plugin implements.
    pub contract: u32,
    /// The target this description serves.
    pub target: DescriptionTarget,
    /// English display text for clients.
    pub label: String,
    /// The program to start, absolute.
    pub executable: PathBuf,
    /// Arguments that precede `--input`, each one argument.
    pub args: Vec<String>,
    /// Paths the helper must read, absolute.
    pub read: Vec<PathBuf>,
}

/// The target a description names.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DescriptionTarget {
    pub provider_id: String,
    pub model_id: String,
}

impl HelperDescription {
    /// Whether this description names the one target this build offers from
    /// a plugin, `local/pp-ocr`; another target is not one Modality knows.
    pub fn names_plugin_target(&self) -> bool {
        self.target.provider_id == PLUGIN_PROVIDER && self.target.model_id == PLUGIN_MODEL
    }
}
