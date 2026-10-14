# 003: Approval covers agent decisions only

Decision status: Proposed. Supersedes the project MCP execution approval in
[ADR 001 "Trust and cross-language calls"](001-harness-foundation.md#trust-and-cross-language-calls)
and the matching sentence of AGENTS.md rule 11. The rest of ADR 001 is unchanged.

## Context

ADR 001 requires a host-local grant before a project MCP entry may start a stdio process or
connect an HTTP/SSE transport. A grant binds the workspace, the server ID and a fingerprint of
the effective configuration; it is stored in `mcp-approvals.json` and managed through
`mcp.approve` and `mcp.revoke`.

The plugin designs then needed the same kind of gate for every other feature that runs
something from configuration: project Hook commands, project workflow definitions and plugin
packages. Each would add its own approve and revoke methods, approval state in listings,
fingerprints and stale-grant errors. This mixes two different things:

- Approval and Permission decide whether an **agent's decision** (a model tool call) may run.
- A command written in configuration runs because the **configuration says so**. No agent
  decided it. Its safety is the responsibility of whoever writes and enables that
  configuration.

## Decisions

### D1. Approval and Permission govern only agent decisions

Approval and Permission apply to actions an agent decides, which are model tool calls. This is
unchanged. Tool calls into an MCP server are still admitted by Approval and Permission like any
other tool.

### D2. No host-local approval for configuration

Commands and connections that run because of configuration are not agent decisions and need no
approval: an MCP stdio server launched on connect, an MCP HTTP/SSE connection, Hook commands,
workflow definitions and plugin packages. They run when they are enabled. There are no grants,
no configuration fingerprints, no approval state and no approve or revoke methods.

### D3. Enablement is ordinary configuration; project entries start disabled

Enabling or disabling an entry is ordinary configuration, unrelated to approval.

- An entry from a project source (files in the workspace, for example `.dimcode/`) is disabled
  by default.
- Project files cannot enable their own entries. The enabled state of a project entry is stored
  in the user's local configuration, keyed by workspace and entry ID, and is changed only by the
  user through the owner's configuration operation.
- User configuration is unchanged: the user's own entries are enabled as the user writes them.

Opening a repository therefore never runs a command from that repository until the user
enables it.

### D4. Accepted consequence

After the user enables a project entry, later changes to its command or arguments in the project
files (for example after pulling new commits) take effect without asking again. ADR 001's
fingerprint would have required a new grant. This is accepted: enabling a project entry means
trusting that project's configuration for that entry. This was never a sandbox (ADR 001 already
stated that approval covers configuration, not executable bytes); OS isolation remains separate
work.

### D5. Unchanged

Host safety settings never enter the Timeline, and replay, restore or fork never restores or
widens authority. File-root authorization and permission presets confer nothing beyond what they
confer today.

## Consequences

- Remove `mcp.approve` and `mcp.revoke`; they return `unknown_method`. Remove `approval` and
  `approvalToken` from `mcp.catalog`. Stop reading `mcp-approvals.json`; an existing file is
  ignored, not migrated. Project entries that had a grant start disabled until the user enables
  them.
- Remove the CLI `mcp approve` command.
- The plugin designs (Hooks, Workflow, PluginManager and others) use D3 for project-sourced
  configuration and add no approval methods.
- AGENTS.md rule 11 is updated to this decision.
