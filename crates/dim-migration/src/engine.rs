use crate::{
    Error, Migration, ObjectResult, Outcome, Result, TaskReport, TaskState, TaskStatus,
    source::{SessionData, Source, fingerprint},
};
use base64::Engine;
use dim_agent_types::*;
use dim_client::Client;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(crate) async fn request<T: DeserializeOwned>(
    client: &mut Client,
    method: &str,
    params: impl Serialize,
) -> Result<T> {
    let params = serde_json::to_value(params)
        .map_err(|_| Error::Object("Target request cannot be encoded".into()))?;
    match tokio::time::timeout(Duration::from_secs(60), client.typed(method, params)).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(error)) => {
            if let Some(rpc) = error.downcast_ref::<dim_client::RpcError>() {
                let code = rpc
                    .data
                    .as_ref()
                    .and_then(|data| data.get("appError"))
                    .and_then(Value::as_str)
                    .unwrap_or("rpc_error");
                if rpc.code == -32601 {
                    return Err(Error::Unsupported(format!(
                        "Target does not expose {method}"
                    )));
                }
                if matches!(code, "service_reloading" | "service_unavailable") {
                    return Err(Error::Deferred(format!(
                        "Target owner of {method} is unavailable"
                    )));
                }
                if matches!(code, "plugin_not_found" | "plugin_locked") {
                    return Err(Error::Unsupported(
                        "Target plugin selection cannot be imported".into(),
                    ));
                }
                if code == "schedule_import_unsupported" {
                    return Err(Error::Unsupported(
                        "Legacy recurrence has no equivalent native schedule".into(),
                    ));
                }
                if rpc.code == -32602
                    || matches!(
                        code,
                        "provider_invalid"
                            | "session_invalid_input"
                            | "settings_conflict"
                            | "provider_import_rejected"
                            | "settings_unknown_source"
                            | "account_provider_unavailable"
                            | "account_unsupported_schema"
                            | "account_invalid_store"
                            | "account_invalid_input"
                            | "account_recovery_required"
                            | "account_required"
                            | "account_identity_rejected"
                            | "account_transient"
                            | "account_changed"
                            | "account_unavailable"
                            | "account_failed"
                            | "mcp_auth_unsupported"
                            | "mcp_unknown_server"
                            | "mcp_invalid_config"
                            | "schedule_invalid"
                            | "schedule_conflict"
                            | "schedule_not_found"
                    )
                {
                    return Err(Error::Object(format!("Target rejected {method}: {code}")));
                }
                return Err(Error::Target(format!(
                    "Target operation {method} failed: {code} ({})",
                    rpc.code
                )));
            }
            Err(Error::Target(format!(
                "Target connection failed during {method}; retry reconciles durable receipts"
            )))
        }
        Err(_) => Err(Error::Target(format!(
            "Target did not confirm {method}; accepted work may have committed, retry to reconcile"
        ))),
    }
}

pub(crate) async fn publish(owner: &Migration, task: &TaskState) -> Result<()> {
    owner.ledger.save(task).await?;
    owner.progress.send_replace(Some(task.clone()));
    Ok(())
}

pub(crate) async fn check(owner: &Migration, id: &str) -> Result<()> {
    if owner.stopping.is_cancelled() || owner.ledger.cancelled(id).await? {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) async fn document(
    owner: &Migration,
    task: &TaskState,
    source: &Source,
    name: &str,
) -> Result<Option<Value>> {
    match source.json(name, &owner.stopping).await {
        Err(Error::Object(reason)) => {
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: "file".into(),
                        id: name.into(),
                        outcome: Outcome::Failed,
                        target_id: None,
                        reason: Some(reason),
                        omitted: Vec::new(),
                    },
                )
                .await?;
            Ok(None)
        }
        result => result,
    }
}

