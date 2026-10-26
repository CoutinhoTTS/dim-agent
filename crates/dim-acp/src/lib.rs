//! Client-owned ACP transport and protocol projection over the public App Server.

pub mod auth;
mod config;
mod content;
mod gateway;
mod interaction;
mod lody;
mod projection;
mod rpc;

use agent_client_protocol::{Agent, Error, UntypedMessage};
use dim_client::Client;
use futures_util::{SinkExt, StreamExt};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{io, sync::Mutex};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::oneshot,
};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LinesCodec},
    sync::CancellationToken,
    task::TaskTracker,
};

const FRAME_LIMIT: usize = 16 * 1024 * 1024;

/// Exclusive transport halves supplied by the native host.
pub struct Io {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
}

struct Inputs {
    client: Client,
    io: Io,
    agent_id: Option<String>,
    done: oneshot::Sender<dim_client::Result<()>>,
    login: Option<auth::Login>,
}

/// One ACP connection, its independent native connection and owned work.
pub struct AcpGateway(Mutex<Option<Inputs>>);

impl AcpGateway {
    pub fn new(
        client: Client,
        io: Io,
        agent_id: Option<String>,
    ) -> (Self, oneshot::Receiver<dim_client::Result<()>>) {
        let (done, finished) = oneshot::channel();
        (
            Self(Mutex::new(Some(Inputs {
                client,
                io,
                agent_id,
                done,
                login: None,
            }))),
            finished,
        )
    }

    pub fn login(self, login: auth::Login) -> Self {
        self.0.lock().unwrap().as_mut().unwrap().login = Some(login);
        self
    }
}

impl TypedPlugin for AcpGateway {
    type Deps = ();

    fn name(&self) -> &str {
        "AcpGateway"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let inputs = self.0.lock().unwrap().take().ok_or_else(|| {
                CordisError::PluginFailed("AcpGateway I/O was already taken".into())
            })?;
            let stop = CancellationToken::new();
            let tasks = TaskTracker::new();
            let closing = stop.clone();
            let draining = tasks.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        closing.cancel();
                        draining.close();
                        draining.wait().await;
                        Ok(())
                    })
                }))
            })?;
            let owned_tasks = tasks.clone();
            tasks.spawn(async move {
                let Inputs {
                    client,
                    io,
                    agent_id,
                    done,
                    login,
                } = inputs;
                let result = serve(client, io, agent_id, login, stop, owned_tasks).await;
                let _ = done.send(result);
            });
            Ok(Effect::Done)
        })
    }
}

async fn serve(
    client: Client,
    io: Io,
    agent_id: Option<String>,
    login: Option<auth::Login>,
    stop: CancellationToken,
    tasks: TaskTracker,
) -> dim_client::Result<()> {
    let incoming_frames = FramedRead::new(io.reader, LinesCodec::new_with_max_length(FRAME_LIMIT))
        .map(|line| line.map_err(io::Error::other));
    let outgoing = SinkExt::<String>::sink_map_err(
        FramedWrite::new(io.writer, LinesCodec::new()),
        io::Error::other,
    )
    .with(async |line: String| {
        if line.len() > FRAME_LIMIT {
            return Err(io::Error::other("ACP frame exceeds 16 MiB"));
        }
        Ok(line)
    });
    let (rpc, native) = rpc::Rpc::start(client, &tasks, stop.clone());
    let (incoming, messages) = tokio::sync::mpsc::channel(128);
    let notifications = incoming.clone();
    let foreground = incoming.clone();
    let foreground_stop = stop.clone();
    let connection = Agent
        .builder()
        .name("Dim ACP Gateway")
        .on_receive_request(
            async move |request: UntypedMessage, responder, _cx| {
                if let Err(error) =
                    incoming.try_send(gateway::Incoming::Request(request, responder))
                    && let gateway::Incoming::Request(_, responder) = error.into_inner()
                {
                    responder
                        .respond_with_error(Error::new(-32001, "ACP request queue is full"))?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: UntypedMessage, _cx| {
                notifications
                    .try_send(gateway::Incoming::Notification(notification))
                    .map_err(|_| Error::new(-32001, "ACP notification queue is full"))?;
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            agent_client_protocol::Lines::new(outgoing, incoming_frames),
            async move |cx| {
                gateway::Gateway::new(rpc, cx, foreground, tasks, foreground_stop, agent_id, login)
                    .run(messages, native)
                    .await
            },
        );
    tokio::select! {
        result = connection => result.map_err(Into::into),
        () = stop.cancelled() => Ok(()),
    }
}
