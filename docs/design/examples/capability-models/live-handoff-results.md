# 独立重启与补通知失败：验证记录

[合同、动作、实现对应与边界](../owner-handoff-verification.md#8-上游仍运行的-modelprovider-重启与-backgroundexec-补通知失败)。
2026-11-17 的 [JSON](live-handoff-results.json)记录父提交、实际输入文件哈希、工具版本、
运行参数及可达状态；它是本次实验快照，不代表后续 main。运行时行为未改。

| 配置 | generated | distinct | queue | depth | exit |
| --- | ---: | ---: | ---: | ---: | ---: |
| ModelReloadHandoff | 21 | 17 | 0 | 8 | 0 |
| ModelReloadHandoff-Failure | 57 | 41 | 0 | 11 | 0 |
| ExecWakeRecovery | 79 | 55 | 0 | 11 | 0 |

三次完整 BFS，无反例、无状态约束截断。JSON 的可达状态覆盖恢复结算且零调用、
实时执行、查询失败、失败后再次恢复，以及通知故障后补齐两项持久通知。
状态数小源于明确的有限边界，不以数量代替覆盖论证。活性依赖文档列明的成功重试和弱公平性；
不是无界证明，不保证永久失败下完成。

仓库根复现（JAVA/TLA_JAR 按 [README](README.md#重跑) 设置；jar 哈希见 JSON）：

```sh
run_dir=$(mktemp -d /tmp/live-handoff.XXXXXX)
for config in ModelReloadHandoff ModelReloadHandoff-Failure ExecWakeRecovery; do
  "$JAVA" -XX:+UseParallelGC -Xmx2g -cp "$TLA_JAR" tlc2.TLC \
    -workers 1 -metadir "$run_dir/$config" -dump "$run_dir/$config.dump" \
    -config "docs/design/examples/capability-models/$config.cfg" \
    "docs/design/examples/capability-models/${config%%-*}.tla"
  result=$?
  test "$result" -eq 0 || exit "$result"
done
```

复现同一指纹选择时增加 JSON 对应的 `-fp` / `-seed`；状态编号仅适用于记录那次运行。

以下四个真实流程各运行一次，各 1 passed / 0 failed。两个新增流程内分别包含三个交接窗口、
同步和异步两种存储故障；不是分别跑了五个测试。context_flow 使用名称过滤（完整名称带 `context_reads::`）。

```sh
cargo test --locked -p dim-agent-capabilities --test context_flow model_only_reload_hands_off_a_running_preparation_at_its_recovery_cut
cargo test --locked -p dim-agent-capabilities --test context_flow model_reload_recovers_the_old_terminal_before_its_publication
cargo test --locked -p dim-agent-integration-tests --test local_execution a_failed_recovery_wake_faults_the_session_and_is_posted_once_on_retry
cargo test --locked -p dim-agent-integration-tests --test local_execution idle_unload_waits_for_tasks_and_a_restart_wakes_for_every_unreported_end
cargo clippy --locked -p dim-agent-capabilities --test context_flow -p dim-agent-integration-tests --test local_execution -- -D warnings
```

上述 scoped Clippy 通过。使用独立 worktree target、隔离 DIMCODE_HOME、4 build jobs、
CARGO_INCREMENTAL=0；没有重跑全部工作区，也没有改存储、恢复或重试策略。
