use super::*;
use dim_agent_capabilities::host::PutSessionFile;
use dim_agent_types::{
    ConversationBegin, ConversationChunk, ConversationFileChunk, ConversationFileReceipt,
    ConversationReceipt, ConversationStatus, InputAppliedData, InputConsumedData, InputMode,
};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

fn invalid(message: &str) -> AgentError {
    AgentError::InvalidInput(message.into())
}

struct Import {
    session: String,
    agent: String,
    request: ConversationBegin,
    next: u64,
    committed: bool,
}

fn read(connection: &Connection, key: &str) -> Result<Import> {
    let row: Option<(String, String, String, u64, bool)> = connection.query_row(
        "SELECT session_id, agent_id, request, next_chunk, committed FROM conversation_imports WHERE import_key=?1",
        [key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, read_sequence(row, 3)?, row.get(4)?)),
    ).optional().map_err(error)?;
    let (session, agent, request, next, committed) =
        row.ok_or_else(|| invalid("Unknown conversation import"))?;
    Ok(Import {
        session,
        agent,
        request: serde_json::from_str(&request).map_err(error)?,
        next,
        committed,
    })
}

fn exists(connection: &Connection, session: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE session_id=?1)",
            [session],
            |row| row.get(0),
        )
        .map_err(error)
}

fn receipt(connection: &Connection, import: &Import) -> Result<ConversationReceipt> {
    let mut query = connection.prepare("SELECT receipt FROM conversation_import_files WHERE import_key=?1 AND receipt IS NOT NULL ORDER BY source_id").map_err(error)?;
    let files = query
        .query_map([&import.request.key], |row| row.get::<_, String>(0))
        .map_err(error)?
        .map(|row| serde_json::from_str(&row.map_err(error)?).map_err(error))
        .collect::<Result<Vec<_>>>()?;
    Ok(ConversationReceipt {
        agent_id: import.agent.clone(),
        session_id: import.session.clone(),
        status: if !exists(connection, &import.session)? {
            ConversationStatus::Deleted
        } else if import.committed {
            ConversationStatus::Committed
        } else {
            ConversationStatus::Preparing
        },
        next_chunk: import.next,
        files,
    })
}

fn preparing(connection: &Connection, import: &Import) -> Result<()> {
    if import.committed {
        return Err(invalid("Conversation import is already committed"));
    }
    if !exists(connection, &import.session)? {
        return Err(invalid("The imported conversation was deleted"));
    }
    Ok(())
}

