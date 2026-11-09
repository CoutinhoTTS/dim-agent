use super::*;
use crate::fold::Source;
use dim_agent_types::{ExtensionEvent, GoalRead, TimelinePosition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

mod deletion;
mod nodes;

pub(crate) const KEY: &str = "goal/recovered";
const SCHEMA: &str = "goal/v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recovered {
    schema: String,
    pub(crate) source_through: u64,
    pub(crate) state: RecoveryState,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) input_id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) before: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryState {
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) opening: Option<u64>,
    pub(crate) fold: Fold,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) driving_root: Option<String>,
}

pub(crate) fn decode(data: Value, position: u64, through: u64) -> Result<Recovered, AgentError> {
    if data.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(AgentError::fault("Goal", "unsupported recovery schema"));
    }
    let recovered: Recovered = serde_json::from_value(data)
        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
    if recovered.source_through > through
        || recovered.source_through >= position
        || recovered
            .state
            .opening
            .is_some_and(|opening| opening > recovered.source_through)
    {
        return Err(AgentError::fault(
            "Goal",
            "invalid recovery source position",
        ));
    }
    if recovered.input_id.is_none() && recovered.before.is_some() {
        return Err(AgentError::fault(
            "Goal",
            "recovery boundary has no input identity",
        ));
    }
    Ok(recovered)
}

pub(super) fn view(branch: &str, position: u64, data: &Value) -> Result<GoalRead, AgentError> {
    let (Some(schema), Some(source), Some(objective), Some(status), Some(judge)) = (
        data.get("schema"),
        data.get("source_through"),
        data.pointer("/state/fold/objective"),
        data.pointer("/state/fold/state"),
        data.pointer("/state/fold/judge"),
    ) else {
        return Err(AgentError::fault("Goal", "missing recovery view fields"));
    };
    if schema.as_str() != Some(SCHEMA) {
        return Err(AgentError::fault("Goal", "unsupported recovery schema"));
    }
    let source = source
        .as_u64()
        .filter(|source| *source < position)
        .ok_or_else(|| AgentError::fault("Goal", "invalid recovery source position"))?;
    let objective = serde_json::from_value(objective.clone())
        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
    let status = serde_json::from_value(status.clone())
        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
    let judge = serde_json::from_value(judge.clone())
        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
    Ok(GoalRead {
        branch_id: branch.into(),
        through: source,
        goal: crate::read::goal(objective, &status, judge),
    })
}

pub(crate) fn request(
    branch: &str,
    from: Option<u64>,
    through: u64,
    kinds: &[&str],
    fields: &[&str],
) -> TimelineQuery {
    TimelineQuery {
        scope: TimelineQueryScope::Branch,
        from: from.map(|sequence| TimelinePosition {
            branch_id: branch.into(),
            sequence,
        }),
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
            group_by: vec![],
            pick: None,
            order_by: vec![],
        },
        page: Some(QueryPage {
            limit: NonZeroU64::new(128).unwrap(),
            after: None,
        }),
    }
}

impl State {
    pub(super) fn checkpoint(
        &mut self,
        owner: &Owner,
        input_id: Option<&str>,
    ) -> Result<(), AgentError> {
        if self.checkpoint_blocked || self.source_through == 0 {
            return Ok(());
        }
        let data = json!({
            "schema":SCHEMA,
            "source_through":self.source_through,
            "state":{"opening":self.start,"fold":self.fold,"driving_root":self.fold.driving.root()},
            "input_id":input_id,
            "before":input_id.and(self.result_id.as_deref()),
        });
        let nodes = self.fold.driving.pending();
        let mut drafts = crate::driving::drafts(&nodes)?;
        drafts.push(
            EventKind::Extension(
                ExtensionEvent::new("goal", "recovered", data).expect("Goal recovery key"),
            )
            .into(),
        );
        let events = owner.timeline.append_batch_registered(drafts, |_| Ok(()))?;
        let event = events.last().expect("Goal recovery result");
        for node in nodes {
            node.pending.store(false, Ordering::Relaxed);
        }
        self.result_position = event.sequence;
        self.result_id = Some(event.id.clone());
        Ok(())
    }
}

impl Owner {
    pub(super) async fn current(&self) -> Result<GoalRead, AgentError> {
        self.await_ready().await?;
        self.await_fold().await?;
        let through = self.timeline.committed();
        let mut dispatched = self.dispatcher.watch();
        let mut timeline = self.timeline.watch();
        let mut published = self.published.subscribe();
        let mut fresh = self.fresh.subscribe();
        let mut target = None;
        loop {
            if let Some(error) = self
                .dispatcher
                .failure()
                .or_else(|| self.timeline.failure())
            {
                return Err(error);
            }
            let valid = fresh.borrow_and_update().clone()?;
            if target.is_none() && self.dispatcher.sequence() >= through && valid {
                let state = self.state.lock().unwrap();
                if state.source_through == 0 && !state.checkpoint_blocked {
                    return Ok(crate::read::view(&self.branch_id, through, &state.fold));
                }
                if !state.checkpoint_blocked {
                    target = Some(state.source_through);
                }
            }
            if let Some(target) = target
                && let Some(read) = published.borrow_and_update().as_ref()
                && read.through >= target
            {
                let mut read = read.clone();
                read.through = read.through.max(through);
                return Ok(read);
            }
            tokio::select! {
                result = dispatched.changed() => result.map_err(|_| AgentError::Closed("Goal".into()))?,
                result = timeline.changed() => result.map_err(|_| AgentError::Closed("Goal".into()))?,
                result = published.changed() => result.map_err(|_| AgentError::Closed("Goal".into()))?,
                result = fresh.changed() => result.map_err(|_| AgentError::Closed("Goal".into()))?,
                _ = self.stop.cancelled() => return Err(AgentError::Closed(self.branch_id.clone())),
            }
        }
    }

