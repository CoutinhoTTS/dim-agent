use dim_agent_runtime::{BranchScope, BranchService, Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub struct Counter(AtomicUsize);
impl Counter {
    pub fn increment(&self) -> usize {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }
}
impl BranchService for Counter {}
pub struct Provider {
    scope: BranchScope,
}
impl Provider {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for Provider {
    type Deps = ();
    fn name(&self) -> &str {
        "ExternalProvider"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(self.scope.key::<Counter>(), Arc::new(Counter::default()))?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for Provider {
    fn keys(&self) -> KeysOf<Self> {}
}
