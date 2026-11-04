//! `session.keys`: the live keyed blocks of a branch segment (keyed-blocks
//! §2 and §3), read over the v3 wire.
use super::*;
use dim_agent_capabilities::context_block;
use dim_agent_capabilities::input_queue::{InputQueue, Withdrawal};
use dim_agent_runtime::testing::branch_events;
use dim_agent_types::{InputBlock, InputMessageData, InputMode, InputWithdrawData};

const OBJECTIVE: &str = "goal:objective";
const STATUS: &str = "goal:status";
const OBJECTIVE_TEXT: &str = "<dim-goal kind=\"goal\" op=\"set\">ship it</dim-goal>";
const STATUS_TEXT: &str = "<dim-goal kind=\"status\" status=\"active\"></dim-goal>";

fn goal(text: &str, key: &str) -> InputBlock {
    context_block::raw(text.into(), Some(key.into()))
}

fn retired(text: &str, key: &str) -> InputBlock {
    context_block::retire(text.into(), key.into())
}

fn message(blocks: Vec<InputBlock>, text: &str) -> InputMessageData {
    InputMessageData {
        blocks,
        ..InputMessageData::text(text, Some(InputMode::Queue))
    }
}

async fn create_session(client: &mut Client, tools: Value) -> String {
    let created = client.rpc("session.create", create(tools)).await;
    assert_ok(&created);
    created["result"]["sessionId"].as_str().unwrap().to_owned()
}

async fn read_keys(client: &mut Client, session: &str, branch: &str, extra: Value) -> Value {
    let mut params = json!({"sessionId":session,"branchId":branch});
    if let Value::Object(fields) = extra {
        params.as_object_mut().unwrap().extend(fields);
    }
    let reply = client.rpc("session.keys", params).await;
    assert_ok(&reply);
    reply["result"].clone()
}

fn entries(result: &Value) -> Vec<Value> {
    result["entries"].as_array().unwrap().clone()
}

fn keys(result: &Value) -> Vec<String> {
    entries(result)
        .iter()
        .map(|entry| entry["key"].as_str().unwrap().to_owned())
        .collect()
}

fn entry(key: &str, event: &dim_agent_types::AgentEvent, text: &str) -> Value {
    json!({"key":key,"sequence":event.sequence,"inputId":event.id,"text":text})
}

