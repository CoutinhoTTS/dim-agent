use super::*;
use crate::history::{
    chain_by_id, projected_message_refs, render_input, settled_request_output, tool_result_message,
};
use crate::host::ModelSettings;
use dim_agent_runtime::{QueryOrder, QueryOrderKey, TimelineQueryDirection};
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestBase {
    pub schema: String,
    pub request_id: String,
    pub source: TimelinePosition,
    pub algorithm: String,
    pub configuration: String,
    pub compression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_identity: Option<String>,
    pub local: f64,
    pub root: Option<Reference>,
}

fn decode<T: DeserializeOwned>(value: QueryValue) -> Result<T> {
    let QueryValue::Value { value } = value else {
        return Err(fault("missing required usage property"));
    };
    serde_json::from_value(value).map_err(|error| fault(error.to_string()))
}

fn optional<T: DeserializeOwned>(value: QueryValue) -> Result<Option<T>> {
    match value {
        QueryValue::Missing => Ok(None),
        value => decode(value).map(Some),
    }
}

fn elements(value: QueryValue) -> Result<Vec<QueryValue>> {
    let QueryValue::Elements { values } = value else {
        return Err(fault("missing usage array projection"));
    };
    Ok(values)
}

fn measured_prompt(usage: &Usage) -> Option<f64> {
    (usage.prompt_tokens.is_finite()
        && usage.prompt_tokens > 0.
        && usage.prompt_tokens
            >= usage.cache_read_tokens.unwrap_or(0.) + usage.cache_write_tokens.unwrap_or(0.))
    .then_some(usage.prompt_tokens)
}

impl ContextUsage {
    pub(super) async fn run(&self) -> Result<()> {
        let initial = self.timeline.sequence();
        let mut machine = self.restore(initial).await?;
        self.publish(&machine, initial);
        let mut covered = initial;
        loop {
            let wanted = self.wanted.load(Ordering::SeqCst);
            if wanted > covered {
                self.advance(&mut machine, wanted).await?;
                covered = wanted;
                self.publish(&machine, covered);
            } else {
                self.wake.notified().await;
            }
        }
    }

    async fn fields(&self, id: &str, through: u64, fields: &[&str]) -> Result<Vec<QueryValue>> {
        let QueryOutput::Rows { mut rows } = self
            .timeline
            .query(chain_by_id(
                id,
                through,
                QuerySelect::Fields {
                    fields: fields.iter().map(|field| (*field).into()).collect(),
                },
            ))
            .await?
            .result
        else {
            unreachable!("usage source properties")
        };
        rows.pop()
            .map(|row| row.values)
            .ok_or_else(|| fault("missing usage source"))
    }

    async fn newest(&self, mut query: TimelineQuery) -> Result<Option<Vec<QueryValue>>> {
        query.query.pick = Some(QueryPick::Last);
        query.page.as_mut().expect("last usage property page").limit = NonZeroU64::MIN;
        let QueryOutput::Rows { mut rows } = self.timeline.query(query).await?.result else {
            unreachable!("last usage properties")
        };
        Ok(rows.pop().map(|row| row.values))
    }

