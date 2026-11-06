//! A host-owned helper process: its own file and network boundary, a timeout
//! and bounded output, see
//! `docs/architecture/rust-agent/plugins/HostPlatform.md`.
use super::{HostPlatform, ProcessSpec, ProcessTree, env_var};
use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{ChildStderr, ChildStdout},
};
use tokio_util::sync::CancellationToken;

/// The boundary of one host-owned helper.
#[derive(Debug)]
pub struct Confinement {
    /// Files or directories the helper may read, each absolute and existing.
    pub read: Vec<PathBuf>,
    /// Directories the helper may read and write, each absolute and existing;
    /// the first is its working directory.
    pub write: Vec<PathBuf>,
    /// Bounds the whole call, including output collection.
    pub timeout: Duration,
    /// The most bytes the helper may write to standard output.
    pub stdout_limit: usize,
    /// The most bytes the helper may write to standard error.
    pub stderr_limit: usize,
}

/// The program a host-owned helper starts. Its working directory is the first
/// write directory of the [`Confinement`].
#[derive(Debug)]
pub struct ConfinedSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    /// Entries added to the minimal environment allowlist. The service's other
    /// variables never reach the helper.
    pub env: Vec<(OsString, OsString)>,
}

/// How a confined helper ended and what it printed. Output is present only
/// when the helper exited within its timeout and output limits.
#[derive(Debug)]
pub struct ConfinedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// False only where the platform cannot confine.
    pub confined: bool,
}

/// The output stream that reached its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfinedStream {
    Stdout,
    Stderr,
}

/// Why a confined helper produced no output.
#[derive(Debug)]
pub enum ConfinedError {
    /// A path was relative, missing, or of the wrong kind. Nothing started.
    InvalidPath { path: PathBuf, reason: String },
    /// The helper could not start, or its output could not be read.
    Spawn(io::Error),
    /// The helper outlived its timeout; its process tree was killed.
    TimedOut,
    /// The helper filled one stream past its limit; its process tree was
    /// killed.
    OutputLimit { stream: ConfinedStream },
    /// The caller cancelled; the process tree was killed.
    Cancelled,
}

impl std::fmt::Display for ConfinedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

impl std::fmt::Display for ConfinedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath { path, reason } if path.as_os_str().is_empty() => {
                write!(f, "invalid confinement path: {reason}")
            }
            Self::InvalidPath { path, reason } => {
                write!(f, "invalid confinement path {}: {reason}", path.display())
            }
            Self::Spawn(error) => write!(f, "the confined helper could not start: {error}"),
            Self::TimedOut => f.write_str("the confined helper exceeded its timeout"),
            Self::OutputLimit { stream } => {
                write!(f, "the confined helper exceeded its {stream} limit")
            }
            Self::Cancelled => f.write_str("the confined helper was cancelled"),
        }
    }
}

impl std::error::Error for ConfinedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            _ => None,
        }
    }
}

