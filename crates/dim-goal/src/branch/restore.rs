use super::*;

impl Owner {
    pub(super) async fn write_guard(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, AgentError> {
        let write = tokio::select! {
            guard = self.write.lock() => guard,
            _ = self.stop.cancelled() => return Err(AgentError::Closed(self.branch_id.clone())),
        };
        self.await_fold().await?;
        self.await_dispatched().await?;
        self.write_status.borrow().clone()?;
        Ok(write)
    }

    /// Waits until the Dispatcher has delivered every fact committed at the
    /// call and returns that cutoff. A decision taken after this wait sees an
    /// input or a turn that committed before it, so the caller does not act on
    /// a stale fold (C4, C5). A caller claiming a decision reads the cutoff
    /// again under its lock: a fact committing while this wait runs leaves the
    /// returned cutoff behind, and the claim catches up to the newer one before
    /// it reads the fold.
    pub(super) async fn await_dispatched(&self) -> Result<u64, AgentError> {
        let through = self.timeline.committed();
        #[cfg(feature = "testing")]
        crate::testing::stage_held(
            &self.session_id,
            &self.branch_id,
            crate::testing::Stage::Cutoff,
            through,
        )
        .await;
        let mut dispatched = self.dispatcher.watch();
        while self.dispatcher.sequence() < through {
            if let Some(error) = self.dispatcher.failure() {
                return Err(error);
            }
            tokio::select! {result=dispatched.changed()=>result.map_err(|_|AgentError::Closed("Goal".into()))?,_=self.stop.cancelled()=>return Err(AgentError::Closed("Goal".into()))}
        }
        Ok(through)
    }

    /// Releases task admission and takes up what the settlement deferred: a
    /// creation, an interrupt or a fold that reached `settle` while the
    /// decisions were suspended ran no task, and no later input may be needed
    /// for it (Pursuit §4.1, §13 E2).
    pub(super) fn resume_settlement(self: &Arc<Self>) {
        if let Some(interrupt) = self.release_settlement() {
            self.interrupted(interrupt);
        }
    }

    /// Releases task admission without taking up the interrupt the settlement
    /// deferred, and answers that interrupt. A state write that failed states
    /// nothing, so it does not decide that pause again: it stays with the fold
    /// until a later settlement takes it up (Pursuit §4.1, §13 E2, #2119).
    pub(super) fn release_settlement(self: &Arc<Self>) -> Option<u64> {
        let interrupt = {
            let mut state = self.state.lock().unwrap();
            state.terminating = false;
            state.fold.pending_interrupt()
        };
        self.settle();
        interrupt
    }

    pub(super) fn resume_decisions(self: &Arc<Self>) {
        if !self.state.lock().unwrap().fold.running() {
            return;
        }
        let pending = self
            .state
            .lock()
            .unwrap()
            .fold
            .pending_conclusion()
            .is_some();
        if pending {
            self.settle_deferred();
        } else {
            self.decide_end();
        }
        self.settle();
        self.answer_change();
    }
}

impl Owner {
    pub(super) async fn load_current_work(&self) -> Result<(), AgentError> {
        let equal = |path: &str, value| QueryFilter::Compare {
            left: QueryOperand::Field { path: path.into() },
            comparison: QueryComparison::Eq,
            right: QueryOperand::Value { value },
        };
        loop {
            let through = self.timeline.committed();
            self.publication(through).await?;
            let mut sets = Vec::with_capacity(2);
            for (origin, terminal, inner, outer) in [
                (
                    "input/message",
                    vec!["input/consumed", "input/deleted"],
                    "data.input_id",
                    "id",
                ),
                (
                    "exec/started",
                    vec!["exec/ended"],
                    "data.task_id",
                    "data.task_id",
                ),
            ] {
                let mut query =
                    recovery::request(&self.branch_id, None, through, &[origin], &[outer]);
                let mut conditions = vec![
                    query.query.filter.take().expect("origin kind"),
                    QueryFilter::Not {
                        condition: Box::new(QueryFilter::Exists {
                            condition: Box::new(QueryFilter::And {
                                conditions: vec![
                                    QueryFilter::In {
                                        value: QueryOperand::Field {
                                            path: "kind".into(),
                                        },
                                        values: terminal
                                            .into_iter()
                                            .map(serde_json::Value::from)
                                            .collect(),
                                    },
                                    QueryFilter::Compare {
                                        left: QueryOperand::Field { path: inner.into() },
                                        comparison: QueryComparison::Eq,
                                        right: QueryOperand::Outer { path: outer.into() },
                                    },
                                ],
                            }),
                        }),
                    },
                ];
                if origin == "input/message" {
                    let present = |path: &str| QueryFilter::Present {
                        value: QueryOperand::Field { path: path.into() },
                    };
                    conditions.push(QueryFilter::Not {
                        condition: Box::new(present("data.handover")),
                    });
                    conditions.push(QueryFilter::Or {
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
                                        condition: Box::new(present("data.mode")),
                                    },
                                    present("data.user"),
                                    QueryFilter::Not {
                                        condition: Box::new(equal(
                                            "data.user",
                                            serde_json::Value::Null,
                                        )),
                                    },
                                ],
                            },
                        ],
                    });
                }
                query.query.filter = Some(QueryFilter::And { conditions });
                let mut ids = HashSet::new();
                loop {
                    let reply = self.timeline.query(query.clone()).await?;
                    let QueryOutput::Rows { rows } = reply.result else {
                        unreachable!("current identity fields")
                    };
                    for row in rows {
                        let [
                            QueryValue::Value {
                                value: serde_json::Value::String(id),
                            },
                        ] = row.values.as_slice()
                        else {
                            return Err(AgentError::fault(
                                "Goal",
                                "missing current execution identity",
                            ));
                        };
                        ids.insert(id.clone());
                    }
                    query.page.as_mut().expect("identity page").after = reply.next;
                    if query.page.as_ref().expect("identity page").after.is_none() {
                        break;
                    }
                }
                sets.push(ids);
            }
            let mut dispatched = self.dispatcher.watch();
            while self.dispatcher.sequence() < through {
                if let Some(error) = self.dispatcher.failure() {
                    return Err(error);
                }
                tokio::select! {result=dispatched.changed()=>result.map_err(|_|AgentError::Closed("Goal".into()))?,_=self.stop.cancelled()=>return Err(AgentError::Closed("Goal".into()))}
            }
            let mut state = self.state.lock().unwrap();
            if self.timeline.committed() != through {
                continue;
            }
            let tasks = sets.pop().expect("task identities");
            state.fold.driving = sets.pop().expect("input identities");
            state.fold.open_tasks = tasks;
            return Ok(());
        }
    }
}
