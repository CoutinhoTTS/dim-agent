## Summary

<!-- What changes and why. Link the design document. -->

## Design rule checklist

Answer each item; write "n/a" with a reason where it does not apply. Rules: [plugin contract §6](../docs/design/plugin-contract.md#6-relations-and-their-obligations), [design rules §4](../docs/architecture/rust-agent/principles.md), [ADR 002](../docs/adr/002-plugin-scope.md), [ADR 006](../docs/adr/006-plugin-contract.md).

Relations: for each new or changed part, name the relations it holds (Capability, Submitter, Provider). The design document's Rule check answers their obligations by number.

Every part:

- [ ] Placement: each part is placed by the owner of its state and lifecycle (process / Session / branch); no wiring module. (A1, A2)
- [ ] Dependencies are declared, including load-order ones; a core part takes no hard dependency on an optional one. (A3, P4)
- [ ] Business configuration stays out of loader rows; service contracts are language-neutral: a name, one group, serde shapes, errors as data. (A4, P1)
- [ ] No reading another part's internal state or deciding for it. (A6, S3)
- [ ] Bus notices are used only for observation and display; decisions use facts or the owner's state. (A7)
- [ ] Missing mechanism capabilities are raised as requirements (problem, why existing mechanisms fail, generic capability, ordering impact, owner/issue). (A8)

Capabilities and Submitters:

- [ ] Commands and results: changes to another owner's state are commands or inputs; the owner writes the result; a command with no effect writes no event and the caller's reply says so. (C4, S1, S4)
- [ ] Handover: every handover point is an explicit event; nothing is revoked after handover.
- [ ] Rebuild and lifecycle: state is rebuilt from the Timeline on every generation; handlers register work and return; background work starts after activation and settles, with terminal facts, before the part's services are withdrawn. (C2, C3, C5, C6, C8)
- [ ] Timeline `before` callbacks only transform the draft synchronously: they never block the write or wait for external work (ask the event owner for a generic field when the draft cannot express it). Entries validate only shape and authorization; feature checks belong to the feature owner. (C9)
- [ ] Batch: facts that take effect together are written in one batch, with the deciding event last. (C7)
- [ ] Context: only handed-over, consumed facts reach the model context.
- [ ] Timeline holds only facts later decisions use; UI-only changes go through Session broadcast notices; process-level state is changed through its owner's typed write operation, not the Timeline. (C10)

Mechanisms:

- [ ] Infrastructure (Timeline, Dispatcher, event envelope, assembly and lifecycle, catalog, Session and branch facades) is unchanged, or the change comes from and closes within the infrastructure itself.
- [ ] Core mechanisms and consumers gain no plugin-specific handling.

## Verification

<!-- Tests and checks run. -->
