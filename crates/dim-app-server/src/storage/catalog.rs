use super::{
    SessionStore, metadata,
    sqlite::error,
    timeline::{Row, read_sequence},
};
use crate::protocol::{Catalog, CatalogResult, SessionInfo, SessionSummary};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dim_agent_runtime::{AgentError, Result};
use dim_agent_types::{ContentBlock, InputMessageData, ModelId};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub(super) fn create(connection: &Connection, session: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO session_catalog SELECT session_id, created_at_ms, NULL, json_extract(config, '$.cwd') FROM agent_sessions WHERE session_id=?1",
        [session],
    ).map_err(error)?;
    Ok(())
}

/// Initializes the catalog after importing a Session's native rows.
/// Call inside the transaction that inserts those rows, before committing.
pub fn initialize_session_catalog(connection: &Connection, session: &str) -> Result<()> {
    create(connection, session)?;
    refresh(connection, session)
}

pub(crate) fn truncate_preview(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

pub(super) fn refresh(connection: &Connection, session: &str) -> Result<()> {
    let last: Option<String> = connection.query_row(
        "SELECT body FROM timeline_events WHERE session_id=?1 AND branch_id='main' ORDER BY sequence DESC LIMIT 1",
        [session], |row| row.get(0),
    ).optional().map_err(error)?;
    if let Some(body) = last {
        let row: Row = serde_json::from_str(&body).map_err(error)?;
        let time = row
            .wires()
            .last()
            .and_then(|wire| wire.event["create_at"].as_u64())
            .ok_or_else(|| error("catalog activity event has no timestamp"))?;
        connection
            .execute(
                "UPDATE session_catalog SET updated_at_ms=?2 WHERE session_id=?1",
                params![session, i64::try_from(time).map_err(error)?],
            )
            .map_err(error)?;
    }
    let preview: Option<String> = connection
        .query_row(
            "SELECT preview FROM session_catalog WHERE session_id=?1",
            [session],
            |row| row.get(0),
        )
        .map_err(error)?;
    if preview.is_some() {
        return Ok(());
    }
    let boundary: Option<String> = connection.query_row(
        "SELECT body FROM timeline_events WHERE session_id=?1 AND branch_id='main' AND sequence<=256 ORDER BY sequence DESC LIMIT 1",
        [session], |row| row.get(0),
    ).optional().map_err(error)?;
    let Some(body) = boundary else { return Ok(()) };
    let row: Row = serde_json::from_str(&body).map_err(error)?;
    let end = row
        .wires()
        .iter()
        .rev()
        .find(|wire| wire.event["sequence"].as_u64().is_some_and(|s| s <= 256))
        .ok_or_else(|| error("catalog preview boundary is missing"))?
        .batch_end;
    let mut query = connection.prepare_cached(
        "SELECT body FROM timeline_events WHERE session_id=?1 AND branch_id='main' AND sequence<=?2 AND kind='input/message' ORDER BY sequence",
    ).map_err(error)?;
    let rows = query
        .query_map(
            params![session, i64::try_from(end).map_err(error)?],
            |row| row.get::<_, String>(0),
        )
        .map_err(error)?;
    for body in rows {
        let row: Row = serde_json::from_str(&body.map_err(error)?).map_err(error)?;
        let Row::Event { record } = row else {
            return Err(error("catalog input is not an event row"));
        };
        let input: InputMessageData =
            serde_json::from_value(record.event["data"].clone()).map_err(error)?;
        if let Some(user) = input.user {
            let text = user
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let preview = (!text.is_empty()).then(|| truncate_preview(&text, 120));
            connection
                .execute(
                    "UPDATE session_catalog SET preview=?2 WHERE session_id=?1",
                    params![session, preview],
                )
                .map_err(error)?;
            break;
        }
    }
    Ok(())
}

pub(super) fn backfill(connection: &Connection) -> Result<()> {
    let sessions = connection
        .prepare("SELECT session_id FROM agent_sessions")
        .map_err(error)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    for session in sessions {
        create(connection, &session)?;
        refresh(connection, &session)?;
    }
    Ok(())
}

impl SessionStore {
    pub(crate) async fn catalog(
        &self,
        request: Catalog,
        agent: Option<String>,
        version: u32,
        hosts: Vec<String>,
        closed: Vec<String>,
    ) -> Result<CatalogResult> {
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&request.query, &request.cwd, request.with_preview))
                    .map_err(error)?
            )
        );
        let cursor = request
            .cursor
            .as_ref()
            .map(|cursor| {
                let invalid = || AgentError::InvalidInput("invalid catalog cursor".into());
                let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
                let (time, id, filters): (u64, String, String) =
                    serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if filters != digest || time > i64::MAX as u64 {
                    return Err(invalid());
                }
                Ok((time, id))
            })
            .transpose()?;
        self.timeline.db.run(move |connection| {
            let index = if request.with_preview { "session_catalog_with_preview" } else { "session_catalog_by_activity" };
            let mut sql = format!(
                "SELECT c.session_id AS session_id, c.cwd, s.created_at_ms, b.sequence, o.agent_id, \
                 m.revision, m.title, m.title_source, c.preview, c.updated_at_ms AS updated_at_ms, \
                 json_extract(s.config, '$.model.providerId'), json_extract(s.config, '$.model.modelId') \
                 FROM session_catalog c INDEXED BY {index} CROSS JOIN agent_sessions s ON s.session_id=c.session_id \
                 CROSS JOIN session_owners o ON o.session_id=c.session_id \
                 CROSS JOIN timeline_branches b ON b.session_id=c.session_id AND b.branch_id='main' \
                 LEFT JOIN session_metadata m ON m.session_id=c.session_id \
                 WHERE o.protocol_version=?1 AND o.agent_id IN (SELECT value FROM json_each(?2)) \
                 AND c.session_id NOT IN (SELECT value FROM json_each(?3))"
            );
            let mut values: Vec<rusqlite::types::Value> = vec![i64::from(version).into(), serde_json::to_string(&hosts).map_err(error)?.into(), serde_json::to_string(&closed).map_err(error)?.into()];
            if request.with_preview { sql.push_str(" AND c.preview IS NOT NULL"); }
            if let Some(agent) = agent { values.push(agent.into()); sql.push_str(&format!(" AND o.agent_id=?{}", values.len())); }
            if let Some(cwd) = request.cwd { values.push(cwd.into()); sql.push_str(&format!(" AND c.cwd=?{}", values.len())); }
            if let Some(query) = request.query {
                values.push(query.to_lowercase().into()); let p = values.len();
                sql.push_str(&format!(" AND (instr(catalog_fold(c.session_id),?{p})>0 OR instr(catalog_fold(c.cwd),?{p})>0 OR instr(catalog_fold(c.preview),?{p})>0 OR instr(catalog_fold(m.title),?{p})>0 OR instr(catalog_fold(json_extract(s.config,'$.model.providerId')),?{p})>0 OR instr(catalog_fold(json_extract(s.config,'$.model.modelId')),?{p})>0)"));
            }
            if let Some((time, id)) = cursor {
                values.push((time as i64).into());
                let time = values.len();
                values.push(id.into());
                let id = values.len();
                sql = format!("{sql} AND c.updated_at_ms=?{time} AND c.session_id>?{id} UNION ALL {sql} AND c.updated_at_ms<?{time}");
            }
            sql.push_str(" ORDER BY updated_at_ms DESC, session_id");
            if let Some(limit) = request.limit { values.push(((limit+1) as i64).into()); sql.push_str(&format!(" LIMIT ?{}", values.len())); }
            let mut statement = connection.prepare_cached(&sql).map_err(error)?;
            let mut rows = statement.query(rusqlite::params_from_iter(values)).map_err(error)?;
            let mut sessions = Vec::new();
            while let Some(row) = rows.next().map_err(error)? {
                sessions.push(SessionSummary {
                    session: SessionInfo {
                        session_id: row.get(0).map_err(error)?, cwd: row.get(1).map_err(error)?,
                        created_at: read_sequence(row, 2).map_err(error)?, sequence: read_sequence(row, 3).map_err(error)?,
                        agent_id: Some(row.get(4).map_err(error)?), title: Some(metadata::decode(row, 5).map_err(error)?.into()),
                        model: ModelId { provider_id: row.get(10).map_err(error)?, model_id: row.get(11).map_err(error)? },
                        running: false, attached_clients: 0,
                    }, preview: row.get(8).map_err(error)?, updated_at: read_sequence(row, 9).map_err(error)?,
                });
            }
            let next_cursor = if request.limit.is_some_and(|limit| sessions.len()>limit) {
                sessions.pop();
                let last = sessions.last().expect("a catalog page has a positive limit");
                Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&(last.updated_at, &last.session.session_id, digest)).map_err(error)?))
            } else { None };
            Ok(CatalogResult { sessions, next_cursor })
        }).await
    }
}