pub(crate) async fn run(
    owner: Arc<Migration>,
    source_home: PathBuf,
    retry: Option<String>,
    connection: dim_client::ConnectionConfig,
) -> Result<TaskReport> {
    let _previous = if let Some(id) = &retry {
        let lease = owner.ledger.lock(id).await?;
        let mut previous = owner.ledger.report(id).await?.task;
        if previous.status == TaskStatus::Running {
            previous.status = TaskStatus::Interrupted;
            previous.error = Some(
                "Previous process ended before final settlement; retry reconciles owner receipts"
                    .into(),
            );
            owner.ledger.save(&previous).await?;
        }
        Some(lease)
    } else {
        None
    };
    let id = uuid::Uuid::new_v4().to_string();
    owner.ledger.create(&id).await?;
    let _lease = owner.ledger.lock(&id).await?;
    let mut task = TaskState {
        format: 1,
        id: id.clone(),
        retry_of: retry,
        source_home,
        status: TaskStatus::Running,
        stage: "scan".into(),
        processed: 0,
        total: None,
        current: None,
        error: None,
    };
    publish(&owner, &task).await?;
    let outcome = execute(&owner, &mut task, &connection).await;
    match outcome {
        Ok(()) => {
            let report = owner.ledger.report(&id).await?;
            task.processed = report.objects.len() as u64;
            task.total = Some(task.processed);
            task.status = if report.objects.iter().any(|item| {
                matches!(
                    item.outcome,
                    Outcome::Conflict
                        | Outcome::Deferred
                        | Outcome::Unsupported
                        | Outcome::Failed
                        | Outcome::Cancelled
                ) || !item.omitted.is_empty()
            }) {
                TaskStatus::Partial
            } else {
                TaskStatus::Completed
            };
            task.stage = "complete".into();
        }
        Err(Error::Cancelled) => {
            task.status = TaskStatus::Cancelled;
            task.error = Some(
                "Stopped after accepted work settled; completed objects remain imported".into(),
            );
        }
        Err(error) => {
            task.status = TaskStatus::Failed;
            task.error = Some(error.to_string());
        }
    }
    task.current = None;
    publish(&owner, &task).await?;
    owner.ledger.report(&id).await
}

async fn execute(
    owner: &Migration,
    task: &mut TaskState,
    connection: &dim_client::ConnectionConfig,
) -> Result<()> {
    let mut client = Client::connect(connection)
        .await
        .map_err(|_| Error::Target("Cannot connect to the target App Server".into()))?;
    if !client.supports(&[
        "session.import.begin",
        "session.import.chunk",
        "session.import.file",
        "session.import.finish",
    ]) {
        return Err(Error::Target(
            "The target App Server does not support recoverable conversation import".into(),
        ));
    }
    let source = Source::open(
        owner.ledger.files.clone(),
        task.source_home.clone(),
        0,
        &owner.stopping,
    )
    .await?;
    let result = execute_source(owner, task, &source, &mut client).await;
    let cleanup = source.close(&CancellationToken::new()).await;
    result.and(cleanup)
}

