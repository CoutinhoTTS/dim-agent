mod local_support;

use dim_agent_capabilities::{
    prompt_sources::BasePromptSettings, skills::SkillsInstructionsSettings,
};
use local_support::*;
use std::sync::Arc;

/// A host base-prompt override fixed at construction.
struct FixedBase(&'static str);
impl BasePromptSettings for FixedBase {
    fn base(
        &self,
        _: tokio_util::sync::CancellationToken,
    ) -> dim_agent_capabilities::host::HostFuture<'_, Option<Arc<str>>> {
        Box::pin(async move { Ok(Some(self.0.into())) })
    }
}

/// A host Skills-instructions override fixed at construction.
struct FixedInstructions(&'static str);
impl SkillsInstructionsSettings for FixedInstructions {
    fn instructions(
        &self,
        _: tokio_util::sync::CancellationToken,
    ) -> dim_agent_capabilities::host::HostFuture<'_, Option<Arc<str>>> {
        Box::pin(async move { Ok(Some(self.0.into())) })
    }
}

/// The host's user overrides replace the built-in base and Skills texts, and
/// the assembly's factory configuration still wins over them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_overrides_replace_the_policy_texts() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("root");
    let env = env(
        &dirs,
        Options {
            base_prompt: Some(Arc::new(FixedBase("CUSTOM-BASE"))),
            skills_instructions: Some(Arc::new(FixedInstructions("CUSTOM-SKILLS"))),
            ..Options::default()
        },
    )
    .await;
    let session = env.session(&cwd, "SESSION-S").await.unwrap();
    run(&session, "hello").await;
    let sent = requests(&session).await;
    let system = system_text(sent.last().unwrap().system.as_deref().unwrap());
    assert!(system.contains("CUSTOM-BASE"), "{system}");
    assert!(!system.contains("You are DimAgent."), "{system}");
    assert!(system.contains("CUSTOM-SKILLS"), "{system}");
    assert!(!system.contains("<skills_instructions>"), "{system}");
    env.agent.close().await.unwrap();
}

/// The factory configuration stays authoritative: a candidate assembly keeps
/// its text even when the host provides an override.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn factory_texts_outrank_host_overrides() {
    let dirs = Dirs::new();
    let cwd = dirs.dir("root");
    let env = env(
        &dirs,
        Options {
            instructions: Some("FACTORY-SKILLS".into()),
            skills_instructions: Some(Arc::new(FixedInstructions("HOST-SKILLS"))),
            ..Options::default()
        },
    )
    .await;
    let session = env.session(&cwd, "SESSION-S").await.unwrap();
    run(&session, "hello").await;
    let sent = requests(&session).await;
    let system = system_text(sent.last().unwrap().system.as_deref().unwrap());
    assert!(system.contains("FACTORY-SKILLS"), "{system}");
    assert!(!system.contains("HOST-SKILLS"), "{system}");
    env.agent.close().await.unwrap();
}
