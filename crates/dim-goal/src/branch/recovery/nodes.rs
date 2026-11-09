use super::*;
use crate::driving::{self, Data, Node, Set};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

impl Owner {
    pub(super) async fn load_driving(
        &self,
        root: Option<String>,
        through: u64,
    ) -> Result<Set, AgentError> {
        let Some(root) = root else {
            return Ok(Set::default());
        };
        let mut seen = HashSet::from([root.clone()]);
        let mut pending = vec![(root.clone(), None::<String>, None::<String>)];
        let mut data = HashMap::<String, Data<String>>::new();
        let mut order = Vec::new();
        while !pending.is_empty() {
            let mut next = Vec::new();
            for batch in pending.chunks(128) {
                let mut query = request(
                    &self.branch_id,
                    None,
                    through,
                    &[driving::KEY],
                    &["id", "data"],
                );
                query.query.filter = Some(QueryFilter::And {
                    conditions: vec![
                        query.query.filter.take().unwrap(),
                        QueryFilter::In {
                            value: QueryOperand::Field { path: "id".into() },
                            values: batch.iter().map(|(id, _, _)| json!(id)).collect(),
                        },
                    ],
                });
                let QueryOutput::Rows { rows } = self.timeline.query(query).await?.result else {
                    unreachable!("Goal driving node fields")
                };
                if rows.len() != batch.len() {
                    return Err(AgentError::fault(
                        "Goal",
                        "a driving node is missing or ambiguous",
                    ));
                }
                let mut found = HashSet::new();
                for row in rows {
                    let [
                        QueryValue::Value {
                            value: Value::String(id),
                        },
                        QueryValue::Value { value },
                    ] = <[_; 2]>::try_from(row.values).expect("driving node fields")
                    else {
                        return Err(AgentError::fault("Goal", "invalid driving node properties"));
                    };
                    let node: Data<String> = serde_json::from_value(value)
                        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
                    let (_, low, high) = batch
                        .iter()
                        .find(|(wanted, _, _)| *wanted == id)
                        .expect("node identity filter");
                    if !found.insert(id.clone())
                        || low.as_ref().is_some_and(|low| node.key <= *low)
                        || high.as_ref().is_some_and(|high| node.key >= *high)
                    {
                        return Err(AgentError::fault("Goal", "invalid driving node order"));
                    }
                    for (child, low, high) in [
                        (&node.left, low.clone(), Some(node.key.clone())),
                        (&node.right, Some(node.key.clone()), high.clone()),
                    ] {
                        if let Some(child) = child {
                            if !seen.insert(child.clone()) {
                                return Err(AgentError::fault(
                                    "Goal",
                                    "driving nodes are shared or cyclic",
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
        let mut loaded = HashMap::<String, Arc<Node>>::new();
        for id in order.into_iter().rev() {
            let node = data.remove(&id).expect("selected driving node");
            let left = node
                .left
                .map(|id| loaded.remove(&id).expect("child loaded first"));
            let right = node
                .right
                .map(|id| loaded.remove(&id).expect("child loaded first"));
            let left_height = left.as_ref().map_or(0, |node| node.data.height);
            let right_height = right.as_ref().map_or(0, |node| node.data.height);
            let expected = driving::payload(node.key, left, right);
            if left_height.abs_diff(right_height) > 1 || node.height != expected.height {
                return Err(AgentError::fault("Goal", "invalid driving node height"));
            }
            loaded.insert(
                id.clone(),
                Arc::new(Node {
                    id,
                    data: expected,
                    pending: AtomicBool::new(false),
                }),
            );
        }
        Ok(Set(Some(
            loaded.remove(&root).expect("loaded driving root"),
        )))
    }
}
