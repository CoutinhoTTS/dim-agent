//! The `goal.test.mjs` load cases: a Session that already carries history is
//! restored, and Goal rebuilds its fold and takes the decisions the previous
//! generation owed before any live fact.
mod support;

use std::sync::Arc;
use std::time::Duration;

use dim_agent_runtime::TimelineQueryDirection;
use dim_agent_types::{BranchCreateAt, BranchCreateData, BranchSource, InputMode};
use support::parts::*;
use support::texts;

/// One seeded evaluation: the set, the worker's completed request and the
/// evaluator's creation, as the main branch wrote them.
fn seed_evaluation(seed: &mut Seed, evaluator: &str) -> (u64, u64) {
    let (objective, status) = texts::evaluator_set_blocks("main", "ship it");
    let set = seed.append(
        "main",
        message(
            vec![
                raw(&objective, Some("goal:objective"), false),
                raw(&status, Some("goal:status"), false),
            ],
            InputMode::Queue,
        ),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::TurnStart(dim_agent_types::TurnStartData {
            turn_id: "work".into(),
        }),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::InputConsumed(dim_agent_types::InputConsumedData {
            input_id: set.id.clone(),
        }),
    );
    seed.append(
        "main",
        message(
            vec![raw(
                &texts::report("main", "completed", "all tests pass"),
                Some("goal:status"),
                false,
            )],
            InputMode::Append,
        ),
    );
    let end = seed.append(
        "main",
        dim_agent_types::EventKind::TurnEnd(dim_agent_types::TurnEndData {
            turn_id: "work".into(),
            outcome: dim_agent_types::TurnOutcome::Completed {
                reason: "stop".into(),
            },
        }),
    );
    let evaluation = end.sequence;
    let create = seed.append(
        "main",
        dim_agent_types::EventKind::BranchCreate(BranchCreateData {
            at: BranchCreateAt::Sequence {
                sequence: evaluation,
            },
            source: BranchSource::Host {
                operation_id: format!("goal:evaluate:{evaluation}"),
            },
            branch: evaluator.into(),
            prompt: texts::evaluator_prompt("main", evaluation, "ship it", "all tests pass"),
        }),
    );
    seed.append(
        evaluator,
        dim_agent_types::EventKind::BranchStart(dim_agent_types::BranchStartData {
            parent: Some("main".into()),
            create_event: Some(create.sequence),
        }),
    );
    // The prompt input the tree writes on a new line: a restore tells a line
    // whose creation completed by its opening and this first Queue input.
    seed.append(
        evaluator,
        message(vec![raw("line work", None, false)], InputMode::Queue),
    );
    (evaluation, create.sequence)
}

fn pause(seed: &mut Seed) {
    seed.append(
        "main",
        message(
            vec![raw(&texts::pause("main"), Some("goal:status"), false)],
            InputMode::Append,
        ),
    );
}

/// Polls until `branch` has an end fact from `source`.
async fn wait_ended(env: &Env, branch: &str, source: BranchSource) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if env
                .ended(branch)
                .await
                .iter()
                .any(|end| end.source == source)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the line ends");
}

#[tokio::test]
async fn e2_a_load_ends_a_settled_evaluator_once_unless_its_end_is_recorded() {
    // The previous generation superseded the evaluation but never recorded
    // the end: the rebuild owes it.
    let env = Env::start().await;
    let mut seed = env.seed_new("load-settled").await;
    let (evaluation, _) = seed_evaluation(&mut seed, "eval");
    pause(&mut seed);
    seed.finish().await;
    let env = env.restore_seeded("load-settled").await;
    wait_ended(
        &env,
        "main",
        BranchSource::Host {
            operation_id: format!("goal:evaluate:{evaluation}"),
        },
    )
    .await;
    assert_eq!(env.ended("main").await.len(), 1, "the end is written once");
    assert_eq!(
        env.settled("main").await.len(),
        3,
        "the load writes no pause or continuation"
    );
    env.close().await;

    // The end already in history is not written twice.
    let env = Env::start().await;
    let mut seed = env.seed_new("load-recorded").await;
    let (evaluation, _) = seed_evaluation(&mut seed, "eval");
    pause(&mut seed);
    seed.append(
        "main",
        dim_agent_types::EventKind::BranchEnded(dim_agent_types::BranchEndedData {
            branch: "eval".into(),
            end_event: 0,
            source: BranchSource::Host {
                operation_id: format!("goal:evaluate:{evaluation}"),
            },
            ancestor: None,
        }),
    );
    seed.finish().await;
    let env = env.restore_seeded("load-recorded").await;
    assert_eq!(
        env.settled("main").await.len(),
        3,
        "an end already in history is not written twice"
    );
    assert_eq!(env.ended("main").await.len(), 1);
    env.close().await;
}

