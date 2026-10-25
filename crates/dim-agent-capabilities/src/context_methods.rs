//! `sessionMethods["session.context"]`: manual compaction and the usage
//! query, exposed from their owners (ContextCompaction §4.1).
use crate::{
    agent_loop::AgentLoop,
    context_preparation::{ContextOpError, ContextPreparation},
    methods::{Interruption, MethodDecl, RpcError, SessionMethods, params, reply},
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::Branch;
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypedPlugin};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const NAMESPACE: &str = "session.context";

pub struct ContextMethodsPlugin;
impl TypedPlugin for ContextMethodsPlugin {
    type Deps = (Keyed<TreeCommands>, Keyed<SessionMethods>);
    fn name(&self) -> &str {
        "ContextMethods"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(tree), Keyed(methods)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            methods.register(
                ctx,
                NAMESPACE,
                vec![
                    MethodDecl::new("session.context.compact", true, Interruption::Report),
                    MethodDecl::new("session.context.usage", false, Interruption::Report),
                ],
                Arc::new(move |method, value, cancel, _| {
                    let tree = tree.clone();
                    Box::pin(async move {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Params {
                            branch_id: String,
                        }
                        let Params { branch_id } = params(value)?;
                        let branch = tree.get(&branch_id)?;
                        match method.as_str() {
                            "session.context.compact" => compact(&branch, &cancel).await,
                            _ => usage(&branch, cancel).await,
                        }
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
/// Writes `context/compact`, or joins the outstanding command, and waits for
/// the result that carries that command.
async fn compact(branch: &Branch, cancel: &CancellationToken) -> Result<Value, RpcError> {
    let command = branch.service::<AgentLoop>()?.compact()?;
    let publication = branch.publication();
    let mut watch = publication.watch();
    let mut after = command;
    loop {
        let head = watch.borrow_and_update().0;
        if head > after {
            let result = branch
                .local(after + 1..=head)
                .await?
                .into_iter()
                .find(|event| {
                    matches!(&event.kind, EventKind::ContextCompaction(data)
                        if data.command == Some(command))
                });
            if let Some(event) = result {
                let EventKind::ContextCompaction(data) = &event.kind else {
                    unreachable!("matched a compaction")
                };
                return match &data.outcome {
                    ContextCompactionResult::Completed { .. } => {
                        reply(json!({"status": "completed", "sequence": event.sequence}))
                    }
                    ContextCompactionResult::Skipped { reason, .. } => {
                        reply(json!({"status": "skipped", "reason": reason}))
                    }
                    ContextCompactionResult::Failed { error, .. } => {
                        Err(RpcError::app(-32000, "compaction_failed", error.clone()))
                    }
                    ContextCompactionResult::Cancelled { reason } => Err(RpcError::app(
                        -32000,
                        "compaction_cancelled",
                        reason.clone(),
                    )),
                };
            }
            after = head;
        }
        tokio::select! {
            changed = watch.changed() => {
                if changed.is_err() {
                    return Err(RpcError::closed());
                }
            }
            // The unload's interrupted reply has already been handed over.
            _ = cancel.cancelled() => return Err(RpcError::interrupted()),
        }
    }
}

async fn usage(branch: &Branch, cancel: CancellationToken) -> Result<Value, RpcError> {
    match branch.service::<ContextPreparation>()?.usage(cancel).await {
        Ok(usage) => reply(usage),
        Err(ContextOpError::NotConfigured) => Err(RpcError::app(
            -32000,
            "context_not_configured",
            ContextOpError::NotConfigured.to_string(),
        )),
        Err(ContextOpError::Model(error)) => Err(RpcError::app(
            -32000,
            "context_model_failed",
            error.to_string(),
        )),
        Err(ContextOpError::Read(error)) => Err(error.into()),
    }
}
