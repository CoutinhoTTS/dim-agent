use dim_agent_runtime::{
    QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick, QuerySelect, QuerySpec,
    QueryValue, Timeline, TimelineQuery, TimelineQueryScope,
};
use dim_agent_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Scope origins survive revocation; only the fixed branch ancestry establishes visibility.
pub(crate) fn scope_is_visible(scope: &PromptScope, branch: &str, ancestors: &[String]) -> bool {
    let (PromptScope::Branch { timeline }
    | PromptScope::Turn { timeline, .. }
    | PromptScope::Task { timeline, .. }) = scope;
    timeline == branch || ancestors.contains(timeline)
}

/// Rebuildable prompt state. Loading and publication belong to capabilities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptSnapshot {
    pub configuration: Option<ModelConfiguration>,
    pub entries: BTreeMap<(String, PromptScope), PromptEntry>,
    pub revision: Option<PromptReference>,
    pub system_revision: Option<PromptReference>,
    pub system: String,
    turns: BTreeSet<String>,
    tasks: BTreeSet<PromptScope>,
}

impl PromptSnapshot {
    pub fn from_checkpoint(state: &PromptCheckpointState) -> Self {
        Self {
            configuration: None,
            entries: state
                .entries
                .iter()
                .map(|entry| ((entry.source.clone(), entry.scope.clone()), entry.clone()))
                .collect(),
            revision: state.revision.clone(),
            system_revision: state.system_revision.clone(),
            system: state.system.clone(),
            turns: state.turns.iter().cloned().collect(),
            tasks: state.tasks.iter().cloned().collect(),
        }
    }

    pub fn checkpoint(&self) -> PromptCheckpointState {
        PromptCheckpointState {
            entries: self.entries.values().cloned().collect(),
            revision: self.revision.clone(),
            system_revision: self.system_revision.clone(),
            system: self.system.clone(),
            turns: self.turns.iter().cloned().collect(),
            tasks: self.tasks.iter().cloned().collect(),
        }
    }

