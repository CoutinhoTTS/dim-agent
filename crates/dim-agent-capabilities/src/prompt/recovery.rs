use super::*;
use dim_agent_runtime::QueryRows;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::num::NonZeroU64;

pub(super) const KEY: &str = "prompt-state/recovered";
pub(super) const SCHEMA: &str = "prompt-state/v1";
pub(super) const SOURCES: &[&str] = &[
    "prompt/changed",
    "context/configured",
    "turn/start",
    "turn/end",
    "branch/start",
    "branch/resume",
    "context/scope",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryReference {
    source: String,
    scope: PromptScope,
    event_id: String,
    element_index: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryState {
    entries: Vec<EntryReference>,
    turns: BTreeSet<String>,
    tasks: BTreeSet<PromptScope>,
    revision: Option<PromptReference>,
    system_revision: Option<PromptReference>,
    configuration_event: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovered {
    schema: String,
    source_through: u64,
    state: RecoveryState,
}

#[derive(Default)]
pub(super) struct Projection {
    pub(super) snapshot: PromptSnapshot,
    pub(super) configuration: Option<ContextConfiguredData>,
    configuration_event: Option<String>,
    entries: BTreeMap<Key, (String, u64)>,
    pub(super) source_through: u64,
    pub(super) written_result: u64,
    published_result: u64,
    pub(super) opening: Option<(u64, PromptSnapshot)>,
}

fn request(
    scope: TimelineQueryScope,
    through: u64,
    kinds: &[&str],
    fields: &[&str],
) -> TimelineQuery {
    TimelineQuery {
        scope,
        from: None,
        through: Some(through),
        include_publishing: false,
        query: QuerySpec {
            rows: None,
            filter: Some(QueryFilter::In {
                value: QueryOperand::Field {
                    path: "kind".into(),
                },
                values: kinds.iter().map(|kind| json!(kind)).collect(),
            }),
            select: QuerySelect::Fields {
                fields: fields.iter().map(|field| (*field).into()).collect(),
            },
            group_by: Vec::new(),
            pick: None,
            order_by: Vec::new(),
        },
        page: Some(QueryPage {
            limit: NonZeroU64::new(128).unwrap(),
            after: None,
        }),
    }
}

fn eq(field: &str, value: Value) -> QueryFilter {
    QueryFilter::Compare {
        left: QueryOperand::Field { path: field.into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value },
    }
}

async fn source_kind(
    owner: &Prompt,
    scope: TimelineQueryScope,
    through: u64,
    id: &str,
    key: &str,
) -> Result<EventKind> {
    let mut query = request(scope, through, &[key], &["data"]);
    query.query.filter = Some(QueryFilter::And {
        conditions: vec![query.query.filter.take().unwrap(), eq("id", json!(id))],
    });
    query.page.as_mut().unwrap().limit = NonZeroU64::MIN;
    let QueryOutput::Rows { mut rows } = owner.timeline.query(query).await?.result else {
        unreachable!("source data")
    };
    let row = rows
        .pop()
        .ok_or_else(|| AgentError::fault("Prompt", "missing source fact"))?;
    let [QueryValue::Value { value: data }] =
        <[_; 1]>::try_from(row.values).expect("source data field")
    else {
        unreachable!("source data value")
    };
    let (domain, name) = key.split_once('/').expect("source type key");
    serde_json::from_value(json!({"type":domain,"name":name,"data":data}))
        .map_err(|error| AgentError::fault("Prompt", error.to_string()))
}

impl Projection {
    fn apply(&mut self, id: &str, kind: &EventKind) {
        match kind {
            EventKind::PromptChanged(data) => {
                for (index, change) in data.changes.iter().enumerate() {
                    match change {
                        PromptChange::Set { entry } if self.snapshot.active(&entry.scope) => {
                            self.entries.insert(
                                (entry.source.clone(), entry.scope.clone()),
                                (id.to_owned(), index as u64),
                            );
                        }
                        PromptChange::Revoke { source, scope } => {
                            self.entries.remove(&(source.clone(), scope.clone()));
                        }
                        _ => {}
                    }
                }
            }
            EventKind::ContextConfigured(data) => {
                self.configuration = Some(data.clone());
                self.configuration_event = Some(id.to_owned());
            }
            _ => {}
        }
        self.snapshot.apply_kind(kind);
        self.entries
            .retain(|key, _| self.snapshot.entries.contains_key(key));
    }

    fn set_turn_active(&mut self, turn_id: &str, active: bool) {
        self.snapshot.set_turn_active(turn_id, active);
        self.entries
            .retain(|key, _| self.snapshot.entries.contains_key(key));
    }

    pub(super) fn checkpoint(&mut self, owner: &Prompt, blocks: Vec<InputBlock>) -> Result<()> {
        let recovered = Recovered {
            schema: SCHEMA.into(),
            source_through: self.source_through,
            state: RecoveryState {
                entries: self
                    .entries
                    .iter()
                    .map(
                        |((source, scope), (event_id, element_index))| EntryReference {
                            source: source.clone(),
                            scope: scope.clone(),
                            event_id: event_id.clone(),
                            element_index: *element_index,
                        },
                    )
                    .collect(),
                turns: self.snapshot.turns.clone(),
                tasks: self.snapshot.tasks.clone(),
                revision: self.snapshot.revision.clone(),
                system_revision: self.snapshot.system_revision.clone(),
                configuration_event: self.configuration_event.clone(),
            },
        };
        let mut drafts: Vec<EventDraft> = blocks
            .into_iter()
            .map(|block| {
                EventKind::InputMessage(InputMessageData {
                    blocks: vec![block],
                    mode: Some(InputMode::Append),
                    ..Default::default()
                })
                .into()
            })
            .collect();
        drafts.push(
            EventKind::Extension(
                ExtensionEvent::new(
                    "prompt-state",
                    "recovered",
                    serde_json::to_value(recovered)
                        .map_err(|error| AgentError::fault("Prompt", error.to_string()))?,
                )
                .expect("private recovery key"),
            )
            .into(),
        );
        let events = owner.timeline.append_batch_registered(drafts, |_| Ok(()))?;
        self.written_result = events.last().expect("recovery result").sequence;
        Ok(())
    }

    pub(super) fn publish(&mut self, owner: &Prompt, sequence: u64) {
        if sequence == self.written_result && sequence > self.published_result {
            owner.snapshot.set(self.snapshot.clone());
            owner.settle_consumed(self.source_through);
            self.published_result = sequence;
        }
    }

    pub(super) fn step(
        &mut self,
        owner: &Prompt,
        id: &str,
        sequence: u64,
        kind: &EventKind,
    ) -> Result<()> {
        if sequence <= self.source_through || !SOURCES.contains(&kind.key()) {
            return Ok(());
        }
        match kind {
            EventKind::TurnStart(data) => {
                return self.step_turn(owner, sequence, &data.turn_id, true);
            }
            EventKind::TurnEnd(data) => {
                return self.step_turn(owner, sequence, &data.turn_id, false);
            }
            _ => {}
        }
        let before = matches!(kind, EventKind::ContextScope(_) | EventKind::BranchStart(_))
            .then(|| self.snapshot.clone());
        self.apply(id, kind);
        self.source_through = sequence;
        if matches!(kind, EventKind::BranchStart(_) | EventKind::BranchResume(_)) {
            self.opening = Some((sequence, self.snapshot.clone()));
        }
        let blocks = before.map_or_else(Vec::new, |before| {
            context_changes(&before, &self.snapshot, true)
        });
        self.checkpoint(owner, blocks)
    }

    fn step_turn(
        &mut self,
        owner: &Prompt,
        sequence: u64,
        turn_id: &str,
        active: bool,
    ) -> Result<()> {
        if sequence <= self.source_through {
            return Ok(());
        }
        let before = (!active).then(|| self.snapshot.clone());
        self.set_turn_active(turn_id, active);
        self.source_through = sequence;
        let blocks = before.map_or_else(Vec::new, |before| {
            context_changes(&before, &self.snapshot, true)
        });
        self.checkpoint(owner, blocks)
    }

    pub(super) async fn restore(owner: &Prompt, through: u64) -> Result<(Self, bool)> {
        let mut latest = request(
            TimelineQueryScope::Branch,
            through,
            &["branch/start", "session/forked"],
            &["position"],
        );
        latest.query.pick = Some(QueryPick::Last);
        latest.page.as_mut().unwrap().limit = NonZeroU64::MIN;
        let QueryOutput::Rows { rows } = owner.timeline.query(latest.clone()).await?.result else {
            unreachable!("opening position")
        };
        latest.from = rows.into_iter().next().map(|row| {
            let [QueryValue::Value { value }] =
                <[_; 1]>::try_from(row.values).expect("opening field")
            else {
                unreachable!("opening value")
            };
            serde_json::from_value(value).expect("position contract")
        });
        latest.query.filter = Some(eq("kind", json!(KEY)));
        latest.query.select = QuerySelect::Fields {
            fields: vec!["position.sequence".into(), "data".into()],
        };
        let QueryOutput::Rows { rows } = owner.timeline.query(latest).await?.result else {
            unreachable!("recovery fields")
        };
        if let Some(row) = rows.into_iter().next() {
            let [
                QueryValue::Value { value: position },
                QueryValue::Value { value },
            ] = <[_; 2]>::try_from(row.values).expect("recovery fields")
            else {
                unreachable!("recovery value")
            };
            if value.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
                return Err(AgentError::fault("Prompt", "unsupported recovery schema"));
            }
            let recovered: Recovered = serde_json::from_value(value)
                .map_err(|error| AgentError::fault("Prompt", error.to_string()))?;
            let position = position.as_u64().expect("recovery position");
            if recovered.source_through > through || recovered.source_through >= position {
                return Err(AgentError::fault(
                    "Prompt",
                    "invalid recovery source position",
                ));
            }
            let mut state = Self {
                snapshot: PromptSnapshot {
                    turns: recovered.state.turns,
                    tasks: recovered.state.tasks,
                    revision: recovered.state.revision,
                    system_revision: recovered.state.system_revision,
                    ..Default::default()
                },
                configuration_event: recovered.state.configuration_event,
                source_through: recovered.source_through,
                written_result: position,
                published_result: position,
                ..Default::default()
            };
            if let Some(id) = &state.configuration_event {
                let mut query = request(
                    TimelineQueryScope::Chain,
                    position,
                    &["context/configured"],
                    &["data"],
                );
                query.query.filter = Some(QueryFilter::And {
                    conditions: vec![query.query.filter.take().unwrap(), eq("id", json!(id))],
                });
                let QueryOutput::Rows { mut rows } = owner.timeline.query(query).await?.result
                else {
                    unreachable!("configuration fields")
                };
                let row = rows
                    .pop()
                    .ok_or_else(|| AgentError::fault("Prompt", "missing adopted configuration"))?;
                let [QueryValue::Value { value }] =
                    <[_; 1]>::try_from(row.values).expect("configuration field")
                else {
                    unreachable!("configuration value")
                };
                let configuration: ContextConfiguredData = serde_json::from_value(value)
                    .map_err(|error| AgentError::fault("Prompt", error.to_string()))?;
                state.snapshot.configuration = Some((*configuration.settings).clone());
                state.configuration = Some(configuration);
            }
            for references in recovered.state.entries.chunks(128) {
                let mut query = request(
                    TimelineQueryScope::Chain,
                    position,
                    &["prompt/changed"],
                    &["id", "element.index", "element.value.entry"],
                );
                query.query.rows = Some(QueryRows::Elements {
                    field: "data.changes".into(),
                });
                let pairs = references
                    .iter()
                    .map(|entry| QueryFilter::And {
                        conditions: vec![
                            eq("id", json!(entry.event_id)),
                            eq("element.index", json!(entry.element_index)),
                        ],
                    })
                    .collect();
                query.query.filter = Some(QueryFilter::And {
                    conditions: vec![
                        query.query.filter.take().unwrap(),
                        QueryFilter::Or { conditions: pairs },
                    ],
                });
                let QueryOutput::Rows { rows } = owner.timeline.query(query).await?.result else {
                    unreachable!("entry fields")
                };
                if rows.len() != references.len() {
                    return Err(AgentError::fault(
                        "Prompt",
                        "missing effective prompt entry",
                    ));
                }
                for row in rows {
                    let [
                        QueryValue::Value { value: id },
                        QueryValue::Value { value: index },
                        QueryValue::Value { value },
                    ] = <[_; 3]>::try_from(row.values).expect("entry fields")
                    else {
                        return Err(AgentError::fault(
                            "Prompt",
                            "invalid effective prompt entry",
                        ));
                    };
                    let entry: PromptEntry = serde_json::from_value(value)
                        .map_err(|error| AgentError::fault("Prompt", error.to_string()))?;
                    let id = id.as_str().expect("entry identity");
                    let index = index.as_u64().expect("original index");
                    if !references.iter().any(|reference| {
                        reference.event_id == id
                            && reference.element_index == index
                            && reference.source == entry.source
                            && reference.scope == entry.scope
                    }) || !state.snapshot.active(&entry.scope)
                    {
                        return Err(AgentError::fault(
                            "Prompt",
                            "effective entry identity or scope mismatch",
                        ));
                    }
                    let key = (entry.source.clone(), entry.scope.clone());
                    if state
                        .entries
                        .insert(key.clone(), (id.to_owned(), index))
                        .is_some()
                    {
                        return Err(AgentError::fault("Prompt", "duplicate effective entry"));
                    }
                    state.snapshot.entries.insert(key, entry);
                }
            }
            state.snapshot.rebuild_system();
            return Ok((state, false));
        }
        let mut state = Self::default();
        let mut query = request(
            TimelineQueryScope::Chain,
            through,
            SOURCES,
            &["id", "kind", "data.turn_id"],
        );
        loop {
            let reply = owner.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("legacy source fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: id },
                    QueryValue::Value { value: kind },
                    turn_id,
                ] = <[_; 3]>::try_from(row.values).expect("legacy source fields")
                else {
                    unreachable!("legacy source values")
                };
                let key = kind.as_str().expect("source kind");
                if matches!(key, "turn/start" | "turn/end") {
                    let QueryValue::Value { value: turn_id } = turn_id else {
                        unreachable!("turn boundary has a turn id")
                    };
                    state.set_turn_active(
                        turn_id.as_str().expect("turn identity"),
                        key == "turn/start",
                    );
                    continue;
                }
                let kind = source_kind(
                    owner,
                    TimelineQueryScope::Chain,
                    through,
                    id.as_str().expect("source identity"),
                    key,
                )
                .await?;
                state.apply(id.as_str().expect("source identity"), &kind);
            }
            query.page.as_mut().unwrap().after = reply.next;
            if query.page.as_ref().unwrap().after.is_none() {
                break;
            }
        }
        state.source_through = through;
        Ok((state, true))
    }
}

impl Prompt {
    pub(super) async fn advance(&self, through: u64) -> Result<()> {
        if self.branch_phase.phase() != BranchPhase::Active {
            return self.recover_sources(through).await;
        }
        let mut progress = self.dispatcher.watch();
        while self.dispatcher.sequence() < through {
            if let Some(error) = self.dispatcher.failure() {
                return Err(error);
            }
            tokio::select! {
                changed = progress.changed() => changed.map_err(|_|AgentError::Closed("Prompt".into()))?,
                _ = self.lifecycle.cancelled() => return Err(AgentError::Closed("Prompt".into())),
                _ = self.branch_phase.closing() => return Err(AgentError::Cancelled("prompt adoption".into())),
            }
        }
        let written = self.projection.lock().unwrap().written_result;
        self.publisher.await_published(written).await?;
        self.projection.lock().unwrap().publish(self, written);
        Ok(())
    }

    pub(super) async fn recover_sources(&self, through: u64) -> Result<()> {
        let from = self
            .projection
            .lock()
            .unwrap()
            .source_through
            .saturating_add(1);
        if from <= through {
            let mut query = request(
                TimelineQueryScope::Branch,
                through,
                SOURCES,
                &["id", "position", "kind", "data.turn_id"],
            );
            query.from = Some(TimelinePosition {
                branch_id: self.timeline.branch_id(),
                sequence: from,
            });
            loop {
                let reply = self.timeline.query(query.clone()).await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("source fields")
                };
                for row in rows {
                    let [
                        QueryValue::Value { value: id },
                        QueryValue::Value { value: position },
                        QueryValue::Value { value: kind },
                        turn_id,
                    ] = <[_; 4]>::try_from(row.values).expect("source fields")
                    else {
                        unreachable!("source values")
                    };
                    let position: TimelinePosition = serde_json::from_value(position)
                        .map_err(|error| AgentError::fault("Prompt", error.to_string()))?;
                    if position.sequence <= self.projection.lock().unwrap().source_through {
                        continue;
                    }
                    let key = kind.as_str().expect("source kind");
                    if matches!(key, "turn/start" | "turn/end") {
                        let QueryValue::Value { value: turn_id } = turn_id else {
                            unreachable!("turn boundary has a turn id")
                        };
                        let mut state = self.projection.lock().unwrap();
                        state.step_turn(
                            self,
                            position.sequence,
                            turn_id.as_str().expect("turn identity"),
                            key == "turn/start",
                        )?;
                        continue;
                    }
                    let kind = source_kind(
                        self,
                        TimelineQueryScope::Branch,
                        through,
                        id.as_str().expect("source identity"),
                        key,
                    )
                    .await?;
                    self.projection.lock().unwrap().step(
                        self,
                        id.as_str().expect("source identity"),
                        position.sequence,
                        &kind,
                    )?;
                }
                query.page.as_mut().unwrap().after = reply.next;
                if query.page.as_ref().unwrap().after.is_none() {
                    break;
                }
            }
        }
        let written = self.projection.lock().unwrap().written_result;
        self.publisher.await_published(written).await?;
        self.projection.lock().unwrap().publish(self, written);
        Ok(())
    }
}
