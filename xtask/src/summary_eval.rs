//! `summary-eval`: the summary-quality evaluation of
//! [#170](https://github.com/arcships/dim/issues/170) (ContextCompaction
//! §13.4). It calls real models, so it is not part of CI: the operator runs it
//! against an isolated product home whose providers are configured, and the
//! output goes outside the repository.
use crate::app_server;
use dim_agent_capabilities::{
    context_model::ProductContextModel,
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
    ModelSelection, ReplacementParts, Role,
};
use dim_app_server::provider::ProviderService;
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
    let cases = read_cases(&cases_directory())?;
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
    let model = selection_with_preferences(&providers, &options.model, &cancel).await?;
    let judge = selection_with_preferences(&providers, &options.judge, &cancel).await?;
    let mut prepared = Vec::new();
    for case in &cases {
        prepared.push(Prepared::new(case, &model, &cancel).await?);
    }
    let mut results = Vec::new();
    for arm in arms(&options) {
        let strategy = SummaryStrategy::new(
            host.clone(),
            resolver.clone(),
            ProductSummaryStrategyConfig {
                reasoning: arm.reasoning,
                target: arm.target,
                system_prompt: arm.prompt.clone(),
            },
        );
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
                        "steps": outcome.steps_json(),
                        "judgeError": judged.error(),
                        "probes": probes,
                    }))? + "\n",
                )?;
            }
        }
        println!("{}", metrics.line(&arm));
        results.push((arm, metrics));
    }
    let rows: Vec<Value> = results
        .iter()
        .map(|(arm, metrics)| metrics.row(arm))
        .collect();
    let verdict = match results.as_slice() {
        [(_, current), (_, candidate)] if options.candidate.is_some() => {
            Some(verdict(current, candidate))
        }
        _ => None,
    };
    let mut report = table(&rows);
    if let Some(verdict) = &verdict {
        report.push_str(&format!("\n{verdict}\n"));
    }
    std::fs::write(options.out.join("summary.md"), report)?;
    server.shutdown();
    server.join().await?;
    if let Some(verdict) = &verdict {
        println!("{verdict}");
    }
    println!("wrote {}", options.out.join("summary.md").display());
    Ok(())
}

/// The model's conversation selection with its saved preferences, built the
/// way the bundled client builds one: it reads them from
/// `provider.management` and passes them at `session.create`
/// (ContextCompaction §13.4), so `inherit` inherits what the user chose. A
/// model with no saved preferences keeps the model config alone.
async fn selection_with_preferences(
    providers: &Arc<ProviderService>,
    target: &ModelId,
    cancel: &CancellationToken,
) -> Result<ModelSelection> {
    let preferences = providers
        .management(Some(target.provider_id.clone()), cancel)
        .await
        .map_err(|error| format!("{}: {error}", target.provider_id))?
        .providers
        .into_iter()
        .next()
        .and_then(|provider| provider.preferences.get(&target.model_id).cloned())
        .unwrap_or_default();
    Ok(providers
        .configuration(Some(target.clone()), None, preferences, cancel)
        .await
        .map_err(|error| format!("{}: {error}", target.provider_id))?)
}

/// `--home` must be an isolated product home holding the providers'
/// configuration; it must never be the product home.
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
    candidate: Option<Arc<str>>,
    target: Option<TargetRule>,
}
impl Options {
    fn parse() -> Result<Self> {
        let usage = "usage: cargo run -p xtask -- summary-eval --home DIR \
                     --model PROVIDER/MODEL --judge PROVIDER/MODEL --runs N --out DIR \
                     [--reasoning inherit|off|low] \
                     [--candidate FILE [--target OUTPUT_SHARE,INPUT_SHARE]]";
        let mut args = std::env::args().skip(2);
        let (mut home, mut model, mut judge, mut runs, mut out, mut reasoning) =
            (None, None, None, None, None, None);
        let (mut candidate, mut target_rule) = (None, None);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--home" => home = Some(PathBuf::from(value)),
                "--model" => model = Some(target(&value)?),
                "--judge" => judge = Some(target(&value)?),
                "--runs" => runs = Some(value.parse::<usize>()?),
                "--out" => out = Some(PathBuf::from(value)),
                "--reasoning" => reasoning = Some(reasoning_value(&value)?),
                "--candidate" => candidate = Some(candidate_prompt(Path::new(&value))?),
                "--target" => target_rule = Some(target_share(&value)?),
                _ => return Err(usage.into()),
            }
        }
        let options = Self {
            home: home.ok_or(usage)?,
            model: model.ok_or(usage)?,
            judge: judge.ok_or(usage)?,
            runs: runs.filter(|runs| *runs > 0).ok_or(usage)?,
            out: out.ok_or(usage)?,
            reasoning,
            candidate,
            target: target_rule,
        };
        if options.candidate.is_some() && options.reasoning.is_none() {
            return Err("--candidate requires --reasoning".into());
        }
        if options.target.is_some() && options.candidate.is_none() {
            return Err("--target requires --candidate".into());
        }
        Ok(options)
    }
}

