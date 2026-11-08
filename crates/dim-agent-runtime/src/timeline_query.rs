//! Property queries and results shared by Timeline services and storage.
//! See [plugin contract §5.4](../../../docs/design/plugin-contract.md#54-the-event-query).

use serde::{Deserialize, Serialize};

mod properties;
pub use properties::*;

/// Which branches a query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineQueryScope {
    /// The queried branch alone.
    Branch,
    /// The branch chain: each ancestor up to its cut, root first, then the
    /// queried branch.
    Chain,
}

/// Direction of one result ordering key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineQueryDirection {
    /// Increasing key values; position order runs from root to branch.
    #[default]
    Ascending,
    /// Decreasing key values.
    Descending,
}
