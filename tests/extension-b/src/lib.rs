use dim_agent_runtime::{Dispatcher, Names, StateCell, When};
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, Plugin, TypedPlugin};
use std::sync::Arc;
use test_extension_a::Counter;
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub inputs: usize,
}
/// Declares the Counter only when `declare` is set; otherwise it reads the
/// Counter undeclared, which the runtime must reject.
pub struct Consumer {
    counter: DepKey<Counter>,
    state: DepKey<StateCell<Snapshot>>,
}
impl Consumer {
    pub fn part(names: &Names<'_>, declare: bool) -> Result<Box<dyn Plugin>, CordisError> {
        let consumer = Self {
            counter: names.key()?,
            state: names.state_cell()?,
        };
        let keys = (names.key()?, names.when::<Keyed<Counter>>(declare)?);
        Ok(names.part_with(consumer, keys))
    }
}
impl TypedPlugin for Consumer {
    type Deps = (Keyed<Dispatcher>, When<Keyed<Counter>>);
    fn name(&self) -> &str {
        "ExternalConsumer"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(dispatcher), When(counter)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let counter = match counter {
                Some(Keyed(counter)) => counter,
                None => ctx.require_as::<Counter>(self.counter.key().clone())?,
            };
            let state = StateCell::new(Snapshot::default());
            ctx.provide_as(self.state.key().clone(), Arc::new(state.clone()))?;
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