    pub(super) async fn restore_result(
        &self,
        through: u64,
        opening: Option<(u64, bool)>,
    ) -> Result<bool, AgentError> {
        let mut query = request(
            &self.branch_id,
            opening.map(|(at, _)| at),
            through,
            &[KEY],
            &["id", "position.sequence", "data"],
        );
        query.query.pick = Some(QueryPick::Last);
        query.page.as_mut().unwrap().limit = NonZeroU64::MIN;
        let QueryOutput::Rows { mut rows } = self.timeline.query(query).await?.result else {
            unreachable!("Goal recovery fields")
        };
        let Some(row) = rows.pop() else {
            return Ok(false);
        };
        let [
            QueryValue::Value { value: id },
            QueryValue::Value { value: position },
            QueryValue::Value { value: data },
        ] = <[_; 3]>::try_from(row.values).expect("Goal recovery fields")
        else {
            unreachable!("Goal recovery values")
        };
        let position = position.as_u64().expect("event position");
        let mut recovered = decode(data, position, through)?;
        if recovered.state.opening != opening.map(|(at, _)| at) {
            return Err(AgentError::fault("Goal", "recovery opening mismatch"));
        }
        recovered.state.fold.driving = self
            .load_driving(recovered.state.driving_root, position)
            .await?;
        let read = crate::read::view(
            &self.branch_id,
            recovered.source_through,
            &recovered.state.fold,
        );
        {
            let mut state = self.state.lock().unwrap();
            state.fold = recovered.state.fold;
            state.recorded = state.fold.record_after;
            state.start = recovered.state.opening;
            state.source_through = recovered.source_through;
            state.result_position = position;
            state.published_position = position;
            state.result_id = Some(id.as_str().expect("result identity").to_owned());
        }
        self.published.send_replace(Some(read));
        Ok(true)
    }

