use crate::{
    Error, Migration, ObjectResult, Outcome, Result, TaskState, engine::request, source::Source,
};
use dim_agent_types::AccountImportStatus;
use dim_client::Client;
use serde_json::{Value, json};

async fn account(
    owner: &Migration,
    task: &TaskState,
    client: &mut Client,
    id: &str,
    method: &str,
    params: Value,
) -> Result<()> {
    let result: Result<AccountImportStatus> = if client.offers(method) {
        request(client, method, params).await
    } else {
        Err(Error::Object(
            "This target profile does not expose the account owner's import operation".into(),
        ))
    };
    let (outcome, reason) = match result {
        Ok(AccountImportStatus::Imported) => (Outcome::Imported, None),
        Ok(AccountImportStatus::AlreadyImported) => (Outcome::AlreadyImported, None),
        Ok(AccountImportStatus::Conflict) => (
            Outcome::Conflict,
            Some(
                "Target account state is configured or source identity changed; target preserved"
                    .into(),
            ),
        ),
        Err(Error::Object(reason)) => (Outcome::Unsupported, Some(reason)),
        Err(Error::Unsupported(reason)) => (Outcome::Unsupported, Some(reason)),
        Err(Error::Deferred(reason)) => (Outcome::Deferred, Some(reason)),
        Err(error) => return Err(error),
    };
    owner
        .ledger
        .object(
            &task.id,
            &ObjectResult {
                kind: "account".into(),
                id: id.into(),
                outcome,
                target_id: None,
                reason,
                omitted: Vec::new(),
            },
        )
        .await
}

pub(crate) async fn import(
    owner: &Migration,
    task: &TaskState,
    source: &Source,
    client: &mut Client,
) -> Result<()> {
    if let Some(auth) = crate::engine::document(owner, task, source, "auth.json").await? {
        if !auth.is_object() {
            return Err(Error::Object("Legacy auth.json must be an object".into()));
        }
        for (id, key, state, method) in [
            (
                "dimcode-api-oauth",
                "nextApiOauth",
                "dimAccountState",
                "account.import_legacy",
            ),
            (
                "openai-codex",
                "openaiCodex",
                "openaiCodexAccountState",
                "account.codex.import_legacy",
            ),
            (
                "xai-grok",
                "xaiGrokBuild",
                "xaiGrokBuildAccountState",
                "account.grok.import_legacy",
            ),
        ] {
            if auth.get(key).is_none() {
                continue;
            }
            let mut document = serde_json::Map::new();
            for field in [key, state] {
                if let Some(value) = auth.get(field) {
                    document.insert(field.into(), value.clone());
                }
            }
            let params = if method == "account.import_legacy" {
                json!({"providerId":id,"document":document})
            } else {
                json!({"document":document})
            };
            account(owner, task, client, id, method, params).await?;
        }
    }
    if let Some(mcp) = crate::engine::document(owner, task, source, "mcp-oauth.json").await? {
        if mcp.get("version") != Some(&json!(1)) {
            return Err(Error::Object(
                "Legacy MCP OAuth schema is unsupported".into(),
            ));
        }
        let servers = mcp
            .get("servers")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Object("Legacy MCP OAuth server records are invalid".into()))?;
        for (id, entry) in servers {
            account(
                owner,
                task,
                client,
                &format!("mcp:{id}"),
                "mcp.auth.import_legacy",
                json!({"id":id,"entry":entry}),
            )
            .await?;
        }
    }
    Ok(())
}
