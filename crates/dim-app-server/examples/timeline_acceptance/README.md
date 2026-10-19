# Timeline SQLite acceptance

This opt-in program uses public TimelineStore/Agent/Session interfaces and a test SQLite VFS. Each run creates an independent database. Set `TIMELINE_ACCEPTANCE_ROOT` to a dedicated test directory; the default is `/mnt/data/timeline-acceptance`.

Build from the repository root, then run one workload at a time:

```sh
cargo build --locked --release -p dim-app-server --example timeline_acceptance
TIMELINE_ACCEPTANCE_ROOT=/absolute/test-directory \
  ./target/release/examples/timeline_acceptance rounds 11
```

| Command | Workload |
| --- | --- |
| `memory 2000` | Long Agent history with context preparation/compaction and allocation measurement |
| `reads` | Cold history/context reads and interrupt delivery during reads |
| `paged-reads` | Paged recovery while appending events |
| `rounds 11` / `rounds 64` | Concurrent session writers, WAL bytes/syncs and publication latency |
| `crashes` | Process termination at transaction/publication barriers and continuity after recovery |
| `powerloss` | Restore only successfully synced main/WAL images and verify recovery continuity |

In `crashes` and `powerloss` the interrupted batch is an input and the request prepared from it. Recovery validates the Session, including every request list's nodes, reads the recovered events back, and writes a request extending it; the Session's rows must then equal those of the same events written without interruption.

The power-loss model assumes fsync reports durable writes honestly; it does not model torn sectors or macOS fullfsync. Rust allocation measurements exclude SQLite allocations and are not RSS. Routine storage behavior remains protected by the normal flow tests; these workloads are optional resource measurements.
