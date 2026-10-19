//! Session restore cost by history size, over a disposable SQLite store.
//!
//! ```text
//! cargo run --release --locked -p dim-app-server --example restore_cost -- \
//!     [plain|tools] [1001,3001,10001] [7]
//! ```
//!
//! One Session per size is grown on `main` through real mock-model turns:
//! `plain` turns submit text (one model call, 10 events), `tools` turns
//! submit `tool:echo` (a tool call and a second model call, 18 events). Each
//! Session is then unloaded with `Session::unload_if_idle` and restored with
//! `Agent::restore_session` plus the first `idle("main")`, once per
//! repetition, with sizes interleaved; a Session create is timed in the same
//! rounds. A forwarding `TimelineStore` times every store call made while a
//! restore runs, splitting it into directory validation (`branches`), event
//! reads, the `branch/resume` append until it is published, and the time no
//! store call was running; reads overlap, so the time per read is the wall
//! time they cover over their count. Each size prints one JSON line of
//! medians; the event count includes the `branch/resume` events earlier
//! restores wrote.
//!
//! The store is `restore-cost.sqlite` in a fresh directory under
//! `RESTORE_COST_ROOT` (default `target/restore-cost`), removed at the end.
//! Growing 10k events takes minutes, so `RESTORE_COST_DB=DIR` keeps the store
//! in `DIR` instead: the first run grows the Sessions and lists them in
//! `DIR/sessions.txt`, later runs with the same mix and sizes reuse them. A
//! non-empty `DIR` without that list is refused.
use dim_agent_capabilities::defaults;
use dim_agent_runtime::*;
use dim_agent_types::{InputMode, RecoverySnapshot};
use dim_app_server::storage::{SqliteStorage, SqliteTimelineStore};
use std::{
    collections::BTreeMap,
    future::Future,
    ops::RangeInclusive,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Branches,
    Read,
    Append,
    Other,
}
struct Span {
    op: Op,
    events: usize,
    start: Instant,
    end: Instant,
}
/// Forwards to the SQLite store and times the calls made while recording.
struct Probe {
    inner: Arc<SqliteTimelineStore>,
    recording: AtomicBool,
    spans: Arc<Mutex<Vec<Span>>>,
}
impl Probe {
    fn timed<'a, T: 'a>(
        &'a self,
        op: Op,
        future: impl Future<Output = Result<T>> + Send + 'a,
        events: fn(&T) -> usize,
    ) -> BoxFuture<'a, Result<T>> {
        let recording = self.recording.load(Ordering::Relaxed);
        Box::pin(async move {
            let start = Instant::now();
            let result = future.await;
            if recording && let Ok(value) = &result {
                self.spans.lock().unwrap().push(Span {
                    op,
                    events: events(value),
                    start,
                    end: Instant::now(),
                });
            }
            result
        })
    }
}
impl TimelineStore for Probe {
    fn create_session<'a>(
        &'a self,
        id: &'a str,
        config: &'a SessionConfig,
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.create_session(id, config)
    }
    fn rollback_session<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<()>> {
        self.inner.rollback_session(id)
    }
    fn session_config<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<SessionConfig>>> {
        self.timed(Op::Other, self.inner.session_config(id), |_| 0)
    }
    fn branches<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Vec<StoredBranch>>> {
        self.timed(Op::Branches, self.inner.branches(id), |branches| {
            branches.iter().map(|branch| branch.sequence as usize).sum()
        })
    }
    fn head<'a>(&'a self, key: &'a TimelineKey) -> BoxFuture<'a, Result<u64>> {
        self.timed(Op::Other, self.inner.head(key), |_| 0)
    }
    fn append(
        &self,
        key: TimelineKey,
        events: Vec<StoredEvent>,
        publish: PublishCallback,
    ) -> Result<()> {
        if !self.recording.load(Ordering::Relaxed) {
            return self.inner.append(key, events, publish);
        }
        let (start, count, spans) = (Instant::now(), events.len(), self.spans.clone());
        self.inner.append(
            key,
            events,
            Box::new(move |result| {
                spans.lock().unwrap().push(Span {
                    op: Op::Append,
                    events: count,
                    start,
                    end: Instant::now(),
                });
                publish(result)
            }),
        )
    }
    fn read_page<'a>(
        &'a self,
        key: &'a TimelineKey,
        after: u64,
        limit: usize,
        through: u64,
    ) -> BoxFuture<'a, Result<Vec<StoredEvent>>> {
        self.timed(
            Op::Read,
            self.inner.read_page(key, after, limit, through),
            Vec::len,
        )
    }
    fn read_segments<'a>(
        &'a self,
        id: &'a str,
        segments: Vec<(String, RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>> {
        self.timed(Op::Read, self.inner.read_segments(id, segments), |read| {
            read.iter().map(Vec::len).sum()
        })
    }
    fn trace<'a>(
        &'a self,
        key: &'a TimelineKey,
        sequence: u64,
    ) -> BoxFuture<'a, Result<Option<TimelineTrace>>> {
        self.inner.trace(key, sequence)
    }
    fn save_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        snapshot: RecoverySnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.save_snapshot(key, snapshot)
    }
    fn latest_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        through: u64,
        interpreter: &'a str,
    ) -> BoxFuture<'a, Result<Option<RecoverySnapshot>>> {
        self.timed(
            Op::Other,
            self.inner.latest_snapshot(key, through, interpreter),
            |_| 0,
        )
    }
}
/// Wall time covered by at least one of `spans`, in ms.
fn covered<'a>(spans: impl Iterator<Item = &'a Span>) -> f64 {
    let mut intervals: Vec<_> = spans.map(|span| (span.start, span.end)).collect();
    intervals.sort();
    let mut total = 0.;
    let mut current: Option<(Instant, Instant)> = None;
    for (start, end) in intervals {
        match &mut current {
            Some((_, last)) if start <= *last => *last = (*last).max(end),
            _ => {
                if let Some((from, to)) = current.replace((start, end)) {
                    total += (to - from).as_secs_f64();
                }
            }
        }
    }
    if let Some((from, to)) = current {
        total += (to - from).as_secs_f64();
    }
    total * 1000.
}
fn ms(from: Instant, to: Instant) -> f64 {
    (to - from).as_secs_f64() * 1000.
}
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
/// One restore, in ms unless named otherwise.
struct Restore {
    total: f64,
    call: f64,
    unload: f64,
    branches: f64,
    reads: usize,
    read_events: usize,
    read_sum: f64,
    read_covered: f64,
    append: f64,
    outside_store: f64,
}

