//! Server-owned stdio only; client-local processes require a separate bridge.
use super::{McpServerError, Result};
use futures_util::{SinkExt, StreamExt};
use rmcp::{
    RoleClient,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::JsonRpcMessageCodec},
};
use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
use tokio_util::{
    codec::{FramedRead, FramedWrite},
    sync::CancellationToken,
};

#[derive(Clone)]
pub(super) struct Config {
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<String>,
}
impl Config {
    pub fn new(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Option<String>,
    ) -> Result<Self> {
        if command.trim().is_empty()
            || command.contains('\0')
            || args.iter().any(|s| s.contains('\0'))
            || env
                .iter()
                .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
            || cwd
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.contains('\0'))
        {
            return Err(McpServerError::Configuration(
                "invalid process configuration",
            ));
        }
        Ok(Self {
            command: command.into(),
            args: args.to_vec(),
            env: env.clone(),
            cwd: cwd.clone(),
        })
    }

    pub fn spawn(&self) -> Result<StdioTransport> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|_| McpServerError::RequestFailed)?;
        let read = FramedRead::new(
            child.stdout.take().expect("piped stdout"),
            JsonRpcMessageCodec::new_with_max_length(4 * 1024 * 1024),
        );
        let write = FramedWrite::new(
            child.stdin.take().expect("piped stdin"),
            JsonRpcMessageCodec::new(),
        );
        Ok(StdioTransport {
            child,
            read,
            write: Arc::new(Mutex::new(Some(write))),
            stopping: CancellationToken::new(),
        })
    }
}

type Writer = FramedWrite<ChildStdin, JsonRpcMessageCodec<TxJsonRpcMessage<RoleClient>>>;
pub(super) struct StdioTransport {
    child: Child,
    read: FramedRead<ChildStdout, JsonRpcMessageCodec<RxJsonRpcMessage<RoleClient>>>,
    write: Arc<Mutex<Option<Writer>>>,
    stopping: CancellationToken,
}
impl Transport<RoleClient> for StdioTransport {
    type Error = McpServerError;
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let write = self.write.clone();
        let stopping = self.stopping.clone();
        async move {
            tokio::select! {
                biased;
                _ = stopping.cancelled() => Err(McpServerError::Closed),
                result = async {
                    let mut write = write.lock().await;
                    write.as_mut().ok_or(McpServerError::Closed)?.send(item).await.map_err(|_| McpServerError::RequestFailed)
                } => result,
            }
        }
    }
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.read.next().await?.ok()
    }
    async fn close(&mut self) -> Result<()> {
        self.stopping.cancel();
        self.write.lock().await.take();
        match tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await {
            Ok(result) => {
                result.map_err(|_| McpServerError::Cleanup)?;
            }
            Err(_) => self
                .child
                .kill()
                .await
                .map_err(|_| McpServerError::Cleanup)?,
        }
        Ok(())
    }
}
