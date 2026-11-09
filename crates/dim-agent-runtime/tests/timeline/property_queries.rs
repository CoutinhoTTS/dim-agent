use super::*;
use dim_agent_runtime::{
    QueryComparison, QueryFilter, QueryOperand, QueryOrder, QueryOrderKey, QueryOutput, QueryPage,
    QueryPick, QueryRow, QuerySelect, QuerySpec, QueryValue, TimelineQueryDirection,
    TimelineReadScope, TimelineReadSegment,
};
use serde_json::{Value, json};
use std::num::NonZeroU64;

async fn setup() -> (Arc<MemoryTimelineStore>, Timeline) {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("properties", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "properties".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    (store, timeline)
}

fn fact(name: &str, data: Value) -> EventDraft {
    EventKind::Extension(ExtensionEvent::new("property", name, data).unwrap()).into()
}

fn scope(through: u64) -> TimelineReadScope {
    TimelineReadScope {
        key: TimelineKey {
            session_id: "properties".into(),
            branch_id: "main".into(),
        },
        through,
        segments: vec![TimelineReadSegment {
            branch_id: "main".into(),
            from: 1,
            through,
            stored_through: through,
            tail: vec![],
        }],
    }
}

fn fields(names: &[&str]) -> QuerySpec {
    QuerySpec {
        rows: None,
        filter: None,
        select: QuerySelect::Fields {
            fields: names.iter().map(|s| (*s).into()).collect(),
        },
        group_by: vec![],
        pick: None,
        order_by: vec![],
    }
}

fn eq(path: &str, value: Value) -> QueryFilter {
    QueryFilter::Compare {
        left: QueryOperand::Field { path: path.into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value },
    }
}

fn page(limit: u64) -> QueryPage {
    QueryPage {
        limit: NonZeroU64::new(limit).unwrap(),
        after: None,
    }
}

fn value(value: Value) -> QueryValue {
    QueryValue::Value { value }
}

fn rows(output: QueryOutput) -> Vec<QueryRow> {
    let QueryOutput::Rows { rows } = output else {
        panic!("field queries return field rows")
    };
    rows
}

#[tokio::test]
async fn timeline_property_queries_keep_a_fixed_chain_and_validate_positions() {
    let (store, main) = setup().await;
    main.append(fact("value", json!({"name":"parent"})))
        .unwrap();
    main.append(branch_create("child", 2)).unwrap();
    let child = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "properties".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 2)],
        store,
    );
    child
        .append(fact("value", json!({"name":"child"})))
        .unwrap();
    let mut request = dim_agent_runtime::TimelineQuery {
        scope: TimelineQueryScope::Chain,
        from: None,
        through: None,
        include_publishing: false,
        query: fields(&["data.name", "position"]),
        page: Some(page(1)),
    };
    request.query.filter = Some(eq("kind", json!("property/value")));
    let first = child.query(request.clone()).await.unwrap();
    assert_eq!(first.through, 1);
    assert_eq!(rows(first.result)[0].values[0], value(json!("parent")));
    child
        .append(fact("value", json!({"name":"later"})))
        .unwrap();
    request.page.as_mut().unwrap().after = first.next;
    assert!(
        child.query(request.clone()).await.is_err(),
        "a continuation must supply its original bound"
    );
    request.through = Some(1);
    let second = child.query(request.clone()).await.unwrap();
    assert_eq!(rows(second.result)[0].values[0], value(json!("child")));
    assert!(second.next.is_none());
    request.page = Some(page(10));
    request.from = Some(TimelinePosition {
        branch_id: "child".into(),
        sequence: 1,
    });
    assert_eq!(
        rows(child.query(request.clone()).await.unwrap().result).len(),
        1
    );
    request.from = Some(TimelinePosition {
        branch_id: "main".into(),
        sequence: 3,
    });
    assert!(child.query(request.clone()).await.is_err());
    request.from = None;
    request.through = Some(3);
    assert!(child.query(request).await.is_err());
}

