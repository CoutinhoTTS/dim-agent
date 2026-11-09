use super::*;
use std::collections::HashMap;

struct Deleted {
    sequence: u64,
    at: u64,
}

impl Owner {
    async fn result_at(
        &self,
        id: &str,
        through: u64,
        read_head: u64,
    ) -> Result<(u64, Recovered), AgentError> {
        let mut query = request(
            &self.branch_id,
            None,
            read_head,
            &[KEY],
            &["position.sequence", "data"],
        );
        query.query.filter = Some(QueryFilter::And {
            conditions: vec![query.query.filter.take().unwrap(), eq("id", json!(id))],
        });
        query.page.as_mut().unwrap().limit = NonZeroU64::MIN;
        let QueryOutput::Rows { mut rows } = self.timeline.query(query).await?.result else {
            unreachable!("Goal boundary fields")
        };
        let row = rows
            .pop()
            .ok_or_else(|| AgentError::fault("Goal", "missing input recovery boundary"))?;
        let [
            QueryValue::Value { value: position },
            QueryValue::Value { value: data },
        ] = <[_; 2]>::try_from(row.values).expect("boundary fields")
        else {
            unreachable!("boundary values")
        };
        let position = position.as_u64().expect("boundary position");
        Ok((position, decode(data, position, through)?))
    }

    async fn before_input(
        &self,
        id: &str,
        sequence: u64,
        through: u64,
        read_head: u64,
    ) -> Result<Option<(u64, Recovered)>, AgentError> {
        let mut query = request(
            &self.branch_id,
            None,
            read_head,
            &[KEY],
            &["data.schema", "data.source_through", "data.before"],
        );
        query.query.filter = Some(QueryFilter::And {
            conditions: vec![
                query.query.filter.take().unwrap(),
                eq("data.input_id", json!(id)),
            ],
        });
        query.query.pick = Some(QueryPick::Last);
        query.page.as_mut().unwrap().limit = NonZeroU64::MIN;
        let QueryOutput::Rows { mut rows } = self.timeline.query(query).await?.result else {
            unreachable!("input boundary fields")
        };
        let Some(row) = rows.pop() else {
            return Ok(None);
        };
        let [
            QueryValue::Value { value: schema },
            QueryValue::Value { value: source },
            QueryValue::Value { value: before },
        ] = <[_; 3]>::try_from(row.values).expect("input boundary fields")
        else {
            unreachable!("input boundary values")
        };
        if schema.as_str() != Some(SCHEMA) {
            return Err(AgentError::fault("Goal", "unsupported recovery schema"));
        }
        if source.as_u64() != Some(sequence) {
            return Err(AgentError::fault(
                "Goal",
                "input recovery boundary source mismatch",
            ));
        }
        let Some(before) = before.as_str() else {
            if before.is_null() {
                return Ok(None);
            }
            return Err(AgentError::fault(
                "Goal",
                "invalid input recovery reference",
            ));
        };
        let result = self.result_at(before, through, read_head).await?;
        if result.1.source_through >= sequence {
            return Err(AgentError::fault(
                "Goal",
                "input boundary does not precede its source",
            ));
        }
        Ok(Some(result))
    }

