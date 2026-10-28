use crate::{Result, args::Options, sanitize::safe};
use dim_migration::{Migration, MigrationPlugin, TaskStatus};
use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

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

const USAGE: &str = "Use migration [run|inspect] [--source OLD_HOME], migration retry TASK [--source OLD_HOME], or migration list|show TASK|cancel TASK; scripts must pass --source-stopped";

pub(crate) fn parse(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> Result<crate::args::Action> {
    let operation = match args.peek() {
        Some(value) if value.starts_with('-') => None,
        _ => args.next(),
    };
    let operation = match operation.as_deref() {
        None => Operation::Run,
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
    let mut command = command.clone();
    if matches!(
        command.operation,
        Operation::Inspect | Operation::Run | Operation::Retry(_)
    ) && !command.source_stopped
        && !std::io::stdin().is_terminal()
    {
        return Err(
            "Exit legacy CLI and Desktop using the source home, then pass --source-stopped".into(),
        );
    }
    if matches!(
        command.operation,
        Operation::List | Operation::Show(_) | Operation::Cancel(_)
    ) && (command.source.is_some() || command.source_stopped)
    {
        return Err(USAGE.into());
    }
    if matches!(command.operation, Operation::Retry(_))
        && let Some(source) = &command.source
    {
        dim_migration::validate_source_home(source, &options.home)?;
    }
    if matches!(command.operation, Operation::Inspect | Operation::Run) {
        let source = match command.source.take() {
            Some(source) => source,
            None => default_source()?,
        };
        let source = dim_migration::validate_source_home(&source, &options.home)?;
        if !command.source_stopped && !confirm_source(&source, &options.home)? {
            return Ok(130);
        }
        command.source = Some(source);
        command.source_stopped = true;
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
    let result = execute(root, &service, options, &command).await;
    plugin.dispose().await?;
    result
}

fn default_source() -> Result<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or("Cannot locate the legacy home; pass --source OLD_HOME")?;
    let mut sources = [
        (".dimcode/v2", "dimcode.sqlite"),
        (".dim/v2", "dimglobal.sqlite"),
    ]
    .into_iter()
    .map(|(directory, database)| (home.join(directory), database))
    .filter(|(path, database)| path.join(database).is_file())
    .map(|(path, _)| path);
    let source = sources.next().ok_or(
        "No default legacy v2 database found; pass --source OLD_HOME for a custom location",
    )?;
    if sources.next().is_some() {
        return Err("Both legacy v2 homes exist; select one with --source OLD_HOME".into());
    }
    Ok(source)
}

fn confirm_source(source: &Path, target: &Path) -> Result<bool> {
    eprintln!("Source: {}", safe(&source.to_string_lossy()));
    eprintln!("Target: {}", safe(&target.to_string_lossy()));
    eprint!("Close legacy CLI and Desktop using this source. Continue? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
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
            if !command.source_stopped && !confirm_source(&source, &options.home)? {
                return Ok(130);
            }
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