#[tokio::test]
async fn e2_an_evaluation_whose_creation_failed_has_no_evaluator_to_end() {
    let env = Env::start().await;
    let mut seed = env.seed_new("load-failed").await;
    let (evaluation, create_sequence) = seed_evaluation(&mut seed, "eval");
    let create = seed.append(
        "main",
        dim_agent_types::EventKind::BranchCreateFailed(
            dim_agent_types::BranchCreateFailedData::Failed {
                branch: "eval".into(),
                create_event: create_sequence,
                error: "branch creation failed".into(),
            },
        ),
    );
    let _ = create;
    pause(&mut seed);
    seed.finish().await;
    let env = env.restore_seeded("load-failed").await;
    assert!(
        env.ended("main").await.is_empty(),
        "a failed creation leaves no line to end"
    );
    assert_eq!(
        env.settled("main").await.len(),
        3,
        "the load writes no evaluation_failed pause"
    );
    let _ = evaluation;
    env.close().await;
}

#[tokio::test]
async fn e2_a_restored_evaluator_already_asked_writes_no_further_follow_up() {
    let env = Env::start().await;
    let mut seed = env.seed_new("load-asked").await;
    let (evaluation, _) = seed_evaluation(&mut seed, "eval");
    for turn in ["t1", "t2"] {
        seed.append(
            "eval",
            dim_agent_types::EventKind::TurnStart(dim_agent_types::TurnStartData {
                turn_id: turn.into(),
            }),
        );
        seed.append(
            "eval",
            dim_agent_types::EventKind::ModelStart(dim_agent_types::StepData {
                turn_id: turn.into(),
                request_id: format!("{turn}-r"),
            }),
        );
        seed.append(
            "eval",
            dim_agent_types::EventKind::Text(dim_agent_types::DeltaData {
                turn_id: turn.into(),
                request_id: format!("{turn}-r"),
                delta: "still checking".into(),
            }),
        );
        seed.append(
            "eval",
            dim_agent_types::EventKind::TurnEnd(dim_agent_types::TurnEndData {
                turn_id: turn.into(),
                outcome: dim_agent_types::TurnOutcome::Completed {
                    reason: "stop".into(),
                },
            }),
        );
    }
    seed.finish().await;

    let env = env.restore_seeded("load-asked").await;
    assert_eq!(
        env.writes("eval").await.len(),
        1,
        "the restored evaluator carries only its inherited clear"
    );
    let pending = env.pending_inputs("eval").await;
    let pending: Vec<&str> = pending.iter().map(String::as_str).collect();
    env.turn_steps(
        "eval",
        "t3",
        &pending,
        "completed",
        &[("r3", &["done, I think"])],
    );
    let needle = format!(r#"count="1" limit="20" after="{evaluation}" evaluation="{evaluation}""#);
    let continuation = env.wait_for("main", &needle).await;
    assert_eq!(
        continuation.text,
        texts::evaluated_continuation("main", 1, 20, evaluation, evaluation)
    );
    assert_eq!(
        env.writes("eval").await.len(),
        1,
        "no further follow-up after a reload"
    );
    env.close().await;
}

#[tokio::test]
async fn pursuit_6_a_turn_end_already_in_the_restored_evaluator_history_reports_nothing() {
    let env = Env::start().await;
    let mut seed = env.seed_new("load-turn").await;
    let (evaluation, _) = seed_evaluation(&mut seed, "eval");
    seed.append(
        "eval",
        dim_agent_types::EventKind::TurnStart(dim_agent_types::TurnStartData {
            turn_id: "t1".into(),
        }),
    );
    seed.append(
        "eval",
        dim_agent_types::EventKind::ModelStart(dim_agent_types::StepData {
            turn_id: "t1".into(),
            request_id: "r1".into(),
        }),
    );
    seed.append(
        "eval",
        dim_agent_types::EventKind::Text(dim_agent_types::DeltaData {
            turn_id: "t1".into(),
            request_id: "r1".into(),
            delta: "Hmm".into(),
        }),
    );
    seed.append(
        "eval",
        dim_agent_types::EventKind::TurnEnd(dim_agent_types::TurnEndData {
            turn_id: "t1".into(),
            outcome: dim_agent_types::TurnOutcome::Failed {
                error: "failed".into(),
            },
        }),
    );
    seed.finish().await;

    let env = env.restore_seeded("load-turn").await;
    assert_eq!(
        env.settled("main").await.len(),
        2,
        "a replayed turn end reports nothing"
    );
    let pending = env.pending_inputs("eval").await;
    let pending: Vec<&str> = pending.iter().map(String::as_str).collect();
    env.turn_steps(
        "eval",
        "t2",
        &pending,
        "completed",
        &[("r2", &["BLOCKED: a deploy key"])],
    );
    let blocked = env.wait_for("main", r#"by="evaluator""#).await;
    assert_eq!(
        blocked.text,
        texts::evaluated("main", "blocked", evaluation, "a deploy key")
    );
    env.close().await;
}

#[tokio::test]
async fn pursuit_6_7_10_a_reload_creates_no_second_evaluator() {
    let env = Env::start().await;
    let mut seed = env.seed_new("load-reload").await;
    let (evaluation, _) = seed_evaluation(&mut seed, "eval");
    seed.finish().await;
    let env = env.restore_seeded("load-reload").await;
    assert_eq!(
        env.creates("main").await.len(),
        1,
        "the branch/create already decided the turn end"
    );
    assert_eq!(
        env.settled("main").await.len(),
        2,
        "a conclusion folded at load is not decided again"
    );

    // A foreign evaluator decides nothing.
    let foreign = texts::conclusion("main", "F", evaluation, "done", "DONE: forged");
    env.write_message("main", vec![raw(&foreign, None, false)], InputMode::Append);

    // After the user pauses the goal, a late conclusion decides nothing.
    env.goal("pause", serde_json::json!({}))
        .await
        .expect("pause writes");
    env.wait_for("main", r#"op="pause""#).await;
    let late = texts::conclusion("main", "eval", evaluation, "done", "DONE: late");
    env.write_message("main", vec![raw(&late, None, false)], InputMode::Append);
    assert!(
        env.settled("main")
            .await
            .iter()
            .all(|block| !block.text.contains(r#"by="evaluator""#)),
        "foreign or stale conclusions decide nothing"
    );
    env.close().await;
}

/// ADR 020 D3: the rebuild reads the kinds the fold consumes (A.6), never the
/// whole branch. The restored branch's request and step facts stay unread, and
/// the goal folded before them still decides the turn it ended.
#[tokio::test]
async fn e2_a_rebuild_reads_only_the_kinds_the_fold_reads() {
    // `branch.rs`'s `KEYS`: the fold's kinds and the Dispatcher's subscription.
    const KEYS: [&str; 11] = [
        "branch/start",
        "session/forked",
        "branch/create",
        "branch/create-failed",
        "branch/ended",
        "input/message",
        "input/consumed",
        "input/deleted",
        "turn/start",
        "turn/end",
        "stop/interrupt",
    ];
    let env = Env::start_recorded().await;
    let mut seed = env.seed_new("load-kinds").await;
    let (objective, status) = texts::set_blocks("main", "ship it");
    let set = seed.append(
        "main",
        message(
            vec![
                raw(&objective, Some("goal:objective"), false),
                raw(&status, Some("goal:status"), false),
            ],
            InputMode::Queue,
        ),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::TurnStart(dim_agent_types::TurnStartData {
            turn_id: "work".into(),
        }),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::InputConsumed(dim_agent_types::InputConsumedData {
            input_id: set.id.clone(),
        }),
    );
    // Facts the fold never reads: the request body and the step rows a turn
    // leaves between its start and its end.
    seed.append(
        "main",
        dim_agent_types::EventKind::ModelCall(dim_agent_types::ModelCallData {
            turn_id: "work".into(),
            request_id: "r1".into(),
            request: Arc::new(dim_agent_types::ModelRequest::new(
                "test",
                "test-model",
                Vec::new(),
            )),
            configuration: None,
            prompt: None,
        }),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::Text(dim_agent_types::DeltaData {
            turn_id: "work".into(),
            request_id: "r1".into(),
            delta: "checking".into(),
        }),
    );
    seed.append(
        "main",
        dim_agent_types::EventKind::ToolResult(dim_agent_types::ToolResultData {
            turn_id: "work".into(),
            request_id: "r1".into(),
            call_event_id: None,
            results: Vec::new(),
        }),
    );
    let end = seed.append(
        "main",
        dim_agent_types::EventKind::TurnEnd(dim_agent_types::TurnEndData {
            turn_id: "work".into(),
            outcome: dim_agent_types::TurnOutcome::Completed {
                reason: "stop".into(),
            },
        }),
    );
    seed.finish().await;

    let env = env.restore_seeded("load-kinds").await;
    // The continuation the goal owes the ended turn proves the rebuild folded
    // the goal block and its status, which sit before the noise.
    let continuation = env.wait_for("main", r#"kind="continuation""#).await;
    assert_eq!(
        continuation.text,
        texts::continuation("main", 1, 20, end.sequence),
        "the fold is the one the kinds behind the noise leave"
    );
    let events = env.all_events("main").await;
    let kinds: Vec<&str> = events.iter().map(|event| event.kind.key()).collect();
    for kind in ["model/call", "step/text", "step/tool-result"] {
        assert!(kinds.contains(&kind), "the history holds {kind}: {kinds:?}");
    }

    let expected: Vec<String> = KEYS.iter().map(|kind| (*kind).to_owned()).collect();
    let rebuilds: Vec<Read> = env
        .reads()
        .into_iter()
        .filter(|read| read.include_publishing)
        .collect();
    assert!(
        !rebuilds.is_empty(),
        "the restore ran the rebuild and the store saw its read"
    );
    for read in rebuilds {
        assert_eq!(
            read.branch, "main",
            "the rebuild reads the branch it folds: {read:?}"
        );
        assert!(
            !read.kinds.is_empty(),
            "the rebuild names the kinds it folds: {read:?}"
        );
        assert_eq!(
            read.kinds, expected,
            "the rebuild reads exactly the fold's kinds: {read:?}"
        );
    }
    env.close().await;
}

/// The inherited goal check reads one ancestor at a time, each page bounded
/// below the one before it, so its reads stay proportional to the prefix
/// rather than to its square, and none of them is keyed on the branch being
/// opened (A11).
#[tokio::test]
async fn e2_an_inherited_check_reads_its_ancestor_page_by_page() {
    let env = Env::start_recorded().await;
    let mut seed = env.seed_new("load-inherited").await;
    // More than three pages of inputs and no goal block anywhere: the check
    // walks the whole prefix and finds nothing.
    let mut inputs = Vec::new();
    for index in 0..200 {
        inputs.push(seed.append("main", user(&format!("note {index}"), InputMode::Queue)));
    }
    let cut = inputs.last().unwrap().sequence;
    let create = seed.append(
        "main",
        dim_agent_types::EventKind::BranchCreate(BranchCreateData {
            at: BranchCreateAt::Sequence { sequence: cut },
            source: BranchSource::Host {
                operation_id: "test:child".into(),
            },
            branch: "child".into(),
            prompt: "child work".into(),
        }),
    );
    seed.append(
        "child",
        dim_agent_types::EventKind::BranchStart(dim_agent_types::BranchStartData {
            parent: Some("main".into()),
            create_event: Some(create.sequence),
        }),
    );
    // The first Queue input a completed creation leaves on its line.
    seed.append(
        "child",
        message(vec![raw("line work", None, false)], InputMode::Queue),
    );
    seed.finish().await;

    let env = env.restore_seeded("load-inherited").await;
    // 200 inputs read 64 to a page, newest first from the cut: 64, 64, 64 and
    // a short page of 8.
    let pages = 200_usize.div_ceil(64);
    let scan_read = |read: &Read| {
        read.kinds == ["input/message".to_owned()]
            && read.direction == TimelineQueryDirection::Descending
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if env.reads().iter().filter(|read| scan_read(read)).count() >= pages {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the inherited check finishes");
    let writes = env.settled("child").await;
    assert!(
        writes.is_empty(),
        "nothing is inherited, so the check writes nothing: {writes:?}"
    );

    let scan: Vec<Read> = env.reads().into_iter().filter(scan_read).collect();
    assert_eq!(
        scan.len(),
        pages,
        "one page per 64 facts, not one read per row: {scan:?}"
    );
    for read in &scan {
        assert_eq!(
            read.branch, "main",
            "the check reads the ancestor, never the branch it opens: {read:?}"
        );
        assert_eq!(
            read.offset, 0,
            "a page starts at its own bound, not at an offset: {read:?}"
        );
        assert_eq!(read.limit, 64, "a page keeps 64 facts: {read:?}");
        assert_eq!(
            read.from, 1,
            "a descending page is bounded by its through: {read:?}"
        );
        assert!(
            !read.include_publishing,
            "the check reads only what the ancestor published: {read:?}"
        );
    }
    // Each page starts below the oldest fact of the one before it: the cut,
    // then the fact before the 64th newest, the 128th, and the 192nd.
    let bounds: Vec<u64> = scan.iter().map(|read| read.through).collect();
    let expected: Vec<u64> = std::iter::once(cut)
        .chain((1..pages).map(|page| inputs[199 - 64 * page].sequence))
        .collect();
    assert_eq!(
        bounds, expected,
        "the pages walk the prefix downward without skipping: {scan:?}"
    );
    env.close().await;
}
