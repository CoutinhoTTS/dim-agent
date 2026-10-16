//! The Settings change contract, for listeners in any crate. The Settings
//! service itself belongs to the App Server, which emits these events; a
//! consumer listens with `ctx.events().on` and rereads its source.
use rutis::{Event, EventKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingsScope {
    User,
    Project(usize),
}

/// A committed write to one Settings source. It carries the revision written,
/// never the document: a listener reads the source when it needs the content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsChanged {
    pub source: &'static str,
    pub scope: SettingsScope,
    pub revision: String,
}
impl Event for SettingsChanged {
    const NAME: &'static str = "settings.changed";
    type Value = ();
}
/// The rutis channel of one source's committed writes, in commit order.
pub fn settings_changes(source: &str) -> EventKey<SettingsChanged> {
    EventKey::dynamic(format!("settings/{source}"))
}
