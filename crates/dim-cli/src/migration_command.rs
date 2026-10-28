use crate::{Result, args::Options, sanitize::safe};
use dim_migration::{Migration, MigrationPlugin, TaskStatus};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) enum Operation {
    Inspect,
    Run,
    Retry(String),
    List,
    Show(String),
    Cancel(String),
}

#[derive(Clone, Debug)]
pub(crate) struct Command {
    pub operation: Operation,
    pub source: Option<PathBuf>,
    pub source_stopped: bool,
}

const USAGE: &str = "Use migration inspect|run --source OLD_HOME --source-stopped, migration retry TASK --source-stopped [--source OLD_HOME], or migration list|show TASK|cancel TASK";

pub(crate) fn parse(args: &mut impl Iterator<Item = String>) -> Result<crate::args::Action> {
    let operation = args.next();
    if matches!(operation.as_deref(), Some("--help" | "-h")) {
        return Ok(crate::args::Action::Help);
    }
    let operation = match operation.as_deref() {
        Some("inspect") => Operation::Inspect,
        Some("run") => Operation::Run,
        Some("retry") => Operation::Retry(args.next().ok_or(USAGE)?),
        Some("list") => Operation::List,
        Some("show") => Operation::Show(args.next().ok_or(USAGE)?),
        Some("cancel") => Operation::Cancel(args.next().ok_or(USAGE)?),
        _ => return Err(USAGE.into()),
    };
    if matches!(&operation,Operation::Retry(id)|Operation::Show(id)|Operation::Cancel(id) if id=="--help" || id=="-h")
    {
        return Ok(crate::args::Action::Help);
    }
    Ok(crate::args::Action::Migration(Command {
        operation,
        source: None,
        source_stopped: false,
    }))
}

pub(crate) async fn run(root: &rutis::Ctx, options: &Options, command: &Command) -> Result<i32> {
    if matches!(
        command.operation,
        Operation::Inspect | Operation::Run | Operation::Retry(_)
    ) && !command.source_stopped
    {
        return Err(
            "Exit legacy CLI and Desktop using the source home, then pass --source-stopped".into(),
        );
    }
    if matches!(command.operation, Operation::Inspect | Operation::Run) && command.source.is_none()
    {
        return Err(USAGE.into());
    }
    if matches!(
        command.operation,
        Operation::List | Operation::Show(_) | Operation::Cancel(_)
    ) && (command.source.is_some() || command.source_stopped)
    {
        return Err(USAGE.into());
    }
    if let Some(source) = &command.source {
        dim_migration::validate_source_home(source, &options.home)?;
    }
    std::fs::create_dir_all(&options.home)?;
    let themes = std::iter::once("system")
        .chain(
            crate::ui::theme::themes::THEMES
                .iter()
                .map(|theme| theme.id),
        )
        .map(str::to_owned)
        .collect();
    let plugin = root.plugin(MigrationPlugin::new(options.home.clone(), themes));
    (&plugin).await.map_err(crate::plugin_error)?;
    let service = root
        .get::<Migration>()
        .ok_or("Migration plugin provided no service")?;
    let result = execute(root, &service, options, command).await;
    plugin.dispose().await?;
    result
}

async fn execute(
    root: &rutis::Ctx,
    service: &std::sync::Arc<Migration>,
    options: &Options,
    command: &Command,
) -> Result<i32> {
    match &command.operation {
        Operation::List => {
            println!("{}", serde_json::to_string_pretty(&service.list().await?)?);
            Ok(0)
        }
        Operation::Show(id) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&service.report(id).await?)?
            );
            Ok(0)
        }
        Operation::Cancel(id) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&service.cancel(id).await?)?
            );
            Ok(0)
        }
        Operation::Inspect => {
            let source = std::fs::canonicalize(command.source.as_ref().ok_or(USAGE)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&service.inspect(source, true).await?)?
            );
            Ok(0)
        }
        Operation::Run | Operation::Retry(_) => {
            let retry = match &command.operation {
                Operation::Retry(id) => Some(id.clone()),
                _ => None,
            };
            let source = match &command.source {
                Some(source) => source.clone(),
                None => {
                    service
                        .report(retry.as_deref().ok_or(USAGE)?)
                        .await?
                        .task
                        .source_home
                }
            };
            let source = dim_migration::validate_source_home(&source, &options.home)?;
            let (_, connection) = crate::daemon_link(root, options, true).await?;
            let connection = connection
                .take()
                .ok_or("App Server did not start")?
                .connection_config()
                .clone();
            let mut progress = service.progress();
            let future = service.run(source, true, retry, connection);
            tokio::pin!(future);
            let mut interrupted = false;
            let report = loop {
                tokio::select! {
                    result=&mut future=>break result?,
                    _=tokio::signal::ctrl_c(),if !interrupted=>{interrupted=true;service.cancel_current();eprintln!("Stopping migration; waiting for accepted work to settle...");},
                    result=progress.changed()=>{
                        if result.is_ok() && !options.json && let Some(state)=progress.borrow_and_update().as_ref() {
                            eprintln!("{}  {}  {}/{}  {}",state.id,safe(&state.stage),state.processed,state.total.map(|total|total.to_string()).unwrap_or_else(||"?".into()),safe(state.current.as_deref().unwrap_or("")));
                        }
                    },
                }
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(match report.task.status {
                TaskStatus::Completed => 0,
                TaskStatus::Partial => 2,
                TaskStatus::Cancelled => 130,
                _ => 1,
            })
        }
    }
}
