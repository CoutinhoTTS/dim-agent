//! #548: the input guide composed from the entries in force
//! ([Prompt §3.3 and §3.4](../../../docs/design/plugins/Prompt.md#33-one-composition-two-paths)).

mod local_support;

use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel},
    defaults,
    history::HistoryRenderer,
    host::*,
    host_platform::HostPlatform,
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules},
    prompt_guide::{GuideEntries, GuideEntry, GuideScope},
    prompt_sources,
    workspace_tools::{self, WorkspaceToolsConfig},
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use local_support::{Dirs, platform, run, system_text};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, FiberView, Key, TypeKey};
use rutis::{Ctx, Plugin};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// The frame's first paragraph: the guide opens with it.
const FRAME_FIRST: &str = "# How to read your input";
/// The frame's last paragraph: the guide ends with it.
const FRAME_LAST: &str = "they do not renew any permission.";

/// The entry the probe registers between adoptions.
const PROBE_ENTRY: &str = r#"<dim-probe source="test">: an entry a contributor registered for this flow; the composition places it between the state and the rule entries."#;

fn configuration(model: &str) -> ModelSelection {
    ModelSelection {
        provider_id: "test".into(),
        model_id: model.into(),
        capability_snapshot: Some(ModelCapabilitySnapshot {
            target: ModelId {
                provider_id: "test".into(),
                model_id: model.into(),
            },
            capabilities: ModelCapabilities {
                context_window: 135791,
                max_output_tokens: 6789,
                ..Default::default()
            },
            reasoning: None,
            service_tier: None,
        }),
        config: None,
        metadata: None,
    }
}

/// The host bindings every assembly here shares, minus the tool host: the
/// workspace tools provide it.
fn mock_model_hosts(builder: &mut AgentBuilder) {
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .process_value(Arc::new(ModelHost(Arc::new(EchoModel))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .require_feature(
            Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
            }),
        );
}

/// The process root with the platform the workspace tools read.
fn platform_root(dirs: &Dirs) -> Ctx {
    let root = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    root.provide_as::<HostPlatform>(Arc::new(platform(&dirs.home)))
        .unwrap();
    root
}

fn workspace(builder: &mut AgentBuilder, dirs: &Dirs) {
    workspace_tools::install(
        builder,
        WorkspaceToolsConfig {
            output_root: dirs.dir("output"),
        },
    );
}

fn work_config(dirs: &Dirs) -> SessionConfig {
    SessionConfig {
        cwd: dirs.dir("work").display().to_string(),
        tools: Some(vec!["exec".into()]),
        ..SessionConfig::default()
    }
}

/// The system text of the last request on `branch`.
async fn last_system(session: &Session, branch: &str) -> String {
    let events = session.history(branch, None).await.unwrap();
    let call = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no model call on {branch}"));
    system_text(
        call.system
            .as_deref()
            .unwrap_or_else(|| panic!("no system on {branch}")),
    )
}

/// The guide a system text carries: from the frame's first paragraph on.
fn guide_of(system: &str) -> &str {
    let at = system
        .find(FRAME_FIRST)
        .unwrap_or_else(|| panic!("the guide is missing from: {system:?}"));
    &system[at..]
}

fn assert_guide_shape(guide: &str, markers: &[&str]) {
    assert_eq!(
        guide.matches(FRAME_FIRST).count(),
        1,
        "one composition: {guide:?}"
    );
    assert!(guide.ends_with(FRAME_LAST), "{guide:?}");
    for marker in markers {
        assert!(
            guide.contains(marker),
            "{marker} entry missing from: {guide:?}"
        );
    }
}

/// One composition: a request on the adopted path and a request on the
/// host-prepared path carry the same frame and the same entries, because
/// both call the same composition over the same loaded writers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_paths_carry_one_composition() {
    let dirs = Dirs::new();

    // The adopted path: PromptSources supplies the prompt host, the Session
    // adopts with it, and the System comes from `context/configured`.
    let root = platform_root(&dirs);
    let mut builder = AgentBuilder::new(AgentConfig::default());
    mock_model_hosts(&mut builder);
    builder.session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))));
    defaults::install_with_prompt_capability(&mut builder, ModelProviderPlugin::new);
    prompt_sources::install_with_clock(
        &mut builder,
        Arc::new(|| "2026-09-30".into()),
        Duration::from_secs(60),
    );
    workspace(&mut builder, &dirs);
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig {
            system_prompt: "configured rules".into(),
            ..work_config(&dirs)
        })
        .await
        .unwrap();
    session
        .configure(
            ConfigureUpdate {
                sources: Vec::new(),
                model: Some(configuration("configured")),
            },
            "main",
        )
        .await
        .unwrap();
    run(&session, "hello").await;
    let events = session.history("main", None).await.unwrap();
    let adopted_call = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        adopted_call.configuration.is_some(),
        "the adopted path carries the adopted System"
    );
    let published = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ContextConfigured(data) => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        adopted_call.request.system.as_deref(),
        published.system.as_ref(),
        "the request carries the System the branch adopted"
    );
    let adopted = system_text(adopted_call.request.system.as_deref().unwrap());
    agent.close().await.unwrap();

    // The host-prepared path: the Session never adopts, and Loop composes
    // into the request it assembles.
    let root = platform_root(&dirs);
    let mut builder = AgentBuilder::new(AgentConfig::default());
    mock_model_hosts(&mut builder);
    workspace(&mut builder, &dirs);
    defaults::install_with_prompt(&mut builder, Arc::new(EmptyPromptHost), None);
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig {
            system_prompt: "host rules".into(),
            ..work_config(&dirs)
        })
        .await
        .unwrap();
    run(&session, "hello").await;
    let events = session.history("main", None).await.unwrap();
    let hosted_call = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        hosted_call.configuration.is_none(),
        "the host-prepared path has no adopted configuration"
    );
    let hosted = system_text(hosted_call.request.system.as_deref().unwrap());
    agent.close().await.unwrap();

    assert!(
        adopted.contains("configured rules\n\n# How to read your input"),
        "{adopted:?}"
    );
    assert!(
        hosted.starts_with("host rules\n\n# How to read your input"),
        "{hosted:?}"
    );
    let (adopted, hosted) = (guide_of(&adopted), guide_of(&hosted));
    assert_eq!(adopted, hosted, "one composition on both paths");
    assert_guide_shape(
        adopted,
        &[
            "<dim-notification",
            "<dim-branch-line",
            "<dim-branch-message",
            "<dim-state source=\"dim/environment/date\">",
            "<dim-rule source=\"dim/instructions/project\"",
        ],
    );
}

