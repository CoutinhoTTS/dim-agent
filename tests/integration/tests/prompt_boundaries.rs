use dim_agent_capabilities::{defaults, host::*, prompt_state::PromptSnapshot};
use dim_agent_runtime::*;
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypedPlugin};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct GatedPrompts {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl LoadPrompt for GatedPrompts {
    fn load(
        &self,
        target: PromptSourceTarget,
        _: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        Box::pin(async move {
            if target.source == "blocked" {
                self.started.notify_one();
                self.release.notified().await;
            }
            Ok(Some(prompt(
                &target.source,
                "1",
                &target.source,
                target.scope,
            )))
        })
    }
}

fn scope(timeline: &str) -> PromptScope {
    PromptScope::Branch {
        timeline: timeline.into(),
    }
}

fn prompt(source: &str, version: &str, body: &str, scope: PromptScope) -> PromptEntry {
    PromptEntry {
        source: source.into(),
        version: version.into(),
        body: body.into(),
        scope,
        delivery: PromptDelivery::Context,
        order: 0,
    }
}

fn assert_healthy(session: &Session) {
    let state = session.state("main").unwrap();
    assert!(!state.closed, "branch unexpectedly closed: {state:?}");
    assert!(
        state.failure.is_none(),
        "branch unexpectedly failed: {state:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_prompt_load_does_not_publish_or_poison_the_pending_load() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(GatedPrompts {
            started: started.clone(),
            release: release.clone(),
        }),
        None,
    );
    // The second request is rebound onto the pending operation's identity.
    builder.branch_part(|names| names.part(TamperControl("prompt/configure-request")));
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();

    let first = tokio::spawn({
        let session = session.clone();
        async move {
            session
                .configure(
                    ConfigureUpdate {
                        sources: vec![PromptSourceTarget {
                            source: "blocked".into(),
                            scope: scope("main"),
                        }],
                        model: None,
                    },
                    "main",
                )
                .await
        }
    });
    started.notified().await;
    let requests_before = session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::PromptConfigureRequest(_)))
        .count();
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        session.configure(
            ConfigureUpdate {
                sources: vec![PromptSourceTarget {
                    source: "other".into(),
                    scope: scope("main"),
                }],
                model: None,
            },
            "main",
        ),
    )
    .await
    .expect("registration rejection must be immediate");
    let requests_after = session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::PromptConfigureRequest(_)))
        .count();
    release.notify_one();

    assert!(matches!(second, Err(AgentError::InvalidInput(_))));
    assert_eq!(requests_after, requests_before);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), first)
            .await
            .expect("accepted prompt load did not settle")
            .unwrap()
            .unwrap(),
        ConfigureOutcome::Adopted
    );
    assert_healthy(&session);
    session
        .submit("still usable".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    assert_healthy(&session);
    agent.close().await.unwrap();
}

struct TamperControl(&'static str);

impl TypedPlugin for TamperControl {
    type Deps = (Keyed<Timeline>,);
    fn name(&self) -> &str {
        "TamperControl"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(timeline),): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            timeline.before(ctx, "TamperControl", &[self.0], |mut draft| {
                match &mut draft.kind {
                    EventKind::PromptConfigureRequest(data)
                    | EventKind::PromptConfigureResult(data) => {
                        data.operation_id = "unknown".into()
                    }
                    _ => unreachable!(),
                }
                Ok(draft)
            })?;
            Ok(Effect::Done)
        })
    }
}

#[tokio::test]
async fn rewritten_control_identity_is_registered_without_leaking_activity() {
    for key in ["prompt/configure-request", "prompt/configure-result"] {
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install_with_prompt(
            &mut builder,
            Arc::new(GatedPrompts {
                started: Arc::new(Notify::new()),
                release: Arc::new(Notify::new()),
            }),
            None,
        );
        builder.branch_part(move |names| names.part(TamperControl(key)));
        let agent = builder.build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            session.configure(
                ConfigureUpdate {
                    sources: vec![PromptSourceTarget {
                        source: "rules".into(),
                        scope: scope("main"),
                    }],
                    model: Some(ModelSelection {
                        provider_id: "mock".into(),
                        model_id: "rewritten-control".into(),
                        capability_snapshot: None,
                        config: None,
                        metadata: None,
                    }),
                },
                "main",
            ),
        )
        .await
        .expect("rewritten control must settle the caller");
        assert_eq!(result.unwrap(), ConfigureOutcome::Adopted);
        assert!(
            session
                .history("main", None)
                .await
                .unwrap()
                .iter()
                .any(|event| event.kind.key() == key
                    && matches!(&event.kind,
                    EventKind::PromptConfigureRequest(data) | EventKind::PromptConfigureResult(data)
                        if data.operation_id == "unknown"))
        );
        assert!(
            session
                .get::<PromptSnapshot>("main")
                .unwrap()
                .unwrap()
                .entries
                .contains_key(&("rules".into(), scope("main")))
        );
        session
            .submit("still usable".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), session.idle("main"))
            .await
            .unwrap()
            .unwrap();
        assert_healthy(&session);
        tokio::time::timeout(std::time::Duration::from_secs(1), agent.close())
            .await
            .unwrap()
            .unwrap();
    }
}

struct EndScopeBeforeAdoption(PromptScope);

impl TypedPlugin for EndScopeBeforeAdoption {
    type Deps = (Keyed<Timeline>, Keyed<Dispatcher>);
    fn name(&self) -> &str {
        "EndScopeBeforeAdoption"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(timeline), Keyed(dispatcher)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            timeline.append(
                EventKind::ContextScope(ContextScopeData {
                    scope: self.0.clone(),
                    status: ContextScopeStatus::Started,
                })
                .into(),
            )?;
            let scope = self.0.clone();
            dispatcher.subscribe(
                ctx,
                "EndScopeBeforeAdoption",
                &["prompt/configure-result"],
                move |_| {
                    let timeline = timeline.clone();
                    let scope = scope.clone();
                    Box::pin(async move {
                        timeline.append(
                            EventKind::ContextScope(ContextScopeData {
                                scope,
                                status: ContextScopeStatus::Ended,
                            })
                            .into(),
                        )?;
                        Ok(())
                    })
                },
            )?;
            Ok(Effect::Done)
        })
    }
}

#[tokio::test]
async fn scope_ended_in_committed_tail_supersedes_loading_result() {
    let scope = PromptScope::Task {
        timeline: "main".into(),
        id: "task".into(),
    };
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    let task_scope = scope.clone();
    builder.branch_part(move |names| names.part(EndScopeBeforeAdoption(task_scope.clone())));
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(GatedPrompts {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }),
        None,
    );
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let result = session
        .configure(
            ConfigureUpdate {
                sources: vec![PromptSourceTarget {
                    source: "task rules".into(),
                    scope,
                }],
                model: None,
            },
            "main",
        )
        .await
        .unwrap();
    assert_eq!(result, ConfigureOutcome::Superseded);
    session.idle("main").await.unwrap();
    assert!(
        !session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .any(|event| matches!(event.kind, EventKind::PromptChanged(_)))
    );
    assert_healthy(&session);
    agent.close().await.unwrap();
}
