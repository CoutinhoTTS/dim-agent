use super::sqlite::error;
use dim_agent_runtime::{
    QueryComparison, QueryFilter, QueryOperand, QueryOrderKey, QuerySelect, QuerySpec, QueryValue,
    Result, TimelineQueryDirection, TimelineReadScope, event_property,
};
use dim_agent_types::AgentEvent;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

mod tree;
use tree::Tree;

pub(super) const SCHEMA: &[(&str, &str)] = &[
    (
        "timeline_member_nodes",
        "CREATE TABLE timeline_member_nodes (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK(sequence > 0), left_id INTEGER, right_id INTEGER, height INTEGER NOT NULL CHECK(height BETWEEN 1 AND 64))",
    ),
    (
        "timeline_member_roots",
        "CREATE TABLE timeline_member_roots (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, index_id TEXT NOT NULL, group_key BLOB NOT NULL, sequence INTEGER NOT NULL CHECK(sequence >= 0), root_id INTEGER, PRIMARY KEY(session_id,branch_id,index_id,group_key,sequence))",
    ),
    (
        "timeline_member_origins",
        "CREATE TABLE timeline_member_origins (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, index_id TEXT NOT NULL, match_key BLOB NOT NULL, sequence INTEGER NOT NULL CHECK(sequence > 0), group_key BLOB NOT NULL, PRIMARY KEY(session_id,branch_id,index_id,match_key,sequence,group_key))",
    ),
    (
        "timeline_member_terminals",
        "CREATE TABLE timeline_member_terminals (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, index_id TEXT NOT NULL, match_key BLOB NOT NULL, sequence INTEGER NOT NULL CHECK(sequence > 0), PRIMARY KEY(session_id,branch_id,index_id,match_key))",
    ),
];

struct Terminal {
    kind: &'static str,
    key: &'static str,
    predicate: Option<(&'static str, &'static str)>,
}

