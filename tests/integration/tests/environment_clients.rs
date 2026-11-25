use crate::local_support;

use dim_agent_capabilities::environment_clients::{
    BRANCH_PART, CLIENTS, ClientChannel, ClientPresence, ClientsChanged, SessionPresences,
    clients_changes,
};
use dim_agent_runtime::{BranchOptions, Session, testing::branch_events};
use dim_agent_types::{BranchSource, EventKind, InputBlock};
use local_support::*;
use rutis::host::{CtxHost, FiberState, InstanceId};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A mutable presence source: the test changes the connections, as another
/// client would, and announces the change on the process bus.
struct Presence(Mutex<Vec<ClientPresence>>);
impl Presence {
    fn set(&self, presences: Vec<ClientPresence>) {
        *self.0.lock().unwrap() = presences;
    }
    fn presences(&self) -> Vec<ClientPresence> {
        self.0.lock().unwrap().clone()
    }
    fn host(self: &Arc<Self>) -> SessionPresences {
        let presence = self.clone();
        Arc::new(move |_| presence.presences())
    }
}

fn connection(channel: ClientChannel, label: Option<&str>) -> ClientPresence {
    ClientPresence {
        channel,
        label: label.map(str::to_owned),
    }
}

fn changed(env: &Env, session: &Session) {
    env.root
        .emit(ClientsChanged {
            name: clients_changes(session.id()),
            session_id: session.id().to_owned(),
        })
        .unwrap();
}

