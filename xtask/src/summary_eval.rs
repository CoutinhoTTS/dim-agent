//! `summary-eval`: the summary-quality evaluation of
//! [#170](https://github.com/arcships/dim/issues/170) (ContextCompaction
//! §13.4). It calls real models, so it is not part of CI: the operator runs it
//! against an isolated product home whose providers are configured, and the
//! output goes outside the repository.
use crate::app_server;
use dim_agent_capabilities::{
    context_model::ProductContextModel,
    file_storage::FileStorage,
    history::{HistoryEntry, summary_message},
    host::{
        CompactionCheckpoint, CompactionCut, CompactionInput, CompactionStrategy, ContextModel,
        HostError, ModelChunk, ModelHost, ModelStream, ProviderConnectionResolver,
        ResolveContextModel, StrategyOutcome,
    },
    model_provider::invoke_with_recovery,
    summary_strategy::{
        ProductSummaryStrategyConfig, SummaryReasoning, SummaryStrategy, TargetRule,
    },
};
use dim_agent_types::{
    BaselineSource, CompactionTrigger, Message, MessageContent, MessagePart, ModelId, ModelRequest,
    ModelSelection, Role,
};
use dim_app_server::provider::{ModelPreferences, ProviderService};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The case files: one JSON history per file, under the xtask manifest.
const CASES: &str = "summary-eval/cases";

/// The target rules evaluated once a reasoning value is chosen: no
/// instruction, and the four `(α, β)` candidates of §13.3.
const TARGETS: [(f64, f64); 4] = [(0.25, 0.05), (0.25, 0.10), (0.5, 0.05), (0.5, 0.10)];

/// A narrowed window also bounds the output allowance, so the case keeps a
/// positive input budget on a model whose own allowance is large (§13.4).
const NARROWED_OUTPUT_SHARE: f64 = 0.25;

const JUDGE_SYSTEM: &str = "You check the statements below against a summary that was written for a coding session. For each statement answer \"present\" when the summary states it, \"absent\" when the summary does not mention it, and \"contradicted\" when the summary states the opposite. Add the summary text that supports the verdict as a short quote. Answer with a JSON array only, one object per statement: {\"id\": \"<statement id>\", \"verdict\": \"present|absent|contradicted\", \"quote\": \"<summary text>\"}.";

pub async fn run() -> Result<()> {
    let options = Options::parse()?;
    let home = isolated_home(&options.home)?;
    let cases = read_cases()?;
    std::fs::create_dir_all(&options.out)?;
    let cancel = CancellationToken::new();
    let server =
        app_server::serve_home(&home, &home, options.out.join("command-output"), |_| {}).await?;
    let providers = server
        .service::<ProviderService>()
        .ok_or("the product profile serves no provider catalog")?;
    let host = server
        .service::<ModelHost>()
        .ok_or("the product profile serves no model host")?;
    let resolver = server
        .service::<ProviderConnectionResolver>()
        .ok_or("the product profile serves no connection resolver")?;
    let storage = server
        .service::<FileStorage>()
        .ok_or("the product profile serves no file storage")?;
    let model = providers
        .configuration(
            Some(options.model.clone()),
            None,
            ModelPreferences::default(),
            &cancel,
        )
        .await
        .map_err(|error| format!("{}: {error}", options.model.provider_id))?;
    let judge = providers
        .configuration(
            Some(options.judge.clone()),
            None,
            ModelPreferences::default(),
            &cancel,
        )
        .await
        .map_err(|error| format!("{}: {error}", options.judge.provider_id))?;
    let mut prepared = Vec::new();
    for case in &cases {
        prepared.push(Prepared::new(case, &model, &cancel).await?);
    }
    let file = CompactionFile::open(&home)?;
    let mut rows = Vec::new();
    for arm in arms(options.reasoning) {
        file.write(arm.reasoning)?;
        let strategy = SummaryStrategy::new(
            host.clone(),
            resolver.clone(),
            Some(storage.clone()),
            ProductSummaryStrategyConfig {
                home: Some(home.clone()),
                target: arm.target,
            },
        )
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let mut metrics = Metrics::default();
        for case in &prepared {
            for run in 0..options.runs {
                let outcome = run_case(&strategy, case, &cancel).await;
                let judged = judge_run(&host, &resolver, &judge, case, &outcome, &cancel).await;
                let probes = probe_records(
                    &case.probes,
                    outcome.summary().as_deref().unwrap_or_default(),
                    &judged,
                );
                metrics.add(&arm, case, &outcome, &probes);
                let directory = options.out.join(&arm.id);
                std::fs::create_dir_all(&directory)?;
                std::fs::write(
                    directory.join(format!("{}-run{run}.json", case.id)),
                    serde_json::to_string_pretty(&json!({
                        "arm": arm.id,
                        "case": case.id,
                        "description": case.description,
                        "run": run,
                        "contextWindow": case.window,
                        "outputTokens": case.output,
                        "outcome": outcome.label(),
                        "error": outcome.error(),
                        "durationMs": outcome.duration.as_secs_f64() * 1000.,
                        "summary": outcome.summary(),
                        "diagnostics": outcome.diagnostics(),
                        "judgeError": judged.error(),
                        "probes": probes,
                    }))? + "\n",
                )?;
            }
        }
        println!("{}", metrics.line(&arm));
        rows.push(metrics.row(&arm));
    }
    drop(file);
    std::fs::write(options.out.join("summary.md"), table(&rows))?;
    server.shutdown();
    server.join().await?;
    println!("wrote {}", options.out.join("summary.md").display());
    Ok(())
}