enum Groups {
    None,
    Presence(&'static str),
    Elements(&'static str, &'static str),
}

struct Profile {
    id: &'static str,
    origin: &'static str,
    key: &'static str,
    terminals: &'static [Terminal],
    groups: Groups,
}

const PROFILES: &[Profile] = &[
    Profile {
        id: "input/v1",
        origin: "input/message",
        key: "id",
        terminals: &[
            Terminal {
                kind: "input/consumed",
                key: "data.input_id",
                predicate: None,
            },
            Terminal {
                kind: "input/deleted",
                key: "data.input_id",
                predicate: None,
            },
        ],
        groups: Groups::Presence("data.handover"),
    },
    Profile {
        id: "tool/v1",
        origin: "step/tool-call",
        key: "id",
        terminals: &[Terminal {
            kind: "step/tool-result",
            key: "data.call_event_id",
            predicate: None,
        }],
        groups: Groups::Elements("data.calls", "name"),
    },
    Profile {
        id: "exec/v1",
        origin: "exec/started",
        key: "data.task_id",
        terminals: &[Terminal {
            kind: "exec/ended",
            key: "data.task_id",
            predicate: None,
        }],
        groups: Groups::None,
    },
    Profile {
        id: "collaboration/v1",
        origin: "collaboration/request",
        key: "id",
        terminals: &[
            Terminal {
                kind: "collaboration/result",
                key: "data.request_event_id",
                predicate: None,
            },
            Terminal {
                kind: "collaboration/delivered",
                key: "data.op_event_id",
                predicate: Some(("data.status", "error")),
            },
        ],
        groups: Groups::None,
    },
];

pub(super) fn branch(connection: &Connection, session: &str, branch: &str) -> Result<()> {
    for profile in PROFILES {
        connection
            .prepare_cached(
                "INSERT OR IGNORE INTO timeline_member_roots VALUES(?1,?2,?3,?4,0,NULL)",
            )
            .map_err(error)?
            .execute(params![session, branch, profile.id, &[0u8][..]])
            .map_err(error)?;
    }
    Ok(())
}

fn match_key(value: QueryValue) -> Result<Option<Vec<u8>>> {
    match value {
        QueryValue::Missing => Ok(None),
        value => value.scalar_key().map(Some),
    }
}

fn groups(profile: &Profile, event: &AgentEvent, branch: &str) -> Result<Vec<Vec<u8>>> {
    let mut result = Vec::new();
    match profile.groups {
        Groups::None => result.push(vec![0]),
        Groups::Presence(field) => {
            if matches!(event_property(event, branch, field)?, QueryValue::Missing) {
                result.push(vec![1, 0]);
            }
        }
        Groups::Elements(field, path) => {
            let QueryValue::Elements { values } =
                event_property(event, branch, &format!("{field}[].{path}"))?
            else {
                return Err(error("member group source is not an array"));
            };
            for value in values {
                if let Some(mut key) = match_key(value)? {
                    key.insert(0, 2);
                    if !result.contains(&key) {
                        result.push(key);
                    }
                }
            }
        }
    }
    Ok(result)
}

fn root(
    connection: &Connection,
    session: &str,
    branch: &str,
    profile: &Profile,
    group: &[u8],
    through: i64,
    #[cfg(feature = "testing")] statistics: Option<&super::query::Statistics>,
) -> Result<Option<i64>> {
    let sql = "SELECT root_id FROM timeline_member_roots WHERE session_id=?1 AND branch_id=?2 AND index_id=?3 AND group_key=?4 AND sequence<=?5 ORDER BY sequence DESC LIMIT 1";
    let mut statement = connection.prepare_cached(sql).map_err(error)?;
    #[cfg(feature = "testing")]
    if statistics.is_some() {
        for status in [
            rusqlite::StatementStatus::VmStep,
            rusqlite::StatementStatus::FullscanStep,
            rusqlite::StatementStatus::Sort,
            rusqlite::StatementStatus::AutoIndex,
        ] {
            statement.reset_status(status);
        }
    }
    let version = statement
        .query_row(
            params![session, branch, profile.id, group, through],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(error)?;
    #[cfg(feature = "testing")]
    if let Some(statistics) = statistics {
        super::query::record_statement(
            statistics,
            connection,
            &statement,
            sql,
            &[
                session.to_owned().into(),
                branch.to_owned().into(),
                profile.id.to_owned().into(),
                group.to_vec().into(),
                through.into(),
            ],
            "member_root",
        )?;
    }
    if version.is_none() {
        if group == [0] {
            return Err(error("member index initialization is missing"));
        }
        root(
            connection,
            session,
            branch,
            profile,
            &[0],
            0,
            #[cfg(feature = "testing")]
            statistics,
        )?;
    }
    Ok(version.flatten())
}

fn save_root(
    connection: &Connection,
    session: &str,
    branch: &str,
    profile: &Profile,
    group: &[u8],
    sequence: i64,
    root: Option<i64>,
) -> Result<()> {
    connection.prepare_cached("INSERT INTO timeline_member_roots VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,branch_id,index_id,group_key,sequence) DO UPDATE SET root_id=excluded.root_id")
        .map_err(error)?.execute(params![session, branch, profile.id, group, sequence, root]).map_err(error)?;
    Ok(())
}

pub(super) fn event(
    connection: &Connection,
    session: &str,
    branch: &str,
    event: &AgentEvent,
) -> Result<()> {
    let kind = event.kind.key();
    let sequence = i64::try_from(event.sequence).map_err(error)?;
    let tree = Tree {
        connection,
        session,
        #[cfg(feature = "testing")]
        statistics: None,
    };
    for profile in PROFILES {
        if profile.origin == kind {
            let groups = groups(profile, event, branch)?;
            if groups.is_empty() {
                continue;
            }
            let Some(key) = match_key(event_property(event, branch, profile.key)?)? else {
                continue;
            };
            let closed = connection.prepare_cached("SELECT 1 FROM timeline_member_terminals WHERE session_id=?1 AND branch_id=?2 AND index_id=?3 AND match_key=?4")
                .map_err(error)?.exists(params![session, branch, profile.id, key]).map_err(error)?;
            if closed {
                continue;
            }
            let mut changes = BTreeMap::new();
            for group in groups {
                let previous = root(
                    connection,
                    session,
                    branch,
                    profile,
                    &group,
                    sequence,
                    #[cfg(feature = "testing")]
                    None,
                )?;
                let next = match changes.get(&previous) {
                    Some(next) => *next,
                    None => {
                        let next = tree.insert(previous, sequence)?;
                        changes.insert(previous, next);
                        next
                    }
                };
                save_root(
                    connection,
                    session,
                    branch,
                    profile,
                    &group,
                    sequence,
                    Some(next),
                )?;
                connection
                    .prepare_cached("INSERT INTO timeline_member_origins VALUES(?1,?2,?3,?4,?5,?6)")
                    .map_err(error)?
                    .execute(params![session, branch, profile.id, key, sequence, group])
                    .map_err(error)?;
            }
        }
        for terminal in profile
            .terminals
            .iter()
            .filter(|terminal| terminal.kind == kind)
        {
            if let Some((field, expected)) = terminal.predicate
                && event_property(event, branch, field)?
                    != (QueryValue::Value {
                        value: expected.into(),
                    })
            {
                continue;
            }
            let Some(key) = match_key(event_property(event, branch, terminal.key)?)? else {
                continue;
            };
            connection.prepare_cached("INSERT INTO timeline_member_terminals VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING")
                .map_err(error)?.execute(params![session, branch, profile.id, key, sequence]).map_err(error)?;
            let origins = connection.prepare_cached("SELECT sequence,group_key FROM timeline_member_origins WHERE session_id=?1 AND branch_id=?2 AND index_id=?3 AND match_key=?4")
                .map_err(error)?.query_map(params![session, branch, profile.id, key], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)))
                .map_err(error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(error)?;
            let mut changes = BTreeMap::new();
            for (origin, group) in origins {
                let previous = root(
                    connection,
                    session,
                    branch,
                    profile,
                    &group,
                    sequence,
                    #[cfg(feature = "testing")]
                    None,
                )?;
                let next = match changes.get(&(previous, origin)) {
                    Some(next) => *next,
                    None => {
                        let next = tree.remove(previous, origin)?;
                        changes.insert((previous, origin), next);
                        next
                    }
                };
                save_root(connection, session, branch, profile, &group, sequence, next)?;
            }
            connection.prepare_cached("DELETE FROM timeline_member_origins WHERE session_id=?1 AND branch_id=?2 AND index_id=?3 AND match_key=?4")
                .map_err(error)?.execute(params![session, branch, profile.id, key]).map_err(error)?;
        }
    }
    Ok(())
}

pub(super) fn row(
    connection: &Connection,
    session: &str,
    branch: &str,
    kind: &str,
    encoded: &str,
) -> Result<()> {
    if !PROFILES.iter().any(|profile| {
        profile.origin == kind
            || profile
                .terminals
                .iter()
                .any(|terminal| terminal.kind == kind)
    }) {
        return Ok(());
    }
    let super::timeline::Row::Event { record } = serde_json::from_str(encoded).map_err(error)?
    else {
        return Err(error("member origin or terminal has an output encoding"));
    };
    let event = serde_json::from_value::<AgentEvent>(record.event).map_err(error)?;
    if event.kind.key() != kind {
        return Err(error("member event kind disagrees with its stored kind"));
    }
    self::event(connection, session, branch, &event)
}

pub(super) fn backfill(connection: &Connection) -> Result<()> {
    build(connection, None)
}

/// Builds member indexes for imported events inside their insertion transaction.
/// The Session's fact rows must not already have been indexed.
pub fn initialize_session_member_index(connection: &Connection, session: &str) -> Result<()> {
    build(connection, Some(session))
}

fn build(connection: &Connection, session: Option<&str>) -> Result<()> {
    let mut branches = connection
        .prepare(
            "SELECT session_id,branch_id FROM timeline_branches WHERE ?1 IS NULL OR session_id=?1",
        )
        .map_err(error)?;
    let mut rows = branches.query([session]).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        branch(
            connection,
            &row.get::<_, String>(0).map_err(error)?,
            &row.get::<_, String>(1).map_err(error)?,
        )?;
    }
    let kinds = PROFILES
        .iter()
        .flat_map(|profile| {
            std::iter::once(profile.origin)
                .chain(profile.terminals.iter().map(|terminal| terminal.kind))
        })
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = connection.prepare(&format!("SELECT session_id,branch_id,kind,body FROM timeline_events WHERE kind IN ({kinds}) AND (?1 IS NULL OR session_id=?1) ORDER BY session_id,branch_id,sequence")).map_err(error)?;
    let mut rows = statement.query([session]).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        self::row(
            connection,
            &row.get::<_, String>(0).map_err(error)?,
            &row.get::<_, String>(1).map_err(error)?,
            &row.get::<_, String>(2).map_err(error)?,
            &row.get::<_, String>(3).map_err(error)?,
        )?;
    }
    Ok(())
}