impl HostPlatform {
    /// Starts a host-owned helper under its own boundary and collects its
    /// bounded output. The helper reads the system baseline, its executable
    /// and `confinement.read`; writes only inside `confinement.write`; has no
    /// network; receives the minimal environment allowlist plus `spec.env`;
    /// and runs in the first write directory. Seatbelt on macOS and bubblewrap
    /// on Linux apply whatever the user's Process isolation switch says,
    /// because that switch governs agent commands and this is a host-owned
    /// helper. Where the platform cannot confine (Windows, and Linux without
    /// bubblewrap) the helper runs with the same timeout, caps, environment
    /// and cancellation and the result reports `confined: false`; the caller
    /// decides whether that is acceptable. Windows has no confinement here, as
    /// it has none for agent commands.
    ///
    /// The timeout, either output limit and `cancel` kill the helper's process
    /// tree with the existing tree-kill and return the typed error. Both
    /// streams are read concurrently, so a full pipe never blocks the helper.
    /// No partial output is returned with an error.
    pub async fn spawn_confined(
        &self,
        spec: ConfinedSpec,
        confinement: Confinement,
        cancel: CancellationToken,
    ) -> Result<ConfinedOutput, ConfinedError> {
        let (process, confined) = self.confined_spec(spec, &confinement).await?;
        if cancel.is_cancelled() {
            return Err(ConfinedError::Cancelled);
        }
        let mut tree = self.spawn(process).map_err(ConfinedError::Spawn)?;
        let stdout = tree.stdout();
        let stderr = tree.stderr();
        let outcome = {
            let collect = collect(&mut tree, stdout, stderr, &confinement);
            tokio::pin!(collect);
            tokio::select! {
                biased;
                outcome = &mut collect => outcome,
                _ = cancel.cancelled() => Outcome::Cancelled,
                _ = tokio::time::sleep(confinement.timeout) => Outcome::TimedOut,
            }
        };
        match outcome {
            Outcome::Exited {
                status,
                stdout,
                stderr,
            } => Ok(ConfinedOutput {
                status,
                stdout,
                stderr,
                confined,
            }),
            Outcome::OutputLimit { stream } => {
                Err(killed(&mut tree, ConfinedError::OutputLimit { stream }).await)
            }
            Outcome::Failed(error) => Err(killed(&mut tree, ConfinedError::Spawn(error)).await),
            Outcome::TimedOut => Err(killed(&mut tree, ConfinedError::TimedOut).await),
            Outcome::Cancelled => Err(killed(&mut tree, ConfinedError::Cancelled).await),
        }
    }

    /// The process to start, with its environment, working directory and, where
    /// the platform can confine, its boundary. The paths of `confinement` are
    /// checked before anything starts.
    async fn confined_spec(
        &self,
        spec: ConfinedSpec,
        confinement: &Confinement,
    ) -> Result<(ProcessSpec, bool), ConfinedError> {
        let Some(cwd) = confinement.write.first() else {
            return Err(ConfinedError::InvalidPath {
                path: PathBuf::new(),
                reason: "write needs at least one directory".into(),
            });
        };
        let cwd = cwd.clone();
        for path in &confinement.write {
            resolve(path, Kind::Directory).await?;
        }
        for path in &confinement.read {
            resolve(path, Kind::Any).await?;
        }
        resolve(&spec.executable, Kind::File).await?;
        let process = ProcessSpec {
            executable: spec.executable,
            args: spec.args,
            cwd,
            env: self.confined_environment(&spec.env),
        };

        #[cfg(unix)]
        {
            let mut read_paths = super::isolation::baseline_read_paths(self, &process.env)
                .await
                .map_err(|reason| ConfinedError::Spawn(io::Error::other(reason)))?;
            read_paths.extend(confinement.read.iter().cloned());
            read_paths.push(process.executable.clone());
            let (read_only, read_only_files) = super::isolation::roots(read_paths, true)
                .await
                .map_err(|reason| ConfinedError::Spawn(io::Error::other(reason)))?;
            let (writable, _) = super::isolation::roots(confinement.write.clone(), false)
                .await
                .map_err(|reason| ConfinedError::Spawn(io::Error::other(reason)))?;
            #[cfg(all(unix, not(target_os = "macos")))]
            let backend = super::isolation::bubblewrap(&self.env).is_some();
            #[cfg(target_os = "macos")]
            let backend = true;
            if !backend {
                return Ok((process, false));
            }
            let prepared = super::isolation::prepare(
                self,
                process,
                &read_only,
                &read_only_files,
                &writable,
                false,
            )
            .map_err(|error| ConfinedError::Spawn(io::Error::other(error.reason)))?;
            Ok((prepared, true))
        }
        #[cfg(windows)]
        Ok((process, false))
    }

    /// The minimal environment allowlist from the captured service environment,
    /// with the helper's own entries replacing a name it repeats.
    fn confined_environment(&self, extra: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        for name in super::ENVIRONMENT {
            if let Some(value) = env_var(&self.env, name) {
                env.push((OsString::from(*name), value.to_os_string()));
            }
        }
        for (name, value) in extra {
            match env
                .iter_mut()
                .find(|(existing, _)| same_name(existing, name))
            {
                Some((_, current)) => *current = value.clone(),
                None => env.push((name.clone(), value.clone())),
            }
        }
        env
    }
}

