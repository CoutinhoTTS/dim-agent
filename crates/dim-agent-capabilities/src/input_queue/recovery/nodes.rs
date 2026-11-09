use super::*;
use std::collections::HashMap;

impl InputQueue {
    pub(super) async fn load_entries(
        &self,
        root: Option<String>,
        through: u64,
        source_through: u64,
    ) -> Result<Option<Arc<entries::Node>>> {
        let Some(root) = root else { return Ok(None) };
        let mut seen = HashSet::from([root.clone()]);
        let mut positions = HashSet::new();
        let mut pending = vec![(root.clone(), None::<String>, None::<String>)];
        let mut data = HashMap::<String, entries::Data<String>>::new();
        let mut order = Vec::new();
        while !pending.is_empty() {
            let mut next = Vec::new();
            for batch in pending.chunks(128) {
                let query = TimelineQuery {
                    scope: TimelineQueryScope::Branch,
                    from: None,
                    through: Some(through),
                    include_publishing: true,
                    query: QuerySpec {
                        rows: None,
                        filter: Some(QueryFilter::And {
                            conditions: vec![
                                QueryFilter::In {
                                    value: QueryOperand::Field {
                                        path: "kind".into(),
                                    },
                                    values: vec![json!(entries::KEY)],
                                },
                                QueryFilter::In {
                                    value: QueryOperand::Field { path: "id".into() },
                                    values: batch.iter().map(|(id, _, _)| json!(id)).collect(),
                                },
                            ],
                        }),
                        select: QuerySelect::Fields {
                            fields: vec!["id".into(), "data".into()],
                        },
                        group_by: Vec::new(),
                        pick: None,
                        order_by: Vec::new(),
                    },
                    page: Some(QueryPage {
                        limit: NonZeroU64::new(128).unwrap(),
                        after: None,
                    }),
                };
                let QueryOutput::Rows { rows } = self.timeline.query(query).await?.result else {
                    unreachable!("queue nodes are property rows")
                };
                if rows.len() != batch.len() {
                    return Err(AgentError::fault(
                        "InputQueue",
                        "a queue node is missing or ambiguous",
                    ));
                }
                let mut found = HashSet::new();
                for row in rows {
                    let Ok(
                        [
                            QueryValue::Value {
                                value: Value::String(id),
                            },
                            QueryValue::Value { value },
                        ],
                    ) = <[_; 2]>::try_from(row.values)
                    else {
                        return Err(AgentError::fault(
                            "InputQueue",
                            "invalid queue node properties",
                        ));
                    };
                    let node: entries::Data<String> = serde_json::from_value(value)
                        .map_err(|error| AgentError::fault("InputQueue", error.to_string()))?;
                    let (_, low, high) = batch
                        .iter()
                        .find(|(wanted, _, _)| *wanted == id)
                        .expect("query restricts node identities");
                    if !found.insert(id.clone())
                        || low.as_ref().is_some_and(|low| node.entry.input_id <= *low)
                        || high
                            .as_ref()
                            .is_some_and(|high| node.entry.input_id >= *high)
                        || node.entry.sequence == 0
                        || node.entry.sequence > source_through
                        || !positions.insert(node.entry.sequence)
                    {
                        return Err(AgentError::fault("InputQueue", "invalid queue node order"));
                    }
                    for (child, low, high) in [
                        (&node.left, low.clone(), Some(node.entry.input_id.clone())),
                        (&node.right, Some(node.entry.input_id.clone()), high.clone()),
                    ] {
                        if let Some(child) = child {
                            if !seen.insert(child.clone()) {
                                return Err(AgentError::fault(
                                    "InputQueue",
                                    "queue nodes are shared or cyclic",
                                ));
                            }
                            next.push((child.clone(), low, high));
                        }
                    }
                    order.push(id.clone());
                    data.insert(id, node);
                }
            }
            pending = next;
        }
        let mut loaded = HashMap::<String, Arc<entries::Node>>::new();
        for id in order.into_iter().rev() {
            let node = data.remove(&id).expect("selected node data");
            let left = node
                .left
                .map(|id| loaded.remove(&id).expect("children are loaded first"));
            let right = node
                .right
                .map(|id| loaded.remove(&id).expect("children are loaded first"));
            let left_height = left.as_ref().map_or(0, |node| node.data.height);
            let right_height = right.as_ref().map_or(0, |node| node.data.height);
            let expected = entries::payload(node.entry, left, right);
            if left_height.abs_diff(right_height) > 1
                || node.height != expected.height
                || node.queued != expected.queued
                || node.handovers != expected.handovers
                || node.first_held != expected.first_held
            {
                return Err(AgentError::fault(
                    "InputQueue",
                    "invalid queue node summary",
                ));
            }
            loaded.insert(
                id.clone(),
                Arc::new(entries::Node {
                    id,
                    data: expected,
                    batch: None,
                }),
            );
        }
        Ok(Some(loaded.remove(&root).expect("the root is loaded")))
    }
}