#[tokio::test]
async fn property_rows_preserve_missing_null_and_array_positions() {
    let (store, timeline) = setup().await;
    timeline
        .append(fact(
            "data",
            json!({"empty":null, "items":[{"id":"a"},{"id":null},{}]}),
        ))
        .unwrap();
    let query = fields(&[
        "data.empty",
        "data.absent",
        "data.items[].id",
        "data.items[0].id",
        "data.items[1].id",
        "data.items[2].id",
        "data.items[3]",
        "data.items[2147483647]",
        "position.branch_id",
        "position.sequence",
    ]);
    let reply = store
        .query(&scope(1), &query, Some(&page(10)))
        .await
        .unwrap();
    assert_eq!(
        rows(reply.result)[0].values,
        vec![
            value(Value::Null),
            QueryValue::Missing,
            QueryValue::Elements {
                values: vec![value(json!("a")), value(Value::Null), QueryValue::Missing]
            },
            value(json!("a")),
            value(Value::Null),
            QueryValue::Missing,
            QueryValue::Missing,
            QueryValue::Missing,
            value(json!("main")),
            value(json!(1)),
        ]
    );
    let mut query = fields(&["id"]);
    query.filter = Some(eq("data.empty", Value::Null));
    assert_eq!(
        rows(
            store
                .query(&scope(1), &query, Some(&page(1)))
                .await
                .unwrap()
                .result
        )
        .len(),
        1
    );
    query.filter = Some(eq("data.absent", Value::Null));
    assert!(
        rows(
            store
                .query(&scope(1), &query, Some(&page(1)))
                .await
                .unwrap()
                .result
        )
        .is_empty()
    );
}

#[tokio::test]
async fn related_queries_share_ancestor_and_tail_bounds_across_pages() {
    let (store, main) = setup().await;
    for key in ["a", "c", "d"] {
        main.append(fact("start", json!({"key":key}))).unwrap();
    }
    main.append(branch_create("child", 4)).unwrap();
    let child = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "properties".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 4)],
        store.clone(),
    );
    child.append(fact("done", json!({"key":"a"}))).unwrap();
    child.append(fact("start", json!({"key":"b"}))).unwrap();
    let tail = child.append(fact("done", json!({"key":"d"}))).unwrap();
    let mut scope = scope(4);
    scope.key.branch_id = "child".into();
    scope.through = 3;
    scope.segments.push(TimelineReadSegment {
        branch_id: "child".into(),
        from: 1,
        through: 3,
        stored_through: 2,
        tail: vec![tail],
    });
    let mut query = fields(&["data.key", "position.branch_id"]);
    query.filter = Some(QueryFilter::And {
        conditions: vec![
            eq("kind", json!("property/start")),
            QueryFilter::Not {
                condition: Box::new(QueryFilter::Exists {
                    condition: Box::new(QueryFilter::And {
                        conditions: vec![
                            eq("kind", json!("property/done")),
                            QueryFilter::Compare {
                                left: QueryOperand::Field {
                                    path: "data.key".into(),
                                },
                                comparison: QueryComparison::Eq,
                                right: QueryOperand::Outer {
                                    path: "data.key".into(),
                                },
                            },
                        ],
                    }),
                }),
            },
        ],
    });
    query.order_by = vec![QueryOrder {
        key: QueryOrderKey::Position,
        direction: TimelineQueryDirection::Descending,
    }];
    let first = store.query(&scope, &query, Some(&page(1))).await.unwrap();
    assert_eq!(
        rows(first.result)[0].values,
        vec![value(json!("b")), value(json!("child"))]
    );
    child.append(fact("start", json!({"key":"later"}))).unwrap();
    scope.segments[1].stored_through = 3;
    scope.segments[1].tail.clear();
    let second = store
        .query(
            &scope,
            &query,
            Some(&QueryPage {
                limit: NonZeroU64::new(1).unwrap(),
                after: first.next,
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        rows(second.result)[0].values,
        vec![value(json!("c")), value(json!("main"))]
    );
    assert!(
        second.next.is_none(),
        "terminal facts before the output cursor still exclude a and d"
    );
    query.select = QuerySelect::Count;
    query.order_by.clear();
    let result = store.query(&scope, &query, None).await.unwrap();
    assert!(matches!(result.result, QueryOutput::Count { count: 2 }));
}

#[tokio::test]
async fn group_picks_keep_fields_from_one_event_and_counts_use_numeric_equality() {
    let (store, timeline) = setup().await;
    for data in [
        json!({"key":1,"price":100,"label":"old"}),
        json!({"key":1.0,"price":2,"label":"new"}),
        json!({"key":true,"price":9,"label":"boolean"}),
    ] {
        timeline.append(fact("config", data)).unwrap();
    }
    let mut query = fields(&["data.price", "data.label"]);
    query.group_by = vec!["data.key".into()];
    query.pick = Some(QueryPick::Last);
    let result = rows(
        store
            .query(&scope(3), &query, Some(&page(10)))
            .await
            .unwrap()
            .result,
    );
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].values, vec![value(json!(2)), value(json!("new"))]);
    query.select = QuerySelect::Count;
    query.pick = None;
    let result = rows(
        store
            .query(&scope(3), &query, Some(&page(10)))
            .await
            .unwrap()
            .result,
    );
    assert_eq!(
        result,
        vec![
            QueryRow {
                group: vec![value(json!(true))],
                values: vec![value(json!(1))]
            },
            QueryRow {
                group: vec![value(json!(1))],
                values: vec![value(json!(2))]
            },
        ]
    );
}