    async fn restore(&self, through: u64) -> Result<Machine> {
        let branches: Vec<_> = self
            .timeline
            .segments(through)?
            .into_iter()
            .map(|(branch, _)| branch)
            .collect();
        let opening = self
            .newest(request(
                TimelineQueryScope::Branch,
                through,
                &["branch/start", "session/forked"],
                &["position"],
            ))
            .await?;
        let opening = match opening {
            Some(mut row) => Some(decode::<TimelinePosition>(row.remove(0))?),
            None => None,
        };
        let mut saved = request(
            TimelineQueryScope::Branch,
            through,
            &[RESULT],
            &["position", "data"],
        );
        saved.from = opening.map(|mut position| {
            position.sequence += 1;
            position
        });
        let saved = if saved
            .from
            .as_ref()
            .is_some_and(|position| position.sequence > through)
        {
            None
        } else {
            self.newest(saved).await?
        };
        if let Some(row) = saved {
            let [position, data] = <[_; 2]>::try_from(row).expect("usage recovery fields");
            let position: TimelinePosition = decode(position)?;
            let data: Value = decode(data)?;
            if !matches!(
                data.get("schema").and_then(Value::as_str),
                Some("context-usage/v1" | SCHEMA)
            ) {
                return Err(fault("unsupported usage recovery schema"));
            }
            let saved: Recovered =
                serde_json::from_value(data).map_err(|error| fault(error.to_string()))?;
            if saved.source_through > through
                || saved.source_through >= position.sequence
                || saved.state.branches != branches
                || saved.state.branches.is_empty()
                || saved.state.initialized > saved.state.at(saved.source_through)
                || saved
                    .state
                    .root
                    .as_ref()
                    .is_some_and(|root| root.last > saved.state.at(saved.source_through))
            {
                return Err(fault("invalid usage recovery boundary"));
            }
            let numeric = Numeric::new(self.timeline.clone(), position.sequence);
            numeric.validate_root(saved.state.root.as_ref()).await?;
            total(&saved.state.summary())?;
            let configuration = saved.state.summary().configuration;
            let event_id = match &configuration {
                Some(Configuration::Ready { counter }) => Some(counter.event_id.as_str()),
                Some(
                    Configuration::Unsupported { event_id }
                    | Configuration::Unavailable { event_id, .. },
                ) => Some(event_id.as_str()),
                None => None,
            };
            let model = if let Some(id) = event_id {
                let request = self.counter_request(id, position.sequence).await?;
                let resolved = self
                    .resolver
                    .0
                    .as_ref()
                    .expect("bound usage resolver")
                    .resolve(Arc::new(request), self.cancel.clone())
                    .await;
                let matching = match (&configuration, &resolved) {
                    (Some(Configuration::Ready { counter }), Ok(model)) => model
                        .contribution_counter()
                        .is_some_and(|current| current.algorithm() == counter.algorithm),
                    (Some(Configuration::Unsupported { .. }), Ok(model)) => {
                        model.contribution_counter().is_none()
                    }
                    (Some(Configuration::Unavailable { .. }), Err(_)) => true,
                    _ => false,
                };
                if !matching {
                    let mut machine = self.initialize(through, branches).await?;
                    if through > 0 {
                        self.save(&mut machine).await?
                    }
                    return Ok(machine);
                }
                resolved.ok()
            } else {
                None
            };
            let mut machine = Machine {
                state: saved.state,
                source_through: saved.source_through,
                result: position.sequence,
                model,
                numeric,
            };
            self.advance(&mut machine, through).await?;
            if let (Some(Configuration::Ready { mut counter }), Some(model)) =
                (machine.state.summary().configuration, &machine.model)
            {
                let limits = Limits {
                    window: model.context_window(),
                    output: model.output_tokens(),
                    threshold: model.threshold_tokens(),
                };
                if [limits.window, limits.output, limits.threshold]
                    .iter()
                    .any(|value| !value.is_finite() || *value < 0.)
                {
                    return Err(fault("invalid restored usage limits"));
                }
                if limits != counter.limits {
                    counter.limits = limits;
                    let at = machine.state.at(through);
                    self.record(
                        &mut machine,
                        at,
                        Summary {
                            configuration: Some(Configuration::Ready { counter }),
                            ..Default::default()
                        },
                        true,
                    )
                    .await?;
                    machine.source_through = through;
                    self.save(&mut machine).await?;
                }
            }
            return Ok(machine);
        }
        let mut machine = self.initialize(through, branches).await?;
        if through > 0 {
            self.save(&mut machine).await?
        }
        Ok(machine)
    }

    async fn configuration_request(&self, id: &str, through: u64) -> Result<ModelRequest> {
        let [settings, system] = <[_; 2]>::try_from(
            self.fields(id, through, &["data.settings", "data.system"])
                .await?,
        )
        .expect("configuration fields");
        let settings: ModelConfiguration = decode(settings)?;
        Ok(ModelSettings {
            provider_id: settings.provider_id,
            model_id: settings.model_id,
            capability_snapshot: settings.capability_snapshot.clone(),
            config: crate::context_model::adopted_config(
                settings.capability_snapshot.as_ref(),
                settings.config.as_ref(),
            ),
            metadata: settings.metadata,
            system: optional(system)?,
            tools: settings.tools,
        }
        .request(vec![]))
    }

