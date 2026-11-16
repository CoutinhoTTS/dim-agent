//! The v50 image conversion, which rewrites only the rows that hold an image
//! and so derives text candidates and invalidates recovery snapshots only for
//! those rows and their Sessions.

use super::*;
use sha2::{Digest, Sha256};

/// The image every converting Session shares, so the conversion gives them one
/// identity and an ownership row each.
const SHARED_IMAGE: &[u8] = b"shared historical image bytes";

fn insert_event(
    connection: &Connection,
    session: &str,
    sequence: u64,
    kind: &str,
    data: serde_json::Value,
) {
    let body = serde_json::json!({
        "encoding": "event",
        "record": {
            "event": {
                "id": format!("event-{session}-{sequence}"),
                "sequence": sequence,
                "create_at": 1000 + sequence,
                "type": kind.split_once('/').unwrap().0,
                "name": kind.split_once('/').unwrap().1,
                "data": data,
            },
            "refs": null,
            "trace": null,
            "batch_end": sequence,
        }
    });
    connection
        .execute(
            "INSERT INTO timeline_events (session_id, branch_id, sequence, last_sequence, kind, body, event_id, context_render, context_state) VALUES (?1, 'main', ?2, ?2, ?3, ?4, ?5, 1, 0)",
            params![session, sequence as i64, kind, body.to_string(), format!("event-{session}-{sequence}")],
        )
        .unwrap();
}

