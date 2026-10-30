use serde::{Deserialize, Serialize};

/// The process boundary the host applies; wider tiers expose more host access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sandbox {
    Workspace,
    Network,
    Host,
}

impl Sandbox {
    pub fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Network => "network",
            Self::Host => "host",
        }
    }
}