    pub(super) async fn counter_request(&self, id: &str, through: u64) -> Result<ModelRequest> {
        let mut kind = self.fields(id, through, &["kind"]).await?;
        let kind: String = decode(kind.remove(0))?;
        let prefix = match kind.as_str() {
            "context/configured" => "data.settings",
            "model/call" => "data.request",
            _ => return Err(fault("usage configuration references another source type")),
        };
        let paths = [
            "providerId",
            "modelId",
            "capabilitySnapshot",
            "config",
            "metadata",
        ]
        .map(|field| format!("{prefix}.{field}"));
        let fields: Vec<_> = paths.iter().map(String::as_str).collect();
        let [provider, model, capabilities, config, metadata] =
            <[_; 5]>::try_from(self.fields(id, through, &fields).await?)
                .expect("counter model controls");
        let mut request = ModelRequest::new(
            decode::<String>(provider)?,
            decode::<String>(model)?,
            vec![],
        );
        request.capability_snapshot = optional(capabilities)?;
        request.config = optional(config)?;
        request.metadata = optional(metadata)?;
        if kind == "context/configured" {
            request.config = crate::context_model::adopted_config(
                request.capability_snapshot.as_ref(),
                request.config.as_ref(),
            );
        }
        Ok(request)
    }

    async fn configure(
        &self,
        machine: &mut Machine,
        id: String,
        request: ModelRequest,
    ) -> Result<Configuration> {
        let control = ModelRequest {
            provider_id: request.provider_id.clone(),
            model_id: request.model_id.clone(),
            capability_snapshot: request.capability_snapshot.clone(),
            messages: Vec::new(),
            tools: None,
            system: None,
            config: request.config.clone(),
            metadata: request.metadata.clone(),
        };
        let model = match self
            .resolver
            .0
            .as_ref()
            .expect("bound usage resolver")
            .resolve(Arc::new(control.clone()), self.cancel.clone())
            .await
        {
            Ok(model) => model,
            Err(error) => {
                machine.model = None;
                return Ok(Configuration::Unavailable {
                    event_id: id,
                    message: error.to_string(),
                });
            }
        };
        let configuration = match model.contribution_counter() {
            None => Configuration::Unsupported { event_id: id },
            Some(counter) => {
                let limits = Limits {
                    window: model.context_window(),
                    output: model.output_tokens(),
                    threshold: model.threshold_tokens(),
                };
                if [limits.window, limits.output, limits.threshold]
                    .iter()
                    .any(|value| !value.is_finite() || *value < 0.)
                    || counter.algorithm().is_empty()
                {
                    return Err(fault("invalid usage counting contract"));
                }
                Configuration::Ready {
                    counter: Box::new(CounterConfiguration {
                        event_id: id,
                        request_identity: request_identity(&request, counter.algorithm())?,
                        algorithm: counter.algorithm().into(),
                        fixed: counter.fixed(
                            request.system.as_deref(),
                            request.tools.as_deref().map(Vec::as_slice),
                        ),
                        limits,
                    }),
                }
            }
        };
        machine.model = Some(model);
        Ok(configuration)
    }