fn terms<'a>(filter: &'a QueryFilter, result: &mut Vec<&'a QueryFilter>) {
    if let QueryFilter::And { conditions } = filter {
        for condition in conditions {
            terms(condition, result);
        }
    } else {
        result.push(filter);
    }
}

fn values<'a>(filter: &'a QueryFilter, field: &str) -> Option<Vec<&'a Value>> {
    match filter {
        QueryFilter::In {
            value: QueryOperand::Field { path },
            values,
        } if path == field => Some(values.iter().collect()),
        QueryFilter::Compare {
            left: QueryOperand::Field { path },
            comparison: QueryComparison::Eq,
            right: QueryOperand::Value { value },
        }
        | QueryFilter::Compare {
            right: QueryOperand::Field { path },
            comparison: QueryComparison::Eq,
            left: QueryOperand::Value { value },
        } if path == field => Some(vec![value]),
        _ => None,
    }
}

fn closes(profile: &Profile, filter: &QueryFilter, seen: &mut Vec<&'static str>) -> bool {
    if let QueryFilter::Or { conditions } = filter {
        return conditions
            .iter()
            .all(|condition| closes(profile, condition, seen));
    }
    let mut conditions = Vec::new();
    terms(filter, &mut conditions);
    let Some((at, kinds)) = conditions
        .iter()
        .enumerate()
        .find_map(|(at, condition)| values(condition, "kind").map(|kinds| (at, kinds)))
    else {
        return false;
    };
    conditions.remove(at);
    if kinds.is_empty() {
        return false;
    }
    for kind in kinds {
        let Some(terminal) = profile
            .terminals
            .iter()
            .find(|terminal| kind.as_str() == Some(terminal.kind))
        else {
            return false;
        };
        let mut joined = false;
        let mut predicate = terminal.predicate.is_none();
        for condition in &conditions {
            match condition {
                QueryFilter::Compare {
                    left: QueryOperand::Field { path: inner },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Outer { path: outer },
                }
                | QueryFilter::Compare {
                    right: QueryOperand::Field { path: inner },
                    comparison: QueryComparison::Eq,
                    left: QueryOperand::Outer { path: outer },
                } if inner == terminal.key && outer == profile.key && !joined => joined = true,
                condition
                    if terminal.predicate.is_some_and(|(field, expected)| {
                        values(condition, field).is_some_and(|values| {
                            values.len() == 1 && values[0].as_str() == Some(expected)
                        })
                    }) && !predicate =>
                {
                    predicate = true
                }
                _ => return false,
            }
        }
        if !joined || !predicate {
            return false;
        }
        if !seen.contains(&terminal.kind) {
            seen.push(terminal.kind);
        }
    }
    true
}