/// The candidate's system prompt, read before any model call.
fn candidate_prompt(path: &Path) -> Result<Arc<str>> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("--candidate {}: {error}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("--candidate {}: not UTF-8 text", path.display()))?;
    Ok(Arc::from(text))
}

/// One target rule for both candidate arms: two shares greater than 0 and at
/// most 1 (ContextCompaction §13.4).
fn target_share(value: &str) -> Result<TargetRule> {
    let (output, input) = value
        .split_once(',')
        .ok_or_else(|| format!("--target takes OUTPUT_SHARE,INPUT_SHARE, not {value}"))?;
    let share = |name: &str, value: &str| -> Result<f64> {
        let share: f64 = value
            .trim()
            .parse()
            .map_err(|_| format!("--target {name} share is not a number: {value}"))?;
        if !(share > 0. && share <= 1.) {
            return Err(format!(
                "--target {name} share must be greater than 0 and at most 1: {value}"
            )
            .into());
        }
        Ok(share)
    };
    Ok(TargetRule {
        output_share: share("output", output)?,
        input_share: share("input", input)?,
    })
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
/// (phase 1) or one of the target rules (phase 2); a candidate arm also
/// carries its prompt (ContextCompaction §13.4).
struct Arm {
    id: String,
    reasoning: SummaryReasoning,
    target: Option<TargetRule>,
    prompt: Option<Arc<str>>,
}
fn arms(options: &Options) -> Vec<Arm> {
    let Some(reasoning) = options.reasoning else {
        return [
            SummaryReasoning::Inherit,
            SummaryReasoning::Off,
            SummaryReasoning::Low,
        ]
        .into_iter()
        .map(|reasoning| Arm {
            id: format!("reasoning-{}", reasoning.name()),
            reasoning,
            target: None,
            prompt: None,
        })
        .collect();
    };
    if let Some(prompt) = &options.candidate {
        return vec![
            Arm {
                id: "current".into(),
                reasoning,
                target: options.target,
                prompt: None,
            },
            Arm {
                id: "candidate".into(),
                reasoning,
                target: options.target,
                prompt: Some(prompt.clone()),
            },
        ];
    }
    std::iter::once(Arm {
        id: "target-none".into(),
        reasoning,
        target: None,
        prompt: None,
    })
    .chain(TARGETS.into_iter().map(|(output_share, input_share)| Arm {
        id: format!("target-{}-{}", percent(output_share), percent(input_share)),
        reasoning,
        target: Some(TargetRule {
            output_share,
            input_share,
        }),
        prompt: None,
    }))
    .collect()
}
fn percent(share: f64) -> String {
    format!("{}", (share * 100.).round() as u64)
}

/// One compaction of a case: the whole history after its checkpoint, and the
/// cut that replaces its earlier part.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    messages: Vec<Message>,
    retain_from: usize,
}

/// A case file's two shapes: one compaction, or a chain of five
/// (ContextCompaction §13.4).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaseFile {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    earlier_summary: Option<String>,
    #[serde(default)]
    messages: Option<Vec<Message>>,
    #[serde(default)]
    retain_from: Option<usize>,
    #[serde(default)]
    steps: Option<Vec<Step>>,
    probes: Vec<Probe>,
}

