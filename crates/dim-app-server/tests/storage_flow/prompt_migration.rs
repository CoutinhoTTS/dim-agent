use super::*;
use dim_agent_capabilities::{defaults, methods::RpcError, prompt::Prompt};
use dim_agent_runtime::{ConfigureOutcome, PromptSourceTarget};
use std::path::Path;

fn builder(store: &Arc<SqliteTimelineStore>) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    builder
}

/// A Session whose `main` adopted the same source twice, then lost every saved
/// reference: the history the guard demands an explicit migration for.
///
/// The two adoptions are what makes the first `prompt/changed` differ from the
/// last, which is the condition `Projection::load` refuses without a saved
/// reference; dropping the recovery rows leaves exactly the store a branch
/// written before `prompt-state/v2` has.
async fn legacy_session(database: &Path) -> String {
    let db = SqliteStorage::open(database.to_owned()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let id = session.id().to_owned();
    let prompt = session.service::<Prompt>("main").unwrap();
    let scope = PromptScope::Branch {
        timeline: "main".into(),
    };
    let target = PromptSourceTarget {
        source: "domain-state".into(),
        scope: scope.clone(),
    };
    for version in ["first", "latest"] {
        let entry = PromptEntry {
            source: target.source.clone(),
            version: version.into(),
            body: format!("{version} domain state"),
            scope: scope.clone(),
            delivery: PromptDelivery::State,
            order: 0,
        };
        assert_eq!(
            prompt
                .clone()
                .configure_loaded(None, vec![(target.clone(), Some(entry))])
                .await
                .unwrap(),
            ConfigureOutcome::Adopted
        );
    }
    drop(prompt);
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
    let connection = Connection::open(database).unwrap();
    let changed: i64 = connection
        .query_row(
            "SELECT count(*) FROM timeline_events WHERE session_id=?1 AND kind='prompt/changed'",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(changed >= 2, "the seed carries {changed} adoptions");
    connection
        .execute(
            "DELETE FROM timeline_events WHERE session_id=?1 AND kind='prompt-state/recovered'",
            [&id],
        )
        .unwrap();
    drop(connection);
    id
}

/// Restores `id` in a fresh Agent and returns its prompt state, or the error
/// the guard raised.
async fn resume(
    database: &Path,
    id: &str,
) -> dim_agent_runtime::Result<dim_agent_capabilities::prompt_state::PromptSnapshot> {
    let db = SqliteStorage::open(database.to_owned()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let result = settled("the branch settles", agent.restore_session(id.to_owned())).await;
    let snapshot = match result {
        Ok(session) => session
            .get::<dim_agent_capabilities::prompt_state::PromptSnapshot>("main")
            .unwrap(),
        Err(error) => {
            let _ = agent.close().await;
            store.close().await.unwrap();
            db.close().await.unwrap();
            return Err(error);
        }
    };
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
    Ok(snapshot.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_branch_answers_its_migration_instruction_on_the_wire() {
    let database = Database::new();
    let id = legacy_session(&database.0).await;
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let agent = MinimalHost::start(&builder(&store)).await.unwrap();
    let failure = settled(
        "the guard refuses a branch with no saved references",
        agent.restore_session(id),
    )
    .await
    .err()
    .expect("legacy prompt adoption must require migration");
    assert!(
        matches!(&failure, AgentError::Framework(kind) if matches!(kind.as_ref(), rutis::host::ErrorKind::PluginFailed(source) if source.downcast_ref::<AgentError>().is_some())),
        "the refusal crosses the branch build as a wrapped AgentError: {failure:?}"
    );
    let wire = RpcError::from(failure);
    let app_error = wire
        .data
        .as_ref()
        .map(|data| data.app_error.as_str())
        .unwrap_or_default();
    assert_eq!(wire.code, -32000, "{wire:?}");
    assert_eq!(app_error, "session_invalid_input", "{wire:?}");
    assert!(
        wire.message.contains("dimcode --migrate-prompt-references"),
        "the client reads the command that shipped: {wire:?}"
    );
    let _ = agent.close().await;
    store.close().await.unwrap();
    db.close().await.unwrap();
}