    async fn initialize(&self, through: u64, branches: Vec<String>) -> Result<Machine> {
        let initialized = Key {
            branch: branches.len() as u32 - 1,
            sequence: through,
        };
        let mut machine = Machine {
            state: State {
                branches,
                initialized,
                root: None,
            },
            source_through: through,
            result: through,
            model: None,
            numeric: Numeric::new(self.timeline.clone(), through),
        };
        let configured = self
            .newest(request(
                TimelineQueryScope::Chain,
                through,
                &["context/configured"],
                &["id"],
            ))
            .await?;
        let mut legacy = None;
        let configuration = if let Some(mut row) = configured {
            let id: String = decode(row.remove(0))?;
            let value = self.configuration_request(&id, through).await?;
            Some(self.configure(&mut machine, id, value).await?)
        } else {
            let measured = self
                .newest(request(
                    TimelineQueryScope::Chain,
                    through,
                    &["model/usage"],
                    &["data.request_id", "data.usage"],
                ))
                .await?;
            let call = if let Some(fields) = measured {
                let [request_id, prompt] =
                    <[_; 2]>::try_from(fields).expect("legacy measurement properties");
                let request_id: String = decode(request_id)?;
                let usage: Usage = decode(prompt)?;
                if measured_prompt(&usage).is_some() {
                    let mut call = request(
                        TimelineQueryScope::Chain,
                        through,
                        &["model/call"],
                        &["id", "position", "data.request_id"],
                    );
                    call.query.filter = Some(QueryFilter::And {
                        conditions: vec![
                            call.query.filter.take().expect("legacy call type"),
                            QueryFilter::In {
                                value: QueryOperand::Field {
                                    path: "data.request_id".into(),
                                },
                                values: vec![request_id.into()],
                            },
                        ],
                    });
                    self.newest(call).await?
                } else {
                    None
                }
            } else {
                None
            };
            match call {
                Some(row) => {
                    let [id, position, request_id] =
                        <[_; 3]>::try_from(row).expect("legacy call identity");
                    let id: String = decode(id)?;
                    let mut fields = self.fields(&id, through, &["data.request"]).await?;
                    let request: ModelRequest = decode(fields.remove(0))?;
                    legacy = Some((
                        decode::<TimelinePosition>(position)?,
                        decode::<String>(request_id)?,
                        request.clone(),
                    ));
                    Some(self.configure(&mut machine, id, request).await?)
                }
                None => None,
            }
        };
        let mut summary = Summary {
            configuration,
            ..Default::default()
        };
        if matches!(summary.configuration, Some(Configuration::Ready { .. })) {
            let stream = self
                .history
                .rendered_stream(&self.timeline, through, 128)
                .await?;
            let counter = machine
                .model
                .as_ref()
                .and_then(|model| model.contribution_counter())
                .expect("ready usage counter");
            let legacy_prefix = if let Some((position, request_id, value)) = legacy {
                let compaction = stream
                    .compaction
                    .as_ref()
                    .and_then(|event| match &event.kind {
                        EventKind::ContextCompaction(data) => Some(data),
                        _ => None,
                    });
                let mut prefix = projected_message_refs(compaction, &stream.history);
                if value.messages.iter().all(|message| {
                    prefix
                        .next()
                        .is_some_and(|actual| actual == message.as_ref())
                }) {
                    let mut counted = counter.fixed(
                        value.system.as_deref(),
                        value.tools.as_deref().map(Vec::as_slice),
                    );
                    for message in value.messages {
                        counted = add(counted, counter.message(&message))?;
                    }
                    Some((position, request_id, counter.estimate(counted)))
                } else {
                    None
                }
            } else {
                None
            };
            for entry in stream.history {
                let contribution = counter.message(&entry.message);
                let at = machine.state.key(&entry.position)?;
                machine.state.root = Some(
                    machine
                        .numeric
                        .append(
                            machine.state.root.take(),
                            Entry {
                                at,
                                summary: Summary {
                                    ordinary: contribution,
                                    ..Default::default()
                                },
                            },
                        )
                        .await?,
                );
            }
            if let Some(event) = stream.compaction {
                let EventKind::ContextCompaction(data) = &event.kind else {
                    unreachable!("stream compaction")
                };
                let cut = stream
                    .compaction_cut
                    .ok_or_else(|| fault("missing initialized compaction cut"))?;
                let mut replacement = Contribution::default();
                for message in projected_message_refs(Some(data), &[]) {
                    replacement = add(replacement, counter.message(message))?;
                }
                summary.compression = Some(Compression {
                    event_id: event.id.clone(),
                    cut: machine.state.key(&cut)?,
                    removed: Contribution::default(),
                    replacement,
                });
            }
            if let Some((position, request_id, local)) = legacy_prefix {
                let mut usage = request(
                    TimelineQueryScope::Chain,
                    through,
                    &["model/usage"],
                    &["data.usage"],
                );
                usage.query.filter = Some(QueryFilter::And {
                    conditions: vec![
                        usage.query.filter.take().expect("legacy usage type"),
                        QueryFilter::In {
                            value: QueryOperand::Field {
                                path: "data.request_id".into(),
                            },
                            values: vec![request_id.clone().into()],
                        },
                    ],
                });
                if let Some(mut fields) = self.newest(usage).await? {
                    let usage: Usage = decode(fields.remove(0))?;
                    if let Some(prompt_tokens) = measured_prompt(&usage)
                        && local.is_finite()
                        && local >= 0.
                    {
                        let Some(Configuration::Ready { counter }) = &summary.configuration else {
                            unreachable!("legacy ready configuration")
                        };
                        let request_at = machine.state.key(&position)?;
                        summary.calibration = Some(Calibration {
                            request_id,
                            request_at,
                            anchor_at: request_at,
                            configuration: counter.identity()?,
                            compression: summary
                                .compression
                                .as_ref()
                                .map(|compression| compression.event_id.clone()),
                            request_identity: None,
                            local,
                            prompt_tokens,
                        });
                    }
                }
            }
        }
        if summary.configuration.is_some() {
            self.record(&mut machine, initialized, summary, true)
                .await?;
        }
        Ok(machine)
    }

