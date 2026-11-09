use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn expand(
    connection: &Connection,
    scope: &TimelineReadScope,
    query: &QuerySpec,
    page: Option<&QueryPage>,
    compiler: &mut Compiler<'_>,
    temporary: &mut Temporary<'_>,
    reader: &mut Reader,
    cancel: &CancellationToken,
    source: &str,
    event_condition: &str,
    after: &str,
    #[cfg(feature = "testing")] statistics: &Statistics,
) -> Result<(String, &'static str)> {
    let session = compiler.bind(scope.key.session_id.clone());
    let source = format!("{source}, array_events AS MATERIALIZED (
      SELECT a.*,query_array(a.array_inline) AS checked_array,
        query_array_length(a.array_id,a.array_length,h.array_length,a.array_inline IS NOT NULL) AS checked_length
      FROM events a LEFT JOIN timeline_array_elements h
        ON h.session_id={session} AND h.array_id=a.array_id AND h.element_index=0
      WHERE {event_condition})");
    if matches!(query.select, QuerySelect::Count)
        && query.group_by.is_empty()
        && query
            .filter
            .as_ref()
            .is_none_or(|filter| filter.row_conditions().1.is_empty())
    {
        return Ok((source, "array_events"));
    }
    let partial = query.filter.as_ref().and_then(before_element_value);
    let partial = partial.as_ref().map_or_else(
        || "1".to_owned(),
        |filter| compiler.filter(filter, "a", None, None),
    );
    let descending = match query.pick {
        Some(QueryPick::Last) => true,
        Some(QueryPick::First) => false,
        None => query
            .order_by
            .first()
            .is_some_and(|order| order.direction == TimelineQueryDirection::Descending),
    };
    let bounded = !compiler.element_values
        && query.group_by.is_empty()
        && (matches!(query.select, QuerySelect::Exists)
            || (matches!(query.select, QuerySelect::Fields { .. })
                && query
                    .order_by
                    .iter()
                    .all(|order| matches!(order.key, QueryOrderKey::Position))));
    let cursor = if bounded && query.pick.is_none() {
        after.replace("r.", "a.")
    } else {
        "1".to_owned()
    };
    let limit = bounded.then(|| {
        if query.pick.is_some() || matches!(query.select, QuerySelect::Exists) {
            1
        } else {
            page.expect("row queries have a page")
                .limit
                .get()
                .saturating_add(1)
        }
    });
    let seek = if bounded
        && query.pick.is_none()
        && let Some(cursor) = page.and_then(|page| page.after.as_ref())
    {
        let integer = |index: usize| match &cursor.keys[index] {
            QueryValue::Value { value } => value.as_u64().expect("validated position cursor"),
            _ => unreachable!("validated position cursor"),
        };
        let ordinal = compiler.bind(i64::try_from(integer(0)).map_err(error)?);
        let sequence = compiler.bind(i64::try_from(integer(1)).map_err(error)?);
        let index = compiler.bind(i64::try_from(integer(2)).map_err(error)?);
        let initial = if descending { i64::MAX } else { -1 };
        format!("CASE WHEN a.ord={ordinal} AND a.seq={sequence} THEN {index} ELSE {initial} END")
    } else {
        if descending {
            i64::MAX.to_string()
        } else {
            "-1".to_owned()
        }
    };
    let interval = if descending {
        format!("candidate.element_index<min(a.checked_length,{seek})")
    } else {
        format!("candidate.element_index<a.checked_length AND candidate.element_index>{seek}")
    };
    let indices = query
        .filter
        .as_ref()
        .and_then(|filter| equality_values(filter, "element.index"))
        .and_then(|values| {
            values
                .into_iter()
                .map(|value| value.as_u64())
                .collect::<Option<Vec<_>>>()
        })
        .map(|mut values| {
            values.sort_unstable();
            values.dedup();
            values
        });
    let (external, inline) = if let Some(indices) = indices {
        let indices = compiler.bind(serde_json::to_string(&indices).map_err(error)?);
        (
            format!("SELECT a.*,CAST(w.value AS INTEGER) AS element_index,NULL AS inline_value
              FROM array_events a CROSS JOIN json_each({indices}) w
              WHERE a.array_id IS NOT NULL AND CAST(w.value AS INTEGER)>=0 AND CAST(w.value AS INTEGER)<a.checked_length"),
            format!("SELECT a.*,CAST(w.value AS INTEGER) AS element_index,
                (a.checked_array -> ('$[' || w.value || ']')) AS inline_value
              FROM array_events a CROSS JOIN json_each({indices}) w
              WHERE a.array_id IS NULL AND CAST(w.value AS INTEGER)>=0 AND CAST(w.value AS INTEGER)<json_array_length(a.checked_array)"),
        )
    } else {
        let inline="SELECT a.*,CAST(w.key AS INTEGER) AS element_index,(a.checked_array -> ('$[' || w.key || ']')) AS inline_value
          FROM array_events a CROSS JOIN json_each(a.checked_array) w WHERE a.array_id IS NULL".to_owned();
        let external = if let Some(limit) = limit {
            let limit = compiler.bind(limit.min(i64::MAX as u64) as i64);
            let direction = if descending { "DESC" } else { "ASC" };
            let partial = partial.replace(
                "CAST(a.element_index AS TEXT)",
                "CAST(candidate.element_index AS TEXT)",
            );
            let cursor = cursor.replace("a.element_index", "candidate.element_index");
            format!("SELECT a.*,CAST(w.value AS INTEGER) AS element_index,NULL AS inline_value
              FROM array_events a CROSS JOIN json_each((SELECT json_group_array(selected_index) FROM (
                SELECT candidate.element_index AS selected_index FROM timeline_array_elements candidate
                WHERE candidate.session_id={session} AND candidate.array_id=a.array_id
                  AND ({interval}) AND ({partial}) AND ({cursor})
                ORDER BY candidate.element_index {direction} LIMIT {limit}))) w
              WHERE a.array_id IS NOT NULL")
        } else {
            format!("SELECT a.*,w.element_index,NULL AS inline_value FROM array_events a
              CROSS JOIN timeline_array_elements w ON w.session_id={session} AND w.array_id=a.array_id
              WHERE a.array_id IS NOT NULL AND w.element_index<a.checked_length")
        };
        (external, inline)
    };
    let bound = if let Some(limit) = limit {
        let direction = if descending { "DESC" } else { "ASC" };
        let limit = compiler.bind(limit.min(i64::MAX as u64) as i64);
        format!(
            " ORDER BY a.ord {direction},a.seq {direction},a.element_index {direction} LIMIT {limit}"
        )
    } else {
        String::new()
    };
    let source=format!("{source}, array_positions AS ({external} UNION ALL {inline}),
      array_requested AS MATERIALIZED (SELECT a.* FROM array_positions a WHERE ({partial}) AND ({cursor}){bound}),
      array_keys AS MATERIALIZED (SELECT a.* FROM array_requested a
        LEFT JOIN timeline_array_elements slot ON slot.session_id={session} AND slot.array_id=a.array_id AND slot.element_index=a.element_index
        LEFT JOIN timeline_content body ON body.session_id={session} AND body.hash=slot.content_ref
        WHERE CASE WHEN a.array_id IS NULL THEN 1 ELSE query_array_slot(body.kind) END)");
    if !compiler.element_values {
        return Ok((
            format!(
                "{source}, expanded AS (SELECT a.*,a.inline_value AS element_value FROM array_keys a)"
            ),
            "expanded",
        ));
    }
    let table=temporary.table("ord INTEGER,seq INTEGER,element_index INTEGER,value TEXT,PRIMARY KEY(ord,seq,element_index)")?;
    let sql = format!(
        "{source} SELECT a.ord,a.seq,a.element_index,a.array_id,a.array_length,a.inline_value FROM array_keys a"
    );
    let mut statement = connection.prepare(&sql).map_err(error)?;
    let mut rows = statement
        .query(params_from_iter(&compiler.parameters))
        .map_err(error)?;
    let mut insert = connection
        .prepare(&format!("INSERT INTO {table} VALUES(?1,?2,?3,?4)"))
        .map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        cancelled(cancel)?;
        let index: i64 = row.get(2).map_err(error)?;
        let id: Option<String> = row.get(3).map_err(error)?;
        let value = if let Some(id) = id {
            let length: i64 = row.get(4).map_err(error)?;
            let array = crate::storage::arrays::Reference {
                id,
                length: u64::try_from(length).map_err(error)?,
            };
            loop {
                cancelled(cancel)?;
                if let Some(value) = reader.element(
                    connection,
                    &scope.key.session_id,
                    &array,
                    u64::try_from(index).map_err(error)?,
                )? {
                    break serde_json::to_string(&value).map_err(error)?;
                }
                reader.begin_hold();
            }
        } else {
            row.get::<_, String>(5).map_err(error)?
        };
        insert
            .execute(params![
                row.get::<_, i64>(0).map_err(error)?,
                row.get::<_, i64>(1).map_err(error)?,
                index,
                value
            ])
            .map_err(error)?;
    }
    drop(rows);
    #[cfg(feature = "testing")]
    record_statement(
        statistics,
        connection,
        &statement,
        &sql,
        &compiler.parameters,
        "array_candidates",
    )?;
    Ok((
        format!(
            "{source}, expanded AS (SELECT a.*,v.value AS element_value FROM array_keys a JOIN {table} v USING(ord,seq,element_index))"
        ),
        "expanded",
    ))
}
