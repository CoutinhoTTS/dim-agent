//! The Settings change contract, for listeners in any crate. The Settings
//! service itself belongs to the App Server, which emits these events; a
//! consumer listens with `ctx.events().on` and rereads its source.
use rutis::Event;
use serde::{Deserialize, Serialize};

/// `"user"` or `{"project": root}`. `root` is the project root number
/// Settings assigned in this process, never a durable identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsScope {
    User,
    Project(usize),
}

/// A committed write to one Settings source. It carries the revision written,
/// never the document: a listener reads the source when it needs the content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChanged {
    /// The source's channel it is sent under ([`settings_changes`]).
    #[serde(skip)]
    pub name: String,
    pub source: String,
    pub scope: SettingsScope,
    pub revision: String,
}
impl Event for SettingsChanged {
    const NAME: &'static str = "settings.changed";
    type Value = ();
    fn name(&self) -> &str {
        &self.name
    }
}
/// The rutis channel of one source's committed writes, in commit order.
pub fn settings_changes(source: &str) -> String {
    format!("settings/{source}")
}
