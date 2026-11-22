#[cfg(windows)]
use super::WINDOWS_SANDBOX_UNAVAILABLE;
#[cfg(unix)]
use super::isolation;
use super::{
    EXEC_SHELL_UNAVAILABLE, HostPlatform, Os, ResolvedShell, SandboxBackend, ShellStatus,
    read_config, resolve_shell,
};
use serde::Serialize;
use std::path::PathBuf;

/// The process facts the `hostPlatform.info` operation answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub os: Os,
    pub arch: String,
    pub home: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub exec_root: PathBuf,
    pub platform_roots: Vec<PathBuf>,
    pub backend: SandboxBackend,
    pub exec_shell: ShellStatus,
}

#[rutis::service]
impl HostPlatform {
    /// Reads the configured exec shell for this invocation without reloading
    /// the plugin. Explicitly constructed platforms keep their supplied shell.
    pub async fn exec_shell(&self) -> ShellStatus {
        let mut resolved = self.shell.lock().await;
        if let Some((storage, root, relative)) = &self.shell_file {
            let config = read_config(storage, *root, relative).await;
            if config != resolved.config
                || matches!(
                    resolved.status,
                    ShellStatus::Unavailable {
                        code: EXEC_SHELL_UNAVAILABLE,
                        ..
                    }
                )
            {
                let status = resolve_shell(config.clone(), &self.env).await;
                *resolved = ResolvedShell { config, status };
            }
        }
        resolved.status.clone()
    }

    /// The sandbox backend a new process would use. Preparation reads the same
    /// backend, so this report and the next process start cannot disagree.
    #[cfg(windows)]
    pub fn sandbox_backend(&self) -> SandboxBackend {
        SandboxBackend::Unavailable {
            reason: WINDOWS_SANDBOX_UNAVAILABLE,
        }
    }

    /// The sandbox backend a new process would use. macOS always has
    /// `sandbox-exec`.
    #[cfg(target_os = "macos")]
    pub fn sandbox_backend(&self) -> SandboxBackend {
        SandboxBackend::Available
    }

    /// The sandbox backend a new process would use: the same bubblewrap
    /// preparation resolves from the captured `PATH`.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn sandbox_backend(&self) -> SandboxBackend {
        match isolation::bubblewrap(&self.env) {
            Some(_) => SandboxBackend::Available,
            None => SandboxBackend::Unavailable {
                reason: isolation::BWRAP_UNAVAILABLE,
            },
        }
    }

    /// The directory the persistent shell owns: `<home>/.dim/exec`.
    pub fn exec_root(&self) -> PathBuf {
        let root = self.home.join(".dim").join("exec");
        std::fs::canonicalize(&root).unwrap_or(root)
    }

    /// The process facts a consumer needs, in one call.
    pub async fn info(&self) -> HostInfo {
        HostInfo {
            os: self.os,
            arch: self.arch.clone(),
            home: self.home.clone(),
            timezone: self.timezone.clone(),
            exec_root: self.exec_root(),
            platform_roots: self.platform_roots.clone(),
            backend: self.sandbox_backend(),
            exec_shell: self.exec_shell().await,
        }
    }
}