async fn until_block(session: &Session, needle: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let found = inputs(session)
                .await
                .iter()
                .flat_map(|input| &input.blocks)
                .any(|block| {
                    let dim_agent_types::InputBlock::Raw { text, .. } = block;
                    text.contains(needle)
                });
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("presence was appended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presence_is_appended_and_unchanged_counts_do_not_duplicate() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("work");
    let presence = Arc::new(Presence(Mutex::new(Vec::new())));
    presence.set(vec![connection(ClientChannel::Tui, Some("eric@laptop"))]);
    let env = env(
        &dirs,
        Options {
            clients: Some(presence.host()),
            ..Options::default()
        },
    )
    .await;
    let session = env.session(&cwd, "").await.unwrap();

    run(&session, "hello").await;
    until_block(&session, "1 client connection attached:").await;
    run(&session, "hello again").await;
    let text = request_text(requests(&session).await.last().unwrap());
    assert!(text.contains("1 client connection attached:"), "{text}");
    assert!(text.contains("- tui \"eric@laptop\" (claimed)"), "{text}");

    presence.set(vec![
        connection(ClientChannel::Tui, Some("eric@laptop")),
        connection(ClientChannel::Tui, None),
        connection(ClientChannel::Web, Some("alice")),
    ]);
    changed(&env, &session);
    run(&session, "again").await;
    until_block(&session, "3 client connections attached:").await;
    run(&session, "third").await;
    let text = request_text(requests(&session).await.last().unwrap());
    assert!(text.contains("- tui \"eric@laptop\" (claimed)"), "{text}");
    assert!(text.contains("- tui\n"), "{text}");
    assert!(text.contains("- web \"alice\" (claimed)"), "{text}");
    assert_eq!(
        text.matches("3 client connections attached:").count(),
        1,
        "an unchanged presence must not write another block:\n{text}"
    );

    presence.set(Vec::new());
    changed(&env, &session);
    run(&session, "done").await;
    until_block(&session, "No client connection is attached.").await;
    run(&session, "later").await;
    let text = request_text(requests(&session).await.last().unwrap());
    assert!(
        text.contains(
            "<dim-state source=\"dim/environment/clients\">No client connection is attached.</dim-state>"
        ),
        "{text}"
    );

    env.agent.close().await.unwrap();
}

/// The instances of the branches that have a presence part, so a flow can
/// tell the line it opens from the lines that were already there.
fn presence_instances(root: &rutis::Ctx) -> HashSet<InstanceId> {
    root.diagnostics()
        .plugins
        .iter()
        .filter(|plugin| plugin.name == BRANCH_PART)
        .map(|plugin| plugin.instance)
        .collect()
}

/// Waits until a presence part is Active in an instance the line did not have
/// before it opened.
async fn until_presence_part_of_new_instance(
    root: &rutis::Ctx,
    known: &HashSet<InstanceId>,
) -> rutis::host::FiberView {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(plugin) = root.diagnostics().plugins.iter().find(|plugin| {
                plugin.name == BRANCH_PART
                    && plugin.state == FiberState::Active
                    && !known.contains(&plugin.instance)
            }) {
                return root.view(plugin.id).expect("the presence part's view");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the line's presence part mounts while the line loads")
}

async fn until_active(root: &rutis::Ctx, id: rutis::host::PluginId) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let active = root
                .diagnostics()
                .plugins
                .iter()
                .any(|plugin| plugin.id == id && plugin.state == FiberState::Active);
            if active {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the replacement part becomes Active while the line loads");
}

/// Replacing the presence part while a line still loads keeps the line's
/// identity input first: the retired receipt writes nothing, the replacement
/// appends the line's one presence snapshot after activation, and the append
/// still waits for the branch to dispatch it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_part_during_loading_writes_one_presence_snapshot_after_the_identity_input() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("work");
    let presence = Arc::new(Presence(Mutex::new(Vec::new())));
    let hold = Arc::new(BranchHold::default());
    let env = env(
        &dirs,
        Options {
            clients: Some(presence.host()),
            branch_hold: Some(hold.clone()),
            ..Options::default()
        },
    )
    .await;
    let session = env.session(&cwd, "").await.unwrap();
    let known = presence_instances(&env.root);
    let at = session.sequence("main").unwrap();

    hold.arm();
    let opening = tokio::spawn({
        let session = session.clone();
        async move {
            session
                .branch(
                    "main".into(),
                    at,
                    BranchOptions {
                        prompt: "line work".into(),
                        source: BranchSource::Host {
                            operation_id: "part-replaced".into(),
                        },
                    },
                )
                .await
        }
    });
    hold.entered.notified().await;
    let part = until_presence_part_of_new_instance(&env.root, &known).await;
    part.restart().await.unwrap();
    until_active(&env.root, part.id).await;
    hold.release.notify_one();
    let child = opening.await.unwrap().expect("the line opens");

    let marker = format!("<dim-state source=\"{CLIENTS}\">");
    let is_presence = |event: &dim_agent_types::AgentEvent| {
        matches!(&event.kind, EventKind::InputMessage(data)
            if data.user.is_none()
                && data.blocks.iter().any(|block| matches!(block,
                    InputBlock::Raw { text, .. } if text.starts_with(&marker))))
    };
    let events = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = branch_events(&session, &child, ..).await.unwrap();
            if events.iter().any(|event| is_presence(event)) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the line publishes its presence snapshot");
    let identity_marker =
        format!(r#"<dim-branch-line branch="{child}" parent="main" source="host">"#);
    let identity = events
        .iter()
        .find(|event| {
            matches!(&event.kind, EventKind::InputMessage(data)
                if data.blocks.first().is_some_and(|block| matches!(block,
                    InputBlock::Raw { text, .. } if text.starts_with(&identity_marker))))
        })
        .expect("the line's identity input")
        .sequence;
    let snapshots: Vec<_> = events.iter().filter(|event| is_presence(event)).collect();
    for snapshot in &snapshots {
        assert!(
            snapshot.sequence > identity,
            "a presence snapshot precedes the identity input: {} before {identity} ({} snapshots)",
            snapshot.sequence,
            snapshots.len()
        );
    }
    assert_eq!(
        snapshots.len(),
        1,
        "the replacement writes the presence snapshot exactly once"
    );
    assert!(
        events.iter().any(|event| matches!(&event.kind,
            EventKind::InputApplied(data) if data.input_id == snapshots[0].id)),
        "the branch dispatches the snapshot and writes its input/applied"
    );

    env.agent.close().await.unwrap();
}