/// A case as the runner runs it, normalized to its steps.
struct Case {
    id: String,
    description: String,
    context_window: Option<u64>,
    earlier_summary: Option<String>,
    probes: Vec<Probe>,
    steps: Vec<Step>,
}
const CHAIN_STEPS: usize = 5;
impl CaseFile {
    fn into_case(self) -> std::result::Result<Case, String> {
        let steps = match (self.messages, self.retain_from, self.steps) {
            (Some(messages), Some(retain_from), None) => vec![Step {
                messages,
                retain_from,
            }],
            (None, None, Some(steps)) if steps.len() == CHAIN_STEPS => steps,
            (None, None, Some(steps)) => {
                return Err(format!(
                    "steps has {} slices, a chain needs {CHAIN_STEPS}",
                    steps.len()
                ));
            }
            (None, None, None) => {
                return Err("a case needs messages and retainFrom, or steps".to_owned());
            }
            (Some(_), Some(_), Some(_)) => {
                return Err("a case has both messages and steps".to_owned());
            }
            (Some(_), None, _) | (None, Some(_), _) => {
                return Err("a case needs both messages and retainFrom".to_owned());
            }
        };
        validate_steps(&steps)?;
        Ok(Case {
            id: self.id,
            description: self.description,
            context_window: self.context_window,
            earlier_summary: self.earlier_summary,
            probes: self.probes,
            steps,
        })
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Probe {
    id: String,
    kind: ProbeKind,
    text: String,
    #[serde(default)]
    item: Option<ItemClass>,
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProbeKind {
    Exact,
    Fact,
    Absent,
}

/// The 3b class a `fact` probe checks (compaction-quality §3). Exact
/// identifiers, the eighth item, are the `exact` probes (ContextCompaction
/// §13.4).
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
enum ItemClass {
    Task,
    Constraint,
    Decision,
    RuledOut,
    OpenItem,
    Judgment,
    NextStep,
}
impl ItemClass {
    fn name(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::RuledOut => "ruledOut",
            Self::OpenItem => "openItem",
            Self::Judgment => "judgment",
            Self::NextStep => "nextStep",
        }
    }
}
const ITEMS: [ItemClass; 7] = [
    ItemClass::Task,
    ItemClass::Constraint,
    ItemClass::Decision,
    ItemClass::RuledOut,
    ItemClass::OpenItem,
    ItemClass::Judgment,
    ItemClass::NextStep,
];

fn cases_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CASES)
}
fn read_cases(directory: &Path) -> Result<Vec<Case>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let bytes = std::fs::read(&file)?;
        let raw: CaseFile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        let case = raw
            .into_case()
            .map_err(|error| format!("{}: {error}", file.display()))?;
        validate_probes(&case.probes).map_err(|error| format!("{}: {error}", file.display()))?;
        cases.push(case);
    }
    Ok(cases)
}

/// `item` names the 3b class a `fact` probe checks; on another kind it would
/// have no meaning, because the breakdown counts `fact` probes only.
fn validate_probes(probes: &[Probe]) -> std::result::Result<(), String> {
    for probe in probes {
        if probe.item.is_some() && probe.kind != ProbeKind::Fact {
            return Err(format!(
                "probe {}: item applies to fact probes only",
                probe.id
            ));
        }
    }
    Ok(())
}

