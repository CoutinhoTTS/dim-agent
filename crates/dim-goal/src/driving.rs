use dim_agent_runtime::AgentError;
use dim_agent_types::{EventDraft, EventKind, ExtensionEvent};
use serde::{Deserialize, Serialize, Serializer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) const KEY: &str = "goal/driving-node";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(bound(deserialize = "R: Deserialize<'de>"))]
pub(crate) struct Data<R> {
    pub(crate) key: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) left: Option<R>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) right: Option<R>,
    pub(crate) height: u8,
}

pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) data: Data<Arc<Node>>,
    pub(crate) pending: AtomicBool,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.id)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Set(pub(crate) Option<Arc<Node>>);

impl Set {
    pub(crate) fn contains(&self, id: &str) -> bool {
        let mut root = &self.0;
        while let Some(node) = root {
            match id.cmp(&node.data.key) {
                std::cmp::Ordering::Less => root = &node.data.left,
                std::cmp::Ordering::Greater => root = &node.data.right,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(crate) fn insert(&mut self, id: String) {
        if !self.contains(&id) {
            self.0 = Some(insert(self.0.take(), id));
        }
    }

    pub(crate) fn remove(&mut self, id: &str) {
        if self.contains(id) {
            self.0 = remove(self.0.take(), id);
        }
    }

    pub(crate) fn root(&self) -> Option<&str> {
        self.0.as_ref().map(|node| node.id.as_str())
    }

    pub(crate) fn pending(&self) -> Vec<Arc<Node>> {
        let mut pending = Vec::new();
        let mut stack: Vec<_> = self.0.iter().cloned().collect();
        while let Some(node) = stack.pop() {
            if !node.pending.load(Ordering::Relaxed) {
                continue;
            }
            stack.extend(node.data.left.iter().chain(&node.data.right).cloned());
            pending.push(node);
        }
        pending
    }
}

pub(crate) fn drafts(nodes: &[Arc<Node>]) -> Result<Vec<EventDraft>, AgentError> {
    nodes
        .iter()
        .map(|node| {
            let data = serde_json::to_value(&node.data)
                .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
            let mut draft: EventDraft = EventKind::Extension(
                ExtensionEvent::new("goal", "driving-node", data).expect("Goal node key"),
            )
            .into();
            draft.id = Some(node.id.clone());
            Ok(draft)
        })
        .collect()
}

pub(crate) fn payload(
    key: String,
    left: Option<Arc<Node>>,
    right: Option<Arc<Node>>,
) -> Data<Arc<Node>> {
    Data {
        key,
        height: 1 + height(&left).max(height(&right)),
        left,
        right,
    }
}

fn height(node: &Option<Arc<Node>>) -> u8 {
    node.as_ref().map_or(0, |node| node.data.height)
}

fn node(key: String, left: Option<Arc<Node>>, right: Option<Arc<Node>>) -> Arc<Node> {
    Arc::new(Node {
        id: uuid::Uuid::new_v4().to_string(),
        data: payload(key, left, right),
        pending: AtomicBool::new(true),
    })
}

fn balance(key: String, left: Option<Arc<Node>>, right: Option<Arc<Node>>) -> Arc<Node> {
    if height(&left) > height(&right) + 1 {
        let root = left.as_ref().expect("left-heavy subtree");
        if height(&root.data.left) >= height(&root.data.right) {
            let right = node(key, root.data.right.clone(), right);
            return node(root.data.key.clone(), root.data.left.clone(), Some(right));
        }
        let pivot = root.data.right.as_ref().expect("left-right rotation");
        let left = node(
            root.data.key.clone(),
            root.data.left.clone(),
            pivot.data.left.clone(),
        );
        let right = node(key, pivot.data.right.clone(), right);
        return node(pivot.data.key.clone(), Some(left), Some(right));
    }
    if height(&right) > height(&left) + 1 {
        let root = right.as_ref().expect("right-heavy subtree");
        if height(&root.data.right) >= height(&root.data.left) {
            let left = node(key, left, root.data.left.clone());
            return node(root.data.key.clone(), Some(left), root.data.right.clone());
        }
        let pivot = root.data.left.as_ref().expect("right-left rotation");
        let left = node(key, left, pivot.data.left.clone());
        let right = node(
            root.data.key.clone(),
            pivot.data.right.clone(),
            root.data.right.clone(),
        );
        return node(pivot.data.key.clone(), Some(left), Some(right));
    }
    node(key, left, right)
}

fn insert(root: Option<Arc<Node>>, id: String) -> Arc<Node> {
    let Some(root) = root else {
        return node(id, None, None);
    };
    match id.cmp(&root.data.key) {
        std::cmp::Ordering::Less => balance(
            root.data.key.clone(),
            Some(insert(root.data.left.clone(), id)),
            root.data.right.clone(),
        ),
        std::cmp::Ordering::Greater => balance(
            root.data.key.clone(),
            root.data.left.clone(),
            Some(insert(root.data.right.clone(), id)),
        ),
        std::cmp::Ordering::Equal => root,
    }
}

fn remove(root: Option<Arc<Node>>, id: &str) -> Option<Arc<Node>> {
    let root = root?;
    Some(match id.cmp(&root.data.key) {
        std::cmp::Ordering::Less => balance(
            root.data.key.clone(),
            remove(root.data.left.clone(), id),
            root.data.right.clone(),
        ),
        std::cmp::Ordering::Greater => balance(
            root.data.key.clone(),
            root.data.left.clone(),
            remove(root.data.right.clone(), id),
        ),
        std::cmp::Ordering::Equal => {
            if root.data.left.is_none() {
                return root.data.right.clone();
            }
            if root.data.right.is_none() {
                return root.data.left.clone();
            }
            let mut successor = root.data.right.as_ref().expect("right subtree");
            while let Some(left) = &successor.data.left {
                successor = left
            }
            let id = successor.data.key.clone();
            let right = remove(root.data.right.clone(), &id);
            balance(id, root.data.left.clone(), right)
        }
    })
}