/// A registration made between adoptions waits for the next adoption that
/// renders the System — the first configuration or a change of sources: it
/// does not change the System a branch already adopted, and that adoption
/// composes it into the new System.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_entry_waits_for_the_next_adoption_that_renders_the_system() {
    let target = PromptSourceTarget {
        source: "rules".into(),
        scope: PromptScope::Branch {
            timeline: "main".into(),
        },
    };
    let hosts = Arc::new(Prompts(Mutex::new(BTreeMap::from([(
        "rules".to_string(),
        PromptEntry {
            source: "rules".into(),
            version: "1".into(),
            body: "rules one".into(),
            scope: PromptScope::Branch {
                timeline: "main".into(),
            },
            delivery: PromptDelivery::System,
            order: 0,
        },
    )]))));
    let flag = Arc::new(AtomicBool::new(false));
    let views = Arc::new(Mutex::new(Vec::new()));
    let (flag_owner, views_owner) = (flag.clone(), views.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install_with_prompt(&mut builder, hosts.clone(), None);
    builder.require_feature(Feature::new("GuideProbeOwner").session(
        "GuideProbeOwner",
        move |names| {
            Ok(Box::new(GuideProbeOwner {
                flag: flag_owner.clone(),
                views: views_owner.clone(),
                key: names.key()?,
            }))
        },
    ));
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert_eq!(
        session
            .configure(
                ConfigureUpdate {
                    sources: vec![target.clone()],
                    model: Some(configuration("one")),
                },
                "main",
            )
            .await
            .unwrap(),
        ConfigureOutcome::Adopted
    );
    run(&session, "first").await;
    let first = last_system(&session, "main").await;
    run(&session, "second").await;
    let second = last_system(&session, "main").await;
    assert_eq!(first, second, "two turns between adoptions share a prefix");
    assert_guide_shape(
        guide_of(&second),
        &[
            "<dim-branch-line",
            "<dim-state source=\"dim/environment/date\">",
        ],
    );
    assert!(!second.contains(PROBE_ENTRY));

    // A contributor's new generation registers an entry in between.
    flag.store(true, Ordering::SeqCst);
    let view = views.lock().unwrap().last().cloned().expect("a probe");
    view.restart().await.unwrap();
    run(&session, "third").await;
    let third = last_system(&session, "main").await;
    assert_eq!(
        third, second,
        "a registration made between adoptions waits for the next adoption that renders the System"
    );
    assert!(!third.contains(PROBE_ENTRY));

    // The next adoption that renders the System composes the entry into the
    // System it publishes.
    hosts.0.lock().unwrap().insert(
        "rules".to_string(),
        PromptEntry {
            source: "rules".into(),
            version: "2".into(),
            body: "rules one".into(),
            scope: PromptScope::Branch {
                timeline: "main".into(),
            },
            delivery: PromptDelivery::System,
            order: 0,
        },
    );
    assert_eq!(
        session
            .configure(
                ConfigureUpdate {
                    sources: vec![target.clone()],
                    model: None,
                },
                "main",
            )
            .await
            .unwrap(),
        ConfigureOutcome::Adopted
    );
    run(&session, "fourth").await;
    let fourth = last_system(&session, "main").await;
    assert!(
        fourth.contains(PROBE_ENTRY),
        "the adoption carries the entry: {fourth:?}"
    );
    assert_eq!(
        fourth.replacen(&format!("{PROBE_ENTRY}\n\n"), "", 1),
        third,
        "the new System differs from the last one only by the registered entry"
    );

    // The withdrawal ends the same way: fixed for this System, gone at the
    // next adoption that renders the System.
    flag.store(false, Ordering::SeqCst);
    let view = views.lock().unwrap().last().cloned().expect("a probe");
    view.restart().await.unwrap();
    run(&session, "fifth").await;
    assert_eq!(
        last_system(&session, "main").await,
        fourth,
        "a withdrawal made in between waits for the next adoption that renders the System"
    );
    hosts.0.lock().unwrap().insert(
        "rules".to_string(),
        PromptEntry {
            source: "rules".into(),
            version: "3".into(),
            body: "rules one".into(),
            scope: PromptScope::Branch {
                timeline: "main".into(),
            },
            delivery: PromptDelivery::System,
            order: 0,
        },
    );
    assert_eq!(
        session
            .configure(
                ConfigureUpdate {
                    sources: vec![target.clone()],
                    model: None,
                },
                "main",
            )
            .await
            .unwrap(),
        ConfigureOutcome::Adopted
    );
    run(&session, "sixth").await;
    let sixth = last_system(&session, "main").await;
    assert!(!sixth.contains(PROBE_ENTRY), "{sixth:?}");
    assert_eq!(
        sixth, third,
        "the withdrawal ends at the next adoption that renders the System"
    );
    agent.close().await.unwrap();
}

