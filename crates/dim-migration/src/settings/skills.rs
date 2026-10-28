use super::{fill, record};
use crate::{Migration, Outcome, Result, TaskState};
use dim_client::Client;
use serde_json::{Value, json};
use std::path::{Component, Path};

pub(super) async fn import(
    owner: &Migration,
    task: &TaskState,
    client: &mut Client,
    document: Value,
) -> Result<()> {
    let Some(fields) = document.as_object() else {
        return record(
            owner,
            task,
            "skills".into(),
            Outcome::Unsupported,
            Some("Legacy Skills configuration must be an object".into()),
        )
        .await;
    };
    for key in fields
        .keys()
        .filter(|key| key.as_str() != "disabledSkillIds")
    {
        record(
            owner,
            task,
            format!("skills.{key}"),
            Outcome::Unsupported,
            Some("No equivalent Skills configuration field".into()),
        )
        .await?;
    }
    let Some(ids) = fields.get("disabledSkillIds") else {
        return Ok(());
    };
    let compatible = ids.as_array().is_some_and(|ids| {
        ids.iter().all(|id| {
            let Some(id) = id.as_str() else { return false };
            if let Some(relative) = id.strip_prefix("user:") {
                !relative.is_empty()
                    && Path::new(relative).components().count() == 1
                    && Path::new(relative)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                    && !relative.contains([':', '\\', '/'])
            } else if let Some((root, relative)) = id
                .strip_prefix("workspace:")
                .and_then(|id| id.rsplit_once(':'))
            {
                Path::new(root).is_absolute()
                    && Path::new(root).ends_with(".agents/skills")
                    && !relative.is_empty()
                    && Path::new(relative)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                    && !relative.contains('\\')
            } else {
                false
            }
        })
    });
    if !compatible {
        return record(owner, task, "skills.disabledSkillIds".into(), Outcome::Deferred,
            Some("The complete list remains in the source: host/plugin IDs and ambiguous nested user IDs need a source-root mapping; no partial list is sealed as configured".into())).await;
    }
    fill(
        owner,
        task,
        client,
        "skills",
        json!({"disabledSkillIds":ids}),
    )
    .await
}
