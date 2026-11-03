# dim-session-title

SessionTitle is an independent rutis plugin installed by the `dimcode` product profile through
`install(&mut AgentBuilder)`. The App Server production library does not depend on or install this crate;
an embedding host chooses it explicitly. Settings and SessionStore remain existing App Server
owners, consumed through their public Rust service bindings. Published history is read through
the existing named `sessions.history` contract, without an App Server implementation dependency
on this plugin or an additional native history entry.

The process part owns its Settings declaration and Session part registration. Each Session part
owns observation, generation tasks, routing, cancellation and manual rename. Persistent title
state and revision checks belong to SessionStore. The public generation request/reply types and
the optional `sessionTitleGenerator` contract belong here; DimOAuth implements that service
without providing a fallback or writing Session state.

Inputs, outputs, errors, configuration, lifecycle and the rule check are defined in the
[SessionTitle design](../../docs/design/plugins/SessionTitle.md). Public flows live in `tests/`,
using the public `dim-client` and App Server APIs. Each fixture installs this crate explicitly. App Server has neither a production nor a test dependency on
SessionTitle; its tests cover its own interfaces and generic plugin policy.
