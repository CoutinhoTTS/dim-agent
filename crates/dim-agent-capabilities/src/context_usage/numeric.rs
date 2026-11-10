use super::{Calibration, Compression, Configuration, Contribution, Key, LastEntry, add, fault};
use dim_agent_runtime::{BoxFuture, QueryOutput, QuerySelect, QueryValue, Result, Timeline};
use dim_agent_types::{EventDraft, EventKind, ExtensionEvent};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

const LEAF: usize = 64;
const FANOUT: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Summary {
    pub ordinary: Contribution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<Configuration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<Compression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Calibration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_entry: Option<LastEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth: Option<f64>,
}

impl Summary {
    pub fn join(&mut self, next: &Self) -> Result<()> {
        self.growth = match (self.growth, next.growth) {
            (Some(left), Some(right)) => Some(left + right),
            (Some(left), None) if next.ordinary == Contribution::default() => Some(left),
            (None, Some(right)) if self.ordinary == Contribution::default() => Some(right),
            _ => None,
        };
        if self
            .growth
            .is_some_and(|growth| !growth.is_finite() || growth < 0.)
        {
            return Err(fault("invalid numeric history growth"));
        }
        self.ordinary = add(self.ordinary, next.ordinary)?;
        if next.configuration.is_some() {
            self.configuration.clone_from(&next.configuration);
        }
        if next.compression.is_some() {
            self.compression.clone_from(&next.compression);
        }
        if next.calibration.is_some() {
            self.calibration.clone_from(&next.calibration);
        }
        if next.local.is_some() {
            self.local = next.local;
        }
        if next.last_entry.is_some() {
            self.last_entry.clone_from(&next.last_entry);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub at: Key,
    pub summary: Summary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub id: String,
    pub first: Key,
    pub last: Key,
    pub height: u8,
    pub summary: Summary,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Node {
    Leaf { entries: Vec<Entry> },
    Branch { children: Vec<Reference> },
}

impl Node {
    fn describe(&self, id: String) -> Result<Reference> {
        let mut summary = Summary::default();
        let (first, last, height) = match self {
            Self::Leaf { entries } => {
                if entries.is_empty() || entries.len() > LEAF {
                    return Err(fault("invalid numeric leaf size"));
                }
                for pair in entries.windows(2) {
                    if pair[0].at >= pair[1].at {
                        return Err(fault("numeric leaf positions are not ordered"));
                    }
                }
                for entry in entries {
                    if entry.summary.last_entry.as_ref().is_some_and(|last| {
                        last.at > entry.at || last.id.as_ref().is_some_and(String::is_empty)
                    }) {
                        return Err(fault("invalid numeric history boundary"));
                    }
                    summary.join(&entry.summary)?;
                }
                (entries[0].at, entries[entries.len() - 1].at, 0)
            }
            Self::Branch { children } => {
                if children.is_empty() || children.len() > FANOUT {
                    return Err(fault("invalid numeric branch size"));
                }
                let height = children[0]
                    .height
                    .checked_add(1)
                    .filter(|height| *height <= 32)
                    .ok_or_else(|| fault("invalid numeric tree height"))?;
                for child in children {
                    if child.id.is_empty() || child.first > child.last || child.height != height - 1
                    {
                        return Err(fault("invalid numeric child reference"));
                    }
                    summary.join(&child.summary)?;
                }
                for pair in children.windows(2) {
                    if pair[0].last >= pair[1].first {
                        return Err(fault("numeric child ranges overlap"));
                    }
                }
                (children[0].first, children[children.len() - 1].last, height)
            }
        };
        Ok(Reference {
            id,
            first,
            last,
            height,
            summary,
        })
    }
}

pub(super) struct Numeric {
    timeline: Arc<Timeline>,
    through: u64,
    pending: HashMap<String, Node>,
}

impl Numeric {
    pub fn new(timeline: Arc<Timeline>, through: u64) -> Self {
        Self {
            timeline,
            through,
            pending: HashMap::new(),
        }
    }

    pub fn published(&mut self, through: u64) {
        self.through = through;
    }

    pub async fn validate_root(&self, root: Option<&Reference>) -> Result<()> {
        if let Some(root) = root {
            self.load(root).await?;
        }
        Ok(())
    }

    async fn load(&self, reference: &Reference) -> Result<Node> {
        if let Some(node) = self.pending.get(&reference.id) {
            return Ok(node.clone());
        }
        let node: Node = {
            let reply = self
                .timeline
                .query(crate::history::chain_by_id(
                    &reference.id,
                    self.through,
                    QuerySelect::Fields {
                        fields: vec!["kind".into(), "data".into()],
                    },
                ))
                .await?;
            let QueryOutput::Rows { mut rows } = reply.result else {
                unreachable!("numeric node fields")
            };
            let row = rows.pop().ok_or_else(|| fault("missing numeric node"))?;
            let [
                QueryValue::Value { value: kind },
                QueryValue::Value { value: data },
            ] = <[_; 2]>::try_from(row.values).expect("numeric node field count")
            else {
                return Err(fault("invalid numeric node fields"));
            };
            if kind.as_str() != Some("context-usage/node") {
                return Err(fault("numeric reference names another event type"));
            }
            serde_json::from_value(data).map_err(|error| fault(error.to_string()))?
        };
        if node.describe(reference.id.clone())? != *reference {
            return Err(fault("numeric node disagrees with its reference"));
        }
        Ok(node)
    }

    fn save(&mut self, node: Node) -> Result<Reference> {
        let id = Uuid::new_v4().to_string();
        let reference = node.describe(id.clone())?;
        self.pending.insert(id, node);
        Ok(reference)
    }

    pub async fn append(&mut self, root: Option<Reference>, entry: Entry) -> Result<Reference> {
        let Some(root) = root else {
            return self.save(Node::Leaf {
                entries: vec![entry],
            });
        };
        if entry.at < root.last {
            return Err(fault("numeric input precedes the saved prefix"));
        }
        let mut roots = self.push(root, entry).await?;
        if roots.len() == 1 {
            Ok(roots.pop().expect("one updated root"))
        } else {
            self.save(Node::Branch { children: roots })
        }
    }

    fn push(&mut self, root: Reference, entry: Entry) -> BoxFuture<'_, Result<Vec<Reference>>> {
        Box::pin(async move {
            match self.load(&root).await? {
                Node::Leaf { mut entries } => {
                    if root.last == entry.at {
                        entries
                            .last_mut()
                            .expect("nonempty leaf")
                            .summary
                            .join(&entry.summary)?;
                    } else if entries.len() == LEAF {
                        let next = self.save(Node::Leaf {
                            entries: vec![entry],
                        })?;
                        return Ok(vec![root, next]);
                    } else {
                        entries.push(entry);
                    }
                    self.pending.remove(&root.id);
                    Ok(vec![self.save(Node::Leaf { entries })?])
                }
                Node::Branch { mut children } => {
                    let last = children.pop().expect("nonempty branch");
                    children.extend(self.push(last, entry).await?);
                    self.pending.remove(&root.id);
                    if children.len() > FANOUT {
                        let tail = children.split_off(FANOUT);
                        Ok(vec![
                            self.save(Node::Branch { children })?,
                            self.save(Node::Branch { children: tail })?,
                        ])
                    } else {
                        Ok(vec![self.save(Node::Branch { children })?])
                    }
                }
            }
        })
    }

    pub fn prefix<'a>(
        &'a self,
        root: Option<&'a Reference>,
        end: Key,
        inclusive: bool,
    ) -> BoxFuture<'a, Result<Summary>> {
        Box::pin(async move {
            let Some(root) = root else {
                return Ok(Summary::default());
            };
            if root.first > end || (!inclusive && root.first == end) {
                return Ok(Summary::default());
            }
            if root.last < end || (inclusive && root.last == end) {
                return Ok(root.summary.clone());
            }
            let mut summary = Summary::default();
            match self.load(root).await? {
                Node::Leaf { entries } => {
                    for entry in entries {
                        if entry.at > end || (!inclusive && entry.at == end) {
                            break;
                        }
                        summary.join(&entry.summary)?;
                    }
                }
                Node::Branch { children } => {
                    for child in children {
                        if child.first > end || (!inclusive && child.first == end) {
                            break;
                        }
                        summary.join(&self.prefix(Some(&child), end, inclusive).await?)?;
                    }
                }
            }
            Ok(summary)
        })
    }

