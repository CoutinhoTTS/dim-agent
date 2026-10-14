# Process extension registry

`dim_agent_runtime::extension_registry` implements the shared lifecycle mechanism
from the [overall design proposal](https://github.com/arcships/dim/pull/15).
An extension point supplies its contribution type, point name and validation.
The registry handles discovery and generation ownership, not domain behavior.
Existing capabilities do not become contributors automatically.

## Assembly and ownership

`register::<P>(&mut builtins)` registers two loader modules and returns their
ordinary assembly rows: the stable registry and an instanced `<point>-mount`
group. They use the application's existing loader, never a second runtime.

A contributor provides `service_key::<P>(name)` in its own plugin. Its module's
`Builtins::declare` callback includes `service_name::<P>(name)` in `provides`.
The declaration string is `dim.extension/<point>/<name>`; the injected key is
`TypeKey::keyed_dynamic::<P::Service>(name)`. Multiple contributions belong in
one declaration callback so they do not replace one another. Callbacks receive
the row's `config`, as required by rutis-loader, not its outer options object.

Discovery reads non-instance loader entries excluding disabled and inactive
rows. It maintains one mount per configured name and records unknown
declarations separately. A missing service or a rejected contribution is
configured but inactive, not absent. Consumers implementing a fail-closed point
must inspect both `inactive()` and `undetermined` before admitting work.

Only mounts inject contributors. Consumers inject `ExtensionRegistry<P>` and
resolve a contribution for each call. Restarting a contributor therefore does
not restart unrelated contributors or their consumers. A mount registers its
cleanup before publishing the active generation.

## Calls and shutdown

`Active::admit()` atomically checks admission and returns a `CallGuard`.
The caller retains this guard until the actual operation settles, including any
work that outlives its response. A retained generation cannot admit work after
unloading starts. Unload closes admission, signals guards, withdraws availability
and waits for all admitted work before releasing the contribution dependency.

The consumer contract chooses how to react to the guard's cancellation: stop and
report unavailability, or drain an already admitted operation. Dropping a response
waiter alone must never drop the guard for a still-running mutation. The registry
does not invent business cancellation rules or write Timeline events.

The host awaits `ready()` after process assembly and before accepting requests.
Do not await it inside plugin `apply`: discovery first waits for initial loading
rows to settle. Missing dependencies remain diagnosed inactive entries rather
than blocking unrelated capabilities. Discovery and mount disposal belong to
the registry plugin's asynchronous effect.

Lifecycle flow coverage lives in `dim-agent-runtime/tests/lifecycle_review.rs`:
initial discovery, rejected and missing contributions, an unknown declaration,
reload during an admitted call, unaffected consumers, removal and shutdown.