/// Each step's only cut is `retainFrom`, so it must not separate a tool call
/// from its result (ContextCompaction §4.3, §13.4); in a chain, step k must
/// repeat step k−1's retained tail verbatim, because a real compaction keeps
/// it and the next compaction sees it before the new messages.
fn validate_steps(steps: &[Step]) -> std::result::Result<(), String> {
    for (index, step) in steps.iter().enumerate() {
        validate_cut(&step.messages, step.retain_from)
            .map_err(|error| format!("step {}: {error}", index + 1))?;
        let Some(previous) = index.checked_sub(1).map(|index| &steps[index]) else {
            continue;
        };
        if !step
            .messages
            .starts_with(&previous.messages[previous.retain_from..])
        {
            return Err(format!(
                "step {} does not begin with step {}'s retained tail",
                index + 1,
                index
            ));
        }
    }
    Ok(())
}
fn validate_cut(messages: &[Message], retain_from: usize) -> std::result::Result<(), String> {
    if retain_from > messages.len() {
        return Err(format!(
            "retainFrom {retain_from} is past the {} messages",
            messages.len()
        ));
    }
    let calls = tool_uses(&messages[..retain_from]);
    for message in &messages[retain_from..] {
        for id in tool_results(std::slice::from_ref(message)) {
            if calls.contains(&id) {
                return Err(format!(
                    "retainFrom {retain_from} separates the tool call {id} from its result"
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

/// One step of a case as the strategy sees it: the conversation request, the
/// history with its `m<i>` sources, and its single cut.
struct PreparedStep {
    request: Arc<ModelRequest>,
    history: Vec<HistoryEntry>,
    retain_from: usize,
    model: Arc<dyn ContextModel>,
}
impl PreparedStep {
    async fn new(
        step: &Step,
        selection: &ModelSelection,
        window: Option<u64>,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let mut request = ModelRequest::new(
            &selection.provider_id,
            &selection.model_id,
            step.messages.clone(),
        );
        request.capability_snapshot = selection.capability_snapshot.clone();
        request.config = selection.config.clone();
        if let Some(window) = window {
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
                "a window of {} leaves no input budget with {} output tokens",
                model.context_window(),
                model.output_tokens()
            )
            .into());
        }
        let history = step
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
            request,
            history,
            retain_from: step.retain_from,
            model,
        })
    }
    fn input(&self, checkpoint: Option<CompactionCheckpoint>) -> CompactionInput {
        let retain_from = format!("m{}", self.retain_from);
        CompactionInput {
            history: self.history.clone(),
            checkpoint,
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

/// A case as the runner runs it: its steps, the first step's checkpoint, and
/// the window facts the strategy ran with, after the case's narrowing.
struct Prepared {
    id: String,
    description: String,
    probes: Vec<Probe>,
    steps: Vec<PreparedStep>,
    checkpoint: Option<CompactionCheckpoint>,
    window: f64,
    output: f64,
}
impl Prepared {
    async fn new(
        case: &Case,
        selection: &ModelSelection,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let mut steps = Vec::new();
        for step in &case.steps {
            steps.push(
                PreparedStep::new(step, selection, case.context_window, cancel)
                    .await
                    .map_err(|error| format!("case {}: {error}", case.id))?,
            );
        }
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
            window: steps[0].model.context_window(),
            output: steps[0].model.output_tokens(),
            steps,
        })
    }
}
/// One step's compaction of the shipped strategy, with what it took.
struct RunStep {
    duration: Duration,
    outcome: std::result::Result<StrategyOutcome, HostError>,
}
impl RunStep {
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
    /// The next chain step's checkpoint: the completed replacement itself, as
    /// a committed compaction becomes the next checkpoint (ContextCompaction
    /// §13.4).
    fn checkpoint(&self) -> Option<CompactionCheckpoint> {
        match &self.outcome {
            Ok(StrategyOutcome::Completed { replacement, .. }) => {
                let replacement: Arc<[Message]> = replacement.as_slice().into();
                Some(CompactionCheckpoint {
                    retain_from: "m0".into(),
                    replacement_parts: Some(ReplacementParts {
                        strategy: replacement.len() as u64,
                        digest: 0,
                    }),
                    replacement,
                    context_baseline: Arc::from([]),
                })
            }
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

/// A case's run: one compaction, or a chain whose steps stop at the first
/// that does not complete (ContextCompaction §13.4).
struct Run {
    steps: Vec<RunStep>,
    duration: Duration,
}
impl Run {
    fn label(&self) -> &'static str {
        if self.steps.iter().any(|step| step.label() == "failed") {
            "failed"
        } else if self.steps.iter().any(|step| step.label() == "skipped") {
            "skipped"
        } else {
            "completed"
        }
    }
    fn error(&self) -> Option<String> {
        self.steps.last().and_then(RunStep::error)
    }
    fn summary(&self) -> Option<String> {
        self.steps.last().and_then(RunStep::summary)
    }
    fn diagnostics(&self) -> Option<Value> {
        self.steps.last().and_then(RunStep::diagnostics)
    }
    /// The steps that ran, each with what a single run records.
    fn steps_json(&self) -> Vec<Value> {
        self.steps
            .iter()
            .map(|step| {
                json!({
                    "outcome": step.label(),
                    "error": step.error(),
                    "durationMs": step.duration.as_secs_f64() * 1000.,
                    "summary": step.summary(),
                    "diagnostics": step.diagnostics(),
                })
            })
            .collect()
    }
    /// Whether any step's summarizer fell back from the arm's reasoning value.
    fn fell_back(&self, arm: &Arm) -> bool {
        self.steps.iter().any(|step| {
            step.details()
                .get("reasoning")
                .and_then(Value::as_str)
                .is_some_and(|applied| applied != arm.reasoning.name())
        })
    }
    fn cut_offs(&self) -> u64 {
        self.steps
            .iter()
            .map(|step| {
                step.details()
                    .get("cutOffCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .sum()
    }
    fn tokens(&self) -> Option<f64> {
        self.steps
            .last()?
            .details()
            .get("estimatedSummaryTokens")
            .and_then(Value::as_f64)
    }
    fn target_tokens(&self) -> Option<f64> {
        self.steps
            .last()?
            .details()
            .get("targetTokens")
            .and_then(Value::as_f64)
    }
}

async fn run_case(strategy: &SummaryStrategy, case: &Prepared, cancel: &CancellationToken) -> Run {
    let started = Instant::now();
    let mut checkpoint = case.checkpoint.clone();
    let mut steps = Vec::new();
    for prepared in &case.steps {
        let step_started = Instant::now();
        let outcome = strategy
            .compact(prepared.input(checkpoint.take()), cancel.clone())
            .await;
        let step = RunStep {
            duration: step_started.elapsed(),
            outcome,
        };
        let next = step.checkpoint();
        let done = step.label() != "completed";
        steps.push(step);
        if done {
            break;
        }
        checkpoint = next;
    }
    Run {
        steps,
        duration: started.elapsed(),
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
                    if normalize(&unescape(summary)).contains(&normalize(&probe.text)) {
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

/// The inverse of the escaping the summary envelope applies to the summary's
/// text (`context_block::text`), one level: the summary as the model wrote it.
fn unescape(text: &str) -> String {
    const ENTITIES: [(&str, char); 3] = [("&lt;", '<'), ("&gt;", '>'), ("&amp;", '&')];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        match ENTITIES.iter().find(|(entity, _)| rest.starts_with(entity)) {
            Some((entity, value)) => {
                out.push(*value);
                rest = &rest[entity.len()..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The arm's metrics over its cases and runs (ContextCompaction §13.4).
#[derive(Default)]
struct Metrics {
    exact: (usize, usize),
    facts: (usize, usize),
    /// `fact` probes that carry an `item`, counted per 3b class
    /// (ContextCompaction §13.4).
    items: BTreeMap<ItemClass, (usize, usize)>,
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
        if run.fell_back(arm) {
            self.fell_back += 1;
        }
        self.cut_offs += run.cut_offs();
        if let Some(tokens) = run.tokens() {
            self.tokens.push(tokens);
            if let Some(target) = run.target_tokens() {
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
                ProbeKind::Fact => {
                    self.facts = count(self.facts, present);
                    if let Some(item) = probe.item {
                        let counts = self.items.entry(item).or_default();
                        *counts = count(*counts, present);
                    }
                }
                ProbeKind::Absent if present => self.violations += 1,
                ProbeKind::Absent => {}
            }
        }
    }
    fn row(&self, arm: &Arm) -> Value {
        let by_item: BTreeMap<&str, Value> = self
            .items
            .iter()
            .map(|(item, counts)| {
                (
                    item.name(),
                    json!({
                        "present": counts.0,
                        "total": counts.1,
                        "recall": recall(*counts),
                    }),
                )
            })
            .collect();
        json!({
            "arm": arm.id,
            "reasoning": arm.reasoning.name(),
            "fellBack": self.fell_back,
            "target": arm.target.map(|target| json!({
                "outputShare": target.output_share,
                "inputShare": target.input_share,
            })),
            "factRecall": recall(self.facts),
            "factRecallByItem": by_item,
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

/// The §6 verdict of the candidate arm against the current one
/// (ContextCompaction §13.4): `δ = 0.05`, a rate with no probe of its kind is
/// left out, and the failures are the `failed` runs.
fn verdict(current: &Metrics, candidate: &Metrics) -> String {
    const DELTA: f64 = 0.05;
    let rate = |current: (usize, usize), candidate: (usize, usize)| match (
        recall(current),
        recall(candidate),
    ) {
        (Some(current), Some(candidate)) => candidate >= current - DELTA,
        _ => true,
    };
    let passes = rate(current.facts, candidate.facts)
        && rate(current.exact, candidate.exact)
        && candidate.violations <= current.violations
        && candidate.failed <= current.failed;
    format!(
        "candidate {}; median T: current {}, candidate {}",
        if passes { "passes" } else { "fails" },
        median_tokens(current),
        median_tokens(candidate),
    )
}
fn median_tokens(metrics: &Metrics) -> String {
    percentile(&metrics.tokens, 0.5).map_or_else(|| "–".to_owned(), |tokens| format!("{tokens:.0}"))
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
    if let Some(items) = item_table(rows) {
        text.push('\n');
        text.push_str(&items);
    }
    text
}

/// `F` per 3b class: one row per `item` a `fact` probe names, one column per
/// arm (ContextCompaction §13.4).
fn item_table(rows: &[Value]) -> Option<String> {
    let items: Vec<ItemClass> = ITEMS
        .into_iter()
        .filter(|item| {
            rows.iter()
                .any(|row| !row["factRecallByItem"][item.name()].is_null())
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut text = String::from("| item |");
    for row in rows {
        text += &format!(" {} |", row["arm"].as_str().unwrap_or_default());
    }
    text.push_str("\n| --- |");
    for _ in rows {
        text.push_str(" --- |");
    }
    text.push('\n');
    for item in items {
        text += &format!("| {} |", item.name());
        for row in rows {
            let recall = row["factRecallByItem"][item.name()]["recall"].as_f64();
            let cell = recall.map_or_else(|| "–".to_owned(), |recall| format!("{recall:.3}"));
            text += &format!(" {cell} |");
        }
        text.push('\n');
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dim_agent_capabilities::host::{
        HostFuture, ModelConnection, ModelDriver, ModelStopReason, ResolveModelConnection,
        ResolvedModelConnection,
    };
    use dim_agent_types::ModelCapabilitySnapshot;
    use dim_app_server::model_catalog::FieldUpdate;
    use dim_app_server::provider::{CreateProvider, PreferencesPatch};
    use std::sync::Mutex;

    /// The conversation request carries the model's saved preferences, as the
    /// bundled client's `session.create` does; without saved preferences it
    /// stays the model config alone (ContextCompaction §13.4).
    #[tokio::test]
    async fn summary_eval_selection_carries_saved_model_preferences() {
        let dir = crate::app_server::scratch("summary-eval-preferences").unwrap();
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let server = crate::app_server::serve_home(&home, &dir, dir.join("command-output"), |_| {})
            .await
            .unwrap();
        let providers = server
            .service::<ProviderService>()
            .expect("the product profile serves a provider catalog");
        let cancel = CancellationToken::new();
        let provider_id = "00000000-0000-4000-8000-0000000000aa";
        let created = providers
            .create(
                serde_json::from_value::<CreateProvider>(json!({
                    "providerId": provider_id,
                    "displayName": "local",
                    "connection": {
                        "driverKind": "openai-compatible",
                        "auth": {"type": "anonymous"},
                        "baseUrl": "http://127.0.0.1:1"
                    },
                    "models": [{
                        "modelId": "deepseek",
                        "displayName": "DeepSeek",
                        "status": "active",
                        "capabilities": {
                            "modalities": ["text"], "toolCalling": true, "streaming": true,
                            "structuredOutput": true, "vision": false, "audio": false,
                            "reasoning": true, "contextWindow": 128000, "maxOutputTokens": 321
                        },
                        "metadata": {"reasoning": {"supported": true, "effortOptions": ["low", "high", "max"]}}
                    }],
                    "enabled": true
                }))
                .unwrap(),
                &cancel,
            )
            .await
            .unwrap();
        let target = ModelId {
            provider_id: provider_id.into(),
            model_id: "deepseek".into(),
        };
        let plain = selection_with_preferences(&providers, &target, &cancel)
            .await
            .unwrap()
            .config
            .expect("a conversation config");
        assert_eq!(plain.max_tokens, Some(321.));
        assert_eq!(plain.reasoning, None);
        providers
            .update_preferences(
                provider_id.into(),
                created.version.expect("a provider version"),
                "deepseek".into(),
                PreferencesPatch {
                    reasoning_effort: Some(FieldUpdate::Set {
                        value: "max".into(),
                    }),
                    service_tier: None,
                },
                &cancel,
            )
            .await
            .unwrap();
        let saved = selection_with_preferences(&providers, &target, &cancel)
            .await
            .unwrap()
            .config
            .expect("a conversation config");
        assert_eq!(saved.max_tokens, Some(321.));
        assert_eq!(
            saved.reasoning.and_then(|reasoning| reasoning.effort),
            Some("max".into())
        );
        server.shutdown();
        server.join().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every committed case parses and validates without a model call.
    #[test]
    fn summary_eval_cases_parse_and_validate() {
        let cases = read_cases(&cases_directory()).unwrap_or_else(|error| panic!("{error}"));
        assert!(
            cases.iter().any(|case| case.id == "chain"),
            "the chain case is present"
        );
    }

    /// Answers each summarizer request with `summary-of-step-N`, N being the
    /// request's position, and keeps every request.
    #[derive(Default)]
    struct Markers {
        requests: Mutex<Vec<Arc<ModelRequest>>>,
    }
    impl ModelDriver for Markers {
        fn invoke(
            &self,
            request: Arc<ModelRequest>,
            _: ModelConnection,
            _: CancellationToken,
        ) -> HostFuture<'_, ModelStream> {
            let text = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request);
                format!("summary-of-step-{}", requests.len())
            };
            Box::pin(async move {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(ModelChunk::MessageStart {
                        message_id: "reply".into(),
                    }),
                    Ok(ModelChunk::Text(text)),
                    Ok(ModelChunk::MessageStop {
                        message_id: "reply".into(),
                        reason: ModelStopReason::EndTurn,
                    }),
                ])) as ModelStream)
            })
        }
    }
    struct Local;
    impl ResolveModelConnection for Local {
        fn resolve(
            &self,
            _: Arc<ModelRequest>,
            _: CancellationToken,
        ) -> HostFuture<'_, ResolvedModelConnection> {
            Box::pin(async {
                Ok(ResolvedModelConnection {
                    connection: ModelConnection::Local,
                    failure_policy: None,
                    recovery: None,
                })
            })
        }
    }

    /// A chain hands each step the previous step's completed replacement
    /// unchanged — one summary envelope around one marker, the way a
    /// committed compaction becomes the next checkpoint (ContextCompaction
    /// §13.4).
    #[tokio::test]
    async fn a_chain_carries_each_completed_replacement_unchanged() {
        let cases = read_cases(&cases_directory()).unwrap();
        let case = cases
            .iter()
            .find(|case| case.id == "chain")
            .expect("the chain case");
        let driver = Arc::new(Markers::default());
        let strategy = SummaryStrategy::new(
            Arc::new(ModelHost(driver.clone())),
            Arc::new(ProviderConnectionResolver(Arc::new(Local))),
            ProductSummaryStrategyConfig::default(),
        );
        let snapshot: ModelCapabilitySnapshot = serde_json::from_value(json!({
            "target": {"providerId": "fake", "modelId": "fake"},
            "capabilities": {"modalities": ["text"], "toolCalling": true, "streaming": true,
                "structuredOutput": false, "vision": false, "audio": false, "reasoning": false,
                "contextWindow": 128_000, "maxOutputTokens": 16_384}
        }))
        .unwrap();
        let selection = ModelSelection {
            provider_id: "fake".into(),
            model_id: "fake".into(),
            capability_snapshot: Some(snapshot),
            config: None,
            metadata: None,
        };
        let cancel = CancellationToken::new();
        let prepared = Prepared::new(case, &selection, &cancel).await.unwrap();
        let run = run_case(&strategy, &prepared, &cancel).await;
        assert_eq!(run.steps.len(), CHAIN_STEPS, "every step completed");
        assert!(
            run.steps.iter().all(|step| step.label() == "completed"),
            "{:?}",
            run.steps.iter().map(RunStep::label).collect::<Vec<_>>()
        );
        let requests = driver.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), CHAIN_STEPS, "one chunk per step");
        for (index, request) in requests.iter().enumerate().skip(1) {
            let text = request
                .messages
                .iter()
                .map(|message| render(std::slice::from_ref(message.as_ref())))
                .collect::<Vec<_>>()
                .join("\n");
            let marker = format!("summary-of-step-{index}");
            assert_eq!(
                text.matches(&marker).count(),
                1,
                "step {} does not carry {marker} exactly once:\n{text}",
                index + 1
            );
            assert_eq!(
                text.matches("<dim-summary>").count(),
                1,
                "step {} does not carry one summary envelope:\n{text}",
                index + 1
            );
            assert_eq!(
                text.matches("&lt;dim-summary&gt;").count(),
                0,
                "step {} carries an escaped summary envelope:\n{text}",
                index + 1
            );
        }
        for (index, step) in run.steps.iter().enumerate() {
            let marker = summary_message(&format!("summary-of-step-{}", index + 1));
            assert_eq!(
                step.summary(),
                Some(render(&[marker])),
                "the report and the judge read the completed replacement"
            );
        }
    }

    /// An `exact` probe matches the summary as the model wrote it: the stored
    /// replacement is the escaped envelope, and a probe that contains `&`,
    /// `<` or `>` must still match its text (ContextCompaction §13.4).
    #[tokio::test]
    async fn exact_probes_match_the_text_the_model_wrote() {
        struct Reply;
        impl ModelDriver for Reply {
            fn invoke(
                &self,
                _: Arc<ModelRequest>,
                _: ModelConnection,
                _: CancellationToken,
            ) -> HostFuture<'_, ModelStream> {
                let text = "The runner returns Box<dyn Error> and keeps a && b verbatim.";
                Box::pin(async move {
                    Ok(Box::pin(futures_util::stream::iter(vec![
                        Ok(ModelChunk::MessageStart {
                            message_id: "reply".into(),
                        }),
                        Ok(ModelChunk::Text(text.into())),
                        Ok(ModelChunk::MessageStop {
                            message_id: "reply".into(),
                            reason: ModelStopReason::EndTurn,
                        }),
                    ])) as ModelStream)
                })
            }
        }
        let cases = read_cases(&cases_directory()).unwrap();
        let case = cases
            .iter()
            .find(|case| case.id == "code")
            .expect("the code case");
        let driver = Arc::new(Reply);
        let strategy = SummaryStrategy::new(
            Arc::new(ModelHost(driver)),
            Arc::new(ProviderConnectionResolver(Arc::new(Local))),
            ProductSummaryStrategyConfig::default(),
        );
        let snapshot: ModelCapabilitySnapshot = serde_json::from_value(json!({
            "target": {"providerId": "fake", "modelId": "fake"},
            "capabilities": {"modalities": ["text"], "toolCalling": true, "streaming": true,
                "structuredOutput": false, "vision": false, "audio": false, "reasoning": false,
                "contextWindow": 128_000, "maxOutputTokens": 16_384}
        }))
        .unwrap();
        let selection = ModelSelection {
            provider_id: "fake".into(),
            model_id: "fake".into(),
            capability_snapshot: Some(snapshot),
            config: None,
            metadata: None,
        };
        let cancel = CancellationToken::new();
        let prepared = Prepared::new(case, &selection, &cancel).await.unwrap();
        let run = run_case(&strategy, &prepared, &cancel).await;
        let summary = run.summary().expect("the reply is a summary");
        assert!(
            !summary.contains("Box<dyn Error>"),
            "the envelope escaped the summary: {summary}"
        );
        let probes = vec![
            Probe {
                id: "error-type".into(),
                kind: ProbeKind::Exact,
                text: "Box<dyn Error>".into(),
                item: None,
            },
            Probe {
                id: "and".into(),
                kind: ProbeKind::Exact,
                text: "a && b".into(),
                item: None,
            },
        ];
        let records = probe_records(&probes, &summary, &Judged::NoSummary);
        assert_eq!(records[0]["verdict"], "present", "{records:?}");
        assert_eq!(records[1]["verdict"], "present", "{records:?}");
    }
}