    pub(super) async fn restore_suffix(
        self: &Arc<Self>,
        through: u64,
    ) -> Result<Option<u64>, AgentError> {
        let from = self.state.lock().unwrap().source_through.saturating_add(1);
        if from > through {
            return Ok(None);
        }
        let mut query = request(
            &self.branch_id,
            Some(from),
            through,
            KEYS,
            &["id", "position.sequence", "kind"],
        );
        let (mut fold, start) = {
            let state = self.state.lock().unwrap();
            (state.fold.clone(), state.start)
        };
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("Goal source fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: id },
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: kind },
                ] = <[_; 3]>::try_from(row.values).expect("Goal source fields")
                else {
                    unreachable!("Goal source values")
                };
                let sequence = sequence.as_u64().expect("source position");
                let kind = kind.as_str().expect("source kind");
                if kind == "input/deleted" {
                    return Ok(Some(sequence));
                }
                self.read_source(
                    &mut fold,
                    id.as_str().expect("source identity"),
                    sequence,
                    kind,
                    through,
                    start,
                )
                .await?;
                let mut state = self.state.lock().unwrap();
                state.fold = fold.clone();
                state.source_through = sequence;
                state.checkpoint(
                    self,
                    (kind == "input/message").then_some(id.as_str().expect("source identity")),
                )?;
            }
            query.page.as_mut().unwrap().after = reply.next;
            if query.page.as_ref().unwrap().after.is_none() {
                break;
            }
        }
        Ok(None)
    }

    async fn fields(
        &self,
        id: &str,
        kind: &str,
        through: u64,
        fields: &[&str],
    ) -> Result<Vec<QueryValue>, AgentError> {
        let mut query = request(&self.branch_id, None, through, &[kind], fields);
        query.query.filter = Some(QueryFilter::And {
            conditions: vec![query.query.filter.take().unwrap(), eq("id", json!(id))],
        });
        query.page.as_mut().unwrap().limit = NonZeroU64::MIN;
        let QueryOutput::Rows { mut rows } = self.timeline.query(query).await?.result else {
            unreachable!("source fields")
        };
        rows.pop()
            .map(|row| row.values)
            .ok_or_else(|| AgentError::fault("Goal", "missing source fact"))
    }

    async fn input_matches(
        &self,
        id: &str,
        through: u64,
        condition: QueryFilter,
    ) -> Result<bool, AgentError> {
        let mut query = request(&self.branch_id, None, through, &["input/message"], &[]);
        query.query.filter = Some(QueryFilter::And {
            conditions: vec![
                query.query.filter.take().unwrap(),
                eq("id", json!(id)),
                condition,
            ],
        });
        query.query.select = QuerySelect::Exists;
        query.page = None;
        let QueryOutput::Exists { exists } = self.timeline.query(query).await?.result else {
            unreachable!("input predicate")
        };
        Ok(exists)
    }

    pub(super) async fn read_source(
        &self,
        fold: &mut Fold,
        id: &str,
        sequence: u64,
        kind: &str,
        through: u64,
        start: Option<u64>,
    ) -> Result<(), AgentError> {
        let fields: &[&str] = match kind {
            "input/message" => &["data.submission_id"],
            "input/consumed" => &["data.input_id"],
            "branch/create" => &["data.source", "data.branch"],
            "branch/ended" | "branch/create-failed" => &["data.branch"],
            "turn/end" => &["data.turn_id", "data.status"],
            "stop/interrupt" => &["data.scope"],
            "exec/started" | "exec/ended" => &["data.task_id"],
            "turn/start" => &[],
            _ => {
                return Err(AgentError::fault(
                    "Goal",
                    "unexpected opening in recovery suffix",
                ));
            }
        };
        let values = if fields.is_empty() {
            vec![]
        } else {
            self.fields(id, kind, through, fields).await?
        };
        let string = |index: usize| match &values[index] {
            QueryValue::Value { value } => value.as_str().expect("source string"),
            _ => unreachable!("source string field"),
        };
        let mut blocks = vec![];
        let source = match kind {
            "input/message" => {
                let operator = self
                    .input_matches(
                        id,
                        through,
                        QueryFilter::And {
                            conditions: vec![
                                QueryFilter::Present {
                                    value: QueryOperand::Field {
                                        path: "data.user".into(),
                                    },
                                },
                                QueryFilter::Not {
                                    condition: Box::new(QueryFilter::Prefix {
                                        value: QueryOperand::Field {
                                            path: "data.blocks[0].text".into(),
                                        },
                                        prefix: "<dim-".into(),
                                    }),
                                },
                            ],
                        },
                    )
                    .await?;
                let driving = self
                    .input_matches(
                        id,
                        through,
                        QueryFilter::Or {
                            conditions: vec![
                                QueryFilter::In {
                                    value: QueryOperand::Field {
                                        path: "data.mode".into(),
                                    },
                                    values: vec![json!("queue"), json!("steer")],
                                },
                                QueryFilter::And {
                                    conditions: vec![
                                        QueryFilter::Not {
                                            condition: Box::new(QueryFilter::Present {
                                                value: QueryOperand::Field {
                                                    path: "data.mode".into(),
                                                },
                                            }),
                                        },
                                        QueryFilter::Present {
                                            value: QueryOperand::Field {
                                                path: "data.user".into(),
                                            },
                                        },
                                    ],
                                },
                            ],
                        },
                    )
                    .await?;
                if self.input_matches(id, through, goal_inputs()).await? {
                    let mut fields = self.fields(id, kind, through, &["data.blocks"]).await?;
                    let QueryValue::Value { value } = fields.remove(0) else {
                        unreachable!("input blocks")
                    };
                    blocks = serde_json::from_value(value)
                        .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
                }
                let submission = match &values[0] {
                    QueryValue::Missing => None,
                    QueryValue::Value { value } => Some(value.as_str().expect("submission id")),
                    _ => unreachable!("submission scalar"),
                };
                Source::Input {
                    id,
                    submission,
                    driving,
                    operator,
                    blocks: &blocks,
                }
            }
            "input/consumed" => Source::Consumed {
                input_id: string(0),
            },
            "branch/create" => {
                let QueryValue::Value { value } = &values[0] else {
                    unreachable!("branch source")
                };
                match value
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .and_then(fold::evaluation_of_operation)
                {
                    Some(evaluation) if value["kind"] == "host" => Source::Evaluation {
                        evaluation,
                        branch: string(1),
                    },
                    _ => Source::Ignored,
                }
            }
            "branch/ended" | "branch/create-failed" => Source::BranchEnded {
                branch: string(0),
                failed: kind == "branch/create-failed",
            },
            "turn/start" => Source::TurnStart,
            "turn/end" => Source::TurnEnd {
                turn_id: string(0),
                status: match string(1) {
                    "completed" => fold::TurnStatus::Completed,
                    "failed" => fold::TurnStatus::Failed,
                    "cancelled" => fold::TurnStatus::Cancelled,
                    _ => unreachable!("turn outcome status"),
                },
            },
            "stop/interrupt" => Source::Interrupt {
                turn: string(0) == "turn",
            },
            "exec/started" | "exec/ended" => Source::Task {
                task_id: string(0),
                started: kind == "exec/started",
            },
            _ => unreachable!(),
        };
        fold.apply_source(sequence, source);
        fold.compact(start);
        Ok(())
    }
}

fn eq(field: &str, value: Value) -> QueryFilter {
    QueryFilter::Compare {
        left: QueryOperand::Field { path: field.into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value },
    }
}
