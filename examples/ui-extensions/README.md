# Client UI example

A UI-only plugin written in plain HTML, CSS and JavaScript. No package manager,
framework or build step is needed.

```text
ui-extensions/
  dim-plugin.json   panel declaration and required capabilities
  ui/
    panel.html      markup
    panel.css       styles using the host's semantic theme variables
    panel.js        mount, data access and cleanup
```

Install this directory with `plugin.install`:

```json
{"source":{"path":"/absolute/path/to/examples/ui-extensions"}}
```

Open a Web conversation and choose **Activity** from **Extensions**. The panel
observes the bound branch and Session activity. **Read recent history** reads
that branch's public events.

`panel.js` exports `mount(root, context)`, loads its HTML and CSS relative to
`import.meta.url`, and returns cleanup synchronously. The host aborts
`context.signal` before cleanup, stopping the HTML fetch, listeners and stale
result rendering. The files run as trusted plugin code in a ShadowRoot.

After editing any file, run `plugin.reload {"name":"conversation-inspector"}`.
The host publishes a fresh resource snapshot; no build or daemon restart is
needed. Disable or remove it through the existing plugin commands.

See the [client UI contract](../../docs/design/client-ui-extensions.md) for the
available capabilities and `import type { PanelModule } from 'dimcode/ui'` for
TypeScript declarations. The example does not publish backend files. Backend
parts can use `uiResources.publishFile({path, mediaType})` with a canonical
absolute host path inside an already authorized FileStorage root. The returned
`{sessionId, id}` may be returned by a declared Session method and converted by
`context.services.resources.url(reference)`. Relative `pluginFiles` paths cannot
be passed directly. See the [resource contract](../../docs/design/client-ui-extensions.md#62-session-文件资源)
for ownership, path sources and withdrawal.
