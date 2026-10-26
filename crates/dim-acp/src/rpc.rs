//! One owned native connection multiplexes calls and reliable notifications.

use agent_client_protocol::Error;
use dim_client::{Client, Message};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(crate) enum Native {
    Notice { method: String, params: Value },
    Closed,
}

struct Call {
    method: &'static str,
    params: Value,
    reply: oneshot::Sender<Result<Value, Error>>,
}

#[derive(Clone)]
pub(crate) struct Rpc {
    calls: mpsc::Sender<Call>,
    features: Arc<Vec<String>>,
}

pub(crate) fn unavailable() -> Error {
    Error::new(
        -32001,
        "App Server connection is unavailable; a submitted operation may have been accepted",
    )
    .data(json!({"code":"daemon_unavailable"}))
}

pub(crate) fn boundary(error: dim_client::Error) -> Error {
    if let Some(error) = error.downcast_ref::<dim_client::RpcError>() {
        let app = error
            .data
            .as_ref()
            .and_then(|data| data.get("appError"))
            .and_then(Value::as_str);
        let code = match error.code {
            -32601 | -32602 => error.code,
            _ => -32001,
        };
        Error::new(code, error.message.clone())
            .data(json!({"code":"native_error","nativeAppError":app}))
    } else {
        unavailable()
    }
}

impl Rpc {
    pub(crate) fn start(
        client: Client,
        tasks: &TaskTracker,
        stop: CancellationToken,
    ) -> (Self, mpsc::Receiver<Native>) {
        let features = Arc::new(client.features().to_vec());
        let (calls, incoming) = mpsc::channel(128);
        let (notices, events) = mpsc::channel(256);
        tasks.spawn(drive(client, incoming, notices, stop));
        (Self { calls, features }, events)
    }

    pub(crate) fn supports(&self, method: &str) -> bool {
        // These v3 baseline calls are not repeated in hello.features.
        matches!(
            method,
            "session.attach"
                | "session.detach"
                | "session.submit"
                | "session.stop"
                | "session.list"
                | "session.close"
        ) || self.features.iter().any(|feature| feature == method)
    }

    pub(crate) async fn call(&self, method: &'static str, params: Value) -> Result<Value, Error> {
        let (reply, result) = oneshot::channel();
        self.calls
            .send(Call {
                method,
                params,
                reply,
            })
            .await
            .map_err(|_| unavailable())?;
        result.await.map_err(|_| unavailable())?
    }

    pub(crate) async fn typed<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<T, Error> {
        let result = self.call(method, params).await?;
        serde_json::from_value(result).map_err(|_| {
            Error::internal_error().data(json!({"code":"invalid_native_response","method":method}))
        })
    }
}

async fn drive(
    mut client: Client,
    mut calls: mpsc::Receiver<Call>,
    notices: mpsc::Sender<Native>,
    stop: CancellationToken,
) {
    let mut pending: HashMap<u64, oneshot::Sender<Result<Value, Error>>> = HashMap::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            call = calls.recv(), if pending.len() < 128 => {
                let Some(call) = call else { break };
                match client.send(call.method, call.params).await {
                    Ok(id) => { pending.insert(id, call.reply); }
                    Err(error) => { let _ = call.reply.send(Err(boundary(error))); break; }
                }
            }
            message = client.next() => match message {
                Ok(Message::Response { id, result }) => {
                    if let Some(reply) = pending.remove(&id) {
                        let _ = reply.send(result.map_err(|error| boundary(Box::new(error))));
                    }
                }
                Ok(Message::Notice { method, params }) => {
                    if notices.try_send(Native::Notice { method, params }).is_err() { break; }
                }
                Err(_) => break,
            }
        }
    }
    for reply in pending.into_values() {
        let _ = reply.send(Err(unavailable()));
    }
    let _ = notices.try_send(Native::Closed);
}
