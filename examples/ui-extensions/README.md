# Client UI examples

The [client UI contract](../../docs/design/client-ui-extensions.md) defines the
mount lifecycle and available capabilities. These packages run in a trusted
browser realm and bundle their own runtime dependencies.

- `dom` is a UI-only package. It needs no build and reads the bound branch's
  activity and history through its declared capabilities.
- `vue` builds an independent ES module containing Vue. Run `npm install` and
  `npm run build` in that directory before installing the package. Its build has
  no external runtime imports and uses no host framework objects.

Install the absolute package directory with `plugin.install`:

```json
{"source":{"path":"/absolute/path/to/examples/ui-extensions/dom"}}
```

Open a conversation in the Web workspace, then select the panel from Extensions.
Use `plugin.reload {"name":"conversation-inspector"}` after changing the DOM
example's files. Each successful reload publishes a new immutable resource
snapshot. Disable or remove the package through the existing plugin commands.

TypeScript authors use `import type { PanelModule } from 'dimcode/ui'`. The
package exports declarations only; there is no browser runtime to initialize.
Keep module top-level code free of side effects. Use the binding's abort signal
for listeners and fetches, and return the framework's unmount operation.

Backend parts publish finite, finalized files through the Session-scoped
`uiResources` service. `publishFile({path, mediaType})` returns `{sessionId, id}`;
return that reference from a declared Session method and render the address
from `context.services.resources.url(reference)`. The part keeps the file
unchanged while published and calls `withdraw(reference)` when it is no longer
needed. Caller unload and Session closing also withdraw it. The browser receives
neither the file path nor publication authority.
