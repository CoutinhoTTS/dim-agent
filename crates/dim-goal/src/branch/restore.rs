use super::*;
use dim_agent_types::TimelinePosition;
use std::collections::BTreeMap;

impl Owner {
    pub(super) async fn restore_at(
        &self,
        through: u64,
        opening: Option<(u64, bool)>,
    ) -> Result<(Fold, Option<u64>), AgentError> {
        let start = opening.map(|(sequence, _)| sequence);
        let request = |filter, fields: &[&str], pick| TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: start.map(|sequence| TimelinePosition {
                branch_id: self.branch_id.clone(),
                sequence,
            }),
            through: Some(through),
            include_publishing: true,
            query: QuerySpec {
                rows: None,
                filter: Some(filter),
                select: QuerySelect::Fields {
                    fields: fields.iter().map(|field| (*field).into()).collect(),
                },
                group_by: vec![],
                pick,
                order_by: vec![],
            },
            page: Some(QueryPage {
                limit: NonZeroU64::new(SCAN_PAGE).unwrap(),
                after: None,
            }),
        };
        let mut fold = Fold::default();
        if let Some((opening, true)) = opening {
            fold.copied_live = Some(self.copied_keys(opening).await?);
        }
        let mut query = request(
            QueryFilter::And {
                conditions: vec![kind("input/message"), input_relation("input/deleted")],
            },
            &["position.sequence"],
            None,
        );
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                    unreachable!("input position")
                };
                fold.deleted_sequences
                    .insert(sequence.as_u64().expect("sequence"));
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        let reply = self
            .timeline
            .query(request(
                live_inputs(QueryFilter::And {
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
                }),
                &["position.sequence"],
                Some(QueryPick::Last),
            ))
            .await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("selected fields")
        };
        if let Some(row) = rows.first() {
            let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                unreachable!("operator position")
            };
            fold.last_operator = sequence.as_u64().expect("sequence");
        }
        let reply = self
            .timeline
            .query(request(
                kind("turn/start"),
                &["position.sequence"],
                Some(QueryPick::Last),
            ))
            .await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("selected fields")
        };
        if let Some(row) = rows.first() {
            let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                unreachable!("turn position")
            };
            fold.turn_start = sequence.as_u64().expect("sequence");
        }
        let reply = self
            .timeline
            .query(request(
                kind("turn/end"),
                &["position.sequence", "data.turn_id", "data.status"],
                Some(QueryPick::Last),
            ))
            .await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("selected fields")
        };
        if let Some(row) = rows.first() {
            let [
                QueryValue::Value { value: sequence },
                QueryValue::Value { value: turn_id },
                QueryValue::Value { value: status },
            ] = row.values.as_slice()
            else {
                unreachable!("turn end fields")
            };
            let sequence = sequence.as_u64().expect("sequence");
            let status = match status.as_str().expect("turn status") {
                "completed" => fold::TurnStatus::Completed,
                "failed" => fold::TurnStatus::Failed,
                "cancelled" => fold::TurnStatus::Cancelled,
                _ => unreachable!("turn status contract"),
            };
            let turn_start = if fold.turn_start <= sequence {
                fold.turn_start
            } else {
                let mut own_start = request(
                    kind("turn/start"),
                    &["position.sequence"],
                    Some(QueryPick::Last),
                );
                own_start.through = Some(sequence);
                let reply = self.timeline.query(own_start).await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("selected fields")
                };
                rows.first().map_or(0, |row| {
                    let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                        unreachable!("turn position")
                    };
                    sequence.as_u64().expect("sequence")
                })
            };
            fold.turn_end = Some(TurnEnd {
                sequence,
                status,
                turn_id: turn_id.as_str().expect("turn id").to_owned(),
                turn_start,
                continuation_driven: false,
            });
        }
        let evaluator = match self.tree.info(self.branch_id.clone()) {
            Ok(info) => {
                matches!(info.source, Some(BranchSource::Host { operation_id }) if fold::evaluation_of_operation(&operation_id).is_some())
            }
            Err(_) => true,
        };
        if evaluator {
            let mut query = request(kind("turn/end"), &[], None);
            query.query.select = QuerySelect::Count;
            query.page = None;
            let reply = self.timeline.query(query).await?;
            let QueryOutput::Count { count } = reply.result else {
                unreachable!("turn count")
            };
            fold.turn_ends = usize::try_from(count).expect("turn count fits memory platform");
        }
        let reply = self
            .timeline
            .query(request(
                QueryFilter::And {
                    conditions: vec![
                        kind("stop/interrupt"),
                        QueryFilter::In {
                            value: QueryOperand::Field {
                                path: "data.scope".into(),
                            },
                            values: vec!["turn".into()],
                        },
                    ],
                },
                &["position.sequence"],
                Some(QueryPick::Last),
            ))
            .await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("selected fields")
        };
        if let Some(row) = rows.first() {
            let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                unreachable!("interrupt position")
            };
            fold.last_interrupt = sequence.as_u64().expect("sequence");
        }
        let mut starts = vec![0, fold.turn_start];
        if let Some(end) = &fold.turn_end {
            starts.push(end.turn_start);
        }
        starts.sort_unstable();
        starts.dedup();
        let mut at_starts = HashMap::<u64, HashSet<String>>::new();
        for at in starts {
            let mut consumed = vec![
                kind("input/consumed"),
                QueryFilter::Compare {
                    left: QueryOperand::Field {
                        path: "data.input_id".into(),
                    },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Outer { path: "id".into() },
                },
            ];
            let mut conditions = vec![QueryFilter::Or {
                conditions: vec![
                    QueryFilter::In {
                        value: QueryOperand::Field {
                            path: "data.mode".into(),
                        },
                        values: vec!["queue".into(), "steer".into()],
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
            }];
            if at != 0 {
                let before = QueryFilter::Compare {
                    left: QueryOperand::Field {
                        path: "position.sequence".into(),
                    },
                    comparison: QueryComparison::Lt,
                    right: QueryOperand::Value { value: at.into() },
                };
                consumed.push(before.clone());
                conditions.push(before);
                conditions.push(QueryFilter::TextCandidate {
                    field: "data".into(),
                    partition_by: vec![],
                    terms: vec!["goal".into()],
                });
                conditions.push(QueryFilter::Prefix {
                    value: QueryOperand::Field {
                        path: "data.blocks[0].text".into(),
                    },
                    prefix: "<dim-goal ".into(),
                });
            }
            conditions.push(QueryFilter::Not {
                condition: Box::new(QueryFilter::Exists {
                    condition: Box::new(QueryFilter::And {
                        conditions: consumed,
                    }),
                }),
            });
            let mut query = request(
                live_inputs(QueryFilter::And { conditions }),
                &["id", "position.sequence"],
                None,
            );
            loop {
                let reply = self.timeline.query(query.clone()).await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("selected fields")
                };
                for row in rows {
                    let [
                        QueryValue::Value { value: id },
                        QueryValue::Value { value: sequence },
                    ] = row.values.as_slice()
                    else {
                        unreachable!("driving input fields")
                    };
                    let id = id.as_str().expect("input id").to_owned();
                    if at == 0 {
                        fold.driving
                            .insert(id, sequence.as_u64().expect("sequence"));
                    } else {
                        at_starts.entry(at).or_default().insert(id);
                    }
                }
                let Some(next) = reply.next else { break };
                query.page.as_mut().unwrap().after = Some(next);
            }
        }
        let mut query = request(
            QueryFilter::And {
                conditions: vec![
                    kind("exec/started"),
                    QueryFilter::Not {
                        condition: Box::new(QueryFilter::Exists {
                            condition: Box::new(QueryFilter::And {
                                conditions: vec![
                                    kind("exec/ended"),
                                    QueryFilter::Compare {
                                        left: QueryOperand::Field {
                                            path: "data.task_id".into(),
                                        },
                                        comparison: QueryComparison::Eq,
                                        right: QueryOperand::Outer {
                                            path: "data.task_id".into(),
                                        },
                                    },
                                ],
                            }),
                        }),
                    },
                ],
            },
            &["data.task_id"],
            None,
        );
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [QueryValue::Value { value: task }] = row.values.as_slice() else {
                    unreachable!("task id")
                };
                fold.open_tasks
                    .insert(task.as_str().expect("task id").to_owned());
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        let mut evaluations = BTreeMap::new();
        let mut query = request(
            QueryFilter::And {
                conditions: vec![
                    kind("branch/create"),
                    QueryFilter::In {
                        value: QueryOperand::Field {
                            path: "data.source.kind".into(),
                        },
                        values: vec!["host".into()],
                    },
                    QueryFilter::Prefix {
                        value: QueryOperand::Field {
                            path: "data.source.operation_id".into(),
                        },
                        prefix: "goal:evaluate:".into(),
                    },
                ],
            },
            &[
                "position.sequence",
                "data.source.operation_id",
                "data.branch",
            ],
            None,
        );
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: operation },
                    QueryValue::Value { value: branch },
                ] = row.values.as_slice()
                else {
                    unreachable!("evaluation identity")
                };
                if let Some(evaluation) =
                    fold::evaluation_of_operation(operation.as_str().expect("operation id"))
                {
                    evaluations.insert(
                        sequence.as_u64().expect("sequence"),
                        (evaluation, branch.as_str().expect("branch id").to_owned()),
                    );
                }
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        let mut query = request(
            live_inputs(goal_inputs()),
            &[
                "position.sequence",
                "id",
                "data.submission_id",
                "data.blocks",
            ],
            None,
        );
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: id },
                    submission,
                    QueryValue::Value { value: blocks },
                ] = row.values.as_slice()
                else {
                    unreachable!("goal input fields")
                };
                let sequence = sequence.as_u64().expect("sequence");
                while evaluations
                    .first_key_value()
                    .is_some_and(|(created, _)| *created < sequence)
                {
                    let (_, (evaluation, branch)) = evaluations.pop_first().unwrap();
                    fold.evaluations.insert(evaluation, branch);
                    fold.decided.insert(evaluation.to_string());
                }
                if let Some(next) = fold.held_next()
                    && next < sequence
                {
                    self.pair_at(&mut fold, next, through).await?;
                }
                let blocks: Vec<InputBlock> =
                    serde_json::from_value(blocks.clone()).expect("input blocks contract");
                let id = id.as_str().expect("input id");
                if blocks
                    .first()
                    .and_then(|block| match block {
                        InputBlock::Raw { text, .. } => text::marker(text),
                    })
                    .is_some_and(|attrs| {
                        attrs.get("kind").is_some_and(|kind| kind == "continuation")
                    })
                {
                    if at_starts
                        .get(&fold.turn_start)
                        .is_some_and(|ids| ids.contains(id))
                    {
                        fold.turn_continuation_driven = true;
                    }
                    if let Some(end) = &mut fold.turn_end
                        && at_starts
                            .get(&end.turn_start)
                            .is_some_and(|ids| ids.contains(id))
                    {
                        end.continuation_driven = true;
                    }
                }
                let submission = match submission {
                    QueryValue::Missing => None,
                    QueryValue::Value { value } => Some(value.as_str().expect("submission id")),
                    _ => unreachable!("scalar submission id"),
                };
                fold.goal_input(sequence, id, submission, &blocks);
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        if let Some(next) = fold.held_next()
            && next <= through
        {
            self.pair_at(&mut fold, next, through).await?;
        }
        for (_, (evaluation, branch)) in evaluations {
            fold.evaluations.insert(evaluation, branch);
            fold.decided.insert(evaluation.to_string());
        }
        let mut query = request(
            QueryFilter::And {
                conditions: vec![
                    QueryFilter::In {
                        value: QueryOperand::Field {
                            path: "kind".into(),
                        },
                        values: vec!["branch/ended".into(), "branch/create-failed".into()],
                    },
                    QueryFilter::Exists {
                        condition: Box::new(QueryFilter::And {
                            conditions: vec![
                                kind("branch/create"),
                                QueryFilter::In {
                                    value: QueryOperand::Field {
                                        path: "data.source.kind".into(),
                                    },
                                    values: vec!["host".into()],
                                },
                                QueryFilter::Prefix {
                                    value: QueryOperand::Field {
                                        path: "data.source.operation_id".into(),
                                    },
                                    prefix: "goal:evaluate:".into(),
                                },
                                QueryFilter::Compare {
                                    left: QueryOperand::Field {
                                        path: "data.branch".into(),
                                    },
                                    comparison: QueryComparison::Eq,
                                    right: QueryOperand::Outer {
                                        path: "data.branch".into(),
                                    },
                                },
                            ],
                        }),
                    },
                ],
            },
            &["kind", "data.branch"],
            None,
        );
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: kind },
                    QueryValue::Value { value: branch },
                ] = row.values.as_slice()
                else {
                    unreachable!("evaluation terminal identity")
                };
                let branch = branch.as_str().expect("branch id").to_owned();
                if kind == "branch/ended" {
                    fold.ended.insert(branch);
                } else {
                    fold.failed.insert(branch);
                }
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        fold.finish_restore(start);
        Ok((fold, start))
    }

    async fn pair_at(
        &self,
        fold: &mut Fold,
        sequence: u64,
        through: u64,
    ) -> Result<(), AgentError> {
        let reply = self
            .timeline
            .query(TimelineQuery {
                scope: TimelineQueryScope::Branch,
                from: None,
                through: Some(through),
                include_publishing: true,
                query: QuerySpec {
                    rows: None,
                    filter: Some(live_inputs(QueryFilter::In {
                        value: QueryOperand::Field {
                            path: "position.sequence".into(),
                        },
                        values: vec![sequence.into()],
                    })),
                    select: QuerySelect::Fields {
                        fields: vec!["data.submission_id".into()],
                    },
                    group_by: vec![],
                    pick: None,
                    order_by: vec![],
                },
                page: Some(QueryPage {
                    limit: NonZeroU64::MIN,
                    after: None,
                }),
            })
            .await?;
        let QueryOutput::Rows { rows } = reply.result else {
            unreachable!("selected fields")
        };
        let submission = rows.first().and_then(|row| match &row.values[..] {
            [QueryValue::Value { value }] => Some(value.as_str().expect("submission id")),
            [QueryValue::Missing] => None,
            _ => unreachable!("scalar submission id"),
        });
        fold.pair_next(sequence, submission);
        Ok(())
    }
}