/// `--home` must be an isolated product home: the evaluation writes its arms'
/// `compaction.json` there and must never touch the product home.
fn isolated_home(path: &Path) -> Result<PathBuf> {
    let home = std::fs::canonicalize(path)
        .map_err(|error| format!("--home {}: {error}", path.display()))?;
    let mut products = Vec::new();
    if let Some(user) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        products.push(PathBuf::from(&user).join(".dimcode-alpha"));
        products.push(PathBuf::from(user).join(".dimcode"));
    }
    if let Some(path) = std::env::var_os("DIMCODE_HOME") {
        products.push(PathBuf::from(path));
    }
    for product in products {
        if std::fs::canonicalize(product).ok().as_deref() == Some(home.as_path()) {
            return Err("--home must be an isolated product home, never the product home".into());
        }
    }
    Ok(home)
}

struct Options {
    home: PathBuf,
    model: ModelId,
    judge: ModelId,
    runs: usize,
    out: PathBuf,
    reasoning: Option<SummaryReasoning>,
}
impl Options {
    fn parse() -> Result<Self> {
        let usage = "usage: cargo run -p xtask -- summary-eval --home DIR \
                     --model PROVIDER/MODEL --judge PROVIDER/MODEL --runs N --out DIR \
                     [--reasoning inherit|off|low]";
        let mut args = std::env::args().skip(2);
        let (mut home, mut model, mut judge, mut runs, mut out, mut reasoning) =
            (None, None, None, None, None, None);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--home" => home = Some(PathBuf::from(value)),
                "--model" => model = Some(target(&value)?),
                "--judge" => judge = Some(target(&value)?),
                "--runs" => runs = Some(value.parse::<usize>()?),
                "--out" => out = Some(PathBuf::from(value)),
                "--reasoning" => reasoning = Some(reasoning_value(&value)?),
                _ => return Err(usage.into()),
            }
        }
        Ok(Self {
            home: home.ok_or(usage)?,
            model: model.ok_or(usage)?,
            judge: judge.ok_or(usage)?,
            runs: runs.filter(|runs| *runs > 0).ok_or(usage)?,
            out: out.ok_or(usage)?,
            reasoning,
        })
    }
}

fn target(value: &str) -> Result<ModelId> {
    let (provider_id, model_id) = value
        .split_once('/')
        .ok_or_else(|| format!("a model is PROVIDER/MODEL, not {value}"))?;
    Ok(ModelId {
        provider_id: provider_id.into(),
        model_id: model_id.into(),
    })
}

fn reasoning_value(value: &str) -> Result<SummaryReasoning> {
    match value {
        "inherit" => Ok(SummaryReasoning::Inherit),
        "off" => Ok(SummaryReasoning::Off),
        "low" => Ok(SummaryReasoning::Low),
        _ => Err(format!("--reasoning takes inherit, off or low, not {value}").into()),
    }
}

