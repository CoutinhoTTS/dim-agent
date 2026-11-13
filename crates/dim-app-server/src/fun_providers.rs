//! `fun.providers.*` for the Fun-owned provider snapshot.
//!
//! The catalog and model hosts live in `dim-provider-fun`; this part only
//! attaches the management methods Fun calls to them, because the method
//! table is an App Server service.

use std::sync::Arc;

use dim_agent_capabilities::methods::{Interruption, MethodCall, MethodDecl, RpcError, reply};
use dim_provider_fun::{FunCatalog, FunCatalogState};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};

use crate::methods::ManagementMethods;

pub struct FunProviderMethodsPlugin {
    methods: Key<ManagementMethods>,
    state: Arc<FunCatalogState>,
}

impl FunProviderMethodsPlugin {
    pub fn new(
        names: &dim_agent_runtime::Names<'_>,
        state: Arc<FunCatalogState>,
    ) -> rutis::Result<Self> {
        Ok(Self {
            methods: names.key()?,
            state,
        })
    }
}

impl DynPlugin for FunProviderMethodsPlugin {
    fn name(&self) -> &str {
        "FunProviderMethods"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.methods.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let methods = ctx.require_at(&self.methods)?;
            let state = self.state.clone();
            methods.register(
                ctx,
                "fun.providers",
                vec![
                    MethodDecl::new("fun.providers.sync", true, Interruption::Drain),
                    MethodDecl::new("fun.providers.status", false, Interruption::Drain),
                ],
                Arc::new(move |call: MethodCall, _stopping, _progress| {
                    let state = state.clone();
                    Box::pin(async move {
                        match call.method.as_str() {
                            "fun.providers.sync" => {
                                let catalog =
                                    FunCatalog::from_value(call.params).map_err(|error| {
                                        RpcError::app(-32000, "provider_invalid_snapshot", error)
                                    })?;
                                let summary = state.replace(catalog).map_err(|error| {
                                    RpcError::app(-32000, "provider_invalid_snapshot", error)
                                })?;
                                reply(summary)
                            }
                            "fun.providers.status" => reply(state.summary()),
                            _ => Err(RpcError::unknown_method()),
                        }
                    })
                }),
            )?;

            // Existing clients ask `provider.management` for the model
            // picker. This is a read-only projection of the same Fun-owned
            // catalog: no writes, no account providers, no OAuth.
            let state = self.state.clone();
            methods.register(
                ctx,
                "provider",
                vec![MethodDecl::new(
                    "provider.management",
                    false,
                    Interruption::Drain,
                )],
                Arc::new(move |_call: MethodCall, _stopping, _progress| {
                    let state = state.clone();
                    Box::pin(async move { reply(state.current().management_view()) })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