fn kind(name: &str) -> QueryFilter {
    QueryFilter::In {
        value: QueryOperand::Field {
            path: "kind".into(),
        },
        values: vec![name.into()],
    }
}

fn input_relation(name: &str) -> QueryFilter {
    QueryFilter::Exists {
        condition: Box::new(QueryFilter::And {
            conditions: vec![
                kind(name),
                QueryFilter::Compare {
                    left: QueryOperand::Field {
                        path: "data.input_id".into(),
                    },
                    comparison: QueryComparison::Eq,
                    right: QueryOperand::Outer { path: "id".into() },
                },
            ],
        }),
    }
}

impl Owner {
    pub(super) async fn open_evaluations(
        &self,
        through: u64,
        opening: Option<u64>,
        fold: &mut Fold,
    ) -> Result<(), AgentError> {
        let mut query = TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: opening.map(|sequence| TimelinePosition {
                branch_id: self.branch_id.clone(),
                sequence,
            }),
            through: Some(through),
            include_publishing: true,
            query: QuerySpec {
                rows: None,
                filter: Some(QueryFilter::And {
                    conditions: vec![
                        kind("branch/create"),
                        QueryFilter::In {
                            value: QueryOperand::Field {
                                path: "data.source.kind".into(),
                            },
                            values: vec!["host".into()],
                        },
                        QueryFilter::Prefix {
                            value: QueryOperand::Field {
                                path: "data.source.operation_id".into(),
                            },
                            prefix: "goal:evaluate:".into(),
                        },
                        QueryFilter::Not {
                            condition: Box::new(QueryFilter::Exists {
                                condition: Box::new(QueryFilter::And {
                                    conditions: vec![
                                        QueryFilter::In {
                                            value: QueryOperand::Field {
                                                path: "kind".into(),
                                            },
                                            values: vec![
                                                "branch/ended".into(),
                                                "branch/create-failed".into(),
                                            ],
                                        },
                                        QueryFilter::Compare {
                                            left: QueryOperand::Field {
                                                path: "data.branch".into(),
                                            },
                                            comparison: QueryComparison::Eq,
                                            right: QueryOperand::Outer {
                                                path: "data.branch".into(),
                                            },
                                        },
                                    ],
                                }),
                            }),
                        },
                    ],
                }),
                select: QuerySelect::Fields {
                    fields: vec!["data.source.operation_id".into(), "data.branch".into()],
                },
                group_by: vec![],
                pick: None,
                order_by: vec![],
            },
            page: Some(QueryPage {
                limit: NonZeroU64::new(SCAN_PAGE).unwrap(),
                after: None,
            }),
        };
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: operation },
                    QueryValue::Value { value: branch },
                ] = row.values.as_slice()
                else {
                    unreachable!("evaluation identity")
                };
                if let Some(evaluation) =
                    fold::evaluation_of_operation(operation.as_str().expect("operation id"))
                {
                    fold.evaluations
                        .insert(evaluation, branch.as_str().expect("branch id").to_owned());
                }
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        Ok(())
    }

    async fn withdrawn_change(
        &self,
        from: u64,
        through: u64,
        recorded: u64,
    ) -> Result<Option<(u64, u64)>, AgentError> {
        let mut query = TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: None,
            through: Some(through),
            include_publishing: true,
            query: QuerySpec {
                rows: None,
                filter: Some(QueryFilter::And {
                    conditions: vec![
                        kind("input/message"),
                        QueryFilter::Compare {
                            left: QueryOperand::Field {
                                path: "position.sequence".into(),
                            },
                            comparison: QueryComparison::Le,
                            right: QueryOperand::Value {
                                value: recorded.into(),
                            },
                        },
                        goal_inputs(),
                        QueryFilter::Exists {
                            condition: Box::new(QueryFilter::And {
                                conditions: vec![
                                    kind("input/deleted"),
                                    QueryFilter::Compare {
                                        left: QueryOperand::Field {
                                            path: "data.input_id".into(),
                                        },
                                        comparison: QueryComparison::Eq,
                                        right: QueryOperand::Outer { path: "id".into() },
                                    },
                                    QueryFilter::Compare {
                                        left: QueryOperand::Field {
                                            path: "position.sequence".into(),
                                        },
                                        comparison: QueryComparison::Ge,
                                        right: QueryOperand::Value { value: from.into() },
                                    },
                                ],
                            }),
                        },
                    ],
                }),
                select: QuerySelect::Fields {
                    fields: vec![
                        "position.sequence".into(),
                        "id".into(),
                        "data.blocks".into(),
                    ],
                },
                group_by: vec![],
                pick: None,
                order_by: vec![],
            },
            page: Some(QueryPage {
                limit: NonZeroU64::new(SCAN_PAGE).unwrap(),
                after: None,
            }),
        };
        let mut compensation = None;
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: id },
                    QueryValue::Value { value: blocks },
                ] = row.values.as_slice()
                else {
                    unreachable!("withdrawn goal fields")
                };
                let blocks: Vec<InputBlock> =
                    serde_json::from_value(blocks.clone()).expect("input blocks contract");
                if fold::change_of(&blocks).is_none() {
                    continue;
                }
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
                                    kind("input/deleted"),
                                    QueryFilter::In {
                                        value: QueryOperand::Field {
                                            path: "data.input_id".into(),
                                        },
                                        values: vec![id.clone()],
                                    },
                                ],
                            }),
                            select: QuerySelect::Fields {
                                fields: vec!["position.sequence".into()],
                            },
                            group_by: vec![],
                            pick: Some(QueryPick::Last),
                            order_by: vec![],
                        },
                        page: Some(QueryPage {
                            limit: NonZeroU64::MIN,
                            after: None,
                        }),
                    })
                    .await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("selected fields")
                };
                let [QueryValue::Value { value: at }] = rows[0].values.as_slice() else {
                    unreachable!("deletion position")
                };
                let at = at.as_u64().expect("sequence");
                if compensation.is_none_or(|(previous, _)| at > previous) {
                    compensation = Some((at, sequence.as_u64().expect("sequence")));
                }
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        Ok(compensation)
    }

    pub(super) async fn write_guard(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, AgentError> {
        loop {
            let mut fresh = self.fresh.subscribe();
            loop {
                let status = fresh.borrow_and_update().clone()?;
                if status {
                    break;
                }
                tokio::select! {
                    result = fresh.changed() => { result.map_err(|_| AgentError::Reloading("Goal".into()))?; }
                    _ = self.stop.cancelled() => return Err(AgentError::Closed(self.branch_id.clone())),
                }
            }
            let write = tokio::select! {
                guard = self.write.lock() => guard,
                _ = self.stop.cancelled() => return Err(AgentError::Closed(self.branch_id.clone())),
            };
            self.await_fold().await?;
            if self.fresh.borrow().clone()? {
                return Ok(write);
            }
        }
    }

    pub(super) fn refresh(self: &Arc<Self>) {
        let owner = self.clone();
        self.tasks.spawn(async move {
            if !owner.phase.activated().await {
                return;
            }
            let result = tokio::select! {
                _ = owner.stop.cancelled() => return,
                result = owner.refresh_state() => result,
            };
            if let Err(error) = result {
                tracing::warn!(%error, "Goal restore failed");
                let _ = owner.fresh.send_replace(Err(error));
            }
        });
    }

    async fn refresh_state(self: &Arc<Self>) -> Result<(), AgentError> {
        let write = self.write.lock().await;
        loop {
            let through = self.timeline.committed();
            let reply = self
                .timeline
                .query(TimelineQuery {
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
                            values: KEYS.iter().map(|key| (*key).into()).collect(),
                        }),
                        select: QuerySelect::Fields {
                            fields: vec!["position.sequence".into()],
                        },
                        group_by: vec![],
                        pick: Some(QueryPick::Last),
                        order_by: vec![],
                    },
                    page: Some(QueryPage {
                        limit: NonZeroU64::MIN,
                        after: None,
                    }),
                })
                .await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("selected fields")
            };
            if let Some(row) = rows.first() {
                let [QueryValue::Value { value: sequence }] = row.values.as_slice() else {
                    unreachable!("delivered position")
                };
                let target = sequence.as_u64().expect("sequence");
                self.stepped
                    .subscribe()
                    .wait_for(|sequence| *sequence >= target)
                    .await
                    .map_err(|_| AgentError::Reloading("Goal".into()))?;
            }
            let opening = self.opening(through).await?;
            let (fold, start) = self.restore_at(through, opening).await?;
            let (from, recorded) = {
                let state = self.state.lock().unwrap();
                (
                    state.refresh_from.expect("deletion starts refresh"),
                    state.recorded.max(fold.record_after),
                )
            };
            let compensation = self.withdrawn_change(from, through, recorded).await?;
            {
                let mut state = self.state.lock().unwrap();
                if *self.stepped.borrow() > through {
                    continue;
                }
                state.fold = fold;
                state.start = start;
                state.decisions = CancellationToken::new();
                state.refresh_from = compensation.map(|(at, _)| at);
                let _ = self.fresh.send_replace(Ok(true));
            }
            drop(write);
            if let Some((at, withdrawn)) = compensation {
                self.task(format!("delete:{at}"), None, move |owner| {
                    Box::pin(async move { owner.record(at, None, false, Some(withdrawn)).await })
                });
            } else {
                self.reconcile(through).await?;
            }
            self.resume_decisions();
            return Ok(());
        }
    }

    pub(super) fn resume_decisions(self: &Arc<Self>) {
        let (pending, start) = {
            let state = self.state.lock().unwrap();
            (state.fold.pending_conclusion().is_some(), state.start)
        };
        if pending {
            self.settle_deferred();
        } else {
            self.decide_end();
        }
        self.settle();
        if let Some(start) = start {
            self.decide_start(start);
        }
        self.answer_change();
    }
}
