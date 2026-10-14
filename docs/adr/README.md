# Architecture decisions

Decision status and implementation status are separate. **Accepted** means the direction
is approved; it does not mean every interface exists. A superseding decision preserves the
reasoning of its predecessor. Architecture documentation describes implemented behavior.

| Decision | Status | Implementation |
| --- | --- | --- |
| [001 — Extensible harness foundation](001-harness-foundation.md) | Recovery/configuration accepted on this branch; event and dependency-loss refinements proposed | In progress; see its delivery table |
| [002 — Plugin scope assignment and configuration placement](https://github.com/arcships/dim/blob/docs/plugin-designs/docs/adr/002-plugin-scope.md) | Proposed in PR #15; D4 correction authorized for this branch | Authoritative structural/business distinction; language-neutral declarations remain deferred |
| [003 — Session-owned MCP and business configuration](003-loader-plugin-management.md) | Approved on this branch; supersedes 001's MCP ownership, aligned with PR #15 D4 | Source-owned v13 business settings and Session content instances; not generic structural management or a package manager |

Source baseline: [initial foundation parent](https://github.com/arcships/dim/commit/aaaffa5768e031d781abb73e45b1915b05dc2aa0).
This baseline includes lifecycle fixes not necessarily present on the default branch.