    async fn record(
        &self,
        machine: &mut Machine,
        at: Key,
        mut entry: Summary,
        estimate: bool,
    ) -> Result<()> {
        if entry.ordinary == Contribution::default()
            && entry.configuration.is_none()
            && entry.compression.is_none()
            && entry.calibration.is_none()
        {
            return Ok(());
        }
        if estimate {
            let mut next = machine.state.summary();
            next.join(&entry)?;
            if matches!(next.configuration, Some(Configuration::Ready { .. })) {
                let counter = machine
                    .model
                    .as_ref()
                    .and_then(|model| model.contribution_counter())
                    .ok_or_else(|| fault("missing ready usage counter"))?;
                let local = counter.estimate(total(&next)?);
                if !local.is_finite() || local < 0. {
                    return Err(fault("invalid local usage estimate"));
                }
                entry.local = Some(local);
            }
        }
        machine.state.root = Some(
            machine
                .numeric
                .append(machine.state.root.take(), Entry { at, summary: entry })
                .await?,
        );
        Ok(())
    }

    async fn advance(&self, machine: &mut Machine, through: u64) -> Result<()> {
        if machine.source_through >= through {
            return Ok(());
        }
        let mut sources = request(
            TimelineQueryScope::Branch,
            through,
            SOURCES,
            &["id", "kind", "position"],
        );
        sources.from = Some(TimelinePosition {
            branch_id: self.timeline.branch_id(),
            sequence: machine.source_through + 1,
        });
        sources.query.order_by = vec![QueryOrder {
            key: QueryOrderKey::Position,
            direction: TimelineQueryDirection::Ascending,
        }];
        loop {
            let reply = self.timeline.query(sources.clone()).await?;
            let QueryOutput::Rows { rows } = reply.result else {
                unreachable!("usage source identities")
            };
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let [id, kind, position] =
                    <[_; 3]>::try_from(row.values).expect("usage source identity");
                let id: String = decode(id)?;
                let kind: String = decode(kind)?;
                let position: TimelinePosition = decode(position)?;
                self.consume(machine, &id, &kind, position.sequence).await?;
                machine.source_through = position.sequence;
            }
            self.save(machine).await?;
            let Some(next) = reply.next else { break };
            sources.page.as_mut().unwrap().after = Some(next);
        }
        Ok(())
    }

    async fn consume(
        &self,
        machine: &mut Machine,
        id: &str,
        kind: &str,
        through: u64,
    ) -> Result<()> {
        let at = machine.state.at(through);
        if kind == "context/configured" {
            let request = self.configuration_request(id, through).await?;
            let previous = machine.state.summary().configuration;
            let next = self.configure(machine, id.into(), request).await?;
            let same_algorithm = match (&previous, &next) {
                (
                    Some(Configuration::Ready { counter: old }),
                    Configuration::Ready { counter: new },
                ) => old.algorithm == new.algorithm,
                _ => false,
            };
            if !same_algorithm && matches!(next, Configuration::Ready { .. }) {
                *machine = self
                    .initialize(through, machine.state.branches.clone())
                    .await?;
                return Ok(());
            }
            return self
                .record(
                    machine,
                    at,
                    Summary {
                        configuration: Some(next),
                        ..Default::default()
                    },
                    true,
                )
                .await;
        }
        if kind == "model/usage" {
            return self.calibrate(machine, id, through).await;
        }
        let Some(model) = machine.model.clone() else {
            return Ok(());
        };
        let Some(counter) = model.contribution_counter() else {
            return Ok(());
        };
        let mut entry = Summary::default();
        match kind {
            "input/consumed" => {
                let mut fields = self.fields(id, through, &["data.input_id"]).await?;
                let input: String = decode(fields.remove(0))?;
                let mut fields = self.fields(&input, through, &["data"]).await?;
                let input: InputMessageData = decode(fields.remove(0))?;
                for projection in render_input(id, &input) {
                    entry.ordinary = add(entry.ordinary, counter.message(&projection.message))?;
                }
            }
            "step/model-output-end" => {
                let [request, status] = <[_; 2]>::try_from(
                    self.fields(id, through, &["data.request_id", "data.status"])
                        .await?,
                )
                .expect("output settlement fields");
                let request: String = decode(request)?;
                let status: String = decode(status)?;
                if let Some(message) = settled_request_output(
                    &self.timeline,
                    &request,
                    through - 1,
                    status == "completed",
                    false,
                )
                .await?
                {
                    entry.ordinary = counter.message(&message);
                }
            }
            "step/tool-result" => {
                let [request, call, calls, contents, errors, contexts] = <[_; 6]>::try_from(
                    self.fields(
                        id,
                        through,
                        &[
                            "data.request_id",
                            "data.call_event_id",
                            "data.results[].call_id",
                            "data.results[].result.content",
                            "data.results[].result.isError",
                            "data.results[].result.contextMessages",
                        ],
                    )
                    .await?,
                )
                .expect("tool result properties");
                let request: String = decode(request)?;
                if optional::<String>(call)?.is_none()
                    && let Some(message) =
                        settled_request_output(&self.timeline, &request, through - 1, false, true)
                            .await?
                {
                    entry.ordinary = add(entry.ordinary, counter.message(&message))?
                }
                let calls = elements(calls)?
                    .into_iter()
                    .map(decode::<String>)
                    .collect::<Result<Vec<_>>>()?;
                let contents = elements(contents)?
                    .into_iter()
                    .map(decode::<ToolContent>)
                    .collect::<Result<Vec<_>>>()?;
                let errors = elements(errors)?
                    .into_iter()
                    .map(optional::<bool>)
                    .collect::<Result<Vec<_>>>()?;
                let contexts = elements(contexts)?
                    .into_iter()
                    .map(optional::<Vec<Message>>)
                    .collect::<Result<Vec<_>>>()?;
                if [contents.len(), errors.len(), contexts.len()]
                    .iter()
                    .any(|length| *length != calls.len())
                {
                    return Err(fault("unaligned tool result properties"));
                }
                for (((call, content), error), context) in
                    calls.into_iter().zip(contents).zip(errors).zip(contexts)
                {
                    entry.ordinary = add(
                        entry.ordinary,
                        counter.message(&tool_result_message(&call, &content, error)),
                    )?;
                    for message in context.into_iter().flatten() {
                        entry.ordinary = add(entry.ordinary, counter.message(&message))?;
                    }
                }
            }
            "context/compaction" => {
                let mut fields = self.fields(id, through, &["data.status"]).await?;
                if decode::<String>(fields.remove(0))? != "completed" {
                    return Ok(());
                }
                let [cut, replacement, baseline] = <[_; 3]>::try_from(
                    self.fields(
                        id,
                        through,
                        &[
                            "data.retain_from",
                            "data.replacement",
                            "data.context_baseline",
                        ],
                    )
                    .await?,
                )
                .expect("compaction properties");
                let cut: String = decode(cut)?;
                let mut fields = self.fields(&cut, through, &["position"]).await?;
                let cut: TimelinePosition = decode(fields.remove(0))?;
                let cut = machine.state.key(&cut)?;
                let previous = machine.state.summary().compression;
                if previous.as_ref().is_some_and(|previous| previous.cut > cut) {
                    return Err(fault("compaction cut precedes the active cut"));
                }
                let replacement: Vec<Message> = decode(replacement)?;
                let baseline: Vec<ContextBaselineEntry> = optional(baseline)?.unwrap_or_default();
                let mut contribution = Contribution::default();
                for message in replacement
                    .iter()
                    .chain(baseline.iter().map(|entry| entry.message.as_ref()))
                {
                    contribution = add(contribution, counter.message(message))?;
                }
                let removed = machine
                    .numeric
                    .prefix(machine.state.root.as_ref(), cut, false)
                    .await?
                    .ordinary;
                entry.compression = Some(Compression {
                    event_id: id.into(),
                    cut,
                    removed,
                    replacement: contribution,
                });
            }
            _ => unreachable!("declared usage source"),
        }
        self.record(machine, at, entry, true).await
    }

    async fn calibrate(&self, machine: &mut Machine, id: &str, through: u64) -> Result<()> {
        if machine.state.summary().configuration.is_none() {
            *machine = self
                .initialize(through, machine.state.branches.clone())
                .await?;
            return Ok(());
        }
        let [request_id, prompt] = <[_; 2]>::try_from(
            self.fields(id, through, &["data.request_id", "data.usage"])
                .await?,
        )
        .expect("provider usage properties");
        let request_id: String = decode(request_id)?;
        let usage: Usage = decode(prompt)?;
        let Some(prompt_tokens) = measured_prompt(&usage) else {
            return Ok(());
        };
        let mut query = request(
            TimelineQueryScope::Chain,
            through,
            &["context-usage/request-base"],
            &["data", "position"],
        );
        query.query.filter = Some(QueryFilter::And {
            conditions: vec![
                query.query.filter.take().expect("request base type"),
                QueryFilter::In {
                    value: QueryOperand::Field {
                        path: "data.request_id".into(),
                    },
                    values: vec![request_id.clone().into()],
                },
            ],
        });
        let Some(fields) = self.newest(query).await? else {
            let summary = machine.state.summary();
            if let Some(mut calibration) = summary.calibration
                && calibration.request_id == request_id
            {
                calibration.prompt_tokens = prompt_tokens;
                self.record(
                    machine,
                    machine.state.at(through),
                    Summary {
                        calibration: Some(calibration),
                        ..Default::default()
                    },
                    false,
                )
                .await?;
            }
            return Ok(());
        };
        let [data, position] = <[_; 2]>::try_from(fields).expect("request base properties");
        let base: RequestBase = decode(data)?;
        let position: TimelinePosition = decode(position)?;
        if !matches!(
            base.schema.as_str(),
            "context-usage/request-base/v1" | "context-usage/request-base/v2"
        ) {
            return Err(fault("unsupported usage request base schema"));
        }
        if base.request_id != request_id || !base.local.is_finite() || base.local < 0. {
            return Err(fault("invalid usage request base"));
        }
        let summary = machine.state.summary();
        let Some(Configuration::Ready { counter }) = &summary.configuration else {
            return Ok(());
        };
        let request_at = machine.state.key(&base.source)?;
        let anchor_at = machine.state.key(&position)?;
        if request_at < machine.state.initialized
            || request_at > anchor_at
            || anchor_at > machine.state.at(through)
            || base.configuration != counter.identity()?
            || base.algorithm != counter.algorithm
            || base.compression.as_deref()
                != summary
                    .compression
                    .as_ref()
                    .map(|compression| compression.event_id.as_str())
            || summary.calibration.as_ref().is_some_and(|current| {
                (current.request_at, current.anchor_at) > (request_at, anchor_at)
            })
        {
            return Ok(());
        }
        self.record(
            machine,
            machine.state.at(through),
            Summary {
                calibration: Some(Calibration {
                    request_id,
                    request_at,
                    anchor_at,
                    configuration: base.configuration,
                    compression: base.compression,
                    request_identity: base.request_identity,
                    local: base.local,
                    prompt_tokens,
                }),
                ..Default::default()
            },
            false,
        )
        .await
    }
}
