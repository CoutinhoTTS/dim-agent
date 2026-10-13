//! Reproducible acceptance measurements over the public TimelineStore API.
mod allocation {
    include!("../../../xtask/src/allocation.rs");
    pub fn reset_peak() {
        PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}
#[path = "timeline_acceptance/durable_vfs.rs"]
mod durable_vfs;
#[global_allocator]
static ALLOCATOR: allocation::MeasuredAllocator = allocation::MeasuredAllocator;

use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel, MockTools},
    defaults,
    history::HistoryRenderer,
    host::*,
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use dim_app_server::storage::{SqliteStorage, SqliteTimelineStore};
use rusqlite::Connection;
use rutis::{CordisError, Ctx, Effect, TypedPlugin};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

type Error = Box<dyn std::error::Error + Send + Sync>;
fn key(session: &str) -> TimelineKey {
    TimelineKey {
        session_id: session.into(),
        branch_id: "main".into(),
    }
}
fn record(sequence: u64, kind: EventKind) -> StoredEvent {
    StoredEvent {
        event: Arc::new(AgentEvent {
            id: uuid::Uuid::new_v4().to_string(),
            sequence,
            create_at: dim_agent_runtime::core::unix_millis(),
            tags: None,
            kind,
        }),
        trace: None,
        batch_end: sequence,
    }
}
async fn committed(
    store: &SqliteTimelineStore,
    session: &str,
    records: Vec<StoredEvent>,
) -> Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    store.append(
        key(session),
        records,
        Box::new(move |result| {
            let _ = sender.send(result);
        }),
    )?;
    receiver
        .await
        .map_err(|error| AgentError::Storage(error.to_string()))?
}
fn directory(label: &str) -> PathBuf {
    let root = std::env::var_os("TIMELINE_ACCEPTANCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/mnt/data/timeline-acceptance"));
    let directory = root.join(format!("{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}
fn quantile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)]
}