#[tokio::test]
async fn array_conditions_match_the_same_element_and_keep_numeric_precision() {
    let (store, timeline) = setup().await;
    timeline.append(fact("items", json!({"items":[{"id":"a","ready":false},{"id":"b","ready":true}],"large":18446744073709551615_u64}))).unwrap();
    timeline
        .append(fact(
            "items",
            json!({"items":[{"id":"a","ready":true}],"large":18446744073709551614_u64}),
        ))
        .unwrap();
    let mut query = fields(&["position.sequence"]);
    query.filter = Some(QueryFilter::Any {
        field: "data.items".into(),
        condition: Box::new(QueryFilter::And {
            conditions: vec![
                QueryFilter::Compare {
                    left: QueryOperand::Element { path: "id".into() },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Value { value: json!("a") },
                },
                QueryFilter::Compare {
                    left: QueryOperand::Element {
                        path: "ready".into(),
                    },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Value { value: json!(true) },
                },
            ],
        }),
    });
    assert_eq!(
        rows(
            store
                .query(&scope(2), &query, Some(&page(10)))
                .await
                .unwrap()
                .result
        )[0]
        .values,
        vec![value(json!(2))]
    );
    query.filter = Some(eq("data.large", json!(18446744073709551615_u64)));
    assert_eq!(
        rows(
            store
                .query(&scope(2), &query, Some(&page(10)))
                .await
                .unwrap()
                .result
        )[0]
        .values,
        vec![value(json!(1))]
    );
}

#[tokio::test]
async fn cursors_bind_queries_and_keyset_pages_compose_in_both_directions() {
    let (store, timeline) = setup().await;
    for key in [2, 1, 1, 3] {
        timeline.append(fact("item", json!({"key":key}))).unwrap();
    }
    for direction in [
        TimelineQueryDirection::Ascending,
        TimelineQueryDirection::Descending,
    ] {
        let mut query = fields(&["position.sequence"]);
        query.order_by = vec![QueryOrder {
            key: QueryOrderKey::Field {
                path: "data.key".into(),
            },
            direction,
        }];
        let whole = rows(
            store
                .query(&scope(4), &query, Some(&page(10)))
                .await
                .unwrap()
                .result,
        );
        let mut paged = Vec::new();
        let mut next = page(1);
        loop {
            let result = store.query(&scope(4), &query, Some(&next)).await.unwrap();
            paged.extend(rows(result.result));
            let Some(cursor) = result.next else {
                break;
            };
            next.after = Some(cursor);
        }
        assert_eq!(paged, whole);
        query.select = QuerySelect::Events;
        assert!(matches!(
            store.query(&scope(4), &query, Some(&next)).await,
            Err(AgentError::InvalidInput(_))
        ));
    }
}