/// A branch with two keys set in one input, then one retired, then one set
/// again: the reply is the newest live block per key, ascending by the
/// carrying input's sequence and by block order inside one input.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_reply_the_newest_live_block_of_each_key_in_sequence_order() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert!(
        client
            .features
            .as_array()
            .unwrap()
            .contains(&json!("session.keys"))
    );
    let id = create_session(&mut client, json!([])).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();

    assert_eq!(
        read_keys(&mut client, &id, "main", json!({"through":0})).await,
        json!({"through":0,"entries":[]}),
        "a read fixed at position zero is the empty projection"
    );
    assert_eq!(
        read_keys(&mut client, &id, "main", json!({})).await["entries"],
        json!([]),
        "a branch with no keyed block lists no keys"
    );

    let first = session
        .input(
            message(
                vec![
                    goal(OBJECTIVE_TEXT, OBJECTIVE),
                    goal(STATUS_TEXT, STATUS),
                    context_block::raw("no key here".into(), None),
                ],
                "start",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(reply["through"], json!(session.sequence("main").unwrap()));
    assert_eq!(
        entries(&reply),
        [
            entry(OBJECTIVE, &first, OBJECTIVE_TEXT),
            entry(STATUS, &first, STATUS_TEXT),
        ],
        "one input's blocks keep block order"
    );

    let second = session
        .input(
            message(
                vec![retired(
                    "<dim-goal kind=\"goal\" op=\"clear\">shipped</dim-goal>",
                    OBJECTIVE,
                )],
                "clear",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(
        entries(&reply),
        [entry(STATUS, &first, STATUS_TEXT)],
        "a retire block produces no entry and ends the key"
    );
    assert!(
        second.sequence > first.sequence,
        "the retire is a later input"
    );

    let third = session
        .input(
            message(
                vec![goal(
                    "<dim-goal kind=\"goal\" op=\"set\">again</dim-goal>",
                    OBJECTIVE,
                )],
                "again",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(
        entries(&reply),
        [
            entry(STATUS, &first, STATUS_TEXT),
            entry(
                OBJECTIVE,
                &third,
                "<dim-goal kind=\"goal\" op=\"set\">again</dim-goal>"
            ),
        ],
        "entries ascend by the carrying input's sequence, not by key"
    );
}

/// `input/deleted` on the input that wrote a key: its blocks never take
/// effect, so the key falls back to the newest block that is still live
/// (here the older objective) and a key with no earlier live block is absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deleted_input_leaves_the_key_at_its_earlier_live_block_or_absent() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = create_session(&mut client, json!(["wait"])).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();

    let first = session
        .input(
            message(
                vec![
                    goal(
                        "<dim-goal kind=\"goal\" op=\"set\">v1</dim-goal>",
                        OBJECTIVE,
                    ),
                    goal(STATUS_TEXT, STATUS),
                ],
                "v1",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();

    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","text":"tool:wait {\"millis\":60000}"}),
            )
            .await,
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let events = branch_events(&session, "main", ..).await.unwrap();
            if events
                .iter()
                .any(|event| matches!(&event.kind, EventKind::ToolUse(_) | EventKind::ToolCall(_)))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the wait tool call opens the turn");
    let held = session
        .input(
            message(
                vec![
                    goal(
                        "<dim-goal kind=\"goal\" op=\"set\">v2</dim-goal>",
                        OBJECTIVE,
                    ),
                    goal(
                        "<dim-goal kind=\"goal\" op=\"set\">new</dim-goal>",
                        "test:new",
                    ),
                ],
                "v2",
            ),
            "main",
        )
        .await
        .unwrap();
    let queue = session.service::<InputQueue>("main").unwrap();
    let (answer, _) = queue
        .withdraw(
            InputWithdrawData {
                input_id: held.id.clone(),
                reason: Some("withdrawn while held".into()),
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(answer, Withdrawal::Deleted { .. }),
        "the turn holds the input unapplied: {answer:?}"
    );
    session.stop("main", "end the wait".into()).await.unwrap();
    session.idle("main").await.unwrap();
    assert!(
        branch_events(&session, "main", ..)
            .await
            .unwrap()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::InputDeleted(data) if data.input_id == held.id)),
        "the deletion is a Timeline fact"
    );

    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(
        entries(&reply),
        [
            entry(
                OBJECTIVE,
                &first,
                "<dim-goal kind=\"goal\" op=\"set\">v1</dim-goal>"
            ),
            entry(STATUS, &first, STATUS_TEXT),
        ],
        "the deleted input's objective never takes effect and its new key never appears"
    );
}

/// A fork's `main` and a child branch both exclude the parent's keys: the
/// scan stops at `session/forked` and at `branch/start`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fork_and_a_child_branch_carry_no_parent_keys() {
    let store = super::deletion::Store::new();
    let (server, agent) = launch_with_agent(Arc::new(MockTools), store.config()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut params = create(json!(["branch"]));
    params["interactive"] = json!(false);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = agent.get_session(&id).unwrap();
    let first = session
        .input(
            message(vec![goal(OBJECTIVE_TEXT, OBJECTIVE)], "parent"),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let at = session.sequence("main").unwrap();
    assert_eq!(
        entries(&read_keys(&mut client, &id, "main", json!({})).await),
        [entry(OBJECTIVE, &first, OBJECTIVE_TEXT)]
    );

    let child = client
        .rpc(
            "session.branch_create",
            json!({"sessionId":id,"parentBranchId":"main","at":at,"prompt":"child"}),
        )
        .await;
    assert_ok(&child);
    let child_id = child["result"]["branch"]["branchId"]
        .as_str()
        .unwrap()
        .to_owned();
    session.idle(&child_id).await.unwrap();
    let reply = read_keys(&mut client, &id, &child_id, json!({})).await;
    assert_eq!(
        reply["through"],
        json!(session.sequence(&child_id).unwrap())
    );
    assert_eq!(
        keys(&reply),
        ["branch:line"],
        "the child's own opening input carries only TimelineTree's line key"
    );
    let child_own = session
        .input(
            message(vec![goal(STATUS_TEXT, STATUS)], "child work"),
            &child_id,
        )
        .await
        .unwrap();
    session.idle(&child_id).await.unwrap();
    let child_keys = read_keys(&mut client, &id, &child_id, json!({})).await;
    assert_eq!(keys(&child_keys), ["branch:line", STATUS]);
    assert_eq!(
        entries(&child_keys).last(),
        Some(&entry(STATUS, &child_own, STATUS_TEXT))
    );
    assert_eq!(
        keys(&read_keys(&mut client, &id, "main", json!({})).await),
        [OBJECTIVE],
        "the parent keeps its own segment"
    );

    let forked = client
        .rpc("session.fork", json!({"sessionId":id,"at":at}))
        .await;
    assert_ok(&forked);
    let fork_id = forked["result"]["sessionId"].as_str().unwrap().to_owned();
    let fork = agent.get_session(&fork_id).unwrap();
    let reply = read_keys(&mut client, &fork_id, "main", json!({})).await;
    assert_eq!(reply["entries"], json!([]));
    assert_eq!(reply["through"], json!(fork.sequence("main").unwrap()));
    assert!(
        branch_events(&fork, "main", ..)
            .await
            .unwrap()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::SessionForked(_))),
        "the fork's copied facts sit below its opening"
    );

    let own = fork
        .input(
            message(vec![goal(STATUS_TEXT, STATUS)], "fork work"),
            "main",
        )
        .await
        .unwrap();
    fork.idle("main").await.unwrap();
    assert_eq!(
        entries(&read_keys(&mut client, &fork_id, "main", json!({})).await),
        [entry(STATUS, &own, STATUS_TEXT)],
        "the fork's own segment opens at session/forked"
    );
}

/// `through` pins the projection: a read before a later change returns the
/// older value, and a position past the head is invalid input, never a clamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn through_pins_the_projection_and_a_future_position_is_invalid() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = create_session(&mut client, json!([])).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();

    let first = session
        .input(
            message(
                vec![goal(
                    "<dim-goal kind=\"goal\" op=\"set\">v1</dim-goal>",
                    OBJECTIVE,
                )],
                "v1",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let pinned = session.sequence("main").unwrap();
    let second = session
        .input(
            message(
                vec![goal(
                    "<dim-goal kind=\"goal\" op=\"set\">v2</dim-goal>",
                    OBJECTIVE,
                )],
                "v2",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let head = session.sequence("main").unwrap();
    assert!(head > pinned);

    let reply = read_keys(&mut client, &id, "main", json!({"through":pinned})).await;
    assert_eq!(reply["through"], json!(pinned));
    assert_eq!(
        entries(&reply),
        [entry(
            OBJECTIVE,
            &first,
            "<dim-goal kind=\"goal\" op=\"set\">v1</dim-goal>"
        )],
        "the pinned read is the projection fixed before the later input"
    );

    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(
        entries(&reply),
        [entry(
            OBJECTIVE,
            &second,
            "<dim-goal kind=\"goal\" op=\"set\">v2</dim-goal>"
        )]
    );

    let future = client
        .rpc(
            "session.keys",
            json!({"sessionId":id,"branchId":"main","through":head + 1}),
        )
        .await;
    assert_eq!(
        future["error"]["data"]["appError"], "invalid_params",
        "{future}"
    );
    assert_eq!(future["error"]["message"], "Future keyed-block position");
}

/// The read is v3 only, like `session.history`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_v2_connection_cannot_read_keys() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = create_session(&mut client, json!([])).await;
    let mut v2 = Client::new_version(&server, false, 2).await;
    let reply = v2
        .rpc("session.keys", json!({"sessionId":id,"branchId":"main"}))
        .await;
    assert_eq!(reply["error"]["data"]["appError"], "unknown_method");
}

/// An ended line is served from the tree at its fixed head: the same entries
/// the open line folded, now read through `WindowRead::Ended`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_line_is_served_at_its_fixed_head() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut params = create(json!(["branch"]));
    params["interactive"] = json!(false);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = server.agent().unwrap().get_session(&id).unwrap();
    session
        .input(
            InputMessageData::text("hello", Some(InputMode::Queue)),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let at = session.sequence("main").unwrap();
    let child = client
        .rpc(
            "session.branch_create",
            json!({"sessionId":id,"parentBranchId":"main","at":at,"prompt":"child"}),
        )
        .await;
    assert_ok(&child);
    let child_id = child["result"]["branch"]["branchId"]
        .as_str()
        .unwrap()
        .to_owned();
    session.idle(&child_id).await.unwrap();
    let keyed = session
        .input(
            message(
                vec![goal(OBJECTIVE_TEXT, OBJECTIVE), goal(STATUS_TEXT, STATUS)],
                "work",
            ),
            &child_id,
        )
        .await
        .unwrap();
    session.idle(&child_id).await.unwrap();
    let open = read_keys(&mut client, &id, &child_id, json!({})).await;

    assert_ok(
        &client
            .rpc(
                "session.branch_end",
                json!({"sessionId":id,"branchId":child_id}),
            )
            .await,
    );
    let head = session.sequence(&child_id).unwrap();
    assert_eq!(
        session.ended_head(&child_id),
        Some(head),
        "the ended line reads at its fixed head"
    );
    let reply = read_keys(&mut client, &id, &child_id, json!({})).await;
    assert_eq!(reply["through"], json!(head));
    assert_eq!(
        reply["entries"], open["entries"],
        "ending appends no keyed block, so the ended read is the open read"
    );
    assert_eq!(
        entries(&reply)
            .iter()
            .filter(|entry| entry["key"] != json!("branch:line"))
            .cloned()
            .collect::<Vec<_>>(),
        [
            entry(OBJECTIVE, &keyed, OBJECTIVE_TEXT),
            entry(STATUS, &keyed, STATUS_TEXT),
        ]
    );
}

/// Every key retired answers like an empty branch, and a branch above the
/// live-key cap fails `too_many_keys` instead of truncating live state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retired_branches_are_empty_and_above_the_cap_is_too_many_keys() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = create_session(&mut client, json!([])).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();
    session
        .input(
            message(
                vec![goal(OBJECTIVE_TEXT, OBJECTIVE), goal(STATUS_TEXT, STATUS)],
                "start",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    session
        .input(
            message(
                vec![
                    retired(
                        "<dim-goal kind=\"goal\" op=\"clear\">done</dim-goal>",
                        OBJECTIVE,
                    ),
                    retired("<dim-goal kind=\"status\">none</dim-goal>", STATUS),
                ],
                "clear",
            ),
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(reply["entries"], json!([]));
    assert_ne!(reply["through"], json!(0), "the branch is not empty");

    let blocks: Vec<InputBlock> = (0..1025)
        .map(|index| goal(&format!("value {index}"), &format!("test:key/{index}")))
        .collect();
    session
        .input(message(blocks, "many keys"), "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let reply = client
        .rpc("session.keys", json!({"sessionId":id,"branchId":"main"}))
        .await;
    assert_eq!(
        reply["error"]["data"]["appError"], "too_many_keys",
        "{reply}"
    );
}

/// The read equals the model baseline at a quiescent point, by key: the same
/// key, the same source input id and the same text bytes. This is the reason
/// the projection is trustworthy: what a client folds from the branch's live
/// keyed blocks is what the model is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_read_equals_the_model_baseline_at_a_quiescent_point() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let id = create_session(&mut client, json!([])).await;
    let session = server.agent().unwrap().get_session(&id).unwrap();

    // The keyed sequence of checkpoint_projection's replay test: set two
    // keys, retire one, then replace the other. Every input is consumed.
    let inputs = [
        (
            vec![goal("<live>a</live>", "a"), goal("<live>b</live>", "b")],
            "x".repeat(500),
        ),
        (vec![retired("<ended>b</ended>", "b")], "y".repeat(500)),
        (vec![goal("<live>a2</live>", "a")], "z".repeat(500)),
    ];
    for (blocks, text) in inputs {
        session.input(message(blocks, &text), "main").await.unwrap();
        session.idle("main").await.unwrap();
    }

    let events = branch_events(&session, "main", ..).await.unwrap();
    let keyed: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputMessage(data)
                if data
                    .blocks
                    .iter()
                    .any(|block| matches!(block, InputBlock::Raw { key: Some(_), .. })) =>
            {
                Some(event.id.clone())
            }
            _ => None,
        })
        .collect();
    let consumed: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputConsumed(data) => Some(data.input_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(keyed.len(), 3);
    for id in &keyed {
        assert!(
            consumed.contains(id),
            "every written keyed input is consumed: {id} is not"
        );
    }
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.kind, EventKind::InputDeleted(_))),
        "a quiescent point holds no withdrawal"
    );

    let history = dim_agent_capabilities::history::project_history(&events, "main").unwrap();
    let baseline = history.last().unwrap().baseline.clone();
    let reply = read_keys(&mut client, &id, "main", json!({})).await;
    assert_eq!(reply["through"], json!(session.sequence("main").unwrap()));

    let text_of = |message: &dim_agent_types::Message| match &message.content {
        dim_agent_types::MessageContent::Text(text) => text.clone(),
        other => panic!("a keyed block renders as one text part: {other:?}"),
    };
    let baseline_map: std::collections::BTreeMap<_, _> = baseline
        .iter()
        .map(|entry| {
            (
                entry.key.clone(),
                (entry.source_event.clone(), text_of(&entry.message)),
            )
        })
        .collect();
    let reply_map: std::collections::BTreeMap<_, _> = entries(&reply)
        .iter()
        .map(|entry| {
            (
                entry["key"].as_str().unwrap().to_owned(),
                (
                    entry["inputId"].as_str().unwrap().to_owned(),
                    entry["text"].as_str().unwrap().to_owned(),
                ),
            )
        })
        .collect();
    assert_eq!(
        reply_map.keys().collect::<Vec<_>>(),
        baseline_map.keys().collect::<Vec<_>>(),
        "both sides report the same live keys"
    );
    // The reply names the `input/message` event that carried the block; the
    // renderer names the `input/consumed` event that placed it in the
    // baseline (`render_input` receives the consumed event, history.rs:855,
    // :1046). Both ids are the same input fact, one event apart in the
    // Timeline.
    let consumed_event = |input_id: &str| {
        events
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::InputConsumed(data) if data.input_id == input_id => Some(&event.id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{input_id} is a consumed input"))
    };
    for (key, (input_id, text)) in &reply_map {
        let (source_event, baseline_text) = &baseline_map[key];
        assert_eq!(text, baseline_text, "{key} carries the baseline's bytes");
        assert!(
            events
                .iter()
                .any(|event| matches!(&event.kind, EventKind::InputMessage(_))
                    && event.id == *input_id),
            "{key} names its input/message event"
        );
        assert_eq!(
            source_event,
            consumed_event(input_id),
            "{key}'s baseline entry names the consumed fact of the same input"
        );
    }
}