    pub fn growth<'a>(
        &'a self,
        root: Option<&'a Reference>,
        after: Key,
        through: Key,
    ) -> BoxFuture<'a, Result<Option<f64>>> {
        Box::pin(async move {
            let Some(root) = root else {
                return Ok(Some(0.));
            };
            if root.last <= after || root.first > through {
                return Ok(Some(0.));
            }
            if root.first > after && root.last <= through {
                return Ok(root
                    .summary
                    .growth
                    .or_else(|| (root.summary.ordinary == Contribution::default()).then_some(0.)));
            }
            let mut growth = 0.;
            match self.load(root).await? {
                Node::Leaf { entries } => {
                    for entry in entries {
                        if entry.at > after && entry.at <= through {
                            let Some(tokens) = entry.summary.growth.or_else(|| {
                                (entry.summary.ordinary == Contribution::default()).then_some(0.)
                            }) else {
                                return Ok(None);
                            };
                            growth += tokens;
                        }
                    }
                }
                Node::Branch { children } => {
                    for child in &children {
                        let Some(tokens) = self.growth(Some(child), after, through).await? else {
                            return Ok(None);
                        };
                        growth += tokens;
                    }
                }
            }
            Ok(Some(growth))
        })
    }

    pub fn drafts(&mut self, root: Option<&Reference>) -> Result<Vec<EventDraft>> {
        let mut pending = std::mem::take(&mut self.pending);
        let mut selected = Vec::new();
        let mut queue: Vec<_> = root.into_iter().cloned().collect();
        while let Some(reference) = queue.pop() {
            let Some(node) = pending.remove(&reference.id) else {
                continue;
            };
            let data = serde_json::to_value(&node).map_err(|error| fault(error.to_string()))?;
            if let Node::Branch { children } = node {
                queue.extend(children);
            }
            selected.push(
                EventDraft::new(EventKind::Extension(
                    ExtensionEvent::new("context-usage", "node", data).expect("numeric node key"),
                ))
                .with_id(reference.id),
            );
        }
        Ok(selected)
    }
}
