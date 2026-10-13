use dim_agent_runtime::{BranchScope, Capability, Dispatcher, InBranch, KeysOf, StateCell, When};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::Arc;
use test_extension_a::Counter;
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub inputs: usize,
}
/// Declares the Counter only when `declare` is set; otherwise it reads the
/// Counter undeclared, which the runtime must reject.
pub struct Consumer {
    scope: BranchScope,
    declare: bool,
}
impl Consumer {
    pub fn new(scope: &BranchScope, declare: bool) -> Self {
        Self {
            scope: *scope,
            declare,
        }
    }
}
impl TypedPlugin for Consumer {
    type Deps = (InBranch<Dispatcher>, When<InBranch<Counter>>);
    fn name(&self) -> &str {
        "ExternalConsumer"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(dispatcher), When(counter)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let counter = match counter {
                Some(InBranch(counter)) => counter,
                None => self.scope.service::<Counter>(ctx)?,
            };
            let state = StateCell::new(Snapshot::default());
            ctx.provide_as(
                self.scope.key::<StateCell<Snapshot>>(),
                Arc::new(state.clone()),
            )?;
            dispatcher.subscribe(ctx, "ExternalConsumer", &["input/message"], move |_| {
                let counter = counter.clone();
                let state = state.clone();
                Box::pin(async move {
                    state.set(Snapshot {
                        inputs: counter.increment(),
                    });
                    Ok(())
                })
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for Consumer {
    fn keys(&self) -> KeysOf<Self> {
        (self.scope, self.declare.then_some(self.scope))
    }
}
