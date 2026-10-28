mod permissions;

use crate::{
    Error, Migration, ObjectResult, Outcome, Result, TaskState, engine::request, source::Source,
};
use dim_agent_capabilities::file_storage::FileStorageError;
use dim_client::Client;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

async fn record(
    owner: &Migration,
    task: &TaskState,
    id: String,
    outcome: Outcome,
    reason: Option<String>,
) -> Result<()> {
    owner
        .ledger
        .object(
            &task.id,
            &ObjectResult {
                kind: "setting".into(),
                id,
                outcome,
                target_id: None,
                reason,
                omitted: Vec::new(),
            },
        )
        .await
}

async fn fill(
    owner: &Migration,
    task: &TaskState,
    client: &mut Client,
    source: &str,
    fields: Value,
) -> Result<()> {
    if !client.offers("settings.fill") {
        return record(
            owner,
            task,
            source.into(),
            Outcome::Unsupported,
            Some("Target Settings does not expose missing-field import".into()),
        )
        .await;
    }
    let result: Result<BTreeMap<String, String>> = request(
        client,
        "settings.fill",
        json!({"source":source,"document":fields}),
    )
    .await;
    match result {
        Ok(results) => {
            for (field, status) in results {
                let outcome = match status.as_str() {
                    "imported" => Outcome::Imported,
                    "unchanged" => Outcome::AlreadyImported,
                    "conflict" => Outcome::Conflict,
                    "rejected" => Outcome::Unsupported,
                    _ => return Err(Error::Target("Unknown Settings field outcome".into())),
                };
                record(owner,task,format!("{source}.{field}"),outcome,if matches!(outcome,Outcome::Conflict|Outcome::Unsupported){Some("Target configuration is preserved; incompatible source data remains in the legacy home".into())}else{None}).await?;
            }
        }
        Err(Error::Object(reason)) => {
            record(
                owner,
                task,
                source.into(),
                Outcome::Unsupported,
                Some(reason),
            )
            .await?
        }
        Err(Error::Unsupported(reason)) => {
            record(
                owner,
                task,
                source.into(),
                Outcome::Unsupported,
                Some(reason),
            )
            .await?
        }
        Err(Error::Deferred(reason)) => {
            record(owner, task, source.into(), Outcome::Deferred, Some(reason)).await?
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

async fn file_fill(
    owner: &Migration,
    task: &TaskState,
    name: &str,
    fields: Map<String, Value>,
) -> Result<()> {
    let path = owner.ledger.files.path(0, name)?;
    let mut lease = owner.ledger.files.lock(&path, &owner.stopping).await?;
    let mut current: Value = match lease.read(&owner.stopping).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| Error::Target(format!("Target {name} is invalid; preserved")))?,
        Err(FileStorageError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({})
        }
        Err(error) => return Err(error.into()),
    };
    let members = current
        .as_object_mut()
        .ok_or_else(|| Error::Target(format!("Target {name} must be an object; preserved")))?;
    let mut changed = false;
    let mut results = Vec::new();
    for (field, value) in fields {
        let outcome = match members.get(&field) {
            Some(existing) if existing == &value => Outcome::AlreadyImported,
            Some(_) => Outcome::Conflict,
            None => {
                members.insert(field.clone(), value);
                changed = true;
                Outcome::Imported
            }
        };
        results.push((field, outcome));
    }
    if changed {
        let bytes = serde_json::to_vec_pretty(&current)
            .map_err(|_| Error::Target("Cannot encode client settings".into()))?;
        #[cfg(unix)]
        let permissions = Some(std::os::unix::fs::PermissionsExt::from_mode(0o600));
        #[cfg(not(unix))]
        let permissions = None;
        lease.replace(bytes, permissions, &owner.stopping).await?;
    }
    for (field, outcome) in results {
        record(owner, task, format!("{name}.{field}"), outcome, None).await?;
    }
    Ok(())
}

pub(crate) async fn import(
    owner: &Migration,
    task: &TaskState,
    source: &Source,
    client: &mut Client,
) -> Result<()> {
    let mut cli = crate::engine::document(owner, task, source, "config.json")
        .await?
        .unwrap_or_else(|| json!({}));
    let desktop = crate::engine::document(owner, task, source, "desktop-config.json").await?;
    let cli_fields = cli
        .as_object_mut()
        .ok_or_else(|| Error::Object("Legacy CLI settings must be an object".into()))?;
    if let Some(desktop) = desktop {
        let settings = desktop
            .get("settings")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                Error::Object("Legacy Desktop settings must contain a settings object".into())
            })?;
        for (key, value) in settings {
            if [
                "locale",
                "subagentModel",
                "assistantModel",
                "subagentMaxConcurrency",
                "customInstructions",
                "defaultToolApprovalMode",
                "sandboxMode",
                "sandboxNetworkMode",
                "sandboxAllowedDomains",
            ]
            .contains(&key.as_str())
            {
                cli_fields.insert(key.clone(), value.clone());
            } else {
                record(
                    owner,
                    task,
                    format!("desktop.{key}"),
                    Outcome::Deferred,
                    Some("Desktop-specific configuration remains in the legacy home".into()),
                )
                .await?;
            }
        }
    }
    let mut preferences = Map::new();
    for (key, value) in cli_fields.iter() {
        let compatible = match key.as_str() {
            "locale" => {
                value.is_null()
                    || value.as_str().is_some_and(|value| {
                        [
                            "en", "zh-CN", "zh-Hant", "ja", "ko", "es", "fr", "de", "pt-BR",
                            "pt-PT", "id",
                        ]
                        .contains(&value)
                    })
            }
            "themeId" => value
                .as_str()
                .is_some_and(|value| owner.themes.iter().any(|theme| theme == value)),
            "tuiMode" => value
                .as_str()
                .is_some_and(|value| ["fullscreen", "scrollback"].contains(&value)),
            "tuiMouseMode" => value
                .as_str()
                .is_some_and(|value| ["on", "off"].contains(&value)),
            "tuiNotifyOnComplete" | "tuiToolDetailsExpanded" => value.is_boolean(),
            _ => false,
        };
        if compatible {
            preferences.insert(key.clone(), value.clone());
        } else if ![
            "subagentModel",
            "logLevel",
            "defaultProvider",
            "defaultModel",
        ]
        .contains(&key.as_str())
        {
            record(
                owner,
                task,
                format!("config.{key}"),
                Outcome::Unsupported,
                Some("No equivalent target field mapping; source preserved".into()),
            )
            .await?;
        }
    }
    file_fill(owner, task, "config.json", preferences).await?;
    if let Some(model) = cli_fields.get("subagentModel") {
        let policy = if model.is_null() {
            Some(json!({"mode":"inherit"}))
        } else if let (Some(provider), Some(model)) = (
            model.get("providerId").and_then(Value::as_str),
            model.get("modelId").and_then(Value::as_str),
        ) {
            Some(json!({"mode":"fixed","model":{"providerId":provider,"modelId":model}}))
        } else {
            None
        };
        match policy {
            Some(policy) => fill(owner, task, client, "collaboration", policy).await?,
            None => {
                record(
                    owner,
                    task,
                    "config.subagentModel".into(),
                    Outcome::Unsupported,
                    Some("Legacy model-decides policy has no equivalent target policy".into()),
                )
                .await?
            }
        }
    }
    if let Some(skills) = crate::engine::document(owner, task, source, "skills.json").await?
        && let Some(ids) = skills.get("disabledSkillIds")
    {
        fill(
            owner,
            task,
            client,
            "skills",
            json!({"disabledSkillIds":ids}),
        )
        .await?;
    }
    if let Some(mcp) = crate::engine::document(owner, task, source, "mcp.json").await?
        && let Some(servers) = mcp.get("mcpServers")
    {
        fill(owner, task, client, "mcp", json!({"mcpServers":servers})).await?;
    }
    if let Some(shell) = crate::engine::document(owner, task, source, "exec-shell.json").await? {
        let valid = shell.get("version") == Some(&json!(1))
            && shell.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .all(|key| ["version", "executablePath"].contains(&key.as_str()))
            })
            && shell.get("executablePath").is_none_or(|path| {
                path.as_str()
                    .is_some_and(|path| std::path::Path::new(path).is_absolute())
            });
        if valid {
            file_fill(
                owner,
                task,
                "exec-shell.json",
                shell.as_object().unwrap().clone(),
            )
            .await?;
        } else {
            record(
                owner,
                task,
                "exec-shell".into(),
                Outcome::Unsupported,
                Some("Legacy shell selection is incompatible".into()),
            )
            .await?;
        }
    }
    if let Some(permission) =
        crate::engine::document(owner, task, source, "permissions.json").await?
    {
        permissions::import(owner, task, client, permission).await?;
    }
    for (file, target) in [
        ("schedule.json", "schedule"),
        ("workflow.json", "workflow"),
        ("web.json", "web"),
        ("prompt-locale.json", "prompt-locale"),
    ] {
        if let Some(document) = crate::engine::document(owner, task, source, file).await? {
            fill(owner, task, client, target, document).await?;
        }
    }
    Ok(())
}