/// A minimal prompt host: one entry per source, replaced by the test.
struct Prompts(Mutex<BTreeMap<String, PromptEntry>>);
impl LoadPrompt for Prompts {
    fn load(
        &self,
        target: PromptSourceTarget,
        _: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        let entry = self.0.lock().unwrap().get(&target.source).cloned();
        Box::pin(async move { Ok(entry) })
    }
}

/// The owner of the probe part: it mounts the probe and keeps its view so
/// the test can reload one generation of it.
struct GuideProbeOwner {
    flag: Arc<AtomicBool>,
    views: Arc<Mutex<Vec<FiberView>>>,
    key: Key<GuideEntries>,
}
impl Plugin for GuideProbeOwner {
    type Deps = ();
    async fn apply(&self, ctx: &Ctx, _: ()) -> rutis::Result {
        let view = ctx.plugin(GuideProbe {
            flag: self.flag.clone(),
            key: self.key.clone(),
        });
        (&view).await.map_err(|error| {
            rutis::Error::from(rutis::host::ErrorKind::PluginFailed(
                format!("guide probe failed: {error}").into(),
            ))
        })?;
        self.views.lock().unwrap().push(view);
        Ok(())
    }
}

/// A contributor whose registration depends on the test's flag: with it
/// set, each generation registers an entry from its own `apply`.
struct GuideProbe {
    flag: Arc<AtomicBool>,
    key: Key<GuideEntries>,
}
impl DynPlugin for GuideProbe {
    fn name(&self) -> &str {
        "GuideProbe"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.key.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let table = ctx.require_at(&self.key)?;
            if self.flag.load(Ordering::SeqCst) {
                table.register(
                    ctx,
                    GuideScope::Session,
                    GuideEntry {
                        markers: vec!["dim-probe".into()],
                        order: 65,
                        text: PROBE_ENTRY.into(),
                    },
                )?;
            }
            Ok(Effect::Done)
        })
    }
}

/// F1: two branches each carry their own branch-scoped entry. Each line's
/// guide holds the entry exactly once, and ending one line leaves the
/// other's guide byte-identical.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_branch_keeps_its_own_entries() {
    let dirs = Dirs::new();
    let root = platform_root(&dirs);
    let mut builder = AgentBuilder::new(AgentConfig::default());
    mock_model_hosts(&mut builder);
    workspace(&mut builder, &dirs);
    defaults::install_with_prompt(&mut builder, Arc::new(EmptyPromptHost), None);
    let agent = builder.build_in(&root);
    let session = agent.create_session(work_config(&dirs)).await.unwrap();
    run(&session, "on main").await;
    let main = last_system(&session, "main").await;
    assert_eq!(
        main.matches("<dim-notification").count(),
        1,
        "main carries its own exec entry once: {main:?}"
    );

    // Opening a second line loads that line's own writers, which register
    // the same markers under that line's scope.
    let at = session.sequence("main").unwrap();
    let child = session
        .branch(
            "main".into(),
            at,
            BranchOptions {
                prompt: "on the line".into(),
                source: BranchSource::Host {
                    operation_id: "child".into(),
                },
            },
        )
        .await
        .expect("the second line loads its own branch-scoped entries");
    session.idle(&child).await.unwrap();
    let line = last_system(&session, &child).await;
    assert_eq!(
        line.matches("<dim-notification").count(),
        1,
        "the line carries its own exec entry once, not both lines': {line:?}"
    );
    assert_eq!(
        guide_of(&line),
        guide_of(&main),
        "both lines load the same writers, so their guides compose the same text"
    );

    // Ending the line withdraws its entries and leaves main's unchanged.
    session
        .end_branch(
            &child,
            BranchSource::Host {
                operation_id: "end".into(),
            },
        )
        .await
        .unwrap();
    run(&session, "after the line ended").await;
    let after = last_system(&session, "main").await;
    assert_eq!(
        after, main,
        "ending another line leaves this line's guide unchanged"
    );
    agent.close().await.unwrap();
}
