# Code Mode: bounded QuickJS tool orchestration

Status: Draft. Phase: P3. Delivery status is tracked in [#1276](https://github.com/arcships/dim/issues/1276), not here.
Language: native Rust (`rquickjs`). The plugin is `crates/dim-codemode/`; it is not a TypeScript
package and runs no Node or TypeScript plugin runtime. Scope: process, Session and branch
([ADR 002](../../adr/002-plugin-scope.md)).

The first delivery is a native Rust crate that a host adds with `dim_codemode::install`. The App
Server product profile installs it with optional parts; a host that does not want the tool omits
the call. The model program runs in a fresh QuickJS context inside the branch part. Durable
child-invocation facts, the reload parent gate, a separate native host process and a user-editable
settings source are deferred and remain tracked in [#1276](https://github.com/arcships/dim/issues/1276).

Issue: [#1276](https://github.com/arcships/dim/issues/1276).
Rules: [design rules](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md), [plugin contract](../plugin-contract.md).

Code Mode lets a model submit one bounded program that invokes the request's adopted Session tools,
collects the results and returns one ordinary tool result. It is a tool orchestration capability,
not a second agent loop, a persistent REPL or a Workflow engine.

## 1. Scope and ownership

### 1.1 Parts

| Part | Scope | Owns |
| --- | --- | --- |
| `CodeMode` | Process | The structural `CodeModeConfig` bounds, the process `CodeModeRuns` map from branch id to its runner, and the registrations of the Session and branch parts. |
| `CodeModeSession` | Session | The `codemode` tool declaration and handler. The handler resolves the caller's branch in `CodeModeRuns` by the tool context's `branch_id`; it does not read Session state to find a branch. |
| `CodeModeBranch` | branch | One fresh QuickJS runtime per call, the `tools` bridge, child-call cancellation and the confirmed child effects copied onto the outer result. |
| ToolExecutor nested invocation | branch, mechanism owner | The pinned `ToolBinding` lookup, Approval admission and the invoked tool's handler. This is not Code Mode business state. |
| QuickJS runtime | branch | The execution boundary for model code. It receives only the serializable `tools` bridge. |

`CodeMode` is a process plugin, not a `Feature`: it provides `CodeModeRuns` and registers the two
parts. `CodeModeSession` and `CodeModeBranch` are plain parts. `CodeModeBranch` is the only Code
Mode Capability: it owns the one-shot execution state on the branch, while the outer
`step/tool-call` and `step/tool-result` remain ToolExecutor's facts.

The first delivery's bounds are structural `CodeModeConfig` values. A user `codemode` Settings
source is deferred; no part invents defaults from another source.

### 1.2 Goals

- Allow one model tool call to run a bounded asynchronous program.
- Let the program invoke tools from the request's fixed `ToolBinding` snapshot.
- Keep every child invocation under the invoked tool's Permission and Approval policy.
- Return one bounded outer `ToolResult`; child results do not become model messages.
- Copy confirmed child effects onto the outer result for the existing compaction digest.
- Never replay a child side effect after cancellation, timeout or reload.

### 1.3 Non-goals

| Not done here | Why |
| --- | --- |
| Persistent REPL, cells or cross-turn JavaScript state | That is durable execution state and belongs in a separate design or Workflow. |
| Durable `step/tool-invoke` child facts and their recovery | The first delivery keeps child calls in the branch's runtime state; the durable contract remains [#1276](https://github.com/arcships/dim/issues/1276). |
| A separate OS-confined native host process | The first delivery runs QuickJS in the daemon process with memory, time and interrupt limits; process-level hardening is future work. |
| Direct access to ToolExecutor internals, handlers, ToolHost, file descriptors or host objects | Those would bypass the pinned binding and admission boundaries. |
| An implicit grant of all child permissions after approving `codemode` | Each invoked tool remains independently subject to Permission and Approval. |
| Recursive `codemode` calls | The tool list excludes `codemode`; the first delivery has no nested Code Mode. |
| TypeScript type stripping | The first delivery evaluates JavaScript; a TypeScript front end is future work. |

### 1.4 Tool exposure and eligibility

The first delivery is additive: `codemode` is declared beside the other adopted tools. The bridge
exposes every ordinary tool in the outer request's pinned `ToolBinding`, including eligible built-in,
extension and MCP tools. `codemode` itself is excluded. `tools.list` and `tools.describe` are
projections of the same pinned snapshot; `tools.call` rejects a name absent from it. A malformed or
unknown call is a local program error and has no child Timeline fact.

## 2. Own contract

### 2.1 The `codemode` tool

`CodeModeSession` registers one tool named `codemode`:

```json
{
  "code": "const listed = await tools.list(); return listed",
  "timeoutMs": 30000,
  "maxCalls": 50
}
```

| Field | Meaning |
| --- | --- |
| `code` | An asynchronous JavaScript function body. It is rejected when absent, not a string or larger than `max_code_bytes`. |
| `timeoutMs` | Optional wall-clock budget, capped by `max_runtime_ms`. |
| `maxCalls` | Optional nested-call budget, capped by `max_calls`. |

`CodeModeConfig` supplies the hard bounds:

```text
max_code_bytes          64 KiB
max_runtime_ms          30 s
max_calls               64
max_parallel_calls      4
max_memory_bytes        256 MiB
max_output_bytes        1 MiB
max_single_result_bytes 256 KiB
```

The outer tool is declared with risk `session`. It therefore enters the normal ToolExecutor
admission path under the Session's permission preset. Approving the outer call does **not** approve
child calls: each child uses the invoked tool's own risk.

The outer result is a normal `ToolResult`. `content` is the program's returned value serialized as
JSON, or a stable terminal message on failure. Its `effects`, when present, are the confirmed effects
of child `ToolResult`s in child sequence order, including effects from an error result. A generated
timeout or failure result carries the effects that completed before it.

### 2.1.1 Impact and public additions

| Owner | Addition | Why it is public |
| --- | --- | --- |
| ToolExecutor mechanism | `definitions(request_id)` on the pinned binding and `nested(call, context, cancel)` admission | The branch part must read the request's declarations and run one child through the same pinned handler and Approval path without writing a second model batch. |
| Session tool binding | `codemode` declaration and result contract | The model must be able to adopt and call the tool. |
| App Server assembly | `dim_codemode::install` adds the process row and its optional parts | The product profile installs it; a host that does not want the tool omits the call, and no Session publication waits for the optional parts. |
| Approval mechanism | Reuse of `approval/request`, `approval/respond` and cancellation | A child call's `BranchSource::Tool.call_id` is its own invocation id; no Approval field or client decoder changes. |
| Client protocol and storage | No change | Child calls write no Timeline facts; the outer call keeps the existing `step/tool-call` and `step/tool-result`, and no native-store step is required. |

Existing tool declarations, ordinary `step/tool-call` semantics, context inputs and the Workflow
contract are unchanged. No new Timeline event, event domain or storage format is introduced.

### 2.2 QuickJS bridge

The program sees only:

```text
tools.list() -> [{name, description}]
tools.describe(name) -> declaration | undefined
tools.call(name, arguments) -> Promise<ToolResult>
```

The QuickJS context has no `process`, environment, filesystem, network, timer, child-process or
module loader, and no Rust/host object. `tools.call` serializes arguments as JSON and rejects a
non-object argument. A child `ToolResult` with `isError` becomes a catchable program error with
`code` and `message`; other results are returned as the serialized `ToolResult`.

The runtime enforces at least `max_code_bytes`, `max_runtime_ms` (QuickJS interrupt handler),
`max_memory_bytes` (QuickJS memory limit), `max_calls`, `max_parallel_calls` (semaphore),
`max_single_result_bytes` and `max_output_bytes`. A synchronous loop is interrupted by the QuickJS
handler; cancellation also sets the interrupt.

### 2.3 Nested invocation contract

The mechanism exposes, on the branch's `ToolExecutor` service:

```text
definitions(request_id) -> [ToolDefinition]
nested(call, context, cancel) -> Result<ToolResult, HostError>
```

`definitions` returns the request's pinned declarations, sorted by name. `nested` resolves the
handler from the same request snapshot, applies the invoked tool's risk through the existing
Approval admission, and runs the handler. It does not write a `step/tool-call` batch and does not
expose the handler to the plugin or the QuickJS context. The child's `ToolCall.id` is a fresh
invocation id used only for Approval correlation and diagnostics; the terminal result is returned to
the waiting program by the bridge, not committed as a second model result.

## 3. State, cancellation and effects

`CodeModeBranch` keeps only runtime handles in memory. The business state needed to settle the outer
call is the existing ordered Timeline state:

| State | Owner | Rebuild |
| --- | --- | --- |
| outer tool call and result | ToolExecutor | From `step/tool-call` / `step/tool-result` |
| active program and its child calls | CodeModeBranch / QuickJS | Runtime-only; cancelled and settled on close or reload, never replayed |
| confirmed child effects | CodeModeBranch | Copied onto the outer result before it returns |

The branch creates a fresh `AsyncRuntime` and `AsyncContext` per call, sets the memory limit and the
interrupt deadline, runs the program, and drops the context when it returns. The program's returned
promise is awaited, so every child call the program awaits settles before the outer handler returns.
A call the program drops is cancelled with the QuickJS context; durable settlement of such a call is
deferred with the child-fact work in [#1276](https://github.com/arcships/dim/issues/1276). The
first delivery does not claim the full reload parent gate.

Child effects are collected by invocation sequence, including effects from a child whose result is
an error, and appended to the outer `ToolResult.effects`. The branch does not shorten, truncate or
drop an effect; `max_calls` bounds the number of child results it can aggregate, and each invoked
tool remains responsible for its own result bounds. ToolExecutor records the returned outer result
unchanged under the Session boundary contract.

No `codemode/started`, `codemode/finished`, `step/tool-invoke` or `step/tool-invocation-result` fact
is written in the first delivery. The outer `step` facts already record the call, and the QuickJS
runtime is one-shot; adding durable child state without a consumer would duplicate it.

## 4. Approval and Permission

There are two authorization decisions:

```text
outer codemode call
  -> Permission / Approval for codemode's declared `session` risk

nested tools.call(name)
  -> Permission / Approval for name's declared risk
```

Child approvals reuse the existing `approval/request`, `approval/respond` and cancellation contract.
`ApprovalRequestData.source` is `BranchSource::Tool { call_id: invocationId, turn_id, request_id }`;
`approval_id` remains the only key accepted by the response operation. A response authorizes only
that approval and child invocation, never a sibling, future or outer `codemode` call. TUI, App Server
and other clients need no decoder change; they already render the existing call identity.

## 5. Dependencies

| Dependency | Scope | Consumer | Purpose |
| --- | --- | --- | --- |
| `partRegistry` | Process | `CodeMode` | Register `CodeModeSession` and `CodeModeBranch`. |
| `CodeModeRuns` | Process | `CodeModeSession`, `CodeModeBranch` | Resolve the calling branch's runner; the process plugin provides it. |
| `toolRegistry` | Session | `CodeModeSession` | Register the `codemode` tool. |
| `ToolExecutor` | branch | `CodeModeBranch` | Read the pinned declarations and run child calls with Approval admission. |
| `TimelineIdentity` | branch | `CodeModeBranch` | The calling branch id. |
| `rquickjs` | library | `CodeModeBranch` | The QuickJS runtime and the `tools` bridge. |

The plugin does not depend on `ToolHost`, a handler registry, a second event bus, direct Session
facade access or a Node/TypeScript runtime.

## 6. Requirements raised to mechanism owners

### R1. ToolExecutor nested invocation

| Field | Content |
| --- | --- |
| Problem | A branch part must run an adopted tool while its own outer tool call is current. Calling a handler or `ToolHost` directly bypasses the pinned binding and Permission; writing a nested `step/tool-call` makes Loop and ToolExecutor treat the child as a second model batch. |
| Existing mechanisms considered | `ToolBinding.handler` alone bypasses Approval; a second `step/tool-call` fails with "concurrent tool batches"; a Session-side registry read is not the request snapshot. |
| Generic capability | `ToolExecutor::definitions(request_id)` and `ToolExecutor::nested(call, context, cancel)` on the branch service, using the request's pinned handler, the invoked tool's risk and the existing Approval admission. No handler exposure and no second batch. |
| Sequence, batch, interruption and reload | The child call runs inside the outer handler's future, under the outer call's cancellation token. A reload settles the outer call through ToolExecutor's existing rebuild; the child is not replayed. The first delivery does not write durable child facts. |
| Owner and issue | Dim runtime / ToolExecutor mechanism owner; [#1276](https://github.com/arcships/dim/issues/1276). |

### R2. QuickJS execution boundary

| Field | Content |
| --- | --- |
| Problem | A model-supplied program cannot be safely run with `eval`, Node or the trusted plugin process. The engine must have no host capability except the tool bridge. |
| Existing mechanisms considered | The `rquickjs` native QuickJS binding, `AsyncRuntime`/`AsyncContext`, `Promise`-wrapped host functions, `set_memory_limit` and `set_interrupt_handler`. |
| Generic capability | One fresh QuickJS context per call with only `tools.list`, `tools.describe` and `tools.call`, a hard memory limit, a wall-clock interrupt and cancellation. A separate native host process is a future hardening, not part of this delivery. |
| Sequence, batch, interruption and reload | The context is dropped after the program's promise settles. Child calls the program awaits settle through ToolExecutor; a dropped call is cancelled with the context and is not replayed. Reload settles the outer call through the existing ToolExecutor contract. |
| Owner and issue | Runtime/host mechanism owner; [#1276](https://github.com/arcships/dim/issues/1276). |

## 7. Rule check

| Rule | How this design complies |
| --- | --- |
| §1 Capability boundary | `CodeModeBranch` is the only Code Mode Capability. `CodeModeSession` only registers a tool; `CodeMode` only owns process configuration and registrations. |
| §4.1 split by state owner | Process owns `CodeModeRuns` and the bounds; Session owns the tool declaration; Branch owns the one-shot QuickJS execution; ToolExecutor owns admission and the outer facts. |
| §4.2 command and handoff | The outer `step/tool-call` is the only command. A child call is a direct branch invocation through `ToolExecutor::nested`; it writes no second command and is never replayed after an uncertain effect. |
| §4.3 handlers register work and return | The `codemode` handler awaits the QuickJS program and its awaited child calls; the branch owns the runtime waiters and settles them before returning. |
| §4.4 core mechanism has no business meaning | The nested invocation API is generic ToolExecutor admission; it names no Code Mode type. |
| §4.5 atomic batches and ordered dispatch | Child calls do not create a model batch. The outer batch remains current until the handler returns; the outer result is committed once. |
| §4.6 context boundary | Child results never enter context. Only the outer `step/tool-result` is model-facing; a value the model needs is in the outer `content`. |
| §4.7 public interaction | The plugin uses `ToolExecutor`, `ToolRegistry`, `CodeModeRuns` and `rquickjs`; it receives no handler or ToolExecutor internals. |
| §4.8 infrastructure requirements | R1 and R2 state the existing mechanisms, the generic addition, cancellation/reload behavior and the owners. |
| §4.9 facts in Timeline | The first delivery adds no Code Mode fact. The outer `step/tool-call`/`step/tool-result` are the durable record; child effects travel on the outer result. |
| ADR 002 D1 | Process `CodeMode`, Session tool, Branch execution and the ToolExecutor mechanism each own their own state. |
| ADR 002 D2 | No wiring plugin, second bus or lifecycle manager is added. |
| ADR 002 D3 | Dependencies point from Code Mode to existing core contracts and ToolExecutor; core does not depend on the optional crate. |
| ADR 002 D4 | The first delivery's bounds are structural `CodeModeConfig`; a user Settings source is deferred. |
| ADR 002 D5 | The nested invocation and QuickJS contracts are Rust data/error shapes; the binding does not leak into the design. |

Ablation: the first delivery intentionally removes durable child facts, a second tool registry, a
persistent REPL, a separate host process, a user settings source and automatic child retries. The
outer `step` facts plus the branch's one-shot QuickJS execution are sufficient for the first
delivery; adding durable child state without a recovery consumer would duplicate the outer result.

## 8. Binding appendix

The following is semantic Rust pseudocode, not a committed spelling.

```text
CodeMode::apply(ctx):
  runs = Arc<CodeModeRuns>
  ctx.provide(runs)
  partRegistry.register(Session, "CodeModeSession", |names| CodeModeSession::new(names, runs))
  partRegistry.register(Branch,  "CodeModeBranch",  |names| CodeModeBranch::new(names, config))

CodeModeSession::apply(ctx):
  tools.register(declaration = {
    name: "codemode",
    risk: "session",
    inputSchema: { code, timeoutMs?, maxCalls? }
  }, handler = |call, context, cancel|:
    runner = runs.get(context.branch_id) or error
    runner.run(call.code, call.timeoutMs, call.maxCalls, context, cancel))

CodeModeBranch::run(...):
  runtime = AsyncRuntime::new(); runtime.set_memory_limit(max)
  runtime.set_interrupt_handler(deadline || cancel)
  context = AsyncContext::full(runtime)
  context.async_with(|ctx|:
    install tools.list / tools.describe from executor.definitions(context.request_id)
    install tools.call = Async(|name, args| executor.nested(ToolCall{name, args}, context, cancel))
    promise = ctx.eval(async function body)
    value = promise.into_future()
    return ToolResult { content: JSON(value), effects: collected_effects })
```

The host adds the process row with `dim_codemode::install`; `install_required` also makes both
parts required for a Session that must publish only after the tool is registered.