    pub(in super::super) async fn restore_deleted(
        &self,
        through: u64,
        opening: Option<(u64, bool)>,
        from: u64,
        recorded: u64,
    ) -> Result<Option<(Fold, Option<u64>, Option<(u64, u64)>)>, AgentError> {
        let (result_id, result_position) = {
            let state = self.state.lock().unwrap();
            (state.result_id.clone(), state.result_position)
        };
        let Some(result_id) = result_id else {
            return Ok(None);
        };
        let mut publication = self.timeline.watch();
        while self.timeline.sequence() < result_position {
            if let Some(error) = self.timeline.failure() {
                return Err(error);
            }
            tokio::select! {
                result = publication.changed() => result.map_err(|_| AgentError::Closed("Goal".into()))?,
                _ = self.stop.cancelled() => return Err(AgentError::Closed(self.branch_id.clone())),
            }
        }
        let read_head = self.timeline.committed();
        let start = opening.map(|(sequence, _)| sequence);
        let mut base = self.result_at(&result_id, through, read_head).await?;
        let mut range_start = base.1.source_through.saturating_add(1);
        let mut range_end = through;
        let mut deleted = HashMap::<String, Deleted>::new();
        let mut invalidated_after = HashSet::new();
        loop {
            if range_start > range_end {
                break;
            }
            let mut query = request(
                &self.branch_id,
                Some(range_start),
                range_end,
                &["input/deleted"],
                &["position.sequence", "data.input_id"],
            );
            loop {
                let reply = self.timeline.query(query.clone()).await?;
                let QueryOutput::Rows { rows } = reply.result else {
                    unreachable!("deleted input identities")
                };
                for row in rows {
                    let [
                        QueryValue::Value { value: at },
                        QueryValue::Value { value: id },
                    ] = <[_; 2]>::try_from(row.values).expect("deletion fields")
                    else {
                        unreachable!("deletion values")
                    };
                    let at = at.as_u64().expect("deletion position");
                    invalidated_after.insert(at);
                    let id = id.as_str().expect("deleted input identity");
                    if let Some(prior) = deleted.get_mut(id) {
                        prior.at = prior.at.max(at);
                        continue;
                    }
                    let mut origin = request(
                        &self.branch_id,
                        start,
                        through,
                        &["input/message"],
                        &["position.sequence"],
                    );
                    origin.query.filter = Some(QueryFilter::And {
                        conditions: vec![origin.query.filter.take().unwrap(), eq("id", json!(id))],
                    });
                    origin.page.as_mut().unwrap().limit = NonZeroU64::MIN;
                    let QueryOutput::Rows { mut rows } = self.timeline.query(origin).await?.result
                    else {
                        unreachable!("deleted input position")
                    };
                    let Some(row) = rows.pop() else { continue };
                    let [QueryValue::Value { value: sequence }] =
                        <[_; 1]>::try_from(row.values).expect("input position")
                    else {
                        unreachable!("input position value")
                    };
                    let sequence = sequence.as_u64().expect("input position");
                    if sequence <= base.1.source_through {
                        let Some(before) =
                            self.before_input(id, sequence, through, read_head).await?
                        else {
                            return Ok(None);
                        };
                        base = before;
                    }
                    deleted.insert(id.to_owned(), Deleted { sequence, at });
                }
                query.page.as_mut().unwrap().after = reply.next;
                if query.page.as_ref().unwrap().after.is_none() {
                    break;
                }
            }
            let next = base.1.source_through.saturating_add(1);
            if next >= range_start {
                break;
            }
            range_end = range_start - 1;
            range_start = next;
        }
        if base.1.state.opening != start {
            return Err(AgentError::fault(
                "Goal",
                "deletion boundary opening mismatch",
            ));
        }
        let mut fold = base.1.state.fold;
        fold.driving = self.load_driving(base.1.state.driving_root, base.0).await?;
        invalidated_after.extend(deleted.values().map(|deleted| deleted.sequence));
        fold.invalidated_after = invalidated_after;
        let mut query = request(
            &self.branch_id,
            Some(base.1.source_through.saturating_add(1)),
            through,
            KEYS,
            &["id", "position.sequence", "kind"],
        );
        let mut sources = 0;
        loop {
            let reply = self.timeline.query(query.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("affected Goal sources")
            };
            for row in rows {
                let [
                    QueryValue::Value { value: id },
                    QueryValue::Value { value: sequence },
                    QueryValue::Value { value: kind },
                ] = <[_; 3]>::try_from(row.values).expect("affected source fields")
                else {
                    unreachable!("affected source values")
                };
                let sequence = sequence.as_u64().expect("source position");
                let kind = kind.as_str().expect("source kind");
                let id = id.as_str().expect("source identity");
                if kind == "input/deleted" || (kind == "input/message" && deleted.contains_key(id))
                {
                    fold.apply_source(sequence, Source::Ignored);
                } else {
                    self.read_source(&mut fold, id, sequence, kind, through, start)
                        .await?;
                }
                sources += 1;
            }
            query.page.as_mut().unwrap().after = reply.next;
            if query.page.as_ref().unwrap().after.is_none() {
                break;
            }
        }
        fold.finish_restore(start);
        let recorded = recorded.max(fold.record_after);
        let mut compensation = None;
        for (id, deleted) in &deleted {
            if deleted.at < from
                || deleted.sequence > recorded
                || compensation.is_some_and(|(at, _)| at >= deleted.at)
            {
                continue;
            }
            if self.input_matches(id, through, goal_inputs()).await? {
                let mut fields = self
                    .fields(id, "input/message", through, &["data.blocks"])
                    .await?;
                let QueryValue::Value { value } = fields.remove(0) else {
                    unreachable!("withdrawn Goal blocks")
                };
                let blocks: Vec<InputBlock> = serde_json::from_value(value)
                    .map_err(|error| AgentError::fault("Goal", error.to_string()))?;
                if fold::change_of(&blocks).is_some() {
                    compensation = Some((deleted.at, deleted.sequence));
                }
            }
        }
        tracing::debug!(
            sources,
            deleted = deleted.len(),
            "Goal deletion restored from an indexed input boundary"
        );
        Ok(Some((fold, start, compensation)))
    }
}
