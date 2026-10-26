use dim_agent_runtime::{BRANCH_GROUP, Names, names};
use rutis::host::{CtxHost, Key};
use rutis::{Ctx, Plugin};
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
    counter: Key<Counter>,
}
impl Provider {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            counter: names.key()?,
        })
    }
}
impl Plugin for Provider {
    type Deps = ();
    fn name(&self) -> &str {
        "ExternalProvider"
    }
    async fn apply(&self, ctx: &Ctx, (): Self::Deps) -> rutis::Result {
        ctx.provide_at(self.counter.clone(), Arc::new(Counter::default()))?;
        Ok(())
    }
}
