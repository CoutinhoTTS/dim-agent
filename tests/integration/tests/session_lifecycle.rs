//! SessionLifecycle follows publication and close admission (AGENTS rule 1,
//! lifecycle §3): Session parts start deferred work only for a published
//! Session and end it as soon as close starts.
mod local_support;

use dim_agent_runtime::*;
use local_support::*;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_close_during_assembly_never_activates_the_session() {
    let dirs = Dirs::new();
    let hold = Arc::new(AssemblyHold::default());
    let probe = Arc::new(LifecycleProbe::default());
    let env = env(
        &dirs,
        Options {
            hold: Some(hold.clone()),
            probe: Some(probe.clone()),
            ..Options::default()
        },
    )
    .await;
    let cwd = dirs.dir("work");
    hold.arm();
    let agent = env.agent.clone();
    let config = SessionConfig {
        cwd: cwd.to_string_lossy().into_owned(),
        ..SessionConfig::default()
    };
    let creating = tokio::spawn(async move { agent.create_session(config).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), hold.entered.notified())
        .await
        .expect("assembly reached the hold");
    probe.wait_for("applied", 1).await;
    let closing = env.agent.close();
    hold.release.notify_one();
    assert!(matches!(
        creating.await.unwrap(),
        Err(AgentError::Cancelled(_))
    ));
    closing.await.unwrap();
    probe.wait_for("inactive", 1).await;
    assert_eq!(probe.count("active"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_close_reports_closing_to_every_session_at_once() {
    let dirs = Dirs::new();
    let probe = Arc::new(LifecycleProbe::default());
    let env = env(
        &dirs,
        Options {
            probe: Some(probe.clone()),
            ..Options::default()
        },
    )
    .await;
    let cwd = dirs.dir("work");
    let _first = env.session(&cwd, "").await.unwrap();
    let _second = env.session(&cwd, "").await.unwrap();
    probe.wait_for("active", 2).await;
    // Hold the first Session's unload: Agent::close closes Sessions in turn,
    // but both must observe closing as soon as it starts.
    probe.hold_next_close();
    let closing = env.agent.close();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        probe.close_held.notified(),
    )
    .await
    .expect("a Session close reached the hold");
    probe.wait_for("closing", 2).await;
    probe.release_close.notify_one();
    closing.await.unwrap();
}
