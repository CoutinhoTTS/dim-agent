use dim_agent_runtime::wait_active;
use dim_app_server::mcp::{McpConnection, McpOwner, McpPublication, McpService, Result};
use rutis::Ctx;
use rutis_loader::{Builtins, Layer, Loader, LoaderPlugin, Patch};
use serde_json::json;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
#[allow(dead_code)] // Session flows use their actual Session loader instead.
pub struct Directory {
    service: std::sync::Arc<McpService>,
    owner: McpOwner,
}
#[allow(dead_code)]
impl Directory {
    pub fn new(service: std::sync::Arc<McpService>, root: &Ctx, loader: Loader) -> Self {
        Self {
            service,
            owner: McpOwner::new(root.clone(), loader),
        }
    }
    pub async fn server(
        &self,
        project: Option<usize>,
        id: String,
        cancel: &CancellationToken,
    ) -> Result<std::sync::Arc<McpConnection>> {
        self.service
            .server(self.owner.clone(), project, id, cancel)
            .await
    }
}
impl std::ops::Deref for Directory {
    type Target = std::sync::Arc<McpService>;
    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

pub struct Host {
    stop: CancellationToken,
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}
impl Host {
    pub async fn start(root: &Ctx) -> (Self, Loader) {
        let mut builtins = Builtins::new();
        let (publication, row) = McpPublication::register(&mut builtins);
        let template = McpPublication::standalone_template(&mut builtins);
        let plugin = LoaderPlugin::new(builtins, Default::default());
        let loader = plugin.handle();
        let view = root.plugin(plugin);
        let stop = CancellationToken::new();
        wait_active(root, &[view], Duration::from_secs(5), &stop)
            .await
            .unwrap();
        let assembly = Layer::new(
            "assembly",
            vec![Patch {
                insert: Some(vec![json!({"id":row,"name":row}), template]),
                ..Default::default()
            }],
        );
        let report = loader
            .reconcile(vec![assembly.clone()], None)
            .await
            .unwrap();
        assert!(report.failures.is_empty() && report.issues.is_empty());
        let task = tokio::spawn(publication.run(stop.clone()));
        (
            Self {
                stop,
                task: tokio::sync::Mutex::new(Some(task)),
            },
            loader,
        )
    }
    pub async fn close(&self) {
        self.stop.cancel();
        if let Some(task) = self.task.lock().await.take() {
            task.await.unwrap();
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
