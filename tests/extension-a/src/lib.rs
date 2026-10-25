use dim_agent_runtime::{BRANCH_GROUP, Names, names};
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, TypedPlugin};
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
/// Registers this crate's names: `counter` is a branch service.
pub fn register_names(catalog: &mut names::ServiceCatalog) {
    names::register::<Counter>(catalog, Some(BRANCH_GROUP));
}
pub struct Provider {
    counter: DepKey<Counter>,
}
impl Provider {
    pub fn new(names: &Names<'_>) -> Result<Self, CordisError> {
        Ok(Self {
            counter: names.key()?,
        })
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
            ctx.provide_as(self.counter.key().clone(), Arc::new(Counter::default()))?;
            Ok(Effect::Done)
        })
    }
}
