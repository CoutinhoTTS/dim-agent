//! The Session part: the guide entry for the markers EventHub writes
//! (design A.6).
use crate::text::ENTRY;
use dim_agent_capabilities::prompt_guide::{GuideEntries, GuideEntry, GuideScope};
use dim_agent_runtime::Names;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};

pub struct EventHubSession {
    guide: Key<GuideEntries>,
}

impl EventHubSession {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            guide: names.key()?,
        })
    }
}

impl DynPlugin for EventHubSession {
    fn name(&self) -> &str {
        "EventHubSession"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.guide.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            ctx.require_at(&self.guide)?.register(
                ctx,
                GuideScope::Session,
                GuideEntry {
                    markers: vec!["dim-event".into(), "dim-event-guide".into()],
                    order: 130,
                    text: ENTRY.into(),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
