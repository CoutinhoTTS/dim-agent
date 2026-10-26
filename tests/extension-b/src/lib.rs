use dim_agent_runtime::{Dispatcher, Names, StateCell};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use std::sync::Arc;
use test_extension_a::Counter;
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub inputs: usize,
}
/// Declares the Counter only when `declare` is set; otherwise it reads the
/// Counter undeclared, which the runtime must reject.
pub struct Consumer {
    dispatcher: Key<Dispatcher>,
    declared: Option<Key<Counter>>,
    counter: Key<Counter>,
    state: Key<StateCell<Snapshot>>,
}
impl Consumer {
    pub fn part(names: &Names<'_>, declare: bool) -> rutis::Result<Box<dyn DynPlugin>> {
        Ok(Box::new(Self {
            dispatcher: names.key()?,
            declared: declare.then(|| names.key::<Counter>()).transpose()?,
            counter: names.key()?,
            state: names.state_cell()?,
        }))
    }
}
impl DynPlugin for Consumer {
    fn name(&self) -> &str {
        "ExternalConsumer"
    }
    fn injects(&self) -> Vec<TypeKey> {
        let mut keys = vec![self.dispatcher.erase()];
        keys.extend(self.declared.as_ref().map(Key::erase));
        keys
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let counter = match &self.declared {
                Some(counter) => ctx.require_at(counter)?,
                None => ctx.require_at(&self.counter)?,
            };
            let state = StateCell::new(Snapshot::default());
            ctx.provide_at(self.state.clone(), Arc::new(state.clone()))?;
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