/// Environment names are case-insensitive on Windows.
fn same_name(left: &OsStr, right: &OsStr) -> bool {
    if cfg!(windows) {
        left.to_str()
            .zip(right.to_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    } else {
        left == right
    }
}

#[derive(Clone, Copy)]
enum Kind {
    File,
    Directory,
    Any,
}

/// Checks that one path is absolute, exists, resolves and has the required
/// kind.
async fn resolve(path: &Path, kind: Kind) -> Result<(), ConfinedError> {
    let invalid = |reason: String| ConfinedError::InvalidPath {
        path: path.to_owned(),
        reason,
    };
    if !path.is_absolute() {
        return Err(invalid("must be absolute".into()));
    }
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let matches = match kind {
        Kind::File => metadata.is_file(),
        Kind::Directory => metadata.is_dir(),
        Kind::Any => metadata.is_file() || metadata.is_dir(),
    };
    if !matches {
        return Err(invalid(
            match kind {
                Kind::File => "must be a file",
                Kind::Directory => "must be a directory",
                Kind::Any => "must be a file or a directory",
            }
            .into(),
        ));
    }
    Ok(())
}

/// What ended a confined call.
enum Outcome {
    Exited {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    OutputLimit {
        stream: ConfinedStream,
    },
    Failed(io::Error),
    TimedOut,
    Cancelled,
}

/// Terminates the helper's process tree after a limit or cancellation, then
/// returns the error to report. The tree's drop still kills what a failed
/// termination leaves behind.
async fn killed(tree: &mut ProcessTree, error: ConfinedError) -> ConfinedError {
    if let Err(failure) = tree.terminate().await {
        tracing::warn!(%failure, "confined helper termination failed");
    }
    error
}

/// Reads both streams while waiting for the root, and stops at the first
/// limit. A stream that closed early does not end the call: the root may still
/// be running.
async fn collect(
    tree: &mut ProcessTree,
    stdout: ChildStdout,
    stderr: ChildStderr,
    confinement: &Confinement,
) -> Outcome {
    let mut stdout_pipe = Box::pin(read_capped(stdout, confinement.stdout_limit));
    let mut stderr_pipe = Box::pin(read_capped(stderr, confinement.stderr_limit));
    let mut status = None;
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;
    loop {
        tokio::select! {
            exited = tree.wait(), if status.is_none() => match exited {
                Ok(exited) => status = Some(exited),
                Err(error) => return Outcome::Failed(error),
            },
            collected = &mut stdout_pipe, if stdout_bytes.is_none() => match collected {
                Ok(Collected::Bytes(bytes)) => stdout_bytes = Some(bytes),
                Ok(Collected::OverLimit) => {
                    return Outcome::OutputLimit { stream: ConfinedStream::Stdout };
                }
                Err(error) => return Outcome::Failed(error),
            },
            collected = &mut stderr_pipe, if stderr_bytes.is_none() => match collected {
                Ok(Collected::Bytes(bytes)) => stderr_bytes = Some(bytes),
                Ok(Collected::OverLimit) => {
                    return Outcome::OutputLimit { stream: ConfinedStream::Stderr };
                }
                Err(error) => return Outcome::Failed(error),
            },
        }
        if status.is_some()
            && stdout_bytes.is_some()
            && stderr_bytes.is_some()
            && let (Some(status), Some(stdout), Some(stderr)) =
                (status, stdout_bytes.take(), stderr_bytes.take())
        {
            return Outcome::Exited {
                status,
                stdout,
                stderr,
            };
        }
    }
}

enum Collected {
    Bytes(Vec<u8>),
    OverLimit,
}

/// Reads until EOF or one byte past `limit`, so a stream that reaches the
/// limit is reported without buffering the rest.
async fn read_capped(pipe: impl AsyncRead + Unpin, limit: usize) -> io::Result<Collected> {
    let mut bytes = Vec::new();
    pipe.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    Ok(if bytes.len() > limit {
        Collected::OverLimit
    } else {
        Collected::Bytes(bytes)
    })
}