fn group(profile: &Profile, filter: &QueryFilter) -> Result<Option<Vec<u8>>> {
    Ok(match (&profile.groups, filter) {
        (Groups::Presence(field), QueryFilter::Not { condition }) if matches!(condition.as_ref(), QueryFilter::Present { value: QueryOperand::Field { path } } if path == field) => {
            Some(vec![1, 0])
        }
        (
            Groups::Elements(field, name),
            QueryFilter::Any {
                field: array,
                condition,
            },
        ) if array == field => {
            if let QueryFilter::Compare {
                left: QueryOperand::Element { path },
                comparison: QueryComparison::Eq,
                right: QueryOperand::Value { value },
            }
            | QueryFilter::Compare {
                right: QueryOperand::Element { path },
                comparison: QueryComparison::Eq,
                left: QueryOperand::Value { value },
            } = condition.as_ref()
                && path == name
            {
                let mut key = QueryValue::Value {
                    value: value.clone(),
                }
                .scalar_key()?;
                key.insert(0, 2);
                Some(key)
            } else {
                None
            }
        }
        _ => None,
    })
}

pub(super) struct Candidates<'a> {
    pub terminal: &'a QueryFilter,
    pub positions: Vec<u64>,
}

#[cfg_attr(feature = "testing", allow(clippy::too_many_arguments))]
pub(super) fn candidates<'a>(
    connection: &Connection,
    scope: &TimelineReadScope,
    query: &'a QuerySpec,
    limit: usize,
    after: Option<(u64, u64, bool)>,
    cancel: &CancellationToken,
    #[cfg(feature = "testing")] statistics: &super::query::Statistics,
) -> Result<Option<Candidates<'a>>> {
    let [segment] = scope.segments.as_slice() else {
        return Ok(None);
    };
    if segment.from != 1
        || !segment.tail.is_empty()
        || !query.group_by.is_empty()
        || query.pick.is_some()
        || !matches!(
            query.select,
            QuerySelect::Fields { .. } | QuerySelect::Events
        )
        || !matches!(query.order_by.as_slice(), [] | [_])
        || query
            .order_by
            .first()
            .is_some_and(|order| order.key != QueryOrderKey::Position)
    {
        return Ok(None);
    }
    let Some(filter) = &query.filter else {
        return Ok(None);
    };
    let mut conditions = Vec::new();
    terms(filter, &mut conditions);
    for profile in PROFILES {
        let mut origin = false;
        let mut terminal = None;
        let mut grouped = None;
        let mut valid = true;
        for condition in &conditions {
            if values(condition, "kind").is_some_and(|values| {
                values.len() == 1 && values[0].as_str() == Some(profile.origin)
            }) && !origin
            {
                origin = true;
                continue;
            }
            if let QueryFilter::Not { condition } = condition
                && let QueryFilter::Exists { condition } = condition.as_ref()
                && terminal.is_none()
            {
                let mut seen = Vec::new();
                if closes(profile, condition, &mut seen) && seen.len() == profile.terminals.len() {
                    terminal = Some(condition.as_ref());
                    continue;
                }
            }
            if grouped.is_none()
                && let Some(key) = group(profile, condition)?
            {
                grouped = Some(key);
                continue;
            }
            valid = false;
            break;
        }
        if !valid || !origin || (!matches!(profile.groups, Groups::None) && grouped.is_none()) {
            continue;
        }
        let Some(terminal) = terminal else { continue };
        let descending = query
            .order_by
            .first()
            .is_some_and(|order| order.direction == TimelineQueryDirection::Descending);
        let through = i64::try_from(segment.through.min(segment.stored_through)).map_err(error)?;
        let mut low = 1;
        let mut high = through;
        if let Some((ordinal, sequence, _)) = after {
            if ordinal != 0 {
                return Ok(Some(Candidates {
                    terminal,
                    positions: Vec::new(),
                }));
            }
            let sequence = i64::try_from(sequence).map_err(error)?;
            if descending {
                high = high.min(sequence.saturating_sub(1));
            } else {
                low = low.max(sequence.saturating_add(1));
            }
        }
        let group = grouped.unwrap_or_else(|| vec![0]);
        let root = root(
            connection,
            &scope.key.session_id,
            &segment.branch_id,
            profile,
            &group,
            through,
            #[cfg(feature = "testing")]
            Some(statistics),
        )?;
        let members = Tree {
            connection,
            session: &scope.key.session_id,
            #[cfg(feature = "testing")]
            statistics: Some(statistics),
        }
        .page(root, low, high, descending, limit, cancel)?;
        return Ok(Some(Candidates {
            terminal,
            positions: members
                .into_iter()
                .map(|sequence| sequence as u64)
                .collect(),
        }));
    }
    Ok(None)
}
