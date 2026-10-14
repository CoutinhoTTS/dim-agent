# Architecture decisions

Decision status and implementation status are separate. **Accepted** means the direction
is approved; it does not mean every interface exists. A superseding decision preserves the
reasoning of its predecessor. Architecture documentation describes implemented behavior.

| Decision | Status | Implementation |
| --- | --- | --- |
| [001 — Extensible harness foundation](001-harness-foundation.md) | Recovery/configuration accepted on this branch; event and dependency-loss refinements proposed | In progress; see its delivery table |
| [002 — Session-owned MCP and editable plugin configuration](002-loader-plugin-management.md) | Approved on this branch; supersedes 001's MCP ownership and user configuration direction | Scoped configuration management and Session content instances implemented; not the full issue #3 or package manager |

Source baseline: [initial foundation parent](https://github.com/arcships/dim/commit/aaaffa5768e031d781abb73e45b1915b05dc2aa0).
This baseline includes lifecycle fixes not necessarily present on the default branch.