struct CaptureTree {
    scope: SessionScope,
    captured: Arc<Mutex<Option<Arc<TreeCommands>>>>,
}
impl TypedPlugin for CaptureTree {
    type Deps = InSession<TreeCommands>;
    fn name(&self) -> &str {
        "measurement.tree"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        InSession(tree): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            *self.captured.lock().unwrap() = Some(tree);
            Ok(Effect::Done)
        })
    }
}
impl Capability for CaptureTree {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
struct Compactor {
    samples: Arc<Mutex<PreparationSamples>>,
}
struct PreparationSamples {
    count: usize,
    warm: [usize; 100],
    recent: [usize; 100],
}
impl ResolveContextModel for Compactor {
    fn resolve(
        &self,
        _: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>> {
        let live = allocation::live();
        let mut samples = self.samples.lock().unwrap();
        let index = samples.count;
        if index < 100 {
            samples.warm[index] = live;
        }
        samples.recent[index % 100] = live;
        samples.count += 1;
        Box::pin(async { Ok(Arc::new(BoundedContext) as Arc<dyn ContextModel>) })
    }
}
struct BoundedContext;
impl ContextModel for BoundedContext {
    fn context_window(&self) -> f64 {
        300_000.
    }
    fn output_tokens(&self) -> f64 {
        64.
    }
    fn threshold_tokens(&self) -> f64 {
        200_256.
    }
    fn count_tokens(&self, request: &ModelRequest) -> f64 {
        (serde_json::to_vec(&request.messages).unwrap().len()
            + serde_json::to_vec(&request.system).unwrap().len()) as f64
            / 4.
    }
    fn compact(
        &self,
        input: CompactionInput,
        _: CancellationToken,
    ) -> HostFuture<'_, ContextCompactionResult> {
        Box::pin(async move {
            let diagnostics = serde_json::from_value(serde_json::json!({
                "reason":"auto", "candidateMessageCount":input.history.len(), "inputMessageCount":input.history.len(), "retainedMessageCount":0,
                "retainedTokenBudget":0,"retainedTokens":0,"serializedChars":100,"estimatedInputTokens":input.estimated_input_tokens,
                "estimatedCompressedTokens":20,"estimatedPostCompactionTokens":20,"summaryInputBudget":1000,
                "preflightInputTokens":100,"preflightOutputTokens":20,"compactedToolPayloadCount":0,"droppedMessageCount":0,
                "chunkCount":1,"summaryChars":5,"estimatedSummaryTokens":5,"modelCallCount":1,"durationMs":1
            })).unwrap();
            Ok(ContextCompactionResult::Completed {
                cursor: input.history.last().unwrap().id.clone(),
                summary: "Prior turns completed.".into(),
                diagnostics: Box::new(diagnostics),
                context_baseline: vec![],
                checkpoint: None,
            })
        })
    }
}
struct CaptureStop {
    scope: BranchScope,
    observed: Arc<Mutex<Vec<Instant>>>,
}
impl TypedPlugin for CaptureStop {
    type Deps = InBranch<Dispatcher>;
    fn name(&self) -> &str {
        "measurement.stop"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        InBranch(dispatcher): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let observed = self.observed.clone();
            dispatcher.subscribe(ctx, self.name(), &["stop/interrupt"], move |_| {
                observed.lock().unwrap().push(Instant::now());
                Box::pin(async { Ok(()) })
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for CaptureStop {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
async fn memory(steps: usize) -> std::result::Result<(), Error> {
    let directory = directory("memory");
    let db = SqliteStorage::open(directory.join("dimcode.sqlite"))?;
    let store = SqliteTimelineStore::new(db.clone());
    let captured = Arc::new(Mutex::new(None));
    let capture = captured.clone();
    let samples = Arc::new(Mutex::new(PreparationSamples {
        count: 0,
        warm: [0; 100],
        recent: [0; 100],
    }));
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .timeline_store(store.clone())
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(ModelHost(Arc::new(EchoModel))))
        .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(Compactor {
            samples: samples.clone(),
        })))))
        .session_plugin(DefaultPrepareModelPlugin::new)
        .session_plugin(move |scope| CaptureTree {
            scope: *scope,
            captured: capture.clone(),
        });
    defaults::install(&mut builder);
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig {
            system_prompt: "s".repeat(800_000),
            tools: vec![],
            ..SessionConfig::default()
        })
        .await?;
    session.idle("main").await?;
    let tree = captured.lock().unwrap().clone().unwrap();
    let timeline = tree.get("main")?;
    let mut idle = Vec::with_capacity(steps / 100 + 2);
    let mut turn_warm = [0usize; 100];
    let mut turn_recent = [0usize; 100];
    let mut global_peak = allocation::peak();
    let wall = Instant::now();
    for index in 1..=steps {
        allocation::reset_peak();
        let event = session
            .submit(
                format!("measurement turn {index}"),
                InputMode::Queue,
                "main",
            )
            .await?;
        let weak = Arc::downgrade(&event);
        drop(event);
        session.idle("main").await?;
        tokio::task::yield_now().await;
        assert!(
            weak.upgrade().is_none(),
            "dispatched input remains resident at step {index}"
        );
        let peak = allocation::peak();
        global_peak = global_peak.max(peak);
        if index <= 100 {
            turn_warm[index - 1] = peak;
        }
        turn_recent[(index - 1) % 100] = peak;
        if index == 1 || index % 100 == 0 || index == steps {
            let live = allocation::live();
            let sample = serde_json::json!({"step":index,"liveRustBytes":live,"sequence":timeline.sequence()});
            println!("{sample}");
            idle.push((index, live, timeline.sequence()));
        }
    }
    let stop = Instant::now();
    session.stop("main", "measure stop".into()).await?;
    let stop_ms = stop.elapsed().as_secs_f64() * 1000.;
    let connection = Connection::open(directory.join("dimcode.sqlite"))?;
    let checkpoints: i64 = connection.query_row(
        "SELECT count(*) FROM timeline_events WHERE checkpoint = 1",
        [],
        |row| row.get(0),
    )?;
    {
        let samples = samples.lock().unwrap();
        let warm = samples.warm.iter().max();
        let late = samples.recent.iter().max();
        println!(
            "{}",
            serde_json::json!({"scenario":"memory","steps":steps,"systemBytes":800_000,"idle":idle,"resolveWarmLiveMax":warm,"resolveLateLiveMax":late,"preparationSamples":samples.count,"wholeTurnWarmPeak":turn_warm.iter().max(),"wholeTurnLatePeak":turn_recent.iter().max(),"globalPeakRustBytes":global_peak.max(allocation::peak()),"checkpointCount":checkpoints,"stopMs":stop_ms,"wallMs":wall.elapsed().as_secs_f64()*1000.,"debugAssertions":cfg!(debug_assertions),"database":directory})
        );
    }
    agent.close().await?;
    store.close().await?;
    db.close().await?;
    Ok(())
}
async fn rounds(branches: usize) -> std::result::Result<(), Error> {
    durable_vfs::install_tracking();
    let directory = directory("rounds");
    let db = SqliteStorage::open(directory.join("dimcode.sqlite"))?;
    let store = SqliteTimelineStore::new(db.clone());
    for index in 0..branches {
        store
            .create_session(&format!("stream-{index}"), &SessionConfig::default())
            .await?;
    }
    durable_vfs::reset_measurement();
    let mut times = Vec::new();
    let mut enqueue = Vec::new();
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let path = directory.join("dimcode.sqlite");
    let observer = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        let mut sequence: i64 = 0;
        let mut commits = Vec::new();
        while !stopped.load(Ordering::Relaxed) {
            let current: i64 = connection.query_row("SELECT sequence FROM timeline_branches WHERE session_id = 'stream-0' AND branch_id = 'main'", [], |row| row.get(0)).unwrap();
            if current != sequence {
                sequence = current;
                commits.push(started.elapsed().as_secs_f64() * 1000.);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        commits
    });
    let mut sequence = 0;
    while started.elapsed() < Duration::from_secs(2) {
        sequence += 1;
        for index in 0..branches {
            let start = Instant::now();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            store.append(
                key(&format!("stream-{index}")),
                vec![record(
                    sequence,
                    EventKind::Text(DeltaData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        delta: "x".repeat(128),
                    }),
                )],
                Box::new(move |result| {
                    let _ = sender.send((result, start.elapsed().as_secs_f64() * 1000.));
                }),
            )?;
            enqueue.push(start.elapsed().as_secs_f64() * 1000.);
            times.push(receiver);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut delay = Vec::new();
    for receiver in times {
        let (result, elapsed) = receiver.await?;
        result?;
        delay.push(elapsed);
    }
    stop.store(true, Ordering::Relaxed);
    let commits = observer.join().unwrap();
    let connection = Connection::open(directory.join("dimcode.sqlite"))?;
    let output_rows: i64 =
        connection.query_row("SELECT count(*) FROM timeline_events", [], |row| row.get(0))?;
    let wal = std::fs::metadata(directory.join("dimcode.sqlite-wal"))?.len();
    let intervals: Vec<_> = commits.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let (written, syncs) = durable_vfs::measurement();
    println!(
        "{}",
        serde_json::json!({"scenario":"rounds","branches":branches,"events":branches*sequence as usize,"observedTransactions":commits.len(),"writeRoundsPerSecond":commits.len() as f64/(started.elapsed().as_secs_f64()),"roundIntervalMinMs":intervals.iter().copied().reduce(f64::min),"publishP50Ms":quantile(&mut delay,0.5),"publishP99Ms":quantile(&mut delay,0.99),"enqueueP99Ms":quantile(&mut enqueue,0.99),"walFileBytes":wal,"walWrittenBytes":written,"walWriteBytesPerSecond":written as f64/started.elapsed().as_secs_f64(),"walSyncs":syncs.len(),"physicalOutputRows":output_rows,"debugAssertions":cfg!(debug_assertions),"database":directory})
    );
    store.close().await?;
    db.close().await?;
    Ok(())
}
async fn paged_reads() -> std::result::Result<(), Error> {
    let directory = directory("paged-reads");
    let db = SqliteStorage::open(directory.join("dimcode.sqlite"))?;
    let store = SqliteTimelineStore::new(db.clone());
    for id in ["history", "writer"] {
        store.create_session(id, &SessionConfig::default()).await?;
    }
    let records = (1..=4096)
        .map(|sequence| {
            record(
                sequence,
                EventKind::InputMessage(InputMessageData::text("x".repeat(2048), None)),
            )
        })
        .collect();
    committed(&store, "history", records).await?;
    let reader = store.clone();
    let reading = tokio::spawn(async move {
        let start = Instant::now();
        for _ in 0..20 {
            drop(reader.read(&key("history"), 1..=4096).await?);
            reader.branches("history").await?;
        }
        Ok::<_, AgentError>(start.elapsed().as_secs_f64() * 1000.)
    });
    let mut publish = vec![];
    for sequence in 1..=50 {
        let start = Instant::now();
        committed(
            &store,
            "writer",
            vec![record(
                sequence,
                EventKind::InputMessage(InputMessageData::text("concurrent write", None)),
            )],
        )
        .await?;
        publish.push(start.elapsed().as_secs_f64() * 1000.);
    }
    let reading_ms = reading.await??;
    println!(
        "{}",
        serde_json::json!({"scenario":"paged-reads", "events":4096,"bodyBytes":4096*2048,
        "readAndRecoveryPasses":20,"readAndRecoveryMs":reading_ms,
        "concurrentPublishP50Ms":quantile(&mut publish,0.5),"concurrentPublishP99Ms":quantile(&mut publish,0.99),
        "debugAssertions":cfg!(debug_assertions)})
    );
    store.close().await?;
    db.close().await?;
    Ok(())
}
async fn reads() -> std::result::Result<(), Error> {
    let directory = directory("reads");
    let db = SqliteStorage::open(directory.join("dimcode.sqlite"))?;
    let store = SqliteTimelineStore::new(db.clone());
    for (name, bytes) in [("p99", 951_000), ("max", 4_900_000)] {
        store
            .create_session(name, &SessionConfig::default())
            .await?;
        committed(
            &store,
            name,
            vec![
                record(
                    1,
                    EventKind::TurnStart(TurnStartData {
                        turn_id: "turn".into(),
                    }),
                ),
                record(
                    2,
                    EventKind::ModelCall(ModelCallData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        request: Arc::new(ModelRequest::new(
                            "mock",
                            "measure",
                            vec![Message::text(Role::User, "x".repeat(bytes))],
                        )),
                        configuration: None,
                        prompt: None,
                    }),
                ),
                record(
                    3,
                    EventKind::ModelOutputEnd(ModelOutputEndData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        outcome: ModelOutputOutcome::Completed {
                            finish_reason: FinishReason::EndTurn,
                        },
                    }),
                ),
                record(
                    4,
                    EventKind::TurnEnd(TurnEndData {
                        turn_id: "turn".into(),
                        outcome: TurnOutcome::Completed {
                            reason: "synthetic history completed".into(),
                        },
                    }),
                ),
            ],
        )
        .await?;
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key(name),
            vec![],
            store.clone(),
        )
        .await?;
        let mut timing = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            drop(timeline.context(4).await?);
            drop(timeline.context(4).await?);
            timing.push(start.elapsed().as_secs_f64() * 1000.);
        }
        let captured = Arc::new(Mutex::new(None));
        let target = captured.clone();
        let observed = Arc::new(Mutex::new(Vec::with_capacity(21)));
        let stop_target = observed.clone();
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install(&mut builder);
        builder
            .timeline_store(store.clone())
            .session_plugin(move |scope| CaptureTree {
                scope: *scope,
                captured: target.clone(),
            })
            .timeline_plugin(move |scope| CaptureStop {
                scope: *scope,
                observed: stop_target.clone(),
            });
        let agent = builder.build();
        let session = agent.restore_session(name.into()).await?;
        session.idle("main").await?;
        let active = captured.lock().unwrap().clone().unwrap().get("main")?;
        let mut dispatch = Vec::new();
        for _ in 0..20 {
            let reader = active.clone();
            let (started, running) = tokio::sync::oneshot::channel();
            let read = tokio::spawn(async move {
                let through = reader.sequence();
                let _ = started.send(());
                drop(reader.context(through).await?);
                drop(reader.context(through).await?);
                Ok::<_, AgentError>(())
            });
            running.await?;
            tokio::task::yield_now().await;
            let start = Instant::now();
            session.stop("main", "measurement stop".into()).await?;
            let event = *observed.lock().unwrap().last().unwrap();
            dispatch.push(event.duration_since(start).as_secs_f64() * 1000.);
            read.await??;
        }
        println!(
            "{}",
            serde_json::json!({"scenario":"reads","scale":name,"contentBytes":bytes,"twoReadsP50Ms":quantile(&mut timing,0.5),"twoReadsP99Ms":quantile(&mut timing,0.99),"stopDispatchP50Ms":quantile(&mut dispatch,0.5),"stopDispatchP99Ms":quantile(&mut dispatch,0.99),"debugAssertions":cfg!(debug_assertions)})
        );
        agent.close().await?;
    }
    store.close().await?;
    db.close().await?;
    Ok(())
}
struct BarrierStore {
    inner: Arc<SqliteTimelineStore>,
    mode: String,
}
impl TimelineStore for BarrierStore {
    fn create_session<'a>(
        &'a self,
        id: &'a str,
        c: &'a SessionConfig,
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.create_session(id, c)
    }
    fn rollback_session<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.rollback_session(id)
    }
    fn session_config<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<SessionConfig>>> {
        self.inner.session_config(id)
    }
    fn branches<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Vec<StoredBranch>>> {
        self.inner.branches(id)
    }
    fn head<'a>(&'a self, k: &'a TimelineKey) -> BoxFuture<'a, Result<u64>> {
        self.inner.head(k)
    }
    fn read_segments<'a>(
        &'a self,
        id: &'a str,
        s: Vec<(String, std::ops::RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>> {
        self.inner.read_segments(id, s)
    }
    fn trace<'a>(
        &'a self,
        k: &'a TimelineKey,
        s: u64,
    ) -> BoxFuture<'a, Result<Option<TimelineTrace>>> {
        self.inner.trace(k, s)
    }
    fn latest_checkpoint<'a>(
        &'a self,
        k: &'a TimelineKey,
        s: u64,
    ) -> BoxFuture<'a, Result<Option<ContextCompactionData>>> {
        self.inner.latest_checkpoint(k, s)
    }
    fn append(&self, k: TimelineKey, e: Vec<StoredEvent>, publish: PublishCallback) -> Result<()> {
        let mode = self.mode.clone();
        self.inner.append(
            k,
            e,
            Box::new(move |result| {
                result.as_ref().unwrap();
                if mode == "committed" {
                    println!("COMMITTED_BEFORE_PUBLICATION");
                    std::io::stdout().flush().unwrap();
                    loop {
                        std::thread::park();
                    }
                }
                publish(result);
                if mode == "published" {
                    println!("PUBLISHED");
                    std::io::stdout().flush().unwrap();
                    loop {
                        std::thread::park();
                    }
                }
            }),
        )
    }
}
async fn interruption_child(
    mode: String,
    path: PathBuf,
    power: bool,
) -> std::result::Result<(), Error> {
    if power {
        durable_vfs::install(path.parent().unwrap().join("synced"));
    }
    let db = SqliteStorage::open(path.clone())?;
    let store = SqliteTimelineStore::new(db);
    store
        .create_session("interrupted", &SessionConfig::default())
        .await?;
    let _blocker = if mode == "before" {
        let blocker = Connection::open(&path)?;
        blocker.execute_batch("BEGIN IMMEDIATE")?;
        Some(blocker)
    } else {
        None
    };
    if mode == "unsynced" {
        durable_vfs::interrupt_next_wal_sync();
    }
    let timeline = Timeline::new(
        rutis::Ctx::root().unwrap(),
        key("interrupted"),
        vec![],
        Arc::new(BarrierStore {
            inner: store,
            mode: mode.clone(),
        }),
    );
    timeline.append_batch_registered(interrupted_batch(), |_| Ok(()))?;
    if mode == "before" {
        assert_eq!(timeline.sequence(), 0);
        println!("QUEUED_BEFORE_COMMIT");
        std::io::stdout().flush()?;
    }
    std::future::pending::<()>().await;
    Ok(())
}
async fn power_parent() -> std::result::Result<(), Error> {
    for mode in ["unsynced", "committed", "published"] {
        let directory = directory(&format!("power-{mode}"));
        let path = directory.join("dimcode.sqlite");
        let mut child = Command::new(std::env::current_exe()?)
            .arg("power-child")
            .arg(mode)
            .arg(&path)
            .stdout(Stdio::piped())
            .spawn()?;
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line)?;
        assert!(!line.is_empty(), "child exited before power barrier");
        child.kill()?;
        child.wait()?;
        let recovery = directory.join("durable-recovery");
        std::fs::create_dir_all(&recovery)?;
        for name in ["dimcode.sqlite", "dimcode.sqlite-wal"] {
            let image = directory.join("synced").join(name);
            if image.exists() {
                std::fs::copy(image, recovery.join(name))?;
            }
        }
        let db = SqliteStorage::open(recovery.join("dimcode.sqlite"))?;
        let store = SqliteTimelineStore::new(db.clone());
        let sequence = store.head(&key("interrupted")).await?;
        assert_eq!(sequence, if mode == "unsynced" { 0 } else { 2 });
        store.branches("interrupted").await?;
        committed(
            &store,
            "interrupted",
            vec![record(
                sequence + 1,
                EventKind::StopInterrupt(StopInterruptData {
                    reason: "filler".into(),
                    scope: InterruptScope::Turn,
                }),
            )],
        )
        .await?;
        assert_eq!(store.head(&key("interrupted")).await?, sequence + 1);
        println!(
            "{}",
            serde_json::json!({"scenario":"simulated-power-loss","stage":mode,"barrier":line.trim(),"recoveredSequence":sequence,"nextSequence":sequence+1,"model":"only successful xSync images survive; truthful fsync; discard unsynced writes","database":directory})
        );
        store.close().await?;
        db.close().await?;
    }
    Ok(())
}
async fn crash_parent() -> std::result::Result<(), Error> {
    for mode in ["before", "committed", "published"] {
        let directory = directory(mode);
        let path = directory.join("dimcode.sqlite");
        let mut child = Command::new(std::env::current_exe()?)
            .arg("crash-child")
            .arg(mode)
            .arg(&path)
            .stdout(Stdio::piped())
            .spawn()?;
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line)?;
        assert!(!line.is_empty(), "child exited before barrier");
        child.kill()?;
        child.wait()?;
        let db = SqliteStorage::open(path.clone())?;
        let store = SqliteTimelineStore::new(db.clone());
        let sequence = store.head(&key("interrupted")).await?;
        assert_eq!(sequence, if mode == "before" { 0 } else { 2 });
        store.branches("interrupted").await?;
        committed(
            &store,
            "interrupted",
            vec![record(
                sequence + 1,
                EventKind::StopInterrupt(StopInterruptData {
                    reason: "filler".into(),
                    scope: InterruptScope::Turn,
                }),
            )],
        )
        .await?;
        assert_eq!(store.head(&key("interrupted")).await?, sequence + 1);
        println!(
            "{}",
            serde_json::json!({"scenario":"process-interruption","stage":mode,"barrier":line.trim(),"recoveredSequence":sequence,"nextSequence":sequence+1,"atomicBatchWidth":2,"database":directory})
        );
        store.close().await?;
        db.close().await?;
    }
    Ok(())
}
fn interrupted_batch() -> Vec<EventDraft> {
    ["durable first input", "durable second input"]
        .into_iter()
        .map(|text| EventKind::InputMessage(InputMessageData::text(text, None)).into())
        .collect()
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::result::Result<(), Error> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("memory") => memory(args.next().map(|n| n.parse()).transpose()?.unwrap_or(2000)).await,
        Some("rounds") => rounds(args.next().map(|n| n.parse()).transpose()?.unwrap_or(64)).await,
        Some("reads") => reads().await,
        Some("paged-reads") => paged_reads().await,
        Some("crash-child") => {
            interruption_child(
                args.next().unwrap(),
                PathBuf::from(args.next().unwrap()),
                false,
            )
            .await
        }
        Some("crashes") => crash_parent().await,
        Some("power-child") => {
            interruption_child(
                args.next().unwrap(),
                PathBuf::from(args.next().unwrap()),
                true,
            )
            .await
        }
        Some("powerloss") => power_parent().await,
        _ => Err("expected memory [steps], rounds [branches], reads, crashes, or powerloss".into()),
    }
}
