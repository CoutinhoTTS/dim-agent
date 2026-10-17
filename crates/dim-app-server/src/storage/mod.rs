mod codec;
mod files;
mod metadata;
mod sqlite;
mod timeline;

pub use metadata::{MetadataError, SessionMetadata, Title, TitleFields, TitleSource};
pub use sqlite::{SqliteStorage, SqliteStoragePlugin};
pub use timeline::{SessionStore, SessionStorePlugin, SqliteTimelineStore};

/// The directory below the database directory where WorkspaceTools keeps each
/// Session's command output, `command-output/<sessionId>/`. The daemon gives
/// it to WorkspaceTools as its output root; a Session delete removes the
/// Session's directory.
pub const COMMAND_OUTPUT_DIRECTORY: &str = "command-output";
