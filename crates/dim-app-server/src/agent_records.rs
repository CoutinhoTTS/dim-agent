use crate::storage::SqliteStorage;
use dim_agent_runtime::{AgentError, Result};
use dim_agent_types::AgentProfile;
use rusqlite::{OptionalExtension, types::Value};
use std::sync::Arc;

pub(crate) struct AgentRecord {
    pub id: String,
    pub profile: std::result::Result<AgentProfile, String>,
    pub default: bool,
}

pub(crate) struct SessionOwner {
    pub session_id: String,
    pub owner: Option<(String, u32)>,
    pub error: Option<String>,
}

pub(crate) struct AgentRecords(pub Arc<SqliteStorage>);

impl AgentRecords {
    pub async fn agents(&self) -> Result<Vec<AgentRecord>> {
        self.0.run(|connection| {
            let mut query = connection.prepare(
                "SELECT agent_id, profile, is_default FROM agents ORDER BY is_default DESC, agent_id",
            ).map_err(storage_error)?;
            let records = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Value>(1)?, row.get::<_, i64>(2)?)))
                .map_err(storage_error)?;
            let records: Vec<_> = records.map(|row| {
                let (id, profile, default) = row.map_err(storage_error)?;
                let default = match default {
                    0 => false, 1 => true,
                    _ => return Err(AgentError::Storage(format!("agent {id} has an invalid default marker"))),
                };
                let profile = match profile {
                    Value::Text(profile) => serde_json::from_str(&profile).map_err(|error| error.to_string()),
                    _ => Err("profile is not JSON text".into()),
                };
                Ok(AgentRecord { id, profile, default })
            }).collect::<Result<_>>()?;
            if !records.is_empty() && records.iter().filter(|record| record.default).count() != 1 {
                return Err(AgentError::Storage("Agent directory has no unique default Agent".into()));
            }
            Ok(records)
        }).await
    }

    pub async fn save(&self, id: &str, profile: &AgentProfile, default: bool) -> Result<()> {
        let id = id.to_owned();
        let expected_profile = profile.clone();
        let profile = serde_json::to_string(profile)
            .map_err(|error| AgentError::Storage(error.to_string()))?;
        self.0.run(move |connection| {
            let existing = connection.query_row(
                "SELECT profile, is_default FROM agents WHERE agent_id = ?1", [&id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            ).optional().map_err(storage_error)?;
            match existing {
                Some((stored, is_default)) => {
                    let stored: AgentProfile = serde_json::from_str(&stored)
                        .map_err(|error| AgentError::Storage(format!("agent {id}: {error}")))?;
                    if stored == expected_profile && is_default == i64::from(default) {
                        Ok(())
                    } else {
                        Err(AgentError::Storage(format!("agent {id} differs from its stored profile")))
                    }
                },
                None => {
                    connection.execute("INSERT INTO agents(agent_id, profile, is_default) VALUES(?1, ?2, ?3)",
                        rusqlite::params![id, profile, default]).map_err(storage_error)?;
                    Ok(())
                }
            }
        }).await
    }

    pub async fn owners(&self) -> Result<Vec<SessionOwner>> {
        self.0.run(|connection| {
            let mut query = connection.prepare(
                "SELECT ids.session_id, s.session_id IS NOT NULL, o.agent_id, o.protocol_version, \
                 a.agent_id IS NOT NULL FROM (\
                   SELECT session_id FROM agent_sessions UNION SELECT session_id FROM session_owners\
                 ) ids LEFT JOIN agent_sessions s ON s.session_id = ids.session_id \
                 LEFT JOIN session_owners o ON o.session_id = ids.session_id \
                 LEFT JOIN agents a ON a.agent_id = o.agent_id ORDER BY ids.session_id",
            ).map_err(storage_error)?;
            query.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, Value>(2)?,
                    row.get::<_, Value>(3)?, row.get::<_, bool>(4)?))
            }).map_err(storage_error)?.map(|row| {
                let (session_id, has_session, agent, version, has_agent) = row.map_err(storage_error)?;
                let agent_id = match agent {
                    Value::Text(id) => Some(id),
                    _ => None,
                };
                let protocol_version = match version {
                    Value::Integer(version) if (1..=3).contains(&version) => Some(version as u32),
                    _ => None,
                };
                let error = if !has_session {
                    Some("Session directory is missing".into())
                } else if agent_id.is_none() {
                    Some("owning Agent is missing or invalid".into())
                } else if protocol_version.is_none() {
                    Some("protocol version is invalid or unsupported".into())
                } else if !has_agent {
                    Some("owning Agent is missing".into())
                } else {
                    None
                };
                let owner = agent_id.zip(protocol_version);
                Ok(SessionOwner { session_id, owner, error })
            }).collect()
        }).await
    }
}

fn storage_error(error: rusqlite::Error) -> AgentError {
    AgentError::Storage(error.to_string())
}
