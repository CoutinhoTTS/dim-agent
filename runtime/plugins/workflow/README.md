# Workflow

Optional TypeScript package implementing the [Workflow contract](../../../docs/design/plugins/Workflow.md).
The process row owns definitions, runs and report settlement; the Session row provides the `workflow`
tool through the declared `workflowRuns` dependency. All native I/O uses existing `sessions`,
`settings`, `workflowFiles` and `pluginFiles` ports. The package does not install itself in the CLI.

## Loading

```sh
npm ci --prefix runtime --ignore-scripts
npm ci --prefix runtime/plugins/workflow --ignore-scripts
(cd runtime && npm install --no-save --package-lock=false --ignore-scripts --install-links ./plugins/workflow)
```

Pass the complete [process.json](process.json) row to
`ServerConfig.session_typescript.process_rows` and [session.json](session.json) to `rows`, with
`anchor` pointing to the host's `package.json` (`runtime/package.json` here). Include `workflow`
in the Session's selected tools. The host must authorize and provision the product-root `workflow/`
directory before enabling the process row. Missing parent directories fail with `io`; the plugin
does not create them through a filesystem fallback.

The process row's trusted `config.packages` is a list of unique `{id, root}` declarations. Roots
must already be canonical and authorized by the host. The native binding exposes only package ids
to the plugin. Model arguments and workflow files cannot grant file authority or install packages.
An unavailable dependency leaves its optional row Pending under existing assembly rules.

## Definitions and execution

`workflow.save` accepts `{source:"user"|"project", cwd?, document}`. User definitions are saved
with owner CAS in the `workflow` Settings document; project definitions use
`.dimcode/workflows/<name>.yaml` through `workflowFiles`. Project definitions start disabled.
Only the user's local `settings.update` can change the protected
`projectEnabled[canonicalCwd][name]` field. An enabled definition takes effect at the next start;
a running workflow keeps its accepted definition and parameter snapshot.

```json
{
  "schema": "dim.workflow/v1",
  "name": "review",
  "description": "Analyze and summarize a target",
  "parameters": [{"name":"target","required":true}],
  "steps": [
    {"stage":{"id":"analyze","prompt":"Analyze {{params.target}}"}},
    {"stage":{"id":"summary","prompt":"Summarize {{stages.analyze.result}}"}}
  ]
}
```

Tool actions are `list`, `start`, `status` and `stop`. Management exposes the same operations
plus `save` as `workflow.*` methods. Start returns the run id; status returns stored stage results.
The tool is refused in non-interactive Sessions. Existing Permission handles its unclassified risk:
ReadOnly denies, WorkspaceWrite asks and FullAccess allows.

Source precedence is enabled project, user, then plugin; invalid enabled definitions are reported
instead of silently falling through. Duplicate names across installed packages are invalid.
Templates accept only declared parameters and results of earlier steps, never sibling stages.
Supplied/default values are inserted literally; an omitted optional parameter inserts empty text.

Each stage runs in its own non-interactive Session. The actual first launch reply fixes the run's
permission ceiling before any sibling receives input. Management starts default to ReadOnly.
The Settings document contains `max_concurrent_runs` and per-run `max_concurrent_stages`, both
defaulting to 4; each new run reads their current values. A parallel step is rendered completely
before admission. A rendered prompt above 32 KiB fails that step without launching any sibling;
stored stage text is bounded to 64 KiB and report previews to 4 KiB, all measured in UTF-8 bytes.

## Ownership and recovery

The process row tracks admitted calls, run tasks, native operations and file writes. Cleanup stops
new work, requests stage shutdown, joins its work and then closes the original file scope. History
tracking uses a fixed published prefix once per second and follows only each stage's own input.
Held inputs can be withdrawn. Stopping consumed work addresses the branch's current turn; the
existing later-turn race and retained applied input limits remain in force.

Completed, failed and interrupted runs report to their recorded destination as one raw Steer block;
explicit stops and management runs without a destination do not report. Before submission, the
inner run record persists `pending` with `attempt_open:true`. Replacement first uses lookup after
the old row's native writers have drained. A complete host restart is the other settlement boundary.
An unknown lookup alone never proves that a live writer has stopped. Accepted or ambiguous lookup
prevents resubmission; ambiguous lookup records a violation. Missing destinations are undeliverable.

Run storage is `workflow/runs.json`, with inner `{version:2,runs:[...]}` inside the host's S10 v1
wrapper. Invalid, v1, unknown-version or missing-marker records return `invalid_data` and preserve
their bytes. There is no migration. Reloaded unfinished runs become interrupted/lost without
relaunch. Active runs, pending reports and uncertain deletion records are retained; other terminal
history is limited to 100. Owned Sessions are deleted before removing their last ownership record.

## Verification and delivery limits

```sh
node runtime/node_modules/typescript/bin/tsc -p runtime/plugins/workflow/tsconfig.json
node --test runtime/plugins/workflow/workflow.test.mjs
```

Actual Node/App Server flows are in
[workflow.rs](../../../crates/dim-app-server/tests/transport_flow/workflow.rs), using public Node CI's
`DIM_TEST_NODE`, `DIM_TEST_JS_RUNTIME` and `DIM_TEST_RUTIS_SDK` settings and disposable homes.
They exercise stage Sessions, published reports, original-row/runtime drain, crashes and failed
record replacement. Appendix D's complete business acceptance remains required before delivery.
The existing [#126](https://github.com/arcships/dim/issues/126) idle-unload work must land before
shipping repeated stage creation. A launch whose reply is lost before its Session id is persisted
can leave an unidentified Session; it is not relaunched or recovered by a new registry.