async fn execute_source(
    owner: &Migration,
    task: &mut TaskState,
    source: &Source,
    client: &mut Client,
) -> Result<()> {
    check(owner, &task.id).await?;
    task.stage = "providers".into();
    publish(owner, task).await?;
    if source
        .inventory
        .tables
        .get("providers")
        .is_some_and(|count| *count > 0)
        || source
            .inventory
            .tables
            .get("provider_selections")
            .is_some_and(|count| *count > 0)
    {
        if client.offers("provider.import_legacy") {
            let result: Value = request(
                client,
                "provider.import_legacy",
                json!({"sourcePath":source.snapshot}),
            )
            .await?;
            let rows = result["items"].as_array().ok_or_else(|| {
                Error::Target("Provider import returned an invalid result".into())
            })?;
            for row in rows {
                let kind = row["kind"]
                    .as_str()
                    .ok_or_else(|| Error::Target("Provider result lacks its kind".into()))?;
                let id = row["id"]
                    .as_str()
                    .ok_or_else(|| Error::Target("Provider result lacks its identity".into()))?;
                let outcome = match row["status"].as_str() {
                    Some("imported") => Outcome::Imported,
                    Some("already_imported") => Outcome::AlreadyImported,
                    Some("rejected") => Outcome::Unsupported,
                    _ => {
                        return Err(Error::Target(
                            "Provider import returned an unknown status".into(),
                        ));
                    }
                };
                let omitted = match row.get("omitted") {
                    Some(value) => serde_json::from_value(value.clone()).map_err(|_| {
                        Error::Target("Provider omitted-field result is invalid".into())
                    })?,
                    None => Vec::new(),
                };
                owner
                    .ledger
                    .object(
                        &task.id,
                        &ObjectResult {
                            kind: kind.into(),
                            id: id.into(),
                            outcome,
                            target_id: None,
                            reason: row["reason"].as_str().map(str::to_owned),
                            omitted,
                        },
                    )
                    .await?;
            }
        } else {
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: "providers".into(),
                        id: "providers".into(),
                        outcome: Outcome::Unsupported,
                        target_id: None,
                        reason: Some("The target profile does not expose Provider import".into()),
                        omitted: Vec::new(),
                    },
                )
                .await?;
        }
    }
    task.stage = "settings".into();
    publish(owner, task).await?;
    match crate::settings::import(owner, task, source, client).await {
        Err(Error::Object(reason)) => {
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: "setting".into(),
                        id: "legacy-settings".into(),
                        outcome: Outcome::Failed,
                        target_id: None,
                        reason: Some(reason),
                        omitted: Vec::new(),
                    },
                )
                .await?
        }
        result => result?,
    }
    task.stage = "accounts".into();
    publish(owner, task).await?;
    match crate::accounts::import(owner, task, source, client).await {
        Err(Error::Object(reason)) => {
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: "account".into(),
                        id: "legacy-accounts".into(),
                        outcome: Outcome::Failed,
                        target_id: None,
                        reason: Some(reason),
                        omitted: Vec::new(),
                    },
                )
                .await?
        }
        result => result?,
    }
    task.stage = "sessions".into();
    let ids = source.session_ids()?;
    task.total = Some(ids.len() as u64);
    task.processed = 0;
    for id in ids {
        check(owner, &task.id).await?;
        task.current = Some(id.clone());
        publish(owner, task).await?;
        let result = match source.session(&id) {
            Ok(data) => session(owner, task, source, client, data).await,
            Err(error) => Err(error),
        };
        let result = match result {
            Ok(result) => result,
            Err(Error::Object(reason)) => ObjectResult {
                kind: "session".into(),
                id: id.clone(),
                outcome: Outcome::Failed,
                target_id: None,
                reason: Some(reason),
                omitted: Vec::new(),
            },
            Err(Error::Deferred(reason)) => ObjectResult {
                kind: "session".into(),
                id: id.clone(),
                outcome: Outcome::Deferred,
                target_id: None,
                reason: Some(reason),
                omitted: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        owner.ledger.object(&task.id, &result).await?;
        task.processed += 1;
        publish(owner, task).await?;
    }
    task.stage = "scheduler".into();
    task.current = None;
    publish(owner, task).await?;
    crate::scheduler::import(owner, task, source, client).await?;
    let archived = owner.ledger.report(&task.id).await?.objects;
    for (table, count) in &source.inventory.tables {
        let prefix = format!("{table}:");
        let preserved = archived
            .iter()
            .filter(|result| {
                result.kind == "historical_record"
                    && result.id.starts_with(&prefix)
                    && matches!(result.outcome, Outcome::Imported | Outcome::AlreadyImported)
            })
            .count() as u64;
        let remaining = count.saturating_sub(preserved);
        if remaining == 0
            || [
                "sessions",
                "session_states",
                "messages",
                "compaction_states",
                "session_relations",
                "session_blobs",
                "blob_data",
                "providers",
                "provider_selections",
                "schema_migrations",
                "scheduled_jobs",
                "terminal.scheduled_jobs",
            ]
            .contains(&table.as_str())
        {
            continue;
        }
        let deferred = table.starts_with("file_")
            || [
                "queue_items",
                "background_tasks",
                "usage_ledger",
                "usage_daily_stats",
                "usage_run_stats",
                "usage_skill_events",
                "permission_decisions",
                "workflow_runs",
                "scheduled_job_executions",
                "terminal.scheduled_job_executions",
            ]
            .contains(&table.as_str());
        owner.ledger.object(&task.id,&ObjectResult {kind:"table".into(),id:table.clone(),outcome:if deferred {Outcome::Deferred}else{Outcome::Unsupported},target_id:None,reason:Some(format!("{remaining} legacy records retained without an imported supported Session or equivalent target owner")),omitted:Vec::new()}).await?;
    }
    match crate::inventory::inspect(owner, task, source).await {
        Err(Error::Object(reason)) => {
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: "inventory".into(),
                        id: "retained-source".into(),
                        outcome: Outcome::Failed,
                        target_id: None,
                        reason: Some(reason),
                        omitted: Vec::new(),
                    },
                )
                .await?
        }
        Err(error) => return Err(error),
        Ok(()) => {}
    }
    Ok(())
}

