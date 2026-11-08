use super::*;
use dim_agent_runtime::{
    QueryComparison, QueryFilter, QueryOperand, QueryOrder, QueryOrderKey, QueryOutput, QueryPage,
    QuerySelect, QuerySpec, TimelineQuery, TimelineQueryDirection,
};
use std::num::NonZeroU64;

fn query(
    scope: TimelineQueryScope,
    direction: TimelineQueryDirection,
    limit: u64,
) -> TimelineQuery {
    TimelineQuery {
        scope,
        from: None,
        through: None,
        include_publishing: false,
        query: QuerySpec {
            filter: None,
            select: QuerySelect::Events,
            group_by: vec![],
            pick: None,
            order_by: vec![QueryOrder {
                key: QueryOrderKey::Position,
                direction,
            }],
        },
        page: Some(QueryPage {
            limit: NonZeroU64::new(limit).unwrap(),
            after: None,
        }),
    }
}

#[tokio::test]
async fn position_pages_cross_batches_and_ancestor_cuts_in_both_directions() {
    use TimelineQueryDirection::{Ascending, Descending};
    use TimelineQueryScope::{Branch, Chain};
    let batched = batched_timeline().await;
    let (main, child) = chain_created_at(7, 5, 3).await;
    let (_, empty) = chain_with_a_tall_ancestor(5, 0).await;
    for (timeline, scope, expected) in [
        (
            &batched,
            Branch,
            (1..=7).map(|n| ("main", n)).collect::<Vec<_>>(),
        ),
        (&main, Branch, (1..=7).map(|n| ("main", n)).collect()),
        (&child, Branch, (1..=3).map(|n| ("child", n)).collect()),
        (
            &child,
            Chain,
            (1..=5)
                .map(|n| ("main", n))
                .chain((1..=3).map(|n| ("child", n)))
                .collect(),
        ),
        (&empty, Chain, (1..=5).map(|n| ("main", n)).collect()),
    ] {
        for direction in [Ascending, Descending] {
            for limit in [1, 2, 3, 16] {
                let mut request = query(scope, direction, limit);
                let mut actual = Vec::new();
                loop {
                    let reply = timeline.query(request.clone()).await.unwrap();
                    request.through = Some(reply.through);
                    let QueryOutput::Events { events } = reply.result else {
                        panic!("events")
                    };
                    assert!(events.len() <= limit as usize);
                    actual.extend(
                        events
                            .iter()
                            .map(|row| (row.position.branch_id.clone(), row.position.sequence)),
                    );
                    let Some(next) = reply.next else { break };
                    request.page.as_mut().unwrap().after = Some(next);
                }
                let mut expected: Vec<_> = expected
                    .iter()
                    .map(|(branch, n)| ((*branch).to_owned(), *n))
                    .collect();
                if direction == Descending {
                    expected.reverse();
                }
                assert_eq!(actual, expected, "{scope:?} {direction:?}, page {limit}");
            }
        }
    }
}

#[tokio::test]
async fn identity_lookup_supplies_an_inclusive_position_for_a_fixed_chain_read() {
    use TimelineQueryDirection::{Ascending, Descending};
    let (_, child) = chain_created_at(7, 5, 3).await;
    let mut lookup = query(TimelineQueryScope::Chain, Ascending, 1);
    lookup.query.select = QuerySelect::Fields {
        fields: vec!["position".into()],
    };
    lookup.query.filter = Some(QueryFilter::Compare {
        left: QueryOperand::Field { path: "id".into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value {
            value: "main-4".into(),
        },
    });
    let found = child.query(lookup.clone()).await.unwrap();
    let QueryOutput::Rows { rows } = found.result else {
        panic!("positions")
    };
    assert_eq!(rows.len(), 1);
    let dim_agent_runtime::QueryValue::Value { value } = &rows[0].values[0] else {
        panic!("position")
    };
    let position: TimelinePosition = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        position,
        TimelinePosition {
            branch_id: "main".into(),
            sequence: 4
        }
    );
    for (direction, expected) in [
        (
            Ascending,
            vec!["main-4", "main-5", "child-1", "child-2", "child-3"],
        ),
        (
            Descending,
            vec!["child-3", "child-2", "child-1", "main-5", "main-4"],
        ),
    ] {
        let mut request = query(TimelineQueryScope::Chain, direction, 16);
        request.from = Some(position.clone());
        request.through = Some(found.through);
        let reply = child.query(request).await.unwrap();
        let QueryOutput::Events { events } = reply.result else {
            panic!("events")
        };
        assert_eq!(
            events
                .iter()
                .map(|row| row.event.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    lookup.query.filter = Some(QueryFilter::Compare {
        left: QueryOperand::Field { path: "id".into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value {
            value: "main-6".into(),
        },
    });
    let QueryOutput::Rows { rows } = child.query(lookup).await.unwrap().result else {
        panic!("positions")
    };
    assert!(
        rows.is_empty(),
        "an identity beyond the ancestor cut is outside the read"
    );
}
