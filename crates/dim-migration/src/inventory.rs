use crate::{
    Error, Migration, ObjectResult, Outcome, Result, TaskState,
    engine::{check, document},
    source::Source,
};
use dim_agent_capabilities::file_storage::EntryKind;
use serde_json::Value;

async fn retained(owner: &Migration, task: &TaskState, id: String, reason: &str) -> Result<()> {
    owner
        .ledger
        .object(
            &task.id,
            &ObjectResult {
                kind: "retained_source".into(),
                id,
                outcome: Outcome::Deferred,
                target_id: None,
                reason: Some(reason.into()),
                omitted: Vec::new(),
            },
        )
        .await
}

pub(crate) async fn inspect(owner: &Migration, task: &TaskState, source: &Source) -> Result<()> {
    for file in [
        "hooks.json",
        "modality.json",
        "web-plugin.json",
        "mcp-projects.json",
        "remote.json",
    ] {
        check(owner, &task.id).await?;
        if document(owner, task, source, file).await?.is_some() {
            retained(owner,task,file.into(),"Source configuration requires an equivalent owner mapping; retained for incremental import").await?;
        }
    }
    for entry in source.entries("data/plugins", &owner.stopping).await? {
        check(owner, &task.id).await?;
        let plugin = entry
            .name
            .to_str()
            .ok_or_else(|| Error::Object("Legacy plugin name is not UTF-8".into()))?;
        if entry.kind != EntryKind::Directory {
            retained(
                owner,
                task,
                format!("data/plugins/{plugin}"),
                "Unrecognized plugin storage entry; source is preserved",
            )
            .await?;
            continue;
        }
        let path = format!("data/plugins/{plugin}/state.json");
        if document(owner, task, source, &path).await?.is_some() {
            retained(owner,task,path,if plugin=="schedule" {"Legacy Schedule state controls tool visibility, not job execution; no equivalent native visibility field exists"} else {"Private plugin state requires its schema owner's import mapping; it is not copied as native state"}).await?;
        }
        let sessions = format!("data/plugins/{plugin}/sessions");
        if !source.entries(&sessions, &owner.stopping).await?.is_empty() {
            retained(
                owner,
                task,
                sessions,
                "Session-private plugin state remains in the source for incremental import",
            )
            .await?;
        }
    }
    for directory in ["skills", "data/skills", "data/workflows/saved"] {
        if !source.entries(directory, &owner.stopping).await?.is_empty() {
            retained(owner,task,directory.into(),if directory.ends_with("saved") {"Legacy executable Workflow definitions cannot be registered as native declarative Workflows; run artifacts are preserved with their Sessions"}else{"Source-owned Skills require an explicit catalog/root mapping; shared external roots are reused without copying"}).await?;
        }
    }
    if let Some(config) = document(owner, task, source, "config.json").await?
        && let Some(path) = config.get("workflowDataDir").and_then(Value::as_str)
    {
        retained(owner,task,format!("workflow-directory:{}",crate::source::fingerprint(&path)?),"Custom Workflow storage remains in the source; available Session run artifacts are imported only inside the authorized source home").await?;
    }
    Ok(())
}
