use super::*;
use serde::Serializer;

pub(super) const KEY: &str = "input-queue/node";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Data<R> {
    pub entry: Entry,
    pub left: Option<R>,
    pub right: Option<R>,
    pub height: u8,
    pub queued: usize,
    pub handovers: usize,
    pub first_held: Option<u64>,
}

#[derive(Debug)]
pub(super) struct Node {
    pub id: String,
    pub data: Data<Arc<Node>>,
    pub batch: Option<u64>,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.id)
    }
}

fn height(node: &Option<Arc<Node>>) -> u8 {
    node.as_ref().map_or(0, |node| node.data.height)
}

pub(super) fn queued(node: &Option<Arc<Node>>) -> usize {
    node.as_ref().map_or(0, |node| node.data.queued)
}

pub(super) fn payload(
    entry: Entry,
    left: Option<Arc<Node>>,
    right: Option<Arc<Node>>,
) -> Data<Arc<Node>> {
    let ordinary = matches!(entry.kind, EntryKind::Input { .. });
    let held = matches!(
        entry.kind,
        EntryKind::Input {
            mode: InputMode::Queue,
            released: false
        }
    );
    let first_held = [
        held.then_some(entry.sequence),
        left.as_ref().and_then(|node| node.data.first_held),
        right.as_ref().and_then(|node| node.data.first_held),
    ]
    .into_iter()
    .flatten()
    .min();
    Data {
        height: 1 + height(&left).max(height(&right)),
        queued: usize::from(ordinary) + queued(&left) + queued(&right),
        handovers: usize::from(!ordinary)
            + left.as_ref().map_or(0, |node| node.data.handovers)
            + right.as_ref().map_or(0, |node| node.data.handovers),
        first_held,
        entry,
        left,
        right,
    }
}

fn node(entry: Entry, left: Option<Arc<Node>>, right: Option<Arc<Node>>, batch: u64) -> Arc<Node> {
    Arc::new(Node {
        id: uuid::Uuid::new_v4().to_string(),
        data: payload(entry, left, right),
        batch: Some(batch),
    })
}

fn balanced(
    entry: Entry,
    left: Option<Arc<Node>>,
    right: Option<Arc<Node>>,
    batch: u64,
) -> Arc<Node> {
    if height(&left) > height(&right) + 1 {
        let root = left.as_ref().expect("left-heavy subtree");
        if height(&root.data.left) >= height(&root.data.right) {
            let right = node(entry, root.data.right.clone(), right, batch);
            return node(
                root.data.entry.clone(),
                root.data.left.clone(),
                Some(right),
                batch,
            );
        }
        let pivot = root.data.right.as_ref().expect("left-right rotation");
        let left = node(
            root.data.entry.clone(),
            root.data.left.clone(),
            pivot.data.left.clone(),
            batch,
        );
        let right = node(entry, pivot.data.right.clone(), right, batch);
        return node(pivot.data.entry.clone(), Some(left), Some(right), batch);
    }
    if height(&right) > height(&left) + 1 {
        let root = right.as_ref().expect("right-heavy subtree");
        if height(&root.data.right) >= height(&root.data.left) {
            let left = node(entry, left, root.data.left.clone(), batch);
            return node(
                root.data.entry.clone(),
                Some(left),
                root.data.right.clone(),
                batch,
            );
        }
        let pivot = root.data.left.as_ref().expect("right-left rotation");
        let left = node(entry, left, pivot.data.left.clone(), batch);
        let right = node(
            root.data.entry.clone(),
            pivot.data.right.clone(),
            root.data.right.clone(),
            batch,
        );
        return node(pivot.data.entry.clone(), Some(left), Some(right), batch);
    }
    node(entry, left, right, batch)
}

pub(super) fn get<'a>(mut root: &'a Option<Arc<Node>>, id: &str) -> Option<&'a Entry> {
    while let Some(node) = root {
        match id.cmp(node.data.entry.input_id.as_str()) {
            std::cmp::Ordering::Less => root = &node.data.left,
            std::cmp::Ordering::Greater => root = &node.data.right,
            std::cmp::Ordering::Equal => return Some(&node.data.entry),
        }
    }
    None
}

pub(super) fn set(root: Option<Arc<Node>>, entry: Entry, batch: u64) -> Option<Arc<Node>> {
    let Some(root) = root else {
        return Some(node(entry, None, None, batch));
    };
    Some(match entry.input_id.cmp(&root.data.entry.input_id) {
        std::cmp::Ordering::Less => balanced(
            root.data.entry.clone(),
            set(root.data.left.clone(), entry, batch),
            root.data.right.clone(),
            batch,
        ),
        std::cmp::Ordering::Greater => balanced(
            root.data.entry.clone(),
            root.data.left.clone(),
            set(root.data.right.clone(), entry, batch),
            batch,
        ),
        std::cmp::Ordering::Equal => node(
            entry,
            root.data.left.clone(),
            root.data.right.clone(),
            batch,
        ),
    })
}

pub(super) fn remove(root: Option<Arc<Node>>, id: &str, batch: u64) -> Option<Arc<Node>> {
    let root = root?;
    Some(match id.cmp(root.data.entry.input_id.as_str()) {
        std::cmp::Ordering::Less => balanced(
            root.data.entry.clone(),
            remove(root.data.left.clone(), id, batch),
            root.data.right.clone(),
            batch,
        ),
        std::cmp::Ordering::Greater => balanced(
            root.data.entry.clone(),
            root.data.left.clone(),
            remove(root.data.right.clone(), id, batch),
            batch,
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
                successor = left;
            }
            let entry = successor.data.entry.clone();
            let right = remove(root.data.right.clone(), &entry.input_id, batch);
            balanced(entry, root.data.left.clone(), right, batch)
        }
    })
}

pub(super) fn first_held(mut root: &Option<Arc<Node>>) -> Option<&Entry> {
    let sequence = root.as_ref()?.data.first_held?;
    loop {
        let node = root.as_ref().expect("held subtree");
        if node
            .data
            .left
            .as_ref()
            .and_then(|child| child.data.first_held)
            == Some(sequence)
        {
            root = &node.data.left;
        } else if node
            .data
            .right
            .as_ref()
            .and_then(|child| child.data.first_held)
            == Some(sequence)
        {
            root = &node.data.right;
        } else {
            return Some(&node.data.entry);
        }
    }
}

pub(super) fn handovers(root: &Option<Arc<Node>>, turn: &str, ids: &mut Vec<String>) {
    let Some(node) = root.as_ref().filter(|node| node.data.handovers != 0) else {
        return;
    };
    handovers(&node.data.left, turn, ids);
    if matches!(&node.data.entry.kind, EntryKind::Handover { turn_id } if turn_id == turn) {
        ids.push(node.data.entry.input_id.clone());
    }
    handovers(&node.data.right, turn, ids);
}

pub(super) fn drafts(
    root: &Option<Arc<Node>>,
    batch: u64,
    output: &mut Vec<EventDraft>,
) -> Result<()> {
    let Some(node) = root.as_ref().filter(|node| node.batch == Some(batch)) else {
        return Ok(());
    };
    drafts(&node.data.left, batch, output)?;
    drafts(&node.data.right, batch, output)?;
    let data = serde_json::to_value(&node.data)
        .map_err(|error| AgentError::fault("InputQueue", error.to_string()))?;
    let mut draft: EventDraft = EventKind::Extension(
        ExtensionEvent::new("input-queue", "node", data).expect("private queue node key"),
    )
    .into();
    draft.id = Some(node.id.clone());
    output.push(draft);
    Ok(())
}