#[tokio::test]
async fn property_query_wire_preserves_expressions_and_rejects_ambiguous_combinations() {
    let (_, timeline) = setup().await;
    let request = |query: &QuerySpec, page: Option<QueryPage>| dim_agent_runtime::TimelineQuery {
        scope: TimelineQueryScope::Branch,
        from: None,
        through: None,
        include_publishing: false,
        query: query.clone(),
        page,
    };
    timeline
        .append(fact(
            "item",
            json!({"key":1,"items":[{"key":"a"},null,{"key":"b"}]}),
        ))
        .unwrap();
    let mut query = fields(&["data.key"]);
    assert_eq!(
        serde_json::from_value::<QuerySpec>(serde_json::to_value(&query).unwrap()).unwrap(),
        query
    );
    assert!(timeline.query(request(&query, None)).await.is_err());
    query.group_by = vec!["data.key".into()];
    assert!(
        timeline
            .query(request(&query, Some(page(1))))
            .await
            .is_err()
    );
    query.group_by.clear();
    query.filter = Some(QueryFilter::Present {
        value: QueryOperand::Outer { path: "id".into() },
    });
    assert!(
        timeline
            .query(request(&query, Some(page(1))))
            .await
            .is_err()
    );
    query.filter = None;
    for path in [
        "data.items[-1]",
        "data.items[+1]",
        "data.items[01]",
        "data.items[ 0]",
        "data.items[2147483648]",
        "data.items[0][1]",
        "data.items[0",
        "data.items[0]x",
    ] {
        query.select = QuerySelect::Fields {
            fields: vec![path.into()],
        };
        assert!(
            timeline
                .query(request(&query, Some(page(1))))
                .await
                .is_err(),
            "invalid path: {path}"
        );
    }
    query.select = QuerySelect::Exists;
    assert!(
        timeline
            .query(request(&query, Some(page(1))))
            .await
            .is_err()
    );
    assert!(matches!(
        timeline.query(request(&query, None)).await.unwrap().result,
        QueryOutput::Exists { exists: true }
    ));
    let mut elements = fields(&["element.index", "element.value"]);
    elements.rows = Some(dim_agent_runtime::QueryRows::Elements {
        field: "data.items".into(),
    });
    let wire = serde_json::to_value(request(&elements, Some(page(1)))).unwrap();
    let decoded = serde_json::from_value(wire).unwrap();
    let first = timeline.query(decoded).await.unwrap();
    assert_eq!(rows(first.result)[0].values[0], value(json!(0)));
    let mut continuation = request(&elements, Some(page(1)));
    continuation.through = Some(first.through);
    continuation.page.as_mut().unwrap().after = first.next;
    let next = timeline.query(continuation).await.unwrap();
    assert_eq!(
        rows(next.result)[0].values,
        vec![value(json!(1)), value(Value::Null)]
    );
    let mut invalid = elements.clone();
    invalid.rows = None;
    assert!(matches!(
        timeline.query(request(&invalid, Some(page(1)))).await,
        Err(AgentError::InvalidInput(_))
    ));
    invalid = elements.clone();
    invalid.select = QuerySelect::Events;
    assert!(matches!(
        timeline.query(request(&invalid, Some(page(1)))).await,
        Err(AgentError::InvalidInput(_))
    ));
    invalid = elements.clone();
    invalid.rows = Some(dim_agent_runtime::QueryRows::Elements {
        field: "data.items[]".into(),
    });
    assert!(matches!(
        timeline.query(request(&invalid, Some(page(1)))).await,
        Err(AgentError::InvalidInput(_))
    ));
    for (field, partition_by) in [
        ("element.value", vec![]),
        ("data.key", vec!["element.index".into()]),
    ] {
        invalid = elements.clone();
        invalid.filter = Some(QueryFilter::TextCandidate {
            field: field.into(),
            partition_by,
            terms: vec!["value".into()],
        });
        assert!(matches!(
            timeline.query(request(&invalid, Some(page(1)))).await,
            Err(AgentError::InvalidInput(_))
        ));
    }
    invalid = elements;
    invalid.filter = Some(QueryFilter::Exists {
        condition: Box::new(eq("element.index", json!(0))),
    });
    assert!(matches!(
        timeline.query(request(&invalid, Some(page(1)))).await,
        Err(AgentError::InvalidInput(_))
    ));
}
