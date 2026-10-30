//! Session-scoped paths the user allowed for this Session, and the
//! `session.grants.*` methods that manage them.
use crate::storage::SessionStore;
use dim_agent_capabilities::{
    methods::{Interruption, MethodDecl, RpcError, SessionMethods, params, reply},
    session_grants::{ReadSessionGrants, SessionGrantsHost},
};
use dim_agent_runtime::{AgentBuilder, Names, SessionIdentity};
use dim_agent_types::grant::{GrantKind, GrantMode, SessionGrant};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub fn install(builder: &mut AgentBuilder) {
    builder.require_feature(
        dim_agent_runtime::Feature::new("SessionGrants").session("SessionGrants", |names| {
            Ok(Box::new(SessionGrantsPlugin::new(names)?))
        }),
    );
}

struct StoreGrants {
    store: Arc<SessionStore>,
    session_id: String,
}
impl ReadSessionGrants for StoreGrants {
    fn grants(
        &self,
        _: CancellationToken,
    ) -> BoxFuture<'_, Result<Vec<SessionGrant>, dim_agent_capabilities::host::HostError>> {
        Box::pin(async move {
            self.store
                .grants(&self.session_id)
                .await
                .map_err(|error| dim_agent_capabilities::host::HostError::failed(error.to_string()))
        })
    }
}

struct SessionGrantsPlugin {
    host: Key<SessionGrantsHost>,
    store: Key<SessionStore>,
    identity: Key<SessionIdentity>,
    methods: Key<SessionMethods>,
}
impl SessionGrantsPlugin {
    fn new(names: &Names<'_>) -> Result<Self, Error> {
        Ok(Self {
            host: names.key()?,
            store: names.key()?,
            identity: names.key()?,
            methods: names.key()?,
        })
    }
}
impl DynPlugin for SessionGrantsPlugin {
    fn name(&self) -> &str {
        "SessionGrants"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.store.erase(),
            self.identity.erase(),
            self.methods.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let store = ctx.require_at(&self.store)?;
            let identity = ctx.require_at(&self.identity)?;
            let methods = ctx.require_at(&self.methods)?;
            let session_id = identity.session_id.clone();
            ctx.provide_at(
                self.host.clone(),
                Arc::new(SessionGrantsHost(Arc::new(StoreGrants {
                    store: store.clone(),
                    session_id: session_id.clone(),
                }))),
            )?;
            let owner = store.clone();
            methods.register(
                ctx,
                "session.grants",
                [
                    "session.grants.add",
                    "session.grants.remove",
                    "session.grants.list",
                    "session.grants.lookup",
                ]
                .into_iter()
                .map(|name| {
                    MethodDecl::new(
                        name,
                        name != "session.grants.list" && name != "session.grants.lookup",
                        Interruption::Report,
                    )
                })
                .collect(),
                Arc::new(move |method, value, _, _| {
                    let store = owner.clone();
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        match method.as_str() {
                            "session.grants.add" => {
                                let p: AddGrant = params(value)?;
                                store
                                    .add_grant(&session_id, p.path.into(), p.kind, p.mode)
                                    .await
                                    .map_err(failed)?;
                                reply(serde_json::json!({}))
                            }
                            "session.grants.remove" => {
                                let p: RemoveGrant = params(value)?;
                                store
                                    .remove_grant(&session_id, &p.path)
                                    .await
                                    .map_err(failed)?;
                                reply(serde_json::json!({}))
                            }
                            "session.grants.list" => {
                                let grants = store.grants(&session_id).await.map_err(failed)?;
                                reply(GrantsView { grants })
                            }
                            "session.grants.lookup" => {
                                let p: LookupGrant = params(value)?;
                                let path = std::path::PathBuf::from(&p.path);
                                let grants = store.grants(&session_id).await.map_err(failed)?;
                                let grant = grants.into_iter().find(|grant| match grant.kind {
                                    GrantKind::File => grant.path == path,
                                    GrantKind::Dir => path.starts_with(&grant.path),
                                });
                                reply(LookupView { grant })
                            }
                            _ => unreachable!("registered session grants method"),
                        }
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

fn failed(error: dim_agent_runtime::AgentError) -> RpcError {
    RpcError::app(-32000, "session_grants_failed", error.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddGrant {
    path: String,
    kind: GrantKind,
    mode: GrantMode,
}
#[derive(Deserialize)]
struct RemoveGrant {
    path: String,
}
#[derive(Deserialize)]
struct LookupGrant {
    path: String,
}
#[derive(Serialize)]
struct GrantsView {
    grants: Vec<SessionGrant>,
}
#[derive(Serialize)]
struct LookupView {
    grant: Option<SessionGrant>,
}
