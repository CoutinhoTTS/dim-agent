# Session and branch instances hold one row

Status: Draft.

Scope: Session, branch.

Rules: [design rules](../architecture/rust-agent/principles.md), [ADR 002](../adr/002-plugin-scope.md),
[ADR 006](../adr/006-plugin-contract.md) D3 and D4, [How Dim uses rutis §2](rutis-usage.md#2-plugins-are-rows-instances-mount-their-parts).

## 1. Scope and non-goals

The loader's job is the rows a configuration names. A Session or branch is also a loader instance,
and the reason is narrow. When a part of another language is mounted, rutis resolves the instance
names it injects (`sessionMethods`, `toolRegistry`, `dispatcher`, …) by finding the copy's instance in
the loader's own instance table:

- `Definition::at` needs the loader's `PluginMounter` ("needs rutis-loader in the tree",
  rutis-bridge `runtime/definitions.rs:83`);
- the mounter calls `Loader::build_at`, which matches the copy's scope chain against the loader's
  instances (rutis-loader `loader/mod.rs:331`).

Everything else an instance does is Dim's:

- the mounters mount parts with `ctx.plugin` and follow the part registry ([How Dim uses rutis §2](rutis-usage.md#2-plugins-are-rows-instances-mount-their-parts) "Mounting");
- creation waits for the required parts and lets the optional ones settle;
- closing goes through rutis's ownership of children.

Today the instance is more than that identity. Its mounters are rows of its group, and creation finds
them, and judges readiness, by scanning every loader entry
([#594](https://github.com/arcships/dim/issues/594)). This design keeps the loader instance for what
needs it, the identity that foreign copies resolve names against, and moves the rest under the
scope's own fiber.

Non-goals:

- **Removing the loader instance.** Foreign copies would then need to resolve instance names without
  one, and rutis 0.19 does not offer that. Following
  [design rules §4 rule 8](../architecture/rust-agent/principles.md), this design depends on no
  change to rutis.
- **Extension instances.** The nested groups created for Sessions that ask for them (Skills content,
  MCP publication) keep their rows. Only the launcher that creates them moves (§2.2).
- **Unchanged parts of the assembly:** the Agent group and its process values, the catalog's groups
  and `Names`, and the reconcile that a name claimed in an instance starts.

## 2. Own contract

### 2.1 Now

```text
session group (instanced)                  branch group (instanced, nested)
  SessionScope                               BranchScope
  SessionExtension (when extensions exist)   required mounter
  required mounter                           optional mounter
  optional mounter
  branch group
  extension groups
```

Creation calls `create_instance`, then finds the instance's rows by scanning `Loader::entries()`:

- to report the required rows' failures;
- to wait for them to be Active;
- to let the optional rows settle (`dim-agent-runtime/src/assembly.rs:1492`, `:1504`, `:1508`).

Locating an instance after a timeout (`locate`, `:1575`) and the assembly check (`scope_rows`,
`:2018`) scan the same way.

### 2.2 Target

```text
session group (instanced)                  branch group (instanced, nested)
  SessionScope ─ ctx.plugin ─┐               BranchScope ─ ctx.plugin ─┐
    required mounter  ◄──────┤                 required mounter ◄──────┤
    optional mounter  ◄──────┤                 optional mounter ◄──────┘
    extension launcher ◄─────┘
  branch group
  extension groups
```

- **One row per instance.** The Session group holds the `SessionScope` row, the branch group and the
  extension groups. The branch group holds the `BranchScope` row. No mounter or launcher is a row.
- **The scope mounts the rest.** After providing its services, the scope's `apply` mounts the required
  mounter and the optional mounter as its own children with `ctx.plugin`. The Session scope also
  mounts the extension launcher (today's `SessionExtension` row), which is unchanged otherwise. The
  scope then returns. It waits for none of them in `apply`, as a row did not.
- **Mounters keep their contract.** They call the generators, mount parts with `ctx.plugin`, follow the
  registry, and hand the copies they mount when the instance opens to its creation (How Dim uses
  rutis §2 "Mounting"). The scope gives them its `Build`, the one each mounter row receives today, so
  `Names` and every key stay the same.
- **Foreign copies keep resolving.** A copy is mounted under a mounter, one level deeper than today but
  still inside the instance. A context's scope chain follows its parent fibers (rutis `ctx.rs:1302`),
  so `build_at` finds the same instance.
- **Creation.** `create_instance` loads one row. Creation then waits as it does today:
  - the scope must be Active;
  - the copies the mounters hand must be Active if required, and must settle if optional.

  Nothing scans the loader's entries. A failure is the scope row's report or a handed copy's state.
- **Timeouts and the assembly check** read the scope's children and the copies its mounters hand,
  instead of the loader's entries. `scope_rows`, `locate` and `rows` go.
- **Closing** is unchanged: the scope's children close with it.

### 2.3 Observable changes

- **Fiber tree:** the mounters and the extension launcher sit under the scope instead of beside it.
  The `session` and `main` subgraphs of the generated graphs regenerate; no service or edge changes.
- **Loader entries:** `Loader::entries()` no longer lists mounters. PluginManager lists process rows
  only, so `plugin.list` does not change.
- **Creation cost:** the scans of #594 are gone, which is #594's own proposal (the scope and the
  mounters know what they mounted). `xtask measure` at 1, 10, 100 and 1000 Sessions records the
  effect.

## 3. Events and state

None. No Timeline fact, bus event or persisted state changes.

## 4. Dependencies

- **rutis-loader:** instanced groups, `create_instance` and its report, `Build`. The loader's
  `PluginMounter` mounts foreign copies.
- **rutis:** `ctx.plugin`, fiber states and the scope chain of a context.
- The part registry and the mounters, as in How Dim uses rutis §2.

## 5. Cross-module requirements

None. A Session or branch that is a plain plugin, with no loader instance at all, would need foreign
copies to resolve instance names without one. That is not available in rutis 0.19, and this design
does not depend on it.

## 6. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (placement, `apply`, effects) | The scope mounts its mounters from `apply` after providing its services and returns. Children close with it. Mounters still start background work only after publication |
| §4 rules 2, 3, 5, 6, 9 | Not applicable: no fact, handler, batch, context path or Timeline write changes |
| §4 rule 4 (state ownership) | Readiness is read from the views the scope and its mounters own, not from another owner's table |
| §4 rule 7 (reuse mechanisms) | Uses the mounters, `ctx.plugin` and rutis's children; adds no mechanism |
| §4 rule 8 (infrastructure unchanged by default) | The change closes inside Dim's assembly. The loader is used as it is, and nothing is asked of rutis |
| ADR 002 D1–D5 | No placement, wiring, dependency or configuration change. Parts stay in their Session or branch |
| ADR 006 D3 | Unchanged: the instance mounts the parts registered for it, now from its scope |
| ADR 006 D4 | Unchanged: names resolve to the nearest enclosing instance of their group, through the same `Build` and the same loader instance |

## 7. Verification

- The Session and branch flows of `dim-agent-capabilities` and `tests/integration`, unchanged in their
  assertions:
  - required parts hold creation until Active;
  - a failed required part fails creation;
  - optional parts settle;
  - registrations follow open instances.
- The foreign part flows (SessionTitle, Scheduler, Workflow and Goal, and the Node-backed App Server
  flows): copies resolve `session` and `branch` names inside their instance.
- Extensions: an extension instance is created for a Session that asks for it, and not for others.
- `Agent::check_assembly()` reports an unawaited child, as today.
- `xtask graph`: only the `session` and `main` subgraphs change, by the mounters moving under the
  scope.
- `xtask measure` before and after.

## 8. Documentation to update with the implementation

- [How Dim uses rutis §2](rutis-usage.md#2-plugins-are-rows-instances-mount-their-parts) "Mounting":
  "Every Session and branch instance holds two mounter rows, the only rows of its group besides the
  scope's own mechanisms" becomes "holds one row, its scope, which mounts the two mounters as its
  children". Appendix A follows.
- [single-tree](../architecture/rust-agent/single-tree.md): the assembly section's description of
  required rows and mounter rows.
- The generated graphs, through `xtask graph`.

## Appendix A. Rust binding

- `AgentBuilder::install` registers only the scope modules and returns `SESSION_GROUP` with
  `[SessionScope, branch group, extension groups]` and `BRANCH_GROUP` with `[BranchScope]`.
- `SessionScopePlugin` and `BranchScopePlugin` build their two `PartMounter`s from their own `Build`
  and `MountSite`, and mount them with `ctx.plugin`. The Session scope also mounts
  `SessionExtensionPlugin` when the assembly has extensions.
- `create_instance` in `assembly.rs` waits on the scope's view and the copies in `Opened`, and takes
  no `required` row list.
