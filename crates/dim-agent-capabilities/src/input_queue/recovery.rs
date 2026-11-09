mod nodes;

use super::*;
use dim_agent_runtime::{
    QueryComparison, QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick, QuerySelect,
    QuerySpec, QueryValue, TimelineQuery, TimelineQueryScope,
};
use serde_json::{Value, json};
use std::{collections::HashSet, num::NonZeroU64};

pub(super) const KEY: &str = "input-queue/recovered";
const SCHEMA: &str = "input-queue/v1";
const SOURCES: &[&str] = &[
    "input/message",
    "input/consumed",
    "input/withdraw",
    "turn/start",
    "turn/end",
    "stop/interrupt",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recovered<R = Arc<entries::Node>> {
    schema: String,
    pub(super) source_through: u64,
    pub(super) state: QueueState<R>,
}

impl<R> Default for Recovered<R> {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            source_through: 0,
            state: QueueState::default(),
        }
    }
}

impl Recovered<String> {
    pub(super) fn decode(value: Value) -> Result<Self> {
        if value.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(AgentError::fault(
                "InputQueue",
                "unsupported queue recovery schema",
            ));
        }
        serde_json::from_value(value)
            .map_err(|error| AgentError::fault("InputQueue", error.to_string()))
    }
}

impl InputQueue {
    async fn remove_written(&self, owed: &mut Vec<Decision>, through: u64) -> Result<()> {
        let mut pending = Vec::with_capacity(owed.len());
        for decision in owed.drain(..) {
            let kinds = match &decision {
                Decision::Apply(..) => vec![json!("input/applied"), json!("input/deleted")],
                Decision::Delete(..) => vec![json!("input/deleted")],
            };
            let reply = self
                .timeline
                .query(TimelineQuery {
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
                                    values: kinds,
                                },
                                QueryFilter::Compare {
                                    left: QueryOperand::Field {
                                        path: "data.input_id".into(),
                                    },
                                    comparison: QueryComparison::Eq,
                                    right: QueryOperand::Value {
                                        value: json!(decision.input_id()),
                                    },
                                },
                            ],
                        }),
                        select: QuerySelect::Exists,
                        group_by: Vec::new(),
                        pick: None,
                        order_by: Vec::new(),
                    },
                    page: None,
                })
                .await?;
            let QueryOutput::Exists { exists } = reply.result else {
                unreachable!("output deduplication tests existence")
            };
            if !exists {
                pending.push(decision);
            }
        }
        *owed = pending;
        Ok(())
    }

    pub(super) fn checkpoint(
        &self,
        recovered: &Recovered,
        decisions: Vec<Decision>,
    ) -> Result<(u64, Option<u64>)> {
        let mut deleted = None;
        let mut drafts: Vec<EventDraft> = decisions
            .into_iter()
            .enumerate()
            .map(|(index, decision)| {
                match decision {
                    Decision::Apply(input_id, mode) => EventKind::InputApplied(InputAppliedData {
                        input_id,
                        mode: Some(mode),
                    }),
                    Decision::Delete(input_id) => {
                        deleted = Some(index);
                        EventKind::InputDeleted(InputDeletedData { input_id })
                    }
                }
                .into()
            })
            .collect();
        entries::drafts(
            &recovered.state.entries_root,
            recovered.source_through,
            &mut drafts,
        )?;
        let data = serde_json::to_value(recovered)
            .map_err(|error| AgentError::fault("InputQueue", error.to_string()))?;
        drafts.push(
            EventKind::Extension(
                ExtensionEvent::new("input-queue", "recovered", data)
                    .expect("the recovery event has a private extension key"),
            )
            .into(),
        );
        let events = self.timeline.append_batch_registered(drafts, |_| Ok(()))?;
        Ok((
            events
                .last()
                .expect("a recovery batch has a result")
                .sequence,
            deleted.map(|index| events[index].sequence),
        ))
    }

    async fn await_publication(&self, sequence: u64) -> Result<()> {
        let mut publication = self.timeline.watch();
        while publication.borrow().0 < sequence {
            if let Some(error) = self.timeline.failure() {
                return Err(error);
            }
            publication
                .changed()
                .await
                .map_err(|_| AgentError::Closed(self.timeline.branch_id()))?;
        }
        Ok(())
    }

    pub(super) async fn rebuild(&self) -> Result<u64> {
        let through = self.timeline.committed();
        if through == 0 {
            return Ok(0);
        }
        let request = |kinds: &[&str], fields: &[&str], pick| TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: None,
            through: Some(through),
            include_publishing: true,
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
                pick,
                order_by: Vec::new(),
            },
            page: Some(QueryPage {
                limit: NonZeroU64::new(if pick.is_some() { 1 } else { 128 }).unwrap(),
                after: None,
            }),
        };
        let QueryOutput::Rows { rows } = self
            .timeline
            .query(request(
                &["branch/start", "session/forked"],
                &["position.sequence"],
                Some(QueryPick::Last),
            ))
            .await?
            .result
        else {
            unreachable!("opening selection returns positions")
        };
        let opening = rows.first().map_or(0, |row| match row.values.as_slice() {
            [QueryValue::Value { value }] => value.as_u64().expect("an opening has a position"),
            _ => unreachable!("an opening has a position"),
        });
        let mut latest = request(
            &[KEY],
            &["position.sequence", "data"],
            Some(QueryPick::Last),
        );
        latest.from = (opening > 0).then(|| TimelinePosition {
            branch_id: self.timeline.branch_id(),
            sequence: opening,
        });
        let QueryOutput::Rows { mut rows } = self.timeline.query(latest).await?.result else {
            unreachable!("queue recovery selects result fields")
        };
        let saved = rows.pop();
        let initialize = saved.is_none();
        let mut published_result = 0;
        let mut recovered = if let Some(row) = saved {
            let Ok(
                [
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: data },
                ],
            ) = <[_; 2]>::try_from(row.values)
            else {
                return Err(AgentError::fault("InputQueue", "invalid recovery result"));
            };
            let sequence = sequence.as_u64().expect("a result has a position");
            let saved = Recovered::<String>::decode(data)?;
            if saved.source_through >= sequence || saved.source_through > through {
                return Err(AgentError::fault("InputQueue", "invalid recovery position"));
            }
            self.await_publication(sequence).await?;
            published_result = sequence;
            let root = self
                .load_entries(saved.state.entries_root, sequence, saved.source_through)
                .await?;
            if entries::queued(&root) != saved.state.queued {
                return Err(AgentError::fault(
                    "InputQueue",
                    "invalid queue result count",
                ));
            }
            Recovered {
                schema: saved.schema,
                source_through: saved.source_through,
                state: QueueState {
                    entries_root: root,
                    queued: saved.state.queued,
                    turn: saved.state.turn,
                    starting: saved.state.starting,
                },
            }
        } else {
            Recovered::default()
        };
        let mut kinds = SOURCES.to_vec();
        if initialize {
            kinds.extend(["input/applied", "input/deleted"]);
        }
        let mut source = request(
            &kinds,
            &[
                "kind",
                "id",
                "position.sequence",
                "data.mode",
                "data.input_id",
                "data.turn_id",
                "data.handover.turn_id",
            ],
            None,
        );
        source.from = Some(TimelinePosition {
            branch_id: self.timeline.branch_id(),
            sequence: recovered.source_through.saturating_add(1),
        });
        let mut owed = Vec::new();
        if recovered.source_through < through {
            loop {
                let reply = self.timeline.query(source.clone()).await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("queue recovery selects source fields")
                };
                let missing_modes: Vec<_> = rows
                    .iter()
                    .filter(|row| {
                        matches!(&row.values[0], QueryValue::Value { value } if value == "input/message")
                            && matches!(row.values[3], QueryValue::Missing)
                    })
                    .map(|row| match &row.values[1] {
                        QueryValue::Value { value } => value.clone(),
                        _ => unreachable!("a source has an identity"),
                    })
                    .collect();
                let mut users = HashSet::new();
                if !missing_modes.is_empty() {
                    let mut user = request(&["input/message"], &["id"], None);
                    user.from = source.from.clone();
                    let kind = user.query.filter.take().expect("input kind condition");
                    user.query.filter = Some(QueryFilter::And {
                        conditions: vec![
                            kind,
                            QueryFilter::In {
                                value: QueryOperand::Field { path: "id".into() },
                                values: missing_modes,
                            },
                            QueryFilter::Present {
                                value: QueryOperand::Field {
                                    path: "data.user.content".into(),
                                },
                            },
                        ],
                    });
                    let QueryOutput::Rows { rows } = self.timeline.query(user).await?.result else {
                        unreachable!("user presence selects input identities")
                    };
                    for row in rows {
                        let [QueryValue::Value { value }] = row.values.as_slice() else {
                            unreachable!("an input has an identity")
                        };
                        users.insert(value.as_str().expect("input identity").to_owned());
                    }
                }
                let changed = !rows.is_empty();
                let batch = if initialize {
                    through
                } else {
                    rows.last()
                        .map_or(recovered.source_through, |row| match &row.values[2] {
                            QueryValue::Value { value } => value.as_u64().expect("source position"),
                            _ => unreachable!("source position"),
                        })
                };
                for row in rows {
                    let Ok(
                        [
                            QueryValue::Value {
                                value: Value::String(kind),
                            },
                            QueryValue::Value {
                                value: Value::String(id),
                            },
                            QueryValue::Value { value: sequence },
                            mode,
                            input,
                            turn,
                            handover,
                        ],
                    ) = <[_; 7]>::try_from(row.values)
                    else {
                        return Err(AgentError::fault("InputQueue", "invalid queue source"));
                    };
                    let sequence = sequence.as_u64().expect("a source has a position");
                    let state = &mut recovered.state;
                    match kind.as_str() {
                        "input/message" => {
                            let mode = match mode {
                                QueryValue::Missing => {
                                    if users.contains(&id) {
                                        InputMode::Queue
                                    } else {
                                        InputMode::Append
                                    }
                                }
                                QueryValue::Value { value } => serde_json::from_value(value)
                                    .map_err(|error| {
                                        AgentError::fault("InputQueue", error.to_string())
                                    })?,
                                _ => unreachable!("an input mode is scalar"),
                            };
                            let handover = match &handover {
                                QueryValue::Missing | QueryValue::Value { value: Value::Null } => {
                                    None
                                }
                                QueryValue::Value { value } => {
                                    Some(value.as_str().expect("handover turn identity"))
                                }
                                _ => unreachable!("handover turn identity is scalar"),
                            };
                            owed.extend(state.input(&id, sequence, mode, handover, batch));
                        }
                        "turn/start" => {
                            let QueryValue::Value { value } = turn else {
                                unreachable!("turn identity")
                            };
                            state.turn = Some(value.as_str().expect("turn identity").to_owned());
                            state.starting = false;
                        }
                        "turn/end" => {
                            let QueryValue::Value { value } = turn else {
                                unreachable!("turn identity")
                            };
                            owed.extend(
                                state.end_turn(value.as_str().expect("turn identity"), batch),
                            );
                        }
                        "stop/interrupt" => state.starting = false,
                        "input/consumed" | "input/withdraw" | "input/applied" | "input/deleted" => {
                            let QueryValue::Value {
                                value: Value::String(input_id),
                            } = input
                            else {
                                return Err(AgentError::fault(
                                    "InputQueue",
                                    "a source has no input identity",
                                ));
                            };
                            match kind.as_str() {
                                "input/consumed" => {
                                    state.remove(&input_id, batch);
                                    owed.retain(|decision| !matches!(decision, Decision::Apply(id, _) if *id == input_id));
                                }
                                "input/withdraw" => owed.extend(state.withdraw(&input_id, batch)),
                                _ => owed.retain(|decision| decision.input_id() != input_id),
                            }
                        }
                        _ => unreachable!("queue recovery selects only queue sources"),
                    }
                    recovered.source_through = sequence;
                }
                if !initialize && changed {
                    self.remove_written(&mut owed, through).await?;
                    let (published, _) = self.checkpoint(&recovered, std::mem::take(&mut owed))?;
                    self.await_publication(published).await?;
                    published_result = published;
                }
                let Some(next) = reply.next else { break };
                source.page.as_mut().expect("source pages").after = Some(next);
            }
        }
        if initialize {
            owed.retain(|decision| match decision {
                Decision::Apply(id, _) => entries::get(&recovered.state.entries_root, id).is_some(),
                Decision::Delete(_) => true,
            });
            recovered.source_through = through;
            let (published, _) = self.checkpoint(&recovered, owed)?;
            self.await_publication(published).await?;
            published_result = published;
        }
        self.publish(published_result, &recovered.state);
        *self.state.lock().unwrap() = recovered;
        Ok(through)
    }
}