    pub fn from_context(context: &dim_agent_runtime::ContextHistory) -> Self {
        let mut state = context
            .checkpoint
            .as_ref()
            .map(|checkpoint| Self::from_checkpoint(&checkpoint.state.prompts))
            .unwrap_or_default();
        state.configuration = context
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.state.configuration.as_ref())
            .map(|data| (*data.settings).clone());
        for event in &context.events {
            state.apply(event);
        }
        state
    }

    pub fn from_events(events: &[Arc<AgentEvent>]) -> Self {
        let mut state = Self::default();
        for event in events {
            state.apply(event);
        }
        state
    }

    pub fn active(&self, scope: &PromptScope) -> bool {
        match scope {
            PromptScope::Branch { .. } => true,
            PromptScope::Turn { id, .. } => self.turns.contains(id),
            PromptScope::Task { .. } => self.tasks.contains(scope),
        }
    }

    pub(crate) fn changed(&mut self, timeline: &str, revision: &str, changes: &[PromptChange]) {
        let revision = PromptReference {
            timeline: timeline.to_owned(),
            revision: revision.to_owned(),
        };
        let mut system_changed = false;
        for change in changes {
            let (source, scope) = match change {
                PromptChange::Set { entry } => (&entry.source, &entry.scope),
                PromptChange::Revoke { source, scope } => (source, scope),
            };
            let key = (source.clone(), scope.clone());
            system_changed |= self
                .entries
                .get(&key)
                .is_some_and(|entry| entry.delivery == PromptDelivery::System);
            match change {
                PromptChange::Set { entry } if self.active(scope) => {
                    system_changed |= entry.delivery == PromptDelivery::System;
                    self.entries.insert(key, entry.clone());
                }
                PromptChange::Set { .. } => {}
                PromptChange::Revoke { .. } => {
                    self.entries.remove(&key);
                }
            }
        }
        self.revision = Some(revision.clone());
        if system_changed {
            self.system_revision = Some(revision);
            let mut entries: Vec<_> = self
                .entries
                .values()
                .filter(|entry| entry.delivery == PromptDelivery::System)
                .collect();
            entries.sort_by(|a, b| {
                a.order
                    .cmp(&b.order)
                    .then(a.source.cmp(&b.source))
                    .then(a.scope.cmp(&b.scope))
            });
            self.system = entries
                .into_iter()
                .map(|entry| entry.body.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
        }
    }

    pub(crate) async fn read(
        timeline: &Timeline,
        through: u64,
    ) -> dim_agent_runtime::Result<(Self, Option<ContextConfiguredData>)> {
        let mut query = TimelineQuery {
            scope: TimelineQueryScope::Chain,
            from: None,
            through: Some(through),
            include_publishing: false,
            query: QuerySpec {
                rows: None,
                filter: Some(QueryFilter::In {
                    value: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    values: vec!["context/configured".into()],
                }),
                select: QuerySelect::Fields {
                    fields: vec!["data".into()],
                },
                group_by: vec![],
                pick: Some(QueryPick::Last),
                order_by: vec![],
            },
            page: Some(QueryPage {
                limit: std::num::NonZeroU64::MIN,
                after: None,
            }),
        };
        let reply = timeline.query(query.clone()).await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("configuration fields")
        };
        let configuration: Option<ContextConfiguredData> = rows.into_iter().next().map(|row| {
            let [QueryValue::Value { value }] =
                <[_; 1]>::try_from(row.values).expect("one configuration field")
            else {
                unreachable!("configuration has data")
            };
            serde_json::from_value(value).expect("configuration follows its event contract")
        });
        let mut snapshot = Self {
            configuration: configuration.as_ref().map(|data| (*data.settings).clone()),
            ..Self::default()
        };
        query.query.filter = Some(QueryFilter::In {
            value: QueryOperand::Field {
                path: "kind".into(),
            },
            values: [
                "prompt/changed",
                "turn/start",
                "turn/end",
                "branch/start",
                "context/scope",
            ]
            .map(Into::into)
            .to_vec(),
        });
        query.query.select = QuerySelect::Fields {
            fields: [
                "kind",
                "data.timeline",
                "data.revision",
                "data.changes",
                "data.turn_id",
                "data.scope",
                "data.status",
            ]
            .map(String::from)
            .to_vec(),
        };
        query.query.pick = None;
        query.page.as_mut().unwrap().limit = std::num::NonZeroU64::new(256).unwrap();
        loop {
            let reply = timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("prompt state fields")
            };
            for row in rows {
                let [
                    QueryValue::Value {
                        value: serde_json::Value::String(kind),
                    },
                    timeline,
                    revision,
                    changes,
                    turn,
                    scope,
                    status,
                ] = <[_; 7]>::try_from(row.values).expect("prompt state fields")
                else {
                    unreachable!("a prompt fact has a kind")
                };
                match kind.as_str() {
                    "prompt/changed" => {
                        let (
                            QueryValue::Value {
                                value: serde_json::Value::String(timeline),
                            },
                            QueryValue::Value {
                                value: serde_json::Value::String(revision),
                            },
                            QueryValue::Value { value: changes },
                        ) = (timeline, revision, changes)
                        else {
                            unreachable!("a prompt change has identity and changes")
                        };
                        let changes: Vec<PromptChange> = serde_json::from_value(changes)
                            .expect("prompt changes follow their event contract");
                        snapshot.changed(&timeline, &revision, &changes);
                    }
                    "turn/start" | "turn/end" => {
                        let QueryValue::Value {
                            value: serde_json::Value::String(turn),
                        } = turn
                        else {
                            unreachable!("a turn has an identity")
                        };
                        if kind == "turn/start" {
                            snapshot.turns.insert(turn);
                        } else {
                            snapshot.turns.remove(&turn);
                        }
                    }
                    "branch/start" => snapshot.turns.clear(),
                    "context/scope" => {
                        let (
                            QueryValue::Value { value: scope },
                            QueryValue::Value { value: status },
                        ) = (scope, status)
                        else {
                            unreachable!("scope event fields")
                        };
                        let scope: PromptScope = serde_json::from_value(scope)
                            .expect("prompt scope follows its event contract");
                        match serde_json::from_value::<ContextScopeStatus>(status)
                            .expect("scope status follows its event contract")
                        {
                            ContextScopeStatus::Started => {
                                snapshot.tasks.insert(scope);
                            }
                            ContextScopeStatus::Ended => {
                                snapshot.tasks.remove(&scope);
                            }
                        }
                    }
                    _ => unreachable!("selected prompt state kind"),
                }
                snapshot.retain_active();
            }
            query.page.as_mut().unwrap().after = reply.next;
            if query.page.as_ref().unwrap().after.is_none() {
                return Ok((snapshot, configuration));
            }
        }
    }

    pub fn apply(&mut self, event: &AgentEvent) {
        match &event.kind {
            EventKind::ContextConfigured(data) => {
                self.configuration = Some((*data.settings).clone());
            }
            EventKind::PromptChanged(data) => {
                self.changed(&data.timeline, &data.revision, &data.changes)
            }
            EventKind::TurnStart(data) => {
                self.turns.insert(data.turn_id.clone());
            }
            EventKind::TurnEnd(data) => {
                self.turns.remove(&data.turn_id);
            }
            EventKind::BranchStart(_) => self.turns.clear(),
            EventKind::ContextScope(data) => match data.status {
                ContextScopeStatus::Started => {
                    self.tasks.insert(data.scope.clone());
                }
                ContextScopeStatus::Ended => {
                    self.tasks.remove(&data.scope);
                }
            },
            _ => return,
        }
        self.retain_active();
    }

    fn retain_active(&mut self) {
        let turns = &self.turns;
        let tasks = &self.tasks;
        self.entries.retain(|(_, scope), _| match scope {
            PromptScope::Branch { .. } => true,
            PromptScope::Turn { id, .. } => turns.contains(id),
            PromptScope::Task { .. } => tasks.contains(scope),
        });
    }
}
