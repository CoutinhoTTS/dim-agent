# Skills

Owner: [skills/session.rs](../../../../crates/dim-agent-capabilities/src/skills/session.rs).
Selects content from the catalog, owns Session loader content instances and
aggregates their contributions into the skill tool and instructions/list prompt sources.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:SkillsCatalog` | Query user/project skills; its `skills.catalog_changed` events drive replacement. |
| `S:SessionConfig` | Select the Session working directory. |
| `S:SessionLoader` | Lock-free creation of Session-owned content instances. |
| `S:SkillContributions` | Private ready content services, registered and withdrawn by content leaves. |
| `S:SessionLifecycle` | Start catalog-driven replacement only after the Session is published. |
| `S:PromptRegistry` | Register the list and instructions sources, until this part unloads. |
| `S:ToolRegistry` | Register the `skill` tool, until this part unloads. |

## Content instances

`SkillContentInstances` is an instanced Session extension template. Each enabled,
active catalog definition creates a `SkillContent` leaf with an immutable private
definition and a unique scoped contribution key passed through
`create_instance().with(...)`. Definitions and bodies are not loader row options.
The leaf provides its content service and registers the same service in the
Session's private contribution collection; disposal withdraws it. `SkillContributions`
provides that collection before either consumer applies, avoiding an apply-time
dependency cycle and undeclared dynamic service reads. All dependencies come from
`Deps`; factories capture only configuration and Session identity.

The catalog still owns source precedence, shadowing and enablement. Skills owns
selection and reconciliation, not a second lifecycle tree. Unchanged definitions
retain their instances. Body-only refresh replaces only changed content and changes
the view digest, without repeating unchanged list text. Views are built from ready
content services, never directly from enabled catalog definitions. A view is
rebuilt at each catalog change; a leaf withdrawn on its own does not change the
current view until then.

Apply listens to `catalog_changes()` with `ctx.events().on` before its first
query and creates the initial instances as part of Session assembly. The
listener only wakes the Session's tracked replacement task: rutis awaits one
channel's listeners in turn, and every Session listens on the same channel.
Replacement after a catalog change waits for `SessionLifecycle::activated()`,
so assembly readiness never observes an instance that the replacement task
retires, and a Session that fails assembly never replaces content. Changes
announced meanwhile are adopted by the first update after publication. Replacement ends when
`SessionLifecycle::closing()` returns: closing wins over a simultaneous change,
and an update in progress is cancelled but awaited, so a closing Session creates
no content and abandons no partial instance. Cancellation is not logged as a
content failure.

## Events and lifetime

No direct Timeline events. The tracked replacement task replaces the Session
view; SkillsBranch requests list adoption. The tool returns full skill content as a
plain tool result, not a second history-rendering route. Cleanup joins the
replacement task and shuts down every content fiber; tool handlers reject work after
generation cancellation. Initial creation also runs in a tracked task, so dropping
apply cannot abandon it. Creation is awaited to settlement even if cancelled,
then an unusable instance is shut down. Failed updates drain partial/current
instances and publish an unavailable view. `FiberView::shutdown` and its retained
error preserve cleanup failures; no lock-taking loader mutation runs in apply or
dispose. Content lifecycle failures use the existing public `SkillsError::Closed`
state, with details logged; disposer failures propagate to Session shutdown.
Catalog errors retain their existing public variants. Closing one Session cannot
dispose another Session's contents. Reload queries
a fresh view. Unavailable settings isolate skills without disabling unrelated
execution. Listed metadata and loaded body can change between view updates;
loading uses the current view rather than an old model-visible list.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs):
`load_returns_the_body_as_a_plain_tool_result`,
`enablement_changes_the_list_and_loading`,
`skill_loads_the_latest_view_after_queue_and_steer_interleave`,
`content_instances_are_session_owned_and_only_changed_content_is_replaced`,
`catalog_changes_during_assembly_wait_for_session_publication`,
`a_closing_session_creates_no_content_instances`,
`unavailable_settings_isolate_skills_and_refresh_recovers_existing_sessions`.

<!-- generated:plugin Skills — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:SkillsCatalog`、`S:PromptRegistry`、`S:SessionConfig`、`S:SessionLifecycle`、`S:SessionLoader`、`S:SkillContributions`、`S:ToolRegistry` |
| 提供 | `S:SessionSkills` |
| 被注入于 | `SkillsBranch` |
<!-- /generated -->
