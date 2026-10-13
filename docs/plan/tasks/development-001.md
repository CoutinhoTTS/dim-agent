---
id: development-001
package: dimcode
module: development
status: done
depends-on: []
---

# Local development with a private source package

## Context

- The user authorized the development entry and CLI catalog/daemon controls. Local loading is restricted to a package built from OAuth source. Private GitLab release CI is deferred.
- Contract: [development workflow](../../architecture/development.md).
- Public baseline: `7ed00aa515a3c30473eb54b1a2d93889e9ffd45a`; private baseline: `05f83f7ce45fffab8fca6a75289f477c91be75a6`.
- GitLab-only delivery. Public author/committer: `zerob13 <zerob13@gmail.com>`. GitHub/npm workflows stay disabled. No release tags or package publication during implementation.

## Ownership

| Work | Owner | Directory | Scope |
| --- | --- | --- | --- |
| Shared archive validation and development entry | Root agent | `temp/dimrust-cli` | `dev`, `scripts`, root documentation |
| Native model catalog and normal daemon stop | Rust CLI agent | `temp/dimrust-cli` | CLI sources, CLI flow tests and CLI README |
| Delegated private development entry | Private OAuth agent | `temp/dim-oauth-plugin` | Dev wrapper, private flow tests and docs |

Shared contracts are owned by the root agent. Disjoint files may be edited in the same checkout; commits and remote writes are performed by the root only. Each implementation receives independent review by another participant and integration exercises the real local package/runtime/CLI chain.

## Delivery

The local source package entry, catalog/daemon commands and private delegation are implemented and independently accepted. The actual two-repository developer flow completes device login, model catalog refresh, authenticated model HTTP, persisted-session resume through normal daemon restarts and graceful stop with a local external HTTP fixture. Evidence and limits are recorded in [verification](../reviews/development-001.md).

- Public review: https://gitlab.bfbops.com/nextim/dimrust-cli/-/merge_requests/2
- Private review: https://gitlab.bfbops.com/nextim/dim-oauth-plugin/-/merge_requests/1
- Private release CI, remote plugin loading and all publication remain deferred.