/// One arm of the protocol: a reasoning value with either no target rule
/// (phase 1) or one of the target rules (phase 2).
struct Arm {
    id: String,
    reasoning: SummaryReasoning,
    target: Option<TargetRule>,
}
fn arms(reasoning: Option<SummaryReasoning>) -> Vec<Arm> {
    match reasoning {
        None => [
            SummaryReasoning::Inherit,
            SummaryReasoning::Off,
            SummaryReasoning::Low,
        ]
        .into_iter()
        .map(|reasoning| Arm {
            id: format!("reasoning-{}", reasoning.name()),
            reasoning,
            target: None,
        })
        .collect(),
        Some(reasoning) => std::iter::once(Arm {
            id: "target-none".into(),
            reasoning,
            target: None,
        })
        .chain(TARGETS.into_iter().map(|(output_share, input_share)| Arm {
            id: format!("target-{}-{}", percent(output_share), percent(input_share)),
            reasoning,
            target: Some(TargetRule {
                output_share,
                input_share,
            }),
        }))
        .collect(),
    }
}
fn percent(share: f64) -> String {
    format!("{}", (share * 100.).round() as u64)
}

/// The home's `compaction.json`, written per arm and restored afterwards.
struct CompactionFile {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}
impl CompactionFile {
    fn open(home: &Path) -> Result<Self> {
        let path = home.join("compaction.json");
        let previous = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, previous })
    }
    fn write(&self, reasoning: SummaryReasoning) -> Result<()> {
        std::fs::write(
            &self.path,
            json!({"summaryReasoning": reasoning.name()}).to_string(),
        )?;
        Ok(())
    }
}
impl Drop for CompactionFile {
    fn drop(&mut self) {
        let restored = match &self.previous {
            Some(bytes) => std::fs::write(&self.path, bytes),
            None => match std::fs::remove_file(&self.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        if let Err(error) = restored {
            eprintln!("could not restore {}: {error}", self.path.display());
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    earlier_summary: Option<String>,
    messages: Vec<Message>,
    retain_from: usize,
    probes: Vec<Probe>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Probe {
    id: String,
    kind: ProbeKind,
    text: String,
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProbeKind {
    Exact,
    Fact,
    Absent,
}

fn read_cases() -> Result<Vec<Case>> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CASES);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let bytes = std::fs::read(&file)?;
        let case: Case = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        validate_cut(&case).map_err(|error| format!("{}: {error}", file.display()))?;
        cases.push(case);
    }
    Ok(cases)
}

/// The only cut is `retainFrom`, so it must not separate a tool call from its
/// result (ContextCompaction §4.3, §13.4).
fn validate_cut(case: &Case) -> std::result::Result<(), String> {
    if case.retain_from > case.messages.len() {
        return Err(format!(
            "retainFrom {} is past the {} messages",
            case.retain_from,
            case.messages.len()
        ));
    }
    let calls = tool_uses(&case.messages[..case.retain_from]);
    for message in &case.messages[case.retain_from..] {
        for id in tool_results(std::slice::from_ref(message)) {
            if calls.contains(&id) {
                return Err(format!(
                    "retainFrom {} separates the tool call {id} from its result",
                    case.retain_from
                ));
            }
        }
    }
    Ok(())
}
fn parts(message: &Message) -> &[MessagePart] {
    match &message.content {
        MessageContent::Text(_) => &[],
        MessageContent::Parts(parts) => parts,
    }
}
fn tool_uses(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .flat_map(parts)
        .filter_map(|part| match part {
            MessagePart::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}
fn tool_results(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(parts)
        .filter_map(|part| match part {
            MessagePart::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
            _ => None,
        })
        .collect()
}

/// A case as the strategy sees it: the conversation request, the history with
/// its `m<i>` sources, the single cut and the checkpoint of an earlier summary.
struct Prepared {
    id: String,
    description: String,
    probes: Vec<Probe>,
    request: Arc<ModelRequest>,
    history: Vec<HistoryEntry>,
    checkpoint: Option<CompactionCheckpoint>,
    retain_from: usize,
    model: Arc<dyn ContextModel>,
    /// The window facts the strategy ran with, after the case's narrowing.
    window: f64,
    output: f64,
}
impl Prepared {
    async fn new(
        case: &Case,
        selection: &ModelSelection,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let mut request = ModelRequest::new(
            &selection.provider_id,
            &selection.model_id,
            case.messages.clone(),
        );
        request.capability_snapshot = selection.capability_snapshot.clone();
        request.config = selection.config.clone();
        if let Some(window) = case.context_window {
            let declared_output = {
                let snapshot = request
                    .capability_snapshot
                    .as_mut()
                    .ok_or("the selected model has no capability snapshot")?;
                snapshot.capabilities.context_window =
                    snapshot.capabilities.context_window.min(window);
                snapshot.capabilities.max_output_tokens as f64
            };
            let declared = request
                .config
                .as_ref()
                .and_then(|config| config.max_tokens)
                .unwrap_or(declared_output);
            request.config.get_or_insert_default().max_tokens =
                Some(declared.min(window as f64 * NARROWED_OUTPUT_SHARE));
        }
        let request = Arc::new(request);
        let model = ProductContextModel
            .resolve(request.clone(), cancel.clone())
            .await?;
        if model.context_window() - model.output_tokens() <= 0. {
            return Err(format!(
                "case {}: a window of {} leaves no input budget with {} output tokens",
                case.id,
                model.context_window(),
                model.output_tokens()
            )
            .into());
        }
        let history = case
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| HistoryEntry {
                id: format!("m{index}"),
                message: message.clone(),
                tool_context: message.role == Role::ToolResult,
                baseline: Arc::from([]),
                source: format!("m{index}"),
            })
            .collect();
        Ok(Self {
            id: case.id.clone(),
            description: case.description.clone(),
            probes: case.probes.clone(),
            checkpoint: case
                .earlier_summary
                .as_ref()
                .map(|summary| CompactionCheckpoint {
                    retain_from: "m0".into(),
                    replacement: Arc::from([summary_message(summary)]),
                    context_baseline: Arc::from([]),
                    replacement_parts: None,
                }),
            history,
            retain_from: case.retain_from,
            window: model.context_window(),
            output: model.output_tokens(),
            model,
            request,
        })
    }
    fn input(&self) -> CompactionInput {
        let retain_from = format!("m{}", self.retain_from);
        CompactionInput {
            history: self.history.clone(),
            checkpoint: self.checkpoint.clone(),
            request: self.request.clone(),
            model: self.model.clone(),
            retain_limit: Some(retain_from.clone()),
            cuts: vec![CompactionCut {
                retain_from,
                entry: self.retain_from,
                within_limit: true,
            }],
            trigger: CompactionTrigger::Manual,
            estimated_input_tokens: self.model.count_tokens(&self.request),
            baseline_source: BaselineSource::FullEstimate,
            baseline_prompt_tokens: None,
            delta_tokens: None,
            provider_budget: None,
            prior: None,
        }
    }
}

/// One compaction of the shipped strategy, with what it took.
struct Run {
    duration: Duration,
    outcome: std::result::Result<StrategyOutcome, HostError>,
}
impl Run {
    fn label(&self) -> &'static str {
        match &self.outcome {
            Ok(StrategyOutcome::Completed { .. }) => "completed",
            Ok(StrategyOutcome::Skipped { .. }) => "skipped",
            Ok(StrategyOutcome::Failed { .. }) => "failed",
            Err(_) => "failed",
        }
    }
    fn error(&self) -> Option<String> {
        match &self.outcome {
            Ok(StrategyOutcome::Failed { error, .. }) => Some(error.clone()),
            Err(HostError::Failed { message, .. }) => Some(message.clone()),
            Err(HostError::Cancelled(reason)) => Some(reason.clone()),
            Ok(_) => None,
        }
    }
    fn summary(&self) -> Option<String> {
        match &self.outcome {
            Ok(StrategyOutcome::Completed { replacement, .. }) => Some(render(replacement)),
            _ => None,
        }
    }
    fn details(&self) -> BTreeMap<String, Value> {
        let record = match &self.outcome {
            Ok(StrategyOutcome::Completed { diagnostics, .. })
            | Ok(StrategyOutcome::Skipped { diagnostics, .. })
            | Ok(StrategyOutcome::Failed { diagnostics, .. }) => Some(&diagnostics.record),
            Err(_) => None,
        };
        match record.map(|record| serde_json::to_value(&record.details)) {
            Some(Ok(Value::Object(details))) => details.into_iter().collect(),
            _ => BTreeMap::new(),
        }
    }
    fn diagnostics(&self) -> Option<Value> {
        let record = match &self.outcome {
            Ok(StrategyOutcome::Completed { diagnostics, .. })
            | Ok(StrategyOutcome::Skipped { diagnostics, .. })
            | Ok(StrategyOutcome::Failed { diagnostics, .. }) => Some(&diagnostics.record),
            Err(_) => None,
        };
        record.and_then(|record| serde_json::to_value(record).ok())
    }
}

async fn run_case(strategy: &SummaryStrategy, case: &Prepared, cancel: &CancellationToken) -> Run {
    let started = Instant::now();
    let outcome = strategy.compact(case.input(), cancel.clone()).await;
    Run {
        duration: started.elapsed(),
        outcome,
    }
}

fn render(replacement: &[Message]) -> String {
    replacement
        .iter()
        .map(|message| match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(serde::Deserialize)]
struct Judgement {
    id: String,
    verdict: Verdict,
    #[serde(default)]
    quote: Option<String>,
}
#[derive(Clone, Copy, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Present,
    Absent,
    Contradicted,
}
impl Verdict {
    fn name(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Contradicted => "contradicted",
        }
    }
}

/// What a run's judge step produced.
enum Judged {
    /// The judge answered; a probe it skipped stays `unparsed`.
    Verdicts(BTreeMap<String, (Verdict, Option<String>)>),
    /// The compaction left no summary, so there is nothing to judge; the
    /// judged probes are absent without a judge call.
    NoSummary,
    /// The judge call or its answer failed; every judged probe is `unparsed`
    /// and the evaluation goes on (ContextCompaction §13.4).
    Unavailable(String),
}
impl Judged {
    fn error(&self) -> Option<&str> {
        match self {
            Self::Unavailable(error) => Some(error),
            _ => None,
        }
    }
}

/// The probe outcomes of one run: `exact` probes are mechanical, `fact` and
/// `absent` probes are judged on the summary alone (ContextCompaction §13.4).
fn probe_records(probes: &[Probe], summary: &str, judged: &Judged) -> Vec<Value> {
    probes
        .iter()
        .map(|probe| {
            let (verdict, quote) = match probe.kind {
                ProbeKind::Exact => (
                    if normalize(summary).contains(&normalize(&probe.text)) {
                        "present"
                    } else {
                        "absent"
                    },
                    None,
                ),
                ProbeKind::Fact | ProbeKind::Absent => match judged {
                    Judged::NoSummary => ("absent", None),
                    Judged::Verdicts(verdicts) => verdicts
                        .get(&probe.id)
                        .map_or(("unparsed", None), |(verdict, quote)| {
                            (verdict.name(), quote.clone())
                        }),
                    Judged::Unavailable(_) => ("unparsed", None),
                },
            };
            json!({"id": probe.id, "kind": probe.kind, "text": probe.text, "verdict": verdict, "quote": quote})
        })
        .collect()
}

/// Judges one run's `fact` and `absent` probes; a failure is recorded, never
/// propagated, so one bad answer cannot end the evaluation.
async fn judge_run(
    host: &Arc<ModelHost>,
    resolver: &Arc<ProviderConnectionResolver>,
    judge: &ModelSelection,
    case: &Prepared,
    run: &Run,
    cancel: &CancellationToken,
) -> Judged {
    let Some(summary) = run.summary() else {
        return Judged::NoSummary;
    };
    if !case
        .probes
        .iter()
        .any(|probe| probe.kind != ProbeKind::Exact)
    {
        return Judged::Verdicts(BTreeMap::new());
    }
    match judge_probes(host, resolver, judge, case, &summary, cancel).await {
        Ok(verdicts) => Judged::Verdicts(verdicts),
        Err(error) => Judged::Unavailable(error),
    }
}

async fn judge_probes(
    host: &Arc<ModelHost>,
    resolver: &Arc<ProviderConnectionResolver>,
    judge: &ModelSelection,
    case: &Prepared,
    summary: &str,
    cancel: &CancellationToken,
) -> std::result::Result<BTreeMap<String, (Verdict, Option<String>)>, String> {
    let statements = case
        .probes
        .iter()
        .filter(|probe| probe.kind != ProbeKind::Exact)
        .map(|probe| format!("- id: {}, statement: {}", probe.id, probe.text))
        .collect::<Vec<_>>()
        .join("\n");
    let text =
        format!("<summary>\n{summary}\n</summary>\n\n<statements>\n{statements}\n</statements>");
    let mut request = ModelRequest::new(
        &judge.provider_id,
        &judge.model_id,
        vec![Message::text(Role::User, text)],
    );
    request.capability_snapshot = judge.capability_snapshot.clone();
    let mut config = judge.config.clone().unwrap_or_default();
    config.temperature = Some(0.);
    request.config = Some(config);
    request.system = Some(Arc::new(dim_agent_types::SystemPrompt::Text(
        JUDGE_SYSTEM.into(),
    )));
    let request = Arc::new(request);
    let reply = match complete(host, resolver, &request, cancel).await {
        Ok(reply) => reply,
        Err(error) => {
            let mut retry = (*request).clone();
            if retry
                .config
                .as_ref()
                .and_then(|config| config.temperature)
                .is_none()
            {
                return Err(format!("the judge failed: {error}"));
            }
            retry.config.as_mut().expect("a config").temperature = None;
            match complete(host, resolver, &Arc::new(retry), cancel).await {
                Ok(reply) => reply,
                Err(retry) => return Err(format!("the judge failed: {error}, then {retry}")),
            }
        }
    };
    let parsed = parse_judgements(&reply)
        .ok_or_else(|| format!("the judge answered with no verdict list: {reply}"))?;
    Ok(parsed
        .into_iter()
        .map(|judgement| {
            (
                judgement.id,
                (judgement.verdict, judgement.quote.filter(|q| !q.is_empty())),
            )
        })
        .collect())
}

async fn complete(
    host: &Arc<ModelHost>,
    resolver: &Arc<ProviderConnectionResolver>,
    request: &Arc<ModelRequest>,
    cancel: &CancellationToken,
) -> Result<String> {
    let resolved = resolver.0.resolve(request.clone(), cancel.clone()).await?;
    let (text, _) = invoke_with_recovery(
        host,
        request,
        resolved,
        cancel,
        &mut (),
        |stream: ModelStream, _, _, _| Box::pin(collected(stream)),
    )
    .await?;
    Ok(text)
}

async fn collected(mut stream: ModelStream) -> std::result::Result<(String, bool), HostError> {
    let mut text = String::new();
    let mut stopped = false;
    while let Some(chunk) = stream.next().await {
        match chunk? {
            ModelChunk::Text(delta) => text.push_str(&delta),
            ModelChunk::MessageStop { .. } => stopped = true,
            _ => {}
        }
    }
    if !stopped {
        return Err(HostError::failed("the judge stream ended without a stop"));
    }
    Ok((text, true))
}

/// The first JSON array of a reply that parses as a verdict list. Prose, a
/// fenced block or a bracketed lead-in before the array must not hide it, and
/// a bracket inside a quoted verdict is part of the JSON, not a boundary.
fn parse_judgements(text: &str) -> Option<Vec<Judgement>> {
    text.match_indices('[').find_map(|(start, _)| {
        serde_json::Deserializer::from_str(&text[start..])
            .into_iter::<Vec<Judgement>>()
            .next()?
            .ok()
    })
}

/// Whitespace-normalized text for the exact probes.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The arm's metrics over its cases and runs (ContextCompaction §13.4).
#[derive(Default)]
struct Metrics {
    exact: (usize, usize),
    facts: (usize, usize),
    violations: usize,
    /// Judged probes the judge did not answer, for the operator to correct
    /// by hand (ContextCompaction §13.4).
    unjudged: usize,
    cut_offs: u64,
    failed: usize,
    skipped: usize,
    /// Runs whose applied reasoning differs from the arm's value, because
    /// Provider's mapping rejected the effort for this model (ContextCompaction
    /// §13.2).
    fell_back: usize,
    tokens: Vec<f64>,
    durations: Vec<f64>,
    ratios: Vec<f64>,
}
impl Metrics {
    fn add(&mut self, arm: &Arm, case: &Prepared, run: &Run, probes: &[Value]) {
        let details = run.details();
        if details
            .get("reasoning")
            .and_then(Value::as_str)
            .is_some_and(|applied| applied != arm.reasoning.name())
        {
            self.fell_back += 1;
        }
        self.cut_offs += details
            .get("cutOffCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if let Some(tokens) = details
            .get("estimatedSummaryTokens")
            .and_then(Value::as_f64)
        {
            self.tokens.push(tokens);
            if let Some(target) = details.get("targetTokens").and_then(Value::as_f64) {
                self.ratios.push(tokens / target);
            }
        }
        self.durations.push(run.duration.as_secs_f64() * 1000.);
        match run.label() {
            "completed" => {}
            "skipped" => self.skipped += 1,
            _ => self.failed += 1,
        }
        for (probe, record) in case.probes.iter().zip(probes) {
            let verdict = record["verdict"].as_str();
            let present = verdict == Some("present");
            if verdict == Some("unparsed") {
                self.unjudged += 1;
            }
            match probe.kind {
                ProbeKind::Exact => {
                    self.exact = count(self.exact, present);
                }
                ProbeKind::Fact => self.facts = count(self.facts, present),
                ProbeKind::Absent if present => self.violations += 1,
                ProbeKind::Absent => {}
            }
        }
    }
    fn row(&self, arm: &Arm) -> Value {
        json!({
            "arm": arm.id,
            "reasoning": arm.reasoning.name(),
            "fellBack": self.fell_back,
            "target": arm.target.map(|target| json!({
                "outputShare": target.output_share,
                "inputShare": target.input_share,
            })),
            "factRecall": recall(self.facts),
            "exactRecall": recall(self.exact),
            "violations": self.violations,
            "unjudged": self.unjudged,
            "cutOffCount": self.cut_offs,
            "failed": self.failed,
            "skipped": self.skipped,
            "medianSummaryTokens": percentile(&self.tokens, 0.5),
            "medianDurationMs": percentile(&self.durations, 0.5),
            "medianTargetRatio": percentile(&self.ratios, 0.5),
            "p90TargetRatio": percentile(&self.ratios, 0.9),
        })
    }
    fn line(&self, arm: &Arm) -> String {
        format!(
            "{}: fact {}/{}, exact {}/{}, violations {}, unjudged {}, cut-offs {}, failed {}, skipped {}, fell back {}",
            arm.id,
            self.facts.0,
            self.facts.1,
            self.exact.0,
            self.exact.1,
            self.violations,
            self.unjudged,
            self.cut_offs,
            self.failed,
            self.skipped,
            self.fell_back
        )
    }
}
fn count(counts: (usize, usize), present: bool) -> (usize, usize) {
    (counts.0 + usize::from(present), counts.1 + 1)
}
fn recall(counts: (usize, usize)) -> Option<f64> {
    (counts.1 > 0).then(|| counts.0 as f64 / counts.1 as f64)
}
fn percentile(values: &[f64], at: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 * at).ceil() as usize).saturating_sub(1);
    Some(sorted[index.min(sorted.len() - 1)])
}

/// The Markdown table of every arm's metrics.
fn table(rows: &[Value]) -> String {
    let mut text = String::from(
        "| arm | reasoning | fell back | target | fact recall | exact recall | violations | unjudged | cut-offs | failed | skipped | median summary tokens | median duration ms | median T/S | p90 T/S |\n\
         | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        let number = |value: &Value| match value.as_f64() {
            Some(value) => format!("{value:.3}"),
            None => "–".to_owned(),
        };
        let target = match row["target"]["outputShare"].as_f64() {
            Some(output) => format!(
                "{}/{}",
                output,
                row["target"]["inputShare"].as_f64().unwrap_or_default()
            ),
            None => "none".to_owned(),
        };
        text += &format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row["arm"].as_str().unwrap_or_default(),
            row["reasoning"].as_str().unwrap_or_default(),
            row["fellBack"],
            target,
            number(&row["factRecall"]),
            number(&row["exactRecall"]),
            row["violations"],
            row["unjudged"],
            row["cutOffCount"],
            row["failed"],
            row["skipped"],
            number(&row["medianSummaryTokens"]),
            number(&row["medianDurationMs"]),
            number(&row["medianTargetRatio"]),
            number(&row["p90TargetRatio"]),
        );
    }
    text
}
