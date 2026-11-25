# LoopMethods

Loop's Session part registers nonmutating `session.turn.get`.
The [read contract](../../../design/input-state-read.md#turn-identity-read) defines its fields.
It declares TreeCommands to query the selected branch and SessionMethods to register the entry.
It owns no persistent state, background worker, events or services. Reads end with their request.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Query the requested branch at its published cutoff. |
| `S:SessionMethods` | Register the read entry. |
