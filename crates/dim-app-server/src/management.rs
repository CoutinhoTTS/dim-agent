//! Management-level view of the stable RPC method port.
use crate::{
    methods::{Interruption, MethodDecl, MethodLevel, Methods, params},
    protocol::Empty,
};
use dim_agent_runtime::{Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;

pub struct Management(pub Arc<Methods>);
impl Management {
    pub fn methods(&self) -> serde_json::Value {
        self.0.catalog(MethodLevel::Management)
    }
}

pub struct ManagementPlugin;
impl TypedPlugin for ManagementPlugin {
    type Deps = (Arc<Methods>,);
    fn name(&self) -> &str {
        "Management"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (methods,): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(Management(methods.clone()));
            let view = service.clone();
            let registration = methods
                .register(
                    "management",
                    vec![MethodDecl::management(
                        "management.methods",
                        false,
                        Interruption::Drain,
                    )],
                    vec![],
                    Arc::new(move |_, value, _| {
                        let view = view.clone();
                        Box::pin(async move {
                            let _: Empty = params(value)?;
                            Ok(view.methods())
                        })
                    }),
                )
                .map_err(|error| CordisError::PluginFailed(error.message.into()))?;
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        registration.unregister_and_drain().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(rutis::TypeKey::of::<Management>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ManagementPlugin {
    fn keys(&self) -> KeysOf<Self> {
        ((),)
    }
}