async fn session(
    owner: &Migration,
    task: &TaskState,
    source: &Source,
    client: &mut Client,
    mut data: SessionData,
) -> Result<ObjectResult> {
    let mut archive = crate::archives::Archive::read(source, &data, &owner.stopping).await?;
    let mut digests = BTreeMap::new();
    let mut raw_files = std::mem::take(&mut archive.files);
    for blob in &data.blobs {
        let bytes = source.blob(blob, &owner.stopping).await?;
        if raw_files.insert(blob.id.clone(), bytes).is_some() {
            return Err(Error::Object(
                "Legacy attachment identity collides with historical artifacts".into(),
            ));
        }
    }
    for (id, bytes) in &raw_files {
        digests.insert(id.clone(), format!("{:x}", Sha256::digest(bytes)));
    }
    let digest = fingerprint(&(&data.source_digest, &archive, &digests))?;
    let historical = archive
        .records
        .iter()
        .map(|record| (record.table.clone(), record.id.clone()))
        .collect::<Vec<_>>();
    let key = format!("legacy-session:{}", data.id);
    let mut omitted = archive.omitted.clone();
    if data
        .mode
        .as_deref()
        .is_some_and(|mode| mode != "agent" && mode != "plan")
    {
        omitted.push("legacy Session mode has no equivalent continuation mapping".into());
    }
    if data.tags.as_array().is_some_and(|tags| !tags.is_empty()) {
        omitted.push("tags".into());
    }
    if data
        .permission
        .as_object()
        .is_some_and(|value| !value.is_empty())
    {
        omitted.push("session permission settings without an equivalent preset".into());
    }
    if let Some(settings) = data.settings.as_object() {
        omitted.extend(
            settings
                .keys()
                .filter(|key| key.as_str() != "__revertState__")
                .map(|key| format!("session.settings.{key}")),
        );
    }
    let mut paths = BTreeMap::new();
    let marker = format!(
        "Imported history from legacy Session {}. Earlier messages and tool results are imported records, not newly executed actions.",
        data.id,
    );
    let note_index = data.records.len();
    let mut note_id = "legacy-import-metadata".to_owned();
    while data.records.iter().any(|record| record.id == note_id) {
        note_id.push('-');
    }
    data.records.push(RecordedMessage {
        id: note_id,
        created_at: None,
        message: Message::text(
            Role::User,
            "Migration import record: historical metadata and artifacts.",
        ),
        in_context: false,
    });
    let preliminary = chunks(&mut data.records, &marker, &mut data.context_error)?;
    if data.context_error.is_some() {
        omitted.push("effective context: continuation blocked".into());
    }
    let begin = ConversationBegin {
        key: key.clone(),
        fingerprint: digest,
        cwd: data.cwd,
        model: data.model,
        policy: None,
        title: data.title,
        chunks: preliminary.len() as u64,
        files: raw_files.len() as u64 + 1,
    };
    let mut receipt: ConversationReceipt = request(client, "session.import.begin", &begin).await?;
    if receipt.status != ConversationStatus::Preparing {
        if receipt.status == ConversationStatus::Committed {
            record_history(
                owner,
                task,
                &historical,
                &receipt.session_id,
                Outcome::AlreadyImported,
            )
            .await?;
        }
        return Ok(ObjectResult {
            kind: "session".into(),
            id: data.id,
            outcome: match receipt.status {
                ConversationStatus::Deleted => Outcome::Deleted,
                ConversationStatus::Conflict => Outcome::Conflict,
                _ => Outcome::AlreadyImported,
            },
            target_id: Some(receipt.session_id),
            reason: if receipt.status == ConversationStatus::Conflict {
                Some(
                    "Legacy content changed for an existing import identity; target retained"
                        .into(),
                )
            } else {
                data.context_error
            },
            omitted,
        });
    }
    for (id, bytes) in &raw_files {
        let stored = transfer_file(
            owner,
            task,
            client,
            &key,
            &receipt.files,
            id,
            &fingerprint(id)?,
            bytes,
        )
        .await?;
        paths.insert(id.clone(), stored.path);
    }
    let metadata = archive.encode(&paths)?;
    let metadata_id = "history:metadata";
    if raw_files.contains_key(metadata_id) {
        return Err(Error::Object(
            "Legacy attachment identity collides with historical metadata".into(),
        ));
    }
    let stored = transfer_file(
        owner,
        task,
        client,
        &key,
        &receipt.files,
        metadata_id,
        "legacy-history.json",
        &metadata,
    )
    .await?;
    data.records[note_index].message = Message::text(
        Role::User,
        format!(
            "Migration import record: historical metadata, queued inputs, results and available artifacts are readable at {}. These records are excluded from continuation context and do not resume old execution.",
            stored.path
        ),
    );
    for record in &mut data.records {
        let MessageContent::Parts(parts) = &mut record.message.content else {
            continue;
        };
        if let Some(attachments) = data.attachments.get(&record.id) {
            for attachment in attachments {
                if attachment.kind != "blob" {
                    omitted.push(format!("attachment.kind.{}", attachment.kind));
                    continue;
                }
                let id = attachment.data["blobId"].as_str().ok_or_else(|| {
                    Error::Object("Legacy attachment has no blob identity".into())
                })?;
                let path = paths.get(id).ok_or_else(|| {
                    Error::Object("Legacy attachment references a missing blob".into())
                })?;
                if attachment.data["role"].as_str() != Some("tool_output") {
                    let blob = data
                        .blobs
                        .iter()
                        .find(|blob| blob.id == id)
                        .ok_or_else(|| {
                            Error::Object("Legacy attachment references a missing blob".into())
                        })?;
                    let media = attachment.data["mediaType"]
                        .as_str()
                        .or(blob.mime_type.as_deref());
                    if let Some(media) = media.filter(|media| media.starts_with("image/")) {
                        parts.push(MessagePart::Image {
                            source: ImageSource {
                                source_type: ImageSourceType::Base64,
                                media_type: media.into(),
                                data: Some(
                                    base64::engine::general_purpose::STANDARD
                                        .encode(&raw_files[id]),
                                ),
                                url: None,
                            },
                        });
                    } else {
                        parts.push(MessagePart::Text {
                            provider_metadata: None,
                            text: format!("Attached imported blob {id}: {path}"),
                            start_time: None,
                            end_time: None,
                        });
                    }
                }
            }
        }
        for part in parts {
            match part {
                MessagePart::Text { text, .. } | MessagePart::Thinking { thinking: text, .. } => {
                    remap(text, &data.blobs, &paths, source)
                }
                MessagePart::ToolResult { content, .. } => match content {
                    ToolContent::Text(text) => remap(text, &data.blobs, &paths, source),
                    ToolContent::Blocks(blocks) => {
                        for block in blocks {
                            if let ContentBlock::Text { text } = block {
                                remap(text, &data.blobs, &paths, source);
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }
    let input = chunks(&mut data.records, &marker, &mut data.context_error)?;
    if input.len() as u64 != begin.chunks {
        return Err(Error::Object(
            "Attachment expansion changed the declared message chunk count".into(),
        ));
    }
    for (index, input) in input.into_iter().enumerate() {
        if (index as u64) < receipt.next_chunk {
            continue;
        }
        check(owner, &task.id).await?;
        receipt = request(
            client,
            "session.import.chunk",
            ConversationChunk {
                key: key.clone(),
                index: index as u64,
                input,
            },
        )
        .await?;
        let mut progress = task.clone();
        progress.stage = "message-chunks".into();
        progress.total = Some(begin.chunks);
        progress.processed = receipt.next_chunk;
        publish(owner, &progress).await?;
    }
    check(owner, &task.id).await?;
    receipt = request(client, "session.import.finish", ConversationKey { key }).await?;
    if receipt.status != ConversationStatus::Committed {
        return Err(Error::Object(
            "Target conversation was deleted before publication".into(),
        ));
    }
    record_history(
        owner,
        task,
        &historical,
        &receipt.session_id,
        Outcome::Imported,
    )
    .await?;
    Ok(ObjectResult {
        kind: "session".into(),
        id: data.id,
        outcome: Outcome::Imported,
        target_id: Some(receipt.session_id),
        reason: data.context_error,
        omitted,
    })
}

async fn record_history(
    owner: &Migration,
    task: &TaskState,
    records: &[(String, String)],
    session: &str,
    outcome: Outcome,
) -> Result<()> {
    for (table, id) in records {
        owner.ledger.object(&task.id,&ObjectResult {kind:"historical_record".into(),id:format!("{table}:{id}"),outcome,target_id:Some(session.into()),reason:Some("Preserved as readable Session-owned historical data; no execution or authority restored".into()),omitted:Vec::new()}).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn transfer_file(
    owner: &Migration,
    task: &TaskState,
    client: &mut Client,
    key: &str,
    receipts: &[ConversationFileReceipt],
    id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<ConversationFileReceipt> {
    if let Some(stored) = receipts.iter().find(|receipt| receipt.id == id) {
        return Ok(stored.clone());
    }
    let mut stored = None;
    let digest = format!("{:x}", Sha256::digest(bytes));
    for index in 0..bytes.len().div_ceil(256 * 1024).max(1) {
        check(owner, &task.id).await?;
        let offset = index * 256 * 1024;
        let end = (offset + 256 * 1024).min(bytes.len());
        stored = request(
            client,
            "session.import.file",
            ConversationFileChunk {
                key: key.into(),
                id: id.into(),
                name: name.into(),
                sha256: digest.clone(),
                bytes: bytes.len() as u64,
                offset: offset as u64,
                content: bytes[offset..end].to_vec(),
            },
        )
        .await?;
        let mut progress = task.clone();
        progress.stage = "file-bytes".into();
        progress.current = Some(format!("{key}/{id}"));
        progress.processed = end as u64;
        progress.total = Some(bytes.len() as u64);
        publish(owner, &progress).await?;
    }
    stored.ok_or_else(|| Error::Target("Target did not confirm the complete imported file".into()))
}

fn remap(
    text: &mut String,
    blobs: &[crate::source::Blob],
    paths: &BTreeMap<String, String>,
    source: &Source,
) {
    for blob in blobs {
        if let Some(target) = paths.get(&blob.id) {
            let path = if std::path::Path::new(&blob.reference).is_absolute() {
                PathBuf::from(&blob.reference)
            } else {
                source.home.join("data").join(&blob.reference)
            };
            if let Some(path) = path.to_str() {
                *text = text.replace(path, target);
            }
        }
    }
}

fn chunks(
    records: &mut [RecordedMessage],
    source: &str,
    context_error: &mut Option<String>,
) -> Result<Vec<InputMessageData>> {
    let all = InputMessageData {
        blocks: vec![InputBlock::Messages {
            source: source.into(),
            messages: records.to_vec(),
            context_error: context_error.clone(),
        }],
        mode: Some(InputMode::Append),
        ..Default::default()
    };
    if all.validate().is_err() && context_error.is_none() {
        *context_error =
            Some("Legacy effective context has incomplete or invalid role/tool pairing".into());
        for record in records.iter_mut() {
            record.in_context = false;
        }
    }
    let mut result = Vec::new();
    let mut group = Vec::new();
    let mut open = HashSet::new();
    for record in records.iter() {
        if record.in_context
            && let MessageContent::Parts(parts) = &record.message.content
        {
            for part in parts {
                match part {
                    MessagePart::ToolUse { id, .. } => {
                        open.insert(id.clone());
                    }
                    MessagePart::ToolResult { tool_use_id, .. } => {
                        open.remove(tool_use_id);
                    }
                    _ => {}
                }
            }
        }
        group.push(record.clone());
        if open.is_empty() {
            let input = InputMessageData {
                blocks: vec![InputBlock::Messages {
                    source: source.into(),
                    messages: std::mem::take(&mut group),
                    context_error: context_error.clone(),
                }],
                mode: Some(InputMode::Append),
                ..Default::default()
            };
            input.validate().map_err(|_| {
                Error::Object("Legacy history cannot be represented as recorded messages".into())
            })?;
            if serde_json::to_vec(&input)
                .map_err(|_| Error::Object("Recorded message cannot be encoded".into()))?
                .len()
                > 1024 * 1024
            {
                return Err(Error::Object(
                    "One complete legacy message/tool group exceeds the 1 MiB input limit".into(),
                ));
            }
            result.push(input);
        }
    }
    if !group.is_empty() {
        return Err(Error::Object("Legacy tool group is incomplete".into()));
    }
    Ok(result)
}
