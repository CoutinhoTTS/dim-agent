## Summary

<!-- What changes and why. Link the design document. -->

## Design rule checklist

Answer each item; write "n/a" with a reason where it does not apply. Rules: [design rules §4](../docs/architecture/rust-agent/principles.md), [ADR 002](../docs/adr/002-plugin-scope.md).

- [ ] Scope: each new part is placed by the owner of its state and lifecycle (process / Session / branch); no wiring module.
- [ ] Commands and results: changes to another module's state are command events; the owner writes the result.
- [ ] Handover: every handover point is an explicit event; nothing is revoked after handover.
- [ ] Before-write intervention uses Timeline `before` callbacks only; no extra checks at RPC/Session entries or inside consumers.
- [ ] Core mechanisms and consumers gain no plugin-specific handling.
- [ ] Batch: decisions consider the whole atomic batch.
- [ ] Context: only handed-over, consumed facts reach the model context.
- [ ] No reading another plugin's internal state or deciding for it.
- [ ] Business configuration stays out of loader rows; contracts are language-neutral.
- [ ] Missing mechanism capabilities are raised as requirements (problem, why existing mechanisms fail, generic capability, ordering impact, owner/issue).

## Verification

<!-- Tests and checks run. -->
