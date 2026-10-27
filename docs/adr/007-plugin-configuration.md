# 007: PluginManager keeps plugin configuration; rutis and content owners keep state

Decision status: Accepted by the repository owner (2026-10-27). Implementation: not started; the
design is [PluginManager](../design/plugins/PluginManager.md), tracked by
[#3](https://github.com/arcships/dim/issues/3) and [#384](https://github.com/arcships/dim/issues/384).
[中文](../zh/adr/007-plugin-configuration.md)

Relation to other decisions:

- Corrects [ADR 002](002-plugin-scope.md) D4: structural plugin configuration does not live in the
  loader's editable layer (issue #3); it lives in PluginManager's own record (D1). D4's split between
  structural and business configuration stands.
- Corrects [ADR 006](006-plugin-contract.md) D3's first point, "Its configuration, enablement and
  reload are that row's": a plugin's code is one row and its reload is that row's, but whether it is
  installed, used or turned off is PluginManager's configuration, and the content a plugin carries is
  not a row (D2, D3). The rest of D3 stands.
- Withdraws the direction recorded for #3 in [ADR 001](001-harness-foundation.md) (its header, delivery table and configuration notes),
  "structural plugin configuration uses the persisted editable layer and loader operations".
- Keeps [ADR 003](003-approval-scope.md): enabled plugin code runs without host-local approval.

## Context

Issue #3 decided that the loader's editable layer, with its persistence, is the single authority for
enabling, disabling and configuring plugins. The first plugin management design (#466) and its
implementation (#470–#474) followed it: the editable layer was the install record, and plugin
content (skills, MCP configuration, hooks) became loader "content rows". The maintainer rejected that
model and the six PRs were closed. Two problems:

- Dim product state stored as rows of an infrastructure library: what is installed and from where is
  Dim's own data, and the editable layer is rutis' place for a host's configuration of rows.
- Content that is not code was made into rows only so that the loader could hold it, coupling skills,
  MCP configuration and hooks to the rutis lifecycle.

## Decisions

### D1. PluginManager keeps plugin configuration, in its own record

PluginManager keeps the plugin configuration of a host in its own file, in Dim's own format: which
plugins are installed, where they came from, which plugin each name uses, and which names are turned
off. It keeps configuration only, never runtime state. The record covers every plugin: built-in rutis
plugins (only the user's decisions about them), default plugins listed in the default configuration
file, and plugins the user installs. The default configuration file also lists the locked names,
which cannot be turned off or switched.

### D2. rutis keeps the state of plugin code

A plugin's code is one rutis plugin, one loader row. Whether a row exists, runs or failed is rutis'
state, read through rutis-loader's API. PluginManager carries out its configuration with
rutis-loader's own operations; the loader keeps no install record, and nothing of PluginManager's
record is stored as loader rows.

### D3. Content owners keep the state of content

Content a plugin carries (skills, MCP configuration, hooks) is a different kind of object from a
rutis plugin. PluginManager registers it with its owner (SkillsCatalog, Mcp, Hooks) and withdraws it
when the plugin stops being used. The owner keeps what was registered as its own state. Owners never
look for plugins.

### D4. Plugins are identified by name and replace each other

Several plugins with the same name may be installed; one of them is used at a time. Turning another
one on replaces the one in use. This is how built-in and default plugins are replaced; they can be
turned off and replaced, not removed. Built-in Rust code cannot be reloaded.

### D5. A plugin's business configuration is the plugin's own

A plugin registers its own Settings sources at run time; neither the App Server's assembly nor
PluginManager declares them. ADR 002 D4's business configuration stays with its owner.

## Consequences

- The PluginManager design is rewritten on these decisions; its old `pluginContents` publication is
  gone.
- Settings gains run-time source registration, and the sources hard-coded at assembly are removed.
- SkillsCatalog, Mcp and Hooks gain register and withdraw operations and keep registered content.
- No requirement is raised on rutis.

## Alternatives considered

- **The loader's editable layer as the record (issue #3, #466).** Rejected: it stores Dim product
  state in rutis and makes content into rows.
- **PluginManager keeping runtime state as well**, such as a full table of what should be loaded.
  Rejected: rutis and the owners already hold that state; a second copy can disagree with it.
- **A conflict check before replacement.** Rejected: plugins replace each other by name, and rutis
  already refuses a second provider of the same service.