impl SessionStore {
    fn import_gate(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.import_gates.lock().unwrap();
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key.to_owned(), Arc::downgrade(&gate));
        gate
    }

    /// Reserves an unpublished Session, or recovers its durable import receipt.
    pub async fn begin_conversation(
        &self,
        request: ConversationBegin,
        config: SessionConfig,
        agent: String,
    ) -> Result<ConversationReceipt> {
        let _gate = self.import_gate(&request.key).lock_owned().await;
        if request.key.is_empty()
            || request.key.len() > 512
            || request.fingerprint.len() != 64
            || !request.fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
            || request.chunks > dim_agent_types::JS_MAX_SAFE_INTEGER / 3
        {
            return Err(invalid("Invalid conversation import identity or size"));
        }
        self.timeline
            .db
            .run(move |connection| {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                let present: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM conversation_imports WHERE import_key=?1)",
                        [&request.key],
                        |row| row.get(0),
                    )
                    .map_err(error)?;
                if present {
                    let import = read(&tx, &request.key)?;
                    if import.request != request || import.agent != agent {
                        let mut result = receipt(&tx, &import)?;
                        if result.status != ConversationStatus::Deleted {
                            result.status = ConversationStatus::Conflict;
                        }
                        return Ok(result);
                    }
                    return receipt(&tx, &import);
                }
                let session = uuid::Uuid::new_v4().to_string();
                create_session(&tx, &session, &config)?;
                tx.execute(
                    "INSERT INTO conversation_imports VALUES (?1, ?2, ?3, ?4, 0, 0)",
                    params![
                        request.key,
                        session,
                        agent,
                        serde_json::to_string(&request).map_err(error)?
                    ],
                )
                .map_err(error)?;
                let result = receipt(&tx, &read(&tx, &request.key)?)?;
                tx.commit().map_err(error)?;
                Ok(result)
            })
            .await
    }

    /// Atomically adopts one bounded input and advances its recoverable cursor.
    pub async fn append_conversation(
        &self,
        chunk: ConversationChunk,
    ) -> Result<ConversationReceipt> {
        let _gate = self.import_gate(&chunk.key).lock_owned().await;
        chunk.input.validate().map_err(invalid)?;
        if chunk.input.effective_mode() != InputMode::Append
            || chunk.input.user.is_some()
            || chunk.input.submission_id.is_some()
        {
            return Err(invalid(
                "Conversation chunks must be append-only recorded inputs",
            ));
        }
        if serde_json::to_vec(&chunk.input).map_err(error)?.len() > 1024 * 1024 {
            return Err(invalid("Conversation input chunk exceeds 1 MiB"));
        }
        self.timeline.db.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
            let import = read(&tx, &chunk.key)?;
            let event_id = format!("conversation:{}:{}", import.session, chunk.index);
            if chunk.index < import.next {
                let value: String = tx.query_row("SELECT json_extract(body, '$.record.event.data') FROM timeline_events WHERE session_id=?1 AND event_id=?2", params![import.session, event_id], |row| row.get(0)).map_err(error)?;
                let previous: dim_agent_types::InputMessageData = serde_json::from_str(&value).map_err(error)?;
                if previous != chunk.input { return Err(invalid("Conversation chunk conflicts with the committed input")); }
                return receipt(&tx, &import);
            }
            preparing(&tx, &import)?;
            if chunk.index != import.next || chunk.index >= import.request.chunks {
                return Err(invalid("Conversation chunk is outside the expected cursor"));
            }
            let sequence = head(&tx, &TimelineKey { session_id: import.session.clone(), branch_id: "main".into() })?;
            let batch_end = sequence + 3;
            let kinds = [
                EventKind::InputMessage(chunk.input),
                EventKind::InputApplied(InputAppliedData { input_id: event_id.clone(), mode: Some(InputMode::Append) }),
                EventKind::InputConsumed(InputConsumedData { input_id: event_id.clone() }),
            ];
            let events: Vec<_> = kinds.into_iter().enumerate().map(|(index, kind)| StoredEvent {
                event: Arc::new(AgentEvent { id: if index == 0 { event_id.clone() } else { uuid::Uuid::new_v4().to_string() }, sequence: sequence + index as u64 + 1,
                    create_at: dim_agent_runtime::core::unix_millis(), tags: None, kind }),
                trace: None, batch_end,
            }).collect();
            let key = TimelineKey { session_id: import.session.clone(), branch_id: "main".into() };
            write_batch(&tx, &key, &events, &[vec![], vec![], vec![]])?;
            tx.execute("UPDATE conversation_imports SET next_chunk=next_chunk+1 WHERE import_key=?1", [&chunk.key]).map_err(error)?;
            let result = receipt(&tx, &read(&tx, &chunk.key)?)?;
            tx.commit().map_err(error)?;
            Ok(result)
        }).await
    }

    /// Stages bounded file bytes and returns a receipt only after the file put settles.
    pub async fn conversation_file(
        &self,
        chunk: ConversationFileChunk,
        cancel: CancellationToken,
    ) -> Result<Option<ConversationFileReceipt>> {
        if chunk.id.is_empty()
            || chunk.id.len() > 512
            || !files::valid_name(&chunk.name)
            || chunk.bytes > files::MAX_BYTES as u64
            || chunk.content.len() > 256 * 1024
            || chunk.offset > chunk.bytes
            || chunk.content.len() as u64 > chunk.bytes - chunk.offset
            || chunk.sha256.len() != 64
            || !chunk.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(invalid("Invalid conversation file chunk"));
        }
        let _gate = self.import_gate(&chunk.key).lock_owned().await;
        let part = chunk.clone();
        let staged = self.timeline.db.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
            let import = read(&tx, &part.key)?;
            preparing(&tx, &import)?;
            tx.execute("INSERT OR IGNORE INTO conversation_import_files VALUES (?1, ?2, ?3, ?4, ?5, X'', NULL)", params![part.key, part.id, part.name, part.sha256, sql_sequence(part.bytes)?]).map_err(error)?;
            let (name, digest, size, mut content, saved): (String, String, u64, Vec<u8>, Option<String>) = tx.query_row("SELECT name, sha256, bytes, content, receipt FROM conversation_import_files WHERE import_key=?1 AND source_id=?2", params![part.key, part.id], |row| Ok((row.get(0)?, row.get(1)?, read_sequence(row, 2)?, row.get(3)?, row.get(4)?))).map_err(error)?;
            if name != part.name || digest != part.sha256 || size != part.bytes {
                return Err(invalid("Conversation file conflicts with its prior request"));
            }
            if let Some(saved) = saved {
                return Ok((import.session, Some(serde_json::from_str(&saved).map_err(error)?), None));
            }
            let count: u64 = tx.query_row("SELECT count(*) FROM conversation_import_files WHERE import_key=?1", [&part.key], |row| read_sequence(row, 0)).map_err(error)?;
            if count > import.request.files { return Err(invalid("Conversation has more files than declared")); }
            let offset = part.offset as usize;
            if offset == content.len() {
                content.extend_from_slice(&part.content);
            } else if content.get(offset..offset + part.content.len()) != Some(part.content.as_slice()) {
                return Err(invalid("Conversation file chunk conflicts with its committed bytes"));
            }
            if content.len() as u64 == size && format!("{:x}", Sha256::digest(&content)) != digest {
                return Err(invalid("Conversation file content digest does not match"));
            }
            tx.execute("UPDATE conversation_import_files SET content=?3 WHERE import_key=?1 AND source_id=?2", params![part.key, part.id, content]).map_err(error)?;
            tx.commit().map_err(error)?;
            let complete = (content.len() as u64 == size).then_some(content);
            Ok((import.session, None, complete))
        }).await?;
        let (session, saved, content) = staged;
        if saved.is_some() {
            return Ok(saved);
        }
        let Some(content) = content else {
            return Ok(None);
        };
        let stored = self
            .files
            .put(session, chunk.name, content, cancel)
            .await
            .map_err(|failure| error(failure.to_string()))?;
        let result = ConversationFileReceipt {
            id: chunk.id.clone(),
            file_id: stored.file_id,
            path: stored
                .path
                .into_os_string()
                .into_string()
                .map_err(|_| invalid("Session file path is not UTF-8"))?,
            bytes: stored.bytes,
        };
        let saved = serde_json::to_string(&result).map_err(error)?;
        self.timeline.db.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
            let import = read(&tx, &chunk.key)?;
            preparing(&tx, &import)?;
            tx.execute("UPDATE conversation_import_files SET receipt=COALESCE(receipt, ?3) WHERE import_key=?1 AND source_id=?2", params![chunk.key, chunk.id, saved]).map_err(error)?;
            let saved: String = tx.query_row("SELECT receipt FROM conversation_import_files WHERE import_key=?1 AND source_id=?2", params![chunk.key, chunk.id], |row| row.get(0)).map_err(error)?;
            tx.commit().map_err(error)?;
            serde_json::from_str(&saved).map(Some).map_err(error)
        }).await
    }

    /// Publishes a complete Session and retains the receipt after user deletion.
    pub async fn finish_conversation(&self, key: String) -> Result<ConversationReceipt> {
        let _gate = self.import_gate(&key).lock_owned().await;
        let result = self.timeline.db.run(move |connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(error)?;
            let import = read(&tx, &key)?;
            let result = receipt(&tx, &import)?;
            if result.status != ConversationStatus::Preparing { return Ok(result); }
            if import.next != import.request.chunks || result.files.len() as u64 != import.request.files {
                return Err(invalid("Conversation history or files are incomplete"));
            }
            if let Some(title) = &import.request.title {
                tx.execute("INSERT INTO session_metadata VALUES (?1, 1, ?2, 'manual')", params![import.session, title]).map_err(error)?;
            }
            tx.execute("DELETE FROM session_file_owners WHERE session_id=?1 AND file_id NOT IN (SELECT json_extract(receipt, '$.fileId') FROM conversation_import_files WHERE import_key=?2 AND receipt IS NOT NULL)", params![import.session, key]).map_err(error)?;
            tx.execute("INSERT INTO session_owners VALUES (?1, ?2, 3)", params![import.session, import.agent]).map_err(error)?;
            tx.execute("UPDATE conversation_imports SET committed=1 WHERE import_key=?1", [&key]).map_err(error)?;
            tx.execute("UPDATE conversation_import_files SET content=X'' WHERE import_key=?1", [&key]).map_err(error)?;
            let result = receipt(&tx, &read(&tx, &key)?)?;
            tx.commit().map_err(error)?;
            Ok(result)
        }).await?;
        self.timeline.sweep_files.notify_one();
        Ok(result)
    }
}