/// Grows one Session per size on `main` and unloads it.
async fn grow(
    agent: &Agent,
    mix: &str,
    sizes: &[u64],
) -> std::result::Result<Vec<(u64, String)>, Error> {
    let mut sessions = vec![];
    for &target in sizes {
        let session = agent.create_session(SessionConfig::default()).await?;
        session.idle("main").await?;
        let start = Instant::now();
        let mut turns = 0;
        while session.sequence("main")? < target {
            let text = match mix {
                "plain" => format!("plain turn {turns}"),
                _ => format!("tool:echo {{\"text\":\"tool turn {turns}\"}}"),
            };
            session.submit(text, InputMode::Queue, "main").await?;
            session.idle("main").await?;
            turns += 1;
        }
        eprintln!(
            "grew {} events in {turns} turns, {:.0} ms",
            session.sequence("main")?,
            ms(start, Instant::now())
        );
        assert!(session.unload_if_idle().await?);
        sessions.push((target, session.id().to_owned()));
    }
    Ok(sessions)
}

#[tokio::main]
async fn main() -> std::result::Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let mix = args.next().unwrap_or_else(|| "plain".into());
    if mix != "plain" && mix != "tools" {
        return Err("expected [plain|tools] [sizes] [reps]".into());
    }
    let sizes: Vec<u64> = args
        .next()
        .unwrap_or_else(|| "1001,3001,10001".into())
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let reps: usize = args.next().map_or(Ok(7), |value| value.parse())?;
    if sizes.is_empty() || reps == 0 {
        return Err("expected at least one size and one repetition".into());
    }

    let kept = std::env::var_os("RESTORE_COST_DB").map(PathBuf::from);
    let directory = kept.clone().unwrap_or_else(|| {
        std::env::var_os("RESTORE_COST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/restore-cost")
            })
            .join(uuid::Uuid::new_v4().to_string())
    });
    std::fs::create_dir_all(&directory)?;
    let listed = directory.join("sessions.txt");
    let header = format!("{mix} {sizes:?}");
    let reused = match std::fs::read_to_string(&listed) {
        Ok(text) => {
            let mut lines = text.lines();
            if lines.next() != Some(header.as_str()) {
                return Err(format!("{} lists other Sessions", listed.display()).into());
            }
            lines
                .map(|line| {
                    let (target, id) = line.split_once(' ').ok_or("malformed Session list")?;
                    Ok((target.parse()?, id.to_owned()))
                })
                .collect::<std::result::Result<Vec<_>, Error>>()?
        }
        Err(_) if std::fs::read_dir(&directory)?.next().is_some() => {
            return Err(format!("{} is not empty", directory.display()).into());
        }
        Err(_) => vec![],
    };
    let db = SqliteStorage::open(directory.join("restore-cost.sqlite"))?;
    let store = SqliteTimelineStore::new(db.clone());
    let probe = Arc::new(Probe {
        inner: store.clone(),
        recording: AtomicBool::new(false),
        spans: Arc::default(),
    });
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    builder.timeline_store(probe.clone());
    let agent = builder.build();

    let sessions = if reused.is_empty() {
        let sessions = grow(&agent, &mix, &sizes).await?;
        if kept.is_some() {
            let mut text = header + "\n";
            for (target, id) in &sessions {
                text += &format!("{target} {id}\n");
            }
            std::fs::write(&listed, text)?;
        }
        sessions
    } else {
        reused
    };

    let mut runs: BTreeMap<u64, Vec<Restore>> = BTreeMap::new();
    let mut events = BTreeMap::new();
    let mut create = vec![];
    for rep in 0..reps {
        // Rotate the order so no size always runs first or last.
        let mut order: Vec<_> = sessions.iter().collect();
        order.rotate_left(rep % sessions.len());
        for (target, id) in order {
            probe.spans.lock().unwrap().clear();
            probe.recording.store(true, Ordering::Relaxed);
            let start = Instant::now();
            let session = agent.restore_session(id.clone()).await?;
            let called = Instant::now();
            session.idle("main").await?;
            let end = Instant::now();
            probe.recording.store(false, Ordering::Relaxed);
            let spans = std::mem::take(&mut *probe.spans.lock().unwrap());
            let of = |op| spans.iter().filter(move |span: &&Span| span.op == op);
            events.insert(*target, session.sequence("main")?);
            let unloading = Instant::now();
            assert!(session.unload_if_idle().await?);
            runs.entry(*target).or_default().push(Restore {
                total: ms(start, end),
                call: ms(start, called),
                unload: ms(unloading, Instant::now()),
                branches: covered(of(Op::Branches)),
                reads: of(Op::Read).count(),
                read_events: of(Op::Read).map(|span| span.events).sum(),
                read_sum: of(Op::Read).map(|span| ms(span.start, span.end)).sum(),
                read_covered: covered(of(Op::Read)),
                append: covered(of(Op::Append)),
                outside_store: ms(start, end) - covered(spans.iter()),
            });
        }
        let start = Instant::now();
        let session = agent.create_session(SessionConfig::default()).await?;
        session.idle("main").await?;
        create.push(ms(start, Instant::now()));
        assert!(session.unload_if_idle().await?);
    }

    for (target, restores) in &runs {
        let of = |value: fn(&Restore) -> f64| median(restores.iter().map(value).collect());
        let totals: Vec<_> = restores.iter().map(|restore| restore.total).collect();
        println!(
            "{}",
            serde_json::json!({"scenario":"restore","mix":mix,"reps":reps,"events":events[target],
            "restoreMedianMs":of(|r| r.total),
            "restoreMinMs":totals.iter().copied().reduce(f64::min),
            "restoreMaxMs":totals.iter().copied().reduce(f64::max),
            "restoreCallMedianMs":of(|r| r.call),"unloadMedianMs":of(|r| r.unload),
            "branchesMedianMs":of(|r| r.branches),
            "reads":restores[0].reads,"readEvents":restores[0].read_events,
            "readSumMedianMs":of(|r| r.read_sum),"readCoveredMedianMs":of(|r| r.read_covered),
            "perReadMedianMs":of(|r| r.read_covered / r.reads.max(1) as f64),
            "appendMedianMs":of(|r| r.append),"outsideStoreMedianMs":of(|r| r.outside_store),
            "debugAssertions":cfg!(debug_assertions)})
        );
    }
    println!(
        "{}",
        serde_json::json!({"scenario":"create","mix":mix,"reps":reps,
        "createMedianMs":median(create),"debugAssertions":cfg!(debug_assertions)})
    );

    agent.close().await?;
    store.close().await?;
    db.close().await?;
    if kept.is_none() {
        std::fs::remove_dir_all(directory)?;
    }
    Ok(())
}
