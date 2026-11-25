# ToolMethods

Owner: [tool_executor/read.rs](../../../../crates/dim-agent-capabilities/src/tool_executor/read.rs).
ToolExecution's Session part registers `session.tool.get`. The caller-visible contract is
[Tool origin read](../../../design/tool-origin-read.md).

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Read the addressed branch’s own facts, including ended branches. |
| `S:SessionMethods` | Register and withdraw the admitted read method. |

It writes no event, holds no StateCell and starts no task. Each admitted request owns its
query and cancellation; its returned call data is released after serialization.