/// A v49 store holding every shape the conversion rewrites and the ones it
/// must leave alone.
///
/// - `affected` holds an inline image in `input/message` and in
///   `step/tool-result`, a URL image, a request whose node content carries the
///   same image bytes, an earlier `step/text` delta, and the
///   `step/tool-result` whose candidates the conversion must derive again.
/// - `shared` holds the same image bytes, so the two Sessions share one file
///   and get an ownership row each.
/// - `untouched` holds no image, so the conversion never reaches it.
///
/// Every Session carries a snapshot: the conversion invalidates derived state
/// exactly where it changed a fact.
fn v49_store_with_images(database: &Database) -> Connection {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;

    let connection = Connection::open(&database.0).unwrap();
    let mut statements = schema_of(49);
    statements.sort_by_key(|sql| {
        !sql.starts_with("CREATE TABLE ") && !sql.starts_with("CREATE VIRTUAL TABLE ")
    });
    for sql in statements {
        connection.execute_batch(&sql).unwrap();
    }
    connection
        .execute("INSERT INTO agent_store_format VALUES (49)", [])
        .unwrap();
    for session in ["affected", "shared", "untouched"] {
        connection
            .execute(
                "INSERT INTO agents (agent_id, profile, is_default, owner_session_id) VALUES ('agent', '{}', 1, NULL)",
                [],
            )
            .ok();
        connection
            .execute(
                "INSERT INTO agent_sessions (session_id, config, created_at_ms, origin) VALUES (?1, '{}', 1, 'client')",
                [session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session_owners (session_id, agent_id, protocol_version) VALUES (?1, 'agent', 4)",
                [session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session_catalog (session_id, updated_at_ms, preview, cwd) VALUES (?1, 1, NULL, '/tmp')",
                [session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO timeline_branches (session_id, branch_id, info, sequence) VALUES (?1, 'main', '{\"id\":\"main\"}', 0)",
                [session],
            )
            .unwrap();
    }
    let encoded = STANDARD.encode(SHARED_IMAGE);
    let image = json!({"type": "image", "source": {"type": "base64", "mediaType": "image/png", "data": encoded}});
    let url = json!({"type": "image", "source": {"type": "url", "mediaType": "image/png", "url": "https://example.invalid/old.png"}});
    insert_event(
        &connection,
        "affected",
        1,
        "input/message",
        json!({"user": {"content": [image.clone(), url]}}),
    );
    // A request row whose node content carries the same image bytes. The row
    // is outside the text index and its node is not a candidate, so neither
    // has a candidate to re-derive; only the content is rewritten.
    let node = {
        let body = json!({"parent": null, "message": {"role": "user", "content": [image.clone()]}});
        let body = body.to_string();
        let hash = format!(
            "{:x}",
            Sha256::new()
                .chain_update("node")
                .chain_update([0])
                .chain_update(body.as_bytes())
                .finalize()
        );
        connection
            .execute(
                "INSERT INTO timeline_content (session_id, hash, kind, body) VALUES ('affected', ?1, 'node', ?2)",
                params![hash, body],
            )
            .unwrap();
        hash
    };
    insert_event(
        &connection,
        "affected",
        2,
        "model/call",
        json!({"turn_id": "turn", "request_id": "request", "request": {"providerId": "mock", "modelId": "test", "messages": []}}),
    );
    connection
        .execute(
            "UPDATE timeline_events SET body=json_set(body, '$.record.refs', json(?1)) WHERE session_id='affected' AND sequence=2",
            [json!({"list": node, "length": 1, "tools": null, "system": null}).to_string()],
        )
        .unwrap();
    insert_event(
        &connection,
        "affected",
        3,
        "step/text",
        json!({"turn_id": "turn", "request_id": "request", "delta": "an earlier delta"}),
    );
    // An indexed `data` row the conversion rewrites: its candidates must be
    // derived again from the converted body.
    insert_event(
        &connection,
        "affected",
        4,
        "step/tool-result",
        json!({"turn_id": "turn", "request_id": "request", "results": [{"call_id": "call", "result": {"content": [image]}}]}),
    );
    // A second Session holding the same accepted bytes.
    insert_event(
        &connection,
        "shared",
        1,
        "input/message",
        json!({"user": {"content": [{"type": "image", "source": {"type": "base64", "mediaType": "image/png", "data": STANDARD.encode(SHARED_IMAGE)}}]}}),
    );
    // A Session the conversion never reaches.
    insert_event(
        &connection,
        "untouched",
        1,
        "input/message",
        json!({"user": {"content": [{"type": "text", "text": "plain question"}]}}),
    );
    insert_event(
        &connection,
        "untouched",
        2,
        "step/text",
        json!({"turn_id": "turn", "request_id": "request", "delta": "plain answer"}),
    );
    for session in ["affected", "shared", "untouched"] {
        let sequence: i64 = connection
            .query_row(
                "SELECT max(sequence) FROM timeline_events WHERE session_id=?1",
                [session],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "UPDATE timeline_branches SET sequence=?2 WHERE session_id=?1",
                params![session, sequence],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO timeline_snapshots (session_id, branch_id, sequence, interpreter, event_id, state) VALUES (?1, 'main', ?2, 'dim/context-projection/v4', ?3, '{}')",
                params![session, sequence, format!("event-{session}-{sequence}")],
            )
            .unwrap();
    }
    // A v49 store carries the index v35 built, so the fixture derives it the
    // way that upgrade did: the conversion must leave every candidate it does
    // not rewrite as it found it.
    dim_app_server::storage::rebuild_text_index(&connection).unwrap();
    connection
}

/// The `timeline_text` rows of `connection`, without their rowid: candidates
/// a conversion derives per row must equal the ones a rebuild derives in
/// order, and neither's rowid is comparable.
fn candidates(connection: &Connection) -> Vec<(String, String, i64, i64, String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT session_id, branch_id, sequence, physical_sequence, property, partition_key \
             FROM timeline_text ORDER BY session_id, branch_id, property, sequence",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// The candidates `text` reaches through the index, as the search itself asks
/// for it: the trigrams of the text, all present. The index keeps no
/// positions, so a phrase cannot be queried.
fn matches(connection: &Connection, text: &str) -> Vec<(String, String)> {
    let text = text.to_lowercase();
    let grams = text
        .as_bytes()
        .windows(3)
        .map(|gram| format!("\"{}\"", std::str::from_utf8(gram).unwrap()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut statement = connection
        .prepare(
            "SELECT m.session_id, m.partition_key FROM timeline_text_fts f \
             CROSS JOIN timeline_text m ON m.id=f.rowid WHERE timeline_text_fts MATCH ?1 \
             ORDER BY m.session_id, m.partition_key",
        )
        .unwrap();
    statement
        .query_map([grams], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// #1589: the v50 conversion derives text candidates for the rows it rewrote
/// and no others, so the index an upgrade leaves equals the one a full rebuild
/// derives — the work differs, the result does not. Snapshots follow the same
/// rule: only the Sessions whose facts changed lose theirs.
#[tokio::test]
async fn the_v50_conversion_rebuilds_only_the_rewritten_rows_and_keeps_other_snapshots() {
    let database = Database::new();
    // A copy of the same v49 store, upgraded by a full rebuild instead, so the
    // two indexes can be compared row for row.
    let reference = Database::new();
    drop(v49_store_with_images(&database));
    std::fs::copy(&database.0, &reference.0).unwrap();

    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), Some(49));
    // The conversion derived candidates for the rows it rewrote that carry
    // candidates — two `input/message` rows and one `step/tool-result` — and
    // for no other row of the store, however many it holds.
    assert_eq!(
        db.converted_indexed(),
        Some(3),
        "the conversion derives candidates only for the rows it rewrote"
    );
    db.close().await.unwrap();

    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(marker(&connection), [i64::from(STORE_FORMAT)]);

    // (a) the input and (b) the tool result carry a file reference, and (c)
    // the request node content does too.
    let input: serde_json::Value = connection
        .query_row(
            "SELECT json_extract(body, '$.record.event.data.user.content') FROM timeline_events WHERE session_id='affected' AND sequence=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|content| serde_json::from_str(&content).unwrap())
        .unwrap();
    assert_eq!(input[0]["source"]["type"], "file", "{input}");
    assert!(input[0]["source"].get("data").is_none(), "{input}");
    let tool: serde_json::Value = connection
        .query_row(
            "SELECT json_extract(body, '$.record.event.data.results[0].result.content') FROM timeline_events WHERE session_id='affected' AND sequence=4",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|content| serde_json::from_str(&content).unwrap())
        .unwrap();
    assert_eq!(tool[0]["source"]["type"], "file", "{tool}");
    let node: serde_json::Value = connection
        .query_row(
            "SELECT body FROM timeline_content WHERE session_id='affected' AND kind='node'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|body| serde_json::from_str(&body).unwrap())
        .unwrap();
    assert_eq!(node["message"]["content"][0]["source"]["type"], "file");

    // (d) a URL image becomes the placeholder at its position.
    assert_eq!(
        input[1],
        serde_json::json!({"type": "text", "text": "[Image omitted: the image cannot be sent to this model.]"}),
        "{input}"
    );

    // (e) the two Sessions share one identity and get an ownership row each.
    let file = input[0]["source"]["fileId"].as_str().unwrap();
    let shared: serde_json::Value = connection
        .query_row(
            "SELECT json_extract(body, '$.record.event.data.user.content') FROM timeline_events WHERE session_id='shared' AND sequence=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|content| serde_json::from_str(&content).unwrap())
        .unwrap();
    assert_eq!(shared[0]["source"]["fileId"].as_str().unwrap(), file);
    let owners = connection
        .prepare("SELECT session_id FROM session_file_owners WHERE file_id=?1 ORDER BY session_id")
        .unwrap()
        .query_map([file], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(owners, ["affected", "shared"]);

    // (f) the Sessions the conversion rewrote lost their snapshots and (g)
    // the one it never reached kept its own.
    let snapshots = connection
        .prepare("SELECT session_id FROM timeline_snapshots ORDER BY session_id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        snapshots,
        ["untouched"],
        "only a Session whose facts changed loses its derived snapshot"
    );
    let converted = candidates(&connection);
    drop(connection);

    // The index the conversion left equals the one a full rebuild derives
    // from the same rows: the same candidates, and the same documents.
    let db = SqliteStorage::open(reference.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), Some(49));
    assert_eq!(
        db.converted_indexed(),
        Some(3),
        "the reference store converts the same rows"
    );
    db.close().await.unwrap();
    let connection = Connection::open(&reference.0).unwrap();
    dim_app_server::storage::rebuild_text_index(&connection).unwrap();
    assert_eq!(
        candidates(&connection),
        converted,
        "the row-by-row candidates are the ones a full rebuild derives"
    );
    for term in [
        "an-earlier-delta",
        "shared-historical-image",
        "plain-question",
        "plain-answer",
    ] {
        assert_eq!(
            matches(&connection, term),
            matches(&Connection::open(&database.0).unwrap(), term),
            "the index a term reaches differs for {term}"
        );
    }
}
