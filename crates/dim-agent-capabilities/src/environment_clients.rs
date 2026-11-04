//! Session client presence as a model-facing snapshot.
//!
//! The App Server owns the connection and UI registries and provides the
//! read-only `SessionClientsHost`. This capability appends a snapshot of the
//! attached connections at branch open and on every `ClientsChanged` fact.
use crate::branch_input::append_own;
use dim_agent_runtime::{Names, PartRegistry, PartScope, SessionIdentity, Timeline};
use dim_agent_types::*;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use std::sync::{Arc, Mutex};

/// The snapshot source name.
pub const CLIENTS: &str = "dim/environment/clients";
/// The branch plugin name.
pub const BRANCH_PART: &str = "EnvironmentClientsBranch";

/// A client-attachment change of one Session, delivered on the process bus.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ClientsChanged {
    #[serde(skip)]
    pub name: String,
    pub session_id: String,
}
impl rutis::Event for ClientsChanged {
    const NAME: &'static str = "environment/clients.changed";
    type Value = ();
    fn name(&self) -> &str {
        &self.name
    }
}
/// The bus channel of one Session's client changes.
pub fn clients_changes(session_id: &str) -> String {
    format!("session/{session_id}/clients")
}

/// The channels the model may see. `tui` is a UI leaf; a leaf's underlying
/// connection is not counted again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClientChannel {
    Tui,
    Cli,
    Web,
    Acp,
    Remote,
    Desktop,
    Unknown,
}
impl ClientChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Cli => "cli",
            Self::Web => "web",
            Self::Acp => "acp",
            Self::Remote => "remote",
            Self::Desktop => "desktop",
            Self::Unknown => "unknown",
        }
    }
    /// The channel a `hello`-declared identity maps to, before the UI leaf
    /// check. A leaf's connection is `tui` and never reaches this mapping.
    pub fn declared(kind: DeclaredClientKind, name: Option<&str>) -> Self {
        match kind {
            DeclaredClientKind::Cli => Self::Cli,
            DeclaredClientKind::Web => Self::Web,
            DeclaredClientKind::Other => match name {
                Some("AcpGateway") => Self::Acp,
                Some("Desktop") | Some("dim-desktop") => Self::Desktop,
                _ => Self::Unknown,
            },
        }
    }
}

/// The protocol's `hello.clientKind`, reduced to what presence needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclaredClientKind {
    Cli,
    Web,
    Other,
}

/// One attached connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientPresence {
    pub channel: ClientChannel,
    /// The client's self-declared `hello.clientName`; never an identity.
    pub label: Option<String>,
}

/// The App Server's read-only presence view.
pub trait SessionClientsSource: Send + Sync + 'static {
    /// The connections attached to `session_id` now. An unknown Session is empty.
    fn presences(&self, session_id: &str) -> Vec<ClientPresence>;
}

/// The process port. The App Server provides it from its connection and UI
/// registries; an assembly without it does not install this projection.
pub struct SessionClientsHost(pub Arc<dyn SessionClientsSource>);

/// Registers the presence branch plugin on behalf of the process plugin that
/// provides `SessionClientsHost`.
pub fn register_parts(ctx: &Ctx, parts: &Arc<PartRegistry>) -> Result<(), Error> {
    parts.register(ctx, PartScope::Branch, BRANCH_PART, |mount| {
        Ok(Some(Box::new(ClientsBranchPlugin::new(mount.names)?)))
    })
}

/// The body: one line per connection, no counts-only channel aggregation.
fn body(presences: &[ClientPresence]) -> String {
    if presences.is_empty() {
        return "No client connection is attached.".into();
    }
    let lines = presences
        .iter()
        .map(|presence| match presence.label.as_deref().map(label) {
            Some(label) => format!("- {} \"{label}\" (claimed)", presence.channel.as_str()),
            None => format!("- {}", presence.channel.as_str()),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let total = presences.len();
    format!(
        "{total} client connection{} attached:\n{lines}",
        if total == 1 { "" } else { "s" }
    )
}

/// A label is untrusted text: quotes and control characters cannot break the
/// body, and a long name is cut to 64 characters.
fn label(label: &str) -> String {
    label
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn snapshot(presences: &[ClientPresence]) -> String {
    format!(
        "<dim-state source=\"{CLIENTS}\">{}</dim-state>",
        body(presences)
    )
}

fn append(timeline: &Timeline, body: &str) -> dim_agent_runtime::Result<()> {
    append_own(
        timeline,
        InputMessageData {
            blocks: vec![InputBlock::Raw {
                text: body.to_owned(),
                key: None,
                retire: false,
            }],
            ..InputMessageData::default()
        },
        Vec::new(),
    )
    .map(|_| ())
}

struct ClientsBranchPlugin {
    clients: Key<SessionClientsHost>,
    identity: Key<SessionIdentity>,
    timeline: Key<Timeline>,
}
impl ClientsBranchPlugin {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            clients: names.key()?,
            identity: names.key()?,
            timeline: names.key()?,
        })
    }
}
impl DynPlugin for ClientsBranchPlugin {
    fn name(&self) -> &str {
        BRANCH_PART
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.clients.erase(),
            self.identity.erase(),
            self.timeline.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let clients = ctx.require_at(&self.clients)?;
            let identity = ctx.require_at(&self.identity)?;
            let timeline = ctx.require_at(&self.timeline)?;
            let last = Arc::new(Mutex::new(None));
            let initial = snapshot(&clients.0.presences(&identity.session_id));
            if let Err(error) = append(&timeline, &initial) {
                tracing::warn!(%error, "client presence was not appended");
            }
            *last.lock().expect("presence body") = Some(initial);
            let session = identity.session_id.clone();
            let last = last.clone();
            ctx.listen::<ClientsChanged>()
                .name(clients_changes(&session))
                .on(move |_, _| {
                    let current = snapshot(&clients.0.presences(&session));
                    let mut last = last.lock().expect("presence body");
                    if last.as_deref() == Some(current.as_str()) {
                    } else if let Err(error) = append(&timeline, &current) {
                        tracing::warn!(%error, "client presence was not appended");
                    } else {
                        *last = Some(current);
                    }
                    Box::pin(async { Ok(None) })
                })?;
            Ok(Effect::Done)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_session_is_stated_as_such() {
        assert_eq!(body(&[]), "No client connection is attached.");
    }

    #[test]
    fn every_connection_is_listed() {
        let presences = vec![
            ClientPresence {
                channel: ClientChannel::Tui,
                label: Some("eric@laptop".into()),
            },
            ClientPresence {
                channel: ClientChannel::Web,
                label: None,
            },
        ];
        assert_eq!(
            body(&presences),
            "2 client connections attached:\n- tui \"eric@laptop\" (claimed)\n- web"
        );
    }

    #[test]
    fn a_label_cannot_break_the_body() {
        let presences = vec![ClientPresence {
            channel: ClientChannel::Cli,
            label: Some("a\"b\nc".into()),
        }];
        assert_eq!(
            body(&presences),
            "1 client connection attached:\n- cli \"a\\\"bc\" (claimed)"
        );
    }
}
